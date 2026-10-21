#!/usr/bin/env bash
# gauntlet-gs026.sh — GS-026 "block-authentication" (INC-I-236).
# REQ-AUTH-024 — Decision: a FAIL means a keyless party can still author a block in
# a scheduled producer's slot on a live node above the signature activation height,
# so the INC-I-236 fix must not ship. A pre-fix control binary accepts the forged and
# unsigned blocks (RED); a HEAD binary rejects both and the chain converges (GREEN).
#
# Stands up an EPHEMERAL local devnet the runner builds itself (throwaway keys, a
# shared chainspec, free high ports). GS026_NODES producer nodes: the last runs the
# pre-fix control binary, the rest run HEAD (the genesis bootstrap gate needs >= 5
# peers, so the default is 6 nodes). The signature gate is armed at a finite height
# through the devnet-only env var. Standalone, no args.
#
# OUTPUT CONTRACT: one `PASS|FAIL gs026-<id> <measured k=v>` line per assertion,
# then the METRIC line (last line), then the exit code.
#   gs026-mixed-below-height-equal-tips  mixed control+HEAD cluster below the gate:
#                                        identical block hash at several heights.
#   gs026-crossing-height-no-fork        all-HEAD cluster crosses the gate height:
#                                        every node agrees on the tip, no fork.
#   gs026-forged-rejected                a block naming a scheduled producer built
#                                        WITHOUT its key is rejected by every HEAD
#                                        node; tips stay equal; production continues.
#   gs026-unsigned-rejected              a block with NO signature is rejected by
#                                        every HEAD node; tips stay equal; continues.
#   METRIC forged_accepted_head= unsigned_accepted_head= nodes= height=
#   exit: 0 all PASS · 1 any FAIL · 2 SKIP (`SKIP gs026: <reason>`)
# INPUT PARTITIONS: HEAD cluster (both injections rejected -> all PASS) · all-control
#   cluster via GS026_ALL_CONTROL=1 (at least one accepts -> forged/unsigned FAIL,
#   the RED proof) · precondition missing (SKIP, exit 2).
# Env: GS026_HEAD_BIN GS026_CONTROL_BIN GS026_CLI GS026_FORGE GS026_NODES
#   GS026_ALL_CONTROL GS026_AH GS026_MIX_FLOOR GS026_P2P_BASE GS026_RPC_BASE
#   GS026_MET_BASE GS026_BOOT_MAX GS026_PHASE_MAX GS026_KEEP (1 = keep scratch).
# SAFETY: starts only processes it launched from its own scratch dir, confirms each
#   process cmdline contains that dir before signalling it, signals only its own PID,
#   never touches the local mainnet node or the test fleet, and removes only its own
#   mktemp dir. No pkill/killall/ssh/systemctl/launchctl, no foreign kill, no symlink.

set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GS026_HEAD_BIN="${GS026_HEAD_BIN:-$ROOT/target/release/doli-node}"
GS026_CONTROL_BIN="${GS026_CONTROL_BIN:-$ROOT/target/gs026/doli-node-b4cff010}"
GS026_CLI="${GS026_CLI:-$ROOT/target/release/doli}"
GS026_FORGE="${GS026_FORGE:-$ROOT/target/release/examples/gs026_forge_block}"
GS026_NODES="${GS026_NODES:-6}"
GS026_ALL_CONTROL="${GS026_ALL_CONTROL:-0}"
GS026_AH="${GS026_AH:-80}"
GS026_MIX_FLOOR="${GS026_MIX_FLOOR:-62}"
GS026_P2P_BASE="${GS026_P2P_BASE:-50360}"
GS026_RPC_BASE="${GS026_RPC_BASE:-28560}"
GS026_MET_BASE="${GS026_MET_BASE:-29060}"
GS026_BOOT_MAX="${GS026_BOOT_MAX:-150}"
GS026_PHASE_MAX="${GS026_PHASE_MAX:-240}"
GS026_KEEP="${GS026_KEEP:-0}"

SLOT_DUR=1
REWARD=100000000
BOND=100000000
EPOCH_SLOTS=30
GEN_DELAY=15
TMPROOT="${TMPDIR:-/tmp}"
TMPROOT="${TMPROOT%/}"
S=""
NODE_PIDS=""
GEN_TS=0
ALL=0
FORGED_ACCEPTED=0
UNSIGNED_ACCEPTED=0
LAST_HEIGHT=0
N="$GS026_NODES"
CTRL_IDX=$((N - 1)) # the node dropped before the all-HEAD crossing
declare -a P2P RPCP MET PK BINS NODE_PID STOPPED

skip() {
  echo "SKIP gs026: $*"
  exit 2
}

[ "$N" -ge 6 ] 2>/dev/null || skip "GS026_NODES=$N must be >= 6 (genesis bootstrap gate needs >= 5 peers)"

for ((i = 0; i < N; i++)); do
  P2P[$i]=$((GS026_P2P_BASE + i))
  RPCP[$i]=$((GS026_RPC_BASE + i))
  MET[$i]=$((GS026_MET_BASE + i))
  STOPPED[$i]=0
  if [ "$GS026_ALL_CONTROL" = 1 ] || [ "$i" -eq "$CTRL_IDX" ]; then
    BINS[$i]="$GS026_CONTROL_BIN"
  else
    BINS[$i]="$GS026_HEAD_BIN"
  fi
done

rpc() {
  curl -s -m 8 "http://127.0.0.1:$1" -H 'Content-Type: application/json' -d "$2" 2>/dev/null
}
ok_body() { case "$1" in *'"result"'*) return 0 ;; esac; return 1; }
port_busy() { lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1; }
udp_busy() { lsof -nP -iUDP:"$1" >/dev/null 2>&1; }
alive() {
  local st
  st="$(ps -p "$1" -o stat= 2>/dev/null | tr -d ' ')"
  [ -n "$st" ] && [ "${st#Z}" = "$st" ]
}
ours() { [ -n "$S" ] && ps -p "$1" -o command= 2>/dev/null | grep -qF -- "$S"; }
# Prints cargo|rustc when a build of THIS checkout runs (names ROOT/, not a worktree under it).
build_running() {
  ps -axww -o args= 2>/dev/null | awk -v r="$ROOT/" -v w="$ROOT/.claude/worktrees/" '
    { b = $1; sub(/.*\//, "", b) }
    (b == "cargo" || b == "rustc") && index($0, r) && !index($0, w) { print b; exit }'
}

# result.<path> from a JSON-RPC body on stdin; nested keys dotted, list index numeric.
jfield() {
  python3 -I -c '
import json, sys
try:
    cur = json.load(sys.stdin)
    cur = cur.get("result", cur) if isinstance(cur, dict) else cur
    for k in sys.argv[1].split("."):
        cur = cur[int(k)] if isinstance(cur, list) else cur[k]
    print(cur)
except Exception:
    pass' "$1" 2>/dev/null
}

chaininfo() { rpc "$1" '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}'; }
best_height() { chaininfo "$1" | jfield bestHeight; }
block_hash() {
  rpc "$1" "{\"jsonrpc\":\"2.0\",\"method\":\"getBlockByHeight\",\"params\":{\"height\":$2},\"id\":1}" |
    jfield hash
}
block_raw() {
  rpc "$1" "{\"jsonrpc\":\"2.0\",\"method\":\"getBlockRaw\",\"params\":{\"height\":$2},\"id\":1}" |
    jfield block
}

stop_one() {
  local PID="$1" n=0
  [ -n "$PID" ] || return 0
  if alive "$PID" && ours "$PID"; then
    kill -TERM "$PID" 2>/dev/null
    while alive "$PID" && [ "$n" -lt 150 ]; do
      sleep 0.2
      n=$((n + 1))
    done
    alive "$PID" && ours "$PID" && kill -KILL "$PID" 2>/dev/null
  fi
  wait "$PID" 2>/dev/null
}

# shellcheck disable=SC2329  # invoked via the EXIT trap
cleanup() {
  for PID in $NODE_PIDS; do
    stop_one "$PID"
  done
  [ -n "$S" ] || return 0
  if [ "$GS026_KEEP" = 1 ]; then
    echo "kept $S"
    return 0
  fi
  case "$S" in */gs026.*) rm -rf "$S" ;; esac
}
trap cleanup EXIT
trap 'exit 1' INT TERM

res() {
  if [ "$2" = 1 ]; then
    echo "PASS $1 $3"
  else
    echo "FAIL $1 $3"
    ALL=1
  fi
}

wallet_pubkey() {
  python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["addresses"][0]["public_key"])' "$1" 2>/dev/null
}

start_node() {
  local idx="$1" dd
  # data_dir nested one level under n$idx: the node stores node_key in the PARENT
  # of the data dir (startup.rs), so a per-node parent gives each node a unique
  # libp2p peer id. Sibling data dirs would share one node_key and self-reject.
  dd="$S/n$idx/data"
  mkdir -p "$dd"
  : >"$dd/.env"
  local args=(--network devnet --data-dir "$dd" --log-level info run
    --producer --producer-key "$S/w$idx.json" --chainspec "$S/chainspec.json"
    --p2p-port "${P2P[$idx]}" --rpc-port "${RPCP[$idx]}" --metrics-port "${MET[$idx]}"
    --no-auto-update --force-start --yes)
  # Bootstrap each node to ALL earlier nodes so the cluster forms a full mesh:
  # every node ends with N-1 peers, which the genesis bootstrap gate (>= 5 peers)
  # requires. A star to node0 alone leaves the children at one peer and blocked.
  local j
  for ((j = 0; j < idx; j++)); do
    args+=(--bootstrap "/ip4/127.0.0.1/tcp/${P2P[$j]}")
  done
  "${BINS[$idx]}" "${args[@]}" >"$S/n$idx/node.log" 2>&1 &
  NODE_PID[$idx]=$!
  NODE_PIDS="$NODE_PIDS ${NODE_PID[$idx]}"
}

running_idx() {
  local i out=""
  for ((i = 0; i < N; i++)); do
    [ "${STOPPED[$i]}" = 1 ] && continue
    out="$out $i"
  done
  echo "$out"
}

min_height() {
  local lo="" i h
  for i in $(running_idx); do
    h="$(best_height "${RPCP[$i]}")"
    case "$h" in '' | *[!0-9]*) return 1 ;; esac
    { [ -z "$lo" ] || [ "$h" -lt "$lo" ]; } && lo="$h"
  done
  [ -n "$lo" ] && echo "$lo"
}

# All running nodes report the identical block hash at each listed height.
same_at_heights() {
  local hh ref cur i
  for hh in "$@"; do
    ref=""
    for i in $(running_idx); do
      cur="$(block_hash "${RPCP[$i]}" "$hh")"
      [ -n "$cur" ] || return 1
      if [ -z "$ref" ]; then
        ref="$cur"
      elif [ "$cur" != "$ref" ]; then
        echo "divergent at h=$hh: $ref vs $cur" >&2
        return 1
      fi
    done
  done
  return 0
}

# --- preconditions --------------------------------------------------------------
command -v curl >/dev/null 2>&1 || skip "curl unavailable"
command -v lsof >/dev/null 2>&1 || skip "lsof unavailable"
command -v python3 >/dev/null 2>&1 || skip "python3 unavailable"
[ -n "$(build_running)" ] && skip "a build is running (cargo/rustc) in this checkout $ROOT"
[ -x "$GS026_HEAD_BIN" ] || skip "HEAD binary $GS026_HEAD_BIN missing or not executable"
[ -x "$GS026_CLI" ] || skip "CLI $GS026_CLI missing or not executable"
[ -x "$GS026_FORGE" ] || skip "forge helper $GS026_FORGE missing (cargo build --release -p doli-node --example gs026_forge_block)"
[ -x "$GS026_CONTROL_BIN" ] || skip "control binary $GS026_CONTROL_BIN missing (build v6.44.2 b4cff010)"
[ "$GS026_AH" -gt 60 ] || skip "GS026_AH=$GS026_AH must exceed bootstrap_blocks=60"
[ "$GS026_MIX_FLOOR" -ge 61 ] && [ "$GS026_MIX_FLOOR" -lt "$GS026_AH" ] ||
  skip "GS026_MIX_FLOOR=$GS026_MIX_FLOOR must be in [61, GS026_AH)"
for ((i = 0; i < N; i++)); do
  for p in "${P2P[$i]}" "${RPCP[$i]}" "${MET[$i]}"; do
    port_busy "$p" && skip "TCP port $p is busy"
  done
  udp_busy $((${P2P[$i]} + 1)) && skip "UDP discovery port $((${P2P[$i]} + 1)) is busy"
done

# --- scratch cluster ------------------------------------------------------------
S="$(mktemp -d "$TMPROOT/gs026.XXXXXX")" || skip "mktemp failed"
PKARGS=()
for ((i = 0; i < N; i++)); do
  "$GS026_CLI" -w "$S/w$i.json" new -n "gs026n$i" >/dev/null 2>&1 ||
    skip "wallet generation failed for node $i"
  PK[$i]="$(wallet_pubkey "$S/w$i.json")"
  [ -n "${PK[$i]}" ] || skip "could not read pubkey for node $i"
  PKARGS+=("${PK[$i]}")
done

now="$(date +%s)"
GEN_TS=$(((now / SLOT_DUR + GEN_DELAY) * SLOT_DUR))
python3 -I - "$S/chainspec.json" "$GEN_TS" "$SLOT_DUR" "$REWARD" "$BOND" "$EPOCH_SLOTS" \
  "${PKARGS[@]}" <<'PY' || skip "chainspec write failed"
import json, sys
path, gts, sd, rw, bond, eps = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5]), int(sys.argv[6])
pks = sys.argv[7:]
spec = {
    "name": "GS-026 ephemeral devnet",
    "id": "ephemeral-devnet",
    "network": "Devnet",
    "genesis": {"timestamp": gts, "message": "GS-026 INC-I-236 ephemeral", "initial_reward": rw},
    "consensus": {"slot_duration": sd, "slots_per_epoch": eps, "bond_amount": bond},
    "genesis_producers": [{"name": "n%d" % i, "public_key": pk, "bond_count": 1} for i, pk in enumerate(pks)],
}
open(path, "w").write(json.dumps(spec, indent=2))
PY

echo "gs026: head=$("$GS026_HEAD_BIN" --version 2>/dev/null) control=$("$GS026_CONTROL_BIN" --version 2>/dev/null) nodes=$N all_control=$GS026_ALL_CONTROL ah=$GS026_AH scratch=$S"
# Start node0 first and wait for its RPC before bootstrapping the rest, so the
# others never dial a listener that is not up yet (devnet start.rs order).
start_node 0
tboot="$(date +%s)"
until ok_body "$(chaininfo "${RPCP[0]}")"; do
  alive "${NODE_PID[0]}" || skip "node 0 exited before RPC came up (see $S/n0/node.log)"
  [ $(($(date +%s) - tboot)) -ge 40 ] && break
  sleep 1
done
for ((i = 1; i < N; i++)); do
  start_node "$i"
  sleep 1
done

# --- wait for the cluster to bootstrap past genesis -----------------------------
t0="$(date +%s)"
booted=0
while [ $(($(date +%s) - t0)) -lt "$GS026_BOOT_MAX" ]; do
  mh="$(min_height)" || mh=""
  if [ -n "$mh" ] && [ "$mh" -ge 2 ]; then
    booted=1
    break
  fi
  for ((i = 0; i < N; i++)); do
    alive "${NODE_PID[$i]}" || skip "node $i exited during bootstrap (see $S/n$i/node.log)"
  done
  sleep 3
done
[ "$booted" = 1 ] || skip "cluster did not reach height 2 within ${GS026_BOOT_MAX}s (genesis quorum needs >= 5 peers)"

# --- gs026-mixed-below-height-equal-tips ----------------------------------------
t0="$(date +%s)"
mix_ok=0
mix_h=0
while [ $(($(date +%s) - t0)) -lt "$GS026_PHASE_MAX" ]; do
  mh="$(min_height)" || mh=""
  if [ -n "$mh" ] && [ "$mh" -ge "$GS026_MIX_FLOOR" ] && [ "$mh" -lt "$GS026_AH" ]; then
    same_at_heights $((mh - 2)) $((mh - 1)) "$mh" && mix_ok=1 && mix_h="$mh"
    break
  fi
  [ -n "$mh" ] && [ "$mh" -ge "$GS026_AH" ] && break
  sleep 2
done
res gs026-mixed-below-height-equal-tips "$mix_ok" "nodes=$N heights=$((mix_h - 2)),$((mix_h - 1)),$mix_h ah=$GS026_AH"

# --- drop the control node before the cluster crosses the gate ------------------
stop_one "${NODE_PID[$CTRL_IDX]}"
STOPPED[$CTRL_IDX]=1

# --- gs026-crossing-height-no-fork (all-HEAD) -----------------------------------
t0="$(date +%s)"
cross_ok=0
while [ $(($(date +%s) - t0)) -lt "$GS026_PHASE_MAX" ]; do
  mh="$(min_height)" || mh=""
  if [ -n "$mh" ] && [ "$mh" -ge $((GS026_AH + 2)) ]; then
    same_at_heights "$GS026_AH" $((GS026_AH + 1)) $((GS026_AH + 2)) && cross_ok=1
    break
  fi
  alive "${NODE_PID[0]}" || break
  sleep 2
done
res gs026-crossing-height-no-fork "$cross_ok" "nodes=$((N - 1)) heights=$GS026_AH,$((GS026_AH + 1)),$((GS026_AH + 2))"

# inject KIND — forge one block in the next slot owned by a running HEAD node.
# Sets REJECTED=1 when no running node adopts the forged block at its height, or
# ACCEPTED=1 when one does; TIPS_EQ=1 when the running nodes agree afterwards.
inject() {
  local kind="$1" tip owner oidx target b64 fhash h i accepted tries sched cur_slot rpc0
  REJECTED=0
  ACCEPTED=0
  TIPS_EQ=0
  rpc0="${RPCP[0]}"
  tip="$(best_height "$rpc0")"
  case "$tip" in '' | *[!0-9]*) echo "no tip for injection" >&2; return 1 ;; esac
  sched="$(rpc "$rpc0" '{"jsonrpc":"2.0","method":"getSlotSchedule","params":{},"id":1}')"
  cur_slot="$(printf '%s' "$sched" | jfield currentSlot)"
  case "$cur_slot" in '' | *[!0-9]*) echo "no schedule for injection" >&2; return 1 ;; esac
  target=$((cur_slot + 1))
  owner="$(printf '%s' "$sched" | python3 -I -c '
import json,sys
d=json.load(sys.stdin).get("result",{})
want=int(sys.argv[1])
for e in d.get("slots",[]):
    if e.get("slot")==want: print(e.get("producer","")); break' "$target" 2>/dev/null)"
  oidx=-1
  for i in $(running_idx); do
    [ "${PK[$i]}" = "$owner" ] && oidx="$i"
  done
  [ "$oidx" -ge 0 ] 2>/dev/null || { echo "slot $target owner not a running node" >&2; return 1; }
  b64="$(block_raw "$rpc0" "$tip")"
  [ -n "$b64" ] || { echo "no tip block bytes" >&2; return 1; }
  h=$((tip + 1))
  rpc "${RPCP[$oidx]}" '{"jsonrpc":"2.0","method":"pauseProduction","params":{},"id":1}' >/dev/null
  local dial=""
  for i in $(running_idx); do
    dial="$dial,/ip4/127.0.0.1/tcp/${P2P[$i]}"
  done
  dial="${dial#,}"
  fhash="$("$GS026_FORGE" --kind "$kind" --tip-b64 "$b64" --height "$h" --slot "$target" \
    --producer "$owner" --genesis-time "$GEN_TS" --slot-duration "$SLOT_DUR" --dial "$dial" 2>>"$S/forge.log" |
    sed -n 's/^INJECTED .*hash=\([0-9a-f]*\).*/\1/p' | head -1)"
  accepted=0
  tries=0
  while [ "$tries" -lt 8 ]; do
    for i in $(running_idx); do
      [ -n "$fhash" ] && [ "$(block_hash "${RPCP[$i]}" "$h")" = "$fhash" ] && accepted=1
    done
    [ "$accepted" = 1 ] && break
    sleep 1
    tries=$((tries + 1))
  done
  rpc "${RPCP[$oidx]}" '{"jsonrpc":"2.0","method":"resumeProduction","params":{},"id":1}' >/dev/null
  [ "$accepted" = 1 ] && ACCEPTED=1 || REJECTED=1
  local ch
  ch="$(best_height "$rpc0")"
  case "$ch" in '' | *[!0-9]*) ch=0 ;; esac
  [ "$ch" -gt 0 ] && same_at_heights $((ch - 1)) && TIPS_EQ=1
  echo "inject $kind slot=$target owner=${owner:0:8} height=$h forged_hash=${fhash:-none} accepted=$accepted" >&2
  LAST_HEIGHT="$ch"
  return 0
}

head_running() { alive "${NODE_PID[0]}"; }

# --- gs026-forged-rejected ------------------------------------------------------
f_ok=0
if head_running && inject forged; then
  [ "$ACCEPTED" = 1 ] && FORGED_ACCEPTED=1
  [ "$REJECTED" = 1 ] && [ "$TIPS_EQ" = 1 ] && f_ok=1
fi
res gs026-forged-rejected "$f_ok" "accepted_by_head=$FORGED_ACCEPTED tips_equal=${TIPS_EQ:-0}"

# let honest production move the tip on before the second injection
sleep 4

# --- gs026-unsigned-rejected ----------------------------------------------------
u_ok=0
if head_running && inject unsigned; then
  [ "$ACCEPTED" = 1 ] && UNSIGNED_ACCEPTED=1
  [ "$REJECTED" = 1 ] && [ "$TIPS_EQ" = 1 ] && u_ok=1
fi
res gs026-unsigned-rejected "$u_ok" "accepted_by_head=$UNSIGNED_ACCEPTED tips_equal=${TIPS_EQ:-0}"

echo "METRIC forged_accepted_head=$FORGED_ACCEPTED unsigned_accepted_head=$UNSIGNED_ACCEPTED nodes=$N height=$LAST_HEIGHT"
exit "$ALL"
