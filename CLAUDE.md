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

## 🔄 Implicit Workflow Routing

When a task is requested, automatically detect the type and follow the appropriate agent pipeline. Explicit `/workflow-*` commands are still available but never required.

### Detection Rules

| Task Signal | Pipeline | Agent Chain |
|---|---|---|
| New functionality: "add X", "implement Y", "create Z" | **feature** | analyst → architect → test-writer → developer → compiler → reviewer |
| Bug/error: "fix", "bug", "broken", "crash", error report | **bugfix** | analyst → test-writer → developer → compiler → reviewer |
| Improvement: "refactor", "optimize", "improve", "clean up" | **improve** | analyst → test-writer → developer → compiler → reviewer |
| Code review: "audit", "review code", "security check" | **audit** | reviewer (read-only) |
| Documentation: "update docs", "document", "write specs" | **docs** | architect (docs mode) |
| Drift fix: "sync", "drift", "specs outdated" | **sync** | architect (sync mode) |
| New project from scratch | **new** | analyst → architect → test-writer → developer → compiler → reviewer |

### Compiler Gate (Automatic)

Between Developer and Reviewer, always run the Pre-Commit Gate (Rule 3, steps 4-5):
```
cargo build && cargo clippy -- -D warnings && cargo fmt --check && cargo test
```
If any step fails → return to Developer. Never pass broken code to Reviewer.

### Skip Conditions

Do NOT activate the pipeline for:
- **Trivial changes**: typo fix, 1-3 line edit, config tweak, single constant change
- **Questions / research / exploration**: reading code, explaining behavior, investigating
- **Ops tasks**: deployment, node management, monitoring (use ops runbook instead)
- **Ambiguous requests**: ask for clarification first, then route

### Agent Execution

Each agent runs as a Claude Code subagent (via Task tool) with its own context window:
- Agents defined in `.claude/agents/` — each has scoped tools and model
- Workflows defined in `.claude/commands/` — each specifies the agent chain
- `--scope` parameter limits context to a specific crate/module
- When no scope is provided, the analyst determines the minimal scope needed
- All agents follow the Source of Truth hierarchy: Codebase > specs/ > docs/

### Pipeline Flow

```
Task detected
  ↓
🔍 Analyst       → Questions, scopes, reads code, generates requirements
  ↓
🏗️ Architect     → Designs architecture, updates specs/ and docs/
  ↓
🧪 Test Writer   → Writes failing tests (TDD red phase)
  ↓
💻 Developer     → Implements until green, commits each module
  ↓
🔨 Compiler      → cargo build + clippy + fmt + test (automatic gate)
  ↓
👁️ Reviewer      → Audits code, security, performance, specs drift
  ↓
📦 Git           → Conventional commit after approval
```

Shorter pipelines (bugfix, improve) skip Architect. Audit/docs/sync use single agents.

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
| `crypto` | BLAKE3, Ed25519, Merkle, Bech32m Addresses | `hash.rs`, `keys.rs`, `merkle.rs`, `address.rs` (Domain separated) |
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

### 🏷 Address Format (Bech32m)

DOLI uses **bech32m** (BIP-350) human-readable addresses. The prefix matches `Network::address_prefix()`.

| Network | Prefix | Example |
|---------|--------|---------|
| Mainnet | `doli1` | `doli1qpzry9x8gf2tvdw0s3jn54khce6mua7l...` |
| Testnet | `tdoli1` | `tdoli1qpzry9x8gf2tvdw0s3jn54khce6mua7l...` |
| Devnet | `ddoli1` | `ddoli1qpzry9x8gf2tvdw0s3jn54khce6mua7l...` |

**Derivation**: `pubkey_hash = BLAKE3(ADDRESS_DOMAIN ∥ public_key)` → bech32m-encode with network prefix.

**Key rule**: All CLI commands and RPC methods accept **both** `doli1...` and 64-char hex. The `crypto::address::resolve()` function handles parsing:
1. `doli1...` → bech32m decode → 32-byte pubkey_hash
2. 64-char hex → raw pubkey_hash (backward compat)
3. Anything else → error with format guidance

**CLI usage**:
```bash
# Send (fee is auto-calculated, no --fee needed)
doli send doli1fznp4jddlf39qzg3kc94qvnsptrhkt0z3pehwq3cnpurk7ylauqstxsxyc 20

# Check balance
doli balance --address doli17engd6utnqs4ag6l6xme7tdhvgh6rcd8ezay5qw0vssqxyw239ts9dygef

# Old hex still works
doli balance --address f66686eb8b98215ea35fd1b79f2db7622fa1e1a7c8ba4a01cf64200311ca8957

# Producer nodes: use -w to point to the producer key file
doli -w ~/.doli/mainnet/keys/producer_1.json send doli1recipient... 20
```

**Fee**: Auto-calculated as `max(1000, inputs * 500)` units. Override with `--fee` if needed.

**Code**: `crates/crypto/src/address.rs` (encode, decode, from_pubkey, resolve)
**Dependency**: `bech32 = "0.11"` (pure Rust, zero transitive deps)

### 💼 Wallet File Format

DOLI wallets use JSON files with two versions:

| Version | Description | Seed Phrase |
|---------|-------------|-------------|
| v1 | Legacy (existing producer keys) | No |
| v2 | BIP-39 derived key (new wallets) | Separate `.seed.txt` file |

**v2 key derivation**: `Ed25519_seed = BIP39_PBKDF2("")[:32]` → `KeyPair::from_seed()`
**Seed storage**: NOT in wallet JSON — written to `<wallet>.seed.txt` at creation, user deletes after backup
**CLI commands**: `doli new` (create v2 wallet + seed file)
**Backward compat**: v1 files load unchanged
**Code**: `bins/cli/src/wallet.rs`
**Dependencies**: `bip39 = "2.1"`, `zeroize` (workspace)

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

| Node | Host | IP | SSH | Ports (P2P/RPC/Metrics) | Data Dir | Binary | Service |
|------|------|----|-----|------------------------|----------|--------|---------|
| **N1** | omegacortex | 72.60.228.233 | `ssh ilozada@omegacortex.ai` | 30303 / 8545 / 9090 | `~/.doli/mainnet/node1/data` | `~/repos/doli/target/release/doli-node` | `doli-mainnet-node1` |
| **N2** | omegacortex | same | same host | 30304 / 8546 / 9091 | `~/.doli/mainnet/node2/data` | same binary | `doli-mainnet-node2` |
| **N3** | N3-VPS | 147.93.84.44 | `ssh ilozada@omegacortex.ai` then `ssh -p 50790 ilozada@147.93.84.44` | 30303 / 8545 / 9090 | `/home/ilozada/.doli/mainnet/data` | `/home/ilozada/doli-node` | `doli-mainnet-node3` |
| **N4** | pro-KVM1 | 72.60.70.166 | `ssh ilozada@omegacortex.ai` then `ssh -p 50790 ilozada@72.60.70.166` | 30303 / 8545 / 9090 | `/home/isudoajl/.doli/mainnet/` | `/opt/doli/target/release/doli-node` | `doli-mainnet-node4` |
| **N5** | fpx | 72.60.115.209 | `ssh ilozada@omegacortex.ai` then `ssh -p 50790 ilozada@72.60.115.209` | 30303 / 8545 / 9090 | `/home/isudoajl/.doli/mainnet/` | `/opt/doli/target/release/doli-node` | `doli-mainnet-node5` |

**All nodes managed by systemd** (`sudo systemctl restart/stop/status doli-mainnet-nodeN`).

**Service files**: `/etc/systemd/system/doli-mainnet-nodeN.service`

**Logs**: `/var/log/doli/nodeN.log` — circular via logrotate (5MB max, 1 rotation). Config: `/etc/logrotate.d/doli`.

```bash
# Check logs
tail -f /var/log/doli/node1.log                              # N1/N2 (omegacortex)
ssh -p 50790 ilozada@147.93.84.44 'tail -f /var/log/doli/node3.log'  # N3 (via jump)
ssh -p 50790 ilozada@72.60.70.166 'tail -f /var/log/doli/node4.log'  # N4 (via jump)

# Manage service
sudo systemctl status doli-mainnet-node1
sudo systemctl restart doli-mainnet-node1
sudo systemctl stop doli-mainnet-node1
```

**Key differences:**
- **N1/N2** (omegacortex): Have Rust toolchain, full repo clone. `cargo build --release` works. Both share the same compiled binary. SSH user is `ilozada`.
- **N3** (147.93.84.44): Own VPS. Binary deployed via SCP from omegacortex. SSH user is `ilozada`. Reachable via omegacortex as jump host.
- **N4/N5** (remote VMs): **No Rust toolchain.** Binary deployed via SCP from omegacortex. Cannot compile locally.
- **N3/N4/N5 SSH**: Only reachable via omegacortex as jump host (`ssh -p 50790`). Direct SSH from local machine fails.
- **N4/N5 process user**: `isudoajl` (not `ilozada`). Systemd service runs as `isudoajl`. SSH as `ilozada`.
- **N4/N5 data dir**: Files live directly in `~/.doli/mainnet/` (no `data/` subdirectory).

### Producer Key Registry (AUTHORITATIVE)

> **CRITICAL**: These are the ONLY valid producer keys. They match the `BOOTSTRAP_MAINTAINER_KEYS` in `crates/updater/src/lib.rs` (updated 2026-02-22).

| Node | Host | Key File | Address (`doli1...`) | Public Key (Ed25519) |
|------|------|----------|---------------------|----------------------|
| **N1** | omegacortex | `~/.doli/mainnet/keys/producer_1.json` | `doli17engd6utnqs4ag6l6xme7tdhvgh6rcd8ezay5qw0vssqxyw239ts9dygef` | `202047256a...c3e6d3df` |
| **N2** | omegacortex | `~/.doli/mainnet/keys/producer_2.json` | `doli12uaj6e7nkl90ry9q2ze27la7w0cg23ny7zk5csyj7ffrlcttcansfzx4mz` | `effe88fefb...9926272b` |
| **N3** | N3-VPS | `/home/ilozada/.doli/mainnet/keys/producer_3.json` | `doli109t8uyux22qqrx9ewzrpxww25scjt5cl49cunkn6m72me2txrgpsqd3rql` | `54323cefd0...25c48c2b` |
| **N4** | pro-KVM1 | `/home/isudoajl/.doli/mainnet/keys/producer_4.json` | `doli1eduw95x5c6erx4dpacpfm90dylhjvjjn43j3nwag3huym6d20sdqzcqyq6` | `a1596a36fd...e9beda1d` |
| **N5** | fpx | `/home/isudoajl/.doli/mainnet/keys/producer_5.json` | `doli1fznp4jddlf39qzg3kc94qvnsptrhkt0z3pehwq3cnpurk7ylauqstxsxyc` | `c5acb5b359...e3c03a9` |

**Producer key files are wallet-compatible** — use directly with `doli -w <key_file>` for balance queries, sends, and producer operations.

**Retired keys** (produced early blocks, no longer active — funds still held):

| Key | Public Key | Approx Blocks | Balance |
|-----|------------|---------------|---------|
| U1 | `fd2f9af2d073c52a11c0994f0f3df607cb19f13cbabf1e30f1f02525d4cda691` | ~1,601 | 1,601 DOLI |
| U2 | `805b7411209cca4465892c483131cec07390befae77d5bca6930f4d55b07eff5` | ~1,644 | 1,644 DOLI |
| U3 | `a44df67f10564a221b9bd6f2e020556940b5bf7036cab7e896a52a2d69a4e272` | ~1,517 | 1,517 DOLI |

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

```bash
# N1/N2 (omegacortex)
ssh ilozada@omegacortex.ai "sudo systemctl stop doli-mainnet-node1"
ssh ilozada@omegacortex.ai "sudo systemctl stop doli-mainnet-node2"

# N3 (via jump)
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@147.93.84.44 'sudo systemctl stop doli-mainnet-node3'"

# N4/N5 (via jump)
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo systemctl stop doli-mainnet-node4'"
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo systemctl stop doli-mainnet-node5'"
```

#### Step 4: Start nodes

Start N1 first (bootstrap), then the rest:

```bash
# N1 (start first — it's the bootstrap)
ssh ilozada@omegacortex.ai "sudo systemctl start doli-mainnet-node1"

# N2
ssh ilozada@omegacortex.ai "sudo systemctl start doli-mainnet-node2"

# N3
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@147.93.84.44 'sudo systemctl start doli-mainnet-node3'"

# N4
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.70.166 'sudo systemctl start doli-mainnet-node4'"

# N5
ssh ilozada@omegacortex.ai "ssh -p 50790 ilozada@72.60.115.209 'sudo systemctl start doli-mainnet-node5'"
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

```bash
# 1. Stop the node
sudo systemctl stop doli-mainnet-nodeN

# 2. Wipe chain state (keep keys and chainspec!)
rm -f chain_state.bin producers.bin utxo.bin
rm -rf blocks/ signed_slots.db/

# 3. Restart
sudo systemctl start doli-mainnet-nodeN
```

**Data dir paths** (where to run the wipe):
- **N1**: `~/.doli/mainnet/node1/data/` (omegacortex)
- **N2**: `~/.doli/mainnet/node2/data/` (omegacortex)
- **N3**: `/home/ilozada/.doli/mainnet/data/` (147.93.84.44)
- **N4**: `/home/isudoajl/.doli/mainnet/` (72.60.70.166, no `data/` subdir)
- **N5**: `/home/isudoajl/.doli/mainnet/` (72.60.115.209, no `data/` subdir)

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