# CLAUDE.md - Project Brain

## MANDATORY RULE: No unsupervised changes

NEVER do any of the following without explicit approval from Ivan FIRST:

1. **No code changes** — Do not modify, add, or delete any code without showing the exact diff and getting approval
2. **No deployments** — Do not push, build, deploy, or restart any node without explicit "proceed" from Ivan
3. **No destructive actions** — Do not kill processes, wipe data, delete files, or stop services without explicit permission
4. **No design changes** — Do not change the approach (e.g., switching from exclusive windows to minimum-delay) without explaining why and getting approval

When asked to implement something:
1. Show the plan
2. Show the exact code changes (diff)
3. WAIT for approval
4. Only then execute

If something is broken after deployment, STOP and report. Do not attempt fixes autonomously.

## CRITICAL: Production Node Protection

**NEVER stop, restart, kill, or deploy to N1 or N2 (omegacortex.ai) while any other node is syncing or broken.** N1 and N2 are the chain tip — if they go down while N3/N4/N5 are syncing, the entire network loses its only source of truth. Only touch N1/N2 when ALL nodes are fully synchronized and producing.

## MANDATORY: Ops Runbook

**Before ANY deployment, node management, upgrade, or infrastructure task**, read `.claude/skills/doli-ops/SKILL.md`. It contains exact CLI syntax (flag order matters!), node SSH details, deployment checklists, and troubleshooting procedures.

# FIRST PRINCIPLE:
Elon Musk says: The best engine part is the one you can remove. In other words, less is more! Let this be our approach, even for the most complex problems: Always opt for the simplest solution without compromising safety.

# SCALE PRINCIPLE:
Always imagine **thousands of producer nodes** in **10-second slot windows** before architecting any fix or solution. This applies to every system: gossip propagation, sync recovery, fork detection, block validation. If a design doesn't work at scale, it doesn't work.

## 🚨 CRITICAL RULES

1. **Environment**: All commands **MUST** run via Nix:
   `nix --extra-experimental-features "nix-command flakes" develop --command bash -c "<command>"`

2. **Truth Hierarchy**: `WHITEPAPER.md` (Law) > `specs/` (Tech) > `docs/` (User) > Code.
   - Conflicts resolve top-down. Code must conform to specs, not the reverse.
   - If code contradicts specs → code is wrong, fix the code.
   - If specs contradict whitepaper → specs are wrong, fix the specs.

3. **Pre-Commit Gate** (Execute in order, all steps mandatory):
   
   | Step | Action | Condition |
   |------|--------|-----------|
   | 1 | **Update `specs/`** | If technical behavior, API, constants, or protocol changed |
   | 2 | **Update `docs/`** | If user-facing behavior, CLI, or configuration changed |
   | 3 | **Update `CLAUDE.md`** | If architecture, crate structure, or constants changed |
   | 4 | **Verify build** | `cargo build && cargo clippy -- -D warnings && cargo fmt --check` |
   | 5 | **Verify tests** | `cargo test` |
   | 6 | **Commit** | Only after steps 1-5 pass |

   **Commit command** (only after all steps pass):
```bash
   git add -A && git commit -m "<type>(<scope>): <description>"
```

4. **Documentation Sync Rules**:
   - `specs/` = Source of truth for implementation. Code divergence = bug.
   - `docs/` = User-facing. Must reflect current CLI, config, and behavior.
   - New feature without docs update = **incomplete implementation**.
   - Modification without docs review = **potential regression**.
   - When updating docs, check related files for consistency.

5. **Bug Fixing**: Use `/fix-bug` workflow. **No masking symptoms.**
   - Root cause analysis required before fix.
   - If fix changes behavior → docs update required (Rule 3).
   - If fix reveals spec inconsistency → update specs first.
   - **Bug Reports**: When investigating complex bugs, create `REPORT.md` in repo root.
   - **On Resolution**: Move resolved bug reports to `docs/legacy/bugs/REPORT_<BUG_NAME>.md`
     - Example: `REPORT.md` → `docs/legacy/bugs/REPORT_UTXO_ROCKSDB_CRASH.md`

     Add this as **Rule 5.1** (after the existing Bug Fixing rule), or append it to Rule 5:

 **CLI Issue Tracking** (`CLI.md`):
   - When using the CLI and encountering a **bug**, **missing sub-command**, or **constraint/limitation**, log it immediately in `CLI.md` at the repo root.
   
     ## CLI Issues

     ### [DATE] - <Short Description>
     - **Type**: Bug | Missing Command | Constraint
     - **Command**: `doli-cli <subcommand>` (what was attempted)
     - **Observed**: What happened (or didn't)
     - **Expected**: What should happen
     - **Priority**: Low | Medium | High
     - **Status**: Open | In Progress | Resolved
     ```
   - **Rules**:
     - One entry per issue, append-only (don't remove — mark as `Resolved` and reference the fixing commit).
     - On resolution, keep the entry in `CLI.md` with updated status — do **not** move to `docs/legacy/bugs/` (that's for deep investigation reports only).
     - Review `CLI.md` before any CLI-related PR to check for low-hanging fixes.
   - **Examples**:
     - Missing `doli-cli wallet export` sub-command → log it.
     - `doli-cli bond status` returns wrong penalty tier → log it.
     - `--format json` flag silently ignored → log it.

6. **Output Filtering**: Always filter verbose output:
Apply always outour redirection to a /tmp/ folder to avoid polluting the console to later apply filters.
  command > /tmp/cmd_output.log 2>&1 && grep -iE "error|warn|fail|pass" /tmp/cmd_output.log | head -20

## 🛠 Commands (Wrapped)

All commands implicitly wrapped in Nix develop shell.

| Action | Command |
|--------|---------|
| **Build** | `cargo build` |
| **Build Release** | `cargo build --release` |
| **Lint** | `cargo clippy -- -D warnings` |
| **Format Check** | `cargo fmt --check` |
| **Format Fix** | `cargo fmt` |
| **Test All** | `cargo test` |
| **Test Crate** | `cargo test -p <crate>` (e.g., `cargo test -p core`) |
| **Fuzz** | `cd testing/fuzz && cargo +nightly fuzz run <target>` |
| **Run Node** | `cargo run -p doli-node -- run` |
| **Run Node (Testnet)** | `cargo run -p doli-node -- --network testnet run` |
| **Run Wallet** | `cargo run -p doli-cli -- <command>` |
| **Full Pre-Commit** | `cargo build && cargo clippy -- -D warnings && cargo fmt --check && cargo test` |

## 🧠 System Architecture

**Type**: Proof of Time (PoT)
**Resource**: Time (VDF)
**Selection**: Deterministic bond-weighted round-robin
**Consensus**: Heaviest chain (Seniority-weighted)
**Engine**: RocksDB + libp2p + Axum

### Crates & Responsibilities

| Crate | Purpose | Key Files |
|-------|---------|-----------|
| `core` | Consensus, Types, Scheduler | `consensus.rs`, `scheduler.rs`, `validation.rs`, `discovery/` |
| `crypto` | BLAKE3, Ed25519, Merkle | `hash.rs`, `keys.rs`, `merkle.rs` (Domain separated) |
| `vdf` | Wesolowski (Reg) & Hash-Chain (Block) | `vdf.rs`, `proof.rs` (GMP/Rug) |
| `network` | Gossipsub, Sync, Equivocation | `service.rs`, `sync/`, `gossip.rs` |
| `storage` | RocksDB (Headers, Bodies, UTXO) | `block_store.rs`, `utxo.rs`, `producer.rs` |
| `mempool` | Tx Pool, Double-spend checks | `pool.rs`, `policy.rs` |
| `updater` | 3/5 Multisig Auto-Update | `lib.rs`, `vote.rs` |

### ⚙️ Consensus Constants

| Param | Mainnet | Devnet | Note |
|-------|---------|--------|------|
| **Slot** | 10s | 10s | `SLOT_DURATION` |
| **Epoch** | 360 slots (1h) | 360 | `SLOTS_PER_EPOCH` |
| **Era** | 12.6M slots (~4y) | 576 | Halving trigger |
| **VDF Block** | 800K iter (~55ms) | 800K | `T_BLOCK` |
| **VDF Reg** | 600M iter (~10m) | 5M | `T_REGISTER_BASE` (Anti-Sybil) |
| **Bond** | 10 DOLI | 1 DOLI | `BOND_UNIT` |
| **Unbond** | 7 days | 10m | `WITHDRAWAL_DELAY_SLOTS` |
| **Selection** | `slot % bonds` | - | Sequential 2s exclusive windows |
| **Fallback** | 5 ranks | 5 ranks | `MAX_FALLBACK_RANKS`, `FALLBACK_TIMEOUT_MS=2000` |
| **Clock Drift** | 1s / 200ms | 1s / 200ms | `MAX_DRIFT=1`, `MAX_DRIFT_MS=200` |

### 🌐 Network & Ports

| Net | ID | Port (P2P/RPC) | Magic | Prefix | Genesis |
|-----|----|----------------|-------|--------|---------|
| Main | 1 | 30303 / 8545 | `D0 11 00 01` | `doli` | 2026-02-01 |
| Test | 2 | 40303 / 18545 | `D0 11 00 02` | `tdoli` | 2026-01-29 |
| Dev | 99 | 50303 / 28545 | `D0 11 00 63` | `ddoli` | Dynamic |

### 🔧 Environment Configuration

Network parameters configurable via `~/.doli/{network}/.env`:

```bash
# Networking (all networks)
DOLI_P2P_PORT, DOLI_RPC_PORT, DOLI_METRICS_PORT, DOLI_BOOTSTRAP_NODES

# Timing (devnet only - locked for mainnet)
DOLI_SLOT_DURATION, DOLI_GENESIS_TIME, DOLI_UNBONDING_PERIOD

# Economics (devnet only - locked for mainnet)
DOLI_BOND_UNIT, DOLI_INITIAL_REWARD, DOLI_BLOCKS_PER_YEAR

# VDF (devnet only - locked for mainnet)
DOLI_VDF_ITERATIONS, DOLI_HEARTBEAT_VDF_ITERATIONS

# Fallback (devnet only - locked for mainnet)
DOLI_FALLBACK_TIMEOUT_MS, DOLI_MAX_FALLBACK_RANKS, DOLI_NETWORK_MARGIN_MS
```

**Locked for mainnet**: Slot duration, genesis time, bond unit, emission, VDF iterations, blocks/year, fallback timing.
**Files**: `.env.example.{devnet,testnet,mainnet}` in repo root.
**Code**: `crates/core/src/network_params.rs`

## 💰 Economics (Deflationary)

- **Supply**: ~25.2M DOLI
- **Rewards**: 100% to producer
- **Halving**: Every Era (~4y)
- **Weights**: Year 0-1 (1x) → Year 3+ (4x)
- **Fork Choice**: Heaviest weight
- **Burnt**: Slashing (100%), Early Withdrawal (75%→0% over 4y), Reg Fees

### Bond Vesting (Withdrawal Penalty)

| Age | Penalty |
|-----|---------|
| <1y | 75% Burn |
| 1-2y | 50% Burn |
| 2-3y | 25% Burn |
| 3y+ | 0% |

## 🛡 Validation & Security

### Block Validation
- Version = 1
- Timestamp advances from parent
- Max size: 1MB + header overhead
- Merkle root matches transactions
- VDF proof valid for slot

### Transaction Validation
- Signature valid (Ed25519)
- Inputs exist and unspent
- No double-spend within block
- Malleability protection: Signature excluded from TxID hash

### Slashing
- **Trigger**: Double-production (same slot, different blocks)
- **Penalty**: 100% bond BURN
- **Detection**: Network gossip + SignedSlotsDB local tracking
- **Proof**: `EquivocationProof` (two signed headers, same slot, same producer)

### Governance
- **Maintainers**: First 5 registered producers (on-chain); `BOOTSTRAP_MAINTAINER_KEYS` as fallback pre-sync
- **Updates**: 3/5 multisig required for release signatures
- **Veto**: 40% stake can block (7-day voting period)

## 📂 File Map

### Core (`crates/core/src/`)
| File | Purpose |
|------|---------|
| `consensus.rs` | Constants, Bond logic, Chain parameters |
| `scheduler.rs` | Round-robin producer selection (`select_producer`) |
| `validation.rs` | 37 error types (`InvalidTimestamp`, `DoubleSpend`, etc.) |
| `discovery/` | Signed announcements, CRDT (`gset.rs`), Bloom filters |

### Network (`crates/network/src/sync/`)
| File | Purpose |
|------|---------|
| `manager.rs` | Sync orchestration |
| `reorg.rs` | Fork choice (Max depth = 100) |
| `equivocation.rs` | Double-production detection (`EquivocationProof`) |
| `headers.rs` | Header-first sync download |
| `bodies.rs` | Body download after headers |

### Storage (`crates/storage/src/`)
| File | Purpose |
|------|---------|
| `block_store.rs` | RocksDB block storage |
| `utxo.rs` | UTXO set (HashMap driven) |
| `producer.rs` | Producer registry |

**Column Families**: `headers`, `bodies`, `height_index`, `slot_index`, `presence`

### Transaction Types (`TxType`)

| Value | Type | Description |
|-------|------|-------------|
| 0 | Transfer | Standard value transfer |
| 1 | Register | Producer registration |
| 2 | Exit | Producer exit request |
| 4 | ClaimBond | Claim unbonded stake |
| 5 | Slash | Slash equivocating producer |
| 6 | Coinbase | Block reward |
| 7 | AddBond | Add to existing bond |
| 8 | WithdrawalRequest | Request early withdrawal |
| 9 | WithdrawalClaim | Claim withdrawal |
| 10 | EpochReward | Epoch-level rewards |
| 11 | MaintainerAdd | Add maintainer (governance) |
| 12 | MaintainerRemove | Remove maintainer (governance) |

## 📋 Documentation Structure

### `specs/` - Technical Specifications
- Protocol details, message formats, algorithms
- **Audience**: Developers, implementers
- **Update when**: Code behavior, API, constants, or protocol changes

### `docs/` - User Documentation
- CLI usage, configuration, tutorials
- **Audience**: Node operators, users
- **Update when**: User-facing behavior, CLI, or configuration changes

### `WHITEPAPER.md` - Protocol Law
- Economic model, consensus philosophy, security model
- **Audience**: Everyone
- **Update when**: Fundamental protocol changes (rare, requires governance)

### `docs/legacy/bugs/` - Resolved Bug Reports
- Investigation reports for complex bugs (root cause, fix, test results)
- **Naming**: `REPORT_<BUG_NAME>.md` (e.g., `REPORT_UTXO_ROCKSDB_CRASH.md`)
- **Workflow**: Create `REPORT.md` at repo root during investigation → move here on resolution

## 🖥 Node Operations & Deployment

### Mainnet Node Inventory

| Node | Host | IP | SSH | Ports (P2P/RPC/Metrics) | Data Dir | Binary | Logs |
|------|------|----|-----|------------------------|----------|--------|------|
| **N1** | omegacortex | 72.60.228.233 | `ssh ilozada@omegacortex.ai` | 30303 / 8545 / 9090 | `~/.doli/mainnet/node1/data` | `~/repos/doli/target/release/doli-node` | `/tmp/node1.log` |
| **N2** | omegacortex | same | same host | 30304 / 8546 / 9091 | `~/.doli/mainnet/node2/data` | same binary | `/tmp/node2.log` |
| **N3** | omegacortex | same | same host | 30305 / 8547 / 9092 | `~/.doli/mainnet/node3/data` | same binary | `/tmp/node3.log` |
| **N4** | pro-KVM1 | 72.60.70.166 | `ssh ilozada@omegacortex.ai` then `ssh -p 50790 ilozada@72.60.70.166` | 30303 / 8545 / 9090 | `/home/isudoajl/.doli/mainnet/` | `/opt/doli/target/release/doli-node` | `/var/log/doli-node.log` |
| **N5** | fpx | 72.60.115.209 | `ssh ilozada@omegacortex.ai` then `ssh -p 50790 ilozada@72.60.115.209` | 30303 / 8545 / 9090 | `/home/isudoajl/.doli/mainnet/` | `/opt/doli/target/release/doli-node` | `/var/log/doli-node.log` |

**Key differences:**
- **N1/N2/N3** (omegacortex): Have Rust toolchain, full repo clone. `cargo build --release` works. All share the same compiled binary. SSH user is `ilozada`.
- **N4/N5** (remote VMs): **No Rust toolchain.** Binary deployed via SCP from omegacortex. Cannot compile locally.
- **N4/N5 SSH**: Only reachable via omegacortex as jump host. Direct SSH from local machine fails.
- **N4/N5 process user**: `isudoajl` (not `ilozada`). SSH as `ilozada`, use `sudo -u isudoajl` to run the node process.
- **N4/N5 data dir**: Files live directly in `~/.doli/mainnet/` (no `data/` subdirectory). Key files are in `keys/producer_4.json` and `keys/producer_5.json`.

### Producer Key Registry (AUTHORITATIVE)

> **CRITICAL**: These are the ONLY valid producer keys. The `BOOTSTRAP_MAINTAINER_KEYS` in `crates/updater/src/lib.rs` are STALE and do NOT match these keys. See the warning below.

| Node | Host | Key File | Public Key (Ed25519) |
|------|------|----------|----------------------|
| **N1** | omegacortex | `~/.doli/mainnet/keys/producer_1.json` | `202047256a8072a8b8f476691b9a5ae87710cc545e8707ca9fe0c803c3e6d3df` |
| **N2** | omegacortex | `~/.doli/mainnet/keys/producer_2.json` | `effe88fefb6d992a1329277a1d49c7296d252bbc368319cb4bc061119926272b` |
| **N3** | omegacortex | `~/.doli/mainnet/keys/producer_3.json` | `54323cefd0eabac89b2a2198c95a8f261598c341a8e579a05e26322325c48c2b` |
| **N4** | pro-KVM1 | `/home/isudoajl/.doli/mainnet/keys/producer_4.json` | `a1596a36fd3344bae323f8cdb7a0be7f4ca2a118de3cca184b465608e9beda1d` |
| **N5** | fpx | `/home/isudoajl/.doli/mainnet/keys/producer_5.json` | `c5acb5b359c7a2093b8c788862cf57c5418e94de8b1fc6a254dc0862ee3c03a9` |

**Retired keys** (produced early blocks, no longer active — funds still held):

| Key | Public Key | Approx Blocks | Balance |
|-----|------------|---------------|---------|
| U1 | `fd2f9af2d073c52a11c0994f0f3df607cb19f13cbabf1e30f1f02525d4cda691` | ~1,601 | 1,601 DOLI |
| U2 | `805b7411209cca4465892c483131cec07390befae77d5bca6930f4d55b07eff5` | ~1,644 | 1,644 DOLI |
| U3 | `a44df67f10564a221b9bd6f2e020556940b5bf7036cab7e896a52a2d69a4e272` | ~1,517 | 1,517 DOLI |

### ⚠️ BOOTSTRAP_MAINTAINER_KEYS Mismatch (ACTION REQUIRED)

The `BOOTSTRAP_MAINTAINER_KEYS` hardcoded in `crates/updater/src/lib.rs:84-96` do **NOT** match the current producer keys:

```
Bootstrap Key 1: 721d2bc74ced1842...  ≠  N1: 202047256a8072a8...
Bootstrap Key 2: d0c62cb4e143d548...  ≠  N2: effe88fefb6d992a...
Bootstrap Key 3: 9fac605a1ebf2acf...  ≠  N3: 54323cefd0eabac8...
Bootstrap Key 4: 97bdb0a9a52d4ed1...  ≠  N4: a1596a36fd3344ba...
Bootstrap Key 5: 82ed55afabfe38d8...  ≠  N5: c5acb5b359c7a209...
```

**Impact**: The auto-update system's fallback signature verification uses keys that nobody controls. Until these are updated, release signature verification against bootstrap keys will fail. On-chain maintainer derivation (first 5 registered producers) should work independently, but the fallback path is broken.

### ⚠️ Chainspec Rules (CONSENSUS-CRITICAL)

> **HARD LESSON (2026-02-22):** N4/N5 had no `chainspec.json` → different `genesis_timestamp` → slot schedule diverged → chain fork. N4 reorged from 37K to 19K blocks.

1. **Chainspec is embedded in the binary** (`chainspec.mainnet.json` via `include_str!`)
2. On first start, if no `chainspec.json` exists in data dir, the binary writes it from embedded
3. Priority: `--chainspec /path` > `$DATA_DIR/chainspec.json` > embedded fallback
4. **Producer nodes exit(1) without chainspec** — code guard in `main.rs`
5. The **canonical chainspec** lives at repo root: `chainspec.mainnet.json`
6. **NEVER** change `genesis.timestamp` or `consensus.slot_duration` — this breaks consensus

### DNS / Bootstrap

| Record | Type | Resolves to | Purpose |
|--------|------|-------------|---------|
| `seed1.doli.network` | A | `72.60.228.233` | Default bootstrap (N1) |
| `seed2.doli.network` | A | `72.60.228.233` | Default bootstrap (N1) |

These are hardcoded in `crates/core/src/network_params.rs` as default mainnet bootstrap nodes. Nodes started without `--bootstrap` will use these automatically.

### Deployment — Full Procedure

#### Step 1: Build on omegacortex

```bash
ssh ilozada@omegacortex.ai "cd ~/repos/doli && git pull && cargo build --release"
```

This updates the binary for N1/N2/N3 (they share `~/repos/doli/target/release/doli-node`). Running nodes keep the old binary in memory until restarted.

#### Step 2: Deploy binary to N4/N5 via SCP

```bash
# Compress (23MB → 8.6MB)
ssh ilozada@omegacortex.ai "gzip -c ~/repos/doli/target/release/doli-node > /tmp/doli-node.gz"

# Copy to N4
ssh ilozada@omegacortex.ai "scp -P 50790 /tmp/doli-node.gz ilozada@72.60.70.166:/tmp/"
# Copy to N5
ssh ilozada@omegacortex.ai "scp -P 50790 /tmp/doli-node.gz ilozada@72.60.115.209:/tmp/"

# Install on N4
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'gunzip -f /tmp/doli-node.gz && sudo cp /tmp/doli-node /opt/doli/target/release/doli-node && sudo chmod +x /opt/doli/target/release/doli-node'"
# Install on N5
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'gunzip -f /tmp/doli-node.gz && sudo cp /tmp/doli-node /opt/doli/target/release/doli-node && sudo chmod +x /opt/doli/target/release/doli-node'"
```

#### Step 3: Stop nodes

**N1/N2/N3** (omegacortex — kill by PID to avoid hitting other nodes):
```bash
# Find PIDs
ssh ilozada@omegacortex.ai "pgrep -la doli-node"

# Kill specific node (replace PID)
ssh ilozada@omegacortex.ai "kill <PID>"

# Or kill by data-dir pattern:
ssh ilozada@omegacortex.ai "kill \$(pgrep -f 'data-dir.*node3')"   # N3 only
```

**N4/N5** (via jump host):
```bash
# N4
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo kill \$(pgrep doli-node) 2>/dev/null; echo done'"
# N5
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo kill \$(pgrep doli-node) 2>/dev/null; echo done'"
```

Wait 3s, then verify stopped:
```bash
ssh ilozada@omegacortex.ai "pgrep -la doli-node"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo pgrep -la doli-node || echo stopped'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo pgrep -la doli-node || echo stopped'"
```

#### Step 4: Start nodes

**N1** (omegacortex, relay server — start first, it's the bootstrap):
```bash
ssh ilozada@omegacortex.ai "nohup /home/ilozada/repos/doli/target/release/doli-node \
  --data-dir /home/ilozada/.doli/mainnet/node1/data run \
  --producer --producer-key /home/ilozada/.doli/mainnet/keys/producer_1.json \
  --chainspec /home/ilozada/.doli/mainnet/chainspec.json \
  --no-auto-update --yes --force-start --relay-server \
  </dev/null >/tmp/node1.log 2>&1 &"
```

**N2** (omegacortex, port offset):
```bash
ssh ilozada@omegacortex.ai "nohup /home/ilozada/repos/doli/target/release/doli-node \
  --data-dir /home/ilozada/.doli/mainnet/node2/data run \
  --producer --producer-key /home/ilozada/.doli/mainnet/keys/producer_2.json \
  --chainspec /home/ilozada/.doli/mainnet/chainspec.json \
  --no-auto-update --yes --force-start \
  --p2p-port 30304 --rpc-port 8546 --metrics-port 9091 \
  --bootstrap /ip4/127.0.0.1/tcp/30303 --relay-server \
  </dev/null >/tmp/node2.log 2>&1 &"
```

**N3** (omegacortex, port offset):
```bash
ssh ilozada@omegacortex.ai "nohup /home/ilozada/repos/doli/target/release/doli-node \
  --data-dir /home/ilozada/.doli/mainnet/node3/data run \
  --producer --producer-key /home/ilozada/.doli/mainnet/keys/producer_3.json \
  --chainspec /home/ilozada/.doli/mainnet/chainspec.json \
  --no-auto-update --yes --force-start \
  --p2p-port 30305 --rpc-port 8547 --metrics-port 9092 \
  --bootstrap /ip4/127.0.0.1/tcp/30303 --relay-server \
  </dev/null >/tmp/node3.log 2>&1 &"
```

**N4** (remote VM):
```bash
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo -u isudoajl bash -c \"nohup /opt/doli/target/release/doli-node run \
  --producer --producer-key /home/isudoajl/.doli/mainnet/producer.json \
  --bootstrap /ip4/72.60.228.233/tcp/30303 \
  --p2p-port 30303 --rpc-port 8545 --metrics-port 9090 --yes \
  </dev/null >/var/log/doli-node.log 2>&1 &\"'"
```

**N5** (remote VM):
```bash
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo -u isudoajl bash -c \"nohup /opt/doli/target/release/doli-node run \
  --producer --producer-key /home/isudoajl/.doli/mainnet/producer.json \
  --bootstrap /ip4/72.60.228.233/tcp/30303 \
  --p2p-port 30303 --rpc-port 8545 --metrics-port 9090 --yes \
  </dev/null >/var/log/doli-node.log 2>&1 &\"'"
```

#### Step 5: Verify

```bash
# All nodes running
ssh ilozada@omegacortex.ai "pgrep -la doli-node"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo pgrep -la doli-node'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo pgrep -la doli-node'"

# All nodes same height and hash (run twice 15s apart, height should advance)
ssh ilozada@omegacortex.ai "for p in 8545 8546 8547; do \
  echo \"N\$((p-8544)): \$(curl -s -X POST http://127.0.0.1:\$p \
  -H 'Content-Type: application/json' \
  -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}' \
  | jq -c '.result | {h: .bestHeight, s: .bestSlot, hash: .bestHash[0:16]}')\"; done"
```

### Wipe & Resync (When a Node is Forked)

**N1/N2/N3** (omegacortex — replace `node3` with `node1`/`node2` as needed):
```bash
# 1. Stop the node
ssh ilozada@omegacortex.ai "kill \$(pgrep -f 'data-dir.*node3')"

# 2. Wipe chain state
ssh ilozada@omegacortex.ai "rm -f ~/.doli/mainnet/node3/data/chain_state.bin \
  ~/.doli/mainnet/node3/data/producers.bin \
  ~/.doli/mainnet/node3/data/utxo.bin && \
  rm -rf ~/.doli/mainnet/node3/data/blocks/ \
  ~/.doli/mainnet/node3/data/signed_slots.db/"

# 3. Restart (see Step 4 above)
```

**N4/N5** (remote VMs — data lives directly in `~/.doli/mainnet/`, no `data/` subdirectory):
```bash
# 1. Stop
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo kill \$(pgrep doli-node) 2>/dev/null; echo done'"

# 2. Wipe chain state (NOTE: path is /home/isudoajl/.doli/mainnet/ — no data/ subdir)
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo rm -f \
  /home/isudoajl/.doli/mainnet/chain_state.bin \
  /home/isudoajl/.doli/mainnet/producers.bin \
  /home/isudoajl/.doli/mainnet/utxo.bin; \
  sudo rm -rf /home/isudoajl/.doli/mainnet/blocks/ \
  /home/isudoajl/.doli/mainnet/signed_slots.db/; echo wiped'"

# 3. Restart (see Step 4 above)
```

### Consensus-Critical vs Rolling Upgrades

| Change type | Examples | Deploy strategy |
|-------------|----------|----------------|
| **Consensus-critical** | Block validation, scheduling, VDF, economics, tx processing | Stop ALL nodes simultaneously, replace binary, start all |
| **Non-consensus** | Sync, networking, RPC, logging, metrics | Rolling: one node at a time, verify health before next |

**For consensus-critical changes:** All nodes MUST run the same binary version simultaneously to prevent forks. Stop all 5, deploy, start N1 first (bootstrap), then N2, then N3/N4/N5.

### Snap Sync

When a node is >1000 blocks behind with 3+ peers, it uses snap sync: downloads a full state snapshot instead of replaying 40K+ blocks with VDF verification. Takes seconds instead of hours.

- Wire protocol: `GetStateRoot`/`StateRoot` for quorum, `GetStateSnapshot`/`StateSnapshot` for download
- State root: `H(H(chain_state) || H(utxo_set) || H(producer_set))` verified by 2+ peers
- Falls back to header-first sync if <3 peers or quorum fails
- Logs: `[SNAP_SYNC]` prefix