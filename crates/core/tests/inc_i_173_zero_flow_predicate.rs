//! INC-I-173 M1 — CATEGORY A: the `allows_empty_io()` / `is_zero_flow()`
//! predicate itself (spec F1 + F3).
//!
//! TDD RED. This file does NOT compile against the current tree: neither
//! `TxType::allows_empty_io` nor `Transaction::is_zero_flow` exists yet. It is
//! kept in its own file so that compile failure cannot hide the runtime
//! evidence in `inc_i_173_fee_gate.rs`.
//!
//! Spec: `specs/state-only-fee-gate-architecture.md` F1 (one exhaustive owner),
//!       F3 (exempt set curated by AUTHORIZATION, not by wire shape).
//! Analysis: `docs/redesigns/state-only-fee-gate-redesign-analysis.md`.
//! Requirements: REQ-173-001, REQ-173-003b, REQ-173-004.
//!
//! ---------------------------------------------------------------------------
//! REQUIRED API
//! ---------------------------------------------------------------------------
//! ```ignore
//! // crates/core/src/transaction/types.rs
//! impl TxType {
//!     /// May a transaction of this type legitimately carry ZERO inputs and
//!     /// ZERO outputs and still be fee/balance exempt?
//!     ///
//!     /// MUST be an exhaustive `match` with **NO `_` arm** — that is the whole
//!     /// point of F1. `TxType` is not `#[non_exhaustive]`, so a `match` without
//!     /// a wildcard turns "somebody added a tx type and forgot to classify it"
//!     /// from a silent production defect (INC-I-057, then INC-I-173) into a
//!     /// build failure.
//!     pub const fn allows_empty_io(self) -> bool { ... }
//! }
//!
//! // crates/core/src/transaction/core.rs
//! impl Transaction {
//!     pub fn is_zero_flow(&self) -> bool {
//!         self.inputs.is_empty() && self.outputs.is_empty() && self.tx_type.allows_empty_io()
//!     }
//! }
//! ```
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT — `TxType::allows_empty_io(self) -> bool`
//! ---------------------------------------------------------------------------
//! ENUMERATION OF OBSERVABLE OUTPUTS
//!   O1: the returned `bool`.
//!   mutable params  : NONE — `self` is `Copy`, taken by value, `const fn`.
//!   receiver mutation: NONE — `const fn`, cannot mutate.
//!   persistent store : NONE — no I/O.
//!   side channels    : NONE — no logging, no panics (total function).
//! CODE PATHS
//!   P-true  : the five arms returning `true`.
//!   P-false : the nineteen arms returning `false`.
//!   (There is no third path: no `_` arm may exist, no early return, no panic.)
//! INPUT PARTITIONS — the input domain is the finite 24-element `TxType` set,
//!   so the partitions ARE the variants. Total coverage is achievable and is
//!   therefore mandatory: 24 partitions.
//!     IP-EXEMPT (5): Registration, DelegateBond, RevokeDelegation,
//!                    AddMaintainer, RemoveMaintainer
//!     IP-AUTHGAP (2): Exit, SlashProducer — 0-in/0-out AND in L1∩L2 AND in
//!                    `is_state_only()`, so every SHAPE-derived predicate would
//!                    have imported them. C1 excludes them by NAME.
//!     IP-HASOUT (2): ClaimReward, ClaimBond — carry a value output.
//!     IP-OTHER (15): the remaining variants.
//! MATRIX: O1 x (P-true|P-false) x 24 partitions = 24 cells, all covered by
//!   `exempt_set_is_exactly_the_five_authorized_types`. The per-partition
//!   named tests below are additional, deliberately redundant, witnesses for
//!   the partitions where a wrong answer is a security defect rather than a
//!   liveness defect.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT — `Transaction::is_zero_flow(&self) -> bool`
//! ---------------------------------------------------------------------------
//! ENUMERATION OF OBSERVABLE OUTPUTS
//!   O2: the returned `bool`.
//!   mutable params / receiver mutation / persistent store / side channels: NONE
//!   (`&self`, pure, total).
//! CODE PATHS — the conjunction short-circuits, giving four reachable shapes:
//!   Q1: inputs empty  && outputs empty  && allows_empty_io  -> true
//!   Q2: inputs empty  && outputs empty  && !allows_empty_io -> false
//!   Q3: inputs empty  && outputs NON-empty                  -> false  [C2 MINT GUARD]
//!   Q4: inputs NON-empty                                    -> false
//! INPUT PARTITIONS
//!   IP-Z1: 0-in/0-out, each of the 24 types            -> Q1 / Q2
//!   IP-Z2: 0-in/1-out(non-zero), each of the 24 types  -> Q3
//!   IP-Z3: 0-in/1-out(amount 0), each of the 24 types  -> Q3 (a zero-VALUE
//!          output is still a NON-EMPTY output vector; the conjunct is
//!          `outputs.is_empty()`, not `total_output() == 0`)
//!   IP-Z4: 0-in/2-out, each of the 24 types            -> Q3
//!   IP-Z5: 1-in/0-out, each of the 24 types            -> Q4
//! MATRIX: O2 x 4 paths x (24 x 5) partitions = 120 cells, covered by the
//!   total tests below.

mod inc_i_173_common;

use doli_core::transaction::{Input, Output, Transaction, TxType};
use inc_i_173_common::{
    tx_with_outputs, zero_flow_tx, ALL_TX_TYPES, EXPECTED_EXEMPT_SET, FROZEN_LEGACY_EXEMPT_SET,
};

// ===========================================================================
// REQ-173-001 (Must) + EXHAUSTIVENESS GUARD
// Acceptance: the exemption is owned by ONE predicate in `crates/core`.
// ===========================================================================

/// REQ-173-001 (Must) — the VALUE-level twin of the compile-time exhaustive
/// `match`.
///
/// The `match` with no `_` arm guarantees that a NEW variant cannot compile
/// until it is classified. It does NOT guarantee it was classified CORRECTLY,
/// and it does not stop somebody from flipping an existing arm. This test is
/// the guard for both: it pins the true-set to exactly the five names in spec
/// F3 by iterating all 24 variants.
#[test]
fn exempt_set_is_exactly_the_five_authorized_types() {
    let actual: Vec<TxType> = ALL_TX_TYPES
        .iter()
        .copied()
        .filter(|t| t.allows_empty_io())
        .collect();

    assert_eq!(
        actual.len(),
        EXPECTED_EXEMPT_SET.len(),
        "O1: allows_empty_io() must be true for EXACTLY {} types, got {:?}",
        EXPECTED_EXEMPT_SET.len(),
        actual
    );

    for expected in EXPECTED_EXEMPT_SET {
        assert!(
            actual.contains(&expected),
            "O1: {:?} must be exempt (spec F3 exempt set)",
            expected
        );
    }
    for t in ALL_TX_TYPES {
        if !EXPECTED_EXEMPT_SET.contains(&t) {
            assert!(
                !t.allows_empty_io(),
                "O1: {:?} must NOT be exempt — the exempt set is closed by spec F3, \
                 widening it is a consensus change that needs its own activation height",
                t
            );
        }
    }
}

/// REQ-173-001 (Must), constraint C3 — the STRICT SUPERSET property.
///
/// The new predicate must contain every member of the frozen legacy
/// `matches!` at `utxo.rs:222`. If it did not, crossing the activation height
/// would REMOVE an exemption that live chain history depends on — genesis
/// `Registration` would stop validating and every fresh sync would wedge.
#[test]
fn exempt_set_is_a_strict_superset_of_the_frozen_legacy_three() {
    for t in FROZEN_LEGACY_EXEMPT_SET {
        assert!(
            t.allows_empty_io(),
            "C3: {:?} is in the FROZEN legacy exempt set (utxo.rs:222) — dropping it \
             above the gate breaks genesis and fresh sync",
            t
        );
    }
    // STRICT: the new set must be genuinely larger, otherwise the incident is
    // not fixed at all.
    let new_count = ALL_TX_TYPES.iter().filter(|t| t.allows_empty_io()).count();
    assert!(
        new_count > FROZEN_LEGACY_EXEMPT_SET.len(),
        "C3: the exempt set must be a STRICT superset of the frozen three — \
         INC-I-173 exists because AddMaintainer/RemoveMaintainer are missing"
    );
}

/// REQ-173-001 (Must) — the two types the incident is actually about.
#[test]
fn maintainer_governance_types_are_exempt() {
    assert!(
        TxType::AddMaintainer.allows_empty_io(),
        "O1: AddMaintainer is 0-in/0-out and 3-of-5 multisig authorized at apply \
         (apply_block/governance.rs:36-93) — it MUST be exempt or it can never be mined"
    );
    assert!(
        TxType::RemoveMaintainer.allows_empty_io(),
        "O1: RemoveMaintainer is 0-in/0-out and 3-of-5 multisig authorized at apply \
         (apply_block/governance.rs:36-93) — it MUST be exempt or it can never be mined"
    );
}

// ===========================================================================
// REQ-173-003b (Must, spec F3 negative) — constraint C1, actor authentication
// ===========================================================================

/// REQ-173-003b (Must), constraint C1 — `Exit` is NOT exempt.
///
/// CITED REASON. `ExitData` is `{ public_key }` with no signature field
/// (`crates/core/src/transaction/data.rs:55-58`). `validate_exit_data` performs
/// NO cryptographic check (`crates/core/src/validation/tx_types.rs:11-42`). The
/// apply handler force-withdraws every bond of the NAMED public key
/// (`apply_block/tx_processing.rs:256-290`) without ever proving the submitter
/// holds that key. Exempting it from the fee gate would ship a free, anonymous,
/// forced-exit primitive against any producer (FM-3).
///
/// `Exit` is 0-in/0-out, is in L1 ∩ L2, and is in `is_state_only()` — so every
/// SHAPE-derived candidate predicate would have imported it. It is excluded by
/// NAME, which is exactly what makes F3 an authorization decision rather than a
/// shape decision.
#[test]
fn exit_is_not_exempt_because_its_apply_handler_authenticates_nobody() {
    assert!(
        !TxType::Exit.allows_empty_io(),
        "C1/FM-3: Exit must stay un-mineable. ExitData carries no signature, \
         validate_exit_data does no crypto, and the apply handler force-withdraws \
         the bonds of whatever pubkey the tx names."
    );
}

/// REQ-173-003b (Must), constraint C1 — `SlashProducer` is NOT exempt.
///
/// CITED REASON. `SlashData.reporter_signature` has ZERO verification readers
/// in `crates/` or `bins/` (grep-confirmed by the failure analyst), and the VDF
/// in the double-production evidence is a PUBLIC hash chain over the producer's
/// PUBLIC key (`crates/core/src/validation/producer.rs:12-51`) — no secret is
/// required to produce it. Evidence is therefore forgeable for ~800 ms of
/// hashing and zero DOLI (FM-1, CRITICAL). Exempting it ships a free, keyless
/// bond-destruction primitive.
///
/// Constraint C10 reinforces this: `SlashProducer` feeds the unbounded
/// thread-per-VDF pre-pass at `validation/block.rs:145-180`.
#[test]
fn slash_producer_is_not_exempt_because_its_evidence_is_forgeable_for_free() {
    assert!(
        !TxType::SlashProducer.allows_empty_io(),
        "C1/FM-1: SlashProducer must stay un-mineable. reporter_signature has zero \
         verification readers and the VDF evidence is publicly computable from the \
         victim's PUBLIC key."
    );
}

/// REQ-173-003b (Must) — anti-vacuity twin for the two negatives above.
///
/// A test that only asserts `false` passes trivially if the implementer wrote
/// `allows_empty_io() { false }` for everything. Pair each negative with a
/// positive that shares the same 0-in/0-out wire shape, so the two can only
/// both hold if the predicate genuinely DISCRIMINATES.
#[test]
fn exit_and_slash_share_the_wire_shape_of_the_exempt_types_yet_differ() {
    let exit = zero_flow_tx(TxType::Exit);
    let slash = zero_flow_tx(TxType::SlashProducer);
    let add_maintainer = zero_flow_tx(TxType::AddMaintainer);

    // Identical wire shape ...
    for t in [&exit, &slash, &add_maintainer] {
        assert!(t.inputs.is_empty(), "same shape: 0 inputs");
        assert!(t.outputs.is_empty(), "same shape: 0 outputs");
    }
    // ... opposite verdict. Shape does not imply authorization.
    assert!(!exit.is_zero_flow(), "O2: Exit is not zero-flow-exempt");
    assert!(
        !slash.is_zero_flow(),
        "O2: SlashProducer is not zero-flow-exempt"
    );
    assert!(
        add_maintainer.is_zero_flow(),
        "O2: AddMaintainer IS zero-flow-exempt — otherwise the two negatives above \
         pass vacuously against a predicate that returns false for everything"
    );
}

// ===========================================================================
// REQ-173-004 (Must), constraint C2 — THE MINT GUARD
// ===========================================================================

/// REQ-173-004 (Must), constraint C2 — TOTAL property over all 24 variants:
/// a transaction with a NON-EMPTY output vector is NEVER zero-flow.
///
/// This is the conjunct that stops `ClaimReward` / `ClaimBond` (0 inputs, one
/// value output) from riding a widened TYPE list into a free mint. It must hold
/// for every type, including the five exempt ones — the exemption is a property
/// of (type AND shape), never of type alone.
#[test]
fn no_transaction_with_outputs_is_ever_zero_flow() {
    // IP-Z2 non-zero output, IP-Z3 zero-amount output, IP-Z4 two outputs.
    for amounts in [
        &[1_000_000_000u64][..],
        &[0u64][..],
        &[1u64, u64::MAX / 2][..],
    ] {
        for t in ALL_TX_TYPES {
            let tx = tx_with_outputs(t, amounts);
            assert!(
                !tx.outputs.is_empty(),
                "fixture sanity: the tx really has outputs"
            );
            assert!(
                !tx.is_zero_flow(),
                "C2 MINT GUARD: {:?} with outputs {:?} must NEVER be zero-flow — \
                 `outputs.is_empty()` is a non-negotiable conjunct",
                t,
                amounts
            );
        }
    }
}

/// REQ-173-004 (Must), constraint C2 — the named victims.
///
/// `ClaimReward` and `ClaimBond` are in `is_state_only()` (L3) despite carrying
/// value outputs — that false doc contract is exactly the confusion INC-I-173
/// must not inherit.
#[test]
fn claim_reward_and_claim_bond_are_never_exempt() {
    for t in [TxType::ClaimReward, TxType::ClaimBond] {
        assert!(
            !t.allows_empty_io(),
            "C2: {:?} must be classified false — it carries a value output by \
             construction (transaction/core.rs:277-314)",
            t
        );
        assert!(
            !tx_with_outputs(t, &[1_000_000_000]).is_zero_flow(),
            "C2: {:?} with a 1000 DOLI output must never be exempt",
            t
        );
    }
}

// ===========================================================================
// `is_zero_flow()` composition — the four reachable paths Q1..Q4
// ===========================================================================

/// REQ-173-001 (Must) — Q1 / Q2: with 0-in/0-out, `is_zero_flow()` reduces
/// exactly to `allows_empty_io()`. No type may disagree.
#[test]
fn zero_flow_reduces_to_allows_empty_io_when_shape_is_zero_in_zero_out() {
    for t in ALL_TX_TYPES {
        let tx = zero_flow_tx(t);
        assert!(
            tx.inputs.is_empty() && tx.outputs.is_empty(),
            "fixture sanity"
        );
        assert_eq!(
            tx.is_zero_flow(),
            t.allows_empty_io(),
            "O2: for a 0-in/0-out {:?}, is_zero_flow() must equal allows_empty_io() — \
             any divergence means a second, hidden classification exists",
            t
        );
    }
}

/// REQ-173-001 (Must) — Q4: a transaction WITH inputs is never zero-flow, even
/// for an exempt type. An exempt type that spends UTXOs is a normal fee-paying
/// transaction.
#[test]
fn no_transaction_with_inputs_is_ever_zero_flow() {
    for t in ALL_TX_TYPES {
        let mut tx = zero_flow_tx(t);
        tx.inputs = vec![Input::new(crypto::hash::hash(b"inc-i-173-prev"), 0)];
        assert!(
            !tx.is_zero_flow(),
            "O2: {:?} WITH an input must never be zero-flow — `inputs.is_empty()` \
             is a non-negotiable conjunct",
            t
        );
    }
}

/// REQ-173-001 (Must) — `allows_empty_io` is `const`, total and side-effect
/// free: calling it twice on the same variant yields the same answer, and it
/// is usable in a `const` context (which the exhaustive-match requirement in
/// F1 depends on).
///
/// HARNESS FIX (developer, INC-I-173 M1): `clippy::assertions_on_constants` is
/// allowed here DELIBERATELY. Asserting on a `const` is the whole point — the
/// two constants below only exist if `allows_empty_io` is `const`-evaluable, so
/// the lint is firing on the property under test. The assertions themselves are
/// unchanged and still run. The lint is unavoidable at the workspace
/// `-D warnings` gate and could not surface before M1, because the file did not
/// compile until the API existed.
#[test]
#[allow(clippy::assertions_on_constants)]
fn allows_empty_io_is_a_pure_total_const_function() {
    const REGISTRATION_IS_EXEMPT: bool = TxType::Registration.allows_empty_io();
    const EXIT_IS_EXEMPT: bool = TxType::Exit.allows_empty_io();
    assert!(REGISTRATION_IS_EXEMPT, "must be evaluable in const context");
    assert!(!EXIT_IS_EXEMPT, "must be evaluable in const context");

    for t in ALL_TX_TYPES {
        assert_eq!(
            t.allows_empty_io(),
            t.allows_empty_io(),
            "O1: {:?} — the predicate must be pure",
            t
        );
    }
}

/// Fixture anti-vacuity: `ALL_TX_TYPES` really does list 24 DISTINCT variants.
/// If a copy/paste duplicated one, every "for all 24" test above would silently
/// under-cover.
#[test]
fn all_tx_types_lists_twenty_four_distinct_variants() {
    assert_eq!(ALL_TX_TYPES.len(), 24, "TxType has 24 live variants");
    for (i, a) in ALL_TX_TYPES.iter().enumerate() {
        for (j, b) in ALL_TX_TYPES.iter().enumerate() {
            if i != j {
                assert_ne!(a, b, "duplicate variant at {} and {}: {:?}", i, j, a);
            }
        }
    }
    // Round-trip through the discriminant closure `from_u32` — this is what
    // makes "24" a fact about the type and not about this array.
    for t in ALL_TX_TYPES {
        let disc = t as u32;
        assert_eq!(
            TxType::from_u32(disc),
            Some(t),
            "{:?} (discriminant {}) must round-trip through from_u32",
            t,
            disc
        );
    }
    // The retired discriminants must stay retired.
    for retired in [23u32, 24, 25, 26, 27, 28, 29, 30] {
        assert_eq!(
            TxType::from_u32(retired),
            None,
            "discriminant {} is retired and must not resurrect",
            retired
        );
    }
}

/// Fixture anti-vacuity for the mint-guard test: `tx_with_outputs` must really
/// produce the `Output` shape the fee gate reads.
#[test]
fn fixture_tx_with_outputs_builds_real_outputs() {
    let tx: Transaction = tx_with_outputs(TxType::ClaimReward, &[42]);
    assert_eq!(tx.outputs.len(), 1);
    let o: &Output = &tx.outputs[0];
    assert_eq!(o.amount, 42);
    assert_eq!(tx.total_output(), 42);
}
