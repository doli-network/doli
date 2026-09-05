//! INC-I-178 M0 — shared harness for the attestation characterization locks.
//!
//! OUTPUT CONTRACT: N/A — fixture file. It declares no `#[test]`; the
//! enumerations live with the functions under test in the sibling
//! `inc_i_178_m0_*` modules. INPUT PARTITIONS: N/A — fixture file.
//!
//! `build_via_production` drives the REAL builder (`assembly.rs`), so every
//! bitfield under test is the one a node would gossip, not a hand-rolled copy.
//! Consumers: `inc_i_178_m0_attestation_lock`, `inc_i_178_m0_block_identity`,
//! `inc_i_178_m1_ingest`, `inc_i_178_m2_ingress`.

#![allow(dead_code)] // each consumer uses a subset

use std::collections::HashSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crypto::{BlsKeyPair, Hash, KeyPair, PublicKey};
use doli_core::transaction::Transaction;
use doli_core::{Attestation, Block, BlockHeader};
use doli_node::node::Node;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

/// What `build_block_content` hands back: header, finalized tx list, body bitfield.
pub type Built = (BlockHeader, Vec<Transaction>, Vec<u8>);

pub const N_REALISTIC: usize = 45;
pub const N_LARGE: usize = 200;
pub const N_SMALL: usize = 12;

pub async fn make_node(n_producers: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    node.params.blocks_per_era = 100_000;
    (node, producers, temp)
}

/// A height that is past genesis and is NOT a reward-epoch start.
/// Derived from the shipped params, never a literal (see commit `18779b1e`).
pub fn safe_build_height(node: &Node) -> u64 {
    let bpe = node.config.network.blocks_per_reward_epoch();
    let mut h = node.config.network.genesis_blocks() + bpe + 1;
    while doli_core::consensus::reward_epoch::is_epoch_start_with(h, bpe)
        || node.config.network.is_in_genesis(h)
    {
        h += 1;
    }
    h
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Block until the wall-clock second has just ticked, so the caller has a full
/// slot of headroom before `assembly.rs:376` aborts on a slot boundary.
pub fn wait_for_fresh_second() -> u64 {
    let start = unix_now();
    loop {
        let now = unix_now();
        if now > start {
            return now;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// The universe the CURRENT encoder builds: `[base | (active \ base) sorted by
/// pubkey bytes]` (`assembly.rs:408-424`). Local to these tests on purpose —
/// M3 extracts the production version; M0 only locks what it must reproduce.
pub fn test_universe(base: &[PublicKey], active: &[PublicKey]) -> Vec<PublicKey> {
    let base_set: HashSet<&PublicKey> = base.iter().collect();
    let mut extra: Vec<PublicKey> = active
        .iter()
        .filter(|pk| !base_set.contains(pk))
        .copied()
        .collect();
    extra.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    let mut universe = base.to_vec();
    universe.extend(extra);
    universe
}

pub async fn active_at(node: &Node, height: u64) -> Vec<PublicKey> {
    let producers = node.producer_set.read().await;
    producers
        .active_producers_at_height(height)
        .iter()
        .map(|p| p.public_key)
        .collect()
}

/// Drive the REAL builder. Retries on a slot-boundary abort (devnet
/// `slot_duration` is 1 s, so the window at `assembly.rs:376` is one second).
pub async fn build_via_production(
    node: &mut Node,
    height: u64,
) -> (BlockHeader, Vec<Transaction>, Vec<u8>) {
    let prev_hash = node.chain_state.read().await.best_hash;
    let our_pubkey = *node.producer_key.as_ref().unwrap().public_key();
    for _ in 0..8 {
        let now = wait_for_fresh_second();
        let current_slot = node.params.timestamp_to_slot(now);
        let built = node
            .build_block_content(
                prev_hash,
                current_slot - 1,
                height,
                current_slot,
                our_pubkey,
            )
            .await
            .expect("build_block_content returned Err");
        if let Some(parts) = built {
            return parts;
        }
    }
    panic!("build_block_content returned None on every attempt (slot boundary)");
}

/// Two builds from identical inputs INSIDE ONE SLOT. `SystemTime::now()` at
/// `assembly.rs:371` is the builder's only unpinnable input; co-locating both
/// calls in one second makes it take the same value, so the comparison is exact
/// rather than "modulo the clock". Retries if the second ticks between them.
pub async fn build_pair_in_one_slot(node: &mut Node, height: u64) -> (Built, Built) {
    let prev_hash = node.chain_state.read().await.best_hash;
    let our_pubkey = *node.producer_key.as_ref().unwrap().public_key();
    for _ in 0..8 {
        let now = wait_for_fresh_second();
        let slot = node.params.timestamp_to_slot(now);
        let a = node
            .build_block_content(prev_hash, slot - 1, height, slot, our_pubkey)
            .await
            .expect("build_block_content returned Err");
        let b = node
            .build_block_content(prev_hash, slot - 1, height, slot, our_pubkey)
            .await
            .expect("build_block_content returned Err");
        if let (Some(a), Some(b)) = (a, b) {
            if a.0.timestamp == b.0.timestamp && a.0.slot == b.0.slot {
                return (a, b);
            }
        }
    }
    panic!("could not obtain two builds inside one slot after 8 attempts");
}

pub fn assemble(header: BlockHeader, txs: Vec<Transaction>, bf: Vec<u8>) -> Block {
    Block {
        header,
        transactions: txs,
        aggregate_bls_signature: Vec::new(),
        attestation_bitfield: bf,
    }
}

pub fn err_text(r: &Result<(), doli_core::validation::ValidationError>) -> String {
    match r {
        Ok(()) => String::new(),
        Err(e) => e.to_string(),
    }
}

/// Seed the minute tracker for the attestation minute the given slot falls in
/// and the next seven, so seconds elapsing between setup and build cannot empty
/// the attester set the encoder reads at `assembly.rs:389`.
pub fn record_attesters(node: &mut Node, slot: u32, attesters: &[PublicKey]) {
    let first = doli_core::attestation::attestation_minute(slot);
    for m in 0..8u32 {
        for pk in attesters {
            node.minute_tracker.record(*pk, first + m);
        }
    }
}

pub fn sample_header(fork_id: Hash) -> BlockHeader {
    BlockHeader {
        version: 2,
        prev_hash: crypto::hash::hash(b"m0-prev"),
        merkle_root: crypto::hash::hash(b"m0-merkle"),
        presence_root: crypto::hash::hash(b"m0-presence"),
        genesis_hash: doli_core::chainspec::ChainSpec::devnet().genesis_hash(),
        timestamp: 1_700_000_000,
        slot: 42,
        producer: *KeyPair::generate().public_key(),
        vdf_output: VdfOutput {
            value: vec![7u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: crypto::hash::hash(b"m0-data"),
        fork_id,
    }
}

/// Publish `bls` as the attester's ON-CHAIN key, the way registration would.
/// Every ingress test that needs the `Valid` verdict goes through here.
pub async fn register_bls(node: &Node, pk: &PublicKey, bls: &BlsKeyPair) {
    let mut ps = node.producer_set.write().await;
    ps.get_by_pubkey_mut(pk)
        .expect("attester must be a ProducerSet member")
        .bls_pubkey = bls.public_key().as_bytes().to_vec();
}

/// One attestation signed on BOTH halves: Ed25519 over the envelope, BLS over
/// `bls_attest_msg(&hash)` — the preimage the ingress rebuilds to verify.
pub fn dual(kp: &KeyPair, bls: &BlsKeyPair, hash: Hash, slot: u32, height: u64) -> Attestation {
    Attestation::new_with_bls(
        hash,
        slot,
        height,
        1,
        kp.private_key(),
        *kp.public_key(),
        bls,
    )
    .expect("dual signing must succeed")
}
