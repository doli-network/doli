//! INC-I-293 run 648 R-2 (SEC-FIXVERIFY-647-103 = REV-647-001): above the gate the
//! by-hash walk started by an orphan (F2) and the attestation-named refetch (F1)
//! ask `best_peer_for_recovery`, the peer with the greatest reported height. When
//! the heavier branch is not the tallest report, that peer sits on our own lighter
//! branch, answers None, and the root of the heavier branch is never fetched.
//! Operator ruling: ask a peer that showed the heavier branch (the orphan's sender,
//! the relayer set, a peer whose reported tip is on that branch), never the tallest.
//!
//! F2 rig (no network handle): registered peers with reported heights; the test
//! answers every `next_request()` by its target peer. F1 rig: real loopback libp2p
//! services on the devnet network id (node + relayer P1 + P_tall + P_heavy).
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(orphan, sender) -> try_trigger_fork_recovery
//   (F2) and fn Node::run_periodic_tasks() ATTEST_FETCH refetch (F1)
//   O1: chain_state best_hash / best_height, canonical entry, persisted tip, sync tip
//   O2: the outbound GetBlockByHash for the heavier branch's missing root(s): which peer
//     receives it (F2: next_request() target; F1: per-service request logs)
//   O3: the walk / tracker after the attempt: is_fork_recovery_active (F2) and the
//     attest_fetch_tracker entry for R1 (F1); the orphan cached (F2)
// PATHS: A (F2, above AH) heavier branch R1..R3 shorter than D1..D5, sender P_heavy
//   reports F+3, P_tall reports F+5 on our tip (red); C = A with P_heavy the tallest
//   report (control); E = A below the AH (pin); B (F1, above AH) relayer P1 gone, P_tall
//   (F+4, our tip D4) and P_heavy (F+3, R3, serves R1..R6) connected (red); D = B with
//   the relayer connected (control); G = B below the AH (pin)
// INPUT PARTITIONS: AH = 0 (devnet, above) | AH = 1e9 (below); sender/heavy peer tallest |
//   not; relayer alive | gone; D 2 producers, R 3 producers; fixed-seed keys
// MATRIX: A x O1 O2 -> r103a · C x O1 O2 -> r103c · E x O1 O2 O3 -> r103e
//   · B x O1 O2 -> r103b · D x O1 O2 -> r103d · G x O1 O2 O3 -> r103f
//! covers: block_handling fork_walk periodic branch_support sync/manager
//! (best_peer_for_recovery)

#[path = "support/block_sig.rs"]
mod block_sig;
#[path = "it/inc_i_289_common.rs"]
mod inc_i_289_common;
#[path = "it/inc_i_237_m2_node_common.rs"]
#[allow(dead_code)]
mod net_common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crypto::{Hash, KeyPair};
use doli_core::validation::ValidationMode;
use doli_core::{Block, Network, NetworkParams};
use inc_i_289_common::*;
use net_common::{addr, reserve_port};
use network::protocols::{SyncRequest, SyncResponse};
use network::{NetworkConfig, NetworkEvent, NetworkService, PeerId};

fn short(h: &Hash) -> String {
    h.to_string()[..8].to_string()
}

fn assert_above_gate() {
    let ah = NetworkParams::load(Network::Devnet).inc_i_288_fork_choice_activation_height;
    assert!(ah <= F, "precondition: devnet AH {ah} is at or below F");
}

// ---------------------------------------------------------------- F2: the walk

/// The bound, in slots from the orphan R3.
const N: usize = 3;

struct Walk {
    r: Rig,
    /// R1..R3 at F+1..F+3.
    rb: Vec<Block>,
    heavy: PeerId,
    tall: PeerId,
    last_slot: u32,
}

#[derive(Default)]
struct WalkObs {
    /// (slot 1..=N, target peer, hash) of every GetBlockByHash handed out.
    fetched: Vec<(usize, PeerId, Hash)>,
    walk_active_after_orphan: bool,
    own_blocks: usize,
}

fn walk_keys() -> Vec<KeyPair> {
    (1..=3u8).map(|i| KeyPair::from_seed([i; 32])).collect()
}

fn is_led_by(keys: &[KeyPair], s: u32, by: &[usize]) -> bool {
    let l = leader(keys, s).public_key();
    by.iter().any(|i| keys[*i].public_key() == l)
}

/// D1..D5 (keys[0], keys[1]) applied on F; R1..R3 on F in the 3 slots after D5
/// (one per leader). P_tall reports (F+5, D5); P_heavy reports (`heavy_height`, R3).
async fn arrange_walk(heavy_height: u64) -> Walk {
    let keys = walk_keys();
    let d_slots: Vec<u32> = (F as u32 + 1..)
        .filter(|s| is_led_by(&keys, *s, &[0, 1]))
        .take(5)
        .collect();
    let r = rig_with_keys(keys, &d_slots).await;
    let d5 = *d_slots.last().unwrap();
    let rb = chain_on(&r, r.f_hash, F + 1, &[d5 + 1, d5 + 2, d5 + 3]);
    let d_by: HashSet<_> = r.ours.iter().map(|b| b.header.producer).collect();
    let r_by: HashSet<_> = rb.iter().map(|b| b.header.producer).collect();
    assert!(
        d_by.len() == 2 && r_by.len() == 3,
        "fixture: D by 2 producers, R by 3"
    );
    let top = r.ours.last().unwrap().clone();
    let tall = peer_at(&r, F + 5, top.hash(), top.header.slot).await;
    let heavy = peer_at(&r, heavy_height, rb[2].hash(), rb[2].header.slot).await;
    Walk {
        r,
        rb,
        heavy,
        tall,
        last_slot: d5 + 3,
    }
}

/// One own block (keys[0], its next led slot) on the current tip.
async fn produce_own(w: &mut Walk) {
    let (tip, height) = {
        let cs = w.r.node.chain_state.read().await;
        (cs.best_hash, cs.best_height)
    };
    let slot = (w.last_slot + 1..)
        .find(|s| is_led_by(&w.r.keys, *s, &[0]))
        .unwrap();
    w.last_slot = slot;
    let b = block(height + 1, slot, tip, &w.r.keys[0], &w.r.node.params);
    w.r.node
        .apply_block(b, ValidationMode::Light)
        .await
        .expect("own block applies");
}

/// Each peer answers the by-hash fetches the node hands it: P_heavy from R1..R3,
/// P_tall (our branch) None for every R hash. The walk poll runs after each answer.
async fn serve_peers(w: &mut Walk, slot: usize, obs: &mut WalkObs) {
    for _ in 0..8 {
        let req = w.r.node.sync_manager.write().await.next_request();
        let Some((q, req)) = req else { break };
        let SyncRequest::GetBlockByHash { hash } = req else {
            continue;
        };
        obs.fetched.push((slot, q, hash));
        let blk = (q == w.heavy)
            .then(|| w.rb.iter().find(|b| b.hash() == hash).cloned())
            .flatten();
        let out =
            (w.r.node.sync_manager.write().await).handle_response(q, SyncResponse::Block(blk));
        for b in out {
            w.r.node
                .handle_new_block(b, q)
                .await
                .expect("handle_new_block returns Ok");
        }
        let parent = (w.r.node.sync_manager.read().await).fork_recovery_current_parent();
        let canonical = parent
            .is_some_and(|p| matches!(w.r.node.block_store.get_height_by_hash(&p), Ok(Some(_))));
        if canonical {
            let done = (w.r.node.sync_manager.write().await).check_fork_recovery_connection(true);
            if let Some(done) = done {
                let res = w.r.node.handle_completed_fork_recovery(done).await;
                eprintln!("[R2-F2] completed walk -> {res:?}");
            }
        }
    }
}

async fn on_r(w: &Walk) -> bool {
    w.r.node.block_store.get_hash_by_height(F + 1).unwrap() == Some(w.rb[0].hash())
}

/// Slot 1: P_heavy gossips R3 (an orphan). Each slot: one own block while not on R,
/// the peers answer, one periodic tick, the peers answer again.
async fn walk_rounds(w: &mut Walk) -> WalkObs {
    let mut obs = WalkObs::default();
    for k in 1..=N {
        if k == 1 {
            let r3 = w.rb[2].clone();
            (w.r.node.handle_new_block(r3, w.heavy).await).expect("handle_new_block returns Ok");
            obs.walk_active_after_orphan =
                (w.r.node.sync_manager.read().await).is_fork_recovery_active();
        }
        if !on_r(w).await {
            produce_own(w).await;
            obs.own_blocks += 1;
        }
        serve_peers(w, k, &mut obs).await;
        tick(&mut w.r).await;
        serve_peers(w, k, &mut obs).await;
    }
    obs
}

fn named(w: &Walk, p: &PeerId) -> &'static str {
    match p {
        p if *p == w.heavy => "P_heavy",
        p if *p == w.tall => "P_tall",
        _ => "other",
    }
}

fn describe(w: &Walk, obs: &WalkObs) -> String {
    let f: Vec<String> = (obs.fetched.iter())
        .map(|(k, p, h)| format!("slot {k}: {} <- {}", named(w, p), short(h)))
        .collect();
    format!(
        "walk active after the orphan: {}; by-hash fetches: [{}]; R1 {} R2 {}; own blocks {}",
        obs.walk_active_after_orphan,
        f.join(", "),
        short(&w.rb[0].hash()),
        short(&w.rb[1].hash()),
        obs.own_blocks
    )
}

fn asked(obs: &WalkObs, peer: PeerId, hs: &[Hash]) -> bool {
    (obs.fetched.iter()).any(|(_, p, h)| *p == peer && hs.contains(h))
}

async fn assert_walk_heals(w: &Walk, obs: &WalkObs, label: &str) {
    let roots = [w.rb[0].hash(), w.rb[1].hash()];
    let (tip, h) = {
        let cs = w.r.node.chain_state.read().await;
        (cs.best_hash, cs.best_height)
    };
    assert!(
        asked(obs, w.heavy, &roots) && !asked(obs, w.tall, &roots) && tip == w.rb[2].hash(),
        "{label}: within N={N} slots of the orphan R3 the walk did not fetch R2/R1 from the \
         sender P_heavy (heavier, shorter branch) and stay off the tallest peer P_tall, or \
         the tip is not R3: tip h={h} {}, want R3 h={} {}; {}",
        short(&tip),
        F + 3,
        short(&w.rb[2].hash()),
        describe(w, obs)
    );
    assert_on(&w.r, &w.rb[2], F + 3, label).await;
}

// R-2 INC-I-293 103 — Decision: a failure means the orphan walk still asks the tallest peer, which sits on our own lighter branch and answers None, so a minority whose heavier branch is shorter never fetches its root and never heals (REV-647-001).
#[tokio::test]
async fn r103a_orphan_walk_asks_the_sender_not_the_tallest_peer() {
    assert_above_gate();
    let mut w = arrange_walk(F + 3).await;
    let obs = walk_rounds(&mut w).await;
    assert_walk_heals(&w, &obs, "r103a (Must)").await;
}

// R-2 INC-I-293 103 control — Decision: a failure means the walk, the bond fork choice or the rig cannot move a taller lighter branch onto a shorter heavier one even when the tallest peer holds it, so r103a's red would not be about the peer choice.
#[tokio::test]
async fn r103c_control_sender_is_also_tallest_walk_heals() {
    assert_above_gate();
    let mut w = arrange_walk(F + 6).await;
    let obs = walk_rounds(&mut w).await;
    assert_walk_heals(&w, &obs, "r103c control").await;
}

// R-2 INC-I-293 103 below-gate pin — Decision: a failure means today's below-AH (mainnet) orphan path changed: below the gate an orphan starts no by-hash walk and asks no peer, and a gated fix must leave that bit-identical (INV-12).
#[tokio::test]
async fn r103e_below_gate_orphan_starts_no_walk_asks_no_peer() {
    if !in_below_ah_child("r103e_below_gate_orphan_starts_no_walk_asks_no_peer") {
        return;
    }
    let mut w = arrange_walk(F + 3).await;
    let obs = walk_rounds(&mut w).await;
    assert!(
        !obs.walk_active_after_orphan && obs.fetched.is_empty(),
        "r103e pin: below the gate an orphan starts a by-hash walk now: {}",
        describe(&w, &obs)
    );
    assert!(
        !on_r(&w).await && tip_hash(&w.r).await != w.rb[2].hash(),
        "r103e pin: below the gate the node now reaches R3 within N={N} slots"
    );
    assert!(cached(&w.r, &w.rb[2].hash()).await, "r103e pin: R3 cached");
}

// ---------------------------------------------------------------- F1: the refetch

/// Periodic ticks allowed (each followed by a TICK_PUMP pump, inside the 30 s TTL).
const BOUND: usize = 30;
const TICK_PUMP: Duration = Duration::from_millis(300);
/// Leaders of slot % 3 = 0, 1 only: the minority branch has 2 producers.
const D_SLOTS: [u32; 4] = [210, 211, 213, 214];
/// Leaders of every residue: the heavier branch has 3 producers.
const R_SLOTS: [u32; 6] = [215, 216, 217, 218, 219, 220];

type Asked = Arc<Mutex<Vec<Hash>>>;

struct Fetch {
    r: Rig,
    rb: Vec<Block>,
    p1: Asked,
    tall: Asked,
    heavy: Asked,
    r1_at: Option<usize>,
    tracker_has_r1: bool,
}

fn fetch_keys() -> Vec<KeyPair> {
    (1..=3u8)
        .map(|i| KeyPair::from_seed([0x90 + i; 32]))
        .collect()
}

async fn start_net_as(network_id: u32, genesis_hash: Hash, port: u16) -> NetworkService {
    std::env::set_var("DOLI_IP_COLOCATION_THRESHOLD", "500");
    let config = NetworkConfig {
        listen_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        bootstrap_nodes: vec![],
        max_peers: 8,
        bootstrap_slots: 8,
        no_dht: true,
        enable_discv5: false,
        node_key_path: None,
        peer_cache_path: None,
        memory_watchdog_threshold_bytes: 0,
        network_id,
        genesis_hash,
        ..NetworkConfig::default()
    };
    NetworkService::new(config)
        .await
        .expect("start NetworkService")
}

/// A mock peer serving `chain` (by hash, headers, bodies); logs every by-hash ask.
fn serve(mut svc: NetworkService, chain: Vec<Block>) -> (Asked, tokio::task::JoinHandle<()>) {
    let asked: Asked = Arc::default();
    let log = asked.clone();
    let task = tokio::spawn(async move {
        while let Some(ev) = svc.next_event().await {
            let NetworkEvent::SyncRequest {
                request, channel, ..
            } = ev
            else {
                continue;
            };
            let resp = match request {
                SyncRequest::GetBlockByHash { hash } => {
                    log.lock().unwrap().push(hash);
                    SyncResponse::Block(chain.iter().find(|b| b.hash() == hash).cloned())
                }
                SyncRequest::GetHeaders {
                    start_hash,
                    max_count,
                } => {
                    let from = (chain.iter())
                        .position(|b| b.header.prev_hash == start_hash)
                        .unwrap_or(chain.len());
                    let hs = chain[from..].iter().take(max_count as usize);
                    SyncResponse::Headers(hs.map(|b| b.header.clone()).collect())
                }
                SyncRequest::GetBodies { hashes } => SyncResponse::Bodies(
                    (chain.iter())
                        .filter(|b| hashes.contains(&b.hash()))
                        .cloned()
                        .collect(),
                ),
                _ => continue,
            };
            let _ = svc.send_sync_response(channel, resp).await;
        }
    });
    (asked, task)
}

/// Replies and disconnects go through the production dispatch; connects only
/// update `live` (no status exchange: the reports are the `add_peer` ones).
async fn pump(r: &mut Rig, dur: Duration, live: &mut HashSet<PeerId>) {
    let until = Instant::now() + dur;
    while Instant::now() < until {
        let net = r.node.network.as_mut().expect("node network attached");
        let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(25), net.next_event()).await
        else {
            continue;
        };
        match &ev {
            NetworkEvent::PeerConnected(p) => {
                live.insert(*p);
                continue;
            }
            NetworkEvent::PeerDisconnected(p) => {
                live.remove(p);
            }
            NetworkEvent::SyncResponse { .. } => {}
            _ => continue,
        }
        let _ = r.node.handle_network_event(ev).await;
    }
}

async fn pump_until(
    r: &mut Rig,
    live: &mut HashSet<PeerId>,
    done: impl Fn(&HashSet<PeerId>) -> bool,
) -> bool {
    for _ in 0..100 {
        if done(live) {
            return true;
        }
        pump(r, Duration::from_millis(100), live).await;
    }
    done(live)
}

fn port_closed(port: u16) -> bool {
    let a = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    std::net::TcpStream::connect_timeout(&a, Duration::from_millis(200)).is_err()
}

async fn r1_on_node(r: &Rig, r1: &Block) -> bool {
    stored(r, r1) || cached(r, &r1.hash()).await || r.node.wedge_retained_tips.contains(&r1.hash())
}

/// The node on D1..D4; P1 (relayer, serves R), P_tall (reports F+4 / D4, serves
/// D) and P_heavy (reports F+3 / R3, serves R1..R6) connected. P1 relayed 3
/// attestations naming R1; unless `relayer_stays` it is shut down before the ticks.
async fn fetch_scenario(relayer_stays: bool) -> Fetch {
    let mut r = rig_with_keys(fetch_keys(), &D_SLOTS).await;
    let rb = chain_on(&r, r.f_hash, F + 1, &R_SLOTS);
    let mut base = Vec::new();
    for h in 1..=F {
        let hh = r.node.block_store.get_hash_by_height(h).unwrap().unwrap();
        base.push(r.node.block_store.get_block(&hh).unwrap().unwrap());
    }
    let r_chain: Vec<Block> = base.iter().chain(rb.iter()).cloned().collect();
    let d_chain: Vec<Block> = base.iter().chain(r.ours.iter()).cloned().collect();

    let ports = [
        reserve_port(),
        reserve_port(),
        reserve_port(),
        reserve_port(),
    ];
    let (nid, gh) = (
        r.node.config.network.id(),
        r.node.chain_state.read().await.genesis_hash,
    );
    r.node.network = Some(start_net_as(nid, gh, ports[0]).await);
    let p1 = start_net_as(nid, gh, ports[1]).await;
    let pt = start_net_as(nid, gh, ports[2]).await;
    let ph = start_net_as(nid, gh, ports[3]).await;
    let ids = [
        *p1.local_peer_id(),
        *pt.local_peer_id(),
        *ph.local_peer_id(),
    ];
    let (p1_asked, p1_task) = serve(p1, r_chain.clone());
    let (tall_asked, _t) = serve(pt, d_chain);
    let (heavy_asked, _h) = serve(ph, r_chain);
    for p in &ports[1..] {
        (r.node.network.as_ref().unwrap())
            .connect(&addr(*p))
            .await
            .unwrap();
    }
    let mut live = HashSet::new();
    let all = pump_until(&mut r, &mut live, |l| ids.iter().all(|p| l.contains(p))).await;
    assert!(
        all,
        "fixture: the node is connected to P1, P_tall and P_heavy"
    );

    let (d4, r3) = (r.ours[3].clone(), rb[2].clone());
    {
        let mut sm = r.node.sync_manager.write().await;
        sm.add_peer(ids[0], F + 3, r3.hash(), r3.header.slot);
        sm.add_peer(ids[1], F + 4, d4.hash(), d4.header.slot);
        sm.add_peer(ids[2], F + 3, r3.hash(), r3.header.slot);
    }
    let aged = Instant::now() - Duration::from_millis(600);
    (r.node.attest_fetch_tracker).insert(rb[0].hash(), (aged, 3, ids[0]));

    if !relayer_stays {
        p1_task.abort();
        (r.node.network.as_ref().unwrap())
            .disconnect(ids[0])
            .await
            .unwrap();
        let gone = pump_until(&mut r, &mut live, |l| !l.contains(&ids[0])).await;
        assert!(gone, "fixture: the node saw P1 disconnect");
        let mut closed = false;
        for _ in 0..50 {
            closed = port_closed(ports[1]);
            if closed {
                break;
            }
            pump(&mut r, Duration::from_millis(100), &mut live).await;
        }
        assert!(closed, "fixture: P1 is gone (its port refuses connections)");
    }

    let mut run = Fetch {
        r,
        rb,
        p1: p1_asked,
        tall: tall_asked,
        heavy: heavy_asked,
        r1_at: None,
        tracker_has_r1: false,
    };
    for i in 0..BOUND {
        if tip_hash(&run.r).await == run.rb[5].hash() {
            break;
        }
        tick(&mut run.r).await;
        pump(&mut run.r, TICK_PUMP, &mut live).await;
        if run.r1_at.is_none() && r1_on_node(&run.r, &run.rb[0]).await {
            run.r1_at = Some(i);
            for b in &run.rb[1..] {
                gossip(&mut run.r, b).await;
            }
        }
    }
    run.tracker_has_r1 = (run.r.node.attest_fetch_tracker).contains_key(&run.rb[0].hash());
    eprintln!(
        "[R2-F1] relayer_stays={relayer_stays} {}",
        fetch_summary(&run).await
    );
    run
}

fn n_asked(log: &Asked, b: &Block) -> usize {
    log.lock()
        .unwrap()
        .iter()
        .filter(|h| **h == b.hash())
        .count()
}

async fn fetch_summary(f: &Fetch) -> String {
    let r1 = &f.rb[0];
    format!(
        "GetBlockByHash(R1) received: P1 {} P_tall {} P_heavy {}; r1_at={:?}; tip {} \
         (R6 {}); tracker_has_r1={}",
        n_asked(&f.p1, r1),
        n_asked(&f.tall, r1),
        n_asked(&f.heavy, r1),
        f.r1_at,
        short(&tip_hash(&f.r).await),
        short(&f.rb[5].hash()),
        f.tracker_has_r1
    )
}

// R-2 INC-I-293 103 — Decision: a failure means the attestation refetch still falls back to the tallest peer, which sits on our own lighter branch and answers None every tick, so with the relayer gone the heavier branch root is never fetched from the peer that holds it (REV-647-001).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r103b_refetch_asks_a_peer_on_the_heavier_branch_not_the_tallest_peer() {
    assert_above_gate();
    let f = fetch_scenario(false).await;
    assert!(
        n_asked(&f.heavy, &f.rb[0]) > 0,
        "r103b (Must): within {BOUND} ticks P_heavy (reported tip R3, serves R1..R6) never \
         received GetBlockByHash(R1); the refetch asked only the tallest peer: {}",
        fetch_summary(&f).await
    );
    let label = format!("r103b (Must): minority not on the heavier tip R6 within {BOUND} ticks");
    assert_on(&f.r, &f.rb[5], F + 6, &label).await;
}

// R-2 INC-I-293 103 control — Decision: a failure means the four-service rig cannot deliver R1 and reach R6 even from a live relayer, so r103b's red would not isolate the fallback peer choice.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r103d_control_relayer_connected_root_fetched_from_relayer() {
    assert_above_gate();
    let f = fetch_scenario(true).await;
    assert!(
        n_asked(&f.p1, &f.rb[0]) > 0 && f.r1_at.is_some(),
        "r103d control: the live relayer P1 serves R1: {}",
        fetch_summary(&f).await
    );
    let label = format!("r103d control: on R6 within {BOUND} ticks");
    assert_on(&f.r, &f.rb[5], F + 6, &label).await;
}

// R-2 INC-I-293 103 below-gate pin — Decision: a failure means today's below-AH (mainnet) attestation fetch changed: below the gate it is one shot to the recorded relayer only (the tracker entry is dropped after firing) and no other peer is asked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r103f_below_gate_fetch_is_one_shot_to_the_relayer_only() {
    if !in_below_ah_child("r103f_below_gate_fetch_is_one_shot_to_the_relayer_only") {
        return;
    }
    let f = fetch_scenario(false).await;
    let s = fetch_summary(&f).await;
    assert!(
        n_asked(&f.tall, &f.rb[0]) == 0 && n_asked(&f.heavy, &f.rb[0]) == 0,
        "r103f pin: below the gate a peer other than the relayer now gets GetBlockByHash(R1): {s}"
    );
    assert!(
        !f.tracker_has_r1 && f.r1_at.is_none(),
        "r103f pin: below the gate the one-shot entry is dropped after firing and R1 never \
         arrives: {s}"
    );
    assert_ne!(
        tip_hash(&f.r).await,
        f.rb[5].hash(),
        "r103f pin: below the gate the node now reaches R6 within {BOUND} ticks"
    );
}
