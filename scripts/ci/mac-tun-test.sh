#!/bin/bash
# macOS TUN smoke test (CI runners have passwordless sudo): runs prism-core
# as root with TUN + auto-route, then checks that
#   - a utun interface comes up with 172.19.0.1 and the 0/1 + 128/1 routes,
#   - ordinary traffic flows through it (curl to the internet works and the
#     core sees the connection on its Tun inbound, owned by curl),
#   - everything is gone again once the core exits.
# usage: mac-tun-test.sh <prism-core>
set -uo pipefail
CORE="$1"
W="$(mktemp -d)"
fail=0
ok() { echo "PASS $*"; }
bad() { echo "FAIL $*"; fail=1; }

cat > "$W/config.yaml" <<'EOF'
mixed-port: 17890
external-controller: 127.0.0.1:19090
secret: ""
log-level: info
tun:
  enable: true
  auto-route: true
proxies: []
rules:
  - MATCH,DIRECT
EOF

echo "---- routes before"
netstat -rn -f inet | head -25
DEFIF=$(route -n get default | awk '/interface:/{print $2}')
echo "default interface: $DEFIF"

sudo "$CORE" -f "$W/config.yaml" -d "$W" > "$W/core.log" 2>&1 &
for _ in $(seq 1 50); do
  curl -s -o /dev/null http://127.0.0.1:19090/version && break
  sleep 0.2
done
sleep 1

IF=$(ifconfig | awk '/^utun/{name=$1} /inet 172\.19\.0\.1 /{print name}' | tr -d ':' | head -1)
[ -n "$IF" ] && ok "utun interface $IF has 172.19.0.1" || bad "no utun with 172.19.0.1"
ROUTES=$(netstat -rn -f inet)
echo "$ROUTES" | grep -Eq "^0/1 .*${IF:-utunX}" && ok "0/1 routed into $IF" || bad "0/1 route missing"
echo "$ROUTES" | grep -Eq "^128.0/1 .*${IF:-utunX}" && ok "128.0/1 routed into $IF" || bad "128.0/1 route missing"

echo "---- routes with TUN"
netstat -rn -f inet | head -30
echo "---- scoped lookup on $DEFIF"
route -n get -ifscope "$DEFIF" 1.1.1.1 || true
# curl --interface uses the same IP_BOUND_IF the core pins its sockets with.
code=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 20 --interface "$DEFIF" https://www.example.com/ || true)
[ "$code" = "200" ] && ok "a socket bound to $DEFIF bypasses the tunnel: HTTP $code" || bad "socket bound to $DEFIF: HTTP $code"

code=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 20 https://www.example.com/ || true)
[ "$code" = "200" ] && ok "https://www.example.com through TUN: HTTP $code" || bad "curl through TUN: HTTP $code"

# A slow download keeps a connection open long enough to inspect it.
curl -sS -o /dev/null --max-time 15 --limit-rate 50k \
  https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/geoip.dat &
DL=$!
sleep 4
CONNS=$(curl -s http://127.0.0.1:19090/connections)
echo "$CONNS" | python3 -c '
import json, sys
c = json.load(sys.stdin).get("connections") or []
tun = [x for x in c if x["metadata"].get("type") == "Tun"]
print("tun connections:", [(x["metadata"].get("host") or x["metadata"].get("destinationIP"), x["metadata"].get("process")) for x in tun])
sys.exit(0 if any(x["metadata"].get("process") == "curl" for x in tun) else 1)
' && ok "core sees the download on its Tun inbound, owned by curl" || bad "no Tun connection owned by curl"
kill $DL 2>/dev/null

# DNS: a query to the TUN's own DNS address is answered.
if dig +time=3 +tries=1 @172.19.0.2 www.example.com A +short | grep -Eq '^[0-9.]+$'; then
  ok "DNS query to 172.19.0.2 answered"
else
  bad "no DNS answer from 172.19.0.2"
fi

sudo pkill -f "$CORE" || true
sleep 2
ifconfig | grep -q "inet 172.19.0.1 " && bad "utun still up after exit" || ok "utun removed after exit"
netstat -rn -f inet | grep -Eq "^0/1 " && bad "0/1 route left behind" || ok "routes removed after exit"
code=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 20 https://www.example.com/ || true)
[ "$code" = "200" ] && ok "network fine after exit" || bad "network broken after exit: HTTP $code"

echo "---- core log"
tail -40 "$W/core.log"
exit $fail
