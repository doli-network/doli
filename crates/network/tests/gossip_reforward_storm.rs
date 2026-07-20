//! INC-I-142 M7 — multi-node gossip "re-forward storm" integration test.
//!
//! Proves the unified application-level staleness gate ([`classify_gossip`] +
//! a **persistent** [`SeenCache`]) closes the INC-I-142 re-delivery storm by
//! IDENTITY dedup, INDEPENDENT of libp2p's `duplicate_cache_time`.
//!
//! # OUTPUT CONTRACT: the relay-forwarding behaviour of `classify_gossip` as
//! # wired into a live gossipsub relay (function under test at integration level:
//! # `classify_gossip(topic, data, &mut StalenessCtx{ seen: &mut SeenCache, .. })`)
//!
//! Observable outputs (as exercised through the relay):
//!   O1: return `MessageAcceptance` at the relay (B) → Accept ⇒ gossipsub
//!       forwards to mesh peers; Ignore ⇒ NO forward. Observed as `b_verdicts`.
//!   O2: mutation of the shared `&mut SeenCache` (`ctx.seen`) — recorded on the
//!       FIRST Accept, causing every later same-identity delivery to Ignore.
//!       Observed indirectly: the re-delivery verdict flips Accept→Ignore.
//!   O3: downstream re-forward count = Message deliveries at C and D past the
//!       publisher (the network-observable consequence of O1). Observed as
//!       `c_count` / `d_count`.
//!   (No Reject is ever produced — fail-open discipline; not separately asserted
//!    here, covered by the staleness unit suite.)
//!
//! Paths:
//!   P1 (first sight, genuinely-new identity): SeenCache empty for key,
//!       payload in-window → O1=Accept, O2=record, O3=forward once to each peer.
//!   P2 (same-identity re-delivery within TTL, PERSISTENT cache): key already in
//!       SeenCache → O1=Ignore, O2=no-op, O3=0 re-forwards (STORM CLOSED).
//!   P3 (same-identity re-delivery, NO persistent cache — pre-M6 baseline):
//!       relay unconditionally O1=Accept → O3>0 re-forwards (STORM OPEN).
//!
//! INPUT PARTITIONS (topic × relay policy × delivery ordinal):
//!   IP-1 attestations, persistent gate, 1st delivery → P1
//!        → assert O1=Accept, O3: C==1 & D==1 (CONTROL).
//!   IP-2 attestations, persistent gate, re-delivery → P2
//!        → assert O1=Ignore, O3: C==0 & D==0 (STORM-CLOSURE).
//!   IP-3 attestations, unconditional-Accept baseline, re-delivery → P3
//!        → assert O1=Accept, O3: C+D>0 (PRE-M6 REPRODUCTION).
//!   IP-4 heartbeats, persistent gate, 1st delivery → P1 (CONTROL).
//!   IP-5 heartbeats, persistent gate, re-delivery → P2 (STORM-CLOSURE).
//!   IP-6 heartbeats, unconditional-Accept baseline, re-delivery → P3 (REPRO).
//!
//! Matrix (outputs × paths): each gate test covers {P1: O1,O3} + {P2: O1,O2,O3};
//! each baseline test covers {P1 sanity: O3} + {P3: O1,O3}. A vacuity guard
//! (relay MUST re-receive the re-delivery) prevents P2's `O3==0` from passing
//! trivially. The FAIL→PASS evidence is structural: the IP-2/IP-5 assertion
//! `O3==0` would FAIL under the IP-3/IP-6 baseline policy (`O3>0`) and PASSES
//! under the persistent gate.
//!
//! # Topology (4 honest nodes + a re-injector, one relay)
//!
//! ```text
//!   A (publisher)     E (re-injector, models a peer re-forwarding X)
//!         \          /
//!          B (relay — applies the gate)
//!         / \
//!        C   D   (downstream honest nodes)
//! ```
//!
//! A, E, C, D are each connected ONLY to B. B is therefore the sole RELAY between
//! any publisher and the two honest downstream nodes. A single forward by B
//! delivers to exactly {C, D}, so the number of downstream Message events for a
//! given identity IS the relay's re-forward count. E is subscribed to nothing
//! (see below) so B never forwards to it.
//!
//! # The re-delivery window (load-bearing)
//!
//! Swarms use a SHORT `duplicate_cache_time` (`TEST_CACHE_TTL`, 2s). A publishes a
//! payload; after the cache window expires, **E** re-publishes the IDENTICAL
//! bytes. Because content-based (BLAKE3) message-ids make identical bytes the same
//! wire identity, and B's receive-path duplicate cache has expired, libp2p
//! RE-DELIVERS the message to B's application layer rather than internally
//! deduping — reproducing exactly the 60–120s escape window that the M9
//! `duplicate_cache_time` backstop is intentionally NOT relied upon to close.
//!
//! ## Why a separate re-injector E (not A re-publishing)
//!
//! libp2p-gossipsub's `Behaviour::publish` rejects a re-publish of an identical
//! message with `PublishError::Duplicate` by checking `duplicate_cache.contains`
//! — and `DuplicateCache::contains` (time_cache.rs) does NOT purge expired
//! entries; only `insert` does. The original publisher never `insert`s again, so
//! its expired entry lingers and re-publishing the same bytes ALWAYS fails,
//! regardless of elapsed time. The RECEIVE path uses `insert` (which purges), so
//! a relay DOES re-deliver a re-received message. Modelling the re-delivery as a
//! DIFFERENT peer (E) re-forwarding X back to the relay is therefore both
//! necessary AND a more faithful model of the real storm (peers bouncing the same
//! message off each other after the dedup window). E is left UNSUBSCRIBED so B
//! never forwards X to it in phase 1 → E's own duplicate cache stays empty for X →
//! E's phase-2 publish succeeds.

use std::time::Duration;

use futures::StreamExt;
use libp2p::gossipsub::{self, IdentTopic, MessageAcceptance};
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, Swarm};

use doli_core::{Attestation, Heartbeat};

use network::gossip::staleness::{
    classify_gossip, GossipTopic, SeenCache, StalenessCtx, SEEN_CACHE_CAPACITY, SEEN_CACHE_TTL_SECS,
};
use network::gossip::{
    new_gossipsub_with_cache_time, MeshConfig, ATTESTATION_TOPIC, HEARTBEATS_TOPIC,
};

// ============================================================================
// Fixed deterministic clock (mirrors the staleness unit-test fixture:
// genesis=1000, slot_duration=10, now=2000 → wall_slot=100).
// ============================================================================

const GENESIS: u64 = 1000;
const SLOT_DUR: u64 = 10;
const NOW_UNIX: u64 = 2000;
const WALL: u32 = 100;

/// Short duplicate cache to FORCE the libp2p re-delivery escape window — this is
/// load-bearing: it proves storm closure is independent of `duplicate_cache_time`.
const TEST_CACHE_TTL: Duration = Duration::from_secs(2);
/// Time budget for mesh (GRAFT) formation.
const MESH_FORM: Duration = Duration::from_secs(5);
/// Per-phase drive window (propagation on localhost is sub-second).
const PHASE_DRIVE: Duration = Duration::from_secs(5);
/// Idle gap between phases; MUST exceed `TEST_CACHE_TTL` on every node so the
/// re-publish of identical bytes re-delivers instead of being deduped.
const GAP: Duration = Duration::from_secs(4);

// ============================================================================
// Payload builders (identical construction to staleness.rs unit tests, so the
// classifier decodes them; fixed identities → reproducible wire bytes).
// ============================================================================

/// A fresh, current-slot attestation with a fixed `(attester, block_hash)`
/// identity. Re-serializing the same struct yields byte-identical output → same
/// wire identity AND same app-level dedup key.
fn attestation_bytes() -> Vec<u8> {
    // attester byte 0 / block_hash ZERO: identical construction to the proven
    // staleness `make_attestation` unit fixture, so `from_bytes` round-trips
    // (a non-canonical PublicKey fails bincode deserialize → fail-open, which
    // would silently defeat the identity gate).
    Attestation {
        block_hash: crypto::Hash::ZERO,
        slot: WALL,
        height: 0,
        attester: crypto::PublicKey::from_bytes([0u8; 32]),
        attester_weight: 0,
        signature: crypto::Signature::from_bytes([0u8; 64]),
        bls_signature: Vec::new(),
    }
    .to_bytes()
}

/// A fresh, current-slot heartbeat with a fixed `(producer, slot)` identity.
fn heartbeat_bytes() -> Vec<u8> {
    Heartbeat {
        version: 1,
        producer: crypto::PublicKey::from_bytes([0u8; 32]),
        slot: WALL,
        prev_block_hash: crypto::Hash::ZERO,
        vdf_output: [0u8; 32],
        signature: crypto::Signature::from_bytes([0u8; 64]),
        witnesses: Vec::new(),
    }
    .serialize()
}

// ============================================================================
// Swarm harness (copied from inc_i_137_producer_gossip_ttl.rs::build_test_swarm)
// ============================================================================

fn test_mesh() -> MeshConfig {
    MeshConfig {
        mesh_n: 3,
        mesh_n_low: 2,
        mesh_n_high: 6,
        gossip_lazy: 3,
    }
}

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

async fn listen_on_localhost(swarm: &mut Swarm<gossipsub::Behaviour>) -> Multiaddr {
    let listen_addr: Multiaddr = "/ip4/127.0.0.1/tcp/0".parse().unwrap();
    swarm.listen_on(listen_addr).unwrap();
    loop {
        if let SwarmEvent::NewListenAddr { address, .. } = swarm.select_next_some().await {
            return address;
        }
    }
}

// ============================================================================
// Classifier application
// ============================================================================

#[derive(Clone, Copy)]
struct StaleParams {
    now_unix: u64,
    genesis_time: u64,
    slot_duration: u64,
    best_slot: u32,
}

/// Apply the unified gate with a node's PERSISTENT cache. Building the ctx per
/// call is correct: the borrowed `cache` carries all cross-call state.
fn classify_with(
    cache: &mut SeenCache,
    p: &StaleParams,
    topic: GossipTopic,
    data: &[u8],
) -> MessageAcceptance {
    let mut ctx = StalenessCtx {
        now_unix: p.now_unix,
        genesis_time: p.genesis_time,
        slot_duration: p.slot_duration,
        best_slot: p.best_slot,
        seen: cache,
    };
    classify_gossip(topic, data, &mut ctx)
}

// ============================================================================
// Drive loop
// ============================================================================

#[derive(Default)]
struct DriveResult {
    /// Relay (B) verdicts for the payload of interest: `true` = Accept(+forward),
    /// `false` = Ignore(no forward).
    b_verdicts: Vec<bool>,
    /// Downstream deliveries of the payload of interest at C / D.
    c_count: usize,
    d_count: usize,
}

/// Drive all four swarms for `wait`, applying the relay policy at B, C, D.
///
/// `Some(cache)` = the wired gate (persistent [`SeenCache`], `classify_gossip`
/// decides). `None` = pre-M6 unconditional `Accept`, no cache. Only Message
/// events whose bytes equal `expected` are counted, so unrelated gossip is
/// ignored. B is the relay whose Accept/Ignore drives (or suppresses) the
/// forward to {C, D}.
#[allow(clippy::too_many_arguments)]
async fn drive(
    a: &mut Swarm<gossipsub::Behaviour>,
    e: &mut Swarm<gossipsub::Behaviour>,
    b: &mut Swarm<gossipsub::Behaviour>,
    c: &mut Swarm<gossipsub::Behaviour>,
    d: &mut Swarm<gossipsub::Behaviour>,
    wait: Duration,
    topic_enum: GossipTopic,
    params: StaleParams,
    expected: &[u8],
    mut seen_b: Option<&mut SeenCache>,
    mut seen_c: Option<&mut SeenCache>,
    mut seen_d: Option<&mut SeenCache>,
) -> DriveResult {
    let mut res = DriveResult::default();
    let _ = tokio::time::timeout(wait, async {
        loop {
            tokio::select! {
                ev = a.select_next_some() => { let _ = ev; }
                ev = e.select_next_some() => { let _ = ev; }
                ev = b.select_next_some() => {
                    if let SwarmEvent::Behaviour(gossipsub::Event::Message {
                        message_id, propagation_source, message,
                    }) = ev {
                        let verdict = match seen_b.as_deref_mut() {
                            Some(cache) => classify_with(cache, &params, topic_enum, &message.data),
                            None => MessageAcceptance::Accept,
                        };
                        if message.data == expected {
                            res.b_verdicts.push(matches!(verdict, MessageAcceptance::Accept));
                        }
                        let _ = b.behaviour_mut().report_message_validation_result(
                            &message_id, &propagation_source, verdict,
                        );
                    }
                }
                ev = c.select_next_some() => {
                    if let SwarmEvent::Behaviour(gossipsub::Event::Message {
                        message_id, propagation_source, message,
                    }) = ev {
                        let verdict = match seen_c.as_deref_mut() {
                            Some(cache) => classify_with(cache, &params, topic_enum, &message.data),
                            None => MessageAcceptance::Accept,
                        };
                        if message.data == expected {
                            res.c_count += 1;
                        }
                        let _ = c.behaviour_mut().report_message_validation_result(
                            &message_id, &propagation_source, verdict,
                        );
                    }
                }
                ev = d.select_next_some() => {
                    if let SwarmEvent::Behaviour(gossipsub::Event::Message {
                        message_id, propagation_source, message,
                    }) = ev {
                        let verdict = match seen_d.as_deref_mut() {
                            Some(cache) => classify_with(cache, &params, topic_enum, &message.data),
                            None => MessageAcceptance::Accept,
                        };
                        if message.data == expected {
                            res.d_count += 1;
                        }
                        let _ = d.behaviour_mut().report_message_validation_result(
                            &message_id, &propagation_source, verdict,
                        );
                    }
                }
            }
        }
    })
    .await;
    res
}

/// Poll all swarms for `wait`, discarding events. Used for the inter-phase gap
/// (keeps connections/mesh alive while every node's duplicate cache expires).
async fn idle(
    a: &mut Swarm<gossipsub::Behaviour>,
    e: &mut Swarm<gossipsub::Behaviour>,
    b: &mut Swarm<gossipsub::Behaviour>,
    c: &mut Swarm<gossipsub::Behaviour>,
    d: &mut Swarm<gossipsub::Behaviour>,
    wait: Duration,
) {
    let _ = tokio::time::timeout(wait, async {
        loop {
            tokio::select! {
                _ = a.select_next_some() => {}
                _ = e.select_next_some() => {}
                _ = b.select_next_some() => {}
                _ = c.select_next_some() => {}
                _ = d.select_next_some() => {}
            }
        }
    })
    .await;
}

// ============================================================================
// Scenario runner
// ============================================================================

struct StormOutcome {
    first: DriveResult,
    second: DriveResult,
}

/// Build the 4-node star, form the mesh, publish `payload`, then re-publish the
/// identical bytes after the cache window. `gate=true` wires a persistent
/// per-node [`SeenCache`]; `gate=false` models the pre-M6 unconditional-Accept
/// relay. Returns the first-delivery and re-delivery drive results.
async fn run_storm(
    topic_enum: GossipTopic,
    topic_str: &str,
    payload: Vec<u8>,
    gate: bool,
) -> StormOutcome {
    let mut a = build_test_swarm().await;
    let mut e = build_test_swarm().await;
    let mut b = build_test_swarm().await;
    let mut c = build_test_swarm().await;
    let mut d = build_test_swarm().await;

    let pa = *a.local_peer_id();
    let pe = *e.local_peer_id();
    let pc = *c.local_peer_id();
    let pd = *d.local_peer_id();

    let addr_b = listen_on_localhost(&mut b).await;
    let _ = listen_on_localhost(&mut a).await;
    let _ = listen_on_localhost(&mut e).await;
    let _ = listen_on_localhost(&mut c).await;
    let _ = listen_on_localhost(&mut d).await;

    let topic = IdentTopic::new(topic_str);
    // A, B, C, D subscribe. E does NOT subscribe: B must never forward X to E, so
    // E's own duplicate cache stays empty for X and its phase-2 publish succeeds.
    a.behaviour_mut().subscribe(&topic).unwrap();
    b.behaviour_mut().subscribe(&topic).unwrap();
    c.behaviour_mut().subscribe(&topic).unwrap();
    d.behaviour_mut().subscribe(&topic).unwrap();

    // Star: A--B, E--B, C--B, D--B. B is the sole relay; C, D are honest downstream.
    a.dial(addr_b.clone()).unwrap();
    e.dial(addr_b.clone()).unwrap();
    c.dial(addr_b.clone()).unwrap();
    d.dial(addr_b.clone()).unwrap();

    idle(&mut a, &mut e, &mut b, &mut c, &mut d, MESH_FORM).await;

    // Topology assertions: B relays for A, E, C, D; A is NOT directly wired to C/D.
    let bp: Vec<_> = b.connected_peers().cloned().collect();
    assert!(
        bp.contains(&pa) && bp.contains(&pe) && bp.contains(&pc) && bp.contains(&pd),
        "B must be the relay connected to A, E, C and D (got {bp:?})"
    );
    let ap: Vec<_> = a.connected_peers().cloned().collect();
    assert!(
        !ap.contains(&pc) && !ap.contains(&pd),
        "A must reach C/D ONLY through relay B (topology violation: {ap:?})"
    );

    let params = StaleParams {
        now_unix: NOW_UNIX,
        genesis_time: GENESIS,
        slot_duration: SLOT_DUR,
        best_slot: WALL,
    };

    // Persistent per-node caches — created ONCE, reused across BOTH phases. This
    // persistence (mirroring the M6 event-loop field) is the whole point.
    let mut sb = SeenCache::new(SEEN_CACHE_TTL_SECS, SEEN_CACHE_CAPACITY);
    let mut sc = SeenCache::new(SEEN_CACHE_TTL_SECS, SEEN_CACHE_CAPACITY);
    let mut sd = SeenCache::new(SEEN_CACHE_TTL_SECS, SEEN_CACHE_CAPACITY);

    // ── Phase 1: first delivery of a genuinely-new identity (CONTROL) ──────
    assert!(
        a.behaviour_mut()
            .publish(topic.clone(), payload.clone())
            .is_ok(),
        "publish #1 must succeed"
    );
    let first = drive(
        &mut a,
        &mut e,
        &mut b,
        &mut c,
        &mut d,
        PHASE_DRIVE,
        topic_enum,
        params,
        &payload,
        if gate { Some(&mut sb) } else { None },
        if gate { Some(&mut sc) } else { None },
        if gate { Some(&mut sd) } else { None },
    )
    .await;

    // ── Gap: let every node's duplicate_cache expire (opens the re-delivery window)
    idle(&mut a, &mut e, &mut b, &mut c, &mut d, GAP).await;

    // ── Phase 2: SAME-identity re-delivery from E (a peer re-forwarding X) ──
    let republish = e.behaviour_mut().publish(topic.clone(), payload.clone());
    assert!(
        republish.is_ok(),
        "E's re-injection of identical bytes must succeed (got {republish:?}); \
         otherwise the re-delivery window never opened and the test would be vacuous"
    );
    let second = drive(
        &mut a,
        &mut e,
        &mut b,
        &mut c,
        &mut d,
        PHASE_DRIVE,
        topic_enum,
        params,
        &payload,
        if gate { Some(&mut sb) } else { None },
        if gate { Some(&mut sc) } else { None },
        if gate { Some(&mut sd) } else { None },
    )
    .await;

    StormOutcome { first, second }
}

// ============================================================================
// Shared assertions
// ============================================================================

/// Storm-closure + control assertions for the persistent-gate scenario.
fn assert_storm_closed(out: &StormOutcome, topic: &str) {
    // CONTROL: a genuinely-new identity is forwarded EXACTLY ONCE to each mesh peer.
    assert_eq!(
        out.first.c_count, 1,
        "[{topic}] CONTROL: C must receive the fresh message exactly once (first forward not dropped)"
    );
    assert_eq!(
        out.first.d_count, 1,
        "[{topic}] CONTROL: D must receive the fresh message exactly once"
    );
    assert!(
        !out.first.b_verdicts.is_empty() && out.first.b_verdicts.iter().all(|&v| v),
        "[{topic}] CONTROL: relay must Accept+forward the first sight (verdicts={:?})",
        out.first.b_verdicts
    );

    // Vacuity guard: the relay MUST have re-received the re-delivery, else 0
    // re-forwards is trivially (and meaninglessly) satisfied.
    assert!(
        !out.second.b_verdicts.is_empty(),
        "[{topic}] the relay must actually re-receive the same-identity re-delivery \
         (the libp2p escape window must open) — else this test is vacuous"
    );
    // STORM-CLOSURE: every re-delivery verdict is Ignore, 0 downstream re-forwards.
    assert!(
        out.second.b_verdicts.iter().all(|&v| !v),
        "[{topic}] STORM-CLOSURE: relay must Ignore every same-identity re-delivery \
         (persistent SeenCache); got verdicts={:?}",
        out.second.b_verdicts
    );
    assert_eq!(
        out.second.c_count, 0,
        "[{topic}] STORM CLOSED: C must receive 0 re-forwards of the re-delivered message"
    );
    assert_eq!(
        out.second.d_count, 0,
        "[{topic}] STORM CLOSED: D must receive 0 re-forwards of the re-delivered message"
    );
}

/// Pre-M6 reproduction assertions for the unconditional-Accept baseline.
fn assert_storm_open(out: &StormOutcome, topic: &str) {
    assert!(
        out.first.c_count >= 1 && out.first.d_count >= 1,
        "[{topic}] baseline sanity: the fresh message must still propagate to C and D"
    );
    assert!(
        !out.second.b_verdicts.is_empty() && out.second.b_verdicts.iter().all(|&v| v),
        "[{topic}] baseline relay reports unconditional Accept on the re-delivery \
         (pre-M6 behaviour_events.rs); verdicts={:?}",
        out.second.b_verdicts
    );
    let reforwards = out.second.c_count + out.second.d_count;
    assert!(
        reforwards > 0,
        "[{topic}] PRE-M6 REPRODUCTION: the same-identity re-delivery MUST be re-forwarded \
         (storm OPEN); got {reforwards} downstream re-forwards. The persistent-SeenCache gate \
         drives this exact count to 0 — that FAIL→PASS delta is the storm-closure evidence."
    );
}

// ============================================================================
// Attestations (leading storm source)
// ============================================================================

#[tokio::test]
async fn attestation_reforward_storm_closed_by_persistent_gate() {
    let out = run_storm(
        GossipTopic::Attestations,
        ATTESTATION_TOPIC,
        attestation_bytes(),
        true,
    )
    .await;
    assert_storm_closed(&out, "attestations");
}

#[tokio::test]
async fn attestation_reforward_storm_open_under_pre_m6_unconditional_accept() {
    let out = run_storm(
        GossipTopic::Attestations,
        ATTESTATION_TOPIC,
        attestation_bytes(),
        false,
    )
    .await;
    assert_storm_open(&out, "attestations");
}

// ============================================================================
// Heartbeats (second topic)
// ============================================================================

#[tokio::test]
async fn heartbeat_reforward_storm_closed_by_persistent_gate() {
    let out = run_storm(
        GossipTopic::Heartbeats,
        HEARTBEATS_TOPIC,
        heartbeat_bytes(),
        true,
    )
    .await;
    assert_storm_closed(&out, "heartbeats");
}

#[tokio::test]
async fn heartbeat_reforward_storm_open_under_pre_m6_unconditional_accept() {
    let out = run_storm(
        GossipTopic::Heartbeats,
        HEARTBEATS_TOPIC,
        heartbeat_bytes(),
        false,
    )
    .await;
    assert_storm_open(&out, "heartbeats");
}
