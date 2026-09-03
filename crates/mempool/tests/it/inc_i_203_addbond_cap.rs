//! INC-I-203 M1 — RED. The shared AddBond cap wrapper does not exist yet.
//!
//! covers: addbond_cap.rs, lib.rs, main.rs, holdings.rs
//!
//! Analysis: `docs/bugfixes/inc-i-203-analysis.md` §F (SSF) + §G.
//! RED evidence: `docs/.workflow/inc-i-203-M1-test-red-evidence.txt`.
//!
//! ===========================================================================
//! THIS FILE IS COMPILE-RED ON PURPOSE
//! ===========================================================================
//! `MAX_BONDS_PER_PRODUCER` is enforced at exactly one place,
//! `validate_block_economics` (`validation_checks.rs:1212`). The mempool has
//! carried the data since INC-I-180 M2 — `ProducerHoldings.bond_count` and
//! `.pending_addbond` are the gate's own two terms — but nothing joins them to
//! `check_addbond_cap`. The joint must be ONE function so the builder
//! (`production/withdrawal_holdings.rs`), `add_transaction` and `revalidate`
//! cannot drift apart; a second copy of the expression is the INC-I-180 M2
//! `allowance_with` lesson repeated.
//!
//! REQUIRED API — this is the contract, not a suggestion:
//!
//! ```ignore
//! // crates/mempool/src/addbond_cap.rs, re-exported from lib.rs
//! pub fn addbond_cap_verdict(
//!     tx: &Transaction,
//!     holdings: &HoldingsLookup,
//!     in_block_prior: u32,
//!     height: u64,
//!     activation_height: u64,
//! ) -> Result<(), String>;
//! ```
//!
//! `Err(String)` carries the bracketed `[ADDBOND_CAP_EXCEEDED]` code the fleet
//! greps, matching `WithdrawalParity::allow`'s `Result<(), String>` shape so
//! the builder can adopt it without a conversion layer.
//!
//! `holdings` is a `&HoldingsLookup`, NOT a `&ProducerHoldings`: the fail-open
//! decision for `Unavailable` belongs inside the shared function. Placing it at
//! each call site is how three sites drift.
//!
//! ===========================================================================
//! OUTPUT CONTRACT: fn addbond_cap_verdict_is_the_gate_expression_minus_in_block
//! ===========================================================================
//! Function under test: `addbond_cap_verdict(&Transaction, &HoldingsLookup,
//!   u32, u64, u64) -> Result<(), String>`
//!   O1 the returned `Result` discriminant
//!   O2 the bracketed error code inside the `Err` payload
//!   No other output exists: every parameter is `&`, there is no receiver, and
//!   the function writes to no store, global or channel. It is pure by
//!   construction — that is the property that lets three call sites share it.
//! PATHS
//!   P-PRE    `height < activation_height` — the gate is a no-op
//!   P-OVER   post-AH, `current + pending + in_block_prior + requested > CAP`
//!   P-UNDER  post-AH, the same sum at or below the cap
//!   P-NOANSWER `HoldingsLookup::Unavailable` — the check does not run
//! INPUT PARTITIONS
//!   IP-BOUNDARY sums of 2999 / 3000 / 3001 across the `>` comparison
//!   IP-PRIOR    `in_block_prior` at 0 and positive (the block-local term)
//!   IP-SATURATE `u32::MAX` terms — the arithmetic must clamp, not panic
//! MATRIX (every enumerated cell has an assertion)
//!   O1,O2 × P-OVER     × IP-BOUNDARY  → leg (a)
//!   O1    × P-PRE      × IP-BOUNDARY  → leg (b)
//!   O1    × P-NOANSWER × IP-BOUNDARY  → leg (c)
//!   O1    × {P-OVER,P-UNDER} × {IP-BOUNDARY,IP-PRIOR,IP-SATURATE}
//!                                     → leg (d), 50+ deterministic tuples
//!
//! ===========================================================================
//! M6 / REV-203-001 + REV-203-011 — COMPILE-RED additions
//! ===========================================================================
//! REQUIRED API, on top of the block above:
//!
//! ```ignore
//! // crates/mempool/src/holdings.rs
//! pub enum HoldingsLookup { Unavailable, Unregistered { pending_addbond: u32 }, Found(ProducerHoldings) }
//! // crates/core/src/validation/tx_types.rs, re-exported from validation
//! pub fn count_bond_outputs(tx: &Transaction) -> u32;
//! ```
//!
//! OUTPUT CONTRACT: fn unregistered_producer_is_evaluated_not_failed_open
//!   Function under test: `addbond_cap_verdict` (signature above).
//!   O1 the returned `Result` discriminant; O2 the bracketed code in `Err`.
//!   Pure: every parameter is `&`, no receiver, no store/global/channel write.
//! PATHS: P-ABSENT (`Unregistered{..}` — the source ANSWERED "not a producer");
//!   P-NOANSWER (`Unavailable` — no source could answer, REQ-BOND-006 fail-open).
//! INPUT PARTITIONS: IP-ABSENT-OVERSIZE (0 + 0 + 3001), IP-ABSENT-PENDING
//!   (0 + 2000 + 1500), IP-ABSENT-ROOM (0 + 0 + 2), IP-BLIND-OVERSIZE.
//! MATRIX: O1,O2 × P-ABSENT × {OVERSIZE, PENDING} ; O1 × P-ABSENT × ROOM ;
//!   O1 × P-NOANSWER × BLIND-OVERSIZE → this test. [RED]
//!
//! OUTPUT CONTRACT: fn absent_producer_expression_rejects_a_strict_subset_of_consensus
//!   Functions under test: `addbond_cap_verdict` and `check_addbond_cap`.
//!   O1 the returned `Result` discriminant of each. Both pure.
//! PATHS: P-ABSENT at `height >= activation_height`.
//! INPUT PARTITIONS: 200 deterministic `(pending, requested, in_block_prior)`
//!   triples at `current = 0`, straddling sums of 2999 / 3000 / 3001.
//! MATRIX: O1 × P-ABSENT × all 200 triples → this test. [RED]
//!
//! OUTPUT CONTRACT: fn count_bond_outputs_equals_the_gate_inline_expression
//!   Function under test: `count_bond_outputs(&Transaction) -> u32`. Pure.
//!   O1 the returned count.
//! PATHS: P-COUNT (the only path; `u32::try_from` saturates on overflow).
//! INPUT PARTITIONS: 0 Bond outputs, 1, mixed Bond+Normal, 3001.
//! MATRIX: O1 × P-COUNT × all four → this test. [RED — the helper is absent]

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::network::Network;
use doli_core::transaction::{Input, Output, OutputType, Transaction};
use doli_core::validation::{check_addbond_cap, count_bond_outputs};
use mempool::{HoldingsLookup, ProducerHoldings};

/// `MAX_BONDS_PER_PRODUCER` (`consensus/constants.rs:390`).
const CAP: u32 = doli_core::MAX_BONDS_PER_PRODUCER;

/// Mainnet and testnet pin the gate to `0` (`defaults.rs:160,449`); devnet
/// freezes it at `u64::MAX` (`:711`). Both bands are exercised below.
const ACTIVE_AH: u64 = 0;
const FROZEN_AH: u64 = u64::MAX;

const HEIGHT: u64 = 100_001;

/// An `AddBond` carrying `requested` Bond outputs — `requested` at the gate is
/// the COUNT of `OutputType::Bond` outputs (`validation_checks.rs:1208-1211`),
/// so the transaction, not a bare integer, is what the wrapper must read.
fn add_bond(pk: &PublicKey, requested: u32) -> Transaction {
    let unit = Network::Testnet.bond_unit();
    let funding = Hash::from_bytes([0x2C; 32]);
    let mut inp = Input::new(funding, 0);
    inp.public_key = Some(*pk);
    Transaction::new_add_bond(vec![inp], *pk, requested, unit * requested as u64, u64::MAX)
}

fn found(bond_count: u32, pending_addbond: u32) -> HoldingsLookup {
    HoldingsLookup::Found(ProducerHoldings {
        bond_count,
        pending_addbond,
        withdrawal_pending: 0,
    })
}

/// A fixed-seed xorshift64*. Deterministic across runs and platforms, and it
/// adds no dependency — a flaky property test is worse than none.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: u32) -> u32 {
        (self.next_u64() % bound as u64) as u32
    }
}

/// REQ-BOND-002, REQ-BOND-004, REQ-BOND-005, REQ-BOND-006 — Decision: a failure
/// means the mempool and the builder evaluate a cap expression that is not the
/// gate's, so either an over-cap AddBond still reaches a block (the INC-I-203
/// wasted slot) or a within-cap one is refused at admission (censorship the
/// gate never asked for).
///
/// **COMPILE-RED today**: `mempool::addbond_cap` does not exist.
///
/// Leg (d) is what makes REQ-BOND-004 verifiable against the REAL code path
/// instead of against a second copy of the expression: it drives the wrapper
/// and `check_addbond_cap` over the same tuples and requires the verdicts to be
/// identical. A hand-written re-derivation inside the wrapper fails here.
#[test]
fn addbond_cap_verdict_is_the_gate_expression_minus_in_block() {
    let p = KeyPair::from_seed([203; 32]);
    let pk = *p.public_key();

    // (a) over cap → reject, with the code the fleet greps.
    let tx = add_bond(&pk, 2);
    let over =
        mempool::addbond_cap::addbond_cap_verdict(&tx, &found(CAP - 1, 0), 0, HEIGHT, ACTIVE_AH);
    let err = over.expect_err(
        "REQ-BOND-002: 2999 held + 0 pending + 2 requested = 3001 > 3000. The gate \
         rejects this block; the filter must reject the transaction first, before it \
         is gossiped and before a producer burns a slot packing it.",
    );
    assert!(
        err.contains("[ADDBOND_CAP_EXCEEDED]"),
        "O2: the rejection must carry the bracketed code `ValidationError::code()` \
         already emits for this variant (`validation/error.rs:501`) — the fleet greps \
         codes, not prose. got: {err}"
    );

    // Boundary: 2998 + 2 == 3000 is NOT `> 3000`.
    assert!(
        mempool::addbond_cap::addbond_cap_verdict(&tx, &found(CAP - 2, 0), 0, HEIGHT, ACTIVE_AH)
            .is_ok(),
        "REQ-BOND-004 OVER-REJECTION: the comparison is `>`, not `>=`. Filling the \
         cap exactly must be allowed — the gate allows it."
    );

    // The block-local term must count.
    assert!(
        mempool::addbond_cap::addbond_cap_verdict(&tx, &found(CAP - 2, 0), 1, HEIGHT, ACTIVE_AH)
            .is_err(),
        "REQ-BOND-004: `in_block_prior` is a real term. 2998 + 0 + 1 + 2 = 3001 > \
         3000, and the builder tallies it across the selection loop \
         (`production/withdrawal_holdings.rs:24`)."
    );

    // Saturating arithmetic — adversarial input must clamp, not panic.
    assert!(
        mempool::addbond_cap::addbond_cap_verdict(
            &add_bond(&pk, 1),
            &found(u32::MAX, u32::MAX),
            u32::MAX,
            HEIGHT,
            ACTIVE_AH
        )
        .is_err(),
        "REQ-BOND-002: saturating terms must reject without overflow panic"
    );

    // (b) below the activation height → allow. Devnet freezes the gate.
    assert!(
        mempool::addbond_cap::addbond_cap_verdict(&tx, &found(CAP - 1, 0), 0, HEIGHT, FROZEN_AH)
            .is_ok(),
        "REQ-BOND-005: below the activation height the gate is a no-op, so the filter \
         must be one too. A height-blind filter changes selection and admission on a \
         band that already has history."
    );

    // (c) no source answered → allow. Over-rejection here is censorship.
    assert!(
        mempool::addbond_cap::addbond_cap_verdict(
            &tx,
            &HoldingsLookup::Unavailable,
            0,
            HEIGHT,
            ACTIVE_AH
        )
        .is_ok(),
        "REQ-BOND-006 FAIL-CLOSED: `holdings.rs:9-11` fixes `Unavailable` to mean \
         SKIP THE CHECK. Refusing here censors every producer while the live \
         ProducerSet handle is write-contended, and under-rejection is still caught \
         by the builder and by consensus."
    );

    // (d) verdict identity against the real rule, over a deterministic spread.
    let mut rng = Rng(0x0000_0203_1203_2026);
    let mut tuples: Vec<(u32, u32, u32, u32)> = vec![
        (CAP - 1, 0, 1, 0),
        (CAP - 1, 0, 2, 0),
        (CAP - 2, 0, 2, 0),
        (CAP - 2, 0, 2, 1),
        (CAP, 0, 1, 0),
        (CAP - 3, 1, 2, 0),
        (0, 0, CAP, 0),
        (0, 0, CAP, 1),
    ];
    while tuples.len() < 50 {
        let current = rng.below(CAP + 8);
        let pending = rng.below(16);
        let requested = 1 + rng.below(8);
        let prior = rng.below(16);
        tuples.push((current, pending, requested, prior));
    }

    let (mut rejecting, mut accepting) = (0usize, 0usize);
    for (current, pending, requested, prior) in tuples {
        let tx = add_bond(&pk, requested);
        let wrapper = mempool::addbond_cap::addbond_cap_verdict(
            &tx,
            &found(current, pending),
            prior,
            HEIGHT,
            ACTIVE_AH,
        );
        let rule = check_addbond_cap(
            current,
            pending.saturating_add(prior),
            requested,
            HEIGHT,
            ACTIVE_AH,
        );
        assert_eq!(
            wrapper.is_err(),
            rule.is_err(),
            "REQ-BOND-004 PARITY BREAK at (current={current}, pending={pending}, \
             requested={requested}, in_block_prior={prior}): the wrapper says \
             {wrapper:?} while `check_addbond_cap(current, pending + in_block_prior, \
             requested)` says {rule:?}. The wrapper must CALL the rule, not restate \
             it — and it must NOT reach for `ProducerHoldings::allowance_with()`, \
             which subtracts `withdrawal_pending` the gate never subtracts \
             (`inc-i-203-analysis.md:337-344`)."
        );
        if wrapper.is_err() {
            rejecting += 1;
        } else {
            accepting += 1;
        }
    }
    assert!(
        rejecting > 0 && accepting > 0,
        "harness: the spread must produce BOTH verdicts, else the identity is \
         vacuously true. rejecting={rejecting} accepting={accepting}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// M6 / REV-203-001 — "not a producer" is an ANSWER, and the gate evaluates it
// ═══════════════════════════════════════════════════════════════════════════

fn unregistered(pending_addbond: u32) -> HoldingsLookup {
    HoldingsLookup::Unregistered { pending_addbond }
}

/// REQ-BOND-006, REQ-BOND-002 — Decision: a failure means `Unregistered` is
/// still folded into the `Unavailable` fail-open arm, so the two live shapes of
/// INC-I-203 survive — a mid-epoch Register still in `pending_updates`, and any
/// funded non-producer key parking one 3001-bond AddBond that every leader
/// re-packs into a doomed block until `max_age`.
///
/// **COMPILE-RED today**: `HoldingsLookup::Unregistered` is a unit variant, and
/// the wrapper maps it to `Ok(())`.
///
/// `Unavailable` must keep failing open on the SAME inputs — that contrast is
/// what makes the change a narrowing of the fail-open arm rather than its
/// removal (REQ-BOND-006 is unchanged for the no-answer case).
#[test]
fn unregistered_producer_is_evaluated_not_failed_open() {
    let p = KeyPair::from_seed([0xA3; 32]);
    let pk = *p.public_key();

    // IP-ABSENT-OVERSIZE — the builder-DoS shape: 0 + 0 + 3001 > 3000.
    let oversize = add_bond(&pk, CAP + 1);
    let err = mempool::addbond_cap::addbond_cap_verdict(
        &oversize,
        &unregistered(0),
        0,
        HEIGHT,
        ACTIVE_AH,
    )
    .expect_err(
        "REV-203-001: the gate evaluates an ABSENT key as current=0 and rejects \
         0 + 0 + 3001 > 3000 (`validation_checks.rs:1197-1218`). The wrapper must \
         mirror it, or one ~150 KB transaction from any funded non-producer key \
         poisons every block that carries it, on every leader, until max_age.",
    );
    // Decision: without the grep code this refusal is invisible to the fleet's
    // log tooling and the DoS cannot be distinguished from ordinary rejections.
    assert!(
        err.contains("[ADDBOND_CAP_EXCEEDED]"),
        "O2: the refusal must carry the bracketed code. got: {err}"
    );

    // IP-ABSENT-PENDING — the incident's own shape: 0 + 2000 + 1500 > 3000.
    assert!(
        mempool::addbond_cap::addbond_cap_verdict(
            &add_bond(&pk, 1_500),
            &unregistered(2_000),
            0,
            HEIGHT,
            ACTIVE_AH
        )
        .is_err(),
        "REV-203-001: `pending_addbond_count(pk)` is counted by the gate for keys \
         it does not carry, so a Register mined mid-epoch with a queued AddBond of \
         2000 has only 1000 of headroom left, not 3000."
    );

    // IP-ABSENT-ROOM — an absent key with room must NOT be over-rejected.
    assert!(
        mempool::addbond_cap::addbond_cap_verdict(
            &add_bond(&pk, 2),
            &unregistered(0),
            0,
            HEIGHT,
            ACTIVE_AH
        )
        .is_ok(),
        "REQ-BOND-004 OVER-REJECTION: 0 + 0 + 2 <= 3000 and consensus ACCEPTS it. \
         Refusing every AddBond from an unregistered key would break the ordinary \
         Register-then-AddBond flow — the fix must EVALUATE, not blanket-reject."
    );

    // IP-BLIND-OVERSIZE — `Unavailable` is untouched (REQ-BOND-006).
    // Decision: if this flips, a write-contended live handle censors every AddBond
    // on the node, which is the failure direction `holdings.rs:9-11` forbids.
    assert!(
        mempool::addbond_cap::addbond_cap_verdict(
            &oversize,
            &HoldingsLookup::Unavailable,
            0,
            HEIGHT,
            ACTIVE_AH
        )
        .is_ok(),
        "REQ-BOND-006: `Unavailable` means NO SOURCE ANSWERED and must keep failing \
         open on the very same transaction. Narrowing the fail-open arm to \
         `Unavailable` is the change; removing it is not."
    );
}

/// REQ-BOND-004, REQ-BOND-006 — Decision: a failure means the absent-producer
/// arm can refuse a transaction the gate would have accepted in some block —
/// censorship no later layer can undo, and the one risk the M6 widening carries.
///
/// **COMPILE-RED today.** HARD CONSTRAINT 2: for every triple, admission
/// rejecting must IMPLY the gate rejecting, at `current = 0`.
#[test]
fn absent_producer_expression_rejects_a_strict_subset_of_consensus() {
    let p = KeyPair::from_seed([0xA4; 32]);
    let pk = *p.public_key();

    // (pending_addbond, requested, in_block_prior) — current is 0 by definition.
    let mut tuples: Vec<(u32, u32, u32)> = vec![
        (CAP - 1, 1, 0),
        (CAP - 1, 2, 0),
        (CAP - 2, 2, 0),
        (CAP - 2, 2, 1),
        (CAP, 1, 0),
        (0, 1, 0),
        (0, CAP + 1, 0),
        (2_000, 1_500, 0),
        (u32::MAX, 1, 1),
    ];
    let mut rng = Rng(0x0000_0203_0006_2026);
    while tuples.len() < 200 {
        let pending = rng.below(CAP + 8);
        let requested = 1 + rng.below(8);
        let prior = rng.below(16);
        tuples.push((pending, requested, prior));
    }
    let boundary_hits = tuples
        .iter()
        .filter(|(p, r, _)| {
            let s = p.saturating_add(*r);
            s == CAP - 1 || s == CAP || s == CAP + 1
        })
        .count();
    assert!(
        boundary_hits >= 3,
        "harness: the spread must straddle the cap boundary. hits={boundary_hits}"
    );

    let (mut rejecting, mut accepting) = (0usize, 0usize);
    for (pending, requested, prior) in tuples {
        let tx = add_bond(&pk, requested);
        // Admission drops the block-local term (`pool.rs` passes 0).
        let filter = mempool::addbond_cap::addbond_cap_verdict(
            &tx,
            &unregistered(pending),
            0,
            HEIGHT,
            ACTIVE_AH,
        );
        let gate = check_addbond_cap(
            0,
            pending.saturating_add(prior),
            requested,
            HEIGHT,
            ACTIVE_AH,
        );
        if filter.is_err() {
            rejecting += 1;
            assert!(
                gate.is_err(),
                "REQ-BOND-004 CENSORSHIP on the ABSENT path: the filter rejects \
                 (current=0, pending={pending}, requested={requested}) while the gate \
                 ACCEPTS it with in_block_prior={prior}."
            );
        } else {
            accepting += 1;
        }
        // The builder passes the term, so there the two must be IDENTICAL.
        let builder = mempool::addbond_cap::addbond_cap_verdict(
            &tx,
            &unregistered(pending),
            prior,
            HEIGHT,
            ACTIVE_AH,
        );
        assert_eq!(
            builder.is_err(),
            gate.is_err(),
            "REQ-BOND-004 PARITY BREAK on the ABSENT path at (pending={pending}, \
             requested={requested}, in_block_prior={prior}): the wrapper must CALL \
             `check_addbond_cap(0, pending + in_block_prior, requested)`, not restate it."
        );
    }
    assert!(
        rejecting > 0 && accepting > 0,
        "harness: the spread must produce BOTH verdicts, else the implication is \
         vacuously true. rejecting={rejecting} accepting={accepting}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// M6 / REV-203-011 — one copy of the count-Bond-outputs term
// ═══════════════════════════════════════════════════════════════════════════

/// REQ-BOND-004 — Decision: a failure means the `requested` term exists in two
/// hand-written copies (`addbond_cap.rs::requested_bonds` and the gate's inline
/// filter), which is the INC-I-180 `allowance_with` drift lesson repeated: the
/// two agree until one is edited, and nothing catches the divergence.
///
/// **COMPILE-RED today**: `doli_core::validation::count_bond_outputs` does not
/// exist. Consensus does not call it yet — wiring the gate is out of scope; this
/// milestone only removes the duplicate from the mempool side.
#[test]
fn count_bond_outputs_equals_the_gate_inline_expression() {
    let p = KeyPair::from_seed([0xA5; 32]);
    let pk = *p.public_key();
    let owner = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes());

    let mut none = add_bond(&pk, 1);
    none.outputs = vec![Output::normal(7, owner)];
    let mut mixed = add_bond(&pk, 3);
    mixed.outputs.push(Output::normal(7, owner));

    for (label, tx, expected) in [
        ("no Bond outputs", none, 0u32),
        ("one Bond output", add_bond(&pk, 1), 1),
        ("mixed Bond + Normal", mixed, 3),
        ("the over-cap shape", add_bond(&pk, CAP + 1), CAP + 1),
    ] {
        let inline = u32::try_from(
            tx.outputs
                .iter()
                .filter(|o| o.output_type == OutputType::Bond)
                .count(),
        )
        .unwrap_or(u32::MAX);
        assert_eq!(
            inline, expected,
            "harness: the fixture `{label}` must carry {expected} Bond outputs"
        );
        assert_eq!(
            count_bond_outputs(&tx),
            inline,
            "REV-203-011 on `{label}`: the shared helper must return exactly the \
             term `validation_checks.rs:1208-1211` computes inline. Two copies of \
             this expression is how INC-I-180's `allowance_with` drifted."
        );
    }
}
