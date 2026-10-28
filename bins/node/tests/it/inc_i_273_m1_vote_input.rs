// OUTPUT CONTRACT: fn on_new_vote(...)
//   driven through Node::handle_network_event(NetworkEvent::NewVote(bytes, handle)) at B on a
//   live A -> B(node, one active producer) -> C line; the handle is stamped by B's swarm.
//   O1: none (bytes moved in)
//   O2: B stays live: the next honest vote is still judged and forwarded
//       (self.vote_ingress_scorer is asserted in inc_i_273_m1_vote_scorer.rs)
//   O3: Verdict, observed on the wire: Accept = C receives the vote from B, Ignore = C does not
//   O4: none   O5: none
//   O6: vote_tx: the updater receives the vote (matched by its unique timestamp)
// PATHS: P1 honest vote from the active producer -> Accept + forwarded;
//   P2 voter id is not a public key -> Ignore, not forwarded, B keeps serving;
//   P3 active voter id, signature not hex of the signature length -> as P2;
//   P4 active voter id, oversized version, signature by another key -> as P2
// INPUT PARTITIONS: voter id {shapes a-d: four non-hex multi-byte ids longer than a key prefix,
//   empty, 1 byte, 16 ASCII,
//   64 non-hex ASCII, 62 hex, 66 hex, 64 hex off the curve, 100 KB hex};
//   signature {128 non-hex, 126 hex, multi-byte}; version {100 KB with a multi-byte char}
// MATRIX: (O3, O6, O2) x (P1 + 12 id + 3 signature + 1 version partitions) = 51 cells,
//   one test per partition; every malformed test also re-runs P1 after the drop.
// covers: gossip_verdicts event_loop gossip_handle mod init vote_ingress

use std::time::{Duration, Instant};

use crypto::{KeyPair, PublicKey};
use doli_node::node::Node;
use network::{NetworkCommand, NetworkEvent};
use tokio::sync::mpsc::Receiver;
use updater::VoteMessage;

use super::inc_i_237_m3_node_common as c;
use super::inc_i_237_m3_node_common::Ev;

pub const VERSION: &str = "9.9.9";

/// A Veto vote with these fields; `sig: None` signs it with `kp`.
pub fn vote_json(kp: &KeyPair, version: &str, voter: &str, ts: u64, sig: Option<&str>) -> Vec<u8> {
    let mut v = VoteMessage::new(version.into(), updater::Vote::Veto, voter.into());
    v.timestamp = ts;
    v.signature = match sig {
        Some(s) => s.into(),
        None => crypto::signature::sign(&v.message_bytes(), kp.private_key()).to_hex(),
    };
    serde_json::to_vec(&v).expect("vote json")
}

/// Signed by `kp`; the voter id is the only malformed field.
pub fn with_id(kp: &KeyPair, voter: &str, ts: u64) -> Vec<u8> {
    vote_json(kp, VERSION, voter, ts, None)
}

/// From `kp`'s own voter id; the signature is the only malformed field.
pub fn with_sig(kp: &KeyPair, sig: &str, ts: u64) -> Vec<u8> {
    vote_json(kp, VERSION, &kp.public_key().to_hex(), ts, Some(sig))
}

fn shape(id: String) -> String {
    assert!(
        id.len() > 16 && !id.is_char_boundary(16),
        "setup: not a shape case"
    );
    id
}

pub fn off_curve_hex() -> String {
    (0u8..=255)
        .map(|b| [b; 32])
        .find(|k| PublicKey::try_from_slice(k).is_err())
        .map(hex::encode)
        .expect("setup: an off-curve 32-byte value")
}

/// Voter ids that are not a public key, by partition name.
pub fn malformed_ids(kp: &KeyPair) -> Vec<(&'static str, String)> {
    let hex = kp.public_key().to_hex();
    vec![
        ("shape a", shape(format!("{}\u{e9}", "a".repeat(15)))),
        ("shape b", shape(format!("{}\u{20ac}", "a".repeat(14)))),
        ("shape c", shape(format!("{}\u{10348}", "a".repeat(13)))),
        ("shape d", shape("\u{20ac}".repeat(6))),
        ("empty", String::new()),
        ("1 byte", "a".into()),
        ("16 ASCII", "x".repeat(16)),
        ("64 non-hex", "z".repeat(64)),
        ("62 hex", hex[..62].into()),
        ("66 hex", format!("{hex}00")),
        ("off-curve", off_curve_hex()),
        ("100 KB", "ab".repeat(50_000)),
    ]
}

pub fn id_case(kp: &KeyPair, name: &str) -> String {
    malformed_ids(kp)
        .into_iter()
        .find(|(n, _)| *n == name)
        .map(|(_, id)| id)
        .expect("known voter id partition")
}

/// Signatures that are not hex of the signature length, by partition name.
pub fn malformed_sigs(kp: &KeyPair, ts: u64) -> Vec<(&'static str, String)> {
    let mut v = VoteMessage::new(VERSION.into(), updater::Vote::Veto, String::new());
    v.timestamp = ts;
    let real = crypto::signature::sign(&v.message_bytes(), kp.private_key()).to_hex();
    vec![
        ("non-hex", "z".repeat(128)),
        ("126 hex", real[..126].into()),
        ("multi-byte", shape(format!("a{}", "\u{e9}".repeat(64)))),
    ]
}

pub fn sig_case(kp: &KeyPair, ts: u64, name: &str) -> String {
    malformed_sigs(kp, ts)
        .into_iter()
        .find(|(n, _)| *n == name)
        .map(|(_, s)| s)
        .expect("known signature partition")
}

/// Drain `rx`; count the forwarded votes stamped `ts`.
pub fn forwarded(rx: &mut Receiver<VoteMessage>, ts: u64) -> usize {
    std::iter::from_fn(|| rx.try_recv().ok())
        .filter(|v| v.timestamp == ts)
        .count()
}

async fn vote_line() -> (c::Line, Receiver<VoteMessage>) {
    let mut l = c::line().await;
    let (tx, rx) = tokio::sync::mpsc::channel(1_000);
    l.b.set_vote_tx(tx);
    let (m, t0) = (l.member.clone(), c::now_secs());
    l.warm(c::is_vote, |i| {
        NetworkCommand::BroadcastVote(c::vote(&m, t0 - 300 - i as u64, true))
    })
    .await;
    (l, rx)
}

/// Pump B's votes through the production dispatch; returns the vote payloads B judged.
async fn pump_votes(node: &mut Node, dur: Duration) -> Vec<Vec<u8>> {
    let (mut seen, until) = (Vec::new(), Instant::now() + dur);
    while Instant::now() < until {
        let net = node.network.as_mut().expect("node network attached");
        if let Ok(Some(ev)) =
            tokio::time::timeout(Duration::from_millis(50), net.next_event()).await
        {
            if let NetworkEvent::NewVote(data, _) = &ev {
                seen.push(data.clone());
                let _ = node.handle_network_event(ev).await;
            }
        }
    }
    seen
}

async fn honest_accepted(l: &mut c::Line, rx: &mut Receiver<VoteMessage>, ts: u64) {
    let good = c::vote(&l.member, ts, true);
    l.send(NetworkCommand::BroadcastVote(good.clone())).await;
    pump_votes(&mut l.b, Duration::from_millis(2_500)).await;
    assert_eq!(
        c::count(&l.c_log, &Ev::Vote(good)),
        1,
        "honest vote must cross B once (Accept)"
    );
    assert_eq!(forwarded(rx, ts), 1, "honest vote must reach the updater");
}

async fn dropped_then_honest_accepted(make: impl FnOnce(&KeyPair, u64) -> Vec<u8>) {
    let (mut l, mut rx) = vote_line().await;
    let t0 = c::now_secs();
    let bad = make(&l.member, t0 - 7);
    l.send(NetworkCommand::BroadcastVote(bad.clone())).await;
    let seen = pump_votes(&mut l.b, Duration::from_millis(2_500)).await;
    assert!(seen.contains(&bad), "setup: B never judged the vote");
    assert_eq!(
        c::count(&l.c_log, &Ev::Vote(bad)),
        0,
        "malformed vote crossed B (verdict must be Ignore)"
    );
    assert_eq!(
        forwarded(&mut rx, t0 - 7),
        0,
        "malformed vote reached the updater"
    );
    honest_accepted(&mut l, &mut rx, t0 - 1).await;
}

async fn id_dropped(name: &'static str) {
    dropped_then_honest_accepted(|kp, ts| with_id(kp, &id_case(kp, name), ts)).await;
}

async fn sig_dropped(name: &'static str) {
    dropped_then_honest_accepted(|kp, ts| with_sig(kp, &sig_case(kp, ts, name), ts)).await;
}

// REQ-VOTE-002 — Decision: a failure means the fix changed the honest path, so a valid vote from an active producer stops crossing B or stops reaching the updater tally.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn honest_vote_from_active_producer_is_accepted_and_forwarded() {
    let (mut l, mut rx) = vote_line().await;
    honest_accepted(&mut l, &mut rx, c::now_secs() - 1).await;
}

// REQ-VOTE-001 — Decision: a failure means a malformed voter id of shape a is not dropped with Ignore, or later honest votes are not served.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn malformed_voter_id_shape_a_is_dropped_and_node_keeps_serving() {
    id_dropped("shape a").await;
}

// REQ-VOTE-001 — Decision: a failure means a malformed voter id of shape b is not dropped with Ignore, or later honest votes are not served.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn malformed_voter_id_shape_b_is_dropped_and_node_keeps_serving() {
    id_dropped("shape b").await;
}

// REQ-VOTE-001 — Decision: a failure means a malformed voter id of shape c is not dropped with Ignore, or later honest votes are not served.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn malformed_voter_id_shape_c_is_dropped_and_node_keeps_serving() {
    id_dropped("shape c").await;
}

// REQ-VOTE-001 — Decision: a failure means a malformed voter id of shape d is not dropped with Ignore, or later honest votes are not served.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn malformed_voter_id_shape_d_is_dropped_and_node_keeps_serving() {
    id_dropped("shape d").await;
}

// REQ-VOTE-002 — Decision: a failure means the voter-id check mishandles an empty id (relays it, forwards it to the tally, or later honest votes are not served).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn empty_voter_id_is_dropped_and_node_keeps_serving() {
    id_dropped("empty").await;
}

// REQ-VOTE-002 — Decision: a failure means the voter-id check mishandles a 1-byte id (relays it, forwards it to the tally, or later honest votes are not served).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_byte_voter_id_is_dropped_and_node_keeps_serving() {
    id_dropped("1 byte").await;
}

// REQ-VOTE-002 — Decision: a failure means a 16-byte non-key voter id is relayed or forwarded, or later honest votes are not served.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sixteen_byte_ascii_voter_id_is_dropped_and_node_keeps_serving() {
    id_dropped("16 ASCII").await;
}

// REQ-VOTE-002 — Decision: a failure means a key-length voter id that is not hex passes the new shape check and is relayed or forwarded.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn non_hex_64_char_voter_id_is_dropped_and_node_keeps_serving() {
    id_dropped("64 non-hex").await;
}

// REQ-VOTE-002 — Decision: a failure means a hex voter id one byte short of a key passes the new shape check and is relayed or forwarded.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn short_hex_voter_id_is_dropped_and_node_keeps_serving() {
    id_dropped("62 hex").await;
}

// REQ-VOTE-002 — Decision: a failure means a hex voter id one byte longer than a key passes the new shape check and is relayed or forwarded.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn long_hex_voter_id_is_dropped_and_node_keeps_serving() {
    id_dropped("66 hex").await;
}

// REQ-VOTE-002 — Decision: a failure means the new check validates hex and length only, so a voter id that is not a curve point is relayed or forwarded.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn off_curve_hex_voter_id_is_dropped_and_node_keeps_serving() {
    id_dropped("off-curve").await;
}

// REQ-VOTE-002 — Decision: a failure means a 100 KB voter id is relayed or forwarded instead of being dropped, or later honest votes are not served.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn oversized_voter_id_is_dropped_and_node_keeps_serving() {
    id_dropped("100 KB").await;
}

// REQ-VOTE-002 — Decision: a failure means a non-hex signature from an active producer id is relayed or forwarded, or later honest votes are not served.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn non_hex_signature_from_active_producer_is_dropped_and_node_keeps_serving() {
    sig_dropped("non-hex").await;
}

// REQ-VOTE-002 — Decision: a failure means a signature one byte short from an active producer id is relayed or forwarded, or later honest votes are not served.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wrong_length_signature_from_active_producer_is_dropped_and_node_keeps_serving() {
    sig_dropped("126 hex").await;
}

// REQ-VOTE-002 — Decision: a failure means a multi-byte signature field is relayed or forwarded, or later honest votes are not served.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multi_byte_signature_from_active_producer_is_dropped_and_node_keeps_serving() {
    sig_dropped("multi-byte").await;
}

// REQ-VOTE-002 — Decision: a failure means a 100 KB multi-byte version is mishandled, or a vote signed by another key is relayed or forwarded.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn oversized_version_with_forged_signature_is_dropped_and_node_keeps_serving() {
    dropped_then_honest_accepted(|kp, ts| {
        let version = shape(format!("{}\u{e9}{}", "9".repeat(15), "9".repeat(100_000)));
        let other = KeyPair::generate();
        vote_json(&other, &version, &kp.public_key().to_hex(), ts, None)
    })
    .await;
}
