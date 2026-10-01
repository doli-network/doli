//! INC-I-235 M1 — a node on the losing sibling of a 1-deep tip fork must reach
//! the canonical branch by reorg (parent walk + fork choice), never by snap.
//!
//! covers: periodic (Wedged arm), fork_recovery (`Node::try_trigger_fork_recovery`
//! seed C3, `handle_completed_fork_recovery`), dispatch (walk fetch while Headers
//! is in flight — exercised end to end).
//!
//! Contract the developer fulfils: the Wedged arm starts the walk through
//! `Node::try_trigger_fork_recovery()`; that function owns the C3 seed, the
//! SEC-001 eligibility gate on walked blocks, and the LB-9 re-walk bound. The
//! tests call it directly as the Wedged-arm stand-in because the coordinator
//! needs >= 300 s without an apply to classify Wedged.
//!
//! OUTPUT CONTRACT
//!   fn Node::try_trigger_fork_recovery(&mut self)
//!     O1 receiver: sync_manager walk active / seed (`fork_recovery_current_parent`)
//!   fn Node::run_periodic_tasks(&mut self) -> Result<()>   (walk poll → reorg)
//!     O1 chain_state.best_hash / best_height   O2 block_store canonical at FORK_H
//!     O3 sync_manager snap state (`is_snap_syncing`)   O4 fork_block_cache (seed source)
//!   PATHS: walk completes heavier → reorg; heavier but ineligible → no reorg;
//!          lighter → no reorg + no re-walk; no Wedged → no walk.
//!   MATRIX: each test names its (output, path) cell.
//!
//! INPUT PARTITIONS
//!   I1 seed2 shape, sibling A evicted (case b), Headers in flight, chase on walk peer
//!   I2 seed2 shape, sibling A cached (case a; its parent IS stored)
//!   I3 cache with stale orphans, stored-parent orphans, and the canonical chain (C3)
//!   I4 heavier walked branch, fork-point block by an UNSCHEDULED producer (SEC-001)
//!   I5 I4 with a scheduled producer (control: the walk does run in this fixture)
//!   I6 lighter walked branch, re-trigger after the 30 s cooldown (LB-9)
//!   I7 non-wedged node receiving an orphan (behind-ness hot path)       — lock
//!   I8 the walk's result handed straight to fork choice (fixture can converge) — lock

use crypto::{Hash, KeyPair};
use doli_core::Block;
use doli_node::node::Node;
use network::protocols::{SyncRequest, SyncResponse};
use network::PeerId;
use tempfile::TempDir;

use super::inc_i_204_m41_common::{
    apply_chain, build_block, build_chain, leader, make_node, unscheduled,
};

const FORK_H: u64 = 6;
const B_SLOT: u32 = 108;
const A_SLOT: u32 = 100;
/// A+1 has a LOWER slot than the local sibling B, as 02f75ac6 (606430) < 605e167d (606437).
const A1_SLOT: u32 = 103;
/// Canonical descendants past A: gap 55 at Wedged entry closes Rule 1/1b (gap >= 50).
const DEEP: usize = 55;

struct Fx {
    node: Node,
    producers: Vec<KeyPair>,
    base: Vec<Block>,
    b: Vec<Block>,
    /// canon[i] sits at height FORK_H + i; canon[0] = A, canon[1] = A1.
    canon: Vec<Block>,
    peers: Vec<PeerId>,
    _tmp: TempDir,
}

struct Shape {
    desc: usize,
    local_len: usize,
    a_eligible: bool,
    gossip_desc: bool,
    a_cached: bool,
}

async fn fixture(s: Shape) -> Fx {
    let (mut node, producers, tmp) = make_node(3).await;
    let params = node.params.clone();
    node.floor_fallback_window = true; // offline blocks carry no VDF: gossip applies Light
    let base = build_chain(1, 1, Hash::ZERO, &producers[0], 5, &params);
    apply_chain(&mut node, &base).await;
    let base_tip = base[4].hash();

    let mut b = Vec::new();
    let mut prev = base_tip;
    for i in 0..s.local_len {
        let slot = B_SLOT + i as u32;
        let blk = build_block(
            FORK_H + i as u64,
            slot,
            prev,
            leader(&producers, slot),
            &params,
        );
        prev = blk.hash();
        b.push(blk);
    }
    apply_chain(&mut node, &b).await;

    let a_by = if s.a_eligible {
        leader(&producers, A_SLOT)
    } else {
        unscheduled(&producers, A_SLOT)
    };
    let mut canon = vec![build_block(FORK_H, A_SLOT, base_tip, a_by, &params)];
    for i in 1..=s.desc {
        let slot = if i == 1 { A1_SLOT } else { 110 + i as u32 };
        let p = canon[i - 1].hash();
        canon.push(build_block(
            FORK_H + i as u64,
            slot,
            p,
            leader(&producers, slot),
            &params,
        ));
    }

    let tip = canon.last().unwrap().clone();
    let peers: Vec<PeerId> = (0..3).map(|_| PeerId::random()).collect();
    {
        let mut sm = node.sync_manager.write().await;
        for p in &peers {
            sm.add_peer(*p, FORK_H + s.desc as u64, tip.hash(), tip.header.slot);
        }
    }
    for blk in &canon[1..] {
        if s.gossip_desc {
            node.handle_new_block(blk.clone(), peers[0]).await.unwrap();
        } else {
            node.fork_block_cache
                .write()
                .await
                .insert(blk.hash(), blk.clone());
        }
    }
    if s.a_cached {
        node.fork_block_cache
            .write()
            .await
            .insert(canon[0].hash(), canon[0].clone());
    }
    assert_eq!(
        node.chain_state.read().await.best_hash,
        b.last().unwrap().hash(),
        "fixture: the node must sit on its own branch before any recovery"
    );
    Fx {
        node,
        producers,
        base,
        b,
        canon,
        peers,
        _tmp: tmp,
    }
}

fn seed2(a_cached: bool) -> Shape {
    Shape {
        desc: DEEP,
        local_len: 1,
        a_eligible: true,
        gossip_desc: true,
        a_cached,
    }
}

impl Fx {
    fn canon_at(&self, h: u64) -> Option<Block> {
        h.checked_sub(FORK_H)
            .and_then(|i| self.canon.get(i as usize))
            .cloned()
    }
    fn serve(&self, hash: Hash) -> Option<Block> {
        self.canon
            .iter()
            .chain(&self.base)
            .find(|b| b.hash() == hash)
            .cloned()
    }
    async fn tip(&self) -> Hash {
        self.node.chain_state.read().await.best_hash
    }
    async fn deliver(&mut self, from: PeerId, blk: Option<Block>) {
        let out = self
            .node
            .sync_manager
            .write()
            .await
            .handle_response(from, SyncResponse::Block(blk));
        for b in out {
            self.node.handle_new_block(b, from).await.unwrap();
        }
    }
    async fn header_in_flight(&mut self) {
        let req = self.node.sync_manager.write().await.next_request();
        assert!(
            matches!(&req, Some((_, r)) if !matches!(r, SyncRequest::GetBlockByHash { .. })),
            "fixture: Syncing{{DownloadingHeaders}} must own an in-flight header request"
        );
    }
    /// One tick = dispatch, ORPHAN_CHASE reply (best+1 by height) on the walk
    /// peer when `chase`, the by-hash reply, then the node's periodic poll.
    /// Returns the by-hash hashes requested.
    async fn drive(&mut self, ticks: usize, chase: bool) -> Vec<Hash> {
        let mut fetched = Vec::new();
        let target = self.canon.last().unwrap().hash();
        for _ in 0..ticks {
            // Epoch boundaries may disarm the Light window; the fixture's blocks carry no VDF.
            self.node.floor_fallback_window = true;
            let req = self.node.sync_manager.write().await.next_request();
            let walk = match &req {
                Some((q, SyncRequest::GetBlockByHash { hash })) => Some((*q, *hash)),
                _ => None,
            };
            if chase {
                let best = self.node.chain_state.read().await.best_height;
                let from = walk.map(|w| w.0).unwrap_or(self.peers[0]);
                let blk = self.canon_at(best + 1);
                if blk.is_some() {
                    self.deliver(from, blk).await;
                }
            }
            if let Some((q, hash)) = walk {
                fetched.push(hash);
                let blk = self.serve(hash);
                self.deliver(q, blk).await;
            }
            let _ = self.node.run_periodic_tasks().await;
            if self.tip().await == target {
                break;
            }
        }
        fetched
    }
}

// ---------------------------------------------------------------------------
// REQ-I235-001 / 002 / 004 / 005 — the seed2 reproduction
// ---------------------------------------------------------------------------

async fn assert_converged(fx: &Fx, fetched: &[Hash], case: &str) {
    assert!(
        fetched.contains(&fx.canon[0].hash()),
        "REQ-I235-002 ({}): the fork-point sibling A was never requested by hash \
         (by-hash fetches={}); seed2 made 0 such requests in 48 min",
        case,
        fetched.len()
    );
    assert_eq!(
        fx.tip().await,
        fx.canon.last().unwrap().hash(),
        "REQ-I235-005 ({}): tip must be the canonical tip; still on the losing sibling = {}",
        case,
        fx.tip().await == fx.b[0].hash()
    );
    let at_fork = fx
        .node
        .block_store
        .get_block_by_height(FORK_H)
        .unwrap()
        .map(|b| b.hash());
    assert_eq!(
        at_fork,
        Some(fx.canon[0].hash()),
        "REQ-I235-004 ({}): A canonical at h=6",
        case
    );
    assert!(
        !fx.node.sync_manager.read().await.is_snap_syncing(),
        "REQ-I235-005 ({}): never snap",
        case
    );
}

// REQ-I235-001 — Decision: a FAIL on current code is the seed2 wedge (no walk is
// served, the chase loop is the only traffic); a FAIL after the fix means one of
// D2 / C1 / C2 / C3 / SEC-001 still blocks the reorg. Cell: O1/O2/O3 × I1.
#[tokio::test]
async fn repro_wedged_losing_sibling_converges_by_reorg_case_b_sibling_evicted() {
    let mut fx = fixture(seed2(false)).await;
    fx.header_in_flight().await;
    fx.node.try_trigger_fork_recovery().await;
    let fetched = fx.drive(12, true).await;
    assert_converged(&fx, &fetched, "case b").await;
}

// REQ-I235-001 — Decision: a FAIL means a cached (unvalidated) A — whose parent is
// stored — seeds or blocks the walk instead of A1. Cell: O1/O2/O3 × I2.
#[tokio::test]
async fn repro_wedged_losing_sibling_converges_by_reorg_case_a_sibling_cached() {
    let mut fx = fixture(seed2(true)).await;
    fx.header_in_flight().await;
    fx.node.try_trigger_fork_recovery().await;
    let fetched = fx.drive(12, true).await;
    assert_converged(&fx, &fetched, "case a").await;
}

// REQ-I235-004 lock — Decision: a FAIL means the fixture itself cannot converge
// even when the walk result reaches fork choice, so a repro FAIL would prove
// nothing about the fix. Green before and after. Cell: O1/O2 × I8.
#[tokio::test]
async fn lock_fixture_converges_once_walk_result_reaches_fork_choice() {
    let mut fx = fixture(seed2(false)).await;
    let done = network::sync::CompletedRecovery {
        blocks: vec![fx.canon[0].clone(), fx.canon[1].clone()],
        connection_point: fx.base[4].hash(),
    };
    fx.node.handle_completed_fork_recovery(done).await.unwrap();
    let after_reorg = fx.node.chain_state.read().await.best_height;
    let _ = fx.drive(12, true).await;
    let end = fx.node.chain_state.read().await.best_height;
    assert_eq!(
        fx.tip().await,
        fx.canon.last().unwrap().hash(),
        "fixture: height after reorg={} end={} target={}",
        after_reorg,
        end,
        FORK_H + DEEP as u64
    );
}

// REQ-I235-002 — Decision: a FAIL means the Wedged arm still only fetches peer
// tips and returns (D2), so no runtime path starts the walk in the wedge.
// Cell: source falsifier — the arm found (non-vacuous) and it calls the starter.
#[test]
fn d2_wedged_arm_starts_the_parent_walk() {
    let src = include_str!("../../src/node/periodic.rs");
    let at = src
        .find("network::RecoveryAction::Wedged")
        .expect("Wedged arm must exist in periodic.rs");
    let arm = &src[at..];
    let end = arm.find("return Ok(());").expect("Wedged arm must return");
    assert!(
        arm[..end].contains("try_trigger_fork_recovery"),
        "D2: the Wedged arm must start the parent walk via try_trigger_fork_recovery()"
    );
}

// ---------------------------------------------------------------------------
// C3 — seed selection (REQ-I235-002)
// ---------------------------------------------------------------------------

// REQ-I235-002 — Decision: a FAIL means the walk seed is still arbitrary
// (`values().next()`): a stored-parent or stale orphan, or a far descendant, is
// chosen instead of A1, and the walk misses or overruns the fork point.
// Three fresh caches: an arbitrary pick passes all three with p ~ 7e-6.
// Cell: O1 × I3.
#[tokio::test]
async fn c3_seed_is_lowest_slot_orphan_with_missing_parent_above_fork_point() {
    for trial in 0..3 {
        let mut fx = fixture(Shape {
            desc: 30,
            local_len: 1,
            a_eligible: true,
            gossip_desc: false,
            a_cached: true,
        })
        .await;
        let params = fx.node.params.clone();
        let mut junk = Vec::new();
        for i in 0..10u32 {
            let parent = &fx.base[(i % 4) as usize];
            let h = (i % 4) as u64 + 2;
            junk.push(build_block(
                h,
                40 + i,
                parent.hash(),
                &fx.producers[1],
                &params,
            ));
            let stale_parent = crypto::hash::hash(format!("stale_{}_{}", trial, i).as_bytes());
            junk.push(build_block(
                3,
                2 + i % 3,
                stale_parent,
                &fx.producers[2],
                &params,
            ));
        }
        for j in junk {
            fx.node.fork_block_cache.write().await.insert(j.hash(), j);
        }
        fx.node.try_trigger_fork_recovery().await;
        let sm = fx.node.sync_manager.read().await;
        assert!(
            sm.is_fork_recovery_active(),
            "trial {}: a walk must start",
            trial
        );
        assert_eq!(
            sm.fork_recovery_current_parent(),
            Some(fx.canon[0].hash()),
            "C3 trial {}: seed must be A1 (lowest-slot cached orphan whose parent is NOT \
             stored, above the stored fork point) so the first fetch is the sibling A",
            trial
        );
    }
}

// ---------------------------------------------------------------------------
// SEC-001 — eligibility gate on walked blocks
// ---------------------------------------------------------------------------

async fn walk_once(fx: &mut Fx) {
    fx.node.try_trigger_fork_recovery().await;
    assert!(
        fx.node.sync_manager.read().await.is_fork_recovery_active(),
        "fixture: the walk must start (only A1 is cached)"
    );
    let _ = fx.drive(3, false).await;
}

fn sec_shape(a_eligible: bool) -> Shape {
    Shape {
        desc: 1,
        local_len: 1,
        a_eligible,
        gossip_desc: false,
        a_cached: false,
    }
}

// REQ-I235-SEC-001 — Decision: a FAIL means a peer-served branch whose fork-point
// block comes from a producer that does not own the slot is reorged onto because
// it is "heavier" than a frozen tip (execute_reorg applies Light, no eligibility).
// Cell: O1/O2 × I4.
#[tokio::test]
async fn sec001_walked_block_by_unscheduled_producer_is_not_reorged_onto() {
    let mut fx = fixture(sec_shape(false)).await;
    walk_once(&mut fx).await;
    assert_eq!(
        fx.tip().await,
        fx.b[0].hash(),
        "SEC-001: an ineligible walked block (A by a non-leader) must not enter the reorg"
    );
}

// REQ-I235-SEC-001 control — Decision: a FAIL means the walk does not run in this
// fixture, so the SEC-001 "no reorg" result above would be vacuous.
// Cell: O1/O2 × I5.
#[tokio::test]
async fn sec001_control_eligible_walked_branch_is_reorged_onto() {
    let mut fx = fixture(sec_shape(true)).await;
    walk_once(&mut fx).await;
    assert_eq!(
        fx.tip().await,
        fx.canon[1].hash(),
        "eligible heavier branch must win"
    );
}

// ---------------------------------------------------------------------------
// LB-9 — a lighter branch is not re-walked every cooldown
// ---------------------------------------------------------------------------

// REQ-I235-SEC-001 (LB-9) — Decision: a FAIL means every Wedged tick past the
// 30 s cooldown re-walks the same lighter branch forever (~120 ungoverned by-hash
// fetches per 150 s). 31 s real wait: RECOVERY_COOLDOWN has no injection seam.
// Cell: O1 × I6.
#[tokio::test]
async fn lb9_lighter_branch_is_not_rewalked_after_cooldown() {
    let mut fx = fixture(Shape {
        desc: 1,
        local_len: 3,
        a_eligible: true,
        gossip_desc: false,
        a_cached: false,
    })
    .await;
    walk_once(&mut fx).await;
    assert_eq!(
        fx.tip().await,
        fx.b[2].hash(),
        "fixture: lighter [A, A1] must lose"
    );

    tokio::time::sleep(std::time::Duration::from_secs(31)).await;
    fx.node.try_trigger_fork_recovery().await;
    assert!(
        !fx.node.sync_manager.read().await.is_fork_recovery_active(),
        "LB-9: a branch already judged not heavier must not be walked again"
    );
}

// ---------------------------------------------------------------------------
// REQ-I235-006 — behind-ness hot path unchanged
// ---------------------------------------------------------------------------

// REQ-I235-006 lock — Decision: a FAIL means walks now start on ordinary orphan
// gossip or on a non-wedged tick, turning every behind-ness orphan into by-hash
// traffic. Cell: O1 × I7.
#[tokio::test]
async fn lock_non_wedged_orphan_gossip_starts_no_walk() {
    let (mut node, producers, _tmp) = make_node(3).await;
    let params = node.params.clone();
    let base = build_chain(1, 1, Hash::ZERO, &producers[0], 5, &params);
    apply_chain(&mut node, &base).await;
    let peer = PeerId::random();
    node.sync_manager
        .write()
        .await
        .add_peer(peer, 7, crypto::hash::hash(b"t"), 7);
    let orphan = build_block(
        7,
        7,
        crypto::hash::hash(b"unknown_h6"),
        leader(&producers, 7),
        &params,
    );
    node.handle_new_block(orphan, peer).await.unwrap();
    let _ = node.run_periodic_tasks().await;
    assert!(!node.sync_manager.read().await.is_fork_recovery_active());
}
