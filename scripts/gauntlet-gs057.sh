#!/usr/bin/env bash
# gauntlet-gs057.sh — GS-057 rpc-relay-does-not-launder-loopback (INC-I-308; AUDIT-P1-RPC-006,
# AUDIT-P2-RPC-001).
#
# Usage: bash scripts/gauntlet-gs057.sh
# Env:   GA_BIN, GS057_RPC (8592), GS057_P2P (30392), GS057_METRICS (9092), GS057_RELAY (8597), GA_WORK
#
# Before INC-I-308 the node trusted every request from 127.0.0.1, so any local relay (explorer
# server.js style: it re-POSTs the client's JSON body to the node from loopback, without the
# client's Authorization) turned a remote anonymous caller into a "loopback" admin caller.
# This row starts ONE isolated scratch node and a throwaway python relay on 127.0.0.1 that forwards
# the body only. Through the relay it asserts:
#   gs057-relay-forwards        getChainInfo through the relay -> result (the relay works)
#   gs057-admin-refused         every ADMIN_METHODS name through the relay -> -32008
#   gs057-node-saw-loopback     the node log names 127.0.0.1 as the source of the refused calls
# Relay and scratch node are stopped by PID at exit (both checked gone).
set -uo pipefail
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/gauntlet-rpc-admin-lib.sh"
mkdir -p "$GA_WORK"
RELAY_PID=""
stop_all(){
  if [ -n "$RELAY_PID" ]; then
    kill -TERM "$RELAY_PID" 2>/dev/null; sleep 1
    kill -0 "$RELAY_PID" 2>/dev/null && kill -KILL "$RELAY_PID" 2>/dev/null
    kill -0 "$RELAY_PID" 2>/dev/null && echo "RELAY_STOPPED pid=$RELAY_PID gone=no" || echo "RELAY_STOPPED pid=$RELAY_PID gone=yes"
  fi
  ga_scratch_stop
}
trap stop_all EXIT

FAILS=""; fail(){ FAILS="$FAILS; $*"; echo "  ✗ $*"; }
echo "GS-057 rpc-relay-does-not-launder-loopback  bin=$GA_BIN  version=$("$GA_BIN" --version 2>/dev/null)"
echo "  bin_sha256=$(shasum -a 256 "$GA_BIN" | awk '{print $1}')"
RELAY_PORT="${GS057_RELAY:-8597}"
ga_port_free "$RELAY_PORT" || { echo "VERDICT: NOT_MADE (relay port $RELAY_PORT busy)"; exit 3; }
ga_scratch_start gs057 "${GS057_RPC:-8592}" "${GS057_P2P:-30392}" "${GS057_METRICS:-9092}" \
  || { echo "VERDICT: NOT_MADE (scratch node did not start)"; exit 3; }

RELAY_PY="$GA_WORK/relay.py"
cat > "$RELAY_PY" <<'PY'
import sys, urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer
PORT, TARGET = int(sys.argv[1]), sys.argv[2]
class H(BaseHTTPRequestHandler):
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        # Forward the body only: no Authorization, like a public explorer proxy.
        req = urllib.request.Request(TARGET, data=body, headers={"Content-Type": "application/json"})
        try:
            out = urllib.request.urlopen(req, timeout=20).read()
        except urllib.error.HTTPError as e:
            out = e.read()
        self.send_response(200); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(out))); self.end_headers(); self.wfile.write(out)
    def log_message(self, *a): pass
HTTPServer(("127.0.0.1", PORT), H).serve_forever()
PY
python3 -I "$RELAY_PY" "$RELAY_PORT" "$SCRATCH_RPC" > "$GA_WORK/relay.log" 2>&1 &
RELAY_PID=$!
echo "RELAY_STARTED pid=$RELAY_PID listen=127.0.0.1:$RELAY_PORT target=$SCRATCH_RPC"
RELAY="http://127.0.0.1:$RELAY_PORT"
for i in $(seq 1 20); do ga_port_free "$RELAY_PORT" || break; sleep 0.5; done

c=$(ga_code "$(ga_rpc "$RELAY" getChainInfo '{}')"); ga_metric relay getChainInfo=$c
[ "$c" = "ok" ] || { echo "VERDICT: NOT_MADE (relay does not forward: getChainInfo -> $c)"; exit 3; }

for m in $(ga_admin_methods); do
  c=$(ga_code "$(ga_rpc "$RELAY" "$m" '{}')"); ga_metric relay $m=$c
  [ "$c" = "$GA_UNAUTH" ] || fail "gs057-admin-refused: $m via relay -> $c"
done
SEEN=$(grep -c "rejected" "$SCRATCH_DIR/node.log" 2>/dev/null)
LOOP=$(grep "rejected" "$SCRATCH_DIR/node.log" 2>/dev/null | grep -c "127.0.0.1")
ga_metric node_log_rejected=$SEEN from_loopback=$LOOP
ga_metric node_log_sample="$(grep -m1 'rejected' "$SCRATCH_DIR/node.log" | sed 's/^.*\(Admin method\)/\1/' | head -c 200)"
[ "${LOOP:-0}" -ge 1 ] || fail "gs057-node-saw-loopback: no refused-from-127.0.0.1 line in node.log"

if [ -z "$FAILS" ]; then echo "VERDICT: PASS"; exit 0; fi
echo "VERDICT: FAIL$FAILS"; exit 1
