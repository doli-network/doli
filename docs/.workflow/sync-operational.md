# Operational Docs Sync — Findings

Date: 2026-03-29

## 1. docs/rewards.md

### Drift Found

1. **Section 10 — Key Constants Reference: File paths**
   - Doc says `crates/core/src/consensus.rs` (11 references)
   - Code: consensus is a module directory: `crates/core/src/consensus/constants.rs`
   - Fix: Update all paths to `crates/core/src/consensus/constants.rs`

2. **Section 10 — attestation function location**
   - Doc says `crates/core/src/attestation.rs`
   - Code: File does exist at `crates/core/src/attestation.rs` — OK, no drift

3. **Section 3 — "collect active producers at epoch end height"**
   - Doc says: "Collect active producers at epoch end height"
   - Code (rewards.rs:20-29): `active_producers_at_height(epoch_start_height)` — uses epoch START height
   - Fix: Change "at epoch end height" to "at epoch start height"

4. **No other drift found** — all constants, formulas, edge cases match code

### Summary: 2 drift items in rewards.md


## 2. docs/running_a_node.md

### Drift Found

1. **Section 5.3 — DOLI_IDLE_TIMEOUT_SECS testnet default**
   - Doc says: `86400 mainnet / 300 testnet / 300 devnet`
   - Code (service/mod.rs:220): testnet = 300
   - OK, matches. No drift.

2. **Section 13 — Network Defaults table: Heartbeat VDF for devnet**
   - Doc says: `1,000` for devnet
   - Code (defaults.rs:187): `heartbeat_vdf_iterations: 1_000` for devnet
   - OK, matches.

3. **Section 13 — Network Defaults table: VDF Iterations for devnet**
   - Doc says: `1` for devnet
   - Code (defaults.rs:185): `vdf_iterations: 1` for devnet
   - OK, matches.

4. No drift found in running_a_node.md — all values match code.

### Summary: 0 drift items in running_a_node.md


## 3. docs/testnet.md

### Drift Found

1. **Network Information table — missing Metrics Port**
   - Doc doesn't mention default metrics port for testnet
   - Code: `default_metrics_port: 19000`
   - Not a drift per se — doc just doesn't list it. Skip (rule: don't add new sections).

2. **Seed / Bootstrap Servers table — missing bootstrap3**
   - Testnet bootstrap_nodes in defaults.rs includes `seeds.testnet.doli.network:40300`
   - Doc only lists bootstrap1 and bootstrap2 in the seed table
   - The doc DOES list all 3 in the Network Information table bootstrap field
   - The Seed/Bootstrap Servers section says only 2 seed nodes, but code has 3 bootstrap nodes
   - Fix: Note that the "seeds.testnet.doli.network" entry is a DNS alias, not a third physical seed

3. **Seeds topology — says only 2 archivers, code has 3**
   - archiver.md lists 3 testnet seeds (ai1, ai2, ai3)
   - testnet.md seed section only lists bootstrap1 and bootstrap2
   - Fix: Add bootstrap3/seeds entry to the seed table

4. No other drift found.

### Summary: 1 drift item in testnet.md


## 4. docs/auto_update_system.md

### Drift Found

1. **Section 1.2 — Key Constants: GRACE_PERIOD**
   - Doc says: `GRACE_PERIOD = 2 min *`
   - Code `constants.rs`: `GRACE_PERIOD = Duration::from_secs(3600)` (1 hour)
   - BUT `defaults.rs` for all networks: `grace_period_secs: 2 * 60` (2 minutes)
   - The network-aware code uses 2 min. The constant is stale. Doc is correct for actual behavior.
   - Note: The const GRACE_PERIOD in constants.rs is 1 hour but not used by network-aware code paths. Document the actual value used (2 min from network defaults).

2. **Section 1.2 — Key Constants: CHECK_INTERVAL**
   - Doc says: `CHECK_INTERVAL = 10 min *`
   - Code `constants.rs`: `CHECK_INTERVAL = Duration::from_secs(6 * 3600)` (6 hours)
   - BUT `defaults.rs`: `update_check_interval_secs: 10 * 60` (10 min)
   - Same situation as GRACE_PERIOD: doc shows the network default, not the constant. Correct behavior.

3. **Section 5 timeline — Grace period says "T+5min to T+7min: GRACE PERIOD (2 min)"**
   - Code: grace_period_secs = 2 * 60 = 120 seconds = 2 minutes
   - This matches. No drift.

4. **Section 1.2 — Constants table: CRASH_THRESHOLD says "3"**
   - Code `watchdog.rs:54`: `DEFAULT_CRASH_THRESHOLD: u32 = 3`
   - Matches. No drift.

5. No substantive drift found in auto_update_system.md.

### Summary: 0 drift items in auto_update_system.md (constants table shows behavior-correct values)


## 5. docs/archiver.md

### Drift Found

1. **Section 1 — Deployment diagram mentions ai1, ai2, ai4, ai5 and SANTIAGO, IVAN on ai3**
   - This is infrastructure detail, not a code drift. Skip.

2. **Section 4 — Archive Format: manifest.json fields**
   - Doc shows: `latest_height`, `latest_hash`, `genesis_hash`
   - Code (archiver.rs:203-207): matches exactly
   - No drift.

3. No code drift found in archiver.md — all archive format details, RPC methods match code.

### Summary: 0 drift items in archiver.md


## TOTAL DRIFT SUMMARY

| Document | Drift Items | Severity |
|----------|------------|----------|
| rewards.md | 2 | Low (path references + start vs end height) |
| running_a_node.md | 0 | N/A |
| testnet.md | 1 | Low (missing third bootstrap entry in seed table) |
| auto_update_system.md | 0 | N/A |
| archiver.md | 0 | N/A |

Total: 3 drift items to fix.
