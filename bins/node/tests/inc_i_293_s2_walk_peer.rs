//! INC-I-293 run 649 S-2 (operator ruling 2: AUDIT-P2-648-001 + the REV-647-001
//! carried callers). Above the gate every by-hash walk asks an evidence peer (a
//! peer that showed the branch: the orphan's sender, or a peer whose reported tip
//! is off our chain), never the tallest peer; a walk peer that answers `None` is
//! marked and the walk moves to the next evidence peer in the same attempt.
//! Today the Wedged caller (`periodic.rs:847`) calls `try_trigger_fork_recovery()`
//! -> `best_peer_for_recovery` (tallest), and `Block(None)` cancels the walk with
//! a 30 s cooldown (`fork_recovery.rs:213-216`).
//!
//! The Wedged terminal needs >= 300 s without an apply (`recovery.rs:402`,
//! `last_block_applied` is a private `Instant` with no test setter), so the Wedged
//! cells call `Node::try_trigger_fork_recovery()`, the entry `periodic.rs:847`
//! calls, as inc_i_235 does. The ReorgCandidate fallback (`block_handling.rs:360`)
//! cannot start a walk from gossip: it runs only after `start_fork_recovery(block,
//! source_peer)` (line 333) returned false while `can_start_fork_recovery()` (line
//! 327) was true, and `ForkRecoveryTracker::start` then fails only on a genesis
//! mismatch that `classify_gossip_block` (line 60) already rejects. It shares the
//! Wedged entry, so s2a1 covers both carried callers.
//!
// OUTPUT CONTRACT: fn Node::try_trigger_fork_recovery() (Wedged / ReorgCandidate
//   fallback entry), fn Node::handle_new_block(orphan, sender) (orphan caller),
//   fn Node::on_sync_response(peer, Block(None|Some)) (walk intake)
//   O1: chain_state best_hash / best_height, canonical entry, persisted tip, sync tip
//   O2: every outbound GetBlockByHash of the walk: target peer, hash, slot, answer
//   O3: the walk after the trigger: started (bool), is_fork_recovery_active
//   O4: a None-answering walk peer: asked again after its None (must not be)
// PATHS: W (Wedged entry, above AH) P_heavy (F+3, R3, serves R) + P_tall (F+5, D5,
//   None) -> s2a1 red; W' = W with P_heavy at F+6 -> c2 control; O (orphan from
//   P_heavy, above AH) -> c1 control; E (orphan from P_e1 (F+3, R3, None), P_e2
//   (F+3, R3, serves R), P_tall) -> s2a3 red; O below AH -> b1 pin; W below AH -> b2 pin
// INPUT PARTITIONS: AH = 0 (devnet, above) | AH = 1e9 (below); evidence peer tallest |
//   not; evidence peer serves | answers None; D 2 producers, R 3; fixed-seed keys
// MATRIX: W x O1 O2 O3 -> s2a1 · W' x O1 O2 -> c2 · O x O1 O2 -> c1
//   · E x O1 O2 O4 -> s2a3 · O(below) x O1 O2 O3 -> b1 · W(below) x O2 O3 -> b2
//! covers: fork_walk (try_trigger_fork_recovery) periodic (Wedged arm)
//! block_handling network_events fork_recovery sync/manager (best_peer_for_recovery)

#[path = "support/block_sig.rs"]
mod block_sig;
#[path = "it/inc_i_289_common.rs"]
mod inc_i_289_common;

use std::collections::HashSet;

use crypto::{Hash, KeyPair};
use doli_core::validation::ValidationMode;
use doli_core::{Block, Network, NetworkParams};
use inc_i_289_common::*;
use network::protocols::{SyncRequest, SyncResponse};
use network::PeerId;

/// The bound, in slots from the trigger.
const N: usize = 3;

fn short(h: &Hash) -> String {
    h.to_string()[..8].to_string()
}

fn assert_above_gate() {
    let ah = NetworkParams::load(Network::Devnet).inc_i_288_fork_choice_activation_height;
    assert!(ah <= F, "precondition: devnet AH {ah} is at or below F");
}

#[derive(Clone, Copy, PartialEq)]
enum Tip {
    /// Our own top D5 (canonical).
    Ours,
    /// R3, off our chain.
    R3,
}

struct PeerSpec {
    name: &'static str,
    height: u64,
    tip: Tip,
    serves: bool,
}

const fn spec(name: &'static str, height: u64, tip: Tip, serves: bool) -> PeerSpec {
    PeerSpec {
        name,
        height,
        tip,
        serves,
    }
}

enum Trigger {
    /// R3 gossiped by the named peer (the orphan caller).
    Orphan(&'static str),
    /// R3 already cached; the Wedged arm's entry `try_trigger_fork_recovery()`.
    Wedged,
}

struct Walk {
    r: Rig,
    /// R1..R3 at F+1..F+3.
    rb: Vec<Block>,
    peers: Vec<(PeerId, &'static str, bool)>,
    last_slot: u32,
}

struct Fetch {
    slot: usize,
    peer: &'static str,
    hash: Hash,
    served: bool,
}

#[derive(Default)]
struct WalkObs {
    fetched: Vec<Fetch>,
    started: Option<bool>,
    active_after_trigger: bool,
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
/// (one per leader); `peers` registered with their reported (height, tip).
async fn arrange(peers: &[PeerSpec]) -> Walk {
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
    let mut ids = Vec::new();
    for p in peers {
        let (h, s) = match p.tip {
            Tip::Ours => (top.hash(), top.header.slot),
            Tip::R3 => (rb[2].hash(), rb[2].header.slot),
        };
        ids.push((peer_at(&r, p.height, h, s).await, p.name, p.serves));
    }
    Walk {
        r,
        rb,
        peers: ids,
        last_slot: d5 + 3,
    }
}

fn id(w: &Walk, name: &str) -> PeerId {
    (w.peers.iter())
        .find(|p| p.1 == name)
        .expect("fixture peer")
        .0
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

/// Each peer answers the by-hash fetches the node hands it through the production
/// intake (`on_sync_response`): a serving peer from R1..R3, every other peer None.
async fn serve_peers(w: &mut Walk, slot: usize, obs: &mut WalkObs) {
    for _ in 0..8 {
        let req = w.r.node.sync_manager.write().await.next_request();
        let Some((q, req)) = req else { break };
        let SyncRequest::GetBlockByHash { hash } = req else {
            continue;
        };
        let (name, serves) = (w.peers.iter())
            .find(|p| p.0 == q)
            .map_or(("other", false), |p| (p.1, p.2));
        let blk = (serves)
            .then(|| w.rb.iter().find(|b| b.hash() == hash).cloned())
            .flatten();
        obs.fetched.push(Fetch {
            slot,
            peer: name,
            hash,
            served: blk.is_some(),
        });
        (w.r.node.on_sync_response(q, SyncResponse::Block(blk)).await)
            .expect("on_sync_response returns Ok");
        let parent = (w.r.node.sync_manager.read().await).fork_recovery_current_parent();
        let canonical = parent
            .is_some_and(|p| matches!(w.r.node.block_store.get_height_by_hash(&p), Ok(Some(_))));
        if canonical {
            let done = (w.r.node.sync_manager.write().await).check_fork_recovery_connection(true);
            if let Some(done) = done {
                let res = w.r.node.handle_completed_fork_recovery(done).await;
                eprintln!("[S2] completed walk -> {res:?}");
            }
        }
    }
}

async fn on_r(w: &Walk) -> bool {
    w.r.node.block_store.get_hash_by_height(F + 1).unwrap() == Some(w.rb[0].hash())
}

/// Slot 1: the trigger. Each slot: one own block while not on R, the peers
/// answer, one periodic tick, the peers answer again.
async fn walk_rounds(w: &mut Walk, trigger: Trigger) -> WalkObs {
    let mut obs = WalkObs::default();
    for k in 1..=N {
        if k == 1 {
            let r3 = w.rb[2].clone();
            match trigger {
                Trigger::Orphan(from) => {
                    let p = id(w, from);
                    (w.r.node.handle_new_block(r3, p).await).expect("handle_new_block returns Ok");
                }
                Trigger::Wedged => {
                    (w.r.node.fork_block_cache.write().await).insert(r3.hash(), r3);
                    obs.started = Some(w.r.node.try_trigger_fork_recovery().await);
                }
            }
            obs.active_after_trigger =
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

fn describe(w: &Walk, obs: &WalkObs) -> String {
    let f: Vec<String> = (obs.fetched.iter())
        .map(|x| {
            let a = if x.served { "block" } else { "None" };
            format!("slot {}: {} <- {} ({a})", x.slot, x.peer, short(&x.hash))
        })
        .collect();
    format!(
        "walk started {:?}, active after trigger {}; by-hash fetches: [{}]; R1 {} R2 {}; \
         own blocks {}",
        obs.started,
        obs.active_after_trigger,
        f.join(", "),
        short(&w.rb[0].hash()),
        short(&w.rb[1].hash()),
        obs.own_blocks
    )
}

fn r_roots(w: &Walk) -> [Hash; 2] {
    [w.rb[0].hash(), w.rb[1].hash()]
}

fn asked(w: &Walk, obs: &WalkObs, peer: &str) -> bool {
    let roots = r_roots(w);
    (obs.fetched.iter()).any(|x| x.peer == peer && roots.contains(&x.hash))
}

fn first_root_fetch<'a>(w: &Walk, obs: &'a WalkObs) -> Option<&'a Fetch> {
    let roots = r_roots(w);
    obs.fetched.iter().find(|x| roots.contains(&x.hash))
}

async fn tip(w: &Walk) -> (Hash, u64) {
    let cs = w.r.node.chain_state.read().await;
    (cs.best_hash, cs.best_height)
}

/// The walk's first root fetch goes to `evidence`, `P_tall` is never asked, and the
/// node is on R3 within N slots.
async fn assert_walk_heals_from(w: &Walk, obs: &WalkObs, evidence: &str, label: &str) {
    let first = first_root_fetch(w, obs).map(|x| x.peer);
    let (t, h) = tip(w).await;
    assert!(
        first == Some(evidence) && !asked(w, obs, "P_tall") && t == w.rb[2].hash(),
        "{label}: within N={N} slots the walk's first by-hash fetch of R2/R1 did not go to \
         the evidence peer {evidence} (first: {first:?}), or the tallest peer P_tall (our \
         own tip) was asked, or the tip is not R3: tip h={h} {}, want R3 h={} {}; {}",
        short(&t),
        F + 3,
        short(&w.rb[2].hash()),
        describe(w, obs)
    );
    assert_on(&w.r, &w.rb[2], F + 3, label).await;
}

const HEAVY_SHORT: PeerSpec = spec("P_heavy", F + 3, Tip::R3, true);
const HEAVY_TALLEST: PeerSpec = spec("P_heavy", F + 6, Tip::R3, true);
const TALL: PeerSpec = spec("P_tall", F + 5, Tip::Ours, false);

// ------------------------------------------------------------------ reds (Must)

// S-2 INC-I-293 run 649 (Must) — Decision: a failure means the Wedged terminal's walk (and the ReorgCandidate fallback, same entry) still asks the tallest peer, which sits on our own lighter branch and answers None, so a minority whose heavier branch is shorter never fetches its root from the peer that showed it (REV-647-001 carried callers).
#[tokio::test]
async fn s2a1_wedged_walk_asks_an_evidence_peer_not_the_tallest() {
    assert_above_gate();
    let mut w = arrange(&[HEAVY_SHORT, TALL]).await;
    let obs = walk_rounds(&mut w, Trigger::Wedged).await;
    assert_eq!(
        obs.started,
        Some(true),
        "s2a1 precondition: the Wedged entry starts a walk from the cached R3: {}",
        describe(&w, &obs)
    );
    assert_walk_heals_from(&w, &obs, "P_heavy", "s2a1 (Must)").await;
}

// S-2 INC-I-293 run 649 AUDIT-P2-648-001 (Must) — Decision: a failure means a walk peer that answers None still cancels the walk and arms the 30 s cooldown, so one evidence peer without the parent holds the walk slot while another evidence peer that serves the branch is never asked.
#[tokio::test]
async fn s2a3_none_answer_moves_the_walk_to_the_next_evidence_peer() {
    assert_above_gate();
    let e1 = spec("P_e1", F + 3, Tip::R3, false);
    let e2 = spec("P_e2", F + 3, Tip::R3, true);
    let mut w = arrange(&[e1, e2, TALL]).await;
    let obs = walk_rounds(&mut w, Trigger::Orphan("P_e1")).await;
    let none_at = (obs.fetched.iter()).position(|x| x.peer == "P_e1" && !x.served);
    assert!(
        none_at.is_some(),
        "s2a3 precondition: the orphan walk asks its sender P_e1 first and P_e1 answers None: {}",
        describe(&w, &obs)
    );
    let none_at = none_at.unwrap();
    let none_slot = obs.fetched[none_at].slot;
    let e2_slot = (obs.fetched.iter().skip(none_at))
        .find(|x| x.peer == "P_e2")
        .map(|x| x.slot);
    let (t, h) = tip(&w).await;
    assert!(
        e2_slot == Some(none_slot) && !asked(&w, &obs, "P_tall") && t == w.rb[2].hash(),
        "s2a3 (Must): after P_e1 answered None (slot {none_slot}) the walk did not move to \
         the next evidence peer P_e2 in the same attempt (P_e2 first asked in slot \
         {e2_slot:?}), or asked the tallest peer P_tall, or the tip is not R3 within N={N} \
         slots: tip h={h} {}, want R3 h={} {}; {}",
        short(&t),
        F + 3,
        short(&w.rb[2].hash()),
        describe(&w, &obs)
    );
    let again = obs
        .fetched
        .iter()
        .skip(none_at + 1)
        .any(|x| x.peer == "P_e1");
    assert!(
        !again,
        "s2a3 (Must): P_e1 answered None but was asked again by the walk (not marked): {}",
        describe(&w, &obs)
    );
    assert_on(&w.r, &w.rb[2], F + 3, "s2a3 (Must)").await;
}

// ------------------------------------------------------------- controls (pass today)

// S-2 INC-I-293 run 649 control — Decision: a failure means this rig (production intake through on_sync_response) cannot heal even through the orphan caller that already asks its serving sender, so the s2a reds would not be about the walk peer.
#[tokio::test]
async fn c1_control_orphan_walk_from_serving_sender_heals() {
    assert_above_gate();
    let mut w = arrange(&[HEAVY_SHORT, TALL]).await;
    let obs = walk_rounds(&mut w, Trigger::Orphan("P_heavy")).await;
    assert_walk_heals_from(&w, &obs, "P_heavy", "c1 control").await;
}

// S-2 INC-I-293 run 649 control — Decision: a failure means the Wedged entry, the walk or bond fork choice cannot move the node onto R3 even when the tallest peer holds R, so s2a1's red would not isolate the peer choice.
#[tokio::test]
async fn c2_control_wedged_walk_heals_when_tallest_is_the_evidence_peer() {
    assert_above_gate();
    let mut w = arrange(&[HEAVY_TALLEST, TALL]).await;
    let obs = walk_rounds(&mut w, Trigger::Wedged).await;
    assert_eq!(obs.started, Some(true), "c2: {}", describe(&w, &obs));
    assert_walk_heals_from(&w, &obs, "P_heavy", "c2 control").await;
}

// ----------------------------------------------------------- below-gate pins (INV-12)

// S-2 INC-I-293 run 649 below-gate pin — Decision: a failure means today's below-AH (mainnet) orphan path changed: below the gate an orphan starts no by-hash walk and asks no peer, and a gated fix must keep that bit-identical.
#[tokio::test]
async fn b1_below_gate_orphan_starts_no_walk() {
    if !in_below_ah_child("b1_below_gate_orphan_starts_no_walk") {
        return;
    }
    let mut w = arrange(&[HEAVY_SHORT, TALL]).await;
    let obs = walk_rounds(&mut w, Trigger::Orphan("P_heavy")).await;
    assert!(
        !obs.active_after_trigger && obs.fetched.is_empty(),
        "b1 pin: below the gate an orphan starts a by-hash walk now: {}",
        describe(&w, &obs)
    );
    assert_ne!(tip(&w).await.0, w.rb[2].hash(), "b1 pin: below gate on R3");
}

// S-2 INC-I-293 run 649 below-gate pin — Decision: a failure means today's below-AH (mainnet) Wedged walk changed: below the gate it asks the tallest peer (best_peer_for_recovery), P_tall answers None and the walk ends; a gated fix must keep that bit-identical.
#[tokio::test]
async fn b2_below_gate_wedged_walk_asks_the_tallest_peer() {
    if !in_below_ah_child("b2_below_gate_wedged_walk_asks_the_tallest_peer") {
        return;
    }
    let mut w = arrange(&[HEAVY_SHORT, TALL]).await;
    let obs = walk_rounds(&mut w, Trigger::Wedged).await;
    let first = first_root_fetch(&w, &obs).map(|x| x.peer);
    assert!(
        obs.started == Some(true) && first == Some("P_tall") && !asked(&w, &obs, "P_heavy"),
        "b2 pin: below the gate the Wedged walk no longer asks the tallest peer P_tall \
         first and only: {}",
        describe(&w, &obs)
    );
    assert_ne!(tip(&w).await.0, w.rb[2].hash(), "b2 pin: below gate on R3");
}
