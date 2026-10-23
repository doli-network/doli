//! INC-I-182 M2 — after its own full withdrawal a producer is Exited with no
//! bonds, so nothing is left to exit: no further withdrawal naming it, signed
//! by it or by anyone else, is accepted on any path.
//!
// covers: exit unbonding_tx producer_tx producer_ledger withdrawal_economics withdrawal_holdings tx_processing pool assembly validation_checks producer_auth unbonding transaction
//
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O1 return; O2 P's (bond_count, withdrawal_pending_count, status); O3 P's queued updates; O4 P's Bond UTXOs
// OUTPUT CONTRACT: fn Mempool::add_transaction(..) -> Result<.., MempoolError>   O5 return; O6 residency
// OUTPUT CONTRACT: fn Node::build_block_content(..)   O7 transactions selected
// PATHS: child process, producer-auth gate 2, holdings gate 0, genesis blocks 0, Light apply
//   P-DONE P's signed full withdrawal (every Bond UTXO, full allowance) mined at 2, epoch 0
//   then P-MEMPOOL tip 2 | P-BUILDER height 3 | P-APPLY block at 3
// MATRIX
//   O1-O4 x P-DONE -> ..._full_withdrawal_leaves_exited_entry_without_bonds
//   O5, O6, O7, O1, O2 x {P-MEMPOOL, P-BUILDER, P-APPLY} x attempts -> ..._after_full_withdrawal_no_key_can_exit_again
// INPUT PARTITIONS:
//   P ledger 3 bonds, 3 Bond UTXOs before; attempts: P signed declaring 3 | P signed declaring 1
//   | unsigned | another producer's key | a third party paying with its own coin, P signs nothing

use storage::ProducerStatus;

use super::inc_i_182_m1_common::{forced_exit, COIN as T_COIN};
use super::inc_i_182_m2_node_fx::*;
use super::inc_i_255_m2_common::{live, outpoints, ACCUSED_TAG, COIN};
use super::inc_i_255_m4_common::resident;
use super::inc_i_255_m6_common::seed_coin;

const MOD: &str = "inc_i_182_m2_after_withdrawal::";
const EXITED: Ledger = Some((0, 0, ProducerStatus::Exited));

/// P's own signed full withdrawal mined at `GATE` (epoch 0: applied at once).
async fn withdrawn() -> Sx {
    let mut s = sx(P_BONDS).await;
    s.r.advance_to(GATE - 1).await;
    let v = s.apply_with(&s.full_withdrawal()).await;
    assert!(
        v.accepted,
        "PRECONDITION: P's signed full withdrawal is mined; {}",
        v.text
    );
    s
}

// REQ-PAUTH-004 — Decision: a failure means a full withdrawal leaves P listed with bonds or live Bond UTXOs, so a second exit would be needed and must be designed.
#[tokio::test]
async fn inc_i_182_m2_req_pauth_004_full_withdrawal_leaves_exited_entry_without_bonds() {
    if !in_child() {
        return run_child(&format!(
            "{MOD}inc_i_182_m2_req_pauth_004_full_withdrawal_leaves_exited_entry_without_bonds"
        ));
    }
    let s = withdrawn().await;
    assert_eq!(
        s.ledger().await,
        EXITED,
        "P's entry stays in the set as Exited with 0 bonds and 0 pending"
    );
    assert_eq!(s.queued().await, 0, "nothing left queued for P");
    assert!(
        live(&s.r.node, &outpoints(ACCUSED_TAG, P_BONDS))
            .await
            .is_empty(),
        "every Bond UTXO P owned is spent"
    );
    assert!(
        !s.schedulable_next().await,
        "P is not scheduled after its exit"
    );
}

// REQ-PAUTH-004 — Decision: a failure means an Exited producer's entry can still be acted on (by P or by a third party), so the exit is not final.
#[tokio::test]
async fn inc_i_182_m2_req_pauth_004_after_full_withdrawal_no_key_can_exit_again() {
    if !in_child() {
        return run_child(&format!(
            "{MOD}inc_i_182_m2_req_pauth_004_after_full_withdrawal_no_key_can_exit_again"
        ));
    }
    // Nothing is left to exit, so the criterion holds by refusal for P and for third parties alike.
    let mut s = withdrawn().await;
    let third = crypto::KeyPair::from_seed([0x7E; 32]);
    let t_coin = seed_coin(&s.r.node, third.public_key(), 0x7E, T_COIN).await;
    let attempts = [
        ("P signed, former allowance", s.self_exit(Form::Signed)),
        (
            "P signed, one bond",
            s.withdrawal(&[s.coin()], 1, COIN - FEE, Form::Signed),
        ),
        ("unsigned", s.self_exit(Form::Unsigned)),
        ("another key", s.self_exit(Form::WrongKey)),
        (
            "third party coin",
            forced_exit(&third, t_coin, &s.pk(), P_BONDS),
        ),
    ];
    for (what, tx) in attempts {
        let m = s.admit(&tx).await;
        assert!(
            !m.accepted && !resident(&s.r.node, &tx).await,
            "{what}: mempool admitted a withdrawal for an Exited P: {}",
            m.text
        );
        let b = s.build_next(&tx).await;
        assert!(!b.accepted, "{what}: builder selected it: {}", b.text);
        let a = s.apply_with(&tx).await;
        assert!(!a.accepted, "{what}: block accepted: {}", a.text);
        assert_eq!(s.ledger().await, EXITED, "{what}: P's entry changed");
    }
}
