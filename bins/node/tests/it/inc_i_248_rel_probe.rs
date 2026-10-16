//! INC-I-248 REL-M1 outcome-metric probe (not an assertion test).
//!
//! Driven by `.claude/scripts/inc-i-248-rel-probe.sh`; returns at once unless
//! `INC_I_248_REL_PROBE=1`. Prints one line and asserts nothing.
//!
//! OUTPUT CONTRACT: N/A — probe, prints one line, no assertion.
//! INPUT PARTITIONS: N/A — one fixed input (healthy replay node, usable canonical
//! root, tip 30_000, 1 sync peer, startup tick at +11 s, 5 colluding answers with
//! tip 31_000 and forged last_updated 1, window-end tick at +27 s).

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::inc_i_248_harness as h;
use crypto::PublicKey;
use doli_core::MaintainerSet;
use network::protocols::SyncResponse;
use storage::MaintainerState;
use tokio::sync::RwLock;

const TIP: u64 = 30_000;
const COLLUDER_TIP: u64 = 31_000;
const FORGED_LAST_UPDATED: u64 = 1;

#[tokio::test]
async fn inc_i_248_rel_probe_healthy_replay_colluding() {
    if std::env::var("INC_I_248_REL_PROBE").ok().as_deref() != Some("1") {
        println!("INC-I-248-REL-PROBE=skipped (run .claude/scripts/inc-i-248-rel-probe.sh)");
        return;
    }
    let (mut node, _temp) = h::testnet_node(&h::producer_keys()).await;

    let canonical_keys: Vec<PublicKey> =
        h::rotated_keys().iter().map(|k| *k.public_key()).collect();
    let canonical = MaintainerSet::with_members(canonical_keys, h::last_rotation_height());
    let healthy = MaintainerState {
        set: canonical.clone(),
        last_derived_height: TIP,
        ..MaintainerState::default()
    };
    healthy
        .save(&node.config.data_dir)
        .expect("canonical state must persist");
    let loaded = MaintainerState::load(&node.config.data_dir).expect("canonical state must load");
    node.set_maintainer_state(Arc::new(RwLock::new(loaded)));
    {
        let mut cs = node.chain_state.write().await;
        cs.best_height = TIP;
        cs.best_hash = h::tip_hash();
    }
    let canonical_digest = h::node_digest(&node, &canonical);

    let attacker_keys: Vec<PublicKey> = h::OUTSIDER_SEEDS.iter().map(|s| h::pk(*s)).collect();
    let attacker = MaintainerSet::with_members(attacker_keys, FORGED_LAST_UPDATED);
    storage::validate_persisted_set(Path::new("probe:attacker"), &attacker)
        .expect("attacker set must be well formed");
    let attacker_digest = h::node_digest(&node, &attacker);

    node.sync_manager.write().await.add_peer(
        network::PeerId::random(),
        TIP,
        h::tip_hash(),
        TIP as u32,
    );
    let t0 = Instant::now();
    node.maintainer_quorum_tick_at(t0 + Duration::from_secs(11))
        .await;

    let forged = SyncResponse::MaintainerStateAt {
        block_hash: h::tip_hash(),
        block_height: COLLUDER_TIP,
        digest: crypto::Hash::from_bytes(attacker_digest),
        set_bytes: bincode::serialize(&attacker).expect("attacker set must serialize"),
        last_derived_height: COLLUDER_TIP,
    };
    h::deliver(&mut node, &forged, 5).await;
    node.maintainer_quorum_tick_at(t0 + Duration::from_secs(27))
        .await;

    let state = h::state_of(&node).await;
    let digest = h::node_digest(&node, &state.set);
    let verdict = if state.set.members.is_empty() {
        "Adopt(empty)"
    } else if digest == attacker_digest {
        "Adopt(attacker)"
    } else if digest == canonical_digest {
        "Keep(canonical)"
    } else {
        "other"
    };
    println!("INC-I-248-REL healthy-replay colluding=5 verdict={verdict}");
}
