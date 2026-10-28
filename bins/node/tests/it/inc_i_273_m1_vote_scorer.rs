// OUTPUT CONTRACT: fn on_new_vote(...)
//   on_new_vote(&mut self, vote_data, source: Option<PeerId>) -> Verdict, called directly and
//   through Node::handle_network_event(NewVote(bytes, handle)) with handle.source() as source;
//   plus GossipHandle::source(&self) -> Option<PeerId>.
//   O1: none (bytes moved in)
//   O2: self.vote_ingress_scorer: MalformedMessage records per source peer (record-only)
//   O3: Verdict: Ignore for every malformed vote; Accept for an honest active vote;
//       source(): Some(peer) when stamped, None when unstamped
//   O4: none   O5: none
//   O6: vote_tx: a malformed vote is never forwarded
// PATHS: P0 GossipHandle::source(); P1 stamped malformed voter id -> Ignore, one record for
//   the source peer; P2 stamped malformed signature (active id) -> as P1; P3 unstamped
//   malformed vote -> Ignore, no record, no panic, the next honest vote is still forwarded;
//   P4 well-formed vote (honest, forged signature, non-member) -> no MalformedMessage
// INPUT PARTITIONS: voter id {the 12 partitions of inc_i_273_m1_vote_input::malformed_ids};
//   signature {non-hex, 126 hex, multi-byte}; handle {stamped, unstamped};
//   call {event path, direct}; well-formed {active + valid, active + forged, non-member + valid}
// MATRIX: O3 x P0 (2); (O2, O3, O6) x P1 (12 x 2 calls) + P2 (3 x 2 calls);
//   (O2, O3, O6) x P3 (2); (O2, O3) x P4 (3, O6 for honest and forged) — asserted below.
// covers: gossip_verdicts event_loop gossip_handle mod init vote_ingress

use std::time::Duration;

use crypto::KeyPair;
use doli_node::node::Node;
use network::gossip::staleness::GossipTopic;
use network::{GossipHandle, Infraction, NetworkEvent, PeerId, Verdict};
use tokio::sync::mpsc::Receiver;
use updater::VoteMessage;

use super::inc_i_237_m3_node_common as c;
use super::inc_i_273_m1_vote_input as v;

pub fn stamped(peer: PeerId) -> GossipHandle {
    GossipHandle::new(GossipTopic::Votes, vec![0x73u8; 20].into(), peer)
}

pub fn malformed(node: &Node, peer: &PeerId) -> usize {
    node.vote_ingress_scorer.get_score(peer).map_or(0, |s| {
        s.infractions
            .iter()
            .filter(|(_, i)| matches!(i, Infraction::MalformedMessage))
            .count()
    })
}

pub async fn node() -> (Node, KeyPair, Receiver<VoteMessage>, tempfile::TempDir) {
    let (mut node, member, t) = c::member_node().await;
    let (tx, rx) = tokio::sync::mpsc::channel(1_000);
    node.set_vote_tx(tx);
    (node, member, rx, t)
}

pub async fn event(node: &mut Node, bytes: Vec<u8>, handle: GossipHandle) {
    let ev = NetworkEvent::NewVote(bytes, handle);
    let r = tokio::time::timeout(Duration::from_secs(30), node.handle_network_event(ev)).await;
    assert!(r.is_ok(), "vote handling hung");
}

/// Each malformed vote via the event path (peer A) and a direct call (peer B).
async fn assert_recorded(
    node: &mut Node,
    rx: &mut Receiver<VoteMessage>,
    cases: Vec<(&str, Vec<u8>)>,
    ts: u64,
) {
    for (case, bytes) in cases {
        let (via_event, direct) = (PeerId::random(), PeerId::random());
        event(node, bytes.clone(), stamped(via_event)).await;
        assert_eq!(
            malformed(node, &via_event),
            1,
            "{case}: the event path must record the handle's source peer once"
        );
        assert_eq!(
            node.on_new_vote(bytes, Some(direct)).await,
            Verdict::Ignore,
            "{case}: a malformed vote must be Ignored, never Accepted"
        );
        assert_eq!(
            malformed(node, &direct),
            1,
            "{case}: direct call must record the source peer once"
        );
        assert_eq!(
            v::forwarded(rx, ts),
            0,
            "{case}: malformed vote reached the updater"
        );
    }
}

// REQ-VOTE-002 — Decision: a failure means the event loop cannot name the peer that sent a vote, so a malformed vote is charged to nobody or to the wrong peer.
#[test]
fn gossip_handle_source_is_the_stamped_peer_and_none_when_unstamped() {
    let peer = PeerId::random();
    assert_eq!(stamped(peer).source(), Some(peer));
    assert_eq!(GossipHandle::now().source(), None);
}

// REQ-VOTE-002 — Decision: a failure means a peer that sends a voter id which is not a public key is not recorded on the record-only scorer, or the vote is Accepted or forwarded.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stamped_malformed_voter_id_is_ignored_and_recorded_once_for_the_source_peer() {
    let (mut node, member, mut rx, _t) = node().await;
    let ts = c::now_secs() - 7;
    let cases = v::malformed_ids(&member)
        .into_iter()
        .map(|(case, id)| (case, v::with_id(&member, &id, ts)))
        .collect();
    assert_recorded(&mut node, &mut rx, cases, ts).await;
}

// REQ-VOTE-002 — Decision: a failure means a peer that sends a signature which is not hex of the signature length is not recorded, or the vote is Accepted or forwarded.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stamped_malformed_signature_is_ignored_and_recorded_once_for_the_source_peer() {
    let (mut node, member, mut rx, _t) = node().await;
    let ts = c::now_secs() - 7;
    let cases = v::malformed_sigs(&member, ts)
        .into_iter()
        .map(|(case, sig)| (case, v::with_sig(&member, &sig, ts)))
        .collect();
    assert_recorded(&mut node, &mut rx, cases, ts).await;
}

// REQ-VOTE-002 — Decision: a failure means a malformed vote on an unstamped handle invents a peer record or is Accepted, or later honest votes are not served.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unstamped_malformed_vote_is_ignored_records_nothing_and_node_keeps_serving() {
    let (mut node, member, mut rx, _t) = node().await;
    let ts = c::now_secs() - 7;
    let cases = [
        (
            "shape a voter id",
            v::with_id(&member, &v::id_case(&member, "shape a"), ts),
        ),
        (
            "non-hex signature",
            v::with_sig(&member, &v::sig_case(&member, ts, "non-hex"), ts),
        ),
    ];
    for (case, bytes) in cases {
        assert_eq!(
            node.on_new_vote(bytes.clone(), None).await,
            Verdict::Ignore,
            "{case}"
        );
        event(&mut node, bytes, GossipHandle::now()).await;
        assert_eq!(
            node.vote_ingress_scorer.stats().peer_count,
            0,
            "{case}: an unstamped vote has no peer to record"
        );
        assert_eq!(v::forwarded(&mut rx, ts), 0, "{case}: reached the updater");
    }
    event(
        &mut node,
        c::vote(&member, ts + 6, true),
        GossipHandle::now(),
    )
    .await;
    assert_eq!(
        v::forwarded(&mut rx, ts + 6),
        1,
        "the honest vote after the drops must be forwarded"
    );
}

// REQ-VOTE-002 — Decision: a failure means a well-formed vote (honest, forged signature, or non-member) is charged as malformed, so honest relays accumulate a penalty they did not earn.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn well_formed_votes_record_no_malformed_whatever_their_verdict() {
    let (mut node, member, mut rx, _t) = node().await;
    let t0 = c::now_secs();
    let rows = [
        (
            "honest active producer",
            c::vote(&member, t0 - 1, true),
            t0 - 1,
            Verdict::Accept,
        ),
        (
            "forged signature",
            c::vote(&member, t0 - 2, false),
            t0 - 2,
            Verdict::Ignore,
        ),
        (
            "non-member",
            c::vote(&KeyPair::generate(), t0 - 3, true),
            t0 - 3,
            Verdict::Ignore,
        ),
    ];
    for (case, bytes, ts, want) in rows {
        let (via_event, direct) = (PeerId::random(), PeerId::random());
        event(&mut node, bytes.clone(), stamped(via_event)).await;
        assert_eq!(
            node.on_new_vote(bytes, Some(direct)).await,
            want,
            "{case}: verdict"
        );
        assert_eq!(
            malformed(&node, &via_event),
            0,
            "{case}: event path recorded malformed"
        );
        assert_eq!(
            malformed(&node, &direct),
            0,
            "{case}: direct call recorded malformed"
        );
        let sent = v::forwarded(&mut rx, ts);
        match case {
            "honest active producer" => assert!(sent >= 1, "{case}: must reach the updater"),
            "forged signature" => assert_eq!(sent, 0, "{case}: reached the updater"),
            _ => {}
        }
    }
}
