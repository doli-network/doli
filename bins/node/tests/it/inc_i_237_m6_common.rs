//! INC-I-237 M6 — node fixtures: a pinned node slot clock, synthetic known
//! blocks, BLS-registered members and height-mutated attestation copies.
//!
//! OUTPUT CONTRACT: N/A — fixture file (helpers only, asserts nothing).
//! INPUT PARTITIONS: N/A — fixture file.
#![allow(dead_code)] // each consumer uses a subset

use std::collections::HashMap;

use crypto::{BlsKeyPair, Hash, KeyPair, PublicKey};
use doli_core::Attestation;
use doli_node::node::Node;
use network::service::Verdict;
use network::PeerId;

use super::inc_i_178_m0_common::{assemble, register_bls, sample_header};
use super::inc_i_237_m3_node_common::now_secs;

pub const SLOT_SECS: u64 = 600;
pub const WALL: u32 = 1_000;

/// Pin the node's slot clock (`node.params`) so `timestamp_to_slot(now) == wall`.
pub fn set_wall(node: &mut Node, wall: u32) {
    node.params.slot_duration = SLOT_SECS;
    node.params.genesis_time = now_secs() - u64::from(wall) * SLOT_SECS - 1;
}

pub fn wall_slot(node: &Node) -> u32 {
    node.params.timestamp_to_slot(now_secs())
}

/// Store a distinct block canonically at `height`; the node then knows its hash.
pub fn store_block(node: &Node, slot: u32, height: u64, tag: u32) -> Hash {
    let mut header = sample_header(Hash::ZERO);
    header.slot = slot;
    header.merkle_root = crypto::hash::hash(&tag.to_le_bytes());
    let block = assemble(header, Vec::new(), Vec::new());
    node.block_store
        .put_block_canonical(&block, height)
        .expect("put_block_canonical");
    block.hash()
}

pub fn is_known(node: &Node, hash: &Hash) -> bool {
    matches!(node.block_store.get_height_by_hash(hash), Ok(Some(_)))
}

/// A member key with its BLS key published on-chain.
pub async fn bls_member(node: &Node, kp: &KeyPair) -> BlsKeyPair {
    let bls = BlsKeyPair::generate();
    register_bls(node, kp.public_key(), &bls).await;
    bls
}

/// Same signed fields; new `height` (a new gossip id) and the given BLS bytes.
pub fn variant(a: &Attestation, height: u64, bls: Vec<u8>) -> Attestation {
    let mut m = a.clone();
    m.height = height;
    m.bls_signature = bls;
    m
}

/// Accept count per (attester, slot) over every verdict the node returned.
#[derive(Default)]
pub struct AcceptLog(HashMap<(PublicKey, u32), usize>);

impl AcceptLog {
    pub async fn feed(&mut self, node: &mut Node, att: &Attestation) -> Verdict {
        let v = node
            .on_new_attestation(att.to_bytes(), PeerId::random())
            .await;
        let n = self.0.entry((att.attester, att.slot)).or_default();
        if v == Verdict::Accept {
            *n += 1;
        }
        v
    }

    pub fn max(&self) -> usize {
        self.0.values().copied().max().unwrap_or(0)
    }

    pub fn get(&self, attester: &PublicKey, slot: u32) -> usize {
        self.0.get(&(*attester, slot)).copied().unwrap_or(0)
    }
}
