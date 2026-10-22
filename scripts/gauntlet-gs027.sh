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

all_equal() {
  local v
  [ "$#" -ge 2 ] || return 1
  for v in "$@"; do { [ -n "$v" ] && [ "$v" = "$1" ]; } || return 1; done
}

# Distinct slash ids the equivocation handler admitted (`slash <id> for <pk> admitted`).
slash_ids_from_log() {
  [ -f "$1" ] || return 1
  grep -oE 'slash [0-9a-f]{64} for [0-9a-f]+ admitted' "$1" | awk '{ print $2 }' | sort -u
  return 0
}

if [ "${GS027_LIB_ONLY:-0}" = 1 ]; then
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

SLOT_DUR="$GS027_SLOT_DUR"
REWARD=200000000
BOND_UNIT=100000000
TMPROOT="${TMPDIR:-/tmp}"
TMPROOT="${TMPROOT%/}"
N="$GS027_NODES"
P="$GS027_PRODUCERS"
S=""
NODE_PIDS=""
declare -a P2P RPCP MET PK NODE_PID

skip() { echo "SKIP gs027: $*"; exit 2; }

rpc() { curl -s -m 8 "http://127.0.0.1:$1" -H 'Content-Type: application/json' -d "$2" 2>/dev/null; }
call() { local p="${3:-}"; [ -n "$p" ] || p='{}'; rpc "$1" "{\"jsonrpc\":\"2.0\",\"method\":\"$2\",\"params\":$p,\"id\":1}"; }
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

# result.<key> from a JSON-RPC body on stdin.
jfield() {
  python3 -I -c '
import json, sys
try:
    d = json.load(sys.stdin)
    print((d.get("result", d) if isinstance(d, dict) else d)[sys.argv[1]])
except Exception:
    pass' "$1" 2>/dev/null
}

best_height() { call "$1" getChainInfo | jfield bestHeight; }
block_json() { call "$1" getBlockByHeight "{\"height\":$2}"; }
block_hash() { block_json "$1" "$2" | jfield hash; }
block_raw() { call "$1" getBlockRaw "{\"height\":$2}" | jfield block; }
producer_json() { call "$1" getProducer "{\"publicKey\":\"$2\"}"; }

stop_one() {
  local PID="$1" n=0
  [ -n "$PID" ] || return 0
  if alive "$PID" && ours "$PID"; then
    kill -TERM "$PID" 2>/dev/null
    while alive "$PID" && [ "$n" -lt 150 ]; do sleep 0.2; n=$((n + 1)); done
    alive "$PID" && ours "$PID" && kill -KILL "$PID" 2>/dev/null
  fi
  wait "$PID" 2>/dev/null
}

# shellcheck disable=SC2329  # invoked via the EXIT trap
cleanup() {
  for PID in $NODE_PIDS; do stop_one "$PID"; done
  [ -n "$S" ] || return 0
  [ "$GS027_KEEP" = 1 ] && echo "kept $S" && return 0
  case "$S" in */gs027.*) rm -rf "$S" ;; esac
}

start_node() {
  local idx="$1" dd="$S/n$1/data"
  # Nested data dir: node_key lives in its parent, so each node gets its own peer id.
  mkdir -p "$dd" "$S/n$idx/blk"
  : >"$dd/.env"
  local args=(--network devnet --data-dir "$dd" --log-level info run
    --chainspec "$S/chainspec.json"
    --p2p-port "${P2P[$idx]}" --rpc-port "${RPCP[$idx]}" --metrics-port "${MET[$idx]}"
    --no-auto-update --force-start --yes)
  [ "$idx" -lt "$P" ] && args+=(--producer --producer-key "$S/w$idx.json")
  [ "$idx" -gt 0 ] && args+=(--bootstrap "/ip4/127.0.0.1/tcp/${P2P[0]}")
  DOLI_INC_I_236_BLOCK_SIGNATURE_ACTIVATION_HEIGHT="$GS027_AH" \
  DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT="$GS027_AH" \
  DOLI_INC_I_181_PENDING_ROOT_ACTIVATION_HEIGHT=0 \
  DOLI_GENESIS_BLOCKS="$GS027_GENESIS_BLOCKS" \
  DOLI_BOOTSTRAP_BLOCKS="$GS027_BOOTSTRAP_BLOCKS" \
  DOLI_BLOCKS_PER_REWARD_EPOCH="$GS027_EPOCH_BLOCKS" \
    "$GS027_NODE_BIN" "${args[@]}" >>"$S/n$idx/node.log" 2>&1 &
  NODE_PID[$idx]=$!
  NODE_PIDS="$NODE_PIDS ${NODE_PID[$idx]}"
}

heights() { local i; for ((i = 0; i < N; i++)); do best_height "${RPCP[$i]}"; done; }
min_height() {
  local lo="" h i
  for ((i = 0; i < N; i++)); do
    h="$(best_height "${RPCP[$i]}")"
    case "$h" in '' | *[!0-9]*) return 1 ;; esac
    { [ -z "$lo" ] || [ "$h" -lt "$lo" ]; } && lo="$h"
  done
  echo "$lo"
}
hashes_at() { local i; for ((i = 0; i < N; i++)); do printf '%s ' "$(block_hash "${RPCP[$i]}" "$1")"; done; }
all_alive() { local i; for ((i = 0; i < N; i++)); do alive "${NODE_PID[$i]}" || return 1; done; }
# Nodes whose latest schedule line shows a G-Set of all GS027_PRODUCERS producers.
gset_ready() {
  local i k=0 last
  for ((i = 0; i < N; i++)); do
    last="$(grep -E 'Producer schedule (view|DIVERGENCE)' "$S/n$i/node.log" | tail -1)"
    case "$last" in *"(count=$P, source=gset)"* | *"] (count=$P) vs known="*) k=$((k + 1)) ;; esac
  done
  echo "$k"
}
# Stop the cluster once every producer persisted its own announcement, then write the
# union of the producer_gset.bin files (bincode Vec: u64 LE count + records) to every node.
seed_gset() {
  local i t0
  for ((i = 0; i < P; i++)); do
    t0="$(date +%s)"
    until [ -s "$S/n$i/data/producer_gset.bin" ]; do [ $(($(date +%s) - t0)) -ge 30 ] && return 1; sleep 1; done
  done
  for PID in $NODE_PIDS; do stop_one "$PID"; done
  NODE_PIDS=""
  python3 -I - "$S" "$N" "$P" <<'PY'
import struct, sys
s, n, p = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
count, body = 0, b""
for i in range(p):
    d = open("%s/n%d/data/producer_gset.bin" % (s, i), "rb").read()
    count, body = count + struct.unpack("<Q", d[:8])[0], body + d[8:]
for i in range(n):
    open("%s/n%d/data/producer_gset.bin" % (s, i), "wb").write(struct.pack("<Q", count) + body)
PY
}
# One poll of every node: supply (two equal reads) and slash residents.
sample() { python3 -I "$S/sample.py" "$S" "${RPCP[@]}" 2>/dev/null; }
# Most frequent sampled supply of node $1 at height $2.
supply_at() {
  awk -v h="$2" '$1 == "S" && $2 == h { c[$3]++ }
    END { b = 0; for (v in c) if (c[v] > b) { b = c[v]; m = v }; if (b) print m }' "$S/n$1/samples.txt" 2>/dev/null
}
max_of() { awk -v k="$2" '$1 == k && $2 > m { m = $2 } END { print m + 0 }' "$S/n$1/samples.txt" 2>/dev/null; }

# wait_height H MAX: poll (sampling) until every node is at height >= H; rc 1 on timeout/death.
wait_height() {
  local mh t0="$(date +%s)"
  while [ $(($(date +%s) - t0)) -lt "$2" ]; do
    sample
    mh="$(min_height)" && [ "$mh" -ge "$1" ] && return 0
    all_alive || return 1; sleep 2
  done
  return 1
}

# First block >= GS027_AH by a producer node that every node holds: publish its signed
# sibling. Sets TARGET_IDX TARGET_PK INJ_H INJ_SLOT OWN_HASH SIB_HASH BOND.
inject_sibling() {
  local t0 tip blk prod i idx tried=0 attempts=0 n raw out dial="" rc
  for ((i = 0; i < N; i++)); do dial="$dial,/ip4/127.0.0.1/tcp/${P2P[$i]}"; done
  t0="$(date +%s)"
  while [ $(($(date +%s) - t0)) -lt "$GS027_PHASE_MAX" ] && [ "$attempts" -lt 3 ]; do
    sample
    tip="$(best_height "${RPCP[0]}")"
    case "$tip" in '' | *[!0-9]*) tip=0 ;; esac
    { [ "$tip" -lt "$GS027_AH" ] || [ "$tip" -le "$tried" ]; } && { sleep 1; continue; }
    tried="$tip"
    blk="$(block_json "${RPCP[0]}" "$tip")"
    prod="$(printf '%s' "$blk" | jfield producer)"
    idx=-1
    for ((i = 0; i < P; i++)); do [ "${PK[$i]}" = "$prod" ] && idx="$i"; done
    [ "$idx" -ge 0 ] || continue
    OWN_HASH="$(printf '%s' "$blk" | jfield hash)"
    INJ_SLOT="$(printf '%s' "$blk" | jfield slot)"
    n=0
    until all_equal "$OWN_HASH" $(hashes_at "$tip") || [ "$n" -ge $((SLOT_DUR * 2)) ]; do sleep 0.5; n=$((n + 1)); done
    all_equal "$OWN_HASH" $(hashes_at "$tip") || continue
    BOND="$(producer_json "${RPCP[0]}" "$prod" | jfield bondAmount)"
    raw="$(block_raw "${RPCP[0]}" $((tip - 1)))"
    attempts=$((attempts + 1))
    out="$("$GS027_FORGE" --kind sibling --tip-b64 "$raw" --height "$tip" --slot "$INJ_SLOT" \
      --producer "$prod" --key-file "$S/w$idx.json" --chainspec "$S/chainspec.json" \
      --differ-from "$OWN_HASH" --genesis-time "$GEN_TS" --slot-duration "$SLOT_DUR" \
      --dial "${dial#,}" 2>>"$S/forge.log")"
    rc=$?
    SIB_HASH="$(printf '%s\n' "$out" | sed -n 's/^INJECTED .*hash=\([0-9a-f]*\).*/\1/p')"
    if [ "$rc" = 0 ] && [ -n "$SIB_HASH" ]; then
      TARGET_IDX="$idx" TARGET_PK="$prod" INJ_H="$tip"
      return 0
    fi
    echo "gs027: forge attempt $attempts at h=$tip rc=$rc (see forge.log)"
  done
  return 1
}

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
for ((i = 0; i < N; i++)); do
  P2P[$i]=$((GS027_P2P_BASE + i))
  RPCP[$i]=$((GS027_RPC_BASE + i))
  MET[$i]=$((GS027_MET_BASE + i))
  for p in "${P2P[$i]}" "${RPCP[$i]}" "${MET[$i]}"; do port_busy "$p" && skip "TCP port $p is busy"; done
  udp_busy $((${P2P[$i]} + 1)) && skip "UDP discovery port $((${P2P[$i]} + 1)) is busy"
done

# --- scratch cluster ------------------------------------------------------------
trap cleanup EXIT
trap 'exit 1' INT TERM
S="$(mktemp -d "$TMPROOT/gs027.XXXXXX")" || skip "mktemp failed"
cat >"$S/sample.py" <<'PY'
import json, sys, urllib.request
urllib.request.install_opener(urllib.request.build_opener(urllib.request.ProxyHandler({})))
def rpc(port, method, params=None):
    body = json.dumps({"jsonrpc": "2.0", "method": method, "params": params or {}, "id": 1}).encode()
    req = urllib.request.Request("http://127.0.0.1:%s" % port, data=body, headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=5) as r:
            return json.load(r).get("result")
    except Exception:
        return None
if sys.argv[1] == "scan":
    out, port, lo, hi = sys.argv[2], sys.argv[3], int(sys.argv[4]), int(sys.argv[5])
    for h in range(lo, hi + 1):
        b = rpc(port, "getBlockByHeight", {"height": h})
        if not isinstance(b, dict):
            continue
        b["transactions"] = [rpc(port, "getTransaction", {"hash": x}) or x for x in b.get("transactions", [])]
        json.dump({"result": b}, open("%s/%d.json" % (out, h), "w"))
        for t in b["transactions"]:
            if isinstance(t, dict) and t.get("txType") == "slash_producer":
                print(h, t.get("hash"))
    sys.exit(0)
for i, port in enumerate(sys.argv[2:]):
    a, b = rpc(port, "getChainStats"), rpc(port, "getChainStats")
    m = rpc(port, "getMempoolTransactions", {"limit": 500})
    with open("%s/n%d/samples.txt" % (sys.argv[1], i), "a") as f:
        if a and b and (a.get("height"), a.get("totalSupply")) == (b.get("height"), b.get("totalSupply")):
            f.write("S %s %s\n" % (a["height"], a["totalSupply"]))
        if isinstance(m, list):
            f.write("M %d\n" % sum(1 for t in m if t.get("txType") == "slashproducer"))
PY

for ((i = 0; i < P; i++)); do
  "$GS027_CLI" -w "$S/w$i.json" new -n "gs027n$i" >/dev/null 2>&1 || skip "wallet generation failed for producer $i"
  PK[$i]="$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["addresses"][0]["public_key"])' "$S/w$i.json" 2>/dev/null)"
  [ -n "${PK[$i]}" ] || skip "could not read pubkey for producer $i"
done
T_START="$(date +%s)"
GEN_TS=$((((T_START + GS027_GENESIS_DELAY) / SLOT_DUR + 1) * SLOT_DUR))
python3 -I - "$S/chainspec.json" "$GEN_TS" "$SLOT_DUR" "$REWARD" "$BOND_UNIT" "${PK[@]}" <<'PY' ||
import json, sys
a = sys.argv
spec = {"name": "GS-027 ephemeral devnet", "id": "ephemeral-devnet", "network": "Devnet",
        "genesis": {"timestamp": int(a[2]), "message": "GS-027 INC-I-255 ephemeral", "initial_reward": int(a[4])},
        "consensus": {"slot_duration": int(a[3]), "slots_per_epoch": 30, "bond_amount": int(a[5])},
        "genesis_producers": [{"name": "p%d" % i, "public_key": pk, "bond_count": 1} for i, pk in enumerate(a[6:])]}
open(a[1], "w").write(json.dumps(spec, indent=2))
PY
  skip "chainspec write failed"

echo "gs027: node=$("$GS027_NODE_BIN" --version 2>/dev/null) nodes=$N producers=$P ah=$GS027_AH epoch_blocks=$GS027_EPOCH_BLOCKS genesis_in=$((GEN_TS - T_START))s scratch=$S"
start_cluster() {
  start_node 0
  until [ -n "$(best_height "${RPCP[0]}")" ] || [ $(($(date +%s) - T_START)) -ge "$GS027_GENESIS_DELAY" ]; do
    alive "${NODE_PID[0]}" || skip "node 0 exited before RPC came up (see $S/n0/node.log)"; sleep 1
  done
  for ((i = 1; i < N; i++)); do start_node "$i"; sleep 1; done
}
start_cluster
seed_gset || skip "a producer did not persist its announcement within 30 s"
start_cluster
while [ "$(date +%s)" -lt $((GEN_TS - 3)) ]; do all_alive || skip "a node exited before genesis"; sleep 2; done
gready="$(gset_ready)"
echo "gs027: gset_ready=$gready/$N at genesis ($((GEN_TS - T_START))s after start)"
[ "$gready" = "$N" ] || skip "producer G-Set incomplete on some node before genesis ($gready/$N); raise GS027_GENESIS_DELAY"

wait_height 2 "$GS027_BOOT_MAX" || skip "cluster did not reach height 2 within ${GS027_BOOT_MAX}s of genesis or a node exited"
wait_height "$GS027_AH" "$GS027_PHASE_MAX" ||
  skip "cluster did not reach GS027_AH=$GS027_AH within ${GS027_PHASE_MAX}s or a node exited (heights: $(heights | tr '\n' ' '))"
mh="$(min_height)" || mh=1
echo "gs027: pre-injection height=$((mh - 1)) tips_equal=$(all_equal $(hashes_at $((mh - 1))) && echo 1 || echo 0)"

TARGET_IDX=-1 TARGET_PK="" INJ_H=0 INJ_SLOT=0 OWN_HASH="" SIB_HASH="" BOND=""
inject_sibling || skip "no producer block above GS027_AH held by every node could be paired with a sibling"
echo "gs027: injected target=${TARGET_PK:0:8} node=$TARGET_IDX height=$INJ_H slot=$INJ_SLOT own=${OWN_HASH:0:12} sibling=${SIB_HASH:0:12} bond=$BOND"
wait_height $((INJ_H + GS027_OBS_BLOCKS)) "$GS027_PHASE_MAX" ||
  echo "gs027: observation window ended early (heights: $(heights | tr '\n' ' '))"
sample

# --- evidence per node ------------------------------------------------------------
declare -a NSL BAD SH SX ST BURN IDS DET
for ((i = 0; i < N; i++)); do
  tip="$(best_height "${RPCP[$i]}")"
  case "$tip" in '' | *[!0-9]*) tip=0 ;; esac
  python3 -I "$S/sample.py" scan "$S/n$i/blk" "${RPCP[$i]}" 1 "$tip" >"$S/n$i/slashes.txt" 2>/dev/null
  NSL[$i]=0 BAD[$i]=0 BURN[$i]=0
  for f in "$S/n$i/blk/"*.json; do
    if c="$(count_slash_txs "$f")"; then NSL[$i]=$((NSL[i] + c)); else BAD[$i]=1; fi
  done
  [ "$tip" -gt 0 ] && [ "$(find "$S/n$i/blk" -name '*.json' | wc -l | tr -d ' ')" -eq "$tip" ] || BAD[$i]=1
  read -r "SH[$i]" "SX[$i]" _ <"$S/n$i/slashes.txt" || SH[$i]="" SX[$i]=""
  producer_json "${RPCP[$i]}" "$TARGET_PK" >"$S/n$i/producer.json"
  ST[$i]="$(producer_status "$S/n$i/producer.json" "$TARGET_PK")" || ST[$i]=none
  sh="${SH[$i]}"
  if [ -n "$sh" ] && [ -n "$BOND" ]; then
    hend="$(max_of "$i" S)"
    s0="$(supply_at "$i" $((sh - 1)))" s1="$(supply_at "$i" "$sh")" se="$(supply_at "$i" "$hend")"
    supply_delta_ok "$REWARD" "$BOND" "$s0" "$s1" &&
      supply_delta_ok $(((hend - sh + 1) * REWARD)) "$BOND" "$s0" "$se" && BURN[$i]=1
    echo "gs027: n$i supply h$((sh - 1))=${s0:-?} h$sh=${s1:-?} h$hend=${se:-?}"
  fi
  IDS[$i]="$(slash_ids_from_log "$S/n$i/node.log" | tr '\n' ' ')"
  DET[$i]="$(grep -c 'EQUIVOCATION DETECTED' "$S/n$i/node.log" 2>/dev/null)"
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
  "burned_once=$burned/$N bond=${BOND:-none} reward=$REWARD"
mh="$(min_height)" || mh=0
res gs027-tips-equal "$(all_equal $(hashes_at "$mh") && echo 1 || echo 0)" "height=$mh nodes=$N"
res gs027-production-continues "$([ -n "$SLASH_H" ] && [ "$mh" -ge $((SLASH_H + GS027_ADVANCE)) ] && echo 1 || echo 0)" \
  "common_tip=$mh slash_height=${SLASH_H:-none} need=+$GS027_ADVANCE"
res gs027-mempool-slash-bounded "$([ "$bounded" = "$N" ] && echo 1 || echo 0)" "bounded=$bounded/$N max_residents=$maxres"
union="$(printf '%s\n' "${IDS[@]}" | tr ' ' '\n' | sed '/^$/d' | sort -u)"
nunion="$(printf '%s\n' "$union" | sed '/^$/d' | wc -l | tr -d ' ')"
res gs027-one-shared-slash-id "$([ "$idsmall" = "$N" ] && [ "$nunion" = 1 ] && [ "$union" = "$SLASH_X" ] && echo 1 || echo 0)" \
  "per_node_le1=$idsmall/$N distinct=$nunion creators=${creators:-none}"

echo "METRIC SLASHED_NODES=$slashed_nodes/$N detected=$detected/$N target=${TARGET_PK:0:8} inj_height=$INJ_H slash_height=${SLASH_H:-none} bond=${BOND:-none} reward=$REWARD"
verdict; exit $?
