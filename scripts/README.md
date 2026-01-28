# Test Scripts Registry

This file documents all test scripts in the `scripts/` directory. Before creating a new test script, check this registry to see if an existing script can be used or modified.

---

## Network & Node Scripts

### launch_testnet.sh

| Property | Value |
|----------|-------|
| **Path** | `scripts/launch_testnet.sh` |
| **Purpose** | Launch a local devnet with 2 producer nodes for testing |
| **What it tests** | Basic node startup, P2P connectivity, block production |
| **Dependencies** | `cargo`, `doli-node`, `doli-crypto` |
| **Run time** | Interactive (runs until stopped) |
| **Output** | `/tmp/doli-testnet/` (data, logs, helper scripts) |

**Usage:**
```bash
./scripts/launch_testnet.sh
```

**Features:**
- Generates producer keys automatically
- Creates helper scripts: `launch_both.sh`, `check_status.sh`
- Configurable ports (P2P: 40303-40304, RPC: 18545-18546)

---

### stress_test_600.sh

| Property | Value |
|----------|-------|
| **Path** | `scripts/stress_test_600.sh` |
| **Purpose** | Simulate 600 producers on devnet (extreme stress test) |
| **What it tests** | Network scalability, resource usage, consensus under load |
| **Dependencies** | `cargo`, `doli-node`, 64GB+ RAM recommended |
| **Run time** | Long-running (until stopped) |
| **Output** | `~/.doli/stress-test-*` (per-node data) |

**Usage:**
```bash
PRODUCER_COUNT=100 ./scripts/stress_test_600.sh  # Reduce for lower resources
./scripts/stress_test_600.sh                      # Full 600 nodes
```

**Environment variables:**
- `PRODUCER_COUNT` - Number of producers (default: 600)
- `BASE_P2P_PORT` - Starting P2P port (default: 50303)
- `BASE_RPC_PORT` - Starting RPC port (default: 28545)
- `LOG_LEVEL` - Log level (default: warn)

**Requirements:**
- 64GB+ RAM (128GB recommended)
- 32+ CPU cores
- 500GB+ SSD

---

## Validator Reward Scripts

### test_staggered_validator_rewards.sh

| Property | Value |
|----------|-------|
| **Path** | `scripts/test_staggered_validator_rewards.sh` |
| **Purpose** | Test that validators only receive rewards from the moment they join |
| **What it tests** | Staggered join timing, epoch rewards, per-node reward tracking |
| **Dependencies** | `cargo`, `doli-node`, `crypto` crate |
| **Run time** | ~10 minutes |
| **Output** | `/tmp/doli-staggered-test/` (logs, summary) |

**Usage:**
```bash
./scripts/test_staggered_validator_rewards.sh
```

**Test scenario:**
- 10 nodes join at 30-second intervals
- Tracks when each node starts producing blocks
- Monitors reward distribution across epoch boundaries

**Key assertions:**
- Nodes receive rewards only for epochs they participated in
- Nodes joining mid-epoch get fair share in next complete epoch

---

### test_validator_rewards_simple.sh

| Property | Value |
|----------|-------|
| **Path** | `scripts/test_validator_rewards_simple.sh` |
| **Purpose** | Simplified 3-node validator reward test |
| **What it tests** | Basic reward distribution with fewer nodes |
| **Dependencies** | `cargo`, `doli-node` |
| **Run time** | ~5 minutes |
| **Output** | `/tmp/doli-simple-test/` (logs) |

**Usage:**
```bash
./scripts/test_validator_rewards_simple.sh
```

**Test scenario:**
- 3 nodes with 45-second stabilization between joins
- Simpler than staggered test for quick verification

---

### test_3node_proportional_rewards.sh

| Property | Value |
|----------|-------|
| **Path** | `scripts/test_3node_proportional_rewards.sh` |
| **Purpose** | Test proportional reward distribution based on block production |
| **What it tests** | Proportional vs equal rewards across epochs |
| **Dependencies** | `cargo`, `doli-node`, `doli-cli` |
| **Run time** | ~6 minutes |
| **Output** | `/tmp/doli-3node-test/` (logs) |

**Usage:**
```bash
./scripts/test_3node_proportional_rewards.sh
```

**Test scenario:**
- Node 1: starts immediately (seed)
- Node 2: joins at +45s
- Node 3: joins at +90s

**Expected results:**
- Epoch 0: Disproportional rewards (Node1 > Node2 > Node3)
- Epoch 1+: Equal rewards (all nodes round-robin)

---

### test_devnet_3node_rewards.sh

| Property | Value |
|----------|-------|
| **Path** | `scripts/test_devnet_3node_rewards.sh` |
| **Purpose** | Test 3-node devnet with staggered joins and detailed rewards analysis |
| **What it tests** | Exact reward distribution (8 decimal precision), epoch boundaries, round-robin convergence |
| **Dependencies** | `cargo`, `doli-node`, `doli-cli` |
| **Run time** | ~5 minutes |
| **Output** | `/tmp/doli-devnet-3node/` (logs, reports) |

**Usage:**
```bash
./scripts/test_devnet_3node_rewards.sh
```

**Test scenario:**
- Node 1: starts immediately (seed/genesis)
- Node 2: joins at +60s
- Node 3: joins at +120s
- Monitors for 5 minutes total (15 epochs)

**Devnet parameters:**
- 1-second slots
- 20 slots per epoch (20s epochs)
- 1.00000000 DOLI block reward
- 1M VDF iterations (~70ms)

**Expected results:**
- Early epochs: Node 1 dominates (was only producer)
- After Node 2 joins: 50/50 distribution
- After Node 3 joins: ~33/33/33 equal distribution (round-robin)

**Report output:**
- `reports/rewards_report.txt` - Summary report
- `reports/detailed_rewards_report.md` - Detailed markdown report with epoch-by-epoch analysis
- Raw logs for each node in `logs/`

---

## Quick Reference

| Script | Nodes | Duration | Purpose |
|--------|-------|----------|---------|
| `launch_testnet.sh` | 2 | Interactive | Basic devnet |
| `stress_test_600.sh` | 600 | Long | Scalability |
| `test_staggered_validator_rewards.sh` | 10 | ~10 min | Staggered rewards |
| `test_validator_rewards_simple.sh` | 3 | ~5 min | Simple rewards |
| `test_3node_proportional_rewards.sh` | 3 | ~6 min | Proportional rewards |
| `test_devnet_3node_rewards.sh` | 3 | ~5 min | Detailed epoch rewards |

---

## Adding New Scripts

When adding a new test script:

1. Create the script in `scripts/`
2. Add an entry to this README with:
   - Path
   - Purpose
   - What it tests
   - Dependencies
   - Run time estimate
   - Output location
   - Usage examples
3. Update the Quick Reference table
4. Commit both the script and this README together

---

## Script Conventions

All scripts should:

1. Use `set -e` to fail on errors
2. Define cleanup traps for spawned processes
3. Use colored output for readability
4. Print clear status messages
5. Save logs to a predictable location (usually `/tmp/doli-*`)
6. Include usage comments at the top
