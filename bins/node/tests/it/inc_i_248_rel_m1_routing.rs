//! INC-I-248 REL-M1 — replies to the node's own GetMaintainerState never reach the sync
//! manager, and the serve side never answers GetMaintainerState with a busy error.
//!
// covers: bins/node/src/node/maintainer_quorum.rs, bins/node/src/node/maintainer_probe_routing.rs, bins/node/src/node/network_events.rs, bins/node/src/node/maintainer_state_serve.rs, bins/node/src/node/state_session_serve.rs, bins/node/src/node/mod.rs, bins/node/src/node/init.rs, bins/node/src/node/event_loop.rs, crates/core/src/maintainer/quorum.rs, crates/core/src/maintainer/mod.rs
//!
//! REQ-248-007 (Must) — Decision: a failure means a busy or error reply to the maintainer probe aborts an in-progress snap sync or header sync (SEC-LOGIC-001 routing, SEC-CONFIG-001), or a serving node refuses the probe as busy.
//!
// OUTPUT CONTRACT: Node::handle_network_event(NetworkEvent::SyncResponse { peer_id, SyncResponse::Error })
//   O1: none; O3: Result (Ok)
//   O2: sync_manager.state() / sync_state_name() / is_snap_syncing(); maintainer_state; maintainer_probe_phase()
//   O4/O5/O6: none
// PATHS: R1 Error from a peer the probe asked -> sync manager unchanged, probe still collecting;
//   R2 Error from a peer the probe did not ask -> reaches the sync manager (control).
// INPUT PARTITIONS: sync state = Syncing:SnapCollecting (3 peers ahead); reply = "busy: ..." Error.
// OUTPUT CONTRACT: Node::admit_sync_request(&mut self, &SyncRequest::GetMaintainerState) -> Option<SyncResponse>
//   O2: sync_requests_this_interval; O3: None (served) | Some(Error busy)
// PATHS: A1 counter at MAX -> None, counter unchanged; A2 control GetHeaders at MAX -> Some(busy)
// MATRIX: R1/R2 x (state name, snap flag, maintainer set, phase) + A1/A2 x (return, counter) = 12 cells.

use crate::inc_i_248_harness as h;
use crate::inc_i_248_rel_m1_fixture as fx;
use doli_node::node::maintainer_quorum::ProbeReason;
use doli_node::node::MAX_SYNC_REQUESTS_PER_INTERVAL;
use network::protocols::{SyncRequest, SyncResponse};
use network::{NetworkEvent, PeerId};

const AHEAD: u64 = fx::S + 1_000;

fn busy() -> SyncResponse {
    SyncResponse::Error("busy: sync serving limit reached".to_string())
}

async fn reply(node: &mut doli_node::node::Node, peer: PeerId, response: SyncResponse) {
    let event = NetworkEvent::SyncResponse {
        peer_id: peer,
        response,
    };
    node.handle_network_event(event)
        .await
        .expect("handle_network_event must not fail");
}

async fn sync_snapshot(node: &doli_node::node::Node) -> (String, bool) {
    let sm = node.sync_manager.read().await;
    (format!("{:?}", sm.sync_state_name()), sm.is_snap_syncing())
}

// REQ-248-007 — Decision: a failure means one busy reply to the maintainer probe drops the node out of snap sync.
#[tokio::test]
async fn inc_i_248_rel_m1_probe_error_reply_never_reaches_sync_manager() {
    let (mut node, _t) = fx::node_with_state(fx::empty_state(fx::S), fx::S).await;
    let asked: Vec<PeerId> = {
        let mut v = Vec::new();
        for _ in 0..3 {
            v.push(fx::add_sync_peer(&node, AHEAD).await);
        }
        v
    };
    let before = sync_snapshot(&node).await;
    assert!(
        before.1,
        "fixture: 3 peers ahead start snap sync, got {}",
        before.0
    );

    node.arm_maintainer_quorum(ProbeReason::PostSnapInstall)
        .await;
    assert_eq!(node.maintainer_probe_phase(), "collecting");
    reply(&mut node, asked[0], busy()).await;
    reply(
        &mut node,
        asked[1],
        SyncResponse::Error("other".to_string()),
    )
    .await;

    assert_eq!(
        sync_snapshot(&node).await,
        before,
        "a probe reply reached the sync manager"
    );
    assert_eq!(node.maintainer_probe_phase(), "collecting");
    fx::assert_empty_at(&node, fx::S, "an error reply is no answer").await;

    let stranger = fx::add_sync_peer(&node, AHEAD).await;
    reply(&mut node, stranger, busy()).await;
    let after = sync_snapshot(&node).await;
    assert!(
        !after.1,
        "control: an error from a peer the probe never asked still reaches the sync manager, got {}",
        after.0
    );
}

// REQ-248-007 — Decision: a failure means a loaded peer answers the maintainer probe with busy, so a snap node can never form its quorum.
#[tokio::test]
async fn inc_i_248_rel_m1_serve_get_maintainer_state_never_busy() {
    let (mut node, _t) = h::testnet_node(&h::producer_keys()).await;
    node.sync_requests_this_interval = MAX_SYNC_REQUESTS_PER_INTERVAL;

    assert!(
        node.admit_sync_request(&SyncRequest::GetMaintainerState)
            .is_none(),
        "GetMaintainerState must be served at the limit"
    );
    assert_eq!(
        node.sync_requests_this_interval,
        MAX_SYNC_REQUESTS_PER_INTERVAL
    );

    let control = node.admit_sync_request(&SyncRequest::GetHeaders {
        start_hash: h::tip_hash(),
        max_count: 1,
    });
    assert!(
        matches!(control, Some(SyncResponse::Error(ref e)) if e.contains("busy")),
        "control: other requests still get busy at the limit"
    );
}
