//! INC-I-293 U-2 (run 651), SEC-FIXVERIFY-647-104, operator ruling 5: the
//! attestation refetch rotates away from a relayer that answered `None` for the
//! hash. Live (run 650, trials 4a/4b/4c): after a restart a by-hash walk asked a
//! peer that answered None (`cancelled: peer does not have block`), then the
//! `[ATTEST_FETCH]` refetch asked THAT peer for the same hash once per second for
//! ~30 s, until the tracker's 30 s sweep.
//! Today (periodic.rs:369-413, branch_support.rs:204-209) above the INC-I-288 gate
//! an entry lives until the block is held or the 30 s sweep, and each tick asks
//! `attest_fetch_peer(recorded)` = the recorded relayer while it is in the peer
//! table. A `Block(None)` answer is not visible to the tracker:
//! `NetworkEvent::SyncResponse { peer_id, response }` carries no request, the None
//! carries no hash, and outside a walk `handle_response` drops it
//! (sync_engine/response.rs:87-102). The cells assert the observable the fix must
//! wire: after P answers None for H, P is never asked for H again; the entry moves
//! to a peer reporting the same off-chain tip (`branch_peer`, as
//! `hand_off_attest_fetch` does on disconnect), else it is retired.
//! Rig: the S-2 fetch-rotation rig (inc_i_293_s2_fetch_rotation.rs): the node's
//! own loopback `NetworkService`, mock peers that log every GetBlockByHash, tracker
//! entries inserted directly, N ticks x 300 ms pump.
//!
// OUTPUT CONTRACT: fn Node::run_periodic_tasks() (INC-I-049 ATTEST_FETCH,
//   periodic.rs:355-420 + branch_support.rs:204-264), then
//   fn Node::handle_network_event(SyncResponse) for every reply
//   O1: outbound GetBlockByHash(H) per target peer per tick (mock logs)
//   O2: the Block(None) replies the node processed (peer, tick)
//   O3: attest_fetch_tracker entry for H: kept / moved / removed
//   O4: H on the node (stored or cached) once an heir serves it
// PATHS: A P answers None for R1, no heir (red today); B P answers None for R1,
//   P2 reports the same off-chain tip and serves R1 (red today); C P silent for R1
//   and J1, None for J0 asked in the same tick as R1 (control); D path A below the
//   INC-I-288 AH, entry
//   re-created after the one-shot (pin)
// INPUT PARTITIONS: AH = 0 (devnet, above) | AH = 1e9 (below); relayer answer
//   None | silent; heir present | absent; entries relayed by P 1 | 3 (> per-peer cap)
// MATRIX: U2A O1 O2 O3 · U2B O1 O2 O4 · U2C O1 O3 · U2D O1 O2 O3
//
//! covers: periodic.rs (ATTEST_FETCH) branch_support.rs (attest_fetch_peer,
//! branch_peer, round_robin) event_loop.rs (on_sync_response)

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

/// Ticks per run (each followed by a TICK_PUMP network pump, ~3 s in all, far
/// inside the tracker's 30 s TTL and the 120 s sync request timeout).
const TICKS: usize = 10;
const TICK_PUMP: Duration = Duration::from_millis(300);
/// ATTEST_FETCH_PER_PEER_PER_TICK (branch_support.rs:18).
const PER_PEER: usize = 2;
const D_SLOTS: [u32; 4] = [210, 211, 213, 214];
const R_SLOTS: [u32; 6] = [215, 216, 217, 218, 219, 220];

type Asked = Arc<Mutex<Vec<Hash>>>;

/// How a mock peer answers GetBlockByHash for a hash it lists as odd.
#[derive(Clone, Copy, PartialEq)]
enum Odd {
    /// `Block(None)`: "peer does not have block".
    None,
    /// No answer: the request stays open (the 120 s timeout is never reached).
    Silent,
}

struct Peer {
    id: PeerId,
    asked: Asked,
    /// Log length after each tick's pump: tick i's asks are `log[marks[i-1]..marks[i]]`.
    marks: Vec<usize>,
}

impl Peer {
    fn asks_in_tick(&self, i: usize, h: &Hash) -> usize {
        let log = self.asked.lock().unwrap();
        let from = if i == 0 { 0 } else { self.marks[i - 1] };
        log[from..self.marks[i]].iter().filter(|x| *x == h).count()
    }

    fn per_tick(&self, h: &Hash) -> Vec<usize> {
        (0..self.marks.len())
            .map(|i| self.asks_in_tick(i, h))
            .collect()
    }

    fn total_in_tick(&self, i: usize, hs: &[Hash]) -> usize {
        hs.iter().map(|h| self.asks_in_tick(i, h)).sum()
    }
}

struct Run {
    r: Rig,
    rb: Vec<Block>,
    p: Peer,
    p2: Option<Peer>,
    live: HashSet<PeerId>,
    /// (tick, peer) of every `Block(None)` reply the node processed.
    nones: Vec<(usize, PeerId)>,
    ticks_run: usize,
}

fn junk_hashes(n: usize) -> Vec<Hash> {
    (0..n as u8)
        .map(|i| {
            let mut b = [0u8; 32];
            for (k, x) in b.iter_mut().enumerate() {
                *x = (k as u8).wrapping_mul(41) ^ 0x3c ^ i.wrapping_mul(17);
            }
            Hash::from_bytes(b)
        })
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

fn keys() -> Vec<KeyPair> {
    (1..=3u8)
        .map(|i| KeyPair::from_seed([0x90 + i; 32]))
        .collect()
}

/// A mock peer serving `chain`; GetBlockByHash for a hash listed in `odd` is
/// answered as listed. Logs every GetBlockByHash hash.
fn serve(mut svc: NetworkService, chain: Vec<Block>, odd: Vec<(Hash, Odd)>) -> Asked {
    let asked: Asked = Arc::default();
    let log = asked.clone();
    tokio::spawn(async move {
        let mut open = Vec::new();
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
                    match odd.iter().find(|(h, _)| *h == hash).map(|(_, how)| *how) {
                        Some(Odd::Silent) => {
                            open.push(channel);
                            continue;
                        }
                        Some(Odd::None) => SyncResponse::Block(None),
                        None => {
                            SyncResponse::Block(chain.iter().find(|b| b.hash() == hash).cloned())
                        }
                    }
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

/// Feeds the node its own network events (replies and disconnects through the
/// production dispatch); records every `Block(None)` reply as (tick, peer).
async fn pump(run: &mut Run, dur: Duration, tick_ix: usize) {
    let until = Instant::now() + dur;
    while Instant::now() < until {
        let net = run.r.node.network.as_mut().expect("node network attached");
        let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(25), net.next_event()).await
        else {
            continue;
        };
        match &ev {
            NetworkEvent::PeerConnected(p) => {
                run.live.insert(*p);
                continue;
            }
            NetworkEvent::PeerDisconnected(p) => {
                run.live.remove(p);
            }
            NetworkEvent::SyncResponse {
                peer_id,
                response: SyncResponse::Block(None),
                ..
            } => run.nones.push((tick_ix, *peer_id)),
            NetworkEvent::SyncResponse { .. } => {}
            _ => continue,
        }
        let _ = run.r.node.handle_network_event(ev).await;
    }
}

/// P relays R1 (answered as `r1`) and `junk` (each answered as listed). With
/// `heir`, P2 reports the same off-chain tip R6 and serves the whole chain.
async fn setup(r1: Odd, junk: &[(Hash, Odd)], heir: bool) -> Run {
    let mut r = rig_with_keys(keys(), &D_SLOTS).await;
    let rb = chain_on(&r, r.f_hash, F + 1, &R_SLOTS);
    let mut chain = Vec::new();
    for h in 1..=F {
        let hh = r.node.block_store.get_hash_by_height(h).unwrap().unwrap();
        chain.push(r.node.block_store.get_block(&hh).unwrap().unwrap());
    }
    chain.extend(rb.iter().cloned());
    let mut odd = vec![(rb[0].hash(), r1)];
    odd.extend_from_slice(junk);

    let (pm, p_port, p2_port) = (reserve_port(), reserve_port(), reserve_port());
    let (nid, gh) = (
        r.node.config.network.id(),
        r.node.chain_state.read().await.genesis_hash,
    );
    r.node.network = Some(start_net_as(nid, gh, pm).await);
    let p_svc = start_net_as(nid, gh, p_port).await;
    let p_id = *p_svc.local_peer_id();
    let p_asked = serve(p_svc, chain.clone(), odd);
    let p2 = if heir {
        let svc = start_net_as(nid, gh, p2_port).await;
        let id = *svc.local_peer_id();
        Some(Peer {
            id,
            asked: serve(svc, chain, Vec::new()),
            marks: Vec::new(),
        })
    } else {
        None
    };
    let mut run = Run {
        r,
        rb,
        p: Peer {
            id: p_id,
            asked: p_asked,
            marks: Vec::new(),
        },
        p2,
        live: HashSet::new(),
        nones: Vec::new(),
        ticks_run: 0,
    };
    let net = run.r.node.network.as_ref().unwrap();
    net.connect(&addr(p_port)).await.unwrap();
    if heir {
        net.connect(&addr(p2_port)).await.unwrap();
    }
    let mut ids = vec![p_id];
    ids.extend(run.p2.as_ref().map(|p| p.id));
    for _ in 0..100 {
        if ids.iter().all(|id| run.live.contains(id)) {
            break;
        }
        pump(&mut run, Duration::from_millis(100), 0).await;
    }
    assert!(
        ids.iter().all(|id| run.live.contains(id)),
        "fixture: the node is connected to every mock peer"
    );
    run.nones.clear();
    let top = run.rb.last().unwrap().clone();
    for id in &ids {
        (run.r.node.sync_manager.write().await).add_peer(*id, F + 6, top.hash(), top.header.slot);
    }
    // R1 first (oldest), then the junk; all relayed by P, past the 500 ms grace.
    let now = Instant::now();
    let mut entries = vec![run.rb[0].hash()];
    entries.extend(junk.iter().map(|(h, _)| *h));
    for (i, h) in entries.iter().enumerate() {
        let t = now - Duration::from_millis(900) + Duration::from_millis(i as u64);
        (run.r.node.attest_fetch_tracker).insert(*h, (t, 3, p_id));
    }
    run
}

async fn ticks(run: &mut Run, n: usize) {
    for _ in 0..n {
        let i = run.ticks_run;
        tick(&mut run.r).await;
        pump(run, TICK_PUMP, i).await;
        run.p.marks.push(run.p.asked.lock().unwrap().len());
        if let Some(p2) = run.p2.as_mut() {
            p2.marks.push(p2.asked.lock().unwrap().len());
        }
        run.ticks_run += 1;
    }
}

/// First tick at which the node processed a `Block(None)` from P.
fn p_none_at(run: &Run) -> usize {
    let at = run
        .nones
        .iter()
        .find(|(_, p)| *p == run.p.id)
        .map(|(t, _)| *t);
    at.unwrap_or_else(|| {
        panic!(
            "fixture: P's Block(None) for R1 never reached the node (P asks per tick {:?}, \
             nones {:?})",
            run.p.per_tick(&run.rb[0].hash()),
            run.nones
        )
    })
}

fn assert_above_gate() {
    let ah = NetworkParams::load(Network::Devnet).inc_i_288_fork_choice_activation_height;
    assert!(
        ah <= F,
        "precondition: devnet AH {ah} is at or below the fork point"
    );
}

fn asked_again_after(p: &Peer, h: &Hash, none_at: usize) -> Vec<usize> {
    ((none_at + 1)..p.marks.len())
        .filter(|i| p.asks_in_tick(*i, h) > 0)
        .collect()
}

// U2A INC-I-293 SEC-FIXVERIFY-647-104 (Must) — Decision: a failure means a relayer that said it does not have the block is still asked for it every tick until the 30 s sweep (the run-650 trials 4a-4c log), so ruling 5 is not met and the refetch spends its per-peer slots on a known-empty peer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn u2a_none_answer_retires_the_relayer_for_that_hash() {
    assert_above_gate();
    let mut run = setup(Odd::None, &[], false).await;
    ticks(&mut run, TICKS).await;
    let r1 = run.rb[0].hash();
    let none_at = p_none_at(&run);
    let p_asks = run.p.per_tick(&r1);
    eprintln!(
        "[U2A] none_at={none_at} p_asks_r1={p_asks:?} nones={:?}",
        run.nones
    );
    assert_eq!(
        p_asks[none_at], 1,
        "fixture: P was asked R1 once at tick {none_at}"
    );
    let again = asked_again_after(&run.p, &r1, none_at);
    assert!(
        again.is_empty(),
        "U2A: peer P {} was asked GetBlockByHash(R1) again at tick {} after it answered \
         None at tick {none_at} (asks of R1 at P per tick: {p_asks:?}); a relayer that \
         answered None for a hash must not be asked for it again",
        run.p.id,
        again.first().copied().unwrap_or_default()
    );
    let entry = run.r.node.attest_fetch_tracker.get(&r1).map(|e| e.2);
    assert!(
        entry != Some(run.p.id),
        "O3: the R1 entry still names P after its None (no heir: retire it)"
    );
}

// U2B INC-I-293 SEC-FIXVERIFY-647-104 (Must) — Decision: a failure means a None answer only silences P and the hash is never asked of the peer that reported the same off-chain tip, so the minority node waits out the sweep instead of fetching the block from a peer that has it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn u2b_none_answer_moves_the_hash_to_an_inheriting_peer() {
    assert_above_gate();
    let mut run = setup(Odd::None, &[], true).await;
    ticks(&mut run, TICKS).await;
    let r1 = run.rb[0].hash();
    let none_at = p_none_at(&run);
    let p_asks = run.p.per_tick(&r1);
    let p2 = run.p2.as_ref().expect("heir");
    let p2_asks = p2.per_tick(&r1);
    let p2_first = p2_asks.iter().position(|n| *n > 0);
    eprintln!(
        "[U2B] none_at={none_at} p_asks_r1={p_asks:?} p2_asks_r1={p2_asks:?} nones={:?}",
        run.nones
    );
    assert!(
        p2_first.is_some_and(|i| i == none_at + 1),
        "U2B: after P answered None for R1 at tick {none_at}, the next fetch of R1 did not \
         go to P2 {} (same off-chain tip): P2 first asked at tick {p2_first:?} of {TICKS}; \
         asks of R1 at P per tick: {p_asks:?}",
        p2.id
    );
    let again = asked_again_after(&run.p, &r1, none_at);
    assert!(
        again.is_empty(),
        "U2B: P was asked R1 again at ticks {again:?} after its None"
    );
    assert!(
        stored(&run.r, &run.rb[0]) || cached(&run.r, &r1).await,
        "O4: R1 served by P2 reached the node"
    );
}

// U2C INC-I-293 SEC-FIXVERIFY-647-104 (Must, control) — Decision: a failure means the rotation also drops hashes P never answered None for (a slow or silent relayer, or a None for ANOTHER hash in the same tick), or breaks the 2-per-peer bound, so honest slow relayers lose their entries.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn u2c_relayer_without_none_keeps_being_asked() {
    assert_above_gate();
    // J0 is asked of P in the same tick as R1 (oldest first) and answered None;
    // R1 and J1 are never answered. Only J0 may leave P.
    let junk = junk_hashes(2);
    let odd = [(junk[0], Odd::None), (junk[1], Odd::Silent)];
    let mut run = setup(Odd::Silent, &odd, false).await;
    ticks(&mut run, TICKS).await;
    let all = [run.rb[0].hash(), junk[0], junk[1]];
    let silent = [run.rb[0].hash(), junk[1]];
    let per_hash: Vec<Vec<usize>> = all.iter().map(|h| run.p.per_tick(h)).collect();
    eprintln!(
        "[U2C] asks of R1, J0, J1 at P per tick {per_hash:?} nones={:?}",
        run.nones
    );
    assert_eq!(
        per_hash[0][0] + per_hash[1][0],
        2,
        "fixture: R1 and J0 are asked of P on the first tick"
    );
    assert!(
        run.nones.iter().any(|(_, p)| *p == run.p.id),
        "fixture: P's None for J0 reached the node"
    );
    for i in 0..TICKS {
        let n = run.p.total_in_tick(i, &all);
        assert!(
            n <= PER_PEER,
            "U2C: P got {n} refetches at tick {i}, over the {PER_PEER}-per-peer cap"
        );
    }
    for h in &silent {
        let asks = run.p.per_tick(h);
        let ticks_asked = asks.iter().filter(|n| **n > 0).count();
        assert!(
            ticks_asked >= 2,
            "U2C: relayer P was asked {h:.8} on only {ticks_asked} of {TICKS} ticks \
             ({asks:?}); P never answered None for it, so it keeps being asked"
        );
        assert_eq!(
            run.r.node.attest_fetch_tracker.get(h).map(|e| e.2),
            Some(run.p.id),
            "O3: entry {h:.8} kept, relayer P"
        );
    }
}

const U2D: &str = "u2d_below_gate_noop_pinned";

// U2D INC-I-293 SEC-FIXVERIFY-647-104 (Must, below-gate pin) — Decision: a failure means below-gate attestation fetch changed (one shot per entry, entry deleted, a re-created entry asked of its relayer again even after a None), so the None-rotation leaks below the INC-I-288 gate to mainnet.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn u2d_below_gate_noop_pinned() {
    if !in_below_ah_child(U2D) {
        return;
    }
    let mut run = setup(Odd::None, &[], false).await;
    ticks(&mut run, 3).await;
    let r1 = run.rb[0].hash();
    let none_at = p_none_at(&run);
    assert_eq!(
        run.p.per_tick(&r1),
        vec![1, 0, 0],
        "U2D pin: below the gate R1 is asked of P once (one-shot) and never refetched"
    );
    assert_eq!(none_at, 0, "U2D pin: P's None arrives on the first tick");
    assert!(
        !run.r.node.attest_fetch_tracker.contains_key(&r1),
        "U2D pin: below the gate the fired entry is deleted"
    );
    assert!(
        run.r.node.attest_fetch_asked.is_empty(),
        "U2D pin: below the gate no refetch queue state is kept"
    );
    // A later attestation re-creates the entry, relayer P (periodic.rs:394-398).
    let t = Instant::now() - Duration::from_millis(900);
    (run.r.node.attest_fetch_tracker).insert(r1, (t, 3, run.p.id));
    ticks(&mut run, 2).await;
    assert_eq!(
        run.p.per_tick(&r1),
        vec![1, 0, 0, 1, 0],
        "U2D pin: below the gate a re-created entry is asked of P again despite its None"
    );
}
