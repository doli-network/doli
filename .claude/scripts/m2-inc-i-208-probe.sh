#!/usr/bin/env bash
# INC-I-208 M2 outcome probe (run 545).
#
# METRIC  The deploy-safety surface of the own-attestation pooling path, as an
#         operator can read it off the SHIPPED tree without running one test.
#
# Externally observable: every counter answers a question a node operator or an
# integrator asks before a deploy — "does this binary change what my producer
# EMITS at the height I am running, and on which networks is that change live?"
# No counter is a test count, a coverage percentage or a verdict.
#
#   GATE_FIELD       -- declarations of the INC-I-208 own-attestation activation
#         height on NetworkParams. 0 = the emitted-content change has no gate at
#         all, which is the INV-DEPLOY-001 violation M1 shipped.
#   FROZEN_NETWORKS  -- shipped default literals that freeze that gate at
#         u64::MAX. 3 = mainnet + testnet + devnet, i.e. no live network changes
#         block content until a separate pin decision-session.
#   PINNED_NETWORKS  -- shipped default literals that set a REAL height for the
#         gate. Must stay 0 in M2: pinning is never bundled onto the gating
#         commit (CLAUDE.md "activation heights", HC-6).
#   ENV_WIRED        -- env_loader entries for the gate, following the pattern
#         every other gate uses (mainnet locked, testnet/devnet overridable).
#   UNGATED_OWN_POOL_INSERT -- own-signature pool inserts on the egress path in
#         startup.rs that no activation height guards. Each one is a producer
#         that changes its emitted bitfield bit / aggregate component /
#         presence_root the moment the binary lands, in a rolling window, at any
#         height. 1 = the M1 defect. 0 is the only deploy-safe value.
#
# USAGE   .claude/scripts/m2-inc-i-208-probe.sh
set -uo pipefail
cd /Users/isudoajl/ownCloud/Projects/doli-network/doli || exit 1

FIELD=inc_i_208_own_attestation_activation_height
PARAMS=crates/core/src/network_params/mod.rs
DEFAULTS=crates/core/src/network_params/defaults.rs
ENVL=crates/core/src/network_params/env_loader.rs
EGRESS=bins/node/src/node/startup.rs

# grep -c prints 0 and exits 1 on no-match; take the count, ignore the status.
count() { local n; n=$(grep -cE "$1" "$2" 2>/dev/null); echo "${n:-0}"; }

GATE_FIELD=$(count "^[[:space:]]*pub ${FIELD}: u64," "$PARAMS")
FROZEN_NETWORKS=$(count "^[[:space:]]*${FIELD}: u64::MAX," "$DEFAULTS")
TOTAL_DEFAULTS=$(count "^[[:space:]]*${FIELD}:" "$DEFAULTS")
PINNED_NETWORKS=$(( TOTAL_DEFAULTS - FROZEN_NETWORKS ))
ENV_WIRED=$(count "^[[:space:]]*${FIELD}: if is_mainnet" "$ENVL")

INSERT_SITES=$(count "parent_sig_pool\.insert" "$EGRESS")
GATE_READS=$(count "$FIELD" "$EGRESS")
GATED_SITES=$(( GATE_READS > 0 ? INSERT_SITES : 0 ))
UNGATED=$(( INSERT_SITES - GATED_SITES ))

echo "GATE_FIELD=${GATE_FIELD}"
echo "FROZEN_NETWORKS=${FROZEN_NETWORKS}"
echo "PINNED_NETWORKS=${PINNED_NETWORKS}"
echo "ENV_WIRED=${ENV_WIRED}"
echo "UNGATED_OWN_POOL_INSERT=${UNGATED}"
