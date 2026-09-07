#!/usr/bin/env bash
# INC-I-171 M5 outcome probe (run 547).
#
# Metric: what the node DOES with the INC-I-171 reproduction transaction -- a
# producer paying out 100% of a Q1 (unvested) bond -- at the two node-local
# sites that decide whether it ever reaches consensus, at a height inside the
# inc_i_171 enforcement window:
#
#   BUILD      the content of the block this node assembles.
#              INCLUDED -> the node builds a block every peer must reject.
#              EXCLUDED -> the builder drops it before broadcast.
#   ADMISSION  the verdict the submitter sees from the mempool.
#              ACCEPTED -> the tx sits in the pool and is relayed.
#              REJECTED(<CODE>) -> refused at submit with the error code.
#
# Both are externally observable: BUILD is the block body a peer receives,
# ADMISSION is the RPC answer a submitter reads. Neither is a test count, a
# coverage figure or a reviewer verdict.
#
# Each test prints its verdict line UNCONDITIONALLY before asserting, so the
# probe returns a value on both the pre-fix (failing) and post-fix (passing)
# tree.
set -uo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

# `|| true`: on the pre-fix tree these tests FAIL, and that failure is the
# measurement, not an error.
BUILD_OUT=$( { cargo test -q -p doli-node --test inc_i_171_m5_vesting_build \
                 req_vest_008_m5_full_value_q1_withdrawal_is_excluded_from_the_built_block \
                 -- --nocapture 2>/dev/null || true; } )
ADMIT_OUT=$( { cargo test -q -p mempool --test inc_i_171_m5_vesting_admission \
                 req_vest_008_m5_full_value_q1_withdrawal_is_rejected_at_admission \
                 -- --nocapture 2>/dev/null || true; } )

BUILD=$(printf '%s\n' "$BUILD_OUT" | sed -n 's/^M5-PROBE-VERDICT-BUILD: //p' | head -1)
ADMIT=$(printf '%s\n' "$ADMIT_OUT" | sed -n 's/^M5-PROBE-VERDICT-ADMISSION: //p' | head -1)

printf 'BUILD=%s\n' "${BUILD:-UNKNOWN(probe found no build verdict line)}"
printf 'ADMISSION=%s\n' "${ADMIT:-UNKNOWN(probe found no admission verdict line)}"
