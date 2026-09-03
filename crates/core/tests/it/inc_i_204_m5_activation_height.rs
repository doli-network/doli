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
//! WHY MAINNET IS `u64::MAX` AND TESTNET IS PINNED. Mainnet is frozen by HC-6 shape:
//! pinning a real height is a separate user decision-session. Testnet WAS frozen for
//! the same reason the file originally recorded — the architecture spec named neither
//! a height nor a margin — but that decision was TAKEN, not deferred, in commit
//! `5b326fe9` (INC-I-203 M5 updates these tests to the new truth):
//!
//!   "User decision 2026-09-02 at live tip 87,934 ... gate crossed unanimously at
//!    h=88,021 (18/18 hash-identical) and live-validated at h=88,054-88,055 ...
//!    88,014 is now IMMUTABLE consensus history on testnet (INC-I-054 rule).
//!    Mainnet stays u64::MAX — pinning it is a separate decision-session."
//!
//! The pin is FORWARD-ONLY: 88_014 was chosen ABOVE the live tip 87_934, never
//! retroactively from a height the chain had already passed (CLAUDE.md #0 / INC-I-054).
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
/// REQ-FORK-014 — Decision: a failure means the testnet fork-choice gate moved off the
/// height testnet ALREADY CROSSED at h=88,021. That height is immutable consensus
/// history (INC-I-054): moving it re-selects every block testnet produced since the
/// crossing under a rule that was not the one in force, and re-opens the mixed-version
/// window that the unanimous crossing closed. Mainnet must stay frozen in the same
/// breath — a shared edit that pins mainnet "while we are here" is the HC-6 failure.
///
/// Verbatim rationale from `5b326fe9`: "User decision 2026-09-02 at live tip 87,934 ...
/// gate crossed unanimously at h=88,021 (18/18 hash-identical) and live-validated at
/// h=88,054-88,055 ... 88,014 is now IMMUTABLE consensus history on testnet (INC-I-054
/// rule). Mainnet stays u64::MAX — pinning it is a separate decision-session."
#[test]
fn req_fork_014_testnet_is_pinned_at_the_crossed_height_and_mainnet_stays_frozen() {
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

    // Mainnet is the counterparty: the testnet decision must NOT have carried mainnet
    // with it. Without this cell, pinning both networks in one edit passes silently.
    assert_eq!(
        NetworkParams::defaults(Network::Mainnet).inc_i_204_fork_choice_activation_height,
        u64::MAX,
        "O1 x P-Mainnet: the testnet pin is a TESTNET decision. Mainnet stays u64::MAX \
         — pinning it is a separate decision-session (HC-6)."
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
    // The two live networks now DIFFER (mainnet frozen at u64::MAX, testnet pinned at
    // 88_014) and that difference is the record of a deliberate testnet-only decision —
    // not a copy-paste and not a half-finished edit that forgot mainnet.
    assert_ne!(
        m.inc_i_204_fork_choice_activation_height, t.inc_i_204_fork_choice_activation_height,
        "mainnet is frozen (u64::MAX) and testnet is pinned (88_014); a shared value \
         means either mainnet was pinned without its own decision-session (HC-6) or \
         testnet was unpinned off crossed consensus history (INC-I-054)"
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
