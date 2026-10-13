//! INC-I-237 M11 — fixtures: keyless variants of a captured maintainer change
//! (AUDIT-P1-509), the apply-and-mine step, block-store placement, and the
//! A -> B(node) -> C(node) relay line shared with the M8 REQ-GRV-029 test.
//!
//! OUTPUT CONTRACT: N/A — fixture file (helpers only, asserts nothing but setup).
//! INPUT PARTITIONS: N/A — fixture file.
// covers: zero_flow_auth pool lib gossip_txs validation_checks mod transaction governance context verdict relay_decision init startup gossip-relay-verdict-status gossip-relay-verdict-architecture

use std::time::{Duration, Instant};

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::maintainer::MaintainerChangeData;
use doli_core::{Block, BlockHeader, Transaction, TxType};
use doli_node::node::Node;
use network::NetworkCommand;
use tempfile::TempDir;
use tokio::sync::mpsc;
use vdf::{VdfOutput, VdfProof};

use super::inc_i_237_m2_node_common as c;
use super::inc_i_237_m8_common as m8;

pub fn data(tx: &Transaction) -> MaintainerChangeData {
    MaintainerChangeData::from_bytes(&tx.extra_data).expect("maintainer change data")
}

fn rewrite(tx: &Transaction, f: impl FnOnce(&mut MaintainerChangeData)) -> Transaction {
    let mut d = data(tx);
    f(&mut d);
    let mut out = tx.clone();
    out.extra_data = d.to_bytes();
    out
}

pub fn with_reason(tx: &Transaction, reason: &str) -> Transaction {
    rewrite(tx, |d| d.reason = Some(reason.to_string()))
}

pub fn with_trailing(tx: &Transaction, bytes: &[u8]) -> Transaction {
    let mut out = tx.clone();
    out.extra_data.extend_from_slice(bytes);
    out
}

pub fn reordered(tx: &Transaction) -> Transaction {
    rewrite(tx, |d| d.signatures.reverse())
}

pub fn duplicated(tx: &Transaction) -> Transaction {
    rewrite(tx, |d| {
        let first = d.signatures[0].clone();
        d.signatures.push(first);
    })
}

/// The four AUDIT-P1-509 keyless rewrites of a captured change (none touches the signed bytes).
pub fn variants(tx: &Transaction) -> Vec<(&'static str, Transaction)> {
    vec![
        ("different reason", with_reason(tx, "rewritten by a relay")),
        ("one trailing byte", with_trailing(tx, &[0x00])),
        ("reordered signature entries", reordered(tx)),
        ("duplicated signature entry", duplicated(tx)),
    ]
}

/// The i-th member of an unbounded family of distinct keyless variants.
pub fn nth_variant(tx: &Transaction, i: u32) -> Transaction {
    match i % 4 {
        0 => with_reason(tx, &format!("r{i}")),
        1 => with_trailing(tx, &i.to_le_bytes()),
        2 => reordered(&with_reason(tx, &format!("o{i}"))),
        _ => duplicated(&with_reason(tx, &format!("d{i}"))),
    }
}

pub fn is_maintainer_change(tx: &Transaction) -> bool {
    matches!(tx.tx_type, TxType::AddMaintainer | TxType::RemoveMaintainer)
}

pub async fn maintainer_residents(node: &Node) -> usize {
    node.mempool
        .read()
        .await
        .iter()
        .filter(|(_, e)| is_maintainer_change(&e.tx))
        .count()
}

/// Mine `tx` at the next height the way apply_block does for it: governance runs, the tip advances, the mempool drops it.
pub async fn apply_and_mine(node: &Node, tx: &Transaction) {
    let h = m8::next_height(node).await;
    let producers = node.producer_set.read().await.clone();
    let _ = node.process_transaction_governance(tx, h, &producers).await;
    {
        let mut cs = node.chain_state.write().await;
        cs.best_height = h;
        cs.best_slot = h as u32;
    }
    node.mempool
        .write()
        .await
        .remove_for_block(std::slice::from_ref(tx));
}

pub async fn is_member(node: &Node, pk: &crypto::PublicKey) -> bool {
    let ms = node.maintainer_state.as_ref().expect("set attached");
    let set = ms.read().await.set.clone();
    set.is_maintainer(pk)
}

/// A block at `height` carrying `txs`, written to the node's block store (canonical or side).
pub fn store_block(node: &Node, height: u64, txs: Vec<Transaction>, canonical: bool) -> Block {
    let header = BlockHeader {
        version: 2,
        prev_hash: Hash::ZERO,
        merkle_root: doli_core::block::compute_merkle_root(&txs),
        presence_root: Hash::ZERO,
        genesis_hash: ConsensusParams::for_network(node.config.network).genesis_hash,
        timestamp: node.params.genesis_time + height * node.params.slot_duration,
        slot: height as u32,
        producer: *KeyPair::generate().public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    let block = Block::new(header, txs);
    if canonical {
        node.block_store.put_block_canonical(&block, height)
    } else {
        node.block_store.put_block(&block, height)
    }
    .expect("block store write");
    block
}

/// Live line A(raw origin) -> B(node) -> C(node) on 127.0.0.1, same maintainer set at B and C.
pub struct Line {
    pub a_cmd: mpsc::Sender<NetworkCommand>,
    pub b: Node,
    pub c: Node,
    pub m: Vec<KeyPair>,
    _t: (TempDir, TempDir),
}

pub async fn pump_both(l: &mut Line, d: Duration) {
    c::pump(&mut l.b, d).await;
    c::pump(&mut l.c, d).await;
}

pub async fn publish(l: &Line, tx: &Transaction) {
    l.a_cmd
        .send(NetworkCommand::BroadcastTransaction(tx.clone()))
        .await
        .unwrap();
}

/// A -> B -> C, warmed until B's own publish is admitted at C.
pub async fn line() -> Line {
    let m = m8::maintainer_keys();
    let (mut b, _kb, tb) = m8::auth_node_with(&m).await;
    let (mut cn, kc, tc) = m8::auth_node_with(&m).await;
    let b_port = c::reserve_port();
    b.network = Some(c::start_net(b_port, vec![]).await);
    cn.network = Some(c::start_net(c::reserve_port(), vec![c::addr(b_port)]).await);
    let mut a = c::start_net(c::reserve_port(), vec![c::addr(b_port)]).await;
    let a_cmd = a.command_sender();
    tokio::spawn(async move { while a.next_event().await.is_some() {} });
    let mut l = Line {
        a_cmd,
        b,
        c: cn,
        m,
        _t: (tb, tc),
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    for tag in 100u8.. {
        assert!(Instant::now() < deadline, "setup: B -> C mesh never formed");
        c::seed(&l.c, &kc, c::prev(tag)).await;
        let warm = c::transfer(&kc, c::prev(tag));
        let b_cmd = l.b.network.as_ref().unwrap().command_sender();
        b_cmd
            .send(NetworkCommand::BroadcastTransaction(warm.clone()))
            .await
            .unwrap();
        pump_both(&mut l, Duration::from_millis(400)).await;
        if m8::has(&l.c, &warm).await {
            break;
        }
    }
    l
}
