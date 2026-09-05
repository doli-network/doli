#!/usr/bin/env bash
# INC-I-178 M5 outcome probe (run 544).
#
# Metric: the aggregate-verification surface that the SHIPPED source publishes
# -- whether a post-AH BLS aggregate verifier exists, whether it is REACHABLE
# from the live apply funnel, and whether its observability is both REGISTERED
# and WRITTEN (INC-I-187 found 28/57 doli_* metrics never written).
#
# Externally observable: every counter is read off SHIPPED non-test source and
# the node's exported Prometheus surface. A node operator can scrape
# /metrics and see the doli_attestation_verify_* series without running a
# single test; an integrator can read the ONE call site in
# validate_block_for_apply. No counter is a test count, a coverage number or
# a verdict.
#
#   VERIFY_CALLSITE_IN_APPLY_FUNNEL -- non-test production call sites of
#         verify_block_attestation inside validation_checks.rs. MUST be
#         exactly 1 (D7: the ONE call site, the 86bac138 lesson).
#   VERIFY_DECISION_FN              -- `pub fn verify_block_attestation` /
#         `pub(crate) fn verify_block_attestation` definitions in
#         bins/node/src/node/attestation/verify.rs.
#   KEY_GATHER_FN                   -- set-bit BLS pubkey gatherer published by
#         bins/node/src/node/attestation/keys.rs (P2: key gathering lives in
#         node, NOT in core).
#   VERIFY_METRICS_REGISTERED       -- doli_attestation_verify_* series handed
#         to REGISTRY.register() in bins/node/src/metrics.rs. MUST reach 3
#         (total, rejected{reason}, skipped_light).
#   VERIFY_METRICS_WRITTEN          -- non-test production files that INCREMENT
#         those series. 0 with REGISTERED=3 is exactly the INC-I-187 defect.
#   VERIFY_REJECT_REASONS           -- distinct `reason` label values written in
#         shipped source. MUST reach 4 (root_mismatch, aggregate_invalid,
#         aggregate_nonempty_for_empty_bitfield, missing_bls_key).
set -uo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

cs=$(grep -c 'verify_block_attestation' bins/node/src/node/validation_checks.rs 2>/dev/null || true)
echo "VERIFY_CALLSITE_IN_APPLY_FUNNEL=${cs:-0}"

vf=$(grep -Ec 'fn verify_block_attestation' bins/node/src/node/attestation/verify.rs 2>/dev/null || true)
echo "VERIFY_DECISION_FN=${vf:-0}"

kf=$(grep -Ec '^pub(\(crate\))? fn ' bins/node/src/node/attestation/keys.rs 2>/dev/null || true)
echo "KEY_GATHER_FN=${kf:-0}"

reg=0
for s in doli_attestation_verify_total doli_attestation_verify_rejected_total doli_attestation_verify_skipped_light_total; do
  grep -q "\"$s\"" bins/node/src/metrics.rs 2>/dev/null && reg=$((reg + 1))
done
echo "VERIFY_METRICS_REGISTERED=$reg"

written=$(grep -rl 'ATTESTATION_VERIFY_TOTAL\|ATTESTATION_VERIFY_REJECTED\|ATTESTATION_VERIFY_SKIPPED_LIGHT' \
  --include="*.rs" bins/node/src crates 2>/dev/null | grep -v '/metrics.rs$' | wc -l | tr -d ' ')
echo "VERIFY_METRICS_WRITTEN=${written:-0}"

reasons=0
for r in root_mismatch aggregate_invalid aggregate_nonempty_for_empty_bitfield missing_bls_key; do
  grep -rq "\"$r\"" --include="*.rs" bins/node/src 2>/dev/null && reasons=$((reasons + 1))
done
echo "VERIFY_REJECT_REASONS=$reasons"
