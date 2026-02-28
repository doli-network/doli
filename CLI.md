# CLI Issues

## 2026-02-04 - `devnet clean` doesn't kill manually started nodes

- **Type**: Constraint
- **Command**: `doli-node devnet clean`
- **Observed**: Only kills nodes tracked in `devnet.toml` (nodes 0 to `node_count-1`). Manually started nodes (e.g., additional producers started with `doli-node run --producer`) remain running and hold ports.
- **Expected**: Should kill ALL doli-node processes running on devnet, or at minimum scan the entire `pids/` directory for all PID files (not just 0..node_count).
- **Priority**: Medium
- **Status**: Resolved
- **Root Cause**: `bins/node/src/devnet.rs` - loop iterated `0..config.node_count`, missing any nodes added after init.
- **Fix**: Added `scan_and_kill_all_pids()` helper that scans `pids/` directory for all `node*.pid` files. Updated both `stop` and `clean` functions to use this helper.

## 2026-02-28 - Header-first sync fails for genesis blocks on mature chains

- **Type**: Bug
- **Command**: N/A (internal sync path)
- **Observed**: When a new node joins a chain with >192 slots (~32 min old), header-first sync rejects genesis blocks because their slot numbers (e.g., slot 10) are more than `MAX_PAST_SLOTS` (192) behind the current slot. Sync stalls until snap sync takes over as fallback.
- **Expected**: Header-first sync should apply historical blocks without the `MAX_PAST_SLOTS` check. `ValidationMode::Light` already exists and skips this check (line 714-776 in `node.rs`), but `apply_block()` hardcodes `ValidationMode::Full` (line 2833).
- **Priority**: Low (snap sync covers this as fallback; only matters if snap sync fails due to <3 peers)
- **Status**: Open
- **Root Cause**: `bins/node/src/node.rs` line 2833 — `validate_block_for_apply()` called with `ValidationMode::Full` for all blocks, including synced historical blocks. The fix is to pass `ValidationMode` as a parameter to `apply_block()` and use `Light` for sync blocks in `run_periodic_tasks()` (line 5206).
- **Workaround**: Snap sync activates automatically when gap >1000 blocks with 3+ peers, bypassing header-first sync entirely.
