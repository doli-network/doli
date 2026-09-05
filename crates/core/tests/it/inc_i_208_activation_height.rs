//! INC-I-208 M3 — `inc_i_208_own_attestation_activation_height`: its per-network values
//! and its independence from every existing gate.
//!
// covers: crates/core/src/network_params/defaults.rs, Cargo.toml, Cargo.lock, inc_i_204_m5_activation_height
//!
//! Requirement: **REQ-208-006** (Must). The behaviour the gate withholds is measured in
//! `bins/node/tests/it/inc_i_208_own_attestation_pooled.rs`; this file pins only the
//! params surface.
//!
//! TDD RED, EXPECTED: this module does not compile against the tree at HEAD —
//! `NetworkParams::inc_i_208_own_attestation_activation_height` does not exist. That
//! compile failure is the red, exactly as `inc_i_204_m5_activation_height.rs` documents
//! for itself.
//!
//! WHY THIS GATE EXISTS. M1 made the attestation egress insert its own freshly-signed
//! BLS half into `parent_sig_pool`. Post-AH the presence bitfield is built only from that
//! pool, so the insert changes the producer's own bit, the aggregate it publishes and
//! therefore `presence_root` — a field inside `BlockHeader::hash()`. That is block
//! CONTENT. INV-DEPLOY-001 and CLAUDE.md require an activation height for it, and INV-12
//! Q3 is NO above the height, so the gate is REQUIRED rather than optional.
//!
//! WHY DEVNET IS FROZEN TOO. Unlike `inc_i_204_fork_choice_activation_height`, which is
//! `0` on devnet because fork choice is not block content, this gate IS block content: a
//! devnet default of `0` forks every live local chain on the next rebuild, because devnet
//! nodes keep their data directory across binaries. Devnet stays `u64::MAX`. INC-I-208
//! M3 pinned mainnet (409_000) and testnet (118_500), 2026-09-05; those heights are
//! IMMUTABLE once crossed (INC-I-054).
//!
//! PROCESS-WIDE HAZARD: this module deliberately calls only `NetworkParams::defaults`,
//! never `NetworkParams::load`. `load` caches per network in a process-wide `OnceLock`
//! and this is ONE test binary, so
//! `inc_i_204_m5_activation_height::the_env_override_is_locked_on_mainnet_and_honoured_elsewhere`
//! must remain its only caller.

// OUTPUT CONTRACT — ENUMERATION OF OBSERVABLE OUTPUTS.
//
//   F1: NetworkParams::defaults(Network) -> NetworkParams        (associated, PURE)
//       O1: .inc_i_208_own_attestation_activation_height <- the new field
//       O2: .inc_i_178_attestation_bls_activation_height <- the nearest neighbour and the
//           likeliest bundling target: same subsystem, and it is PINNED on testnet
//           (112_619), i.e. crossed history that must not move
//       O3: .inc_i_204_fork_choice_activation_height     <- pinned on testnet (88_014)
//       O4: .inc_i_147_activation_height                 <- crossed on mainnet (129_500)
//       (no mutable params, no receiver, no store writes — a pure constructor. The three
//        absent channels are declared rather than left unmentioned.)
//       PATHS: P-Mainnet, P-Testnet, P-Devnet (the three struct literals in defaults.rs).
//   MATRIX:
//       O1 x P-Mainnet -> 409_000, x P-Testnet -> 118_500, x P-Devnet -> u64::MAX
//       O2,O3,O4 x P-Mainnet after writing O1 -> unmoved           [independence test]
//   INPUT PARTITIONS: the independence test writes a SENTINEL that is neither `0`,
//   `u64::MAX`, nor any shipped gate value, so an alias cannot pass by coincidence.
//   NOT CLAIMED HERE: the env-override surface. `NetworkParams::load` has exactly one
//   caller in this binary (see the header) and stealing it would void that test.

use doli_core::network_params::NetworkParams;
use doli_core::Network;

/// Neither `0`, nor `u64::MAX`, nor any shipped activation height — so no assertion below
/// can pass because two fields happen to share a value.
const SENTINEL: u64 = 208_208;

// INC-I-208 M3 — Decision: renamed from `..._is_frozen_on_every_network`; the
// 2026-09-05 user decision pinned mainnet and testnet, so devnet alone stays frozen.
/// REQ-208-006 — Decision: a failure means a pinned height moved, OR devnet's freeze was
/// lifted with no decision-session. Once crossed, an activation height is IMMUTABLE
/// consensus history (INC-I-054 / INV-PARAMS-001) — the literals below are the tripwire.
/// Devnet stays frozen: a default off `u64::MAX` forks every live local chain on the
/// next rebuild, because the change alters block CONTENT (`presence_root`) and devnet
/// nodes keep their data directory across binaries.
#[test]
fn req_208_006_the_own_attestation_gate_is_pinned_on_mainnet_and_testnet_and_frozen_on_devnet() {
    for (network, expected) in [
        (Network::Mainnet, 409_000),
        (Network::Testnet, 118_500),
        (Network::Devnet, u64::MAX),
    ] {
        assert_eq!(
            NetworkParams::defaults(network).inc_i_208_own_attestation_activation_height,
            expected,
            "{network:?}: expected {expected}. Pooling its own BLS half changes the bit \
             this producer sets, the aggregate it publishes and therefore presence_root \
             inside BlockHeader::hash() — block CONTENT, so INV-DEPLOY-001 applies. \
             Mainnet/testnet are PINNED and IMMUTABLE once crossed (INC-I-054); devnet \
             stays frozen for the same reason as the INC-I-178 gate."
        );
    }
}

/// REQ-208-006 — Decision: a failure means the new gate is an alias of, or was bundled
/// onto, a gate the chain has already crossed — moving the own-attestation gate would
/// then move `inc_i_178_attestation_bls_activation_height` (PINNED on mainnet at 409_000)
/// or `inc_i_147_activation_height` (CROSSED on mainnet at 129_500) with it. That is the
/// INC-I-054 shape exactly. Both neighbours share the SAME pinned value on mainnet, so
/// plain equality is useless here; independence is demonstrated by WRITING one field and
/// reading the others.
#[test]
fn req_208_006_the_own_attestation_gate_is_a_distinct_independently_settable_field() {
    let mut probe = NetworkParams::defaults(Network::Mainnet);

    let bls_before = probe.inc_i_178_attestation_bls_activation_height;
    let fork_choice_before = probe.inc_i_204_fork_choice_activation_height;
    let inc_147_before = probe.inc_i_147_activation_height;

    // Anti-vacuity: if a neighbour already held the sentinel, "unmoved" below would be
    // satisfied by an alias that had simply been written to the same value.
    assert_ne!(
        bls_before, SENTINEL,
        "precondition: the sentinel must not already be the BLS gate's value"
    );
    assert_ne!(
        fork_choice_before, SENTINEL,
        "precondition: the sentinel must not already be the fork-choice gate's value"
    );
    assert_ne!(
        inc_147_before, SENTINEL,
        "precondition: the sentinel must not already be inc_i_147's value"
    );

    probe.inc_i_208_own_attestation_activation_height = SENTINEL;

    assert_eq!(
        probe.inc_i_208_own_attestation_activation_height, SENTINEL,
        "the new gate must be a real, independently settable field"
    );
    assert_eq!(
        probe.inc_i_178_attestation_bls_activation_height, bls_before,
        "INV-PARAMS-001: moving the own-attestation gate must not move the \
         attestation-BLS gate. They are one subsystem, so bundling is the natural \
         mistake — and the BLS gate is PINNED on testnet at 112_619, immutable \
         consensus history (INC-I-054)."
    );
    assert_eq!(
        probe.inc_i_204_fork_choice_activation_height, fork_choice_before,
        "INV-PARAMS-001: the fork-choice gate is pinned on testnet (88_014) and must \
         not move"
    );
    assert_eq!(
        probe.inc_i_147_activation_height, inc_147_before,
        "INV-PARAMS-001: inc_i_147 is CROSSED on mainnet (129_500) and must not move"
    );
}
