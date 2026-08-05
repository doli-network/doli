// OUTPUT CONTRACT: fn select_checkpoint_evictions(
//     checkpoints: &[(u64, bool)], keep_recent: usize,
// ) -> Vec<usize>
//
// Outputs:
//   O1: return Vec<usize> -- indices into `checkpoints` to evict
//
// Paths:
//   P1: len > keep_recent AND healthy outside window -> protect highest healthy
//   P2: len > keep_recent AND no healthy -> plain rotation
//   P3: len > keep_recent AND healthy inside window -> no extra retention
//   P4: len <= keep_recent -> evict nothing
//   P5: empty input -> evict nothing
//
// INPUT PARTITIONS:
//   P1a: sorted, healthy outside window (10 ckpts, healthy idx 0,1, keep=5)
//   P1b: unsorted, same scenario (shuffled order)
//   P1c: single healthy outside window (7 ckpts, idx 0 healthy, keep=5)
//   P2a: all unhealthy (8 ckpts, keep=5)
//   P3a: healthy inside window (6 ckpts, highest is healthy, keep=5)
//   P4a: under capacity (3 ckpts, keep=5)
//   P5a: empty slice
//
// MATRIX: 1 output x 7 partitions = 7 cells
//   P1a: O1(evictions={0,2,3,4})
//   P1b: O1(evictions={3,4,6,8})
//   P1c: O1(evictions={1})
//   P2a: O1(evictions={0,1,2})
//   P3a: O1(evictions={0})
//   P4a: O1(evictions={})
//   P5a: O1(evictions={})
//
// Test file: bins/node/src/node/checkpoint_health.rs
// Tests: test_m3_protect_old_healthy, test_m3_evict_beyond_window_keeps_healthy,
//        test_m3_no_healthy_behaves_like_plain_rotation,
//        test_m3_healthy_already_in_window_no_extra,
//        test_m3_fewer_than_keep_recent_evicts_nothing,
//        test_m3_empty_input, test_m3_unsorted_input
