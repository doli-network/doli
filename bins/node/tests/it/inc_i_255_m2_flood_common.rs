//! INC-I-255 M2 flood-control fixtures: a node with many registered accused
//! producers, distinct evidence per accused, and an every-path block verdict.
//!
//! OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.

#![allow(dead_code)]

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::transaction::{Transaction, TxType};
use doli_core::Block;
use doli_node::node::Node;
use tempfile::TempDir;

use super::inc_i_255_m1_common::*;
use crate::inc_i_156_m1_harness::install_production_utxo_backend;

/// Consensus per-block slash cap fixed by the milestone contract.
pub const SLASH_CAP: usize = 8;

/// Devnet node keyed by `reporter` (its only scheduled producer) whose live
/// producer set also holds every `accused` key as an Active producer.
pub struct Flock {
    pub node: Node,
    pub reporter: KeyPair,
    pub accused: Vec<KeyPair>,
    _tmp: TempDir,
}

pub async fn flock(n: usize) -> Flock {
    let reporter = KeyPair::generate();
    let tmp = TempDir::new().unwrap();
    let node = Node::new_for_test(tmp.path().to_path_buf(), vec![reporter.clone()])
        .await
        .expect("Node::new_for_test failed");
    install_production_utxo_backend(&node).await;
    let accused: Vec<KeyPair> = (0..n).map(|_| KeyPair::generate()).collect();
    let unit = node.config.network.bond_unit();
    {
        let mut ps = node.producer_set.write().await;
        for kp in &accused {
            ps.register_genesis_producer(*kp.public_key(), 1, unit)
                .expect("FIXTURE: register accused");
        }
        node.state_db
            .write_producer_set(&ps)
            .expect("FIXTURE: persist producer set");
    }
    Flock {
        node,
        reporter,
        accused,
        _tmp: tmp,
    }
}

impl Flock {
    pub fn pk(&self, i: usize) -> PublicKey {
        *self.accused[i].public_key()
    }

    /// One valid canonical slash per accused in `range`.
    pub fn slashes(&self, range: std::ops::Range<usize>) -> Vec<Transaction> {
        range
            .map(|i| slash_of(&self.accused[i], 0xA1, 0xB2))
            .collect()
    }
}

/// Valid canonical slash for `kp` whose evidence pair is chosen by the salts.
pub fn slash_of(kp: &KeyPair, salt_a: u8, salt_b: u8) -> Transaction {
    let g = chain_genesis();
    let a = evidence_header(kp, g, SLOT, salt_a, 0);
    let b = evidence_header(kp, g, SLOT, salt_b, 0);
    canonical(*kp.public_key(), a, b)
}

pub fn slash_count(block: &Block, selected: &[Hash]) -> usize {
    block
        .transactions
        .iter()
        .filter(|t| t.tx_type == TxType::SlashProducer && selected.contains(&t.hash()))
        .count()
}

pub fn slashes_naming(block: &Block, selected: &[Hash], pk: &PublicKey) -> usize {
    block
        .transactions
        .iter()
        .filter(|t| selected.contains(&t.hash()))
        .filter(|t| t.slash_data().is_some_and(|d| d.producer_pubkey == *pk))
        .count()
}

pub async fn resident_slash_total(node: &Node) -> usize {
    let pool = node.mempool.read().await;
    pool.iter()
        .filter(|(_, e)| e.tx.tx_type == TxType::SlashProducer)
        .count()
}

/// `validate_block_for_apply` and `apply_block` in every mode must refuse
/// `block` with one identical `InvalidSlash` message and leave the tip; returns it.
pub async fn refused_everywhere(node: &mut Node, block: &Block, case: &str) -> String {
    let mut seen: Vec<(String, Option<String>, String)> = Vec::new();
    let height = tip(node).await.0 + 1;
    for mode in MODES {
        let r = node.validate_block_for_apply(block, height, mode).await;
        seen.push((
            format!("validate {mode:?}"),
            block_slash_msg(&r),
            format!("{r:?}"),
        ));
    }
    for mode in MODES {
        let before = tip(node).await;
        let r = node.apply_block(block.clone(), mode).await;
        assert_eq!(
            tip(node).await,
            before,
            "{case}: apply {mode:?} moved the tip"
        );
        let raw = format!("{:?}", r.as_ref().err().map(|e| format!("{e:#}")));
        seen.push((format!("apply {mode:?}"), apply_slash_msg(&r), raw));
    }
    for (path, msg, raw) in &seen {
        assert!(
            msg.is_some(),
            "{case}: {path} must refuse with InvalidSlash; got {raw}"
        );
    }
    let first = seen[0].1.clone().unwrap_or_default();
    for (path, msg, _) in &seen {
        assert_eq!(
            msg.as_deref(),
            Some(first.as_str()),
            "{case}: {path} message differs from the first path's"
        );
    }
    first
}
