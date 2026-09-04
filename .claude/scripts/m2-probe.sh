#!/usr/bin/env bash
# INC-I-178 M2 outcome probe (run 544).
#
# Metric: BLS authentication coverage of the attestation gossip path in the
# SHIPPED source tree. Any operator or reviewer can run this on a checkout and
# get the same numbers without running the test suite. It counts production
# source properties — never tests, coverage, or agent verdicts.
#
#   UNVERIFIED_POOL_SITES  — production sites that write the parent signature
#                            pool from raw wire bytes with no BLS verify in
#                            front of them (network_events.rs, both ingresses)
#   INGRESS_VERIFY_SITES   — production `bls_verify(` call sites in the node
#                            (the shared ingress body)
#   EGRESS_DUAL_SIGN_SITES — production `new_with_bls` call sites (the single
#                            dual-signing egress)
#   SLOT_IN_BLS_MSG        — BLS attestation preimages that still append the
#                            slot suffix (R1 freezes the message to the block
#                            hash ALONE, so this must reach 0)
#   BLS_SCORING_REASON     — `InvalidBlsAttestation` references in the peer
#                            scorer (the invalid-BLS budget)
set -uo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

count() { # count() PATTERN PATHSPEC...
  local pat="$1"
  shift
  git grep -c -- "$pat" -- "$@" 2>/dev/null | awk -F: '{s+=$NF} END {print s+0}'
}

# Unverified pool writes: the M1 ingress clones the wire blob straight into the
# pool. D4 moves every write behind the shared verify, so network_events.rs must
# hold zero references to the pool afterwards.
echo "UNVERIFIED_POOL_SITES=$(count 'parent_sig_pool' 'bins/node/src/node/network_events.rs')"

# Per-signature BLS verification anywhere in the shipped node binary.
echo "INGRESS_VERIFY_SITES=$(count 'bls_verify(' 'bins/node/src')"

# The single dual-signing egress.
echo "EGRESS_DUAL_SIGN_SITES=$(count 'new_with_bls' 'bins/node/src')"

# R1: the frozen preimage is the block hash ALONE. Any BLS attestation message
# builder that still appends the slot keeps the pre-R1 format alive.
slot_msg=0
for f in $(git ls-files 'crates/crypto/src/bls.rs' 'crates/core/src/attestation/message.rs' 2>/dev/null); do
  n=$(awk '
    /fn attestation_message|fn bls_attest_msg/ {inmsg=1}
    inmsg && /slot\.to_be_bytes/ {c++; inmsg=0}
    /^}/ {inmsg=0}
    END {print c+0}
  ' "$f")
  slot_msg=$((slot_msg + n))
done
echo "SLOT_IN_BLS_MSG=$slot_msg"

# The invalid-BLS peer-scoring budget.
echo "BLS_SCORING_REASON=$(count 'InvalidBlsAttestation' 'crates/network/src/scoring.rs')"
