//! INC-I-237 M8 — fixtures: test nodes with a 4-member current maintainer set,
//! on auth-active params (testnet, delegation auth AH 0) or devnet (AH u64::MAX),
//! and genuine / forged zero-flow txs of the 4 authenticated types; delegation
//! parties are registered and active on every node under test (REQ-GRV-038).
//!
//! OUTPUT CONTRACT: N/A — fixture file (helpers only, asserts nothing).
//! INPUT PARTITIONS: N/A — fixture file.
// covers: zero_flow_auth pool lib gossip_txs validation_checks mod transaction governance context verdict relay_decision init startup gossip-relay-verdict-status gossip-relay-verdict-architecture

use std::collections::HashSet;
use std::sync::Arc;

use crypto::{KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::maintainer::{
    signing_message_at, MaintainerSignature, MAINTAINER_AUTH_VALID_BEFORE_UNSET,
};
use doli_core::transaction::{DelegateBondData, RevokeDelegationData};
use doli_core::{MaintainerSet, Network, Transaction};
use doli_node::node::Node;
use mempool::{Mempool, MempoolPolicy};
use storage::{MaintainerState, UtxoSet};
use tempfile::TempDir;
use tokio::sync::RwLock;

use super::inc_i_237_m2_node_common as c;

pub const MAINTAINERS: usize = 4;

/// Tip past every testnet maintainer / zero-flow AH (15_087, 15_087, 25_500), as on the live testnet.
pub const AUTH_TIP: u64 = 30_000;

/// Testnet params in the node config, node params and mempool, so every admission design sees AH 0.
pub fn on_testnet(node: &mut Node) {
    node.config.network = Network::Testnet;
    node.params = ConsensusParams::for_network(Network::Testnet);
    let mut mp = Mempool::new(
        MempoolPolicy::testnet(),
        node.params.clone(),
        Network::Testnet,
    );
    mp.share_oracle_sunset_flag(node.oracle_sunset_triggered.clone());
    mp.share_active_producers_weighted(node.mempool_active_producers_snapshot.clone());
    mp.share_pending_producer_keys(node.mempool_pending_producer_keys.clone());
    mp.share_producer_holdings(node.mempool_producer_holdings.clone());
    mp.share_producer_set(node.producer_set.clone());
    node.mempool = Arc::new(RwLock::new(mp));
}

/// Attach the current maintainer set through the production setter (run.rs path).
pub fn attach(node: &mut Node, maintainers: &[KeyPair]) {
    let members: Vec<PublicKey> = maintainers.iter().map(|k| *k.public_key()).collect();
    let state = MaintainerState {
        set: MaintainerSet::with_members(members, 0),
        ..Default::default()
    };
    node.set_maintainer_state(Arc::new(RwLock::new(state)));
}

pub fn maintainer_keys() -> Vec<KeyPair> {
    (0..MAINTAINERS).map(|_| KeyPair::generate()).collect()
}

/// Auth-active node (testnet params) with `maintainers` as the current set.
pub async fn auth_node_with(maintainers: &[KeyPair]) -> (Node, KeyPair, TempDir) {
    let (mut node, kp, t) = c::make_node().await;
    on_testnet(&mut node);
    attach(&mut node, maintainers);
    {
        let mut cs = node.chain_state.write().await;
        cs.best_height = AUTH_TIP;
        cs.best_slot = AUTH_TIP as u32;
    }
    (node, kp, t)
}

pub async fn auth_node() -> (Node, KeyPair, Vec<KeyPair>, TempDir) {
    let m = maintainer_keys();
    let (node, kp, t) = auth_node_with(&m).await;
    (node, kp, m, t)
}

/// Devnet node (delegation auth AH u64::MAX) with a 4-member current set.
pub async fn devnet_node() -> (Node, KeyPair, Vec<KeyPair>, TempDir) {
    let m = maintainer_keys();
    let (mut node, kp, t) = c::make_node().await;
    attach(&mut node, &m);
    (node, kp, m, t)
}

pub async fn next_height(node: &Node) -> u64 {
    node.chain_state.read().await.best_height + 1
}

#[derive(Clone, Copy, Debug)]
pub enum Sig {
    Genuine,
    Absent,
    Malformed,
    Wrong,
}

pub const FORGED_SIGS: [Sig; 3] = [Sig::Absent, Sig::Malformed, Sig::Wrong];

fn sign_or_forge(msg: &crypto::Hash, delegator: &KeyPair, sig: Sig) -> crypto::Signature {
    match sig {
        Sig::Genuine | Sig::Absent => crypto::signature::sign_hash(msg, delegator.private_key()),
        Sig::Malformed => crypto::Signature::from_bytes([0xFF; 64]),
        Sig::Wrong => crypto::signature::sign_hash(msg, KeyPair::generate().private_key()),
    }
}

pub const DELEGATOR_BONDS: u32 = 2;

/// Delegator and delegate of one delegation tx.
pub struct Parties {
    pub delegator: KeyPair,
    pub delegate: KeyPair,
}

impl Parties {
    /// Keys no node has registered.
    pub fn throwaway() -> Self {
        Self {
            delegator: KeyPair::generate(),
            delegate: KeyPair::generate(),
        }
    }

    /// Fresh keys registered and active on every node; `delegated` applies delegator -> delegate (1 bond).
    pub async fn registered(nodes: &[&Node], delegated: bool) -> Self {
        let p = Self::throwaway();
        for node in nodes {
            let unit = node.config.network.bond_unit();
            let mut ps = node.producer_set.write().await;
            ps.register_genesis_producer(*p.delegator.public_key(), DELEGATOR_BONDS, unit)
                .expect("register delegator");
            ps.register_genesis_producer(*p.delegate.public_key(), 1, unit)
                .expect("register delegate");
            if delegated {
                ps.delegate_bonds(p.delegator.public_key(), p.delegate.public_key(), 1)
                    .expect("fixture delegation");
            }
        }
        p
    }
}

/// DelegateBond of 1 bond from `p.delegator` to `p.delegate`.
pub fn delegate_bond(p: &Parties, sig: Sig) -> Transaction {
    let mut d = DelegateBondData::new(*p.delegator.public_key(), *p.delegate.public_key(), 1);
    d.signature = sign_or_forge(&d.signing_message(), &p.delegator, sig);
    let mut tx = Transaction::new_delegate_bond(d.clone());
    if let Sig::Absent = sig {
        tx.extra_data = d.to_bytes_legacy();
    }
    tx
}

/// RevokeDelegation by `p.delegator` naming `p.delegate`.
pub fn revoke(p: &Parties, sig: Sig) -> Transaction {
    let mut d = RevokeDelegationData::new(*p.delegator.public_key(), *p.delegate.public_key());
    d.signature = sign_or_forge(&d.signing_message(), &p.delegator, sig);
    let mut tx = Transaction::new_revoke_delegation(d.clone());
    if let Sig::Absent = sig {
        tx.extra_data = d.to_bytes_legacy();
    }
    tx
}

/// Genuine DelegateBond by fresh parties registered and active on every node.
pub async fn genuine_delegate_bond(nodes: &[&Node]) -> Transaction {
    delegate_bond(&Parties::registered(nodes, false).await, Sig::Genuine)
}

/// Genuine RevokeDelegation of a delegation applied on every node.
pub async fn genuine_revoke(nodes: &[&Node]) -> Transaction {
    revoke(&Parties::registered(nodes, true).await, Sig::Genuine)
}

/// The exact bytes apply verifies a maintainer change against at `height`.
pub fn maintainer_message(node: &Node, is_add: bool, target: &PublicKey, height: u64) -> Vec<u8> {
    signing_message_at(
        node.params.genesis_hash.as_bytes(),
        is_add,
        target,
        MAINTAINER_AUTH_VALID_BEFORE_UNSET,
        height,
        node.config
            .network
            .params()
            .inc_i_176_auth_binding_activation_height,
    )
}

pub fn msig(kp: &KeyPair, message: &[u8]) -> MaintainerSignature {
    MaintainerSignature::new(
        *kp.public_key(),
        crypto::signature::sign(message, kp.private_key()),
    )
}

pub fn maintainer_tx(
    is_add: bool,
    target: PublicKey,
    sigs: Vec<MaintainerSignature>,
) -> Transaction {
    if is_add {
        Transaction::new_add_maintainer(target, sigs)
    } else {
        Transaction::new_remove_maintainer(target, sigs, None)
    }
}

/// Genuine change: `signers` sign the bytes apply verifies at the next height.
pub async fn signed_change(
    node: &Node,
    is_add: bool,
    target: PublicKey,
    signers: &[&KeyPair],
) -> Transaction {
    let msg = maintainer_message(node, is_add, &target, next_height(node).await);
    maintainer_tx(
        is_add,
        target,
        signers.iter().map(|k| msig(k, &msg)).collect(),
    )
}

pub async fn genuine_add(node: &Node, m: &[KeyPair]) -> Transaction {
    let s: Vec<&KeyPair> = m.iter().take(3).collect();
    signed_change(node, true, *KeyPair::generate().public_key(), &s).await
}

/// Removes the last member, signed by the first three.
pub async fn genuine_remove(node: &Node, m: &[KeyPair]) -> Transaction {
    let s: Vec<&KeyPair> = m.iter().take(3).collect();
    signed_change(node, false, *m[MAINTAINERS - 1].public_key(), &s).await
}

/// Maintainer changes that fail the CURRENT set's multisig at the next height.
pub async fn forged_changes(
    node: &Node,
    m: &[KeyPair],
    is_add: bool,
) -> Vec<(String, Transaction)> {
    let target = if is_add {
        *KeyPair::generate().public_key()
    } else {
        *m[MAINTAINERS - 1].public_key()
    };
    let msg = maintainer_message(node, is_add, &target, next_height(node).await);
    let outsiders: Vec<KeyPair> = (0..3).map(|_| KeyPair::generate()).collect();
    let garbage = |k: &KeyPair| MaintainerSignature {
        pubkey: *k.public_key(),
        signature: crypto::Signature::from_bytes([0xE7; 64]),
    };
    let mut rows: Vec<(&str, Vec<MaintainerSignature>)> = vec![
        (
            "2 member signatures (threshold 3)",
            m[..2].iter().map(|k| msig(k, &msg)).collect(),
        ),
        (
            "3 non-member signatures",
            outsiders.iter().map(|k| msig(k, &msg)).collect(),
        ),
        (
            "3 members over other bytes",
            m[..3].iter().map(|k| msig(k, b"other")).collect(),
        ),
        (
            "3 garbage member signatures",
            m[..3].iter().map(garbage).collect(),
        ),
    ];
    if !is_add {
        rows.push((
            "target counted as a signer",
            m[1..].iter().map(|k| msig(k, &msg)).collect(),
        ));
    }
    let kind = if is_add {
        "AddMaintainer"
    } else {
        "RemoveMaintainer"
    };
    rows.into_iter()
        .map(|(row, sigs)| {
            (
                format!("{kind}: {row}"),
                maintainer_tx(is_add, target, sigs),
            )
        })
        .collect()
}

/// Apply-time state effect of `tx` at `height`, on copies (the node's state is restored).
pub async fn apply_effect(node: &Node, tx: &Transaction, height: u64) -> bool {
    let mut ps = node.producer_set.read().await.clone();
    let before_updates = ps.pending_update_count();
    let (mut dirty, mut regs) = (HashSet::new(), Vec::new());
    node.process_transaction_producer_effects(
        tx,
        height,
        height as u32,
        &UtxoSet::new(),
        &mut ps,
        &mut dirty,
        &mut regs,
    );
    let queued = ps.pending_update_count() != before_updates;
    let ms = node
        .maintainer_state
        .as_ref()
        .expect("maintainer set attached");
    let saved = ms.read().await.clone();
    let producers = node.producer_set.read().await.clone();
    let _ = node
        .process_transaction_governance(tx, height, &producers)
        .await;
    let changed = ms.read().await.set.members != saved.set.members;
    *ms.write().await = saved;
    queued || changed
}

pub async fn pool_len(node: &Node) -> usize {
    node.mempool.read().await.len()
}

pub async fn has(node: &Node, tx: &Transaction) -> bool {
    node.mempool.read().await.contains(&tx.hash())
}
