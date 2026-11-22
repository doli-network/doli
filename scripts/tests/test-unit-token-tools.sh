#!/usr/bin/env bash
# INC-I-308 send-back 1 (D1+D2) — operator tools reach a node whose token is ONLY in its unit env.
# Usage: scripts/tests/test-unit-token-tools.sh <doli-node> <doli>
# Decision: a FAIL means `sudo doli -r http://127.0.0.1:<port> guardian halt` (or the shell/Python
# helpers) is refused -32008 on a structural host (token in the systemd Environment=, no token file),
# the token leaks to tool output, the node log or argv, or a caller who cannot read the token file
# pauses the node anyway / gets a hint that names the wrong file.
# OUTPUT CONTRACT: per tool — exit code, stdout+stderr (no -32008, no token); node state via the
# public getGuardianStatus (production_paused); node.log and `ps -axo command` (no token).
# INPUT PARTITIONS: caller in {can read the token file, cannot read it (chmod 000)};
#   tool in {doli guardian halt, doli guardian resume, doli chain-verify, shell helper, Python helper}.
# SAFETY: one scratch testnet node (own temp data dir, own ports, no bootstrap, loopback shim),
# stopped by its own PID at exit. Nothing under ~/testnet is touched.
set -uo pipefail
NODE_BIN="${1:?usage: $0 <doli-node> <doli>}"; CLI_BIN="${2:?usage: $0 <doli-node> <doli>}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
NODE_BIN="$(cd "$(dirname "$NODE_BIN")" && pwd)/$(basename "$NODE_BIN")"; CLI_BIN="$(cd "$(dirname "$CLI_BIN")" && pwd)/$(basename "$CLI_BIN")"
FAILS=0
pass() { echo "PASS $1"; }
fail() { echo "FAIL $1${2:+ — $2}"; FAILS=$((FAILS + 1)); }

T="$(mktemp -d "${TMPDIR:-/tmp}/unit-token-tools.XXXXXX")"
export GA_BIN="$NODE_BIN" GA_WORK="$T/ga"; mkdir -p "$GA_WORK" "$T/home" "$T/out"
# shellcheck source=../gauntlet-rpc-admin-lib.sh
. "$ROOT/scripts/gauntlet-rpc-admin-lib.sh"
PSPID=""
cleanup() {
  [ -n "$PSPID" ] && kill "$PSPID" 2>/dev/null
  [ -n "${SCRATCH_DIR:-}" ] && chmod 600 "$SCRATCH_DIR/data/rpc-admin.token" 2>/dev/null
  ga_scratch_stop
  rm -rf "$T"
}
trap cleanup EXIT

TOK="$(python3 -I -c 'import secrets; print(secrets.token_hex(32))')"
base=38640; while ! { ga_port_free "$base" && ga_port_free $((base + 1)) && ga_port_free $((base + 2)); }; do base=$((base + 3)); done
RPC=$base
ga_scratch_start unit "$RPC" $((base + 1)) $((base + 2)) "DOLI_RPC_ADMIN_TOKEN=$TOK" || { echo "VERDICT: NOT_MADE (scratch node)"; exit 3; }
DATA="$SCRATCH_DIR/data"
URL="http://127.0.0.1:$RPC"

paused() { ga_rpc "$URL" getGuardianStatus '{}' | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["result"]["production_paused"])' 2>/dev/null; }
# tool <name> <cmd...> — run as an operator shell: no token env, no DOLI_DATA_DIR, empty HOME
tool() {
  local name="$1"; shift
  env -u DOLI_RPC_ADMIN_TOKEN -u DOLI_DATA_DIR HOME="$T/home" "$@" > "$T/out/$name" 2>&1
}
( while :; do ps -axo command >> "$T/ps.txt" 2>/dev/null; sleep 0.1; done ) &
PSPID=$!

tool halt "$CLI_BIN" -r "$URL" guardian halt --yes; rc=$?
{ [ $rc = 0 ] && ! grep -q -- -32008 "$T/out/halt"; } && pass "1 doli guardian halt reaches pauseProduction" \
  || fail "1 doli guardian halt reaches pauseProduction" "rc=$rc $(tr '\n' ' ' < "$T/out/halt" | cut -c1-300)"
[ "$(paused)" = True ] && pass "1 node is paused after halt" || fail "1 node is paused after halt" "production_paused=$(paused)"
tool resume "$CLI_BIN" -r "$URL" guardian resume; rc=$?
{ [ $rc = 0 ] && ! grep -q -- -32008 "$T/out/resume"; } && pass "2 doli guardian resume reaches resumeProduction" \
  || fail "2 doli guardian resume reaches resumeProduction" "rc=$rc $(tr '\n' ' ' < "$T/out/resume" | cut -c1-300)"
[ "$(paused)" = False ] && pass "2 node is running after resume" || fail "2 node is running after resume" "production_paused=$(paused)"
tool verify "$CLI_BIN" -r "$URL" chain-verify; rc=$?
# a chain at tip 0 answers -32602 (range) — that answer is past the admin gate
{ ! grep -q -- -32008 "$T/out/verify" && grep -qE "Tip Height|RPC error -32602" "$T/out/verify"; } && pass "3 doli chain-verify reaches verifyChainIntegrity" \
  || fail "3 doli chain-verify reaches verifyChainIntegrity" "rc=$rc $(tr '\n' ' ' < "$T/out/verify" | cut -c1-300)"

shcall() {
  tool "sh-$1" bash -c '. "$1/scripts/lib/rpc-admin-token.sh"
    curl -s -m 20 -X POST "http://127.0.0.1:$2" -H "Content-Type: application/json" \
      -H @<(rpc_admin_header_for "$3" "127.0.0.1:$2") -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$3\",\"params\":{}}"' _ "$ROOT" "$RPC" "$1"
  [ "$(ga_code "$(cat "$T/out/sh-$1")")" = ok ] && pass "4 shell helper $1 accepted" || fail "4 shell helper $1 accepted" "$(cut -c1-200 "$T/out/sh-$1")"
}
shcall pauseProduction; [ "$(paused)" = True ] && pass "4 shell helper paused the node" || fail "4 shell helper paused the node"
shcall resumeProduction

pycall() {
  tool "py-$1" python3 -I -c 'import json, sys, urllib.request, importlib.util as u
s = u.spec_from_file_location("rpc_admin_token", sys.argv[1]); m = u.module_from_spec(s); s.loader.exec_module(m)
h = {"Content-Type": "application/json"}; h.update(m.headers_for(sys.argv[3], int(sys.argv[2])))
body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": sys.argv[3], "params": {}}).encode()
print(urllib.request.urlopen(urllib.request.Request("http://127.0.0.1:" + sys.argv[2], body, h), timeout=20).read().decode())' \
    "$ROOT/scripts/lib/rpc_admin_token.py" "$RPC" "$1"
  [ "$(ga_code "$(cat "$T/out/py-$1")")" = ok ] && pass "5 Python helper $1 accepted" || fail "5 Python helper $1 accepted" "$(cut -c1-200 "$T/out/py-$1")"
}
pycall pauseProduction; [ "$(paused)" = True ] && pass "5 Python helper paused the node" || fail "5 Python helper paused the node"
pycall resumeProduction

# Negative: the caller cannot read the token file (non-root user vs a root-owned file).
if [ -e "$DATA/rpc-admin.token" ]; then chmod 000 "$DATA/rpc-admin.token"; fi
tool neg "$CLI_BIN" -r "$URL" guardian halt --yes; rc=$?
[ $rc != 0 ] && pass "6 unreadable token file -> halt exits non-zero" || fail "6 unreadable token file -> halt exits non-zero"
grep -q -- -32008 "$T/out/neg" && grep -qF "$DATA/rpc-admin.token" "$T/out/neg" && grep -q "run with sudo" "$T/out/neg" \
  && pass "6 -32008 hint names the node's token file and sudo" \
  || fail "6 -32008 hint names the node's token file and sudo" "$(grep -m1 hint "$T/out/neg" | cut -c1-300)"
[ "$(paused)" = False ] && pass "6 nothing paused without the token" || fail "6 nothing paused without the token"

kill "$PSPID" 2>/dev/null; wait "$PSPID" 2>/dev/null; PSPID=""
grep -qF "$TOK" "$T"/out/* && fail "7 token absent from tool output" "$(grep -lF "$TOK" "$T"/out/* | xargs -n1 basename | tr '\n' ' ')" || pass "7 token absent from tool output"
grep -qF "$TOK" "$SCRATCH_DIR/node.log" && fail "7 token absent from node log" || pass "7 token absent from node log"
grep -qF "$TOK" "$T/ps.txt" && fail "7 token absent from ps -axo command" || pass "7 token absent from ps -axo command ($(wc -l < "$T/ps.txt" | tr -d ' ') lines)"

ga_scratch_stop
echo "---"
[ "$FAILS" -eq 0 ] && { echo "ALL PASS"; exit 0; } || { echo "FAILS: $FAILS"; exit 1; }
