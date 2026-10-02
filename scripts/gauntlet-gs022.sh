#!/usr/bin/env bash
# ============================================================================
# gauntlet-gs022.sh — GS-022 "losing-sibling-fork-walk" (INC-I-235).
#
# Sourced by scripts/gauntlet.sh. OPT-IN (--gs022 AND GAUNTLET_GS022_CONFIRM=1),
# testnet-only. Runs scripts/inc-i-235-drill.sh against ONE non-seed producer
# (GS022_TARGET, default n3) and judges its evidence block.
#
#   gs022-sibling-induced     the drill formed a 1-deep losing sibling.
#   gs022-converged-by-reorg  the target converged to the canonical hash.
#   gs022-no-snapsync         no snap sync on the target in the drill window.
#   gs022-fleet-converged     every node's tip matches the seed at its height.
#
# Env: GS022 (flag 0/1), GAUNTLET_GS022_CONFIRM, GS022_TARGET, GS022_TIMEOUT, WORK.
# ============================================================================

GS022_TARGET="${GS022_TARGET:-n3}"
GS022_TIMEOUT="${GS022_TIMEOUT:-900}"

_gs022_evidence() { printf '%s/gs022_evidence.txt' "${WORK:-${TMPDIR:-/tmp}}"; }

_gs022_inject() {
    local ev rc
    if [ "${GAUNTLET_GS022_CONFIRM:-0}" != "1" ]; then
        printf '  [gs022] GAUNTLET_GS022_CONFIRM=1 not set — drill skipped, nothing injected\n'
        return 0
    fi
    ev="$(_gs022_evidence)"
    printf '  [gs022] running inc-i-235-drill.sh --target %s --timeout %s\n' "$GS022_TARGET" "$GS022_TIMEOUT"
    bash "$ROOT/scripts/inc-i-235-drill.sh" --target "$GS022_TARGET" \
        --timeout "$GS022_TIMEOUT" --out "$ev"
    rc=$?
    printf '  [gs022] drill exit=%s evidence=%s\n' "$rc" "$ev"
    return 0
}

gs022_inject() { _gs022_inject "$@"; }

# rc 0 PASS · 1 FAIL · 2 SKIP; appends to the caller-owned FAIL/SKIP_REASONS.
_gs022_assert() {
    local t="${1:-}" ev
    ev="$(_gs022_evidence)"
    if [ ! -s "$ev" ]; then
        SKIP_REASONS="$SKIP_REASONS; $t: no drill evidence (needs --gs022 + GAUNTLET_GS022_CONFIRM=1)"
        return 2
    fi
    case "$t" in
        gs022-sibling-induced)
            grep -qE '^sibling: height=[0-9]+ ' "$ev" && return 0
            FAIL_REASONS="$FAIL_REASONS; $t: no losing sibling was induced" ;;
        gs022-converged-by-reorg)
            grep -qE '^converged=1 ' "$ev" && return 0
            FAIL_REASONS="$FAIL_REASONS; $t: $(grep -E '^converged=' "$ev")" ;;
        gs022-no-snapsync)
            grep -qE 'snap_sync_in_window=0$' "$ev" && return 0
            FAIL_REASONS="$FAIL_REASONS; $t: snap sync observed on $GS022_TARGET" ;;
        gs022-fleet-converged)
            awk '/^-- convergence table/{f=1;next} f&&NR>0&&$1!="node"&&$4!="yes"{bad=1} END{exit bad}' "$ev" && return 0
            FAIL_REASONS="$FAIL_REASONS; $t: a node tip does not match the seed" ;;
        *)
            FAIL_REASONS="$FAIL_REASONS; $t: unknown GS-022 assertion token" ;;
    esac
    return 1
}
