#!/usr/bin/env bash
# ============================================================================
# inc-i-178-fleet-probe.sh — INC-I-178 M7 outcome-metric probe.
#
# Counts how many of the 18 local testnet nodes (seed + n1..n17, metrics on
# 9000 + 9000+i) carry the INC-I-178 build. Run it BEFORE and AFTER the deploy;
# the two counts are the milestone's outcome metric.
#
# The marker is a CAPABILITY, not a version: the INC-I-178 build reports 6.26.3,
# byte-identical to the fleet it replaces, and only it registers the series
# `doli_attestation_verify_total`. REGISTRATION is the signal — pre-AH that counter
# reads 0 on every new-build node, so a value-based predicate reports 0/18 on a
# successful deploy — and the match is anchored on the whole series name because
# `doli_attestation_verify_rejected_total` contains it.
# ALWAYS exits 0: the pre-deploy run legitimately counts 0 and a rolling restart
# refuses connections by design. Read-only /metrics GET — never an RPC port, never
# a write. The final stdout line ends in a BARE integer, for `awk '{print $NF}'`.
# Env: PROBE_METRICS_PORTS (override of 9000..9017), PROBE_TIMEOUT (default 3).
# ============================================================================
set -u

PROBE_METRICS_PORTS="${PROBE_METRICS_PORTS:-9000 9001 9002 9003 9004 9005 9006 9007 9008 9009 9010 9011 9012 9013 9014 9015 9016 9017}"
PROBE_TIMEOUT="${PROBE_TIMEOUT:-3}"
PROBE_SERIES="doli_attestation_verify_total"

count=0
total=0
carrying=""
missing=""
unreachable=""

for port in $PROBE_METRICS_PORTS; do
    total=$(( total + 1 ))
    body="$(curl -sf --max-time "$PROBE_TIMEOUT" "http://127.0.0.1:$port/metrics" 2>/dev/null)"
    if [ -z "$body" ]; then
        unreachable="$unreachable $port"
        continue
    fi
    if printf '%s\n' "$body" | grep -qE "^${PROBE_SERIES}([[:space:]{]|\$)"; then
        count=$(( count + 1 ))
        carrying="$carrying $port"
    else
        missing="$missing $port"
    fi
done

echo "INC-I-178 fleet probe — capability marker: $PROBE_SERIES on http://127.0.0.1:<port>/metrics"
echo "  scanned      :$( [ "$total" -gt 0 ] && printf ' %s ports' "$total" || printf ' none')"
echo "  on new build :${carrying:- none}"
echo "  on old build :${missing:- none}"
echo "  unreachable  :${unreachable:- none}"
echo "nodes on the INC-I-178 build $count"

exit 0
