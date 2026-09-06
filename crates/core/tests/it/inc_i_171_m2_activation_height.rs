//! INC-I-171 M2 — the two vesting-penalty gates and the `vesting_quarter_slots` lock.
//!
// covers: crates/core/src/network_params/mod.rs, crates/core/src/network_params/defaults.rs, crates/core/src/network_params/env_loader.rs
//!
//! Requirements: **REQ-VEST-003** (Must — the rule ships behind its OWN new activation
//! height, frozen on every network) and **REQ-VEST-006** (Must — the tier comes from
//! `NetworkParams`, and that param is not operator-settable). The INV-PARAMS-002 triad.
//!
//! TDD RED, EXPECTED: this module does not compile against the tree at HEAD —
//! `NetworkParams::inc_i_171_vesting_penalty_activation_height` and
//! `…_disable_height` do not exist. That compile failure is the red, exactly as
//! `inc_i_208_activation_height.rs` documents for itself.
//!
//! EVERY EXPECTED VALUE IS READ FROM THE CONSTRUCTOR, never copied from the spec.
//! INC-I-201 and INC-I-209 were both caused by a test literal that had gone stale
//! against `defaults.rs`.
//!
//! PROCESS-WIDE HAZARD: this module calls only `NetworkParams::defaults`, never
//! `NetworkParams::load`. `load` caches per network in a process-wide `OnceLock` and
//! this is ONE test binary, so
//! `inc_i_204_m5_activation_height::the_env_override_is_locked_on_mainnet_and_honoured_elsewhere`
//! must remain its only caller — the behavioural half of the
//! `DOLI_VESTING_QUARTER_SLOTS` lock rides inside that function for the same reason the
//! INC-I-178 M4 assertions do. What is asserted HERE is the structural half.

// OUTPUT CONTRACT — ENUMERATION OF OBSERVABLE OUTPUTS.
//
//   F1: NetworkParams::defaults(Network) -> NetworkParams   (associated, PURE)
//       Mutable params: NONE. Receiver mutation: NONE. Store writes: NONE.
//       O1: .inc_i_171_vesting_penalty_activation_height  <- new
//       O2: .inc_i_171_vesting_penalty_disable_height     <- new, the paired undo
//       O3: .vesting_quarter_slots                        <- the guard's premise
//       O4: .inc_i_204_fork_choice_activation_height      <- PINNED mainnet+testnet
//       O5: .inc_i_178_attestation_bls_activation_height  <- PINNED mainnet+testnet
//       PATHS: P-Mainnet, P-Testnet, P-Devnet (the three struct literals).
//   F2: source text of env_loader.rs                      (structural tripwire)
//       O6: presence of the `DOLI_VESTING_QUARTER_SLOTS` override key.
//   MATRIX: O1,O2 x each path -> u64::MAX; O3 x each path -> non-zero, <= u32::MAX;
//           O4,O5 x {P-Mainnet, P-Testnet} -> unequal to O1/O2 and unmoved when O1 is
//           written; O6 -> absent.
//   INPUT PARTITIONS: the independence test writes a SENTINEL that is neither `0`,
//     `u64::MAX`, nor any shipped gate value, so an alias cannot pass by coincidence.
//   NOT CLAIMED HERE: the predicate's behaviour (`inc_i_171_m2_payout_bound.rs`) and the
//     per-bond arithmetic (`inc_i_171_m2_penalty_golden.rs`).

use doli_core::network_params::NetworkParams;
use doli_core::Network;

/// Neither `0`, nor `u64::MAX`, nor any shipped activation height.
const SENTINEL: u64 = 171_171;

const ENV_LOADER_RS: &str = include_str!("../../src/network_params/env_loader.rs");

/// REQ-VEST-003 — Decision: a failure says one of the two gates shipped ARMED. The rule
/// rejects blocks, so an unfrozen default on any network rejects live withdrawals the
/// moment the binary rolls out — and on mainnet the height becomes immutable consensus
/// history the instant it is crossed (INC-I-054). Devnet is frozen too: a devnet default
/// of `0` arms the rule against every local chain that keeps its data directory.
/// Pinning either height is a separate decision-session behind the Preconditions.
#[test]
fn req_vest_003_both_vesting_gates_are_frozen_on_every_network() {
    for network in [Network::Mainnet, Network::Testnet, Network::Devnet] {
        let params = NetworkParams::defaults(network);
        assert_eq!(
            params.inc_i_171_vesting_penalty_activation_height,
            u64::MAX,
            "{network:?}: the vesting-penalty rule must ship dormant"
        );
        assert_eq!(
            params.inc_i_171_vesting_penalty_disable_height,
            u64::MAX,
            "{network:?}: the paired undo must ship dormant too — a disable height below \
             the activation height would make the rule permanently unreachable"
        );
    }
}

/// REQ-VEST-003 — Decision: a failure says the new gate is an ALIAS of, or was bundled
/// onto, a gate the chain has already crossed. Moving the vesting gate would then move
/// `inc_i_204_fork_choice_activation_height` or
/// `inc_i_178_attestation_bls_activation_height`, both PINNED on mainnet and testnet —
/// that is the INC-I-054 shape exactly. Equality alone is not enough evidence, so
/// independence is also demonstrated by WRITING one field and reading the neighbours.
#[test]
fn req_vest_003_the_vesting_gates_are_not_bundled_onto_a_pinned_gate() {
    for network in [Network::Mainnet, Network::Testnet] {
        let params = NetworkParams::defaults(network);
        for (name, pinned) in [
            (
                "inc_i_204_fork_choice_activation_height",
                params.inc_i_204_fork_choice_activation_height,
            ),
            (
                "inc_i_178_attestation_bls_activation_height",
                params.inc_i_178_attestation_bls_activation_height,
            ),
        ] {
            assert_ne!(
                params.inc_i_171_vesting_penalty_activation_height, pinned,
                "{network:?}: the vesting gate must not share {name}'s pinned height"
            );
            assert_ne!(
                params.inc_i_171_vesting_penalty_disable_height, pinned,
                "{network:?}: the disable height must not share {name}'s pinned height"
            );
        }
    }

    let mut probe = NetworkParams::defaults(Network::Mainnet);
    let fork_choice_before = probe.inc_i_204_fork_choice_activation_height;
    let bls_before = probe.inc_i_178_attestation_bls_activation_height;
    let disable_before = probe.inc_i_171_vesting_penalty_disable_height;
    assert_ne!(
        fork_choice_before, SENTINEL,
        "precondition: the sentinel must not already be the fork-choice gate's value"
    );
    assert_ne!(
        bls_before, SENTINEL,
        "precondition: the sentinel must not already be the BLS gate's value"
    );

    probe.inc_i_171_vesting_penalty_activation_height = SENTINEL;

    assert_eq!(
        probe.inc_i_171_vesting_penalty_activation_height, SENTINEL,
        "the new gate must be a real, independently settable field"
    );
    assert_eq!(
        probe.inc_i_204_fork_choice_activation_height, fork_choice_before,
        "INV-PARAMS-001: arming the vesting gate must not move the fork-choice gate"
    );
    assert_eq!(
        probe.inc_i_178_attestation_bls_activation_height, bls_before,
        "INV-PARAMS-001: arming the vesting gate must not move the attestation-BLS gate"
    );
    assert_eq!(
        probe.inc_i_171_vesting_penalty_disable_height, disable_before,
        "the enable and disable heights are two fields, not one — otherwise arming the \
         rule disables it in the same assignment"
    );
}

/// REQ-VEST-006 — Decision: a failure says a shipped default IS the value the predicate
/// rejects, so `VestingQuarterInvalid` would fire on that network for every withdrawal
/// once the rule is armed — the zero guard would be a liveness bug rather than a
/// safeguard. Read from the constructor, never a literal: INC-I-201 and INC-I-209 were
/// both stale-literal failures.
#[test]
fn req_vest_006_the_shipped_vesting_quarter_is_never_a_value_the_predicate_rejects() {
    for network in [Network::Mainnet, Network::Testnet, Network::Devnet] {
        let quarter = NetworkParams::defaults(network).vesting_quarter_slots;
        assert_ne!(
            quarter, 0,
            "{network:?}: a zero quarter makes every armed withdrawal invalid"
        );
        assert!(
            quarter <= u32::MAX as u64,
            "{network:?}: quarter {quarter} does not fit in Slot (u32) and the predicate \
             rejects it"
        );
    }
}

/// REQ-VEST-006 — Decision: a failure says `DOLI_VESTING_QUARTER_SLOTS` is still an
/// override on some network. `vesting_quarter_slots` selects the penalty tier, so once
/// the rule is armed one operator with a `.env` file computes a different ceiling from
/// its peers and rejects (or accepts) blocks nobody else does — a chain split with no
/// code change and no deploy. Precondition PC-3/T4. Verified read-only before writing
/// this test: the variable is set nowhere in `~/testnet`, `~/Library/LaunchAgents` or
/// `scripts/`, so locking every network breaks no running node.
///
/// STRUCTURAL half only. The behavioural half — `NetworkParams::load` returning the
/// compiled default on all three networks with the variable set — rides in
/// `inc_i_204_m5_activation_height::the_env_override_is_locked_on_mainnet_and_honoured_elsewhere`,
/// because `load` caches per network in a process-wide `OnceLock` and a second caller in
/// this binary would silently void whichever test lost the race.
#[test]
fn req_vest_006_the_vesting_quarter_env_override_is_removed_on_every_network() {
    assert!(
        ENV_LOADER_RS.contains("vesting_quarter_slots"),
        "anti-vacuity: the loader must still assign the field, or the assertion below \
         would pass because the file was renamed or emptied"
    );
    assert!(
        !ENV_LOADER_RS.contains("DOLI_VESTING_QUARTER_SLOTS"),
        "REQ-VEST-006: the override key must be gone from \
         crates/core/src/network_params/env_loader.rs — locked on EVERY network, not \
         mainnet-only as at env_loader.rs:212-217"
    );
}
