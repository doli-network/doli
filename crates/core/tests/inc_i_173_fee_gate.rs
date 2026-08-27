//! INC-I-173 M1 — CATEGORY B: the AH-gated fee/balance exemption inside
//! `validate_transaction_with_utxos` (spec F1 + F2 + F3).
//!
//! TDD RED. Does NOT compile against the current tree:
//! `ValidationContext::with_inc_i_173_activation_height` does not exist yet.
//!
//! Spec: `specs/state-only-fee-gate-architecture.md` F1/F2/F3, constraints
//!       C1, C2, C3, C4, C8.
//! Analysis: `docs/redesigns/state-only-fee-gate-redesign-analysis.md`.
//! Requirements: REQ-173-001, REQ-173-002, REQ-173-003, REQ-173-003b,
//!               REQ-173-004.
//!
//! ---------------------------------------------------------------------------
//! REQUIRED API
//! ---------------------------------------------------------------------------
//! ```ignore
//! // crates/core/src/validation/types.rs
//! pub struct ValidationContext { ... pub inc_i_173_activation_height: u64 }
//! // defaults to u64::MAX in ValidationContext::new  (FAIL-CLOSED)
//! impl ValidationContext {
//!     pub fn with_inc_i_173_activation_height(mut self, height: u64) -> Self { ... }
//! }
//!
//! // crates/core/src/validation/utxo.rs ~:222 — the gate. Twin idiom of :245.
//! let is_state_only_tx = if ctx.current_height >= ctx.inc_i_173_activation_height {
//!     tx.is_zero_flow()
//! } else {
//!     tx.inputs.is_empty() && tx.outputs.is_empty()
//!         && matches!(tx.tx_type,
//!             TxType::Registration | TxType::DelegateBond | TxType::RevokeDelegation)
//! };
//! ```
//! The `else` arm MUST stay CHARACTER-IDENTICAL to today's expression — it is
//! frozen consensus history (INV-COMPAT-001).
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT — `validate_transaction_with_utxos(tx, ctx, provider)`
//! ---------------------------------------------------------------------------
//! ENUMERATION OF OBSERVABLE OUTPUTS
//!   O1: the `Result<(), ValidationError>` DISCRIMINANT (accept vs reject).
//!       This is the consensus-visible output: a block containing a tx that
//!       returns `Err` is rejected whole at `tx_processing.rs:99`.
//!   O2: the `ValidationError` VARIANT on the reject path. Asserted wherever
//!       the variant distinguishes "rejected by the fee gate" from "rejected
//!       earlier, structurally" — without it a test can pass for the wrong
//!       reason (the INC-I-152 mutant-B2 failure mode).
//!   mutable params   : NONE — `tx` and `ctx` are shared refs, `provider` is `&U`.
//!   receiver mutation: NONE — free function.
//!   persistent store : NONE — the function performs no I/O. The `UtxoProvider`
//!                      is READ-only (`get_utxo(&self, ..)`).
//!   side channels    : `tracing` records only. DECLARED UNASSERTED — nothing is
//!                      logged that is not already in O1/O2.
//! CODE PATHS (of the INC-I-173 gate only; upstream structural validation is a
//! pre-filter, not a path of this change)
//!   PA: current_height >= inc_i_173_activation_height  -> `tx.is_zero_flow()`
//!   PB: current_height <  inc_i_173_activation_height  -> FROZEN 3-type matches!
//!   PC: predicate false -> balance check, then fee check -> InsufficientFunds
//!       or InsufficientFee
//!   PD: predicate true  -> both checks SKIPPED -> Ok (if structurally valid)
//! INPUT PARTITIONS
//!   IP-H1 height = AH - 1                      -> PB   (REQ-173-003, C8)
//!   IP-H2 height = AH exactly                  -> PA   (the gate is `>=`)
//!   IP-H3 height = AH + 300_000                -> PA
//!   IP-H4 height inside the genesis window     -> REQ-173-002 (C3); driven on
//!         BOTH branches by varying the AH, not the height, because the genesis
//!         `Registration` branch only exists while `is_in_genesis(height)`
//!   IP-T1 the 5 exempt types, 0-in/0-out
//!   IP-T2 Exit + SlashProducer, 0-in/0-out     -> C1, REJECT at every height
//!   IP-T3 ClaimReward/ClaimBond WITH a value output -> C2, REJECT at every height
//!   IP-T4 all 24 types, 0-in/0-out             -> REQ-173-003 total table
//! MATRIX: (O1,O2) x {PA,PB} x {IP-H1..IP-H4} x {IP-T1..IP-T4}.
//!   The REQ-173-003 total table alone is 24 cells at IP-H1; the named tests
//!   below cover the security-relevant cells at IP-H2/IP-H3 as well.
//!
//! ---------------------------------------------------------------------------
//! VALIDATION MODE (constraint C8) — STATED EXPLICITLY
//! ---------------------------------------------------------------------------
//! `validate_transaction_with_utxos` takes NO `ValidationMode`. This file calls
//! it DIRECTLY, so there is no wrapper that could swallow an error — strictly
//! stronger than `ValidationMode::Full`. The INC-I-064 tolerance that C8 warns
//! about lives one layer up, at `apply_block/tx_processing.rs:112`, and is keyed
//! EXCLUSIVELY on `ValidationMode::Replay`. That keying is asserted at the node
//! layer in `bins/node/tests/inc_i_173_state_only_fee_gate.rs`.
//!
//! ---------------------------------------------------------------------------
//! BASELINE PROVENANCE (REQ-173-003)
//! ---------------------------------------------------------------------------
//! Every expected verdict in `PRE_CHANGE_VERDICTS` below was MEASURED against
//! the pre-change tree on 2026-08-10 by running the current
//! `validate_transaction_with_utxos` over these exact fixtures at heights
//! 1 / 199_999 / 200_000 / 500_000 on `Network::Mainnet`. They are NOT inferred
//! from reading the code. Re-measure before changing any entry.

mod inc_i_173_common;

use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{Transaction, TxType};
use doli_core::validation::{validate_transaction_with_utxos, ValidationContext, ValidationError};
use doli_core::Network;
use inc_i_173_common::{
    genesis_registration_tx, tx_with_outputs, zero_flow_tx, EmptyUtxos, ABOVE_GATE, ALL_TX_TYPES,
    AT_GATE, BELOW_GATE, DEVNET_GENESIS_HEIGHT, EXPECTED_EXEMPT_SET, MAINNET_GENESIS_HEIGHT,
    TEST_AH,
};

// ---------------------------------------------------------------------------
// Context builders
// ---------------------------------------------------------------------------

/// A mainnet-shaped context at `height` with the INC-I-173 gate pinned at
/// `TEST_AH`.
fn ctx_at(height: u64) -> ValidationContext {
    ValidationContext::new(ConsensusParams::mainnet(), Network::Mainnet, 0, height)
        .with_inc_i_173_activation_height(TEST_AH)
}

/// A mainnet-shaped context at `height` with an EXPLICIT gate value. Used by
/// REQ-173-002, where the height must stay inside the genesis window while the
/// branch is varied.
fn ctx_at_with_gate(height: u64, gate: u64) -> ValidationContext {
    ValidationContext::new(ConsensusParams::mainnet(), Network::Mainnet, 0, height)
        .with_inc_i_173_activation_height(gate)
}

/// The real devnet shape: `inc_i_173_activation_height == 0`, so EVERY devnet
/// height — including genesis — is ABOVE the gate.
fn devnet_ctx_at(height: u64) -> ValidationContext {
    ValidationContext::new(ConsensusParams::devnet(), Network::Devnet, 0, height)
        .with_inc_i_173_activation_height(0)
}

fn verdict(tx: &Transaction, ctx: &ValidationContext) -> Result<(), ValidationError> {
    validate_transaction_with_utxos(tx, ctx, &EmptyUtxos)
}

fn is_insufficient_fee(r: &Result<(), ValidationError>) -> bool {
    matches!(r, Err(ValidationError::InsufficientFee { .. }))
}

fn is_insufficient_funds(r: &Result<(), ValidationError>) -> bool {
    matches!(r, Err(ValidationError::InsufficientFunds { .. }))
}

/// A plausible far-future height. Deliberately NOT `u64::MAX` — unrelated
/// height arithmetic elsewhere in the validator (era/epoch derivation, lock
/// windows) would then saturate and the test could pass for the wrong reason.
const FAR_FUTURE: u64 = 100_000_000;

// ===========================================================================
// REQ-173-001 (Must) — above the gate the exemption derives from is_zero_flow()
// Acceptance: AddMaintainer / RemoveMaintainer with 0-in/0-out and 0 fee PASS
//             at height >= AH, and FAIL with InsufficientFee below it.
// ===========================================================================

/// REQ-173-001 (Must) — IP-T1 x IP-H2/IP-H3, path PA -> PD.
///
/// This is the incident itself: `AddMaintainer` and `RemoveMaintainer` are
/// 0-in/0-out, are admitted to the mempool, are relayed, and have fully
/// implemented apply handlers — but the block builder skips them every slot
/// (`assembly.rs:235`) and every node rejects a block containing one
/// (`tx_processing.rs:99`) because the fee gate at `utxo.rs:222` carries a
/// narrower list than every other state-only definition in the tree.
#[test]
fn req_173_001_maintainer_txs_are_accepted_at_and_above_the_gate() {
    for t in [TxType::AddMaintainer, TxType::RemoveMaintainer] {
        let tx = zero_flow_tx(t);
        assert!(
            tx.inputs.is_empty() && tx.outputs.is_empty(),
            "fixture sanity"
        );

        // IP-H2: the gate is `>=`, so equality is ABOVE.
        let at = verdict(&tx, &ctx_at(AT_GATE));
        assert!(
            at.is_ok(),
            "O1 PA: {:?} must be ACCEPTED at height == AH ({}), got {:?}",
            t,
            AT_GATE,
            at
        );

        // IP-H3: far above.
        let above = verdict(&tx, &ctx_at(ABOVE_GATE));
        assert!(
            above.is_ok(),
            "O1 PA: {:?} must be ACCEPTED far above the gate ({}), got {:?}",
            t,
            ABOVE_GATE,
            above
        );
    }
}

/// REQ-173-001 (Must) — IP-T1 x IP-H1, path PB -> PC.
///
/// The BELOW-the-gate half. Asserting the specific `InsufficientFee` variant
/// (O2) — not merely `is_err()` — is what proves the transaction reached the
/// FEE GATE and was rejected there, rather than dying earlier in structural
/// validation for an unrelated reason.
#[test]
fn req_173_001_maintainer_txs_are_rejected_with_insufficient_fee_below_the_gate() {
    for t in [TxType::AddMaintainer, TxType::RemoveMaintainer] {
        let r = verdict(&zero_flow_tx(t), &ctx_at(BELOW_GATE));
        assert!(
            is_insufficient_fee(&r),
            "O2 PB: {:?} at AH-1 ({}) must be rejected by the FEE GATE with \
             InsufficientFee — this is the frozen legacy behavior that must not \
             change below the gate. Got {:?}",
            t,
            BELOW_GATE,
            r
        );
    }
}

/// REQ-173-001 (Must), constraint C4 — the gate reads the BLOCK's height from
/// `ValidationContext`, and nothing else.
///
/// Anti-vacuity: drive the SAME transaction across the boundary and assert the
/// verdict FLIPS at exactly `AH`. If the implementer wired the gate to a
/// constant, to `chain_state.best_height`, or to a call site, one of these
/// three assertions fails.
#[test]
fn req_173_001_the_verdict_flips_at_exactly_the_activation_height() {
    let tx = zero_flow_tx(TxType::AddMaintainer);
    assert!(
        verdict(&tx, &ctx_at(TEST_AH - 2)).is_err(),
        "C4: AH-2 is below the gate"
    );
    assert!(
        verdict(&tx, &ctx_at(TEST_AH - 1)).is_err(),
        "C4: AH-1 is below the gate"
    );
    assert!(
        verdict(&tx, &ctx_at(TEST_AH)).is_ok(),
        "C4: AH itself is ABOVE the gate — the comparison is `>=`, matching the \
         twin idiom at utxo.rs:245"
    );
    assert!(
        verdict(&tx, &ctx_at(TEST_AH + 1)).is_ok(),
        "C4: AH+1 is above the gate"
    );
}

/// REQ-173-001 (Must) — FAIL-CLOSED default.
///
/// A `ValidationContext` built WITHOUT `.with_inc_i_173_activation_height(..)`
/// keeps the field at `u64::MAX`, so it can never be above the gate. This is
/// what makes a forgotten call site a LIVENESS bug rather than a silent
/// consensus divergence.
#[test]
fn req_173_001_a_context_that_never_sets_the_height_stays_below_the_gate_forever() {
    let ctx = ValidationContext::new(ConsensusParams::mainnet(), Network::Mainnet, 0, ABOVE_GATE);
    assert_eq!(
        ctx.inc_i_173_activation_height,
        u64::MAX,
        "F2: the field must default to u64::MAX (fail-closed)"
    );
    let r = verdict(&zero_flow_tx(TxType::AddMaintainer), &ctx);
    assert!(
        is_insufficient_fee(&r),
        "O2: with the default u64::MAX height, even h={} stays BELOW the gate and \
         the legacy branch rejects. Got {:?}",
        ABOVE_GATE,
        r
    );
}

// ===========================================================================
// REQ-173-002 (Must), constraint C3 — the genesis Registration transaction
// Acceptance: the exact tx from assembly.rs:137-143 validates ABOVE AND BELOW.
// ===========================================================================

/// REQ-173-002 (Must), constraint C3 — IP-H4.
///
/// WHY THE GATE IS VARIED AND THE HEIGHT IS NOT. The genesis `Registration`
/// takes its 0-in/0-out branch only while `Network::is_in_genesis(height)`
/// holds (`validation/registration.rs:37`; mainnet `genesis_blocks = 360`).
/// Outside that window the same transaction is rejected with
/// `InvalidRegistration("registration must have inputs for bond")` — a
/// STRUCTURAL rejection that has nothing to do with the fee gate. To compare
/// the two fee-gate branches on the SAME transaction we therefore hold the
/// height inside the genesis window and move the ACTIVATION HEIGHT across it.
/// That is the exact discrimination C3 demands: both branches must exempt
/// `Registration`.
#[test]
fn req_173_002_genesis_registration_validates_below_the_gate() {
    let tx = genesis_registration_tx();
    assert_eq!(tx.tx_type, TxType::Registration);
    assert_eq!(tx.version, 1, "assembly.rs:138 — version 1");
    assert!(tx.inputs.is_empty(), "assembly.rs:140 — inputs: vec![]");
    assert!(tx.outputs.is_empty(), "assembly.rs:141 — outputs: vec![]");

    // gate far in the future -> PB (frozen legacy branch)
    let r = verdict(&tx, &ctx_at_with_gate(MAINNET_GENESIS_HEIGHT, u64::MAX));
    assert!(
        r.is_ok(),
        "C3 PB: the genesis Registration MUST validate below the gate — this is \
         live consensus history on every network. Got {:?}",
        r
    );
}

/// REQ-173-002 (Must), constraint C3 — the ABOVE-the-gate half.
#[test]
fn req_173_002_genesis_registration_validates_above_the_gate() {
    let tx = genesis_registration_tx();

    // gate at 0 -> every height is PA (the new is_zero_flow() branch)
    let r = verdict(&tx, &ctx_at_with_gate(MAINNET_GENESIS_HEIGHT, 0));
    assert!(
        r.is_ok(),
        "C3 PA: the genesis Registration MUST validate above the gate — if the new \
         predicate is not a strict superset of the frozen three, crossing the gate \
         breaks genesis and every fresh sync. Got {:?}",
        r
    );
}

/// REQ-173-002 (Must), constraint C3 — the REAL devnet shape.
///
/// Devnet pins `inc_i_173_activation_height = 0`, so a devnet node is ABOVE the
/// gate from block 1 and its genesis registrations take the NEW branch on the
/// very first block it ever produces. A wrong `Registration` arm would not be a
/// far-future mainnet risk — it would brick `scripts/launch_testnet.sh` on the
/// next run.
#[test]
fn req_173_002_genesis_registration_validates_on_devnet_where_the_gate_is_zero() {
    let tx = genesis_registration_tx();
    let ctx = devnet_ctx_at(DEVNET_GENESIS_HEIGHT);
    assert_eq!(ctx.inc_i_173_activation_height, 0);
    assert!(
        ctx.current_height >= ctx.inc_i_173_activation_height,
        "devnet is above the gate at every height"
    );
    let r = verdict(&tx, &ctx);
    assert!(
        r.is_ok(),
        "C3: devnet genesis Registration above the gate must validate. Got {:?}",
        r
    );
}

/// REQ-173-002 (Must) — the OTHER two frozen-legacy members, both branches.
///
/// `DelegateBond` and `RevokeDelegation` complete the frozen three. They must
/// be accepted on BOTH branches at BOTH heights.
#[test]
fn req_173_002_delegation_types_validate_on_both_branches() {
    for t in [TxType::DelegateBond, TxType::RevokeDelegation] {
        for h in [BELOW_GATE, AT_GATE, ABOVE_GATE] {
            let r = verdict(&zero_flow_tx(t), &ctx_at(h));
            assert!(
                r.is_ok(),
                "C3: {:?} is in the FROZEN legacy exempt set — it must be accepted \
                 at h={} on both branches. Got {:?}",
                t,
                h,
                r
            );
        }
    }
}

// ===========================================================================
// REQ-173-003 (Must, constraint C8) — BIT-IDENTITY BELOW THE GATE
// Acceptance: for each of the 24 types, the verdict at height = AH-1 equals the
//             pre-change verdict.
// ===========================================================================

/// Accept/reject expectation for a 0-in/0-out transaction of each type,
/// MEASURED against the pre-change tree (see BASELINE PROVENANCE above).
///
/// `true`  = `Ok(())`
/// `false` = `Err(..)`
///
/// The second element records WHY, so a future reader can tell a fee-gate
/// rejection from a structural one without re-running the probe.
const PRE_CHANGE_VERDICTS: [(TxType, bool, &str); 24] = [
    (TxType::Transfer, false, "structural: ERRTX001 needs inputs"),
    (
        TxType::Registration,
        false,
        "structural: outside genesis, registration must have inputs for bond",
    ),
    (TxType::Exit, false, "FEE GATE: InsufficientFee"),
    (
        TxType::ClaimReward,
        false,
        "structural: ERRTX002 needs outputs",
    ),
    (
        TxType::ClaimBond,
        false,
        "structural: ERRTX002 needs outputs",
    ),
    (TxType::SlashProducer, false, "FEE GATE: InsufficientFee"),
    (TxType::Coinbase, false, "structural: ERRTX001 needs inputs"),
    (
        TxType::AddBond,
        false,
        "structural: add bond must have inputs",
    ),
    (
        TxType::RequestWithdrawal,
        false,
        "structural: withdrawal must have Bond UTXO inputs",
    ),
    (
        TxType::ClaimWithdrawal,
        false,
        "structural: tombstoned discriminant 9, never supported",
    ),
    (
        TxType::EpochReward,
        false,
        "structural: epoch reward needs at least one output",
    ),
    (TxType::RemoveMaintainer, false, "FEE GATE: InsufficientFee"),
    (TxType::AddMaintainer, false, "FEE GATE: InsufficientFee"),
    (
        TxType::DelegateBond,
        true,
        "FEE GATE: exempt (frozen three)",
    ),
    (
        TxType::RevokeDelegation,
        true,
        "FEE GATE: exempt (frozen three)",
    ),
    (
        TxType::ProtocolActivation,
        false,
        "FEE GATE: InsufficientFee (Option A NOT taken in M1)",
    ),
    (
        TxType::PriceAttestation,
        false,
        "structural: ERRTX-ORACLE001, oracle_activation_height = u64::MAX",
    ),
    (
        TxType::MintAsset,
        false,
        "structural: MintAsset requires at least one input",
    ),
    (
        TxType::BurnAsset,
        false,
        "structural: BurnAsset requires at least one input",
    ),
    (
        TxType::CreatePool,
        false,
        "structural: ERRTX001 needs inputs",
    ),
    (
        TxType::AddLiquidity,
        false,
        "structural: ERRTX001 needs inputs",
    ),
    (
        TxType::RemoveLiquidity,
        false,
        "structural: ERRTX001 needs inputs",
    ),
    (TxType::Swap, false, "structural: ERRTX001 needs inputs"),
    (TxType::ZKSettle, false, "structural: ERRTX001 needs inputs"),
];

/// REQ-173-003 (Must), constraint C8 — IP-H1 x IP-T4, path PB. 24 cells.
///
/// VALIDATION MODE: this calls `validate_transaction_with_utxos` DIRECTLY.
/// There is no `ValidationMode` parameter on that function and therefore no
/// wrapper that could swallow an error — strictly stronger than
/// `ValidationMode::Full`. See the VALIDATION MODE section in the module header.
///
/// Below the gate the behavior must be BIT-IDENTICAL to the pre-change binary,
/// because ~30 external auto-update producers cannot be stopped for a
/// synchronized restart and will run mixed versions across the whole
/// below-the-gate range.
#[test]
fn req_173_003_all_24_types_keep_their_pre_change_verdict_below_the_gate() {
    assert_eq!(
        PRE_CHANGE_VERDICTS.len(),
        ALL_TX_TYPES.len(),
        "the table must cover every live TxType"
    );

    for (t, expected_ok, why) in PRE_CHANGE_VERDICTS {
        let r = verdict(&zero_flow_tx(t), &ctx_at(BELOW_GATE));
        assert_eq!(
            r.is_ok(),
            expected_ok,
            "REQ-173-003 / C8: {:?} at AH-1 ({}) must keep its pre-change verdict \
             (expected ok={}, reason: {}). Got {:?}",
            t,
            BELOW_GATE,
            expected_ok,
            why,
            r
        );
    }
}

/// REQ-173-003 (Must) — the O2 half of bit-identity for the types that actually
/// REACH the fee gate.
///
/// `is_ok()` parity alone is not bit-identity: an implementer could preserve the
/// accept/reject split while changing WHICH error fires, and a peer running the
/// old binary would then disagree about the error surfaced to RPC. For the five
/// types that reach the gate with a 0-in/0-out shape, pin the VARIANT too.
#[test]
fn req_173_003_fee_gate_rejections_keep_their_error_variant_below_the_gate() {
    for t in [
        TxType::Exit,
        TxType::SlashProducer,
        TxType::AddMaintainer,
        TxType::RemoveMaintainer,
        TxType::ProtocolActivation,
    ] {
        let r = verdict(&zero_flow_tx(t), &ctx_at(BELOW_GATE));
        assert!(
            is_insufficient_fee(&r),
            "REQ-173-003 / C8: {:?} must still fail with InsufficientFee (not some \
             other variant) at AH-1. Got {:?}",
            t,
            r
        );
    }
}

/// REQ-173-003 (Must) — the ONLY verdicts allowed to change above the gate are
/// `AddMaintainer` and `RemoveMaintainer`.
///
/// This is the blast-radius bound of the whole change, expressed as a test: run
/// the same 24-type sweep at `AH` and at `AH + 300_000` and assert every cell is
/// unchanged from the below-the-gate table EXCEPT the two maintainer types.
#[test]
fn req_173_003_only_the_two_maintainer_types_change_verdict_above_the_gate() {
    for h in [AT_GATE, ABOVE_GATE] {
        for (t, below_ok, why) in PRE_CHANGE_VERDICTS {
            let r = verdict(&zero_flow_tx(t), &ctx_at(h));
            let expected_ok =
                matches!(t, TxType::AddMaintainer | TxType::RemoveMaintainer) || below_ok;
            assert_eq!(
                r.is_ok(),
                expected_ok,
                "REQ-173-003: above the gate (h={}) only AddMaintainer and \
                 RemoveMaintainer may flip. {:?} (below reason: {}) got {:?}",
                h,
                t,
                why,
                r
            );
        }
    }
}

// ===========================================================================
// REQ-173-003b (Must, spec F3 negative), constraint C1
// Acceptance: Exit and SlashProducer are REJECTED at EVERY height.
// ===========================================================================

/// REQ-173-003b (Must), constraint C1 — IP-T2 across every height partition.
///
/// `Exit` and `SlashProducer` are 0-in/0-out, sit in L1 ∩ L2, and are members
/// of `is_state_only()` — so both shape-derived candidate predicates would have
/// exempted them. They must stay REJECTED below the gate, AT the gate, and far
/// above it, because their apply handlers accept an actor identity without
/// verifying a signature (Exit: `ExitData` has no signature at all,
/// `validation/tx_types.rs:11-42` does no crypto; SlashProducer:
/// `reporter_signature` has zero verification readers and the VDF evidence is
/// publicly computable). Each is routed to its own incident.
///
/// Asserting `InsufficientFee` specifically (O2) proves they reached and were
/// stopped BY THE FEE GATE — the last gate before a free, keyless forced-exit
/// and a free, keyless bond burn.
#[test]
fn req_173_003b_exit_is_rejected_at_every_height() {
    for h in [1, BELOW_GATE, AT_GATE, ABOVE_GATE, FAR_FUTURE] {
        let r = verdict(&zero_flow_tx(TxType::Exit), &ctx_at(h));
        assert!(
            is_insufficient_fee(&r),
            "C1/FM-3: Exit must be rejected by the fee gate at EVERY height (h={}). \
             Got {:?}",
            h,
            r
        );
    }
}

/// REQ-173-003b (Must), constraint C1 + C10.
#[test]
fn req_173_003b_slash_producer_is_rejected_at_every_height() {
    for h in [1, BELOW_GATE, AT_GATE, ABOVE_GATE, FAR_FUTURE] {
        let r = verdict(&zero_flow_tx(TxType::SlashProducer), &ctx_at(h));
        assert!(
            is_insufficient_fee(&r),
            "C1/FM-1: SlashProducer must be rejected by the fee gate at EVERY height \
             (h={}). Got {:?}",
            h,
            r
        );
    }
}

/// REQ-173-003b (Must) — anti-vacuity for the two negatives above.
///
/// At the SAME heights and through the SAME code path, the exempt types must be
/// ACCEPTED above the gate. Without this, both negatives pass against a broken
/// implementation that rejects everything.
#[test]
fn req_173_003b_negatives_are_not_vacuous_the_exempt_types_pass_at_the_same_heights() {
    for h in [AT_GATE, ABOVE_GATE] {
        for t in EXPECTED_EXEMPT_SET {
            // Registration is excluded here: outside the genesis window it is
            // rejected STRUCTURALLY, which is covered by REQ-173-002 instead.
            if t == TxType::Registration {
                continue;
            }
            let r = verdict(&zero_flow_tx(t), &ctx_at(h));
            assert!(
                r.is_ok(),
                "anti-vacuity: {:?} must be ACCEPTED at h={} — otherwise the Exit / \
                 SlashProducer negatives prove nothing. Got {:?}",
                t,
                h,
                r
            );
        }
    }
}

// ===========================================================================
// REQ-173-004 (Must), constraint C2 — THE MINT GUARD
// Acceptance: ClaimReward/ClaimBond with a non-zero output are REJECTED at
//             every height.
// ===========================================================================

/// REQ-173-004 (Must), constraint C2 — IP-T3 across every height partition.
///
/// `inputs.is_empty() && outputs.is_empty()` must remain a CONJUNCT. If the
/// predicate ever became type-only, a `ClaimReward` with 0 inputs and a
/// 1 000 000 DOLI output would skip BOTH the balance check and the fee check
/// and mint coins from nothing.
///
/// The expected error is `InsufficientFunds` (inputs 0 < outputs), which is the
/// balance check — proving the tx reached the non-exempt branch rather than
/// being skipped.
#[test]
fn req_173_004_claim_reward_with_a_large_output_is_rejected_at_every_height() {
    // 1 000 000 DOLI at 1e8 units/DOLI.
    let tx = tx_with_outputs(TxType::ClaimReward, &[100_000_000_000_000]);
    for h in [1, BELOW_GATE, AT_GATE, ABOVE_GATE, FAR_FUTURE] {
        let r = verdict(&tx, &ctx_at(h));
        assert!(
            is_insufficient_funds(&r),
            "C2 MINT GUARD: a 0-input ClaimReward minting 1 000 000 DOLI must be \
             rejected by the BALANCE check at h={}. Got {:?}",
            h,
            r
        );
    }
}

/// REQ-173-004 (Must), constraint C2 — the `ClaimBond` twin.
#[test]
fn req_173_004_claim_bond_with_a_non_zero_output_is_rejected_at_every_height() {
    let tx = tx_with_outputs(TxType::ClaimBond, &[1_000_000_000]);
    for h in [1, BELOW_GATE, AT_GATE, ABOVE_GATE, FAR_FUTURE] {
        let r = verdict(&tx, &ctx_at(h));
        assert!(
            is_insufficient_funds(&r),
            "C2 MINT GUARD: a 0-input ClaimBond returning 10 DOLI must be rejected \
             by the BALANCE check at h={}. Got {:?}",
            h,
            r
        );
    }
}

/// REQ-173-004 (Must), constraint C2 — the TOTAL property at the VALIDATOR
/// level: no type, exempt or not, gets fee/balance exemption once it carries
/// an output. Applied to the five EXEMPT types, where the risk is greatest.
#[test]
fn req_173_004_no_exempt_type_escapes_the_balance_check_once_it_has_an_output() {
    for t in EXPECTED_EXEMPT_SET {
        let tx = tx_with_outputs(t, &[100_000_000_000_000]);
        for h in [BELOW_GATE, AT_GATE, ABOVE_GATE] {
            let r = verdict(&tx, &ctx_at(h));
            assert!(
                r.is_err(),
                "C2 MINT GUARD: exempt type {:?} carrying a 1 000 000 DOLI output must \
                 still be REJECTED at h={} — exemption is a property of (type AND \
                 shape). Got {:?}",
                t,
                h,
                r
            );
        }
    }
}

/// REQ-173-004 (Must) — a zero-AMOUNT output is still a NON-EMPTY output
/// vector. The conjunct reads `outputs.is_empty()`, not `total_output() == 0`.
/// An implementer who "optimised" it to a value comparison would open the mint
/// path for any tx that adds one dust output.
#[test]
fn req_173_004_a_zero_amount_output_still_disqualifies_the_exemption() {
    for t in EXPECTED_EXEMPT_SET {
        let tx = tx_with_outputs(t, &[0]);
        let r = verdict(&tx, &ctx_at(ABOVE_GATE));
        assert!(
            r.is_err(),
            "C2: {:?} with ONE zero-amount output is not 0-out and must not be \
             exempt at h={}. Got {:?}",
            t,
            ABOVE_GATE,
            r
        );
    }
}
