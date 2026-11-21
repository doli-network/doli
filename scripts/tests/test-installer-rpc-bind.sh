#!/usr/bin/env bash
# INC-I-308 M3 — installers: RPC bind, token placement, operator guidance.
# REQ-RPC-ADMIN-INSTALL — Decision: a FAIL means an installer exposes RPC on 0.0.0.0, leaks the admin token into a unit/argv, or leaves the operator without the token-file guidance.
# OUTPUT CONTRACT: installer/unit text (rpc-bind value, Environment= lines, argv, token-file writes) and printed post-install text;
#   explorer relay (listen address, forwarded headers). No runtime outputs — static text assertions only.
# INPUT PARTITIONS: CLI unit writer (systemd + launchd); shell/ps1/deb/docker installers; local plist writers;
#   operator fleet generator (0.0.0.0 by design); explorer relay (JSON-RPC proxy + WebSocket raw-header path).
set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 2

FAILS=0
pass() { echo "PASS $1"; }
fail() { echo "FAIL $1${2:+ — $2}"; FAILS=$((FAILS + 1)); }
need() { [ -f "$1" ] || { fail "file present: $1"; return 1; }; }

CLI_FILES=(bins/cli/src/cmd_service.rs bins/cli/src/service_unit.rs)
SAFE_INSTALLERS=(scripts/install.sh scripts/install.ps1 bins/node/postinst bins/node/postinst.sh docker/docker-entrypoint.sh
  scripts/install-local-services.sh scripts/install-stress-services.sh testnetlinux/scripts/install-services.sh)
FLEET=scripts/install-services.sh

# rpc_bind_any <file> — prints offending lines where --rpc-bind is 0.0.0.0 (same line, or next line for plist/array form)
rpc_bind_any() {
  awk 'prev && /0\.0\.0\.0/ { print FILENAME ":" NR ": " $0 }
       { prev = 0 }
       /rpc-bind|rpc_bind/ { if ($0 ~ /0\.0\.0\.0/) print FILENAME ":" NR ": " $0; else prev = 1 }' "$1"
}
# token_leak <file> — token placed in a unit Environment, an env assignment, argv, or written to the token file
token_leak() {
  grep -nE 'Environment=.*(TOKEN|token)|<key>DOLI_RPC_ADMIN_TOKEN</key>|DOLI_RPC_ADMIN_TOKEN=|--admin-token|--rpc-admin-token|(>|tee|fs::write|Out-File|Set-Content|install -m)[^#]*rpc-admin\.token' "$1" \
    | sed "s|^|$1:|"
}
# printed_text <file> — echo/printf lines plus heredoc bodies of a shell script
printed_text() {
  awk '
    tag != "" { if ($0 ~ "^[[:space:]]*" tag "[[:space:]]*$") { tag = ""; next } print; next }
    /^[[:space:]]*(echo|printf|say|info|warn|ok|note|log)[[:space:]]/ { print }
    match($0, /<<-?[[:space:]]*["\047]?[A-Za-z_][A-Za-z0-9_]*["\047]?/) {
      t = substr($0, RSTART, RLENGTH); sub(/<<-?[[:space:]]*/, "", t); gsub(/["\047]/, "", t); tag = t
    }' "$1"
}
# rust_strings_before_tests <file> — string-literal lines of the non-test part of a Rust file
rust_strings_before_tests() {
  awk '/#\[cfg\(test\)\]/ { exit } /"[^"]*"/ { print }' "$1"
}

# a. doli service install templates
abad=""
for f in "${CLI_FILES[@]}"; do
  need "$f" || continue
  abad+="$(rpc_bind_any "$f")"
  abad+="$(grep -nE '"0\.0\.0\.0"' "$f" | sed "s|^|$f:|")"
  abad+="$(token_leak "$f")"
done
abad+="$(grep -nE 'DOLI_RPC_ADMIN_TOKEN|rpc-admin\.token' bins/cli/src/service_unit.rs 2>/dev/null | sed 's|^|service_unit.rs:|')"
[ -z "$abad" ] && pass "a doli service install: no 0.0.0.0, no token in unit/plist" \
  || fail "a doli service install: no 0.0.0.0, no token in unit/plist" "$(printf '\n%s' "$abad")"

# b. end-user / local installers: no 0.0.0.0 rpc-bind, no token written
bbad=""
for f in "${SAFE_INSTALLERS[@]}"; do
  need "$f" || continue
  bbad+="$(rpc_bind_any "$f")"
  bbad+="$(token_leak "$f")"
done
[ -z "$bbad" ] && pass "b installers: no 0.0.0.0 rpc-bind, no token written" \
  || fail "b installers: no 0.0.0.0 rpc-bind, no token written" "$(printf '\n%s' "$bbad")"

# c. operator fleet generator prints token guidance (token file + ssh tunnel)
if need "$FLEET"; then
  txt="$(printed_text "$FLEET")"
  printf '%s\n' "$txt" | grep -q 'rpc-admin\.token' && pass "c $FLEET prints rpc-admin.token guidance" \
    || fail "c $FLEET prints rpc-admin.token guidance"
  printf '%s\n' "$txt" | grep -q 'ssh -L' && pass "c $FLEET prints ssh -L tunnel guidance" \
    || fail "c $FLEET prints ssh -L tunnel guidance"
fi

# d. install.sh and doli service install print the token-file location
if need scripts/install.sh; then
  printed_text scripts/install.sh | grep -q 'rpc-admin\.token' && pass "d install.sh prints rpc-admin.token guidance" \
    || fail "d install.sh prints rpc-admin.token guidance"
fi
if need bins/cli/src/cmd_service.rs; then
  rust_strings_before_tests bins/cli/src/cmd_service.rs | grep -q 'rpc-admin\.token' \
    && pass "d cmd_service.rs prints rpc-admin.token guidance" || fail "d cmd_service.rs prints rpc-admin.token guidance"
fi

# e. explorer relay: loopback listen, no Authorization forwarded
JS=testnetlinux/explorer/server.js
if need "$JS"; then
  lst="$(grep -nE '\.listen\(' "$JS")"
  ok=0
  printf '%s\n' "$lst" | grep -qE "\.listen\([^,]+,[[:space:]]*[\"']127\.0\.0\.1[\"']" && ok=1
  if [ "$ok" = 0 ] && printf '%s\n' "$lst" | grep -qE '\.listen\([^,]+,[[:space:]]*HOST[[:space:]]*[,)]'; then
    grep -qE "HOST[[:space:]]*=[^;]*[\"']127\.0\.0\.1[\"']" "$JS" \
      && ! grep -qE "HOST[[:space:]]*=[^;]*[\"']0\.0\.0\.0[\"']" "$JS" && ok=1
  fi
  [ "$ok" = 1 ] && pass "e $JS listens on 127.0.0.1" || fail "e $JS listens on 127.0.0.1" "listen: $lst"
  hdr="$(grep -niE "[\"']?authorization[\"']?[[:space:]]*:" "$JS")"
  [ -z "$hdr" ] && pass "e $JS sets no authorization header" || fail "e $JS sets no authorization header" "$hdr"
  if grep -q 'rawHeaders' "$JS"; then
    grep -qiE "/authorization/i|[\"']authorization[\"']" "$JS" \
      && pass "e $JS strips authorization from forwarded raw headers" \
      || fail "e $JS strips authorization from forwarded raw headers" "req.rawHeaders forwarded wholesale"
  else
    pass "e $JS forwards no raw headers"
  fi
fi

# g. explorer network.html: -32008 on verifyChainIntegrity is shown, never swallowed; no token in the browser
HTML=testnetlinux/explorer/network.html
if need "$HTML"; then
  grep -q -- '-32008' "$HTML" && pass "g $HTML handles -32008" || fail "g $HTML handles -32008" "no -32008 branch"
  grep -qi 'admin token' "$HTML" && pass "g $HTML shows admin token message" \
    || fail "g $HTML shows admin token message" "no visible 'admin token' text"
  gbad="$(grep -nE 'Authorization|Bearer|rpc-admin\.token' "$HTML")"
  [ -z "$gbad" ] && pass "g $HTML carries no token or auth header" || fail "g $HTML carries no token or auth header" "$gbad"
fi

# f. no installer puts a token on a command line or in a unit Environment= line
fbad=""
for f in "${CLI_FILES[@]}" "${SAFE_INSTALLERS[@]}" "$FLEET"; do
  [ -f "$f" ] || continue
  fbad+="$(grep -nE 'Environment=.*(TOKEN|token)|ExecStart=.*(TOKEN|token)|--admin-token|--rpc-admin-token|DOLI_RPC_ADMIN_TOKEN=[^[:space:]]+[[:space:]]+[^[:space:]]*doli' "$f" | sed "s|^|$f:|")"
done
[ -z "$fbad" ] && pass "f no token on a command line or Environment= line" \
  || fail "f no token on a command line or Environment= line" "$(printf '\n%s' "$fbad")"

echo "---"
[ "$FAILS" = 0 ] && { echo "ALL PASS"; exit 0; }
echo "$FAILS check(s) failed"; exit 1
