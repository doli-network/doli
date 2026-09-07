#!/usr/bin/env bash
# INC-I-171 M4 outcome probe (run 547).
#
# Metric: the node's own verdict on the INC-I-171 reproduction block -- a
# producer paying out 100% of a Q1 (unvested) bond, in a block at a height
# inside the inc_i_171 enforcement window.
#
# Externally observable: this is the consensus decision a node broadcasts by
# accepting or rejecting a block. It is not a test count, a coverage figure or
# a reviewer verdict -- it is the behaviour an integrator observes when the
# block reaches a node, printed by the node's own validator through
# validate_block_economics().
#
#   ACCEPTED                                        -- the INC-I-171 loss is live
#   REJECTED(ECON_WITHDRAWAL_PAYOUT_EXCEEDS_NET)    -- the bound is enforced
#
# The reproduction test prints the verdict unconditionally, before asserting,
# so the probe returns a value on both the pre-fix (failing) and post-fix
# (passing) tree.
set -uo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

# `|| true`: the pre-fix tree FAILS this test, and that failure is the
# measurement, not an error.
OUT=$( { cargo test -q -p doli-node --test inc_i_171_m4_vesting_bound \
           req_vest_001_m4_full_value_q1_withdrawal_is_rejected_at_activation \
           -- --nocapture 2>/dev/null || true; } )

VERDICT=$(printf '%s\n' "$OUT" | sed -n 's/^M4-PROBE-VERDICT: //p' | head -1)
printf '%s\n' "${VERDICT:-UNKNOWN(probe found no verdict line)}"
