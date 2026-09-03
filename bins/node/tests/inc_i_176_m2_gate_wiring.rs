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
//! INPUT PARTITIONS (network = Testnet for every row; heights are DERIVED from
//! the shipped params at run time, never re-typed as literals — see
//! [`gate_22`]. Shipped testnet #22 today: `15_087`.)
//!   IP-W1 add,    legacy-signed, h = #22 - 1     -> P-LEG-ACCEPT (parity)
//!   IP-W2 add,    legacy-signed, h = #22         -> P-LEG-REJECT
//!   IP-W3 add,    bound-signed,  h = #22         -> P-BND-ACCEPT
//!   IP-W4 add,    bound-signed,  h = #22 - 1     -> P-BND-REJECT
//!   IP-W5 remove, legacy-signed, h = #22 - 1     -> P-LEG-ACCEPT (parity)
//!   IP-W6 remove, legacy-signed, h = #22         -> P-LEG-REJECT
//!   IP-W7 remove, bound-signed,  h = #22         -> P-BND-ACCEPT
//!   IP-W8 remove, bound-signed,  h = #22 - 1     -> P-BND-REJECT
//!   IP-W9 add,    bound-signed,  h = #22 + 1_000 -> P-BND-ACCEPT above the edge
//!
//! GATE COLLISION, RECORDED NOT HIDDEN (INC-I-203 M5). Testnet pins #20 and #22
//! to the SAME height (`15_087`), so `#22 - 1` is below BOTH gates and no height
//! exists that is below #22 and at/above #20 — the two gates are INSEPARABLE on
//! this network. #20 selects entry-counting over distinct-signer counting, and
//! every fixture here supplies three DISTINCT valid maintainer signers, for which
//! the two #20 arms return the SAME verdict.
//! `req_176_022_the_gate_20_collision_cannot_explain_the_below_gate_rows` proves
//! that agreement AND proves the two arms are otherwise distinguishable, so the
//! below-gate rows still isolate the MESSAGE as the only effective variable.
//!
//! MATRIX
//!   O1 x {IP-W1..IP-W9} = 9 cells (all `None`)
//!   O2 x {IP-W1..IP-W9} = 9 cells
//!   O3 x {IP-W3, IP-W7} = 2 cells
//!
//! ANTI-VACUITY PAIRING (each pair differs in exactly ONE input)
//!   IP-W1 <-> IP-W2  only the height (#22 - 1 vs #22) — THE BOUNDARY
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
use doli_core::{MaintainerSet, Network};
use inc_i_176_m2_common::{
    bound_message, change_tx, make_node, members, quorum, sig, submit, threshold,
};

// ===========================================================================
// The gates, DERIVED — never re-typed as literals
//
// INC-I-203 M5 — Decision: the shipped testnet #22 was re-pinned 300_000 ->
// 15_087 (fresh testnet genesis, 2026-08-24) while this file still drove
// 299_999 / 300_000, which put BOTH rows above the moved gate and made every
// below-gate assertion measure the wrong arm. The heights are now read through
// `Network::Testnet.params()` — the SAME accessor the production site uses
// (`governance.rs:47`, `self.config.network.params()`) — so a future re-pin
// moves the experiment instead of invalidating it.
// ===========================================================================

/// `inc_i_176_auth_binding_activation_height` (#22) for `Network::Testnet`.
fn gate_22() -> u64 {
    Network::Testnet
        .params()
        .inc_i_176_auth_binding_activation_height
}

/// `maintainer_derivation_activation_height` (#20) for `Network::Testnet`.
fn gate_20() -> u64 {
    Network::Testnet
        .params()
        .maintainer_derivation_activation_height
}

/// One block BELOW #22 — the legacy arm.
fn below_gate() -> u64 {
    let g = gate_22();
    assert!(
        g >= 1,
        "harness: #22 is pinned at 0, so there is NO height below it. `#22 - 1` \
         would underflow and the below-gate rows would collapse onto the gate. \
         This suite cannot measure a boundary that has no lower side."
    );
    g - 1
}

/// EXACTLY #22 — the boundary. The comparison must be `>=` (`set.rs:161`), so
/// this height takes the BOUND arm.
fn at_gate() -> u64 {
    gate_22()
}

/// Well above #22, to show the bound arm is not an edge artefact.
fn above_gate() -> u64 {
    gate_22() + 1_000
}

// ===========================================================================
// Harness integrity — the derived heights must straddle the SHIPPED gate
// ===========================================================================

/// The harness reads the gate the RUNNING NODE reads, and the three heights
/// straddle it. Without this, a re-pinned #22 — or a harness that stopped
/// switching the node to Testnet — would leave every test below silently
/// exercising one arm twice.
///
/// The threats this is armed against are named, and each is separately
/// falsifiable: (a) `make_node` stops switching `config.network` to Testnet, so
/// the derived gate belongs to a different network than the node under test;
/// (b) `gate_22`/`gate_20` regress to a hardcoded literal or to the wrong
/// `NetworkParams` field; (c) #22 is re-pinned to `0`, which has no lower side.
#[tokio::test]
async fn req_176_022_harness_gate_literals_match_the_shipped_params() {
    // (a) THE LOAD-BEARING CHECK — the harness node's network must be the one
    // the heights are derived from. `make_node` lives in a shared module this
    // file does not own; if it stops switching to Testnet, the node resolves
    // devnet's #22 = 20 while every row is driven at ~15_087, and all nine rows
    // silently land on the bound arm.
    let (node, _m, _t) = make_node(4).await;
    assert_eq!(
        node.config.network,
        Network::Testnet,
        "harness: the heights below are derived from Network::Testnet, but the \
         node under test resolves its gates from `self.config.network`. If those \
         two disagree, every row in this file is driven against a gate the node \
         never consults."
    );

    // (b) The derived accessors must still read the shipped params, through the
    // same path production uses. Trips on a hardcoded fallback or a wrong field.
    let p = Network::Testnet.params();
    assert_eq!(
        gate_22(),
        p.inc_i_176_auth_binding_activation_height,
        "harness: gate_22() must READ `inc_i_176_auth_binding_activation_height` \
         off the shipped testnet params, never a re-typed literal"
    );
    assert_eq!(
        gate_20(),
        p.maintainer_derivation_activation_height,
        "harness: gate_20() must READ `maintainer_derivation_activation_height` \
         off the shipped testnet params, never a re-typed literal"
    );

    // (c) The three heights straddle the gate, and the lower side exists.
    assert!(
        gate_22() >= 1,
        "harness: #22 = {} has no height below it; the below-gate rows cannot \
         exist and this suite would only ever measure one arm",
        gate_22()
    );
    assert!(
        below_gate() < gate_22(),
        "harness: below_gate() {} must be STRICTLY below #22 {}",
        below_gate(),
        gate_22()
    );
    assert!(
        at_gate() >= gate_22(),
        "harness: at_gate() {} must be at or above #22 {}",
        at_gate(),
        gate_22()
    );
    assert!(
        above_gate() > gate_22(),
        "harness: above_gate() {} must be strictly above #22 {}",
        above_gate(),
        gate_22()
    );

    // The at/above rows must stay at or above #20 so they take the
    // distinct-signer arm — the post-INC-I-172 semantics this file assumes.
    // The BELOW row cannot: testnet pins #20 == #22, so no height is below #22
    // and at/above #20. That inseparability is handled by
    // `req_176_022_the_gate_20_collision_cannot_explain_the_below_gate_rows`.
    assert!(
        at_gate() >= gate_20(),
        "harness: the at-gate rows must sit at or above #20 {} so they take the \
         DISTINCT-SIGNER arm; at_gate() is {}",
        gate_20(),
        at_gate()
    );
}

/// ANTI-CONFOUND / POSITIVE CONTROL — gate #20 cannot explain a below-gate result.
///
/// Testnet pins #20 and #22 to the SAME height, so `below_gate()` crosses BOTH.
/// #20 selects `verify_multisig_legacy` (counts signature ENTRIES) over
/// `verify_multisig` (counts DISTINCT SIGNERS). If those two could disagree for
/// this file's fixture, every below-gate row would be explainable by #20 rather
/// than by the message format, and the suite would prove nothing.
///
/// OUTPUT CONTRACT:
///   Functions under test: `MaintainerSet::verify_multisig_legacy(&[MaintainerSignature], &[u8]) -> bool`
///                         `MaintainerSet::verify_multisig(&[MaintainerSignature], &[u8]) -> bool`
///   O1 (return value)   the `bool` verdict. The ONLY observable output.
///   O2 (mutable params) NONE — both take `&self` and shared slices.
///   O3 (receiver mut.)  NONE — neither mutates `MaintainerSet`.
///   O4 (persistent)     NONE — no store is touched.
///   CODE PATHS  P-AGREE   distinct signers   -> both arms accept
///               P-DIVERGE repeated signer    -> entries accept, distinct rejects
///   INPUT PARTITIONS (4-member set, threshold 3, one message)
///     IP-C1 three DISTINCT maintainer signatures -> P-AGREE
///     IP-C2 the SAME maintainer signature x3     -> P-DIVERGE
///   MATRIX  O1 x {IP-C1, IP-C2} x {legacy arm, distinct arm} = 4 cells, all asserted.
///   ANTI-VACUITY  IP-C1 <-> IP-C2 differ in exactly one input (signer identity).
///   Without IP-C2, "the arms agree" would be unfalsifiable — it would also hold
///   if the two functions were the same function.
#[test]
fn req_176_022_the_gate_20_collision_cannot_explain_the_below_gate_rows() {
    let maintainers: Vec<KeyPair> = (0..4).map(|_| KeyPair::generate()).collect();
    let set =
        MaintainerSet::with_members(maintainers.iter().map(|kp| *kp.public_key()).collect(), 0);
    assert_eq!(
        set.threshold, 3,
        "fixture: a 4-member set carries threshold 3 (`set.rs:112`)"
    );

    let target = *KeyPair::generate().public_key();
    let msg = signing_message_legacy(true, &target);

    // IP-C1 — THIS FILE'S FIXTURE: three DISTINCT signers, exactly what
    // `quorum()` builds. Both #20 arms must return the same verdict, so crossing
    // #20 changes nothing about any below-gate row.
    let distinct: Vec<_> = maintainers.iter().take(3).map(|kp| sig(kp, &msg)).collect();
    assert!(
        set.verify_multisig_legacy(&distinct, &msg),
        "IP-C1 / pre-#20 arm: three DISTINCT valid maintainer signatures are \
         three ENTRIES, which meets threshold 3"
    );
    assert!(
        set.verify_multisig(&distinct, &msg),
        "IP-C1 / post-#20 arm: three DISTINCT valid maintainer signatures are \
         three SIGNERS, which meets threshold 3. Agreeing with the line above is \
         what licenses driving the below-gate rows across #20."
    );

    // IP-C2 — ANTI-VACUITY: the two arms are genuinely different functions.
    // Without this row, the agreement above could just mean they are aliases.
    let repeated: Vec<_> = (0..3).map(|_| sig(&maintainers[0], &msg)).collect();
    assert!(
        set.verify_multisig_legacy(&repeated, &msg),
        "IP-C2 / pre-#20 arm counts ENTRIES: three copies of ONE signature reach \
         threshold 3. This is AUDIT-P0-010, preserved as frozen history."
    );
    assert!(
        !set.verify_multisig(&repeated, &msg),
        "IP-C2 / post-#20 arm counts DISTINCT SIGNERS: three copies of ONE \
         signature count as one. The two arms MUST be distinguishable, or \
         IP-C1's agreement is vacuous and #20 remains an untested confound."
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
        submit(&node, &tx, below_gate()).await,
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
        gate_22()
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

    assert_eq!(submit(&node, &tx, below_gate()).await, None, "O1");
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

    assert_eq!(submit(&node, &tx, below_gate()).await, None, "O1");
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

    assert_eq!(submit(&node, &tx, below_gate()).await, None, "O1");
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
    for height in [at_gate(), above_gate()] {
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
            gate_22()
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

    assert_eq!(submit(&node, &tx, at_gate()).await, None, "O1");
    let after = members(&node).await;
    assert_eq!(
        after.len(),
        4,
        "O2 / REQ-176-022: at height {} (== #22) a signature over the LEGACY \
         bytes must no longer seat a maintainer. While it does, the unbound, \
         domain-tag-less, chain-blind authorization is still a valid bearer token \
         above the gate and M2 has changed nothing.",
        at_gate()
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

    assert_eq!(submit(&node, &tx, at_gate()).await, None, "O1");

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

    assert_eq!(submit(&node, &tx, at_gate()).await, None, "O1");
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
        submit(&node, &tx, below_gate()).await;
        assert!(
            members(&node).await.contains(&target),
            "BOUNDARY: height == #22 - 1 ({}) must still take the LEGACY arm",
            below_gate()
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
        submit(&node, &tx, gate_22()).await;
        assert!(
            !members(&node).await.contains(&target),
            "BOUNDARY: height == #22 ({}) must ALREADY take the BOUND arm. The \
             comparison is `>=` (set.rs:269 / authmsg.rs:224). A `>` shifts this \
             gate one block relative to every other maintainer gate.",
            gate_22()
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
        submit(&node, &tx, gate_22()).await;
        assert!(
            members(&node).await.contains(&target),
            "BOUNDARY / ANTI-VACUITY: at height == #22 the BOUND bytes must be \
             accepted. Row 2's refusal must be caused by the arm switching, not \
             by governance being dead at that height."
        );
    }
}
