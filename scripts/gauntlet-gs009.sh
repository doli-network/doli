#!/usr/bin/env bash
# ============================================================================
# gauntlet-gs009.sh — GS-009 "fleet rolling-restart" scenario for the gauntlet.
#
# Sourced by scripts/gauntlet.sh. Replays INC-I-143 (lineage INC-I-062 →
# INC-I-075): a tight-wave restart of ALL producers put multiple scheduled slot
# leaders simultaneously behind STARTUP_GATE → 34-slot production stall →
# genuine sibling fork at h=108456 → permanent INTEGRITY −1.
#
# PERTURBATIVE + OPT-IN: like --chaos, this genuinely stops/starts nodes. It is
# NOT part of the default observational run. It arms with `--gs009` AND requires
# GAUNTLET_GS009_CONFIRM=1. It restarts ONLY producers (n1..n12) — NEVER the
# seed — via scripts/testnet.sh stop/start (launchd-managed; NEVER pkill/kill,
# whose launchd respawn causes splits). On any non-injected run the three
# GS-009 assertions SKIP (rc=2), so the scenario never spuriously fails a gate.
#
# PASS criteria (asserted over the post-restart monitoring window):
#   gs009-no-stall        — no production stall longer than GS009_STALL_MAX_SLOTS
#                           (default 6 slots; env-overridable).
#   gs009-no-sibling-fork — no observed height with >=2 distinct block hashes
#                           across producers (sibling-fork check via RPC).
#   gs009-fleet-rejoin    — every producer rejoins within 1 of the canonical tip.
#
# Env: GS009_STALL_MAX_SLOTS (6), GS009_SLOT_SECS (10), GS009_MONITOR_WINDOW
#      (120s), GS009_SAMPLE_SECS (5s). Uses gauntlet.sh globals/helpers
#      (LIVE, WORK, ROOT, say, C_*, port_of, height_of, net_max_height).
# ============================================================================

# _gs009_hash_at <port> <height> — block hash at height via RPC, or "".
_gs009_hash_at() {
    curl -sf -m 2 -X POST "http://127.0.0.1:$1" -H 'Content-Type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"getBlockByHeight\",\"params\":{\"height\":$2},\"id\":1}" 2>/dev/null \
    | python3 -c "import sys,json
try: print(json.load(sys.stdin)['result']['hash'])
except Exception: print('')" 2>/dev/null
}

# _gs009_siblings_at <height> — count of DISTINCT non-empty block hashes reported
# by producer nodes at <height>. >=2 means a sibling fork at that height.
_gs009_siblings_at() {
    local hgt="$1" name p hash seen n
    seen=""; n=0
    for name in $LIVE; do
        case "$name" in n[0-9]|n[0-9][0-9]) ;; *) continue ;; esac
        p="$(port_of "$name")"
        hash="$(_gs009_hash_at "$p" "$hgt")"
        [ -z "$hash" ] && continue
        case " $seen " in *" $hash "*) ;; *) seen="$seen $hash"; n=$(( n + 1 )) ;; esac
    done
    echo "$n"
}

# gs009_inject — the fleet rolling-restart perturbation + its monitoring window.
# Writes $WORK/gs009_{max_stall,siblings,notrejoined}.txt and a gs009_injected
# marker. No-op (assertions SKIP) unless GAUNTLET_GS009_CONFIRM=1.
gs009_inject() {
    if [ "${GAUNTLET_GS009_CONFIRM:-0}" != "1" ]; then
        say "  ${C_Y}[gs009] GAUNTLET_GS009_CONFIRM=1 not set — SKIPPING fleet restart"
        say "         (perturbative + opt-in like --chaos; GS-009 assertions will SKIP)${C_0}"
        return 0
    fi
    local producers n
    producers=$(echo "$LIVE" | tr ' ' '\n' | grep -E '^n[0-9]+$' | tr '\n' ' ')
    producers=$(echo "$producers" | xargs || true)
    if [ -z "$producers" ]; then
        say "  ${C_R}[gs009] no producer nodes (nN) live — cannot inject${C_0}"
        return 0
    fi
    # HARD SAFETY: the seed is NEVER in $producers (grep excludes it). Restarting
    # the seed would remove the only stable DNS bootstrap and poison recovery.
    say "  ${C_Y}[gs009] tight-wave restart of producers: $producers${C_0}"
    for n in $producers; do "$ROOT/scripts/testnet.sh" stop  "$n" >/dev/null 2>&1; done
    for n in $producers; do "$ROOT/scripts/testnet.sh" start "$n" >/dev/null 2>&1; done

    # ── monitor: sample net tip; track max consecutive stall + sibling forks ──
    local win="${GS009_MONITOR_WINDOW:-120}" step="${GS009_SAMPLE_SECS:-5}" slot="${GS009_SLOT_SECS:-10}"
    local t0 now h last_h last_change stall max_stall sib total_sib
    t0=$(date +%s); last_change=$t0; max_stall=0; total_sib=0
    last_h="$(net_max_height)"
    while [ $(( $(date +%s) - t0 )) -lt "$win" ]; do
        sleep "$step"
        h="$(net_max_height)"; now=$(date +%s)
        if [ "${h:-0}" -gt "${last_h:-0}" ]; then last_h="$h"; last_change="$now"; fi
        stall=$(( (now - last_change) / slot ))
        [ "$stall" -gt "$max_stall" ] && max_stall="$stall"
        sib="$(_gs009_siblings_at "$(( ${last_h:-1} ))")"
        [ "${sib:-0}" -ge 2 ] && total_sib=$(( total_sib + 1 ))
    done
    echo "$max_stall" > "$WORK/gs009_max_stall.txt"
    echo "$total_sib" > "$WORK/gs009_siblings.txt"

    # ── rejoin: each producer within 1 of the canonical tip ──
    local tip notrejoined=0
    tip="$(net_max_height)"
    for n in $producers; do
        h="$(height_of "$(port_of "$n")")"
        if [ -z "$h" ] || [ "$h" = "-1" ] || [ "${h:-0}" -lt $(( tip - 1 )) ]; then
            notrejoined=$(( notrejoined + 1 ))
        fi
    done
    echo "$notrejoined" > "$WORK/gs009_notrejoined.txt"
    touch "$WORK/gs009_injected"
    say "  [gs009] max_stall=${max_stall} slots · sibling-fork samples=${total_sib} · not-rejoined=${notrejoined}"
}

# _gs009_assert <token> — evaluate one GS-009 assertion. rc 0 pass · 1 fail ·
# 2 skip. Appends to FAIL_REASONS / SKIP_REASONS (gauntlet.sh scope), matching
# the format of gauntlet.sh's own assert().
_gs009_assert() {
    local t="$1" why m s r max
    if [ ! -f "$WORK/gs009_injected" ]; then
        why="GS-009 not injected this run (needs --gs009 + GAUNTLET_GS009_CONFIRM=1) — perturbative, opt-in"
        SKIP_REASONS="$SKIP_REASONS; $why"; return 2
    fi
    case "$t" in
        gs009-no-stall)
            m=$(cat "$WORK/gs009_max_stall.txt" 2>/dev/null); max="${GS009_STALL_MAX_SLOTS:-6}"
            if [ "${m:-999}" -le "$max" ]; then return 0
            else FAIL_REASONS="$FAIL_REASONS; $t: production stalled ${m} slots (max ${max}) after fleet restart"; return 1; fi ;;
        gs009-no-sibling-fork)
            s=$(cat "$WORK/gs009_siblings.txt" 2>/dev/null)
            if [ "${s:-1}" -eq 0 ]; then return 0
            else FAIL_REASONS="$FAIL_REASONS; $t: ${s} sample(s) saw >=2 distinct block hashes at one height (sibling fork)"; return 1; fi ;;
        gs009-fleet-rejoin)
            r=$(cat "$WORK/gs009_notrejoined.txt" 2>/dev/null)
            if [ "${r:-1}" -eq 0 ]; then return 0
            else FAIL_REASONS="$FAIL_REASONS; $t: ${r} producer(s) did not rejoin canonical tip"; return 1; fi ;;
        *)
            FAIL_REASONS="$FAIL_REASONS; $t: unknown gs009 token"; return 1 ;;
    esac
}
