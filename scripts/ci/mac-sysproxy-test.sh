#!/bin/bash
# Checks that the networksetup calls Prism makes for the system proxy work
# for a normal admin user without sudo, then restores the settings.
set -uo pipefail
fail=0
SVC=$(networksetup -listallnetworkservices | tail -n +2 | grep -v '^\*' | head -1)
echo "service: $SVC"
before=$(networksetup -getwebproxy "$SVC")
run() {
  out=$("$@" 2>&1)
  if [ $? -ne 0 ] || echo "$out" | grep -q '\*\* Error'; then
    echo "FAIL $*: $out"
    fail=1
  else
    echo "PASS $*"
  fi
}
run networksetup -setwebproxy "$SVC" 127.0.0.1 17890
run networksetup -setsecurewebproxy "$SVC" 127.0.0.1 17890
run networksetup -setsocksfirewallproxy "$SVC" 127.0.0.1 17890
run networksetup -setproxybypassdomains "$SVC" localhost 127.0.0.1 '*.local'
run networksetup -setwebproxystate "$SVC" on
networksetup -getwebproxy "$SVC" | grep -q 'Enabled: Yes' && echo "PASS proxy reads back as enabled" || { echo "FAIL proxy not enabled"; fail=1; }
run networksetup -setwebproxystate "$SVC" off
run networksetup -setsecurewebproxystate "$SVC" off
run networksetup -setsocksfirewallproxystate "$SVC" off
# What the app does to DNS while TUN runs, and the restore.
dns_before=$(networksetup -getdnsservers "$SVC")
run networksetup -setdnsservers "$SVC" 223.5.5.5 119.29.29.29
networksetup -getdnsservers "$SVC" | grep -q '223.5.5.5' && echo "PASS DNS reads back as 223.5.5.5" || { echo "FAIL DNS not set"; fail=1; }
if echo "$dns_before" | grep -q "aren't any"; then
  run networksetup -setdnsservers "$SVC" Empty
else
  run networksetup -setdnsservers "$SVC" $dns_before
fi
echo "dns before: $dns_before / after restore: $(networksetup -getdnsservers "$SVC" | tr '\n' ' ')"
echo "before: $before"
exit $fail
