//! INC-I-288 F-2c fixtures on top of the F-2 rig: a child re-exec at a chosen
//! INC-I-288 AH, production-weight caching, the recorded weight of one block, and
//! a fork-walk driver (by-hash fetch, response, periodic-style poll) that does not
//! run the coordinator.
//!
//! OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.
#![allow(dead_code)]

use crypto::Hash;
use doli_core::{Block, Network, NetworkParams};
use network::protocols::{SyncRequest, SyncResponse};
use network::PeerId;

pub use super::inc_i_288_f2_common::*;

const AH_CHILD_ENV: &str = "INC_I_288_F2C_AH_CHILD";

/// Parent: re-exec `name` alone with the INC-I-288 AH set to `ah`; returns true
/// in the child. `NetworkParams` is cached per process.
pub fn in_ah_child(name: &str, ah: u64) -> bool {
    if std::env::var_os(AH_CHILD_ENV).is_some() {
        assert_eq!(
            NetworkParams::load(Network::Devnet).inc_i_288_fork_choice_activation_height,
            ah,
            "child must pick up the AH env override"
        );
        return true;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--test-threads=1", "--nocapture"])
        .env(AH_CHILD_ENV, "1")
        .env(AH_ENV, ah.to_string())
        .output()
        .expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "AH={ah} child failed or did not run:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    false
}

/// `b`'s producer weight at `height`, as `ProducerInfo::effective_weight` gives it.
pub async fn production_weight(r: &Rig, b: &Block, height: u64) -> u64 {
    r.node
        .producer_set
        .read()
        .await
        .get_by_pubkey(&b.header.producer)
        .map(|p| p.effective_weight(height))
        .expect("fixture: the block's producer is registered")
}

/// A block as fork choice holds it after a walk or a door: body cached, weight
/// recorded at the PRODUCTION weight. Returns the weight.
pub async fn cache_at_production_weight(r: &Rig, b: &Block, height: u64) -> u64 {
    let w = production_weight(r, b, height).await;
    r.node
        .fork_block_cache
        .write()
        .await
        .insert(b.hash(), b.clone());
    r.node.sync_manager.write().await.record_fork_block_weight(
        b.hash(),
        b.header.prev_hash,
        w,
        height,
    );
    w
}

/// The producer weight the reorg handler recorded for `h` (None: not recorded).
pub async fn recorded_weight(r: &Rig, h: &Hash) -> Option<u64> {
    r.node
        .sync_manager
        .read()
        .await
        .reorg_handler()
        .get_block_weight(h)
        .map(|w| w.producer_weight)
}

/// `n` blocks by one leader X (not the node's key), all at X's first slot at or
/// after `from`, chained on `prev` from `height`.
pub fn same_slot_burst(r: &Rig, from: u32, prev: Hash, height: u64, n: u64) -> Vec<Block> {
    let own = *r.keys[0].public_key();
    let slot = (from..from + 30)
        .find(|s| *leader(&r.keys, *s).public_key() != own)
        .expect("fixture: a slot led by another producer");
    let x = leader(&r.keys, slot).clone();
    let mut prev = prev;
    (0..n)
        .map(|i| {
            let b = block(height + i, slot, prev, &x, &r.node.params);
            prev = b.hash();
            b
        })
        .collect()
}

/// A chain on `prev` from `height`, one block per slot in `slots`, each by its
/// slot's scheduled leader.
pub fn chain_on(r: &Rig, prev: Hash, height: u64, slots: &[u32]) -> Vec<Block> {
    let mut prev = prev;
    slots
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let b = block(
                height + i as u64,
                *s,
                prev,
                leader(&r.keys, *s),
                &r.node.params,
            );
            prev = b.hash();
            b
        })
        .collect()
}

/// True when `h` has a canonical height-index entry (the node's `is_canonical`).
pub fn canonical(r: &Rig, h: &Hash) -> bool {
    matches!(r.node.block_store.get_height_by_hash(h), Ok(Some(_)))
}

/// Drives an active fork walk for up to `ticks` rounds: the by-hash fetch is
/// answered from `serve`, then the node's walk poll runs (periodic.rs: connect
/// at a canonical parent -> `handle_completed_fork_recovery`). Returns every
/// hash the walk fetched.
pub async fn drive_walk(r: &mut Rig, serve: &[Block], ticks: usize) -> Vec<Hash> {
    let mut fetched = Vec::new();
    for _ in 0..ticks {
        let req = r.node.sync_manager.write().await.next_request();
        if let Some((q, SyncRequest::GetBlockByHash { hash })) = req {
            fetched.push(hash);
            let blk = serve.iter().find(|b| b.hash() == hash).cloned();
            let out = r
                .node
                .sync_manager
                .write()
                .await
                .handle_response(q, SyncResponse::Block(blk));
            for b in out {
                r.node
                    .handle_new_block(b, q)
                    .await
                    .expect("handle_new_block returns Ok");
            }
        }
        let parent = r
            .node
            .sync_manager
            .read()
            .await
            .fork_recovery_current_parent();
        if parent.is_some_and(|p| canonical(r, &p)) {
            let done = r
                .node
                .sync_manager
                .write()
                .await
                .check_fork_recovery_connection(true);
            if let Some(done) = done {
                let res = r.node.handle_completed_fork_recovery(done).await;
                eprintln!("[F-2c] completed walk -> {res:?}");
            }
        }
        if !r.node.sync_manager.read().await.is_fork_recovery_active() {
            break;
        }
    }
    fetched
}

/// A peer for the walk (`best_peer_for_recovery`), its status naming `tip`.
pub async fn walk_peer(r: &Rig, height: u64, tip: &Block) -> PeerId {
    let peer = PeerId::random();
    r.node
        .sync_manager
        .write()
        .await
        .add_peer(peer, height, tip.hash(), tip.header.slot);
    peer
}
