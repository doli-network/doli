//! INC-I-237 M5 (F-13) — sustained-flood soak through the REAL node seam:
//! raw gossipsub attackers A1..A3 -> B (`Node::new_for_test`, one member) -> C,
//! honest H -> B. Own binary: the counting `#[global_allocator]` is per-binary.
//!
//! OUTPUT CONTRACT: the A -> B(node) -> C system under flood (contract Part B1)
//!   O1-O4: none   O5: process heap CURRENT / PEAK; B's LaneSnapshot bytes;
//!   LATE_VERDICTS[Attestations]
//!   O6: at C — honest attestations, forged zero-flow txs, mutant-family and
//!   junk-hash crossings; at B — attestation receive -> verdict latency
//! PATHS: P1 honest attestation under flood -> at C; P2 forged zero-flow batch
//!   -> never at C; P3 mutant / junk-hash family -> <= 2 at C each; P4 heap
//!   retained after drain < 64 MiB; P5 lanes inside budgets; P6 genuine
//!   transfer after the flood -> at C (non-vacuity of P2)
//! INPUT PARTITIONS: tx {0xBA batch of forged DelegateBonds ~1.6 MB, every tx
//!   unique}; attestation {mutants of one genuine, junk hash on one slot,
//!   honest second member}; vote {hex-case variants of one signed vote};
//!   duration {SOAK_SECS 30 default, 600 ignored}
//! MATRIX: every O x P asserted in `check`; offered / admitted rates printed.
//! M7.1 (REQ-GRV-026): `run` prints the per-(swarm, structure) attribution of
//!   the retained heap (`inc_i_237_m5_soak_support/attribution.rs`) before `check`;
//!   `SOAK_ATTACKER_DUP_TTL_SECS` = the attackers' duplicate-cache TTL (differential run).
//! covers: vendor/libp2p-gossipsub/src/behaviour.rs crates/network/src/gossip/config.rs
//!   crates/network/src/service/retention.rs bins/node/src/node/mod.rs

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::BASE_BLOCK_SIZE;
use doli_core::transaction::DelegateBondData;
use doli_core::{Attestation, Network, Transaction};
use doli_node::node::Node;
use futures::StreamExt;
use libp2p::gossipsub::{self, IdentTopic, MessageAcceptance, RetentionStats};
use libp2p::swarm::SwarmEvent;
use libp2p::{identity::Keypair, Multiaddr, Swarm};
use network::gossip::staleness::GossipTopic;
use network::gossip::{
    encode_tx_batch, new_gossipsub, new_gossipsub_with_cache_time, MeshConfig, ATTESTATION_TOPIC,
    TRANSACTIONS_TOPIC, VOTES_TOPIC,
};
use network::service::LATE_VERDICTS;
use network::transport::build_transport;
use network::{NetworkCommand, NetworkEvent, NetworkService};
use storage::ProducerInfo;
use tokio::sync::{mpsc, oneshot};

#[allow(dead_code)]
#[path = "it/inc_i_237_m2_node_common.rs"]
mod inc_i_237_m2_node_common;
#[path = "it/inc_i_237_m3_node_common.rs"]
mod inc_i_237_m3_node_common;
use inc_i_237_m2_node_common as c2;
use inc_i_237_m3_node_common as c3;
#[path = "inc_i_237_m5_soak_support/attribution.rs"]
mod attribution;
#[path = "inc_i_237_m5_soak_support/calibration.rs"]
mod calibration;
#[path = "inc_i_237_m5_soak_support/gauge.rs"]
mod gauge;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static SERIAL: Mutex<()> = Mutex::new(());

struct CountingAlloc;

fn grow(bytes: usize) {
    let now = CURRENT.fetch_add(bytes, Relaxed) + bytes;
    PEAK.fetch_max(now, Relaxed);
}

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            grow(layout.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            grow(layout.size());
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        CURRENT.fetch_sub(layout.size(), Relaxed);
        System.dealloc(ptr, layout);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = System.realloc(ptr, layout, new_size);
        if !p.is_null() {
            if new_size >= layout.size() {
                grow(new_size - layout.size());
            } else {
                CURRENT.fetch_sub(layout.size() - new_size, Relaxed);
            }
        }
        p
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

const MIB: usize = 1 << 20;
const WARM_SECS: u64 = 10;
/// Quiescence before each heap sample: longer than the gossipsub mcache window
/// (history_length 5 x 1 s heartbeat), so in-process attacker caches are empty.
const DRAIN: Duration = Duration::from_secs(7);
const FAMILY_SLOT: u32 = 700;
const JUNK_SLOT: u32 = 701;
const HONEST_BASE: u32 = 10_000;
/// Offered per attacker link per 20 ms tick: tx batches 50/s (above what one
/// loopback link carries), mutants 250/s, junk-hash 250/s, vote variants 50/s.
const PER_TICK: [usize; 4] = [1, 5, 5, 1];
const TOPICS: [&str; 3] = [TRANSACTIONS_TOPIC, ATTESTATION_TOPIC, VOTES_TOPIC];

struct Attacker {
    big: mpsc::Sender<Vec<u8>>,
    small: mpsc::Sender<(usize, Vec<u8>)>,
    /// Published Ok per kind: tx batch, mutant, junk, vote.
    published: Arc<[AtomicU64; 4]>,
    retention: mpsc::Sender<oneshot::Sender<RetentionStats>>,
}

/// A raw production-config gossipsub peer: publishes any bytes, Ignores what it
/// receives. `dup_ttl` replaces only its duplicate-cache TTL.
fn attacker(dial: &str, dup_ttl: Option<Duration>) -> Attacker {
    let kp = Keypair::generate_ed25519();
    let peer_id = kp.public().to_peer_id();
    let mesh = MeshConfig {
        mesh_n: 6,
        mesh_n_low: 4,
        mesh_n_high: 12,
        gossip_lazy: 6,
    };
    let mut swarm: Swarm<gossipsub::Behaviour> = Swarm::new(
        build_transport(&kp, None).expect("transport"),
        match dup_ttl {
            Some(ttl) => new_gossipsub_with_cache_time(&kp, &mesh, ttl),
            None => new_gossipsub(&kp, &mesh),
        }
        .expect("production gossipsub"),
        peer_id,
        libp2p::swarm::Config::with_tokio_executor()
            .with_idle_connection_timeout(Duration::from_secs(3_600)),
    );
    let topics: Vec<IdentTopic> = TOPICS.iter().map(|t| IdentTopic::new(*t)).collect();
    for t in &topics {
        swarm.behaviour_mut().subscribe(t).unwrap();
    }
    swarm.dial(dial.parse::<Multiaddr>().unwrap()).unwrap();
    let (big, mut big_rx) = mpsc::channel::<Vec<u8>>(2);
    let (small, mut small_rx) = mpsc::channel::<(usize, Vec<u8>)>(256);
    let published: Arc<[AtomicU64; 4]> = Arc::default();
    let count = published.clone();
    let (retention, mut retention_rx) = mpsc::channel::<oneshot::Sender<RetentionStats>>(1);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                ev = swarm.select_next_some() => {
                    if let SwarmEvent::Behaviour(gossipsub::Event::Message {
                        propagation_source, message_id, ..
                    }) = ev {
                        let _ = swarm.behaviour_mut().report_message_validation_result(
                            &message_id, &propagation_source, MessageAcceptance::Ignore,
                        );
                    }
                }
                Some(data) = big_rx.recv() => {
                    if swarm.behaviour_mut().publish(topics[0].clone(), data).is_ok() {
                        count[0].fetch_add(1, Relaxed);
                    }
                }
                Some((kind, data)) = small_rx.recv() => {
                    let topic = topics[if kind == 3 { 2 } else { 1 }].clone();
                    if swarm.behaviour_mut().publish(topic, data).is_ok() {
                        count[kind].fetch_add(1, Relaxed);
                    }
                }
                Some(reply) = retention_rx.recv() => {
                    let _ = reply.send(swarm.behaviour().retention_stats());
                }
            }
        }
    });
    Attacker {
        big,
        small,
        published,
        retention,
    }
}

fn sig_bytes(i: u32) -> [u8; 64] {
    let mut s = [0xF0u8; 64];
    s[..4].copy_from_slice(&i.to_le_bytes());
    s
}

fn find(hay: &[u8], needle: &[u8]) -> usize {
    hay.windows(needle.len())
        .position(|w| w == needle)
        .expect("signature pattern in the encoded batch")
}

/// One encoded 0xBA batch of forged DelegateBonds; `message(n)` rewrites every
/// tx's delegator and signature so every tx of every message is new.
struct ForgedBatch {
    base: Vec<u8>,
    sig0: usize,
    stride: usize,
    txs: usize,
}

impl ForgedBatch {
    fn new() -> Self {
        let delegate = *KeyPair::generate().public_key();
        let one = |i: u32| {
            let mut who = [0xD0u8; 32];
            who[..4].copy_from_slice(&i.to_le_bytes());
            let mut d = DelegateBondData::new(PublicKey::from_bytes(who), delegate, 1);
            d.signature = crypto::Signature::from_bytes(sig_bytes(i));
            Transaction::new_delegate_bond(d)
        };
        let txs = 10_000.min((BASE_BLOCK_SIZE - 4_096) / one(0).serialize().len());
        let base = encode_tx_batch(&(0..txs as u32).map(one).collect::<Vec<_>>());
        let sig0 = find(&base, &sig_bytes(0));
        let stride = find(&base, &sig_bytes(1)) - sig0;
        let last = sig0 + (txs - 1) * stride;
        assert_eq!(
            &base[last..last + 64],
            &sig_bytes(txs as u32 - 1)[..],
            "uniform stride"
        );
        Self {
            base,
            sig0,
            stride,
            txs,
        }
    }

    fn message(&self, n: u64) -> Vec<u8> {
        let mut m = self.base.clone();
        for i in 0..self.txs {
            let s = self.sig0 + i * self.stride;
            m[s + 4..s + 12].copy_from_slice(&n.to_le_bytes());
            m[s - 68 + 4..s - 68 + 12].copy_from_slice(&n.to_le_bytes());
        }
        m
    }
}

/// Every attacker payload family, built from the member's key.
struct Payloads {
    batch: ForgedBatch,
    genuine: Attestation,
    junk: Vec<Attestation>,
    vote: updater::VoteMessage,
}

impl Payloads {
    fn new(member: &KeyPair) -> Self {
        let junk = (0..256u32)
            .map(|i| {
                let mut h = [0x6Au8; 32];
                h[..4].copy_from_slice(&i.to_le_bytes());
                let pk = *member.public_key();
                Attestation::new(
                    Hash::from_bytes(h),
                    JUNK_SLOT,
                    1,
                    1,
                    member.private_key(),
                    pk,
                )
            })
            .collect();
        let vote = serde_json::from_slice(&c3::vote(member, c3::now_secs() - 60, true)).unwrap();
        Self {
            batch: ForgedBatch::new(),
            genuine: c3::attestation(member, 0x61, FAMILY_SLOT),
            junk,
            vote,
        }
    }

    fn mutant(&self, n: u64) -> Vec<u8> {
        let mut m = self.genuine.clone();
        m.height = 1_000 + n;
        m.bls_signature = vec![0xB0; 96];
        m.bls_signature[..8].copy_from_slice(&n.to_le_bytes());
        m.to_bytes()
    }

    fn junk(&self, n: u64) -> Vec<u8> {
        let mut m = self.junk[n as usize % self.junk.len()].clone();
        m.height = n;
        m.to_bytes()
    }

    /// Bit k of `n` upper-cases the k-th hex letter of producer_id ++ signature.
    fn vote_variant(&self, n: u64) -> Vec<u8> {
        let mut v = self.vote.clone();
        let mut bit = 0;
        for s in [&mut v.producer_id, &mut v.signature] {
            *s = s
                .chars()
                .map(|ch| {
                    if !ch.is_ascii_alphabetic() || bit >= 64 {
                        return ch;
                    }
                    bit += 1;
                    if n >> (bit - 1) & 1 == 1 {
                        ch.to_ascii_uppercase()
                    } else {
                        ch
                    }
                })
                .collect();
        }
        serde_json::to_vec(&v).unwrap()
    }
}

#[derive(Default)]
struct Flags {
    paused: AtomicBool,
    stop: AtomicBool,
}

fn flood(j: u64, a: &Attacker, p: Arc<Payloads>, flags: Arc<Flags>) {
    let (big, small) = (a.big.clone(), a.small.clone());
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut n = j << 40;
        while !flags.stop.load(Relaxed) {
            tick.tick().await;
            if flags.paused.load(Relaxed) {
                continue;
            }
            for _ in 0..PER_TICK[0] {
                if big.capacity() == 0 {
                    break;
                }
                n += 1;
                let _ = big.try_send(p.batch.message(n));
            }
            for (kind, per) in PER_TICK.iter().enumerate().skip(1) {
                for _ in 0..*per {
                    n += 1;
                    let data = match kind {
                        1 => p.mutant(n),
                        2 => p.junk(n),
                        _ => p.vote_variant(n),
                    };
                    let _ = small.try_send((kind, data));
                }
            }
        }
    });
}

#[derive(Default)]
struct AtC {
    zero_flow: AtomicU64,
    genuine_txs: AtomicU64,
    family: AtomicU64,
    junk: AtomicU64,
    votes: AtomicU64,
    honest: Mutex<HashSet<u32>>,
}

fn observe_c(mut svc: NetworkService, member: PublicKey, honest: PublicKey) -> Arc<AtC> {
    let at = Arc::new(AtC::default());
    let out = at.clone();
    tokio::spawn(async move {
        while let Some(ev) = svc.next_event().await {
            match ev {
                NetworkEvent::NewTransaction(txs, _) => {
                    let zf = txs.iter().filter(|t| t.is_zero_flow()).count() as u64;
                    out.zero_flow.fetch_add(zf, Relaxed);
                    out.genuine_txs.fetch_add(txs.len() as u64 - zf, Relaxed);
                }
                NetworkEvent::NewAttestation(d, _, _) => match Attestation::from_bytes(&d) {
                    Some(a) if a.attester == honest => {
                        out.honest.lock().unwrap().insert(a.slot);
                    }
                    Some(a) if a.attester == member && a.slot == FAMILY_SLOT => {
                        out.family.fetch_add(1, Relaxed);
                    }
                    Some(a) if a.attester == member && a.slot == JUNK_SLOT => {
                        out.junk.fetch_add(1, Relaxed);
                    }
                    _ => {}
                },
                NetworkEvent::NewVote(..) => {
                    out.votes.fetch_add(1, Relaxed);
                }
                _ => {}
            }
        }
    });
    at
}

#[derive(Default, Debug)]
struct AtB {
    attestations: u64,
    votes: u64,
    tx_events: u64,
    max_verdict: Duration,
    lane_max: [usize; 3],
}

/// Drive B's node through the production dispatch; record what it was handed.
async fn pump(node: &mut Node, until: Instant, at: &mut AtB) {
    let stats = node.network.as_ref().expect("B network").lane_stats();
    while Instant::now() < until {
        let net = node.network.as_mut().expect("B network");
        let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(50), net.next_event()).await
        else {
            continue;
        };
        let s = stats.snapshot();
        for (m, v) in at
            .lane_max
            .iter_mut()
            .zip([s.consensus_bytes, s.bulk_bytes, s.backlog_bytes])
        {
            *m = (*m).max(v);
        }
        let recv = match &ev {
            NetworkEvent::NewAttestation(_, _, h) => Some(h.recv_at),
            _ => None,
        };
        match &ev {
            NetworkEvent::NewAttestation(..) => at.attestations += 1,
            NetworkEvent::NewVote(..) => at.votes += 1,
            NetworkEvent::NewTransaction(..) => at.tx_events += 1,
            _ => {}
        }
        if matches!(
            ev,
            NetworkEvent::NewAttestation(..)
                | NetworkEvent::NewVote(..)
                | NetworkEvent::NewTransaction(..)
                | NetworkEvent::ProducerAnnouncementsReceived(..)
                | NetworkEvent::ProducerDigestReceived { .. }
                | NetworkEvent::SyncRequest { .. }
        ) {
            let _ = node.handle_network_event(ev).await;
        }
        if let Some(r) = recv {
            at.max_verdict = at.max_verdict.max(r.elapsed());
        }
    }
}

async fn register_member(node: &Node, kp: &KeyPair) {
    let bond = Network::Devnet.bond_unit();
    let info = ProducerInfo::new(*kp.public_key(), 1_000_000, bond, (Hash::ZERO, 0), 0, bond);
    node.producer_set
        .write()
        .await
        .register(info, 1_000_000)
        .expect("register second member");
}

#[derive(Debug)]
#[allow(dead_code)] // every field is read through the printed Debug report
struct Report {
    secs: u64,
    tx_msgs: usize,
    offered_per_link_per_s: [f64; 4],
    at_b: AtB,
    honest_sent: usize,
    honest_at_c: usize,
    zero_flow_at_c: u64,
    family_at_c: u64,
    junk_at_c: u64,
    votes_at_c: u64,
    genuine_txs_at_c: u64,
    late_delta: u64,
    pre_flood_mib: f64,
    warm_mib: f64,
    end_mib: f64,
    retained_mib: f64,
    peak_mib: f64,
}

async fn run(secs: u64) -> (Report, attribution::Attribution) {
    let late0 = LATE_VERDICTS.get(GossipTopic::Attestations);
    let (mut b, member, _t) = c3::member_node().await;
    // B's node slot clock (the M6 wall prune) pinned to FAMILY_SLOT for the run.
    b.params.slot_duration = 600;
    b.params.genesis_time = c3::now_secs() - u64::from(FAMILY_SLOT) * 600 - 1;
    let honest = KeyPair::generate();
    register_member(&b, &honest).await;
    let funded = KeyPair::generate();
    c2::seed(&b, &funded, c2::prev(0x5E)).await;
    let b_port = c2::reserve_port();
    b.network = Some(c2::start_net(b_port, vec![]).await);
    let c_net = c2::start_net(c2::reserve_port(), vec![c2::addr(b_port)]).await;
    let c_ret = c_net.retention();
    let at_c = observe_c(c_net, *member.public_key(), *honest.public_key());
    let h = c2::start_net(c2::reserve_port(), vec![c2::addr(b_port)]).await;
    let h_cmd = h.command_sender();
    let h_ret = h.retention();
    c3::observe(h);
    let dup_ttl = std::env::var("SOAK_ATTACKER_DUP_TTL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .map(Duration::from_secs);
    let attackers: Vec<Attacker> = (0..3)
        .map(|_| attacker(&c2::addr(b_port), dup_ttl))
        .collect();

    let mut at_b = AtB::default();
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut i = 0;
    while at_c.honest.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "setup: H -> B -> C never formed");
        let w = c3::attestation(&honest, 0x51, 50 + i).to_bytes();
        h_cmd
            .send(NetworkCommand::BroadcastAttestation(w))
            .await
            .unwrap();
        i += 1;
        pump(
            &mut b,
            Instant::now() + Duration::from_millis(500),
            &mut at_b,
        )
        .await;
    }

    let payloads = Arc::new(Payloads::new(&member));
    let pre = CURRENT.load(Relaxed);
    let tx_msgs = payloads.batch.txs;
    let flags = Arc::new(Flags::default());
    for (j, a) in attackers.iter().enumerate() {
        flood(j as u64 + 1, a, payloads.clone(), flags.clone());
    }
    let honest_sent = secs as usize;
    let honest_cmd = h_cmd.clone();
    let hk = honest.clone();
    tokio::spawn(async move {
        for s in 0..honest_sent as u32 {
            let a = c3::attestation(&hk, 0x70, HONEST_BASE + s).to_bytes();
            let _ = honest_cmd
                .send(NetworkCommand::BroadcastAttestation(a))
                .await;
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
    let start = Instant::now();
    pump(&mut b, start + Duration::from_secs(WARM_SECS), &mut at_b).await;
    flags.paused.store(true, Relaxed);
    pump(&mut b, Instant::now() + DRAIN, &mut at_b).await;
    let warm = CURRENT.load(Relaxed);
    let warm_rows = attribution::sample(&b, &attackers, &h_ret, &c_ret).await;
    flags.paused.store(false, Relaxed);
    pump(&mut b, start + Duration::from_secs(secs) + DRAIN, &mut at_b).await;
    flags.stop.store(true, Relaxed);
    let flood_secs = secs as f64;
    pump(&mut b, Instant::now() + DRAIN, &mut at_b).await;
    let end = CURRENT.load(Relaxed);
    let peak = PEAK.load(Relaxed);
    let end_rows = attribution::sample(&b, &attackers, &h_ret, &c_ret).await;
    let table = attribution::Attribution::new(&warm_rows, &end_rows, end as i64 - warm as i64);
    table.print();

    let transfer = c2::transfer(&funded, c2::prev(0x5E));
    h_cmd
        .send(NetworkCommand::BroadcastTransaction(transfer))
        .await
        .unwrap();
    pump(&mut b, Instant::now() + Duration::from_secs(5), &mut at_b).await;

    let mut offered = [0f64; 4];
    for (k, o) in offered.iter_mut().enumerate() {
        let total: u64 = attackers.iter().map(|a| a.published[k].load(Relaxed)).sum();
        *o = total as f64 / attackers.len() as f64 / flood_secs;
    }
    let honest_at_c = at_c
        .honest
        .lock()
        .unwrap()
        .iter()
        .filter(|s| **s >= HONEST_BASE)
        .count();
    let mib = |b: usize| b as f64 / MIB as f64;
    let report = Report {
        secs,
        tx_msgs,
        offered_per_link_per_s: offered,
        at_b,
        honest_sent,
        honest_at_c,
        zero_flow_at_c: at_c.zero_flow.load(Relaxed),
        family_at_c: at_c.family.load(Relaxed),
        junk_at_c: at_c.junk.load(Relaxed),
        votes_at_c: at_c.votes.load(Relaxed),
        genuine_txs_at_c: at_c.genuine_txs.load(Relaxed),
        late_delta: LATE_VERDICTS.get(GossipTopic::Attestations) - late0,
        pre_flood_mib: mib(pre),
        warm_mib: mib(warm),
        end_mib: mib(end),
        retained_mib: (end as f64 - warm as f64) / MIB as f64,
        peak_mib: mib(peak),
    };
    (report, table)
}

fn check(r: &Report) {
    println!("F-13 soak report: {r:#?}");
    let delivery = r.honest_at_c as f64 / r.honest_sent as f64;
    assert!(
        r.offered_per_link_per_s[0] >= 1.0 && r.at_b.attestations >= 100,
        "non-vacuity: flood never reached B ({:?} offered/s/link, {} attestation events at B)",
        r.offered_per_link_per_s,
        r.at_b.attestations
    );
    assert!(
        delivery >= 0.99,
        "REQ-GRV-SEC-002 / 010: honest attestation delivery at C {delivery:.3} ({}/{}) < 0.99",
        r.honest_at_c,
        r.honest_sent
    );
    assert_eq!(
        r.zero_flow_at_c, 0,
        "REQ-GRV-004: forged zero-flow txs at C"
    );
    assert!(
        r.family_at_c <= 2 && r.junk_at_c <= 2,
        "AUDIT-P1-506: mutant family {} / junk-hash {} crossed B (bound 2 each)",
        r.family_at_c,
        r.junk_at_c
    );
    assert!(
        r.at_b.max_verdict < Duration::from_secs(2) && r.late_delta == 0,
        "REQ-GRV-017 / 008: B attestation verdict p100 {:?}, late delta {}",
        r.at_b.max_verdict,
        r.late_delta
    );
    assert!(
        r.retained_mib < 64.0,
        "REQ-GRV-007: retained heap {:.1} MiB >= 64 MiB (warm {:.1}, end {:.1}, peak {:.1})",
        r.retained_mib,
        r.warm_mib,
        r.end_mib,
        r.peak_mib
    );
    let [cons, bulk, backlog] = r.at_b.lane_max;
    assert!(
        cons <= 64 * MIB && bulk <= 16 * MIB && backlog <= 64 * MIB,
        "REQ-GRV-007: B lanes peaked at {cons} / {bulk} / {backlog} B (64 / 16 / 64 MiB)"
    );
    assert!(
        r.genuine_txs_at_c >= 1,
        "non-vacuity: a genuine transfer after the flood never reached C"
    );
}

fn soak_secs(default: u64) -> u64 {
    std::env::var("SOAK_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

// REQ-GRV-026 (M7.1) — Decision: a failure in `check_attribution` means a swarm's retention accessor reads empty or the summed (swarm, structure) rows explain < 90% of the retained heap, so a cap chosen from the table would target the wrong owner.
// REQ-GRV-007 / REQ-GRV-017 / REQ-GRV-SEC-002 / REQ-GRV-004 (F-13) — Decision: a failure means a sustained multi-topic flood through the real node grows retained heap past 64 MiB, delays attestation verdicts past 2 s, relays forged zero-flow txs or attestation variants past B, or drains C's bucket for B so a genuine attester goes dark.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::await_holding_lock)]
async fn soak() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (report, table) = run(soak_secs(30)).await;
    check(&report);
    attribution::check_attribution(&table);
}

/// The 10-minute REQ-GRV-007 soak. Runner:
/// `SOAK_SECS=600 cargo test -p doli-node --release --test inc_i_237_m5_soak -- --ignored --nocapture soak_10min`
// REQ-GRV-007 (F-13) — Decision: same as `soak`, at the 10-minute duration REQ-GRV-007 names; a failure the 30 s run misses is a slow leak.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "10-minute soak; run explicitly (see doc comment)"]
#[allow(clippy::await_holding_lock)]
async fn soak_10min() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    check(&run(soak_secs(600)).await.0);
}
