#!/usr/bin/env bash
# gauntlet-gs063.sh — GS-063 backfill-hostile-peer-snap-gap (INC-I-308 R-1/R-2).
#
# Usage: bash scripts/gauntlet-gs063.sh   (GA_BIN = node binary, default target/release/doli-node)
# Env:   GA_BIN, GS063_RPC (8663), GS063_P2P (30463), GS063_METRICS (9163), GS063_PROXY (8667),
#        GS063_HONEST (http://127.0.0.1:8501), GS063_BOOT (30300 30301 30302 30303),
#        GS063_SNAP_SECS (900), GS063_FILL_SECS (180), GS063_K (20), GA_WORK
#
# Starts ONE victim node (own data dir under GA_WORK, own ports, loopback bind shim, --no-dht,
# bootstrap = the local testnet P2P ports, snap sync ON). It snap-syncs, so it has the gap
# below snap_h (h=1 may be stored by orphan chase). The shared ~/testnet nodes are only READ (RPC getChainInfo/getBlockRaw/
# getBlockByHeight); nothing under ~/testnet is written, stopped or restarted.
# A hostile peer (scripts/gauntlet-gs063-proxy.py) answers the tip preflight with the victim's own
# hash and tampers the block at one height; heights below a floor answer "no block" so each pass ends.
#   gs063-hostile-refused   passes swap / flip_header / flip_tail at the gap top, then swap at
#                           top-K after K honest blocks: backfillStatus.error is set, and at every
#                           tampered height the victim holds no block or the honest bytes.
#   gs063-honest-prefix     the mid-walk pass stores exactly the K honest blocks above the tamper.
#   gs063-honest-fills      backfillFromPeer from the honest node starts, reports no error within
#                           GS063_FILL_SECS, and the sampled heights hold the honest bytes.
# METRIC imported/rate/projected_secs report how far the honest fill got (the full gap is ~tip blocks).
# h=1 and the gap middle are not reached by a top-down walk in a bounded run; the in-process tests
# r1_backfill_refuses_forged_{h1_and_snap_gap,gap_middle} cover them.
# The victim and the proxy are stopped by PID at exit; the victim data dir is deleted (logs kept).
set -uo pipefail
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/gauntlet-rpc-admin-lib.sh"
mkdir -p "$GA_WORK"
RPC="${GS063_RPC:-8663}"; P2P="${GS063_P2P:-30463}"; MET="${GS063_METRICS:-9163}"; PX="${GS063_PROXY:-8667}"
HONEST="${GS063_HONEST:-http://127.0.0.1:8501}"; K="${GS063_K:-20}"
PROXY_PID=""
cleanup(){ [ -n "$PROXY_PID" ] && kill "$PROXY_PID" 2>/dev/null; ga_scratch_stop; }
trap cleanup EXIT
FAILS=""; fail(){ FAILS="$FAILS; $*"; echo "  ✗ $*"; }
j(){ python3 -I -c "import json,sys; d=json.loads(sys.stdin.read() or 'null'); print(eval(sys.argv[1]))" "$1" 2>/dev/null; }

echo "GS-063 backfill-hostile-peer-snap-gap  bin=$GA_BIN  version=$("$GA_BIN" --version 2>/dev/null)"
echo "  bin_sha256=$(shasum -a 256 "$GA_BIN" | awk '{print $1}')"
HTIP=$(ga_rpc "$HONEST" getChainInfo '{}' | j 'd["result"]["bestHeight"]')
[ -n "$HTIP" ] || { echo "VERDICT: NOT_MADE (honest node $HONEST does not answer)"; exit 3; }
ga_metric honest_tip=$HTIP

# Victim: the lib's scratch start pins --no-snap-sync, so this row starts its own (same safety rules).
for port in "$RPC" "$P2P" "$MET" "$PX"; do ga_port_free "$port" || { echo "VERDICT: NOT_MADE (port $port busy)"; exit 3; }; done
[ -x "$GA_BIN" ] || { echo "VERDICT: NOT_MADE (no binary $GA_BIN)"; exit 3; }
SCRATCH_DIR="$GA_WORK/gs063"; mkdir -p "$SCRATCH_DIR/data"
SHIM="$(ga_shim)"; BOOT=""
for b in ${GS063_BOOT:-30300 30301 30302 30303}; do BOOT="$BOOT --bootstrap /ip4/127.0.0.1/tcp/$b"; done
( cd "$SCRATCH_DIR" && exec env -u DOLI_RPC_ADMIN_TOKEN -u DOLI_RPC_TRUSTED_PROXIES \
    ${SHIM:+DYLD_INSERT_LIBRARIES=$SHIM} "$GA_BIN" --network testnet --data-dir "$SCRATCH_DIR/data" \
    --log-level info run --p2p-port "$P2P" --rpc-port "$RPC" --rpc-bind 127.0.0.1 \
    --metrics-port "$MET" --no-dht --no-discv5 --no-auto-update --yes $BOOT \
    > "$SCRATCH_DIR/node.log" 2>&1 ) &
SCRATCH_PID=$!; SCRATCH_RPC="http://127.0.0.1:$RPC"; V="$SCRATCH_RPC"
echo "SCRATCH_STARTED pid=$SCRATCH_PID rpc=$RPC p2p=$P2P shim=${SHIM:-none} dir=$SCRATCH_DIR"

vhash(){ ga_rpc "$V" getBlockByHeight "{\"height\":$1}" | j 'd["result"]["hash"] if d.get("result") else ""'; }
vraw(){ ga_rpc "$V" getBlockRaw "{\"height\":$1}" | j 'd["result"]["block"] if d.get("result") else ""'; }
hraw(){ ga_rpc "$HONEST" getBlockRaw "{\"height\":$1}" | j 'd["result"]["block"] if d.get("result") else ""'; }

# Wait for snap: victim near the honest tip and h=tip/2 absent (orphan chase may store h=1, so h=1 is
# not a snap signal). snap_h = lowest height of the stored top run (bisect over (tip/2, tip]).
deadline=$(( $(date +%s) + ${GS063_SNAP_SECS:-900} )); VTIP=0
while [ "$(date +%s)" -lt "$deadline" ]; do
  kill -0 "$SCRATCH_PID" 2>/dev/null || { tail -5 "$SCRATCH_DIR/node.log"; echo "VERDICT: NOT_MADE (victim exited)"; exit 3; }
  VTIP=$(ga_rpc "$V" getChainInfo '{}' | j 'd["result"]["bestHeight"]'); VTIP=${VTIP:-0}
  [ "$VTIP" -gt 1000 ] && [ -z "$(vhash $((VTIP / 2)))" ] && [ $((HTIP - VTIP)) -lt 200 ] && break
  sleep 5
done
[ "$VTIP" -gt 1000 ] && [ -z "$(vhash $((VTIP / 2)))" ] || { echo "VERDICT: NOT_MADE (no snap-synced gap; tip=$VTIP)"; exit 3; }
lo=$((VTIP / 2)); hi=$VTIP
while [ $((hi - lo)) -gt 1 ]; do mid=$(((lo + hi) / 2)); if [ -n "$(vhash $mid)" ]; then hi=$mid; else lo=$mid; fi; done
SNAP=$hi; TOP=$((SNAP - 1))
# The real gap: walk down from TOP until a stored height; the hostile passes need K+10 empty heights.
GAP_LO=$TOP
while [ $((TOP - GAP_LO)) -lt $((K + 10)) ] && [ "$GAP_LO" -gt 1 ] && [ -z "$(vhash $((GAP_LO - 1)))" ]; do GAP_LO=$((GAP_LO - 1)); done
ga_metric victim_tip=$VTIP snap_h=$SNAP gap_top=$TOP gap_checked="$GAP_LO-$TOP" h1_stored="$([ -n "$(vhash 1)" ] && echo yes || echo no)"
[ -z "$(vhash "$TOP")" ] && [ $((TOP - GAP_LO)) -ge $((K + 10)) ] || { echo "VERDICT: NOT_MADE (gap too small: $GAP_LO-$TOP)"; exit 3; }

TF=$(find "$SCRATCH_DIR/data" -name rpc-admin.token 2>/dev/null | head -1)
TOKEN=""; [ -n "$TF" ] && TOKEN=$(tr -d '[:space:]' < "$TF")
CFG="$GA_WORK/gs063-proxy.json"; PLOG="$GA_WORK/gs063-proxy.log"
echo '{"mode":"none","target":0,"floor":0}' > "$CFG"
python3 -I "$GA_ROOT/scripts/gauntlet-gs063-proxy.py" "$PX" "$HONEST" "$V" "$CFG" "$PLOG" &
PROXY_PID=$!; sleep 1
kill -0 "$PROXY_PID" 2>/dev/null || { echo "VERDICT: NOT_MADE (proxy did not start)"; exit 3; }

status(){ ga_rpc "$V" backfillStatus '{}' "$TOKEN"; }
wait_idle(){ local t=$(( $(date +%s) + $1 )) s
  while [ "$(date +%s)" -lt "$t" ]; do s=$(status); [ "$(printf '%s' "$s" | j 'd["result"]["running"]')" = "False" ] && { printf '%s' "$s"; return 0; }; sleep 1; done
  status; return 1; }
backfill(){ ga_rpc "$V" backfillFromPeer "{\"rpc_url\":\"$1\"}" "$TOKEN"; }

clean_at(){ # height -> fail unless the victim holds nothing or the honest bytes at it
  local got; got=$(vraw "$1")
  [ -z "$got" ] || [ "$got" = "$(hraw "$1")" ] || fail "$2: victim holds non-honest bytes at h=$1"
}

hostile(){ # name mode target floor expected_imported
  printf '{"mode":"%s","target":%s,"floor":%s}\n' "$2" "$3" "$4" > "$CFG"
  local r s err imp; r=$(backfill "http://127.0.0.1:$PX")
  s=$(wait_idle 120) || fail "gs063-hostile-refused: pass $1 still running after 120s"
  err=$(printf '%s' "$s" | j 'd["result"]["error"]'); imp=$(printf '%s' "$s" | j 'd["result"]["imported"]')
  ga_metric pass=$1 mode=$2 target=$3 "reply=$(printf '%s' "$r" | head -c 160)" imported=$imp "error=$(printf '%s' "$err" | head -c 160)"
  { [ -n "$err" ] && [ "$err" != "None" ]; } || fail "gs063-hostile-refused: pass $1 reported no error"
  clean_at "$3" gs063-hostile-refused
  [ "$imp" = "$5" ] || fail "gs063-honest-prefix: pass $1 imported=$imp want $5"
}

hostile swap swap "$TOP" "$TOP" 0
hostile flip_header flip_header "$TOP" "$TOP" 0
hostile flip_tail flip_tail "$TOP" "$TOP" 0
[ -z "$(vraw "$TOP")" ] || fail "gs063-hostile-refused: gap top h=$TOP filled by a hostile pass"
hostile midwalk swap $((TOP - K)) $((TOP - K - 3)) "$K"
for h in $(seq $((TOP - K + 1)) "$TOP"); do
  [ "$(vraw "$h")" = "$(hraw "$h")" ] || { fail "gs063-honest-prefix: h=$h not the honest block"; break; }
done
ga_metric tamper_log="$(grep -c TAMPER "$PLOG" 2>/dev/null) tampers, $(grep -c PREFLIGHT "$PLOG" 2>/dev/null) preflights"

# Honest fill from the real testnet node.
t0=$(date +%s); r=$(backfill "$HONEST")
[ "$(printf '%s' "$r" | j 'd["result"]["started"]')" = "True" ] || fail "gs063-honest-fills: not started: $(printf '%s' "$r" | head -c 200)"
s=$(wait_idle "${GS063_FILL_SECS:-180}"); done_rc=$?; el=$(( $(date +%s) - t0 ))
imp=$(printf '%s' "$s" | j 'd["result"]["imported"]'); err=$(printf '%s' "$s" | j 'd["result"]["error"]')
rate=$(( ${imp:-0} / (el > 0 ? el : 1) ))
ga_metric honest_imported=$imp elapsed=$el rate_per_s=$rate finished=$([ $done_rc = 0 ] && echo yes || echo no) \
  projected_secs=$([ "$rate" -gt 0 ] && echo $(( (TOP - K) / rate )) || echo unknown) "error=$err"
[ -z "$err" ] || [ "$err" = "None" ] || fail "gs063-honest-fills: error $err"
[ "${imp:-0}" -gt 0 ] || fail "gs063-honest-fills: nothing imported in ${el}s"
for h in $((TOP - K)) $((TOP - K - 1)) $((TOP - K - ${imp:-1} + 1)); do
  [ "$h" -ge 1 ] && [ "$(vraw "$h")" = "$(hraw "$h")" ] || fail "gs063-honest-fills: h=$h not the honest block"
done

if [ -z "$FAILS" ]; then echo "VERDICT: PASS"; exit 0; fi
echo "VERDICT: FAIL$FAILS"; exit 1
