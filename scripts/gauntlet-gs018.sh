#!/usr/bin/env bash
# ============================================================================
# gauntlet-gs018.sh — GS-018 "attestation-bitfield-integrity" (INC-I-178).
#
# Sourced by scripts/gauntlet.sh. OBSERVATIONAL, STATE-NEUTRAL, chain-read-only,
# testnet-only, no confirm-var, part of the DEFAULT run.
#
#   gs018-presence-root-consistent — samples recent heights from every answering
#     node and requires ONE presenceRoot per height. Cross-node agreement needs
#     >= GS018_MIN_NODES answering; below that it SKIPs, because agreement
#     measured over two nodes is not cross-node agreement. attestationCount is
#     NEVER read as a headcount: it is a popcount of the presence_root HASH, so a
#     verdict driven by it is driven by hash entropy. A block carrying an
#     aggregateBlsSig is not a defect either — its presence IS the AH-crossed
#     litmus, so it is recorded, never failed on.
#   gs018-active-producers-dual-sign — REQ-BLS-006 AC-2, "100% of active
#     producers emit BLS-signed attestations". Observable since INC-I-178 M7.5:
#     the ingress VALID arm publishes doli_attestation_bls_valid_total
#     (fleet-wide, first-seen halves only) and
#     doli_attestation_bls_valid_attester_total{attester="<8 hex>"}, whose label
#     joins to a getProducers row by pubkey prefix. The denominator is the chain's
#     status=="active" rows, NEVER the node count. If NO node exposes the
#     fleet-wide series the fleet is below M7.5 and this SKIPs — the key is the
#     ABSENCE of the emission signal, never a version string, because M7.5 bumps
#     no version. An empty fleet-wide label union also SKIPs: a node restarted
#     seconds ago has ingested nothing, and a zero-length observation window must
#     never manufacture a red gauntlet. It FAILs only when the capability is
#     present, at least one attester has been observed, and some ACTIVE producer
#     still carries no series — that producer is not dual-signing.
#   gs018-post-ah-aggregate-verifies — gated on the AH litmus, since the
#     activation height is u64::MAX on every network and no RPC exposes it:
#     doli_attestation_verify_total > 0, OR getAttestationStats.blocksWithBls > 0,
#     OR aggregateBlsSig present on a sampled block. None of the three means
#     pre-AH, which SKIPs — a post-AH assertion that FAILs pre-AH would be red on
#     every run from the day it ships. Once crossed it reads the verify counters
#     and fails only on a non-zero rejected total.
#
# Build detection is by CAPABILITY, never by version: the INC-I-178 build reports
# `6.26.3`, byte-identical to the fleet it replaces. The marker is the presence of
# the series `doli_attestation_verify_total` on /metrics; the M7.5 dual-sign check
# uses its own, later marker, `doli_attestation_bls_valid_total`.
#
# Every precondition (RPC down, python3 missing, non-testnet fleet) is a SKIP
# (rc 2) with a written SKIP_REASONS entry, never a FAIL: gauntlet.sh:684-689
# treats rc 0 and rc 2 identically, so SKIP_REASONS is the only thing separating
# "checked green" from "never checked", and one false FAIL is how a scenario
# earns a standing waiver and stops guarding anything.
#
# Env: GS018_PORTS, GS018_METRICS_PORTS, GS018_LOG_DIR, GS018_SAMPLE, GS018_LAG,
#      GS018_MIN_NODES, GS018_NETWORK, GS018_TIMEOUT, GAUNTLET_WINDOW.
# ============================================================================

GS018_PORTS="${GS018_PORTS:-8500 8501 8502 8503 8504 8505 8506 8507 8508 8509 8510 8511 8512 8513 8514 8515 8516 8517}"
GS018_METRICS_PORTS="${GS018_METRICS_PORTS:-9000 9001 9002 9003 9004 9005 9006 9007 9008 9009 9010 9011 9012 9013 9014 9015 9016 9017}"
GS018_LOG_DIR="${GS018_LOG_DIR:-$HOME/testnet/logs}"
GS018_SAMPLE="${GS018_SAMPLE:-5}"
# Heights below the tip, so a node one slot behind is not read as a divergence.
GS018_LAG="${GS018_LAG:-3}"
GS018_MIN_NODES="${GS018_MIN_NODES:-3}"
GS018_NETWORK="${GS018_NETWORK:-testnet}"
GS018_TIMEOUT="${GS018_TIMEOUT:-5}"
GS018_WINDOW_SECS="${GAUNTLET_WINDOW:-45}"
GS018_VERIFY_SERIES="doli_attestation_verify_total"
GS018_BLS_VALID_SERIES="doli_attestation_bls_valid_total"
GS018_BLS_ATTESTER_SERIES="doli_attestation_bls_valid_attester_total"

GS018_PROBED=0
GS018_UP=""
GS018_MIN_HEIGHT=""
GS018_WHY=""

# Pre-window baseline for every node log, captured at SOURCE time: gauntlet.sh
# sources this lib before it sleeps its window, so the whole run is the window.
# Used for one INFORMATIONAL count only — no GS-018 verdict reads a log.
GS018_SRC_OFFSETS="$(
    for _gs018_f in "$GS018_LOG_DIR"/*.log; do
        [ -r "$_gs018_f" ] || continue
        printf '%s:%s\n' "$_gs018_f" "$(wc -c < "$_gs018_f" 2>/dev/null | tr -d ' ')"
    done
)"

# ── helpers ─────────────────────────────────────────────────────────────────

# _gs018_rpc <port> <method> [params-json] — raw JSON-RPC POST, empty on failure.
_gs018_rpc() {
    local port="$1" method="$2" params="${3:-[]}"
    curl -sf --max-time "$GS018_TIMEOUT" -X POST "http://127.0.0.1:$port" \
        -H 'Content-Type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$method\",\"params\":$params}" 2>/dev/null
}

# _gs018_metrics <port> — Prometheus text body, empty on failure.
_gs018_metrics() {
    curl -sf --max-time "$GS018_TIMEOUT" "http://127.0.0.1:$1/metrics" 2>/dev/null
}

_gs018_have_python() { python3 -c 'pass' >/dev/null 2>&1; }

_gs018_scalar() {
    sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\{0,1\}\([^\",}]*\).*/\1/p" | sed -n '1p'
}

# _gs018_block_fields — stdin lines '<port> <json>', stdout '<port> <root> <agg>'.
# One python3 spawn per height instead of one per node-height pair.
_gs018_block_fields() {
    python3 -c '
import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    port, _, body = line.partition(" ")
    try:
        d = json.loads(body)
        r = d.get("result") or {}
        root = r.get("presenceRoot") or "-"
        agg = "agg" if r.get("aggregateBlsSig") else "noagg"
        print("%s %s %s" % (port, root, agg))
    except Exception:
        print("%s - unreadable" % port)' 2>/dev/null
}

# _gs018_producer_counts — stdin getProducers reply, stdout '<active> <total>'.
_gs018_producer_counts() {
    python3 -c '
import sys, json
try:
    d = json.load(sys.stdin)
    r = d.get("result", d) if isinstance(d, dict) else d
    ps = r.get("producers", r) if isinstance(r, dict) else r
    if not isinstance(ps, list):
        raise ValueError("shape")
    act = sum(1 for p in ps if isinstance(p, dict) and str(p.get("status", "")).lower() == "active")
    print("%d %d" % (act, len(ps)))
except Exception:
    print("ERR")' 2>/dev/null
}

# _gs018_active_pubkeys — stdin getProducers reply, stdout the full lowercase
# public key of every status=="active" row, one per line. This is the AC-2
# denominator's identity list; the node count is never a substitute for it.
_gs018_active_pubkeys() {
    python3 -c '
import sys, json
try:
    d = json.load(sys.stdin)
    r = d.get("result", d) if isinstance(d, dict) else d
    ps = r.get("producers", r) if isinstance(r, dict) else r
    if not isinstance(ps, list):
        raise ValueError("shape")
    for p in ps:
        if not isinstance(p, dict):
            continue
        if str(p.get("status", "")).lower() != "active":
            continue
        k = p.get("publicKey") or p.get("public_key") or ""
        if k:
            print(str(k).lower())
except Exception:
    pass' 2>/dev/null
}

# _gs018_attester_labels — stdin /metrics body, stdout the `attester` label of
# every doli_attestation_bls_valid_attester_total sample ABOVE zero. A sample at
# 0 is not an observation of dual-signing, so it is not counted.
_gs018_attester_labels() {
    awk -v s="$GS018_BLS_ATTESTER_SERIES" '
        index($0, s "{") != 1 { next }
        $NF + 0 <= 0 { next }
        match($0, /attester="[^"]*"/) {
            print tolower(substr($0, RSTART + 10, RLENGTH - 11))
        }' 2>/dev/null
}

# _gs018_new_build_count — nodes exposing the INC-I-178 capability marker.
_gs018_new_build_count() {
    local p body n=0
    for p in $GS018_METRICS_PORTS; do
        body="$(_gs018_metrics "$p")"
        [ -n "$body" ] || continue
        printf '%s\n' "$body" | grep -qE "^${GS018_VERIFY_SERIES}([[:space:]{]|\$)" && n=$(( n + 1 ))
    done
    printf '%s' "$n"
}

# _gs018_new_warn_count — [ATTEST_INGEST] unverifiable-BLS-half warnings past the
# source-time offsets. INFORMATIONAL ONLY: this line fires exclusively on a
# relayed INVALID half, so its absence proves nothing about dual-signing.
_gs018_new_warn_count() {
    local logf off n total=0
    while IFS=':' read -r logf off; do
        [ -n "$logf" ] && [ -r "$logf" ] || continue
        n="$(tail -c "+$(( ${off:-0} + 1 ))" "$logf" 2>/dev/null \
            | grep -acE '\[ATTEST_INGEST\].*unverifiable BLS half' || true)"
        n="$(printf '%s' "$n" | tr -dc '0-9')"
        total=$(( total + ${n:-0} ))
    done <<EOF
$GS018_SRC_OFFSETS
EOF
    printf '%s' "$total"
}

# ── preflight ───────────────────────────────────────────────────────────────

# _gs018_probe_fleet — one pass over GS018_PORTS. Fills GS018_UP with the
# answering ports and GS018_MIN_HEIGHT with the lowest tip; sets GS018_WHY and
# returns 1 when the fleet cannot host an observation.
_gs018_probe_fleet() {
    [ "$GS018_PROBED" = "1" ] && { [ -n "$GS018_UP" ] || return 1; return 0; }
    GS018_PROBED=1
    local p body net h foreign=""
    for p in $GS018_PORTS; do
        body="$(_gs018_rpc "$p" getChainInfo '{}')"
        [ -n "$body" ] || continue
        GS018_UP="$GS018_UP $p"
        net="$(printf '%s' "$body" | _gs018_scalar network)"
        [ -n "$net" ] && [ "$net" != "$GS018_NETWORK" ] && foreign="$foreign $p=$net"
        h="$(printf '%s' "$body" | _gs018_scalar bestHeight | tr -dc '0-9')"
        if [ -n "$h" ]; then
            if [ -z "$GS018_MIN_HEIGHT" ] || [ "$h" -lt "$GS018_MIN_HEIGHT" ]; then
                GS018_MIN_HEIGHT="$h"
            fi
        fi
    done
    if [ -z "$GS018_UP" ]; then
        GS018_WHY="no node answered getChainInfo on ports $GS018_PORTS — nothing to observe"
        return 1
    fi
    if [ -n "$foreign" ]; then
        GS018_WHY="refusing to probe a non-$GS018_NETWORK fleet ($(printf '%s' "$foreign" | sed 's/^ //')) — GS-018 is testnet-only"
        GS018_UP=""
        return 1
    fi
    return 0
}

# _gs018_preflight <token> — python3 + a live testnet fleet. Writes SKIP_REASONS.
_gs018_preflight() {
    local t="$1"
    if ! _gs018_have_python; then
        SKIP_REASONS="$SKIP_REASONS; $t: python3 is not usable on this host and every JSON-RPC reply is parsed with it — no verdict without it"
        return 1
    fi
    if ! _gs018_probe_fleet; then
        SKIP_REASONS="$SKIP_REASONS; $t: $GS018_WHY"
        return 1
    fi
    return 0
}

# _gs018_heights — the sampled heights, newest first, or empty.
_gs018_heights() {
    local top i h out=""
    [ -n "$GS018_MIN_HEIGHT" ] || return 0
    top=$(( GS018_MIN_HEIGHT - GS018_LAG ))
    i=0
    while [ "$i" -lt "$GS018_SAMPLE" ]; do
        h=$(( top - i ))
        [ "$h" -ge 1 ] || break
        out="$out $h"
        i=$(( i + 1 ))
    done
    printf '%s' "${out# }"
}

# ── assertions ──────────────────────────────────────────────────────────────

_gs018_root_check() {
    local t="$1" heights h p body lines fields port root agg
    local nodes seen roots compared=0 aggseen=0 first="" diverged=""
    heights="$(_gs018_heights)"
    if [ -z "$heights" ]; then
        SKIP_REASONS="$SKIP_REASONS; $t: no node reported a usable bestHeight — no sample window"
        return 2
    fi
    for h in $heights; do
        lines=""
        for p in $GS018_UP; do
            body="$(_gs018_rpc "$p" getBlockByHeight "{\"height\":$h}")"
            [ -n "$body" ] || continue
            lines="$lines$p $(printf '%s' "$body" | tr -d '\n')
"
        done
        [ -n "$lines" ] || continue
        fields="$(printf '%s' "$lines" | _gs018_block_fields)"
        nodes=0; seen=""; roots=""; first=""
        while read -r port root agg; do
            [ -n "$port" ] || continue
            [ "$root" = "-" ] && continue
            nodes=$(( nodes + 1 ))
            [ "$agg" = "agg" ] && aggseen=$(( aggseen + 1 ))
            [ -n "$first" ] || first="$root"
            seen="$seen $port=$(printf '%.16s' "$root")"
            case " $roots " in *" $root "*) ;; *) roots="$roots $root" ;; esac
        done <<EOF
$fields
EOF
        [ "$nodes" -ge "$GS018_MIN_NODES" ] || continue
        compared=$(( compared + 1 ))
        if [ "$(printf '%s' "$roots" | wc -w | tr -d ' ')" -gt 1 ]; then
            diverged="$diverged height $h:$seen;"
        fi
    done
    if [ -n "$diverged" ]; then
        FAIL_REASONS="$FAIL_REASONS; $t: presenceRoot diverges across nodes — $(printf '%s' "$diverged" | sed 's/;$//') (one height with two presence roots is a consensus divergence in the attestation bitfield)"
        return 1
    fi
    if [ "$compared" -eq 0 ]; then
        SKIP_REASONS="$SKIP_REASONS; $t: fewer than $GS018_MIN_NODES nodes returned a presenceRoot at any of the sampled heights ($heights) — agreement over fewer nodes is not cross-node agreement"
        return 2
    fi
    INFO_REASONS="$INFO_REASONS; $t: presenceRoot identical across all answering nodes at $compared sampled height(s) ($heights), $(printf '%s' "$GS018_UP" | wc -w | tr -d ' ') node(s) answering, $aggseen sampled block(s) carried an aggregateBlsSig"
    return 0
}

_gs018_dual_check() {
    local t="$1" p k pre body counts nodes
    local active="unknown" registered="unknown" pubkeys="" named=0
    local capable=0 labels="" matched="" missing="" m=0
    for p in $GS018_UP; do
        body="$(_gs018_rpc "$p" getProducers '{"active_only": false}')"
        [ -n "$body" ] || continue
        counts="$(printf '%s' "$body" | _gs018_producer_counts)"
        case "$counts" in ERR|"") continue ;; esac
        active="${counts%% *}"; registered="${counts##* }"
        pubkeys="$(printf '%s' "$body" | _gs018_active_pubkeys)"
        break
    done
    named="$(printf '%s' "$pubkeys" | wc -w | tr -d ' ')"
    nodes="$(printf '%s' "$GS018_METRICS_PORTS" | wc -w | tr -d ' ')"
    if [ "$active" = "unknown" ] || [ "$active" -eq 0 ] || [ "$named" -ne "$active" ]; then
        SKIP_REASONS="$SKIP_REASONS; $t: the AC-2 denominator is not readable — getProducers gave $active active row(s) of $registered registered but $named usable public key(s); the node count is never a substitute for the chain's active set"
        return 2
    fi

    for p in $GS018_METRICS_PORTS; do
        body="$(_gs018_metrics "$p")"
        [ -n "$body" ] || continue
        printf '%s\n' "$body" | grep -qE "^${GS018_BLS_VALID_SERIES}([[:space:]{]|\$)" || continue
        capable=$(( capable + 1 ))
        labels="$labels $(printf '%s\n' "$body" | _gs018_attester_labels | tr '\n' ' ')"
    done
    if [ "$capable" -eq 0 ]; then
        SKIP_REASONS="$SKIP_REASONS; $t: no node of $nodes exposes $GS018_BLS_VALID_SERIES on /metrics, so the per-producer BLS-emission signal is ABSENT and this fleet is below INC-I-178 M7.5 — dual-signing stays unobservable per producer and no verdict is possible. Keyed on the signal's absence, never on a version string: M7.5 bumps no version and the series is zero-initialised at process start, so a running M7.5 node always exposes it. $(_gs018_new_build_count) node(s) do carry the earlier M5 marker $GS018_VERIFY_SERIES, which separates a pre-M7.5 INC-I-178 fleet from a fleet with no INC-I-178 build at all. Denominator: $active active producer(s) of $registered chain-registered"
        return 2
    fi

    labels="$(printf '%s' "$labels" | tr ' ' '\n' | grep -v '^$' | sort -u | tr '\n' ' ')"
    if [ -z "$labels" ]; then
        SKIP_REASONS="$SKIP_REASONS; $t: $capable of $nodes node(s) expose $GS018_BLS_VALID_SERIES, but $GS018_BLS_ATTESTER_SERIES carries no series above 0 anywhere in the fleet — no verifying BLS half has been ingested yet, so there is no observation window (a node restarted seconds ago has counted nothing) and a zero-length window must never be read as 'nobody dual-signs'. Denominator: $active active producer(s) of $registered chain-registered"
        return 2
    fi

    for k in $pubkeys; do
        pre="$(printf '%.8s' "$k")"
        case " $labels" in
            *" $pre "*) matched="$matched $pre"; m=$(( m + 1 )) ;;
            *) missing="$missing $pre" ;;
        esac
    done
    if [ "$m" -lt "$active" ]; then
        FAIL_REASONS="$FAIL_REASONS; $t: only $m of $active active producer(s) emit a verifying BLS half ($GS018_BLS_ATTESTER_SERIES) — no series for$missing. Those producers are not dual-signing, which is REQ-BLS-006 AC-2 failing; pinning the activation height now would strip them from every aggregate. Observed over $capable of $nodes node(s) carrying $GS018_BLS_VALID_SERIES; denominator from the chain (getProducers status==active), never the node count. $(_gs018_new_warn_count) unverifiable-BLS-half warning(s) over ~${GS018_WINDOW_SECS}s, which separates a producer that emits nothing from one whose half does not verify"
        return 1
    fi
    INFO_REASONS="$INFO_REASONS; $t: $m/$active active producer(s) emit a verifying BLS half —$matched. Denominator from the chain (getProducers status==active, $registered registered), never the node count; union read over $capable of $nodes node(s) exposing $GS018_BLS_VALID_SERIES"
    return 0
}

_gs018_postah_check() {
    local t="$1" p body v h heights n crossed="" surface=0 rejected=0 r verified=0
    for p in $GS018_METRICS_PORTS; do
        body="$(_gs018_metrics "$p")"
        [ -n "$body" ] || continue
        v="$(printf '%s\n' "$body" | awk -v s="$GS018_VERIFY_SERIES" '$1==s {print $2; exit}')"
        [ -n "$v" ] || continue
        surface=$(( surface + 1 ))
        verified=$(( verified + $(printf '%s' "$v" | tr -dc '0-9') ))
        r="$(printf '%s\n' "$body" | awk '/^doli_attestation_verify_rejected_total/ {s+=$NF} END {print s+0}')"
        rejected=$(( rejected + $(printf '%s' "$r" | tr -dc '0-9') ))
    done
    [ "$verified" -gt 0 ] && crossed="$crossed $GS018_VERIFY_SERIES=$verified"
    for p in $GS018_UP; do
        body="$(_gs018_rpc "$p" getAttestationStats '{}')"
        [ -n "$body" ] || continue
        n="$(printf '%s' "$body" | _gs018_scalar blocksWithBls | tr -dc '0-9')"
        if [ -n "$n" ] && [ "$n" -gt 0 ]; then
            crossed="$crossed blocksWithBls=$n@$p"
            break
        fi
    done
    if [ -z "$crossed" ]; then
        heights="$(_gs018_heights)"
        p="${GS018_UP# }"; p="${p%% *}"
        for h in $heights; do
            body="$(_gs018_rpc "$p" getBlockByHeight "{\"height\":$h}")"
            [ -n "$body" ] || continue
            case "$body" in *'"aggregateBlsSig"'*) crossed="$crossed aggregateBlsSig@h=$h"; break ;; esac
        done
    fi
    if [ -z "$crossed" ]; then
        SKIP_REASONS="$SKIP_REASONS; $t: pre-AH — inc_i_178_attestation_bls_activation_height is not crossed on this fleet (no $GS018_VERIFY_SERIES movement, no blocksWithBls, no aggregateBlsSig on a sampled block), so no aggregate exists to verify; no RPC exposes the height, these three are the litmus"
        return 2
    fi
    if [ "$surface" -eq 0 ]; then
        SKIP_REASONS="$SKIP_REASONS; $t: activation is crossed ($(printf '%s' "$crossed" | sed 's/^ //')) but no node exposes $GS018_VERIFY_SERIES on /metrics — the fleet is not on the INC-I-178 build, so the verify counters cannot be read"
        return 2
    fi
    if [ "$rejected" -gt 0 ]; then
        FAIL_REASONS="$FAIL_REASONS; $t: $rejected aggregate attestation(s) rejected across $surface node(s) (doli_attestation_verify_rejected_total > 0) while activation is crossed ($(printf '%s' "$crossed" | sed 's/^ //')) — a produced aggregate failed verification on at least one node"
        return 1
    fi
    INFO_REASONS="$INFO_REASONS; $t: activation crossed ($(printf '%s' "$crossed" | sed 's/^ //')); $verified aggregate verification(s) over $surface node(s) with 0 rejections"
    return 0
}

# ── dispatcher ──────────────────────────────────────────────────────────────
# rc 0 PASS · 1 FAIL · 2 SKIP, plus the caller-owned FAIL/SKIP/INFO_REASONS.

_gs018_assert() {
    local t="${1:-}"
    case "$t" in
        gs018-presence-root-consistent|gs018-active-producers-dual-sign|gs018-post-ah-aggregate-verifies) ;;
        *)
            FAIL_REASONS="$FAIL_REASONS; $t: unknown GS-018 assertion token"
            return 1 ;;
    esac
    _gs018_preflight "$t" || return 2
    case "$t" in
        gs018-presence-root-consistent) _gs018_root_check "$t"; return $? ;;
        gs018-active-producers-dual-sign) _gs018_dual_check "$t"; return $? ;;
        gs018-post-ah-aggregate-verifies) _gs018_postah_check "$t"; return $? ;;
    esac
    FAIL_REASONS="$FAIL_REASONS; $t: unknown GS-018 assertion token"
    return 1
}

# ── standalone ──────────────────────────────────────────────────────────────
# gauntlet.sh has no single-scenario filter, so running GS-018 on its own goes
# through here. Prints the runner's own result shape (gauntlet.sh:673-682).

_gs018_main() {
    local t rc s_ok=1
    [ "${1:-}" = "--quick" ] && GS018_SAMPLE=3
    FAIL_REASONS=""; SKIP_REASONS=""; INFO_REASONS=""
    for t in gs018-presence-root-consistent gs018-active-producers-dual-sign \
             gs018-post-ah-aggregate-verifies; do
        _gs018_assert "$t"; rc=$?
        { [ "$rc" = "0" ] || [ "$rc" = "2" ]; } || s_ok=0
    done
    if [ "$s_ok" = "1" ]; then
        printf "  PASS %-5s %-32s %s\n" "[obs]" "GS-018" "attestation-bitfield-integrity"
    else
        printf "  FAIL %-5s %-32s %s\n" "[obs]" "GS-018" "attestation-bitfield-integrity"
        printf "       %s\n" "${FAIL_REASONS# ; }"
    fi
    [ -n "$SKIP_REASONS" ] && printf "       skip:%s\n" "${SKIP_REASONS# ;}"
    [ -n "$INFO_REASONS" ] && printf "       note:%s\n" "${INFO_REASONS# ;}"
    [ "$s_ok" = "1" ] || return 1
    return 0
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    _gs018_main "$@"
    exit $?
fi
