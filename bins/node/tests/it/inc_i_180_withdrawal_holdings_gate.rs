//! INC-I-180 M1 — the withdrawal-holdings gate.
//! Requirements: REQ-I180-001 (Must), REQ-I180-002 (Must), REQ-I180-003 (Must).
//! Brief: `docs/.workflow/inc-i-180-M1-brief.md` (F1..F7, binding).
//!
//! covers: tx_processing.rs, validation_checks.rs, set_core.rs, info.rs,
//!         network_params mod.rs, defaults.rs, env_loader.rs
//!
//! ---------------------------------------------------------------------------
//! THE DEFECT THIS FILE REPRODUCES
//! ---------------------------------------------------------------------------
//! `RequestWithdrawal` has two decision-independent ledger effects at apply time:
//!
//!   1. `process_transaction_utxos` spends EVERY Bond UTXO input, FIRST and
//!      unconditionally.
//!   2. `process_transaction_producer_effects` (tx_processing.rs, the
//!      `RequestWithdrawal` arm) returns `()`, runs AFTER that mutation, and
//!      queues the weight-removing `PendingProducerUpdate::RequestWithdrawal`
//!      ONLY if `data.bond_count <= bond_count - withdrawal_pending_count`.
//!
//! On shortfall the sole effect is one `warn!`. Bonds destroyed, weight kept.
//! The shortfall precondition is `U > P` — the UTXO ledger holds more Bond
//! outputs than the ProducerSet ledger counts — which is the NORMAL state
//! inside the epoch-deferred-flush window, and also the INC-I-085 orphan shape.
//! Mainnet n11 (`b03fe629…`): 434 unbacked selection-weight units, permanent.
//!
//! The rule "a producer has enough bonds to withdraw" is enforced at NO layer
//! today. This file states it at both layers and locks their agreement.
//!
//! ---------------------------------------------------------------------------
//! HOW THE GATE IS DRIVEN — AND WHY NOT BY FIELD NAME
//! ---------------------------------------------------------------------------
//! `NetworkParams::withdrawal_holdings_gate_activation_height` does not exist
//! yet. A file that named it would not COMPILE, and a compile error is a much
//! weaker RED than a failing assertion: it proves only that a symbol is absent,
//! never that the behaviour is wrong. So this file names no new symbol and
//! drives the gate the way the network drives it — by (network, height) — on
//! the DEVNET node `Node::new_for_test` builds.
//!
//!   [`PRE_AH`] = 5           — must land BELOW the devnet gate
//!   [`POST_AH`] = 1_000_007  — must land AT OR ABOVE it
//!
//! Binding assumption, stated not hidden: the devnet default must satisfy
//! `5 < AH <= 1_000_007`. The brief pins devnet to `20`. The EXACT value is
//! locked separately, by field name, in
//! `crates/core/tests/it/inc_i_180_activation_height.rs` — that file IS
//! compile-RED, on purpose, and is where REQ-I180-003 lives. The two files
//! together bind behaviour to a named, pinned height without letting a missing
//! symbol suppress the behavioural evidence.
//!
//! If the devnet gate were ever pinned to `0`, the four `PRE_AH` rows below
//! would fail — the assumption is self-policing, not a silent coupling.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT
//! ---------------------------------------------------------------------------
//! Functions under test:
//!   `Node::validate_block_economics(&Block, height, ValidationMode) -> Result<()>`
//!   `Node::process_transaction_producer_effects(&Transaction, height, slot,
//!        &UtxoSet, &mut ProducerSet, &mut HashSet<Hash>, &mut Vec<PublicKey>)
//!        -> ()`   (returns unit — EVERY output of it is a parameter mutation)
//!
//! OUTPUT CONTRACT: the block-level accept/reject verdict, the producer-set
//! mutations the withdrawal makes, and the agreement between the two.
//!   O1: `validate_block_economics` Result — Ok (block admitted) | Err (block
//!       rejected pre-mutation, so no Bond UTXO is ever spent).
//!   O2: `ProducerSet` entry AFTER the epoch-boundary flush — `bond_count`,
//!       `status` (Active | Exited) and `selection_weight()`. This is the
//!       quantity that stayed at 434 on mainnet n11.
//!   O3: Bond-UTXO spend-ness, observed through its ONLY gate. `apply_block`
//!       calls `validate_block_economics` (mod.rs:113) BEFORE
//!       `process_transaction_utxos`, so `O1 == Err` is exactly "the Bond UTXOs
//!       are never spent". Its post-admission half is the VALIDATION⇄APPLY
//!       PARITY: `O1 == Ok` must imply the withdrawal was ENQUEUED. Any
//!       (Ok, not-enqueued) pair is the defect — UTXOs spent, weight kept.
//!   O4: `withdrawal_pending_count` on the producer (the in-epoch double-spend
//!       guard; it is also the `- withdrawal_pending` term of the allowance).
//!   O5: the queued-update list itself (`pending_updates_for`) — the path
//!       witness that distinguishes "enqueued" from "coincidentally exited".
//!   NOT outputs: no persistent store is written by either function under test
//!       (both operate on in-memory handles), no side channel, no blocking
//!       syscall — TERMINATION is not an output here.
//!
//! PATHS
//!   PA: height <  AH, validation   → O1 == Ok ALWAYS (gate skipped entirely)
//!   PB: height <  AH, apply        → enqueue iff `n <= bond_count - pending_wd`
//!                                    (NO pending-AddBond term; the historical,
//!                                    buggy arithmetic, preserved for replay)
//!   PC: height >= AH, validation   → Err iff `n > allowance` (brief F2)
//!   PD: height >= AH, apply        → enqueue iff `n <= allowance` (brief F3;
//!                                    must accept EXACTLY what PC accepts)
//!   PE: epoch-boundary flush       → FIFO drain (brief F4), AddBond before
//!                                    RequestWithdrawal
//!   where `allowance = bond_count + pending_addbond + in_block_addbond
//!                      - withdrawal_pending - in_block_withdrawn`
//!
//! INPUT PARTITIONS: distinct arithmetic / reachability classes
//!   IP-A  n11 shape: bond_count 433, unflushed AddBond(1), withdraw 434.
//!         `n > bond_count` but `n == allowance` → must be ACCEPTED and land.
//!   IP-B  plain shortfall: bond_count 433, no pending, withdraw 434.
//!         `n > allowance` by exactly 1 → must be REJECTED.
//!   IP-C  exact fit: bond_count 433, no pending, withdraw 433 → ACCEPTED.
//!   IP-D  two withdrawals in ONE block, 433 then 1, bond_count 433. Each is
//!         individually within the allowance; jointly they exceed it. Isolates
//!         the `in_block_withdrawn` term, and is a SECOND live instance of the
//!         defect (today the 2nd is silently skipped after its UTXOs are spent).
//!   IP-E  unknown producer (not in the ProducerSet) → allowance 0.
//!   IP-F  degenerate `n == 0` → within any allowance, must not exit anyone.
//!   IP-G  `n == u32::MAX` → saturating arithmetic, reject, no panic.
//!   IP-H  IP-B at an EPOCH-BOUNDARY height with an INCOMPLETE block store,
//!         evaluated in Full, Light AND Replay. The epoch-reward section
//!         returns `Ok(())` early in Full mode when `calculate_epoch_rewards`
//!         errors (`validation_checks.rs`, `[INC_I_081_MISSING_CHECK_SKIP]`),
//!         which is the normal state of a freshly snap-synced node. All three
//!         modes must reach the SAME verdict — the INC-I-034 divergence class.
//!   IP-I  IP-C at that same position → ACCEPTED in all three modes (liveness:
//!         the hoist must not make legal withdrawals unminable at boundaries).
//!
//! MATRIX (every enumerated cell has an assertion)
//!   O1,O2,O4,O5×PC,PD,PE×IP-A → req_i180_002_post_ah_pending_addbond_makes_the_434th_withdrawable
//!   O2,O4,O5   ×PB,PE   ×IP-A → req_i180_003_pre_ah_n11_replay_preserves_the_silent_skip
//!   O1,O3      ×PC     ×IP-B → req_i180_001_post_ah_over_allowance_block_is_rejected
//!   O1,O2,O3,O5×PA,PB,PE×IP-B → req_i180_003_pre_ah_over_allowance_keeps_the_legacy_verdict
//!   O1,O2,O3,O5×PC,PD,PE×IP-C → req_i180_001_post_ah_exact_fit_is_accepted_and_lands
//!   O1,O3,O5   ×PC,PD  ×IP-D → req_i180_001_post_ah_two_withdrawals_in_one_block_are_summed
//!   O1,O3,O5   ×PA,PB  ×IP-D → req_i180_003_pre_ah_two_withdrawals_in_one_block_keep_legacy
//!   O1,O3,O5   ×PC,PD  ×IP-E → req_i180_001_post_ah_unknown_producer_is_rejected
//!   O1,O2,O5   ×PC,PD  ×IP-F → req_i180_001_post_ah_zero_bond_withdrawal_is_accepted
//!   O1,O3      ×PC     ×IP-G → req_i180_001_post_ah_u32_max_saturates_and_rejects
//!   O3         ×PC,PD  ×ALL  → req_i180_001_post_ah_validation_and_apply_never_disagree
//!   O1         ×PC     ×IP-H → req_i180_001_epoch_boundary_verdict_is_mode_independent
//!   O1         ×PC     ×IP-I → req_i180_001_epoch_boundary_legal_withdrawal_stays_admitted
//!   O1         ×PC     ×IP-H → req_i180_001_epoch_boundary_missing_epochreward_skip_intact
//!   (source)   ×n/a    ×n/a  → inc_i_080_addbond_cap_stays_below_the_epoch_reward_return

//! The harness lives in `inc_i_180_common` — every case seeds a real Bond UTXO
//! for each withdrawal input, because the gate resolves input types against the
//! pre-block `UtxoSet` and an unseeded fixture would make every withdrawal look
//! like it destroys zero bonds.

use std::collections::HashSet;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::validation::ValidationMode;
use storage::{ProducerSet, ProducerStatus, UtxoSet};

use crate::inc_i_180_common::{
    block_with, make_node, run_case, seed_bond_utxos, verdict_in_mode, withdrawal_tx,
    withdrawal_tx_with_inputs, EPOCH_BOUNDARY_POST_AH, N11_BONDS, POST_AH, PRE_AH, SLOT,
};

/// The three modes a block is validated in. Every consensus rule must reach the
/// same verdict in all three, or two nodes on the same chain disagree.
const ALL_MODES: [ValidationMode; 3] = [
    ValidationMode::Full,
    ValidationMode::Light,
    ValidationMode::Replay,
];

// ═══════════════════════════════════════════════════════════════════════════
// REQ-I180-002 / REQ-I180-001 — IP-A: the n11 replay
// ═══════════════════════════════════════════════════════════════════════════

/// O1,O2,O4,O5 × PC,PD,PE × IP-A — **RED today**.
///
/// The producer is flushed at 433 bonds and has an AddBond(1) queued but not
/// yet flushed, so the UTXO ledger holds 434 Bond outputs while the ProducerSet
/// counts 433. A full-value retirement withdrawal for 434 is exactly what a
/// retiring operator sends, and it is exactly what mainnet n11 sent.
///
/// Post-activation this must be ACCEPTED — `n == bond_count + pending_addbond`
/// — enqueued, and at the epoch boundary the AddBond flushes FIRST (brief F4)
/// so `apply_withdrawal` drains all 434 and auto-exits the producer.
///
/// Today `remaining` omits the pending term, `434 > 433`, and the enqueue is
/// SKIPPED: the producer stays Active at weight 434 with zero bonds behind it.
#[tokio::test]
async fn req_i180_002_post_ah_pending_addbond_makes_the_434th_withdrawable() {
    let (node, kp, _t) = make_node().await;
    let o = run_case(&node, &kp, N11_BONDS, 1, &[N11_BONDS + 1], POST_AH).await;

    assert!(
        o.validation_ok,
        "O1/PC: n == bond_count + pending_addbond is WITHIN the allowance — \
         a retirement withdrawal inside the deferred-flush window must not be \
         rejected. got {o:?}"
    );
    assert_eq!(
        o.queued_withdrawals, 1,
        "O5/PD: the deferred RequestWithdrawal MUST be queued. Zero here is the \
         defect: the Bond UTXOs are already spent by process_transaction_utxos \
         and nothing removes the weight. got {o:?}"
    );
    assert_eq!(
        o.withdrawal_pending,
        N11_BONDS + 1,
        "O4/PD: the in-epoch double-withdrawal guard must be charged"
    );
    assert_eq!(
        o.bond_count, 0,
        "O2/PE: FIFO flush — AddBond lands first (433→434), then the 434-bond \
         withdrawal drains it to 0"
    );
    assert_eq!(
        o.status,
        ProducerStatus::Exited,
        "O2/PE: auto-exit at bond_count == 0"
    );
    assert_eq!(
        o.weight, 0,
        "O2/PE: THE incident quantity. On mainnet n11 this stayed at 434 with \
         no bonds behind it — 434 unbacked selection-weight units, permanent"
    );
    assert!(o.parity_holds(1), "O3: validation⇄apply parity. got {o:?}");
}

/// O2,O4,O5 × PB,PE × IP-A — **GREEN today, and must STAY green.**
///
/// REQ-I180-003 / INV-CONSENSUS-002: below the activation height every
/// observable must be bit-identical to the pre-fix binary, including the wrong
/// ones. Historical blocks replay through this path; "fixing" them would fork
/// the chain at every height where the shortfall already happened.
#[tokio::test]
async fn req_i180_003_pre_ah_n11_replay_preserves_the_silent_skip() {
    let (node, kp, _t) = make_node().await;
    let o = run_case(&node, &kp, N11_BONDS, 1, &[N11_BONDS + 1], PRE_AH).await;

    assert!(
        o.validation_ok,
        "O1/PA: pre-activation the gate does not exist — the block is admitted. \
         A failure here means the devnet activation height was pinned at or \
         below {PRE_AH}, which breaks this file's stated band assumption"
    );
    assert_eq!(
        o.queued_withdrawals, 0,
        "O5/PB: the historical arithmetic is `434 > 433 - 0` → SKIP. Preserved \
         verbatim for replay safety"
    );
    assert_eq!(
        o.withdrawal_pending, 0,
        "O4/PB: the skip path never charges the pending counter"
    );
    assert_eq!(
        o.bond_count,
        N11_BONDS + 1,
        "O2/PE: only the AddBond flushes — 433 → 434"
    );
    assert_eq!(
        o.status,
        ProducerStatus::Active,
        "O2/PE: the producer stays Active — the historical, defective outcome"
    );
    assert_eq!(
        o.weight,
        (N11_BONDS + 1) as u64,
        "O2/PE: weight 434 with its Bond UTXOs already spent. This IS the bug, \
         and below the gate it must remain reproducible bit-for-bit"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// REQ-I180-001 — IP-B: plain over-allowance, rejected pre-mutation
// ═══════════════════════════════════════════════════════════════════════════

/// O1,O3 × PC × IP-B — **RED today**.
///
/// No pending AddBond, so 434 exceeds the allowance by exactly 1. The block
/// must be rejected by `validate_block_economics`, which `apply_block` calls
/// (mod.rs:113) BEFORE `process_transaction_utxos` — so the Bond UTXOs are
/// never spent. That pre-mutation position is the whole point of brief F1: the
/// producer-effects pass returns `()` and cannot fail, so it can never be the
/// enforcement site.
#[tokio::test]
async fn req_i180_001_post_ah_over_allowance_block_is_rejected() {
    let (node, kp, _t) = make_node().await;
    let o = run_case(&node, &kp, N11_BONDS, 0, &[N11_BONDS + 1], POST_AH).await;

    assert!(
        !o.validation_ok,
        "O1/PC: 434 > 433 + 0 - 0. The carrying block must be INVALID so no \
         Bond UTXO is ever spent (O3). Today validate_block_economics returns \
         Ok and the shortfall is discovered post-mutation. got {o:?}"
    );
    assert!(
        o.parity_holds(1),
        "O3: a rejected block has no apply obligations. got {o:?}"
    );
}

/// O1,O2,O3,O5 × PA,PB,PE × IP-B — **GREEN today, must STAY green.**
#[tokio::test]
async fn req_i180_003_pre_ah_over_allowance_keeps_the_legacy_verdict() {
    let (node, kp, _t) = make_node().await;
    let o = run_case(&node, &kp, N11_BONDS, 0, &[N11_BONDS + 1], PRE_AH).await;

    assert!(
        o.validation_ok,
        "O1/PA: pre-activation the block is admitted — bit-identical to ca0b3093"
    );
    assert_eq!(
        o.queued_withdrawals, 0,
        "O5/PB: and the withdrawal is silently skipped"
    );
    assert_eq!(o.bond_count, N11_BONDS, "O2/PE: ledger untouched");
    assert_eq!(
        o.status,
        ProducerStatus::Active,
        "O2/PE: producer stays Active"
    );
    assert_eq!(
        o.weight, N11_BONDS as u64,
        "O2/PE: the legacy (Ok, not-enqueued) pair — the defect, preserved"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// REQ-I180-001 — IP-C: exact fit, the accept boundary
// ═══════════════════════════════════════════════════════════════════════════

/// O1,O2,O3,O5 × PC,PD,PE × IP-C — the boundary that proves the gate is not a
/// blanket "reject all withdrawals". A producer withdrawing exactly what it
/// holds must still be able to retire. GREEN today for the apply half; the
/// value of the row is that it must STAY green after the fix.
#[tokio::test]
async fn req_i180_001_post_ah_exact_fit_is_accepted_and_lands() {
    let (node, kp, _t) = make_node().await;
    let o = run_case(&node, &kp, N11_BONDS, 0, &[N11_BONDS], POST_AH).await;

    assert!(
        o.validation_ok,
        "O1/PC: n == allowance is the ACCEPT side of the boundary. A gate that \
         rejects here would make full retirement impossible. got {o:?}"
    );
    assert_eq!(o.queued_withdrawals, 1, "O5/PD: enqueued");
    assert_eq!(o.bond_count, 0, "O2/PE: drained");
    assert_eq!(o.status, ProducerStatus::Exited, "O2/PE: auto-exit");
    assert_eq!(o.weight, 0, "O2/PE: no residual weight");
    assert!(o.parity_holds(1), "O3: parity. got {o:?}");
}

// ═══════════════════════════════════════════════════════════════════════════
// REQ-I180-001 — IP-D: two withdrawals in ONE block (the in-block term)
// ═══════════════════════════════════════════════════════════════════════════

/// O1,O3,O5 × PC,PD × IP-D — **RED today**, and a SECOND live instance of the
/// defect that needs no epoch-deferral window at all.
///
/// A block carries `RequestWithdrawal(433)` then `RequestWithdrawal(1)` for a
/// producer holding 433. Each passes the per-tx check in isolation. At apply,
/// the first charges `withdrawal_pending_count` to 433, so the second sees
/// `remaining == 0`, is skipped — and its Bond UTXO is already gone.
///
/// Validation must therefore carry an `in_block_withdrawn` tally (brief F2),
/// exactly as `check_addbond_cap` carries `in_block` for AddBonds, or the two
/// layers disagree and the bug is recreated inside a single block.
#[tokio::test]
async fn req_i180_001_post_ah_two_withdrawals_in_one_block_are_summed() {
    let (node, kp, _t) = make_node().await;
    let o = run_case(&node, &kp, N11_BONDS, 0, &[N11_BONDS, 1], POST_AH).await;

    assert!(
        !o.validation_ok,
        "O1/PC: 433 + 1 > 433. The in-block tally must sum withdrawals for the \
         same producer. Today each tx is judged alone and the block is admitted. \
         got {o:?}"
    );
    assert!(
        o.parity_holds(2),
        "O3: if validation admits this block, BOTH withdrawals must be queued. \
         The (Ok, 1-of-2-queued) pair is the defect. got {o:?}"
    );
}

/// O1,O3,O5 × PA,PB × IP-D — **GREEN today, must STAY green.**
#[tokio::test]
async fn req_i180_003_pre_ah_two_withdrawals_in_one_block_keep_legacy() {
    let (node, kp, _t) = make_node().await;
    let o = run_case(&node, &kp, N11_BONDS, 0, &[N11_BONDS, 1], PRE_AH).await;

    assert!(
        o.validation_ok,
        "O1/PA: pre-activation the block is admitted"
    );
    assert_eq!(
        o.queued_withdrawals, 1,
        "O5/PB: the first lands, the second is silently skipped — the exact \
         legacy shape, preserved for replay"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// REQ-I180-001 — IP-E/F/G: adversarial and degenerate inputs
// ═══════════════════════════════════════════════════════════════════════════

/// O1,O3,O5 × PC,PD × IP-E — **RED today**.
///
/// `producer_pubkey` is attacker-chosen bytes inside a mined transaction. A
/// name the ProducerSet has never seen has allowance 0, so any positive
/// withdrawal against it must invalidate the block. Today the apply pass logs
/// `"WithdrawalRequest: producer not found"` and returns — after the inputs
/// were spent. Nothing must panic on this path.
#[tokio::test]
async fn req_i180_001_post_ah_unknown_producer_is_rejected() {
    let (node, _kp, _t) = make_node().await;
    let ghost = KeyPair::generate();
    let pk = *ghost.public_key();

    {
        let mut guard = node.producer_set.write().await;
        *guard = ProducerSet::new(); // the ghost is in NO set
    }
    let tx = withdrawal_tx(&pk, 7, 0x33);
    seed_bond_utxos(&node, &tx, &pk).await;
    let block = block_with(&node, POST_AH, pk, vec![tx]);
    let verdict = node
        .validate_block_economics(&block, POST_AH, ValidationMode::Light)
        .await;

    assert!(
        verdict.is_err(),
        "O1/PC: allowance for an unknown producer is 0, so a 7-bond withdrawal \
         must invalidate the block. Otherwise 7 Bond UTXOs are spent with zero \
         producer-set effect — REQ-I180-001 verbatim. got {verdict:?}"
    );

    // O5/PD: and the apply pass must not panic on the same input.
    let mut applied = ProducerSet::new();
    let utxo = UtxoSet::new();
    let mut dirty: HashSet<Hash> = HashSet::new();
    let mut regs: Vec<PublicKey> = Vec::new();
    for tx in block.transactions.iter().skip(1) {
        node.process_transaction_producer_effects(
            tx,
            POST_AH,
            SLOT,
            &utxo,
            &mut applied,
            &mut dirty,
            &mut regs,
        );
    }
    assert_eq!(
        applied.pending_update_count(),
        0,
        "O5/PD: nothing is queued for a producer that does not exist"
    );
}

/// O1,O2,O5 × PC,PD × IP-F — degenerate `n == 0`. Within any allowance, so the
/// block is valid; and `apply_withdrawal(0)` must never trip the
/// `bond_count == 0` auto-exit on a producer that still holds bonds.
#[tokio::test]
async fn req_i180_001_post_ah_zero_bond_withdrawal_is_accepted() {
    let (node, kp, _t) = make_node().await;
    let o = run_case(&node, &kp, N11_BONDS, 0, &[0], POST_AH).await;

    assert!(
        o.validation_ok,
        "O1/PC: 0 <= allowance — a no-op withdrawal is not a consensus fault. \
         got {o:?}"
    );
    assert_eq!(
        o.bond_count, N11_BONDS,
        "O2/PE: a 0-bond withdrawal must not touch the ledger"
    );
    assert_eq!(
        o.status,
        ProducerStatus::Active,
        "O2/PE: and must NEVER auto-exit an active producer"
    );
}

/// O1,O3 × PC × IP-G — **RED today**. `bond_count` is a `u32` read straight
/// out of attacker-supplied `extra_data`. The allowance arithmetic must
/// saturate, reject, and not panic (debug builds panic on overflow).
#[tokio::test]
async fn req_i180_001_post_ah_u32_max_saturates_and_rejects() {
    let (node, kp, _t) = make_node().await;
    let o = run_case(&node, &kp, N11_BONDS, 1, &[u32::MAX], POST_AH).await;

    assert!(
        !o.validation_ok,
        "O1/PC: u32::MAX bonds must be rejected, with saturating arithmetic and \
         no panic on either side of the comparison. got {o:?}"
    );
    assert!(o.parity_holds(1), "O3: parity. got {o:?}");
}

// ═══════════════════════════════════════════════════════════════════════════
// REQ-I180-001 — O3: the parity invariant, swept
// ═══════════════════════════════════════════════════════════════════════════

/// O3 × PC,PD × every partition — **RED today**.
///
/// The single sentence of REQ-I180-001: *the UTXO effect and the producer-set
/// effect succeed or fail together.* `apply_block` spends the Bond inputs iff
/// `validate_block_economics` returned Ok, so post-activation
/// `validation_ok == every withdrawal in the block was enqueued` must hold for
/// EVERY input class at once. One sweep, so a fix that repairs one partition
/// while re-opening another cannot pass.
#[tokio::test]
async fn req_i180_001_post_ah_validation_and_apply_never_disagree() {
    let (node, kp, _t) = make_node().await;

    // (bond_count, pending_addbond, withdrawals, label)
    let cases: &[(u32, u32, &[u32], &str)] = &[
        (N11_BONDS, 1, &[N11_BONDS + 1], "IP-A n11 shape"),
        (N11_BONDS, 0, &[N11_BONDS + 1], "IP-B shortfall by 1"),
        (N11_BONDS, 0, &[N11_BONDS], "IP-C exact fit"),
        (N11_BONDS, 0, &[N11_BONDS, 1], "IP-D two in one block"),
        (N11_BONDS, 0, &[0], "IP-F zero bonds"),
        (N11_BONDS, 1, &[u32::MAX], "IP-G u32::MAX"),
        (1, 0, &[1], "minimal exact fit"),
        (1, 2, &[3], "pending dominates the allowance"),
        (1, 0, &[2], "minimal shortfall"),
    ];

    let mut disagreements: Vec<String> = Vec::new();
    for (bonds, pending, withdrawals, label) in cases {
        let o = run_case(&node, &kp, *bonds, *pending, withdrawals, POST_AH).await;
        if !o.parity_holds(withdrawals.len()) {
            disagreements.push(format!(
                "{label}: validation_ok={} but {}/{} withdrawals queued — Bond \
                 UTXOs spent with no producer-set effect",
                o.validation_ok,
                o.queued_withdrawals,
                withdrawals.len()
            ));
        }
    }

    assert!(
        disagreements.is_empty(),
        "O3: validation and apply must accept EXACTLY the same set post-AH \
         (brief F3). Disagreements:\n  {}",
        disagreements.join("\n  ")
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// ISSUE-005 — the verdict must not depend on ValidationMode
// ═══════════════════════════════════════════════════════════════════════════
//
// QA round 2, R2-E1: devnet, `blocks_per_epoch = 4`, h = 44, one
// `RequestWithdrawal` for 500 against 433 held, no EpochReward tx:
//   Full  = ADMITTED
//   Light = [ECON_WITHDRAWAL_OVER_HOLDINGS]
// The `return Ok(())` that skips the missing-EpochReward proof when the local
// store cannot prove one was required sat ABOVE the gate, so in Full mode the
// gate never ran. Full is the normal tip-following path and an incomplete
// store is the normal state of a freshly snap-synced node, so this is the
// reachable half of the INC-I-034 divergence class.

/// O1 × PC × IP-H — **RED before the QA-2 fix in Full mode** (R2-E1 replay).
#[tokio::test]
async fn req_i180_001_epoch_boundary_verdict_is_mode_independent() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();

    for mode in ALL_MODES {
        let tx = withdrawal_tx_with_inputs(&pk, 500, 500, 0x51);
        seed_bond_utxos(&node, &tx, &pk).await;
        let verdict = verdict_in_mode(
            &node,
            &pk,
            N11_BONDS,
            0,
            vec![tx],
            EPOCH_BOUNDARY_POST_AH,
            mode,
        )
        .await;

        let err = verdict.expect_err(
            "O1: an over-allowance withdrawal is rejected in Light and Replay \
             but was ADMITTED in Full, because the epoch-reward section returned \
             Ok(()) above the gate. Two nodes, same block, opposite verdicts",
        );
        assert!(
            err.contains("[ECON_WITHDRAWAL_OVER_HOLDINGS]"),
            "O1/{mode:?}: 500 bonds against an allowance of 433 is over-holdings \
             in every mode. got: {err}"
        );
    }
}

/// O1 × PC × IP-I — liveness at the same position. A hoist that rejected legal
/// withdrawals at epoch boundaries would stall every retirement that lands on
/// one.
#[tokio::test]
async fn req_i180_001_epoch_boundary_legal_withdrawal_stays_admitted() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();

    for mode in ALL_MODES {
        let tx = withdrawal_tx_with_inputs(&pk, N11_BONDS, N11_BONDS, 0x52);
        seed_bond_utxos(&node, &tx, &pk).await;
        let verdict = verdict_in_mode(
            &node,
            &pk,
            N11_BONDS,
            0,
            vec![tx],
            EPOCH_BOUNDARY_POST_AH,
            mode,
        )
        .await;

        assert!(
            verdict.is_ok(),
            "O1/{mode:?}: an exact-fit withdrawal is legal at an epoch boundary \
             too, and the missing-EpochReward check must still be skipped when \
             the store cannot prove a reward was owed. got {verdict:?}"
        );
    }
}

/// O1 × PC × IP-H — the SCOPING witness. The `[INC_I_081_MISSING_CHECK_SKIP]`
/// early return must still fire: in Full mode at an epoch boundary with an
/// incomplete store, a block carrying NO EpochReward is admitted. If a later
/// refactor converted that return into a flag so the rest of the function ran,
/// the INC-I-080 AddBond cap — LIVE on mainnet and testnet at height 0 — would
/// start executing where it never has, retroactively changing consensus.
#[tokio::test]
async fn req_i180_001_epoch_boundary_missing_epochreward_skip_intact() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();

    let verdict = verdict_in_mode(
        &node,
        &pk,
        N11_BONDS,
        0,
        vec![],
        EPOCH_BOUNDARY_POST_AH,
        ValidationMode::Full,
    )
    .await;

    assert!(
        verdict.is_ok(),
        "O1/Full: the local block store is empty, so calculate_epoch_rewards \
         returns IncompleteEpochStoreError and the missing-EpochReward check \
         cannot be proved. The block must be ADMITTED, exactly as before the \
         INC-I-180 gate was hoisted above this section. got {verdict:?}"
    );
}

/// The scoping constraint itself, asserted on the source. INC-I-080's cap is
/// live at height 0 on mainnet and testnet, so making it run in a case where it
/// currently does not would change the verdict of already-canonical blocks
/// (INV-CONSENSUS-001). Devnet pins that cap to `u64::MAX`, so no behavioural
/// fixture on `Node::new_for_test` can observe the cap at all — this positional
/// assertion is the only executable guard available, and it fails loudly if the
/// cap block is ever moved above the early return.
#[test]
fn inc_i_080_addbond_cap_stays_below_the_epoch_reward_return() {
    const SRC: &str = include_str!("../../src/node/validation_checks.rs");

    let gate_block = SRC
        .find("=== INC-I-180: withdrawal-holdings gate")
        .expect("the INC-I-180 gate block must still exist");
    let early_return = locate_missing_epochreward_return(SRC, gate_block);
    let cap_block = SRC
        .find("=== INC-I-080: per-producer AddBond cap")
        .expect("the INC-I-080 cap block must still exist");
    let epoch_section = SRC
        .find("=== EpochReward validation ===")
        .expect("the EpochReward section must still exist");

    assert!(
        cap_block > early_return,
        "INC-I-080's cap must stay BELOW the [INC_I_081_MISSING_CHECK_SKIP] \
         early return. It is enforced from height 0 on mainnet and testnet, so \
         a block that the early return currently spares would flip from \
         admitted to rejected — a retroactive consensus change on a live chain"
    );
    assert!(
        gate_block < epoch_section,
        "INC-I-180's gate must stay ABOVE the EpochReward section so the early \
         return cannot skip it. Its activation height is u64::MAX on mainnet \
         and a not-yet-reached height on testnet, so hoisting it changes no \
         already-canonical verdict"
    );
}

/// Finds the real `[INC_I_081_MISSING_CHECK_SKIP]` return site: the LAST
/// marker occurrence (the log call, not the INC-I-180 comment that names it),
/// anchored below `gate_block` and followed by `return Ok(());` nearby.
fn locate_missing_epochreward_return(src: &str, gate_block: usize) -> usize {
    let offset = src
        .rfind("[INC_I_081_MISSING_CHECK_SKIP]")
        .expect("the missing-EpochReward skip marker must still exist");
    assert!(
        offset > gate_block,
        "matched marker landed inside the INC-I-180 comment, not the real return site"
    );
    let window_end = (offset + 400).min(src.len());
    assert!(
        src[offset..window_end].contains("return Ok(());"),
        "matched marker is not followed by the real `return Ok(());` nearby"
    );
    offset
}

/// Negative control: proves `locate_missing_epochreward_return` plus the
/// cap-vs-return comparison actually fails when the cap precedes the return,
/// so the guard above is not vacuously true.
#[test]
fn inc_i_080_cap_before_return_fails_the_guard() {
    let gate_marker = "=== INC-I-180: withdrawal-holdings gate (height-gated) ===";
    let cap_marker = "=== INC-I-080: per-producer AddBond cap (height-gated) ===";
    let synthetic = format!(
        "{gate_marker}\n{cap_marker}\n\
         warn!(\"[INC_I_081_MISSING_CHECK_SKIP] cannot enforce\");\nreturn Ok(());\n"
    );

    let gate_block = synthetic.find(gate_marker).unwrap();
    let cap_block = synthetic.find(cap_marker).unwrap();
    let early_return = locate_missing_epochreward_return(&synthetic, gate_block);

    assert!(
        cap_block <= early_return,
        "inverted fixture must fail the same comparison the real guard relies on"
    );
}
