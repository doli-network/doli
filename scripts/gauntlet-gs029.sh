#!/usr/bin/env bash
# gauntlet-gs029.sh — GS-029 "mixed-version devnet: producer transactions, same-dir restart and
# gate crossing" (INC-I-182, INC-I-257, INC-I-255, INC-I-236). A FAIL means old and HEAD nodes
# disagree on blocks, state or the producer set while mixed, a same-dir restart onto HEAD rebuilds
# or resyncs, or the crossing of the gates forks or admits an unsigned producer tx or block, so
# the release must not ship as a rolling upgrade.
#
# EPHEMERAL devnet (throwaway keys, one chainspec, free high ports): GS029_NODES nodes; nodes 0-3
# genesis producers (one bond each: genesis completion backs exactly one bond per producer, so
# node 0's withdrawal is a whole-bond exit); node 3 (producer) and node 5 (validator) run
# GS029_OLD_NODE_BIN, the rest GS029_NODE_BIN. The four gates = GS029_AH for HEAD nodes and the
# CLI (the old binary ignores them), pending-root 0, reward epoch GS029_EPOCH_BLOCKS. Phases:
# funding; transfer (through the old validator), Registration of a fresh wallet, AddBond, partial
# withdrawal; boundary check; restart of nodes 3 and 5 onto HEAD on their data dirs; boundary;
# crossing; probes above the gate. A background sampler records supply and tips.
#
# OUTPUT CONTRACT: one `PASS|FAIL <id> <measured k=v>` line per assertion, then the METRIC line
# (last line), then the exit code.
#   gs029-mixed-txs-mined           the four txs at one height below GS029_AH on every node;
#                                   withdrawal payout normal; the transfer sent via an old node
#   gs029-old-producer-built        >= 1 block of phases 1-3 built by the old producer
#   gs029-mixed-producer-set-equal  normalised producer set equal on every node after the boundary
#   gs029-mixed-no-fork             hash history 1..end of phase 3 equal; no fork in sampled tips
#   gs029-mixed-state-equal         supply equal per sampled height; state roots equal
#   gs029-old-blocks-canonical      every tip sampled in phases 1-3 is on the final chain
#   gs029-restart-same-datadir      old nodes restart on HEAD at >= their stop height; no epoch
#                                   rebuild, snap sync or equivocation; a boundary before the gate
#   gs029-crossing-no-fork          hash history from the restart to GS029_AH + 10 equal
#   gs029-unsigned-tx-refused       old-CLI AddBond (tx forge fallback) refused with ERRTX-PAUTH001
#   gs029-unsigned-block-refused    an INJECTED unsigned block held by no node
#   gs029-signed-tx-mined           HEAD-CLI AddBond mined by GS029_AH + 12
#   gs029-post-gate-unbonding       HEAD-CLI withdrawal: one unbonding output locked above its block
#   METRIC heights= old_built= old_built_producer_txs= new_producer_blocks= max_tip_diverge_slots=
#          ah= gate= negative=
#   exit: 0 all PASS · 1 any FAIL · 2 SKIP (`SKIP gs029: <reason>`)
# INPUT PARTITIONS: HEAD (all PASS) · GS029_NEGATIVE=1 HEAD gates at the bootstrap height + 2,
#   mixed phase observed only (fork: gs029-mixed-no-fork or gs029-old-blocks-canonical FAIL,
#   exit 1; a stall with no fork evidence is `SKIP gs029: control inconclusive`) · precondition
#   missing or a phase overran with no fork evidence (SKIP).
# Env: GS029_NODE_BIN GS029_OLD_NODE_BIN GS029_CLI GS029_OLD_CLI GS029_FORGE (block forge)
#   GS029_TX_FORGE (tx forge fallback) GS029_NEGATIVE GS029_STALL_SLOTS GS029_NODES GS029_AH
#   GS029_EPOCH_BLOCKS GS029_GENESIS_BLOCKS GS029_BOOTSTRAP_BLOCKS GS029_SLOT_DUR
#   GS029_GENESIS_DELAY GS029_P2P_BASE GS029_RPC_BASE GS029_MET_BASE GS029_BOOT_MAX
#   GS029_KEEP (1 = keep scratch)
# SAFETY: signals only PIDs it launched whose cmdline names its own mktemp dir and removes only
#   that dir; loopback only.

set -u
set -o pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/gauntlet-devnet-lib.sh
. "$ROOT/scripts/gauntlet-devnet-lib.sh"
# shellcheck source=scripts/gauntlet-gs029-probes.sh
. "$ROOT/scripts/gauntlet-gs029-probes.sh"
ALL=0

res() {
  local id="$1" ok="$2"
  shift 2
  [ "$ok" = 1 ] && echo "PASS $id $*" && return 0
  echo "FAIL $id $*"
  ALL=1
}

verdict() { [ "$ALL" = 0 ]; }
sha() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -c1-16; }

GS029_NODE_BIN="${GS029_NODE_BIN:-$ROOT/target/release/doli-node}"
GS029_OLD_NODE_BIN="${GS029_OLD_NODE_BIN:-$ROOT/target/gs026/doli-node-b4cff010}"
GS029_CLI="${GS029_CLI:-$ROOT/target/release/doli}"
GS029_OLD_CLI="${GS029_OLD_CLI:-$ROOT/target/gs026/doli-b4cff010}"
GS029_FORGE="${GS029_FORGE:-$ROOT/target/release/examples/gs026_forge_block}"
GS029_TX_FORGE="${GS029_TX_FORGE:-$ROOT/target/release/examples/gs028_forge_tx}"
GS029_NEGATIVE="${GS029_NEGATIVE:-0}"
GS029_STALL_SLOTS="${GS029_STALL_SLOTS:-6}"
GS029_NODES="${GS029_NODES:-6}"
GS029_AH="${GS029_AH:-72}"
GS029_EPOCH_BLOCKS="${GS029_EPOCH_BLOCKS:-8}"
GS029_GENESIS_BLOCKS="${GS029_GENESIS_BLOCKS:-10}"
GS029_BOOTSTRAP_BLOCKS="${GS029_BOOTSTRAP_BLOCKS:-12}"
GS029_SLOT_DUR="${GS029_SLOT_DUR:-10}"
GS029_GENESIS_DELAY="${GS029_GENESIS_DELAY:-90}"
GS029_P2P_BASE="${GS029_P2P_BASE:-50420}"
GS029_RPC_BASE="${GS029_RPC_BASE:-28620}"
GS029_MET_BASE="${GS029_MET_BASE:-29120}"
GS029_BOOT_MAX="${GS029_BOOT_MAX:-180}"
GS029_KEEP="${GS029_KEEP:-0}"

REWARD=200000000
BOND_UNIT=100000000
TMPROOT="${TMPDIR:-/tmp}"
TMPROOT="${TMPROOT%/}"
N="$GS029_NODES"
P=4
E="$GS029_EPOCH_BLOCKS"
AH="$GS029_AH"
S=""
GATE="$AH"
[ "$GS029_NEGATIVE" = 1 ] && GATE=$((GS029_BOOTSTRAP_BLOCKS + 2))
DN_TAG=gs029
DN_CLI="$GS029_CLI"
DN_FORGE="$GS029_FORGE"
DN_SLOT="$GS029_SLOT_DUR"
DN_REWARD="$REWARD"
DN_BOND_UNIT="$BOND_UNIT"
DN_KEEP="$GS029_KEEP"
# Chainspec bond_count is not honoured by genesis completion (one real bond per producer), so
# every genesis producer starts with one bond and node 0's withdrawal of one bond is its exit.
# shellcheck disable=SC2191
DN_ENV=(DOLI_INC_I_236_BLOCK_SIGNATURE_ACTIVATION_HEIGHT="$GATE"
  DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT="$GATE"
  DOLI_INC_I_257_UNBONDING_ACTIVATION_HEIGHT="$GATE"
  DOLI_INC_I_182_PRODUCER_AUTH_ACTIVATION_HEIGHT="$GATE"
  DOLI_INC_I_181_PENDING_ROOT_ACTIVATION_HEIGHT=0
  DOLI_GENESIS_BLOCKS="$GS029_GENESIS_BLOCKS"
  DOLI_BOOTSTRAP_BLOCKS="$GS029_BOOTSTRAP_BLOCKS"
  DOLI_BLOCKS_PER_REWARD_EPOCH="$GS029_EPOCH_BLOCKS")
DN_CLI_ENV=("${DN_ENV[@]}")

PHASE=boot WHY="" ROOTS="" LIM13=0 B3=none BR=none RAN=0 FORK=None HIST_EQ=None HFIRST=None
TXH_T=none TXH_R=none TXH_A=none TXH_W=none TXW_TYPE=none NORM_EQ=None NORM_SHOW=none
UTX_CODES="" UTX_IDS="" UTX_FORGE=skipped UB_H=none UB_HASH=none UB_HELD=0
SIG_H=none PG_H=none PG_NOUT=0 PG_TYPE=none PG_LOCK=0
declare -a RSTOP RFIRST FTIP

# --- preconditions --------------------------------------------------------------
[ "$N" -ge 6 ] 2>/dev/null || skip "GS029_NODES=$N must be >= 6 (bootstrap gate; nodes 3 and 5 run the old binary)"
[ "$GS029_NEGATIVE" = 1 ] || [ "$AH" -ge $((GS029_BOOTSTRAP_BLOCKS + 6 * E)) ] 2>/dev/null ||
  skip "GS029_AH=$AH must be >= GS029_BOOTSTRAP_BLOCKS + 6 x GS029_EPOCH_BLOCKS (mixed txs, restart and a boundary below the gate)"
is_num "$GS029_STALL_SLOTS" || skip "GS029_STALL_SLOTS must be digits"
for c in curl lsof python3; do command -v "$c" >/dev/null 2>&1 || skip "$c unavailable"; done
[ -n "$(build_running)" ] && skip "a build is running (cargo/rustc) in this checkout $ROOT"
for b in "$GS029_NODE_BIN" "$GS029_OLD_NODE_BIN" "$GS029_CLI" "$GS029_OLD_CLI" "$GS029_FORGE"; do
  [ -x "$b" ] || skip "binary $b missing or not executable"
done
[ "$(sha "$GS029_NODE_BIN")" != "$(sha "$GS029_OLD_NODE_BIN")" ] || skip "old and HEAD node binaries are identical"
for ((i = 0; i < N; i++)); do DN_BIN[i]="$GS029_NODE_BIN"; done
DN_BIN[3]="$GS029_OLD_NODE_BIN"
DN_BIN[5]="$GS029_OLD_NODE_BIN"
check_ports "$GS029_P2P_BASE" "$GS029_RPC_BASE" "$GS029_MET_BASE"
[ "$GS029_NEGATIVE" = 1 ] && echo "gs029: CONTROL expect_fail=gs029-mixed-no-fork,gs029-old-blocks-canonical preconditions=tips-equal-below-gate gate=$GATE"

# --- scratch cluster ------------------------------------------------------------
trap cleanup EXIT
trap 'exit 1' INT TERM
S="$(mktemp -d "$TMPROOT/gs029.XXXXXX")" || skip "mktemp failed"
gen_wallets 5
# `send` takes the bech32 address (network HRP + 1 + data) that only the CLI prints; the
# wallet file holds the hex pubkey hash, which `send` refuses.
NEW_ADDR="$("$DN_CLI" -w "$S/w4.json" -n devnet addresses 2>/dev/null | sed -nE 's/^Address:[[:space:]]+([a-z]+1[0-9a-z]+).*/\1/p' | head -1)"
case "$NEW_ADDR" in '' | *[!0-9a-z]*) skip "fresh wallet has no bech32 address" ;; esac
printf '%s' "$NEW_ADDR" | grep -qE '^[a-z]+1[0-9a-z]{20,}$' || skip "fresh wallet address is not bech32 (${NEW_ADDR:0:12})"
T_START="$(date +%s)"
DN_GEN_TS=$((((T_START + GS029_GENESIS_DELAY) / DN_SLOT + 1) * DN_SLOT))
write_chainspec "GS-029 ephemeral devnet" "GS-029 ephemeral" || skip "chainspec write failed"
echo "gs029: nodes=$N producers=$P ah=$AH gate=$GATE negative=$GS029_NEGATIVE epoch_blocks=$E scratch=$S"
for ((i = 0; i < N; i++)); do
  role="head"
  [ "${DN_BIN[$i]}" = "$GS029_OLD_NODE_BIN" ] && role=old
  [ -n "${DN_KEY[$i]:-}" ] && role="$role-producer"
  echo "gs029: n$i role=$role bin=${DN_BIN[$i]} sha256=$(sha "${DN_BIN[$i]}") pk=${DN_PK[$i]:-none}"
done
start_cluster $((T_START + GS029_GENESIS_DELAY))
seed_gset || skip "a producer did not persist its announcement within 30 s"
start_cluster $((T_START + GS029_GENESIS_DELAY))
while [ "$(date +%s)" -lt $((DN_GEN_TS - 3)) ]; do all_alive || skip "a node exited before genesis"; sleep 2; done
gready="$(gset_ready)"
[ "$gready" = "$N" ] || skip "producer G-Set incomplete on some node before genesis ($gready/$N); raise GS029_GENESIS_DELAY"
sampler_start
wait_height 2 "$GS029_BOOT_MAX" || skip "cluster did not reach height 2 within ${GS029_BOOT_MAX}s of genesis or a node exited"
R0="${DN_RPC[0]}"
PORTS="$(echo "${DN_RPC[@]}" | tr ' ' ',')"
F_ADDR="$(pfield "$R0" "${DN_PK[0]}" addressHash)" P1_ADDR="$(pfield "$R0" "${DN_PK[1]}" addressHash)"
{ is_hex "$F_ADDR" && is_hex "$P1_ADDR"; } || skip "producers 0 and 1 have no address hash"

# --- phases (a phase that overruns is a fork when the samples show one, else a SKIP) ------
if [ "$GS029_NEGATIVE" = 1 ]; then
  PHASE=negative-observe
  reach $((GATE + 2 * E + 2)) && RAN=1
else
  head_phases && RAN=1
fi
echo "gs029: phases ran=$RAN last_phase=$PHASE ${WHY:+why=\"$WHY\"}"
final_scan
LIM="$LIM13"
{ [ "$GS029_NEGATIVE" = 1 ] || [ "$RAN" = 0 ] || ! is_num "$LIM" || [ "$LIM" -lt 1 ]; } && LIM="$FMIN"
{ is_num "$LIM" && [ "$LIM" -ge 1 ]; } || skip "no node answered after the phases ($PHASE: $WHY)"
forkeval "$LIM"
FORKED=$?
if [ "$RAN" = 0 ] && [ "$FORKED" != 0 ]; then
  [ "$GS029_NEGATIVE" = 1 ] && skip "control inconclusive ($PHASE: $WHY; no fork evidence up to height $LIM)"
  skip "$PHASE overran ($WHY; no fork evidence up to height $LIM)"
fi
if [ "$GS029_NEGATIVE" = 1 ]; then
  # shellcheck disable=SC2046
  devnet_py history 1 $((GATE - 1)) $(blk_dirs) >"$S/hist.pre.json"
  [ "$(jfield equal <"$S/hist.pre.json")" = True ] ||
    skip "control inconclusive (history below the gate not equal: first_diff=$(jfield first_diff <"$S/hist.pre.json"))"
fi

# --- assertions ---------------------------------------------------------------------
devnet_py attrib "$S/n3/blk" "${DN_PK[3]}" 1 "$LIM" >"$S/attrib.json"
OLD_BUILT="$(jlines old_built <"$S/attrib.json" | sed '/^$/d' | wc -l | tr -d ' ')"
OLD_PTX="$(jfield old_built_producer_txs <"$S/attrib.json")"
devnet_py attrib "$S/n0/blk" "${DN_PK[4]}" 1 "$FMIN" >"$S/attrib.new.json"
NEW_BLOCKS="$(jlines old_built <"$S/attrib.new.json" | sed '/^$/d' | wc -l | tr -d ' ')"
ok=0
[ "$OLD_BUILT" -ge 1 ] && ok=1
res gs029-old-producer-built "$ok" "old_built=$OLD_BUILT heights=1..$LIM producer_txs=${OLD_PTX:-none} old_pk=${DN_PK[3]:0:8}"
ok=0
[ "$FORK" = False ] && [ "$HIST_EQ" = True ] && ok=1
res gs029-mixed-no-fork "$ok" "first_diff=$FIRST sides=${SIDES:-none} heights=1..$LIM fork=$FORK history_equal=$HIST_EQ"
ok=0
{ [ "$FORK" = True ] || [ "$FORK" = False ]; } && [ "$ABSENT" = 0 ] && ok=1
res gs029-old-blocks-canonical "$ok" "absent=$ABSENT first_absent=${ABSENT1:-none} heights=1..$LIM"

if [ "$GS029_NEGATIVE" != 1 ]; then
  ok=0
  for h in "$TXH_T" "$TXH_R" "$TXH_A" "$TXH_W"; do is_num "$h" && [ "$h" -lt "$AH" ] && ok=$((ok + 1)); done
  [ "$ok" = 4 ] && [ "$TXW_TYPE" = normal ] && ok=1
  res gs029-mixed-txs-mined "$([ "$ok" = 1 ] && echo 1 || echo 0)" \
    "transfer=$TXH_T register=$TXH_R add_bond=$TXH_A withdrawal=$TXH_W withdrawal_type=$TXW_TYPE via_old=n5" \
    "errors=\"$(for k in transfer register addbond withdraw; do printf '%s: %s; ' "$k" "$(cli_err "$k")"; done)\""
  ok=0
  # new producer registered with 1 bond; node 0 exited (whole-bond withdrawal); node 1 at 2 bonds.
  [ "$NORM_EQ" = True ] && case "$NORM_SHOW" in new=*/1\ p0=exited/0\ p1=*/2) ok=1 ;; esac
  res gs029-mixed-producer-set-equal "$ok" "equal=$NORM_EQ at=$((${B3/none/0} + 1)) $NORM_SHOW"
  read -r SUP_N SUP_BAD <<<"$(supply_check "$LIM")"
  ok=0
  [ "$SUP_BAD" = none ] && [ "${SUP_N:-0}" -ge 1 ] && case "$ROOTS" in *MISMATCH*) ;; *:ok*) ok=1 ;; esac
  res gs029-mixed-state-equal "$ok" "supply_heights=$SUP_N supply_first_diff=$SUP_BAD roots=\"${ROOTS# }\""
  restart_counts
  ok=0
  is_num "${RFIRST[3]:-}" && is_num "${RFIRST[5]:-}" && [ "${RFIRST[3]}" -ge "${RSTOP[3]}" ] &&
    [ "${RFIRST[5]}" -ge "${RSTOP[5]}" ] && [ "$REBUILD$SNAPS$EQUIV" = 000 ] && is_num "$BR" && [ "$BR" -lt "$AH" ] && ok=1
  res gs029-restart-same-datadir "$ok" \
    "stop=${RSTOP[3]:-none},${RSTOP[5]:-none} first=${RFIRST[3]:-none},${RFIRST[5]:-none} epoch_rebuild=$REBUILD snap_sync=$SNAPS equivocation=$EQUIV boundary_after=$BR"
  RMIN="${RSTOP[3]:-none}"
  is_num "${RSTOP[5]:-}" && is_num "$RMIN" && [ "${RSTOP[5]}" -lt "$RMIN" ] && RMIN="${RSTOP[5]}"
  # shellcheck disable=SC2046
  devnet_py history "$RMIN" $((AH + 10)) $(blk_dirs) >"$S/hist.cross.json"
  ok=0
  [ "$(jfield equal <"$S/hist.cross.json")" = True ] && ok=1
  res gs029-crossing-no-fork "$ok" "heights=$RMIN..$((AH + 10)) first_diff=$(jfield first_diff <"$S/hist.cross.json")"
  mined=0
  for id in $UTX_IDS; do is_num "$(mined_at "$R0" "$id")" && mined=$((mined + 1)); done
  ok=0
  { [ "$(n_pauth "$UTX_CODES")" = "$N" ] || [ "$(n_pauth "$UTX_FORGE")" = "$N" ]; } && [ "$mined" = 0 ] && ok=1
  res gs029-unsigned-tx-refused "$ok" "code=$(distinct "$UTX_CODES") forge_code=$(distinct "$UTX_FORGE") mined=$mined"
  held_final=0
  is_hex "$UB_HASH" && held_final="$(grep -lF "$UB_HASH" "$S"/n*/blk/*.json 2>/dev/null | wc -l | tr -d ' ')"
  ok=0
  is_hex "$UB_HASH" && [ "$UB_HELD$held_final" = 00 ] && ok=1
  res gs029-unsigned-block-refused "$ok" "injected=${UB_HASH:0:16} height=$UB_H held=$UB_HELD/$N in_final_chain=$held_final"
  ok=0
  is_num "$SIG_H" && [ "$SIG_H" -le $((AH + 12)) ] && ok=1
  res gs029-signed-tx-mined "$ok" "height=$SIG_H need<=$((AH + 12)) cli_last=\"$(cli_err signed)\""
  ok=0
  is_num "$PG_H" && [ "$PG_NOUT" = 1 ] && [ "$PG_TYPE" = unbonding ] && [ "$PG_LOCK" -gt "$PG_H" ] && ok=1
  res gs029-post-gate-unbonding "$ok" "height=$PG_H outputs=$PG_NOUT type=$PG_TYPE lock=$PG_LOCK cli_last=\"$(cli_err postgate)\""
fi

echo "METRIC heights=1..$FMIN old_built=$OLD_BUILT old_built_producer_txs=${OLD_PTX:-none} new_producer_blocks=$NEW_BLOCKS max_tip_diverge_slots=$(diverge_slots) ah=$AH gate=$GATE negative=$GS029_NEGATIVE"
verdict; exit $?
