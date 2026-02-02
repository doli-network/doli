# Devnet Subcommands Implementation Report

## Overview

Implemented CLI subcommands for local devnet management, replacing manual shell scripts with integrated Rust commands.

## Commits

| Commit | Description |
|--------|-------------|
| `b8913ab` | feat(node): add devnet subcommands for local multi-node testing |
| `e82d6e0` | docs: document doli-node devnet subcommands |
| `d57c1e9` | fix(devnet): pipe confirmation for --force-start flag |
| `56c1a69` | fix(node): skip network logging for devnet subcommands |
| `126356e` | docs(skill): update network-setup for devnet subcommands |

## Commands Implemented

```bash
doli-node devnet init --nodes N    # Initialize devnet with N producers (1-20)
doli-node devnet start             # Start all nodes
doli-node devnet stop              # Stop all nodes
doli-node devnet status            # Show chain status table
doli-node devnet clean [--keep-keys]  # Remove devnet data
```

## Files Created/Modified

### Created
- `bins/node/src/devnet.rs` (~450 lines) - Core devnet management module

### Modified
- `bins/node/src/main.rs` - Added DevnetCommands enum and handler
- `bins/node/Cargo.toml` - Added reqwest dependency
- `docs/running_a_node.md` - Added devnet commands documentation
- `scripts/README.md` - Added note about CLI commands
- `.claude/skills/network-setup/SKILL.md` - Updated to v2.4.0

## Directory Structure

```
~/.doli/devnet/
├── devnet.toml          # Config (node_count, base ports)
├── chainspec.json       # Generated genesis with all producers
├── keys/producer_*.json # Wallet files (compatible with doli-cli)
├── data/node*/          # Node data directories
├── logs/node*.log       # Log files
└── pids/node*.pid       # PID tracking
```

## Port Allocation

| Node | P2P Port | RPC Port | Metrics Port |
|------|----------|----------|--------------|
| 0    | 50303    | 28545    | 9090         |
| 1    | 50304    | 28546    | 9091         |
| N    | 50303+N  | 28545+N  | 9090+N       |

## Fixed: Node Startup Failure

### Problem

When running `doli-node devnet start`, node 0 started but crashed before RPC became available due to stdin pipe closure after `--force-start` confirmation.

### Root Cause

The devnet code piped "I UNDERSTAND\n" to stdin for `--force-start` confirmation, then closed the pipe. This caused the spawned process to exit unexpectedly.

### Solution

Added `--yes` flag to `doli-node run` that bypasses interactive confirmations:

```bash
doli-node run --producer --producer-key key.json --force-start --yes
```

**Changes:**
1. `bins/node/src/main.rs`: Added `--yes` flag to Run command
2. `bins/node/src/devnet.rs`: Use `--yes` + `Stdio::null()` instead of piping stdin

This follows the standard pattern used by tools like `apt -y`, `rm -f`, etc.

## Tests

All existing tests pass:
```
running 9 tests
test devnet::tests::test_devnet_config_ports ... ok
test metrics::tests::test_record_block_processed ... ok
test metrics::tests::test_update_chain_metrics ... ok
test producer::guard::tests::test_lock_file_released_on_shutdown ... ok
test producer::guard::tests::test_lock_file_prevents_second_instance ... ok
test metrics::tests::test_metrics_handler ... ok
test producer::signed_slots::tests::test_signed_slots_prevents_double_sign ... ok
test producer::signed_slots::tests::test_signed_slots_persists_across_restart ... ok
test producer::signed_slots::tests::test_signed_slots_prune ... ok
```
