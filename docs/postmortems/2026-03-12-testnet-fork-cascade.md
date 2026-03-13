# Incident Report: Testnet Fork Cascade & Recurring Sync Failures

**Date:** 2026-03-12
**Severity:** High (testnet) / Medium (mainnet risk)
**Duration:** ~6 hours active investigation
**Networks affected:** Testnet (primary), Mainnet (deploy only — no incident)
**Author:** E. Weil

---

## Timeline

### 14:00 — Initial observation: Seeds reporting 0 peers

Routine health check showed all 3 seed nodes (ai1, ai2, ai3) reporting 0 peers. Investigation revealed `getPeerInfo` RPC method was **never implemented** — the health-check script was calling a non-existent method. `getNetworkInfo` confirmed seeds actually had 14+ peers. No real connectivity issue.

**Root cause:** `health-check.sh` called `getPeerInfo` which fell through to `method_not_found`. Script interpreted the error as 0 peers.

**Fix:** Implemented `getPeerInfo` RPC method (new handler in `methods.rs`, `PeerInfoEntry` struct in `types.rs`, wired in `node.rs`). Fixed `health-check.sh` to use `getNetworkInfo` as fallback.

### ~15:30 — Compiled and deployed v3.4.4 to testnet

Compiled on ai2 (per procedure). Deployed to testnet ai1 + ai2 + ai3. All 12 testnet nodes + 3 seeds updated. Verified 12/12 synced at same height.

### ~16:00 — Deployed to mainnet

Same binary deployed to mainnet ai1, ai2, ai3. All 12 producers + 3 seeds updated. Verified convergence.

### ~16:30 — NT4 fork detected on testnet

Testnet status showed NT4 in FORK state. Investigation began.

### ~16:45 — Seed3 (ai3) contaminated

Seed3 testnet had accepted the fork chain from NT4, propagating bad blocks to other nodes. NT2 also fell into fork state.

### ~17:00 — NT2 stuck in infinite reorg loop

NT2 logs revealed the core issue:
```
Fork sync: reorg depth 40 exceeds safety limit (10 blocks).
Refusing automatic reorg (ancestor h=16109, current h=16149).
```

Fork sync binary search successfully found the common ancestor at h=16109, downloaded canonical blocks, but then **refused to execute the reorg** because depth (40) > `MAX_SAFE_REORG_DEPTH` (10). The node then reset sync state and retried — creating an infinite loop.

Additionally, the **sync retry timer** (fires every 1 second) was resetting fork_sync state mid-binary-search, preventing the binary search from completing across multiple ticks.

### ~17:30 — NT2 self-recovered via systemd restart

After enough failed attempts, systemd restarted NT2. On restart, it snap-synced from peers and caught up to the canonical chain. Recovery was accidental, not by design.

### ~18:00 — Root cause analysis completed

**Root cause #1:** `MAX_SAFE_REORG_DEPTH = 10` (node.rs:2498) was too conservative. After snap sync, nodes commonly have fork depths of 20-50 blocks. The safety limit was meant to prevent catastrophic reorgs but instead prevented ALL fork recovery beyond 10 blocks, forcing nodes into infinite loops.

**Root cause #2:** Sync retry at `manager.rs:3358` fires every 1 second and resets sync state to Idle. When fork_sync binary search is active (spans multiple ticks), the retry interrupts it before it can complete, creating a second infinite loop.

**Trigger:** Deploy restart caused some nodes to snap-sync. After snap sync, their chain diverged slightly from peers. Fork sync detected the divergence but couldn't resolve it due to the two bugs above.

### ~18:30 — Fixes implemented

**Fix 1 — Reorg safety limit (node.rs:2498):**
```rust
// Before:
const MAX_SAFE_REORG_DEPTH: u64 = 10;

// After:
const MAX_SAFE_REORG_DEPTH: u64 = 500;
```
Undo data is kept for 2,000 blocks (`UNDO_KEEP_DEPTH`), so 500 provides a 4x safety margin. Nodes can now automatically reorg up to 500 blocks deep when fork_sync confirms they're in the minority.

**Fix 2 — Protect fork_sync from sync retry (manager.rs:3358):**
```rust
// Before:
if matches!(self.state, SyncState::Idle) && self.should_sync() {

// After:
if matches!(self.state, SyncState::Idle)
    && self.should_sync()
    && !self.is_fork_sync_active()
{
```
When fork_sync binary search is active, the sync retry now skips, allowing the binary search to complete across multiple ticks without interruption.

### ~19:00 — Compiled and deployed fixes

- Committed as `5d1fc5c`: "fix(sync): raise reorg safety limit and protect fork_sync from retry loop"
- Compiled on ai2
- Deployed to testnet: ai2 first, then ai3 (seed), then ai1
- Verified 12/12 testnet nodes synced at v3.4.4

### ~20:00 — Deployed to mainnet

Same binary deployed to mainnet ai1, ai2, ai3. All nodes converged at v3.4.4.

### ~20:30 — N4 mainnet block store gap

Discovered N4 mainnet had a block store gap of 16,521 blocks (from snap sync). Attempted CLI backfill (`restore --from --backfill`) but hit genesis hash mismatch bug — the embedded genesis hash differs from the current chain's genesis (post-reset chain). Used `backfillFromPeer` RPC with seed RPC URL instead.

### ~22:00 — Infrastructure audit: ai1 vs ai2

Comprehensive comparison of network config, firewall, services between ai1 and ai2 to find external triggers.

**Finding:** 9 of 24 service files across ai1 and ai2 are "old generation" — missing `LimitNOFILE=65535`, `--force-start`, `--yes`, and log file routing. Affected nodes: N4, N7, N11 (mainnet), NT7-NT12 odd/even split (testnet).

The missing `LimitNOFILE=65535` means those nodes run with the default 1024 file descriptor limit. With 14+ peers and RocksDB, this can cause silent connection drops under load.

Bootstrap DNS was confirmed embedded in code (`network_params.rs`) — the `--bootstrap` flags in service files are redundant.

### 00:21 — NT8 and NT10 stuck (two distinct failure modes)

**NT8 — Header peer exhaustion:** All peers returned 0 headers → all blacklisted → node cycled Idle ↔ DownloadingHeaders indefinitely. Root cause: the 60s blacklist cooldown (cleanup) is longer than the 30s stuck-sync timeout, so blacklisted peers never expire before the next retry. No escalation path exists when all peers are blacklisted.

**NT10 — Rollback loop without forward sync:** Completely different bug. NT10 was stuck in: fork detected → `consecutive_fork_blocks` hits 10 → `maybe_auto_resync()` → `rollback_one_block()` → `reset_sync_for_rollback()` → Idle → `cleanup()` → `start_sync()` → header-first sync (gap < 1000 blocks) → 0 headers (peers reject rolled-back fork tip) → fork detected again. Result: rolling back 1 block every ~100s, never syncing forward. Root cause: after rollback from a fork, `start_sync()` chooses header-first because the gap is small, but header-first always fails when the tip is still on a fork (just 1 block shorter).

Both recovered after manual restart. Backfill initiated from testnet seed RPC (`http://127.0.0.1:18500`).

---

## Root Causes

| # | Cause | Severity | Status |
|---|-------|----------|--------|
| 1 | `MAX_SAFE_REORG_DEPTH = 10` too low | Critical | **Fixed** — raised to 500 |
| 2 | Sync retry interrupts fork_sync binary search | Critical | **Fixed** — guard added |
| 3 | 9/24 service files missing `LimitNOFILE=65535` | Medium | **Open** — needs service file update |
| 4 | Production circuit breaker | Medium | **Already implemented** — Layer 10 (`can_produce()` blocks when >5 blocks ahead) + Layer 10.5 (`max_solo_production_secs` halts after 50s without peer gossip) in `manager.rs:1190-1243` |
| 5 | CLI `restore --backfill` genesis hash mismatch on post-reset chain | Low | **Open** — embedded genesis differs from current chain |
| 6 | NT8: Header peer exhaustion — all peers blacklisted (60s cooldown > 30s stuck timeout) | Medium | **Open** — blacklist timing prevents recovery |
| 7 | NT10: Rollback loop without forward sync — post-rollback `start_sync()` chooses header-first which always fails against fork tip | Medium | **Open** — needs post-rollback sync strategy override |

---

## What Worked

- Snap sync recovery: nodes that were restarted by systemd recovered automatically via snap sync
- `backfillFromPeer` RPC: successfully fills block store gaps from seed's RPC without restart
- DNS round-robin bootstrap: all nodes can rediscover the network after isolation
- Seed infrastructure: 3 seeds across 3 servers provided redundancy when Seed3 was contaminated

## What Didn't Work

- Fork sync with depth > 10: completely broken, infinite loop
- Sync retry: actively harmful when fork_sync is running
- `health-check.sh`: was calling non-existent RPC method, giving false 0-peer readings
- NT8/NT10 self-recovery: nodes couldn't recover from sync stalls without manual restart
- CLI backfill on post-reset chains: genesis hash mismatch blocks the operation

---

## Actions Taken

| # | Action | Status |
|---|--------|--------|
| 1 | Implemented `getPeerInfo` RPC method | Done |
| 2 | Fixed `health-check.sh` to use `getNetworkInfo` | Done |
| 3 | Raised `MAX_SAFE_REORG_DEPTH` from 10 to 500 | Done — deployed |
| 4 | Protected fork_sync from sync retry interruption | Done — deployed |
| 5 | Deployed v3.4.4 to all testnet nodes (ai1, ai2, ai3) | Done |
| 6 | Deployed v3.4.4 to all mainnet nodes (ai1, ai2, ai3) | Done |
| 7 | Backfilled N4 mainnet from seed RPC | Done |
| 8 | Restarted NT8/NT10 and initiated backfill | Done |
| 9 | Audited ai1 vs ai2 infrastructure config | Done |

---

## Open Action Items

| # | Action | Priority | Owner |
|---|--------|----------|-------|
| 1 | Update 9 old-generation service files (add `LimitNOFILE=65535`, log routing) | High | Ops |
| 2 | ~~Implement production circuit breaker~~ | ~~High~~ | **Already implemented** — Layer 10 + 10.5 in `can_produce()` |
| 3 | Fix CLI `restore --backfill` genesis hash fallback for post-reset chains (`--skip-genesis-check`) | Low | Dev |
| 4 | Fix NT8: header peer exhaustion — reduce blacklist cooldown, add total-stuck escalation | Medium | Dev |
| 5 | Fix NT10: rollback loop — post-rollback sync must skip header-first, use fork_sync or snap sync | Medium | Dev |
| 6 | Tag v3.4.4 release for external producers | Medium | Ops |

---

## Lessons Learned

1. **Safety limits can be unsafe.** `MAX_SAFE_REORG_DEPTH = 10` was intended to prevent dangerous reorgs but instead prevented ALL recovery from forks deeper than 10 blocks. The "safe" limit created infinite loops that were worse than the reorg it was trying to prevent.

2. **Timers that reset state are dangerous.** The 1-second sync retry was actively harmful — it interrupted multi-tick operations (fork_sync binary search) that needed to run to completion. Background timers should always check if an operation is in progress before resetting state.

3. **Service file consistency is consensus-critical.** The mix of old and new service file formats (missing `LimitNOFILE`, log routing) creates unpredictable behavior across nodes. Service files should be generated from a template, never hand-edited.

4. **`backfillFromPeer` takes an RPC URL, not a peer ID.** Passing a libp2p peer ID causes silent failure. This was documented correctly in `docs/archiver.md` but not consulted before execution.

5. **Deploy restarts are the most common fork trigger.** Every deploy causes service restarts, which cause snap syncs, which create chain divergences that fork_sync must resolve. The fixes deployed today make fork_sync actually work for these cases.
