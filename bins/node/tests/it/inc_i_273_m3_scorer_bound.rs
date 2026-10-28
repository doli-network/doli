// OUTPUT CONTRACT: fn on_new_vote(...)
//   driven through Node::handle_network_event(NewVote(bytes, stamped handle)), one malformed
//   vote (voter id not a public key) per distinct source peer.
//   O1: none (bytes moved in)
//   O2: self.vote_ingress_scorer: tracked peer count and per-peer MalformedMessage records
//   O3: Verdict (asserted per case in inc_i_273_m1_vote_scorer.rs)
//   O4: none   O5: none
//   O6: vote_tx (a malformed vote is never forwarded; asserted in inc_i_273_m1_vote_scorer.rs)
// PATHS: P1 up to 1024 distinct peers -> every peer recorded once, count == peers;
//   P2 past 1024 distinct peers -> count stays <= 1024, the newest peer is still recorded
// INPUT PARTITIONS: distinct peers {1024 (at the bound), 2048 (twice the bound)}
// MATRIX: O2 x {P1, P2}
// covers: vote_ingress gossip_verdicts

use crypto::KeyPair;
use doli_node::node::Node;
use network::PeerId;

use super::inc_i_237_m3_node_common as c;
use super::inc_i_273_m1_vote_input as v;
use super::inc_i_273_m1_vote_scorer as s;

const BOUND: usize = 1024;

/// One malformed vote from each of `peers` fresh peers; returns them in send order.
async fn flood(node: &mut Node, member: &KeyPair, peers: usize) -> Vec<PeerId> {
    let bytes = v::with_id(member, &v::id_case(member, "64 non-hex"), c::now_secs() - 7);
    let mut sent = Vec::with_capacity(peers);
    for _ in 0..peers {
        let p = PeerId::random();
        s::event(node, bytes.clone(), s::stamped(p)).await;
        sent.push(p);
    }
    sent
}

// REQ-VOTE-002 — Decision: a failure means the cap resets the scorer early, so peers below the bound lose their records.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scorer_records_every_peer_up_to_the_bound() {
    let (mut node, member, _rx, _t) = s::node().await;
    let peers = flood(&mut node, &member, BOUND).await;
    assert_eq!(
        node.vote_ingress_scorer.stats().peer_count,
        BOUND,
        "every peer up to the bound must be tracked"
    );
    for (i, p) in peers.iter().enumerate() {
        assert_eq!(s::malformed(&node, p), 1, "peer #{i}: one malformed record");
    }
}

// REQ-VOTE-002 — Decision: a failure means the record-only vote scorer keeps one entry per distinct peer with no cap, or the cap drops the newest peer's record.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scorer_stays_bounded_past_twice_the_bound_and_records_the_newest_peer() {
    let (mut node, member, _rx, _t) = s::node().await;
    let peers = flood(&mut node, &member, 2 * BOUND).await;
    let n = node.vote_ingress_scorer.stats().peer_count;
    println!("M3-PROBE scorer_peers_after_2048={n}");
    assert!(n <= BOUND, "scorer tracks {n} peers, bound is {BOUND}");
    let last = peers.last().expect("setup: peers sent");
    assert_eq!(
        s::malformed(&node, last),
        1,
        "the newest peer must still be recorded"
    );
}
