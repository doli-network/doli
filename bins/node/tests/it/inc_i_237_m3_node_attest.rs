//! INC-I-237 M3 (R2 / F-9) — the REAL node's attestation verdict and the
//! ATTEST_FETCH gate: B = `Node` on a live A -> B -> C line; gossip events go
//! through the production dispatch (`Node::handle_network_event`).
//!
//! OUTPUT CONTRACT: Node::handle_network_event(NetworkEvent::NewAttestation(data, src, handle));
//!   Node::run_periodic_tasks(&mut self)
//!   O1: none
//!   O2: self.attest_fetch_tracker (the deferred GetBlockByHash record)
//!   O3: Result (ignored)
//!   O4: none   O5: none
//!   O6: attestation gossip at C (B's verdict decides the relay);
//!       GetBlockByHash requests that reach the recorded peer
//! PATHS: P1 member, valid signature, unknown block -> relayed, fetch recorded;
//!   P2 garbage signature -> not relayed, no fetch; P3 non-member, valid
//!   signature -> not relayed, no fetch; P4 pending-activation member -> relayed;
//!   P5 forged variant of the victim's (attester, slot) first, genuine second ->
//!   genuine relayed; P6 40 matured fetch entries -> <= 8 GetBlockByHash per tick
//! INPUT PARTITIONS: signature {valid, garbage}; attester {member, pending
//!   member, non-member}; block {unknown to B}; order {forged first}; tracker {1, 40}
//! MATRIX: O6 x P1-P5; O2 x P1-P3; O6 (requests) x P6 — asserted below.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use crypto::{Hash, KeyPair};
use doli_core::Network;
use network::{GossipHandle, NetworkCommand, NetworkEvent, PeerId};
use storage::ProducerInfo;

use super::inc_i_237_m2_node_common as c2;
use super::inc_i_237_m3_node_common as c;
use super::inc_i_237_m3_node_common::Ev;

const FETCH_CAP_PER_TICK: usize = 8;

async fn publish(l: &mut c::Line, data: Vec<u8>) {
    l.send(NetworkCommand::BroadcastAttestation(data)).await;
    c::pump(&mut l.b, Duration::from_millis(2_500)).await;
}

async fn warmed() -> c::Line {
    let mut l = c::line().await;
    let m = l.member.clone();
    l.warm(c::is_attestation, |i| {
        NetworkCommand::BroadcastAttestation(c::attestation(&m, 0x10, 10 + i).to_bytes())
    })
    .await;
    l
}

// F-9 / REQ-GRV-SEC-001 — Decision: a failure means B relays an attestation whose signature or attester membership it did not verify (Attestation::verify is self-certifying), or gates the relay on block knowledge / pending activation and drops genuine early attestations.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn f_9_req_grv_sec_001_only_verified_member_attestations_cross_b() {
    let mut l = warmed().await;
    let pending = KeyPair::generate();
    let bond = Network::Devnet.bond_unit();
    let info = ProducerInfo::new(
        *pending.public_key(),
        1_000_000,
        bond,
        (Hash::ZERO, 0),
        0,
        bond,
    );
    l.b.producer_set
        .write()
        .await
        .register(info, 1_000_000)
        .expect("register pending-activation member");
    let outsider = KeyPair::generate();
    let forged = c::forge(&c::attestation(&l.member, 0x20, 300)).to_bytes();
    let non_member = c::attestation(&outsider, 0x21, 301).to_bytes();
    let member = c::attestation(&l.member, 0x22, 302).to_bytes();
    let early = c::attestation(&pending, 0x23, 303).to_bytes();
    for d in [&forged, &non_member, &member, &early] {
        publish(&mut l, d.clone()).await;
    }
    let at_c = |d: &Vec<u8>| c::count(&l.c_log, &Ev::Attestation(d.clone()));
    assert_eq!(
        at_c(&forged),
        0,
        "SEC-001: garbage-signature attestation crossed B"
    );
    assert_eq!(
        at_c(&non_member),
        0,
        "F-9: non-member attestation crossed B"
    );
    assert_eq!(
        at_c(&member),
        1,
        "F-9: member attestation for an unknown block must cross once"
    );
    assert_eq!(at_c(&early), 1, "pending-activation member must cross once");
}

// REQ-GRV-SEC-002 — Decision: a failure means a forged-signature attestation carrying a victim's (attester, slot, block) and arriving first suppresses the genuine one at B, so an attacker can erase a producer's attendance fleet-wide.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn req_grv_sec_002_forged_attestation_first_does_not_suppress_the_genuine() {
    let mut l = warmed().await;
    let genuine = c::attestation(&l.member, 0x30, 400);
    let forged = c::forge(&genuine).to_bytes();
    publish(&mut l, forged.clone()).await;
    publish(&mut l, genuine.to_bytes()).await;
    assert_eq!(
        c::count(&l.c_log, &Ev::Attestation(forged)),
        0,
        "forged variant crossed B"
    );
    assert_eq!(
        c::count(&l.c_log, &Ev::Attestation(genuine.to_bytes())),
        1,
        "the genuine attestation must still cross B once"
    );
}

// REQ-GRV-SEC-001 / F-9 — Decision: a failure means an attestation from a forged or unknown attester records a GetBlockByHash, so forged gossip drives an ungoverned request per invented block hash.
#[tokio::test]
async fn f_9_only_member_attestations_record_a_block_fetch() {
    let (mut node, member, _t) = c::member_node().await;
    let outsider = KeyPair::generate();
    let peer = PeerId::random();
    let cases = [
        (
            "garbage signature",
            c::forge(&c::attestation(&member, 0x41, 7)),
            false,
        ),
        ("non-member", c::attestation(&outsider, 0x42, 7), false),
        ("member", c::attestation(&member, 0x43, 7), true),
    ];
    for (row, att, fetch) in cases {
        let ev = NetworkEvent::NewAttestation(att.to_bytes(), peer, GossipHandle::now());
        let _ = node.handle_network_event(ev).await;
        assert_eq!(
            node.attest_fetch_tracker.contains_key(&att.block_hash),
            fetch,
            "{row}: fetch recorded = {fetch} expected"
        );
    }
}

// REQ-GRV-SEC-001 / F-9 — Decision: a failure means one periodic tick turns every pending unknown-hash entry into a GetBlockByHash at once (the C-2 request storm), or the ATTEST_FETCH recovery stops fetching at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn f_9_attest_fetch_fires_at_most_8_get_block_by_hash_per_tick() {
    let (mut b, _member, _t) = c::member_node().await;
    let b_port = c2::reserve_port();
    b.network = Some(c2::start_net(b_port, vec![]).await);
    let a = c2::start_net(c2::reserve_port(), vec![c2::addr(b_port)]).await;
    let a_peer = *a.local_peer_id();
    let a_log = c::observe(a);
    let peers = b.network.as_ref().unwrap().peers_arc();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !peers.read().await.contains_key(&a_peer) {
        assert!(Instant::now() < deadline, "setup: A never connected to B");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let matured = Instant::now() - Duration::from_millis(600);
    let hashes: HashSet<Hash> = (0..40u8)
        .map(|i| Hash::from_bytes([0xC0 ^ i; 32]))
        .collect();
    for h in &hashes {
        b.attest_fetch_tracker.insert(*h, (matured, 1, a_peer));
    }
    b.run_periodic_tasks().await.expect("periodic tick");
    tokio::time::sleep(Duration::from_secs(2)).await;
    let fetched = c::count_where(
        &a_log,
        |e| matches!(e, Ev::BlockByHash(h) if hashes.contains(h)),
    );
    assert!(
        fetched >= 1,
        "non-vacuity: ATTEST_FETCH must still fetch (got 0)"
    );
    assert!(
        fetched <= FETCH_CAP_PER_TICK,
        "one tick fired {fetched} GetBlockByHash (cap {FETCH_CAP_PER_TICK})"
    );
}
