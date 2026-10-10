//! INC-I-237 — a flood of UNIQUE gossip messages toward a slow mesh peer
//! grows the node's memory without bound.
//!
//! libp2p-gossipsub 0.46.1 keeps a per-connection `Handler.send_queue`
//! (handler.rs:98) with no bound: `on_behaviour_event` pushes every forwarded
//! message (handler.rs:412) and the queue only drains as fast as the remote
//! reads. When one peer floods unique tx-topic messages and another mesh peer
//! reads slowly, the queue for the slow peer keeps every byte. Dedup cannot
//! help: every message is unique.
//!
//! # Topology (all in-process, 127.0.0.1, production transport + gossipsub)
//!
//! ```text
//!   F (flooder) ──► A (node under test) ──► S (stalled reader)
//! ```
//!
//! S joins the mesh, then the test STOPS polling S's swarm (S stays alive and
//! connected). S's connection task blocks once its event channel is full, so
//! S stops reading its socket, the yamux window (256 KB) and the kernel
//! buffers fill, and A can no longer drain its send_queue to S.
//!
//! # Measurement
//!
//! A counting `#[global_allocator]` tracks live heap bytes for the whole `it`
//! test binary. ASSUMPTION: after S stops being polled, only A and F allocate
//! in bulk. F is paced (at most `MAX_IN_FLIGHT` messages ahead of A), so F's
//! own send queue stays small. Both A and F keep full messages in `mcache`
//! for `history_length` (5) heartbeats of 1 s, so the test settles for
//! `SETTLE_AFTER_FLOOD` (8 s) before the final sample. After the settle, the
//! retained bytes are A's send_queue to S plus small constant state (dedup
//! IDs, score maps). Run it filtered (`-- inc_i_237`) so no other test of the
//! `it` binary allocates in parallel; the other tests only pay one relaxed
//! atomic add per allocation.
//!
//! # Cap
//!
//! Offered traffic is N x 40 KB = 200 MB. `RETAINED_CAP_BYTES` = 64 MiB. A
//! bounded per-peer queue (or a drop of the slow peer) keeps the retained
//! growth to the queue bound plus the yamux window, far below 64 MiB. An
//! unbounded queue retains almost all 200 MB.
//!
//! # OUTPUT CONTRACT
//!
//! Outputs:
//!   O1: live heap bytes retained by the process after the flood (A's
//!       per-peer send_queue toward S dominates)
//!   O2: number of flood messages A received and accepted (non-vacuity)
//!   O3: number of successful `publish` calls on F (non-vacuity)
//!   O4: S is still connected to A and still in A's mesh (the slow-peer path
//!       is really exercised, not short-circuited by a disconnect/prune)
//!
//! Paths:
//!   P1: unique message, fast peer (F->A, A polled) -> drained, not retained
//!   P2: unique message, slow peer (A->S, S stalled) -> pushed to send_queue;
//!       retained until S reads                      [O1 — the bug]
//!   P3: duplicate message -> dropped by dedup before forwarding (not
//!       exercised here: the INC-I-237 dedup hotfix covers it; a dedup cannot
//!       bound P2 because every flood message is unique)
//!
//! INPUT PARTITIONS:
//!   IP-1: unique x slow peer  -> P2, asserts O1 <= cap, O2 == N, O3 == N, O4
//!   IP-2: unique x fast peer  -> P1 (F->A link, implicit in O2 == N)
//!   IP-3: duplicate x any     -> P3 (out of scope; see note on P3)
//!
//! Deviations from production: no relay/kad/identify/request-response
//! behaviours (gossipsub only); transport is `build_transport(kp, None)` (no
//! relay client); mesh = mainnet defaults 6/4/12/6;
//! `DOLI_IP_COLOCATION_THRESHOLD=500` because all peers share 127.0.0.1.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::{Duration, Instant};

use futures::StreamExt;
use libp2p::gossipsub::{self, IdentTopic, MessageAcceptance};
use libp2p::swarm::SwarmEvent;
use libp2p::{identity::Keypair, Multiaddr, PeerId, Swarm};
use network::gossip::{new_gossipsub, MeshConfig, TRANSACTIONS_TOPIC};
use network::transport::build_transport;

// ---------------------------------------------------------------------------
// Counting allocator: live heap bytes for the whole test binary.
// ---------------------------------------------------------------------------

struct Counting;

static LIVE_BYTES: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            LIVE_BYTES.fetch_add(layout.size() as isize, Ordering::Relaxed);
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            LIVE_BYTES.fetch_add(layout.size() as isize, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        LIVE_BYTES.fetch_sub(layout.size() as isize, Ordering::Relaxed);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = System.realloc(ptr, layout, new_size);
        if !p.is_null() {
            LIVE_BYTES.fetch_add(
                new_size as isize - layout.size() as isize,
                Ordering::Relaxed,
            );
        }
        p
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn live_bytes() -> isize {
    LIVE_BYTES.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

/// Number of unique flood messages.
const N_MESSAGES: usize = 5_000;
/// Payload size per message (mainnet tx batches are ~40 KB).
const PAYLOAD_BYTES: usize = 40_000;
/// F may run at most this many messages ahead of A (keeps F's queue small).
const MAX_IN_FLIGHT: usize = 32;
/// Retained-growth cap. 5,000 x 40 KB = 200 MB offered; see module doc.
const RETAINED_CAP_BYTES: isize = 64 * 1024 * 1024;
/// > history_length (5) x heartbeat (1 s): mcache copies expire.
const SETTLE_AFTER_FLOOD: Duration = Duration::from_secs(8);

type GSwarm = Swarm<gossipsub::Behaviour>;

fn mainnet_mesh() -> MeshConfig {
    MeshConfig {
        mesh_n: 6,
        mesh_n_low: 4,
        mesh_n_high: 12,
        gossip_lazy: 6,
    }
}

fn build_swarm() -> GSwarm {
    let kp = Keypair::generate_ed25519();
    let peer_id = PeerId::from(kp.public());
    let transport = build_transport(&kp, None).expect("transport");
    let behaviour = new_gossipsub(&kp, &mainnet_mesh()).expect("production gossipsub");
    Swarm::new(
        transport,
        behaviour,
        peer_id,
        libp2p::swarm::Config::with_tokio_executor()
            .with_idle_connection_timeout(Duration::from_secs(300)),
    )
}

async fn listen(swarm: &mut GSwarm) -> Multiaddr {
    swarm
        .listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .unwrap();
    loop {
        if let SwarmEvent::NewListenAddr { address, .. } = swarm.select_next_some().await {
            return address;
        }
    }
}

/// Handle one event on A: Accept every gossip message (as the node does for
/// valid txs). Returns true when the event was a received gossip message.
fn on_a_event(a: &mut GSwarm, ev: SwarmEvent<gossipsub::Event>) -> bool {
    if let SwarmEvent::Behaviour(gossipsub::Event::Message {
        propagation_source,
        message_id,
        ..
    }) = ev
    {
        let _ = a.behaviour_mut().report_message_validation_result(
            &message_id,
            &propagation_source,
            MessageAcceptance::Accept,
        );
        return true;
    }
    false
}

/// Poll A and F (never S) for `dur`, accepting messages on A.
async fn drive_a_f(a: &mut GSwarm, f: &mut GSwarm, dur: Duration) -> usize {
    let deadline = tokio::time::sleep(dur);
    tokio::pin!(deadline);
    let mut got = 0;
    loop {
        tokio::select! {
            ev = a.select_next_some() => { if on_a_event(a, ev) { got += 1; } }
            _ = f.select_next_some() => {}
            _ = &mut deadline => return got,
        }
    }
}

fn unique_payload(i: usize) -> Vec<u8> {
    let mut v = vec![(i % 251) as u8; PAYLOAD_BYTES];
    v[..8].copy_from_slice(&(i as u64).to_le_bytes());
    v
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inc_i_237_gossip_flood_memory_is_bounded() {
    std::env::set_var("DOLI_IP_COLOCATION_THRESHOLD", "500");
    let started = Instant::now();
    let topic = IdentTopic::new(TRANSACTIONS_TOPIC);
    let topic_hash = topic.hash();

    let mut a = build_swarm();
    let mut f = build_swarm();
    let mut s = build_swarm();
    let (a_id, f_id, s_id) = (*a.local_peer_id(), *f.local_peer_id(), *s.local_peer_id());
    for sw in [&mut a, &mut f, &mut s] {
        sw.behaviour_mut().subscribe(&topic).unwrap();
    }
    let addr_a = listen(&mut a).await;
    f.dial(addr_a.clone()).unwrap();
    s.dial(addr_a).unwrap();

    // Phase 1: form the mesh A<->F, A<->S (poll all three).
    let mesh_deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let a_mesh: Vec<PeerId> = a.behaviour().mesh_peers(&topic_hash).cloned().collect();
        let f_ok = f.behaviour().mesh_peers(&topic_hash).any(|p| *p == a_id);
        let s_ok = s.behaviour().mesh_peers(&topic_hash).any(|p| *p == a_id);
        if a_mesh.contains(&f_id) && a_mesh.contains(&s_id) && f_ok && s_ok {
            break;
        }
        assert!(Instant::now() < mesh_deadline, "mesh did not form in 30 s");
        tokio::select! {
            ev = a.select_next_some() => { on_a_event(&mut a, ev); }
            _ = f.select_next_some() => {}
            _ = s.select_next_some() => {}
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }

    // Phase 2: S stalls (never polled again, never dropped). Settle, then
    // take the baseline.
    drive_a_f(&mut a, &mut f, Duration::from_secs(3)).await;
    let baseline = live_bytes();

    // Phase 3: F floods N unique messages, paced against A's receipt.
    let mut published = 0usize;
    let mut received = 0usize;
    let flood_deadline = Instant::now() + Duration::from_secs(120);
    while received < N_MESSAGES {
        while published < N_MESSAGES && published - received < MAX_IN_FLIGHT {
            f.behaviour_mut()
                .publish(topic.clone(), unique_payload(published))
                .expect("F publish");
            published += 1;
        }
        assert!(
            Instant::now() < flood_deadline,
            "flood stalled: published={published} received_by_A={received}"
        );
        tokio::select! {
            ev = a.select_next_some() => { if on_a_event(&mut a, ev) { received += 1; } }
            _ = f.select_next_some() => {}
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
    let flood_done_at = started.elapsed();

    // Phase 4: let mcache copies expire, then sample.
    received += drive_a_f(&mut a, &mut f, SETTLE_AFTER_FLOOD).await;
    let retained = live_bytes() - baseline;

    let s_connected = a.is_connected(&s_id);
    let s_in_mesh = a.behaviour().mesh_peers(&topic_hash).any(|p| *p == s_id);
    println!(
        "INC-I-237: published_by_F={published} received_by_A={received} \
         S_connected={s_connected} S_in_A_mesh={s_in_mesh} \
         offered={} MB retained={retained} bytes ({} MiB) cap={RETAINED_CAP_BYTES} bytes \
         flood_done_at={flood_done_at:?} total={:?}",
        N_MESSAGES * PAYLOAD_BYTES / 1_000_000,
        retained / (1024 * 1024),
        started.elapsed()
    );

    // Non-vacuity: the flood really reached A, and the slow-peer path is live.
    assert_eq!(published, N_MESSAGES, "F did not publish every message");
    assert!(received >= N_MESSAGES, "A received only {received}");
    assert!(
        s_connected,
        "S disconnected; the slow-peer path was not tested"
    );
    assert!(s_in_mesh, "S left A's mesh; A never forwarded to S");

    // The bug: A retains the flood in its unbounded send_queue toward S.
    assert!(
        retained <= RETAINED_CAP_BYTES,
        "INC-I-237: retained {retained} bytes > cap {RETAINED_CAP_BYTES} bytes \
         after {N_MESSAGES} unique x {PAYLOAD_BYTES} B flood toward a stalled peer \
         (unbounded gossipsub send_queue)"
    );

    drop(s);
}
