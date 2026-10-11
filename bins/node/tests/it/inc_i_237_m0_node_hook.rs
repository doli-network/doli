//! INC-I-237 M0 (C6) — `Node::handle_network_event` observes every gossip event
//! on the success AND the early-return / error paths, using the event's handle.
//!
//! OUTPUT CONTRACT: async fn Node::handle_network_event(&mut self, event) -> Result<()>
//!   O1: none   O2: node state (not asserted: M0 must not change it)
//!   O3: Result — any value; the hook runs after the match on Ok and Err
//!   O4: none
//!   O5: received counter{topic}, verdict histogram{topic}, block-recv histogram
//!       (deltas under `common::lock()`)
//!   O6: none
//! PATHS: P1 handler returns Ok normally (empty ProducersAnnounced), P2 handler returns early
//!   (NewVote with garbage JSON), P3 block path (orphan NewBlock: early drop, Ok
//!   or Err), P4 control event (no gossip metric moves)
//! INPUT PARTITIONS: producers topic with a fresh handle, tracked topic with a 1.2 s old
//!   handle (votes), NewBlock with a 1.0 s old handle and a header 3 s old
//! MATRIX: O5 x P1-P4 — all asserted.

use std::time::{Duration, Instant};

use super::inc_i_237_m0_common::{self as c, block_recv, received, verdict};
use crypto::{Hash, KeyPair};
use doli_core::{Block, BlockHeader};
use doli_node::node::Node;
use network::{GossipHandle, NetworkEvent, PeerId};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

const TOPICS: [&str; 5] = ["blocks", "txs", "attestations", "votes", "producers"];

async fn node() -> (Node, TempDir) {
    let temp = TempDir::new().expect("tempdir");
    let node = Node::new_for_test(temp.path().to_path_buf(), vec![KeyPair::generate()])
        .await
        .expect("Node::new_for_test");
    (node, temp)
}

fn aged(ms: u64) -> GossipHandle {
    GossipHandle {
        recv_at: Instant::now()
            .checked_sub(Duration::from_millis(ms))
            .unwrap(),
    }
}

fn orphan_block(ts: u64) -> Block {
    let header = BlockHeader {
        version: 2,
        prev_hash: Hash::from_bytes([0x37u8; 32]),
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: ts,
        slot: 7,
        producer: *KeyPair::generate().public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, vec![])
}

async fn handle(node: &mut Node, ev: NetworkEvent) {
    let r = tokio::time::timeout(Duration::from_secs(30), node.handle_network_event(ev)).await;
    assert!(r.is_ok(), "handle_network_event hung");
}

fn all_received() -> Vec<f64> {
    TOPICS.iter().map(|t| received(t)).collect()
}

// REQ-GRV-008 — Decision: a failure means the node hook misses a handler path (early return, `?` error) or stamps at the node instead of reading the swarm's recv_at, so verdict latency under-reports exactly the slow and failing paths.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::await_holding_lock)] // serialises the process-global gossip metrics
async fn req_grv_008_handle_network_event_observes_gossip_on_every_path() {
    let (mut node, _tmp) = node().await;
    let _g = c::lock();

    // P1: handler returns Ok normally.
    let (p0, ph0) = (received("producers"), verdict("producers"));
    handle(
        &mut node,
        NetworkEvent::ProducersAnnounced(vec![], GossipHandle::now()),
    )
    .await;
    assert_eq!(received("producers") - p0, 1.0, "producers event counted");
    assert_eq!(
        verdict("producers").count - ph0.count,
        1,
        "producers event timed on the normal-return path"
    );

    // P2: tracked topic, garbage vote -> handler returns early.
    let (v0, vh0) = (received("votes"), verdict("votes"));
    handle(
        &mut node,
        NetworkEvent::NewVote(b"\xde\xad not json".to_vec(), aged(1_200)),
    )
    .await;
    let vh1 = verdict("votes");
    assert_eq!(
        received("votes") - v0,
        1.0,
        "vote counted on the early-return path"
    );
    assert_eq!(
        vh1.count - vh0.count,
        1,
        "vote timed on the early-return path"
    );
    let dt = vh1.sum - vh0.sum;
    assert!(
        (1.2..10.0).contains(&dt),
        "latency {dt}s must come from the event's recv_at (1.2 s old)"
    );

    // P3: block path, orphan block (Ok or Err, both observed).
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let (b0, bh0, br0) = (received("blocks"), verdict("blocks"), block_recv());
    let ev = NetworkEvent::NewBlock(orphan_block(now_secs - 3), PeerId::random(), aged(1_000));
    let r = tokio::time::timeout(Duration::from_secs(30), node.handle_network_event(ev)).await;
    eprintln!(
        "orphan NewBlock result: {:?}",
        r.as_ref().map(|x| x.as_ref().map_err(|e| e.to_string()))
    );
    assert!(r.is_ok(), "block path hung");
    let (bh1, br1) = (verdict("blocks"), block_recv());
    assert_eq!(received("blocks") - b0, 1.0, "block counted");
    assert_eq!(bh1.count - bh0.count, 1, "block verdict timed");
    assert!(
        bh1.sum - bh0.sum >= 1.0,
        "block verdict latency from recv_at (1.0 s old)"
    );
    assert_eq!(br1.count - br0.count, 1, "block recv latency observed");
    let recv_delay = br1.sum - br0.sum;
    assert!(
        (1.0..8.0).contains(&recv_delay),
        "block recv latency {recv_delay}s = (wall now - handle age) - header ts, ~2 s"
    );

    // P4: control event moves no gossip metric.
    let before = all_received();
    handle(&mut node, NetworkEvent::PeerDisconnected(PeerId::random())).await;
    assert_eq!(
        all_received(),
        before,
        "a control event must not be counted as gossip"
    );
}
