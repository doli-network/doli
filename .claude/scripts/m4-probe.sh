#!/usr/bin/env bash
# INC-I-178 M4 outcome probe (run 544).
#
# Metric: the activation-height-gated attestation surface that the SHIPPED
# source publishes -- the forward-only NetworkParams activation height for the
# BLS attestation redesign, its three per-network defaults, the production
# sites that read the gate, the canonical presence-root commitment function,
# and how many of the five M4-due ParentSignaturePool accessors have a real
# production (non-test) call site.
#
# Externally observable: every counter is read off SHIPPED non-test source and
# the crate's public surface. A downstream consumer (node operator, CLI, RPC
# crate, third-party integrator) can read the activation height out of
# NetworkParams and reach `presence_commitment` from `doli_core` without
# running a single test. No counter is a test count, a coverage number, or a
# verdict.
#
#   ATTESTATION_BLS_AH_FIELD        -- declarations of
#         `pub inc_i_178_attestation_bls_activation_height: u64`
#         in crates/core/src/network_params/mod.rs. 0 = the height does not
#         exist; 1 = the forward-only gate is declared (D8, REQ-BLS-005).
#   ATTESTATION_BLS_AH_DEFAULTS_UMAX-- per-network defaults pinned to u64::MAX
#         in network_params/defaults.rs. MUST reach exactly 3 (mainnet,
#         testnet, devnet): a live devnet must not fork on rebuild, and the
#         pin is a separate decision session.
#   AH_GATED_ATTESTATION_SITES      -- non-test production source files that
#         read the gate (encoder, stray-bit width validator, post_commit
#         decoder, builder commitment). 0 = nothing is gated yet.
#   PRESENCE_COMMITMENT_FN          -- `pub fn presence_commitment`
#         definitions in crates/core/src (the ONE named pure commitment fn,
#         D6 / REQ-BLS-003).
#   POOL_ACCESSORS_WIRED            -- of the five M4-due ParentSignaturePool
#         accessors (signatures_for, contains_parent, parent_count,
#         total_signatures, clear), how many have >= 1 production call site
#         in bins/*/src or crates/*/src. This is the wiring-debt ledger
#         expressed as a number.
set -uo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

AH='inc_i_178_attestation_bls_activation_height'

field=$(grep -c "pub ${AH}: u64" crates/core/src/network_params/mod.rs 2>/dev/null || true)
echo "ATTESTATION_BLS_AH_FIELD=${field:-0}"

defaults=$(grep -c "${AH}: u64::MAX" crates/core/src/network_params/defaults.rs 2>/dev/null || true)
echo "ATTESTATION_BLS_AH_DEFAULTS_UMAX=${defaults:-0}"

gated=0
for f in \
  bins/node/src/node/production/assembly.rs \
  bins/node/src/node/attestation/commit.rs \
  bins/node/src/node/apply_block/post_commit.rs \
  bins/node/src/node/validation_checks.rs \
  ; do
  grep -q "${AH}\|attestation_bls_active" "$f" 2>/dev/null && gated=$((gated + 1))
done
echo "AH_GATED_ATTESTATION_SITES=$gated"

pc=$(grep -rhoc 'pub fn presence_commitment' crates/core/src 2>/dev/null \
  | awk '{s+=$1} END {print s+0}')
echo "PRESENCE_COMMITMENT_FN=${pc:-0}"

wired=0
for pat in \
  '\.signatures_for(' \
  '\.contains_parent(' \
  '\.parent_count(' \
  '\.total_signatures(' \
  'sig_pool[a-z_]*\(\)\?\.\(write()\.\(await\.\)\?\)\?clear()' \
  ; do
  n=$(grep -rl "$pat" --include=*.rs bins/node/src bins/cli/src crates/*/src 2>/dev/null | wc -l | tr -d ' ')
  [ "${n:-0}" -gt 0 ] && wired=$((wired + 1))
done
echo "POOL_ACCESSORS_WIRED=$wired"
