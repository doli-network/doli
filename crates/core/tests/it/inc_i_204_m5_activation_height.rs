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
//! WHY TESTNET WAS PINNED FIRST, THEN MAINNET. Testnet WAS frozen for the reason the
//! file originally recorded — the architecture spec named neither a height nor a margin
//! — but that decision was TAKEN, not deferred, in commit `5b326fe9`:
//!
//!   "User decision 2026-09-02 at live tip 87,934 ... gate crossed unanimously at
//!    h=88,021 (18/18 hash-identical) and live-validated at h=88,054-88,055 ...
//!    88,014 is now IMMUTABLE consensus history on testnet (INC-I-054 rule)."
//!
//! INC-I-208 M3 — a SEPARATE user decision, 2026-09-05, pinned mainnet at 409_000
//! (alongside the INC-I-178 and INC-I-208 gates, all three to the same height). Both
//! pins are FORWARD-ONLY (CLAUDE.md #0): 88_014 was chosen ABOVE the live tip 87_934,
//! never retroactively from a height the chain had already passed (INC-I-054).
//! Devnet is `0`, mirroring the devnet arm of the gate it supersedes
//! (`inc_i_147_activation_height`, `defaults.rs:737`). Fork choice is not block content,
//! so devnet `0` is not a genesis reset (CLAUDE.md #0).
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

// INC-I-208 M3 — Decision: renamed from `req_fork_014_mainnet_is_frozen_at_u64_max`; the
// 2026-09-05 user decision pinned mainnet at 409_000, so the old name asserted a freeze
// that no longer exists.
/// REQ-FORK-014 — Decision: a failure means the pinned mainnet height moved off 409_000.
/// Once crossed, an activation height is IMMUTABLE consensus history (INC-I-054 /
/// INV-PARAMS-001) — the literal below is the tripwire against moving it either way.
#[test]
fn req_fork_014_mainnet_is_pinned_at_409_000() {
    assert_eq!(
        NetworkParams::defaults(Network::Mainnet).inc_i_204_fork_choice_activation_height,
        409_000,
        "O1 x P-Mainnet: PINNED 2026-09-05. Moving this off 409_000 either re-activates \
         the new fork-choice rule retroactively or unpins a height the fleet is about to \
         (or has already started to) converge on."
    );
}

/// The live tip when the testnet pin was chosen (commit `5b326fe9`). The gate had to
/// land strictly ABOVE it, or it would have been retroactive on crossing.
const TESTNET_TIP_AT_PIN: u64 = 87_934;

/// The pinned testnet gate. IMMUTABLE consensus history since h=88,021 (18/18 nodes
/// hash-identical), so this literal is a record of what the chain DID, not a choice.
const TESTNET_PINNED_GATE: u64 = 88_014;

// INC-I-203 M5 — Decision: renamed from
// `req_fork_014_testnet_is_frozen_at_u64_max_because_no_height_is_derivable`; the
// decision the old name deferred was TAKEN in `5b326fe9`, so the old name asserted a
// freeze that no longer exists and the test failed on the shipped params.
// INC-I-208 M3 — Decision: renamed from
// `..._and_mainnet_stays_frozen`; the 2026-09-05 decision pinned mainnet at 409_000, so
// "stays frozen" no longer holds. Both pins are IMMUTABLE once crossed (INC-I-054).
/// REQ-FORK-014 — Decision: a failure means the testnet fork-choice gate moved off the
/// height testnet ALREADY CROSSED at h=88,021, OR the mainnet pin moved off 409_000. Both
/// are IMMUTABLE consensus history / a settled decision-session; moving either re-selects
/// blocks under a rule that was not the one in force at production time.
///
/// Verbatim rationale from `5b326fe9`: "User decision 2026-09-02 at live tip 87,934 ...
/// gate crossed unanimously at h=88,021 (18/18 hash-identical) and live-validated at
/// h=88,054-88,055 ... 88,014 is now IMMUTABLE consensus history on testnet (INC-I-054
/// rule)." Mainnet was pinned separately, 2026-09-05, at 409_000.
#[test]
fn req_fork_014_testnet_is_pinned_at_the_crossed_height_and_mainnet_is_pinned_at_409_000() {
    let t = NetworkParams::defaults(Network::Testnet).inc_i_204_fork_choice_activation_height;

    assert_eq!(
        t, TESTNET_PINNED_GATE,
        "O1 x P-Testnet: testnet crossed this gate at h=88,021 with 18/18 nodes \
         hash-identical. 88_014 is IMMUTABLE consensus history (INC-I-054); any other \
         value re-selects the blocks produced since the crossing under a rule that was \
         not in force when they were produced."
    );

    // The two ways a re-pin goes wrong are named separately, so a failure says WHICH.
    assert_ne!(
        t, 0,
        "O1 x P-Testnet: 0 would apply the new fork-choice authority retroactively from \
         genesis — the exact INC-I-054 shape CLAUDE.md rule #0 forbids"
    );
    assert_ne!(
        t,
        u64::MAX,
        "O1 x P-Testnet: u64::MAX would UNPIN a gate testnet has already crossed, so \
         nodes on this binary would answer weight ties differently from the 18 that \
         crossed at h=88,021"
    );

    // FORWARD-ONLY (CLAUDE.md #0). The pin was chosen above the live tip, never behind
    // it. Without this, "pinned at 88_014" would not distinguish a forward pin from a
    // retroactive one that happens to carry the same number.
    assert!(
        t > TESTNET_TIP_AT_PIN,
        "O1 x P-Testnet: the gate {} must be strictly ABOVE the live tip {} it was \
         pinned over. A gate at or below the tip activates retroactively on the blocks \
         the chain had already produced.",
        t,
        TESTNET_TIP_AT_PIN
    );

    // Mainnet is the counterparty: it carries its OWN pin, 409_000, decided separately on
    // 2026-09-05. Without this cell, testnet drifting onto the mainnet value (or vice
    // versa) passes silently.
    assert_eq!(
        NetworkParams::defaults(Network::Mainnet).inc_i_204_fork_choice_activation_height,
        409_000,
        "O1 x P-Mainnet: PINNED 2026-09-05 at 409_000, independently of the testnet pin \
         above. IMMUTABLE once crossed (INC-I-054)."
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
// INC-I-178 M4 — the attestation-BLS gate joins this ledger (D8).
//
// It lives here, not in its own file, because this file IS the per-network literal
// ledger: a gate that is pinned anywhere but recorded nowhere is how INC-I-054 moved
// a crossed height without anyone noticing.
// ===========================================================================

/// The env var name for the INC-I-178 gate, derived from its field name exactly as
/// [`ENV_VAR`] is derived from the INC-I-204 one.
const BLS_ENV_VAR: &str = "DOLI_INC_I_178_ATTESTATION_BLS_ACTIVATION_HEIGHT";

// INC-I-208 M3 — Decision: renamed from
// `..._is_frozen_on_mainnet_and_devnet_and_pinned_on_testnet`; the 2026-09-05 decision
// pinned mainnet too, so devnet alone stays frozen now.
/// REQ-BLS-005 — Decision: a failure means the attestation-BLS rules (new bit
/// semantics, a new `presence_root` preimage inside `BlockHeader::hash()`, new
/// rejection paths) moved on some network without the decision-session that D8 and
/// CLAUDE.md require. Devnet must stay FROZEN — it forks every running local chain on
/// the next rebuild. Mainnet (409_000, 2026-09-05) and testnet (112_619, `fbc9730d`,
/// v6.27.0) are both PINNED and CROSSED-or-about-to-cross, so per INV-PARAMS-001 /
/// INC-I-054 both heights are IMMUTABLE once crossed: the literals below are the
/// tripwire against moving either, in either direction.
#[test]
fn req_bls_005_m4_the_attestation_bls_gate_is_pinned_on_mainnet_and_testnet_and_frozen_on_devnet() {
    for (network, expected) in [
        (Network::Mainnet, 409_000),
        (Network::Testnet, 112_619),
        (Network::Devnet, u64::MAX),
    ] {
        assert_eq!(
            NetworkParams::defaults(network).inc_i_178_attestation_bls_activation_height,
            expected,
            "{network:?}: expected {expected}. Devnet stays u64::MAX. Mainnet's 409_000 \
             and testnet's 112_619 are IMMUTABLE once crossed (INC-I-054); moving either \
             deactivates live rules or activates them retroactively."
        );
    }
}

/// REQ-BLS-005 — Decision: a failure means the new gate is an alias of, or was bundled
/// onto, a height the chain has already crossed. Moving the attestation-BLS gate would
/// then move a crossed gate with it — the INC-I-054 shape. Mainnet's BLS and
/// fork-choice gates now share one pinned value (409_000), so plain equality is
/// useless here; independence is demonstrated by MOVING one field and reading the
/// others.
#[test]
fn req_bls_005_m4_the_attestation_bls_gate_is_a_distinct_independently_settable_field() {
    let mut probe = NetworkParams::defaults(Network::Mainnet);
    let fork_choice_before = probe.inc_i_204_fork_choice_activation_height;
    let inc_147_before = probe.inc_i_147_activation_height;
    let oracle_before = probe.oracle_activation_height;

    probe.inc_i_178_attestation_bls_activation_height = 4_243;

    assert_eq!(
        probe.inc_i_178_attestation_bls_activation_height, 4_243,
        "the new gate must be settable"
    );
    assert_eq!(
        probe.inc_i_204_fork_choice_activation_height, fork_choice_before,
        "INV-PARAMS-001: moving the BLS gate must not move the fork-choice gate"
    );
    assert_eq!(
        probe.inc_i_147_activation_height, inc_147_before,
        "INV-PARAMS-001: inc_i_147 is CROSSED on mainnet (129_500) and must not move"
    );
    assert_eq!(
        probe.oracle_activation_height, oracle_before,
        "INV-PARAMS-001: the other u64::MAX gate must not be the same field wearing \
         two names — both are frozen, so only a write proves they are distinct"
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
        "mainnet: the pinned 409_000 must not collapse onto 129_500 (crossed)"
    );
    // INC-I-203 M5 — Decision: was `assert_eq!(mainnet, testnet)` recording that the two
    // frozen networks legitimately agreed. `5b326fe9` pinned testnet at 88_014 while
    // mainnet stayed u64::MAX, so they now legitimately DIFFER; the assertion is
    // re-aimed at the same anti-bundling property with the real testnet value.
    assert_ne!(
        t.inc_i_204_fork_choice_activation_height, t.inc_i_147_activation_height,
        "testnet: the pinned fork-choice gate 88_014 must not collapse onto \
         inc_i_147_activation_height 80_700, which testnet crossed long ago"
    );
    // Anti-vacuity for the testnet arm: `assert_ne!` alone would also pass if the field
    // were u64::MAX or 0, i.e. if the gate had been unpinned or made retroactive. Naming
    // both concrete values makes the inequality a statement about THESE two gates.
    assert_eq!(
        t.inc_i_204_fork_choice_activation_height, TESTNET_PINNED_GATE,
        "testnet: the anti-bundling claim is about the PINNED gate 88_014"
    );
    assert_eq!(
        t.inc_i_147_activation_height, 80_700,
        "testnet: inc_i_147_activation_height is crossed history and must not move \
         (INC-I-054); if it did, the inequality above would be measuring a moved gate"
    );
    assert!(
        t.inc_i_204_fork_choice_activation_height > t.inc_i_147_activation_height,
        "testnet: the M5 gate {} must sit ABOVE the gate it supersedes {}. Below it, \
         the new authority would claim heights the old rule already decided.",
        t.inc_i_204_fork_choice_activation_height,
        t.inc_i_147_activation_height
    );
    // The two networks are pinned to DIFFERENT heights (mainnet 409_000, testnet 88_014)
    // decided in separate sessions — a shared value here means one pin leaked into the
    // other's decision.
    assert_ne!(
        m.inc_i_204_fork_choice_activation_height, t.inc_i_204_fork_choice_activation_height,
        "mainnet is pinned at 409_000 and testnet at 88_014, from separate \
         decision-sessions; a shared value means one leaked into the other"
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
    // INC-I-208 M3 — pinned 2026-09-05, IMMUTABLE once crossed (INC-I-054).
    assert_eq!(p.inc_i_178_attestation_bls_activation_height, 409_000);
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
    assert_eq!(p.inc_i_176_auth_binding_activation_height, 15_087); // INC-I-178 — the BLS gate joins the ledger; PINNED on testnet 2026-09-05 (v6.27.0).
    assert_eq!(p.inc_i_178_attestation_bls_activation_height, 112_619);
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
    // INC-I-208 M3 — devnet stays frozen; mainnet+testnet are pinned (INC-I-054).
    assert_eq!(p.inc_i_178_attestation_bls_activation_height, u64::MAX);
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
///
/// INC-I-178 M4 — REQ-BLS-005 — Decision: the same two halves for
/// `DOLI_INC_I_178_ATTESTATION_BLS_ACTIVATION_HEIGHT`. A failure of its ANTI-VACUITY
/// half means the testnet rehearsal cannot arm the BLS gate without a code change; a
/// failure of its LOCK half means one operator can arm new block-content rules on
/// mainnet from a `.env` file, and every block that node builds is rejected by the rest
/// of the fleet (or worse, accepted by a minority that shares the file).
/// It rides in THIS function rather than its own because `NetworkParams::load` caches
/// per network in a process-wide `OnceLock`: a second `load` caller in this binary would
/// race, and whichever ran first would silently void the other's override.
#[test]
fn the_env_override_is_locked_on_mainnet_and_honoured_elsewhere() {
    // Neither network's default, so neither branch can pass by coincidence.
    const SENTINEL: &str = "9";
    const BLS_SENTINEL: &str = "11";
    let original = std::env::var(ENV_VAR);
    let bls_original = std::env::var(BLS_ENV_VAR);
    std::env::set_var(ENV_VAR, SENTINEL);
    std::env::set_var(BLS_ENV_VAR, BLS_SENTINEL);

    let mainnet_params = NetworkParams::load(Network::Mainnet);
    let devnet_params = NetworkParams::load(Network::Devnet);
    let testnet_params = NetworkParams::load(Network::Testnet);

    let mainnet = mainnet_params.inc_i_204_fork_choice_activation_height;
    let devnet = devnet_params.inc_i_204_fork_choice_activation_height;
    let testnet = testnet_params.inc_i_204_fork_choice_activation_height;

    let bls_mainnet = mainnet_params.inc_i_178_attestation_bls_activation_height;
    let bls_devnet = devnet_params.inc_i_178_attestation_bls_activation_height;
    let bls_testnet = testnet_params.inc_i_178_attestation_bls_activation_height;

    // Restore BEFORE asserting so a failure cannot leak process state into the rest of
    // the binary.
    match original {
        Ok(v) => std::env::set_var(ENV_VAR, v),
        Err(_) => std::env::remove_var(ENV_VAR),
    }
    match bls_original {
        Ok(v) => std::env::set_var(BLS_ENV_VAR, v),
        Err(_) => std::env::remove_var(BLS_ENV_VAR),
    }

    assert_eq!(
        bls_devnet, 11,
        "REQ-BLS-005 ANTI-VACUITY: {BLS_ENV_VAR} must be honoured on devnet. If this \
         fails the variable is not wired and the mainnet lock below is meaningless."
    );
    assert_eq!(
        bls_testnet, 11,
        "REQ-BLS-005 ANTI-VACUITY: honoured on testnet — this is how the testnet \
         rehearsal arms the attestation-BLS gate without a code change"
    );
    // INC-I-208 M3 — the compiled default moved to 409_000 (pinned, IMMUTABLE once
    // crossed); the LOCK property under test is unchanged — env must not override it.
    assert_eq!(
        bls_mainnet, 409_000,
        "REQ-BLS-005 THE LOCK: mainnet stays at the compiled 409_000. An override here \
         would let one operator switch on new bit semantics and a new presence_root \
         preimage — block CONTENT, inside BlockHeader::hash() — while the rest of \
         mainnet is still on the old rules."
    );

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
    // INC-I-208 M3 — the compiled default moved to 409_000 (pinned, IMMUTABLE once
    // crossed); the LOCK property under test is unchanged — env must not override it.
    assert_eq!(
        mainnet, 409_000,
        "THE LOCK: mainnet activation heights are locked to the compiled default \
         (env_loader.rs:483-494 is the template). An override here would let one \
         operator arm a different fork-choice rule than the rest of the network."
    );
}
