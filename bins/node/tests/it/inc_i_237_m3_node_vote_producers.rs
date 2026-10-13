//! INC-I-237 M3 (R2) — the REAL node's vote and producer-gossip verdicts on a
//! live A -> B(node) -> C line (production dispatch at B).
//!
//! OUTPUT CONTRACT: Node::handle_network_event(NewVote | ProducerAnnouncementsReceived
//!   | ProducerDigestReceived)
//!   O1: none
//!   O2: self.producer_gset (merge result); self.announcement_merge_busy (single flight)
//!   O3: Result (ignored)
//!   O4: none   O5: none
//!   O6: what C receives (vote / producer set / digest); B's delta reply at A
//! PATHS: votes P1 active producer, valid signature -> relayed; P2 non-member ->
//!   not; P3 bad signature -> not; P4 exited producer -> not (X6: the vote
//!   consumer counts only `is_active()` members). Producer sets P5 merge adds a
//!   verified entry -> relayed; P6 merge adds 0 (known + forged) -> not; P7 wrong
//!   network -> not; P8 merge in flight (single-flight drop) -> not, and a later
//!   batch with the same entry merges and is relayed. Digest P9 -> never
//!   relayed, B answers the forwarder.
//! INPUT PARTITIONS: voter {active, exited, non-member}; signature {valid,
//!   garbage}; batch {new, known + forged, wrong network}; merge slot {free, busy}
//! MATRIX: O6 x P1-P9; O2 x P5, P8 — asserted below.

use std::sync::atomic::Ordering;
use std::time::Duration;

use crypto::KeyPair;
use doli_core::{Network, ProducerAnnouncement, ProducerGSet};
use network::NetworkCommand;
use storage::{ProducerInfo, ProducerStatus};

use super::inc_i_237_m3_node_common as c;
use super::inc_i_237_m3_node_common::Ev;

async fn vote_line() -> (c::Line, tokio::sync::mpsc::Receiver<updater::VoteMessage>) {
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

async fn producer_line() -> c::Line {
    let mut l = c::line().await;
    let gh = l.b.chain_state.read().await.genesis_hash;
    let nid = l.b.config.network.id();
    l.warm(c::is_producers, move |i| {
        let kp = KeyPair::from_seed([i as u8 + 1; 32]);
        NetworkCommand::BroadcastProducerAnnouncements(vec![ProducerAnnouncement::new(
            &kp, nid, 1, gh,
        )])
    })
    .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    l
}

fn set_of(anns: &[&ProducerAnnouncement]) -> Ev {
    Ev::Producers(anns.iter().map(|a| a.pubkey).collect())
}

async fn publish_set(l: &mut c::Line, anns: &[&ProducerAnnouncement]) {
    let batch = anns.iter().map(|a| (*a).clone()).collect();
    l.send(NetworkCommand::BroadcastProducerAnnouncements(batch))
        .await;
    c::pump(&mut l.b, Duration::from_millis(3_000)).await;
}

// REQ-GRV-003 / X6 — Decision: a failure means B relays a vote the updater tally will drop (non-member, exited, bad signature), or uses a different membership source than the tally (X6), so forged veto traffic floods every node.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn x6_vote_crosses_b_only_from_an_active_producer_with_a_valid_signature() {
    let (mut l, _rx) = vote_line().await;
    let exited = KeyPair::generate();
    let bond = Network::Devnet.bond_unit();
    {
        let mut set = l.b.producer_set.write().await;
        let info = ProducerInfo::new(
            *exited.public_key(),
            0,
            bond,
            (crypto::Hash::ZERO, 0),
            0,
            bond,
        );
        set.register(info, 0).expect("register");
        set.get_by_pubkey_mut(exited.public_key()).unwrap().status = ProducerStatus::Exited;
    }
    let t0 = c::now_secs();
    let rows = [
        ("active producer", c::vote(&l.member, t0 - 1, true), 1),
        ("non-member", c::vote(&KeyPair::generate(), t0 - 2, true), 0),
        ("bad signature", c::vote(&l.member, t0 - 3, false), 0),
        ("exited producer", c::vote(&exited, t0 - 4, true), 0),
    ];
    for (_, data, _) in &rows {
        l.send(NetworkCommand::BroadcastVote(data.clone())).await;
        c::pump(&mut l.b, Duration::from_millis(2_500)).await;
    }
    for (row, data, want) in rows {
        assert_eq!(c::count(&l.c_log, &Ev::Vote(data)), want, "{row}");
    }
}

// REQ-GRV-003 — Decision: a failure means B relays a producer batch whose GSet merge added nothing it verified (known, forged or wrong-network entries), or stops relaying a batch that taught it a new producer.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn producer_batch_crosses_b_iff_the_gset_merge_added_a_verified_entry() {
    let mut l = producer_line().await;
    let (x, z, w) = (
        KeyPair::generate(),
        KeyPair::generate(),
        KeyPair::generate(),
    );
    let ax = c::announcement(&l.b, &x, 1).await;
    let az = c::forge_announcement(&c::announcement(&l.b, &z, 1).await);
    let gh = l.b.chain_state.read().await.genesis_hash;
    let aw = ProducerAnnouncement::new(&w, 0xDEAD, 1, gh);
    publish_set(&mut l, &[&ax]).await;
    assert!(
        c::gset_has(&l.b, &x).await,
        "setup: B merged the new announcement"
    );
    publish_set(&mut l, &[&ax, &az]).await;
    publish_set(&mut l, &[&aw]).await;
    assert_eq!(
        c::count(&l.c_log, &set_of(&[&ax])),
        1,
        "new verified entry must cross once"
    );
    assert_eq!(
        c::count(&l.c_log, &set_of(&[&ax, &az])),
        0,
        "known + forged batch crossed B"
    );
    assert_eq!(
        c::count(&l.c_log, &set_of(&[&aw])),
        0,
        "wrong-network batch crossed B"
    );
}

// REQ-GRV-003 / R2 — Decision: a failure means a batch dropped by the single-flight merge is still relayed (unverified content forwarded), or the drop poisons the entry so the digest/delta repair never merges it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_flight_dropped_batch_is_ignored_and_a_later_delta_converges() {
    let mut l = producer_line().await;
    let (y, v) = (KeyPair::generate(), KeyPair::generate());
    let (ay, av) = (
        c::announcement(&l.b, &y, 1).await,
        c::announcement(&l.b, &v, 1).await,
    );
    l.b.announcement_merge_busy.store(true, Ordering::SeqCst);
    publish_set(&mut l, &[&ay]).await;
    assert!(
        !c::gset_has(&l.b, &y).await,
        "setup: the busy merge slot dropped the batch"
    );
    assert_eq!(
        c::count(&l.c_log, &set_of(&[&ay])),
        0,
        "dropped batch crossed B"
    );
    l.b.announcement_merge_busy.store(false, Ordering::SeqCst);
    publish_set(&mut l, &[&ay, &av]).await;
    assert!(
        c::gset_has(&l.b, &y).await && c::gset_has(&l.b, &v).await,
        "delta did not converge"
    );
    assert_eq!(
        c::count(&l.c_log, &set_of(&[&ay, &av])),
        1,
        "converging delta must cross once"
    );
}

// REQ-GRV-003 / X5 — Decision: a failure means B relays a peer's bloom digest multi-hop (the far node answers the wrong peer), or B stops answering the forwarder with its delta.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn producer_digest_is_not_relayed_and_b_answers_the_forwarder() {
    let mut l = producer_line().await;
    let (x, k) = (KeyPair::generate(), KeyPair::generate());
    let ax = c::announcement(&l.b, &x, 1).await;
    l.b.producer_gset
        .write()
        .await
        .merge_one(ax)
        .expect("B knows x");
    let gh = l.b.chain_state.read().await.genesis_hash;
    let mut theirs = ProducerGSet::new(l.b.config.network.id(), gh);
    theirs
        .merge_one(c::announcement(&l.b, &k, 1).await)
        .expect("digest content");
    let digests_before = c::count_where(&l.c_log, |e| matches!(e, Ev::Digest(_)));
    l.send(NetworkCommand::BroadcastProducerDigest(
        theirs.to_bloom_filter(),
    ))
    .await;
    c::pump(&mut l.b, Duration::from_millis(3_000)).await;
    let x_pk = *x.public_key();
    let answered = c::count_where(
        &l.a_log,
        |e| matches!(e, Ev::Producers(p) if p.contains(&x_pk)),
    );
    assert!(
        answered >= 1,
        "B must answer the forwarder with the delta it lacks"
    );
    let digests = c::count_where(&l.c_log, |e| matches!(e, Ev::Digest(_)));
    assert_eq!(digests - digests_before, 0, "A's digest crossed B");
}
