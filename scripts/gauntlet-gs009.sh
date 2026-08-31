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
# whose launchd respawn causes splits). On any non-injected run ALL FOUR
# GS-009 assertions SKIP (rc=2), so the scenario never spuriously fails a gate.
#
# PASS criteria (asserted over the post-restart monitoring window):
#   gs009-no-stall        — no production stall longer than GS009_STALL_MAX_SLOTS
#                           (default 6 slots; env-overridable).
#   gs009-no-sibling-fork — no observed height with >=2 distinct block hashes
#                           across producers (sibling-fork check via RPC).
#   gs009-fleet-rejoin    — every producer rejoins within 1 of the canonical tip.
#   gs009-trust-root-provenance
#                         — INC-I-196: every producer that resolved an OnChain
#                           release trust root must still resolve a USABLE one
#                           after the restart (provenance OnChain, usable true,
#                           keys >= threshold). Reads getUpdateStatus, which is
#                           wired to node_updater::resolve_trust_root — the
#                           actual verification decision — NOT getMaintainerSet,
#                           which reports only the persisted state the bug left
#                           intact. A PROPERTY, never before==after: a legitimate
#                           maintainer rotation changes the key set on purpose
#                           (the mainnet rotation did) and must not fail. SKIPS
#                           when no
#                           producer held an OnChain root, or when no eligible
#                           producer answers afterwards — a scenario that cannot
#                           be satisfied on the local fleet is the GS-011 trap.
#                           NOTE ON SCOPE, so this never over-claims: for a
#                           RUNNING node INC-I-196 was latent (resolution is a
#                           lazy closure consumed by the update service), so the
#                           restart is not the trigger. This asserts the property
#                           holds, at a moment GS-009 already creates.
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

# _gs009_trust_root_at <port> — the RESOLVED release trust root as
# "provenance|keys|threshold|usable", or "" when the RPC does not answer or reports no
# root. MUST be getUpdateStatus, NOT getMaintainerSet: getMaintainerSet returns the
# PERSISTED MaintainerState, but INC-I-196 lived in TrustRoot::resolve, a pure function
# that CONSUMES that state without mutating it. Under the bug the persisted set was
# intact and correctly rotated while resolve() returned an empty root, so the
# getMaintainerSet tuple was bit-identical on a fully bricked host. getUpdateStatus is
# wired straight to node_updater::resolve_trust_root (bins/node/src/node/startup.rs:472)
# and publishes provenance/keys/threshold/usable — the actual verification decision.
_gs009_trust_root_at() {
    curl -sf -m 3 -X POST "http://127.0.0.1:$1" -H 'Content-Type: application/json' \
        -d '{"jsonrpc":"2.0","method":"getUpdateStatus","params":{},"id":1}' 2>/dev/null \
    | python3 -c "import sys,json
try:
    t = json.load(sys.stdin)['result']['trust_root']
    if not t: raise ValueError
    print('%s|%s|%s|%s' % (t['provenance'], t['keys'], t['threshold'], t['usable']))
except Exception: print('')" 2>/dev/null
}

# _gs009_snapshot_roots <outfile> <producers> — one "<node> <provenance|keys|threshold|usable>"
# line per producer that ANSWERS. A node that does not answer contributes no line, and is
# therefore never eligible and never counted as a failure.
_gs009_snapshot_roots() {
    local out="$1" n r
    : > "$out"
    for n in $2; do
        r="$(_gs009_trust_root_at "$(port_of "$n")")"
        [ -n "$r" ] && echo "$n $r" >> "$out"
    done
}

# gs009_inject — the fleet rolling-restart perturbation + its monitoring window.
# Writes $WORK/gs009_{max_stall,siblings,notrejoined}.txt, the trust-root tallies
# gs009_root_{eligible,broken,unknown}.txt, the two gs009_root_{before,after}.txt
# snapshots, and a gs009_injected marker.
# No-op (assertions SKIP) unless GAUNTLET_GS009_CONFIRM=1.
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
    # INC-I-196: read the resolved trust root BEFORE the restart. This reading decides
    # ELIGIBILITY only — which producers had an OnChain root to lose. It is never
    # compared byte-for-byte against the after reading; see the header note.
    _gs009_snapshot_roots "$WORK/gs009_root_before.txt" "$producers"
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

    # ── INC-I-196: is the resolved trust root still USABLE after the restart? ──
    # This is a PROPERTY check, deliberately not a before==after comparison. Byte-equality
    # would re-encode the very antipattern that caused INC-I-196 — "differs from a frozen
    # reference, therefore refuse" — and would fail on every node at once during a
    # LEGITIMATE maintainer rotation, which is exactly what the mainnet key rotation
    # did on purpose.
    #
    # Eligible = producers that reported OnChain provenance BEFORE the restart. The
    # pre-restart reading is used ONLY to decide who is in scope: a node legitimately on
    # the compiled Bootstrap root has no on-chain root to lose and must not be able to
    # fail this scenario (the GS-011 trap). A node that does not answer AFTERWARDS is
    # counted as unknown, never as a failure — it cannot prove the property either way,
    # and gs009-fleet-rejoin is what covers a node that did not come back.
    local eligible=0 broken=0 unknown=0 before after node prov keys thr usable
    _gs009_snapshot_roots "$WORK/gs009_root_after.txt" "$producers"
    while read -r node before; do
        case "$before" in OnChain\|*) ;; *) continue ;; esac
        eligible=$(( eligible + 1 ))
        after="$(awk -v n="$node" '$1==n{print $2}' "$WORK/gs009_root_after.txt" 2>/dev/null)"
        if [ -z "$after" ]; then
            unknown=$(( unknown + 1 ))
            say "  ${C_Y}[gs009] $node did not answer getUpdateStatus after the restart — trust root unknown${C_0}"
            continue
        fi
        prov="${after%%|*}"; usable="${after##*|}"
        keys="$(echo "$after" | cut -d'|' -f2)"; thr="$(echo "$after" | cut -d'|' -f3)"
        if [ "$prov" != "OnChain" ] || [ "$usable" != "True" ] || [ "${keys:-0}" -lt "${thr:-1}" ]; then
            broken=$(( broken + 1 ))
            say "  ${C_R}[gs009] $node lost its usable on-chain trust root across the restart: '${before}' -> '${after}'${C_0}"
        fi
    done < "$WORK/gs009_root_before.txt"
    echo "$eligible" > "$WORK/gs009_root_eligible.txt"
    echo "$broken"   > "$WORK/gs009_root_broken.txt"
    echo "$unknown"  > "$WORK/gs009_root_unknown.txt"

    touch "$WORK/gs009_injected"
    say "  [gs009] max_stall=${max_stall} slots · sibling-fork samples=${total_sib} · not-rejoined=${notrejoined} · trust-root eligible=${eligible} broken=${broken} unknown=${unknown}"
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
        gs009-trust-root-provenance)
            local elig unk; elig=$(cat "$WORK/gs009_root_eligible.txt" 2>/dev/null)
            unk=$(cat "$WORK/gs009_root_unknown.txt" 2>/dev/null)
            r=$(cat "$WORK/gs009_root_broken.txt" 2>/dev/null)
            if [ "${elig:-0}" -eq 0 ]; then
                why="$t: no producer resolved an OnChain trust root before the restart — nothing to protect on this fleet"
                SKIP_REASONS="$SKIP_REASONS; $why"; return 2
            fi
            if [ "${unk:-0}" -ge "${elig:-0}" ]; then
                why="$t: none of the ${elig} eligible producer(s) answered getUpdateStatus after the restart — property not observed (see gs009-fleet-rejoin)"
                SKIP_REASONS="$SKIP_REASONS; $why"; return 2
            fi
            if [ "${r:-1}" -eq 0 ]; then return 0
            else FAIL_REASONS="$FAIL_REASONS; $t: ${r} of ${elig} producer(s) came back without a usable OnChain release trust root (INC-I-196 shape: the node refuses every release however signed, and auto-update cannot ship its own fix)"; return 1; fi ;;
        *)
            FAIL_REASONS="$FAIL_REASONS; $t: unknown gs009 token"; return 1 ;;
    esac
}
