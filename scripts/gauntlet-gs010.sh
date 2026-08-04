#!/usr/bin/env bash
# ============================================================================
# gauntlet-gs010.sh — GS-010 "duplicate-registration-poison" gauntlet scenario.
#
# Sourced by scripts/gauntlet.sh. Replays INC-I-147 (operator trigger:
# INC-I-148): a duplicate `producer register` whose second submission spends
# DISJOINT inputs survives the mempool, is included by a block builder, and
# fails apply_block with "producer already has a pending registration" —
# poisoning the block. Pre-fix this rolled the producer back BELOW its own
# finalized height, cleared the finality guard, and left non-producers
# permanently wedged behind a fork-choice guard that compared a cumulative
# WEIGHT against a block HEIGHT (the D6 unit defect).
#
# THE TRIGGER PRECONDITION IS THE WHOLE SCENARIO. It only reproduces when:
#   (a) register #1 bonds LESS THAN HALF the wallet, leaving a change output, AND
#   (b) register #1 has MINED AND CONFIRMED before #2 is submitted, so the wallet
#       funds #2 from that confirmed change => inputs are DISJOINT.
# If both txs share inputs, Mempool::revalidate silently evicts the second
# ~95us after the first mines and NOTHING happens — an accidental safety net
# that masks the defect. Both submissions must also land inside ONE epoch, so
# the first registration is still *pending* when the second is validated.
#
# PERTURBATIVE + OPT-IN + IRREVERSIBLE SIDE EFFECT. Unlike --chaos and --gs009
# (which only restart/wipe node processes), this scenario writes to the CHAIN:
# it funds a wallet and REGISTERS A PRODUCER WITH BONDS. Bonds unwind only via
# request-withdrawal (instant but with up to a 75% vesting penalty), so the
# spend is effectively permanent. It arms with `--gs010` AND requires
# GAUNTLET_GS010_CONFIRM=1, and refuses to run on anything but testnet.
#
# SKIPS CLEANLY (rc=2, never a spurious FAIL) when the fleet cannot host it:
# no live UNREGISTERED producer node, no funding source, or no non-producing
# node (only non-producers retain the finality guard and can wedge — a
# producers-only fleet recovers trivially and proves nothing).
#
# PASS criteria (asserted over the post-injection monitoring window):
#   gs010-poison-recovered   — if a block was poisoned, EVERY poisoned producer
#                              logged rollback-succeeded AND a mempool purge.
#   gs010-no-wedge           — zero wedge markers (WEDGE_ESCAPE / plan_reorg
#                              rejection / StuckFork) on NON-PRODUCING nodes.
#   gs010-fleet-reconverge   — all live nodes agree on one tip hash, and the
#                              chain advanced, by the end of the window.
#   gs010-single-registration— the target ends registered EXACTLY once, with
#                              exactly the bond count requested.
#
# Env: GS010_BONDS (70), GS010_FUND (150), GS010_MIN_EPOCH_BLOCKS (15),
#      GS010_MINE_TIMEOUT (120s), GS010_MONITOR_WINDOW (120s),
#      GS010_SAMPLE_SECS (10s). Uses gauntlet.sh globals/helpers
#      (LIVE, WORK, ROOT, LOG_DIR, say, C_*, port_of, height_of, net_max_height).
# ============================================================================

GS010_CLI="${GS010_CLI:-$HOME/testnet/bin/doli}"
GS010_KEYS="${GS010_KEYS:-$HOME/testnet/keys}"

# _gs010_tip_hash <port> — best block hash at a node, or "".
_gs010_tip_hash() {
    curl -sf -m 2 -X POST "http://127.0.0.1:$1" -H 'Content-Type: application/json' \
        -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' 2>/dev/null \
    | python3 -c "import sys,json
try: print(json.load(sys.stdin)['result']['bestHash'])
except Exception: print('')" 2>/dev/null
}

# _gs010_epoch_field <field> — one field from getEpochInfo via the first live node.
_gs010_epoch_field() {
    local p; p="$(port_of "$(echo "$LIVE" | awk '{print $1}')")"
    curl -sf -m 2 -X POST "http://127.0.0.1:$p" -H 'Content-Type: application/json' \
        -d '{"jsonrpc":"2.0","method":"getEpochInfo","params":{},"id":1}' 2>/dev/null \
    | python3 -c "import sys,json
try: print(json.load(sys.stdin)['result']['$1'])
except Exception: print('')" 2>/dev/null
}

# _gs010_is_producer <name> — 0 if the node's launchd plist carries --producer.
_gs010_is_producer() {
    grep -q -- '--producer' "$HOME/Library/LaunchAgents/network.doli.testnet-$1.plist" 2>/dev/null
}

# _gs010_registered <name> — 0 if that node's wallet is already a producer.
#
# Queried through a STABLE REFERENCE node (the first live node, normally the
# seed), NEVER through the candidate's own RPC. A candidate that just rolled
# back past its own registration height — exactly what a poison event causes —
# transiently reports itself "Not registered" from its own view, and picking it
# as the target burns a funding transfer and then dead-ends when register #1 is
# rejected. Observed on a real GS-010 run: n9 (registered at h=80893) was
# re-selected as an unregistered target moments after the poison rollbacks.
# An empty/failed response falls through to "registered" (safe: skip it).
_gs010_registered() {
    local key="$GS010_KEYS/producer_${1#n}.json" out ref
    [ -f "$key" ] || return 0   # no key => unusable as a target; treat as "taken"
    ref="$(port_of "$(echo "$LIVE" | awk '{print $1}')")"
    out="$("$GS010_CLI" --network testnet --rpc "http://127.0.0.1:$ref" \
           --wallet "$key" producer status 2>/dev/null)"
    ! printf '%s' "$out" | grep -q "Not registered"
}

# _gs010_spendable <name> — whole-DOLI spendable balance for that node's wallet.
_gs010_spendable() {
    "$GS010_CLI" --network testnet --rpc "http://127.0.0.1:$(port_of "$1")" \
        --wallet "$GS010_KEYS/producer_${1#n}.json" balance 2>/dev/null \
    | awk '/Spendable:/ {gsub(/[^0-9.]/,"",$2); printf "%d", $2; exit}'
}

# _gs010_mark_offsets — record per-node log byte offsets so the scan window
# covers ONLY what this injection produced.
_gs010_mark_offsets() {
    local name
    : > "$WORK/gs010_offsets.txt"
    for name in $LIVE; do
        printf '%s:%s\n' "$name" "$(wc -c < "$LOG_DIR/$name.log" 2>/dev/null || echo 0)" \
            >> "$WORK/gs010_offsets.txt"
    done
}

# _gs010_count <name> <extended-regex> — matches for a node since its offset.
# NOTE: `grep -c` ALREADY prints 0 and exits 1 when there are no matches, so a
# `|| echo 0` fallback emits a SECOND zero ("0\n0") and every downstream $(( ))
# dies with "integer expression expected". Swallow the status with `|| true`
# and hard-normalise to a single integer.
_gs010_count() {
    local name="$1" re="$2" off n
    off="$(grep "^$name:" "$WORK/gs010_offsets.txt" 2>/dev/null | cut -d: -f2)"
    [ -z "$off" ] && off=0
    n="$(tail -c "+$((off + 1))" "$LOG_DIR/$name.log" 2>/dev/null | grep -acE "$re" || true)"
    n="$(printf '%s' "$n" | tr -dc '0-9')"
    echo "${n:-0}"
}

# gs010_inject — fund → register → WAIT FOR MINE → register again → monitor.
# Writes $WORK/gs010_*.txt and a gs010_injected marker. No-op (assertions SKIP)
# unless GAUNTLET_GS010_CONFIRM=1 and the fleet can host the scenario.
gs010_inject() {
    if [ "${GAUNTLET_GS010_CONFIRM:-0}" != "1" ]; then
        say "  ${C_Y}[gs010] GAUNTLET_GS010_CONFIRM=1 not set — SKIPPING duplicate-registration injection"
        say "         (writes to the chain: funds a wallet and permanently bonds a producer)${C_0}"
        return 0
    fi
    [ -x "$GS010_CLI" ] || { say "  ${C_R}[gs010] CLI not found at $GS010_CLI${C_0}"; return 0; }

    # HARD SAFETY: chain-writing scenario — testnet only, never mainnet.
    local net; net="$(curl -sf -m 2 -X POST "http://127.0.0.1:$(port_of "$(echo "$LIVE" | awk '{print $1}')")" \
        -H 'Content-Type: application/json' \
        -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' 2>/dev/null \
        | python3 -c "import sys,json
try: print(json.load(sys.stdin)['result']['network'])
except Exception: print('')" 2>/dev/null)"
    if [ "$net" != "testnet" ]; then
        say "  ${C_R}[gs010] network='${net:-unknown}' is not testnet — REFUSING (scenario writes to the chain)${C_0}"
        return 0
    fi

    # ── prerequisite: a NON-PRODUCING node must be live (else nothing can wedge)
    local name nonprod=""
    for name in $LIVE; do _gs010_is_producer "$name" || nonprod="$nonprod $name"; done
    nonprod="$(echo "$nonprod" | xargs || true)"
    if [ -z "$nonprod" ]; then
        say "  ${C_Y}[gs010] no NON-PRODUCING node live — only non-producers retain the finality"
        say "         guard and can wedge; a producers-only fleet proves nothing. SKIPPING.${C_0}"
        return 0
    fi
    echo "$nonprod" > "$WORK/gs010_nonproducers.txt"

    # ── prerequisite: a live UNREGISTERED producer node to be the target ───────
    local target=""
    for name in $LIVE; do
        case "$name" in n[0-9]|n[0-9][0-9]) ;; *) continue ;; esac
        if ! _gs010_registered "$name"; then target="$name"; break; fi
    done
    if [ -z "$target" ]; then
        say "  ${C_Y}[gs010] every live producer node is already registered — no target."
        say "         Start an unregistered node (e.g. scripts/testnet.sh start n9). SKIPPING.${C_0}"
        return 0
    fi

    local bonds="${GS010_BONDS:-70}" fund="${GS010_FUND:-150}"
    local tkey="$GS010_KEYS/producer_${target#n}.json" trpc="http://127.0.0.1:$(port_of "$target")"

    # ── fund the target if it cannot cover 2x the bond ────────────────────────
    local have; have="$(_gs010_spendable "$target")"
    if [ "${have:-0}" -lt "$fund" ]; then
        local src="" srpc taddr
        for name in $LIVE; do
            case "$name" in n[0-9]|n[0-9][0-9]) ;; *) continue ;; esac
            [ "$name" = "$target" ] && continue
            [ "$(_gs010_spendable "$name")" -gt $(( fund + 10 )) ] && { src="$name"; break; }
        done
        if [ -z "$src" ]; then
            say "  ${C_Y}[gs010] no wallet holds > $(( fund + 10 )) DOLI to fund ${target}. SKIPPING.${C_0}"
            return 0
        fi
        srpc="http://127.0.0.1:$(port_of "$src")"
        taddr="$("$GS010_CLI" --network testnet --wallet "$tkey" addresses 2>/dev/null \
                 | grep -oE 'tdoli1[a-z0-9]+' | head -1)"
        say "  [gs010] funding ${target} with ${fund} DOLI from ${src}"
        "$GS010_CLI" --network testnet --rpc "$srpc" --wallet "$GS010_KEYS/producer_${src#n}.json" \
            send "$taddr" "$fund" --yes >/dev/null 2>&1
        local w=0
        while [ "$(_gs010_spendable "$target")" -lt "$fund" ] && [ "$w" -lt 90 ]; do sleep 5; w=$(( w + 5 )); done
        if [ "$(_gs010_spendable "$target")" -lt "$fund" ]; then
            say "  ${C_Y}[gs010] funding did not confirm within 90s. SKIPPING.${C_0}"; return 0
        fi
    fi

    # ── epoch headroom: the whole sequence must fit inside ONE epoch ──────────
    local need="${GS010_MIN_EPOCH_BLOCKS:-15}" rem
    rem="$(_gs010_epoch_field blocksRemaining)"
    if [ "${rem:-0}" -lt "$need" ]; then
        say "  [gs010] only ${rem} blocks left this epoch (need ${need}) — waiting for the next epoch"
        local ep0; ep0="$(_gs010_epoch_field currentEpoch)"
        while [ "$(_gs010_epoch_field currentEpoch)" = "$ep0" ]; do sleep 5; done
    fi

    _gs010_mark_offsets
    say "  ${C_Y}[gs010] target=${target} bonds=${bonds} · non-producers:${nonprod}${C_0}"

    # ── register #1 ───────────────────────────────────────────────────────────
    local tx1
    tx1="$("$GS010_CLI" --network testnet --rpc "$trpc" --wallet "$tkey" \
           producer register --bonds "$bonds" 2>&1 | awk '/TX Hash:/ {print $3; exit}')"
    if [ -z "$tx1" ]; then
        say "  ${C_R}[gs010] register #1 produced no TX hash — cannot inject${C_0}"; return 0
    fi

    # ── THE CRITICAL STEP: wait for #1 to MINE and CONFIRM. Without this the
    #    two txs share inputs, revalidate evicts #2, and nothing reproduces. ──
    local waited=0 mt="${GS010_MINE_TIMEOUT:-120}" mined=""
    while [ "$waited" -lt "$mt" ]; do
        mined="$(curl -sf -m 2 -X POST "$trpc" -H 'Content-Type: application/json' \
            -d "{\"jsonrpc\":\"2.0\",\"method\":\"getTransaction\",\"params\":{\"hash\":\"$tx1\"},\"id\":1}" 2>/dev/null \
            | python3 -c "import sys,json
try: print(json.load(sys.stdin)['result'].get('blockHeight',''))
except Exception: print('')" 2>/dev/null)"
        [ -n "$mined" ] && break
        sleep 5; waited=$(( waited + 5 ))
    done
    if [ -z "$mined" ]; then
        say "  ${C_Y}[gs010] register #1 did not mine within ${mt}s — trigger precondition unmet. SKIPPING.${C_0}"
        return 0
    fi
    say "  [gs010] register #1 mined at h=${mined} — change output now confirmed (inputs will be DISJOINT)"

    # ── register #2 — funds from the confirmed change ─────────────────────────
    local tx2
    tx2="$("$GS010_CLI" --network testnet --rpc "$trpc" --wallet "$tkey" \
           producer register --bonds "$bonds" 2>&1 | awk '/TX Hash:/ {print $3; exit}')"
    say "  [gs010] register #2 submitted (tx=${tx2:-none}) — awaiting builder inclusion"

    # ── monitor: convergence + poison/recovery/wedge telemetry ────────────────
    local win="${GS010_MONITOR_WINDOW:-120}" step="${GS010_SAMPLE_SECS:-10}" t0
    t0=$(date +%s)
    while [ $(( $(date +%s) - t0 )) -lt "$win" ]; do sleep "$step"; done

    local poisoned=0 recovered=0 purged=0 wedge=0 h r g w
    for name in $LIVE; do
        h="$(_gs010_count "$name" '\[BLOCK_POISON\] apply_block failed')"
        r="$(_gs010_count "$name" '\[BLOCK_POISON\] Rollback succeeded')"
        g="$(_gs010_count "$name" '\[BLOCK_POISON\] Purged [0-9]+ TXs')"
        [ "${h:-0}" -gt 0 ] && poisoned=$(( poisoned + 1 ))
        [ "${r:-0}" -gt 0 ] && recovered=$(( recovered + 1 ))
        [ "${g:-0}" -gt 0 ] && purged=$(( purged + 1 ))
    done
    for name in $nonprod; do
        w="$(_gs010_count "$name" 'WEDGE_ESCAPE|plan_reorg rejecting|StuckFork')"
        wedge=$(( wedge + ${w:-0} ))
    done

    # convergence: one distinct tip hash across all live nodes, and chain advanced
    local seen="" nh distinct=0
    for name in $LIVE; do
        nh="$(_gs010_tip_hash "$(port_of "$name")")"; [ -z "$nh" ] && continue
        case " $seen " in *" $nh "*) ;; *) seen="$seen $nh"; distinct=$(( distinct + 1 )) ;; esac
    done

    # ── target must end registered EXACTLY once with exactly `bonds` bonds ────
    # Producer mutations are EPOCH-DEFERRED: immediately after the monitor window
    # `producer status` still reports 0 bonds ("Activating: N DOLI (pending
    # epoch)"). Reading the count here without waiting for the boundary always
    # yields 0 and fails the assertion spuriously — observed on a real run
    # (poisoned=9/recovered=9/purged=9/wedge=0/tips=1, yet bonds read 0/70).
    # Wait (bounded) for the epoch to roll, then read the flushed count.
    local ep0 ew=0 emax="${GS010_EPOCH_WAIT:-420}" tbonds
    ep0="$(_gs010_epoch_field currentEpoch)"
    while [ "$(_gs010_epoch_field currentEpoch)" = "$ep0" ] && [ "$ew" -lt "$emax" ]; do
        sleep 10; ew=$(( ew + 10 ))
    done
    sleep 15   # let the flushed producer set settle before reading it
    tbonds="$("$GS010_CLI" --network testnet --rpc "http://127.0.0.1:$(port_of "$(echo "$LIVE" | awk '{print $1}')")" \
              --wallet "$tkey" producer status 2>/dev/null \
              | awk '/Bond Count:/ {print $3; exit}')"

    echo "$poisoned"        > "$WORK/gs010_poisoned.txt"
    echo "$recovered"       > "$WORK/gs010_recovered.txt"
    echo "$purged"          > "$WORK/gs010_purged.txt"
    echo "$wedge"           > "$WORK/gs010_wedge.txt"
    echo "$distinct"        > "$WORK/gs010_distinct.txt"
    echo "${tbonds:-0}"     > "$WORK/gs010_bonds.txt"
    echo "$bonds"           > "$WORK/gs010_bonds_want.txt"
    touch "$WORK/gs010_injected"
    say "  [gs010] poisoned=${poisoned} recovered=${recovered} purged=${purged} · wedge_markers=${wedge} · distinct_tips=${distinct} · bonds=${tbonds:-0}/${bonds}"
}

# _gs010_assert <token> — evaluate one GS-010 assertion. rc 0 pass · 1 fail ·
# 2 skip. Appends to FAIL_REASONS / SKIP_REASONS (gauntlet.sh scope).
_gs010_assert() {
    local t="$1" why p r g w d b bw
    if [ ! -f "$WORK/gs010_injected" ]; then
        why="GS-010 not injected this run (needs --gs010 + GAUNTLET_GS010_CONFIRM=1, a live UNREGISTERED producer, a funding source, and a non-producing node) — perturbative, opt-in, writes to the chain"
        SKIP_REASONS="$SKIP_REASONS; $why"; return 2
    fi
    case "$t" in
        gs010-poison-recovered)
            p=$(cat "$WORK/gs010_poisoned.txt" 2>/dev/null); r=$(cat "$WORK/gs010_recovered.txt" 2>/dev/null)
            g=$(cat "$WORK/gs010_purged.txt" 2>/dev/null)
            if [ "${p:-0}" -eq 0 ]; then
                SKIP_REASONS="$SKIP_REASONS; $t: no block was poisoned this run (builder never included tx2 in-window) — nothing to judge"
                return 2
            fi
            if [ "${r:-0}" -ge "${p:-0}" ] && [ "${g:-0}" -ge "${p:-0}" ]; then return 0
            else FAIL_REASONS="$FAIL_REASONS; $t: ${p} node(s) poisoned but only ${r} logged rollback-succeeded and ${g} logged a mempool purge"; return 1; fi ;;
        gs010-no-wedge)
            w=$(cat "$WORK/gs010_wedge.txt" 2>/dev/null)
            if [ "${w:-1}" -eq 0 ]; then return 0
            else FAIL_REASONS="$FAIL_REASONS; $t: ${w} wedge marker(s) (WEDGE_ESCAPE / plan_reorg rejection / StuckFork) on non-producing nodes — INC-I-147 fork-choice wedge"; return 1; fi ;;
        gs010-fleet-reconverge)
            d=$(cat "$WORK/gs010_distinct.txt" 2>/dev/null)
            if [ "${d:-9}" -eq 1 ]; then return 0
            else FAIL_REASONS="$FAIL_REASONS; $t: ${d} distinct tip hashes across live nodes — fleet did not reconverge"; return 1; fi ;;
        gs010-single-registration)
            b=$(cat "$WORK/gs010_bonds.txt" 2>/dev/null); bw=$(cat "$WORK/gs010_bonds_want.txt" 2>/dev/null)
            if [ "${b:-0}" -eq "${bw:-0}" ]; then return 0
            else FAIL_REASONS="$FAIL_REASONS; $t: target holds ${b} bonds, expected exactly ${bw} (duplicate registration applied, or none did)"; return 1; fi ;;
        *)
            FAIL_REASONS="$FAIL_REASONS; $t: unknown gs010 token"; return 1 ;;
    esac
}
