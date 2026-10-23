#!/usr/bin/env bash
# gauntlet-gs028.sh — GS-028 "withdraw-then-equivocate and forced exit" (INC-I-257, INC-I-182).
# A FAIL means a producer that withdraws its whole bond and then signs two blocks for one slot
# keeps part of the bond or its producer standing, or that a third party can force another
# producer out, so the unbonding and producer-auth fixes must not ship.
#
# EPHEMERAL devnet (throwaway keys, one chainspec, free high ports): GS028_NODES nodes, the
# first GS028_PRODUCERS genesis producers; node 1 = victim V, node 2 = third party T, target X
# from nodes 0, 3, 4. The four gates (236, 255, 257, 182) = GS028_AH for the nodes and the CLI
# (pending-root gate 0); GS028_EPOCH_BLOCKS-block reward epochs. G-Set seeding as in GS-027.
# A background sampler records supply and tips; removed(h) comes from the devnet helper.
#
# OUTPUT CONTRACT: one `PASS|FAIL <id> <measured k=v>` line per assertion (each must hold on
# EVERY node), then the METRIC line (last line), then the exit code.
#   gs028-withdrawal-before-evidence  X's whole-bond withdrawal at one W, one output, W below
#                                     the sibling height; HEAD also: unbonding, lock above S
#   gs028-one-slash-mined             one slash_producer tx, one height S > W, one tx id
#   gs028-slash-burns-locked          removed(S) == U and the `[SLASH] burned` log line == U
#   gs028-full-bond-leaves-supply     removed(W) + removed(S) == B
#   gs028-producer-gets-nothing       W:0 never normal or spendable; absent at the end;
#                                     unbonding balance 0
#   gs028-forced-exit-refused         third-party forced exit against V (HEAD: also a wrong
#                                     tail) refused with ERRTX-PAUTH001 by every node, never
#                                     mined, V's status and bond count unchanged
#   gs028-tips-equal                  one block hash at the common tip height
#   gs028-production-continues        common tip >= S + GS028_ADVANCE
#   gs028-target-status-slashed       getProducer(X).status == slashed after the boundary that
#                                     follows S; no `Deferred slash failed` log line
#   METRIC SLASHED_NODES=<k>/<n> target= W= S= B= U= attempts= status= probe_code= control=
#   exit: 0 all PASS · 1 any FAIL · 2 SKIP (`SKIP gs028: <reason>`)
# INPUT PARTITIONS: HEAD (all PASS) · GS028_CONTROL=1 pre-unbonding node and CLI, forge payout
#   normal (funds and forced-exit FAIL; exit 1 only when the preconditions PASS, else
#   `SKIP gs028: control inconclusive`) · precondition missing (SKIP).
# Env: GS028_NODE_BIN GS028_CLI GS028_FORGE (block forge) GS028_TX_FORGE (tx forge)
#   GS028_CONTROL GS028_NODES GS028_PRODUCERS GS028_AH GS028_EPOCH_BLOCKS
#   GS028_GENESIS_BLOCKS GS028_BOOTSTRAP_BLOCKS GS028_SLOT_DUR GS028_GENESIS_DELAY
#   GS028_ADVANCE GS028_P2P_BASE GS028_RPC_BASE GS028_MET_BASE GS028_BOOT_MAX
#   GS028_KEEP (1 = keep scratch)
# SAFETY: signals only PIDs it launched whose cmdline names its own mktemp dir and removes
#   only that dir; loopback only.

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
yes_no() { "$@" && echo 1 || echo 0; }

GS028_CONTROL="${GS028_CONTROL:-0}"
if [ "$GS028_CONTROL" = 1 ]; then
  NODE_DEF="$ROOT/target/gs028/doli-node-f737cc2d" CLI_DEF="$ROOT/target/gs028/doli-f737cc2d" PAYOUT=normal
else
  NODE_DEF="$ROOT/target/release/doli-node" CLI_DEF="$ROOT/target/release/doli" PAYOUT=unbonding
fi
GS028_NODE_BIN="${GS028_NODE_BIN:-$NODE_DEF}"
GS028_CLI="${GS028_CLI:-$CLI_DEF}"
GS028_FORGE="${GS028_FORGE:-$ROOT/target/release/examples/gs026_forge_block}"
GS028_TX_FORGE="${GS028_TX_FORGE:-$ROOT/target/release/examples/gs028_forge_tx}"
GS028_NODES="${GS028_NODES:-6}"
GS028_PRODUCERS="${GS028_PRODUCERS:-5}"
GS028_AH="${GS028_AH:-30}"
GS028_EPOCH_BLOCKS="${GS028_EPOCH_BLOCKS:-15}"
GS028_GENESIS_BLOCKS="${GS028_GENESIS_BLOCKS:-10}"
GS028_BOOTSTRAP_BLOCKS="${GS028_BOOTSTRAP_BLOCKS:-12}"
GS028_SLOT_DUR="${GS028_SLOT_DUR:-10}"
GS028_GENESIS_DELAY="${GS028_GENESIS_DELAY:-90}"
GS028_ADVANCE="${GS028_ADVANCE:-6}"
GS028_P2P_BASE="${GS028_P2P_BASE:-50400}"
GS028_RPC_BASE="${GS028_RPC_BASE:-28600}"
GS028_MET_BASE="${GS028_MET_BASE:-29100}"
GS028_BOOT_MAX="${GS028_BOOT_MAX:-180}"
GS028_KEEP="${GS028_KEEP:-0}"

REWARD=200000000
BOND_UNIT=100000000
TMPROOT="${TMPDIR:-/tmp}"
TMPROOT="${TMPROOT%/}"
N="$GS028_NODES"
P="$GS028_PRODUCERS"
E="$GS028_EPOCH_BLOCKS"
AH="$GS028_AH"
S=""
DN_TAG=gs028
DN_CLI="$GS028_CLI"
DN_FORGE="$GS028_FORGE"
DN_SLOT="$GS028_SLOT_DUR"
DN_REWARD="$REWARD"
DN_BOND_UNIT="$BOND_UNIT"
DN_KEEP="$GS028_KEEP"
# shellcheck disable=SC2191
DN_ENV=(DOLI_INC_I_236_BLOCK_SIGNATURE_ACTIVATION_HEIGHT="$GS028_AH"
  DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT="$GS028_AH"
  DOLI_INC_I_257_UNBONDING_ACTIVATION_HEIGHT="$GS028_AH"
  DOLI_INC_I_182_PRODUCER_AUTH_ACTIVATION_HEIGHT="$GS028_AH"
  DOLI_INC_I_181_PENDING_ROOT_ACTIVATION_HEIGHT=0
  DOLI_GENESIS_BLOCKS="$GS028_GENESIS_BLOCKS"
  DOLI_BOOTSTRAP_BLOCKS="$GS028_BOOTSTRAP_BLOCKS"
  DOLI_BLOCKS_PER_REWARD_EPOCH="$GS028_EPOCH_BLOCKS")
DN_CLI_ENV=("${DN_ENV[@]}")

# Seconds allowed for every node to climb to height $1 (1.5 slots per block + 60 s).
cap() {
  local mh
  mh="$(min_height)" || mh=0
  echo $((($1 > mh ? $1 - mh : 0) * DN_SLOT * 3 / 2 + 60))
}
nb() { echo $((($1 / E + 1) * E)); }
pfield() { producer_json "$1" "$2" | jfield "$3"; }
bfield() { call "$1" getBalance "{\"address\":\"$2\"}" | jfield "$3"; }
txid_in() { sed -nE 's/.*(TX|TX Hash|Transaction hash): ([0-9a-f]{64}).*/\2/p' "$1" | head -1; }
mined_at() { call "$1" getTransaction "{\"hash\":\"$2\"}" | jfield blockHeight; }

# "<outputs> <type> <amount> <lock>" of tx $1 (node 0); "0 none 0 0" when unreadable.
tx_out() {
  call "${DN_RPC[0]}" getTransaction "{\"hash\":\"$1\"}" | python3 -I -c '
import json, sys
try:
    o = json.load(sys.stdin)["result"]["outputs"]
    print(len(o), o[0]["outputType"], int(o[0]["amount"]), int(o[0]["lockUntil"]))
except Exception:
    print("0 none 0 0")'
}

# <type>:<spendable> of outpoint $3:0 in the UTXOs of address $2 on port $1; absent | error.
utxo_of() {
  call "$1" getUtxos "{\"address\":\"$2\"}" | python3 -I -c '
import json, sys
try:
    rows = json.load(sys.stdin)["result"]
    r = [u for u in rows if u.get("txHash") == sys.argv[1] and u.get("outputIndex") == 0]
except Exception:
    print("error")
    sys.exit()
print("%s:%s" % (r[0].get("outputType"), str(r[0].get("spendable")).lower()) if r else "absent")' "$3"
}

# Forge the third-party forced exit against V (suffix -wrong-tail adds T's own tail); submit to all.
fx() {
  "$GS028_TX_FORGE" --kind forced-exit"$1" --payout "$PAYOUT" --payer-key "$S/w2.json" \
    --victim "$V_PK" --bonds "$VB" --lock-until "$LOCK" --ports "$PORTS" >"$S/fx$1.out" 2>>"$S/txforge.log"
}
fx_codes() { sed -n 's/^SUBMIT port=[0-9]* accepted=\([01]\) code=\([A-Za-z0-9-]*\).*/\1:\2/p' "$1" | tr '\n' ' '; }
fx_refused() { printf '%s' "$1" | tr ' ' '\n' | grep -c '^0:ERRTX-PAUTH001$'; }
fx_distinct() { printf '%s' "$1" | tr ' ' '\n' | sed '/^$/d; s/^[01]://' | sort -u | tr '\n' ',' | sed 's/,$//'; }

# The `[SLASH] burned N units ... at h=H` amount N for height $2 in node $1's log.
burn_logged() {
  sed -n 's/.*\[SLASH\] burned \([0-9]*\) units.* at h=\([0-9]*\).*/\2 \1/p' "$(node_log "$1")" 2>/dev/null |
    awk -v h="$2" '$1 == h { v = $2 } END { print v }'
}

# removed(h) on node $1 at height $2 for subject tx $3; empty when undefined.
rmv() {
  local sp sh v
  sp="$(supply_at "$1" $(($2 - 1)))" sh="$(supply_at "$1" "$2")"
  { is_num "$sp" && is_num "$sh" && is_hex "$3"; } || return 1
  v="$(devnet_py removed "$S/n$1/blk" "$2" "$sp" "$sh" "$3" | jfield removed)"
  is_num "$v" && echo "$v"
}

# --- preconditions --------------------------------------------------------------
[ "$N" -ge 6 ] 2>/dev/null || skip "GS028_NODES=$N must be >= 6 (genesis bootstrap gate needs >= 5 peers)"
{ [ "$P" -ge 5 ] 2>/dev/null && [ "$P" -le "$N" ]; } ||
  skip "GS028_PRODUCERS=$P must be in [5, GS028_NODES] (3 must remain when X and V leave)"
{ [ "$AH" -ge "$E" ] && [ "$AH" -gt "$GS028_BOOTSTRAP_BLOCKS" ] && [ "$E" -ge $((2 * P)) ]; } 2>/dev/null ||
  skip "GS028_AH=$AH must be past epoch 0 and the bootstrap blocks; GS028_EPOCH_BLOCKS=$E must be >= 2 x producers"
for c in curl lsof python3; do command -v "$c" >/dev/null 2>&1 || skip "$c unavailable"; done
[ -n "$(build_running)" ] && skip "a build is running (cargo/rustc) in this checkout $ROOT"
[ -x "$GS028_NODE_BIN" ] || skip "node binary $GS028_NODE_BIN missing or not executable"
[ -x "$GS028_CLI" ] || skip "CLI $GS028_CLI missing or not executable"
[ -x "$GS028_FORGE" ] || skip "block forge $GS028_FORGE missing (cargo build --release -p doli-node --example gs026_forge_block)"
[ -x "$GS028_TX_FORGE" ] || skip "tx forge $GS028_TX_FORGE missing (cargo build --release -p doli-node --example gs028_forge_tx)"
for ((i = 0; i < N; i++)); do DN_BIN[i]="$GS028_NODE_BIN"; done
check_ports "$GS028_P2P_BASE" "$GS028_RPC_BASE" "$GS028_MET_BASE"
[ "$GS028_CONTROL" = 1 ] && echo "gs028: CONTROL expect_fail=gs028-slash-burns-locked,gs028-full-bond-leaves-supply,gs028-producer-gets-nothing,gs028-forced-exit-refused,gs028-target-status-slashed preconditions=gs028-withdrawal-before-evidence,gs028-tips-equal"

# --- scratch cluster ------------------------------------------------------------
trap cleanup EXIT
trap 'exit 1' INT TERM
S="$(mktemp -d "$TMPROOT/gs028.XXXXXX")" || skip "mktemp failed"
gen_wallets
T_START="$(date +%s)"
DN_GEN_TS=$((((T_START + GS028_GENESIS_DELAY) / DN_SLOT + 1) * DN_SLOT))
write_chainspec "GS-028 ephemeral devnet" "GS-028 ephemeral" || skip "chainspec write failed"
echo "gs028: node=$GS028_NODE_BIN cli=$GS028_CLI control=$GS028_CONTROL payout=$PAYOUT nodes=$N producers=$P ah=$AH epoch_blocks=$E scratch=$S"
start_cluster $((T_START + GS028_GENESIS_DELAY))
seed_gset || skip "a producer did not persist its announcement within 30 s"
start_cluster $((T_START + GS028_GENESIS_DELAY))
while [ "$(date +%s)" -lt $((DN_GEN_TS - 3)) ]; do all_alive || skip "a node exited before genesis"; sleep 2; done
gready="$(gset_ready)"
[ "$gready" = "$N" ] || skip "producer G-Set incomplete on some node before genesis ($gready/$N); raise GS028_GENESIS_DELAY"
sampler_start
wait_height 2 "$GS028_BOOT_MAX" || skip "cluster did not reach height 2 within ${GS028_BOOT_MAX}s of genesis or a node exited"
wait_height $((AH + 1)) "$(cap $((AH + 1)))" ||
  skip "cluster did not pass GS028_AH=$AH or a node exited (heights: $(heights | tr '\n' ' '))"

# --- forced-exit probe: T builds a withdrawal naming V ------------------------------
R0="${DN_RPC[0]}"
V_PK="${DN_PK[1]}"
T_ADDR="$(pfield "$R0" "${DN_PK[2]}" addressHash)"
is_hex "$T_ADDR" || skip "third party T has no address hash"
bal="$(bfield "$R0" "$T_ADDR" confirmed)"
if ! { is_num "$bal" && [ "$bal" -gt 1000 ]; }; then
  mh="$(min_height)" || mh="$AH"
  wait_height $((mh + E)) "$(cap $((mh + E)))" || skip "cluster stalled while the third party waited for funds"
  bal="$(bfield "$R0" "$T_ADDR" confirmed)"
  { is_num "$bal" && [ "$bal" -gt 1000 ]; } || skip "third party unfunded (confirmed=${bal:-none})"
fi
VB="$(pfield "$R0" "$V_PK" bondCount)"
is_num "$VB" || skip "victim V has no bond count"
V_BEFORE="$(pfield "$R0" "$V_PK" status)/$VB"
PORTS="$(echo "${DN_RPC[@]}" | tr ' ' ',')"
tip="$(best_height "$R0")"
is_num "$tip" || tip="$AH"
LOCK=$((tip + 200))
fx ""
FX_RC=$?
FX_TX="$(sed -n 's/^FORGED .*txid=\([0-9a-f]*\).*/\1/p' "$S/fx.out" | head -1)"
FX_CODES="$(fx_codes "$S/fx.out")"
WT_CODES=skipped
if [ "$GS028_CONTROL" != 1 ]; then
  fx -wrong-tail
  WT_CODES="$(fx_codes "$S/fx-wrong-tail.out")"
fi
echo "gs028: forced exit rc=$FX_RC tx=${FX_TX:0:12} codes=$FX_CODES wrong_tail=$WT_CODES"

# --- target X (not the next scheduled producer) withdraws its whole bond -------------
sched="$(call "$R0" getSlotSchedule "{\"count\":$E}")"
xpk="$(printf '%s' "$sched" | python3 -I -c '
import json, sys
try:
    s = json.load(sys.stdin)["result"]["slots"]
except Exception:
    s = []
s = [e.get("producer") for e in s if isinstance(e, dict)]
nxt = s[1] if len(s) > 1 else ""
c = [p for p in sys.argv[1:] if p != nxt] or sys.argv[1:]
n = [s[2:].count(p) for p in c]
print(c[n.index(max(n))] if max(n) > 0 else "")' "${DN_PK[0]}" "${DN_PK[3]}" "${DN_PK[4]}")"
X=3
for i in 0 3 4; do [ "${DN_PK[$i]}" = "$xpk" ] && X="$i"; done
X_PK="${DN_PK[$X]}"
B="$(pfield "$R0" "$X_PK" bondAmount)" XB="$(pfield "$R0" "$X_PK" bondCount)" X_ADDR="$(pfield "$R0" "$X_PK" addressHash)"
{ is_num "$B" && is_num "$XB" && is_hex "$X_ADDR"; } || skip "target X has no bond amount, bond count or address"
cli_as "$X" 0 producer request-withdrawal --count "$XB" </dev/null >"$S/withdraw.out" 2>&1
WTX="$(txid_in "$S/withdraw.out")"
is_hex "$WTX" || skip "withdrawal not submitted: $(tail -1 "$S/withdraw.out")"
W="$(wait_tx_mined "$WTX" $((12 * DN_SLOT)))" || skip "withdrawal $WTX not at one height on every node within 12 slots"
read -r W_NOUT W_TYPE U W_LOCK <<<"$(tx_out "$WTX")"
MID=""
for ((i = 0; i < N; i++)); do MID="$MID$(utxo_of "${DN_RPC[$i]}" "$X_ADDR" "$WTX"),"; done
echo "gs028: target=${X_PK:0:8} node=$X B=$B withdrawal=${WTX:0:12} W=$W outputs=$W_NOUT type=$W_TYPE U=$U lock=$W_LOCK mid=$MID"

# --- evidence: sibling of X's first block above W ------------------------------------
DN_SIB_PK="$X_PK"
DN_SIB_MIN=$((W + 1))
DN_PHASE_MAX=$((($(nb "$W") - W + 2) * DN_SLOT))
DN_INJ_H=0 DN_INJ_SLOT=0 DN_OWN_HASH="" DN_SIB_HASH=""
inject_sibling >"$S/inject.out"
INJ_RC=$?
cat "$S/inject.out"
ATTEMPTS="$(grep -c 'forge attempt' "$S/inject.out")"
[ "$INJ_RC" = 0 ] || skip "evidence window missed: no block by the target above W=$W held by every node before the boundary"
ATTEMPTS=$((ATTEMPTS + 1))
echo "gs028: injected height=$DN_INJ_H slot=$DN_INJ_SLOT own=${DN_OWN_HASH:0:12} sibling=${DN_SIB_HASH:0:12} attempts=$ATTEMPTS"
wait_height $((DN_INJ_H + 6)) "$(cap $((DN_INJ_H + 6)))" || echo "gs028: slash window ended early (heights: $(heights | tr '\n' ' '))"
mkdir -p "$S/early"
devnet_py scan "$S/early" "$R0" $((DN_INJ_H + 1)) $((DN_INJ_H + 6)) | jlines slashes >"$S/early.txt"
read -r S0 _ <"$S/early.txt" || S0=""
is_num "$S0" || S0=$((DN_INJ_H + 6))
END=$(($(nb "$S0") + GS028_ADVANCE))
wait_height "$END" "$(cap "$END")" || echo "gs028: observation window ended early (heights: $(heights | tr '\n' ' '))"
sampler_stop

# --- evidence per node ------------------------------------------------------------
declare -a NSL SH SX RW RS LB ST AFT UNB DEF VA FXM
STOR=0
for ((i = 0; i < N; i++)); do
  r="${DN_RPC[$i]}"
  tip="$(best_height "$r")"
  is_num "$tip" || tip=1
  devnet_py scan "$S/n$i/blk" "$r" 1 "$tip" >"$S/n$i/scan.json"
  jlines slashes <"$S/n$i/scan.json" >"$S/n$i/slashes.txt"
  NSL[i]="$(sed '/^$/d' "$S/n$i/slashes.txt" | wc -l | tr -d ' ')"
  [ "$(jfield written <"$S/n$i/scan.json")" = "$tip" ] || NSL[i]=incomplete
  read -r "SH[$i]" "SX[$i]" _ <"$S/n$i/slashes.txt" || SH[i]="" SX[i]=""
  RW[i]="$(rmv "$i" "$W" "$WTX")"
  RS[i]="" LB[i]=""
  if is_num "${SH[$i]}"; then
    RS[i]="$(rmv "$i" "${SH[$i]}" "${SX[$i]}")"
    LB[i]="$(burn_logged "$i" "${SH[$i]}")"
  fi
  ST[i]="$(pfield "$r" "$X_PK" status)"
  AFT[i]="$(utxo_of "$r" "$X_ADDR" "$WTX")"
  UNB[i]="$(bfield "$r" "$X_ADDR" unbonding)"
  DEF[i]="$(grep -c 'Deferred slash failed' "$(node_log "$i")" 2>/dev/null)"
  STOR=$((STOR + $(grep -c 'STOR041' "$(node_log "$i")" 2>/dev/null)))
  VA[i]="$(pfield "$r" "$V_PK" status)/$(pfield "$r" "$V_PK" bondCount)"
  FXM[i]=0
  is_hex "$FX_TX" && is_num "$(mined_at "$r" "$FX_TX")" && FXM[i]=1
done

# --- assertions ---------------------------------------------------------------------
SLASH_H="${SH[0]:-}" SLASH_X="${SX[0]:-}"
one=0 mined_once=0 burned=0 full=0 slashed=0 gone=0 vsame=0 fxm=0 snodes=0 deferred=0
all_equal "$SLASH_H" "${SH[@]}" && all_equal "$SLASH_X" "${SX[@]}" && is_num "$SLASH_H" && [ "$SLASH_H" -gt "$W" ] && one=1
for ((i = 0; i < N; i++)); do
  [ "${NSL[$i]}" = 1 ] && mined_once=$((mined_once + 1))
  b=0
  [ -n "${RS[$i]}" ] && [ "${RS[$i]}" = "$U" ] && [ "${LB[$i]}" = "$U" ] && b=1 && burned=$((burned + 1))
  [ -n "${RW[$i]}" ] && [ -n "${RS[$i]}" ] && [ $((RW[i] + RS[i])) = "$B" ] && full=$((full + 1))
  [ "${ST[$i]}" = slashed ] && slashed=$((slashed + 1))
  [ "${AFT[$i]}" = absent ] && [ "${UNB[$i]}" = 0 ] && gone=$((gone + 1))
  [ "${VA[$i]}" = "$V_BEFORE" ] && vsame=$((vsame + 1))
  fxm=$((fxm + FXM[i]))
  deferred=$((deferred + ${DEF[$i]:-0}))
  [ "${NSL[$i]}" = 1 ] && [ "${ST[$i]}" = slashed ] && [ "$b" = 1 ] && snodes=$((snodes + 1))
done
[ "$mined_once" = "$N" ] || one=0
WBE_OK=0
[ "$W_NOUT" = 1 ] && [ "$W" -lt "$DN_INJ_H" ] && WBE_OK=1
[ "$GS028_CONTROL" = 1 ] || { [ "$W_TYPE" = unbonding ] && [ "$W_LOCK" -gt "${SLASH_H:-$DN_INJ_H}" ]; } || WBE_OK=0
res gs028-withdrawal-before-evidence "$WBE_OK" \
  "W=$W outputs=$W_NOUT type=$W_TYPE lock=$W_LOCK sibling_height=$DN_INJ_H slash_height=${SLASH_H:-none} control=$GS028_CONTROL"
res gs028-one-slash-mined "$one" "mined_once=$mined_once/$N slash_height=${SLASH_H:-none} slash=${SLASH_X:0:12} W=$W"
B1="$(yes_no test "$burned" = "$N")" B2="$(yes_no test "$full" = "$N")" B3=0
res gs028-slash-burns-locked "$B1" "burned_ok=$burned/$N removed_S=${RS[0]:-none} log_burn=${LB[0]:-none} U=$U"
res gs028-full-bond-leaves-supply "$B2" "ok=$full/$N removed_W=${RW[0]:-none} removed_S=${RS[0]:-none} B=$B"
case "$W_TYPE,$MID" in *normal* | *:true*) ;; *) [ "$gone" = "$N" ] && B3=1 ;; esac
res gs028-producer-gets-nothing "$B3" \
  "w_type=$W_TYPE mid=${MID%,} after=$(echo "${AFT[@]}" | tr ' ' ',') unbonding=$(echo "${UNB[@]}" | tr ' ' ',')"
refused="$(fx_refused "$FX_CODES")" wt=skipped fx_ok=0
[ "$WT_CODES" = skipped ] || wt="$(fx_refused "$WT_CODES")"
[ "$refused" = "$N" ] && [ "$fxm" = 0 ] && [ "$vsame" = "$N" ] && { [ "$wt" = skipped ] || [ "$wt" = "$N" ]; } && fx_ok=1
res gs028-forced-exit-refused "$fx_ok" "refused=$refused/$N code=$(fx_distinct "$FX_CODES") mined=$fxm" \
  "wrong_tail_refused=$wt victim=$V_BEFORE->${VA[0]} unchanged=$vsame/$N"
mh="$(min_height)" || mh=0
# shellcheck disable=SC2046
TIPS_OK="$(yes_no all_equal $(hashes_at "$mh"))"
res gs028-tips-equal "$TIPS_OK" "height=$mh nodes=$N"
go=0
is_num "$SLASH_H" && [ "$mh" -ge $((SLASH_H + GS028_ADVANCE)) ] && go=1
res gs028-production-continues "$go" "common_tip=$mh slash_height=${SLASH_H:-none} need=+$GS028_ADVANCE"
st_ok=0
[ "$slashed" = "$N" ] && [ "$deferred" = 0 ] && st_ok=1
res gs028-target-status-slashed "$st_ok" \
  "slashed=$slashed/$N status=$(echo "${ST[@]}" | tr ' ' ',') deferred_failed=$deferred stor041=$STOR"

PROBE_CODE="$(echo "$FX_CODES" | awk '{ sub(/^[01]:/, "", $1); print $1 }')"
echo "METRIC SLASHED_NODES=$snodes/$N target=${X_PK:0:8} W=$W S=${SLASH_H:-none} B=$B U=$U attempts=$ATTEMPTS status=${ST[0]:-none} probe_code=${PROBE_CODE:-none} control=$GS028_CONTROL"
[ "$GS028_CONTROL" = 1 ] && ! { [ "$WBE_OK$TIPS_OK" = 11 ] && [ "$B1$B2$B3" != 111 ]; } &&
  skip "control inconclusive (preconditions=$WBE_OK$TIPS_OK funds=$B1$B2$B3)"
verdict; exit $?
