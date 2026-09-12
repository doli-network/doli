#!/usr/bin/env bash
# INC-I-217 M1 outcome probe (run 552).
#
# Metric: how many times the SHIPPED node's operator-facing BLS-mismatch
# diagnostics actually fire when a producer registered with BLS key A is
# driven through the attestation path by a node holding BLS key B.
#
# Externally observable: these are the node's own warn!() tokens, the same
# instrument the 2026-09-11 mainnet census used to identify the two live
# mismatched producers from seed logs (docs/.workflow/bls-mismatch-census-
# 2026-09-11.md). An operator greps them out of a running node's log file.
# The probe counts LOG EMISSIONS from production code paths -- not tests,
# not assertions, not verdicts, not coverage.
#
#   INGRESS_TOKEN -- "[ATTEST_INGEST] unverifiable BLS half from <attester>"
#         emitted by bins/node/src/node/attestation/ingress.rs when a received
#         attestation's BLS half fails against the attester's on-chain key.
#   EGRESS_TOKEN  -- "[ATTEST_EGRESS] own BLS half does not verify against the
#         on-chain key" emitted by bins/node/src/node/startup.rs when the node
#         cannot verify its OWN half before broadcast.
#
# Before M1 the tree has no way to drive either condition on demand, so both
# counters read 0 -- the diagnostics are unexercised code. After M1 the
# reproduction harness drives the condition and the tokens fire.
set -uo pipefail
cd "${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel)}" || exit 1

OUT=$(cargo test -p doli-node --test bls_rotation_repro -- --nocapture 2>&1)

INGRESS=$(printf '%s' "$OUT" | grep -c 'ATTEST_INGEST\] unverifiable BLS half')
EGRESS=$(printf '%s' "$OUT" | grep -c 'ATTEST_EGRESS\] own BLS half does not verify')

echo "=== INC-I-217 M1 outcome probe — shipped BLS-mismatch diagnostics fired ==="
echo "INGRESS_UNVERIFIABLE_BLS_HALF_EMISSIONS $INGRESS"
echo "EGRESS_OWN_BLS_HALF_MISMATCH_EMISSIONS $EGRESS"
echo "=== end ==="
