//! INC-I-237 M12.1 — fixtures: an auth node with producers P and Q registered
//! and active, signed delegation txs, the epoch-boundary outcome of a tx, the
//! node's own RPC `sendTransaction`, and a counting pump for the A -> B -> C line.
//!
//! OUTPUT CONTRACT: N/A — fixture file (helpers only, asserts nothing but setup).
//! INPUT PARTITIONS: N/A — fixture file.
// covers: zero_flow_policy zero_flow_auth zero_flow_newness zero_flow_standing pool lib transaction

use std::collections::HashSet;
use std::time::{Duration, Instant};

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::transaction::{DelegateBondData, RevokeDelegationData};
use doli_core::{Transaction, TxType};
use doli_node::node::Node;
use network::Verdict;
use rpc::types::JsonRpcRequest;
use rpc::{RpcContext, RpcError};
use serde_json::{json, Value};
use storage::{PendingProducerUpdate, ProducerSet, ProducerStatus, UtxoSet};
use tempfile::TempDir;

use super::inc_i_237_m11_common::Line;
use super::inc_i_237_m2_node_common as c;
use super::inc_i_237_m8_common as m8;

pub const BOND_UNIT: u64 = 1_000_000_000;
pub const BONDS: u32 = 2;
pub const PARITY_CODE: &str = "MPTX016";
pub const STOP_GAP_CODE: &str = "MPTX013";

pub fn delegate_tx(d: &KeyPair, to: &PublicKey, n: u32) -> Transaction {
    let mut data = DelegateBondData::new(*d.public_key(), *to, n);
    data.signature = crypto::signature::sign_hash(&data.signing_message(), d.private_key());
    Transaction::new_delegate_bond(data)
}

pub fn revoke_tx(d: &KeyPair, named: &PublicKey) -> Transaction {
    let mut data = RevokeDelegationData::new(*d.public_key(), *named);
    data.signature = crypto::signature::sign_hash(&data.signing_message(), d.private_key());
    Transaction::new_revoke_delegation(data)
}

/// Validly signed DelegateBond of 1 bond between two keys no node has registered.
pub fn throwaway_delegation() -> Transaction {
    delegate_tx(&KeyPair::generate(), KeyPair::generate().public_key(), 1)
}

/// Auth node (testnet params, tip past every AH); producers P and Q registered and active.
pub struct World {
    pub node: Node,
    pub p: KeyPair,
    pub q: KeyPair,
    pub rpc_ctx: RpcContext,
    _t: TempDir,
}

pub async fn world() -> World {
    let (node, _kp, _m, t) = m8::auth_node().await;
    let (p, q) = (KeyPair::generate(), KeyPair::generate());
    {
        let mut ps = node.producer_set.write().await;
        for k in [&p, &q] {
            ps.register_genesis_producer(*k.public_key(), 1, BOND_UNIT)
                .expect("register producer");
        }
    }
    let rpc_ctx = rpc_context(&node);
    World {
        node,
        p,
        q,
        rpc_ctx,
        _t: t,
    }
}

/// The node's RPC context as startup wires it: same chain state, block store, mempool, producer and maintainer sets.
pub fn rpc_context(node: &Node) -> RpcContext {
    let mut ctx = RpcContext::new_for_network(
        node.chain_state.clone(),
        node.block_store.clone(),
        node.utxo_set.clone(),
        node.mempool.clone(),
        node.params.clone(),
        node.config.network,
    )
    .with_producer_set(node.producer_set.clone());
    ctx.maintainer_state = node.maintainer_state.clone();
    ctx
}

pub async fn send_rpc(ctx: &RpcContext, tx: &Transaction) -> Result<Value, RpcError> {
    let resp = ctx
        .handle_request(JsonRpcRequest {
            jsonrpc: "2.0".into(),
            method: "sendTransaction".into(),
            params: json!({"tx": hex::encode(tx.serialize())}),
            id: json!(1),
        })
        .await;
    match resp.error {
        Some(e) => Err(e),
        None => Ok(resp.result.unwrap_or(Value::Null)),
    }
}

/// Error code + message, as the M11 suites match codes.
pub fn error_text(e: &RpcError) -> String {
    let data = e
        .data
        .as_ref()
        .and_then(|d| d["error_code"].as_str())
        .unwrap_or("");
    format!("{data} {}", e.message)
}

fn delegator_of(tx: &Transaction) -> Option<PublicKey> {
    match tx.tx_type {
        TxType::DelegateBond => tx.delegate_bond_data().map(|d| d.delegator),
        TxType::RevokeDelegation => tx.revoke_delegation_data().map(|d| d.delegator),
        _ => None,
    }
}

fn delegation_of(ps: &ProducerSet, k: &PublicKey) -> Option<(Option<PublicKey>, u32)> {
    ps.get_by_pubkey(k)
        .map(|i| (i.delegated_to, i.delegated_bonds))
}

impl World {
    /// A fresh producer registered and active with `bonds` bonds.
    pub async fn producer(&self, bonds: u32) -> KeyPair {
        let k = KeyPair::generate();
        self.node
            .producer_set
            .write()
            .await
            .register_genesis_producer(*k.public_key(), bonds, BOND_UNIT)
            .expect("register producer");
        k
    }

    /// A fresh registered delegator with BONDS * 2 bonds, delegated now (BONDS) to `to` if any.
    pub async fn delegator(&self, to: Option<&KeyPair>) -> KeyPair {
        let d = self.producer(BONDS * 2).await;
        if let Some(to) = to {
            self.node
                .producer_set
                .write()
                .await
                .delegate_bonds(d.public_key(), to.public_key(), BONDS)
                .expect("fixture delegation");
        }
        d
    }

    pub async fn deactivate(&self, k: &KeyPair) {
        let mut ps = self.node.producer_set.write().await;
        ps.get_by_pubkey_mut(k.public_key())
            .expect("registered")
            .status = ProducerStatus::Exited;
    }

    pub async fn set_withdrawal_pending(&self, k: &KeyPair, n: u32) {
        let mut ps = self.node.producer_set.write().await;
        ps.get_by_pubkey_mut(k.public_key())
            .expect("registered")
            .withdrawal_pending_count = n;
    }

    pub async fn queue(&self, u: PendingProducerUpdate) {
        self.node.producer_set.write().await.queue_update(u);
    }

    /// One gossip message carrying `tx`: (verdict, resident afterwards).
    pub async fn gossip(&self, tx: &Transaction) -> (Verdict, bool) {
        let v = self.node.handle_gossip_txs(vec![tx.clone()]).await;
        (v, m8::has(&self.node, tx).await)
    }

    pub async fn rpc(&self, tx: &Transaction) -> Result<Value, RpcError> {
        send_rpc(&self.rpc_ctx, tx).await
    }

    pub async fn evict(&self, tx: &Transaction) {
        self.node
            .mempool
            .write()
            .await
            .remove_transaction(&tx.hash());
    }

    /// True iff mining `tx` at the next height changes its delegator's delegation after the epoch boundary.
    pub async fn boundary_effect(&self, tx: &Transaction) -> bool {
        let who = delegator_of(tx).expect("delegation tx");
        let h = m8::next_height(&self.node).await;
        let mut base = self.node.producer_set.read().await.clone();
        let mut with = base.clone();
        let (mut dirty, mut regs) = (HashSet::new(), Vec::new());
        self.node.process_transaction_producer_effects(
            tx,
            h,
            h as u32,
            &UtxoSet::new(),
            &mut with,
            &mut dirty,
            &mut regs,
        );
        base.apply_pending_updates();
        with.apply_pending_updates();
        delegation_of(&base, &who) != delegation_of(&with, &who)
    }

    /// Rows refused on both sites: (row, gossip verdict, resident, rpc result text).
    pub async fn refusal_failures(
        &self,
        rows: Vec<(&str, Transaction)>,
        code: &str,
    ) -> Vec<String> {
        let mut bad = Vec::new();
        for (row, tx) in rows {
            assert!(
                !self.boundary_effect(&tx).await,
                "fixture: the epoch boundary turns `{row}` into a no-op"
            );
            let (v, resident) = self.gossip(&tx).await;
            self.evict(&tx).await;
            let res = self.rpc(&tx).await;
            let rpc_resident = m8::has(&self.node, &tx).await;
            self.evict(&tx).await;
            let text = match &res {
                Ok(v) => format!("Ok({v})"),
                Err(e) => error_text(e),
            };
            if v != Verdict::Ignore || resident || !text.contains(code) || rpc_resident {
                bad.push(format!(
                    "{row}: gossip verdict={v:?} resident={resident}; rpc={text} resident={rpc_resident}"
                ));
            }
        }
        bad
    }

    /// Rows admitted on both sites: gossip Accept + resident, then (evicted) RPC Ok + resident.
    pub async fn admission_failures(&self, rows: Vec<(&str, Transaction)>) -> Vec<String> {
        let mut bad = Vec::new();
        for (row, tx) in rows {
            assert!(
                self.boundary_effect(&tx).await,
                "fixture: the epoch boundary applies `{row}`"
            );
            let (v, resident) = self.gossip(&tx).await;
            self.evict(&tx).await;
            let res = self.rpc(&tx).await;
            let rpc_resident = m8::has(&self.node, &tx).await;
            self.evict(&tx).await;
            if v != Verdict::Accept || !resident || res.is_err() || !rpc_resident {
                let text = res.as_ref().err().map(error_text).unwrap_or_default();
                bad.push(format!(
                    "{row}: gossip verdict={v:?} resident={resident}; rpc_err={text:?} resident={rpc_resident}"
                ));
            }
        }
        bad
    }
}

/// C's connected peers.
pub async fn peers_of(node: &Node) -> Vec<String> {
    let net = node.network.as_ref().expect("node network attached");
    let peers = net.peers_arc();
    let ids: Vec<String> = peers.read().await.keys().map(|p| p.to_string()).collect();
    ids
}

pub fn peer_id(node: &Node) -> String {
    node.network
        .as_ref()
        .expect("node network attached")
        .local_peer_id()
        .to_string()
}

/// Feed `node` its tx gossip events for `dur`, recording which `watch` txids arrived.
pub async fn pump_counting(
    node: &mut Node,
    dur: Duration,
    watch: &HashSet<Hash>,
    arrived: &mut HashSet<Hash>,
) {
    let until = Instant::now() + dur;
    while Instant::now() < until {
        let net = node.network.as_mut().expect("node network attached");
        if let Ok(Some(ev)) =
            tokio::time::timeout(Duration::from_millis(50), net.next_event()).await
        {
            if let Some(txs) = c::txs_of(&ev) {
                arrived.extend(txs.iter().map(|t| t.hash()).filter(|h| watch.contains(h)));
                let _ = node.handle_network_event(ev).await;
            }
        }
    }
}

/// Pump B, then pump C while recording the `watch` txids that arrive at C (B's forwards).
pub async fn pump_line_counting(
    l: &mut Line,
    dur: Duration,
    watch: &HashSet<Hash>,
    arrived: &mut HashSet<Hash>,
) {
    c::pump(&mut l.b, dur).await;
    pump_counting(&mut l.c, dur, watch, arrived).await;
}

/// Pump until `marker` arrives at C or `secs` pass; returns whether it arrived.
pub async fn pump_until_marker(
    l: &mut Line,
    marker: Hash,
    watch: &HashSet<Hash>,
    arrived: &mut HashSet<Hash>,
    secs: u64,
) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !arrived.contains(&marker) && Instant::now() < deadline {
        pump_line_counting(l, Duration::from_millis(500), watch, arrived).await;
    }
    let reached = arrived.contains(&marker);
    if reached {
        pump_line_counting(l, Duration::from_secs(1), watch, arrived).await;
    }
    reached
}
