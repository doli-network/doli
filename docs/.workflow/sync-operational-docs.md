# Operational Docs Sync -- 2026-03-29 (Full Verification Pass)

> Source of truth: code in crates/core/src/, bins/node/src/, crates/updater/src/, crates/storage/src/, crates/network/src/
> Checked 11 docs against actual source code. Fixed 5 files, 43 env vars verified.

## All Fixes Applied

### Session 1 (previous pass)

1. **crates/storage/src/archiver.rs** line 16: Doc comment said `.sha256` but code uses `.blake3`. Fixed.
2. **docs/auto_update_system.md** section 3.1.1: Vote weight formula was wrong. Said `vote_weight = 1.0 + min(years, 4) * 0.75` (seniority only). Code in `crates/updater/src/params.rs:100-104` shows `vote_weight = bond_count * (1.0 + min(years, 4) * 0.75)`. Bond count IS a factor. Fixed.
3. **docs/auto_update_system.md** section 3.1.2: Wrongly claimed voting weight is "time (seniority), not stake (bond units)". Corrected to reflect both factors.
4. **docs/running_a_node.md** section 12: Added missing CLI subcommands (update, maintainer, release) and 13 missing run flags (--rpc-bind, --external-address, --relay-server, --no-auto-rollback, --force-start, --yes, --chainspec, --checkpoint-height, --checkpoint-hash, --archive-to, --no-dht, --metrics-port, --bootstrap).

### Session 2 (this pass)

5. **docs/running_a_node.md** section 5.3: Added 9 missing env vars from `env_loader.rs`:
   - `DOLI_MAX_PEERS`, `DOLI_GRACE_PERIOD_SECS`, `DOLI_SLOTS_PER_REWARD_EPOCH`
   - `DOLI_VESTING_QUARTER_SLOTS`, `DOLI_MIN_VOTING_AGE_SECS`, `DOLI_UPDATE_CHECK_INTERVAL_SECS`
   - `DOLI_REGISTRATION_BASE_FEE`, `DOLI_BOOTSTRAP_BLOCKS`, `DOLI_EVICTION_GRACE_SECS`, `DOLI_MESH_N`

6. **docs/running_a_node.md** section 5.4: Expanded mainnet locked params from 7 to 23+ entries.

7. **docs/running_a_node.md** section 2.1: Fixed `SHA256SUMS.txt` -> `CHECKSUMS.txt`.

8. **docs/running_a_node.md** section 10: Fixed node_key backup path.

9. **docs/running_a_node.md** section 5.5: Added `DOLI_DATA_DIR` env var and platform-specific data directory documentation.

10. **docs/testnet.md**: Added `seeds.testnet.doli.network:40300` to both bootstrap DNS lists.

### Session 3 (continuation — transport + completeness pass)

11. **docs/running_a_node.md** section 5.3: Added 20 more env vars to complete the table (all 43 DOLI_ env vars now documented):
    - Transport-level: `DOLI_YAMUX_WINDOW` (256KB), `DOLI_CONN_LIMIT` (max_peers+10), `DOLI_PENDING_LIMIT` (5), `DOLI_IDLE_TIMEOUT_SECS` (86400/300/300)
    - VDF variants: `DOLI_HEARTBEAT_VDF_ITERATIONS` (1000), `DOLI_VDF_REGISTER_ITERATIONS` (1000)
    - Registration: `DOLI_MAX_REGISTRATION_FEE` (10 DOLI), `DOLI_MAX_REGISTRATIONS_PER_BLOCK` (5)
    - Diagnostics: `DOLI_CRASH_WINDOW_SECS` (3600), `DOLI_PRESENCE_WINDOW_MS` (200)
    - Genesis: `DOLI_GENESIS_BLOCKS` (360/36/40), `DOLI_AUTOMATIC_GENESIS_BOND`, `DOLI_BOOTSTRAP_GRACE_PERIOD_SECS` (15/15/5)
    - Consensus timing: `DOLI_FALLBACK_TIMEOUT_MS` (2000), `DOLI_MAX_FALLBACK_RANKS` (2), `DOLI_NETWORK_MARGIN_MS` (200), `DOLI_INACTIVITY_THRESHOLD` (50)
    - Gossip mesh: `DOLI_MESH_N_LOW` (8/20/8), `DOLI_MESH_N_HIGH` (24/50/24), `DOLI_GOSSIP_LAZY` (12/25/12)
    - Updated `DOLI_MESH_N` with per-network defaults (12/25/12)

## Env Var Coverage Summary

| Source | Count | Status |
|--------|-------|--------|
| `env_loader.rs` (NetworkParams) | 38 | All documented in 5.3 table |
| `network/service/mod.rs` (transport) | 4 | All documented in 5.3 table |
| `network/service/swarm_events.rs` (eviction) | 1 | Documented in 5.3 table |
| **Total** | **43** | **All documented** |

## Verified Correct (No Drift)

- **docs/becoming_a_producer.md** -- All bond/vesting/penalty values match code
- **docs/devnet.md** -- All parameters match defaults.rs devnet section
- **docs/disaster-recovery.md** -- Recovery procedures match code
- **docs/genesis.md** -- Genesis timestamp sources and procedures match code
- **docs/genesis_ceremony.md** -- Server layout v9, service rules all match
- **docs/archiver.md** -- Archive format, RPC methods, catch-up all match code
- **docs/troubleshooting.md** -- Error paths and diagnostics match code
- **docs/infrastructure.md** -- Testnet genesis date (March 29, 1774749145) correct, DNS table correct
- **docs/testnet.md** -- All parameters match, bootstrap DNS complete
- **docs/auto_update_system.md** -- Vote weight formula, thresholds, maintainer keys all match code
- **docs/running_a_node.md** -- CLI flags, env vars, ports, data dirs all match code
