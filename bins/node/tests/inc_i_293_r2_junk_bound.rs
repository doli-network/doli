//! INC-I-293 R-2 (run 648), SEC-FIXVERIFY-647-102: above the INC-I-288 gate the
//! attestation-named refetch (periodic.rs ATTEST_FETCH, branch_support.rs
//! `attest_fetch_peer`) falls back to the highest connected peer once the
//! recorded relayer has left the sync peer table. Junk hashes named only by a
//! relayer that has gone are then fetched from an honest third party at up to
//! FETCH_CAP_PER_TICK = 8 per tick for the tracker's 30 s life.
//! Operator rule: the refetch asks only a peer that showed the branch (the
//! relayer set, or a peer whose tip is on that branch), bounded per peer. A
//! peer that never showed a junk hash is never asked for it (B = 0).
//! Rig: the F1 rig (inc_i_293_f1_attest_fetch.rs): the node's own loopback
//! `NetworkService`, two bare services, tracker entries inserted directly,
//! per-peer GetBlockByHash logs, a relayer whose port is closed.
//!
// OUTPUT CONTRACT: fn Node::run_periodic_tasks() (INC-I-049 ATTEST_FETCH,
//   periodic.rs:350-424 + branch_support.rs:185-190), then
//   fn Node::handle_network_event(SyncResponse | PeerDisconnected), then
//   fn Node::handle_new_block (R2..R6 gossip, control only)
//   O1: outbound GetBlockByHash per hash per peer (P2's request log)
//   O2: attest_fetch_tracker entries for the 9 junk hashes
//   O3: chain tip (best hash / height) and R1 on the node (control)
//   O4: P2 still connected and in the sync peer table (non-vacuity of O1 = 0)
// PATHS: A above gate, junk relayed only by P_gone, P_gone gone, P2 tallest;
//   C path A plus an R1 entry relayed by P2 (legitimate fetch);
//   D path A below the gate (AH env override, child process)
// INPUT PARTITIONS: AH = 0 (devnet, above) | AH = 1e9 (below);
//   relayer of a hash connected | gone; asked peer showed the hash | never did
// MATRIX: R102A O1 O4 (O2 open above the gate: keep or drop is the fix's choice)
//   · R102C O1(R1) O3 · R102D O1 O2
//
//! covers: periodic.rs (ATTEST_FETCH) branch_support.rs (attest_fetch_peer)

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
use doli_core::{Block, Network, NetworkParams};
use inc_i_289_common::*;
use net_common::{addr, reserve_port};
use network::protocols::{SyncRequest, SyncResponse};
use network::{NetworkConfig, NetworkEvent, NetworkService, PeerId};

/// Ticks for the junk cells (each followed by TICK_PUMP, then one DRAIN pump).
/// At 8 per tick today P2 receives 8 * K = 40 junk requests.
const K: usize = 5;
/// Junk requests P2 may receive in K ticks: it never showed any junk hash.
const B: usize = 0;
/// Control: ticks allowed for R1 fetch and the reorg to R6 (as F1).
const BOUND: usize = 30;
const TICK_PUMP: Duration = Duration::from_millis(300);
const DRAIN: Duration = Duration::from_millis(1_000);
/// One more than FETCH_CAP_PER_TICK, so the cap itself is crossed.
const JUNK_N: u8 = 9;
const D_SLOTS: [u32; 4] = [210, 211, 213, 214];
const R_SLOTS: [u32; 6] = [215, 216, 217, 218, 219, 220];

type Asked = Arc<Mutex<Vec<Hash>>>;

struct Run {
    r: Rig,
    rb: Vec<Block>,
    p2_id: PeerId,
    p2_asked: Asked,
    live: HashSet<PeerId>,
    r1_at: Option<usize>,
    ticks: usize,
}

/// Fixed bytes: no block hashes to these, so no peer can serve them.
fn junk() -> Vec<Hash> {
    (0..JUNK_N)
        .map(|i| Hash::from_bytes([0xD0 + i; 32]))
        .collect()
}

fn keys() -> Vec<KeyPair> {
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

/// An honest mock peer: serves `chain` by hash, headers and bodies (junk gets
/// `Block(None)`) and logs every GetBlockByHash hash it receives.
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
/// update `live` (no status exchange).
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

/// P_gone relays (tracker entries) the 9 junk hashes and leaves before the
/// first tick. P2 is connected, tallest, honest, and relays none of them; with
/// `p2_relays_r1` it also relays R1 (the oldest entry) and R2..R6 are gossiped
/// once R1 is on the node.
async fn scenario(p2_relays_r1: bool, max_ticks: usize) -> Run {
    let mut r = rig_with_keys(keys(), &D_SLOTS).await;
    let rb = chain_on(&r, r.f_hash, F + 1, &R_SLOTS);
    let mut chain = Vec::new();
    for h in 1..=F {
        let hh = r.node.block_store.get_hash_by_height(h).unwrap().unwrap();
        chain.push(r.node.block_store.get_block(&hh).unwrap().unwrap());
    }
    chain.extend(rb.iter().cloned());

    let (pm, pg_port, p2_port) = (reserve_port(), reserve_port(), reserve_port());
    let (nid, gh) = (
        r.node.config.network.id(),
        r.node.chain_state.read().await.genesis_hash,
    );
    r.node.network = Some(start_net_as(nid, gh, pm).await);
    let pg = start_net_as(nid, gh, pg_port).await;
    let p2 = start_net_as(nid, gh, p2_port).await;
    let (pg_id, p2_id) = (*pg.local_peer_id(), *p2.local_peer_id());
    let (_pg_asked, pg_task) = serve(pg, chain.clone());
    let (p2_asked, _p2_task) = serve(p2, chain);
    let net = r.node.network.as_ref().unwrap();
    net.connect(&addr(pg_port)).await.unwrap();
    net.connect(&addr(p2_port)).await.unwrap();
    let mut live = HashSet::new();
    let both = pump_until(&mut r, &mut live, |l| {
        l.contains(&pg_id) && l.contains(&p2_id)
    })
    .await;
    assert!(both, "fixture: the node is connected to P_gone and P2");

    let (top, below) = (rb.last().unwrap(), &rb[4]);
    {
        let mut sync = r.node.sync_manager.write().await;
        sync.add_peer(pg_id, F + 5, below.hash(), below.header.slot);
        sync.add_peer(p2_id, F + 6, top.hash(), top.header.slot);
    }
    let now = Instant::now();
    for (i, h) in junk().iter().enumerate() {
        let aged = now - Duration::from_millis(600 + i as u64);
        r.node.attest_fetch_tracker.insert(*h, (aged, 3, pg_id));
    }
    if p2_relays_r1 {
        let aged = now - Duration::from_millis(1_000);
        r.node
            .attest_fetch_tracker
            .insert(rb[0].hash(), (aged, 3, p2_id));
    }

    pg_task.abort();
    let net = r.node.network.as_ref().unwrap();
    net.disconnect(pg_id).await.unwrap();
    let gone = pump_until(&mut r, &mut live, |l| !l.contains(&pg_id)).await;
    assert!(gone, "fixture: the node saw P_gone disconnect");
    let mut closed = false;
    for _ in 0..50 {
        closed = port_closed(pg_port);
        if closed {
            break;
        }
        pump(&mut r, Duration::from_millis(100), &mut live).await;
    }
    assert!(
        closed,
        "fixture: P_gone is gone (its port refuses connections)"
    );
    assert!(
        !r.node.sync_manager.read().await.has_peer(&pg_id),
        "fixture: P_gone left the sync peer table"
    );

    let mut run = Run {
        r,
        rb,
        p2_id,
        p2_asked,
        live,
        r1_at: None,
        ticks: max_ticks,
    };
    for i in 0..max_ticks {
        if p2_relays_r1 && tip_hash(&run.r).await == run.rb[5].hash() {
            run.ticks = i;
            break;
        }
        tick(&mut run.r).await;
        pump(&mut run.r, TICK_PUMP, &mut run.live).await;
        if p2_relays_r1 && run.r1_at.is_none() && r1_on_node(&run.r, &run.rb[0]).await {
            run.r1_at = Some(i);
            for b in &run.rb[1..] {
                gossip(&mut run.r, b).await;
            }
        }
    }
    pump(&mut run.r, DRAIN, &mut run.live).await;
    // Each read takes the log lock on its own (a guard held across the
    // eprintln arguments would deadlock the next lock).
    let total = run.p2_asked.lock().unwrap().len();
    let (junk_n, r1_n) = (
        junk_asked(&run.p2_asked),
        asked_for(&run.p2_asked, &run.rb[0]),
    );
    eprintln!(
        "[R2-102] p2_relays_r1={p2_relays_r1} ticks={} r1_at={:?} p2_junk_asked={junk_n} \
         p2_asked_total={total} p2_asked_r1={r1_n} junk_in_tracker={} p2_live={}",
        run.ticks,
        run.r1_at,
        junk_in_tracker(&run.r),
        run.live.contains(&run.p2_id),
    );
    run
}

/// GetBlockByHash requests (with repeats) P2 received for the junk hashes.
fn junk_asked(log: &Asked) -> usize {
    let j: HashSet<Hash> = junk().into_iter().collect();
    log.lock().unwrap().iter().filter(|h| j.contains(h)).count()
}

fn junk_in_tracker(r: &Rig) -> usize {
    (junk().iter())
        .filter(|h| r.node.attest_fetch_tracker.contains_key(h))
        .count()
}

fn asked_for(log: &Asked, b: &Block) -> bool {
    log.lock().unwrap().contains(&b.hash())
}

fn assert_above_gate() {
    let ah = NetworkParams::load(Network::Devnet).inc_i_288_fork_choice_activation_height;
    assert!(
        ah <= F,
        "precondition: devnet AH {ah} is at or below the fork point"
    );
}

// R102A INC-I-293 SEC-FIXVERIFY-647-102 — Decision: a failure means junk named only by a gone relayer is still fetched from an honest peer that never showed it, so three victims can still spend that peer's 24-per-interval serving budget and the fix is not done.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r102a_inc_i_293_gone_relayers_junk_not_fetched_from_honest_tallest_peer() {
    assert_above_gate();
    let run = scenario(false, K).await;
    let n = junk_asked(&run.p2_asked);
    assert_eq!(
        n, B,
        "R102A: over K={K} ticks the honest tallest peer P2, which never showed any of the \
         {JUNK_N} junk hashes, received {n} GetBlockByHash requests for them (bound B={B}); \
         their only relayer P_gone is gone"
    );
    let in_table = (run.r.node.sync_manager.read().await).has_peer(&run.p2_id);
    assert!(
        run.live.contains(&run.p2_id) && in_table,
        "R102A non-vacuity: P2 stayed connected and in the sync peer table"
    );
}

// R102C INC-I-293 SEC-FIXVERIFY-647-102 — Decision: a failure means the rig cannot count P2's requests or the fix also stops the legitimate fetch from a peer that relayed the root, so a minority would again stay on its dead branch.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r102c_inc_i_293_control_root_relayed_by_p2_fetched_from_p2_and_node_reorgs() {
    assert_above_gate();
    let run = scenario(true, BOUND).await;
    assert!(
        asked_for(&run.p2_asked, &run.rb[0]),
        "R102C: P2, which relayed R1 itself, never received GetBlockByHash(R1)"
    );
    assert!(run.r1_at.is_some(), "R102C: R1 reached the node");
    assert_on(
        &run.r,
        &run.rb[5],
        F + 6,
        &format!("R102C: control on R6 within {BOUND} ticks"),
    )
    .await;
}

// R102D INC-I-293 SEC-FIXVERIFY-647-102 — Decision: a failure means the below-gate one-shot fetch to the recorded relayer changed (it now reaches a third peer or keeps the entry), so the fix leaked below the gate onto mainnet.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r102d_inc_i_293_below_gate_pins_one_shot_fetch_to_gone_relayer_only() {
    if !in_below_ah_child("r102d_inc_i_293_below_gate_pins_one_shot_fetch_to_gone_relayer_only") {
        return;
    }
    let run = scenario(false, K).await;
    // Pinned today (776e37a0): below the gate each junk entry fires once at the
    // recorded (gone) relayer and is dropped, so P2 is never asked for junk.
    assert_eq!(
        junk_asked(&run.p2_asked),
        0,
        "R102D pin: below the gate P2 now receives GetBlockByHash for the junk hashes"
    );
    assert_eq!(
        junk_in_tracker(&run.r),
        0,
        "R102D pin: below the gate a junk entry now outlives its one-shot fetch (K={K} ticks)"
    );
}
