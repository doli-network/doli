#!/usr/bin/env bash
# ============================================================================
# gauntlet-gs010.sh — GS-010 "duplicate-registration-poison" gauntlet scenario.
#
# Sourced by scripts/gauntlet.sh. Replays INC-I-147 (operator trigger:
# INC-I-148): a duplicate producer registration whose second submission spends
# DISJOINT inputs survives the mempool, is included by a block builder, and
# fails apply_block with "producer already has a pending registration" —
# poisoning the block. Pre-fix this rolled the producer back BELOW its own
# finalized height, cleared the finality guard, and left non-producers
# permanently wedged behind a fork-choice guard that compared a cumulative
# WEIGHT against a block HEIGHT (the D6 unit defect).
#
# INJECTION IS RAW JSON-RPC, NOT THE CLI. INC-I-148 fixed
# `doli producer register` to consult the PLURAL getProducers and REFUSE a
# second registration for a key that already has a pending one
# (bins/cli/src/cmd_producer/register.rs:37) — it bails BEFORE computing the
# VDF, so no transaction is ever built. That fix is correct and is NOT weakened
# or bypassed here. But `sendTransaction` is publicly dispatchable
# (crates/rpc/src/methods/dispatch.rs:22), so the duplicate-registration vector
# is still real — it only moved off the CLI and onto the wire. The CLI check is
# UX; the genuine guard is node-side (mempool admission populating
# ctx.pending_producer_keys, plus revalidate eviction). GS-010 therefore
# exercises the NODE: register #2 is rebuilt from register #1's own on-chain
# bytes by scripts/gs010_build_dup_register.py and POSTed raw.
#
# THE TRIGGER PRECONDITION IS THE WHOLE SCENARIO. It only reproduces when:
#   (a) register #1 bonds LESS THAN HALF the wallet, leaving a change output, AND
#   (b) register #1 has MINED AND CONFIRMED before #2 is submitted, so #2 is
#       funded from that confirmed change => inputs are DISJOINT.
# If both txs share inputs, Mempool::revalidate silently evicts the second
# ~95us after the first mines and NOTHING happens — an accidental safety net
# that masks the defect. Both submissions must also land inside ONE epoch, so
# the first registration is still *pending* when the second is validated.
#
# DISJOINTNESS IS NOW STRUCTURAL, NOT INCIDENTAL. The builder funds #2 from
# `tx1_hash:change_vout` — an output CREATED BY TX1. A transaction cannot spend
# its own outputs, so the two input sets are disjoint by construction instead of
# depending on wallet coin-selection happening to pick the change UTXO. The
# builder still refuses to emit unless tx1 minted exactly one change output and
# that outpoint is live in the node's UTXO set, so precondition (a)+(b) remain
# enforced — they are just no longer the only thing holding the property up.
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
# PASS criteria (asserted over the post-injection monitoring window). These
# assert the POST-FIX outcome: the duplicate never enters the system at all.
#   gs010-dup-rejected       — the raw sendTransaction was REFUSED at mempool
#                              admission with INVALID_REGISTRATION, on the
#                              target's node AND on a non-producing node.
#                              Acceptance = the INC-I-147 D1 parity gap is back.
#   gs010-no-poison          — zero [BLOCK_POISON] apply_block failures and zero
#                              poison rollbacks fleet-wide. Pre-fix this scenario
#                              measured 9 poisoned + 9 rolled-back nodes; post-fix
#                              the duplicate is rejected before any builder can
#                              see it, so the correct count is ZERO.
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
GS010_BUILDER="${GS010_BUILDER:-$ROOT/scripts/gs010_build_dup_register.py}"

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

# _gs010_raw_send <rpc-url> <payload-file> — POST a prebuilt sendTransaction
# body and classify the node's answer. Echoes exactly one line:
#     accepted|<tx hash>|
#     rejected|<error_code>|<message>
#     unreachable||
# `-f` is deliberately NOT passed to curl: a JSON-RPC rejection is the RESULT
# we are measuring, and dropping the body on a non-2xx status would turn a
# measured rejection into an indistinguishable "unreachable".
_gs010_raw_send() {
    local url="$1" body="$2" resp
    [ -f "$body" ] || { echo 'unreachable||'; return; }
    resp="$(curl -s -m 15 -X POST "$url" -H 'Content-Type: application/json' \
            --data-binary "@$body" 2>/dev/null)"
    if [ -z "$resp" ]; then echo 'unreachable||'; return; fi
    printf '%s' "$resp" | python3 -c '
import sys, json
def clean(s):
    return str(s).replace("|", "/").replace("\n", " ").strip()
try:
    d = json.load(sys.stdin)
except Exception:
    print("unreachable||"); raise SystemExit
err = d.get("error")
if err:
    data = err.get("data") or {}
    code = data.get("error_code") or err.get("code") or "UNKNOWN"
    print("rejected|%s|%s" % (clean(code), clean(err.get("message", ""))))
else:
    res = d.get("result") or {}
    h = res.get("hash", "") if isinstance(res, dict) else str(res)
    print("accepted|%s|" % clean(h))
' 2>/dev/null || echo 'unreachable||'
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

    # ── register #2 — RAW sendTransaction, rebuilt from #1's on-chain bytes ───
    # NOT `doli producer register`: that path is closed by the INC-I-148 CLI
    # pre-check and is no longer a duplicate-injection vector. Raw RPC still is,
    # so this is both the honest reproduction and a test of the layer that
    # actually guards the invariant (mempool admission).
    if [ ! -f "$GS010_BUILDER" ]; then
        say "  ${C_R}[gs010] builder not found at $GS010_BUILDER — cannot inject${C_0}"; return 0
    fi
    local build brc tx2 dupin
    build="$(python3 "$GS010_BUILDER" --rpc "$trpc" --wallet "$tkey" \
             --tx1 "$tx1" --height "$mined" 2>&1)"; brc=$?
    if [ "$brc" -ne 0 ]; then
        say "  ${C_Y}[gs010] could not build the duplicate registration — SKIPPING.${C_0}"
        say "         ${build}"
        return 0
    fi
    # Split the builder's JSON into the curl body + the fields the assertions
    # need. Done in ONE python pass so the ~11 KB tx hex never crosses argv.
    if ! printf '%s' "$build" | python3 -c '
import sys, json
d = json.load(sys.stdin)
body = {"jsonrpc": "2.0", "method": "sendTransaction",
        "params": {"tx": d["tx_hex"]}, "id": 1}
with open(sys.argv[1], "w") as fh:
    json.dump(body, fh)
with open(sys.argv[2], "w") as fh:
    fh.write("%s\n%s\n%s\n" % (d["tx_hash"], d["input"], ",".join(d["tx1_inputs"])))
' "$WORK/gs010_tx2_body.json" "$WORK/gs010_tx2_meta.txt" 2>/dev/null; then
        say "  ${C_R}[gs010] builder output was not parseable — cannot inject${C_0}"; return 0
    fi
    tx2="$(sed -n 1p "$WORK/gs010_tx2_meta.txt")"
    dupin="$(sed -n 2p "$WORK/gs010_tx2_meta.txt")"
    say "  [gs010] duplicate built: tx=${tx2:-none} spending ${dupin} (tx1's OWN change ⇒ inputs DISJOINT)"

    # Primary injection: the target's own node, exactly where an operator would
    # have pointed the CLI.
    local send1 send2 nprobe
    send1="$(_gs010_raw_send "$trpc" "$WORK/gs010_tx2_body.json")"
    say "  [gs010] raw sendTransaction → ${target}: ${send1}"

    # Secondary probe: a NON-PRODUCING node. INC-I-147 measured the seed holding
    # four toxic registrations for 49-102 minutes across 577 revalidate passes —
    # a non-producer never builds a block, so it never gets the poison-block path
    # that lets producers shed them. Only probed when the primary REFUSED, so the
    # tx cannot have been gossiped here first and answer "already known".
    send2='skipped||'
    nprobe="$(echo "$nonprod" | awk '{print $1}')"
    case "$send1" in
        rejected*)
            if [ -n "$nprobe" ]; then
                send2="$(_gs010_raw_send "http://127.0.0.1:$(port_of "$nprobe")" \
                         "$WORK/gs010_tx2_body.json")"
                say "  [gs010] raw sendTransaction → ${nprobe} (non-producer): ${send2}"
            fi ;;
    esac

    echo "$send1" > "$WORK/gs010_send_primary.txt"
    echo "$send2" > "$WORK/gs010_send_secondary.txt"
    echo "${tx2:-none}" > "$WORK/gs010_tx2_hash.txt"
    echo "${dupin:-none}" > "$WORK/gs010_dup_input.txt"
    say "  [gs010] injected — observing the fleet for poison / rollback / wedge"

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
    say "  [gs010] send=${send1} · poisoned=${poisoned} rolled_back=${recovered} purged=${purged} · wedge_markers=${wedge} · distinct_tips=${distinct} · bonds=${tbonds:-0}/${bonds}"
}

# _gs010_assert <token> — evaluate one GS-010 assertion. rc 0 pass · 1 fail ·
# 2 skip. Appends to FAIL_REASONS / SKIP_REASONS (gauntlet.sh scope).
_gs010_assert() {
    local t="$1" why p r g w d b bw s1 s2 c1 m1 c2
    if [ ! -f "$WORK/gs010_injected" ]; then
        why="GS-010 not injected this run (needs --gs010 + GAUNTLET_GS010_CONFIRM=1, a live UNREGISTERED producer, a funding source, and a non-producing node) — perturbative, opt-in, writes to the chain"
        SKIP_REASONS="$SKIP_REASONS; $why"; return 2
    fi
    case "$t" in
        gs010-dup-rejected)
            # The duplicate must be REFUSED at mempool admission. Pre-fix,
            # ctx.pending_producer_keys was Vec::new() at every admission site
            # (crates/mempool/src/pool.rs:291,661), so the check at
            # validation/registration.rs:173 was a guaranteed no-op and EVERY
            # node accepted the duplicate — that is INC-I-147 defect D1.
            s1="$(cat "$WORK/gs010_send_primary.txt" 2>/dev/null)"
            s2="$(cat "$WORK/gs010_send_secondary.txt" 2>/dev/null)"
            c1="$(printf '%s' "$s1" | cut -d'|' -f2)"; m1="$(printf '%s' "$s1" | cut -d'|' -f3)"
            c2="$(printf '%s' "$s2" | cut -d'|' -f2)"
            case "$s1" in
                accepted*)
                    FAIL_REASONS="$FAIL_REASONS; $t: the node ADMITTED the duplicate registration to its mempool (hash ${c1}) — the INC-I-147 D1 admission parity gap is open again; a block builder will select it and poison its own block"
                    return 1 ;;
                unreachable*)
                    SKIP_REASONS="$SKIP_REASONS; $t: the target node did not answer the raw sendTransaction — nothing measured"
                    return 2 ;;
            esac
            # Rejected — but it must be rejected for the RIGHT reason. A malformed
            # tx, a stale fee or a spent input would also produce a rejection and
            # would be a FALSE PASS, so an unrelated reason is INCONCLUSIVE.
            case "$c1$m1" in
                *INVALID_REGISTRATION*|*"pending registration"*|*"already registered"*) ;;
                *)
                    SKIP_REASONS="$SKIP_REASONS; $t: the duplicate was rejected but for an unrelated reason (${s1}) — the duplicate-registration guard was never reached, so this run proves nothing"
                    return 2 ;;
            esac
            case "$s2" in
                accepted*)
                    FAIL_REASONS="$FAIL_REASONS; $t: the target's node refused the duplicate but a NON-PRODUCING node ADMITTED it (${s2}) — a non-producer never builds a block, so it holds the toxic tx until revalidate evicts it (INC-I-147: 49-102 minutes across 577 passes)"
                    return 1 ;;
                rejected*)
                    case "$c2" in
                        INVALID_REGISTRATION) return 0 ;;
                        *) SKIP_REASONS="$SKIP_REASONS; $t: primary refused correctly, but the non-producer probe answered ${s2} — partial evidence only"
                           return 2 ;;
                    esac ;;
            esac
            return 0 ;;
        gs010-no-poison)
            # Post-fix the duplicate never reaches a block builder, so the honest
            # expectation is ZERO poison and ZERO poison-rollback. A real pre-fix
            # run measured poisoned=9 / rolled_back=9 / purged=9.
            p=$(cat "$WORK/gs010_poisoned.txt" 2>/dev/null); r=$(cat "$WORK/gs010_recovered.txt" 2>/dev/null)
            g=$(cat "$WORK/gs010_purged.txt" 2>/dev/null)
            if [ "${p:-1}" -eq 0 ] && [ "${r:-1}" -eq 0 ]; then return 0; fi
            FAIL_REASONS="$FAIL_REASONS; $t: ${p} node(s) logged [BLOCK_POISON] apply_block failure and ${r} rolled back (${g} purged) — the duplicate reached a block, so it was admitted somewhere despite the mempool guard"
            return 1 ;;
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
            else FAIL_REASONS="$FAIL_REASONS; $t: target holds ${b} bonds, expected exactly ${bw} (the duplicate registration applied on top of #1, or neither did)"; return 1; fi ;;
        *)
            FAIL_REASONS="$FAIL_REASONS; $t: unknown gs010 token"; return 1 ;;
    esac
}
