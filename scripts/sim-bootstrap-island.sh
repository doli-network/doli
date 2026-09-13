#!/usr/bin/env bash
# ============================================================================
# sim-bootstrap-island.sh — live reproduction of INC-I-221 on the LOCAL testnet.
#
# Two temporary NON-producer nodes (A, B) start with a copy of the seed's chain
# data (no peers.cache, no producer_gset.bin, no checkpoints/), and each one knows
# only the seed and the other node. The seed is stopped, so A and B pair with
# each other at 1 peer and fall behind. After SIM_ISLAND_SECS the seed returns.
#
#   * A binary WITHOUT the INC-I-221 fix re-dials bootstrap peers only at 0
#     peers, so A and B stay at 1 peer and behind:        exit 1 (STUCK).
#   * A binary WITH the fix re-dials while peers < production minimum, so A and
#     B reach >=2 peers and the fleet tip:                 exit 0 (RECOVERED).
#
# Run it twice for a proof: control on the old binary (must exit 1), treatment
# on the new binary (must exit 0). A control that does not stall means the
# experiment has an escape path; the script then exits 7 or 1 — never call such
# a run a proof.
#
# Usage:
#   SIM_CONFIRM=1 bash scripts/sim-bootstrap-island.sh <doli-node-binary> [label]
#   SIM_CONFIRM=1 bash scripts/sim-bootstrap-island.sh ~/testnet/bin/doli-node.old old
#   SIM_CONFIRM=1 bash scripts/sim-bootstrap-island.sh ~/testnet/bin/doli-node   new
#
# Safety:
#   * LOCAL testnet only (refuses unless the seed RPC reports network=testnet).
#   * Stops ONLY the seed, via scripts/testnet.sh (launchd). No fleet node other
#     than the seed is touched; nothing is wiped; nothing is written to the chain.
#   * The EXIT trap always stops A/B (exact pids, path-checked) and restarts the
#     seed if this script stopped it. It deletes A/B data dirs and keeps logs.
#   * `set -e` is intentionally NOT used: a benign zero-match `grep -c` must not
#     abort the run between "seed stopped" and the restore path.
#
# Env (defaults in brackets):
#   SIM_CONFIRM=1 (required)   SIM_TESTNET_DIR [$HOME/testnet]
#   SIM_SEED_RPC [8500]        SIM_SEED_P2P [30300]   SIM_REF_RPC [8501]
#   SIM_ISLAND_SECS [60]       SIM_OBSERVE_SECS [300]
#   SIM_A_P2P [30391] SIM_B_P2P [30393] SIM_A_RPC [8591] SIM_B_RPC [8593]
#   SIM_A_MET [9091]  SIM_B_MET [9093]  (discv5 uses P2P+1 over UDP)
#
# Exit: 0 recovered · 1 stuck · 2 usage/confirm · 3 pre-check · 4 seed stop
#       5 data copy · 6 temporary node died · 7 island not formed (invalid run)
# Output: summary on stdout; node logs in ${TMPDIR:-/tmp}/doli-bootstrap-island-<label>-<ts>/
# ============================================================================
set -u

BIN="${1:-}"
LABEL="${2:-run}"
REPO="$(cd "$(dirname "$0")/.." && pwd)"
TESTNET_DIR="${SIM_TESTNET_DIR:-$HOME/testnet}"
SEED_RPC="${SIM_SEED_RPC:-8500}"; SEED_P2P="${SIM_SEED_P2P:-30300}"; REF_RPC="${SIM_REF_RPC:-8501}"
ISLAND_SECS="${SIM_ISLAND_SECS:-60}"; OBSERVE="${SIM_OBSERVE_SECS:-300}"
A_P2P="${SIM_A_P2P:-30391}"; B_P2P="${SIM_B_P2P:-30393}"
A_RPC="${SIM_A_RPC:-8591}"; B_RPC="${SIM_B_RPC:-8593}"
A_MET="${SIM_A_MET:-9091}"; B_MET="${SIM_B_MET:-9093}"
SIM="${TMPDIR:-/tmp}"; SIM="${SIM%/}/doli-bootstrap-island-${LABEL}-$(date +%s)"
PID_A=""; PID_B=""; SEED_STOPPED_BY_ME=0; RESULT=""

log() { echo "$(date -u +%H:%M:%S) $*"; }
rpc_num() { curl -s -m 3 -X POST -H 'Content-Type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$2\",\"params\":{}}" "http://127.0.0.1:$1" \
  | grep -o "\"$3\":[0-9]*" | cut -d: -f2; }
rpc_str() { curl -s -m 3 -X POST -H 'Content-Type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$2\",\"params\":{}}" "http://127.0.0.1:$1" \
  | grep -o "\"$3\":\"[^\"]*\"" | cut -d'"' -f4; }
seed_pid() { pgrep -f "$TESTNET_DIR/seed/data"; }
seed_up() { lsof -nP -iTCP:"$SEED_P2P" -sTCP:LISTEN >/dev/null 2>&1; }
mine() { [ -n "$1" ] && ps -p "$1" -o command= 2>/dev/null | grep -q "$SIM/"; }

cleanup() {
  for p in $PID_A $PID_B; do mine "$p" && kill -TERM "$p"; done
  for _ in $(seq 1 30); do
    alive=0; for p in $PID_A $PID_B; do mine "$p" && alive=1; done
    [ "$alive" = 0 ] && break; sleep 1
  done
  for p in $PID_A $PID_B; do mine "$p" && { log "cleanup: force-kill $p"; kill -KILL "$p"; }; done
  if [ "$SEED_STOPPED_BY_ME" = 1 ] && [ -z "$(seed_pid)" ]; then
    log "cleanup: starting the seed"; "$REPO/scripts/testnet.sh" start seed >/dev/null 2>&1
  fi
  [ -d "$SIM" ] && rm -rf "$SIM/A" "$SIM/B"
  log "cleanup: temporary nodes stopped; seed pid=$(seed_pid | tr '\n' ' ')logs in $SIM"
  [ -n "$RESULT" ] && echo "$RESULT"
}
trap cleanup EXIT
trap 'log "signal received"; exit 130' INT TERM HUP

# ── 0. guards (nothing is touched before these pass) ─────────────────────────
if [ -z "$BIN" ]; then sed -n '2,45p' "$0"; exit 2; fi
[ "${SIM_CONFIRM:-0}" = 1 ] || { log "REFUSED: this stops the local seed. Re-run with SIM_CONFIRM=1."; exit 2; }
[ -x "$BIN" ] || { log "ABORT: binary not executable: $BIN"; exit 3; }
if command -v codesign >/dev/null 2>&1; then
  codesign -v "$BIN" 2>/dev/null || { log "ABORT: codesign invalid: $BIN"; exit 3; }
fi
[ "$(rpc_str "$SEED_RPC" getChainInfo network)" = testnet ] \
  || { log "ABORT: seed RPC :$SEED_RPC is not a live local testnet node"; exit 3; }
[ -d "$TESTNET_DIR/seed/data" ] || { log "ABORT: no seed data dir under $TESTNET_DIR"; exit 3; }
for p in "$A_P2P" "$B_P2P" "$A_RPC" "$B_RPC" "$A_MET" "$B_MET"; do
  lsof -nP -iTCP:"$p" -sTCP:LISTEN >/dev/null 2>&1 && { log "ABORT: tcp $p busy"; exit 3; }
done
for p in $((A_P2P + 1)) $((B_P2P + 1)); do
  lsof -nP -iUDP:"$p" >/dev/null 2>&1 && { log "ABORT: udp $p busy"; exit 3; }
done
SEED_ID="$(rpc_str "$SEED_RPC" getNetworkInfo peerId)"
log "label=$LABEL binary: $("$BIN" --version 2>&1 | head -1)"
log "before: seed h=$(rpc_num "$SEED_RPC" getChainInfo bestHeight) ref h=$(rpc_num "$REF_RPC" getChainInfo bestHeight)"

# ── 1. stop the seed ─────────────────────────────────────────────────────────
SEED_STOPPED_BY_ME=1
"$REPO/scripts/testnet.sh" stop seed >/dev/null 2>&1
for _ in $(seq 1 30); do [ -z "$(seed_pid)" ] && break; sleep 1; done
[ -n "$(seed_pid)" ] && { log "ABORT: the seed did not stop"; exit 4; }
seed_up && { log "ABORT: seed P2P port still listening"; exit 4; }
log "seed stopped"

# ── 2. copy the seed chain data (consistent: the seed is stopped) ───────────
mkdir -p "$SIM/A" "$SIM/B"
for n in A B; do
  rsync -a --exclude checkpoints --exclude peers.cache --exclude producer_gset.bin \
    --exclude producer.lock --exclude signed_slots.db \
    "$TESTNET_DIR/seed/data/" "$SIM/$n/data/" || { log "ABORT: data copy $n"; exit 5; }
done
[ -e "$SIM/A/node_key" ] && { log "ABORT: node_key was copied"; exit 5; }
log "data copied ($(du -sh "$SIM/A/data" | cut -f1) each)"

# ── 3. start A, then B (each bootstraps to the seed + the other node) ───────
start_node() { # name p2p rpc metrics partner_p2p
  nohup "$BIN" --network testnet --data-dir "$SIM/$1/data" --log-level info run \
    --p2p-port "$2" --rpc-port "$3" --rpc-bind 127.0.0.1 --metrics-port "$4" \
    --bootstrap "/ip4/127.0.0.1/tcp/$SEED_P2P" --bootstrap "/ip4/127.0.0.1/tcp/$5" \
    --no-auto-update --yes --force-start > "$SIM/$1.log" 2>&1 &
  echo $!
}
PID_A=$(start_node A "$A_P2P" "$A_RPC" "$A_MET" "$B_P2P"); log "A started pid=$PID_A"
sleep 5
PID_B=$(start_node B "$B_P2P" "$B_RPC" "$B_MET" "$A_P2P"); log "B started pid=$PID_B"

# ── 4. the island must form: both at exactly 1 peer ─────────────────────────
pa=""; pb=""
for _ in $(seq 1 60); do
  pa=$(rpc_num "$A_RPC" getNetworkInfo peerCount); pb=$(rpc_num "$B_RPC" getNetworkInfo peerCount)
  [ -n "$pa" ] && [ -n "$pb" ] && [ "$pa" -ge 1 ] && [ "$pb" -ge 1 ] && break
  mine "$PID_A" || { log "ABORT: A died"; tail -5 "$SIM/A.log"; exit 6; }
  mine "$PID_B" || { log "ABORT: B died"; tail -5 "$SIM/B.log"; exit 6; }
  sleep 2
done
log "island: A peers=${pa:-noRPC} B peers=${pb:-noRPC}"
if [ "${pa:-0}" != 1 ] || [ "${pb:-0}" != 1 ]; then
  RESULT="VERDICT: INVALID — island not formed (A=${pa:-none} B=${pb:-none} peers); the experiment has an escape path or the nodes did not pair"
  exit 7
fi

sample() {
  echo "$(date -u +%H:%M:%S) $1 A p=$(rpc_num "$A_RPC" getNetworkInfo peerCount) h=$(rpc_num "$A_RPC" getChainInfo bestHeight) | B p=$(rpc_num "$B_RPC" getNetworkInfo peerCount) h=$(rpc_num "$B_RPC" getChainInfo bestHeight) | ref h=$(rpc_num "$REF_RPC" getChainInfo bestHeight) | seed=$(seed_up && echo up || echo down)"
}

# ── 5. island phase with the seed down ──────────────────────────────────────
t=0
while [ "$t" -le "$ISLAND_SECS" ]; do sample "island+${t}s"; [ "$t" -lt "$ISLAND_SECS" ] && sleep 15; t=$((t + 15)); done

# ── 6. the seed returns; observe ─────────────────────────────────────────────
"$REPO/scripts/testnet.sh" start seed >/dev/null 2>&1
TS=$(date +%s); log "seed start issued"
REC_A=""; REC_B=""
while [ $(( $(date +%s) - TS )) -le "$OBSERVE" ]; do
  el=$(( $(date +%s) - TS ))
  pa=$(rpc_num "$A_RPC" getNetworkInfo peerCount); ha=$(rpc_num "$A_RPC" getChainInfo bestHeight)
  pb=$(rpc_num "$B_RPC" getNetworkInfo peerCount); hb=$(rpc_num "$B_RPC" getChainInfo bestHeight)
  hr=$(rpc_num "$REF_RPC" getChainInfo bestHeight)
  echo "$(date -u +%H:%M:%S) seed+${el}s A p=$pa h=$ha | B p=$pb h=$hb | ref h=$hr | seed=$(seed_up && echo up || echo down)"
  if [ -z "$REC_A" ] && [ -n "$pa" ] && [ -n "$ha" ] && [ -n "$hr" ] && [ "$pa" -ge 2 ] && [ "$ha" -ge $((hr - 1)) ]; then REC_A=$el; fi
  if [ -z "$REC_B" ] && [ -n "$pb" ] && [ -n "$hb" ] && [ -n "$hr" ] && [ "$pb" -ge 2 ] && [ "$hb" -ge $((hr - 1)) ]; then REC_B=$el; fi
  if [ -n "$REC_A" ] && [ -n "$REC_B" ]; then break; fi
  mine "$PID_A" || log "WARNING: A process gone"
  mine "$PID_B" || log "WARNING: B process gone"
  sleep 15
done

# ── 7. evidence + verdict ────────────────────────────────────────────────────
for n in A B; do
  s=$(sed 's/\x1b\[[0-9;]*m//g' "$SIM/$n.log")
  echo "$n log: BOOTSTRAP_REDIAL=$(grep -c BOOTSTRAP_REDIAL <<<"$s") seed_connected=$(grep -c "Peer connected: ${SEED_ID:-none}" <<<"$s") panics=$(grep -c -i panicked <<<"$s")"
done
if [ -n "$REC_A" ] && [ -n "$REC_B" ]; then
  RESULT="VERDICT: RECOVERED — A at seed+${REC_A}s, B at seed+${REC_B}s"
  exit 0
fi
RESULT="VERDICT: STUCK — A=${REC_A:+seed+${REC_A}s}${REC_A:-stuck} B=${REC_B:+seed+${REC_B}s}${REC_B:-stuck} within ${OBSERVE}s of the seed returning"
exit 1
