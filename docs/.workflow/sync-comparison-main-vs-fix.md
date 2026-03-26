# Sync System Comparison: `main` vs `fix/sync-state-explosion-root-causes`

> Generated 2026-03-26. Based on `origin/main` at `6cfefbb5` and fix branch at `5ba0eb90`.

## The Numbers

| Dimension | `main` | Fix Branch | Winner |
|-----------|--------|------------|--------|
| SyncManager fields | 55+ flat | ~35 via 4 sub-structs | **Fix** |
| SyncState variants | 7 (data-carrying) | 3 (pure discriminants) | **Fix** |
| Production gate layers | 11 (incl. hash agreement deadlock) | 3 checks | **Fix** |
| Recovery state | 5 independent booleans | 4-variant enum | **Fix** |
| Snap sync quorum | Unbounded (deadlocks at 50+ peers) | Capped at 5 | **Fix** |
| Death spiral guard | None | `confirmed_height_floor` | **Fix** |
| Sync tests | 62 | 272 (4.4x) | **Fix** |
| Root-cause fixes | 0 | 21 | **Fix** |
| Patches/workarounds | 3 | 1 | **Fix** |
| Active modules | 7 (incl. `fork_sync.rs`) | 6 (replaced `fork_sync` with `ForkAction`) | Tie |
| Dead code | `chain_follower.rs` (876 lines, undeclared) | None | **Fix** |
| Lines changed | +53 (4 patches) | +5,783 / -3,385 | **Fix** (structural) |

## Question-by-Question

### 1. Which is better architecturally?

**Fix branch, decisively.** The SyncManager redesign is the kind of work that prevents entire classes of bugs:

- **State explosion eliminated**: 7 data-carrying variants -> 3 pure discriminants. On main, `SyncState::DownloadingHeaders { peer, target_slot, ... }` can carry stale data when re-entered. On the fix branch, data lives in `SyncPipelineData` and is cleared atomically on state transition.

- **Illegal states unrepresentable**: Main uses 5 independent booleans for recovery (`needs_genesis_resync`, `resync_in_progress`, `post_recovery_grace`, etc.) -- you can be simultaneously in resync AND grace, which is nonsensical. Fix branch uses `RecoveryPhase` enum (4 variants) -- impossible to be in two phases at once.

- **Production gate simplified**: Main's 11-layer gate includes hash agreement (Layer 9) that **deadlocks the chain** when relay nodes outvote producers (INC-I-008). Fix branch removed it entirely.

### 2. Which is stronger operationally?

**Fix branch.** Every fix is incident-driven -- real failures on a real testnet:

| Incident | What Broke | Main's Status | Fix Branch |
|----------|-----------|--------------|------------|
| INC-I-005 | Sync cascade feedback loop -- 43 resets/sec destroying sync state | **Unfixed** | Idempotent `start_sync()` guard |
| INC-I-006 | Hardcoded mainnet epoch in rebuild path | **Unfixed** | Network-aware accessors |
| INC-I-008 | Hash agreement deadlocked chain | **Unfixed** | Layer removed |
| INC-I-009 | Yamux 86GB RAM at 136 nodes | **Unfixed** | max_peers=25, Yamux 512KB |
| INC-I-012 | Post-snap header deadlock | **Unfixed** | Height-based fallback |
| INC-I-013 | Bootstrap backoff stuck 6/106 nodes | **Unfixed** | Bootstrap-only connections |

Main has **zero** of these fixes. It has 3 patches that the fix branch either already solved differently or superseded.

### 3. Which is more scalable?

**Fix branch.** The critical scalability fix is snap sync quorum cap:

- **Main**: Quorum = `total_peers/2 + 1`. At 100 peers, quorum = 51. Getting 51 peers to agree on the same state root within 15s is nearly impossible on an active chain. Result: **snap sync deadlocks on large networks.**
- **Fix branch**: Quorum = `max(3, min(total_peers/2+1, 5))`. At 100 peers, quorum = 5. Five agreeing peers is trivially achievable. Scalable to 1000+ nodes.

Other scalability wins on fix branch:
- Bootstrap-only connections (transient, don't consume permanent slots)
- Yamux window 512KB (vs default that explodes RAM)
- `conn_limit` proportional to `max_peers`

### 4. Which has a better foundation?

**Fix branch.** The foundation is the state machine design, and it's not close:

- Main's SyncManager is a 55-field struct where any method can touch any field. The interaction surface is O(n^2).
- Fix branch groups fields into `SyncPipeline`, `SnapSyncState`, `NetworkState`, `ForkState` -- each with clear ownership boundaries. Adding a new snap sync feature? Touch `SnapSyncState`. New fork logic? Touch `ForkState`.

The 3-state SyncState is also a better foundation. Adding a new sync strategy on main means adding a new variant with embedded data, updating every match arm, and hoping no stale data leaks between states. On the fix branch, you add a new `SyncPhase` label and a new `SyncPipelineData` variant -- the state machine itself doesn't change.

### 5. Is it worth merging?

**Yes, but merge main INTO the fix branch, not the other way around.**

Main has significant **feature work** that the fix branch needs:
- AMM pools, lending, NFT export, per-byte fees, bridge improvements
- Mempool poison fix, coinbase mismatch fix
- CLI improvements, version bumps

But main has **nothing** in the sync system that the fix branch doesn't already do better, with one exception:

> **Cherry-pick needed**: Main commit `90091c90` fixes `consecutive_sync_failures` being inflated by empty headers. The fix branch still has this bug.

### 6. Merge difficulty?

**Moderate-high** (25 conflicting files total), but the sync conflicts are **trivial** -- keep fix branch versions for all sync files. The real work is resolving non-sync conflicts (CLI, node core, Cargo.lock, new features touching shared files).

## Structural Differences

### Files only on main

| File | Lines | Status |
|------|-------|--------|
| `chain_follower.rs` | 876 | **Dead code** -- exists on disk but NOT declared in `mod.rs`. Zero references. |
| `fork_sync.rs` | 723 | **Active** -- declared in `mod.rs`, used by `block_handling.rs` (10 references). Replaced by `ForkAction` enum on fix branch. |

### Files only on fix branch

| File | Lines | Purpose |
|------|-------|---------|
| `manager/snap_sync.rs` | 330 | Extracted snap sync logic (voting, quorum, download, fallback) |
| `reorg/tests.rs` | 512 | Reorg handler tests (17 tests) |

### Main's 4 sync patches

| Commit | Change | Fix Branch Status |
|--------|--------|-------------------|
| `90091c90` | Rollback threshold 12->50, stop empty headers inflating sync_fails | **Partially missing** -- threshold change superseded, but sync_fails inflation bug still present |
| `cc056c41` | Genesis deadlock -- Layer6.5 blocked production at height 0 | **Superseded** -- Layer6.5 doesn't exist on fix branch |
| `94b62f01` | Deterministic fork tiebreak + post-recovery sync loop | **Superseded** -- fix branch has its own tiebreak + RecoveryPhase enum |
| `7dee86f5` | Structured diagnostic labels for fork root-cause analysis | **Superseded** -- fix branch has richer logging throughout |

## Recommendation

```
1. Merge origin/main INTO fix branch (fix branch = sync authority)
2. For ALL sync file conflicts: keep fix branch version
3. Cherry-pick the sync_fails inflation fix from 90091c90
4. For non-sync conflicts: manual resolution (feature code from main + structural changes from fix)
5. Run full test suite + testnet validation
6. PR fix branch back to main
```

The fix branch represents 29 commits of battle-tested sync hardening against 9 real incidents. Main's sync code is the version that **caused** those incidents. There is no scenario where reverting to main's sync system is the right call.

---

## Merge Process

### Step 1: Merge main into fix branch

```bash
git fetch origin main
git merge origin/main
```

This will produce ~25 conflicting files. Do NOT merge the other direction.

### Step 2: Resolve conflicts

**Sync files (`crates/network/src/sync/`)**: Always keep the fix branch version. Every sync file on the fix branch is strictly superior to main.

**Non-sync files** (manual resolution required):
- `crates/core/src/validation.rs` — main added AMM/lending/NFT validation rules. Keep main's additions, but preserve any fix branch structural changes.
- `crates/core/src/transaction.rs` — main added new TxTypes (Pool, Lending, etc.). Take main's additions.
- `bins/node/src/node/apply_block.rs` — main added apply logic for new TxTypes. Take main's additions, preserve fix branch's structural changes.
- `bins/node/src/node/block_handling.rs` — main uses `ForkSync`, fix branch uses `ForkAction`. Keep fix branch version, then add any new block handling for new TxTypes from main.
- `bins/node/src/node/event_loop.rs` — keep fix branch structure, integrate any new event handlers from main.
- `bins/node/src/node/rollback.rs` — keep fix branch (has death spiral prevention, confirmed_height_floor).
- `crates/network/src/service.rs` — keep fix branch (has bootstrap-only connections, Yamux 512KB, conn_limit changes).
- `Cargo.toml` / `Cargo.lock` — merge dependency additions from both sides.
- All other feature files (pool, lending, NFT, bridge) — take from main wholesale.

### Step 3: Cherry-pick sync_fails inflation fix

```bash
# Check if the merge already brought this in. If the conflict in sync_engine.rs
# didn't include it, apply manually:
# Main commit 90091c90 stops empty headers from inflating consecutive_sync_failures.
# Verify sync_engine.rs ~line 740 area handles this correctly.
```

### Step 4: Build and test

```bash
cargo build --release 2>&1 | tail -20
cargo clippy -- -D warnings
cargo fmt --check
cargo test
```

### Step 5: Testnet validation

```bash
# Copy binary
cp target/release/doli-node ~/testnet/bin/doli-node

# Reset testnet and verify:
# - All nodes sync to same height
# - Block production resumes
# - No fork cascades
# - Snap sync works for fresh nodes
```

### Step 6: Merge back to main

```bash
# Option A: Fast-forward merge
git checkout main
git merge fix/sync-state-explosion-root-causes

# Option B: PR (preferred for audit trail)
gh pr create --base main --head fix/sync-state-explosion-root-causes \
  --title "fix(sync): SyncManager redesign + 9 incident fixes" \
  --body "Merges the complete sync system overhaul (INC-I-005 through INC-I-013)"
```
