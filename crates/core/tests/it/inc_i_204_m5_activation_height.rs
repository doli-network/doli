//! INC-I-204 M5 — `inc_i_204_fork_choice_activation_height`: its per-network values,
//! its independence from every existing gate, and the "nothing else moved" guard.
//!
//! Requirement: **REQ-FORK-014** (Must). Binding source for every literal below:
//! `docs/.workflow/inc-i-204-M5-design-brief.md` S2. Nothing here is re-litigated.
//!
//! TDD RED, EXPECTED: this module does not compile against the tree at HEAD —
//! `NetworkParams::inc_i_204_fork_choice_activation_height` does not exist. That compile
//! failure is the red, exactly as `crates/core/tests/inc_i_176_m2_activation_height.rs`
//! documents for itself.
//!
//! WHY M5 NEEDS A HEIGHT AT ALL. Not because block CONTENT changes — brief S11 verified
//! it does not, so INV-8 synchronized deploy is NOT triggered and the binary rolls. It
//! needs one because SELECTION changes: which of two equal-weight branches a node picks.
//! INV-12 Q2 is YES (a producer publishing a competing block at the same height creates
//! the input) and Q3 is NO above the height, so an activation height is REQUIRED.
//! `tests_m5_red_witness.rs` W1/W1b measure the divergence that makes Q3 NO.
//!
//! WHY BOTH LIVE NETWORKS ARE `u64::MAX`. Mainnet is frozen by HC-6 shape: pinning a
//! real height is a separate user decision-session. Testnet is frozen because the
//! architecture spec requires the gate to sit "above both tips with auto-update
//! convergence margin" but names neither a height nor a margin — no value is derivable
//! from the document, and inventing one is forbidden. Devnet is `0`, mirroring the
//! devnet arm of the gate it supersedes (`inc_i_147_activation_height`,
//! `defaults.rs:737`). Fork choice is not block content, so devnet `0` is not a genesis
//! reset (CLAUDE.md #0).
//!
//! ---------------------------------------------------------------------------
//! PROCESS-WIDE HAZARD — READ BEFORE ADDING A MODULE TO `tests/it/main.rs`
//! ---------------------------------------------------------------------------
//! `NetworkParams::load` caches per network in a `OnceLock`, process-wide, and this is
//! ONE test binary. [`the_env_override_is_locked_on_mainnet_and_honoured_elsewhere`]
//! must therefore be the only caller of `load` in the whole `it` binary. Verified when
//! this file was written: `inc_i_180_activation_height.rs` calls neither `load` nor
//! `set_var`. A future module that calls `load` first will silently void that test —
//! it will still pass, while asserting the cached defaults instead of the override.

// OUTPUT CONTRACT — ENUMERATION OF OBSERVABLE OUTPUTS.
//
//   F1: NetworkParams::defaults(Network) -> NetworkParams        (associated, PURE)
//       O1: .inc_i_204_fork_choice_activation_height  <- the new field
//       O2: .inc_i_147_activation_height              <- the superseded gate; read as
//           the anti-bundling counterparty AND pinned as "not moved" (CROSSED on
//           mainnet at 129_500 — immutable consensus history, INC-I-054)
//       O3: every OTHER *_activation_height field     <- the collateral-damage surface
//       (no mutable params, no receiver, no store writes — a pure constructor. The
//        three absent channels are declared rather than left unmentioned.)
//       PATHS: P-Mainnet, P-Testnet, P-Devnet (the three struct literals at
//         defaults.rs:18 / :332 / :610).
//
//   F2: NetworkParams::load(Network) -> &'static NetworkParams   (env + OnceLock)
//       O1: .inc_i_204_fork_choice_activation_height after env resolution
//       PATHS: P-locked (mainnet ignores the var), P-honoured (testnet/devnet parse it)
//       INPUT PARTITIONS: var set to a sentinel that is neither network's default.
//
//   MATRIX: 3 outputs x 3 networks for F1 (every cell claimed by a named test below),
//   plus 1 output x 2 paths for F2.

use doli_core::network_params::NetworkParams;
use doli_core::Network;

/// The env var name the loader must wire, derived from the field name exactly as every
/// other gate derives it (`DOLI_` + SCREAMING_SNAKE).
const ENV_VAR: &str = "DOLI_INC_I_204_FORK_CHOICE_ACTIVATION_HEIGHT";

// ===========================================================================
// O1 — the per-network values. Brief S2.
// ===========================================================================

/// REQ-FORK-014 — Decision: a failure means the new fork-choice rule is LIVE on mainnet
/// without a user decision-session ever happening; every mainnet node would start
/// answering weight ties differently from every node still on the old binary, splitting
/// the chain at the first tie.
#[test]
fn req_fork_014_mainnet_is_frozen_at_u64_max() {
    assert_eq!(
        NetworkParams::defaults(Network::Mainnet).inc_i_204_fork_choice_activation_height,
        u64::MAX,
        "O1 x P-Mainnet: FROZEN. Pinning a real mainnet height is a separate user \
         decision-session (HC-6 shape, brief S2). A test-writer, an architect or a \
         developer choosing a mainnet height here is the failure mode this pin exists \
         to make loud."
    );
}

/// REQ-FORK-014 — Decision: a failure means someone invented a testnet height. The
/// architecture spec requires the gate above both tips with an auto-update convergence
/// margin and names NEITHER, so any concrete value here is fabricated rather than
/// derived, and a fabricated value that lands below a tip is immediately immutable.
#[test]
fn req_fork_014_testnet_is_frozen_at_u64_max_because_no_height_is_derivable() {
    assert_eq!(
        NetworkParams::defaults(Network::Testnet).inc_i_204_fork_choice_activation_height,
        u64::MAX,
        "O1 x P-Testnet: FROZEN pending a user decision. The spec states no height and \
         no margin size (brief S2), so nothing in the document determines a value."
    );
}

/// REQ-FORK-014 — Decision: a failure means the M5 suite's post-activation cells are
/// never exercised by any real node, so the unified authority ships with no environment
/// running it; devnet is the only network where the post-AH branch is reachable during
/// the whole dormant window.
#[test]
fn req_fork_014_devnet_activates_from_genesis() {
    assert_eq!(
        NetworkParams::defaults(Network::Devnet).inc_i_204_fork_choice_activation_height,
        0,
        "O1 x P-Devnet: mirrors the devnet arm of the gate this supersedes \
         (inc_i_147_activation_height devnet = 0, defaults.rs:737). Devnet is a \
         disposable local chain and fork choice is not block content, so 0 is not a \
         genesis reset."
    );
}

// ===========================================================================
// ANTI-BUNDLING — INV-PARAMS-001 / INC-I-054.
// ===========================================================================

/// REQ-FORK-014 — Decision: a failure means the new rule was bundled onto
/// `inc_i_147_activation_height`, which is CROSSED on mainnet (129_500) and on testnet
/// (80_700). Bundling makes the new rule retroactive to a height the chain passed long
/// ago — every historical block would be re-selected under a rule that did not exist
/// when it was produced. This is precisely how INC-I-054 deactivated live security
/// features.
#[test]
fn req_fork_014_the_gate_is_not_bundled_onto_inc_i_147() {
    let m = NetworkParams::defaults(Network::Mainnet);
    let t = NetworkParams::defaults(Network::Testnet);

    assert_ne!(
        m.inc_i_204_fork_choice_activation_height, m.inc_i_147_activation_height,
        "mainnet: u64::MAX must not collapse onto 129_500 (crossed)"
    );
    assert_ne!(
        t.inc_i_204_fork_choice_activation_height, t.inc_i_147_activation_height,
        "testnet: u64::MAX must not collapse onto 80_700 (crossed)"
    );
    // The two live networks share a value with EACH OTHER (both u64::MAX) by design —
    // both are frozen. That is not bundling; bundling is sharing a value with a
    // DIFFERENT RULE on the same network, which is what the two assertions above forbid.
    assert_eq!(
        m.inc_i_204_fork_choice_activation_height, t.inc_i_204_fork_choice_activation_height,
        "mainnet and testnet are both frozen, so they legitimately agree; recorded here \
         so a reader does not mistake the agreement for a copy-paste"
    );
}

/// REQ-FORK-014 — Decision: a failure means the "new field" is an alias or a re-read of
/// an existing one, so moving the M5 gate would move a crossed gate with it. On devnet
/// both values are `0`, so equality proves nothing there — independence has to be
/// demonstrated by MOVING one and reading the other.
///
/// This is the non-vacuity half of the anti-bundling pair: without it, a developer who
/// wired `inc_i_204_fork_choice_activation_height` as a getter returning
/// `inc_i_147_activation_height` would pass every assertion above on mainnet and
/// testnet, because both happen to differ there for unrelated reasons.
#[test]
fn req_fork_014_the_gate_is_a_distinct_independently_settable_field() {
    let d = NetworkParams::defaults(Network::Devnet);
    assert_eq!(d.inc_i_204_fork_choice_activation_height, 0);
    assert_eq!(d.inc_i_147_activation_height, 0);

    // Both fields must exist on the same struct value and be separately assignable.
    // A `let mut` copy is enough: if one is an alias of the other, writing one changes
    // both and the second assertion fails.
    let mut probe = NetworkParams::defaults(Network::Devnet);
    probe.inc_i_204_fork_choice_activation_height = 4_242;
    assert_eq!(
        probe.inc_i_204_fork_choice_activation_height, 4_242,
        "the new gate must be settable"
    );
    assert_eq!(
        probe.inc_i_147_activation_height, 0,
        "INV-PARAMS-001: moving the M5 gate must NOT move inc_i_147. If this fails the \
         two are one field wearing two names, and the M5 gate cannot be pinned on \
         mainnet without also moving a gate crossed at height 129_500."
    );
}

// ===========================================================================
// O2/O3 — NOTHING ELSE MOVED.
//
// The M5 edit adds a field to three exhaustive struct literals (defaults.rs:18, :332,
// :610) and one more in env_loader.rs:38. That is exactly the edit that silently
// perturbs a neighbour, and a MAINNET neighbour already crossed is consensus history
// that can never be put back (INV-PARAMS-001 / INC-I-054).
//
// Values read out of crates/core/src/network_params/defaults.rs at the M5 branch point.
// ===========================================================================

/// REQ-FORK-014 — O2 by name, on all three networks. Decision: a failure means the
/// superseded gate itself moved while being superseded — the INC-I-054 shape exactly
/// (that incident moved `security_audit_activation_height` 27,547 -> 71,290 and
/// deactivated live security features on a chain that had already crossed it).
#[test]
fn req_fork_014_inc_i_147_the_gate_being_superseded_did_not_move() {
    assert_eq!(
        NetworkParams::defaults(Network::Mainnet).inc_i_147_activation_height,
        129_500,
        "O2: mainnet inc_i_147 is CROSSED and therefore IMMUTABLE"
    );
    assert_eq!(
        NetworkParams::defaults(Network::Testnet).inc_i_147_activation_height,
        80_700,
        "O2: testnet inc_i_147, likewise crossed"
    );
    assert_eq!(
        NetworkParams::defaults(Network::Devnet).inc_i_147_activation_height,
        0,
        "O2: devnet inc_i_147"
    );
}

/// REQ-FORK-014 — O3 x P-Mainnet. Every other activation height, pinned.
#[test]
fn req_fork_014_no_mainnet_activation_height_was_moved() {
    let p = NetworkParams::defaults(Network::Mainnet);

    assert_eq!(p.inc_i_026_scheduler_activation_height, 0);
    assert_eq!(p.fork_id_activation_height, 0);
    assert_eq!(p.encrypted_content_activation_height, 0);
    assert_eq!(p.encrypted_content_v2_activation_height, 0);
    assert_eq!(p.epoch_state_reorg_activation_height, 0);
    assert_eq!(p.security_audit_activation_height, 0);
    assert_eq!(p.ghost_exclusion_activation_height, 0);
    assert_eq!(p.epoch_prune_activation_height, 0);
    assert_eq!(p.inc_i_190_floor_bound_activation_height, 332_664);
    assert_eq!(p.inc_i_068_weight_filter_activation_height, 0);
    assert_eq!(p.received_delegation_cap_activation_height, 0);
    assert_eq!(p.delegation_auth_activation_height, 0);
    assert_eq!(p.addbond_cap_enforcement_activation_height, 0);
    assert_eq!(p.withdrawal_holdings_gate_activation_height, 317_861);
    assert_eq!(p.defi_activation_height, 0);
    assert_eq!(p.amm_activation_height, 0);
    assert_eq!(p.oracle_activation_height, u64::MAX);
    assert_eq!(p.large_block_activation_height, 0);
    assert_eq!(p.inc_i_092_activation_height, 0);
    assert_eq!(p.inc_i_096_activation_height, 0);
    assert_eq!(p.maintainer_derivation_activation_height, 172_000);
    assert_eq!(p.inc_i_173_activation_height, 317_861);
    assert_eq!(p.inc_i_176_auth_binding_activation_height, 317_861);
}

/// REQ-FORK-014 — O3 x P-Testnet.
#[test]
fn req_fork_014_no_testnet_activation_height_was_moved() {
    let p = NetworkParams::defaults(Network::Testnet);

    assert_eq!(p.inc_i_026_scheduler_activation_height, 0);
    assert_eq!(p.fork_id_activation_height, 0);
    assert_eq!(p.encrypted_content_activation_height, 0);
    assert_eq!(p.encrypted_content_v2_activation_height, 0);
    assert_eq!(p.epoch_state_reorg_activation_height, 0);
    assert_eq!(p.security_audit_activation_height, 0);
    assert_eq!(p.ghost_exclusion_activation_height, 0);
    assert_eq!(p.epoch_prune_activation_height, 0);
    assert_eq!(p.inc_i_190_floor_bound_activation_height, 58_000);
    assert_eq!(p.inc_i_068_weight_filter_activation_height, 0);
    assert_eq!(p.received_delegation_cap_activation_height, 0);
    assert_eq!(p.delegation_auth_activation_height, 0);
    assert_eq!(p.addbond_cap_enforcement_activation_height, 0);
    assert_eq!(p.withdrawal_holdings_gate_activation_height, 15_087);
    assert_eq!(p.defi_activation_height, u64::MAX);
    assert_eq!(p.amm_activation_height, 0);
    assert_eq!(p.oracle_activation_height, u64::MAX);
    assert_eq!(p.large_block_activation_height, 0);
    assert_eq!(p.inc_i_092_activation_height, 0);
    assert_eq!(p.inc_i_096_activation_height, 0);
    assert_eq!(p.maintainer_derivation_activation_height, 15_087);
    assert_eq!(p.inc_i_173_activation_height, 25_500);
    assert_eq!(p.inc_i_176_auth_binding_activation_height, 15_087);
}

/// REQ-FORK-014 — O3 x P-Devnet.
#[test]
fn req_fork_014_no_devnet_activation_height_was_moved() {
    let p = NetworkParams::defaults(Network::Devnet);

    assert_eq!(p.inc_i_026_scheduler_activation_height, 0);
    assert_eq!(p.fork_id_activation_height, 0);
    assert_eq!(p.encrypted_content_activation_height, 0);
    assert_eq!(p.encrypted_content_v2_activation_height, 0);
    assert_eq!(p.epoch_state_reorg_activation_height, 0);
    assert_eq!(p.security_audit_activation_height, 0);
    assert_eq!(p.ghost_exclusion_activation_height, 0);
    assert_eq!(p.epoch_prune_activation_height, 0);
    assert_eq!(p.inc_i_190_floor_bound_activation_height, 0);
    assert_eq!(p.inc_i_068_weight_filter_activation_height, 0);
    assert_eq!(p.received_delegation_cap_activation_height, u64::MAX);
    assert_eq!(p.delegation_auth_activation_height, u64::MAX);
    assert_eq!(p.addbond_cap_enforcement_activation_height, u64::MAX);
    assert_eq!(p.withdrawal_holdings_gate_activation_height, 20);
    assert_eq!(p.defi_activation_height, u64::MAX);
    assert_eq!(p.amm_activation_height, 0);
    assert_eq!(p.oracle_activation_height, u64::MAX);
    assert_eq!(p.large_block_activation_height, 0);
    assert_eq!(p.inc_i_092_activation_height, 0);
    assert_eq!(p.inc_i_096_activation_height, 0);
    assert_eq!(p.maintainer_derivation_activation_height, 0);
    assert_eq!(p.inc_i_173_activation_height, 0);
    assert_eq!(p.inc_i_176_auth_binding_activation_height, 20);
}

// ===========================================================================
// F2 — the env override.
// ===========================================================================

/// REQ-FORK-014 — Decision: a failure of the ANTI-VACUITY half means the variable was
/// never wired, so the mainnet-lock assertion below proves nothing; a failure of the
/// LOCK half means one operator can move mainnet's fork-choice gate from a `.env` file
/// and select a different branch from every peer — a self-inflicted chain split with no
/// consensus change and no deploy.
///
/// Both partitions in ONE test on purpose: `NetworkParams::load` caches per network in
/// a process-wide `OnceLock`, so the var must be set before the FIRST load of either
/// network in this binary. See the module header — nothing else in the `it` binary may
/// call `load`.
#[test]
fn the_env_override_is_locked_on_mainnet_and_honoured_elsewhere() {
    // Neither network's default, so neither branch can pass by coincidence.
    const SENTINEL: &str = "9";
    let original = std::env::var(ENV_VAR);
    std::env::set_var(ENV_VAR, SENTINEL);

    let mainnet = NetworkParams::load(Network::Mainnet).inc_i_204_fork_choice_activation_height;
    let devnet = NetworkParams::load(Network::Devnet).inc_i_204_fork_choice_activation_height;
    let testnet = NetworkParams::load(Network::Testnet).inc_i_204_fork_choice_activation_height;

    // Restore BEFORE asserting so a failure cannot leak process state into the rest of
    // the binary.
    match original {
        Ok(v) => std::env::set_var(ENV_VAR, v),
        Err(_) => std::env::remove_var(ENV_VAR),
    }

    assert_eq!(
        devnet, 9,
        "ANTI-VACUITY, asserted FIRST: {ENV_VAR} must be honoured on devnet. If this \
         fails the variable is not wired and the mainnet lock below is meaningless."
    );
    assert_eq!(
        testnet, 9,
        "ANTI-VACUITY: honoured on testnet too — this is how the testnet rehearsal will \
         arm the gate once a height is chosen, without a code change"
    );
    assert_eq!(
        mainnet,
        u64::MAX,
        "THE LOCK: mainnet activation heights are locked to the compiled default \
         (env_loader.rs:483-494 is the template). An override here would let one \
         operator arm a different fork-choice rule than the rest of the network."
    );
}
