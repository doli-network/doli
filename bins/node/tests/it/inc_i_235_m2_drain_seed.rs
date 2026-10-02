//! INC-I-235 M2 — drain determinism (REQ-I235-009), walk-seed survival under
//! fork-cache eviction (W2), canonical-aware walk connection (REQ-I235-008).
//!
//! covers: block_handling (ORPHAN_APPLY drain, cache_block_with_eviction),
//! fork_walk (select_walk_seed / try_trigger_fork_recovery), periodic (walk poll).
//!
//! OUTPUT CONTRACT
//!   fn Node::handle_new_block(&mut self, Block, PeerId) -> Result<()>  (ExtendsTip + drain)
//!     O1 chain_state.best_hash / best_height   O2 block_store canonical by height
//!     O3 fork_block_cache (drained entries removed; orphans inserted + evicted)
//!   fn Node::try_trigger_fork_recovery(&mut self)
//!     O4 sync_manager walk active / `fork_recovery_current_parent`
//!   fn Node::run_periodic_tasks(&mut self)  (walk poll -> fork choice)
//!     O1, O2 as above
//!   PATHS: drain picks child (longest / tie-lower-slot / skip invalid);
//!          eviction past 100 entries; walk connects at stored vs non-canonical body.
//!
//! INPUT PARTITIONS
//!   D1 two tip children, lower-slot one has a cached child
//!   D2 two tip children, higher-slot one has a cached child (longest beats slot)
//!   D3 two childless tip children (tie -> lower slot)
//!   D4 lower-slot child invalid, higher-slot sibling valid
//!   S1 walk-seed chain + >80 higher-slot orphans (eviction fires)
//!   C1 fork-point sibling body stored but NOT canonical (rolled back), local branch 2 deep

use crypto::{Hash, KeyPair};
use doli_core::{Block, Transaction};
use doli_node::node::Node;
use network::protocols::{SyncRequest, SyncResponse};
use network::PeerId;
use tempfile::TempDir;

use super::inc_i_204_m41_common::{apply_chain, build_block, build_chain, leader, make_node};

const TRIALS: usize = 8;

async fn tip_node() -> (Node, Vec<KeyPair>, Block, TempDir) {
    let (mut node, producers, tmp) = make_node(3).await;
    let params = node.params.clone();
    node.floor_fallback_window = true;
    let base = build_chain(1, 1, Hash::ZERO, &producers[0], 5, &params);
    apply_chain(&mut node, &base[..4]).await;
    let t = base[4].clone();
    (node, producers, t, tmp)
}

fn child(node: &Node, producers: &[KeyPair], h: u64, slot: u32, prev: Hash) -> Block {
    build_block(h, slot, prev, leader(producers, slot), &node.params)
}

fn invalid_child(node: &Node, producers: &[KeyPair], h: u64, slot: u32, prev: Hash) -> Block {
    let mut b = child(node, producers, h, slot, prev);
    let reward = node.params.block_reward(h);
    let pool = doli_core::consensus::reward_pool_pubkey_hash();
    let cb = Transaction::new_coinbase(reward * 3 + 7, pool, h, 0);
    b.header.merkle_root = doli_core::block::compute_merkle_root(std::slice::from_ref(&cb));
    b.transactions = vec![cb];
    b
}

async fn cache(node: &Node, blocks: &[&Block]) {
    let mut c = node.fork_block_cache.write().await;
    for b in blocks {
        c.insert(b.hash(), (*b).clone());
    }
}

async fn tip(node: &Node) -> (Hash, u64) {
    let s = node.chain_state.read().await;
    (s.best_hash, s.best_height)
}

// ---------------------------------------------------------------------------
// REQ-I235-009 — ORPHAN_APPLY drain is deterministic
// ---------------------------------------------------------------------------

// REQ-I235-009 — Decision: a FAIL means the drain still takes an arbitrary
// HashMap child of the tip, so identical caches converge to different tips on
// different nodes (the onset differential of INC-I-235). Cell: O1/O2 x D1.
#[tokio::test]
async fn m2_repro_drain_prefers_sibling_with_longest_cached_chain() {
    for trial in 0..TRIALS {
        let (mut node, p, t, _tmp) = tip_node().await;
        let low = child(&node, &p, 6, 8, t.hash());
        let high = child(&node, &p, 6, 9, t.hash());
        let d = child(&node, &p, 7, 10, low.hash());
        cache(&node, &[&low, &high, &d]).await;
        node.handle_new_block(t.clone(), PeerId::random())
            .await
            .unwrap();
        assert_eq!(
            tip(&node).await,
            (d.hash(), 7),
            "trial {}: drain must apply the lower-slot sibling and its cached child",
            trial
        );
    }
}

// REQ-I235-009 — Decision: a FAIL means the drain orders by slot (or by chance)
// rather than by cached chain length, leaving the longer cached branch unapplied.
// Cell: O1/O2 x D2.
#[tokio::test]
async fn m2_repro_drain_longest_chain_beats_lower_slot() {
    for trial in 0..TRIALS {
        let (mut node, p, t, _tmp) = tip_node().await;
        let low = child(&node, &p, 6, 8, t.hash());
        let high = child(&node, &p, 6, 9, t.hash());
        let d = child(&node, &p, 7, 10, high.hash());
        cache(&node, &[&low, &high, &d]).await;
        node.handle_new_block(t.clone(), PeerId::random())
            .await
            .unwrap();
        assert_eq!(
            tip(&node).await,
            (d.hash(), 7),
            "trial {}: the sibling rooting the longer cached chain must win",
            trial
        );
        let at6 = node
            .block_store
            .get_block_by_height(6)
            .unwrap()
            .map(|b| b.hash());
        assert_eq!(at6, Some(high.hash()), "trial {}", trial);
    }
}

// REQ-I235-009 — Decision: a FAIL means equal-length siblings are not broken by
// the lower slot, so the tie is still decided by HashMap order. Cell: O1 x D3.
#[tokio::test]
async fn m2_repro_drain_tie_goes_to_lower_slot() {
    for trial in 0..TRIALS {
        let (mut node, p, t, _tmp) = tip_node().await;
        let low = child(&node, &p, 6, 8, t.hash());
        let high = child(&node, &p, 6, 9, t.hash());
        cache(&node, &[&high, &low]).await;
        node.handle_new_block(t.clone(), PeerId::random())
            .await
            .unwrap();
        assert_eq!(
            tip(&node).await,
            (low.hash(), 6),
            "trial {}: tie -> lower slot",
            trial
        );
    }
}

// REQ-I235-009 — Decision: a FAIL means one invalid cached child of the tip
// (`break` on apply error) stops the drain and strands its valid sibling in the
// cache. Cell: O1/O3 x D4.
#[tokio::test]
async fn m2_repro_drain_invalid_child_does_not_stop_valid_sibling() {
    {
        let (mut probe, p, t, _tmp) = tip_node().await;
        let bad = invalid_child(&probe, &p, 6, 8, t.hash());
        apply_chain(&mut probe, std::slice::from_ref(&t)).await;
        assert!(
            probe
                .apply_block(bad, doli_core::validation::ValidationMode::Light)
                .await
                .is_err(),
            "fixture: the invalid child must be rejected by apply_block"
        );
    }
    for trial in 0..TRIALS {
        let (mut node, p, t, _tmp) = tip_node().await;
        let bad = invalid_child(&node, &p, 6, 8, t.hash());
        let good = child(&node, &p, 6, 9, t.hash());
        cache(&node, &[&bad, &good]).await;
        node.handle_new_block(t.clone(), PeerId::random())
            .await
            .unwrap();
        assert_eq!(
            tip(&node).await,
            (good.hash(), 6),
            "trial {}: the valid sibling must be applied after the invalid one fails",
            trial
        );
        assert!(
            !node
                .fork_block_cache
                .read()
                .await
                .contains_key(&good.hash()),
            "trial {}: the applied sibling must leave the cache",
            trial
        );
    }
}

// ---------------------------------------------------------------------------
// W2 — the walk seed survives fork-cache eviction
// ---------------------------------------------------------------------------

const FORK_H: u64 = 6;

// W2 (REQ-I235-011 companion) — Decision: a FAIL means slot-sorted eviction
// drops the lowest-slot (fork-nearest) root of the competing branch first, so
// under orphan pressure the walk seeds from a far descendant or not at all.
// Cell: O3/O4 x S1.
#[tokio::test]
async fn m2_repro_seed_walk_root_survives_cache_eviction() {
    let (mut node, p, tmp) = make_node(3).await;
    let _tmp = tmp;
    let params = node.params.clone();
    node.floor_fallback_window = true;
    let base = build_chain(1, 1, Hash::ZERO, &p[0], 5, &params);
    apply_chain(&mut node, &base).await;
    let b = build_block(FORK_H, 108, base[4].hash(), leader(&p, 108), &params);
    apply_chain(&mut node, std::slice::from_ref(&b)).await;

    let a = build_block(FORK_H, 100, base[4].hash(), leader(&p, 100), &params);
    let mut canon = vec![a.clone()];
    for i in 1..=20u32 {
        let slot = if i == 1 { 103 } else { 110 + i };
        let prev = canon.last().unwrap().hash();
        canon.push(build_block(
            FORK_H + i as u64,
            slot,
            prev,
            leader(&p, slot),
            &params,
        ));
    }
    let peer = PeerId::random();
    let top = canon.last().unwrap();
    node.sync_manager
        .write()
        .await
        .add_peer(peer, FORK_H + 20, top.hash(), top.header.slot);
    for blk in &canon[1..] {
        node.handle_new_block(blk.clone(), peer).await.unwrap();
    }
    for i in 0..81u32 {
        let junk_parent = crypto::hash::hash(format!("w2_junk_{}", i).as_bytes());
        let slot = 1000 + i;
        let j = build_block(FORK_H + 1, slot, junk_parent, leader(&p, slot), &params);
        node.handle_new_block(j, peer).await.unwrap();
    }
    let len = node.fork_block_cache.read().await.len();
    assert!(
        len < 101,
        "fixture: eviction must have fired (cache len {})",
        len
    );
    assert!(
        node.fork_block_cache
            .read()
            .await
            .contains_key(&canon[1].hash()),
        "W2: the walk seed A1 (root of the longest cached orphan chain) was evicted"
    );
    node.try_trigger_fork_recovery().await;
    let sm = node.sync_manager.read().await;
    assert!(sm.is_fork_recovery_active(), "W2: a walk must start");
    assert_eq!(
        sm.fork_recovery_current_parent(),
        Some(a.hash()),
        "W2: after eviction the walk must still seed at A1 (first fetch = sibling A)"
    );
}

// ---------------------------------------------------------------------------
// REQ-I235-008 — walk connection is canonical-aware
// ---------------------------------------------------------------------------

async fn drive_walk(node: &mut Node, serve: &[Block], target: Hash, ticks: usize) {
    for _ in 0..ticks {
        node.floor_fallback_window = true;
        let req = node.sync_manager.write().await.next_request();
        if let Some((q, SyncRequest::GetBlockByHash { hash })) = req {
            let blk = serve.iter().find(|b| b.hash() == hash).cloned();
            let out = node
                .sync_manager
                .write()
                .await
                .handle_response(q, SyncResponse::Block(blk));
            for b in out {
                node.handle_new_block(b, q).await.unwrap();
            }
        }
        let _ = node.run_periodic_tasks().await;
        if node.chain_state.read().await.best_hash == target {
            break;
        }
    }
}

// REQ-I235-008 — Decision: a FAIL means a rolled-back fork-point sibling (INC-I-147
// D4: body kept, canonical index removed) counts as "stored", so its cached child
// is never a walk seed and the heavier competing branch is never reorged onto.
// Cell: O1/O2/O4 x C1.
#[tokio::test]
async fn m2_repro_canon_aware_walk_reorgs_past_noncanonical_fork_body() {
    let (mut node, p, tmp) = make_node(3).await;
    let _tmp = tmp;
    let params = node.params.clone();
    node.floor_fallback_window = true;
    let base = build_chain(1, 1, Hash::ZERO, &p[0], 5, &params);
    apply_chain(&mut node, &base).await;

    let a = build_block(FORK_H, 100, base[4].hash(), leader(&p, 100), &params);
    apply_chain(&mut node, std::slice::from_ref(&a)).await;
    let rb = node
        .rollback_one_block(doli_node::node::RollbackAuthority::CoordinatorApproved { depth: 1 })
        .await
        .unwrap();
    assert_eq!(rb, doli_node::node::RollbackOutcome::RolledBack);
    let b1 = build_block(FORK_H, 108, base[4].hash(), leader(&p, 108), &params);
    apply_chain(&mut node, std::slice::from_ref(&b1)).await;
    assert!(
        node.block_store.get_block(&a.hash()).unwrap().is_some()
            && node
                .block_store
                .get_height_by_hash(&a.hash())
                .unwrap()
                .is_none(),
        "fixture: A must be a stored but non-canonical body"
    );
    assert_eq!(tip(&node).await, (b1.hash(), FORK_H));

    let a1 = build_block(FORK_H + 1, 103, a.hash(), leader(&p, 103), &params);
    {
        let mut sm = node.sync_manager.write().await;
        for _ in 0..3 {
            sm.add_peer(PeerId::random(), FORK_H + 1, a1.hash(), a1.header.slot);
        }
    }
    cache(&node, &[&a1]).await;

    node.try_trigger_fork_recovery().await;
    let mut serve = vec![a.clone(), a1.clone()];
    serve.extend(base.iter().cloned());
    drive_walk(&mut node, &serve, a1.hash(), 12).await;

    assert_eq!(
        tip(&node).await,
        (a1.hash(), FORK_H + 1),
        "REQ-I235-008: [A, A1] is heavier than [B1]; the node must reorg onto A1"
    );
    for (h, blk) in [(FORK_H, &a), (FORK_H + 1, &a1)] {
        let at = node
            .block_store
            .get_block_by_height(h)
            .unwrap()
            .map(|b| b.hash());
        assert_eq!(at, Some(blk.hash()), "INV-SYNC-012: canonical at h={}", h);
    }
}
