//! INC-I-248 REL-M1 — shared fixture for the REL-M1 node tests.
//!
//! OUTPUT CONTRACT: N/A — fixture module. Its asserts are fixture preconditions only.
//! INPUT PARTITIONS: N/A — fixture module.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::inc_i_248_harness as h;
use crypto::PublicKey;
use doli_core::genesis::MAINNET_GENESIS_PRODUCERS;
use doli_core::maintainer::PeerMaintainerAnswer;
use doli_core::MaintainerSet;
use doli_node::node::Node;
use network::protocols::SyncResponse;
use network::PeerId;
use storage::MaintainerState;
use tempfile::TempDir;
use tokio::sync::{Notify, RwLock};

pub const MAINNET_RETIREMENT: u64 = 331_457;
pub const MAINNET_TIP: u64 = 480_001;
pub const S: u64 = h::SNAP_HEIGHT;
/// First startup tick (uptime >= 10 s).
pub const STARTUP: Duration = Duration::from_secs(11);
/// A tick at least one full window after an arm at `STARTUP` or earlier.
pub const WINDOW_END: Duration = Duration::from_secs(30);
pub const QUIET: Duration = Duration::from_millis(300);

pub fn retired_five() -> Vec<PublicKey> {
    MAINNET_GENESIS_PRODUCERS
        .iter()
        .map(|(hex, _)| PublicKey::from_hex(hex).expect("fixture: retired key hex"))
        .collect()
}

pub fn rotated_pubkeys() -> Vec<PublicKey> {
    h::rotated_keys().iter().map(|k| *k.public_key()).collect()
}

pub fn well_formed(members: Vec<PublicKey>, last_updated: u64) -> MaintainerSet {
    let set = MaintainerSet::with_members(members, last_updated);
    storage::validate_persisted_set(Path::new("fixture:set"), &set)
        .expect("fixture: set must be well formed");
    set
}

pub fn canonical(last_updated: u64) -> MaintainerSet {
    well_formed(rotated_pubkeys(), last_updated)
}

/// Three outsider keys (X1..X3); a forged answer carries it.
pub fn attacker(last_updated: u64) -> MaintainerSet {
    well_formed(
        h::OUTSIDER_SEEDS.iter().map(|s| h::pk(*s)).collect(),
        last_updated,
    )
}

pub fn retired_set(last_updated: u64) -> MaintainerSet {
    well_formed(retired_five(), last_updated)
}

pub fn empty_state(at: u64) -> MaintainerState {
    MaintainerState {
        set: MaintainerSet::new(),
        last_derived_height: at,
        ..MaintainerState::default()
    }
}

pub fn state(set: MaintainerSet, at: u64) -> MaintainerState {
    MaintainerState {
        set,
        last_derived_height: at,
        ..MaintainerState::default()
    }
}

/// A `MaintainerStateAt` frame for `set`, digest bound to the node's genesis.
pub fn frame(node: &Node, set: &MaintainerSet, tip: u64) -> SyncResponse {
    SyncResponse::MaintainerStateAt {
        block_hash: h::tip_hash(),
        block_height: tip,
        digest: crypto::Hash::from_bytes(h::node_digest(node, set)),
        set_bytes: bincode::serialize(set).expect("fixture: set must serialize"),
        last_derived_height: tip,
    }
}

pub fn answer(node: &Node, set: &MaintainerSet, tip: u64) -> PeerMaintainerAnswer {
    PeerMaintainerAnswer {
        set: set.clone(),
        last_derived_height: tip,
        digest: h::node_digest(node, set),
        tip_height: tip,
    }
}

/// Testnet harness node whose `maintainer_state.bin` holds `st` (loaded back from disk), tip `tip`.
pub async fn node_with_state(st: MaintainerState, tip: u64) -> (Node, TempDir) {
    let (mut node, temp) = h::testnet_node(&h::producer_keys()).await;
    st.save(&node.config.data_dir)
        .expect("fixture: maintainer state must persist");
    let loaded = MaintainerState::load(&node.config.data_dir).expect("fixture: state must load");
    node.set_maintainer_state(Arc::new(RwLock::new(loaded)));
    set_tip(&node, tip).await;
    (node, temp)
}

pub async fn set_tip(node: &Node, tip: u64) {
    let mut cs = node.chain_state.write().await;
    cs.best_height = tip;
    cs.best_hash = h::tip_hash();
}

pub async fn add_sync_peer(node: &Node, tip: u64) -> PeerId {
    let peer = PeerId::random();
    node.sync_manager
        .write()
        .await
        .add_peer(peer, tip, h::tip_hash(), tip as u32);
    peer
}

pub fn attach_notify(node: &mut Node) -> Arc<Notify> {
    let notify = Arc::new(Notify::new());
    node.set_update_recheck(notify.clone());
    notify
}

pub async fn fired(notify: &Notify, within: Duration) -> bool {
    tokio::time::timeout(within, notify.notified())
        .await
        .is_ok()
}

pub fn file_path(node: &Node) -> PathBuf {
    node.config.data_dir.join("maintainer_state.bin")
}

pub fn file_bytes(node: &Node) -> Option<Vec<u8>> {
    std::fs::read(file_path(node)).ok()
}

/// Memory AND disk hold `set` at `ldh`.
pub async fn assert_state(node: &Node, set: &MaintainerSet, ldh: u64, ctx: &str) {
    let mem = h::state_of(node).await;
    assert_eq!(mem.set, *set, "{ctx}: in-memory set");
    assert_eq!(
        mem.last_derived_height, ldh,
        "{ctx}: in-memory last_derived_height"
    );
    let disk = h::disk_state_of(node);
    assert_eq!(disk.set, *set, "{ctx}: disk set");
    assert_eq!(
        disk.last_derived_height, ldh,
        "{ctx}: disk last_derived_height"
    );
}

pub async fn assert_empty_at(node: &Node, ldh: u64, ctx: &str) {
    assert_state(node, &MaintainerSet::new(), ldh, ctx).await;
}

pub fn hex_sorted(keys: &[PublicKey]) -> Vec<String> {
    let mut v: Vec<String> = keys.iter().map(|k| k.to_hex()).collect();
    v.sort();
    v
}
