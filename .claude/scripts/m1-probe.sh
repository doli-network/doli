#!/usr/bin/env bash
# INC-I-178 M1 outcome probe (run 544).
#
# Metric: the size of the DEAD attestation surface that ships inside the doli-core
# library — the minute-keyed BLS store, RegionAggregate, presence.rs, and the
# unreachable `h < BITFIELD_BODY_ACTIVATION_HEIGHT` legacy era.
#
# Externally observable: any operator or reviewer can run this on a checkout and
# get the same numbers without running the test suite. It counts SHIPPED SOURCE,
# not tests, coverage, or verdicts.
#
#   DEAD_SYMBOLS   — dead-surface symbols still DEFINED in tracked Rust source
#   LEGACY_ARMS    — `< BITFIELD_BODY_ACTIVATION_HEIGHT` comparison sites (all
#                    unsatisfiable: the constant is 0 and heights are u64)
#   SURFACE_LOC    — total LOC of the doli-core attestation + presence modules
#   MAX_MODULE_LOC — largest single file in that surface (CLAUDE.md #19 budget: < 500)
set -uo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

# Definition sites only (`git grep` sees the working tree, never .claude/worktrees or target).
dead=0
for pat in \
  'bls_sigs *:' \
  'fn bls_sigs_for_minute' \
  'fn bls_sig_count' \
  'struct RegionAggregate' \
  'fn from_attestations' \
  'struct PresenceCommitment' \
  'struct PresenceCommitmentV2' \
  ; do
  n=$(git grep -c -- "$pat" -- '*.rs' 2>/dev/null | awk -F: '{s+=$NF} END {print s+0}')
  dead=$((dead + n))
done
echo "DEAD_SYMBOLS=$dead"

arms=$(git grep -- '< *doli_core::consensus::BITFIELD_BODY_ACTIVATION_HEIGHT' -- '*.rs' 2>/dev/null | grep -vc '^bins/node/tests/' || true)
echo "LEGACY_ARMS=${arms:-0}"

files=$(git ls-files 'crates/core/src/attestation.rs' 'crates/core/src/attestation/*.rs' 'crates/core/src/presence.rs' 2>/dev/null)
if [ -z "$files" ]; then
  echo "SURFACE_LOC=0"
  echo "MAX_MODULE_LOC=0"
else
  # shellcheck disable=SC2086
  echo "SURFACE_LOC=$(cat $files | wc -l | tr -d ' ')"
  # shellcheck disable=SC2086
  echo "MAX_MODULE_LOC=$(wc -l $files | awk '$2!="total"{if($1>m)m=$1} END{print m+0}')"
fi
