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
