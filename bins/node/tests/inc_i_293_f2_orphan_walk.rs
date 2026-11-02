//! INC-I-293 F2 (run 647, trial .4 n2/n3): a producing minority on its own branch
//! D1..D4 above the fork point F receives ONLY the heavier branch's descendants
//! R4..R6 as gossip. Their parents R1..R3 are not stored, not canonical and not
//! retained siblings; the sending peer serves them by hash. Today the Orphan arm
//! only chases by height and caches, so no by-hash walk starts, and the Wedged
//! terminal (the only orphan-reachable walk starter) needs 300 s with no applied
//! block, which the minority's own production keeps resetting.
//!
//! The peer is a registered `PeerId` that answers every `GetBlockByHash` the node's
//! sync manager hands out (`next_request`), as `drive_walk` in the F-2c fixtures
//! does. The test node has no network handle, so the by-height ORPHAN_CHASE request
//! is not observable here; the observable is the by-hash fetch (or its absence).
//!
//! Bound: N = 3 slots from the first orphan. One slot = the peer gossips one
//! descendant, the minority applies one own block while its tip is still on D, the
//! peer answers every pending by-hash fetch (the node's walk poll runs after each
//! answer), one periodic tick, the peer answers again. Live, that is under 30 s, far
//! below 300 s. The test node has no network handle, so a request that the tick's
//! `next_request` sends (periodic.rs, after the walk poll) is dropped: the peer
//! answers the fetch before the tick, as it answers the tick's fetch live.
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(block, peer) (Orphan arm: parent neither
//   stored nor retained -> ORPHAN_CHASE by height + cache), then fn Node::run_periodic_tasks()
//   O1: chain_state best_hash / best_height, canonical entry, persisted tip, sync tip
//   O2: the fork walk: is_fork_recovery_active() after the first orphan and the
//     GetBlockByHash requests next_request() hands to the peer (R4's parent R3 first)
//   O3: fork_block_cache holds R4..R6 (or the catch-up orphan X2); the shallow-rollback
//     counters (reported)
//   O4: sync network_tip_slot, raised by the orphan bookkeeping beside ORPHAN_CHASE
//   PATHS: P1 at/above the INC-I-288 AH, descendants only (red); P2 = P1 + the walk
//     started by hand at the first orphan (control: walk + fork choice + cached-child
//     drain reach R6 in the bound); P3 = P1 + the roots R1..R3 gossiped first
//     (control: the existing doors); P4 below the AH (child re-exec), descendants only (pin);
//     P5 at/above the AH, a same-chain catch-up orphan X2 (parent X1 unknown) at slot
//     best_slot + MAX_REORG_DEPTH + 1 (red at 05126061); P6 = P5 at slot
//     best_slot + MAX_REORG_DEPTH exactly (boundary control)
//   INPUT PARTITIONS: D = 4 blocks + 1 own block per slot, all by keys[0] (support 1);
//     R = 6 blocks by keys[1], keys[2] (support 2); X1, X2 on D4; fixed-seed keys
//   MATRIX: P1 x O1 O2 -> f2a · P2 x O1 O2 -> f2c1 · P3 x O1 -> f2c2 · P4 x O1 O2 O3 -> f2d
//     · P5 x O1 O2 O3 O4 -> f2e · P6 x O2 O3 -> f2f
//   covers: block_handling fork_walk fork_recovery periodic recovery block_lifecycle

#[path = "support/block_sig.rs"]
mod block_sig;
#[path = "it/inc_i_289_common.rs"]
mod inc_i_289_common;

use crypto::{Hash, KeyPair};
use doli_core::validation::ValidationMode;
use doli_core::Block;
use inc_i_289_common::*;
use network::protocols::{SyncRequest, SyncResponse};
use network::PeerId;

/// The bound, in slots from the first orphan.
const N: usize = 3;

#[derive(Clone, Copy, PartialEq)]
enum Feed {
    /// The peer gossips R4..R6 only (trial .4).
    Descendants,
    /// As `Descendants`, and the walk is started by hand at the first orphan.
    WalkStartedByHand,
    /// The peer gossips the roots R1..R3 first, then R4..R6.
    WithRoots,
}

struct Run {
    r: Rig,
    /// R1..R6 at F+1..F+6.
    maj: Vec<Block>,
    peer: PeerId,
    last_slot: u32,
}

struct Obs {
    /// (slot index 1..=N, hash) for every GetBlockByHash the peer answered.
    fetched: Vec<(usize, Hash)>,
    walk_active_after_first_orphan: bool,
    own_blocks: usize,
}

fn fixed_keys() -> Vec<KeyPair> {
    (1..=3u8).map(|i| KeyPair::from_seed([i; 32])).collect()
}

/// The first `n` slots after `after` whose leader is (or is not) keys[0].
fn slots_led(keys: &[KeyPair], after: u32, n: usize, own: bool) -> Vec<u32> {
    (after + 1..)
        .filter(|s| (leader(keys, *s).public_key() == keys[0].public_key()) == own)
        .take(n)
        .collect()
}

fn short(h: &Hash) -> String {
    h.to_string()[..8].to_string()
}

fn canonical(r: &Rig, h: &Hash) -> bool {
    matches!(r.node.block_store.get_height_by_hash(h), Ok(Some(_)))
}

/// D1..D4 by keys[0] applied on F; R1..R6 on F by keys[1], keys[2]; the peer's
/// status names R6 (so `best_peer_for_recovery` picks it).
async fn arrange() -> Run {
    let keys = fixed_keys();
    let d_slots = slots_led(&keys, F as u32, 4, true);
    let r = rig_with_keys(keys, &d_slots).await;
    let r_slots = slots_led(&r.keys, F as u32, 6, false);
    let maj = chain_on(&r, r.f_hash, F + 1, &r_slots);
    let own = *r.keys[0].public_key();
    assert!(
        r.ours.iter().all(|b| b.header.producer == own),
        "fixture: D is by keys[0] only"
    );
    assert!(
        maj.iter().all(|b| b.header.producer != own)
            && maj[0].header.producer != maj[1].header.producer,
        "fixture: R is by keys[1] and keys[2]"
    );
    let peer = PeerId::random();
    let top = &maj[5];
    (r.node.sync_manager.write().await).add_peer(peer, F + 6, top.hash(), top.header.slot);
    let last_slot = d_slots
        .last()
        .copied()
        .max(r_slots.last().copied())
        .unwrap();
    Run {
        r,
        maj,
        peer,
        last_slot,
    }
}

async fn from_peer(run: &mut Run, b: &Block) {
    run.r
        .node
        .handle_new_block(b.clone(), run.peer)
        .await
        .expect("handle_new_block returns Ok");
}

/// One own block (keys[0], its next led slot) on the current tip.
async fn produce_own(run: &mut Run) {
    let (tip, height) = {
        let cs = run.r.node.chain_state.read().await;
        (cs.best_hash, cs.best_height)
    };
    let slot = slots_led(&run.r.keys, run.last_slot, 1, true)[0];
    run.last_slot = slot;
    let b = block(height + 1, slot, tip, &run.r.keys[0], &run.r.node.params);
    run.r
        .node
        .apply_block(b, ValidationMode::Light)
        .await
        .expect("own block applies");
}

/// The peer answers every pending by-hash fetch; the node's walk poll
/// (periodic.rs: connect at a canonical parent) runs after each answer.
async fn serve_peer(run: &mut Run, slot: usize, fetched: &mut Vec<(usize, Hash)>) {
    for _ in 0..8 {
        let req = run.r.node.sync_manager.write().await.next_request();
        let Some((q, SyncRequest::GetBlockByHash { hash })) = req else {
            break;
        };
        fetched.push((slot, hash));
        let blk = run.maj.iter().find(|b| b.hash() == hash).cloned();
        let out =
            (run.r.node.sync_manager.write().await).handle_response(q, SyncResponse::Block(blk));
        for b in out {
            run.r
                .node
                .handle_new_block(b, q)
                .await
                .expect("handle_new_block returns Ok");
        }
        let parent = (run.r.node.sync_manager.read().await).fork_recovery_current_parent();
        if parent.is_some_and(|p| canonical(&run.r, &p)) {
            let done = (run.r.node.sync_manager.write().await).check_fork_recovery_connection(true);
            if let Some(done) = done {
                let res = run.r.node.handle_completed_fork_recovery(done).await;
                eprintln!("[F2] completed walk -> {res:?}");
            }
        }
    }
}

async fn on_heavier(r: &Rig, maj: &[Block]) -> bool {
    r.node.block_store.get_hash_by_height(F + 1).unwrap() == Some(maj[0].hash())
}

async fn rounds(run: &mut Run, feed: Feed) -> Obs {
    let mut obs = Obs {
        fetched: Vec::new(),
        walk_active_after_first_orphan: false,
        own_blocks: 0,
    };
    if feed == Feed::WithRoots {
        let roots: Vec<Block> = run.maj[..3].to_vec();
        for b in &roots {
            from_peer(run, b).await;
        }
    }
    for k in 1..=N {
        let b = run.maj[2 + k].clone();
        from_peer(run, &b).await;
        if k == 1 {
            if feed == Feed::WalkStartedByHand {
                // After the F2 fix the orphan arm has already started it (one walk at a time).
                let started = run.r.node.try_trigger_fork_recovery().await;
                let active = (run.r.node.sync_manager.read().await).is_fork_recovery_active();
                assert!(
                    started || active,
                    "control: the walk starts by hand from the cached orphan"
                );
            }
            obs.walk_active_after_first_orphan =
                (run.r.node.sync_manager.read().await).is_fork_recovery_active();
        }
        if !on_heavier(&run.r, &run.maj).await {
            produce_own(run).await;
            obs.own_blocks += 1;
        }
        serve_peer(run, k, &mut obs.fetched).await;
        tick(&mut run.r).await;
        serve_peer(run, k, &mut obs.fetched).await;
    }
    obs
}

/// The Must: tip == R6 within N slots, with the walk evidence in the message.
async fn assert_on_heavier_branch(run: &Run, obs: &Obs, label: &str) {
    let r6 = &run.maj[5];
    let (tip, h) = {
        let cs = run.r.node.chain_state.read().await;
        (cs.best_hash, cs.best_height)
    };
    assert!(
        tip == r6.hash() && h == F + 6,
        "{label}: the producing minority is not on the heavier branch within N={N} slots of \
         the first orphan: tip h={h} {}, want R6 h={} {}; walk active after the first \
         orphan: {}; by-hash fetches answered: {:?}; own blocks applied meanwhile: {}",
        short(&tip),
        F + 6,
        short(&r6.hash()),
        obs.walk_active_after_first_orphan,
        obs.fetched,
        obs.own_blocks
    );
    assert_on(&run.r, r6, F + 6, label).await;
}

// F2 INC-I-293 — Decision: a failure means an orphan whose parent is unknown starts no by-hash walk, so a producing minority that sees only the heavier branch's descendants stays on its own branch until a 300 s stale-tip clock that its own blocks keep resetting.
#[tokio::test]
async fn f2a_orphan_with_unknown_parent_walks_and_minority_joins_heavier_branch() {
    let mut run = arrange().await;
    let obs = rounds(&mut run, Feed::Descendants).await;
    assert_on_heavier_branch(&run, &obs, "F2 (Must)").await;
    let r3 = run.maj[2].hash();
    assert!(
        obs.walk_active_after_first_orphan || obs.fetched.contains(&(1, r3)),
        "F2: no by-hash walk from the first orphan R4: walk active {}, fetched {:?}, want R3 {} in slot 1",
        obs.walk_active_after_first_orphan,
        obs.fetched,
        short(&r3)
    );
}

// F2 INC-I-293 control — Decision: a failure means the walk, fork choice or cached-child drain cannot reach R6 in the bound even once a walk runs, so f2a's red is not about the missing walk trigger.
#[tokio::test]
async fn f2c1_control_walk_started_at_first_orphan_reaches_heavier_branch() {
    let mut run = arrange().await;
    let obs = rounds(&mut run, Feed::WalkStartedByHand).await;
    assert!(
        obs.fetched.contains(&(1, run.maj[2].hash())),
        "control: the walk fetches R3 by hash in slot 1: {:?}",
        obs.fetched
    );
    assert_on_heavier_branch(&run, &obs, "F2 control (walk by hand)").await;
}

// F2 INC-I-293 control — Decision: a failure means the rig cannot heal even when the roots arrive, so f2a would be red for a reason other than F2.
#[tokio::test]
async fn f2c2_control_roots_gossiped_existing_doors_reach_heavier_branch() {
    let mut run = arrange().await;
    let obs = rounds(&mut run, Feed::WithRoots).await;
    assert_on_heavier_branch(&run, &obs, "F2 control (roots gossiped)").await;
}

// F2 INC-I-293 below-gate pin — Decision: a failure means today's below-AH (mainnet) orphan path changed: a gated fix must leave it bit-identical (INV-12), and an ungated one must say so.
#[tokio::test]
async fn f2d_below_gate_orphan_starts_no_walk_today() {
    if !in_below_ah_child("f2d_below_gate_orphan_starts_no_walk_today") {
        return;
    }
    let mut run = arrange().await;
    let obs = rounds(&mut run, Feed::Descendants).await;
    let (tip, h) = {
        let cs = run.r.node.chain_state.read().await;
        (cs.best_hash, cs.best_height)
    };
    eprintln!(
        "[F2 below AH] tip h={h} {}; (shallow_rollback_count, cumulative_depth) = {:?}; own blocks {}",
        short(&tip),
        rollbacks(&run.r),
        obs.own_blocks
    );
    assert!(
        !obs.walk_active_after_first_orphan && obs.fetched.is_empty(),
        "below AH pin: an orphan starts no by-hash walk today: active {}, fetched {:?}",
        obs.walk_active_after_first_orphan,
        obs.fetched
    );
    assert!(
        !on_heavier(&run.r, &run.maj).await && tip != run.maj[5].hash(),
        "below AH pin: the minority is not on R within N={N} slots today (tip h={h})"
    );
    for b in &run.maj[3..] {
        assert!(
            cached(&run.r, &b.hash()).await,
            "below AH pin: R4..R6 cached"
        );
    }
}

struct CatchUp {
    walk_active: bool,
    /// The first request the sync manager hands out after the orphan.
    first_request: Option<SyncRequest>,
    x1: Hash,
}

/// A same-chain catch-up: X1 on our tip D4 (never sent), X2 on X1 at
/// `best_slot + gap` sent by the peer as gossip.
async fn catch_up_orphan(gap: u32) -> (Run, CatchUp, Block) {
    let mut run = arrange().await;
    let (tip, height, best_slot) = {
        let cs = run.r.node.chain_state.read().await;
        (cs.best_hash, cs.best_height, cs.best_slot)
    };
    let x = chain_on(&run.r, tip, height + 1, &[best_slot + 1, best_slot + gap]);
    from_peer(&mut run, &x[1]).await;
    let walk_active = (run.r.node.sync_manager.read().await).is_fork_recovery_active();
    let first_request = (run.r.node.sync_manager.write().await)
        .next_request()
        .map(|(_, q)| q);
    let obs = CatchUp {
        walk_active,
        first_request,
        x1: x[0].hash(),
    };
    (run, obs, x[1].clone())
}

fn by_hash(q: &Option<SyncRequest>, want: &Hash) -> bool {
    matches!(q, Some(SyncRequest::GetBlockByHash { hash }) if hash == want)
}

// F2 INC-I-293 catch-up — Decision: a failure means a node behind on the same chain starts a by-hash walk from a tip gossip orphan more than MAX_REORG_DEPTH slots ahead, which pre-empts header-first sync for up to 1000 ticks and is then discarded (SEC-FIXVERIFY-647-001).
#[tokio::test]
async fn f2e_catch_up_orphan_beyond_reorg_depth_starts_no_walk() {
    let gap = network::MAX_REORG_DEPTH as u32 + 1;
    let (run, obs, x2) = catch_up_orphan(gap).await;
    assert!(
        !obs.walk_active && !by_hash(&obs.first_request, &obs.x1),
        "F2 catch-up: an orphan {gap} slots above our tip (> MAX_REORG_DEPTH) started a \
         by-hash walk: walk active {}, first request {:?}",
        obs.walk_active,
        obs.first_request
    );
    assert!(cached(&run.r, &x2.hash()).await, "F2 catch-up: X2 cached");
    let tip_slot = (run.r.node.sync_manager.read().await).network_tip_slot();
    assert_eq!(
        tip_slot, x2.header.slot,
        "F2 catch-up: the orphan bookkeeping beside ORPHAN_CHASE still ran"
    );
}

// F2 INC-I-293 boundary control — Decision: a failure means an orphan exactly MAX_REORG_DEPTH slots above our tip no longer starts the walk, so the depth screen cuts F2 coverage inside the window.
#[tokio::test]
async fn f2f_orphan_within_reorg_depth_starts_the_walk() {
    let gap = network::MAX_REORG_DEPTH as u32;
    let (run, obs, x2) = catch_up_orphan(gap).await;
    assert!(
        obs.walk_active && by_hash(&obs.first_request, &obs.x1),
        "F2 boundary: an orphan {gap} slots above our tip did not start the by-hash walk: \
         walk active {}, first request {:?}",
        obs.walk_active,
        obs.first_request
    );
    assert!(cached(&run.r, &x2.hash()).await, "F2 boundary: X2 cached");
}
