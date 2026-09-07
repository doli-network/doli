#!/usr/bin/env bash
# INC-I-171 M6 outcome probe (run 547).
#
# Metric: the two INC-I-171 shadow series in the node's RENDERED Prometheus
# text, after ONE INC-I-171 reproduction block (a full-raw-value Q1
# RequestWithdrawal) is validated BELOW the vesting activation height.
#
# Externally observable: the bytes an operator gets from
# http://127.0.0.1:9000/metrics. The probe renders that exporter surface
# through the SAME prometheus::TextEncoder over the SAME global
# `doli_node::metrics::REGISTRY` the axum /metrics handler uses
# (bins/node/src/metrics.rs), so what it prints is what a scrape returns.
# It is not a test count, not a coverage percentage and not a verdict: the
# numbers are the counter values the exporter publishes.
#
# This is the INC-I-187 tripwire in probe form. INC-I-187 found 28/57 doli_*
# metrics registered but never written; a metric that only exists in the
# registry renders as a series that never moves. This probe therefore reads
# the value AFTER a hot-path block validation, not the registration list.
#
#   doli_vesting_would_reject_total{code="..."}  -- RequestWithdrawal txs the
#         INC-I-171 payout bound WOULD have rejected had the gate been live.
#         Precondition T3 for pinning ("zero honest would-rejects across >= 1
#         quarter boundary and >= 1 reorg") is read off this series.
#   doli_vesting_shadow_evaluated_total          -- RequestWithdrawal txs the
#         shadow actually evaluated. Distinguishes "zero would-rejects" from
#         "the shadow never ran": T3 is only evidence when this is non-zero.
#
# Usage: bash .claude/scripts/m6-inc-i-171-probe.sh
# Exit 0 always; the output IS the measurement.

set -uo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 0

TEST_BIN="inc_i_171_m6_shadow_counter"
TEST_FN="req_vest_011_metrics_render_carries_both_shadow_series"

RAW=$(cargo test -q -p doli-node --test "$TEST_BIN" "$TEST_FN" \
        -- --exact --nocapture 2>&1)

RENDERED=$(printf '%s\n' "$RAW" | grep -E '^M6-PROBE-AFTER ' | sed 's/^M6-PROBE-AFTER //')

echo "=== INC-I-171 M6 outcome probe — rendered /metrics shadow series ==="
for SERIES in doli_vesting_would_reject_total doli_vesting_shadow_evaluated_total; do
    LINE=$(printf '%s\n' "$RENDERED" | grep -E "^${SERIES}([{ ])" | head -3)
    if [ -z "$LINE" ]; then
        echo "${SERIES} ABSENT"
    else
        printf '%s\n' "$LINE"
    fi
done
echo "=== end ==="
