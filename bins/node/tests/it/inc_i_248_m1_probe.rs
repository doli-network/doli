//! INC-I-248 M1 outcome-metric probe (not an assertion test).
//!
//! Driven by `.claude/scripts/inc-i-248-m1-probe.sh`; returns at once unless
//! `INC_I_248_M1_PROBE=1`. Prints one line and asserts nothing.
//!
//! OUTPUT CONTRACT: N/A — probe, prints one line, no assertion.
//! INPUT PARTITIONS: N/A — one fixed input (A rotated to S1..S5, B snap-joined at H, 3 answers).

use crate::inc_i_248_harness as h;
use serde::Serialize;

#[derive(Serialize)]
#[allow(dead_code)]
enum SyncResponseMirror {
    V0,
    V1,
    V2,
    V3,
    V4,
    V5,
    V6,
    V7,
    V8,
    MaintainerStateAt {
        block_hash: crypto::Hash,
        block_height: u64,
        digest: crypto::Hash,
        set_bytes: Vec<u8>,
        last_derived_height: u64,
    },
}

#[tokio::test]
async fn inc_i_248_m1_probe_snap_join_root() {
    if std::env::var("INC_I_248_M1_PROBE").ok().as_deref() != Some("1") {
        println!("INC-I-248-M1-PROBE=skipped (run .claude/scripts/inc-i-248-m1-probe.sh)");
        return;
    }
    let a = h::build_replay_node_a().await;
    let view = h::peer_view(&a.node).await;
    let snapshot = h::snapshot_of(&a.node).await;
    let (mut node_b, _temp_b) = h::snap_joined_node_b(snapshot).await;

    let wire = bincode::serialize(&SyncResponseMirror::MaintainerStateAt {
        block_hash: view.block_hash,
        block_height: view.block_height,
        digest: crypto::Hash::from_bytes(view.digest),
        set_bytes: view.set_bytes.clone(),
        last_derived_height: view.last_derived_height,
    })
    .expect("mirror must serialize");

    for _ in 0..3 {
        if let Ok(response) = bincode::deserialize::<network::protocols::SyncResponse>(&wire) {
            let event = network::NetworkEvent::SyncResponse {
                peer_id: network::PeerId::random(),
                response,
            };
            let _ = node_b.handle_network_event(event).await;
        }
    }

    node_b
        .maybe_bootstrap_maintainer_set(h::SNAP_HEIGHT + 1)
        .await;

    let state = h::state_of(&node_b).await;
    let class = if state.set.members.is_empty() {
        "empty"
    } else if h::node_digest(&node_b, &state.set) == view.digest {
        "canonical"
    } else if h::is_producer_seed(&state.set) {
        "producer-seed"
    } else {
        "other"
    };
    let verdict = h::verdict_name(&h::verify_rotated_release(&node_b).await);
    println!("INC-I-248-M1 snap-join root={class} verify={verdict}");
}
