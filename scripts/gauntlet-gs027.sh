#!/usr/bin/env bash
# gauntlet-gs027.sh — GS-027 "working-equivocation-slashing" (INC-I-255). A FAIL means a
# producer that signs two blocks for one slot above the slashing gate is not slashed exactly
# once on every node, so the INC-I-255 fix must not ship (a pre-fix binary is the RED proof).
#
# EPHEMERAL devnet (throwaway keys, one chainspec, free high ports): GS027_NODES nodes, the
# first GS027_PRODUCERS genesis producers. Slashing and block-signature gates = GS027_AH via
# the devnet env vars (pending-root gate 0); GS027_EPOCH_BLOCKS-block reward epochs put a
# boundary after the slash. Supply = getChainStats totalSupply (the UTXO-set sum), read per
# height as the most frequent sample.
# Devnet workaround: genesis-phase blocks are validated against each node's producer G-Set,
# which producer gossip leaves partial (measured: 1/6 nodes complete after 400 s, chains
# split from height 1). The runner starts every node once, stops it, merges the persisted
# own-announcement files (producer_gset.bin) into every data dir and restarts the cluster
# before genesis (GS027_GENESIS_DELAY s after start); it SKIPs unless all nodes hold it.
#
# OUTPUT CONTRACT: one `PASS|FAIL <id> <measured k=v>` line per assertion (each must hold on
# EVERY node), then the METRIC line (last line), then the exit code.
#   gs027-one-slash-mined              one slash_producer tx (txs via getTransaction), one height+hash
#   gs027-producer-slashed-everywhere  getProducer status of the target is slashed
#   gs027-bond-burned-once             supply delta reward-bond at the slash block, -bond overall
#   gs027-tips-equal                   one block hash at the common tip height
#   gs027-production-continues         common tip >= slash height + GS027_ADVANCE
#   gs027-mempool-slash-bounded        <= 1 slash resident in every mempool sample
#   gs027-one-shared-slash-id          <= 1 admitted slash id per node log; one id, the mined one
#   METRIC SLASHED_NODES=<k>/<n> detected=<nodes that logged the equivocation>/<n> target= inj_height=
#          slash_height= bond= reward=
#   exit: 0 all PASS · 1 any FAIL · 2 SKIP (`SKIP gs027: <reason>`)
# INPUT PARTITIONS: HEAD (all PASS) · pre-fix binary via GS027_NODE_BIN (no slash, FAIL) ·
#   precondition missing or cluster not past GS027_AH (SKIP).
# Env: GS027_NODE_BIN GS027_CLI GS027_FORGE GS027_NODES GS027_PRODUCERS GS027_AH
#   GS027_EPOCH_BLOCKS GS027_GENESIS_BLOCKS GS027_BOOTSTRAP_BLOCKS GS027_SLOT_DUR
#   GS027_GENESIS_DELAY GS027_OBS_BLOCKS GS027_ADVANCE GS027_P2P_BASE GS027_RPC_BASE
#   GS027_MET_BASE GS027_BOOT_MAX GS027_PHASE_MAX GS027_KEEP (1 = keep scratch)
#   GS027_LIB_ONLY (1 = define the helpers and return).
# SAFETY: signals only PIDs it launched whose cmdline names its own mktemp dir and removes
#   only that dir. No pkill/killall/ssh/systemctl/launchctl, no foreign kill, no symlink.

set -u
set -o pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/gauntlet-devnet-lib.sh
. "$ROOT/scripts/gauntlet-devnet-lib.sh"
ALL=0

res() {
  local id="$1" ok="$2"
  shift 2
  [ "$ok" = 1 ] && echo "PASS $id $*" && return 0
  echo "FAIL $id $*"
  ALL=1
}

verdict() { [ "$ALL" = 0 ]; }

# Slash txs in a block whose transactions are getTransaction objects; rc != 0 when
# the block is an error, malformed, or holds an entry with no txType (a bare hash).
count_slash_txs() {
  python3 -I -c '
import json, sys
try:
    d = json.load(open(sys.argv[1]))
except Exception:
    sys.exit(2)
if isinstance(d, dict) and "error" in d:
    sys.exit(3)
r = d.get("result", d) if isinstance(d, dict) else None
txs = r.get("transactions") if isinstance(r, dict) else None
if not isinstance(txs, list):
    sys.exit(4)
n = 0
for t in txs:
    if not isinstance(t, dict) or not isinstance(t.get("txType"), str):
        sys.exit(5)
    n += t["txType"] == "slash_producer"
print(n)' "$1"
}

# Status of producer $2 in a getProducer object or getProducers list; empty + rc 1 if absent.
producer_status() {
  python3 -I -c '
import json, sys
try:
    d = json.load(open(sys.argv[1]))
except Exception:
    sys.exit(2)
if isinstance(d, dict) and "error" in d:
    sys.exit(3)
r = d.get("result", d) if isinstance(d, dict) else d
for p in (r if isinstance(r, list) else [r]):
    if isinstance(p, dict) and p.get("publicKey") == sys.argv[2] and p.get("status"):
        print(p["status"])
        sys.exit(0)
sys.exit(1)' "$1" "$2"
}

# supply_delta_ok REWARD BOND BEFORE AFTER: rc 0 iff AFTER - BEFORE == REWARD - BOND.
supply_delta_ok() {
  local v
  [ "$#" = 4 ] || return 1
  for v in "$@"; do case "$v" in '' | *[!0-9]*) return 1 ;; esac; done
  [ $((10#$4 - 10#$3)) -eq $((10#$1 - 10#$2)) ] || return 1
  return 0
}

# Distinct slash ids the equivocation handler admitted (`slash <id> for <pk> admitted`).
slash_ids_from_log() {
  [ -f "$1" ] || return 1
  grep -oE 'slash [0-9a-f]{64} for [0-9a-f]+ admitted' "$1" | awk '{ print $2 }' | sort -u
  return 0
}

if [ "${GS027_LIB_ONLY:-0}" = 1 ]; then
  # shellcheck disable=SC2317
  return 0 2>/dev/null || exit 0
fi

GS027_NODE_BIN="${GS027_NODE_BIN:-$ROOT/target/release/doli-node}"
GS027_CLI="${GS027_CLI:-$ROOT/target/release/doli}"
GS027_FORGE="${GS027_FORGE:-$ROOT/target/release/examples/gs026_forge_block}"
GS027_NODES="${GS027_NODES:-6}"
GS027_PRODUCERS="${GS027_PRODUCERS:-4}"
GS027_AH="${GS027_AH:-20}"
GS027_EPOCH_BLOCKS="${GS027_EPOCH_BLOCKS:-4}"
GS027_GENESIS_BLOCKS="${GS027_GENESIS_BLOCKS:-10}"
GS027_BOOTSTRAP_BLOCKS="${GS027_BOOTSTRAP_BLOCKS:-12}"
GS027_SLOT_DUR="${GS027_SLOT_DUR:-10}"
GS027_GENESIS_DELAY="${GS027_GENESIS_DELAY:-90}"
GS027_OBS_BLOCKS="${GS027_OBS_BLOCKS:-14}"
GS027_ADVANCE="${GS027_ADVANCE:-6}"
GS027_P2P_BASE="${GS027_P2P_BASE:-50380}"
GS027_RPC_BASE="${GS027_RPC_BASE:-28580}"
GS027_MET_BASE="${GS027_MET_BASE:-29080}"
GS027_BOOT_MAX="${GS027_BOOT_MAX:-180}"
GS027_PHASE_MAX="${GS027_PHASE_MAX:-600}"
GS027_KEEP="${GS027_KEEP:-0}"

REWARD=200000000
BOND_UNIT=100000000
TMPROOT="${TMPDIR:-/tmp}"
TMPROOT="${TMPROOT%/}"
N="$GS027_NODES"
P="$GS027_PRODUCERS"
S=""
DN_TAG=gs027
DN_CLI="$GS027_CLI"
DN_FORGE="$GS027_FORGE"
DN_SLOT="$GS027_SLOT_DUR"
DN_REWARD="$REWARD"
DN_BOND_UNIT="$BOND_UNIT"
DN_PHASE_MAX="$GS027_PHASE_MAX"
DN_KEEP="$GS027_KEEP"
DN_SIB_PK=""
DN_SIB_MIN="$GS027_AH"
# shellcheck disable=SC2191
DN_ENV=(DOLI_INC_I_236_BLOCK_SIGNATURE_ACTIVATION_HEIGHT="$GS027_AH"
  DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT="$GS027_AH"
  DOLI_INC_I_181_PENDING_ROOT_ACTIVATION_HEIGHT=0
  DOLI_GENESIS_BLOCKS="$GS027_GENESIS_BLOCKS"
  DOLI_BOOTSTRAP_BLOCKS="$GS027_BOOTSTRAP_BLOCKS"
  DOLI_BLOCKS_PER_REWARD_EPOCH="$GS027_EPOCH_BLOCKS")

# --- preconditions --------------------------------------------------------------
[ "$N" -ge 6 ] 2>/dev/null || skip "GS027_NODES=$N must be >= 6 (genesis bootstrap gate needs >= 5 peers)"
{ [ "$P" -ge 4 ] 2>/dev/null && [ "$P" -le "$N" ]; } ||
  skip "GS027_PRODUCERS=$P must be in [4, GS027_NODES] (3 must remain after the slash)"
{ [ "$GS027_AH" -ge "$GS027_EPOCH_BLOCKS" ] && [ "$GS027_AH" -gt "$GS027_BOOTSTRAP_BLOCKS" ]; } 2>/dev/null ||
  skip "GS027_AH=$GS027_AH must be past epoch 0 (GS027_EPOCH_BLOCKS) and GS027_BOOTSTRAP_BLOCKS"
for c in curl lsof python3; do command -v "$c" >/dev/null 2>&1 || skip "$c unavailable"; done
[ -n "$(build_running)" ] && skip "a build is running (cargo/rustc) in this checkout $ROOT"
[ -x "$GS027_NODE_BIN" ] || skip "node binary $GS027_NODE_BIN missing or not executable"
[ -x "$GS027_CLI" ] || skip "CLI $GS027_CLI missing or not executable"
[ -x "$GS027_FORGE" ] || skip "forge helper $GS027_FORGE missing (cargo build --release -p doli-node --example gs026_forge_block)"
for ((i = 0; i < N; i++)); do DN_BIN[i]="$GS027_NODE_BIN"; done
check_ports "$GS027_P2P_BASE" "$GS027_RPC_BASE" "$GS027_MET_BASE"

# --- scratch cluster ------------------------------------------------------------
trap cleanup EXIT
trap 'exit 1' INT TERM
S="$(mktemp -d "$TMPROOT/gs027.XXXXXX")" || skip "mktemp failed"
gen_wallets
T_START="$(date +%s)"
DN_GEN_TS=$((((T_START + GS027_GENESIS_DELAY) / DN_SLOT + 1) * DN_SLOT))
write_chainspec "GS-027 ephemeral devnet" "GS-027 INC-I-255 ephemeral" || skip "chainspec write failed"

echo "gs027: node=$("$GS027_NODE_BIN" --version 2>/dev/null) nodes=$N producers=$P ah=$GS027_AH epoch_blocks=$GS027_EPOCH_BLOCKS genesis_in=$((DN_GEN_TS - T_START))s scratch=$S"
start_cluster $((T_START + GS027_GENESIS_DELAY))
seed_gset || skip "a producer did not persist its announcement within 30 s"
start_cluster $((T_START + GS027_GENESIS_DELAY))
while [ "$(date +%s)" -lt $((DN_GEN_TS - 3)) ]; do all_alive || skip "a node exited before genesis"; sleep 2; done
gready="$(gset_ready)"
echo "gs027: gset_ready=$gready/$N at genesis ($((DN_GEN_TS - T_START))s after start)"
[ "$gready" = "$N" ] || skip "producer G-Set incomplete on some node before genesis ($gready/$N); raise GS027_GENESIS_DELAY"

wait_height 2 "$GS027_BOOT_MAX" || skip "cluster did not reach height 2 within ${GS027_BOOT_MAX}s of genesis or a node exited"
wait_height "$GS027_AH" "$GS027_PHASE_MAX" ||
  skip "cluster did not reach GS027_AH=$GS027_AH within ${GS027_PHASE_MAX}s or a node exited (heights: $(heights | tr '\n' ' '))"
mh="$(min_height)" || mh=1
# shellcheck disable=SC2046
echo "gs027: pre-injection height=$((mh - 1)) tips_equal=$(all_equal $(hashes_at $((mh - 1))) && echo 1 || echo 0)"

DN_TARGET_IDX=-1 DN_TARGET_PK="" DN_INJ_H=0 DN_INJ_SLOT=0 DN_OWN_HASH="" DN_SIB_HASH="" DN_BOND=""
inject_sibling || skip "no producer block above GS027_AH held by every node could be paired with a sibling"
echo "gs027: injected target=${DN_TARGET_PK:0:8} node=$DN_TARGET_IDX height=$DN_INJ_H slot=$DN_INJ_SLOT own=${DN_OWN_HASH:0:12} sibling=${DN_SIB_HASH:0:12} bond=$DN_BOND"
wait_height $((DN_INJ_H + GS027_OBS_BLOCKS)) "$GS027_PHASE_MAX" ||
  echo "gs027: observation window ended early (heights: $(heights | tr '\n' ' '))"
sample

# --- evidence per node ------------------------------------------------------------
declare -a NSL BAD SH SX ST BURN IDS DET
for ((i = 0; i < N; i++)); do
  tip="$(best_height "${DN_RPC[$i]}")"
  is_num "$tip" || tip=0
  devnet_py scan "$S/n$i/blk" "${DN_RPC[$i]}" 1 "$tip" 2>/dev/null | jlines slashes >"$S/n$i/slashes.txt"
  NSL[i]=0 BAD[i]=0 BURN[i]=0
  for f in "$S/n$i/blk/"*.json; do
    if c="$(count_slash_txs "$f")"; then NSL[i]=$((NSL[i] + c)); else BAD[i]=1; fi
  done
  [ "$tip" -gt 0 ] && [ "$(find "$S/n$i/blk" -name '*.json' | wc -l | tr -d ' ')" -eq "$tip" ] || BAD[i]=1
  read -r "SH[$i]" "SX[$i]" _ <"$S/n$i/slashes.txt" || SH[i]="" SX[i]=""
  producer_json "${DN_RPC[$i]}" "$DN_TARGET_PK" >"$S/n$i/producer.json"
  ST[i]="$(producer_status "$S/n$i/producer.json" "$DN_TARGET_PK")" || ST[i]=none
  sh="${SH[$i]}"
  if [ -n "$sh" ] && [ -n "$DN_BOND" ]; then
    hend="$(max_of "$i" S)"
    s0="$(supply_at "$i" $((sh - 1)))" s1="$(supply_at "$i" "$sh")" se="$(supply_at "$i" "$hend")"
    supply_delta_ok "$REWARD" "$DN_BOND" "$s0" "$s1" &&
      supply_delta_ok $(((hend - sh + 1) * REWARD)) "$DN_BOND" "$s0" "$se" && BURN[i]=1
    echo "gs027: n$i supply h$((sh - 1))=${s0:-?} h$sh=${s1:-?} h$hend=${se:-?}"
  fi
  IDS[i]="$(slash_ids_from_log "$S/n$i/node.log" | tr '\n' ' ')"
  DET[i]="$(grep -c 'EQUIVOCATION DETECTED' "$S/n$i/node.log" 2>/dev/null)"
done

# --- assertions ---------------------------------------------------------------------
mined=0 slashed=0 burned=0 bounded=0 idsmall=0 slashed_nodes=0 maxres=0 creators="" detected=0
for ((i = 0; i < N; i++)); do
  one=0
  [ "${NSL[$i]}" = 1 ] && [ "${BAD[$i]}" = 0 ] && one=1 && mined=$((mined + 1))
  [ "${ST[$i]}" = slashed ] && slashed=$((slashed + 1))
  [ "${BURN[$i]}" = 1 ] && burned=$((burned + 1))
  [ "$one" = 1 ] && [ "${ST[$i]}" = slashed ] && [ "${BURN[$i]}" = 1 ] && slashed_nodes=$((slashed_nodes + 1))
  r="$(max_of "$i" M)" k="$(printf '%s' "${IDS[$i]}" | wc -w | tr -d ' ')"
  [ "$r" -gt "$maxres" ] && maxres="$r"
  [ "$r" -le 1 ] && bounded=$((bounded + 1))
  [ "$k" -le 1 ] && idsmall=$((idsmall + 1))
  [ "$k" -ge 1 ] && creators="${creators}n$i,"
  [ "${DET[$i]:-0}" -ge 1 ] 2>/dev/null && detected=$((detected + 1))
done
SLASH_H="${SH[0]:-}" SLASH_X="${SX[0]:-}" one_place=0
all_equal "$SLASH_H" "${SH[@]}" && all_equal "$SLASH_X" "${SX[@]}" && one_place=1
res gs027-one-slash-mined "$([ "$mined" = "$N" ] && [ "$one_place" = 1 ] && echo 1 || echo 0)" \
  "mined_once=$mined/$N slash_height=${SLASH_H:-none} slash=${SLASH_X:0:12} unclassifiable=$(echo "${BAD[@]}" | tr -cd 1 | wc -c | tr -d ' ')"
res gs027-producer-slashed-everywhere "$([ "$slashed" = "$N" ] && echo 1 || echo 0)" \
  "slashed=$slashed/$N status=$(echo "${ST[@]}" | tr ' ' ',')"
res gs027-bond-burned-once "$([ "$burned" = "$N" ] && echo 1 || echo 0)" \
  "burned_once=$burned/$N bond=${DN_BOND:-none} reward=$REWARD"
mh="$(min_height)" || mh=0
# shellcheck disable=SC2046
res gs027-tips-equal "$(all_equal $(hashes_at "$mh") && echo 1 || echo 0)" "height=$mh nodes=$N"
res gs027-production-continues "$([ -n "$SLASH_H" ] && [ "$mh" -ge $((SLASH_H + GS027_ADVANCE)) ] && echo 1 || echo 0)" \
  "common_tip=$mh slash_height=${SLASH_H:-none} need=+$GS027_ADVANCE"
res gs027-mempool-slash-bounded "$([ "$bounded" = "$N" ] && echo 1 || echo 0)" "bounded=$bounded/$N max_residents=$maxres"
union="$(printf '%s\n' "${IDS[@]}" | tr ' ' '\n' | sed '/^$/d' | sort -u)"
nunion="$(printf '%s\n' "$union" | sed '/^$/d' | wc -l | tr -d ' ')"
res gs027-one-shared-slash-id "$([ "$idsmall" = "$N" ] && [ "$nunion" = 1 ] && [ "$union" = "$SLASH_X" ] && echo 1 || echo 0)" \
  "per_node_le1=$idsmall/$N distinct=$nunion creators=${creators:-none}"

echo "METRIC SLASHED_NODES=$slashed_nodes/$N detected=$detected/$N target=${DN_TARGET_PK:0:8} inj_height=$DN_INJ_H slash_height=${SLASH_H:-none} bond=${DN_BOND:-none} reward=$REWARD"
verdict; exit $?
