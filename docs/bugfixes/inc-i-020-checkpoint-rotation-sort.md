# INC-I-020: Checkpoint Rotation Lexicographic Sort Bug

## Summary
Auto-checkpoint rotation uses lexicographic filename sort to determine which checkpoints to keep. Directory names like `h526` sort after `h4535` because `'5' > '4'` in ASCII. Result: old unhealthy checkpoints survive, new healthy ones are immediately deleted.

## Root Cause
Two locations use `dirs.sort_by_key(|e| e.file_name())` on checkpoint directories:
1. `bins/node/src/node/periodic.rs:550` — rotation logic (keep last 5)
2. `crates/rpc/src/methods/guardian.rs:166` — guardian status (find last/last_healthy)

Directory names are `h{height}-{timestamp}` with NO zero-padding. Lexicographic comparison of `h526` vs `h4535` compares character by character: `h` == `h`, then `5` > `4`, so `h526` sorts after `h4535`.

## Impact
- All healthy checkpoints at heights 1000+ are immediately deleted after creation
- `getGuardianStatus` returns `last_healthy_checkpoint: null` (no safe recovery point)
- Guardian safety net is non-functional for any chain that crosses 3→4 digit heights
- Both mainnet seeds (ai1, ai2) affected — confirmed from production logs

## Architecture Context
- `periodic.rs` runs every ~1s via `run_periodic_tasks()`
- Auto-checkpoint triggers at `current_height >= last_checkpoint_height + interval`
- Rotation runs inline after each successful checkpoint creation
- `guardian.rs` reads checkpoint dirs on each `getGuardianStatus` RPC call
- No other code reads checkpoint directories

## Fix (single milestone)
Extract a `parse_checkpoint_height()` helper. Sort numerically by parsed height in both locations.

### Requirements
- **REQ-FIX-001** (Must): Sort checkpoints numerically by height, not lexicographically
- **REQ-FIX-002** (Must): Fix both periodic.rs rotation AND guardian.rs status
- **REQ-FIX-003** (Must): Unit test covering mixed 3/4/5-digit height directories
- **REQ-FIX-004** (Should): Test verifies rotation keeps the 5 highest-height checkpoints

## Blast Radius
Minimal — only two sort operations in two files. No wire format, no consensus, no serialization changes.
