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
     - **Command**: `doli <subcommand>` (what was attempted)
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
     - Missing `doli wallet export` sub-command → log it.
     - `doli bond status` returns wrong penalty tier → log it.
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
| **Run Wallet** | `cargo run -p doli -- <command>` |
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
| `storage` | RocksDB blocks + unified StateDb + Block Archiver | `block_store.rs`, `state_db.rs`, `utxo.rs`, `producer.rs`, `archiver.rs` |
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
| **Max Bonds** | 3,000/producer | 3,000 | `MAX_BONDS_PER_PRODUCER` (30K DOLI max stake) |
| **Vesting** | 4yr (12,614,400 slots) | 1d (8,640) / configurable | `VESTING_PERIOD_SLOTS` (per-bond FIFO) |
| **Vesting Quarter** | 1yr (3,153,600 slots) | 6h (2,160) / configurable | `VESTING_QUARTER_SLOTS` |
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
- **Burnt**: Slashing (100%), Early Withdrawal (75%→0% over 1 day, per-bond FIFO), Reg Fees
- **Bond Unit**: Fixed at 10 DOLI across all eras (never decreases)

### Bond Vesting (Per-Bond FIFO — Network-Differentiated)

Each bond has its own `StoredBondEntry` with `creation_slot`. Withdrawal uses **FIFO order** (oldest first), with per-bond penalty based on individual age. **Instant payout** — funds available in the same block, no delay. Bonds removed at next epoch boundary.

**Mainnet** (4-year, 1-year quarters):

| Quarter | Age | Penalty |
|---------|-----|---------|
| Y1 | 0-1yr | 75% Burn |
| Y2 | 1-2yr | 50% Burn |
| Y3 | 2-3yr | 25% Burn |
| Y4+ | 3yr+ | 0% |

`VESTING_QUARTER_SLOTS = 3,153,600` (1yr), `VESTING_PERIOD_SLOTS = 12,614,400` (4yr).

**Testnet** (1-day, 6h quarters):

| Quarter | Age | Penalty |
|---------|-----|---------|
| Q1 | 0-6h | 75% Burn |
| Q2 | 6-12h | 50% Burn |
| Q3 | 12-18h | 25% Burn |
| Q4+ | 18h+ | 0% |

Testnet: `vesting_quarter_slots = 2,160` (6h) via `NetworkParams`. Devnet configurable via `DOLI_VESTING_QUARTER_SLOTS`.

**Key fields on ProducerInfo**: `bond_entries: Vec<StoredBondEntry>`, `withdrawal_pending_count: u32` (prevents double-withdrawal in same epoch).

**RPC**: `getBondDetails` returns real per-bond data (creation_slot, penalty_pct, vested status per bond).

**CLI**: `producer status` shows per-bond maturation tiers. `producer request-withdrawal --count N` shows interactive FIFO breakdown with per-tier penalties before confirmation.

## 🛡 Validation & Security

### Block Validation
- genesis_hash matches (FIRST check — rejects different genesis immediately)
- Version = 2
- Timestamp advances from parent
- Slot = timestamp_to_slot(timestamp) (derived, not free field)
- Max size: 1MB + header overhead
- Merkle root matches transactions
- VDF proof valid for slot

### Chain Identity (genesis_hash)
- `genesis_hash = BLAKE3(genesis_time || network_id || slot_duration || message)`
- Present in every BlockHeader (v2+), included in block hash
- Mainnet and testnet chainspecs are embedded in binary — disk files and `--chainspec` ignored
- Devnet uses disk chainspec or CLI `--chainspec` flag
- Prevents genesis-time-hijack attacks (even 1s difference → different hash → rejected)

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
- **Network targeting**: `metadata.json` in GitHub Release assets controls which networks receive an update
  - `{"networks": ["mainnet", "testnet"]}` = both, `["testnet"]` = testnet only (staged rollout)
  - Missing metadata.json = targets all networks (backward compat)
- **Veto**: 40% stake can block (5-min veto period early network; target 7 days)

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
| `block_store.rs` | RocksDB block storage (headers, bodies, indexes) |
| `state_db.rs` | Unified StateDb: atomic WriteBatch per block (UTXOs, producers, chain state) |
| `utxo.rs` | In-memory UTXO working set for fast reads |
| `producer.rs` | Producer registry (per-bond `StoredBondEntry` tracking, FIFO withdrawal) |
| `archiver.rs` | Block archiver for disaster recovery (`--archive-to`, atomic file writes, catch-up) |

**BlockStore Column Families**: `headers`, `bodies`, `height_index`, `slot_index`, `presence`
**StateDb Column Families**: `cf_utxo`, `cf_utxo_by_pubkey`, `cf_producers`, `cf_exit_history`, `cf_meta`

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
| 8 | WithdrawalRequest | Instant bond withdrawal (FIFO, per-bond penalty, payout in same block) |
| 9 | WithdrawalClaim | Reserved (unused — withdrawal is now instant via TxType 8) |
| 10 | EpochReward | Epoch-level rewards |
| 11 | MaintainerAdd | Add maintainer (governance) |
| 12 | MaintainerRemove | Remove maintainer (governance) |
| 13 | DelegateBond | Delegate bonds to another producer |
| 14 | RevokeDelegation | Revoke delegated bonds |
| 15 | ProtocolActivation | Activate new protocol version (3/5 maintainer multisig, on-chain) |

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

## 📦 Block Archiver (Disaster Recovery)

The block archiver streams every applied block to a filesystem directory for off-chain backup.

**How it works:**
- `--archive-to /path/` flag on `doli-node run` enables archiving
- Each block is serialized (bincode) and written atomically (tmp + rename)
- `manifest.json` tracks latest archived height + hash
- On startup, catches up any missed blocks from BlockStore
- Non-blocking: uses `mpsc::channel` with `try_send` — never stalls production

**File layout:**
```
/path/archive/
  0000000001.block
  0000000001.blake3   # BLAKE3 checksum sidecar
  0000000002.block
  0000000002.blake3
  ...
  manifest.json    # {"latest_height": N, "latest_hash": "...", "genesis_hash": "..."}
```

**Archiver node on omegacortex:**
- Service: `doli-mainnet-archiver` (systemd)
- DNS: `archive.doli.network` → 198.51.100.1 (omegacortex)
- Data: `~/.doli/mainnet/archiver/data`
- Archive: `~/.doli/mainnet/archive/`
- Ports: P2P=30306, RPC=8548, Metrics=9093
- Non-producer, sync-only + archive
- Log: `/var/log/doli/archiver.log`

**Recovery options:**
- **Full restore**: `restore --from /path/to/archive --yes` — imports all blocks + rebuilds state
- **Backfill only**: `restore --from /path/to/archive --backfill --yes` — fills snap sync gaps, no state rebuild
- **P2P backfill**: Automatic — node detects gaps on startup, requests missing blocks from peers in background

**Code:** `crates/storage/src/archiver.rs`

## 🖥 Operations & Deployment

All operational procedures are in the ops runbook: **`.claude/skills/doli-ops/SKILL.md`**

**Always read the ops skill before any infrastructure task.** Key sections:
- **Section 2**: Node inventory (N1-N6), SSH access, service management, logs
- **Section 3**: Deployment procedures (consensus-critical simultaneous vs rolling)
- **Section 4**: Auto-update system (signing, veto, grace period)
- **Section 5**: doli-node upgrade procedures
- **Section 6**: Producer bond management (registration, add-bond, withdrawal)
- **Section 7**: Troubleshooting (fork recovery, sync issues, RocksDB)
- **Section 8**: Producer key registry & balance checking
- **Section 9**: N6 node details
- **Section 10**: Chainspec rules, DNS/bootstrap, snap sync
- **Section 11**: On-chain protocol activation (consensus-critical changes)

### Block Archiver (Disaster Recovery)

A dedicated sync-only node (`doli-mainnet-archiver` on omegacortex) streams finalized blocks to flat files for disaster recovery.

| Property | Value |
|----------|-------|
| DNS | `archive.doli.network` |
| Service | `doli-mainnet-archiver` |
| Archive dir | `~/.doli/mainnet/archive/` |
| RPC | 8548 |

**Key design**: Finality-gated — blocks are only archived after FinalityTracker declares them irreversible (67%+ attestation weight). Each block has a BLAKE3 checksum sidecar (`.blake3`) and `manifest.json` includes `genesis_hash`.

**Restore**: `doli-node --network mainnet restore --from /path/to/archive --yes` imports blocks, verifies checksums + genesis_hash, then auto-rebuilds state. Use `--backfill` to fill snap sync gaps without state rebuild.

**P2P backfill**: Every node automatically detects and fills historical block gaps on startup via `GetBlockByHeight` requests to peers. Background, rate-limited (100ms), resumable. No configuration needed.

**Code**: `crates/storage/src/archiver.rs` (file-based), `bins/node/src/node.rs` (P2P backfill: `detect_backfill_gap`, `maybe_backfill_block`, `handle_backfill_response`), `bins/node/src/main.rs` (CLI `--backfill` flag).
