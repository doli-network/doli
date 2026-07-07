//! INC-I-137 — Producer-announcement gossip staleness filter (INC-I-120 Layer 3).
//!
//! Producer announcements on `PRODUCERS_TOPIC = "/doli/producers/1"` are
//! re-forwarded with **unconditional `MessageAcceptance::Accept`**. Stale
//! full-set snapshots (announcements >1h old) re-circulate forever, dominating
//! log/bandwidth volume (~8.5M lines/day during the 06/25-28 mainnet collapse).
//!
//! This test file exercises the planned `classify_producer_gossip(data, now_unix)`
//! function which DOES NOT EXIST YET. All tests are expected to fail at compile
//! time (RED state in TDD) until the Developer implements the function in
//! `crates/network/src/gossip/validation.rs`.
//!
//! # OUTPUT CONTRACT: fn classify_producer_gossip(data: &[u8], now_unix: u64)
//!
//! Outputs:
//!   O1: return — MessageAcceptance (Accept | Ignore | Reject)
//!       No mutable params, no receiver, no persistent store writes, no events.
//!       Pure function: single return value is the only observable output.
//!
//! Paths:
//!   P1 (all-stale set):    Non-empty ProducerSet where ALL announcements have
//!                          timestamp < now_unix - PRODUCER_ANNOUNCEMENT_MAX_AGE_SECS
//!                          → Ignore (suppress re-forward, no penalty)
//!   P2 (all-fresh set):    Non-empty ProducerSet where ALL announcements are
//!                          within TTL → Accept
//!   P3 (mixed freshness):  Non-empty ProducerSet with SOME stale + SOME fresh
//!                          → Accept (any fresh ⇒ forward whole snapshot)
//!   P4 (boundary-exact):   Announcement timestamp == now_unix - MAX_AGE (inclusive)
//!                          → Accept; timestamp == now_unix - MAX_AGE - 1 → Ignore
//!   P5 (empty set):        decode_producer_set yields empty Vec → Accept (fail-open)
//!   P6 (garbage bytes):    Undecodable-as-ProducerSet bytes → Accept (fail-open)
//!   P7 (digest bytes):     Bloom digest bytes (not a ProducerSet) → Accept (fail-open)
//!   P8 (now_unix=0):       All-stale set but now_unix=0 → Accept (fail-open,
//!                          mirrors classify_block_gossip genesis_time=0 handling)
//!
//! INPUT PARTITIONS:
//!   IP-1: 3-announcement all-stale set (timestamp = now - 4000, now=10_000_000)
//!         → exercises P1, asserts O1 = Ignore
//!   IP-2: 3-announcement all-fresh set (timestamp = now - 10)
//!         → exercises P2, asserts O1 = Accept
//!   IP-3: Mixed set: 2 stale (now-5000) + 1 fresh (now-10)
//!         → exercises P3, asserts O1 = Accept
//!   IP-4a: Boundary: ann timestamp = now - 3600 (exactly TTL, inclusive-fresh)
//!          → exercises P4, asserts O1 = Accept
//!   IP-4b: Boundary: ann timestamp = now - 3601 (one past TTL)
//!          → exercises P4, asserts O1 = Ignore
//!   IP-5: Empty ProducerSet (0 announcements encoded)
//!         → exercises P5, asserts O1 = Accept
//!   IP-6: Garbage bytes (b"not-a-producer-set")
//!         → exercises P6, asserts O1 = Accept
//!   IP-7: Bloom digest bytes (encode_digest)
//!         → exercises P7, asserts O1 = Accept
//!   IP-8: All-stale set with now_unix=0
//!         → exercises P8, asserts O1 = Accept
//!   IP-9: CRDT convergence integration — fresh set propagates A->B->C
//!         → exercises P2 end-to-end, asserts convergence
//!   IP-10: CRDT suppression integration — stale set suppressed at relay
//!          → exercises P1 end-to-end, asserts storm suppressed
//!
//! Matrix: 1 output × 10 input partitions = 10 assertions
//!   IP-1:  O1(Ignore)  ✓  test_all_stale_producer_set_ignored
//!   IP-2:  O1(Accept)  ✓  test_fresh_producer_set_accepted
//!   IP-3:  O1(Accept)  ✓  test_mixed_freshness_accepted
//!   IP-4a: O1(Accept)  ✓  test_boundary_exactly_at_ttl_accepted
//!   IP-4b: O1(Ignore)  ✓  test_boundary_one_past_ttl_ignored
//!   IP-5:  O1(Accept)  ✓  test_empty_set_accepted
//!   IP-6:  O1(Accept)  ✓  test_garbage_bytes_accepted
//!   IP-7:  O1(Accept)  ✓  test_digest_bytes_accepted
//!   IP-8:  O1(Accept)  ✓  test_now_zero_accepted
//!   IP-9:  O1(Accept at relay) ✓  test_inc_i_137_fresh_producer_set_still_converges (Phase FRESH)
//!   IP-10: O1(Ignore at relay) ✓  test_inc_i_137_fresh_producer_set_still_converges (Phase STALE)

use std::time::Duration;

use futures::StreamExt;
use libp2p::gossipsub::{self, IdentTopic, MessageAcceptance, MessageId};
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, Swarm};

use doli_core::discovery::{
    encode_digest, encode_producer_set, ProducerAnnouncement, ProducerBloomFilter,
};

// The function under test — DOES NOT EXIST YET. This import will cause a
// compile error, which is the expected RED state in TDD. The Developer must
// create `classify_producer_gossip` and `PRODUCER_ANNOUNCEMENT_MAX_AGE_SECS`
// in `crates/network/src/gossip/validation.rs` and re-export them from
// `crates/network/src/gossip/mod.rs`.
use network::gossip::{
    classify_producer_gossip, new_gossipsub_with_cache_time, MeshConfig,
    PRODUCER_ANNOUNCEMENT_MAX_AGE_SECS,
};

// ============================================================================
// Helpers
// ============================================================================

/// Fixed reference time for deterministic tests (no wall-clock dependency).
const NOW: u64 = 10_000_000;

/// Build a signed ProducerAnnouncement with a specific timestamp.
fn make_announcement(timestamp: u64) -> ProducerAnnouncement {
    let keypair = crypto::KeyPair::generate();
    ProducerAnnouncement::new_with_timestamp(&keypair, 1, 0, timestamp, crypto::Hash::ZERO)
}

/// Helper: check if the verdict is Accept (MessageAcceptance has no PartialEq).
fn is_accept(v: MessageAcceptance) -> bool {
    matches!(v, MessageAcceptance::Accept)
}

/// Helper: check if the verdict is Ignore.
fn is_ignore(v: MessageAcceptance) -> bool {
    matches!(v, MessageAcceptance::Ignore)
}

// ============================================================================
// Unit Tests — Classifier Matrix (pure, deterministic, fast)
// ============================================================================

// Requirement: INC-I-137 (Must)
// Acceptance: All-stale producer set → Ignore (suppress re-forward)
// Path: P1 / Input Partition: IP-1
//
// THIS IS THE REPRODUCTION TEST: pre-fix the topic is unconditionally Accepted;
// post-fix this must be Ignore.
#[test]
fn test_all_stale_producer_set_ignored() {
    // 3 announcements, all with timestamp = NOW - 4000 (>3600 TTL, all stale)
    let stale_ts = NOW - 4000;
    let anns = vec![
        make_announcement(stale_ts),
        make_announcement(stale_ts),
        make_announcement(stale_ts),
    ];
    let data = encode_producer_set(&anns);
    let verdict = classify_producer_gossip(&data, NOW);
    assert!(
        is_ignore(verdict),
        "INC-I-137 REPRODUCTION: all-stale producer set MUST be Ignored to suppress \
         gossip storm. Pre-fix this is unconditionally Accepted — the bug."
    );
}

// Requirement: INC-I-137 (Must)
// Acceptance: Fresh producer set → Accept (CRDT convergence preserved)
// Path: P2 / Input Partition: IP-2
#[test]
fn test_fresh_producer_set_accepted() {
    let fresh_ts = NOW - 10; // 10 seconds old, well within TTL
    let anns = vec![
        make_announcement(fresh_ts),
        make_announcement(fresh_ts),
        make_announcement(fresh_ts),
    ];
    let data = encode_producer_set(&anns);
    let verdict = classify_producer_gossip(&data, NOW);
    assert!(
        is_accept(verdict),
        "Fresh producer set MUST be Accepted — new announcements must still propagate \
         for CRDT convergence."
    );
}

// Requirement: INC-I-137 (Must)
// Acceptance: Mixed-freshness set → Accept (any fresh ⇒ forward whole snapshot)
// Path: P3 / Input Partition: IP-3
#[test]
fn test_mixed_freshness_accepted() {
    let stale_ts = NOW - 5000; // stale
    let fresh_ts = NOW - 10; // fresh
    let anns = vec![
        make_announcement(stale_ts),
        make_announcement(stale_ts),
        make_announcement(fresh_ts), // one fresh ⇒ whole set forwards
    ];
    let data = encode_producer_set(&anns);
    let verdict = classify_producer_gossip(&data, NOW);
    assert!(
        is_accept(verdict),
        "Mixed-freshness producer set (any fresh announcement) MUST be Accepted — \
         the snapshot contains at least one ann a receiving node could merge."
    );
}

// Requirement: INC-I-137 (Must)
// Acceptance: Boundary — exactly at TTL → Accept (inclusive-fresh)
// Path: P4 / Input Partition: IP-4a
#[test]
fn test_boundary_exactly_at_ttl_accepted() {
    // PRODUCER_ANNOUNCEMENT_MAX_AGE_SECS = 3600
    // timestamp = NOW - 3600: age == TTL exactly → inclusive-fresh → Accept
    let boundary_ts = NOW - PRODUCER_ANNOUNCEMENT_MAX_AGE_SECS;
    let anns = vec![make_announcement(boundary_ts)];
    let data = encode_producer_set(&anns);
    let verdict = classify_producer_gossip(&data, NOW);
    assert!(
        is_accept(verdict),
        "An announcement exactly at the TTL boundary (age == MAX_AGE) MUST be Accepted \
         (inclusive-fresh: >= means fresh, only strictly > is stale)."
    );
}

// Requirement: INC-I-137 (Must)
// Acceptance: Boundary — one second past TTL → Ignore
// Path: P4 / Input Partition: IP-4b
#[test]
fn test_boundary_one_past_ttl_ignored() {
    // timestamp = NOW - 3601: age == TTL + 1 → stale → Ignore
    let past_boundary_ts = NOW - PRODUCER_ANNOUNCEMENT_MAX_AGE_SECS - 1;
    let anns = vec![make_announcement(past_boundary_ts)];
    let data = encode_producer_set(&anns);
    let verdict = classify_producer_gossip(&data, NOW);
    assert!(
        is_ignore(verdict),
        "An announcement one second past the TTL boundary (age == MAX_AGE + 1) MUST be \
         Ignored — this is the first strictly-stale timestamp."
    );
}

// Requirement: INC-I-137 (Must)
// Acceptance: Empty set → Accept (fail-open)
// Path: P5 / Input Partition: IP-5
#[test]
fn test_empty_set_accepted() {
    let anns: Vec<ProducerAnnouncement> = vec![];
    let data = encode_producer_set(&anns);
    let verdict = classify_producer_gossip(&data, NOW);
    assert!(
        is_accept(verdict),
        "Empty producer set MUST be Accepted (fail-open). An empty set cannot be \
         classified as stale and should not be suppressed."
    );
}

// Requirement: INC-I-137 (Must)
// Acceptance: Garbage bytes → Accept (fail-open)
// Path: P6 / Input Partition: IP-6
#[test]
fn test_garbage_bytes_accepted() {
    let data = b"this-is-not-a-valid-producer-set-message";
    let verdict = classify_producer_gossip(data, NOW);
    assert!(
        is_accept(verdict),
        "Garbage bytes (undecodable as ProducerSet) MUST be Accepted (fail-open). \
         The filter only targets decoded non-empty stale sets — everything else passes \
         through to the existing handler which handles unknown formats."
    );
}

// Requirement: INC-I-137 (Must)
// Acceptance: Digest bytes → Accept (fail-open)
// Path: P7 / Input Partition: IP-7
#[test]
fn test_digest_bytes_accepted() {
    // Build a real bloom digest via encode_digest — this is a different wire
    // format than ProducerSet and must not be suppressed (it's the designed
    // delta-sync convergence path).
    let mut bloom = ProducerBloomFilter::new(100);
    let keypair = crypto::KeyPair::generate();
    bloom.insert(keypair.public_key());
    let data = encode_digest(&bloom);
    let verdict = classify_producer_gossip(&data, NOW);
    assert!(
        is_accept(verdict),
        "Bloom digest bytes MUST be Accepted (fail-open). Digest is the designed \
         new-node convergence path and must never be suppressed by the staleness filter."
    );
}

// Requirement: INC-I-137 (Must)
// Acceptance: now_unix=0 → Accept (fail-open, clock unavailable)
// Path: P8 / Input Partition: IP-8
#[test]
fn test_now_zero_accepted() {
    // All-stale set, but now_unix=0 → fail-open (mirrors classify_block_gossip's
    // genesis_time=0 handling: if the clock is unavailable, don't filter).
    let stale_ts = 5_000_000; // Would be stale at any real `now`
    let anns = vec![
        make_announcement(stale_ts),
        make_announcement(stale_ts),
        make_announcement(stale_ts),
    ];
    let data = encode_producer_set(&anns);
    let verdict = classify_producer_gossip(&data, 0);
    assert!(
        is_accept(verdict),
        "When now_unix=0 (clock unavailable), ALL producer sets MUST be Accepted \
         (fail-open). Gossip liveness > filter strictness on misconfiguration."
    );
}

// ============================================================================
// CRDT-Convergence Integration Test (mirrors INC-I-114 harness)
// ============================================================================

/// Short duplicate cache for fast test execution.
const TEST_CACHE_TTL: Duration = Duration::from_secs(3);

/// Topic matching PRODUCERS_TOPIC for the integration test.
const TEST_TOPIC: &str = "/doli/producers/1";

/// Mesh config for a 4-node test network: small full mesh.
fn test_mesh() -> MeshConfig {
    MeshConfig {
        mesh_n: 3,
        mesh_n_low: 2,
        mesh_n_high: 6,
        gossip_lazy: 3,
    }
}

/// Build a gossipsub swarm with DOLI's config (short cache TTL override).
async fn build_test_swarm() -> Swarm<gossipsub::Behaviour> {
    std::env::set_var("DOLI_IP_COLOCATION_THRESHOLD", "500");

    libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            Default::default(),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )
        .expect("TCP transport")
        .with_behaviour(|key| {
            new_gossipsub_with_cache_time(key, &test_mesh(), TEST_CACHE_TTL)
                .expect("gossipsub config")
        })
        .expect("behaviour")
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
        .build()
}

/// Listen on a random localhost port and return the actual listen address.
async fn listen_on_localhost(swarm: &mut Swarm<gossipsub::Behaviour>) -> Multiaddr {
    let listen_addr: Multiaddr = "/ip4/127.0.0.1/tcp/0".parse().unwrap();
    swarm.listen_on(listen_addr).unwrap();

    loop {
        if let SwarmEvent::NewListenAddr { address, .. } = swarm.select_next_some().await {
            return address;
        }
    }
}

/// Drive all swarms, applying the classifier verdict at each relay hop.
///
/// Unlike INC-I-114's `drive_all_collect_on_c` which uses a boolean
/// `report_accept` flag, this function calls `classify_producer_gossip` on
/// each received message to compute the Accept/Ignore verdict — exercising
/// the REAL classifier decision at each relay.
async fn drive_with_classifier(
    a: &mut Swarm<gossipsub::Behaviour>,
    b: &mut Swarm<gossipsub::Behaviour>,
    c: &mut Swarm<gossipsub::Behaviour>,
    wait: Duration,
    fixed_now: u64,
) -> Vec<(PeerId, MessageId, Vec<u8>)> {
    let mut messages_on_c = Vec::new();
    let _ = tokio::time::timeout(wait, async {
        loop {
            tokio::select! {
                event = a.select_next_some() => {
                    if let SwarmEvent::Behaviour(gossipsub::Event::Message {
                        message_id,
                        propagation_source,
                        message,
                    }) = event {
                        let verdict = classify_producer_gossip(&message.data, fixed_now);
                        let _ = a.behaviour_mut().report_message_validation_result(
                            &message_id,
                            &propagation_source,
                            verdict,
                        );
                    }
                }
                event = b.select_next_some() => {
                    if let SwarmEvent::Behaviour(gossipsub::Event::Message {
                        message_id,
                        propagation_source,
                        message,
                    }) = event {
                        let verdict = classify_producer_gossip(&message.data, fixed_now);
                        eprintln!(
                            "[B] message_id={} from={} verdict={}",
                            message_id,
                            propagation_source,
                            if is_accept(classify_producer_gossip(&message.data, fixed_now)) {
                                "Accept"
                            } else {
                                "Ignore"
                            }
                        );
                        let _ = b.behaviour_mut().report_message_validation_result(
                            &message_id,
                            &propagation_source,
                            verdict,
                        );
                    }
                }
                event = c.select_next_some() => {
                    if let SwarmEvent::Behaviour(gossipsub::Event::Message {
                        propagation_source,
                        message_id,
                        message,
                    }) = event {
                        let verdict = classify_producer_gossip(&message.data, fixed_now);
                        let _ = c.behaviour_mut().report_message_validation_result(
                            &message_id,
                            &propagation_source,
                            verdict,
                        );
                        messages_on_c.push((
                            propagation_source,
                            message_id,
                            message.data.clone(),
                        ));
                    }
                }
            }
        }
    })
    .await;
    messages_on_c
}

// Requirement: INC-I-137 (Must)
// Acceptance: Fresh producer-set announcement propagates A->B->C (convergence
//             preserved). Stale producer-set announcement is suppressed at B
//             (storm stopped).
// Path: P2 + P1 end-to-end / Input Partitions: IP-9 + IP-10
//
// This integration test mirrors the INC-I-114 harness topology (A--B--C) but
// uses `classify_producer_gossip` at each relay hop instead of hardcoded booleans.
// Fixed timestamps avoid wall-clock flakiness.
#[tokio::test]
async fn test_inc_i_137_fresh_producer_set_still_converges() {
    // ── Build 3 swarms (A--B--C topology) ─────────────────────────────
    let mut swarm_a = build_test_swarm().await;
    let mut swarm_b = build_test_swarm().await;
    let mut swarm_c = build_test_swarm().await;

    let peer_a = *swarm_a.local_peer_id();
    let peer_b = *swarm_b.local_peer_id();
    let peer_c = *swarm_c.local_peer_id();

    eprintln!("Peer A: {peer_a}");
    eprintln!("Peer B: {peer_b}");
    eprintln!("Peer C: {peer_c}");

    // ── Listen ──────────────────────────────────────────────────────────
    let addr_b = listen_on_localhost(&mut swarm_b).await;
    let _addr_a = listen_on_localhost(&mut swarm_a).await;
    let _addr_c = listen_on_localhost(&mut swarm_c).await;

    // ── Subscribe all to producers topic ────────────────────────────────
    let topic = IdentTopic::new(TEST_TOPIC);
    swarm_a.behaviour_mut().subscribe(&topic).unwrap();
    swarm_b.behaviour_mut().subscribe(&topic).unwrap();
    swarm_c.behaviour_mut().subscribe(&topic).unwrap();

    // ── Connect: A->B, C->B (line topology through B) ──────────────────
    swarm_a.dial(addr_b.clone()).unwrap();
    swarm_c.dial(addr_b.clone()).unwrap();

    // Drive swarms to establish connections + mesh via heartbeats.
    eprintln!("Waiting for mesh formation (5s)...");
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            tokio::select! {
                _ = swarm_a.select_next_some() => {}
                _ = swarm_b.select_next_some() => {}
                _ = swarm_c.select_next_some() => {}
            }
        }
    })
    .await;

    // Verify topology: A not directly connected to C
    let a_peers: Vec<_> = swarm_a.connected_peers().cloned().collect();
    assert!(
        !a_peers.contains(&peer_c),
        "A must NOT be directly connected to C (topology violation)"
    );
    let b_peers: Vec<_> = swarm_b.connected_peers().cloned().collect();
    assert!(b_peers.contains(&peer_a), "B must be connected to A");
    assert!(b_peers.contains(&peer_c), "B must be connected to C");
    eprintln!("Topology verified: A--B--C");

    // ══════════════════════════════════════════════════════════════════════
    // Phase FRESH (IP-9): A publishes a FRESH producer-set snapshot.
    // B receives, classifier returns Accept, forwards to C.
    // Verifies CRDT convergence is preserved.
    // ══════════════════════════════════════════════════════════════════════
    eprintln!("\n=== Phase FRESH: A publishes fresh producer-set snapshot ===");

    let fixed_now: u64 = 10_000_000;
    let fresh_ts = fixed_now - 10; // 10 seconds old, well within TTL
    let fresh_anns = vec![
        make_announcement(fresh_ts),
        make_announcement(fresh_ts),
        make_announcement(fresh_ts),
    ];
    let fresh_data = encode_producer_set(&fresh_anns);

    let publish_result = swarm_a
        .behaviour_mut()
        .publish(topic.clone(), fresh_data.clone());
    assert!(
        publish_result.is_ok(),
        "A must be able to publish fresh producer set: {:?}",
        publish_result
    );
    eprintln!(
        "A published fresh producer-set snapshot (3 anns, ts={})",
        fresh_ts
    );

    let phase_fresh_msgs = drive_with_classifier(
        &mut swarm_a,
        &mut swarm_b,
        &mut swarm_c,
        Duration::from_secs(5),
        fixed_now,
    )
    .await;

    let fresh_matching: Vec<_> = phase_fresh_msgs
        .iter()
        .filter(|(_, _, data)| data == &fresh_data)
        .collect();

    assert_eq!(
        fresh_matching.len(),
        1,
        "Phase FRESH FAILED: C should receive the fresh producer-set snapshot exactly \
         once (CRDT convergence preserved), got {} deliveries.",
        fresh_matching.len()
    );

    let (source, _, _) = &fresh_matching[0];
    assert_eq!(
        *source, peer_b,
        "C should receive the fresh snapshot from B (relay), not directly from A"
    );
    eprintln!("Phase FRESH PASS: C received fresh producer-set once from B.");

    // ══════════════════════════════════════════════════════════════════════
    // Phase STALE (IP-10): A publishes an ALL-STALE producer-set snapshot.
    // B receives, classifier returns Ignore, does NOT forward to C.
    // Verifies gossip storm suppression.
    // ══════════════════════════════════════════════════════════════════════
    eprintln!("\n=== Phase STALE: A publishes all-stale producer-set snapshot ===");

    let stale_ts = fixed_now - 5000; // 5000 seconds old, well past 3600 TTL
    let stale_anns = vec![
        make_announcement(stale_ts),
        make_announcement(stale_ts),
        make_announcement(stale_ts),
    ];
    let stale_data = encode_producer_set(&stale_anns);

    let publish_stale = swarm_a
        .behaviour_mut()
        .publish(topic.clone(), stale_data.clone());
    assert!(
        publish_stale.is_ok(),
        "A must be able to publish stale producer set: {:?}",
        publish_stale
    );
    eprintln!(
        "A published stale producer-set snapshot (3 anns, ts={})",
        stale_ts
    );

    let phase_stale_msgs = drive_with_classifier(
        &mut swarm_a,
        &mut swarm_b,
        &mut swarm_c,
        Duration::from_secs(5),
        fixed_now,
    )
    .await;

    let stale_matching: Vec<_> = phase_stale_msgs
        .iter()
        .filter(|(_, _, data)| data == &stale_data)
        .collect();

    assert_eq!(
        stale_matching.len(),
        0,
        "Phase STALE FAILED: C received {} stale producer-set snapshots. \
         With classify_producer_gossip returning Ignore for all-stale sets, \
         B should NOT forward the message to C (storm suppressed).",
        stale_matching.len()
    );

    eprintln!(
        "Phase STALE PASS: C did NOT receive the stale producer-set snapshot. \
         Gossip storm suppressed by classify_producer_gossip."
    );
}
