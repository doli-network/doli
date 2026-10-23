//! INC-I-182 M2 — a producer listed with bonds but owning no live Bond UTXO
//! exits with its own key: zero Bond inputs, fee and payout from its own
//! Normal coin, one Unbonding payout. Accepted on mempool, builder and apply;
//! the same tx unsigned or signed by another key is refused with the rule's code.
//!
// covers: exit unbonding_tx producer_tx producer_ledger withdrawal_economics withdrawal_holdings tx_processing pool assembly validation_checks producer_auth unbonding transaction
//
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64, u32) -> Result<AddTransactionResult, MempoolError>
//   O1 return (Ok | Err text)
// OUTPUT CONTRACT: fn Node::build_block_content(&mut self, ..) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>
//   O2 transactions selected; O3 the builder's log lines naming the tx
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O4 return; O5 P's (bond_count, withdrawal_pending_count, status); O6 P's queued updates
// PATHS: child process, producer-auth gate 2, holdings gate 0, genesis blocks 0, Light apply
//   P-MEMPOOL admission at tip 2 | P-BUILDER admitted at tip 1 judged at 1, built at 2
//   | P-APPLY block at 2 (epoch 0) | P-FLUSH block at 6, epoch boundary at 8
// MATRIX
//   O1, O2, O4, O5, O6 x {P-MEMPOOL, P-BUILDER, P-APPLY} x Signed -> ..._self_exit_zero_bond_accepted_on_three_paths
//   O4, O5, O6 x P-FLUSH x Signed -> ..._self_exit_zero_bond_exits_at_epoch_flush
//   O1, O2, O3, O4, O5 x {P-MEMPOOL, P-BUILDER, P-APPLY} x {Unsigned, WrongKey} -> ..._zero_bond_exit_unsigned_or_wrong_key_refused
//   {P-MEMPOOL, P-BUILDER, P-APPLY} x Signed -> count 3 (inc_i_182_m2_outcome_probe, PAUTH_SELF_EXIT_NODE_PATHS)
// INPUT PARTITIONS:
//   P ledger: 3 bonds, 0 Bond UTXOs at its own address; declared: full allowance (3)
//   inputs: P's one Normal coin (777_000), P signs it; payout: Unbonding 776_999 to P, lock 10_000
//   auth tail: P's key | none | another producer's key

use storage::ProducerStatus;

use super::inc_i_182_m1_common::Verdict;
use super::inc_i_182_m2_node_fx::*;
use super::inc_i_255_m4_common::{admit_at, resident};

const MOD: &str = "inc_i_182_m2_self_exit::";

async fn mempool(form: Form) -> Verdict {
    let mut s = sx(0).await;
    s.r.advance_to(GATE).await;
    s.admit(&s.self_exit(form)).await
}

async fn builder(form: Form) -> Verdict {
    let mut s = sx(0).await;
    s.r.advance_to(GATE - 1).await;
    let tx = s.self_exit(form);
    let admission = admit_at(&s.r.node, &tx, GATE - 2).await;
    assert!(
        resident(&s.r.node, &tx).await,
        "PRECONDITION: {form:?} self-exit resident when judged below the gate; got {admission:?}"
    );
    s.build_next(&tx).await
}

/// Apply at `GATE` (epoch 0); accepted only when P has left the set.
async fn apply(form: Form) -> Verdict {
    let mut s = sx(0).await;
    s.r.advance_to(GATE - 1).await;
    let mut v = s.apply_with(&s.self_exit(form)).await;
    let exited = matches!(s.ledger().await, Some((0, 0, ProducerStatus::Exited)));
    v.accepted = v.accepted && exited && s.queued().await == 0;
    v
}

// REQ-PAUTH-004 — Decision: a failure means a producer with no live Bond UTXO cannot leave the set with its own key on some path, so the new rule left it stuck.
#[tokio::test]
async fn inc_i_182_m2_req_pauth_004_self_exit_zero_bond_accepted_on_three_paths() {
    if !in_child() {
        return run_child(&format!(
            "{MOD}inc_i_182_m2_req_pauth_004_self_exit_zero_bond_accepted_on_three_paths"
        ));
    }
    let m = mempool(Form::Signed).await;
    assert!(
        m.accepted,
        "signed self-exit refused by the mempool: {}",
        m.text
    );
    let b = builder(Form::Signed).await;
    assert!(
        b.accepted,
        "signed self-exit not selected by the builder: {}",
        b.text
    );
    let a = apply(Form::Signed).await;
    assert!(
        a.accepted,
        "signed self-exit: block refused or P still listed (want (0, 0, Exited), queued 0): {}",
        a.text
    );
}

// REQ-PAUTH-004 — Decision: a failure means a self-exit mined mid-epoch never takes P out at the boundary flush (or acts before it).
#[tokio::test]
async fn inc_i_182_m2_req_pauth_004_self_exit_zero_bond_exits_at_epoch_flush() {
    if !in_child() {
        return run_child(&format!(
            "{MOD}inc_i_182_m2_req_pauth_004_self_exit_zero_bond_exits_at_epoch_flush"
        ));
    }
    let mut s = sx(0).await;
    s.r.advance_to(5).await;
    let v = s.apply_with(&s.self_exit(Form::Signed)).await;
    assert!(v.accepted, "signed self-exit at height 6: {}", v.text);
    assert_eq!(
        s.ledger().await,
        Some((P_BONDS, P_BONDS, ProducerStatus::Active)),
        "before the flush P is still listed with its bonds pending withdrawal"
    );
    assert!(s.queued().await > 0, "the withdrawal waits in the queue");
    s.r.advance_to(8).await;
    assert_eq!(
        s.ledger().await,
        Some((0, 0, ProducerStatus::Exited)),
        "after the boundary flush at 8 P has exited"
    );
    assert_eq!(s.queued().await, 0, "nothing left queued for P");
    assert!(
        !s.schedulable_next().await,
        "P is not scheduled after its exit"
    );
}

// REQ-PAUTH-004 — Decision: a failure means the zero-Bond exit opens a way around the producer signature: a tx without P's own auth still moves P out on some path.
#[tokio::test]
async fn inc_i_182_m2_req_pauth_004_zero_bond_exit_unsigned_or_wrong_key_refused() {
    if !in_child() {
        return run_child(&format!(
            "{MOD}inc_i_182_m2_req_pauth_004_zero_bond_exit_unsigned_or_wrong_key_refused"
        ));
    }
    for form in [Form::Unsigned, Form::WrongKey] {
        let m = mempool(form).await;
        assert!(
            !m.accepted && m.coded(),
            "{form:?} self-exit on mempool: want refusal with {CODE}; {}",
            m.text
        );
        let b = builder(form).await;
        assert!(
            !b.accepted && b.coded(),
            "{form:?} self-exit on builder: want skip logged with {CODE}; {}",
            b.text
        );
        let mut s = sx(0).await;
        s.r.advance_to(GATE - 1).await;
        let a = s.apply_with(&s.self_exit(form)).await;
        assert!(
            !a.accepted && a.coded(),
            "{form:?} self-exit on apply: want refusal with {CODE}; {}",
            a.text
        );
        assert_eq!(
            s.ledger().await,
            Some((P_BONDS, 0, ProducerStatus::Active)),
            "{form:?}: a refused block leaves P listed and untouched"
        );
    }
}

// REQ-PAUTH-004 — Decision: the outcome metric; fewer than 3 means a node path still refuses the producer's own signed zero-Bond exit.
#[tokio::test]
async fn inc_i_182_m2_outcome_probe() {
    if !in_child() {
        return run_child(&format!("{MOD}inc_i_182_m2_outcome_probe"));
    }
    let seen = [
        ("mempool", mempool(Form::Signed).await),
        ("builder", builder(Form::Signed).await),
        ("apply", apply(Form::Signed).await),
    ];
    let n = seen.iter().filter(|(_, v)| v.accepted).count();
    println!("{METRIC}{n}");
    assert!(
        n == 3,
        "signed self-exit accepted on {n} of 3 paths: {seen:?}"
    );
}
