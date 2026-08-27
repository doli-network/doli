//! INC-I-173 M3 — ITEM 6 / spec F7: the CROSS-LIST TOTAL test (REQ-173-011).
//!
//! For ALL 24 `TxType` variants: `allows_empty_io(t) => t ∈ L1 ∧ t ∈ L2`, where
//! L1 and L2 are the negated exclusion chains at
//! `crates/core/src/validation/transaction.rs:39-63` (the "must have inputs"
//! chain, `[ERRTX001]`) and `:67-88` (the "must have outputs" chain,
//! `[ERRTX002]`).
//!
//! ---------------------------------------------------------------------------
//! WHY THIS TEST EXISTS AND WHY IT IS THE WHOLE ITEM
//! ---------------------------------------------------------------------------
//! M1 made `TxType::allows_empty_io()` an EXHAUSTIVE match with no `_` arm, so
//! rustc refuses to build if a new variant is unclassified. That catches
//! omission. It does NOT catch the failure this test exists for: a type
//! classified `true` in `allows_empty_io` while L1 or L2 still REJECTS its 0-in
//! or 0-out shape. Such a type never reaches the fee gate at all — it dies
//! structurally two checks earlier — so the exemption is INERT and the fix
//! SHIPS DOING NOTHING. That is precisely how AddMaintainer/RemoveMaintainer came
//! to be relayable but unmineable in the first place.
//!
//! L1 and L2 STAY CHARACTER-IDENTICAL (Prohibition 3). F7 is a TEST, not an edit.
//!
//! ---------------------------------------------------------------------------
//! MEMBERSHIP IS PROBED BEHAVIOURALLY, NEVER BY COPYING THE EXPRESSIONS
//! ---------------------------------------------------------------------------
//! The contract is explicit: "Do not copy the two expressions into the test; a
//! copy drifts silently and proves nothing." A copied predicate is a SECOND
//! hand-maintained list — the exact defect class of this incident. So membership
//! is established by CALLING `validate_transaction` with a 0-in/0-out transaction
//! of each type and reading which error, if any, comes back:
//!
//!   * `[ERRTX001]` in the message  => the type is NOT in L1
//!   * `[ERRTX002]` in the message  => the type is NOT in L2
//!   * any other error, or `Ok`     => the chain let this shape through, which
//!     is what F7 requires
//!
//! The error strings are stable, load-bearing identifiers — they are the same
//! anchors the M1 suite reads, and they are what an operator greps for.
//!
//! **The two chains MUST be probed with two DIFFERENT transactions.** L1 returns
//! EARLY (`transaction.rs:59`), so a 0-in/0-out transaction of a type excluded by
//! both chains reports `[ERRTX001]` and never reaches L2 — L2 membership would be
//! silently unmeasurable and every L2 answer would be "in L2" by default. This
//! was observed, not reasoned: the first draft of this file probed both chains
//! with one 0-in/0-out transaction and its own anti-vacuity test caught it
//! (`Transfer` reported "in L2"). So:
//!   * L1 is probed with **0 inputs and TWO outputs** — only L1 can fire.
//!   * L2 is probed with **ONE input and 0 outputs** — L1 is satisfied, so only
//!     L2 can fire.
//!
//! Neither probe is the 0-in/0-out shape itself; that shape is what the FEE GATE
//! sees, and the M1 suite already covers it. F7 is about the two STRUCTURAL
//! chains that stand in front of it.
//!
//! **TWO outputs, not one, and this is load-bearing.** `Transaction::is_coinbase()`
//! is defined by SHAPE, not by type: `tx_type == Transfer && inputs.is_empty() &&
//! outputs.len() == 1` (`crates/core/src/transaction/core.rs:122-124`). A
//! `Transfer` with 0 inputs and exactly ONE output therefore satisfies
//! `is_coinbase()`, L1's first exclusion, and passes the chain — a 0-input
//! `Transfer` returns `Ok(())`. This was MEASURED, not reasoned: the one-output
//! draft of this probe reported `Transfer` as "in L1" and the anti-vacuity test
//! below caught it. A second output breaks the coinbase shape and nothing else,
//! since `is_coinbase()` is the only output-COUNT-sensitive predicate in either
//! chain.
//!
//! ---------------------------------------------------------------------------
//! TDD STATUS — HONEST
//! ---------------------------------------------------------------------------
//! This file COMPILES and is EXPECTED TO PASS against the tree at `32e0a650`:
//! M1 already aligned the five exempt types with L1/L2. It is a REGRESSION LOCK,
//! not a red test, and the contract asks for it on those terms ("Pure test").
//! Its value is forward-looking: it fires the moment someone flips a sixth type
//! to `true` in `allows_empty_io` without checking L1/L2 — which the exhaustive
//! match cannot detect, because a `true` arm always compiles.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT — `validate_transaction(tx, ctx)` used as an L1/L2 PROBE
//! ---------------------------------------------------------------------------
//! ENUMERATION OF OBSERVABLE OUTPUTS
//!   O1: presence of `[ERRTX001]` in the returned error text -> "not in L1".
//!   O2: presence of `[ERRTX002]` in the returned error text -> "not in L2".
//!   O3: the derived implication `allows_empty_io(t) => in_L1(t) && in_L2(t)`,
//!       evaluated for all 24 variants. THE property.
//!   O4: the CONTRAPOSITIVE population — that at least one variant IS excluded by
//!       each chain, so O1/O2 are shown to be real discriminators rather than
//!       predicates that never fire.
//!   mutable params   : NONE. receiver mutation: NONE. persistent store: NONE.
//!   side channels    : `tracing` only. DECLARED UNASSERTED.
//! CODE PATHS: L1 (`:39-63`) and L2 (`:67-88`), each with a taken and a not-taken
//!   branch. L1 short-circuits, so the two are probed independently.
//!
//! ---------------------------------------------------------------------------
//! INPUT PARTITIONS
//! ---------------------------------------------------------------------------
//!   IP-T  all 24 `TxType` variants (`ALL_TX_TYPES`, the value-level twin of the
//!         exhaustive match inside `allows_empty_io`)
//!   IP-S1 shape = 0 inputs, TWO outputs -> probes L1 in isolation.
//!         TWO, never one: `Transaction::is_coinbase()` is defined by SHAPE
//!         (`Transfer` + 0 inputs + EXACTLY one output), and L1's first exclusion
//!         lets a coinbase through, so a one-output probe would validate `Ok(())`
//!         for `Transfer` and report it as "in L1" without L1 ever firing
//!         (Amendment 2). The line here read "ONE output" until M3 QA iteration 1
//!         (OBS-8); the code and the prose at `:144` always used two.
//!   IP-S2 shape = ONE input, 0 outputs  -> probes L2 in isolation
//!   IP-H1 height = `ABOVE_GATE`
//!   IP-H2 height = `BELOW_GATE`
//!         L1/L2 read no activation height, so IP-H1 and IP-H2 MUST agree;
//!         asserting that is what proves the probe reads the structural chains
//!         and not the INC-I-173 fee gate.
//! MATRIX: (O1) x IP-T x IP-S1 x {IP-H1,IP-H2};
//!         (O2) x IP-T x IP-S2 x {IP-H1,IP-H2};
//!         (O3) x IP-T; (O4) x {Transfer} u IP-T.

mod inc_i_173_common;

use crypto::Hash;
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{Input, Output, Transaction, TxType};
use doli_core::validation::{validate_transaction, ValidationContext};
use doli_core::Network;
use inc_i_173_common::{payload_for, ABOVE_GATE, ALL_TX_TYPES, BELOW_GATE, TEST_AH};

/// The `[ERRTX001]` anchor emitted by the L1 chain
/// (`crates/core/src/validation/transaction.rs:60`).
const ERR_NEEDS_INPUTS: &str = "[ERRTX001]";
/// The `[ERRTX002]` anchor emitted by the L2 chain
/// (`crates/core/src/validation/transaction.rs:85`).
const ERR_NEEDS_OUTPUTS: &str = "[ERRTX002]";

fn ctx_at(height: u64) -> ValidationContext {
    ValidationContext::new(ConsensusParams::mainnet(), Network::Mainnet, 0, height)
        .with_inc_i_173_activation_height(TEST_AH)
}

fn verdict_text(tx: &Transaction, height: u64) -> String {
    match validate_transaction(tx, &ctx_at(height)) {
        Ok(()) => String::new(),
        Err(e) => e.to_string(),
    }
}

/// `(in_L1, in_L2)` for `t`, established BEHAVIOURALLY with two SEPARATE probes.
///
/// L1 returns EARLY, so one transaction cannot measure both chains — see the
/// header note. Whatever happens after a chain is passed (a type-specific
/// structural refusal, the AMM gate, the fee gate, or `Ok`) is irrelevant and is
/// deliberately not asserted: each probe answers ONE question, "did this chain
/// let this type through".
fn membership(t: TxType, height: u64) -> (bool, bool) {
    // IP-S1 — 0 inputs, TWO outputs. Only L1 can fire. TWO because ONE would
    // make a `Transfer` satisfy the shape-based `is_coinbase()` and skip L1
    // entirely (see the header note; measured, not reasoned).
    let recipient = crypto::hash::hash(b"inc-i-173-m3-f7");
    let l1_probe = Transaction {
        version: 1,
        tx_type: t,
        inputs: vec![],
        outputs: vec![
            Output::normal(1_000_000, recipient),
            Output::normal(1_000_000, recipient),
        ],
        extra_data: payload_for(t),
    };
    // IP-S2 — ONE input, 0 outputs. L1 is satisfied, so only L2 can fire.
    let l2_probe = Transaction {
        version: 1,
        tx_type: t,
        inputs: vec![Input::new(Hash::ZERO, 0)],
        outputs: vec![],
        extra_data: payload_for(t),
    };

    (
        !verdict_text(&l1_probe, height).contains(ERR_NEEDS_INPUTS),
        !verdict_text(&l2_probe, height).contains(ERR_NEEDS_OUTPUTS),
    )
}

// ===========================================================================
// O3 — THE property
// ===========================================================================

/// REQ-173-011 (Must) — for ALL 24 `TxType` variants,
/// `allows_empty_io(t) => t ∈ L1 ∧ t ∈ L2`.
///
/// The exhaustive `match` in `allows_empty_io` guarantees every variant has an
/// ANSWER; it guarantees nothing about whether that answer is REACHABLE. A type
/// marked `true` while L1 or L2 still rejects its shape is exempted from a fee
/// check it can never arrive at — an inert fix, indistinguishable from no fix,
/// and exactly the defect that kept AddMaintainer/RemoveMaintainer unmineable
/// while every list in the tree said they were state-only.
///
/// Driven at BOTH a below-gate and an above-gate height. L1 and L2 are UNGATED,
/// so the two answers must agree; a disagreement means the probe is picking up
/// the fee gate instead of the structural chains and this test is measuring the
/// wrong thing.
#[test]
fn req_173_011_every_empty_io_type_is_accepted_by_both_shape_chains() {
    for t in ALL_TX_TYPES {
        let (l1_above, l2_above) = membership(t, ABOVE_GATE);
        let (l1_below, l2_below) = membership(t, BELOW_GATE);

        assert_eq!(
            (l1_above, l2_above),
            (l1_below, l2_below),
            "INSTRUMENT SOUNDNESS ({:?}): L1 and L2 are UNGATED \
             (validation/transaction.rs:39-88 reads no activation height), so \
             membership must be identical above and below the gate. Above: \
             (L1={}, L2={}); below: (L1={}, L2={}). A difference means this probe \
             is reading the INC-I-173 fee gate rather than the shape chains.",
            t,
            l1_above,
            l2_above,
            l1_below,
            l2_below
        );

        if t.allows_empty_io() {
            assert!(
                l1_above,
                "REQ-173-011 / F7 ({:?}): `allows_empty_io` says this type may be \
                 0-in/0-out, but the L1 chain at validation/transaction.rs:39-63 \
                 still rejects it with {}. The exemption is INERT — the \
                 transaction dies two checks BEFORE the fee gate, so the fix ships \
                 doing nothing. Add the type to the L1 exclusion chain (L1/L2 \
                 themselves stay character-identical otherwise, Prohibition 3).",
                t, ERR_NEEDS_INPUTS
            );
            assert!(
                l2_above,
                "REQ-173-011 / F7 ({:?}): `allows_empty_io` says this type may be \
                 0-in/0-out, but the L2 chain at validation/transaction.rs:67-88 \
                 still rejects it with {}. Same inert-fix failure as L1.",
                t, ERR_NEEDS_OUTPUTS
            );
        }
    }
}

// ===========================================================================
// O4 — ANTI-VACUITY: the probe really discriminates
// ===========================================================================

/// REQ-173-011 (Must) — the L1/L2 probe is a real discriminator.
///
/// Without this, `req_173_011_every_empty_io_type_is_accepted_by_both_shape_chains`
/// would also pass against a `validate_transaction` that never emitted
/// `[ERRTX001]` or `[ERRTX002]` at all — a broken validator would look like a
/// perfectly aligned one. At least one variant must be excluded by EACH chain,
/// and `Transfer` is the canonical one: it needs both inputs and outputs.
#[test]
fn req_173_011_the_shape_probe_actually_fires() {
    let (l1, l2) = membership(TxType::Transfer, ABOVE_GATE);
    assert!(
        !l1,
        "ANTI-VACUITY: a 0-input `Transfer` must be rejected by L1 with {}. If it \
         is not, this probe never fires and the whole cross-list test is vacuous.",
        ERR_NEEDS_INPUTS
    );
    assert!(
        !l2,
        "ANTI-VACUITY: a 0-output `Transfer` must be rejected by L2 with {}",
        ERR_NEEDS_OUTPUTS
    );

    let excluded_by_l1 = ALL_TX_TYPES
        .iter()
        .filter(|t| !membership(**t, ABOVE_GATE).0)
        .count();
    let excluded_by_l2 = ALL_TX_TYPES
        .iter()
        .filter(|t| !membership(**t, ABOVE_GATE).1)
        .count();
    assert!(
        excluded_by_l1 > 0 && excluded_by_l2 > 0,
        "ANTI-VACUITY: L1 excluded {} types and L2 excluded {} of 24. Both must be \
         non-zero, or the chains are not being exercised.",
        excluded_by_l1,
        excluded_by_l2
    );
}

/// REQ-173-011 (Must) — the exempt set is EXACTLY five, and every member of it
/// is in both chains.
///
/// The implication in the main test is vacuously true if NOTHING is
/// `allows_empty_io`. This pins the population: five types, named, matching
/// `EXPECTED_EXEMPT_SET` in the shared fixture. It is the same guard the M1 suite
/// applies to the predicate; here it guards the CROSS-LIST claim.
#[test]
fn req_173_011_the_exempt_population_is_exactly_the_five_curated_types() {
    let exempt: Vec<TxType> = ALL_TX_TYPES
        .iter()
        .copied()
        .filter(|t| t.allows_empty_io())
        .collect();

    assert_eq!(
        exempt.len(),
        5,
        "REQ-173-011: the exempt set must hold exactly 5 types (Registration, \
         DelegateBond, RevokeDelegation, AddMaintainer, RemoveMaintainer); found \
         {:?}. If a sixth was added, the cross-list assertion above must be \
         re-read as a NEW claim about that type's L1/L2 membership, not as an \
         unchanged regression lock.",
        exempt
    );
    for t in &exempt {
        let (l1, l2) = membership(*t, ABOVE_GATE);
        assert!(
            l1 && l2,
            "REQ-173-011 ({:?}): an exempt type must be in BOTH chains (L1={}, \
             L2={})",
            t,
            l1,
            l2
        );
    }
}

/// REQ-173-011 (Should) — the total table, printed.
///
/// Not an assertion of new behaviour: it fails only if the counts are
/// self-inconsistent. Its purpose is that when the main test DOES fire, the
/// reviewer has the full 24-row picture in the same run rather than one variant's
/// name.
#[test]
fn req_173_011_total_table_is_self_consistent() {
    let mut rows = Vec::new();
    for t in ALL_TX_TYPES {
        let (l1, l2) = membership(t, ABOVE_GATE);
        rows.push(format!(
            "  {:>18?}  allows_empty_io={:<5}  L1={:<5}  L2={:<5}",
            t,
            t.allows_empty_io(),
            l1,
            l2
        ));
    }
    assert_eq!(
        rows.len(),
        24,
        "REQ-173-011: `ALL_TX_TYPES` must cover all 24 live variants; the array is \
         the value-level twin of the compile-time exhaustive match inside \
         `allows_empty_io`. Table:\n{}",
        rows.join("\n")
    );

    let violations: Vec<&String> = ALL_TX_TYPES
        .iter()
        .zip(rows.iter())
        .filter(|(t, _)| {
            let (l1, l2) = membership(**t, ABOVE_GATE);
            t.allows_empty_io() && !(l1 && l2)
        })
        .map(|(_, row)| row)
        .collect();
    assert!(
        violations.is_empty(),
        "REQ-173-011 / F7: {} type(s) are `allows_empty_io = true` but are \
         REJECTED by L1 and/or L2, so their exemption is inert:\n{}\n\nFull \
         table:\n{}",
        violations.len(),
        violations
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        rows.join("\n")
    );
}
