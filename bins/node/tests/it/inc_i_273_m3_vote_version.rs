// OUTPUT CONTRACT: fn on_new_vote(...)
//   on_new_vote(&mut self, vote_data, source: Option<PeerId>) -> Verdict, called directly and
//   through Node::handle_network_event(NewVote(bytes, handle)) with a stamped handle.
//   O1: none (bytes moved in)
//   O2: self.vote_ingress_scorer: MalformedMessage records per source peer
//   O3: Verdict (direct call; the event path hands it to the swarm)
//   O4: none   O5: none
//   O6: vote_tx: the VoteMessage handed to the updater
// PATHS: P1 malformed version, valid self-signature (non-member) -> Ignore, not forwarded,
//   one record per source peer; P2 malformed version signed by the active member -> as P1;
//   P3 well-formed version from the active member -> Accept, forwarded unchanged, no record;
//   P4 well-formed version from a non-member -> Ignore, forwarded unchanged, no record
// INPUT PARTITIONS: version {65 bytes, 200,025 bytes multi-line, LF, CR, TAB, ESC, DEL, space,
//   non-ASCII letter, full-width digit, empty} on P1; {65 bytes} on P2; {6.45.1, v6.45.1,
//   6.45.1-rc.1, 6.45.1+build.7, 32-byte max triple, 64-byte graphic} on P3; {6.45.1} on P4;
//   call {event path, direct}
// MATRIX: (O2, O6) x 19 partitions x 2 calls + O3 x 19 direct calls; one test per P1/P2 case.
// covers: vote_ingress gossip_verdicts

use crypto::KeyPair;
use doli_node::node::Node;
use network::{PeerId, Verdict};
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::Receiver;
use updater::{Vote, VoteMessage};

use super::inc_i_273_m1_vote_scorer as s;

/// A vote for `version` signed by `kp` under its own voter id.
fn signed(kp: &KeyPair, version: &str) -> (VoteMessage, Vec<u8>) {
    let mut vote = VoteMessage::new(version.into(), Vote::Approve, kp.public_key().to_hex());
    vote.signature = crypto::signature::sign(&vote.message_bytes(), kp.private_key()).to_hex();
    assert!(
        vote.verify(&vote.producer_id),
        "setup: signature must verify"
    );
    let bytes = serde_json::to_vec(&vote).expect("vote json");
    (vote, bytes)
}

fn drain(rx: &mut Receiver<VoteMessage>) -> Vec<VoteMessage> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

struct Seen {
    event_peer: PeerId,
    direct_peer: PeerId,
    verdict: Verdict,
    via_event: Vec<VoteMessage>,
    via_direct: Vec<VoteMessage>,
}

/// One delivery on the event path (fresh peer), then one direct call (fresh peer).
async fn deliver(node: &mut Node, rx: &mut Receiver<VoteMessage>, bytes: &[u8]) -> Seen {
    let (event_peer, direct_peer) = (PeerId::random(), PeerId::random());
    s::event(node, bytes.to_vec(), s::stamped(event_peer)).await;
    let via_event = drain(rx);
    let verdict = node.on_new_vote(bytes.to_vec(), Some(direct_peer)).await;
    let via_direct = drain(rx);
    Seen {
        event_peer,
        direct_peer,
        verdict,
        via_event,
        via_direct,
    }
}

fn assert_dropped(node: &Node, rx: &mut Receiver<VoteMessage>, case: &str, seen: &Seen) {
    assert_eq!(seen.verdict, Verdict::Ignore, "{case}: verdict");
    assert_eq!(
        seen.via_event.len(),
        0,
        "{case}: the event path forwarded the vote to the updater"
    );
    assert_eq!(
        seen.via_direct.len(),
        0,
        "{case}: the direct call forwarded the vote to the updater"
    );
    assert!(
        matches!(rx.try_recv(), Err(TryRecvError::Empty)),
        "{case}: vote_tx must stay empty"
    );
    assert_eq!(
        s::malformed(node, &seen.event_peer),
        1,
        "{case}: the event path must record the handle's source peer once"
    );
    assert_eq!(
        s::malformed(node, &seen.direct_peer),
        1,
        "{case}: the direct call must record the source peer once"
    );
}

fn assert_forwarded_unchanged(case: &str, sent: &VoteMessage, got: &[VoteMessage]) {
    assert_eq!(
        got.len(),
        1,
        "{case}: exactly one vote must reach the updater"
    );
    let g = &got[0];
    assert_eq!(
        (
            &g.version,
            &g.producer_id,
            &g.signature,
            g.timestamp,
            g.vote
        ),
        (
            &sent.version,
            &sent.producer_id,
            &sent.signature,
            sent.timestamp,
            sent.vote
        ),
        "{case}: the forwarded vote differs from the one sent"
    );
}

fn assert_well_formed(node: &Node, case: &str, sent: &VoteMessage, seen: &Seen, want: Verdict) {
    assert_eq!(seen.verdict, want, "{case}: verdict");
    assert_forwarded_unchanged(&format!("{case} (event path)"), sent, &seen.via_event);
    assert_forwarded_unchanged(&format!("{case} (direct call)"), sent, &seen.via_direct);
    assert_eq!(
        s::malformed(node, &seen.event_peer),
        0,
        "{case}: event path recorded malformed"
    );
    assert_eq!(
        s::malformed(node, &seen.direct_peer),
        0,
        "{case}: direct call recorded malformed"
    );
}

async fn self_signed_dropped(case: &str, version: &str) {
    let (mut node, _member, mut rx, _t) = s::node().await;
    let (_, bytes) = signed(&KeyPair::generate(), version);
    let seen = deliver(&mut node, &mut rx, &bytes).await;
    assert_dropped(&node, &mut rx, case, &seen);
}

/// 64 bytes spanning the ASCII graphic range from 0x21 to 0x7E.
fn graphic_64() -> String {
    let v: String = (0x21u8..=0x5F)
        .map(char::from)
        .chain(std::iter::once('~'))
        .collect();
    assert!(
        v.len() == 64 && v.bytes().all(|b| b.is_ascii_graphic()),
        "setup: 64 graphic bytes"
    );
    v
}

// REQ-VOTE-002 — Decision: a failure means a version one byte past the 64-byte cap passes the shape check.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn version_of_65_bytes_is_ignored_not_forwarded_and_recorded() {
    self_signed_dropped("65 bytes", &"1".repeat(65)).await;
}

// REQ-VOTE-002 — Decision: a failure means a 200 KB multi-line version reaches the updater and its log.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_multiline_version_is_ignored_not_forwarded_and_recorded() {
    let version = "6.45.1\n".repeat(28_575);
    assert_eq!(version.len(), 200_025, "setup: version length");
    let (mut node, _member, mut rx, _t) = s::node().await;
    let (_, bytes) = signed(&KeyPair::generate(), &version);
    let seen = deliver(&mut node, &mut rx, &bytes).await;
    let forwarded: usize = seen.via_direct.iter().map(|v| v.version.len()).sum();
    println!("M3-PROBE version_bytes_forwarded={forwarded}");
    assert_eq!(forwarded, 0, "version bytes forwarded to the updater");
    assert_dropped(&node, &mut rx, "200,025 bytes multi-line", &seen);
}

// REQ-VOTE-002 — Decision: a failure means a version with a line feed and a second line passes the shape check.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn version_with_line_feed_is_ignored_not_forwarded_and_recorded() {
    self_signed_dropped("LF", "6.45.1\nforged line").await;
}

// REQ-VOTE-002 — Decision: a failure means a version with a carriage return passes the shape check.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn version_with_carriage_return_is_ignored_not_forwarded_and_recorded() {
    self_signed_dropped("CR", "6.45.1\r").await;
}

// REQ-VOTE-002 — Decision: a failure means a version with a tab passes the shape check.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn version_with_tab_is_ignored_not_forwarded_and_recorded() {
    self_signed_dropped("TAB", "6.45.1\t").await;
}

// REQ-VOTE-002 — Decision: a failure means a version with an ESC control byte passes the shape check.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn version_with_escape_byte_is_ignored_not_forwarded_and_recorded() {
    self_signed_dropped("ESC", "6.45.1\u{1b}").await;
}

// REQ-VOTE-002 — Decision: a failure means a version with a DEL byte (0x7F, just past the graphic range) passes the shape check.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn version_with_delete_byte_is_ignored_not_forwarded_and_recorded() {
    self_signed_dropped("DEL", "6.45.1\u{7f}").await;
}

// REQ-VOTE-002 — Decision: a failure means a version with a space (0x20, just below the graphic range) passes the shape check.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn version_with_space_is_ignored_not_forwarded_and_recorded() {
    self_signed_dropped("space", "6.45 .1").await;
}

// REQ-VOTE-002 — Decision: a failure means a version with a non-ASCII letter passes the shape check.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn version_with_non_ascii_letter_is_ignored_not_forwarded_and_recorded() {
    self_signed_dropped("non-ASCII letter", "6.45.1\u{e9}").await;
}

// REQ-VOTE-002 — Decision: a failure means a version with a full-width digit (a digit, but not ASCII) passes the shape check.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn version_with_full_width_digit_is_ignored_not_forwarded_and_recorded() {
    self_signed_dropped("full-width digit", "6.45.\u{ff11}").await;
}

// REQ-VOTE-002 — Decision: a failure means an empty version passes the shape check.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_version_is_ignored_not_forwarded_and_recorded() {
    self_signed_dropped("empty", "").await;
}

// REQ-VOTE-002 — Decision: a failure means the version check runs only for non-members, so a 65-byte version signed by the active producer is Accepted and forwarded.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn version_of_65_bytes_from_active_member_is_ignored_not_forwarded_and_recorded() {
    let (mut node, member, mut rx, _t) = s::node().await;
    let (_, bytes) = signed(&member, &"1".repeat(65));
    let seen = deliver(&mut node, &mut rx, &bytes).await;
    assert_dropped(&node, &mut rx, "65 bytes, active member", &seen);
}

// REQ-VOTE-002 — Decision: a failure means the version check rejects a release-shaped version up to 64 graphic bytes, so honest votes stop reaching the tally.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn well_formed_versions_from_active_member_are_accepted_and_forwarded_unchanged() {
    let (mut node, member, mut rx, _t) = s::node().await;
    let max_triple = "4294967295.4294967295.4294967295";
    assert_eq!(max_triple.len(), 32, "setup: max triple length");
    let versions = [
        "6.45.1".to_string(),
        "v6.45.1".into(),
        "6.45.1-rc.1".into(),
        "6.45.1+build.7".into(),
        max_triple.into(),
        graphic_64(),
    ];
    for version in versions {
        let (sent, bytes) = signed(&member, &version);
        let seen = deliver(&mut node, &mut rx, &bytes).await;
        assert_well_formed(&node, &version, &sent, &seen, Verdict::Accept);
    }
}

// REQ-VOTE-002 — Decision: a failure means the fix changed the non-member path: a well-formed non-member vote stops reaching the updater or is charged as malformed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn well_formed_version_from_non_member_is_ignored_but_forwarded_and_not_recorded() {
    let (mut node, _member, mut rx, _t) = s::node().await;
    let (sent, bytes) = signed(&KeyPair::generate(), "6.45.1");
    let seen = deliver(&mut node, &mut rx, &bytes).await;
    assert_well_formed(&node, "non-member 6.45.1", &sent, &seen, Verdict::Ignore);
}
