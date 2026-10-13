//! INC-I-237 M12.1 / REQ-GRV-037 (GUARD) — honest delegation flows the epoch
//! boundary applies stay admissible on both delegation sites: gossip
//! (`Node::handle_gossip_txs`) and RPC `sendTransaction`. The queued Register /
//! AddBond rows pin that admission counts the delegator's queued updates.
//!
//! OUTPUT CONTRACT: Node::handle_gossip_txs(Vec<Transaction>) -> network::Verdict and
//!   RpcContext::handle_request(sendTransaction) -> JsonRpcResponse
//!   O1: none   O2: node.mempool membership   O3: Verdict::Accept; RPC Ok(hash)
//!   O4: none (producer set is a fixture input; the boundary runs on copies)
//!   O5: none   O6: none (network = None)
//! PATHS: P1 first-time DelegateBond, active delegator with free bonds -> active
//!   delegate; P2 revoke naming the effective delegate; P3 queued Register, then
//!   DelegateBond; P4 queued AddBond, then DelegateBond above the applied bonds
//!   and within applied + queued; P5 queued Register of the delegate, then
//!   DelegateBond to it -> all Accept + resident, RPC Ok + resident
//! INPUT PARTITIONS: delegator {applied active, queued Register only, applied +
//!   queued AddBond}; delegate {applied active, queued Register only};
//!   effective delegation {none, to the named delegate};
//!   params {testnet: auth AH 0, tip 30_000}.
//! MATRIX: every row first asserts the boundary applies it (non-vacuity).
// covers: zero_flow_policy zero_flow_auth zero_flow_newness zero_flow_standing pool lib transaction

use crypto::{Hash, KeyPair};
use storage::{PendingProducerUpdate, ProducerInfo};

use super::inc_i_237_m12_common::{delegate_tx, revoke_tx, world, BONDS, BOND_UNIT};
use super::inc_i_237_m8_common as m8;

// REQ-GRV-037 — Decision: a failure means the standing check is stronger than the boundary for an honest first-time delegation between two active producers.
#[tokio::test]
async fn m12_req_grv_037_first_time_delegation_is_admitted() {
    let w = world().await;
    let d = w.delegator(None).await;
    let rows = vec![(
        "first-time D -> P",
        delegate_tx(&d, w.p.public_key(), BONDS),
    )];
    let bad = w.admission_failures(rows).await;
    assert!(bad.is_empty(), "REQ-GRV-037: {bad:#?}");
}

// REQ-GRV-037 — Decision: a failure means a revoke of the delegator's effective delegation, which the boundary applies, is refused.
#[tokio::test]
async fn m12_req_grv_037_revoke_naming_the_effective_delegate_is_admitted() {
    let w = world().await;
    let d = w.delegator(Some(&w.p)).await;
    let rows = vec![("revoke naming current P", revoke_tx(&d, w.p.public_key()))];
    let bad = w.admission_failures(rows).await;
    assert!(bad.is_empty(), "REQ-GRV-037: {bad:#?}");
}

// REQ-GRV-037 — Decision: a failure means admission reads only applied state, refusing a register-then-delegate in one epoch that the boundary applies in queue order.
#[tokio::test]
async fn m12_req_grv_037_register_then_delegate_in_one_epoch_is_admitted() {
    let w = world().await;
    let d = KeyPair::generate();
    let h = m8::next_height(&w.node).await;
    let info = ProducerInfo::new(
        *d.public_key(),
        h,
        u64::from(BONDS) * BOND_UNIT,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );
    w.queue(PendingProducerUpdate::Register {
        info: Box::new(info),
        height: h,
    })
    .await;
    let rows = vec![(
        "queued Register, then D -> P",
        delegate_tx(&d, w.p.public_key(), BONDS),
    )];
    let bad = w.admission_failures(rows).await;
    assert!(bad.is_empty(), "REQ-GRV-037: {bad:#?}");
}

// REQ-GRV-037 — Decision: a failure means admission counts only applied bonds, refusing a delegation of bonds added this epoch that the boundary applies.
#[tokio::test]
async fn m12_req_grv_037_add_bond_then_delegate_within_the_new_bonds_is_admitted() {
    let w = world().await;
    let d = w.producer(1).await;
    let added = 2u32;
    w.queue(PendingProducerUpdate::AddBond {
        pubkey: *d.public_key(),
        outpoints: (0..added)
            .map(|i| (Hash::from_bytes([0xAB; 32]), i))
            .collect(),
        bond_unit: BOND_UNIT,
        creation_slot: 0,
    })
    .await;
    let rows = vec![(
        "1 applied + 2 queued bonds, then D -> P with 3",
        delegate_tx(&d, w.p.public_key(), 1 + added),
    )];
    let bad = w.admission_failures(rows).await;
    assert!(bad.is_empty(), "REQ-GRV-037: {bad:#?}");
}

// REQ-GRV-037 — Decision: a failure means admission reads only the delegate's applied state, refusing a delegation to a producer registered this epoch that the boundary applies in queue order.
#[tokio::test]
async fn m12_req_grv_037_delegate_registered_this_epoch_is_admitted() {
    let w = world().await;
    let d = w.delegator(None).await;
    let to = KeyPair::generate();
    let h = m8::next_height(&w.node).await;
    let info = ProducerInfo::new(
        *to.public_key(),
        h,
        u64::from(BONDS) * BOND_UNIT,
        (Hash::ZERO, 0),
        0,
        BOND_UNIT,
    );
    w.queue(PendingProducerUpdate::Register {
        info: Box::new(info),
        height: h,
    })
    .await;
    let rows = vec![(
        "queued Register of T, then D -> T",
        delegate_tx(&d, to.public_key(), BONDS),
    )];
    let bad = w.admission_failures(rows).await;
    assert!(bad.is_empty(), "REQ-GRV-037: {bad:#?}");
}
