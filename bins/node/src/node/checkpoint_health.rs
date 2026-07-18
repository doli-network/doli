//! Checkpoint health decision logic (INC-I-136 M2 + M3).
//!
//! Pure functions that determine whether a checkpoint should be tagged
//! `healthy` (M2), and which checkpoints to evict during rotation (M3),
//! separating the peer-agreement signal from a local self-consistency
//! predicate (block-body contiguity + undo-data presence).

/// The result of evaluating checkpoint health.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointHealthDecision {
    /// Whether the checkpoint should be tagged as healthy.
    pub healthy: bool,
    /// Whether the node is isolated (no connected peers).
    pub isolated: bool,
    /// Echo of the input self-consistency flag.
    pub self_consistent: bool,
}

/// Decide whether a checkpoint should be tagged `healthy`.
///
/// # Arguments
/// - `self_consistent`: true iff block store has contiguous bodies AND undo
///   data is present for the rollback window.
/// - `peer_count`: number of connected peers.
/// - `peers_agreeing`: how many peers agree on the chain tip hash.
/// - `unique_hashes`: how many distinct chain tip hashes are seen across peers.
/// - `window_healthy`: true if the rolling health window has at least one
///   healthy sample in the last ~10 minutes.
///
/// # Semantics
/// 1. `self_consistent == false` -> `healthy = false` ALWAYS.
/// 2. `self_consistent == true AND peer_count == 0` -> `healthy = true, isolated = true` (F2).
/// 3. `self_consistent == true AND point-healthy` -> `healthy = true, isolated = false`.
/// 4. `self_consistent == true AND peers disagree AND !window_healthy` -> `healthy = false`.
/// 5. `self_consistent == true AND window_healthy` -> `healthy = true`.
pub fn decide_checkpoint_health(
    self_consistent: bool,
    peer_count: usize,
    peers_agreeing: usize,
    unique_hashes: usize,
    window_healthy: bool,
) -> CheckpointHealthDecision {
    let isolated = peer_count == 0;

    if !self_consistent {
        return CheckpointHealthDecision {
            healthy: false,
            isolated,
            self_consistent: false,
        };
    }

    // Self-consistent from here on.

    // F2: An isolated-but-consistent node MUST produce a healthy checkpoint.
    if isolated {
        return CheckpointHealthDecision {
            healthy: true,
            isolated: true,
            self_consistent: true,
        };
    }

    // Point-healthy: all peers agree on a single chain tip.
    let point_healthy = peer_count > 0 && peers_agreeing >= peer_count && unique_hashes <= 1;

    let healthy = point_healthy || window_healthy;

    CheckpointHealthDecision {
        healthy,
        isolated: false,
        self_consistent: true,
    }
}

/// Given checkpoints as (height, healthy) -- NOT assumed sorted -- and the
/// recent-window size, return the set of indices to EVICT.
///
/// Retains the top `keep_recent` by height, PLUS the single highest-height
/// healthy checkpoint even if it falls outside that window.
/// Evicted = everything else.
///
/// # Arguments
/// - `checkpoints`: slice of `(height, healthy)` tuples, one per checkpoint
///   directory on disk. Index in the slice = identity of the checkpoint.
/// - `keep_recent`: how many most-recent (by height) checkpoints to keep
///   unconditionally.
///
/// # Returns
/// Indices into `checkpoints` that should be removed from disk.
pub(crate) fn select_checkpoint_evictions(
    checkpoints: &[(u64, bool)],
    keep_recent: usize,
) -> Vec<usize> {
    if checkpoints.len() <= keep_recent {
        return Vec::new();
    }

    // Build (original_index, height) pairs, sort by height descending.
    let mut indexed: Vec<(usize, u64)> = checkpoints
        .iter()
        .enumerate()
        .map(|(i, &(h, _))| (i, h))
        .collect();
    indexed.sort_by_key(|b| std::cmp::Reverse(b.1)); // descending by height

    // Retained set = top `keep_recent` by height.
    let mut retained: std::collections::HashSet<usize> =
        indexed[..keep_recent].iter().map(|&(idx, _)| idx).collect();

    // Also retain the single highest-height healthy checkpoint, if any.
    // Walk height-descending to find the first healthy entry.
    for &(idx, _) in &indexed {
        if checkpoints[idx].1 {
            retained.insert(idx);
            break;
        }
    }

    // Evict everything not retained, return sorted indices.
    let mut evictions: Vec<usize> = (0..checkpoints.len())
        .filter(|i| !retained.contains(i))
        .collect();
    evictions.sort();
    evictions
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================
    // OUTPUT CONTRACT: fn decide_checkpoint_health(
    //     self_consistent: bool, peer_count: usize, peers_agreeing: usize,
    //     unique_hashes: usize, window_healthy: bool,
    // ) -> CheckpointHealthDecision
    //
    // Outputs:
    //   O1: return.healthy   -- bool
    //   O2: return.isolated  -- bool
    //   O3: return.self_consistent -- bool (echo)
    //
    // Paths:
    //   P1: self_consistent == false  -> healthy=false ALWAYS
    //   P2: self_consistent == true AND peer_count == 0 -> healthy=true, isolated=true
    //   P3: self_consistent == true AND point-healthy -> healthy=true, isolated=false
    //   P4: self_consistent == true AND disagree AND !window -> healthy=false
    //   P5: self_consistent == true AND window_healthy -> healthy=true
    //
    // INPUT PARTITIONS:
    //   P1a: isolated (peer_count=0)
    //   P1b: with agreeing peers
    //   P1c: with window_healthy=true
    //   P2a: no window_healthy
    //   P2b: with window_healthy
    //   P3a: full agreement (peers_agreeing == peer_count)
    //   P3b: minimal (1 peer)
    //   P4a: partial agreement, multi-tip
    //   P4b: full count but multi-tip
    //   P5a: peers disagree but window healthy
    //
    // MATRIX: 3 outputs x 10 partitions = 30 cells
    //   P1a: O1(false) O2(true) O3(false)
    //   P1b: O1(false) O2(false) O3(false)
    //   P1c: O1(false) O2(false) O3(false)
    //   P2a: O1(true) O2(true) O3(true)
    //   P2b: O1(true) O2(true) O3(true)
    //   P3a: O1(true) O2(false) O3(true)
    //   P3b: O1(true) O2(false) O3(true)
    //   P4a: O1(false) O2(false) O3(true)
    //   P4b: O1(false) O2(false) O3(true)
    //   P5a: O1(true) O2(false) O3(true)
    // ============================================================

    // P1: self_consistent == false -> healthy=false ALWAYS
    #[test]
    fn test_p1a_inconsistent_isolated_is_unhealthy() {
        let d = decide_checkpoint_health(false, 0, 0, 0, false);
        assert!(!d.healthy, "P1a O1");
        assert!(d.isolated, "P1a O2");
        assert!(!d.self_consistent, "P1a O3");
    }

    #[test]
    fn test_p1b_inconsistent_with_agreeing_peers_is_unhealthy() {
        let d = decide_checkpoint_health(false, 3, 3, 1, false);
        assert!(!d.healthy, "P1b O1");
        assert!(!d.isolated, "P1b O2");
        assert!(!d.self_consistent, "P1b O3");
    }

    #[test]
    fn test_p1c_inconsistent_with_window_healthy_is_unhealthy() {
        let d = decide_checkpoint_health(false, 2, 2, 1, true);
        assert!(!d.healthy, "P1c O1");
        assert!(!d.isolated, "P1c O2");
        assert!(!d.self_consistent, "P1c O3");
    }

    // P2: self_consistent == true AND peer_count == 0 -> healthy=true, isolated=true
    #[test]
    fn test_p2a_consistent_isolated_no_window_is_healthy() {
        let d = decide_checkpoint_health(true, 0, 0, 0, false);
        assert!(d.healthy, "P2a O1");
        assert!(d.isolated, "P2a O2");
        assert!(d.self_consistent, "P2a O3");
    }

    #[test]
    fn test_p2b_consistent_isolated_with_window_is_healthy() {
        let d = decide_checkpoint_health(true, 0, 0, 0, true);
        assert!(d.healthy, "P2b O1");
        assert!(d.isolated, "P2b O2");
        assert!(d.self_consistent, "P2b O3");
    }

    // P3: self_consistent == true AND point-healthy -> healthy=true
    #[test]
    fn test_p3a_consistent_full_peer_agreement() {
        let d = decide_checkpoint_health(true, 5, 5, 1, false);
        assert!(d.healthy, "P3a O1");
        assert!(!d.isolated, "P3a O2");
        assert!(d.self_consistent, "P3a O3");
    }

    #[test]
    fn test_p3b_consistent_minimal_peer_agreement() {
        let d = decide_checkpoint_health(true, 1, 1, 1, false);
        assert!(d.healthy, "P3b O1");
        assert!(!d.isolated, "P3b O2");
        assert!(d.self_consistent, "P3b O3");
    }

    // P4: self_consistent == true AND peers disagree AND !window -> unhealthy
    #[test]
    fn test_p4a_consistent_peers_split_no_window() {
        let d = decide_checkpoint_health(true, 3, 1, 2, false);
        assert!(!d.healthy, "P4a O1");
        assert!(!d.isolated, "P4a O2");
        assert!(d.self_consistent, "P4a O3");
    }

    #[test]
    fn test_p4b_consistent_multi_tip_no_window() {
        let d = decide_checkpoint_health(true, 3, 3, 2, false);
        assert!(!d.healthy, "P4b O1");
        assert!(!d.isolated, "P4b O2");
        assert!(d.self_consistent, "P4b O3");
    }

    // P5: self_consistent == true AND window_healthy -> healthy=true
    #[test]
    fn test_p5a_consistent_peers_disagree_but_window_healthy() {
        let d = decide_checkpoint_health(true, 3, 1, 2, true);
        assert!(d.healthy, "P5a O1");
        assert!(!d.isolated, "P5a O2");
        assert!(d.self_consistent, "P5a O3");
    }

    // Edge cases
    #[test]
    fn test_edge_all_false_inputs() {
        let d = decide_checkpoint_health(false, 0, 0, 0, false);
        assert!(!d.healthy);
        assert!(d.isolated);
        assert!(!d.self_consistent);
    }

    #[test]
    fn test_edge_large_peer_count() {
        let d = decide_checkpoint_health(true, 1000, 1000, 1, false);
        assert!(d.healthy);
        assert!(!d.isolated);
    }

    #[test]
    fn test_edge_peers_agreeing_exceeds_peer_count() {
        let d = decide_checkpoint_health(true, 2, 5, 1, false);
        assert!(d.healthy);
    }

    #[test]
    fn test_edge_zero_unique_hashes_with_peers() {
        let d = decide_checkpoint_health(true, 3, 3, 0, false);
        assert!(d.healthy);
    }

    // ============================================================
    // OUTPUT CONTRACT: fn select_checkpoint_evictions(
    //     checkpoints: &[(u64, bool)], keep_recent: usize,
    // ) -> Vec<usize>
    //
    // Outputs:
    //   O1: return Vec<usize> -- indices into `checkpoints` to evict
    //       (complement = retained set = top `keep_recent` by height
    //        UNION highest-height healthy checkpoint, if any)
    //
    // Paths:
    //   P1: len > keep_recent AND healthy checkpoint exists outside window
    //       -> evict lowest heights EXCEPT the highest healthy
    //   P2: len > keep_recent AND no healthy checkpoints exist
    //       -> evict all but top keep_recent (plain rotation)
    //   P3: len > keep_recent AND highest healthy is inside window
    //       -> evict all but top keep_recent (no extra retention)
    //   P4: len <= keep_recent -> evict nothing
    //   P5: empty input -> evict nothing
    //
    // INPUT PARTITIONS:
    //   P1a: sorted input, healthy outside window (heights 1..=10,
    //        healthy at idx 0,1, keep=5) -- immunity for idx 1
    //   P1b: unsorted input, same logical scenario -- must sort internally
    //   P1c: single healthy outside window among many unhealthy
    //        (7 checkpoints, idx 0 healthy, keep=5) -- retain idx 0
    //   P2a: all unhealthy, 8 checkpoints, keep=5 -- plain rotation
    //   P3a: most-recent (highest height) is healthy, inside window,
    //        6 checkpoints, keep=5 -- bounded to 5
    //   P4a: fewer than keep_recent (3 checkpoints, keep=5)
    //   P5a: empty slice
    //
    // MATRIX: 1 output x 7 partitions = 7 cells
    //   P1a: O1(evictions={0,2,3,4})     -- idx 1 immune
    //   P1b: O1(evictions={3,4,6,8})     -- idx 1 immune (shuffled)
    //   P1c: O1(evictions={1})           -- idx 0 immune
    //   P2a: O1(evictions={0,1,2})       -- no healthy, plain rotation
    //   P3a: O1(evictions={0})           -- healthy in window, bounded
    //   P4a: O1(evictions={})            -- under capacity
    //   P5a: O1(evictions={})            -- empty
    // ============================================================

    // ---- M3 tests: select_checkpoint_evictions (INC-I-136 F3+F5) ----

    // Requirement: REQ-GUARD-004 F3 (Must)
    // Acceptance: After an arbitrarily long incident (>5 checkpoints created),
    //             the last pre-incident healthy checkpoint still exists on disk.
    // Partition: P1a -- sorted input, healthy outside window
    #[test]
    fn test_m3_protect_old_healthy() {
        // Heights 1..=10 (index i -> height i+1).
        // Healthy only at indices 0 (height 1) and 1 (height 2).
        // keep_recent = 5 -> retain heights 6-10 (indices 5-9).
        // Immunity: most-recent healthy = height 2 (index 1) -> also retained.
        // Evictions = {0, 2, 3, 4} (indices with heights 1, 3, 4, 5).
        let checkpoints: Vec<(u64, bool)> = (1..=10)
            .map(|h| {
                let healthy = h <= 2;
                (h, healthy)
            })
            .collect();

        let evictions = select_checkpoint_evictions(&checkpoints, 5);
        let mut evictions_sorted = evictions.clone();
        evictions_sorted.sort();

        // Index 1 (height 2, the highest healthy) MUST be retained (immunity).
        assert!(
            !evictions_sorted.contains(&1),
            "F3: index 1 (height 2, highest healthy) must be immune from eviction, \
             but was evicted. evictions={:?}",
            evictions_sorted,
        );

        // Index 0 (height 1, also healthy but NOT the highest healthy) IS evicted.
        assert!(
            evictions_sorted.contains(&0),
            "index 0 (height 1, lower healthy) should be evicted. evictions={:?}",
            evictions_sorted,
        );

        // Full expected eviction set.
        assert_eq!(
            evictions_sorted,
            vec![0, 2, 3, 4],
            "F3: expected evictions {{0,2,3,4}}, got {:?}",
            evictions_sorted,
        );
    }

    // Requirement: REQ-GUARD-005 F5 (Must)
    // Acceptance: A multi-day incident cannot rotate away all pre-incident anchors.
    // Partition: P1c -- single healthy outside window among many unhealthy
    #[test]
    fn test_m3_evict_beyond_window_keeps_healthy() {
        // 7 checkpoints: index 0 healthy (height 1), indices 1-6 unhealthy (heights 2-7).
        // keep_recent = 5 -> retain heights 3-7 (indices 2-6).
        // Immunity: highest healthy = height 1 (index 0) -> also retained.
        // Evictions = {1} (index 1, height 2, unhealthy, outside window).
        let checkpoints = vec![
            (1, true),  // index 0 -- healthy, outside window
            (2, false), // index 1 -- unhealthy, outside window
            (3, false), // index 2 -- unhealthy, in window
            (4, false), // index 3 -- unhealthy, in window
            (5, false), // index 4 -- unhealthy, in window
            (6, false), // index 5 -- unhealthy, in window
            (7, false), // index 6 -- unhealthy, in window
        ];

        let evictions = select_checkpoint_evictions(&checkpoints, 5);
        let mut evictions_sorted = evictions.clone();
        evictions_sorted.sort();

        // Index 0 (the only healthy checkpoint) MUST be retained even outside window.
        assert!(
            !evictions_sorted.contains(&0),
            "F5: index 0 (height 1, only healthy) must be retained outside window, \
             but was evicted. evictions={:?}",
            evictions_sorted,
        );

        // Only index 1 should be evicted.
        assert_eq!(
            evictions_sorted,
            vec![1],
            "F5: expected evictions {{1}}, got {:?}",
            evictions_sorted,
        );
    }

    // Requirement: REQ-GUARD-004 F3 (Must)
    // Acceptance: When no healthy checkpoint exists, rotation behaves like
    //             the old unconditional keep-last-N.
    // Partition: P2a -- all unhealthy
    #[test]
    fn test_m3_no_healthy_behaves_like_plain_rotation() {
        // 8 checkpoints, all unhealthy, heights 1..=8.
        // keep_recent = 5 -> retain heights 4-8 (indices 3-7).
        // No healthy to protect -> evictions = {0, 1, 2}.
        let checkpoints: Vec<(u64, bool)> = (1..=8).map(|h| (h, false)).collect();

        let evictions = select_checkpoint_evictions(&checkpoints, 5);
        let mut evictions_sorted = evictions.clone();
        evictions_sorted.sort();

        assert_eq!(
            evictions_sorted,
            vec![0, 1, 2],
            "No healthy checkpoints -> plain rotation: expected {{0,1,2}}, got {:?}",
            evictions_sorted,
        );
    }

    // Requirement: REQ-GUARD-004 F3 (Must)
    // Acceptance: When the highest healthy is already inside the recent window,
    //             retention is bounded to keep_recent (no double-count).
    // Partition: P3a -- healthy inside window
    #[test]
    fn test_m3_healthy_already_in_window_no_extra() {
        // 6 checkpoints, heights 1..=6. Index 5 (height 6, the highest) is healthy.
        // keep_recent = 5 -> retain heights 2-6 (indices 1-5).
        // Healthy (index 5) is already in window -> no extra slot needed.
        // Evictions = {0} (index 0, height 1).
        let checkpoints = vec![
            (1, false), // index 0 -- outside window
            (2, false), // index 1 -- in window
            (3, false), // index 2 -- in window
            (4, false), // index 3 -- in window
            (5, false), // index 4 -- in window
            (6, true),  // index 5 -- in window, healthy
        ];

        let evictions = select_checkpoint_evictions(&checkpoints, 5);
        let mut evictions_sorted = evictions.clone();
        evictions_sorted.sort();

        assert_eq!(
            evictions_sorted,
            vec![0],
            "Healthy in window -> bounded retention: expected {{0}}, got {:?}",
            evictions_sorted,
        );
    }

    // Requirement: REQ-GUARD-005 F5 (Must)
    // Acceptance: Fewer checkpoints than keep_recent evicts nothing.
    // Partition: P4a -- under capacity
    #[test]
    fn test_m3_fewer_than_keep_recent_evicts_nothing() {
        let checkpoints = vec![(10, true), (20, false), (30, false)];

        let evictions = select_checkpoint_evictions(&checkpoints, 5);

        assert!(
            evictions.is_empty(),
            "Under capacity (3 < 5): expected no evictions, got {:?}",
            evictions,
        );
    }

    // Requirement: REQ-GUARD-005 F5 (Must)
    // Acceptance: Empty input produces no evictions (no panic).
    // Partition: P5a -- empty
    #[test]
    fn test_m3_empty_input() {
        let checkpoints: Vec<(u64, bool)> = vec![];

        let evictions = select_checkpoint_evictions(&checkpoints, 5);

        assert!(
            evictions.is_empty(),
            "Empty input: expected no evictions, got {:?}",
            evictions,
        );
    }

    // Requirement: REQ-GUARD-004 F3 (Must)
    // Acceptance: Function handles unsorted input correctly (must sort internally).
    // Partition: P1b -- unsorted input, same logical scenario as P1a
    #[test]
    fn test_m3_unsorted_input() {
        // Same scenario as test_m3_protect_old_healthy but with shuffled input.
        // Heights 1..=10, healthy at heights 1 and 2, keep_recent=5.
        // Shuffled order: [7, 2, 10, 1, 4, 9, 3, 6, 5, 8]
        //                  i0  i1  i2  i3  i4  i5  i6  i7  i8  i9
        let checkpoints = vec![
            (7, false),  // index 0
            (2, true),   // index 1 -- healthy (highest healthy)
            (10, false), // index 2
            (1, true),   // index 3 -- healthy (lower)
            (4, false),  // index 4
            (9, false),  // index 5
            (3, false),  // index 6
            (6, false),  // index 7
            (5, false),  // index 8
            (8, false),  // index 9
        ];

        // Top-5 by height: heights 10,9,8,7,6 -> indices {2,5,9,0,7}.
        // Highest healthy: height 2 -> index 1. Must be retained (immunity).
        // Evicted: everything else -> indices {3,4,6,8} (heights 1,4,3,5).
        let evictions = select_checkpoint_evictions(&checkpoints, 5);
        let mut evictions_sorted = evictions.clone();
        evictions_sorted.sort();

        // Index 1 (height 2, highest healthy) MUST be retained.
        assert!(
            !evictions_sorted.contains(&1),
            "F3 unsorted: index 1 (height 2, highest healthy) must be immune, \
             but was evicted. evictions={:?}",
            evictions_sorted,
        );

        // Index 3 (height 1, lower healthy) IS evicted.
        assert!(
            evictions_sorted.contains(&3),
            "F3 unsorted: index 3 (height 1, lower healthy) should be evicted. \
             evictions={:?}",
            evictions_sorted,
        );

        // Full expected eviction set.
        assert_eq!(
            evictions_sorted,
            vec![3, 4, 6, 8],
            "F3 unsorted: expected evictions {{3,4,6,8}}, got {:?}",
            evictions_sorted,
        );
    }
}
