//! INC-I-176 **M2** — **REQ-176-022**: the owned message constructor is wired
//! into production, gated on `inc_i_176_auth_binding_activation_height` (#22).
//!
//! This is the CENTRAL EVIDENCE file for M2. Its sibling
//! `bins/node/tests/inc_i_176_m2_domain_separation.rs` carries the AUDIT-P0-011
//! payoff, the non-fatality proof and the `ProtocolActivation` scope fence; the
//! two share `inc_i_176_m2_common`, which also documents the harness hazard and
//! its solution. The split exists for the 800-line test-file budget (CLAUDE.md
//! rule 19).
//!
//! Everything in `crates/core/tests/inc_i_176_m2_*.rs` states properties of
//! values and of a leaf function. THIS file states what the RUNNING NODE does at
//! the single NON-FATAL apply site,
//! `bins/node/src/node/apply_block/governance.rs`.
//!
//! Design decisions (binding): `docs/.workflow/inc-i-176-M2-design-decision.md`.
//!
//! ---------------------------------------------------------------------------
//! TDD RED — EXPECTED, NOT A DEFECT
//! ---------------------------------------------------------------------------
//! This file does **NOT compile** against the tree at `3f8bf185`:
//! `doli_core::maintainer::MAINTAINER_AUTH_VALID_BEFORE_UNSET` and
//! `NetworkParams::inc_i_176_auth_binding_activation_height` do not exist yet.
//! Once they do, the ABOVE-GATE tests are RUNTIME-red until `governance.rs` stops
//! calling `data.signing_message(is_add)`; the BELOW-GATE tests are GREEN today
//! and **must stay green** — they are the frozen-history parity proof.
//!
//! MEASURED against a probe build of this suite with the two symbols stubbed
//! locally: 7 tests pass (every below-gate and parity row) and 8 fail (every
//! at/above-gate row), i.e. the RED/GREEN split is exactly the one documented
//! here and not an artefact of a broken harness.
//!
//! ---------------------------------------------------------------------------
//! WHAT M2 REWIRES (the developer's one change in this file's blast radius)
//! ---------------------------------------------------------------------------
//! In `governance.rs`, the `AddMaintainer` and `RemoveMaintainer` arms replace
//!
//! ```ignore
//! let message = data.signing_message(is_add);          // -> signing_message_legacy
//! ```
//!
//! with
//!
//! ```ignore
//! let message = doli_core::maintainer::signing_message_at(
//!     genesis_hash.as_bytes(),
//!     is_add,
//!     &data.target,
//!     doli_core::maintainer::MAINTAINER_AUTH_VALID_BEFORE_UNSET,
//!     height,
//!     auth_binding_activation_height,   // params().inc_i_176_auth_binding_activation_height
//! );
//! ```
//!
//! The `ProtocolActivation` arm is **UNTOUCHED but NOT CLEARED** (audit
//! AUDIT-P1-101): it is a different signing family (`activate:{v}:{e}`) and out of
//! M2's scope, yet it carries the SAME collision class under the SAME 5-key root,
//! with no genesis binding and no expiry, ungated at every height on every network.
//! Out of scope is not fixed — it needs its own milestone and its own activation
//! height. `MaintainerChangeData::signing_message` is **NOT**
//! changed either: it stays the legacy-only helper the CLI signer uses
//! (Decision 3), which is why
//! `crates/updater/tests/inc_i_172_m2_release_sign_arg_validation.rs::the_collision_still_exists_and_only_m3_closes_it`
//! stays green WITHOUT being edited. Both fences are asserted in the sibling file
//! and in `crates/core/tests/inc_i_176_m2_scope_fence.rs`.
//!
//! ---------------------------------------------------------------------------
//! THE HARNESS HAZARD
//! ---------------------------------------------------------------------------
//! `Node::new_for_test` is hardwired to Devnet, whose #22 is `0`, so a below-gate
//! case is unreachable through devnet defaults. SOLVED in `inc_i_176_m2_common`
//! by switching `node.config.network` to Testnet — the same mechanism
//! `bins/node/tests/inc_i_172_m2_fail_close.rs` uses for gate #20. **The full
//! `Node` harness reaches BOTH arms**; no weaker dispatcher-only substitute is
//! used. Read that module's header before changing any height in this file.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT
//! ---------------------------------------------------------------------------
//! Function under test:
//!   G1 `Node::process_transaction_governance(&self, &Transaction, height: u64,
//!       &ProducerSet) -> Option<(u32, u64)>`
//!
//! ENUMERATION OF OBSERVABLE OUTPUTS
//!   O1 (return value)          `Option<(u32, u64)>`. For the two maintainer arms
//!                              it is ALWAYS `None` — that is the "non-fatal, no
//!                              new reject path" channel, and it is asserted, not
//!                              assumed.
//!   O2 (receiver mutation)     `maintainer_state.set.members` — THE acceptance
//!                              oracle. The message the production path built is
//!                              not returned anywhere, so it is observed through
//!                              WHICH SIGNATURE SET IT ACCEPTS. That is strictly
//!                              stronger than reading a message out: it proves the
//!                              bytes are the ones a signer must actually produce.
//!   O3 (receiver mutation)     `maintainer_state.set.threshold` — re-derived by
//!                              `add_maintainer` / `remove_maintainer`.
//!   O4 (persistent store)      `<data_dir>/maintainer_state.bin` — asserted in
//!                              the sibling file.
//!   O5 (mutable params)        NONE — `&self` and `&Transaction`; `ProducerSet`
//!                              is passed by shared reference.
//!   O6 (process state)         no panic, no error, no block rejection — asserted
//!                              in the sibling file.
//!   side channels              `warn!`/`info!` lines. DECLARED UNASSERTED — this
//!                              harness captures no log output; O2 covers the same
//!                              decision with a stronger instrument.
//!
//! CODE PATHS
//!   P-LEG-ACCEPT  height <  #22, signatures over the LEGACY bytes   -> applied
//!   P-LEG-REJECT  height >= #22, signatures over the LEGACY bytes   -> skipped
//!   P-BND-ACCEPT  height >= #22, signatures over the BOUND bytes    -> applied
//!   P-BND-REJECT  height <  #22, signatures over the BOUND bytes    -> skipped
//!
//! INPUT PARTITIONS (network = Testnet for every row; #20 = 127_200, #22 = 300_000)
//!   IP-W1 add,    legacy-signed, h = 299_999  -> P-LEG-ACCEPT (parity, green today)
//!   IP-W2 add,    legacy-signed, h = 300_000  -> P-LEG-REJECT (RED today)
//!   IP-W3 add,    bound-signed,  h = 300_000  -> P-BND-ACCEPT (RED today)
//!   IP-W4 add,    bound-signed,  h = 299_999  -> P-BND-REJECT (green today)
//!   IP-W5 remove, legacy-signed, h = 299_999  -> P-LEG-ACCEPT (parity, green today)
//!   IP-W6 remove, legacy-signed, h = 300_000  -> P-LEG-REJECT (RED today)
//!   IP-W7 remove, bound-signed,  h = 300_000  -> P-BND-ACCEPT (RED today)
//!   IP-W8 remove, bound-signed,  h = 299_999  -> P-BND-REJECT (green today)
//!   IP-W9 add,    bound-signed,  h = 301_000  -> P-BND-ACCEPT well above the edge
//!
//! MATRIX
//!   O1 x {IP-W1..IP-W9} = 9 cells (all `None`)
//!   O2 x {IP-W1..IP-W9} = 9 cells
//!   O3 x {IP-W3, IP-W7} = 2 cells
//!
//! ANTI-VACUITY PAIRING (each pair differs in exactly ONE input)
//!   IP-W1 <-> IP-W2  only the height (299_999 vs 300_000) — THE BOUNDARY
//!   IP-W3 <-> IP-W4  only the height
//!   IP-W1 <-> IP-W4  only which bytes were signed
//!   IP-W2 <-> IP-W3  only which bytes were signed
//!   plus `fixture_the_two_message_arms_are_distinguishable`, without which every
//!   accept/refuse pair would be unfalsifiable.
//!
//! ---------------------------------------------------------------------------
//! WHAT THIS FILE DOES NOT DO (scope fence)
//! ---------------------------------------------------------------------------
//! 1. It adds NO reject condition and asserts nothing about
//!    `crates/core/src/validation/tx_types.rs`, whose `git diff HEAD` must stay
//!    EMPTY (binding user decision 1).
//! 2. It moves NO payload byte. `MaintainerChangeData` is constructed through its
//!    existing public API only.
//! 3. It asserts NO expiry ENFORCEMENT. Nothing in M2 compares a height to
//!    `valid_before`; that is M3's non-fatal check.
//! 4. It pins no mainnet height and touches no `HardForkSchedule` entry.

mod inc_i_176_m2_common;

use crypto::KeyPair;
use doli_core::maintainer::signing_message_legacy;
use doli_core::network_params::NetworkParams;
use doli_core::Network;
use inc_i_176_m2_common::{
    bound_message, change_tx, make_node, members, quorum, submit, threshold, ABOVE_GATE, AT_GATE,
    BELOW_GATE, TESTNET_GATE_20, TESTNET_GATE_22,
};

// ===========================================================================
// Harness integrity — the literals above must be the shipped ones
// ===========================================================================

/// The harness constants are bound to `NetworkParams`. Without this, a re-pinned
/// #22 would leave every test below silently exercising one arm twice.
#[test]
fn req_176_022_harness_gate_literals_match_the_shipped_params() {
    let p = NetworkParams::defaults(Network::Testnet);
    assert_eq!(
        p.inc_i_176_auth_binding_activation_height, TESTNET_GATE_22,
        "harness: this file drives heights {} / {} around a #22 of {}. If the \
         shipped gate moved, both rows may now be on the same side of it and \
         every result below is meaningless.",
        BELOW_GATE, AT_GATE, TESTNET_GATE_22
    );
    assert_eq!(
        p.maintainer_derivation_activation_height, TESTNET_GATE_20,
        "harness: both test heights must be above #20 so the distinct-signer \
         counter applies on BOTH sides and the message is the only variable"
    );
}

/// ANTI-VACUITY / POSITIVE CONTROL — the two message arms must be different bytes.
///
/// If `signing_message_legacy` and the bound message ever coincided, every
/// "accepted below / refused above" pair in this file would be unfalsifiable.
#[tokio::test]
async fn fixture_the_two_message_arms_are_distinguishable() {
    let (node, _m, _t) = make_node(4).await;
    let target = *KeyPair::generate().public_key();

    for is_add in [true, false] {
        let legacy = signing_message_legacy(is_add, &target);
        let bound = bound_message(&node, is_add, &target);
        assert_ne!(
            legacy, bound,
            "POSITIVE CONTROL: the legacy and bound messages must be distinct \
             bytes for is_add={}, or the gate changes nothing and every assertion \
             in this file passes vacuously",
            is_add
        );
        assert_eq!(bound.len(), 32, "the bound message is a BLAKE3-256 digest");
    }
}

// ===========================================================================
// REQ-176-022 — BELOW THE GATE: EXACTLY the legacy bytes, BIT-IDENTICAL
//
// These are the frozen-history parity assertions. They are GREEN today and must
// STAY green: below #22 the fleet verifies `format!("{}:{}", action, hex)` and a
// node that changed its mind about a historical height would hold a different
// maintainer trust root from every peer.
// ===========================================================================

/// IP-W1 / O1, O2 — P-LEG-ACCEPT for `AddMaintainer`. PARITY, must stay green.
#[tokio::test]
async fn req_176_022_below_the_gate_add_accepts_exactly_the_legacy_bytes() {
    let (node, maintainers, _t) = make_node(4).await;
    let target = *KeyPair::generate().public_key();
    let signers = quorum(&maintainers);

    // The message is built by `signing_message_legacy` — the ONE owner of the
    // frozen format — so "bit-identical" is asserted against the format's owner,
    // not against a re-typed literal.
    let tx = change_tx(
        true,
        &target,
        &signing_message_legacy(true, &target),
        &signers,
    );

    assert_eq!(
        submit(&node, &tx, BELOW_GATE).await,
        None,
        "O1: a maintainer change never reports a ProtocolActivation"
    );

    let after = members(&node).await;
    assert_eq!(
        after.len(),
        5,
        "O2 / PARITY: below #22 ({}) the production path MUST build EXACTLY \
         `signing_message_legacy(true, target)`. A signature over those bytes is \
         what every node on the live fleet accepts at historical heights; \
         rejecting it here forks this node's trust root away from its peers.",
        TESTNET_GATE_22
    );
    assert!(
        after.contains(&target),
        "O2: the new maintainer must be seated"
    );
}

/// IP-W4 / O2 — P-BND-REJECT. The mirror of IP-W1: below the gate the BOUND bytes
/// must NOT be accepted. Green today; it is what makes IP-W1 a two-sided result.
#[tokio::test]
async fn req_176_022_below_the_gate_add_refuses_the_bound_bytes() {
    let (node, maintainers, _t) = make_node(4).await;
    let target = *KeyPair::generate().public_key();
    let signers = quorum(&maintainers);

    let msg = bound_message(&node, true, &target);
    let tx = change_tx(true, &target, &msg, &signers);

    assert_eq!(submit(&node, &tx, BELOW_GATE).await, None, "O1");
    let after = members(&node).await;
    assert_eq!(
        after.len(),
        4,
        "O2: below #22 the production path must NOT accept a signature over the \
         bound message. If it does, the gate is not being consulted and the new \
         message form has been applied retroactively to frozen history."
    );
    assert!(!after.contains(&target));
}

/// IP-W5 / O1, O2 — P-LEG-ACCEPT for `RemoveMaintainer`. PARITY, must stay green.
///
/// The remove arm is asserted separately because it goes through a DIFFERENT
/// verifier (`verify_multisig_excluding_at`) and takes `is_add = false`. A wiring
/// change that fixed only the add arm, or that hardcoded `true`, passes every
/// add-side test in this file.
#[tokio::test]
async fn req_176_022_below_the_gate_remove_accepts_exactly_the_legacy_bytes() {
    let (node, maintainers, _t) = make_node(5).await;
    let target = *maintainers[0].public_key();
    let signers = quorum(&maintainers[1..]);

    let tx = change_tx(
        false,
        &target,
        &signing_message_legacy(false, &target),
        &signers,
    );

    assert_eq!(submit(&node, &tx, BELOW_GATE).await, None, "O1");
    let after = members(&node).await;
    assert_eq!(
        after.len(),
        4,
        "O2 / PARITY: below #22 the remove arm MUST build EXACTLY \
         `signing_message_legacy(false, target)` — note the `false`, which is the \
         term a copy-pasted add arm would get wrong"
    );
    assert!(
        !after.contains(&target),
        "O2: the maintainer must be unseated"
    );
}

/// IP-W8 / O2 — P-BND-REJECT for the remove arm.
#[tokio::test]
async fn req_176_022_below_the_gate_remove_refuses_the_bound_bytes() {
    let (node, maintainers, _t) = make_node(5).await;
    let target = *maintainers[0].public_key();
    let signers = quorum(&maintainers[1..]);

    let msg = bound_message(&node, false, &target);
    let tx = change_tx(false, &target, &msg, &signers);

    assert_eq!(submit(&node, &tx, BELOW_GATE).await, None, "O1");
    assert_eq!(
        members(&node).await.len(),
        5,
        "O2: below #22 a bound-message signature must not unseat a maintainer"
    );
}

// ===========================================================================
// REQ-176-022 — AT AND ABOVE THE GATE: the BOUND message
// ===========================================================================

/// IP-W3, IP-W9 / O1, O2, O3 — P-BND-ACCEPT for `AddMaintainer`. **RED today.**
#[tokio::test]
async fn req_176_022_at_and_above_the_gate_add_accepts_exactly_the_bound_bytes() {
    for height in [AT_GATE, ABOVE_GATE] {
        let (node, maintainers, _t) = make_node(4).await;
        let target = *KeyPair::generate().public_key();
        let signers = quorum(&maintainers);

        let msg = bound_message(&node, true, &target);
        let tx = change_tx(true, &target, &msg, &signers);

        assert_eq!(submit(&node, &tx, height).await, None, "O1");

        let after = members(&node).await;
        assert_eq!(
            after.len(),
            5,
            "O2 / REQ-176-022: at height {} (>= #22 {}) the production path MUST \
             build EXACTLY `signing_message(node.params.genesis_hash, true, \
             target, MAINTAINER_AUTH_VALID_BEFORE_UNSET)`. Today governance.rs \
             calls `data.signing_message(true)`, which delegates to the legacy \
             format, so this is the RED evidence for M2.",
            height,
            TESTNET_GATE_22
        );
        assert!(
            after.contains(&target),
            "O2: the new maintainer must be seated"
        );
        assert_eq!(
            threshold(&node).await,
            3,
            "O3: a 5-member set carries threshold 3"
        );
    }
}

/// IP-W2 / O2 — P-LEG-REJECT for `AddMaintainer`. **RED today.**
///
/// This is the half that makes the AUDIT-P0-011 closure REAL rather than merely
/// wired: above the gate a signature over the old, collision-prone bytes must stop
/// working. It proves the MECHANISM, at a height above #22 — it does not prove the
/// defect is shut on any live network, which depends entirely on where #22 is
/// pinned (mainnet: nowhere; audit AUDIT-P1-102).
#[tokio::test]
async fn req_176_022_at_the_gate_add_refuses_the_legacy_bytes() {
    let (node, maintainers, _t) = make_node(4).await;
    let target = *KeyPair::generate().public_key();
    let signers = quorum(&maintainers);

    let tx = change_tx(
        true,
        &target,
        &signing_message_legacy(true, &target),
        &signers,
    );

    assert_eq!(submit(&node, &tx, AT_GATE).await, None, "O1");
    let after = members(&node).await;
    assert_eq!(
        after.len(),
        4,
        "O2 / REQ-176-022: at height {} (== #22) a signature over the LEGACY \
         bytes must no longer seat a maintainer. While it does, the unbound, \
         domain-tag-less, chain-blind authorization is still a valid bearer token \
         above the gate and M2 has changed nothing.",
        AT_GATE
    );
    assert!(!after.contains(&target));
}

/// IP-W7 / O1, O2, O3 — P-BND-ACCEPT for `RemoveMaintainer`. **RED today.**
#[tokio::test]
async fn req_176_022_at_the_gate_remove_accepts_exactly_the_bound_bytes() {
    let (node, maintainers, _t) = make_node(5).await;
    let target = *maintainers[0].public_key();
    let signers = quorum(&maintainers[1..]);

    let msg = bound_message(&node, false, &target);
    let tx = change_tx(false, &target, &msg, &signers);

    assert_eq!(submit(&node, &tx, AT_GATE).await, None, "O1");

    let after = members(&node).await;
    assert_eq!(
        after.len(),
        4,
        "O2 / REQ-176-022: at #22 the remove arm must build the BOUND message with \
         `is_add = false`"
    );
    assert!(!after.contains(&target));
    assert_eq!(
        threshold(&node).await,
        3,
        "O3: a 4-member set has threshold 3"
    );
}

/// IP-W6 / O2 — P-LEG-REJECT for `RemoveMaintainer`. **RED today.**
#[tokio::test]
async fn req_176_022_at_the_gate_remove_refuses_the_legacy_bytes() {
    let (node, maintainers, _t) = make_node(5).await;
    let target = *maintainers[0].public_key();
    let signers = quorum(&maintainers[1..]);

    let tx = change_tx(
        false,
        &target,
        &signing_message_legacy(false, &target),
        &signers,
    );

    assert_eq!(submit(&node, &tx, AT_GATE).await, None, "O1");
    assert_eq!(
        members(&node).await.len(),
        5,
        "O2: at #22 a legacy-message signature must not unseat a maintainer"
    );
}

// ===========================================================================
// THE BOUNDARY — `>=`, not `>`
// ===========================================================================

/// IP-W1 <-> IP-W2 <-> IP-W3 / O2 — **the off-by-one proof.**
///
/// `activation_height - 1` selects LEGACY; `activation_height` selects BOUND. The
/// comparison must be `>=`, matching `MaintainerSet::verify_multisig_at`
/// (`set.rs:269`) and `signing_message_at` (`authmsg.rs:224`) exactly.
///
/// A `>` here shifts this gate ONE BLOCK relative to every other maintainer gate,
/// and the two gates are read by the same code path at the same call site. The
/// single block of disagreement is enough to give two nodes different maintainer
/// roots — and the maintainer root is the updater's binary-install trust root.
///
/// All three rows are driven on FRESH nodes with the SAME fixture so the only
/// difference between them is `(height, which bytes were signed)`.
#[tokio::test]
async fn req_176_022_the_boundary_is_greater_or_equal_not_greater_than() {
    // Row 1 — one block BELOW the gate, legacy bytes: ACCEPTED.
    {
        let (node, maintainers, _t) = make_node(4).await;
        let target = *KeyPair::generate().public_key();
        let signers = quorum(&maintainers);
        let tx = change_tx(
            true,
            &target,
            &signing_message_legacy(true, &target),
            &signers,
        );
        submit(&node, &tx, TESTNET_GATE_22 - 1).await;
        assert!(
            members(&node).await.contains(&target),
            "BOUNDARY: height == #22 - 1 ({}) must still take the LEGACY arm",
            TESTNET_GATE_22 - 1
        );
    }

    // Row 2 — EXACTLY at the gate, legacy bytes: REFUSED.
    {
        let (node, maintainers, _t) = make_node(4).await;
        let target = *KeyPair::generate().public_key();
        let signers = quorum(&maintainers);
        let tx = change_tx(
            true,
            &target,
            &signing_message_legacy(true, &target),
            &signers,
        );
        submit(&node, &tx, TESTNET_GATE_22).await;
        assert!(
            !members(&node).await.contains(&target),
            "BOUNDARY: height == #22 ({}) must ALREADY take the BOUND arm. The \
             comparison is `>=` (set.rs:269 / authmsg.rs:224). A `>` shifts this \
             gate one block relative to every other maintainer gate.",
            TESTNET_GATE_22
        );
    }

    // Row 3 — EXACTLY at the gate, bound bytes: ACCEPTED. Without this row, row 2
    // could be explained by "nothing is accepted at that height".
    {
        let (node, maintainers, _t) = make_node(4).await;
        let target = *KeyPair::generate().public_key();
        let signers = quorum(&maintainers);
        let msg = bound_message(&node, true, &target);
        let tx = change_tx(true, &target, &msg, &signers);
        submit(&node, &tx, TESTNET_GATE_22).await;
        assert!(
            members(&node).await.contains(&target),
            "BOUNDARY / ANTI-VACUITY: at height == #22 the BOUND bytes must be \
             accepted. Row 2's refusal must be caused by the arm switching, not \
             by governance being dead at that height."
        );
    }
}
