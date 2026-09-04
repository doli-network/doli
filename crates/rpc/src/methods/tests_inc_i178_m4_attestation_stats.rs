//! INC-I-178 M4 — REQ-BLS-004: `getAttestationStats` must not count a zero-pooled
//! post-AH block as a block that carries attestations.
//!
//! The scan gates on `presence_root != Hash::ZERO`. Above the activation height a block
//! with no pooled signatures carries `presence_commitment([], [])` — a real hash — so it
//! passes that gate and inflates `blocksWithAttestations` while crediting nobody.
//!
//! Pre-AH a `presence_root` is `BLAKE3(bitfield)`, so matching the canonical-empty VALUE
//! would require a preimage collision; the recognition needs no activation height here.
//!
//! OUTPUT CONTRACT
//! F1: `RpcContext::get_attestation_stats(&self) -> Result<Value, RpcError>`
//!   O1 `blocksWithAttestations`
//!   O2 `blocksWithBls`
//!   O3 `epochStart` / `currentHeight` — the scan window
//!   O4 mutable params / O5 store writes / O6 statics / O7 channels — NONE
//!   PATHS: PA root == Hash::ZERO | PB root == canonical empty | PC ordinary root
//!   INPUT PARTITIONS: one epoch window holding exactly one block of each path.
//!   MATRIX: O1 discriminates all three paths in one call; O2/O3 pin the window;
//!     O4-O7 are structural (the method takes `&self` and only reads).

use std::sync::Arc;

use super::*;
use doli_core::network::Network;
use doli_core::{presence_commitment, Block, BlockHeader};
use storage::{BlockStore, ChainState, UtxoSet};
use tempfile::TempDir;
use tokio::sync::RwLock;
use vdf::{VdfOutput, VdfProof};

fn block_with(height: u64, root: crypto::Hash) -> Block {
    let header = BlockHeader {
        version: 2,
        prev_hash: crypto::hash::hash(&height.to_le_bytes()),
        merkle_root: crypto::Hash::ZERO,
        presence_root: root,
        genesis_hash: crypto::Hash::ZERO,
        timestamp: 1_700_000_000 + height,
        slot: height as u32,
        producer: *crypto::KeyPair::from_seed([9u8; 32]).public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: crypto::Hash::ZERO,
        fork_id: crypto::Hash::ZERO,
    };
    Block::new(header, Vec::new())
}

/// One epoch window holding heights 1, 2 and 3 — the sentinel, the canonical empty and
/// an ordinary attested root, in that order.
async fn harness() -> (RpcContext, TempDir) {
    let tempdir = TempDir::new().expect("tempdir");
    let block_store = Arc::new(BlockStore::open(tempdir.path()).expect("blockstore"));

    let ordinary_root = crypto::hash::hash(&[0b1010_1101u8, 0x41]);
    assert_ne!(
        ordinary_root,
        presence_commitment(&[], &[]),
        "fixture: the ordinary root must not be the canonical empty"
    );
    for (h, root) in [
        (1u64, crypto::Hash::ZERO),
        (2u64, presence_commitment(&[], &[])),
        (3u64, ordinary_root),
    ] {
        block_store
            .put_block_canonical(&block_with(h, root), h)
            .expect("put_block_canonical");
    }

    let mut cs = ChainState::new(crypto::Hash::ZERO);
    cs.best_height = 3;
    let chain_state = Arc::new(RwLock::new(cs));
    let utxo_set = Arc::new(RwLock::new(UtxoSet::new()));
    let params = doli_core::consensus::ConsensusParams::default();
    let mempool = Arc::new(RwLock::new(mempool::Mempool::new(
        mempool::MempoolPolicy::default(),
        params.clone(),
        Network::Mainnet,
    )));

    let ctx = RpcContext::new_for_network(
        chain_state,
        block_store,
        utxo_set,
        mempool,
        params,
        Network::Mainnet,
    )
    // Keep the whole fixture inside epoch 0 so the scan covers heights 1..=3.
    .with_blocks_per_reward_epoch(8);

    (ctx, tempdir)
}

/// REQ-BLS-004 — Decision: a failure means `getAttestationStats` reports a silent
/// post-AH epoch as an attesting one. That RPC is what the monitoring stack and the
/// explorer read to decide whether attestation is alive, so an attestation outage would
/// register as full health for as long as producers keep building empty commitments.
#[tokio::test]
async fn req_bls_004_m4_the_canonical_empty_is_not_a_block_with_attestations() {
    let (ctx, _tempdir) = harness().await;
    let out = ctx
        .get_attestation_stats()
        .await
        .expect("getAttestationStats");

    // The scan runs `epoch_start.max(1)..=current_height`, so the fixture heights
    // 1..=3 are exactly the window.
    assert_eq!(out["epochStart"].as_u64(), Some(0), "fixture: window start");
    assert_eq!(
        out["currentHeight"].as_u64(),
        Some(3),
        "fixture: window end"
    );
    assert_eq!(
        out["blocksWithAttestations"].as_u64(),
        Some(1),
        "only the ordinary root carries attestations: Hash::ZERO is the legacy \
         sentinel and the canonical empty is the post-AH spelling of the same thing"
    );
    assert_eq!(
        out["blocksWithBls"].as_u64(),
        Some(0),
        "O2: no fixture block carries an aggregate"
    );
}
