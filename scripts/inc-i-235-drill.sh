#!/usr/bin/env bash
# inc-i-235-drill.sh — INC-I-235 live-testnet fork-walk drill (REQ-I235-014).
# Induces a 1-deep LOSING sibling on ONE non-seed producer of the LOCAL testnet
# and records how it converges. Run with --help for the contract.
set -uo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/inc-i-235-drill.sh --target nN [--dry-run] [--attempts N] [--timeout S] [--out FILE]
                                  [--isolate-gap N [--isolate-exec CMD]]

Induces a 1-deep losing sibling block on ONE non-seed testnet producer (nN) and
observes whether it converges back to the canonical tip WITHOUT snap sync.

  --target nN     non-seed node; must exist as $TESTNET_DIR/nN (default TESTNET_DIR=~/testnet)
  --dry-run       print the plan; send no mutating RPC
  --attempts N    induction attempts (default 3)
  --timeout S     convergence observation window in seconds (default 900)
  --out FILE      also write the evidence block to FILE
  --isolate-gap N after B' forms, stop the target (scripts/testnet.sh stop nN, launchd;
                  never a signal) until the seed is N blocks above B', then start it again.
                  N >= 50 closes Rule 1/1b (MINOR_FORK_GAP_MAX) = the INC-I-235 seed2 shape,
                  so only the Wedged terminal and its by-hash walk can converge the node.
  --isolate-exec CMD  run CMD (bash -c) while the target is stopped, e.g. a binary swap
                  for an old-vs-new control; the drill does not inspect what CMD does.

Env: DRILL_RPC_HOST (127.0.0.1 only), DRILL_RPC_BASE (default 8500; 8400/8900/30200 refused)

Steps: learn the producer rotation from [BLOCK_PRODUCED] log lines; enterRecoveryMode
on the target before the slot preceding its own slot (it drops block A);
exitRecoveryMode before its own slot (it builds sibling B' on A's parent);
pauseProduction right after B' (keeps the fork 1-deep); then observe.

RPC used: getChainInfo, getBlockByHeight (read); enterRecoveryMode, exitRecoveryMode,
pauseProduction, resumeProduction (target only; restored on every exit path).

Evidence captured (log lines after drill start only):
  [FORK] RECOVERY_START, [FORK] RECOVERY_DONE ... reorged, [WEDGED] reason=,
  [COORDINATOR] action=, [ORPHAN_CHASE], [ROLLBACK] Initiating, Reorg complete,
  and the ABSENCE of snap sync ([SNAP_SYNC] / action=SnapSync).
Exit 0 only when the target converges to the canonical hash with no snap sync.
EOF
}

TESTNET_DIR="${TESTNET_DIR:-$HOME/testnet}"
LOG_DIR="$TESTNET_DIR/logs"
HOST="${DRILL_RPC_HOST:-127.0.0.1}"
BASE="${DRILL_RPC_BASE:-8500}"
TARGET="" DRY=0 ATTEMPTS=3 TIMEOUT=900 OUT=""
ISO_GAP="" ISO_EXEC="" STOPPED=0
TESTNET_SH="$(cd "$(dirname "$0")" && pwd)/testnet.sh"
SLOT_SECS=10
MUTATED=0

die() { printf 'drill: %s\n' "$*" >&2; exit 2; }
refuse() { printf 'drill: refusing: %s\n' "$*" >&2; exit 2; }
note() { printf '[%s] %s\n' "$(date -u +%H:%M:%S)" "$*"; }

TARGET_SET=0
while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help) usage; exit 0 ;;
        --dry-run) DRY=1 ;;
        --target) TARGET="${2-}"; TARGET_SET=1; shift ;;
        --attempts) ATTEMPTS="${2-}"; shift ;;
        --timeout) TIMEOUT="${2-}"; shift ;;
        --out) OUT="${2-}"; shift ;;
        --isolate-gap) ISO_GAP="${2-}"; shift ;;
        --isolate-exec) ISO_EXEC="${2-}"; shift ;;
        *) die "unknown argument '$1' (see --help)" ;;
    esac
    shift
done

# ── preflight guards (run before any planning) ─────────────────────────────
case "$HOST" in
    127.0.0.1|localhost) ;;
    *) refuse "DRILL_RPC_HOST=$HOST is not loopback; this drill is local-testnet only" ;;
esac
[[ "$BASE" =~ ^[0-9]+$ ]] || refuse "DRILL_RPC_BASE=$BASE is not a port number"
case "$BASE" in
    8400|8900|30200) refuse "DRILL_RPC_BASE=$BASE is a mainnet port base" ;;
esac
[ "$TARGET_SET" = 1 ] || refuse "--target nN is required"
[ "$TARGET" != "seed" ] || refuse "--target seed: the seed is never drilled"
[[ "$TARGET" =~ ^n([1-9][0-9]?)$ ]] || refuse "--target '$TARGET' is not a producer name of the form nN"
TARGET_N="${BASH_REMATCH[1]}"
[ -d "$TESTNET_DIR/$TARGET" ] || refuse "--target $TARGET: no $TESTNET_DIR/$TARGET directory"
[[ "$ATTEMPTS" =~ ^[1-9][0-9]*$ ]] || die "--attempts must be a positive integer"
[[ "$TIMEOUT" =~ ^[1-9][0-9]*$ ]] || die "--timeout must be a positive integer"
[ -z "$ISO_GAP" ] || [[ "$ISO_GAP" =~ ^[1-9][0-9]*$ ]] || die "--isolate-gap must be a positive integer"
[ -z "$ISO_EXEC" ] || [ -n "$ISO_GAP" ] || die "--isolate-exec needs --isolate-gap"
TPORT=$((BASE + TARGET_N))
SPORT="$BASE"

if [ "$DRY" = 1 ]; then
    ISO_PLAN=""
    if [ -n "$ISO_GAP" ]; then
        ISO_PLAN="  7b. isolate: testnet.sh stop $TARGET (verify RPC down), wait until the seed is at
      gap >= $ISO_GAP above B'${ISO_EXEC:+, run --isolate-exec}, then testnet.sh start $TARGET,
      wait for RPC, pauseProduction again
"
    fi
    cat <<EOF
dry-run: INC-I-235 drill plan (nothing is sent)
  target           $TARGET  rpc http://$HOST:$TPORT  log $LOG_DIR/$TARGET.log
  canonical ref    seed     rpc http://$HOST:$SPORT
  1. probe live nodes with getChainInfo on $BASE..$((BASE + 30)); require network=testnet
  2. record log byte offsets (evidence window starts here)
  3. learn rotation from [BLOCK_PRODUCED]; pick S_t ($TARGET's next slot) and S_prev = S_t-1
  4. enterRecoveryMode on $TARGET ~1 s before S_prev starts (block A dropped)
  5. exitRecoveryMode ~9.5 s later, before S_t ($TARGET builds sibling B' on A's parent)
  6. pauseProduction on $TARGET once [BLOCK_PRODUCED] slot=S_t is logged
  7. verify B' hash differs from the seed's block at the same height (retry up to $ATTEMPTS)
${ISO_PLAN}  8. observe up to ${TIMEOUT}s for RECOVERY_START, RECOVERY_DONE ... reorged, [WEDGED],
     [COORDINATOR] action=, [ORPHAN_CHASE], [ROLLBACK] Initiating, Reorg complete;
     assert no [SNAP_SYNC] / SnapSync lines
  9. ALWAYS restore: exitRecoveryMode + resumeProduction on $TARGET
 10. print the evidence block and per-node height/hash convergence table${OUT:+ (also to $OUT)}
EOF
    exit 0
fi

command -v python3 >/dev/null || die "python3 is required"

# ── RPC helpers ─────────────────────────────────────────────────────────────
rpc() {
    local port="$1" method="$2" params="${3:-[]}"
    curl -s --max-time 4 -X POST -H 'content-type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$method\",\"params\":$params}" \
        "http://$HOST:$port" 2>/dev/null
}

# jfield JSON KEY — prints result[KEY] (empty on error).
jfield() {
    printf '%s' "$1" | python3 -c '
import sys, json
try:
    r = json.load(sys.stdin).get("result") or {}
    v = r.get(sys.argv[1], "") if isinstance(r, dict) else ""
    print(v)
except Exception:
    print("")' "$2"
}

tip() { local j; j="$(rpc "$1" getChainInfo '{}')"; printf '%s %s' "$(jfield "$j" bestHeight)" "$(jfield "$j" bestHash)"; }
hash_at() { jfield "$(rpc "$1" getBlockByHeight "{\"height\":$2}")" hash; }

mutate() {
    local method="$1" r
    r="$(rpc "$TPORT" "$method")"
    note "$TARGET $method -> ${r:-<no response>}"
}

restore() {
    if [ "$STOPPED" = 1 ]; then
        STOPPED=0
        note "$TARGET was stopped by the drill; starting it"
        bash "$TESTNET_SH" start "$TARGET" >/dev/null 2>&1
        wait_rpc 180 || note "$TARGET RPC not answering after start"
    fi
    [ "$MUTATED" = 1 ] || return 0
    MUTATED=0
    mutate exitRecoveryMode
    mutate resumeProduction
}
trap restore EXIT
trap 'restore; exit 130' INT TERM

# wait_rpc SECS — 0 once the target answers getChainInfo.
wait_rpc() {
    local end=$(( $(date +%s) + $1 ))
    while [ "$(date +%s)" -lt "$end" ]; do
        [ -n "$(jfield "$(rpc "$TPORT" getChainInfo '{}')" bestHeight)" ] && return 0
        sleep 3
    done
    return 1
}

# ── live fleet ──────────────────────────────────────────────────────────────
NODES=() PORTS=()
port_of() { if [ "$1" = seed ]; then echo "$BASE"; else echo $((BASE + ${1#n})); fi; }
for i in $(seq 0 30); do
    p=$((BASE + i)); name="n$i"; [ "$i" = 0 ] && name=seed
    j="$(rpc "$p" getChainInfo '{}')"
    [ -n "$(jfield "$j" bestHeight)" ] || continue
    net="$(jfield "$j" network)"
    [ "$net" = testnet ] || refuse "port $p reports network='$net', not testnet"
    NODES+=("$name"); PORTS+=("$p")
done
[ "${#NODES[@]}" -ge 3 ] || die "need >= 3 live nodes, found ${#NODES[@]}"
printf ' %s ' "${NODES[@]}" | grep -q " seed " || die "seed (port $SPORT) is not answering"
printf ' %s ' "${NODES[@]}" | grep -q " $TARGET " || die "$TARGET (port $TPORT) is not answering"
[ -f "$LOG_DIR/$TARGET.log" ] || die "no log $LOG_DIR/$TARGET.log"
note "live nodes: ${NODES[*]}"

OFFS=()
for n in "${NODES[@]}"; do
    f="$LOG_DIR/$n.log"; o=0
    [ -f "$f" ] && o="$(wc -c < "$f" | tr -d ' ')"
    OFFS+=("$n:$o")
done
offset_of() { local e; for e in "${OFFS[@]}"; do [ "${e%%:*}" = "$1" ] && { echo "${e#*:}"; return; }; done; echo 0; }
START_TS="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

# window NODE — log text written since drill start, ANSI stripped.
window() {
    local f="$LOG_DIR/$1.log"
    [ -f "$f" ] || return 0
    tail -c +$(( $(offset_of "$1") + 1 )) "$f" | sed $'s/\x1b\\[[0-9;]*m//g'
}
window_has() { window "$1" | grep -E "$2" >/dev/null; }

# ── rotation ────────────────────────────────────────────────────────────────
# Prints "S_prev S_t PRODUCER_PREV GENESIS_TS" for the target, or nothing.
plan_slots() {
    local n
    for n in "${NODES[@]}"; do
        [ -f "$LOG_DIR/$n.log" ] || continue
        tail -n 20000 "$LOG_DIR/$n.log" | sed $'s/\x1b\\[[0-9;]*m//g' \
            | grep -F '[BLOCK_PRODUCED]' | tail -n 200 | sed "s/^/$n /"
    done | python3 -c '
import sys, re, time, datetime
target, slot_secs = sys.argv[1], int(sys.argv[2])
by_slot, gen = {}, []
for line in sys.stdin:
    m = re.match(r"(\S+) (\S+Z) .*slot=(\d+)", line)
    if not m:
        continue
    node, ts, slot = m.group(1), m.group(2), int(m.group(3))
    t = datetime.datetime.strptime(ts[:19], "%Y-%m-%dT%H:%M:%S").replace(tzinfo=datetime.timezone.utc).timestamp()
    by_slot[slot] = node
    gen.append(int(t) - slot * slot_secs)
if not gen:
    sys.exit(0)
g = min(gen)
mine = sorted(s for s, n in by_slot.items() if n == target)
if len(mine) < 2:
    sys.exit(0)
period = min(b - a for a, b in zip(mine, mine[1:]) if b > a)
now_slot = (int(time.time()) - g) // slot_secs
s_t = mine[-1]
for _ in range(50):
    s_t += period
    if s_t - 1 <= now_slot + 1:
        continue
    prev = by_slot.get(s_t - 1 - period)
    if prev and prev != target:
        print(s_t - 1, s_t, prev, g)
        break
' "$TARGET" "$SLOT_SECS"
}

sleep_until() {
    python3 -c 'import sys,time; d=float(sys.argv[1])-time.time(); time.sleep(d) if d>0 else None' "$1"
}

# ── induction ───────────────────────────────────────────────────────────────
SIB_HASH="" SIB_HEIGHT="" SIB_SLOT="" CANON_HASH="" PREV_PRODUCER=""
for attempt in $(seq 1 "$ATTEMPTS"); do
    plan="$(plan_slots)"
    if [ -z "$plan" ]; then note "attempt $attempt: rotation unknown for $TARGET; waiting 30s"; sleep 30; continue; fi
    read -r S_PREV S_T PREV_PRODUCER GEN <<<"$plan"
    note "attempt $attempt: S_prev=$S_PREV ($PREV_PRODUCER) S_t=$S_T ($TARGET) genesis_ts=$GEN"
    sleep_until "$((GEN + S_PREV * SLOT_SECS - 1))"
    MUTATED=1
    mutate enterRecoveryMode
    sleep_until "$(python3 -c "print($GEN + $S_T * $SLOT_SECS - 0.5)")"
    mutate exitRecoveryMode
    line=""
    for _ in $(seq 1 100); do
        line="$(window "$TARGET" | grep -F '[BLOCK_PRODUCED]' | grep -E "slot=$S_T( |$)" | tail -n 1)"
        [ -n "$line" ] && break
        sleep 0.2
    done
    if [ -n "$line" ]; then
        mutate pauseProduction
    fi
    if [ -z "$line" ] || window_has "$TARGET" "slot=$S_T BLOCKED: Syncing"; then
        note "attempt $attempt: no block for slot $S_T on $TARGET; restoring and retrying"
        restore; sleep 20; continue
    fi
    SIB_HASH="$(sed -n 's/.*hash=\([0-9a-f]*\).*/\1/p' <<<"$line")"
    SIB_HEIGHT="$(sed -n 's/.*height=\([0-9]*\).*/\1/p' <<<"$line")"
    SIB_SLOT="$S_T"
    sleep 4
    CANON_HASH="$(hash_at "$SPORT" "$SIB_HEIGHT")"
    if [ -n "$CANON_HASH" ] && [ "$CANON_HASH" != "$SIB_HASH" ]; then
        note "sibling formed: $TARGET h=$SIB_HEIGHT B'=${SIB_HASH:0:16} vs seed ${CANON_HASH:0:16}"
        break
    fi
    note "attempt $attempt: no sibling (seed h=$SIB_HEIGHT hash=${CANON_HASH:-none}); restoring and retrying"
    SIB_HASH=""; restore; sleep 20
done

# ── isolation (gap >= N: Rule 1/1b closed) ──────────────────────────────────
ISO_REPORT=""
if [ -n "$SIB_HASH" ] && [ -n "$ISO_GAP" ]; then
    note "isolating $TARGET: testnet.sh stop $TARGET"
    STOPPED=1
    bash "$TESTNET_SH" stop "$TARGET" >/dev/null 2>&1
    down=0
    for _ in $(seq 1 30); do
        [ -z "$(jfield "$(rpc "$TPORT" getChainInfo '{}')" bestHeight)" ] && { down=1; break; }
        sleep 2
    done
    [ "$down" = 1 ] || { note "FAILED: $TARGET still answers RPC after stop"; exit 1; }
    ISO_T0="$(date -u +%H:%M:%SZ)"
    note "$TARGET stopped at $ISO_T0; waiting for seed gap >= $ISO_GAP above h=$SIB_HEIGHT"
    iso_end=$(( $(date +%s) + ISO_GAP * 20 + 300 )) gap=0
    while [ "$(date +%s)" -lt "$iso_end" ]; do
        read -r sH _ <<<"$(tip "$SPORT")"
        [ -n "$sH" ] && gap=$(( sH - SIB_HEIGHT ))
        [ "$gap" -ge "$ISO_GAP" ] && break
        sleep 10
    done
    note "isolation end: seed h=$sH gap=$gap (B' at h=$SIB_HEIGHT)"
    if [ -n "$ISO_EXEC" ]; then
        note "isolate-exec: $ISO_EXEC"
        bash -c "$ISO_EXEC"; note "isolate-exec rc=$?"
    fi
    bash "$TESTNET_SH" start "$TARGET" >/dev/null 2>&1
    STOPPED=0
    ISO_T1="$(date -u +%H:%M:%SZ)"
    wait_rpc 180 || { note "FAILED: $TARGET RPC not up 180s after start"; exit 1; }
    mutate pauseProduction
    read -r tH tHash <<<"$(tip "$TPORT")"
    ISO_REPORT="isolation: stopped=$ISO_T0 started=$ISO_T1 gap_at_start=$gap target_tip=$tH ${tHash:0:16}"
    note "$ISO_REPORT"
    if [ "$tHash" != "$SIB_HASH" ]; then note "WARNING: target tip is not B' after restart"; fi
fi

# ── observation ─────────────────────────────────────────────────────────────
CONVERGED=0 CONV_BY="none" SNAP=0 CONV_TS=""
if [ -n "$SIB_HASH" ]; then
    deadline=$(( $(date +%s) + TIMEOUT ))
    while [ "$(date +%s)" -lt "$deadline" ]; do
        if window_has "$TARGET" '\[SNAP_SYNC\]|SnapSync'; then SNAP=1; break; fi
        th="$(hash_at "$TPORT" "$SIB_HEIGHT")"; sh="$(hash_at "$SPORT" "$SIB_HEIGHT")"
        read -r tH tHash <<<"$(tip "$TPORT")"; read -r sH _ <<<"$(tip "$SPORT")"
        if [ -n "$th" ] && [ "$th" = "$sh" ] && [ "$th" != "$SIB_HASH" ] \
            && [ -n "$tH" ] && [ -n "$sH" ] && [ $((sH - tH)) -le 1 ] \
            && [ "$(hash_at "$SPORT" "$tH")" = "$tHash" ]; then
            CONVERGED=1; CONV_TS="$(date -u +%Y-%m-%dT%H:%M:%SZ)"; break
        fi
        sleep 5
    done
    if window_has "$TARGET" '\[FORK\] RECOVERY_DONE.*reorged'; then CONV_BY="RECOVERY_DONE reorged"
    elif window_has "$TARGET" '\[ROLLBACK\] Initiating|Reorg complete'; then CONV_BY="pre-existing rollback/reorg path"
    fi
fi
restore

# ── evidence ────────────────────────────────────────────────────────────────
evidence() {
    local n p h hash ref
    echo "━━━ INC-I-235 DRILL EVIDENCE"
    echo "start=$START_TS end=$(date -u +%Y-%m-%dT%H:%M:%SZ) target=$TARGET prev_producer=${PREV_PRODUCER:-?}"
    echo "sibling: height=${SIB_HEIGHT:-none} slot=${SIB_SLOT:-none} hash=${SIB_HASH:-none} canonical=${CANON_HASH:-none}"
    echo "converged=$CONVERGED at=${CONV_TS:-never} by=$CONV_BY snap_sync_in_window=$SNAP"
    [ -z "$ISO_REPORT" ] || echo "$ISO_REPORT"
    echo "-- $TARGET log lines in window"
    window "$TARGET" | grep -E '\[FORK\] RECOVERY_START|\[FORK\] RECOVERY_DONE|\[FORK\] RECOVERY_SKIP|\[FORK\] Walked block|BLOCKED: Sync|\[WEDGED\] reason=|\[COORDINATOR\] action=|\[ORPHAN_CHASE\]|\[ROLLBACK\] Initiating|Reorg complete|\[SNAP_SYNC\]|SnapSync' | tail -n 60
    echo "-- convergence table"
    printf '%-6s %-8s %-18s %s\n' node height hash "matches-seed-at-height"
    for n in "${NODES[@]}"; do
        p="$(port_of "$n")"
        read -r h hash <<<"$(tip "$p")"
        if [ -n "${h:-}" ] && [ "$(hash_at "$SPORT" "$h")" = "$hash" ]; then ref=yes; else ref=no; fi
        printf '%-6s %-8s %-18s %s\n' "$n" "${h:--}" "${hash:0:16}" "$ref"
    done
}
if [ -n "$OUT" ]; then evidence | tee "$OUT"; else evidence; fi

if [ -z "$SIB_HASH" ]; then note "FAILED: no sibling induced in $ATTEMPTS attempts"; exit 1; fi
if [ "$SNAP" = 1 ]; then note "FAILED: snap sync observed on $TARGET"; exit 1; fi
if [ "$CONVERGED" != 1 ]; then note "FAILED: $TARGET did not converge within ${TIMEOUT}s"; exit 1; fi
note "OK: $TARGET converged by $CONV_BY with no snap sync"
exit 0
