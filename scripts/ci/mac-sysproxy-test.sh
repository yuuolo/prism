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
echo "before: $before"
exit $fail
