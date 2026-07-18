//! INC-I-136 M2: Checkpoint health decision tests (TDD -- tests BEFORE implementation).
//!
//! These tests verify the pure `decide_checkpoint_health()` function that
//! determines whether a checkpoint should be tagged as `healthy`. The function
//! is peer-INDEPENDENT: an isolated node with self-consistent state CAN produce
//! a healthy checkpoint (F2). A node with body gaps or missing undo data can
//! NEVER produce a healthy checkpoint (F1/F4).

// ============================================================
// OUTPUT CONTRACT: fn decide_checkpoint_health(
//     self_consistent: bool, peer_count: usize, peers_agreeing: usize,
//     unique_hashes: usize, window_healthy: bool,
// ) -> CheckpointHealthDecision
//
// Outputs:
//   O1: return.healthy   -- bool: whether the checkpoint should be tagged healthy
//   O2: return.isolated  -- bool: whether the node is isolated (peer_count==0)
//   O3: return.self_consistent -- bool: echo of the input self_consistent flag
//
// Paths:
//   P1: self_consistent == false  -> healthy=false ALWAYS
//   P2: self_consistent == true AND peer_count == 0 -> healthy=true, isolated=true (F2)
//   P3: self_consistent == true AND point-healthy -> healthy=true, isolated=false
//   P4: self_consistent == true AND peers disagree AND window_healthy==false -> healthy=false
//   P5: self_consistent == true AND window_healthy==true (regardless of peer disagreement) -> healthy=true
//
// INPUT PARTITIONS:
//   P1a: self_consistent=false, peer_count=0 (isolated + inconsistent)
//   P1b: self_consistent=false, peer_count=3, peers_agreeing=3, unique_hashes=1 (peers agree but state bad)
//   P1c: self_consistent=false, window_healthy=true (window says ok but state bad)
//   P2a: self_consistent=true, peer_count=0, window_healthy=false (isolated, no history)
//   P2b: self_consistent=true, peer_count=0, window_healthy=true (isolated, has history)
//   P3a: self_consistent=true, peer_count=5, peers_agreeing=5, unique_hashes=1 (full agreement)
//   P3b: self_consistent=true, peer_count=1, peers_agreeing=1, unique_hashes=1 (minimal agreement)
//   P4a: self_consistent=true, peer_count=3, peers_agreeing=1, unique_hashes=2, window=false (split)
//   P4b: self_consistent=true, peer_count=3, peers_agreeing=3, unique_hashes=2, window=false (agree but multi-tip)
//   P5a: self_consistent=true, peer_count=3, peers_agreeing=1, unique_hashes=2, window=true (recent healthy)
//
// MATRIX: 3 outputs x 10 partitions = 30 cells
//   P1a: O1(false) O2(true)  O3(false)
//   P1b: O1(false) O2(false) O3(false)
//   P1c: O1(false) O2(false) O3(false)
//   P2a: O1(true)  O2(true)  O3(true)
//   P2b: O1(true)  O2(true)  O3(true)
//   P3a: O1(true)  O2(false) O3(true)
//   P3b: O1(true)  O2(false) O3(true)
//   P4a: O1(false) O2(false) O3(true)
//   P4b: O1(false) O2(false) O3(true)
//   P5a: O1(true)  O2(false) O3(true)
// ============================================================

use doli_node::node::checkpoint_health::decide_checkpoint_health;

// ============================================================
// P1: self_consistent == false -> healthy=false ALWAYS
// Requirement: REQ-GUARD-002 (Must), REQ-GUARD-003 (Must)
// Acceptance: A forked/gappy node cannot produce a healthy checkpoint
// ============================================================

#[test]
fn test_p1a_inconsistent_isolated_is_unhealthy() {
    // P1a: self_consistent=false, peer_count=0
    // An isolated node with bad state must NEVER be tagged healthy.
    let d = decide_checkpoint_health(
        false, // self_consistent
        0,     // peer_count
        0,     // peers_agreeing
        0,     // unique_hashes
        false, // window_healthy
    );
    assert!(!d.healthy, "P1a O1: inconsistent state must be unhealthy");
    assert!(d.isolated, "P1a O2: peer_count==0 means isolated");
    assert!(
        !d.self_consistent,
        "P1a O3: self_consistent must echo the input"
    );
}

#[test]
fn test_p1b_inconsistent_with_agreeing_peers_is_unhealthy() {
    // P1b: self_consistent=false, peer_count=3, peers_agreeing=3, unique_hashes=1
    // Even when ALL peers agree, if the local state is inconsistent,
    // the checkpoint is not healthy. State trumps peer agreement.
    let d = decide_checkpoint_health(
        false, // self_consistent
        3,     // peer_count
        3,     // peers_agreeing
        1,     // unique_hashes
        false, // window_healthy
    );
    assert!(
        !d.healthy,
        "P1b O1: inconsistent state must be unhealthy even with peer agreement"
    );
    assert!(!d.isolated, "P1b O2: peer_count>0 means not isolated");
    assert!(
        !d.self_consistent,
        "P1b O3: self_consistent must echo the input"
    );
}

#[test]
fn test_p1c_inconsistent_with_window_healthy_is_unhealthy() {
    // P1c: self_consistent=false, window_healthy=true
    // Even when the rolling window says "was recently healthy",
    // bad state means the checkpoint is unhealthy. Self-consistency
    // is a hard prerequisite.
    let d = decide_checkpoint_health(
        false, // self_consistent
        2,     // peer_count
        2,     // peers_agreeing
        1,     // unique_hashes
        true,  // window_healthy
    );
    assert!(
        !d.healthy,
        "P1c O1: inconsistent state must override window_healthy"
    );
    assert!(!d.isolated, "P1c O2: peer_count>0 means not isolated");
    assert!(
        !d.self_consistent,
        "P1c O3: self_consistent must echo the input"
    );
}

// ============================================================
// P2: self_consistent == true AND peer_count == 0 -> healthy, isolated
// Requirement: REQ-GUARD-002 (Must) -- F2: isolated-but-consistent
// Acceptance: healthy=true achievable with peers=0 when state is self-consistent
// ============================================================

#[test]
fn test_p2a_consistent_isolated_no_window_is_healthy() {
    // P2a: self_consistent=true, peer_count=0, window_healthy=false
    // The core F2 fix: an isolated node with self-consistent state
    // MUST be tagged healthy (with isolated=true).
    let d = decide_checkpoint_health(
        true,  // self_consistent
        0,     // peer_count
        0,     // peers_agreeing
        0,     // unique_hashes
        false, // window_healthy
    );
    assert!(
        d.healthy,
        "P2a O1: isolated-but-consistent node MUST be tagged healthy (F2)"
    );
    assert!(
        d.isolated,
        "P2a O2: peer_count==0 must report isolated=true"
    );
    assert!(
        d.self_consistent,
        "P2a O3: self_consistent must echo the input"
    );
}

#[test]
fn test_p2b_consistent_isolated_with_window_is_healthy() {
    // P2b: self_consistent=true, peer_count=0, window_healthy=true
    // Isolation with both self-consistency AND recent healthy window.
    let d = decide_checkpoint_health(
        true, // self_consistent
        0,    // peer_count
        0,    // peers_agreeing
        0,    // unique_hashes
        true, // window_healthy
    );
    assert!(
        d.healthy,
        "P2b O1: isolated-but-consistent with window must be healthy"
    );
    assert!(
        d.isolated,
        "P2b O2: peer_count==0 must report isolated=true"
    );
    assert!(
        d.self_consistent,
        "P2b O3: self_consistent must echo the input"
    );
}

// ============================================================
// P3: self_consistent == true AND point-healthy -> healthy, not isolated
// Requirement: REQ-GUARD-002 (Must)
// Acceptance: Normal operation with peer agreement
// ============================================================

#[test]
fn test_p3a_consistent_full_peer_agreement() {
    // P3a: self_consistent=true, peer_count=5, peers_agreeing=5, unique_hashes=1
    let d = decide_checkpoint_health(
        true,  // self_consistent
        5,     // peer_count
        5,     // peers_agreeing
        1,     // unique_hashes
        false, // window_healthy (irrelevant -- point-healthy suffices)
    );
    assert!(
        d.healthy,
        "P3a O1: consistent + full peer agreement = healthy"
    );
    assert!(!d.isolated, "P3a O2: peer_count>0 means not isolated");
    assert!(
        d.self_consistent,
        "P3a O3: self_consistent must echo the input"
    );
}

#[test]
fn test_p3b_consistent_minimal_peer_agreement() {
    // P3b: self_consistent=true, peer_count=1, peers_agreeing=1, unique_hashes=1
    let d = decide_checkpoint_health(
        true,  // self_consistent
        1,     // peer_count
        1,     // peers_agreeing
        1,     // unique_hashes
        false, // window_healthy
    );
    assert!(
        d.healthy,
        "P3b O1: consistent + single peer agreement = healthy"
    );
    assert!(!d.isolated, "P3b O2: not isolated");
    assert!(d.self_consistent, "P3b O3: self_consistent echoes");
}

// ============================================================
// P4: self_consistent == true AND peers disagree AND window==false -> unhealthy
// Requirement: REQ-GUARD-002 (Must)
// Acceptance: Possible minority fork
// ============================================================

#[test]
fn test_p4a_consistent_peers_split_no_window() {
    // P4a: self_consistent=true, peer_count=3, peers_agreeing=1, unique_hashes=2, window=false
    // Peers are split -- possible minority fork. Without window, unhealthy.
    let d = decide_checkpoint_health(
        true,  // self_consistent
        3,     // peer_count
        1,     // peers_agreeing
        2,     // unique_hashes
        false, // window_healthy
    );
    assert!(
        !d.healthy,
        "P4a O1: consistent but peers disagree + no window = unhealthy"
    );
    assert!(!d.isolated, "P4a O2: not isolated");
    assert!(d.self_consistent, "P4a O3: self_consistent echoes");
}

#[test]
fn test_p4b_consistent_multi_tip_no_window() {
    // P4b: self_consistent=true, peer_count=3, peers_agreeing=3, unique_hashes=2, window=false
    // All peers agree with us but we see 2 distinct tips in the network.
    // This means we are on one fork and some other peers are on another.
    let d = decide_checkpoint_health(
        true,  // self_consistent
        3,     // peer_count
        3,     // peers_agreeing
        2,     // unique_hashes
        false, // window_healthy
    );
    assert!(
        !d.healthy,
        "P4b O1: multi-tip network + no window = unhealthy (possible fork)"
    );
    assert!(!d.isolated, "P4b O2: not isolated");
    assert!(d.self_consistent, "P4b O3: self_consistent echoes");
}

// ============================================================
// P5: self_consistent == true AND window_healthy == true -> healthy
// Requirement: REQ-GUARD-002 (Must)
// Acceptance: Rolling health window overrides transient peer disagreement
// ============================================================

#[test]
fn test_p5a_consistent_peers_disagree_but_window_healthy() {
    // P5a: self_consistent=true, peer_count=3, peers_agreeing=1, unique_hashes=2, window=true
    // Peers are currently disagreeing, but the rolling window shows recent health.
    // INC-I-055 window logic: transient disconnections should not mark unhealthy.
    let d = decide_checkpoint_health(
        true, // self_consistent
        3,    // peer_count
        1,    // peers_agreeing
        2,    // unique_hashes
        true, // window_healthy
    );
    assert!(
        d.healthy,
        "P5a O1: consistent + window_healthy overrides peer disagreement"
    );
    assert!(!d.isolated, "P5a O2: not isolated");
    assert!(d.self_consistent, "P5a O3: self_consistent echoes");
}

// ============================================================
// Edge cases: boundary and adversarial inputs
// ============================================================

#[test]
fn test_edge_all_false_inputs() {
    // All zeros/false -- worst case: no state, no peers, no window.
    let d = decide_checkpoint_health(false, 0, 0, 0, false);
    assert!(!d.healthy, "edge: all-false inputs must be unhealthy");
    assert!(d.isolated, "edge: no peers = isolated");
    assert!(!d.self_consistent, "edge: self_consistent echoes");
}

#[test]
fn test_edge_large_peer_count() {
    // Large peer count with full agreement.
    let d = decide_checkpoint_health(true, 1000, 1000, 1, false);
    assert!(d.healthy, "edge: 1000 agreeing peers = healthy");
    assert!(!d.isolated, "edge: not isolated");
}

#[test]
fn test_edge_peers_agreeing_exceeds_peer_count() {
    // Defensive: peers_agreeing > peer_count should not crash.
    // Treat as point-healthy (all agree and more).
    let d = decide_checkpoint_health(true, 2, 5, 1, false);
    // Implementation should handle gracefully -- peers_agreeing >= peer_count = point-healthy.
    assert!(d.healthy, "edge: peers_agreeing > peer_count = healthy");
}

#[test]
fn test_edge_zero_unique_hashes_with_peers() {
    // unique_hashes=0 with peers. This means no hash comparison data available.
    // point_healthy requires unique_hashes <= 1, so 0 qualifies.
    let d = decide_checkpoint_health(true, 3, 3, 0, false);
    assert!(
        d.healthy,
        "edge: 0 unique hashes with agreeing peers = healthy"
    );
}
