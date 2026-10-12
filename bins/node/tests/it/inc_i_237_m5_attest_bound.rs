//! INC-I-237 M5 (M5-DEV-F1, decision C) — the swarm Ignores an attestation
//! whose slot is above wall + 2, so one member key cannot mint first-seen
//! (attester, slot) entries for future slots. Live A -> B(real node) line on
//! 127.0.0.1; B's swarm clock: slot 600 s, wall slot 1,000 for the whole run.
//!
//! OUTPUT CONTRACT: swarm intake (`decide` age + future bound) -> NewAttestation
//!   -> Node::on_new_attestation; contract §C1
//!   O2: B.attest_seen (membership read back through on_new_attestation:
//!   Ignore iff (attester, slot) already seen, unknown block)   O3: none
//!   O6: NewAttestation events the swarm hands B's node (slot)
//! PATHS: P1 slot in [wall - 12, wall + 2] -> reaches the node, enters
//!   attest_seen; P2 slot > wall + 2 -> Ignored at the swarm, never at the
//!   node; P3 slot < wall - 12 -> Ignored (existing age bound)
//! INPUT PARTITIONS: slot {0, wall-112..=wall-13, wall-12..=wall+2,
//!   wall+3..=wall+102, wall+10,000, u32::MAX}; one member key
//! MATRIX: O6 x P1-P3 and O2 x P1-P3 — asserted below.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use crypto::KeyPair;
use doli_core::Attestation;
use doli_node::node::Node;
use network::gossip::staleness::ATTEST_STALE_SLOTS;
use network::service::Verdict;
use network::{NetworkCommand, NetworkConfig, NetworkEvent, NetworkService, PeerId};

use super::inc_i_237_m2_node_common as c2;
use super::inc_i_237_m3_node_common as c;

const SLOT_SECS: u64 = 600;
const WALL: u32 = 1_000;
/// Entries one member key can hold while wall is fixed: slots wall-12..=wall+2.
const BOUND: usize = ATTEST_STALE_SLOTS as usize + 2 + 1;

async fn clocked_net(port: u16, genesis_time: u64) -> NetworkService {
    std::env::set_var("DOLI_IP_COLOCATION_THRESHOLD", "500");
    let listen_addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let config = NetworkConfig {
        listen_addr,
        max_peers: 8,
        bootstrap_slots: 8,
        no_dht: true,
        enable_discv5: false,
        node_key_path: None,
        peer_cache_path: None,
        memory_watchdog_threshold_bytes: 0,
        genesis_time,
        slot_duration: SLOT_SECS,
        ..NetworkConfig::default()
    };
    NetworkService::new(config)
        .await
        .expect("start NetworkService")
}

/// Dispatch B's attestation events through the production path, recording each slot.
async fn pump(node: &mut Node, dur: Duration, slots: &mut Vec<u32>) {
    let until = Instant::now() + dur;
    while Instant::now() < until {
        let net = node.network.as_mut().expect("node network attached");
        if let Ok(Some(ev)) =
            tokio::time::timeout(Duration::from_millis(50), net.next_event()).await
        {
            if let NetworkEvent::NewAttestation(data, _, _) = &ev {
                slots.extend(Attestation::from_bytes(data).map(|a| a.slot));
                let _ = node.handle_network_event(ev).await;
            }
        }
    }
}

fn window() -> Vec<u32> {
    (WALL - ATTEST_STALE_SLOTS..=WALL + 2).collect()
}

fn sent_slots() -> Vec<u32> {
    let mut s = vec![0];
    s.extend(WALL - 112..WALL - ATTEST_STALE_SLOTS);
    s.extend(window());
    s.extend(WALL + 3..=WALL + 102);
    s.extend([WALL + 10_000, u32::MAX]);
    s
}

struct Run {
    b: Node,
    member: KeyPair,
    node_slots: Vec<u32>,
    _t: tempfile::TempDir,
}

/// A publishes one member attestation per slot of `sent_slots()` to B.
async fn drive() -> Run {
    let (mut b, member, t) = c::member_node().await;
    let genesis = c::now_secs() - u64::from(WALL) * SLOT_SECS - 1;
    // B's node slot clock (the M6 wall prune) matches its swarm clock.
    b.params.slot_duration = SLOT_SECS;
    b.params.genesis_time = genesis;
    let b_port = c2::reserve_port();
    b.network = Some(clocked_net(b_port, genesis).await);
    let a = c2::start_net(c2::reserve_port(), vec![c2::addr(b_port)]).await;
    let a_cmd = a.command_sender();
    let _a_log = c::observe(a);
    let mut node_slots = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut i = 0u8;
    while node_slots.is_empty() {
        assert!(Instant::now() < deadline, "setup: A -> B never formed");
        let w = c::attestation(&member, 0x40 + i, WALL).to_bytes();
        a_cmd
            .send(NetworkCommand::BroadcastAttestation(w))
            .await
            .unwrap();
        i += 1;
        pump(&mut b, Duration::from_millis(500), &mut node_slots).await;
    }
    let m = member.clone();
    tokio::spawn(async move {
        for s in sent_slots() {
            let att = c::attestation(&m, 0x60, s).to_bytes();
            let _ = a_cmd.send(NetworkCommand::BroadcastAttestation(att)).await;
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });
    pump(&mut b, Duration::from_secs(4), &mut node_slots).await;
    Run {
        b,
        member,
        node_slots,
        _t: t,
    }
}

// REQ-GRV-SEC-002 (M5-DEV-F1) — Decision: a failure means an attestation for a future slot still crosses B's swarm into the node verdict (and, first seen, is forwarded at every hop).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn m5_dev_f1_far_future_attestation_never_reaches_the_node() {
    let r = drive().await;
    let mut got = r.node_slots.clone();
    got.sort_unstable();
    got.dedup();
    for edge in [WALL - ATTEST_STALE_SLOTS, WALL + 2] {
        assert!(
            got.contains(&edge),
            "non-vacuity: window edge {edge} must reach B's node; got {got:?}"
        );
    }
    let future: Vec<u32> = got.iter().copied().filter(|s| *s > WALL + 2).collect();
    assert!(
        future.is_empty(),
        "M5-DEV-F1: {} attestations above wall + 2 reached B's node: {:?}",
        future.len(),
        &future[..future.len().min(5)]
    );
}

// REQ-GRV-SEC-002 (M5-DEV-F1) — Decision: a failure means one member key grows B's AttestSeen by one entry per future slot it signs (unbounded memory until best passes slot + 12).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn m5_dev_f1_attest_seen_stays_within_the_slot_window_for_one_member_key() {
    let mut r = drive().await;
    let mut seen = Vec::new();
    for s in sent_slots() {
        let probe = c::attestation(&r.member, 0x7F, s).to_bytes();
        if r.b.on_new_attestation(probe, PeerId::random()).await == Verdict::Ignore {
            seen.push(s);
        }
    }
    let above: Vec<u32> = seen.iter().copied().filter(|s| *s > WALL + 2).collect();
    assert!(
        seen.len() <= BOUND,
        "M5-DEV-F1: attest_seen holds {} entries for one member key (bound {BOUND}); {} above wall + 2",
        seen.len(),
        above.len()
    );
    assert_eq!(
        seen,
        window(),
        "non-vacuity: exactly the in-window slots entered attest_seen"
    );
}
