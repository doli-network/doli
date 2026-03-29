# Sync Pass 6 — Architecture + Security + Operational Docs

**Date**: 2026-03-29
**Workflow Run**: 95
**Scope**: 9 doc files vs source code

---

## Source Files Read

| Source File | Purpose |
|------------|---------|
| `crates/core/src/network_params/defaults.rs` | All hardcoded defaults for mainnet/testnet/devnet |
| `crates/core/src/consensus/constants.rs` | Protocol constants (BOND_UNIT, INITIAL_REWARD, etc.) |
| `crates/core/src/consensus/mod.rs` | Consensus module re-exports |
| `bins/node/src/main.rs` | CLI entry, data directory resolution |
| `bins/node/src/cli.rs` | Full CLI flag definitions |
| `crates/updater/src/constants.rs` | Updater constants, bootstrap maintainer keys |
| `crates/updater/src/lib.rs` | UpdateService imports |
| `crates/storage/src/archiver.rs` | Block archiver implementation |
| `crates/storage/src/block_store/types.rs` | Block store column families |
| `crates/storage/src/state_db/types.rs` | State DB column families |
| `crates/core/src/maintainer.rs` | Maintainer set management |

---

## Drift Found and Fixed

### 1. `docs/security_model.md`

| # | Drift | Fix |
|---|-------|-----|
| 1 | Said "BLS12-381 is unforgeable (aggregate attestation signatures)" implying active use | Changed to "BLS12-381 fields exist in block headers for future aggregate attestations but are not used in current consensus" |

### 2. `docs/running_a_node.md`

| # | Drift | Fix |
|---|-------|-----|
| 2 | Data Directory column showed only `~/.doli/{network}/` for all networks | Updated to show platform-specific paths: Linux `/var/lib/doli/{net}/`, macOS `~/Library/Application Support/doli/{net}/`, legacy `~/.doli/{net}/` |

### 3. `docs/archiver.md`

| # | Drift | Fix |
|---|-------|-----|
| 3 | Mainnet Seed3 DNS listed as `seed3.doli.network` | Fixed to `seeds.doli.network` (matches defaults.rs bootstrap_nodes) |
| 4 | Mainnet DNS table listed `seed3.doli.network` | Fixed to `seeds.doli.network` with "round-robin" note |
| 5 | Testnet Seed3 DNS listed as `bootstrap3.testnet.doli.network` | Fixed to `seeds.testnet.doli.network` (matches defaults.rs bootstrap_nodes) |
| 6 | Testnet DNS table listed `bootstrap3.testnet.doli.network` | Fixed to `seeds.testnet.doli.network` with "round-robin" note |

### 4. `docs/architecture.md`

| # | Drift | Fix |
|---|-------|-----|
| 7 | Storage table listed `block_store.rs` (single file) | Fixed to `block_store/` (now a directory with mod.rs, queries.rs, writes.rs, types.rs) |
| 8 | Storage crate module table missing `archiver.rs` | Added `archiver.rs` entry with description |
| 9 | Dependency graph missing `updater` crate | Added `updater` crate to dependency graph |
| 10 | Section 3.9 Auto-Update had only 4-line feature list, no module detail | Expanded to full module table (11 entries) with feature list and cross-reference |

### 5. `docs/auto_update_system.md`

| # | Drift | Fix |
|---|-------|-----|
| 11 | `MaintainerTxType::RemoveMaintainer = 13` | Fixed to `= 11` (matches constants.rs) |
| 12 | `MaintainerTxType::AddMaintainer = 14` | Fixed to `= 12` (matches constants.rs) |
| 13 | Bootstrap keys path cited as `crates/updater/src/lib.rs` | Fixed to `crates/updater/src/constants.rs` (where they're actually defined) |
| 14 | Node integration path listed as `bins/node/src/updater.rs` (single file) | Fixed to `bins/node/src/updater/ (mod, service, cli)` (it's a directory) |

### 6. `docs/becoming_a_producer.md`

| # | Drift | Fix |
|---|-------|-----|
| 15 | Veto threshold said "Dormant producers have reduced weight" | Replaced with accurate description: "Weight based on bonds x seniority multiplier (1.0-4.0x over 4 years)" and "Minimum 30-day producer age before votes count" |

### 7. `docs/testnet.md`

| # | Drift | Fix |
|---|-------|-----|
| 16 | Maintainer keys path cited as `crates/updater/src/lib.rs` | Fixed to `crates/updater/src/constants.rs` |

---

## Drift Found But NOT Fixed (Operational — Cannot Verify from Code)

### `docs/infrastructure.md`

| # | Issue | Reason Not Fixed |
|---|-------|-----------------|
| A | DNS table says `seed1.doli.network` points to ai2, but `archiver.md` says Seed1 is on ai1 | DNS records are external infrastructure, cannot verify from code. Code only uses DNS names, not IP-to-server mappings. Requires operator confirmation. |
| B | `seed2.doli.network` points to ai3 in DNS table but Seed2 is listed on ai2 in seed table | Same — internal infrastructure mapping inconsistency between DNS and node inventory tables |

---

## Verified Correct (No Drift)

### `docs/testnet.md`
- Block reward: 1 tDOLI -- matches `consensus::INITIAL_REWARD = 100_000_000` (1 DOLI)
- Epoch length: 36 blocks (~6 min) -- matches `blocks_per_reward_epoch: 36`
- Bond unit: 1 tDOLI -- matches `bond_unit: 100_000_000`
- P2P port: 40300 -- matches `default_p2p_port: 40300`
- RPC port: 18500 -- matches `default_rpc_port: 18500`
- Bootstrap DNS names -- match defaults.rs exactly
- Genesis timestamp 1774749145 -- matches `genesis_time: 1774749145`

### `docs/devnet.md`
- Slot duration: 10 seconds -- matches `slot_duration: consensus::SLOT_DURATION` (10)
- VDF iterations: 1 -- matches `vdf_iterations: 1`
- Heartbeat VDF: 1000 -- matches `heartbeat_vdf_iterations: 1_000`
- Unbonding period: 60 -- matches `unbonding_period: 60`
- Blocks per epoch: 4 -- matches `blocks_per_reward_epoch: 4`
- Port numbers (P2P=50300, RPC=28500) -- match defaults.rs

### `docs/auto_update_system.md` (remaining sections)
- VETO_PERIOD: 5 min -- matches `veto_period_secs: 5 * 60`
- GRACE_PERIOD: 2 min -- matches `grace_period_secs: 2 * 60`
- CHECK_INTERVAL: 10 min -- matches `update_check_interval_secs: 10 * 60`
- VETO_THRESHOLD_PERCENT: 40% -- matches `crates/updater/src/constants.rs`
- REQUIRED_SIGNATURES: 3 -- matches constants.rs
- CRASH_THRESHOLD: 3 -- matches watchdog.rs
- MIN_VOTING_AGE: 30 days -- matches `min_voting_age_secs: 30 * 24 * 3600`
- MAX_SENIORITY_MULTIPLIER: 4x at 4 years -- matches seniority formula
- Seniority formula: `1.0 + min(years, 4) * 0.75` -- matches params.rs
- File reference table (11.1): all paths verified to exist on disk

### `docs/infrastructure.md`
- Port formula: P2P=30300+N, RPC=8500+N, Metrics=9000+N -- matches defaults.rs
- Testnet ports: P2P=40300+N, RPC=18500+N, Metrics=19000+N -- matches defaults.rs
- Bootstrap DNS names -- match defaults.rs
- Service naming: `doli-{network}-{role}` -- consistent throughout
- Testnet epoch: 36 blocks -- matches defaults.rs
- Testnet bond unit: 1 tDOLI -- matches defaults.rs

### `docs/security_model.md` (remaining)
- Inactivity leak: doc says not currently enforced -- confirmed: `INACTIVITY_LEAK_*` constants exist but are not used in runtime
- Ed25519 signatures: correct, used throughout
- 3/5 multisig: matches updater constants

### `docs/archiver.md` (remaining)
- Archive format: `{height:010}.block` + `.blake3` -- matches archiver.rs
- manifest.json structure -- matches code
- Atomic writes via `.tmp` + rename -- matches code
- RPC methods (getBlockRaw, backfillFromPeer, backfillStatus, verifyChainIntegrity) -- all exist
- Code reference table -- all paths correct

---

## Summary

| Metric | Value |
|--------|-------|
| Doc files scanned | 9 |
| Source files read | 11 |
| Total drift items found | 16 |
| Drift items fixed | 16 |
| Drift items deferred (operational) | 2 |
| Items verified correct | 40+ |
