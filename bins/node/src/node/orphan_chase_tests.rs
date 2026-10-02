//! INC-I-235 M2 — ORPHAN_CHASE self-loop guard (REQ-I235-011) and the
//! Stability Pillar 1 lock (first orphan from a sender is still chased).
//!
//! SEAM (developer adds): `Node::orphan_chase_requests: u64` counts every
//! ORPHAN_CHASE `GetBlockByHeight` the node decides to send, counted at the
//! decision, independent of `self.network` being `Some`; and
//! `block_handling::ORPHAN_CHASE_REPEAT_WINDOW: Duration` (pub(super)).
//!
//! OUTPUT CONTRACT
//!   fn Node::handle_new_block(&mut self, Block, PeerId) -> Result<()>  (Orphan arm)
//!     O1 receiver: orphan_chase_requests (request decision)   O2 fork_block_cache
//!   PATHS: first (peer, height) -> chase; repeat within window -> no chase;
//!          other peer / other height -> chase; repeat after window -> chase.
//!
//! INPUT PARTITIONS
//!   P1 first arrival   P2 same orphan, same peer, 200x in < 1 s
//!   P3 same height, different peer   P4 tip moved (new need_height)   P5 window elapsed

use super::*;
use tempfile::TempDir;

async fn node_at_h5() -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..3).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    let mut prev = Hash::ZERO;
    for h in 1..=5u64 {
        let b = block(&node.params, h, h as u32, prev, &producers[0]);
        prev = b.hash();
        node.apply_block(b, ValidationMode::Light).await.unwrap();
    }
    (node, producers, temp)
}

fn block(params: &ConsensusParams, h: u64, slot: u32, prev: Hash, by: &KeyPair) -> Block {
    let cb = Transaction::new_coinbase(
        params.block_reward(h),
        doli_core::consensus::reward_pool_pubkey_hash(),
        h,
        0,
    );
    let header = BlockHeader {
        version: 2,
        prev_hash: prev,
        merkle_root: doli_core::block::compute_merkle_root(std::slice::from_ref(&cb)),
        presence_root: Hash::ZERO,
        genesis_hash: doli_core::chainspec::ChainSpec::devnet().genesis_hash(),
        timestamp: params.genesis_time + slot as u64 * params.slot_duration,
        slot,
        producer: *by.public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, vec![cb])
}

fn orphan(node: &Node, p: &[KeyPair], tag: &str, slot: u32) -> Block {
    let parent = crypto_hash(tag.as_bytes());
    block(&node.params, 7, slot, parent, &p[1])
}

// REQ-I235-011 — Decision: a FAIL means a by-height chase whose answer is the same
// orphan re-requests the same height from the same peer on every arrival (the
// 47 req/s self-loop seen on seed2). Bound: <= 1 per (peer, height) per window.
// Cell: O1 x P2.
#[tokio::test]
async fn m2_repro_chase_same_orphan_same_peer_is_bounded() {
    let (mut node, p, _tmp) = node_at_h5().await;
    let peer = PeerId::random();
    let o = orphan(&node, &p, "loop", 9);
    for _ in 0..200 {
        node.handle_new_block(o.clone(), peer).await.unwrap();
    }
    assert!(
        node.orphan_chase_requests <= 1,
        "REQ-I235-011: {} chase requests for one (peer, height) in < 1 s",
        node.orphan_chase_requests
    );
}

// REQ-I235-011 (Stability Pillar 1 lock) — Decision: a FAIL means the guard
// suppresses the first chase of a new orphan from its sender, which re-opens
// the pre-v6.16.1 stall the Orphan Chase pillar fixed. Cell: O1/O2 x P1.
#[tokio::test]
async fn m2_repro_chase_lock_first_arrival_issues_request() {
    let (mut node, p, _tmp) = node_at_h5().await;
    let o = orphan(&node, &p, "first", 9);
    node.handle_new_block(o.clone(), PeerId::random())
        .await
        .unwrap();
    assert_eq!(
        node.orphan_chase_requests, 1,
        "Pillar 1: first orphan must be chased"
    );
    assert!(node.fork_block_cache.read().await.contains_key(&o.hash()));
}

// REQ-I235-011 (Stability Pillar 1 lock) — Decision: a FAIL means the guard is
// keyed too coarsely (height only, or global), so a second sender or a moved tip
// is never chased. Cell: O1 x P3, P4.
#[tokio::test]
async fn m2_repro_chase_lock_other_peer_and_new_height_still_chased() {
    let (mut node, p, _tmp) = node_at_h5().await;
    let (p1, p2) = (PeerId::random(), PeerId::random());
    let o = orphan(&node, &p, "multi", 9);
    node.handle_new_block(o.clone(), p1).await.unwrap();
    node.handle_new_block(o.clone(), p2).await.unwrap();
    assert_eq!(node.orphan_chase_requests, 2, "a second sender is chased");

    let tip = node.chain_state.read().await.best_hash;
    let h6 = block(&node.params, 6, 6, tip, &p[0]);
    node.apply_block(h6, ValidationMode::Light).await.unwrap();
    node.handle_new_block(o, p1).await.unwrap();
    assert_eq!(
        node.orphan_chase_requests, 3,
        "a moved tip (new need_height) is chased again from the same peer"
    );
}

// REQ-I235-011 — Decision: a FAIL means a suppressed (peer, height) is never
// re-chased, so a lost chase response strands the node; or the window is so long
// that retry is slower than a sync request timeout (10 s). Cell: O1 x P5.
#[tokio::test]
async fn m2_repro_chase_window_expiry_rechases() {
    let window = super::block_handling::ORPHAN_CHASE_REPEAT_WINDOW;
    assert!(
        window > Duration::ZERO && window <= Duration::from_secs(10),
        "window {:?} must be in (0, 10 s]",
        window
    );
    let (mut node, p, _tmp) = node_at_h5().await;
    let peer = PeerId::random();
    let o = orphan(&node, &p, "expire", 9);
    node.handle_new_block(o.clone(), peer).await.unwrap();
    node.handle_new_block(o.clone(), peer).await.unwrap();
    assert_eq!(node.orphan_chase_requests, 1);
    tokio::time::sleep(window + Duration::from_millis(200)).await;
    node.handle_new_block(o, peer).await.unwrap();
    assert_eq!(
        node.orphan_chase_requests, 2,
        "after the window the same (peer, height) is chased again"
    );
}
