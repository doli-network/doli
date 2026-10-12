//! INC-I-237 M3 (R2) — per-topic relay through the node verdict on a live
//! A -> B -> C line of real `NetworkService`s (127.0.0.1). B runs the M3 fake
//! node in Keep mode: its verdict is withheld until the test releases it.
//!
//! OUTPUT CONTRACT: swarm path for attestations / votes / producer gossip
//!   (Defer -> node verdict -> report on the ORIGINAL id -> mesh forward)
//!   O1-O5: none
//!   O6: gossip events per node (kind, bytes, forwarder, handle topic)
//! PATHS: P1 verdict withheld -> not forwarded; P2 Accept -> forwarded once;
//!   P3 Ignore -> never forwarded; P4 producer digest -> processed by B with
//!   A as forwarder, never forwarded
//! INPUT PARTITIONS: topic {attestation, vote, producer set, producer digest};
//!   verdict {withheld, Accept, Ignore}
//! MATRIX: O6 at C x P1-P3 per topic; O6 at B (stamped handle topic) x P1;
//!   O6 at B and C x P4. Topology asserted (A and C keep exactly one peer).

use std::collections::HashSet;
use std::time::Duration;

use crypto::KeyPair;
use network::service::Verdict;
use network::NetworkCommand;

use super::inc_i_237_m2_common as h;
use super::inc_i_237_m3_fake_node::{
    announcement, attestation, digest, keys, now_secs, spawn_fake3, vote_json, wait_for, Fake3,
    Kind, Mode,
};

struct Line {
    a: Fake3,
    b: Fake3,
    c: Fake3,
    member: KeyPair,
}

async fn line() -> Line {
    let member = KeyPair::from_seed([0x31; 32]);
    let members: HashSet<_> = [*member.public_key()].into();
    let b_port = h::reserve_port();
    let b = spawn_fake3(
        h::start_node(b_port, vec![]).await,
        Mode::Judge,
        members.clone(),
        Duration::ZERO,
    );
    let a = spawn_fake3(
        h::start_node(h::reserve_port(), vec![h::addr(b_port)]).await,
        Mode::Judge,
        members.clone(),
        Duration::ZERO,
    );
    let c = spawn_fake3(
        h::start_node(h::reserve_port(), vec![h::addr(b_port)]).await,
        Mode::Judge,
        members,
        Duration::ZERO,
    );
    Line { a, b, c, member }
}

/// Publish `make(i)` from A until C received one (the B -> C mesh on the topic).
async fn warm(l: &Line, kind: Kind, make: impl Fn(u32) -> NetworkCommand) {
    for i in 0..60u32 {
        if l.c.count_kind(kind) > 0 {
            for (name, f) in [("A", &l.a), ("C", &l.c)] {
                assert_eq!(
                    f.peer_count().await,
                    1,
                    "topology: {name} must have one peer"
                );
            }
            return;
        }
        l.a.cmd.send(make(i)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("setup: {kind:?} never crossed A -> B -> C");
}

/// Withheld -> 0 at C; released Ignore -> 0 at C; released Accept -> 1 at C.
async fn verdict_gated(
    l: &Line,
    kind: Kind,
    topic: &str,
    ignored: Vec<u8>,
    accepted: Vec<u8>,
    publish: impl Fn(Vec<u8>) -> NetworkCommand,
) {
    l.b.set(Mode::Keep);
    for data in [&ignored, &accepted] {
        l.a.cmd.send(publish(data.clone())).await.unwrap();
        let b_got = wait_for(Duration::from_secs(3), || l.b.count(kind, data) > 0).await;
        assert!(b_got, "setup: B received the {kind:?} probe");
    }
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    assert_eq!(
        l.c.count(kind, &accepted),
        0,
        "{kind:?} forwarded before B's verdict"
    );
    assert_eq!(
        l.c.count(kind, &ignored),
        0,
        "{kind:?} forwarded before B's verdict"
    );
    let stamped =
        l.b.got
            .lock()
            .unwrap()
            .iter()
            .filter(|g| g.kind == kind && g.data == accepted)
            .all(|g| g.topic.as_deref() == Some(topic));
    assert!(
        stamped,
        "{kind:?}: B's event must carry a verdict handle for {topic:?}"
    );
    assert_eq!(
        l.b.release(kind, &ignored, Verdict::Ignore),
        1,
        "ignored probe kept once"
    );
    assert_eq!(
        l.b.release(kind, &accepted, Verdict::Accept),
        1,
        "accepted probe kept once"
    );
    let crossed = wait_for(Duration::from_secs(3), || l.c.count(kind, &accepted) > 0).await;
    assert!(crossed, "{kind:?}: Accept must forward the message to C");
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    assert_eq!(
        l.c.count(kind, &accepted),
        1,
        "{kind:?}: forwarded exactly once"
    );
    assert_eq!(
        l.c.count(kind, &ignored),
        0,
        "{kind:?}: Ignore must not forward"
    );
}

// REQ-GRV-003 — Decision: a failure means B forwards an attestation before (or without) its own verdict, so a forged attestation crosses every honest hop.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn req_grv_003_attestation_waits_for_the_node_verdict_at_b() {
    let l = line().await;
    let m = l.member.clone();
    warm(&l, Kind::Attestation, |i| {
        NetworkCommand::BroadcastAttestation(attestation(&m, 1, 10 + i).to_bytes())
    })
    .await;
    let ignored = attestation(&l.member, 2, 500).to_bytes();
    let accepted = attestation(&l.member, 3, 501).to_bytes();
    verdict_gated(
        &l,
        Kind::Attestation,
        "Attestations",
        ignored,
        accepted,
        NetworkCommand::BroadcastAttestation,
    )
    .await;
}

// REQ-GRV-003 — Decision: a failure means B forwards a vote before (or without) its own verdict, so a non-producer's vote is relayed fleet-wide.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn req_grv_003_vote_waits_for_the_node_verdict_at_b() {
    let l = line().await;
    let m = l.member.clone();
    let t0 = now_secs();
    warm(&l, Kind::Vote, |i| {
        NetworkCommand::BroadcastVote(vote_json(&m, t0 - 100 - i as u64))
    })
    .await;
    let ignored = vote_json(&l.member, t0 - 1);
    let accepted = vote_json(&l.member, t0 - 2);
    verdict_gated(
        &l,
        Kind::Vote,
        "Votes",
        ignored,
        accepted,
        NetworkCommand::BroadcastVote,
    )
    .await;
}

// REQ-GRV-003 — Decision: a failure means B forwards a producer announcement batch before (or without) its own GSet verdict, so stale or forged batches are relayed fleet-wide.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn req_grv_003_producer_set_waits_for_the_node_verdict_at_b() {
    let l = line().await;
    warm(&l, Kind::Producers, |i| {
        NetworkCommand::BroadcastProducerAnnouncements(vec![announcement(&KeyPair::from_seed(
            [i as u8 + 1; 32],
        ))])
    })
    .await;
    let (x, y) = (
        announcement(&KeyPair::from_seed([0xA1; 32])),
        announcement(&KeyPair::from_seed([0xA2; 32])),
    );
    l.b.set(Mode::Keep);
    for (ann, v) in [(&x, Verdict::Ignore), (&y, Verdict::Accept)] {
        l.a.cmd
            .send(NetworkCommand::BroadcastProducerAnnouncements(vec![
                ann.clone()
            ]))
            .await
            .unwrap();
        let k = keys(std::slice::from_ref(ann));
        assert!(
            wait_for(Duration::from_secs(3), || l.b.count(Kind::Producers, &k)
                > 0)
            .await,
            "setup: B got {v:?} probe"
        );
    }
    let (kx, ky) = (keys(&[x]), keys(&[y]));
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    assert_eq!(
        l.c.count(Kind::Producers, &kx) + l.c.count(Kind::Producers, &ky),
        0,
        "forwarded before B's verdict"
    );
    let stamped =
        l.b.got
            .lock()
            .unwrap()
            .iter()
            .filter(|g| g.data == ky)
            .all(|g| g.topic.as_deref() == Some("Producers"));
    assert!(
        stamped,
        "producer set: B's event must carry a verdict handle"
    );
    assert_eq!(l.b.release(Kind::Producers, &kx, Verdict::Ignore), 1);
    assert_eq!(l.b.release(Kind::Producers, &ky, Verdict::Accept), 1);
    assert!(
        wait_for(Duration::from_secs(3), || l.c.count(Kind::Producers, &ky)
            > 0)
        .await,
        "Accept must forward"
    );
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    assert_eq!(l.c.count(Kind::Producers, &ky), 1, "forwarded exactly once");
    assert_eq!(
        l.c.count(Kind::Producers, &kx),
        0,
        "Ignore must not forward"
    );
}

// REQ-GRV-003 / X5 — Decision: a failure means a bloom digest is relayed multi-hop (the far node then sends its delta to the wrong peer), or B no longer learns which peer forwarded it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn req_grv_003_producer_digest_is_never_forwarded_and_names_its_forwarder() {
    let l = line().await;
    warm(&l, Kind::Producers, |i| {
        NetworkCommand::BroadcastProducerAnnouncements(vec![announcement(&KeyPair::from_seed(
            [i as u8 + 1; 32],
        ))])
    })
    .await;
    let (k1, k2) = (
        KeyPair::from_seed([0xB1; 32]),
        KeyPair::from_seed([0xB2; 32]),
    );
    for d in [digest(&[&k1]), digest(&[&k1, &k2])] {
        l.a.cmd
            .send(NetworkCommand::BroadcastProducerDigest(d))
            .await
            .unwrap();
    }
    let from_a = |f: &Fake3| {
        f.got
            .lock()
            .unwrap()
            .iter()
            .filter(|g| g.kind == Kind::Digest && g.from == Some(l.a.peer_id))
            .count()
    };
    assert!(
        wait_for(Duration::from_secs(3), || from_a(&l.b) == 2).await,
        "B must process both digests with A as the forwarder"
    );
    tokio::time::sleep(Duration::from_millis(2_000)).await;
    assert_eq!(l.c.count_kind(Kind::Digest), 0, "a digest crossed B");
    let probe = announcement(&KeyPair::from_seed([0xB3; 32]));
    l.a.cmd
        .send(NetworkCommand::BroadcastProducerAnnouncements(vec![
            probe.clone()
        ]))
        .await
        .unwrap();
    let k = keys(&[probe]);
    assert!(
        wait_for(Duration::from_secs(3), || l.c.count(Kind::Producers, &k)
            > 0)
        .await,
        "non-vacuity: the producers mesh B -> C is live"
    );
}
