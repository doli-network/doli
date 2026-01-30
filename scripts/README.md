# Scripts Registry

This file documents all scripts in the `scripts/` directory. Before creating a new script, check this registry to see if an existing script can be used or modified.

---

## Release & Build Scripts

### smoke_test_release.sh

| Property | Value |
|----------|-------|
| **Path** | `scripts/smoke_test_release.sh` |
| **Purpose** | Verify release artifacts work correctly before distribution |
| **What it does** | Downloads binary, verifies checksum, starts node, tests RPC/P2P, clean shutdown |
| **Dependencies** | `curl`, `sha256sum`, `nc` (optional) |
| **Run time** | ~30-60 seconds |
| **Output** | Test results to stdout, logs in `/tmp/doli-smoke-test-*` |

**Usage:**
```bash
# Test locally built binary
./scripts/smoke_test_release.sh --binary target/release/doli-node

# Test specific release version
./scripts/smoke_test_release.sh --version v1.0.0

# Test from direct URL
./scripts/smoke_test_release.sh --url https://example.com/doli.tar.gz

# Test with specific target
./scripts/smoke_test_release.sh --version v1.0.0 --target x86_64-unknown-linux-musl

# Keep test directory for debugging
./scripts/smoke_test_release.sh --binary ./doli-node --keep
```

**Options:**
- `--binary PATH` - Path to pre-existing binary (skip download)
- `--version VERSION` - Version tag to download (e.g., v1.0.0)
- `--url URL` - Direct URL to download tarball
- `--target TARGET` - Target triple (default: auto-detect)
- `--timeout SECONDS` - Test timeout (default: 60)
- `--keep` - Keep test directory on success

**Exit codes:**
| Code | Meaning |
|------|---------|
| 0 | All tests passed |
| 1 | Test failed |
| 2 | Invalid arguments |
| 3 | Download/checksum failed |

**Tests performed:**
1. Binary acquisition (download or copy)
2. Checksum verification (if available)
3. Node startup in devnet mode
4. RPC endpoint responding
5. P2P port listening
6. Clean shutdown (SIGTERM)

---

### build_release.sh

| Property | Value |
|----------|-------|
| **Path** | `scripts/build_release.sh` |
| **Purpose** | Build release binaries for distribution |
| **What it does** | Compiles optimized binaries, generates checksums, creates tarballs |
| **Dependencies** | `cargo`, `cross` (for cross-compilation), `sha256sum` |
| **Run time** | ~10-30 minutes (depending on targets) |
| **Output** | `release/` directory with tarballs and checksums |

**Usage:**
```bash
# Build for current platform
./scripts/build_release.sh

# Build for all platforms (requires cross)
./scripts/build_release.sh --all

# Build for specific target
./scripts/build_release.sh --target x86_64-unknown-linux-musl

# Build with specific version
./scripts/build_release.sh --version v1.0.0 --linux

# List supported targets
./scripts/build_release.sh --list-targets
```

**Options:**
- `--all` - Build for all supported platforms
- `--linux` - Build for all Linux platforms
- `--macos` - Build for all macOS platforms (requires macOS host)
- `--target <TARGET>` - Build for specific target
- `--version <VERSION>` - Set version string
- `--skip-tests` - Skip running tests before build
- `--clean` - Clean build directory first

**Supported targets:**
| Target | Description |
|--------|-------------|
| `x86_64-unknown-linux-gnu` | Linux Intel/AMD (dynamically linked) |
| `x86_64-unknown-linux-musl` | Linux Intel/AMD (statically linked) |
| `aarch64-unknown-linux-gnu` | Linux ARM64 (dynamically linked) |
| `aarch64-unknown-linux-musl` | Linux ARM64 (statically linked) |
| `x86_64-apple-darwin` | macOS Intel |
| `aarch64-apple-darwin` | macOS Apple Silicon |

**Output:**
- `release/doli-{version}-{target}.tar.gz` - Binary tarball
- `release/doli-{version}-{target}.tar.gz.sha256` - Checksum file

---

## Utility Scripts

### generate_chainspec.sh

| Property | Value |
|----------|-------|
| **Path** | `scripts/generate_chainspec.sh` |
| **Purpose** | Generate chainspec JSON from wallet files |
| **What it does** | Reads producer wallet JSON files and creates a chainspec with correct public keys |
| **Dependencies** | `bash`, `python3`, `jq` (optional) |
| **Run time** | Instant |
| **Output** | JSON to stdout or specified file |

**Usage:**
```bash
# Generate chainspec for testnet from wallet files
./scripts/generate_chainspec.sh testnet ~/.doli/testnet/producer_keys testnet.json

# Generate and output to stdout
./scripts/generate_chainspec.sh mainnet ~/.doli/mainnet/producer_keys

# Generate devnet chainspec
./scripts/generate_chainspec.sh devnet ./keys devnet.json
```

**Key features:**
- Automatically extracts public keys from wallet JSON files
- Validates public key format (64 hex chars)
- Sets network-specific parameters (timestamps, rewards, slot duration)
- Prevents manual pubkey copying errors (common source of bugs!)

**Example workflow:**
```bash
# 1. Generate wallet files
for i in 1 2 3 4 5; do
    ./target/release/doli wallet new --output producer_$i.json
done

# 2. Generate chainspec
./scripts/generate_chainspec.sh testnet . chainspec.json

# 3. Start node with chainspec
./target/release/doli-node --network testnet run --chainspec chainspec.json --producer
```

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
- 5-second slots
- 60 slots per epoch (~5 minute epochs)
- 100 DOLI block reward
- ~1M VDF iterations (~70ms)
- Dynamic genesis_time (set at network start)
- Fast grace periods (3-5s vs 15-30s on testnet)

**Expected results:**
- Early epochs: Node 1 dominates (was only producer)
- After Node 2 joins: 50/50 distribution
- After Node 3 joins: ~33/33/33 equal distribution (round-robin)

**Report output:**
- `reports/rewards_report.txt` - Summary report
- `reports/detailed_rewards_report.md` - Detailed markdown report with epoch-by-epoch analysis
- Raw logs for each node in `logs/`

---

### test_whitepaper_full.sh

| Property | Value |
|----------|-------|
| **Path** | `scripts/test_whitepaper_full.sh` |
| **Purpose** | Comprehensive test of ALL WHITEPAPER.md functionalities |
| **What it tests** | Genesis, VDF, multi-node sync, producer selection, rewards, inactivity, fallback, seniority weights |
| **Dependencies** | `cargo build --release`, `doli-node`, `doli-cli` |
| **Run time** | ~5-10 minutes |
| **Output** | `/tmp/doli-whitepaper-test-*/` (logs, reports) |

**Usage:**
```bash
./scripts/test_whitepaper_full.sh
```

**Test categories:**
1. Genesis & Distribution (no premine, genesis message)
2. VDF & Proof of Time (computation time, slot progression)
3. Multi-node Network (sync, same genesis)
4. Producer Selection (round-robin, schedule)
5. Epoch Rewards (distribution, amounts)
6. Inactivity Handling (removal, reactivation)
7. Fallback Mechanism (secondary producers)
8. Chain Synchronization (height sync)
9. Seniority Weights (time-based weight progression)

**Output:**
- `reports/summary.txt` - Test results summary
- `logs/node*.log` - Individual node logs

**See also:** `docs/whitepaper_test_plan.md` for detailed manual test procedures.

---

### test_critical_features.sh

| Property | Value |
|----------|-------|
| **Path** | `scripts/test_critical_features.sh` |
| **Purpose** | E2E validation of critical whitepaper features on REAL devnet nodes |
| **What it tests** | Block production, reward maturity, round-robin selection, sync, seniority, inactivity, VDF timing, genesis, epochs |
| **Dependencies** | `cargo build --release`, `doli-node`, `doli-cli` |
| **Run time** | ~3-5 minutes |
| **Output** | `/tmp/doli-critical-test-*/` (logs, summary) |

**Usage:**
```bash
./scripts/test_critical_features.sh
```

**Test categories:**
1. Network Setup & Block Production
2. Reward Maturity (Coinbase Lockup)
3. Producer Selection (Deterministic Round-Robin)
4. Chain Synchronization Across Nodes
5. Seniority Weight System
6. Inactivity Detection
7. VDF Timing Verification
8. Genesis Block Verification
9. Transaction Fee System
10. Epoch Transitions

**Output:**
- `summary.txt` - Test results summary
- `logs/node*.log` - Individual node logs

---

## Governance & Auto-Update Scripts

### test_12node_governance.sh

| Property | Value |
|----------|-------|
| **Path** | `scripts/test_12node_governance.sh` |
| **Purpose** | Test 12-node network with era progression and governance system |
| **What it tests** | Multi-node sync, era transitions, vote submission RPC |
| **Dependencies** | `cargo`, `doli-node`, `doli-cli`, `jq` |
| **Run time** | ~20 minutes (2 eras) |
| **Output** | `/tmp/doli-12node-governance-*/` (logs, reports) |

**Usage:**
```bash
DOLI_TEST_KEYS=1 ./scripts/test_12node_governance.sh
```

**Test scenario:**
- 5 genesis producer nodes
- 2 era progression (~20 minutes)
- Vote submission RPC endpoint testing

---

### test_autoupdate_e2e.sh

| Property | Value |
|----------|-------|
| **Path** | `scripts/test_autoupdate_e2e.sh` |
| **Purpose** | End-to-end test of complete auto-update flow with real nodes |
| **What it tests** | Version reporting, vote submission, approval/rejection flows, vote propagation |
| **Dependencies** | `cargo`, `doli-node`, `doli-cli`, `jq`, `python3` |
| **Run time** | ~2 minutes |
| **Output** | `/tmp/doli-autoupdate-e2e-*/` (logs, reports, mock server) |

**Usage:**
```bash
./scripts/test_autoupdate_e2e.sh
```

**Test scenarios:**
1. Version reporting via getNodeInfo RPC
2. Vote submission and broadcasting
3. Approval flow (< 40% veto)
4. Rejection flow (>= 40% veto)
5. Majority rejection (60% veto)
6. Vote propagation across network
7. Node sync stability during voting

**Mock server:**
- Serves `latest.json` with signed release metadata
- Uses test maintainer keys (DOLI_TEST_KEYS=1)
- Runs on port 28800

---

### test_governance_scenarios.sh

| Property | Value |
|----------|-------|
| **Path** | `scripts/test_governance_scenarios.sh` |
| **Purpose** | Test ALL governance approval/rejection scenarios with real nodes |
| **What it tests** | Veto threshold calculations, vote submission, edge cases |
| **Dependencies** | `cargo`, `doli-node`, `doli-cli`, `jq` |
| **Run time** | ~2 minutes |
| **Output** | `/tmp/doli-governance-scenarios-*/` (logs, reports) |

**Usage:**
```bash
./scripts/test_governance_scenarios.sh
```

**Test scenarios:**
| Scenario | Vetos | Percentage | Expected |
|----------|-------|------------|----------|
| 1 | 0/5 | 0% | APPROVED |
| 2 | 1/5 | 20% | APPROVED |
| 3 | 2/5 | 40% | REJECTED |
| 4 | 3/5 | 60% | REJECTED |
| 5 | 5/5 | 100% | REJECTED |
| 6 | Non-producer | N/A | Filtered |
| 7 | Duplicate | N/A | Latest wins |
| 8 | Boundary | 20% | APPROVED |

---

### test_update_notification.sh

| Property | Value |
|----------|-------|
| **Path** | `scripts/test_update_notification.sh` |
| **Purpose** | Test the mandatory update notification system and CLI commands |
| **What it tests** | Update status display, vote auto-detection, CLI commands |
| **Dependencies** | `cargo`, `doli-node`, `doli-cli`, `jq` |
| **Run time** | ~10 seconds |
| **Output** | `/tmp/doli-notification-test-*/` (test artifacts) |

**Usage:**
```bash
./scripts/test_update_notification.sh
```

**Test scenarios:**
1. Update status shows pending update details
2. Update status shows changelog
3. Update status shows veto threshold
4. Vote command auto-detects pending version
5. Vote command creates signed veto message
6. Vote command creates signed approve message
7. Status handles no pending update
8. Vote handles no pending update

**CLI commands tested:**
- `doli-node update status` - Show pending update details
- `doli-node update vote --veto --key <file>` - Vote to veto
- `doli-node update vote --approve --key <file>` - Vote to approve

---

### test_version_enforcement.sh

| Property | Value |
|----------|-------|
| **Path** | `scripts/test_version_enforcement.sh` |
| **Purpose** | Test the "No Update = No Produce" version enforcement system |
| **What it tests** | Veto period, grace period, enforcement, apply command |
| **Dependencies** | `cargo`, `doli-node`, `doli-cli`, `jq` |
| **Run time** | ~10 seconds |
| **Output** | `/tmp/doli-enforcement-test-*/` (test artifacts) |

**Usage:**
```bash
./scripts/test_version_enforcement.sh
```

**Test scenarios:**
1. Veto period status display
2. Grace period status after approval
3. Enforcement active status (outdated warning)
4. Update apply command behavior
5. Apply rejected for unapproved updates
6. Timeline display in status
7. **Security: --force cannot bypass veto period**

**Update phases tested:**
- Day 0-7: Veto period (can vote to reject)
- Day 7-9: Grace period (48h to update)
- Day 9+: Enforcement (outdated nodes cannot produce)

---

## Utility Scripts

### update.sh

| Property | Value |
|----------|-------|
| **Path** | `scripts/update.sh` |
| **Purpose** | Manual binary update from GitHub Releases |
| **What it does** | Downloads, verifies, and installs doli-node binary |
| **Dependencies** | `curl`, `sha256sum` |
| **Run time** | ~30 seconds |

**Usage:**
```bash
# Update to latest version
./scripts/update.sh

# Update to specific version
./scripts/update.sh v1.0.1

# Via curl (no clone required)
curl -L https://raw.githubusercontent.com/e-weil/doli/main/scripts/update.sh | bash
curl -L https://raw.githubusercontent.com/e-weil/doli/main/scripts/update.sh | bash -s v1.0.1
```

**Features:**
- Detects platform automatically (linux-x64, linux-arm64, macos-x64, macos-arm64)
- Downloads from GitHub Releases CDN
- Verifies SHA-256 hash before installation
- Creates backup of current binary
- Requires sudo for installation to `/usr/local/bin`

**Environment variables:**
- `INSTALL_DIR` - Installation directory (default: `/usr/local/bin`)

---

## Quick Reference

| Script | Nodes | Duration | Purpose |
|--------|-------|----------|---------|
| `build_release.sh` | 0 | ~10-30 min | **Build release binaries** |
| `smoke_test_release.sh` | 1 | ~30-60 sec | **Release verification** |
| `update.sh` | 0 | ~30 sec | **Manual binary update** |
| `launch_testnet.sh` | 2 | Interactive | Basic devnet |
| `stress_test_600.sh` | 600 | Long | Scalability |
| `test_staggered_validator_rewards.sh` | 10 | ~10 min | Staggered rewards |
| `test_validator_rewards_simple.sh` | 3 | ~5 min | Simple rewards |
| `test_3node_proportional_rewards.sh` | 3 | ~6 min | Proportional rewards |
| `test_devnet_3node_rewards.sh` | 3 | ~5 min | Detailed epoch rewards |
| `test_whitepaper_full.sh` | 3 | ~5-10 min | **Complete WHITEPAPER verification** |
| `test_critical_features.sh` | 3 | ~3-5 min | **Real devnet E2E validation** |
| `test_12node_governance.sh` | 5 | ~20 min | Era progression & governance |
| `test_governance_scenarios.sh` | 5 | ~2 min | **All governance scenarios** |
| `test_autoupdate_e2e.sh` | 5 | ~2 min | **E2E auto-update flow** |
| `test_update_notification.sh` | 0 | ~10 sec | **Update notification CLI** |
| `test_version_enforcement.sh` | 0 | ~10 sec | **Version enforcement system** |

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
