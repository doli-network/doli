#!/usr/bin/env bash
# INC-I-178 M3 outcome probe (run 544).
#
# Metric: the CANONICAL attestation-universe surface published by the shipped
# `doli-core` library, plus the number of hand-rolled universe constructions
# that still ship in non-test source.
#
# Externally observable: any downstream consumer of the `doli-core` crate (the
# node, the CLI, the RPC crate, a third-party integrator) can call the exported
# function without running a single test. The numbers come from SHIPPED SOURCE
# and its public export chain, not from tests, coverage, or verdicts.
#
#   CANONICAL_UNIVERSE_FN     — `pub fn attestation_universe` definitions in
#                               crates/core/src (the one shared implementation)
#   CANONICAL_UNIVERSE_EXPORT — hops of the public export chain that actually
#                               re-export it (attestation/mod.rs, lib.rs).
#                               2 = reachable as `doli_core::attestation_universe`
#   HANDROLLED_UNIVERSE_SITES — non-test source sites that still build the
#                               `[base | (active \ base) sorted]` order inline.
#                               M3 must NOT move this (no call-site switch);
#                               M4/M5 drive it to 0 behind the activation height.
set -uo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

defs=$(grep -rhoc 'pub fn attestation_universe' crates/core/src 2>/dev/null \
  | awk '{s+=$1} END {print s+0}')
echo "CANONICAL_UNIVERSE_FN=${defs:-0}"

chain=0
grep -q 'attestation_universe' crates/core/src/attestation/mod.rs 2>/dev/null && chain=$((chain + 1))
grep -q 'attestation_universe' crates/core/src/lib.rs 2>/dev/null && chain=$((chain + 1))
echo "CANONICAL_UNIVERSE_EXPORT=$chain"

hand=0
for f in \
  bins/node/src/node/production/assembly.rs \
  bins/node/src/node/apply_block/post_commit.rs \
  crates/rpc/src/methods/schedule.rs \
  ; do
  n=$(grep -c 'sort_by(|a, b| a\(\.0\)\?\.as_bytes()\.cmp(' "$f" 2>/dev/null || true)
  hand=$((hand + ${n:-0}))
done
echo "HANDROLLED_UNIVERSE_SITES=$hand"
