//! INC-I-176 **M2** — **REQ-176-022** on **DEVNET**: above height 20 the BOUND
//! arm actually EXECUTES, through a real `Node`, through real applied blocks.
//!
//! Requirements: **REQ-176-022** (Must — the owned message constructor is wired
//! into production behind `inc_i_176_auth_binding_activation_height`, #22).
//! The per-network VALUE of #22 is **REQ-176-021**
//! (`crates/core/tests/inc_i_176_m2_activation_height.rs`); the ordering
//! constraint is **REV-176-M1a-001**
//! (`crates/core/tests/inc_i_176_m2_ordering.rs`).
//!
//! ---------------------------------------------------------------------------
//! WHY THIS FILE EXISTS — it is the entire payoff of devnet #22 = 20
//! ---------------------------------------------------------------------------
//! Devnet #22 could have been `u64::MAX`. That would have left the five
//! INC-I-174 node suites untouched (their governance blocks live at heights 0-7)
//! exactly as `20` does — and it would have left the BOUND arm **dead on every
//! network a developer can run**: mainnet #22 is `u64::MAX` (unpinned in M2) and
//! testnet #22 is `300_000`, ~146k blocks beyond the measured tip. M3's expiry
//! check and M4's signer would then be built against the legacy arm only, and the
//! first real execution of the bound arm anywhere would be on a live chain.
//!
//! `20` was chosen over `u64::MAX` for one reason: so that THIS FILE can exist.
//! **Without this file the decision buys nothing** — it would be a number nobody
//! crosses. So the load-bearing test here is not the negative control and not the
//! parity control; it is
//! [`req_176_022_devnet_above_the_gate_add_signed_over_the_bound_message_is_applied`],
//! the one that proves the bound arm runs.
//!
//! ---------------------------------------------------------------------------
//! HOW THIS FILE DIFFERS FROM ITS TWO SIBLINGS
//! ---------------------------------------------------------------------------
//! * `bins/node/tests/inc_i_176_m2_gate_wiring.rs` and
//!   `bins/node/tests/inc_i_176_m2_domain_separation.rs` drive
//!   `process_transaction_governance` DIRECTLY, on a node whose
//!   `config.network` has been switched to Testnet so a below-gate height exists
//!   at all (see `inc_i_176_m2_common`'s header: on devnet at #22 = 0 there was
//!   no below-gate height — that hazard is now gone, devnet has heights 0..19).
//! * THIS file leaves the node on **DEVNET** — the network the decision is about
//!   — and goes through `Node::apply_block`, i.e. the real block-application
//!   path that reaches `process_transaction_governance` from `apply_block`. That
//!   is what makes "the bound arm executes" a statement about the node rather
//!   than about a function.
//!
//! ---------------------------------------------------------------------------
//! WHERE THE HARNESS LIVES (800-line test-file budget, CLAUDE.md rule 19)
//! ---------------------------------------------------------------------------
//! * `bins/node/tests/inc_i_176_m2_devnet_common/mod.rs` — the DEVNET harness:
//!   the pinned gate literals, `seeded_devnet_node`, `build_block`,
//!   `advance_to`, `apply_governance_block` (which carries O1 and O7 so no row
//!   can omit them), the O2-O6 accessors, `legacy_message_independent` and
//!   `quorum`. Block construction and seeding follow
//!   `bins/node/tests/inc_i_174_maintainer_undo.rs` so the two stay comparable —
//!   the INC-I-174 suites are exactly what this gate value exists to keep green,
//!   so their harness is the right one to mirror.
//! * `bins/node/tests/inc_i_176_m2_common/mod.rs` — reused ONLY for the pieces
//!   that are network-agnostic: `change_tx` (payload construction) and
//!   `bound_message` (which reads the genesis hash BACK OFF THE NODE, so on a
//!   devnet node it is the devnet genesis and a production site that bound to a
//!   hardcoded `ChainSpec::mainnet()` hash would fail here).
//!
//! ---------------------------------------------------------------------------
//! THE LEGACY ENCODER IS DELIBERATELY NOT THE CRATE'S
//! ---------------------------------------------------------------------------
//! `inc_i_176_m2_devnet_common::legacy_message_independent` rebuilds
//! `format!("{}:{}", action, hex)` from its FORMAT STRING, in the test tree,
//! exactly as the five INC-I-174 suites do. It is NOT
//! `doli_core::maintainer::signing_message_legacy`, and the INC-I-174 builders
//! are NOT rerouted through the crate constructor — a separate encoder is what
//! keeps "below the gate the legacy bytes are still accepted" a real claim
//! instead of a restatement of the implementation. That was considered and
//! explicitly rejected for M2.
//! [`fixture_the_in_file_legacy_encoder_reproduces_the_frozen_format`] binds the
//! two copies as an informational PARITY LOCK; the acceptance assertions
//! themselves use the in-tree encoder.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT
//! ---------------------------------------------------------------------------
//! FUNCTION UNDER TEST
//!   F1 `Node::apply_block(&mut self, Block, ValidationMode) -> Result<_>`
//!      carrying an `AddMaintainer` / `RemoveMaintainer`, i.e. observed through
//!      the real `apply_block -> process_transaction_governance` site
//!      (`bins/node/src/node/apply_block/governance.rs`).
//!
//! ENUMERATION OF OBSERVABLE OUTPUTS (Rust rules: `&mut self` receiver + stores)
//!   O1 return value       `Result<_>` of `apply_block`. MUST be `Ok` on EVERY
//!                         row, including the refusals. #22 adds NO reject path:
//!                         the site is non-fatal, it warns and skips, and the
//!                         block that carried the authorization still applies.
//!   O2 receiver mutation  `maintainer_state.set.members` — THE acceptance
//!                         oracle. The message the production path built is never
//!                         returned anywhere, so it is observed through WHICH
//!                         SIGNATURE SET IT ACCEPTS. Strictly stronger than
//!                         reading a message out: it proves the bytes are the ones
//!                         a signer must actually produce.
//!   O3 receiver mutation  `maintainer_state.set.threshold` — re-derived by
//!                         `add_maintainer` / `remove_maintainer`.
//!   O4 receiver mutation  `maintainer_state.set.last_updated` — the height the
//!                         root claims it came from; served as
//!                         `getMaintainerSet.last_change_block`. Membership can be
//!                         right while this is wrong.
//!   O5 receiver mutation  `maintainer_state.last_derived_height` — the seed arm.
//!   O6 persistent store   `<data_dir>/maintainer_state.bin`, read back
//!                         INDEPENDENTLY with `MaintainerState::load`. An
//!                         in-memory-only acceptance is undone by the next restart,
//!                         and this file is the updater's install trust root.
//!   O7 receiver mutation  `chain_state.best_height` — the block itself was
//!                         applied. Without it every refusal row could be
//!                         explained by "the block never landed".
//!   mutable params        NONE — `apply_block` takes the block by value and
//!                         `&mut self`; O2..O7 ARE the receiver enumeration.
//!   side channels         `warn!("[MAINTAINER] Rejected ...")` / `info!`.
//!                         DECLARED UNASSERTED — `bins/node` has no tracing-capture
//!                         dev-dependency and adding one would edit a non-test
//!                         manifest. O2 covers the same decision with a stronger
//!                         instrument. Stated, not silently skipped.
//!
//! CODE PATHS (devnet: #20 = 0, #22 = 20, #21 = 0)
//!   P-BND-ACCEPT  height >= 20, signatures over the BOUND  bytes -> applied
//!   P-LEG-REJECT  height >= 20, signatures over the LEGACY bytes -> skipped
//!   P-LEG-ACCEPT  height <  20, signatures over the LEGACY bytes -> applied
//!   P-BND-REJECT  height <  20, signatures over the BOUND  bytes -> skipped
//!
//! INPUT PARTITIONS
//!   IP-A1 add,    bound-signed,  h = 21 -> P-BND-ACCEPT  (THE POINT OF THE FILE)
//!   IP-A2 remove, bound-signed,  h = 21 -> P-BND-ACCEPT  (different verifier,
//!                                          `verify_multisig_excluding_at`, and
//!                                          `is_add = false` inside the preimage)
//!   IP-A3 add,    legacy-signed, h = 21 -> P-LEG-REJECT  (negative control)
//!   IP-B1 add,    legacy-signed, h =  5 -> P-LEG-ACCEPT  (below-gate parity —
//!                                          this is what keeps the five INC-I-174
//!                                          suites green, pinned here rather than
//!                                          left implicit)
//!   IP-B2 add,    bound-signed,  h =  5 -> P-BND-REJECT  (the mirror of IP-B1,
//!                                          without which IP-B1 is one-sided)
//!   IP-E1 add,    legacy-signed, h = 20 -> P-LEG-REJECT  } the boundary is `>=`,
//!   IP-E2 add,    bound-signed,  h = 20 -> P-BND-ACCEPT  } not `>`
//!   IP-E3 add,    legacy-signed, h = 19 -> P-LEG-ACCEPT  }
//!
//! MATRIX
//!   O1 x {IP-A1,A2,A3,B1,B2,E1,E2,E3} = 8 cells (all `Ok`)
//!   O2 x {IP-A1,A2,A3,B1,B2,E1,E2,E3} = 8 cells
//!   O3 x {IP-A1, IP-A2}               = 2 cells
//!   O4 x {IP-A1, IP-A2, IP-A3}        = 3 cells
//!   O5 x {IP-A1, IP-A3}               = 2 cells
//!   O6 x {IP-A1, IP-A2, IP-A3, IP-B1} = 4 cells
//!   O7 x {IP-A1,A2,A3,B1,B2,E1,E2,E3} = 8 cells
//!
//! ANTI-VACUITY PAIRING (each pair differs in exactly ONE input)
//!   IP-A1 <-> IP-A3  only WHICH BYTES were signed, at the same height
//!   IP-B1 <-> IP-B2  only WHICH BYTES were signed, at the same height
//!   IP-A3 <-> IP-B1  only the HEIGHT, with the same bytes — THE GATE
//!   IP-E3 <-> IP-E1  only the HEIGHT, one block apart — THE BOUNDARY
//!   plus `fixture_the_two_message_arms_are_distinguishable`, without which every
//!   accept/refuse pair in this file would be unfalsifiable.
//!
//! ---------------------------------------------------------------------------
//! WHAT THIS FILE DOES NOT DO (scope fence)
//! ---------------------------------------------------------------------------
//! 1. It adds NO reject condition and asserts nothing about
//!    `crates/core/src/validation/tx_types.rs`, whose `git diff HEAD` must stay
//!    EMPTY (binding user decision 1). O1 asserts the OPPOSITE: every row applies.
//! 2. It moves NO payload byte. `MaintainerChangeData` is built through the
//!    existing public API only; `inc_i_176_m1a_wire_freeze` and
//!    `inc_i_176_m1a_wire_decode` own the wire format and must stay green.
//! 3. It asserts NO expiry ENFORCEMENT. Nothing in M2 compares a height to
//!    `valid_before`; every bound message here carries
//!    `MAINTAINER_AUTH_VALID_BEFORE_UNSET`. The check is M3's.
//! 4. It pins no mainnet height, adds no `HardForkSchedule` entry, bumps no
//!    version, and edits no file under `src/`.
//! 5. It does NOT touch the five INC-I-174 suites. Their heights 0-7 are below
//!    devnet #22 = 20, which is the whole reason 20 was chosen; their `git diff
//!    HEAD` must stay 0 lines.

mod inc_i_176_m2_common;
mod inc_i_176_m2_devnet_common;

use crypto::KeyPair;
use doli_core::maintainer::{signing_message_legacy, MIN_MAINTAINERS};
use doli_core::network_params::NetworkParams;
use doli_core::Network;
use inc_i_176_m2_common::{bound_message, change_tx};
use inc_i_176_m2_devnet_common::{
    advance_to, apply_governance_block, last_derived_height, legacy_message_independent, on_disk,
    quorum, root_last_updated, root_members, root_threshold, seeded_devnet_node, ABOVE_GATE,
    AT_GATE, BELOW_GATE, DEVNET_GATE_20, DEVNET_GATE_22, EDGE_BELOW,
};

// ===========================================================================
// FIXTURE INTEGRITY — without these, every row below is unfalsifiable
// ===========================================================================

/// The harness literals must be the SHIPPED devnet gates.
#[test]
fn fixture_devnet_gate_literals_match_the_shipped_params() {
    let p = NetworkParams::defaults(Network::Devnet);
    assert_eq!(
        p.inc_i_176_auth_binding_activation_height, DEVNET_GATE_22,
        "harness: this file drives heights {}, {}, {} and {} around a devnet #22 \
         of {}. If the shipped gate moved, rows that were meant to straddle it may \
         now sit on the same side and every result below is meaningless.",
        BELOW_GATE, EDGE_BELOW, AT_GATE, ABOVE_GATE, DEVNET_GATE_22
    );
    assert_eq!(
        p.maintainer_derivation_activation_height, DEVNET_GATE_20,
        "harness: devnet #20 must still be 0 — the premise of the check below"
    );
    for h in [BELOW_GATE, EDGE_BELOW, AT_GATE, ABOVE_GATE] {
        assert!(
            h >= p.maintainer_derivation_activation_height,
            "harness: every height this file drives ({}) must sit at or above the \
             SHIPPED devnet #20 ({}), so `verify_multisig_at` takes the \
             DISTINCT-SIGNER arm on BOTH sides of #22 and the MESSAGE is the only \
             variable between rows. Below #20 the entry-counting verifier applies \
             and a row could pass for a counting reason instead of a binding one.",
            h,
            p.maintainer_derivation_activation_height
        );
    }
    assert!(
        p.inc_i_176_auth_binding_activation_height >= p.maintainer_derivation_activation_height,
        "REV-176-M1a-001 (lower half) restated at the point of use: devnet #22 \
         ({}) must never sit below #20 ({}), or the bound message would be \
         verified by the pre-INC-I-172 entry-counting counter (AUDIT-P1-016)",
        p.inc_i_176_auth_binding_activation_height,
        p.maintainer_derivation_activation_height
    );
}

/// PARITY LOCK (informational) — the in-file legacy encoder reproduces the frozen
/// format owned by `signing_message_legacy`.
///
/// This does NOT make the acceptance rows tautological: those rows sign with
/// [`legacy_message_independent`], and this test merely records that the in-file
/// copy and the crate's copy agree today. If they ever diverge, the crate changed
/// a FROZEN consensus format and this fires before any acceptance row does.
#[test]
fn fixture_the_in_file_legacy_encoder_reproduces_the_frozen_format() {
    for is_add in [true, false] {
        let target = *KeyPair::generate().public_key();
        assert_eq!(
            legacy_message_independent(is_add, &target),
            signing_message_legacy(is_add, &target),
            "PARITY LOCK: the in-file encoder `format!(\"{{}}:{{}}\", action, hex)` \
             — the same one the five INC-I-174 suites use — must still equal \
             `signing_message_legacy(is_add = {})`. A divergence means the FROZEN \
             legacy format moved, which would fork this node's maintainer trust \
             root away from every peer at historical heights.",
            is_add
        );
    }
}

/// POSITIVE CONTROL / ANTI-VACUITY — the two message arms must be different bytes.
///
/// If the legacy and bound preimages ever coincided, every "accepted here /
/// refused there" pair in this file would pass for the wrong reason.
#[tokio::test]
async fn fixture_the_two_message_arms_are_distinguishable() {
    let (node, _p, _t) = seeded_devnet_node(4).await;
    let target = *KeyPair::generate().public_key();

    for is_add in [true, false] {
        let legacy = legacy_message_independent(is_add, &target);
        let bound = bound_message(&node, is_add, &target);
        assert_ne!(
            legacy, bound,
            "POSITIVE CONTROL: the legacy and bound messages must be distinct \
             bytes for is_add={}, or the gate changes nothing and every assertion \
             in this file passes vacuously",
            is_add
        );
        assert_eq!(
            bound.len(),
            32,
            "the bound message is a BLAKE3-256 digest (REQ-176-021 / M1a wire \
             freeze), not a formatted string"
        );
    }
}

// ===========================================================================
// IP-A1 — **THE POINT OF THE FILE.** Above #22 on DEVNET the BOUND arm EXECUTES.
// ===========================================================================

/// IP-A1 x P-BND-ACCEPT. O1 O2 O3 O4 O5 O6 O7.
///
/// # This is the test devnet #22 = 20 exists to make possible.
///
/// A real devnet `Node`, real blocks applied one at a time up to height 21, and
/// an `AddMaintainer` whose signatures are over
/// `signing_message(node.params.genesis_hash, true, &target,
/// MAINTAINER_AUTH_VALID_BEFORE_UNSET)` — the BOUND bytes. The new maintainer
/// must be seated.
///
/// At a devnet gate of `u64::MAX` this arm would be unreachable on every network
/// a developer can run (mainnet #22 = `u64::MAX`, testnet #22 = `300_000`), and
/// M3's expiry check and M4's signer would be built against code that had never
/// executed. That is what `20` buys, and this test is where it is collected.
///
/// The genesis hash is read BACK OFF THE NODE by
/// `inc_i_176_m2_common::bound_message`, never from a network constant, so a
/// production site that bound to a hardcoded `ChainSpec::mainnet()` hash would
/// fail here.
#[tokio::test]
async fn req_176_022_devnet_above_the_gate_add_signed_over_the_bound_message_is_applied() {
    // 4 seated members so the ADD lands inside MAX_MAINTAINERS (5).
    let (mut node, producers, tmp) = seeded_devnet_node(4).await;
    let params = node.params.clone();

    let seated = root_members(&node).await;
    let before_threshold = root_threshold(&node).await;
    assert_eq!(before_threshold, 3, "harness: calculate_threshold(4) == 3");
    assert_eq!(
        root_last_updated(&node).await,
        0,
        "harness: the root was seeded at height 0, so a rotation at h={} is \
         unambiguously distinguishable from the seed",
        ABOVE_GATE
    );

    let prev = advance_to(&mut node, &producers[0], &params, ABOVE_GATE).await;

    let newcomer = *KeyPair::generate().public_key();
    let signers = quorum(&producers, &seated, before_threshold);
    let msg = bound_message(&node, true, &newcomer);
    let tx = change_tx(true, &newcomer, &msg, &signers);

    apply_governance_block(&mut node, &producers[0], &params, prev, ABOVE_GATE, tx).await;

    // O2 — THE acceptance oracle.
    let after = root_members(&node).await;
    assert!(
        after.contains(&newcomer),
        "O2 / REQ-176-022: at devnet height {} (>= #22 {}) the production path MUST \
         build EXACTLY `signing_message(node.params.genesis_hash, true, target, \
         MAINTAINER_AUTH_VALID_BEFORE_UNSET)` and seat the target. This is the ONE \
         place the BOUND arm actually executes: mainnet #22 is u64::MAX and \
         testnet #22 is ~146k blocks past the tip, so if this fails, no network a \
         developer can run has ever run this code and M3/M4 are being built \
         against the legacy arm alone.",
        ABOVE_GATE,
        DEVNET_GATE_22
    );
    assert_eq!(after.len(), 5, "O2: 4 seated members + the newcomer");
    // O3
    assert_eq!(
        root_threshold(&node).await,
        3,
        "O3: `add_maintainer` re-derives the threshold — a 5-member set carries 3"
    );
    // O4
    assert_eq!(
        root_last_updated(&node).await,
        ABOVE_GATE,
        "O4: the rotation must be STAMPED with the block height it came from. \
         `getMaintainerSet.last_change_block` is the fleet-wide divergence \
         instrument; membership can be right while this is wrong."
    );
    // O5
    assert_eq!(
        last_derived_height(&node).await,
        ABOVE_GATE,
        "O5: the apply path sets `last_derived_height = height` on a successful \
         rotation"
    );
    // O6 — read the file back independently.
    let disk = on_disk(&tmp);
    assert!(
        disk.set.members.contains(&newcomer),
        "O6: an in-memory-only acceptance is undone by the next restart. The \
         updater reads maintainer_state.bin to decide which keys may authorize a \
         ROOT BINARY INSTALL on this host."
    );
    assert_eq!(disk.set.members, after, "O6: memory/disk parity");
    assert_eq!(disk.set.last_updated, ABOVE_GATE, "O6: O4 must persist too");
}

// ===========================================================================
// IP-A2 — the REMOVE arm above #22. Different verifier, different `is_add`.
// ===========================================================================

/// IP-A2 x P-BND-ACCEPT. O1 O2 O3 O4 O6 O7.
///
/// Asserted separately from IP-A1 because this arm goes through a DIFFERENT
/// verifier — `verify_multisig_excluding_at`, which drops the target's own
/// signature — and takes `is_add = false`, which is the term a copy-paste of the
/// add arm gets wrong. A wiring change that bound only the add arm passes every
/// add-side test in this file and fails here.
///
/// The `false` is INSIDE the signed preimage precisely so an `add` authorization
/// can never be replayed as a `remove` (REQ-176-012).
#[tokio::test]
async fn req_176_022_devnet_above_the_gate_remove_signed_over_the_bound_message_is_applied() {
    // 5 seated members: `can_remove()` needs len > MIN_MAINTAINERS (3).
    let (mut node, producers, tmp) = seeded_devnet_node(5).await;
    let params = node.params.clone();

    let seated = root_members(&node).await;
    assert!(
        seated.len() > MIN_MAINTAINERS,
        "harness sanity: the removal must be LEGAL — {} seated members must exceed \
         MIN_MAINTAINERS {}. If it is refused for a size reason this test goes \
         green without ever exercising the bound arm.",
        seated.len(),
        MIN_MAINTAINERS
    );
    let threshold = root_threshold(&node).await;
    let victim = seated[4];

    let prev = advance_to(&mut node, &producers[0], &params, ABOVE_GATE).await;

    // Signers must EXCLUDE the target — `verify_multisig_excluding_at` drops it.
    let candidates: Vec<KeyPair> = producers
        .iter()
        .filter(|kp| *kp.public_key() != victim)
        .cloned()
        .collect();
    let signers = quorum(&candidates, &seated, threshold);
    let msg = bound_message(&node, false, &victim);
    let tx = change_tx(false, &victim, &msg, &signers);

    apply_governance_block(&mut node, &producers[0], &params, prev, ABOVE_GATE, tx).await;

    let after = root_members(&node).await;
    assert!(
        !after.contains(&victim),
        "O2 / REQ-176-022: at devnet height {} the REMOVE arm must build the BOUND \
         message with `is_add = false` and unseat the target. The remove arm uses \
         a different verifier (`verify_multisig_excluding_at`) and a different \
         action byte; a change that bound only the add arm leaves this one \
         accepting the unbound, replayable legacy authorization.",
        ABOVE_GATE
    );
    assert_eq!(after.len(), 4, "O2: 5 seated members minus the victim");
    assert_eq!(
        root_threshold(&node).await,
        3,
        "O3: a 4-member set carries threshold 3"
    );
    assert_eq!(root_last_updated(&node).await, ABOVE_GATE, "O4");
    let disk = on_disk(&tmp);
    assert!(
        !disk.set.members.contains(&victim),
        "O6: a removal that lives only in memory is undone by the next restart, \
         and the removed key keeps its install authority on this host"
    );
}

// ===========================================================================
// IP-A3 — THE NEGATIVE CONTROL. Above #22 the LEGACY bytes are NOT applied,
// and the refusal is NON-FATAL.
// ===========================================================================

/// IP-A3 x P-LEG-REJECT. O1 O2 O4 O5 O6 O7.
///
/// The half that makes the AUDIT-P0-011 closure real where it is reachable —
/// devnet, the ONLY network whose #22 the chain actually crosses today: above the
/// gate a signature over the old, collision-prone, chain-blind bytes must stop
/// working. On mainnet #22 is unpinned and the defect stays OPEN there
/// (AUDIT-P1-102). Without this,
/// IP-A1 is satisfied by a node that accepts BOTH message forms — which would
/// leave the unbound legacy authorization a valid bearer token above the gate and
/// M2 would have changed nothing.
///
/// The refusal must be **warn-and-skip**: `apply_block` returns `Ok`, the chain
/// advances, nothing panics and no block is rejected (O1/O7, asserted by
/// [`apply_governance_block`]). #22 adds no reject path — that is binding user
/// decision 1, and `crates/core/src/validation/tx_types.rs` keeps a 0-line diff.
#[tokio::test]
async fn req_176_022_devnet_above_the_gate_the_legacy_message_is_not_applied_and_is_non_fatal() {
    let (mut node, producers, tmp) = seeded_devnet_node(4).await;
    let params = node.params.clone();

    let seated = root_members(&node).await;
    let threshold = root_threshold(&node).await;
    let before_derived = last_derived_height(&node).await;

    let prev = advance_to(&mut node, &producers[0], &params, ABOVE_GATE).await;

    let newcomer = *KeyPair::generate().public_key();
    let signers = quorum(&producers, &seated, threshold);
    // The IN-FILE encoder — the same bytes the five INC-I-174 suites sign.
    let tx = change_tx(
        true,
        &newcomer,
        &legacy_message_independent(true, &newcomer),
        &signers,
    );

    // O1 + O7 are asserted inside: the block APPLIES, the chain advances, no
    // panic, no error. A refusal here must never be fatal.
    apply_governance_block(&mut node, &producers[0], &params, prev, ABOVE_GATE, tx).await;

    let after = root_members(&node).await;
    assert!(
        !after.contains(&newcomer),
        "O2 / REQ-176-022 NEGATIVE CONTROL: at devnet height {} (>= #22 {}) a \
         signature over the LEGACY bytes must NOT seat a maintainer. While it \
         does, the unbound, domain-tag-less, chain-blind authorization is still a \
         valid bearer token above the gate — AUDIT-P0-011 is open and the bound \
         arm proved by the sibling test is merely an ADDITIONAL accepted form \
         rather than a REPLACEMENT.",
        ABOVE_GATE,
        DEVNET_GATE_22
    );
    assert_eq!(
        after, seated,
        "O2: the seated set must be byte-identical to before the refused rotation"
    );
    assert_eq!(
        root_last_updated(&node).await,
        0,
        "O4: a refused authorization must not stamp the root with the block \
         height. A root that advertises `last_change_block = {}` while its \
         membership never changed reports a divergence no canonical block \
         explains.",
        ABOVE_GATE
    );
    assert_eq!(
        last_derived_height(&node).await,
        before_derived,
        "O5: `last_derived_height = height` is set only on a SUCCESSFUL rotation"
    );
    assert!(
        !on_disk(&tmp).set.members.contains(&newcomer),
        "O6: a refused authorization must not reach the trust-root FILE either — \
         the file outlives the process and is what the updater reads"
    );
}

// ===========================================================================
// IP-B1 / IP-B2 — THE BELOW-GATE PARITY CONTROL.
//
// This is exactly what keeps the five INC-I-174 suites green. Pinned here rather
// than left implicit in "those tests still pass".
// ===========================================================================

/// IP-B1 x P-LEG-ACCEPT. O1 O2 O6 O7. **Must stay green forever.**
///
/// Below devnet #22 the LEGACY bytes are still accepted — the property the five
/// INC-I-174 node suites depend on. They drive governance at block heights 0-7
/// and sign `format!("{}:{}", action, hex)` with their own in-file encoder; the
/// reason all 25 of those tests pass with a **0-line `git diff HEAD`** is that
/// heights 0-7 are below 20 and take this arm.
///
/// That dependency is asserted HERE, in the INC-I-176 diff, so a reader does not
/// have to infer it from another suite's green status — and so that if devnet #22
/// is ever lowered, this file says why it broke before those five do.
///
/// The height used is {BELOW_GATE}, deliberately ABOVE the INC-I-174 band, so a
/// failure here is unambiguously about this gate and not about that harness.
#[tokio::test]
async fn req_176_022_devnet_below_the_gate_the_legacy_message_is_still_accepted() {
    let (mut node, producers, tmp) = seeded_devnet_node(4).await;
    let params = node.params.clone();

    let seated = root_members(&node).await;
    let threshold = root_threshold(&node).await;

    let prev = advance_to(&mut node, &producers[0], &params, BELOW_GATE).await;

    let newcomer = *KeyPair::generate().public_key();
    let signers = quorum(&producers, &seated, threshold);
    let tx = change_tx(
        true,
        &newcomer,
        &legacy_message_independent(true, &newcomer),
        &signers,
    );

    apply_governance_block(&mut node, &producers[0], &params, prev, BELOW_GATE, tx).await;

    let after = root_members(&node).await;
    assert!(
        after.contains(&newcomer),
        "O2 / PARITY: below devnet #22 ({}) at height {} the production path MUST \
         still build EXACTLY the legacy `format!(\"{{}}:{{}}\", action, hex)`. \
         THIS IS WHAT KEEPS THE FIVE INC-I-174 NODE SUITES GREEN: they drive \
         governance at block heights 0-7 with that same in-file encoder, and all \
         25 of those tests must keep passing with a 0-line git diff — a hard \
         acceptance criterion of this milestone. If this fails, devnet #22 has \
         been lowered into their working band.",
        DEVNET_GATE_22,
        BELOW_GATE
    );
    assert_eq!(after.len(), 5, "O2: the newcomer was seated");
    assert!(
        on_disk(&tmp).set.members.contains(&newcomer),
        "O6: below-gate acceptance must persist exactly as above-gate acceptance \
         does — the arm changes WHICH BYTES are verified, nothing downstream"
    );
}

/// IP-B2 x P-BND-REJECT. O1 O2 O7.
///
/// The mirror of IP-B1: below the gate the BOUND bytes must NOT be accepted.
/// Without it IP-B1 is one-sided — a node that accepted anything would satisfy
/// it — and "the gate is consulted below it too" would be unproven. If the bound
/// message were accepted here, the new form would have been applied RETROACTIVELY
/// to frozen history.
#[tokio::test]
async fn req_176_022_devnet_below_the_gate_the_bound_message_is_refused() {
    let (mut node, producers, _tmp) = seeded_devnet_node(4).await;
    let params = node.params.clone();

    let seated = root_members(&node).await;
    let threshold = root_threshold(&node).await;

    let prev = advance_to(&mut node, &producers[0], &params, BELOW_GATE).await;

    let newcomer = *KeyPair::generate().public_key();
    let signers = quorum(&producers, &seated, threshold);
    let msg = bound_message(&node, true, &newcomer);
    let tx = change_tx(true, &newcomer, &msg, &signers);

    apply_governance_block(&mut node, &producers[0], &params, prev, BELOW_GATE, tx).await;

    let after = root_members(&node).await;
    assert!(
        !after.contains(&newcomer),
        "O2: below devnet #22 ({}) the production path must NOT accept a signature \
         over the BOUND message. If it does, the gate is not being consulted and \
         the new message form has been applied retroactively to frozen history.",
        DEVNET_GATE_22
    );
    assert_eq!(after, seated, "O2: the seated set is unchanged");
}

// ===========================================================================
// IP-E1 / IP-E2 / IP-E3 — THE BOUNDARY IS `>=`, NOT `>`
// ===========================================================================

/// IP-E3 <-> IP-E1 <-> IP-E2. O2 x three rows — **the off-by-one proof.**
///
/// `#22 - 1` selects LEGACY; `#22` selects BOUND. The comparison must be `>=`,
/// matching `signing_message_at` and `MaintainerSet::verify_multisig_at` exactly.
///
/// A `>` here shifts this gate ONE BLOCK relative to every other maintainer gate,
/// and both gates are read by the same code path at the same call site. One block
/// of disagreement is enough to give two nodes different maintainer roots — and
/// the maintainer root is the updater's binary-install trust root.
///
/// All three rows run on FRESH devnet nodes with the SAME fixture, so the only
/// difference between them is `(height, which bytes were signed)`. Row 3 is what
/// stops row 2's refusal from being explained by "governance is dead at height
/// {AT_GATE}".
#[tokio::test]
async fn req_176_022_devnet_boundary_is_greater_or_equal_not_greater_than() {
    // Row 1 — one block BELOW the gate, LEGACY bytes: ACCEPTED.
    {
        let (mut node, producers, _t) = seeded_devnet_node(4).await;
        let params = node.params.clone();
        let seated = root_members(&node).await;
        let threshold = root_threshold(&node).await;
        let prev = advance_to(&mut node, &producers[0], &params, EDGE_BELOW).await;

        let newcomer = *KeyPair::generate().public_key();
        let signers = quorum(&producers, &seated, threshold);
        let tx = change_tx(
            true,
            &newcomer,
            &legacy_message_independent(true, &newcomer),
            &signers,
        );
        apply_governance_block(&mut node, &producers[0], &params, prev, EDGE_BELOW, tx).await;

        assert!(
            root_members(&node).await.contains(&newcomer),
            "BOUNDARY row 1: height == #22 - 1 ({}) must still take the LEGACY arm",
            EDGE_BELOW
        );
    }

    // Row 2 — EXACTLY at the gate, LEGACY bytes: REFUSED.
    {
        let (mut node, producers, _t) = seeded_devnet_node(4).await;
        let params = node.params.clone();
        let seated = root_members(&node).await;
        let threshold = root_threshold(&node).await;
        let prev = advance_to(&mut node, &producers[0], &params, AT_GATE).await;

        let newcomer = *KeyPair::generate().public_key();
        let signers = quorum(&producers, &seated, threshold);
        let tx = change_tx(
            true,
            &newcomer,
            &legacy_message_independent(true, &newcomer),
            &signers,
        );
        apply_governance_block(&mut node, &producers[0], &params, prev, AT_GATE, tx).await;

        assert!(
            !root_members(&node).await.contains(&newcomer),
            "BOUNDARY row 2: height == #22 ({}) must ALREADY take the BOUND arm. \
             The comparison is `>=` (signing_message_at / verify_multisig_at). A \
             `>` shifts this gate one block relative to every other maintainer \
             gate, and one block of disagreement gives two nodes different \
             binary-install trust roots.",
            AT_GATE
        );
    }

    // Row 3 — EXACTLY at the gate, BOUND bytes: ACCEPTED. Anti-vacuity for row 2.
    {
        let (mut node, producers, _t) = seeded_devnet_node(4).await;
        let params = node.params.clone();
        let seated = root_members(&node).await;
        let threshold = root_threshold(&node).await;
        let prev = advance_to(&mut node, &producers[0], &params, AT_GATE).await;

        let newcomer = *KeyPair::generate().public_key();
        let signers = quorum(&producers, &seated, threshold);
        let msg = bound_message(&node, true, &newcomer);
        let tx = change_tx(true, &newcomer, &msg, &signers);
        apply_governance_block(&mut node, &producers[0], &params, prev, AT_GATE, tx).await;

        assert!(
            root_members(&node).await.contains(&newcomer),
            "BOUNDARY row 3 / ANTI-VACUITY: at height == #22 ({}) the BOUND bytes \
             must be ACCEPTED. Row 2's refusal must be caused by the arm \
             SWITCHING, not by governance being dead at that height.",
            AT_GATE
        );
    }
}
