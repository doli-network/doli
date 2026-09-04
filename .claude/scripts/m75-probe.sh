#!/usr/bin/env bash
# INC-I-178 M7.5 outcome probe (REQ-BLS-006 AC-2 / architecture precondition P3).
#
# METRIC  DUAL_SIGN_RATIO — of the producers the CHAIN reports as active, how many
#         are OBSERVED emitting a BLS half that a peer actually verified. This is
#         the number GS-018's dual-signing sub-assertion reports. It is external:
#         it is read from the live fleet's Prometheus exposition and the chain's
#         own getProducers reply, never from a test or a verdict.
#
# SIGNAL  doli_attestation_bls_valid_attester_total{attester="<8 hex>"} — one
#         series per attester whose first verifying 96-byte half this node pooled.
#         Its ABSENCE across the whole fleet is what "unmeasurable" means: the
#         fleet is on a build older than M7.5, so no per-producer emission signal
#         exists and no ratio can be computed (the pre-M7.5 GS-018 SKIP).
#
# USAGE   bash .claude/scripts/m75-probe.sh
# ENV     M75_RPC_PORTS, M75_METRICS_PORTS, M75_TIMEOUT
set -u
RPC_PORTS="${M75_RPC_PORTS:-8500 8501 8502 8503 8504 8505 8506 8507 8508 8509 8510 8511 8512 8513 8514 8515 8516 8517}"
MET_PORTS="${M75_METRICS_PORTS:-9000 9001 9002 9003 9004 9005 9006 9007 9008 9009 9010 9011 9012 9013 9014 9015 9016 9017}"
TIMEOUT="${M75_TIMEOUT:-4}"
MARKER="doli_attestation_bls_valid_total"
SERIES="doli_attestation_bls_valid_attester_total"

# Active producer pubkeys straight from the chain (status==active), first match wins.
ACTIVE_KEYS=""
for p in $RPC_PORTS; do
    body="$(curl -s -m "$TIMEOUT" -X POST -H 'Content-Type: application/json' \
        -d '{"jsonrpc":"2.0","id":1,"method":"getProducers","params":{"active_only":false}}' \
        "http://127.0.0.1:$p" 2>/dev/null)"
    [ -n "$body" ] || continue
    ACTIVE_KEYS="$(printf '%s' "$body" | python3 -c '
import sys, json
try:
    d = json.load(sys.stdin)
    r = d.get("result", d) if isinstance(d, dict) else d
    ps = r.get("producers", r) if isinstance(r, dict) else r
    if not isinstance(ps, list):
        raise ValueError("shape")
    for q in ps:
        if isinstance(q, dict) and str(q.get("status", "")).lower() == "active":
            k = q.get("pubkey") or q.get("publicKey") or q.get("public_key") or ""
            if k:
                print(str(k).lower())
except Exception:
    pass' 2>/dev/null)"
    [ -n "$ACTIVE_KEYS" ] && break
done
ACTIVE=$(printf '%s' "$ACTIVE_KEYS" | grep -c . || true)

# Union across the fleet of attester label prefixes with a non-zero count, plus
# the count of nodes that expose the zero-initialised capability marker.
MARKED=0
OBSERVED_PREFIXES=""
for p in $MET_PORTS; do
    body="$(curl -s -m "$TIMEOUT" "http://127.0.0.1:$p/metrics" 2>/dev/null)"
    [ -n "$body" ] || continue
    printf '%s\n' "$body" | grep -qE "^${MARKER}([[:space:]{]|\$)" && MARKED=$(( MARKED + 1 ))
    OBSERVED_PREFIXES="$OBSERVED_PREFIXES
$(printf '%s\n' "$body" | awk -v s="$SERIES" '
        index($0, s"{") == 1 && $NF + 0 > 0 {
            if (match($0, /attester="[0-9a-fA-F]+"/)) {
                v = substr($0, RSTART + 10, RLENGTH - 11); print tolower(v)
            }
        }')"
done
OBSERVED_PREFIXES="$(printf '%s\n' "$OBSERVED_PREFIXES" | grep . | sort -u || true)"

MATCHED=0
MATCHED_LIST=""
for k in $ACTIVE_KEYS; do
    for pre in $OBSERVED_PREFIXES; do
        case "$k" in
            "$pre"*) MATCHED=$(( MATCHED + 1 )); MATCHED_LIST="$MATCHED_LIST $pre"; break ;;
        esac
    done
done

echo "CAPABILITY_MARKER_NODES=$MARKED"
echo "DUAL_SIGN_ACTIVE=$ACTIVE"
echo "DUAL_SIGN_OBSERVED=$MATCHED"
echo "DUAL_SIGN_ATTESTERS=$(printf '%s' "$MATCHED_LIST" | sed 's/^ //')"
if [ "$MARKED" -eq 0 ]; then
    echo "DUAL_SIGN_RATIO=UNMEASURABLE"
elif [ "$ACTIVE" -eq 0 ]; then
    echo "DUAL_SIGN_RATIO=UNMEASURABLE"
else
    echo "DUAL_SIGN_RATIO=$MATCHED/$ACTIVE"
fi
