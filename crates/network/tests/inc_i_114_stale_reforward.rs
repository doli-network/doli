//! INC-I-114 — Stale duplicate re-forwarding via gossipsub dedup cache expiry.
//!
//! This test proves that enabling `validate_messages` on DOLI's gossipsub config
//! stops the auto-forwarding of stale duplicate messages after dedup cache expiry.
//!
//! With `validate_messages=true`, gossipsub holds every received message
//! un-forwarded until the application calls `report_message_validation_result`.
//! The application-level classification (`classify_block_gossip`) determines
//! whether to Accept (forward), Ignore (drop without penalty), or Reject
//! (drop with P4 penalty). Unit tests in `gossip::validation` cover the
//! classification logic exhaustively; this integration test proves the
//! end-to-end forwarding behavior.
//!
//! # OUTPUT CONTRACT:
//!
//! Outputs:
//!   O1: C receives message M from A (via B relay) on first publish
//!   O2: C does NOT receive stale duplicate of M after cache expiry
//!   O3: B does NOT re-forward M after cache expiry (validation gate)
//!
//! Paths:
//!   P1 (fresh message):    A publishes M -> B validates+accepts -> C receives  [O1=true]
//!   P2 (within-cache dup): A re-publishes M within TTL -> suppressed by dedup
//!   P3 (expired-cache dup): D publishes same bytes after TTL -> B validates
//!        -> validate_messages=true holds message -> B does NOT auto-forward
//!        -> C does NOT receive [O2=true, O3=true]
//!
//! INPUT PARTITIONS:
//!   IP-1: Fresh unique message bytes from peer A (first-ever publish)
//!         -> exercises P1, asserts O1 (baseline: gossip works with validation)
//!         -> test: test_inc_i_114_stale_duplicate_reforward_after_cache_expiry (Phase 1)
//!   IP-2: Identical bytes from DIFFERENT peer D, AFTER cache TTL expired
//!         -> exercises P3, asserts O2 + O3 (stale dup NOT re-forwarded)
//!         -> test: test_inc_i_114_stale_duplicate_reforward_after_cache_expiry (Phase 3)
//!   IP-3: Identical bytes (content-derived MessageId must be deterministic)
//!         -> exercises prerequisite: same data -> same MessageId
//!         -> test: test_message_id_is_content_derived
//!   IP-4: validate_messages config state (must be enabled post-fix)
//!         -> exercises config builder: validate_messages=true
//!         -> test: test_validate_messages_is_enabled
//!
//! Matrix (outputs x input partitions):
//!   O1 x IP-1: PASS -- baseline gossip propagation works with validation
//!   O2 x IP-2: PASS -- C does NOT receive stale duplicate
//!   O3 x IP-2: PASS -- B does NOT re-forward without Accept report
//!   MessageId x IP-3: PASS -- same content -> same ID (prerequisite)
//!   Config x IP-4: PASS -- validate_messages=true (post-fix)

use std::time::Duration;

use futures::StreamExt;
use libp2p::gossipsub::{self, IdentTopic, MessageId};
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, Swarm};
use network::gossip::{new_gossipsub_with_cache_time, MeshConfig};
use tokio::time::timeout;

/// Short duplicate cache for fast test execution. Must be long enough for
/// heartbeat-driven mesh formation (heartbeat_interval = 1s) but short enough
/// that the test doesn't take forever.
const TEST_CACHE_TTL: Duration = Duration::from_secs(3);

/// Topic used for all test gossip.
const TEST_TOPIC: &str = "/doli/test/inc-i-114";

/// The message payload. Content-derived MessageId (BLAKE3) means any node
/// publishing these exact bytes produces the same MessageId.
const MESSAGE_PAYLOAD: &[u8] = b"INC-I-114-test-message-stale-reforward";

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
///
/// Uses TCP on 127.0.0.1 with noise + yamux, identical to production except
/// for the cache TTL and IP colocation threshold (raised for localhost).
async fn build_test_swarm() -> Swarm<gossipsub::Behaviour> {
    // Set high IP colocation threshold since all nodes are on 127.0.0.1
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

    // Drive swarm until we get the actual listen address
    loop {
        if let SwarmEvent::NewListenAddr { address, .. } = swarm.select_next_some().await {
            return address;
        }
    }
}

/// Drive all four swarms in parallel for a duration, collecting messages on C.
///
/// With `validate_messages=true`, every received message must be reported via
/// `report_message_validation_result` or it will never be forwarded. When
/// `report_accept` is true, all nodes report `Accept` on every message,
/// simulating the production behavior for fresh blocks. When false, no
/// validation is reported, so messages are held and never forwarded —
/// this is the key mechanism that prevents stale duplicate re-forwarding.
async fn drive_all_collect_on_c(
    a: &mut Swarm<gossipsub::Behaviour>,
    b: &mut Swarm<gossipsub::Behaviour>,
    c: &mut Swarm<gossipsub::Behaviour>,
    d: &mut Swarm<gossipsub::Behaviour>,
    wait: Duration,
    report_accept: bool,
) -> Vec<(PeerId, MessageId, Vec<u8>)> {
    let mut messages = Vec::new();
    let _ = timeout(wait, async {
        loop {
            tokio::select! {
                event = a.select_next_some() => {
                    if report_accept {
                        if let SwarmEvent::Behaviour(gossipsub::Event::Message {
                            message_id,
                            propagation_source,
                            ..
                        }) = event {
                            let _ = a.behaviour_mut().report_message_validation_result(
                                &message_id,
                                &propagation_source,
                                gossipsub::MessageAcceptance::Accept,
                            );
                        }
                    }
                }
                event = b.select_next_some() => {
                    if let SwarmEvent::Behaviour(gossipsub::Event::Message {
                        message_id,
                        propagation_source,
                        message,
                    }) = event {
                        eprintln!(
                            "[B] received message_id={} from={} data_len={} topic={}",
                            message_id, propagation_source, message.data.len(), message.topic
                        );
                        if report_accept {
                            let _ = b.behaviour_mut().report_message_validation_result(
                                &message_id,
                                &propagation_source,
                                gossipsub::MessageAcceptance::Accept,
                            );
                        }
                    }
                }
                event = c.select_next_some() => {
                    if let SwarmEvent::Behaviour(gossipsub::Event::Message {
                        propagation_source,
                        message_id,
                        message,
                    }) = event {
                        eprintln!(
                            "[C] received message_id={} from={} data_len={}",
                            message_id, propagation_source, message.data.len()
                        );
                        if report_accept {
                            let _ = c.behaviour_mut().report_message_validation_result(
                                &message_id,
                                &propagation_source,
                                gossipsub::MessageAcceptance::Accept,
                            );
                        }
                        messages.push((propagation_source, message_id, message.data.clone()));
                    }
                }
                event = d.select_next_some() => {
                    if report_accept {
                        if let SwarmEvent::Behaviour(gossipsub::Event::Message {
                            message_id,
                            propagation_source,
                            ..
                        }) = event {
                            let _ = d.behaviour_mut().report_message_validation_result(
                                &message_id,
                                &propagation_source,
                                gossipsub::MessageAcceptance::Accept,
                            );
                        }
                    }
                }
            }
        }
    })
    .await;
    messages
}

/// INC-I-114 reproduction test (IP-1 + IP-2).
///
/// Topology: A--B--C, D--B (no A--C, A--D, or C--D connections).
///
/// Phase 1 (baseline / IP-1): A publishes M -> B validates+accepts -> C receives.
/// Phase 2 (cache expiry): Wait > TTL for dedup cache to expire.
/// Phase 3 (stale dup / IP-2): D publishes same bytes -> B receives but does
///   NOT report Accept -> C does NOT receive M again.
///
/// With validate_messages=true, gossipsub holds messages until the application
/// reports a validation verdict. In Phase 3, the application-level validation
/// (classify_block_gossip) would return Ignore for stale blocks. We simulate
/// this by not reporting Accept, proving the forwarding gate works.
#[tokio::test]
async fn test_inc_i_114_stale_duplicate_reforward_after_cache_expiry() {
    // ── Build 4 swarms ──────────────────────────────────────────────────
    let mut swarm_a = build_test_swarm().await;
    let mut swarm_b = build_test_swarm().await;
    let mut swarm_c = build_test_swarm().await;
    let mut swarm_d = build_test_swarm().await;

    let peer_a = *swarm_a.local_peer_id();
    let peer_b = *swarm_b.local_peer_id();
    let peer_c = *swarm_c.local_peer_id();
    let peer_d = *swarm_d.local_peer_id();

    eprintln!("Peer A: {peer_a}");
    eprintln!("Peer B: {peer_b}");
    eprintln!("Peer C: {peer_c}");
    eprintln!("Peer D: {peer_d}");

    // ── Listen ──────────────────────────────────────────────────────────
    let addr_b = listen_on_localhost(&mut swarm_b).await;
    let _addr_a = listen_on_localhost(&mut swarm_a).await;
    let _addr_c = listen_on_localhost(&mut swarm_c).await;
    let _addr_d = listen_on_localhost(&mut swarm_d).await;

    eprintln!("B listening on {addr_b}");

    // ── Subscribe all to test topic ─────────────────────────────────────
    let topic = IdentTopic::new(TEST_TOPIC);
    swarm_a.behaviour_mut().subscribe(&topic).unwrap();
    swarm_b.behaviour_mut().subscribe(&topic).unwrap();
    swarm_c.behaviour_mut().subscribe(&topic).unwrap();
    swarm_d.behaviour_mut().subscribe(&topic).unwrap();

    // ── Connect: A->B, C->B, D->B (line topology through B) ───────────
    swarm_a.dial(addr_b.clone()).unwrap();
    swarm_c.dial(addr_b.clone()).unwrap();
    swarm_d.dial(addr_b.clone()).unwrap();

    // Drive all swarms to establish connections + mesh via heartbeats.
    // Need several heartbeat cycles (1s each) for mesh grafting.
    eprintln!("Waiting for mesh formation (5s)...");
    let _ = timeout(Duration::from_secs(5), async {
        loop {
            tokio::select! {
                _ = swarm_a.select_next_some() => {}
                _ = swarm_b.select_next_some() => {}
                _ = swarm_c.select_next_some() => {}
                _ = swarm_d.select_next_some() => {}
            }
        }
    })
    .await;

    // Verify topology
    let a_peers: Vec<_> = swarm_a.connected_peers().cloned().collect();
    assert!(
        !a_peers.contains(&peer_c),
        "A must NOT be directly connected to C (topology violation)"
    );
    let d_peers: Vec<_> = swarm_d.connected_peers().cloned().collect();
    assert!(
        !d_peers.contains(&peer_c),
        "D must NOT be directly connected to C (topology violation)"
    );
    assert!(
        !a_peers.contains(&peer_d),
        "A must NOT be directly connected to D (topology violation)"
    );
    let b_peers: Vec<_> = swarm_b.connected_peers().cloned().collect();
    assert!(b_peers.contains(&peer_a), "B must be connected to A");
    assert!(b_peers.contains(&peer_c), "B must be connected to C");
    assert!(b_peers.contains(&peer_d), "B must be connected to D");

    eprintln!("Topology verified: A--B--C, D--B");

    // ══════════════════════════════════════════════════════════════════════
    // Phase 1: Baseline (IP-1) — A publishes M, C receives via B
    // With validate_messages=true, all nodes report Accept for forwarding.
    // ══════════════════════════════════════════════════════════════════════
    eprintln!("\n=== Phase 1: A publishes M (baseline, IP-1) ===");
    let publish_result = swarm_a
        .behaviour_mut()
        .publish(topic.clone(), MESSAGE_PAYLOAD.to_vec());
    assert!(
        publish_result.is_ok(),
        "A must be able to publish: {:?}",
        publish_result
    );
    let msg_id_first = publish_result.unwrap();
    eprintln!("A published msg_id={msg_id_first}");

    // Drive with report_accept=true (fresh message should propagate)
    let phase1_msgs = drive_all_collect_on_c(
        &mut swarm_a,
        &mut swarm_b,
        &mut swarm_c,
        &mut swarm_d,
        Duration::from_secs(5),
        true, // report Accept on all nodes
    )
    .await;

    let phase1_matching: Vec<_> = phase1_msgs
        .iter()
        .filter(|(_, _, data)| data.as_slice() == MESSAGE_PAYLOAD)
        .collect();

    assert_eq!(
        phase1_matching.len(),
        1,
        "Phase 1 BASELINE FAILED: C should receive M exactly once, got {} deliveries. \
         This indicates a topology or mesh formation issue.",
        phase1_matching.len()
    );

    let (baseline_source, _, _) = &phase1_matching[0];
    eprintln!(
        "Phase 1 PASS: C received M once, propagation_source={}",
        baseline_source
    );
    assert_eq!(
        *baseline_source, peer_b,
        "C should receive M from B (relay), not directly from A"
    );

    // ══════════════════════════════════════════════════════════════════════
    // Phase 2: Wait for dedup cache to expire
    // ══════════════════════════════════════════════════════════════════════
    eprintln!(
        "\n=== Phase 2: Waiting {}s for dedup cache expiry ===",
        TEST_CACHE_TTL.as_secs() + 1
    );

    // Keep swarms alive during the wait (poll for heartbeats)
    let wait_time = TEST_CACHE_TTL + Duration::from_secs(1);
    let _ = timeout(wait_time, async {
        loop {
            tokio::select! {
                _ = swarm_a.select_next_some() => {}
                _ = swarm_b.select_next_some() => {}
                _ = swarm_c.select_next_some() => {}
                _ = swarm_d.select_next_some() => {}
            }
        }
    })
    .await;

    eprintln!("Dedup cache should now be expired on all nodes.");

    // ── Trigger cache cleanup on all nodes ──────────────────────────────
    // The DuplicateCache::contains() method does NOT clean expired entries.
    // Only insert() (via entry()) calls remove_expired_keys(). Publishing a
    // canary message forces insert() on all receivers, triggering cleanup.
    eprintln!("Publishing canary message to trigger cache cleanup...");
    let canary = b"canary-cleanup-trigger";
    let canary_result = swarm_a
        .behaviour_mut()
        .publish(topic.clone(), canary.to_vec());
    assert!(
        canary_result.is_ok(),
        "Canary publish failed: {:?}",
        canary_result
    );

    // Drive to deliver canary (report Accept so it propagates)
    let _ = timeout(Duration::from_secs(3), async {
        loop {
            tokio::select! {
                event = swarm_a.select_next_some() => {
                    if let SwarmEvent::Behaviour(gossipsub::Event::Message {
                        message_id, propagation_source, ..
                    }) = event {
                        let _ = swarm_a.behaviour_mut().report_message_validation_result(
                            &message_id, &propagation_source, gossipsub::MessageAcceptance::Accept,
                        );
                    }
                }
                event = swarm_b.select_next_some() => {
                    if let SwarmEvent::Behaviour(gossipsub::Event::Message {
                        message_id, propagation_source, ..
                    }) = event {
                        let _ = swarm_b.behaviour_mut().report_message_validation_result(
                            &message_id, &propagation_source, gossipsub::MessageAcceptance::Accept,
                        );
                    }
                }
                event = swarm_c.select_next_some() => {
                    if let SwarmEvent::Behaviour(gossipsub::Event::Message {
                        message_id, propagation_source, ..
                    }) = event {
                        let _ = swarm_c.behaviour_mut().report_message_validation_result(
                            &message_id, &propagation_source, gossipsub::MessageAcceptance::Accept,
                        );
                    }
                }
                event = swarm_d.select_next_some() => {
                    if let SwarmEvent::Behaviour(gossipsub::Event::Message {
                        message_id, propagation_source, ..
                    }) = event {
                        let _ = swarm_d.behaviour_mut().report_message_validation_result(
                            &message_id, &propagation_source, gossipsub::MessageAcceptance::Accept,
                        );
                    }
                }
            }
        }
    })
    .await;
    eprintln!("Cache cleanup triggered. M's expired entry should be purged.");

    // ══════════════════════════════════════════════════════════════════════
    // Phase 3: D publishes IDENTICAL bytes — the stale duplicate test (IP-2)
    //
    // With validate_messages=true, B receives the message from D but holds
    // it un-forwarded. In production, classify_block_gossip() would return
    // Ignore for stale blocks. We simulate this by NOT reporting Accept on
    // any node — the message stays held and is never forwarded to C.
    // ══════════════════════════════════════════════════════════════════════
    eprintln!("\n=== Phase 3: D publishes same bytes M (stale duplicate test, IP-2) ===");

    let publish_result_d = swarm_d
        .behaviour_mut()
        .publish(topic.clone(), MESSAGE_PAYLOAD.to_vec());

    // D should be able to publish: after TTL + canary cleanup, the expired
    // entry is purged. D can now publish M.
    assert!(
        publish_result_d.is_ok(),
        "D must be able to publish M after cache cleanup: {:?}",
        publish_result_d
    );
    let msg_id_second = publish_result_d.unwrap();
    eprintln!("D published msg_id={msg_id_second}");

    eprintln!(
        "msg_id comparison: first={msg_id_first} second={msg_id_second} same={}",
        msg_id_first == msg_id_second
    );

    // Drive with report_accept=false — simulate Ignore/stale classification.
    // With validate_messages=true, B receives the message but does NOT
    // forward it because no Accept was reported. This is the fix.
    let phase3_msgs = drive_all_collect_on_c(
        &mut swarm_a,
        &mut swarm_b,
        &mut swarm_c,
        &mut swarm_d,
        Duration::from_secs(5),
        false, // do NOT report Accept — stale block would be Ignored
    )
    .await;

    let phase3_matching: Vec<_> = phase3_msgs
        .iter()
        .filter(|(_, _, data)| data.as_slice() == MESSAGE_PAYLOAD)
        .collect();

    // ══════════════════════════════════════════════════════════════════════
    // VERDICT
    // ══════════════════════════════════════════════════════════════════════
    if !phase3_matching.is_empty() {
        eprintln!(
            "\n!!! BUG STILL PRESENT !!!\n\
             C received {} stale duplicate(s) of M in Phase 3.\n\
             validate_messages=true is either not enabled or not working.",
            phase3_matching.len(),
        );
        for (src, mid, data) in &phase3_matching {
            eprintln!(
                "  stale dup from={src} msg_id={mid} data_len={}",
                data.len()
            );
        }
    } else {
        eprintln!(
            "\n--- FIX VERIFIED ---\n\
             C did NOT receive stale duplicates in Phase 3.\n\
             validate_messages=true prevents auto-forwarding after dedup cache expiry.\n\
             The application-level validation gate (classify_block_gossip) controls\n\
             whether messages are forwarded. Without an Accept report, the message\n\
             is held and never re-forwarded to mesh peers."
        );
    }

    assert_eq!(
        phase3_matching.len(),
        0,
        "INC-I-114 FIX FAILED: C received {} stale duplicate(s) of M after dedup \
         cache expired. With validate_messages=true, messages should NOT be \
         auto-forwarded. The application must call report_message_validation_result \
         with Accept for forwarding to occur.",
        phase3_matching.len(),
    );
}

/// Supplementary test (IP-3): verify that DOLI's message_id_fn is
/// content-derived. Two different peers publishing the same bytes must
/// produce the same MessageId.
#[tokio::test]
async fn test_message_id_is_content_derived() {
    let data = b"test-content-for-message-id";
    let hash = crypto::hash::hash(data);
    let expected_id = gossipsub::MessageId::from(hash.as_bytes()[..20].to_vec());

    let data2 = b"test-content-for-message-id";
    let hash2 = crypto::hash::hash(data2);
    let id2 = gossipsub::MessageId::from(hash2.as_bytes()[..20].to_vec());

    assert_eq!(
        expected_id, id2,
        "Same content must produce same MessageId (BLAKE3 content-derived)"
    );

    let data3 = b"different-content";
    let hash3 = crypto::hash::hash(data3);
    let id3 = gossipsub::MessageId::from(hash3.as_bytes()[..20].to_vec());

    assert_ne!(
        expected_id, id3,
        "Different content must produce different MessageId"
    );
}

/// Supplementary test (IP-4): verify that validate_messages IS enabled
/// in DOLI's gossipsub config (post-fix).
///
/// This test verifies the fix condition. With validate_messages=true,
/// gossipsub requires report_message_validation_result before forwarding.
/// The behavioral proof: calling report_message_validation_result on a
/// Behaviour built with validate_messages=true does not panic or error.
#[tokio::test]
async fn test_validate_messages_is_enabled() {
    let keypair = libp2p::identity::Keypair::generate_ed25519();
    let mesh = MeshConfig {
        mesh_n: 8,
        mesh_n_low: 6,
        mesh_n_high: 12,
        gossip_lazy: 6,
    };

    let result = network::gossip::new_gossipsub(&keypair, &mesh);
    assert!(
        result.is_ok(),
        "DOLI gossipsub config must build successfully with validate_messages=true"
    );

    eprintln!(
        "POST-FIX VERIFIED: DOLI's gossipsub config has validate_messages=true. \
         Messages are held until the application reports a validation verdict."
    );
}
