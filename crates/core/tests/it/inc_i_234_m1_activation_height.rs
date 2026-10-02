//! INC-I-234 M1 — `inc_i_234_activation_height` param plumbing: per-network values,
//! the fail-closed `ValidationContext` default + builder, and builder parity at every
//! production ctx site.
//!
// covers: crates/core/src/network_params/defaults.rs, crates/core/src/network_params/mod.rs, crates/core/src/validation/types.rs, bins/node/src/node/apply_block/tx_processing.rs, bins/node/src/node/production/assembly.rs, bins/node/src/node/validation_checks/mod.rs, crates/mempool/src/pool.rs
//!
//! Env-override lock (REQ-234-002) lives in `crates/core/src/network_params/tests_inc_i_234.rs`
//! because `env_loader::load_from_env` is `pub(super)`.
//!
//! PROCESS-WIDE HAZARD: calls only `NetworkParams::defaults`, never `NetworkParams::load`
//! (see `inc_i_204_m5_activation_height.rs`).
//!
//! TDD RED: does not compile until the field and builder exist.

// OUTPUT CONTRACT: fn NetworkParams::defaults(network: Network) -> NetworkParams
//   O1 return .inc_i_234_activation_height          <- the new field
//   O2 return .bls_key_rotation_activation_height   <- nearest neighbour, must be undisturbed
//   O3 mutable params / receiver / store writes: NONE (pure constructor)
//   PATHS: P-Mainnet | P-Testnet | P-Devnet (three struct literals)
// OUTPUT CONTRACT: fn ValidationContext::new(params, network, time, height) -> ValidationContext
//   O1 return .inc_i_234_activation_height <- u64::MAX
//   O2 / O3: NONE
//   PATHS: one (single struct literal)
// OUTPUT CONTRACT: fn ValidationContext::with_inc_i_234_activation_height(self, h: u64) -> Self
//   O1 return .inc_i_234_activation_height <- h
//   O2 return: other gate fields untouched
//   O3 receiver consumed by value
//   PATHS: one
// INPUT PARTITIONS
//   defaults: IP-M mainnet (612_000) | IP-T testnet (78_199) | IP-D devnet (0)
//   new:      current_height below / equal to / far above the testnet AH (0, 78_199, u64::MAX-1)
//   builder:  sentinel h distinct from every default (234_234)
//   sites:    each of the 4 files holding the 6 production ctx sites
// MATRIX
//   defaults  O1 x {IP-M, IP-T, IP-D}            -> req_234_001_per_network_values
//   defaults  O2 x {IP-M, IP-T, IP-D}            -> req_234_001_neighbour_gate_undisturbed
//   new       O1 x {0, 78_199, u64::MAX-1}       -> req_234_001_context_defaults_fail_closed
//   builder   O1, O2                             -> req_234_001_builder_sets_only_its_field
//   sites     builder call count per file        -> req_234_003_every_ctx_site_sets_the_gate

use doli_core::consensus::ConsensusParams;
use doli_core::network_params::NetworkParams;
use doli_core::validation::ValidationContext;
use doli_core::Network;

const CTX_SENTINEL: u64 = 234_234;

// REQ-234-001 — Decision: a failure means the gate ships live on mainnet, dormant on testnet, or unarmed on devnet.
#[test]
fn req_234_001_per_network_values() {
    for (network, expected) in [
        (Network::Mainnet, 612_000),
        (Network::Testnet, 78_199),
        (Network::Devnet, 0),
    ] {
        assert_eq!(
            NetworkParams::defaults(network).inc_i_234_activation_height,
            expected,
            "{network:?}: inc_i_234_activation_height must be {expected}"
        );
    }
}

// REQ-234-001 — Decision: a failure means the new field was bundled into / overwrote the INC-I-217 gate.
#[test]
fn req_234_001_neighbour_gate_undisturbed() {
    for (network, expected) in [
        (Network::Mainnet, 457_855),
        (Network::Testnet, 176_200),
        (Network::Devnet, u64::MAX),
    ] {
        let p = NetworkParams::defaults(network);
        assert_eq!(
            p.bls_key_rotation_activation_height, expected,
            "{network:?}"
        );
        if network != Network::Mainnet {
            assert_ne!(
                p.inc_i_234_activation_height, p.bls_key_rotation_activation_height,
                "{network:?}: own field, never reused"
            );
        }
    }
}

// REQ-234-001 — Decision: a failure means a ctx site that forgets the builder enforces the new rule early (fork).
#[test]
fn req_234_001_context_defaults_fail_closed() {
    for height in [0, 78_199, u64::MAX - 1] {
        let ctx = ValidationContext::new(ConsensusParams::mainnet(), Network::Mainnet, 0, height);
        assert_eq!(ctx.inc_i_234_activation_height, u64::MAX, "h={height}");
    }
}

// REQ-234-001 — Decision: a failure means the builder writes the wrong field or clobbers another gate.
#[test]
fn req_234_001_builder_sets_only_its_field() {
    let base = ValidationContext::new(ConsensusParams::mainnet(), Network::Mainnet, 0, 7);
    let rot_before = base.bls_key_rotation_activation_height;
    let sig_before = base.sig_verification_height;
    let height_before = base.current_height;

    let ctx = base.with_inc_i_234_activation_height(CTX_SENTINEL);

    assert_eq!(ctx.inc_i_234_activation_height, CTX_SENTINEL);
    assert_eq!(ctx.bls_key_rotation_activation_height, rot_before);
    assert_eq!(ctx.sig_verification_height, sig_before);
    assert_eq!(ctx.current_height, height_before);
}

const TX_PROCESSING: &str =
    include_str!("../../../../bins/node/src/node/apply_block/tx_processing.rs");
const ASSEMBLY: &str = include_str!("../../../../bins/node/src/node/production/assembly.rs");
const VALIDATION_CHECKS: &str =
    include_str!("../../../../bins/node/src/node/validation_checks/mod.rs");
const MEMPOOL_POOL: &str = include_str!("../../../mempool/src/pool.rs");

// REQ-234-003 — Decision: a failure means one ctx site disagrees with apply at h=AH (INC-I-173 class split).
#[test]
fn req_234_003_every_ctx_site_sets_the_gate() {
    for (name, src, expected) in [
        ("apply_block/tx_processing.rs", TX_PROCESSING, 1usize),
        ("production/assembly.rs", ASSEMBLY, 1),
        ("validation_checks/mod.rs", VALIDATION_CHECKS, 2),
        ("mempool/pool.rs", MEMPOOL_POOL, 2),
    ] {
        let anchor = src
            .matches(".with_bls_key_rotation_activation_height(")
            .count();
        assert_eq!(anchor, expected, "{name}: site census drifted; re-audit");
        let got = src.matches(".with_inc_i_234_activation_height(").count();
        assert_eq!(
            got, expected,
            "{name}: every ctx site must set inc_i_234_activation_height"
        );
    }
}
