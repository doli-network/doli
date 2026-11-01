//! INC-I-293 S-2 (run 649), operator ruling 4 (AUDIT-P2-648-003 / REV-648-001):
//! above the INC-I-288 gate the attestation-named refetch takes matured tracker
//! entries oldest-first and gives each target peer at most
//! ATTEST_FETCH_PER_PEER_PER_TICK = 2 per tick (branch_support.rs:18-37
//! `round_robin`, periodic.rs:369-409). An entry that was asked and not answered
//! keeps its place, so two older unservable hashes that name the root's OWN
//! relayer take that relayer's two slots on every tick until the 30 s sweep, and
//! the fork root is never asked for. Honest relayers forward junk too.
//! Contract: an asked-but-unanswered entry rotates (or a per-relayer cap
//! applies), so the root R1 relayed by P2 is asked of P2 within a bound that
//! gives each older entry one turn, and the minority node ends on R6.
//! Rig: the R-2 root-priority rig (inc_i_293_r2_root_priority.rs): the node's own
//! loopback `NetworkService` and two more (P2 relayer, P3 bystander), tracker
//! entries inserted directly, per-peer GetBlockByHash logs, N ticks x 300 ms pump.
//!
// OUTPUT CONTRACT: fn Node::run_periodic_tasks() (INC-I-049 ATTEST_FETCH,
//   periodic.rs:355-414 + branch_support.rs:22-37), then
//   fn Node::handle_network_event(SyncResponse) for every reply, then
//   fn Node::handle_new_block (R2..R6 gossip once R1 is on the node)
//   O1: chain_state best_hash / best_height, canonical entry, persisted tip, sync tip
//   O2: the outbound GetBlockByHash(R1) at P2: first tick index and count
//   O3: the outbound GetBlockByHash(junk_i) at P2 (count per junk hash); at P3 (0)
//   O4: attest_fetch_tracker: junk / root entries kept or removed
// PATHS: A 2 junk entries relayed by P2, older than the root (red today);
//   B 6 junk entries relayed by P2 (red today); C no junk (control);
//   D path B below the INC-I-288 AH (pin)
// INPUT PARTITIONS: AH = 0 (devnet, above) | AH = 1e9 (below);
//   junk relayed by the root's relayer 0 | 2 (= per-peer cap) | 6 (3 x cap)
// MATRIX: S2R1 O1 O2 (O3 O4 logged) · S2R2 O1 O2 (O3 O4 logged)
//   · S2R3 O1 O2 · S2R4 O2 O3 O4 (O1 logged)
//
//! covers: periodic.rs (ATTEST_FETCH) branch_support.rs (round_robin)

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
const TICK_PUMP: Duration = Duration::from_millis(300);
/// ATTEST_FETCH_PER_PEER_PER_TICK (branch_support.rs:18).
const PER_PEER: usize = 2;
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
    p3_asked: Asked,
    /// Tick index after whose pump P2 had first logged GetBlockByHash(R1).
    p2_r1_at: Option<usize>,
    /// Tick index at which R1 was first on the node.
    r1_at: Option<usize>,
}

/// Fixed junk hashes: random-looking, in no chain, so P2 answers `Block(None)`.
fn junk_hashes(n: usize) -> Vec<Hash> {
    (0..n as u8)
        .map(|i| {
            let mut b = [0u8; 32];
            for (k, x) in b.iter_mut().enumerate() {
                *x = (k as u8).wrapping_mul(53) ^ 0xa7 ^ i.wrapping_mul(29);
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

/// `junk_n` unservable entries relayed by P2, all OLDER than the root R1's entry,
/// which P2 also relayed. P2 and P3 serve the whole heavier chain.
async fn scenario(junk_n: usize) -> Run {
    let mut r = rig_with_keys(keys(), &D_SLOTS).await;
    let rb = chain_on(&r, r.f_hash, F + 1, &R_SLOTS);
    let mut chain = Vec::new();
    for h in 1..=F {
        let hh = r.node.block_store.get_hash_by_height(h).unwrap().unwrap();
        chain.push(r.node.block_store.get_block(&hh).unwrap().unwrap());
    }
    chain.extend(rb.iter().cloned());

    let (pm, p2_port, p3_port) = (reserve_port(), reserve_port(), reserve_port());
    let (nid, gh) = (
        r.node.config.network.id(),
        r.node.chain_state.read().await.genesis_hash,
    );
    r.node.network = Some(start_net_as(nid, gh, pm).await);
    let p2 = start_net_as(nid, gh, p2_port).await;
    let p3 = start_net_as(nid, gh, p3_port).await;
    let (p2_id, p3_id) = (*p2.local_peer_id(), *p3.local_peer_id());
    let p2_asked = serve(p2, chain.clone());
    let p3_asked = serve(p3, chain);
    let net = r.node.network.as_ref().unwrap();
    net.connect(&addr(p2_port)).await.unwrap();
    net.connect(&addr(p3_port)).await.unwrap();
    let mut live = HashSet::new();
    for _ in 0..100 {
        if live.contains(&p2_id) && live.contains(&p3_id) {
            break;
        }
        pump(&mut r, Duration::from_millis(100), &mut live).await;
    }
    assert!(
        live.contains(&p2_id) && live.contains(&p3_id),
        "fixture: the node is connected to P2 and P3"
    );

    let top = rb.last().unwrap();
    for p in [p2_id, p3_id] {
        (r.node.sync_manager.write().await).add_peer(p, F + 6, top.hash(), top.header.slot);
    }
    // Junk entries first and older (each at the 3-attestation cap, relayer P2);
    // then the root entry, relayer P2 too. All past the 500 ms grace.
    let junk = junk_hashes(junk_n);
    let now = Instant::now();
    for (i, h) in junk.iter().enumerate() {
        let t = now - Duration::from_millis(1_000) + Duration::from_millis(i as u64);
        (r.node.attest_fetch_tracker).insert(*h, (t, 3, p2_id));
    }
    let root_t = now - Duration::from_millis(600);
    (r.node.attest_fetch_tracker).insert(rb[0].hash(), (root_t, 3, p2_id));

    let mut run = Run {
        r,
        rb,
        junk,
        p2_asked,
        p3_asked,
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
    let junk_asks: Vec<usize> = run.junk.iter().map(|h| count(&run.p2_asked, h)).collect();
    let junk_p3: usize = run.junk.iter().map(|h| count(&run.p3_asked, h)).sum();
    let junk_tracked = (run.junk.iter())
        .filter(|h| run.r.node.attest_fetch_tracker.contains_key(h))
        .count();
    eprintln!(
        "[S2] junk={junk_n} p2_r1_at={:?} p2_r1_asks={} r1_at={:?} ticks={ticks} \
         tip_is_r6={} junk_asks_at_p2={junk_asks:?} junk_asks_at_p3={junk_p3} \
         junk_tracked={junk_tracked} root_tracked={}",
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

/// Above the gate: P2 gets GetBlockByHash(R1) by tick index `last` (each older
/// junk entry gets one turn of P2's PER_PEER slots first), then the tip is R6.
async fn assert_root_rotated_in(label: &str, junk_n: usize, last: usize) {
    assert_above_gate();
    let run = scenario(junk_n).await;
    assert!(
        run.p2_r1_at.is_some_and(|i| i <= last),
        "{label}: P2 did not receive GetBlockByHash(R1) by tick index {last} \
         (first at tick {:?} of {BOUND}); {junk_n} older asked-and-unanswered \
         entries relayed by P2 kept their place and took P2's {PER_PEER} slots",
        run.p2_r1_at
    );
    assert_on(
        &run.r,
        &run.rb[5],
        F + 6,
        &format!("{label}: minority not on the heavier tip R6 within {BOUND} ticks"),
    )
    .await;
}

// S2R1 INC-I-293 S-2 — Decision: a failure means two unanswered entries naming the root's own relayer take its 2 per-peer slots every tick, so the fork root waits out the 30 s sweep and the refetch needs rotation or a per-relayer cap before the gate ships.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s2r1_inc_i_293_root_rotates_past_two_same_relayer_junk() {
    // Tick 0 may spend P2's 2 slots on the junk; tick 1 must reach R1.
    assert_root_rotated_in("S2R1", PER_PEER, 1).await;
}

// S2R2 INC-I-293 S-2 — Decision: a failure means a relayer's 6 older unanswered entries starve its root past one turn each (3 ticks), so the fix rotates only the first pair or caps too loosely to bound the root's wait.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s2r2_inc_i_293_root_rotates_past_six_same_relayer_junk() {
    // 6 junk / 2 per tick = 3 ticks of junk; tick index 3 must reach R1.
    assert_root_rotated_in("S2R2", 3 * PER_PEER, 3).await;
}

// S2R3 INC-I-293 S-2 — Decision: a failure means the rig cannot deliver R1 from P2 even without junk, so the S2R1/S2R2 reds would not isolate the kept queue place.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s2r3_inc_i_293_control_no_junk_root_fetched_first_tick() {
    assert_above_gate();
    let run = scenario(0).await;
    assert_eq!(
        run.p2_r1_at,
        Some(0),
        "S2R3: without junk P2 gets GetBlockByHash(R1) on the first tick"
    );
    assert!(run.r1_at.is_some(), "S2R3: R1 reached the node");
    assert_on(
        &run.r,
        &run.rb[5],
        F + 6,
        &format!("S2R3: control on R6 within {BOUND} ticks"),
    )
    .await;
}

const S2R4: &str = "s2r4_inc_i_293_below_gate_pin_one_shot_drains_same_relayer_junk";

// S2R4 INC-I-293 S-2 — Decision: a failure means below-gate attestation fetch changed (one shot per entry, entry deleted, 8 per tick with no per-peer cap), so a rotation fix leaks below the gate to mainnet and the operator must rule on it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s2r4_inc_i_293_below_gate_pin_one_shot_drains_same_relayer_junk() {
    if !in_below_ah_child(S2R4) {
        return;
    }
    let junk_n = 3 * PER_PEER;
    let run = scenario(junk_n).await;
    // Pinned at c5e398fc: below the gate there is no per-peer cap; tick 0 fires
    // all 7 matured entries (< cap 8) once each and deletes them, so P2's
    // unanswered junk never holds the root's place.
    let junk_asks: Vec<usize> = run.junk.iter().map(|h| count(&run.p2_asked, h)).collect();
    assert_eq!(
        junk_asks,
        vec![1; junk_n],
        "S2R4 pin: below the gate each junk hash is asked of P2 exactly once"
    );
    assert!(
        (run.junk.iter()).all(|h| !run.r.node.attest_fetch_tracker.contains_key(h)),
        "S2R4 pin: below the gate fired junk entries are deleted"
    );
    assert_eq!(
        run.p2_r1_at,
        Some(0),
        "S2R4 pin: below the gate P2 gets GetBlockByHash(R1) on the first tick"
    );
    assert_eq!(
        count(&run.p2_asked, &run.rb[0].hash()),
        1,
        "S2R4 pin: below the gate the root is fetched once (one-shot)"
    );
}
