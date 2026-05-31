//! INC-I-103 Fix 2 (regression): cap stale-peer removal per cleanup cycle.
//!
//! Before fix: cleanup() iterates ALL peers exceeding stale_timeout (300s) and
//! removes them in a single uncapped pass. On 2026-05-30 this drained 22 sync
//! peers in 660 microseconds, leaving the node with zero sync peers and no
//! recovery path (periodic.rs:786 iterates sync.peer_ids() only — empty set,
//! no refresh).
//!
//! Required behavior: at most `max(3, peers.len()/3)` stale peers are removed
//! per cleanup cycle. Remaining stale peers stay in place until the next cycle,
//! giving the refresh loop time to update timestamps before the table drains.
//!
//! OUTPUT CONTRACT: fn cleanup(&mut self) -> ()
//!   O1: side-effect — number of peers removed from self.peers per call
//! PATHS:
//!   P1: many stale peers (> cap) -> O1 = max(3, peers.len()/3)
//!   P2: all peers fresh         -> O1 = 0
//!   P3: stale count <= cap      -> O1 = stale count (all removed)
//! INPUT PARTITIONS:
//!   IP1: 22 peers all stale (>300s)         -> P1: removes 7, retains 15
//!   IP2: 22 peers all fresh (<300s)         -> P2: removes 0
//!   IP3: 1 peer stale                       -> P3: removes 1 (cap > count)
//!   IP4: 3 peers stale (cap=3)              -> P3: removes 3 (cap == count)
//!   IP5: 9 peers stale (cap=max(3,3)=3)     -> P1: removes 3
//!   IP6: 30 peers stale (cap=max(3,10)=10)  -> P1: removes 10
//! MATRIX: 1 output (O1) x 6 partitions = 6 cells
//! .test_verified

use std::time::{Duration, Instant};

use crypto::Hash;
use libp2p::PeerId;

use crate::sync::manager::{SyncConfig, SyncManager, SyncPipelineData, SyncState};

/// Build a SyncManager whose cleanup() will only exercise the stale-peer
/// removal loop (no syncing, no decay, no stuck recovery).
fn quiet_manager() -> SyncManager {
    let mut mgr = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    mgr.state = SyncState::Idle;
    mgr.pipeline_data = SyncPipelineData::None;
    mgr
}

/// Insert `n` peers and force their `last_status_response` to `now - age_secs`
/// so they are stale relative to `config.stale_timeout` (default 300s).
fn add_n_peers_with_age(mgr: &mut SyncManager, n: usize, age_secs: u64) {
    let backdated = Instant::now()
        .checked_sub(Duration::from_secs(age_secs))
        .expect("test machine uptime must exceed backdate window");
    for _ in 0..n {
        let peer = PeerId::random();
        mgr.add_peer(peer, 0, Hash::ZERO, 0);
        // add_peer stamps last_status_response = Instant::now(); override it.
        if let Some(status) = mgr.peers.get_mut(&peer) {
            status.last_status_response = backdated;
        }
    }
}

/// IP1 (FAIL→PASS): 22 peers all stale. Pre-fix: cleanup() removes all 22 in
/// a single pass. Post-fix: cleanup() removes max(3, 22/3) = 7 per cycle.
#[test]
fn test_inc_i103_cap_stale_removal_22_peers() {
    let mut mgr = quiet_manager();
    add_n_peers_with_age(&mut mgr, 22, 600);
    assert_eq!(mgr.peers.len(), 22, "precondition: 22 peers present");

    mgr.cleanup();
    assert_eq!(
        mgr.peers.len(),
        15,
        "INC-I-103: first cleanup must cap removals at max(3, 22/3)=7, leaving 15 (got {})",
        mgr.peers.len()
    );

    mgr.cleanup();
    assert_eq!(
        mgr.peers.len(),
        10,
        "INC-I-103: second cleanup must cap removals at max(3, 15/3)=5, leaving 10 (got {})",
        mgr.peers.len()
    );
}

/// IP2: 22 peers all fresh. Cap path is irrelevant — no removals expected.
#[test]
fn test_inc_i103_no_removal_when_fresh() {
    let mut mgr = quiet_manager();
    add_n_peers_with_age(&mut mgr, 22, 10); // 10s << 300s threshold
    assert_eq!(mgr.peers.len(), 22);

    mgr.cleanup();
    assert_eq!(
        mgr.peers.len(),
        22,
        "Fresh peers must not be removed — cap is irrelevant when nothing is stale"
    );
}

/// IP3: a single stale peer is removed. Decision (per session prompt): fleets
/// at or below the cap floor get no special protection — the cap only limits
/// per-cycle drainage, it does not protect against legitimate single-peer
/// cleanup. Tested explicitly so the behavior is locked in.
#[test]
fn test_inc_i103_single_stale_peer_removed() {
    let mut mgr = quiet_manager();
    add_n_peers_with_age(&mut mgr, 1, 600);
    mgr.cleanup();
    assert_eq!(
        mgr.peers.len(),
        0,
        "Single stale peer: removed (cap=max(3,0)=3 >= 1 stale)"
    );
}

/// IP4 boundary: peers.len()=3, all stale, cap=max(3, 1)=3 → all removed.
#[test]
fn test_inc_i103_cap_boundary_3_peers() {
    let mut mgr = quiet_manager();
    add_n_peers_with_age(&mut mgr, 3, 600);
    mgr.cleanup();
    assert_eq!(mgr.peers.len(), 0, "3 stale peers, cap=3: all removed");
}

/// IP5 boundary: peers.len()=9, all stale, cap=max(3, 9/3)=3 → 3 removed.
#[test]
fn test_inc_i103_cap_boundary_9_peers() {
    let mut mgr = quiet_manager();
    add_n_peers_with_age(&mut mgr, 9, 600);
    mgr.cleanup();
    assert_eq!(
        mgr.peers.len(),
        6,
        "9 stale peers, cap=max(3, 3)=3: 3 removed, 6 remain (got {})",
        mgr.peers.len()
    );
}

/// IP6 boundary: peers.len()=30, all stale, cap=max(3, 30/3)=10 → 10 removed.
#[test]
fn test_inc_i103_cap_boundary_30_peers() {
    let mut mgr = quiet_manager();
    add_n_peers_with_age(&mut mgr, 30, 600);
    mgr.cleanup();
    assert_eq!(
        mgr.peers.len(),
        20,
        "30 stale peers, cap=max(3, 10)=10: 10 removed, 20 remain (got {})",
        mgr.peers.len()
    );
}
