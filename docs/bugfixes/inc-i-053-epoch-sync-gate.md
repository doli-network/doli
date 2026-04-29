# INC-I-053: Epoch Mode Missing Startup Sync Gate

## Architecture Context
- `production/mod.rs:try_produce_block()` → outer production function
- `production/gates.rs` → delegates to `SyncManager::can_produce()`
- `production/scheduling.rs` → bootstrap and epoch eligibility
- `sync/manager/production_gate.rs` → SyncManager's production gate

## Root Cause
After restart, epoch-mode nodes produce blocks before syncing the canonical chain tip.

### Why existing guards don't prevent this:
1. **SyncManager behind-tip guard (Check 5)**: REMOVED in INC-I-026 to prevent deadlocks
2. **mod.rs behind-peers check (line 148-170)**: Uses `max_behind=5`, but simultaneously-restarted nodes see each other's stale height → `blocks_behind=0-1` → guard doesn't fire
3. **Bootstrap mode has guards** (scheduling.rs:90-167): grace periods, min height, behind-peers checks — but these don't apply in epoch mode

### Evidence
N4 restarted at 16:51:35, had 1 peer (N5, also just restarted) at 16:51:36, produced fork block at 16:51:49 (14s later). `gap=1` because both N4/N5 are at the same stale height.

## Fix
Add a startup grace period to `try_produce_block()` for epoch mode. After the node starts, defer production for `startup_grace_secs` (15s for testnet, 30s for mainnet). This gives time for:
- Gossip blocks to arrive from the canonical chain
- Peers to connect and report accurate heights
- Anti-entropy to deliver the producer list

The grace period starts when `first_peer_connected` is set (same as bootstrap mode). Only applies in epoch mode — bootstrap mode already has its own guards.

### Blast Radius
- Only affects epoch-mode production timing after restart
- Does NOT change the scheduler or validation
- Single-node restarts: node waits 15-30s then resumes (self-heals from gossip)
- Simultaneous restarts: all nodes wait, then sync canonical tip before producing

## Triage

━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.9, log-traced)
Reasoning: Root cause is a missing guard in one code path (try_produce_block epoch mode). Localized to 1 file. Deterministic reproduction.
━━━━━━━━━━━━━━━━━━━━━━
