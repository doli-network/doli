//! INC-I-293 R-2 (run 648), SEC-FIXVERIFY-647-101: above the INC-I-288 gate the
//! attestation-named refetch keeps each tracker entry until its block is held or
//! the 30 s sweep, takes matured entries oldest-first and fires at most
//! FETCH_CAP_PER_TICK = 8 per tick (periodic.rs:359, :368, :385, :417). Nine live
//! unservable hashes OLDER than a fork root's entry then hold all 8 slots on every
//! tick, so the root is never asked for and expires. Contract: a minority node on
//! D1..D4 that tracks the heavier branch root R1 (relayed by the connected, serving
//! peer P2) asks P2 for R1 within ROOT_BOUND ticks and ends on R6 within BOUND
//! ticks, although P_JUNK (connected, answers `Block(None)` for the junk) relayed 9
//! older junk hashes. R2..R6 are gossiped only after R1 is on the node (as in the
//! f1 rig), so F3 stays out. Real loopback libp2p: the node's own service + two.
//!
// OUTPUT CONTRACT: fn Node::run_periodic_tasks() (INC-I-049 ATTEST_FETCH,
//   periodic.rs:355-425), then fn Node::handle_network_event(SyncResponse) for every
//   reply, then fn Node::handle_new_block (R2..R6 gossip)
//   O1: chain_state best_hash / best_height, canonical entry, persisted tip, sync tip
//   O2: the outbound GetBlockByHash(R1): the tick index at which P2 first logs it,
//       and how often P2 logs it
//   O3: the outbound GetBlockByHash(junk_i) at P_JUNK (count per junk hash)
//   O4: attest_fetch_tracker: junk / root entries kept or removed
// PATHS: A 9 junk entries older than the root, above the gate (red today);
//   C no junk, above the gate (control); D path A below the INC-I-288 AH (pin)
// INPUT PARTITIONS: AH = 0 (devnet, above) | AH = 1e9 (below); junk 0 | 9 (> cap 8)
// MATRIX: R101A O1 O2 (O3 O4 logged) · R101C O1 O2 · R101D O2 O3 O4 (O1 logged)
//
//! covers: periodic.rs (ATTEST_FETCH) network_events.rs (tracker) branch_support.rs

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

/// Periodic ticks allowed for the tip to reach R6 (each followed by a TICK_PUMP
/// network pump, ~10 s in all, inside the tracker's 30 s TTL, so no junk entry
/// expires during the run).
const BOUND: usize = 30;
/// Ticks allowed for P2 to receive GetBlockByHash(R1) (tick indices 0..2).
const ROOT_BOUND: usize = 3;
const TICK_PUMP: Duration = Duration::from_millis(300);
/// One more than FETCH_CAP_PER_TICK (periodic.rs:359).
const JUNK: usize = 9;
/// Leaders of slot % 3 = 0, 1 only: the minority branch has 2 producers.
const D_SLOTS: [u32; 4] = [210, 211, 213, 214];
/// Leaders of every residue: the heavier branch has 3 producers.
const R_SLOTS: [u32; 6] = [215, 216, 217, 218, 219, 220];

type Asked = Arc<Mutex<Vec<Hash>>>;

struct Run {
    r: Rig,
    rb: Vec<Block>,
    junk: Vec<Hash>,
    p2_asked: Asked,
    pj_asked: Asked,
    /// Tick index after whose pump P2 had first logged GetBlockByHash(R1).
    p2_r1_at: Option<usize>,
    /// Tick index at which R1 was first on the node.
    r1_at: Option<usize>,
}

/// Fixed junk hashes: random-looking, served by no peer.
fn junk_hashes() -> Vec<Hash> {
    (0..JUNK as u8)
        .map(|i| {
            let mut b = [0u8; 32];
            for (k, x) in b.iter_mut().enumerate() {
                *x = (k as u8).wrapping_mul(37) ^ 0x5c ^ i.wrapping_mul(101);
            }
            Hash::from_bytes(b)
        })
        .collect()
}

/// A loopback service on the node's network id and genesis (f1 rig).
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

/// A mock peer: answers GetBlockByHash / GetHeaders / GetBodies from `chain`
/// (`Block(None)` for any hash not in it), logs every GetBlockByHash hash.
fn serve(mut svc: NetworkService, chain: Vec<Block>) -> Asked {
    let asked: Asked = Arc::default();
    let log = asked.clone();
    tokio::spawn(async move {
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
    asked
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

async fn r1_on_node(r: &Rig, r1: &Block) -> bool {
    stored(r, r1) || cached(r, &r1.hash()).await || r.node.wedge_retained_tips.contains(&r1.hash())
}

fn count(log: &Asked, h: &Hash) -> usize {
    log.lock().unwrap().iter().filter(|x| *x == h).count()
}

async fn scenario(junk_n: usize) -> Run {
    let mut r = rig_with_keys(keys(), &D_SLOTS).await;
    let rb = chain_on(&r, r.f_hash, F + 1, &R_SLOTS);
    let mut chain = Vec::new();
    for h in 1..=F {
        let hh = r.node.block_store.get_hash_by_height(h).unwrap().unwrap();
        chain.push(r.node.block_store.get_block(&hh).unwrap().unwrap());
    }
    chain.extend(rb.iter().cloned());

    let (pm, pj_port, p2_port) = (reserve_port(), reserve_port(), reserve_port());
    let (nid, gh) = (
        r.node.config.network.id(),
        r.node.chain_state.read().await.genesis_hash,
    );
    r.node.network = Some(start_net_as(nid, gh, pm).await);
    let pj = start_net_as(nid, gh, pj_port).await;
    let p2 = start_net_as(nid, gh, p2_port).await;
    let (pj_id, p2_id) = (*pj.local_peer_id(), *p2.local_peer_id());
    // P_JUNK serves the same chain; the junk is in no chain, so it gets Block(None).
    let pj_asked = serve(pj, chain.clone());
    let p2_asked = serve(p2, chain);
    let net = r.node.network.as_ref().unwrap();
    net.connect(&addr(pj_port)).await.unwrap();
    net.connect(&addr(p2_port)).await.unwrap();
    let mut live = HashSet::new();
    for _ in 0..100 {
        if live.contains(&pj_id) && live.contains(&p2_id) {
            break;
        }
        pump(&mut r, Duration::from_millis(100), &mut live).await;
    }
    assert!(
        live.contains(&pj_id) && live.contains(&p2_id),
        "fixture: the node is connected to P_JUNK and P2"
    );

    let top = rb.last().unwrap();
    for p in [pj_id, p2_id] {
        (r.node.sync_manager.write().await).add_peer(p, F + 6, top.hash(), top.header.slot);
    }
    // Junk entries first and older (each at the 3-attestation cap, relayer
    // P_JUNK); then the root entry, relayer P2. All past the 500 ms grace.
    let junk: Vec<Hash> = junk_hashes().into_iter().take(junk_n).collect();
    let now = Instant::now();
    for (i, h) in junk.iter().enumerate() {
        let t = now - Duration::from_millis(1_000) + Duration::from_millis(i as u64);
        (r.node.attest_fetch_tracker).insert(*h, (t, 3, pj_id));
    }
    let root_t = now - Duration::from_millis(600);
    (r.node.attest_fetch_tracker).insert(rb[0].hash(), (root_t, 3, p2_id));

    let mut run = Run {
        r,
        rb,
        junk,
        p2_asked,
        pj_asked,
        p2_r1_at: None,
        r1_at: None,
    };
    let mut ticks = BOUND;
    for i in 0..BOUND {
        if tip_hash(&run.r).await == run.rb[5].hash() {
            ticks = i;
            break;
        }
        tick(&mut run.r).await;
        pump(&mut run.r, TICK_PUMP, &mut live).await;
        if run.p2_r1_at.is_none() && count(&run.p2_asked, &run.rb[0].hash()) > 0 {
            run.p2_r1_at = Some(i);
        }
        if run.r1_at.is_none() && r1_on_node(&run.r, &run.rb[0]).await {
            run.r1_at = Some(i);
            for b in &run.rb[1..] {
                gossip(&mut run.r, b).await;
            }
        }
    }
    let junk_asks: Vec<usize> = run.junk.iter().map(|h| count(&run.pj_asked, h)).collect();
    let junk_tracked = (run.junk.iter())
        .filter(|h| run.r.node.attest_fetch_tracker.contains_key(h))
        .count();
    eprintln!(
        "[R101] junk={junk_n} p2_r1_at={:?} p2_r1_asks={} r1_at={:?} ticks={ticks} \
         tip_is_r6={} junk_asks_at_pj={junk_asks:?} junk_tracked={junk_tracked} \
         root_tracked={}",
        run.p2_r1_at,
        count(&run.p2_asked, &run.rb[0].hash()),
        run.r1_at,
        tip_hash(&run.r).await == run.rb[5].hash(),
        (run.r.node.attest_fetch_tracker).contains_key(&run.rb[0].hash()),
    );
    run
}

fn assert_above_gate() {
    let ah = NetworkParams::load(Network::Devnet).inc_i_288_fork_choice_activation_height;
    assert!(
        ah <= F,
        "precondition: devnet AH {ah} is at or below the fork point"
    );
}

// R101A INC-I-293 — Decision: a failure means one member key relaying 9 unservable hashes before a fork root starves the above-gate refetch, so a minority can be kept off the heavier branch and the attestation fetch needs a quota or support-first order before the gate ships.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r101a_inc_i_293_root_fetched_although_older_junk_fills_tracker() {
    assert_above_gate();
    let run = scenario(JUNK).await;
    assert!(
        run.p2_r1_at.is_some_and(|i| i < ROOT_BOUND),
        "R101A: P2 did not receive GetBlockByHash(R1) within {ROOT_BOUND} ticks \
         (first at tick {:?} of {BOUND}); {JUNK} older unservable tracker entries \
         took every fetch slot",
        run.p2_r1_at
    );
    assert_on(
        &run.r,
        &run.rb[5],
        F + 6,
        &format!("R101A: minority not on the heavier tip R6 within {BOUND} ticks"),
    )
    .await;
}

// R101C INC-I-293 — Decision: a failure means the rig cannot deliver R1 from P2 even without junk, so R101A's red would not isolate the junk starvation.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r101c_inc_i_293_control_no_junk_root_fetched_first_tick() {
    assert_above_gate();
    let run = scenario(0).await;
    assert_eq!(
        run.p2_r1_at,
        Some(0),
        "R101C: without junk P2 gets GetBlockByHash(R1) on the first tick"
    );
    assert!(run.r1_at.is_some(), "R101C: R1 reached the node");
    assert_on(
        &run.r,
        &run.rb[5],
        F + 6,
        &format!("R101C: control on R6 within {BOUND} ticks"),
    )
    .await;
}

const R101D: &str = "r101d_inc_i_293_below_gate_pin_one_shot_drains_junk_root_fetched_second_tick";

// R101D INC-I-293 — Decision: a failure means below-gate attestation fetch changed (one shot per entry, entry deleted, cap drains junk 8 per tick), so the operator must rule whether a fix for 101 leaks below the gate to mainnet.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r101d_inc_i_293_below_gate_pin_one_shot_drains_junk_root_fetched_second_tick() {
    if !in_below_ah_child(R101D) {
        return;
    }
    let run = scenario(JUNK).await;
    // Pinned at 776e37a0: tick 0 fires the 8 oldest junk entries and deletes
    // them; tick 1 fires the 9th junk and the root. Entries are not kept, so
    // the junk drains and cannot starve the root.
    let junk_asks: Vec<usize> = run.junk.iter().map(|h| count(&run.pj_asked, h)).collect();
    assert_eq!(
        junk_asks,
        vec![1; JUNK],
        "R101D pin: below the gate each junk hash is asked of P_JUNK exactly once"
    );
    assert!(
        (run.junk.iter()).all(|h| !run.r.node.attest_fetch_tracker.contains_key(h)),
        "R101D pin: below the gate fired junk entries are deleted"
    );
    assert_eq!(
        run.p2_r1_at,
        Some(1),
        "R101D pin: below the gate P2 gets GetBlockByHash(R1) on the second tick"
    );
    assert_eq!(
        count(&run.p2_asked, &run.rb[0].hash()),
        1,
        "R101D pin: below the gate the root is fetched once (one-shot)"
    );
}
