//! INC-I-237 M11 / INC-I-246 admission stop-gap — a signed delegation tx is
//! refused at admission when its effect does not match the delegator's effective
//! delegation (current state + queued updates), when apply would refuse it (cap),
//! or when its txid is already mined on the canonical chain. Apply is unchanged.
//!
//! OUTPUT CONTRACT: Node::handle_gossip_txs(Vec<Transaction>) -> network::Verdict
//!   O1: none   O2: self.mempool membership   O3: Verdict::{Accept, Ignore}
//!   O4: none (producer set and block store are fixture inputs; apply runs on copies)
//!   O5: none   O6: none
//! PATHS: P1 Revoke naming a delegate != effective delegated_to -> Ignore;
//!   P2 DelegateBond repeating the effective delegation -> Ignore; P3 DelegateBond
//!   over the received cap -> Ignore; P4 DelegateBond txid in a canonical block ->
//!   Ignore (a mined RevokeDelegation is exempt: refused only via P1, while a
//!   byte-identical second revoke of the current delegate -> Accept);
//!   P5 (GUARD) first-time delegate / matching revoke / same-epoch sequences /
//!   txid only on a side block -> Accept; P6 (GUARD) apply of a replayed revoke
//!   still clears the current delegation at the boundary
//! INPUT PARTITIONS: delegator state {undelegated, delegated to P, delegated to Q};
//!   queued update {none, DelegateBond, RevokeDelegation}; chain index {absent,
//!   canonical block, side block only}; delegate cap {free, full};
//!   params {testnet: auth AH 0, cap 3000 active}.
// covers: zero_flow_auth pool lib gossip_txs validation_checks mod transaction governance context verdict relay_decision init startup gossip-relay-verdict-status gossip-relay-verdict-architecture

use std::collections::HashSet;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::transaction::{DelegateBondData, RevokeDelegationData};
use doli_core::Transaction;
use doli_node::node::Node;
use network::Verdict;
use storage::{PendingProducerUpdate, UtxoSet};
use tempfile::TempDir;

use super::inc_i_237_m11_common as m11;
use super::inc_i_237_m8_common as m8;

const BOND_UNIT: u64 = 1_000_000_000;
const BONDS: u32 = 2;

fn delegate_tx(d: &KeyPair, to: &PublicKey, n: u32) -> Transaction {
    let mut data = DelegateBondData::new(*d.public_key(), *to, n);
    data.signature = crypto::signature::sign_hash(&data.signing_message(), d.private_key());
    Transaction::new_delegate_bond(data)
}

fn revoke_tx(d: &KeyPair, named: &PublicKey) -> Transaction {
    let mut data = RevokeDelegationData::new(*d.public_key(), *named);
    data.signature = crypto::signature::sign_hash(&data.signing_message(), d.private_key());
    Transaction::new_revoke_delegation(data)
}

/// Auth node; producers P and Q registered and active.
struct World {
    node: Node,
    p: KeyPair,
    q: KeyPair,
    _t: TempDir,
}

async fn world() -> World {
    let (node, _kp, _m, t) = m8::auth_node().await;
    let (p, q) = (KeyPair::generate(), KeyPair::generate());
    {
        let mut ps = node.producer_set.write().await;
        for k in [&p, &q] {
            ps.register_genesis_producer(*k.public_key(), 1, BOND_UNIT)
                .expect("register producer");
        }
    }
    World { node, p, q, _t: t }
}

impl World {
    /// A fresh registered delegator with BONDS * 2 bonds, delegated now to `to` (if any).
    async fn delegator(&self, to: Option<&KeyPair>) -> KeyPair {
        let d = KeyPair::generate();
        let mut ps = self.node.producer_set.write().await;
        ps.register_genesis_producer(*d.public_key(), BONDS * 2, BOND_UNIT)
            .expect("register delegator");
        if let Some(to) = to {
            ps.delegate_bonds(d.public_key(), to.public_key(), BONDS)
                .expect("fixture delegation");
        }
        d
    }

    async fn queue(&self, u: PendingProducerUpdate) {
        self.node.producer_set.write().await.queue_update(u);
    }

    async fn delegated_to(&self, d: &KeyPair) -> Option<PublicKey> {
        let ps = self.node.producer_set.read().await;
        ps.get_by_pubkey(d.public_key())
            .and_then(|i| i.delegated_to)
    }

    async fn relays(&self, tx: &Transaction) -> (Verdict, bool) {
        let v = self.node.handle_gossip_txs(vec![tx.clone()]).await;
        (v, m8::has(&self.node, tx).await)
    }

    async fn apply_queues(&self, tx: &Transaction) -> bool {
        let h = m8::next_height(&self.node).await;
        m8::apply_effect(&self.node, tx, h).await
    }
}

// REQ-GRV-036 — Decision: a failure means a public past RevokeDelegation (D -> P) is still admitted and relayed after D delegated to Q, and its apply strips D's current delegation (INC-I-246).
#[tokio::test]
async fn m11_revoke_naming_a_delegate_other_than_the_current_one_is_refused() {
    let w = world().await;
    let d = w.delegator(Some(&w.q)).await;
    let r = revoke_tx(&d, w.p.public_key());
    assert!(
        w.apply_queues(&r).await,
        "fixture: apply queues the mismatched revoke"
    );
    let (v, admitted) = w.relays(&r).await;
    assert!(
        v == Verdict::Ignore && !admitted,
        "REQ-GRV-036 revoke names P, D delegated to Q: verdict={v:?} admitted={admitted}"
    );
}

// REQ-GRV-036 — Decision: a failure means a DelegateBond that repeats D's current delegation (a boundary no-op) is still admitted and relayed.
#[tokio::test]
async fn m11_delegate_bond_repeating_the_current_delegation_is_refused() {
    let w = world().await;
    let d = w.delegator(Some(&w.p)).await;
    let b = delegate_tx(&d, w.p.public_key(), BONDS);
    assert!(w.apply_queues(&b).await, "fixture: apply queues the repeat");
    let (v, admitted) = w.relays(&b).await;
    assert!(
        v == Verdict::Ignore && !admitted,
        "REQ-GRV-036 repeat D -> P: verdict={v:?} admitted={admitted}"
    );
}

// REQ-GRV-036 — Decision: a failure means admission takes and relays a DelegateBond that apply refuses because the delegate's received cap is full.
#[tokio::test]
async fn m11_delegate_bond_over_the_received_cap_is_refused() {
    let w = world().await;
    let cap = w.node.config.network.params().received_delegation_cap;
    assert!(
        cap < u64::from(u32::MAX),
        "fixture: cap active on testnet ({cap})"
    );
    {
        let mut ps = w.node.producer_set.write().await;
        let p = ps
            .get_by_pubkey_mut(w.p.public_key())
            .expect("P registered");
        p.received_delegations.push((Hash::ZERO, cap as u32));
    }
    let d = w.delegator(None).await;
    let b = delegate_tx(&d, w.p.public_key(), 1);
    assert!(
        !w.apply_queues(&b).await,
        "fixture: apply refuses it by cap"
    );
    let (v, admitted) = w.relays(&b).await;
    assert!(
        v == Verdict::Ignore && !admitted,
        "REQ-GRV-036 over cap: verdict={v:?} admitted={admitted}"
    );
}

// REQ-GRV-036 (GUARD) — Decision: a failure means the stop-gap is stronger than apply for an honest first-time DelegateBond or a RevokeDelegation that names the current delegate.
#[tokio::test]
async fn m11_first_time_delegate_bond_and_matching_revoke_still_relay() {
    let w = world().await;
    let fresh = w.delegator(None).await;
    let held = w.delegator(Some(&w.p)).await;
    let rows = vec![
        (
            "first-time D -> P",
            delegate_tx(&fresh, w.p.public_key(), BONDS),
        ),
        (
            "revoke naming current P",
            revoke_tx(&held, w.p.public_key()),
        ),
    ];
    let mut bad = Vec::new();
    for (row, tx) in rows {
        let (v, admitted) = w.relays(&tx).await;
        if v != Verdict::Accept || !admitted {
            bad.push(format!("{row}: verdict={v:?} admitted={admitted}"));
        }
    }
    assert!(bad.is_empty(), "honest delegation txs refused: {bad:#?}");
}

// REQ-GRV-036 (GUARD) — Decision: a failure means the effective delegation ignores updates already queued this epoch, so an honest delegate-then-revoke or revoke-then-delegate is refused.
#[tokio::test]
async fn m11_same_epoch_sequences_against_queued_updates_still_relay() {
    let w = world().await;
    let a = w.delegator(None).await;
    w.queue(PendingProducerUpdate::DelegateBond {
        delegator: *a.public_key(),
        delegate: *w.p.public_key(),
        bond_count: BONDS,
    })
    .await;
    let b = w.delegator(Some(&w.q)).await;
    let c = w.delegator(Some(&w.p)).await;
    for k in [&b, &c] {
        w.queue(PendingProducerUpdate::RevokeDelegation {
            delegator: *k.public_key(),
        })
        .await;
    }
    let rows = vec![
        (
            "queued D -> P, then revoke P",
            revoke_tx(&a, w.p.public_key()),
        ),
        (
            "queued revoke of Q, then D -> P",
            delegate_tx(&b, w.p.public_key(), BONDS),
        ),
        (
            "queued revoke of P, then D -> P again",
            delegate_tx(&c, w.p.public_key(), BONDS),
        ),
    ];
    let mut bad = Vec::new();
    for (row, tx) in rows {
        let (v, admitted) = w.relays(&tx).await;
        if v != Verdict::Accept || !admitted {
            bad.push(format!("{row}: verdict={v:?} admitted={admitted}"));
        }
    }
    assert!(
        bad.is_empty(),
        "same-epoch honest sequences refused: {bad:#?}"
    );
}

// REQ-GRV-036 — Decision: a failure means a RevokeDelegation already mined on the canonical chain can be replayed byte-for-byte to strip a later re-delegation to another producer, or the mined check strands an honest second revoke of the same delegate (byte-identical to the first; residual owed to the INC-I-246 consensus fix).
#[tokio::test]
async fn m11_replay_of_a_canonically_mined_revoke_is_refused() {
    let w = world().await;
    let moved = w.delegator(Some(&w.q)).await;
    let back = w.delegator(Some(&w.p)).await;
    let (r_moved, r_back) = (
        revoke_tx(&moved, w.p.public_key()),
        revoke_tx(&back, w.p.public_key()),
    );
    m11::store_block(&w.node, 100, vec![r_moved.clone(), r_back.clone()], true);
    for r in [&r_moved, &r_back] {
        assert_eq!(
            w.node.block_store.get_tx_block_height(&r.hash()).unwrap(),
            Some(100),
            "fixture: revoke in the chain tx index"
        );
    }
    assert_eq!(
        w.delegated_to(&moved).await,
        Some(*w.q.public_key()),
        "fixture: re-delegated to Q"
    );
    let (v, admitted) = w.relays(&r_moved).await;
    assert!(
        v == Verdict::Ignore && !admitted,
        "REQ-GRV-036 mined revoke replay after D -> Q: verdict={v:?} admitted={admitted}"
    );
    let (v, admitted) = w.relays(&r_back).await;
    assert!(
        v == Verdict::Accept && admitted,
        "honest second revoke of the current delegate P stranded: verdict={v:?} admitted={admitted}"
    );
}

// REQ-GRV-036 — Decision: a failure means a DelegateBond already mined on the canonical chain can be replayed byte-for-byte to force a revoked delegation back.
#[tokio::test]
async fn m11_replay_of_a_canonically_mined_delegate_bond_is_refused() {
    let w = world().await;
    let d = w.delegator(Some(&w.p)).await;
    w.node
        .producer_set
        .write()
        .await
        .revoke_delegation(d.public_key())
        .expect("fixture revoke");
    let b = delegate_tx(&d, w.p.public_key(), BONDS);
    m11::store_block(&w.node, 101, vec![b.clone()], true);
    assert_eq!(
        w.node.block_store.get_tx_block_height(&b.hash()).unwrap(),
        Some(101),
        "fixture: B in the chain tx index"
    );
    assert_eq!(
        w.delegated_to(&d).await,
        None,
        "fixture: delegation since revoked"
    );
    let (v, admitted) = w.relays(&b).await;
    assert!(
        v == Verdict::Ignore && !admitted,
        "REQ-GRV-036 mined delegate replay: verdict={v:?} admitted={admitted}"
    );
}

// REQ-GRV-036 (GUARD) — Decision: a failure means the mined check trusts the raw tx index, so a tx seen only on an abandoned side block can never be re-submitted.
#[tokio::test]
async fn m11_tx_indexed_only_on_a_side_block_is_still_admitted() {
    let w = world().await;
    let d = w.delegator(Some(&w.p)).await;
    let r = revoke_tx(&d, w.p.public_key());
    let fresh = w.delegator(None).await;
    let b = delegate_tx(&fresh, w.p.public_key(), BONDS);
    let other = w.delegator(Some(&w.q)).await;
    m11::store_block(&w.node, 102, vec![r.clone(), b.clone()], false);
    let canon = m11::store_block(
        &w.node,
        102,
        vec![revoke_tx(&other, w.q.public_key())],
        true,
    );
    let mut bad = Vec::new();
    for (row, tx) in [("RevokeDelegation", &r), ("DelegateBond", &b)] {
        assert_eq!(
            w.node.block_store.get_tx_block_height(&tx.hash()).unwrap(),
            Some(102),
            "fixture: {row} indexed at 102"
        );
        assert!(
            !canon.transactions.iter().any(|t| t.hash() == tx.hash()),
            "fixture: canonical 102 does not carry the {row}"
        );
        let (v, admitted) = w.relays(tx).await;
        if v != Verdict::Accept || !admitted {
            bad.push(format!("{row}: verdict={v:?} admitted={admitted}"));
        }
    }
    assert!(bad.is_empty(), "side-block-only txs refused: {bad:#?}");
}

// INC-I-246 (GUARD, apply parity) — Decision: a failure means the admission stop-gap leaked into apply, a consensus change that needs its own activation height.
#[tokio::test]
async fn m11_apply_of_a_replayed_revoke_still_clears_the_current_delegation() {
    let w = world().await;
    let d = w.delegator(Some(&w.q)).await;
    let r = revoke_tx(&d, w.p.public_key());
    let h = m8::next_height(&w.node).await;
    let mut ps = w.node.producer_set.read().await.clone();
    let (mut dirty, mut regs) = (HashSet::new(), Vec::new());
    w.node.process_transaction_producer_effects(
        &r,
        h,
        h as u32,
        &UtxoSet::new(),
        &mut ps,
        &mut dirty,
        &mut regs,
    );
    ps.apply_pending_updates();
    let after = ps
        .get_by_pubkey(d.public_key())
        .and_then(|i| i.delegated_to);
    assert_eq!(
        after, None,
        "INC-I-246 apply defect unchanged: revoke naming P clears D -> Q"
    );
}
