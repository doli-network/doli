//! INC-I-180 M1 QA-1 — the two allowance bindings the first pass left open.
//! Requirements: REQ-I180-001 (Must), REQ-I180-003 (Must).
//!
//! covers: bins/node/src/node/validation_checks.rs (INC-I-180 gate),
//!         bins/node/src/node/apply_block/tx_processing.rs (Exit + withdrawal arms)
//!
//! ---------------------------------------------------------------------------
//! THE TWO DEFECTS THIS FILE REPRODUCES (QA run 525, PROBE A and PROBE B)
//! ---------------------------------------------------------------------------
//! ISSUE-001 — `TxType::Exit` bumps `withdrawal_pending_count += bond_count`
//! IMMEDIATELY (`tx_processing.rs`, Exit arm) but was invisible to the gate's
//! `match`, which handled only `AddBond` and `RequestWithdrawal`. `Exit` is
//! required to carry ZERO inputs and ZERO outputs, so it shares a block with a
//! withdrawal with no UTXO conflict at all. Measured: a post-AH block
//! `[Exit(p), RequestWithdrawal(p, 434)]` against `p` at 433 bonds + one
//! unflushed AddBond was ADMITTED; apply then queued only the Exit's own
//! withdrawal and SKIPPED the withdrawal tx's — 434 Bond UTXOs spent, producer
//! left `Active, weight=1`.
//!
//! ISSUE-002 — nothing bound `WithdrawalRequestData.bond_count` to the number
//! of Bond UTXOs the transaction actually destroys. The allowance bounds the
//! DECLARED count from above only, so under-declaring passed trivially while
//! `process_transaction_utxos` spent every input. Measured: `bond_count=1` with
//! 434 Bond inputs ⇒ 434 UTXOs destroyed, 1 bond removed, producer left
//! `Active, weight=433` — the mainnet n11 number, from ONE transaction.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT:
//! ---------------------------------------------------------------------------
//! Function under test:
//!   `Node::validate_block_economics(&Block, height, ValidationMode) -> Result<()>`
//! plus its apply-layer counterpart
//!   `Node::process_transaction_producer_effects(..) -> ()`  (unit return; every
//!   output is a parameter mutation).
//!
//!   O1: the block accept/reject verdict. `apply_block` calls the validator
//!       BEFORE `process_transaction_utxos`, so `Err` == "no Bond UTXO is spent".
//!   O2: post-flush `ProducerSet` state — `bond_count`, `status`, `weight`.
//!   O4: `withdrawal_pending_count` after apply, before the flush.
//!   O5: the queued `PendingProducerUpdate::RequestWithdrawal` count.
//!   NOT outputs: no persistent store is written by either function; no side
//!       channel; TERMINATION is not an output here.
//!
//! PATHS
//!   PX: height >= AH, gate's `Exit` arm charges `in_block_withdrawn`
//!   PY: height >= AH, gate's Bond-input count binding (type AND owner)
//!   PZ: height <  AH, both rules skipped — verdict bit-identical to ca0b3093
//!
//! INPUT PARTITIONS:
//!   IP-X1  `[Exit(p), RequestWithdrawal(p, 434)]`, p = 433 + pending 1 (PROBE A)
//!   IP-X2  `[Exit, Exit, RequestWithdrawal(20)]`, p = 10 + pending 25 — the
//!          DOUBLE-CHARGE discriminator: apply re-reads an UNCHANGED
//!          `bond_count` per Exit, so two Exits charge 2×10. A mirror that
//!          charged once would compute allowance 25 and ADMIT a request apply
//!          then skips.
//!   IP-X3  same ledger, `RequestWithdrawal(15)` — the ACCEPT boundary of the
//!          double charge (a triple charge would compute 5 and reject).
//!   IP-X4  `[Exit(p)]` alone — must stay ADMITTED (liveness).
//!   IP-Y1  declared 1, 434 Bond inputs (PROBE B, under-declared)
//!   IP-Y2  declared 430 of allowance 434, 400 Bond inputs (over-declared)
//!   IP-Y3  declared 434, 434 Bond inputs (exact — ACCEPT side)
//!   IP-Y4  declared 10 of allowance 11, 10 inputs resolving `Normal`
//!
//! IP-Y2 and IP-Y4 (and the IP-Y3 fixture-regression row) were re-expressed at
//! `declared < allowance` for run 525: DRAIN-EVERYTHING makes `declared ==
//! allowance` a FULL EXIT with its own rule, so their original constructions
//! became legal repairs. Their accept twins live in
//! `inc_i_180_drain_everything.rs`; the property each row was written for is
//! preserved under the PARTIAL shape. See
//! `docs/.workflow/inc-i-180-M1-drain-everything-design.md` §3.
//!   IP-W1  declared 100 for B, 100 Bond inputs owned by A (R2-B8)
//!   IP-W2  declared 100 for B, 100 Bond inputs owned by B (accept control)
//!   IP-W3  declared 100 for B, 60 owned by B + 40 owned by A
//!   IP-W4  IP-W1 below the gate
//!   IP-W5  declared 60 for B, 60 Bond inputs owned by B + 40 owned by A
//!          (AUDIT-P1-001: the count equality holds, exclusivity does not)
//!   IP-W6  IP-W5 below the gate
//!   IP-X5  `[RequestWithdrawal(p, 434), Exit(p)]` — IP-X1 reversed (R2-A2)
//!   IP-X6  `[AddBond(5), Exit, RequestWithdrawal(n)]`, n = 5 then 6 (R2-A5)
//!   IP-Z1  IP-X1 below the gate      IP-Z2  IP-Y1 below the gate
//!
//! MATRIX (every enumerated cell has an assertion)
//!   O1,O2,O5×PX×IP-X1 → req_i180_001_post_ah_exit_plus_withdrawal_is_rejected
//!   O1     ×PX×IP-X2 → req_i180_001_post_ah_two_exits_charge_the_allowance_twice
//!   O1,O4,O5×PX×IP-X3 → req_i180_001_post_ah_double_charge_accept_boundary
//!   O1     ×PX×IP-X4 → req_i180_001_post_ah_exit_only_block_stays_admitted
//!   O1,O2  ×PY×IP-Y1 → req_i180_001_post_ah_under_declared_bond_count_is_rejected
//!   O1     ×PY×IP-Y2 → req_i180_001_post_ah_over_declared_bond_count_is_rejected
//!   O1,O2,O5×PY×IP-Y3 → req_i180_001_post_ah_declared_count_matching_inputs_lands
//!   O1     ×PY×IP-Y4 → req_i180_001_post_ah_non_bond_inputs_are_not_bonds
//!   O1     ×PY×IP-W1 → req_i180_001_post_ah_cross_owner_bond_inputs_are_rejected
//!   O1     ×PY×IP-W2 → req_i180_001_post_ah_same_owner_bond_inputs_are_accepted
//!   O1     ×PY×IP-W3 → req_i180_001_post_ah_mixed_owner_bond_inputs_are_rejected
//!   O1     ×PZ×IP-W4 → req_i180_003_pre_ah_cross_owner_bond_inputs_keep_legacy
//!   O1     ×PY×IP-W5 → req_i180_001_post_ah_foreign_bond_riders_are_rejected
//!   O1     ×PZ×IP-W6 → req_i180_003_pre_ah_foreign_bond_riders_keep_legacy
//!   O1,O2,O5×PZ×IP-Z1 → req_i180_003_pre_ah_exit_plus_withdrawal_keeps_legacy
//!   O1,O2,O5×PZ×IP-Z2 → req_i180_003_pre_ah_under_declared_keeps_legacy
//!   O1,O2,O4,O5×PX×IP-X5 → req_i180_001_post_ah_withdrawal_then_exit_is_admitted_in_parity
//!   O1,O2,O5×PX×IP-X6 → req_i180_001_post_ah_in_block_addbond_extends_the_allowance

use crypto::KeyPair;
use doli_core::validation::ValidationMode;
use storage::ProducerStatus;

use crate::inc_i_180_common::{
    add_bond_tx, exit_tx, make_node, run_block_case, seed_bond_utxos, seed_bond_utxos_split,
    seed_normal_utxos, verdict_in_mode, withdrawal_tx, withdrawal_tx_with_inputs, N11_BONDS,
    POST_AH, PRE_AH,
};

// ═══════════════════════════════════════════════════════════════════════════
// ISSUE-001 — `Exit` consumes the allowance
// ═══════════════════════════════════════════════════════════════════════════

/// O1,O2,O5 × PX × IP-X1 — **RED before the QA-1 fix** (PROBE A replay).
#[tokio::test]
async fn req_i180_001_post_ah_exit_plus_withdrawal_is_rejected() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();
    let txs = vec![exit_tx(&pk), withdrawal_tx(&pk, N11_BONDS + 1, 0x21)];
    let o = run_block_case(&node, &kp, N11_BONDS, 1, txs, POST_AH).await;

    assert!(
        !o.validation_ok,
        "O1/PX: the Exit already charges the whole 433-bond holding to \
         withdrawal_pending_count at apply, so the allowance left for the \
         withdrawal is 1, not 434. A gate whose `match` ignores TxType::Exit \
         admits this block and apply then silently skips the withdrawal. \
         got {o:?}"
    );
    assert_eq!(
        o.queued_withdrawals, 1,
        "O5: the apply-side witness, unchanged by the fix — only the EXIT's own \
         RequestWithdrawal is queued; the withdrawal tx hits the shortfall \
         branch. This is exactly the (admitted, not-enqueued) pair the gate must \
         make unreachable"
    );
    assert_eq!(
        (o.bond_count, o.status, o.weight),
        (1, ProducerStatus::Active, 1),
        "O2: and the residual is the pending-AddBond count — unbacked, \
         reward-earning selection weight, the n11 shape"
    );
}

/// O1 × PX × IP-X2 — **RED before the QA-1 fix**, and the row that pins the
/// mirror to PARITY rather than to arithmetic elegance.
#[tokio::test]
async fn req_i180_001_post_ah_two_exits_charge_the_allowance_twice() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();
    let txs = vec![
        exit_tx(&pk),
        exit_tx(&pk),
        withdrawal_tx_with_inputs(&pk, 20, 20, 0x22),
    ];
    let o = run_block_case(&node, &kp, 10, 25, txs, POST_AH).await;

    assert!(
        !o.validation_ok,
        "O1/PX: apply's Exit arm re-reads an UNCHANGED bond_count and uses `+=`, \
         so two Exits charge 10 twice: remaining = 10 + 25 - 20 = 15 and a \
         20-bond request is SKIPPED. A mirror that charged 10 only once would \
         compute allowance 25, admit this block, and recreate the split inside \
         one block. Parity with apply is the requirement. got {o:?}"
    );
}

/// O1,O4,O5 × PX × IP-X3 — the ACCEPT side of the same double charge. Without
/// this row a mirror that OVER-charges (three times, or `bond_count` per Exit
/// plus the withdrawal) would pass the row above while denying a legal request.
#[tokio::test]
async fn req_i180_001_post_ah_double_charge_accept_boundary() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();
    let txs = vec![
        exit_tx(&pk),
        exit_tx(&pk),
        withdrawal_tx_with_inputs(&pk, 15, 15, 0x23),
    ];
    let o = run_block_case(&node, &kp, 10, 25, txs, POST_AH).await;

    assert!(
        o.validation_ok,
        "O1/PX: 15 == 10 + 25 - 20 is exactly what apply accepts, so the gate \
         must accept it too. got {o:?}"
    );
    assert_eq!(
        o.queued_withdrawals, 3,
        "O5: two from the Exits, one from the request — apply enqueued all three"
    );
    assert_eq!(
        o.withdrawal_pending, 35,
        "O4: 10 + 10 + 15 — the in-epoch guard charged for every queued unit"
    );
}

/// O1 × PX × IP-X4 — liveness. Charging the allowance must not make a plain
/// retirement `Exit` invalid.
#[tokio::test]
async fn req_i180_001_post_ah_exit_only_block_stays_admitted() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();
    let o = run_block_case(&node, &kp, N11_BONDS, 0, vec![exit_tx(&pk)], POST_AH).await;

    assert!(
        o.validation_ok,
        "O1/PX: an Exit carries no Bond inputs and has nothing to over-withdraw. \
         The gate charges the allowance for LATER txs; it must never reject the \
         Exit itself. got {o:?}"
    );
    assert_eq!(
        (o.bond_count, o.status, o.weight),
        (0, ProducerStatus::Exited, 0),
        "O2: and the retirement still lands at the epoch boundary"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// ISSUE-002 — the declared count is bound to the Bond UTXOs destroyed
// ═══════════════════════════════════════════════════════════════════════════

/// O1,O2 × PY × IP-Y1 — **RED before the QA-1 fix** (PROBE B replay).
#[tokio::test]
async fn req_i180_001_post_ah_under_declared_bond_count_is_rejected() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();
    let tx = withdrawal_tx_with_inputs(&pk, 1, N11_BONDS + 1, 0x24);
    let o = run_block_case(&node, &kp, N11_BONDS, 1, vec![tx], POST_AH).await;

    assert!(
        !o.validation_ok,
        "O1/PY: the allowance bounds the DECLARED count from above only, so \
         declaring 1 passes it trivially while process_transaction_utxos spends \
         all 434 Bond inputs. The UTXO effect and the producer-set effect must \
         be equal in MAGNITUDE, not merely both non-fatal. got {o:?}"
    );
    assert_eq!(
        (o.bond_count, o.status, o.weight),
        (N11_BONDS, ProducerStatus::Active, N11_BONDS as u64),
        "O2: the apply-side witness, unchanged by the fix — 1 bond removed \
         against 434 destroyed UTXOs leaves weight 433 behind nothing. This is \
         numerically the mainnet n11 incident from a single transaction"
    );
}

/// O1 × PY × IP-Y2 — the other side of the binding. Over-declaring is inside
/// the allowance yet destroys fewer bonds than it removes.
///
/// REWRITTEN for DRAIN-EVERYTHING (run 525). The original construction was
/// `declared = 434` against allowance `433 + 1 = 434`, which the user's
/// decision reclassifies as a FULL EXIT: the ledger lands on zero, auto-exit
/// fires, and no weight can survive unbacked, so it is now the n11 REPAIR and
/// is ACCEPTED (that accept lives in `inc_i_180_drain_everything.rs` as IP-D1).
/// The property this row exists for — over-declaring against too few destroyed
/// bonds — is preserved by moving it below the full-exit boundary: `430 < 434`
/// is a PARTIAL, where the strict equality still governs.
#[tokio::test]
async fn req_i180_001_post_ah_over_declared_bond_count_is_rejected() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();
    let tx = withdrawal_tx_with_inputs(&pk, 430, 400, 0x25);
    seed_bond_utxos(&node, &tx, &pk).await;
    let verdict = verdict_in_mode(
        &node,
        &pk,
        N11_BONDS,
        1,
        vec![tx],
        POST_AH,
        ValidationMode::Light,
    )
    .await;

    let err = verdict.expect_err(
        "O1/PY: 430 is WITHIN the allowance (433 + 1 = 434) and strictly below \
         it, so the holdings half admits it and the full-exit exemption does \
         not apply, but only 400 Bond UTXOs are destroyed. The producer would \
         lose 430 weight units against 400 spent bonds",
    );
    assert!(
        err.contains("[ECON_WITHDRAWAL_BOND_COUNT_MISMATCH]"),
        "O1/PY: a PARTIAL withdrawal keeps the strict declared == bond_inputs \
         rule verbatim. got: {err}"
    );
}

/// O1,O2,O5 × PY × IP-Y3 — the ACCEPT side. A binding that rejected here would
/// make every real retirement withdrawal invalid.
#[tokio::test]
async fn req_i180_001_post_ah_declared_count_matching_inputs_lands() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();
    let tx = withdrawal_tx_with_inputs(&pk, N11_BONDS + 1, N11_BONDS + 1, 0x26);
    let o = run_block_case(&node, &kp, N11_BONDS, 1, vec![tx], POST_AH).await;

    assert!(
        o.validation_ok,
        "O1/PY: declared == Bond inputs == allowance is the legal retirement \
         shape and must be ADMITTED. got {o:?}"
    );
    assert_eq!(o.queued_withdrawals, 1, "O5: and enqueued");
    assert_eq!(
        (o.bond_count, o.status, o.weight),
        (0, ProducerStatus::Exited, 0),
        "O2: FIFO flush lands the AddBond first, then drains all 434"
    );
}

/// O1 × PY × IP-Y4 — the binding counts BOND-typed inputs, not inputs. A
/// withdrawal padded with ordinary coins must not buy allowance with them.
///
/// REWRITTEN for DRAIN-EVERYTHING (run 525). The original was ledger 10,
/// allowance 10, declared 10 — a FULL EXIT under the user's decision, and with
/// zero Bond UTXOs owned it is exactly the n11 repair shape, now ACCEPTED. The
/// property is preserved by adding one pending AddBond: allowance becomes 11,
/// declared 10 is a PARTIAL, and Normal inputs still buy nothing.
#[tokio::test]
async fn req_i180_001_post_ah_non_bond_inputs_are_not_bonds() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();
    let tx = withdrawal_tx_with_inputs(&pk, 10, 10, 0x27);
    seed_normal_utxos(&node, &tx, &pk).await;
    let verdict =
        verdict_in_mode(&node, &pk, 10, 1, vec![tx], POST_AH, ValidationMode::Light).await;

    let err = verdict.expect_err(
        "O1/PY: ten Normal inputs destroy ZERO bonds, so a declared count of 10 \
         removes 10 weight units against nothing. Counting `tx.inputs.len()` \
         instead of Bond-typed inputs would admit this",
    );
    assert!(
        err.contains("[ECON_WITHDRAWAL_BOND_COUNT_MISMATCH]"),
        "O1/PY: 10 declared is strictly below the allowance of 11, so this is a \
         PARTIAL and the strict rule governs — 10 declared, 0 Bond inputs. A fix \
         that let the full-exit exemption reach a partial would admit it. \
         got: {err}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// ISSUE-006 — the counted Bond inputs must belong to the NAMED producer
// ═══════════════════════════════════════════════════════════════════════════
//
// QA round 2, R2-B8: a `RequestWithdrawal` signed by producer A, spending 100
// of A's OWN Bond UTXOs, naming producer B in `extra_data`, was ADMITTED. The
// Bond lock is bypassed for this tx type (`validation/utxo.rs:152-153`) and A
// signs its own inputs, so the spend is valid. The ledgers then move in
// opposite directions: A loses 100 Bond UTXOs, B loses 100 `bond_count`, and A
// keeps `selection_weight = 433` behind 333 bonds. The n11 outcome, from one
// transaction, at zero cost.
//
//   IP-W1  declare B, spend 100 Bond UTXOs owned by A            ⇒ REJECT
//   IP-W2  declare B, spend 100 Bond UTXOs owned by B            ⇒ ACCEPT
//   IP-W3  declare B, 60 owned by B + 40 owned by A              ⇒ REJECT
//   IP-W4  IP-W1 below the gate                                  ⇒ ACCEPT

const CROSS_OWNER_BONDS: u32 = 100;

/// O1 × PY × IP-W1 — **RED before the QA-2 fix** (R2-B8 replay).
#[tokio::test]
async fn req_i180_001_post_ah_cross_owner_bond_inputs_are_rejected() {
    let (node, kp_b, _t) = make_node().await;
    let pk_b = *kp_b.public_key();
    let pk_a = *KeyPair::generate().public_key();

    let tx = withdrawal_tx_with_inputs(&pk_b, CROSS_OWNER_BONDS, CROSS_OWNER_BONDS, 0x31);
    seed_bond_utxos(&node, &tx, &pk_a).await;
    let verdict = verdict_in_mode(
        &node,
        &pk_b,
        CROSS_OWNER_BONDS,
        0,
        vec![tx],
        POST_AH,
        ValidationMode::Light,
    )
    .await;

    let err = verdict.expect_err(
        "O1/PY: 100 is exactly B's allowance and exactly the input count, so an \
         owner-agnostic Bond filter admits this block. The UTXOs destroyed are \
         A's; the weight removed is B's. Equal magnitude on two DIFFERENT \
         ledgers is not conservation",
    );
    assert!(
        err.contains("[ECON_WITHDRAWAL_BOND_COUNT_MISMATCH]"),
        "O1/PY: foreign Bond UTXOs must not count toward the declared total, so \
         the binding rule is what fires — 100 declared against 0 of B's own \
         bonds. got: {err}"
    );
}

/// O1 × PY × IP-W2 — the ACCEPT control for the row above. Without it, a fix
/// that derived the owner with the wrong domain (plain `hash` instead of
/// `hash_with_domain(ADDRESS_DOMAIN, ..)`) would pass IP-W1 while rejecting
/// every honest withdrawal on the network.
#[tokio::test]
async fn req_i180_001_post_ah_same_owner_bond_inputs_are_accepted() {
    let (node, kp_b, _t) = make_node().await;
    let pk_b = *kp_b.public_key();

    let tx = withdrawal_tx_with_inputs(&pk_b, CROSS_OWNER_BONDS, CROSS_OWNER_BONDS, 0x32);
    seed_bond_utxos(&node, &tx, &pk_b).await;
    let verdict = verdict_in_mode(
        &node,
        &pk_b,
        CROSS_OWNER_BONDS,
        0,
        vec![tx],
        POST_AH,
        ValidationMode::Light,
    )
    .await;

    assert!(
        verdict.is_ok(),
        "O1/PY: the honest shape — the named producer spends its own bonds, at \
         `hash_with_domain(ADDRESS_DOMAIN, producer_pubkey)`, which is where \
         `Transaction::new_add_bond`, `new_registration` and genesis all put \
         them. got {verdict:?}"
    );
}

/// O1 × PY × IP-W3 — a partial substitution must not net out. Counting all Bond
/// inputs regardless of owner makes 60 of B's plus 40 of A's look like 100.
#[tokio::test]
async fn req_i180_001_post_ah_mixed_owner_bond_inputs_are_rejected() {
    let (node, kp_b, _t) = make_node().await;
    let pk_b = *kp_b.public_key();
    let pk_a = *KeyPair::generate().public_key();

    let tx = withdrawal_tx_with_inputs(&pk_b, CROSS_OWNER_BONDS, CROSS_OWNER_BONDS, 0x33);
    seed_bond_utxos_split(&node, &tx, &pk_b, 60, &pk_a).await;
    let verdict = verdict_in_mode(
        &node,
        &pk_b,
        CROSS_OWNER_BONDS,
        0,
        vec![tx],
        POST_AH,
        ValidationMode::Light,
    )
    .await;

    let err = verdict.expect_err(
        "O1/PY: 60 of B's bonds cannot back a 100-bond weight removal just \
         because 40 of A's bonds ride along in the same transaction",
    );
    assert!(
        err.contains("[ECON_WITHDRAWAL_BOND_COUNT_MISMATCH]"),
        "O1/PY: 100 declared, 60 of the named producer's bonds spent. got: {err}"
    );
}

/// O1 × PZ × IP-W4 — **GREEN today, must STAY green.** The owner binding is
/// under the SAME activation height; below it the historical verdict stands.
#[tokio::test]
async fn req_i180_003_pre_ah_cross_owner_bond_inputs_keep_legacy() {
    let (node, kp_b, _t) = make_node().await;
    let pk_b = *kp_b.public_key();
    let pk_a = *KeyPair::generate().public_key();

    let tx = withdrawal_tx_with_inputs(&pk_b, CROSS_OWNER_BONDS, CROSS_OWNER_BONDS, 0x34);
    seed_bond_utxos(&node, &tx, &pk_a).await;
    let verdict = verdict_in_mode(
        &node,
        &pk_b,
        CROSS_OWNER_BONDS,
        0,
        vec![tx],
        PRE_AH,
        ValidationMode::Light,
    )
    .await;

    assert!(
        verdict.is_ok(),
        "O1/PZ: below the gate no binding exists at all, so a historical block \
         of this shape must still replay as admitted. got {verdict:?}"
    );
}

/// Bonds of the named producer in the IP-W5 construction. The declared count
/// equals this number, so the count equality of R2 is satisfied outright.
const OWNED_BONDS: u32 = 60;

/// O1 × PY × IP-W5 — **RED before the AUDIT-P1-001 fix.** R2 as first shipped
/// compared the declared count against the named producer's OWN Bond inputs
/// only, so an actor holding two producer keys declared B's true count and let
/// 40 of A's Bond UTXOs ride along in the same transaction: `60 == 60` passed,
/// `spend_transaction_utxos` destroyed all 100, and only B's ledger moved. A's
/// 40 units of weight survive with nothing behind them — the n11 shape the gate
/// exists to close, at the same magnitude, one extra key.
#[tokio::test]
async fn req_i180_001_post_ah_foreign_bond_riders_are_rejected() {
    let (node, kp_b, _t) = make_node().await;
    let pk_b = *kp_b.public_key();
    let pk_a = *KeyPair::generate().public_key();

    let tx = withdrawal_tx_with_inputs(&pk_b, OWNED_BONDS, CROSS_OWNER_BONDS, 0x3A);
    seed_bond_utxos_split(&node, &tx, &pk_b, OWNED_BONDS as usize, &pk_a).await;
    let verdict = verdict_in_mode(
        &node,
        &pk_b,
        CROSS_OWNER_BONDS,
        0,
        vec![tx],
        POST_AH,
        ValidationMode::Light,
    )
    .await;

    let err = verdict.expect_err(
        "O1/PY: B declares its own 60 and spends its own 60, so the allowance \
         rule and the count equality both pass. The 40 Bond UTXOs of A in the \
         same transaction are destroyed with no ledger effect at all — a count \
         equality is not an exclusivity rule",
    );
    assert!(
        err.contains("[ECON_WITHDRAWAL_BOND_COUNT_MISMATCH]"),
        "O1/PY: every Bond-typed input must belong to the named producer — 100 \
         Bond inputs, 60 of them B's. got: {err}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// REQ-I180-003 — below the gate both rules are absent, bit-identically
// ═══════════════════════════════════════════════════════════════════════════

/// O1 × PZ × IP-W6 — **GREEN today, must STAY green.** The exclusivity half
/// ships under the SAME activation height as the count equality, so the twin of
/// IP-W5 below the gate keeps the historical verdict.
#[tokio::test]
async fn req_i180_003_pre_ah_foreign_bond_riders_keep_legacy() {
    let (node, kp_b, _t) = make_node().await;
    let pk_b = *kp_b.public_key();
    let pk_a = *KeyPair::generate().public_key();

    let tx = withdrawal_tx_with_inputs(&pk_b, OWNED_BONDS, CROSS_OWNER_BONDS, 0x3B);
    seed_bond_utxos_split(&node, &tx, &pk_b, OWNED_BONDS as usize, &pk_a).await;
    let verdict = verdict_in_mode(
        &node,
        &pk_b,
        CROSS_OWNER_BONDS,
        0,
        vec![tx],
        PRE_AH,
        ValidationMode::Light,
    )
    .await;

    assert!(
        verdict.is_ok(),
        "O1/PZ: pre-activation neither half of R2 runs, so a canonical block of \
         this shape must still replay as admitted. got {verdict:?}"
    );
}

/// O1,O2,O5 × PZ × IP-Z1 — **GREEN today, must STAY green.** Historical blocks
/// replay through this path; enforcing either rule below the gate forks the
/// chain at every height where the shortfall already happened.
#[tokio::test]
async fn req_i180_003_pre_ah_exit_plus_withdrawal_keeps_legacy() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();
    let txs = vec![exit_tx(&pk), withdrawal_tx(&pk, N11_BONDS + 1, 0x28)];
    let o = run_block_case(&node, &kp, N11_BONDS, 1, txs, PRE_AH).await;

    assert!(
        o.validation_ok,
        "O1/PZ: pre-activation the gate does not run — the block is admitted \
         exactly as before. got {o:?}"
    );
    assert_eq!(
        o.queued_withdrawals, 1,
        "O5/PZ: only the Exit's own withdrawal is queued — the historical shape"
    );
    assert_eq!(
        (o.bond_count, o.status, o.weight),
        (1, ProducerStatus::Active, 1),
        "O2/PZ: 434 bonds flushed, 433 withdrawn, 1 unbacked weight unit left. \
         This IS the bug, and below the gate it must remain reproducible"
    );
}

/// O1,O2,O5 × PZ × IP-Z2 — **GREEN today, must STAY green.**
#[tokio::test]
async fn req_i180_003_pre_ah_under_declared_keeps_legacy() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();
    let tx = withdrawal_tx_with_inputs(&pk, 1, N11_BONDS + 1, 0x29);
    let o = run_block_case(&node, &kp, N11_BONDS, 1, vec![tx], PRE_AH).await;

    assert!(
        o.validation_ok,
        "O1/PZ: pre-activation nothing binds the declared count to the inputs. \
         got {o:?}"
    );
    assert_eq!(
        o.queued_withdrawals, 1,
        "O5/PZ: the 1-bond request is queued"
    );
    assert_eq!(
        (o.bond_count, o.status, o.weight),
        (N11_BONDS, ProducerStatus::Active, N11_BONDS as u64),
        "O2/PZ: 433 weight units against 434 destroyed Bond UTXOs — preserved"
    );
}

/// O1 × PY × IP-Y3 — guard against a fixture regression: if `seed_bond_utxos`
/// ever stopped seeding, every post-AH accept row above would fail for the
/// wrong reason. Assert the seeding is what makes the accept row pass.
///
/// REWRITTEN for DRAIN-EVERYTHING (run 525). The original was ledger 10,
/// allowance 10, declared 10 — a FULL EXIT under the user's decision, so the
/// UNSEEDED half (zero Bond inputs, zero owned Bond UTXOs) becomes the n11
/// repair and is ACCEPTED, which would have made the two halves indistinguish-
/// able. One pending AddBond moves the pair to a PARTIAL (allowance 11,
/// declared 10) where the verdict still flips on the UTXO view alone.
#[tokio::test]
async fn req_i180_001_post_ah_unseeded_inputs_are_not_bonds() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();
    let tx = withdrawal_tx_with_inputs(&pk, 10, 10, 0x2A);

    let unseeded = verdict_in_mode(
        &node,
        &pk,
        10,
        1,
        vec![tx.clone()],
        POST_AH,
        ValidationMode::Light,
    )
    .await;
    let err =
        unseeded.expect_err("O1/PY: an input absent from the pre-block UTXO view is not a Bond");
    assert!(
        err.contains("[ECON_WITHDRAWAL_BOND_COUNT_MISMATCH]"),
        "O1/PY: 10 declared against 0 resolvable Bond inputs, strictly below \
         the allowance of 11. got: {err}"
    );

    seed_bond_utxos(&node, &tx, &pk).await;
    let seeded = verdict_in_mode(&node, &pk, 10, 1, vec![tx], POST_AH, ValidationMode::Light).await;
    assert!(
        seeded.is_ok(),
        "O1/PY: with the same ten inputs resolving to Bond outputs the block is \
         admitted — the verdict flips on the UTXO view alone, which is what the \
         accept rows in this file rest on. got {seeded:?}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// OBS-R2-001 — orderings QA probed by hand, now deliverable
// ═══════════════════════════════════════════════════════════════════════════

/// O1,O4,O5,O2 × PX × IP-X5 (R2-A2). The MIRROR of IP-X1: a withdrawal placed
/// BEFORE the Exit is admitted, because apply admits it too. The gate is
/// order-sensitive precisely because apply is, and the double charge
/// (`wp = 434 + 433`) is conservative — it can only deny later withdrawals,
/// never create weight. Left untested, a future "tidy up the arithmetic"
/// refactor would look harmless.
#[tokio::test]
async fn req_i180_001_post_ah_withdrawal_then_exit_is_admitted_in_parity() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();
    let txs = vec![withdrawal_tx(&pk, N11_BONDS + 1, 0x35), exit_tx(&pk)];
    let o = run_block_case(&node, &kp, N11_BONDS, 1, txs, POST_AH).await;

    assert!(
        o.validation_ok,
        "O1/PX: at the moment the withdrawal is evaluated nothing has charged \
         the allowance yet, and apply reaches the same state. got {o:?}"
    );
    assert_eq!(
        (o.queued_withdrawals, o.withdrawal_pending),
        (2, 867),
        "O5,O4: both are queued and the producer is charged 434 + 433 against \
         434 bonds — arithmetically over-charged, and that is the SAFE side"
    );
    assert_eq!(
        (o.bond_count, o.status, o.weight),
        (0, ProducerStatus::Exited, 0),
        "O2: the flush drains everything and leaves no unbacked weight, which is \
         why the over-charge is admissible"
    );
}

/// O1 × PX × IP-X6 (R2-A5). An AddBond earlier in the SAME block extends the
/// allowance by exactly its Bond-output count, matching apply's `in_flight`
/// term. Boundary probed on both sides: 5 lands, 6 is rejected.
#[tokio::test]
async fn req_i180_001_post_ah_in_block_addbond_extends_the_allowance() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();

    let accept = vec![
        add_bond_tx(&node, &pk, 5, 0x36),
        exit_tx(&pk),
        withdrawal_tx_with_inputs(&pk, 5, 5, 0x37),
    ];
    let o = run_block_case(&node, &kp, N11_BONDS, 0, accept, POST_AH).await;
    assert!(
        o.validation_ok,
        "O1/PX: allowance = 433 + 0 + 5 - 0 - 433 = 5, and apply's remaining is \
         the same 5. got {o:?}"
    );
    assert_eq!(
        o.queued_withdrawals, 2,
        "O5: the Exit's own withdrawal plus the request — apply enqueued both"
    );
    assert_eq!(
        (o.bond_count, o.status, o.weight),
        (0, ProducerStatus::Exited, 0),
        "O2: the AddBond flushes first, then the drain takes all 438"
    );

    let reject = vec![
        add_bond_tx(&node, &pk, 5, 0x38),
        exit_tx(&pk),
        withdrawal_tx_with_inputs(&pk, 6, 6, 0x39),
    ];
    let o6 = run_block_case(&node, &kp, N11_BONDS, 0, reject, POST_AH).await;
    assert!(
        !o6.validation_ok,
        "O1/PX: one past the boundary is a shortfall apply would silently skip. \
         A gate that ignored the in-block AddBond would reject BOTH cases and a \
         gate that over-counted it would admit both — this pair pins it. \
         got {o6:?}"
    );
}
