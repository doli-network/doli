//! INC-I-288 F-2 fixtures on top of the C-1 rig: the live fork-choice rig (every
//! gate from `Network::Devnet.params()`, M5 live), a restart of the node's
//! SyncManager, the below-AH child re-exec, and the "applied once" observable.
//!
//! OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.
#![allow(dead_code)]

use crypto::Hash;
use doli_core::{Block, Network, NetworkParams};

pub use super::inc_i_288_c1_common::*;

pub const AH_ENV: &str = "DOLI_INC_I_288_FORK_CHOICE_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "INC_I_288_F2_BELOW_AH_CHILD";
pub const BELOW_AH: u64 = 1_000_000_000;

/// The C-1 rig on the live SyncConfig (M5 live, as on testnet above 88_014).
pub async fn live_rig(our_slots: &[u32], their_slots: &[u32]) -> Rig {
    rig_on(our_slots, their_slots, Some(sync_config(true))).await
}

/// [`live_rig`] on fixed producer `keys` (every block hash is then fixed).
pub async fn live_rig_keys(
    keys: Vec<crypto::KeyPair>,
    our_slots: &[u32],
    their_slots: &[u32],
) -> Rig {
    rig_on_keys(keys, our_slots, their_slots, Some(sync_config(true))).await
}

/// A process restart as `Node::new` does it (init.rs: a new SyncManager, then
/// `update_local_tip` and the post-restart lockout only). Fork cache and the
/// retained set are in-memory, so a new process starts with them empty.
pub async fn restart(r: &mut Rig) {
    let (h, hash, slot) = {
        let cs = r.node.chain_state.read().await;
        (cs.best_height, cs.best_hash, cs.best_slot)
    };
    let mut sm = fresh_sync_manager(sync_config(true));
    sm.update_local_tip(h, hash, slot);
    sm.engage_post_restart_lockout();
    *r.node.sync_manager.write().await = sm;
    r.node.fork_block_cache.write().await.clear();
    r.node.wedge_retained_tips.clear();
}

pub async fn knows(r: &Rig, h: &Hash) -> bool {
    r.node
        .sync_manager
        .read()
        .await
        .reorg_handler()
        .knows_block(h)
}

/// The block store holds `b` (it was applied at least once, even if rolled back).
pub fn stored(r: &Rig, b: &Block) -> bool {
    r.node.block_store.has_block(&b.hash()).unwrap()
}

/// Parent: re-exec `name` alone with the INC-I-288 AH far above every test
/// height; returns true in the child. `NetworkParams` is cached per process.
pub fn in_below_ah_child(name: &str) -> bool {
    if std::env::var_os(CHILD_ENV).is_some() {
        assert_eq!(
            NetworkParams::load(Network::Devnet).inc_i_288_fork_choice_activation_height,
            BELOW_AH,
            "child must pick up the AH env override"
        );
        return true;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1")
        .env(AH_ENV, BELOW_AH.to_string())
        .output()
        .expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "below-AH child failed or did not run:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    false
}
