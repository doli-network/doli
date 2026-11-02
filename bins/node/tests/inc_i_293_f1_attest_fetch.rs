//! INC-I-293 F1 (run 647, Q-2): the attestation-named block fetch is one-shot to
//! the last relayer. A minority node on its own branch D1..D4 (2 producers) holds
//! a tracker entry for R1, the root of the heavier branch R1..R6 (3 producers),
//! recorded from 3 attestations relayed by P1 (the cap). P1 is gone before the
//! fetch tick; P2 stays connected and serves every block by hash, headers and
//! bodies. Contract: within BOUND ticks the node asks a connected peer for R1 and
//! ends on R6. R2..R6 are gossiped only after R1 is on the node (root-first, as in
//! trial .3), so F3 (cached children of a retained root) stays out of this test.
//! Real loopback libp2p: the node's own `NetworkService` plus two bare services.
//!
// OUTPUT CONTRACT: fn Node::run_periodic_tasks() (INC-I-049 deferred ATTEST_FETCH,
//   periodic.rs:350-408), then fn Node::handle_network_event(SyncResponse |
//   PeerDisconnected) for every reply, then fn Node::handle_new_block (R2..R6 gossip)
//   O1: chain_state best_hash / best_height, canonical entry, persisted tip, sync tip
//   O2: the outbound GetBlockByHash(R1): which mock peer receives it (P1 / P2 logs)
//   O3: R1 on the node (block_store, fork_block_cache or wedge_retained_tips)
// PATHS: A relayer P1 gone before the first fetch tick, P2 connected and serving;
//   C relayer P1 connected and serving (control); B path A below the INC-I-288 AH
// INPUT PARTITIONS: AH = 0 (devnet, above) | AH = 1e9 (below); relayer alive | gone
// MATRIX: F1A O1-O3 · F1C O1-O3 · F1B O1-O3 (pins today's below-gate behaviour)
//
//! covers: periodic.rs (ATTEST_FETCH) network_events.rs (tracker) command_handling.rs

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

/// Periodic ticks allowed (each followed by a TICK_PUMP network pump, ~9 s in
/// all, inside the tracker's 30 s TTL). The lab's R1 window is 30 slots.
const BOUND: usize = 30;
const TICK_PUMP: Duration = Duration::from_millis(300);
/// Leaders of slot % 3 = 0, 1 only: the minority branch has 2 producers.
const D_SLOTS: [u32; 4] = [210, 211, 213, 214];
/// Leaders of every residue: the heavier branch has 3 producers.
const R_SLOTS: [u32; 6] = [215, 216, 217, 218, 219, 220];

type Asked = Arc<Mutex<Vec<Hash>>>;

struct Run {
    r: Rig,
    rb: Vec<Block>,
    p1_asked: Asked,
    p2_asked: Asked,
    /// Tick index at which R1 was first on the node.
    r1_at: Option<usize>,
    /// Ticks run until the tip was R6 (BOUND if never).
    ticks: usize,
}

/// A loopback service on the node's network id and genesis, so no side drops
/// the other for a network mismatch (`start_net_as` of the M3 republish test).
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

fn keys() -> Vec<KeyPair> {
    (1..=3u8)
        .map(|i| KeyPair::from_seed([0x90 + i; 32]))
        .collect()
}

/// A mock peer: answers GetBlockByHash / GetHeaders / GetBodies from `chain`,
/// logs every GetBlockByHash hash it receives, ignores everything else.
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

/// Feeds the node its own network events: replies and disconnects go through
/// the production dispatch; connects only update `live` (no status exchange).
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

async fn scenario(relayer_stays: bool) -> Run {
    let mut r = rig_with_keys(keys(), &D_SLOTS).await;
    let rb = chain_on(&r, r.f_hash, F + 1, &R_SLOTS);
    let mut chain = Vec::new();
    for h in 1..=F {
        let hh = r.node.block_store.get_hash_by_height(h).unwrap().unwrap();
        chain.push(r.node.block_store.get_block(&hh).unwrap().unwrap());
    }
    chain.extend(rb.iter().cloned());

    let (pm, p1_port, p2_port) = (reserve_port(), reserve_port(), reserve_port());
    let (nid, gh) = (
        r.node.config.network.id(),
        r.node.chain_state.read().await.genesis_hash,
    );
    r.node.network = Some(start_net_as(nid, gh, pm).await);
    let p1 = start_net_as(nid, gh, p1_port).await;
    let p2 = start_net_as(nid, gh, p2_port).await;
    let (p1_id, p2_id) = (*p1.local_peer_id(), *p2.local_peer_id());
    let (p1_asked, p1_task) = serve(p1, chain.clone());
    let (p2_asked, _p2_task) = serve(p2, chain);
    let net = r.node.network.as_ref().unwrap();
    net.connect(&addr(p1_port)).await.unwrap();
    net.connect(&addr(p2_port)).await.unwrap();
    let mut live = HashSet::new();
    let both = pump_until(&mut r, &mut live, |l| {
        l.contains(&p1_id) && l.contains(&p2_id)
    })
    .await;
    assert!(both, "fixture: the node is connected to P1 and P2");

    let top = rb.last().unwrap();
    for p in [p1_id, p2_id] {
        (r.node.sync_manager.write().await).add_peer(p, F + 6, top.hash(), top.header.slot);
    }
    // 3 attestations naming R1, relayed by P1 (the cap), grace already elapsed.
    let aged = Instant::now() - Duration::from_millis(600);
    (r.node.attest_fetch_tracker).insert(rb[0].hash(), (aged, 3, p1_id));

    if !relayer_stays {
        p1_task.abort();
        let net = r.node.network.as_ref().unwrap();
        net.disconnect(p1_id).await.unwrap();
        let gone = pump_until(&mut r, &mut live, |l| !l.contains(&p1_id)).await;
        assert!(gone, "fixture: the node saw P1 disconnect");
        let mut closed = false;
        for _ in 0..50 {
            closed = port_closed(p1_port);
            if closed {
                break;
            }
            pump(&mut r, Duration::from_millis(100), &mut live).await;
        }
        assert!(closed, "fixture: P1 is gone (its port refuses connections)");
    }

    let mut run = Run {
        r,
        rb,
        p1_asked,
        p2_asked,
        r1_at: None,
        ticks: BOUND,
    };
    for i in 0..BOUND {
        if tip_hash(&run.r).await == run.rb[5].hash() {
            run.ticks = i;
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
    let (n1, n2) = (
        run.p1_asked.lock().unwrap().len(),
        run.p2_asked.lock().unwrap().len(),
    );
    let tip_r6 = tip_hash(&run.r).await == run.rb[5].hash();
    eprintln!(
        "[F1] relayer_stays={relayer_stays} r1_at={:?} ticks={} tip_is_r6={} \
         p1_asked={} p2_asked={} p2_asked_r1={} tracker_has_r1={}",
        run.r1_at,
        run.ticks,
        tip_r6,
        n1,
        n2,
        asked_for(&run.p2_asked, &run.rb[0]),
        run.r
            .node
            .attest_fetch_tracker
            .contains_key(&run.rb[0].hash()),
    );
    run
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

// F1A INC-I-293 — Decision: a failure means a lost one-shot fetch to a gone relayer still decides whether a minority ever receives the heavier branch root, so the fetch must retry from a connected peer before the run ships.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn f1a_inc_i_293_relayer_gone_root_fetched_from_connected_peer() {
    assert_above_gate();
    let run = scenario(false).await;
    assert!(
        asked_for(&run.p2_asked, &run.rb[0]),
        "F1A: within {BOUND} ticks the connected peer P2 never received GetBlockByHash(R1); \
         the only fetch went to the gone relayer P1 (r1_at={:?})",
        run.r1_at
    );
    assert_on(
        &run.r,
        &run.rb[5],
        F + 6,
        &format!("F1A: minority not on the heavier tip R6 within {BOUND} ticks"),
    )
    .await;
}

// F1C INC-I-293 — Decision: a failure means the rig cannot deliver R1 even from a live relayer, so F1A's red would not isolate the gone-relayer loss.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn f1c_inc_i_293_control_relayer_connected_node_reorgs_within_bound() {
    assert_above_gate();
    let run = scenario(true).await;
    assert!(
        asked_for(&run.p1_asked, &run.rb[0]),
        "F1C: the live relayer P1 got GetBlockByHash(R1)"
    );
    assert!(run.r1_at.is_some(), "F1C: R1 reached the node");
    assert_on(
        &run.r,
        &run.rb[5],
        F + 6,
        &format!("F1C: control on R6 within {BOUND} ticks"),
    )
    .await;
}

// F1B INC-I-293 — Decision: a failure means below-gate delivery of a gone relayer's root changed (the path is ungated), so the operator must rule whether that change reaches mainnet before the fix ships.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn f1b_inc_i_293_below_gate_pins_today_root_lost_with_relayer_no_recovery_in_bound() {
    if !in_below_ah_child(
        "f1b_inc_i_293_below_gate_pins_today_root_lost_with_relayer_no_recovery_in_bound",
    ) {
        return;
    }
    let run = scenario(false).await;
    // Pinned today (11aee480): the one-shot fetch is lost below the gate too; no
    // other mechanism (blind Rule 1/1b, header-first) asks P2 for R1 or moves the
    // tip to R6 inside the bound.
    assert!(
        !asked_for(&run.p2_asked, &run.rb[0]),
        "F1B pin: below the gate P2 now receives GetBlockByHash(R1) within {BOUND} ticks"
    );
    assert!(
        run.r1_at.is_none(),
        "F1B pin: below the gate R1 now reaches the node"
    );
    assert_ne!(
        tip_hash(&run.r).await,
        run.rb[5].hash(),
        "F1B pin: below the gate the node now reaches R6 within {BOUND} ticks"
    );
}
