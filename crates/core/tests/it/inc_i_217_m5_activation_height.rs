//! INC-I-217 M5 — `bls_key_rotation_activation_height`: its per-network values, its
//! independence from every gate the chain has already crossed, and its fail-closed
//! default inside `ValidationContext`.
//!
// covers: crates/core/src/network_params/defaults.rs, crates/core/src/network_params/mod.rs, crates/core/src/validation/types.rs, crates/updater/src/hardfork.rs
//!
//! Requirement: **REQ-ROT-004** (Must). The behaviour the gate withholds — a
//! `RotateBlsKey` transaction reaching consensus — is measured in
//! `inc_i_217_m5_block_gate.rs` and `inc_i_217_m5_rotate_stateless.rs`; this file pins
//! only the params + context surface.
//!
//! TDD RED, EXPECTED: this module does not compile against the tree at HEAD —
//! `NetworkParams::bls_key_rotation_activation_height`,
//! `ValidationContext::bls_key_rotation_activation_height` and
//! `ValidationContext::with_bls_key_rotation_activation_height` do not exist yet. That
//! compile failure is the red, exactly as `inc_i_208_activation_height.rs` and
//! `inc_i_204_m5_activation_height.rs` document for themselves.
//!
//! WHY THIS GATE EXISTS. `TxType::RotateBlsKey` installs a new BLS attestation key for a
//! registered producer. A user-submittable transaction reaches the path (INV-12 Q1 = YES)
//! and the post-gate verdict on that transaction is NOT bit-identical to the pre-gate
//! verdict (Q3 = NO), so an activation height is REQUIRED, not optional. M5 ships the
//! gate FROZEN at `u64::MAX` on all three networks: pinning a real height is a separate
//! decision-session (HC-6 / INC-I-075), and a devnet default of `0` would fork every live
//! local chain on the next rebuild because devnet nodes keep their data directory across
//! binaries — the same reasoning that keeps the INC-I-178 and INC-I-208 gates frozen
//! on devnet.
//!
//! PROCESS-WIDE HAZARD: this module deliberately calls only `NetworkParams::defaults`,
//! never `NetworkParams::load`. `load` caches per network in a process-wide `OnceLock`
//! and this is ONE test binary, so
//! `inc_i_204_m5_activation_height::the_env_override_is_locked_on_mainnet_and_honoured_elsewhere`
//! must remain its only caller.

// OUTPUT CONTRACT — ENUMERATION OF OBSERVABLE OUTPUTS.
//
//   F1: NetworkParams::defaults(Network) -> NetworkParams        (associated, PURE)
//       O1 return .bls_key_rotation_activation_height  <- the new field
//       O2 return .inc_i_208_own_attestation_activation_height <- nearest neighbour,
//          PINNED on mainnet (409_000) and testnet (118_500) = crossed history
//       O3 return .inc_i_178_attestation_bls_activation_height <- same subsystem (BLS
//          attestation keys), the likeliest bundling target; PINNED both networks
//       O4 return .inc_i_171_vesting_penalty_activation_height <- PINNED 418_000 /
//          133_640
//       O5 return .inc_i_204_fork_choice_activation_height     <- PINNED 409_000 /
//          88_014, and `0` on devnet (so "every neighbour is u64::MAX" cannot pass
//          this matrix by accident)
//       O6 mutable params / O7 receiver / O8 store writes / O9 statics / O10 channels:
//          NONE — a pure constructor. Declared rather than left unmentioned.
//       PATHS: P-Mainnet, P-Testnet, P-Devnet (the three struct literals in defaults.rs).
//   F2: ValidationContext::new(params, network, time, height) -> ValidationContext
//       O1 return .bls_key_rotation_activation_height <- MUST default to u64::MAX
//       O2 mutable params / O3 receiver / O4 store: NONE
//       PATHS: one, unconditional (a single struct literal).
//   F3: ValidationContext::with_bls_key_rotation_activation_height(self, h) -> Self
//       O1 return .bls_key_rotation_activation_height <- h
//       O2 return: every OTHER gate field, which must be untouched
//       O3 receiver: consumed by value, no aliasing channel
//       PATHS: one, unconditional (a `#[must_use]` builder).
//   F4: the shipped HardForkSchedule source text (read, not executed — `doli-updater`
//       depends on `doli-core`, so this crate's tests cannot link it)
//       O1: the number of `HardForkInfo` entries in `for_network`
//       PATHS: the one `for_network` body.
//
//   MATRIX 4 functions x 3 network paths: every cell above is claimed by a named test.
//   INPUT PARTITIONS:
//     I1 the three shipped networks (F1).
//     I2 a SENTINEL that is neither `0`, nor `u64::MAX`, nor any shipped gate value, so
//        no assertion can pass because two fields happen to share a value (F1 independence,
//        F3 round-trip).
//     I3 a second, different sentinel for the builder, so a round-trip cannot pass by
//        reading a field that was already at the written value.
//   NOT CLAIMED HERE: the env-override surface (`NetworkParams::load`) — see the header.

use doli_core::consensus::ConsensusParams;
use doli_core::network_params::NetworkParams;
use doli_core::validation::ValidationContext;
use doli_core::Network;

/// Neither `0`, nor `u64::MAX`, nor any shipped activation height.
const SENTINEL: u64 = 217_217;

/// A second, distinct sentinel for the `ValidationContext` builder.
const CTX_SENTINEL: u64 = 217_905;

/// The shipped hard-fork schedule, read as TEXT. `doli-updater` depends on `doli-core`,
/// so `doli-core`'s tests cannot link it; the source is the only reachable evidence.
const HARDFORK_SRC: &str = include_str!("../../../updater/src/hardfork.rs");

// REQ-ROT-004 — Decision: whether M5 shipped a live rotation gate on any network without
// the separate pinning decision-session — i.e. whether `RotateBlsKey` transactions became
// consensus-reachable the moment the binary rolled out. A failure on Devnet also means a
// live local chain forks on the next rebuild, because devnet keeps its data directory.
#[test]
fn req_rot_004_the_rotation_gate_is_frozen_on_every_network() {
    // INC-I-217 testnet pin (2026-09-12, tip 176_119): testnet asserts the exact pinned
    // value; mainnet and devnet stay FROZEN. Once crossed on testnet the value is IMMUTABLE
    // (INC-I-054 shape) — this triad is the tripwire against moving it.
    for (network, expected) in [
        (Network::Mainnet, u64::MAX),
        (Network::Testnet, 176_200),
        (Network::Devnet, u64::MAX),
    ] {
        assert_eq!(
            NetworkParams::defaults(network).bls_key_rotation_activation_height,
            expected,
            "{network:?}: BLS key rotation gate must be exactly {expected}. Mainnet stays \
             FROZEN: pinning a real height is a separate decision-session (HC-6 / \
             INC-I-075): the rule changes the verdict on a user-submittable transaction, \
             so INV-12 Q1=YES and Q3=NO. Devnet is frozen too — a `0` default forks every \
             live local chain on the next rebuild. Testnet was pinned 2026-09-12."
        );
    }
}

// REQ-ROT-004 — Decision: whether M5 moved a height the chain has ALREADY CROSSED. That is
// the INC-I-054 shape exactly (security_audit 27_547 -> 71_290 deactivated live security
// features). Mainnet values are consensus HISTORY and are immutable; the testnet values
// are crossed too. The literals below are read from the tree at HEAD, not invented.
#[test]
fn req_rot_004_the_neighbouring_gates_are_undisturbed_on_every_network() {
    let mainnet = NetworkParams::defaults(Network::Mainnet);
    assert_eq!(
        mainnet.inc_i_204_fork_choice_activation_height, 409_000,
        "INV-PARAMS-001: mainnet fork-choice gate is CROSSED consensus history"
    );
    assert_eq!(
        mainnet.inc_i_178_attestation_bls_activation_height, 409_000,
        "INV-PARAMS-001: mainnet attestation-BLS gate is CROSSED consensus history"
    );
    assert_eq!(
        mainnet.inc_i_208_own_attestation_activation_height, 409_000,
        "INV-PARAMS-001: mainnet own-attestation gate is CROSSED consensus history"
    );
    assert_eq!(
        mainnet.inc_i_171_vesting_penalty_activation_height, 418_000,
        "INV-PARAMS-001: mainnet vesting-penalty gate is CROSSED consensus history"
    );

    let testnet = NetworkParams::defaults(Network::Testnet);
    assert_eq!(testnet.inc_i_204_fork_choice_activation_height, 88_014);
    assert_eq!(testnet.inc_i_178_attestation_bls_activation_height, 112_619);
    assert_eq!(testnet.inc_i_208_own_attestation_activation_height, 118_500);
    assert_eq!(testnet.inc_i_171_vesting_penalty_activation_height, 133_640);

    let devnet = NetworkParams::defaults(Network::Devnet);
    assert_eq!(
        devnet.inc_i_204_fork_choice_activation_height, 0,
        "devnet activates fork choice from genesis — this row proves the matrix is not \
         satisfied by `every neighbour is u64::MAX`"
    );
    assert_eq!(devnet.inc_i_178_attestation_bls_activation_height, u64::MAX);
    assert_eq!(devnet.inc_i_208_own_attestation_activation_height, u64::MAX);
    assert_eq!(devnet.inc_i_171_vesting_penalty_activation_height, u64::MAX);
}

// REQ-ROT-004 — Decision: whether the rotation gate is an ALIAS of, or was bundled onto, a
// gate the chain has already crossed. Every neighbour below currently holds `u64::MAX` on
// at least one network and 409_000 on mainnet, so plain equality proves nothing:
// independence is demonstrated by WRITING the new field and re-reading the others.
#[test]
fn req_rot_004_the_rotation_gate_is_a_distinct_independently_settable_field() {
    let mut probe = NetworkParams::defaults(Network::Mainnet);

    let own_attestation_before = probe.inc_i_208_own_attestation_activation_height;
    let attestation_bls_before = probe.inc_i_178_attestation_bls_activation_height;
    let vesting_before = probe.inc_i_171_vesting_penalty_activation_height;
    let fork_choice_before = probe.inc_i_204_fork_choice_activation_height;

    // Anti-vacuity: if a neighbour already held the sentinel, "unmoved" below would be
    // satisfied by an alias that had simply been written to the same value.
    for (name, value) in [
        ("inc_i_208_own_attestation", own_attestation_before),
        ("inc_i_178_attestation_bls", attestation_bls_before),
        ("inc_i_171_vesting_penalty", vesting_before),
        ("inc_i_204_fork_choice", fork_choice_before),
    ] {
        assert_ne!(
            value, SENTINEL,
            "precondition: the sentinel must not already be {name}'s value"
        );
    }

    probe.bls_key_rotation_activation_height = SENTINEL;

    assert_eq!(
        probe.bls_key_rotation_activation_height, SENTINEL,
        "the rotation gate must be a real, independently settable field"
    );
    assert_eq!(
        probe.inc_i_208_own_attestation_activation_height, own_attestation_before,
        "INV-PARAMS-001: moving the rotation gate must not move the own-attestation gate \
         (mainnet 409_000, CROSSED)"
    );
    assert_eq!(
        probe.inc_i_178_attestation_bls_activation_height, attestation_bls_before,
        "INV-PARAMS-001: the attestation-BLS gate governs the same BLS keys this feature \
         rotates, so bundling is the natural mistake — and it is CROSSED on both networks"
    );
    assert_eq!(
        probe.inc_i_171_vesting_penalty_activation_height, vesting_before,
        "INV-PARAMS-001: the vesting-penalty gate is pinned at 418_000 and must not move"
    );
    assert_eq!(
        probe.inc_i_204_fork_choice_activation_height, fork_choice_before,
        "INV-PARAMS-001: the fork-choice gate is CROSSED on both networks"
    );
}

// REQ-ROT-004 — Decision: whether a `ValidationContext` built by a call site that never
// learned about this gate lands ABOVE it. Fail-closed is the only safe default: a site
// that forgets the builder must stay BELOW the gate forever, which is a liveness bug
// (rotations refused on that path) and never a silent consensus divergence.
#[test]
fn req_rot_004_validation_context_defaults_the_rotation_gate_to_fail_closed() {
    for height in [0u64, 1, 409_000, 1_000_000, u64::MAX - 1] {
        let ctx = ValidationContext::new(ConsensusParams::mainnet(), Network::Mainnet, 0, height);
        assert_eq!(
            ctx.bls_key_rotation_activation_height,
            u64::MAX,
            "ValidationContext::new must default the rotation gate to u64::MAX so a \
             construction site that forgets the builder stays BELOW the gate; at \
             current_height={height} it did not"
        );
        assert!(
            ctx.current_height < ctx.bls_key_rotation_activation_height,
            "fail-closed means `current_height < gate` for EVERY representable height \
             a default context can carry (height={height})"
        );
    }
}

// REQ-ROT-004 — Decision: whether the plumbing builder actually reaches the field the
// validator reads, and whether it silently clobbers a neighbouring gate on the way. All
// six production plumbing sites (mempool x2, block validation x2, builder, apply) depend
// on this one setter; if it writes the wrong field the rule is unreachable on every path
// at once — the INV-VALIDATION-001 failure mode.
#[test]
fn req_rot_004_the_context_builder_round_trips_without_touching_other_gates() {
    let base = ValidationContext::new(ConsensusParams::mainnet(), Network::Mainnet, 0, 7);

    let sig_before = base.sig_verification_height;
    let scheduler_before = base.inc_i_026_scheduler_activation_height;
    let fork_id_before = base.fork_id_activation_height;

    assert_ne!(
        base.bls_key_rotation_activation_height, CTX_SENTINEL,
        "anti-vacuity: the round-trip must not read a value the field already held"
    );

    let ctx = base.with_bls_key_rotation_activation_height(CTX_SENTINEL);

    assert_eq!(
        ctx.bls_key_rotation_activation_height, CTX_SENTINEL,
        "with_bls_key_rotation_activation_height must write the field the validator reads"
    );
    assert_eq!(
        ctx.sig_verification_height, sig_before,
        "the rotation builder must not move sig_verification_height"
    );
    assert_eq!(
        ctx.inc_i_026_scheduler_activation_height, scheduler_before,
        "the rotation builder must not move the scheduler gate"
    );
    assert_eq!(
        ctx.fork_id_activation_height, fork_id_before,
        "the rotation builder must not move the fork-id gate"
    );

    // Both sides of the gate must be REACHABLE through the builder, not just writable.
    let below = ValidationContext::new(ConsensusParams::mainnet(), Network::Mainnet, 0, 99)
        .with_bls_key_rotation_activation_height(100);
    assert!(below.current_height < below.bls_key_rotation_activation_height);

    let above = ValidationContext::new(ConsensusParams::mainnet(), Network::Mainnet, 0, 100)
        .with_bls_key_rotation_activation_height(100);
    assert!(above.current_height >= above.bls_key_rotation_activation_height);
}

// REQ-ROT-004 — Decision: whether M5 added a `HardForkSchedule` entry. `current_fork_id()`
// evaluates the schedule at `u64::MAX`, so ANY new entry changes `fork_id` IMMEDIATELY on
// every node that has the new binary — the running fleet partitions on the first gossip
// round instead of at a future height. CLAUDE.md: consensus changes use a constant gate,
// never the schedule. `doli-updater` depends on `doli-core`, so this crate cannot link it;
// the shipped source text is the only evidence reachable from here.
#[test]
fn req_rot_004_no_hardfork_schedule_entry_was_added_for_the_rotation_feature() {
    let start = HARDFORK_SRC.find("pub fn for_network").expect(
        "hardfork.rs must still define `for_network` — if it was renamed, this \
                 tripwire needs updating, not deleting",
    );
    let end = HARDFORK_SRC[start..]
        .find("#[cfg(test)]")
        .map_or(HARDFORK_SRC.len(), |offset| start + offset);
    let shipped = &HARDFORK_SRC[start..end];

    let entries = shipped.matches("schedule.add(HardForkInfo {").count();
    assert_eq!(
        entries, 2,
        "HardForkSchedule::for_network must still hold exactly the 2 shipped testnet \
         entries. `current_fork_id()` evaluates the schedule at u64::MAX, so a new entry \
         changes fork_id immediately and partitions the live fleet. Gate rotation with \
         `bls_key_rotation_activation_height` instead."
    );
    assert!(
        shipped.contains("activation_height: 3_100"),
        "the shipped testnet entry at 3_100 must be untouched"
    );
    assert!(
        shipped.contains("activation_height: 4_836"),
        "the shipped testnet entry at 4_836 must be untouched"
    );
    assert!(
        !HARDFORK_SRC.contains("RotateBlsKey") && !HARDFORK_SRC.contains("bls_key_rotation"),
        "the rotation feature must not appear in the hard-fork schedule at all"
    );
}
