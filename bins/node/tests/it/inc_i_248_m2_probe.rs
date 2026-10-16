//! INC-I-248 M2 outcome-metric probe (not an assertion test).
//!
//! Driven by `.claude/scripts/inc-i-248-m2-probe.sh`; returns at once unless
//! `INC_I_248_M2_PROBE=1`. Prints one line and asserts nothing.
//!
//! OUTPUT CONTRACT: N/A — probe, prints one line, no assertion.
//! INPUT PARTITIONS: N/A — one fixed input (restart holding the retired five at H=480_001, 1 sync peer, 11 s uptime, 1 periodic tick, 3 canonical answers).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::inc_i_248_harness as h;
use crypto::PublicKey;
use doli_core::MaintainerSet;
use network::protocols::SyncResponse;
use storage::MaintainerState;
use tokio::sync::RwLock;

const TIP: u64 = 480_001;
const CANONICAL_LAST_UPDATED: u64 = 331_457;

fn retired_five() -> Vec<PublicKey> {
    updater::RETIRED_COMPROMISED_MAINTAINER_KEYS
        .iter()
        .map(|k| PublicKey::from_hex(k).expect("retired key must parse"))
        .collect()
}

#[tokio::test]
async fn inc_i_248_m2_probe_restart_root() {
    if std::env::var("INC_I_248_M2_PROBE").ok().as_deref() != Some("1") {
        println!("INC-I-248-M2-PROBE=skipped (run .claude/scripts/inc-i-248-m2-probe.sh)");
        return;
    }
    let (mut node, _temp) = h::testnet_node(&h::producer_keys()).await;

    let restart = MaintainerState {
        set: MaintainerSet::with_members(retired_five(), 0),
        last_derived_height: TIP,
        ..MaintainerState::default()
    };
    restart
        .save(&node.config.data_dir)
        .expect("retired-five state must persist");
    let loaded =
        MaintainerState::load(&node.config.data_dir).expect("retired-five state must load");
    node.set_maintainer_state(Arc::new(RwLock::new(loaded)));
    {
        let mut cs = node.chain_state.write().await;
        cs.best_height = TIP;
        cs.best_hash = h::tip_hash();
    }

    let canonical_keys: Vec<PublicKey> =
        h::rotated_keys().iter().map(|k| *k.public_key()).collect();
    let canonical = MaintainerSet::with_members(canonical_keys, CANONICAL_LAST_UPDATED);
    storage::validate_persisted_set(Path::new("probe:canonical"), &canonical)
        .expect("canonical set must be well formed");
    let canonical_digest = h::node_digest(&node, &canonical);

    node.sync_manager.write().await.add_peer(
        network::PeerId::random(),
        TIP,
        h::tip_hash(),
        TIP as u32,
    );
    tokio::time::sleep(Duration::from_secs(11)).await;
    let _ = node.run_periodic_tasks().await;

    let answer = SyncResponse::MaintainerStateAt {
        block_hash: h::tip_hash(),
        block_height: TIP,
        digest: crypto::Hash::from_bytes(canonical_digest),
        set_bytes: bincode::serialize(&canonical).expect("canonical set must serialize"),
        last_derived_height: TIP,
    };
    h::deliver(&mut node, &answer, 3).await;

    let state = h::state_of(&node).await;
    let class = if state.set.members.is_empty() {
        "empty"
    } else if h::sorted(&state.set.members) == h::sorted(&retired_five()) {
        "retired-five"
    } else if h::node_digest(&node, &state.set) == canonical_digest {
        "canonical"
    } else {
        "other"
    };
    let verdict = h::verdict_name(&h::verify_rotated_release(&node).await);
    println!("INC-I-248-M2 restart root={class} verify={verdict}");
}
