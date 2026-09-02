#!/usr/bin/env bash
# ============================================================================
# gauntlet-gs016.sh — GS-016 "finality-wedge operator escape" scenario (C-12).
#
# Sourced by scripts/gauntlet.sh. This is the RECORDED LIVE DRILL half of the
# C-12 obligation (specs/fork-lifecycle-architecture.md:283): a testnet recovery
# from `tip == finality AND 0 < gap < 50 AND sibling-exhausted`, ABOVE h=80,700,
# with NO snap sync and NO poison-arm rollback.
#
# WHAT IT REPLAYS. INC-I-190 wedged 13 of 27 nodes at `tip == finality`. The
# only exit the fleet had was LB-4: the BLOCK_POISON arm rolling back THROUGH
# the finality guard, unaudited and uncorroborated. The alternative was snap
# sync, which buys recovery with archival history (INC-I-190 left a permanent
# 314592-314640 body hole). INC-I-204 M4.1 replaces LB-4 with `forceReorgTo`:
# admin-gated, corroborated at >= 2/3 of LOCAL producer weight, eligibility-
# gated per block, expiring, single-shot. This scenario proves the replacement
# WORKS ON A LIVE FLEET — not that the code compiles.
#
# THE FLOOR IS THE TRAP. Below h=80,700 the local testnet still runs
# `plan_reorg`'s PRE-ACTIVATION branch (inc_i_147_activation_height), which
# mainnet no longer runs. A pass under that branch proves nothing about the
# path mainnet takes, so this scenario REFUSES to run below the floor rather
# than reporting a green it did not earn. Measured 2026-09-01: h=83,178.
#
# CHAIN-WRITING AND OPT-IN. Like GS-010 and GS-014 this scenario mutates a live
# node's chain — it retracts applied blocks on ONE wedged node and re-applies
# the fleet's branch. It runs only with `--gs016` AND GAUNTLET_GS016_CONFIRM=1,
# and refuses any network that is not testnet. It is STATE-NEUTRAL for the
# FLEET: the rescued node converges onto the branch every other node already
# holds. Nothing is submitted to the chain and no other node is touched.
#
# EVERY PRECONDITION IS A SKIP, NEVER A FAILURE. The live fleet runs v6.26.1,
# which predates `forceReorgTo` entirely, and a healthy fleet has no wedged
# node by definition. A run that cannot host the scenario reports SKIP with the
# measured reason. Manufacturing a wedge to satisfy the drill would mean
# deliberately forking a live testnet; that is not in scope for this scenario.
#
# CAPABILITY PROBE, NOT A VERSION STRING. Whether the fleet exposes the method
# is asked of the fleet: a deliberately malformed hash returns -32602 when the
# method exists and -32601 when it does not. It arms nothing either way.
#
# ASSERTIONS key off STRUCTURED telemetry and distinct-event phrases, never
# bare keywords: `[WEDGED] reason=finality_conflict` (the M3 terminal that IS
# this cell), `[FORCE_REORG] ... outcome=`, `[SNAP_SYNC] Applying snapshot`,
# `[BLOCK_POISON]`. The word "rollback" logs ~1/sec at depth 0 and is unusable.
#
# Env: GS016_MIN_HEIGHT (80700), GS016_LAND_TIMEOUT (180s), GS016_POLL_SECS (5),
#      GS016_GAP_MAX (50). Uses gauntlet.sh globals/helpers ($WORK, $LIVE,
#      $LOG_DIR, port_of, height_of, net_max_height, say, C_*,
#      FAIL_REASONS/SKIP_REASONS/INFO_REASONS).
# ============================================================================

GS016_MIN_HEIGHT="${GS016_MIN_HEIGHT:-80700}"
GS016_LAND_TIMEOUT="${GS016_LAND_TIMEOUT:-180}"
GS016_POLL_SECS="${GS016_POLL_SECS:-5}"
GS016_GAP_MAX="${GS016_GAP_MAX:-50}"

# ── helpers ─────────────────────────────────────────────────────────────────

# _gs016_rpc <port> <method> [params-json] — raw JSON-RPC POST, empty on failure.
_gs016_rpc() {
    local port="$1" method="$2" params="${3:-}"
    [ -n "$params" ] || params='{}'
    curl -sf --max-time 15 -X POST "http://127.0.0.1:$port" \
        -H 'Content-Type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$method\",\"params\":$params}" 2>/dev/null
}

# _gs016_jq <expr> — read stdin JSON, print `eval(expr)` on the parsed dict `d`.
_gs016_jq() {
    python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    v = ($1)
    print('' if v is None else v)
except Exception:
    pass" 2>/dev/null
}

# _gs016_tip <port> — 'height hash', or empty.
_gs016_tip() {
    _gs016_rpc "$1" getChainInfo \
        | _gs016_jq "'%s %s' % (d['result']['bestHeight'], d['result']['bestHash'])"
}

# _gs016_hash_at <port> <height> — canonical block hash at that height, or empty.
_gs016_hash_at() {
    _gs016_rpc "$1" getBlockByHeight "{\"height\":$2}" | _gs016_jq "d['result']['hash']"
}

# _gs016_missing <port> — verifyChainIntegrity missing height ranges, one per
# line as 'from-to'. Empty output means a complete store (or an unreachable node,
# which the caller distinguishes via the separate reachability check).
_gs016_missing() {
    _gs016_rpc "$1" verifyChainIntegrity | python3 -c "
import sys, json
try:
    r = json.load(sys.stdin)['result']
    for m in (r.get('missingRanges') or r.get('missing_ranges') or []):
        if isinstance(m, dict):
            print('%s-%s' % (m.get('from', m.get('start')), m.get('to', m.get('end'))))
        elif isinstance(m, (list, tuple)) and len(m) == 2:
            print('%s-%s' % (m[0], m[1]))
        else:
            print(str(m))
except Exception:
    pass" 2>/dev/null
}

# _gs016_has_method <port> — 0 if the node exposes forceReorgTo. Probes with a
# deliberately malformed hash: -32602 means the method exists and rejected the
# argument, -32601 means the build predates the method. Arms nothing either way.
_gs016_has_method() {
    local code
    code="$(_gs016_rpc "$1" forceReorgTo '{"hash":"not-a-hash"}' \
        | _gs016_jq "d.get('error', {}).get('code')")"
    [ "$code" = "-32602" ]
}

# _gs016_mark_offsets — per-node log byte offsets so the scan window covers ONLY
# what this drill produced.
_gs016_mark_offsets() {
    local name
    : > "$WORK/gs016_offsets.txt"
    for name in $LIVE; do
        printf '%s:%s\n' "$name" "$(wc -c < "$LOG_DIR/$name.log" 2>/dev/null || echo 0)" \
            >> "$WORK/gs016_offsets.txt"
    done
}

# _gs016_count <name> <extended-regex> — matches for a node since its offset.
# `grep -c` prints 0 AND exits 1 on no match, so swallow the status and
# hard-normalise, or every downstream arithmetic expansion dies.
_gs016_count() {
    local name="$1" re="$2" off n
    off="$(grep "^$name:" "$WORK/gs016_offsets.txt" 2>/dev/null | cut -d: -f2)"
    [ -z "$off" ] && off=0
    n="$(tail -c "+$((off + 1))" "$LOG_DIR/$name.log" 2>/dev/null | grep -acE "$re" || true)"
    n="$(printf '%s' "$n" | tr -dc '0-9')"
    echo "${n:-0}"
}

# _gs016_recent <name> <extended-regex> [bytes] — matches in the tail of a
# node's log, for evidence that PREDATES the drill (the wedge itself).
_gs016_recent() {
    local name="$1" re="$2" bytes="${3:-400000}" n
    n="$(tail -c "$bytes" "$LOG_DIR/$name.log" 2>/dev/null | grep -acE "$re" || true)"
    n="$(printf '%s' "$n" | tr -dc '0-9')"
    echo "${n:-0}"
}

# _gs016_find_wedged <netmax> — echo 'name port height gap' for the first node in
# the recorded cell: 0 < gap <= GS016_GAP_MAX AND a `[WEDGED] reason=
# finality_conflict` terminal in its recent log. That terminal is the M3
# classifier naming THIS cell (`local_height - 1 < finality` after the recovery
# ladder ran out of rungs), so it is the shape itself and not a proxy for it.
_gs016_find_wedged() {
    local netmax="$1" name p h gap
    for name in $LIVE; do
        p="$(port_of "$name")"
        h="$(height_of "$p")"
        [ -n "${h:-}" ] || continue
        gap=$(( netmax - h ))
        [ "$gap" -gt 0 ] || continue
        [ "$gap" -le "$GS016_GAP_MAX" ] || continue
        [ "$(_gs016_recent "$name" '\[WEDGED\] reason=finality_conflict')" -gt 0 ] || continue
        echo "$name $p $h $gap"
        return 0
    done
    return 1
}

# ── injection ───────────────────────────────────────────────────────────────
# gs016_inject — find a node in the recorded wedge cell, name the fleet's branch
# for it via forceReorgTo, and wait for the escape to land. Writes
# $WORK/gs016_*.txt and a gs016_injected marker. No-op (assertions SKIP) unless
# confirmed, above the floor, and hostable.
gs016_inject() {
    if [ "${GAUNTLET_GS016_CONFIRM:-0}" != "1" ]; then
        say "  ${C_Y}[gs016] GAUNTLET_GS016_CONFIRM=1 not set — SKIPPING the C-12 wedge-escape drill"
        say "         (mutates one live node's chain: retracts applied blocks and re-applies"
        say "          the fleet's branch)${C_0}"
        return 0
    fi

    # HARD SAFETY: chain-mutating scenario — testnet only, never mainnet.
    local first net
    first="$(echo "$LIVE" | awk '{print $1}')"
    net="$(_gs016_rpc "$(port_of "$first")" getChainInfo | _gs016_jq "d['result']['network']")"
    if [ "$net" != "testnet" ]; then
        say "  ${C_R}[gs016] network='${net:-unknown}' is not testnet — REFUSING (scenario mutates a chain)${C_0}"
        return 0
    fi

    # TRAP T10: below the floor the testnet runs plan_reorg's PRE-ACTIVATION
    # branch, which mainnet does not. A pass there would be a fake green.
    local netmax
    netmax="$(net_max_height)"
    if [ "${netmax:-0}" -lt "$GS016_MIN_HEIGHT" ]; then
        say "  ${C_R}[gs016] fleet tip h=${netmax:-0} is BELOW the h=${GS016_MIN_HEIGHT} floor (trap T10):"
        say "         the pre-activation plan_reorg branch mainnet no longer runs would decide"
        say "         this drill. REFUSING — a pass here would prove nothing.${C_0}"
        echo "below-floor $netmax $GS016_MIN_HEIGHT" > "$WORK/gs016_refused.txt"
        return 0
    fi

    # CAPABILITY: does this fleet even have the escape? (v6.26.1 does not.)
    if ! _gs016_has_method "$(port_of "$first")"; then
        say "  ${C_Y}[gs016] the fleet does not expose forceReorgTo (probe returned no -32602)."
        say "         This build predates INC-I-204 M4.1 — SKIPPING, not failing.${C_0}"
        echo "no-method" > "$WORK/gs016_refused.txt"
        return 0
    fi

    # THE CELL: a node at tip == finality, 0 < gap <= 50, sibling-exhausted.
    local found name port h gap
    if ! found="$(_gs016_find_wedged "$netmax")"; then
        say "  ${C_Y}[gs016] no node is in the recorded cell (0 < gap <= ${GS016_GAP_MAX} with a"
        say "         [WEDGED] reason=finality_conflict terminal). A healthy fleet has none by"
        say "         definition, and this scenario will not manufacture a fork on a live"
        say "         testnet to create one. SKIPPING.${C_0}"
        echo "no-wedged-node" > "$WORK/gs016_refused.txt"
        return 0
    fi
    name="$(echo "$found" | awk '{print $1}')"
    port="$(echo "$found" | awk '{print $2}')"
    h="$(echo "$found"    | awk '{print $3}')"
    gap="$(echo "$found"  | awk '{print $4}')"
    say "  ${C_C}[gs016] wedged node ${name} (rpc ${port}) at h=${h}, fleet at h=${netmax}, gap=${gap}${C_0}"

    # The operator names the FLEET's branch at a height the wedged node has not
    # reached. Read it from a node that is NOT the wedged one.
    local donor dp target_h target_hash=""
    target_h=$(( h + 1 ))
    for donor in $LIVE; do
        [ "$donor" = "$name" ] && continue
        dp="$(port_of "$donor")"
        target_hash="$(_gs016_hash_at "$dp" "$target_h")"
        [ -n "$target_hash" ] && break
    done
    if [ -z "$target_hash" ]; then
        say "  ${C_Y}[gs016] no healthy donor could serve the branch hash at h=${target_h} — SKIPPING.${C_0}"
        echo "no-donor" > "$WORK/gs016_refused.txt"
        return 0
    fi
    say "  [gs016] operator target: h=${target_h} hash=${target_hash:0:16}… (from ${donor})"

    # Baselines the assertions are a strict delta over.
    _gs016_missing "$port" | sort -u > "$WORK/gs016_missing_before.txt"
    printf '%s %s %s %s %s %s\n' "$name" "$port" "$h" "$gap" "$target_h" "$target_hash" \
        > "$WORK/gs016_target.txt"
    _gs016_mark_offsets

    local armed
    armed="$(_gs016_rpc "$port" forceReorgTo "{\"hash\":\"$target_hash\"}" \
        | _gs016_jq "d.get('result', {}).get('status') or ('error:' + str(d.get('error', {}).get('code')))")"
    say "  ${C_R}[gs016] armed forceReorgTo on ${name} -> ${armed:-unreachable}${C_0}"
    echo "${armed:-unreachable}" > "$WORK/gs016_armed.txt"
    if [ "$armed" != "armed" ]; then
        say "  ${C_Y}[gs016] the node did not accept the directive — nothing was mutated.${C_0}"
        touch "$WORK/gs016_injected"
        return 0
    fi

    # Wait for the escape to land. The directive RETAINS on unknown-target, so a
    # branch still in flight is not a refusal — that is the arm-ahead design.
    local waited=0 cur
    while [ "$waited" -lt "$GS016_LAND_TIMEOUT" ]; do
        cur="$(_gs016_hash_at "$port" "$target_h")"
        if [ "$cur" = "$target_hash" ]; then break; fi
        sleep "$GS016_POLL_SECS"; waited=$(( waited + GS016_POLL_SECS ))
    done
    cur="$(_gs016_hash_at "$port" "$target_h")"
    echo "${cur:-none} $waited" > "$WORK/gs016_landed.txt"
    _gs016_missing "$port" | sort -u > "$WORK/gs016_missing_after.txt"
    _gs016_tip "$port" > "$WORK/gs016_tip_after.txt"

    if [ "$cur" = "$target_hash" ]; then
        say "  ${C_G}[gs016] ${name} landed on the operator-named branch at h=${target_h} after ${waited}s${C_0}"
    else
        say "  ${C_R}[gs016] ${name} did NOT land within ${GS016_LAND_TIMEOUT}s (h=${target_h} holds ${cur:-nothing})${C_0}"
    fi
    touch "$WORK/gs016_injected"
}

# ── assertions ──────────────────────────────────────────────────────────────
# rc 0 pass · 1 fail · 2 skip. Appends to FAIL_REASONS / SKIP_REASONS /
# INFO_REASONS (gauntlet.sh scope).
_gs016_assert() {
    local t="$1" why name port target_h target_hash landed waited armed
    if [ ! -f "$WORK/gs016_injected" ]; then
        local r; r="$(cat "$WORK/gs016_refused.txt" 2>/dev/null)"
        SKIP_REASONS="$SKIP_REASONS; GS-016 not injected this run (${r:-needs --gs016 + GAUNTLET_GS016_CONFIRM=1}) — the C-12 live drill is opt-in, testnet-only, refuses below h=$GS016_MIN_HEIGHT (trap T10), and needs a node in the recorded wedge cell"
        return 2
    fi

    name="$(awk '{print $1}' "$WORK/gs016_target.txt" 2>/dev/null)"
    port="$(awk '{print $2}' "$WORK/gs016_target.txt" 2>/dev/null)"
    target_h="$(awk '{print $5}' "$WORK/gs016_target.txt" 2>/dev/null)"
    target_hash="$(awk '{print $6}' "$WORK/gs016_target.txt" 2>/dev/null)"
    landed="$(awk '{print $1}' "$WORK/gs016_landed.txt" 2>/dev/null)"
    waited="$(awk '{print $2}' "$WORK/gs016_landed.txt" 2>/dev/null)"
    armed="$(cat "$WORK/gs016_armed.txt" 2>/dev/null)"

    if [ "$armed" != "armed" ]; then
        SKIP_REASONS="$SKIP_REASONS; GS-016 the wedged node did not accept the directive (${armed:-unreachable}) — nothing was mutated, so no assertion has evidence"
        return 2
    fi

    local ok=1
    case "$t" in
      gs016-escape-lands-on-named-branch)
        # REQ-FORK-012 acceptance: a node at tip == finality recovers onto the
        # branch the OPERATOR named. Asserted at the named height rather than at
        # the tip, so normal forward sync after the rescue cannot mask it.
        if [ "$landed" = "$target_hash" ]; then ok=0
        else why="${name} h=${target_h} holds ${landed:-nothing}, expected ${target_hash:0:16}… after ${waited:-0}s"; fi ;;

      gs016-no-new-gap-after-escape)
        # REQ-FORK-011: recovery may never be bought with archival history.
        # A strict delta — a pre-existing hole is not this drill's doing.
        local new_ranges
        new_ranges="$(comm -13 "$WORK/gs016_missing_before.txt" "$WORK/gs016_missing_after.txt" 2>/dev/null | tr '\n' ' ')"
        new_ranges="$(printf '%s' "$new_ranges" | sed 's/ *$//')"
        if [ -z "$new_ranges" ]; then ok=0
        else why="verifyChainIntegrity on ${name} reports NEW missing ranges after the escape: ${new_ranges}"; fi ;;

      gs016-no-snap-sync-in-window)
        # Hard-list item 5: the escape must never reach for snap sync. Keyed on
        # the distinct apply event, not on the string "snap".
        local n; n="$(_gs016_count "$name" '\[SNAP_SYNC\] Applying snapshot')"
        if [ "${n:-0}" -eq 0 ]; then ok=0
        else why="${name} applied ${n} snapshot(s) during the escape window — the rescue destroyed history instead of retracting blocks"; fi ;;

      gs016-no-poison-bypass-in-window)
        # Hard-list item 6 / LB-4: the poison arm is what this milestone
        # REPLACES. If it ran, the escape is not what recovered the node.
        local n; n="$(_gs016_count "$name" '\[BLOCK_POISON\]')"
        if [ "${n:-0}" -eq 0 ]; then ok=0
        else why="${name} logged ${n} BLOCK_POISON event(s) during the escape window — LB-4 ran, so this drill did not prove its replacement"; fi ;;

      *)
        why="unknown gs016 assertion token '$t'" ;;
    esac

    if [ "$ok" -ne 0 ]; then FAIL_REASONS="$FAIL_REASONS; $t: $why"; fi
    return "$ok"
}
