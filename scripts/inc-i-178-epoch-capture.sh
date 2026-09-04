#!/usr/bin/env bash
# ============================================================================
# inc-i-178-epoch-capture.sh — REQ-BLS-009 / precondition P5 live epoch capture.
#
# Captures ONE reward epoch off the local 18-node testnet and writes the M6 replay
# fixture format `inc-i-178-m6-epoch-replay/1` that
# bins/node/tests/it/inc_i_178_m6_replay_fixture.rs loads. Read-only:
# getNetworkParams / getChainInfo / getBlockByHeight / getProducers /
# getAttestationStats only. It never writes to a node, never restarts anything,
# and touches nothing outside its own output file and its raw journal.
#
# The epoch length is READ from getNetworkParams.blocksPerRewardEpoch, never
# assumed: the constant is 360 but it is a per-network param and the local testnet
# runs 36. At 10 s/slot that is ~6 min there and ~60 min at 360.
#
# It back-fills the already-elapsed part of the current epoch from RPC, then polls
# forward one slot at a time to the epoch's last block, so the fixture covers the
# whole range even when the run starts mid-epoch.
#
# INDICES, NOT PUBKEYS. `producer`, `attester` and `parent_attesters` are indices
# into the epoch producer universe sorted by RAW PUBKEY BYTES — the universe order
# every consensus site uses. Pubkeys are journalled during the run and resolved to
# indices only when the fixture is written, so a producer appearing late cannot
# shift an index already emitted.
#
# TWO CAPTURE GAPS, STATED RATHER THAN FAKED:
#   1. `attendance` is NOT the minute tracker's snapshot. That structure is internal
#      to the node and no RPC exposes it. The closest obtainable source is the
#      per-poll growth of getAttestationStats.producers[].attestedMinutes, which
#      reconstructs WHICH producers gained a minute — not the tracker's full rows —
#      and it is bound to the tip block observed at that poll. Back-filled blocks
#      (those already on chain when the run started) carry an EMPTY attendance
#      array: the deltas are only observable live.
#   2. `parent_attesters` is ALWAYS EMPTY. It is the set holding a valid pooled BLS
#      signature over parent_hash; pre-AH that pool is not exposed by any RPC and no
#      aggregate exists at all (inc_i_178_attestation_bls_activation_height is
#      u64::MAX). Emitting invented bits would make a replay agree with itself.
#   A consumer must treat both fields as lower bounds, never as ground truth.
#
# The fixture is rewritten atomically every CAPTURE_FLUSH_POLLS polls, so a run
# killed at minute 50 still leaves a loadable file. It is written ONLY when at
# least one block was captured: the M6 loader panics on an empty `blocks` array,
# and a fixture that panics on load is worse than no fixture.
#
# Usage:  nohup bash scripts/inc-i-178-epoch-capture.sh \
#             > ~/testnet/logs/inc-i-178-epoch-capture.log 2>&1 &
# Env:    CAPTURE_OUT (default ~/testnet/inc-i-178-epoch-capture.json),
#         CAPTURE_PORTS, CAPTURE_POLL_SECS (10 = SLOT_DURATION),
#         CAPTURE_MAX_SECONDS (default 5400), CAPTURE_LAG (default 2 blocks below
#         the tip, so a block still being gossiped is not read twice),
#         CAPTURE_FLUSH_POLLS (default 6), CAPTURE_NETWORK (default testnet),
#         CAPTURE_BLOCKS_PER_EPOCH (override the value read from getNetworkParams).
# Exit:   0 a fixture with >= 1 block was written · 1 nothing capturable.
# ============================================================================
set -u

CAPTURE_OUT="${CAPTURE_OUT:-$HOME/testnet/inc-i-178-epoch-capture.json}"
CAPTURE_PORTS="${CAPTURE_PORTS:-8500 8501 8502 8503 8504 8505 8506 8507 8508 8509 8510 8511 8512 8513 8514 8515 8516 8517}"
CAPTURE_POLL_SECS="${CAPTURE_POLL_SECS:-10}"
CAPTURE_MAX_SECONDS="${CAPTURE_MAX_SECONDS:-5400}"
CAPTURE_LAG="${CAPTURE_LAG:-2}"
CAPTURE_FLUSH_POLLS="${CAPTURE_FLUSH_POLLS:-6}"
CAPTURE_NETWORK="${CAPTURE_NETWORK:-testnet}"
CAPTURE_TIMEOUT="${CAPTURE_TIMEOUT:-5}"
RAW="$CAPTURE_OUT.raw.jsonl"

log() { printf '%s %s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "$*"; }

_rpc() {
    local port="$1" method="$2" params="${3:-[]}"
    curl -sf --max-time "$CAPTURE_TIMEOUT" -X POST "http://127.0.0.1:$port" \
        -H 'Content-Type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$method\",\"params\":$params}" 2>/dev/null
}

# _rpc_any <method> [params] — first answering node wins; the reference port is
# tried first so a healthy run stays on one node.
_rpc_any() {
    local method="$1" params="${2:-[]}" p body
    for p in $REF_PORT $CAPTURE_PORTS; do
        [ -n "$p" ] || continue
        body="$(_rpc "$p" "$method" "$params")"
        if [ -n "$body" ]; then
            printf '%s' "$body"
            return 0
        fi
    done
    return 1
}

_scalar() { sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\{0,1\}\([^\",}]*\).*/\1/p" | sed -n '1p'; }

# ── preflight ───────────────────────────────────────────────────────────────

if ! python3 -c 'pass' >/dev/null 2>&1; then
    log "FATAL python3 is not usable — every reply is parsed with it"
    exit 1
fi

REF_PORT=""
for _p in $CAPTURE_PORTS; do
    _body="$(_rpc "$_p" getChainInfo '{}')"
    [ -n "$_body" ] || continue
    _net="$(printf '%s' "$_body" | _scalar network)"
    if [ -n "$_net" ] && [ "$_net" != "$CAPTURE_NETWORK" ]; then
        log "FATAL port $_p reports network=$_net — this capture is ${CAPTURE_NETWORK}-only"
        exit 1
    fi
    REF_PORT="$_p"
    break
done
if [ -z "$REF_PORT" ]; then
    log "FATAL no node answered getChainInfo on ports $CAPTURE_PORTS"
    exit 1
fi
log "reference node 127.0.0.1:$REF_PORT (network=$CAPTURE_NETWORK)"

mkdir -p "$(dirname "$CAPTURE_OUT")" 2>/dev/null
: > "$RAW" || { log "FATAL cannot write the raw journal at $RAW"; exit 1; }

STATS="$(_rpc_any getAttestationStats '{}')"
if [ -z "$STATS" ]; then
    log "FATAL getAttestationStats answered on no node — the epoch is not determinable"
    exit 1
fi
EPOCH="$(printf '%s' "$STATS" | _scalar epoch | tr -dc '0-9')"
TIP="$(printf '%s' "$STATS" | _scalar currentHeight | tr -dc '0-9')"
EPOCH_START="$(printf '%s' "$STATS" | _scalar epochStart | tr -dc '0-9')"
[ -n "$EPOCH" ] && [ -n "$TIP" ] && [ -n "$EPOCH_START" ] \
    || { log "FATAL could not read epoch/currentHeight/epochStart"; exit 1; }

# blocks_per_reward_epoch is a NETWORK PARAM, not the 360 constant: this testnet
# runs 36. Read it, then fall back to the epoch boundary the node just reported,
# then to the constant.
EPOCH_BLOCKS="${CAPTURE_BLOCKS_PER_EPOCH:-}"
if [ -z "$EPOCH_BLOCKS" ]; then
    EPOCH_BLOCKS="$(_rpc_any getNetworkParams '{}' | _scalar blocksPerRewardEpoch | tr -dc '0-9')"
fi
if [ -z "$EPOCH_BLOCKS" ] || [ "$EPOCH_BLOCKS" -eq 0 ] 2>/dev/null; then
    if [ "$EPOCH" -gt 0 ] && [ $(( EPOCH_START % EPOCH )) -eq 0 ]; then
        EPOCH_BLOCKS=$(( EPOCH_START / EPOCH ))
    else
        EPOCH_BLOCKS=360
    fi
    log "getNetworkParams gave no blocksPerRewardEpoch — derived $EPOCH_BLOCKS"
fi
EPOCH_END=$(( EPOCH_START + EPOCH_BLOCKS - 1 ))
log "epoch $EPOCH covers heights $EPOCH_START..$EPOCH_END ($EPOCH_BLOCKS blocks/epoch); tip is $TIP"

# ── producer universe ───────────────────────────────────────────────────────
# Journalled as pubkeys. Any block producer not in this list is added when the
# fixture is compiled, so no index can shift mid-run.

_universe_record() {
    python3 -c '
import sys, json
stats_raw, prods_raw, epoch = sys.argv[1], sys.argv[2], int(sys.argv[3])
keys = []
def add(k):
    k = str(k or "").lower()
    if k and k not in keys:
        keys.append(k)
for raw, path in ((stats_raw, "producers"), (prods_raw, "producers")):
    try:
        d = json.loads(raw)
        r = d.get("result", d) if isinstance(d, dict) else d
        ps = r.get(path, r) if isinstance(r, dict) else r
        for p in ps if isinstance(ps, list) else []:
            if isinstance(p, dict):
                add(p.get("publicKey") or p.get("public_key"))
    except Exception:
        pass
print(json.dumps({"t": "universe", "epoch": epoch, "producers": keys}))' \
        "$1" "$2" "$3" 2>/dev/null
}

PRODS="$(_rpc_any getProducers '{"active_only": false}')"
UNIVERSE="$(_universe_record "$STATS" "${PRODS:-{\}}" "$EPOCH")"
if [ -z "$UNIVERSE" ]; then
    log "FATAL could not build the producer universe"
    exit 1
fi
printf '%s\n' "$UNIVERSE" >> "$RAW"
log "universe: $(printf '%s' "$UNIVERSE" | python3 -c 'import sys,json;print(len(json.load(sys.stdin)["producers"]))' 2>/dev/null) producer(s)"

# ── block journalling ───────────────────────────────────────────────────────

# _block_records — stdin: one getBlockByHeight reply per line. stdout: one block
# record per readable reply. A reply missing a 64-hex prevHash is dropped rather
# than emitted: parse_hash() in the M6 loader panics on anything else.
_block_records() {
    python3 -c '
import sys, json, re
HEX = re.compile(r"^[0-9a-f]{64}$")
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        d = json.loads(line)
        r = d.get("result") or {}
        h = int(r["height"]); s = int(r.get("slot", h))
        prod = str(r.get("producer") or "").lower()
        par = str(r.get("prevHash") or "").lower()
        if not prod or not HEX.match(par):
            continue
        print(json.dumps({"t": "block", "height": h, "slot": s,
                          "producer": prod, "parent_hash": par}))
    except Exception:
        continue' 2>/dev/null
}

# _fetch_range <from> <to> — journal every readable block in the range.
_fetch_range() {
    local from="$1" to="$2" h
    [ "$from" -le "$to" ] || return 0
    h="$from"
    while [ "$h" -le "$to" ]; do
        _rpc_any getBlockByHeight "{\"height\":$h}" | tr -d '\n'
        printf '\n'
        h=$(( h + 1 ))
    done | _block_records >> "$RAW"
}

# _max_height — highest journalled block height, or empty.
_max_height() {
    python3 -c '
import sys, json
best = None
for line in sys.stdin:
    try:
        r = json.loads(line)
    except Exception:
        continue
    if r.get("t") == "block":
        h = r["height"]
        best = h if best is None or h > best else best
print("" if best is None else best)' < "$RAW" 2>/dev/null
}

# ── attendance reconstruction ───────────────────────────────────────────────
# Prints two lines: the new per-producer attestedMinutes state, then the
# attendance record for this poll (or `-`). Producers whose attestedMinutes GREW
# since the previous poll are the ones credited with the reported currentMinute.

_att_delta() {
    python3 -c '
import sys, json
prev_raw, tip = sys.argv[1], sys.argv[2]
prev = {}
for item in prev_raw.split():
    k, _, v = item.partition(":")
    if k:
        try:
            prev[k] = int(v)
        except ValueError:
            pass
try:
    d = json.load(sys.stdin)
    r = d.get("result", d)
    ps = r.get("producers") or []
    minute = int(r.get("currentMinute", 0))
except Exception:
    print(prev_raw); print("-"); sys.exit(0)
state, grew = [], []
for p in ps:
    if not isinstance(p, dict):
        continue
    k = str(p.get("publicKey") or "").lower()
    if not k:
        continue
    try:
        m = int(p.get("attestedMinutes", 0))
    except (TypeError, ValueError):
        continue
    state.append("%s:%d" % (k, m))
    if k in prev and m > prev[k]:
        grew.append(k)
print(" ".join(state))
if grew and tip.isdigit():
    print(json.dumps({"t": "att", "height": int(tip), "minute": minute,
                      "attesters": grew}))
else:
    print("-")' "$1" "$2" 2>/dev/null
}

# ── fixture compilation ─────────────────────────────────────────────────────
# Rebuilt from the journal every flush, then moved into place atomically.

_compile() {
    python3 -c '
import sys, json, os
raw, out, epoch = sys.argv[1], sys.argv[2], int(sys.argv[3])
universe, blocks, att = [], {}, {}
with open(raw) as fh:
    for line in fh:
        line = line.strip()
        if not line:
            continue
        try:
            r = json.loads(line)
        except Exception:
            continue
        t = r.get("t")
        if t == "universe":
            universe = [str(k).lower() for k in r.get("producers") or []]
        elif t == "block":
            blocks.setdefault(r["height"], r)
        elif t == "att":
            att.setdefault(r["height"], []).append(r)
keys = set(universe)
for b in blocks.values():
    keys.add(b["producer"])
for recs in att.values():
    for r in recs:
        keys.update(r.get("attesters") or [])
# Raw pubkey byte order == lowercase fixed-width hex order: the universe order
# every consensus site uses.
order = sorted(keys)
idx = {k: i for i, k in enumerate(order)}
out_blocks = []
for h in sorted(blocks):
    b = blocks[h]
    attendance = []
    for r in att.get(h, []):
        for k in r.get("attesters") or []:
            if k in idx:
                attendance.append({"attester": idx[k], "minute": int(r.get("minute", 0))})
    out_blocks.append({
        "height": b["height"],
        "slot": b["slot"],
        "producer": idx[b["producer"]],
        "parent_hash": b["parent_hash"],
        "attendance": attendance,
        # Pre-AH the pooled-BLS set is not exposed by any RPC and no aggregate
        # exists; an invented set would make a replay agree with itself.
        "parent_attesters": [],
    })
doc = {
    "format": "inc-i-178-m6-epoch-replay/1",
    "label": "testnet-epoch-%d" % epoch,
    "epoch": epoch,
    "producer_count": len(order),
    "blocks": out_blocks,
}
if not out_blocks or not order:
    print("0")
    sys.exit(0)
tmp = out + ".tmp"
with open(tmp, "w") as fh:
    json.dump(doc, fh, indent=2)
    fh.write("\n")
os.replace(tmp, out)
print(len(out_blocks))' "$RAW" "$CAPTURE_OUT" "$EPOCH" 2>/dev/null
}

# ── back-fill, then poll to the end of the epoch ────────────────────────────

BACKFILL_TO=$(( TIP - CAPTURE_LAG ))
[ "$BACKFILL_TO" -gt "$EPOCH_END" ] && BACKFILL_TO="$EPOCH_END"
if [ "$BACKFILL_TO" -ge "$EPOCH_START" ]; then
    log "back-filling heights $EPOCH_START..$BACKFILL_TO (attendance is empty for these: deltas are only observable live)"
    _fetch_range "$EPOCH_START" "$BACKFILL_TO"
fi
MAXH="$(_max_height)"
log "journalled up to height ${MAXH:-none}"

ATT_STATE=""
POLLS=0
STARTED=$SECONDS
while :; do
    if [ -n "$MAXH" ] && [ "$MAXH" -ge "$EPOCH_END" ]; then
        log "epoch $EPOCH complete at height $MAXH"
        break
    fi
    if [ $(( SECONDS - STARTED )) -ge "$CAPTURE_MAX_SECONDS" ]; then
        log "deadline reached after $(( SECONDS - STARTED ))s — compiling what was captured"
        break
    fi
    sleep "$CAPTURE_POLL_SECS"
    POLLS=$(( POLLS + 1 ))

    STATS="$(_rpc_any getAttestationStats '{}')"
    if [ -n "$STATS" ]; then
        DELTA="$(printf '%s' "$STATS" | _att_delta "$ATT_STATE" "${MAXH:-}")"
        NEW_STATE="$(printf '%s\n' "$DELTA" | sed -n '1p')"
        REC="$(printf '%s\n' "$DELTA" | sed -n '2p')"
        [ -n "$NEW_STATE" ] && ATT_STATE="$NEW_STATE"
        [ -n "$REC" ] && [ "$REC" != "-" ] && printf '%s\n' "$REC" >> "$RAW"
    fi

    CHAIN="$(_rpc_any getChainInfo '{}')"
    BEST="$(printf '%s' "${CHAIN:-}" | _scalar bestHeight | tr -dc '0-9')"
    if [ -n "$BEST" ]; then
        TARGET=$(( BEST - CAPTURE_LAG ))
        [ "$TARGET" -gt "$EPOCH_END" ] && TARGET="$EPOCH_END"
        FROM=$(( ${MAXH:-$(( EPOCH_START - 1 ))} + 1 ))
        [ "$FROM" -lt "$EPOCH_START" ] && FROM="$EPOCH_START"
        if [ "$FROM" -le "$TARGET" ]; then
            _fetch_range "$FROM" "$TARGET"
            MAXH="$(_max_height)"
        fi
    else
        log "no node answered getChainInfo this poll — retrying next slot"
    fi

    if [ $(( POLLS % CAPTURE_FLUSH_POLLS )) -eq 0 ]; then
        N="$(_compile)"
        log "poll $POLLS: height ${MAXH:-none}/$EPOCH_END, ${N:-0} block(s) in $CAPTURE_OUT"
    fi
done

N="$(_compile)"
N="${N:-0}"
if [ "$N" = "0" ]; then
    log "FAILED no block was capturable — refusing to write a fixture with an empty blocks array (the M6 loader panics on it)"
    exit 1
fi
log "wrote $CAPTURE_OUT — $N block(s), epoch $EPOCH ($EPOCH_START..$EPOCH_END)"
log "REMINDER parent_attesters is empty for every block (pre-AH, pool not exposed) and back-filled blocks carry no attendance"
exit 0
