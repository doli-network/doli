//! INC-I-288 E-2 fixtures on top of the C-1 rig: peer status, gossip from a named
//! peer, the node's periodic tick, the coordinator dispatch call, and the rollback
//! and network-tip observables.
//!
//! OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.
#![allow(dead_code)]

use crypto::{Hash, KeyPair};
use doli_core::validation::ValidationMode;
use doli_core::Block;
use network::{PeerId, RecoveryAction};

pub use super::inc_i_288_c1_common::*;

/// A peer whose status names (`height`, `tip`, `slot`).
pub async fn peer_at(r: &Rig, height: u64, tip: Hash, slot: u32) -> PeerId {
    let peer = PeerId::random();
    r.node
        .sync_manager
        .write()
        .await
        .add_peer(peer, height, tip, slot);
    peer
}

/// A gossip block from `from` (the node's gossip intake).
pub async fn deliver(r: &mut Rig, b: &Block, from: PeerId) {
    r.node
        .handle_new_block(b.clone(), from)
        .await
        .expect("handle_new_block returns Ok");
}

/// The node's periodic tick: evidence report, coordinator classify, executor.
pub async fn tick(r: &mut Rig) {
    r.node
        .run_periodic_tasks()
        .await
        .expect("run_periodic_tasks returns Ok");
}

/// The coordinator dispatch call the tick makes; returns the action and the gap.
pub async fn dispatch(r: &mut Rig) -> (RecoveryAction, u64) {
    let budget = r.node.shallow_rollback_count;
    let (action, ctx) = r
        .node
        .sync_manager
        .write()
        .await
        .classify_and_dispatch(budget);
    (action, ctx.map(|c| c.gap()).unwrap_or(0))
}

/// `n` EmptyHeaders reports, as the tick makes them (peers cannot extend our tip).
pub async fn empty_headers(r: &Rig, peer: PeerId, n: usize) {
    let mut sync = r.node.sync_manager.write().await;
    let gap = sync.network_tip_height().saturating_sub(sync.local_tip().0);
    for _ in 0..n {
        sync.report_empty_headers(peer, gap);
    }
}

pub async fn net_tip(r: &Rig) -> u64 {
    r.node.sync_manager.read().await.network_tip_height()
}

pub async fn removed(r: &Rig, h: &Hash) -> bool {
    r.node.sync_manager.read().await.removed_by_rollback(h)
}

pub async fn cached(r: &Rig, h: &Hash) -> bool {
    r.node.fork_block_cache.read().await.contains_key(h)
}

/// (shallow_rollback_count, cumulative_rollback_depth).
pub fn rollbacks(r: &Rig) -> (u32, u32) {
    (
        r.node.shallow_rollback_count,
        r.node.cumulative_rollback_depth,
    )
}

pub fn own_key(r: &Rig) -> KeyPair {
    r.node
        .producer_key
        .clone()
        .expect("fixture: test node has a producer key")
}

/// First slot >= `from` whose scheduled leader is (`own`) or is not the node's key.
pub fn slot_led(r: &Rig, from: u32, own: bool) -> u32 {
    let me = *own_key(r).public_key();
    (from..from + 30)
        .find(|s| (*leader(&r.keys, *s).public_key() == me) == own)
        .expect("fixture: a slot for the wanted leader")
}

/// A block at `height` on `prev`, by the slot's scheduled leader.
pub fn child(r: &Rig, height: u64, slot: u32, prev: Hash) -> Block {
    let by = leader(&r.keys, slot).clone();
    block(height, slot, prev, &by, &r.node.params)
}

/// What the heal intake and one tick left behind.
#[derive(Debug)]
pub struct HealObs {
    pub net_after_intake: u64,
    pub net_after_tick: u64,
    pub rollbacks_before: (u32, u32),
    pub rollbacks_after: (u32, u32),
    pub old_tip_removed: bool,
    pub orphans_cached: Vec<bool>,
}

/// The heal: peers report `peer_tips` (height, block); `orphans` arrive by gossip
/// from the last of them, parents unknown; then one periodic tick runs.
pub async fn heal_tick(r: &mut Rig, peer_tips: &[(u64, Block)], orphans: &[Block]) -> HealObs {
    let old_tip = r.node.chain_state.read().await.best_hash;
    let mut from = PeerId::random();
    for (h, b) in peer_tips {
        from = peer_at(r, *h, b.hash(), b.header.slot).await;
    }
    let rollbacks_before = rollbacks(r);
    for b in orphans {
        deliver(r, b, from).await;
    }
    let net_after_intake = net_tip(r).await;
    tick(r).await;
    let mut orphans_cached = Vec::new();
    for b in orphans {
        orphans_cached.push(cached(r, &b.hash()).await);
    }
    HealObs {
        net_after_intake,
        net_after_tick: net_tip(r).await,
        rollbacks_before,
        rollbacks_after: rollbacks(r),
        old_tip_removed: removed(r, &old_tip).await,
        orphans_cached,
    }
}

/// The behind side: own A1, A2 (F+2); a peer reports F+5; peers cannot extend our
/// tip (EmptyHeaders x2); one tick: the coordinator walks down to A1 (h = F+1).
/// Its ShallowRollback starts the 30 s cooldown (Gate 2).
pub async fn after_coordinator_rollback() -> (Rig, PeerId) {
    let mut r = rig(&[210, 211], &[]).await;
    let a1 = r.ours[0].clone();
    let peer = peer_at(&r, F + 5, crypto::hash::hash(b"canonical tip"), 230).await;
    empty_headers(&r, peer, 2).await;
    tick(&mut r).await;
    assert_eq!(
        rollbacks(&r),
        (1, 1),
        "precondition: the coordinator walked down one block"
    );
    assert_on(&r, &a1, F + 1, "precondition: tip A1 after the rollback").await;
    (r, peer)
}

/// A block at F+2 on A1, not the removed A2; `own` picks the node's key as leader.
pub fn block_on_a1(r: &Rig, own: bool) -> Block {
    let b = child(r, F + 2, slot_led(r, 212, own), r.ours[0].hash());
    let me = *own_key(r).public_key();
    assert_eq!(
        b.header.producer == me,
        own,
        "precondition: producer origin"
    );
    assert_ne!(
        b.hash(),
        r.ours[1].hash(),
        "precondition: not the removed A2"
    );
    b
}

/// The production path's local steps (production/mod.rs): apply_block(Light) of
/// the node's own block, then last_produced_slot. No broadcast, no peer mark.
pub async fn apply_as_produced(r: &mut Rig, b: &Block) {
    r.node
        .apply_block(b.clone(), ValidationMode::Light)
        .await
        .expect("own block applies");
    r.node.last_produced_slot = Some(b.header.slot as u64);
}

/// The periodic pending-sync apply (periodic.rs): apply_block(Light), no peer mark.
pub async fn apply_as_synced(r: &mut Rig, b: &Block) {
    r.node
        .apply_block(b.clone(), ValidationMode::Light)
        .await
        .expect("synced block applies");
}
