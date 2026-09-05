#!/usr/bin/env bash
# ============================================================================
# gauntlet-gs019.sh — GS-019 "attestation-aggregate-poisoning" (INC-I-178).
#
# Sourced by scripts/gauntlet.sh. OPT-IN (--gs019 AND GAUNTLET_GS019_CONFIRM=1),
# INJECTING by design, testnet-only. NEVER part of a default run.
#
#   gs019-poison-rejected — a forged aggregate over a bitfield the victim never
#     signed must be rejected by every node, and the victim must not be credited.
#   gs019-fleet-liveness-through-poison — the fleet keeps producing across the
#     poison window (no stall, no fork).
#   gs019-victim-attendance-preserved — the impersonated producer's attendance
#     and reward qualification survive the attempt.
#
# ALL THREE SKIP ON THIS BUILD, permanently, with the reason
# `no injection path; needs a submit RPC`. That is not a workaround, it is the
# measured state of the ingress surface: no submitAttestation, directAttestation
# or sendAttestation exists anywhere in crates/rpc/src/methods/ — the dispatch
# table carries only the read-only getAttestationStats and the unrelated oracle
# PriceAttestation. The single ingress is the libp2p gossipsub topic
# /doli/attestations/1, which requires a Noise-encrypted transport, mesh
# admission, a payload that deserializes as Attestation, a passing Ed25519
# .verify() AND ProducerSet membership. `curl` cannot reach it. A token that
# returned 0 here would certify a poison rejection nobody ever attempted.
#
# The flag, the confirm-var, the testnet guard and the marker plumbing are all
# real and armed so the scenario works the day a submit RPC exists: retrofitting
# a consent gate onto a scenario that already injects is how a destructive run
# escapes review. `_gs019_inject` writes $WORK/gs019_injected ONLY after both
# consents and a successful injection; today it writes nothing at all.
#
# Env: GS019 (flag 0/1), GAUNTLET_GS019_CONFIRM, GS019_PORTS, GS019_LOG_DIR,
#      GS019_NETWORK, GS019_TIMEOUT, WORK.
# ============================================================================

GS019_PORTS="${GS019_PORTS:-8500 8501 8502 8503 8504 8505 8506 8507 8508 8509 8510 8511 8512 8513 8514 8515 8516 8517}"
GS019_LOG_DIR="${GS019_LOG_DIR:-$HOME/testnet/logs}"
GS019_NETWORK="${GS019_NETWORK:-testnet}"
GS019_TIMEOUT="${GS019_TIMEOUT:-5}"
GS019_NET_WHY=""
# The one string that converts this SKIP into a work item.
GS019_NO_INGRESS="no injection path; needs a submit RPC — no submitAttestation/directAttestation/sendAttestation exists in crates/rpc/src/methods/, and the only ingress is the libp2p gossipsub topic /doli/attestations/1 (Noise transport + mesh admission + Attestation deserialization + Ed25519 verify + ProducerSet membership), which curl cannot reach"

# ── helpers ─────────────────────────────────────────────────────────────────

# $WORK is created by gauntlet.sh AFTER this library is sourced, so the marker
# path is resolved at call time, never at source time.
_gs019_marker() { printf '%s/gs019_injected' "${WORK:-${TMPDIR:-/tmp}}"; }

# _gs019_rpc <port> <method> [params-json] — raw JSON-RPC POST, empty on failure.
_gs019_rpc() {
    local port="$1" method="$2" params="${3:-[]}"
    curl -sf --max-time "$GS019_TIMEOUT" -X POST "http://127.0.0.1:$port" \
        -H 'Content-Type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$method\",\"params\":$params}" 2>/dev/null
}

# _gs019_testnet_ok — every answering node must report GS019_NETWORK. Sets
# GS019_NET_WHY and returns 1 when the fleet is unreachable or is not testnet.
_gs019_testnet_ok() {
    local p body net up="" foreign=""
    for p in $GS019_PORTS; do
        body="$(_gs019_rpc "$p" getChainInfo '{}')"
        [ -n "$body" ] || continue
        up="$up $p"
        net="$(printf '%s' "$body" | sed -n 's/.*"network"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | sed -n '1p')"
        [ -n "$net" ] && [ "$net" != "$GS019_NETWORK" ] && foreign="$foreign $p=$net"
    done
    if [ -z "$up" ]; then
        GS019_NET_WHY="no node answered getChainInfo on ports $GS019_PORTS — an offline fleet cannot host an injection scenario"
        return 1
    fi
    if [ -n "$foreign" ]; then
        GS019_NET_WHY="refusing to run an injection scenario against a non-$GS019_NETWORK fleet ($(printf '%s' "$foreign" | sed 's/^ //')) — GS-019 is testnet-only"
        return 1
    fi
    return 0
}

# _gs019_armed_state — human-readable consent state for the SKIP reason.
_gs019_armed_state() {
    if [ "${GS019:-0}" != "1" ]; then
        printf 'not armed (--gs019 absent)'
    elif [ "${GAUNTLET_GS019_CONFIRM:-}" != "1" ]; then
        printf 'flagged but unconfirmed (GAUNTLET_GS019_CONFIRM=1 absent)'
    else
        printf 'armed (--gs019 + GAUNTLET_GS019_CONFIRM=1)'
    fi
}

# ── injector ────────────────────────────────────────────────────────────────
# Both consents are checked HERE, and the marker is written ONLY on a delivered
# poison. Today nothing is delivered, so no marker is ever written.

_gs019_inject() {
    if [ "${GS019:-0}" != "1" ]; then
        printf '  [gs019] not armed (--gs019 absent) — nothing injected\n'
        return 0
    fi
    if [ "${GAUNTLET_GS019_CONFIRM:-}" != "1" ]; then
        printf '  [gs019] GAUNTLET_GS019_CONFIRM=1 not set — nothing injected\n'
        return 0
    fi
    if ! _gs019_testnet_ok; then
        printf '  [gs019] %s — nothing injected\n' "$GS019_NET_WHY"
        return 0
    fi
    printf '  [gs019] %s\n' "$GS019_NO_INGRESS"
    printf '  [gs019] nothing injected and no %s marker written\n' "$(_gs019_marker)"
    return 0
}

# gauntlet.sh's perturbation dispatch calls the unprefixed name, as it does for
# gs009/gs010/gs014/gs016.
gs019_inject() { _gs019_inject "$@"; }

# ── dispatcher ──────────────────────────────────────────────────────────────
# rc 0 PASS · 1 FAIL · 2 SKIP, plus the caller-owned FAIL/SKIP/INFO_REASONS.
# Every precondition is a SKIP, never a FAIL: one false FAIL is how a scenario
# earns a standing waiver and stops guarding anything.

_gs019_assert() {
    local t="${1:-}" marker
    case "$t" in
        gs019-poison-rejected|gs019-fleet-liveness-through-poison|gs019-victim-attendance-preserved) ;;
        *)
            FAIL_REASONS="$FAIL_REASONS; $t: unknown GS-019 assertion token"
            return 1 ;;
    esac
    if ! _gs019_testnet_ok; then
        SKIP_REASONS="$SKIP_REASONS; $t: $GS019_NET_WHY"
        return 2
    fi
    marker="$(_gs019_marker)"
    if [ -f "$marker" ]; then
        # The marker is only ever written by a DELIVERED poison, so a marker with
        # no ingress is a stale file from another run — trusting it would turn an
        # unexecuted poison into a pass.
        SKIP_REASONS="$SKIP_REASONS; $t: $GS019_NO_INGRESS; $marker exists but no poison could have been delivered through it on this build ($(_gs019_armed_state)) — treating the marker as stale"
        return 2
    fi
    SKIP_REASONS="$SKIP_REASONS; $t: $GS019_NO_INGRESS ($(_gs019_armed_state))"
    return 2
}

# ── standalone ──────────────────────────────────────────────────────────────
# gauntlet.sh has no single-scenario filter, so running GS-019 on its own goes
# through here. Prints the runner's own result shape (gauntlet.sh:673-682).

_gs019_main() {
    local t rc s_ok=1 tag="obs"
    [ "${GS019:-0}" = "1" ] && tag="inj"
    FAIL_REASONS=""; SKIP_REASONS=""; INFO_REASONS=""
    _gs019_inject
    for t in gs019-poison-rejected gs019-fleet-liveness-through-poison \
             gs019-victim-attendance-preserved; do
        _gs019_assert "$t"; rc=$?
        { [ "$rc" = "0" ] || [ "$rc" = "2" ]; } || s_ok=0
    done
    if [ "$s_ok" = "1" ]; then
        printf "  PASS %-5s %-32s %s\n" "[$tag]" "GS-019" "attestation-aggregate-poisoning"
    else
        printf "  FAIL %-5s %-32s %s\n" "[$tag]" "GS-019" "attestation-aggregate-poisoning"
        printf "       %s\n" "${FAIL_REASONS# ; }"
    fi
    [ -n "$SKIP_REASONS" ] && printf "       skip:%s\n" "${SKIP_REASONS# ;}"
    [ -n "$INFO_REASONS" ] && printf "       note:%s\n" "${INFO_REASONS# ;}"
    [ "$s_ok" = "1" ] || return 1
    return 0
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    _gs019_main "$@"
    exit $?
fi
