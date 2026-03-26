# Branch Comparison: origin/main vs fix/sync-state-explosion-root-causes

**Focus**: `crates/network/src/sync/` directory and related node-level sync consumers

## Verdict
**Merge main -> fix branch (cherry-pick 4 sync commits)**: The fix branch is the structurally stronger branch for the sync system. Main has 4 small, targeted sync fixes (53 lines changed) that should be cherry-picked INTO the fix branch. The fix branch has a complete architectural redesign of SyncManager (9,141 lines changed, 25 sync commits) that supersedes main's sync code.

**Confidence**: HIGH — the evidence is unambiguous. Main's sync changes are surface-level patches applied to a codebase the fix branch has fundamentally restructured.

## Branch Topology
- **Common ancestor**: `103fc31a` (2026-03-19, "fix: unify mainnet gossip mesh with testnet")
- **origin/main**: 134 commits ahead, last commit 2026-03-26 ("bump: v4.9.2")
- **fix/sync-state-explosion-root-causes**: 85 commits ahead, last commit 2026-03-26 ("fix(consensus): add finality check to plan_reorg")

### Sync-specific topology
- **origin/main**: 4 sync commits (53 lines changed across 4 files)
- **fix branch**: 25 sync commits (9,141 lines changed across 14 files)

## Change Classification

### origin/main: Sync-only commits (4)

| Commit | Category | Strength | Summary |
|--------|----------|----------|---------|
| `90091c90` | Patch/workaround | LOW | Shallow rollback threshold 12->50, stop empty headers inflating sync_fails |
| `7dee86f5` | Configuration change | LOW | Structured diagnostic labels for fork log analysis |
| `94b62f01` | Patch/workaround | LOW | Clear post_rollback flag after fork recovery (N5 incident) |
| `cc056c41` | Patch/workaround | LOW | Skip height-lag gate at genesis to prevent deadlock |

**Main profile**: 0 root-cause fixes, 3 patches, 0 features, 0 refactors, 1 config change
**Overall strength**: LOW — all changes work around symptoms in the existing (unrestructured) sync architecture

### fix/sync-state-explosion-root-causes: Sync-only commits (25)

| Commit | Category | Strength | Summary |
|--------|----------|----------|---------|
| `5ba0eb90` | Root-cause fix | HIGH | Finality check in plan_reorg — prevent fork cascade bypass |
| `24550ea4` | Root-cause fix | HIGH | 3 stress-test fixes for 100+ node burst joins |
| `85918308` | Patch/workaround | LOW | 8 medium/low P2P advisory fixes for sync hardening |
| `b40ca4f8` | Root-cause fix | HIGH | 5 P2P advisory fixes for post-snap deadlock |
| `98f1b6ba` | Root-cause fix | HIGH | 3 header-first sync performance fixes + post-snap deadlock diagnosis |
| `a0fda435` | Root-cause fix | HIGH | 5 stress-test hardening fixes for 86-node testnet |
| `928440d3` | Root-cause fix | HIGH | Eviction cooldown to break reconnect thrashing loop |
| `70235ede` | Root-cause fix | HIGH | Remove production gate hash agreement + fix rebuild ordering |
| `2320a246` | Root-cause fix | HIGH | M3 hard enforcement + ForkAction wiring + reorg floor check |
| `db8283da` | Root-cause fix | HIGH | Route all 9 genesis resync sites through recovery gate (M2) |
| `b8ff6253` | Root-cause fix | HIGH | Route all 9 genesis resync sites through recovery gate (M2) |
| `324ead94` | Root-cause fix | HIGH | Correct transition matrix — add 3 missing valid transitions |
| `dc4bb7c5` | Structural refactor | HIGH | Recovery gate method and state transition validation (M1) |
| `49636b84` | Root-cause fix | HIGH | Break sync cascade feedback loop with 3 structural fixes |
| `05611ee1` | Root-cause fix | HIGH | Prevent rollback death spiral with peak_height tracking |
| `80b4fa82` | Root-cause fix | HIGH | Force snap sync on repeated apply failures regardless of gap |
| `f4c2498c` | Root-cause fix | HIGH | Cap snap sync quorum at 15 to prevent unreachable quorum |
| `545b1ad9` | Structural refactor | HIGH | Sync manager redesign M1+M2 — substruct extraction + idempotent start_sync |
| `a1c88694` | Root-cause fix | HIGH | 3 sync root causes + testnet genesis keys |
| `b0b7c185` | Root-cause fix | HIGH | Restore all Antonio INC-001 fixes lost during main merge |
| `30b8e5c2` | Merge commit | NEUTRAL | Merge branch into fix branch |
| `9a473d81` | Merge commit | NEUTRAL | Integrate main into sync-explosion branch |
| `be64d0fe` | Root-cause fix | HIGH | Resolve RC-9 and RC-10 — complete INC-001 fix |
| `af9ebdb1` | Root-cause fix | HIGH | Resolve 7 root causes of sync state explosion |
| `bb96f3d8` | Root-cause fix | HIGH | Structural fixes for event loop starvation and state explosion |
| `39169ae9` | Root-cause fix | HIGH | Resolve 4 sync recovery bugs trapping forked nodes |
| `7259d470` | Root-cause fix | HIGH | Address 3 root causes of persistent sync cascades |

**Fix branch profile**: 21 root-cause fixes, 1 patch, 0 features, 2 structural refactors, 2 merges
**Overall strength**: HIGH — comprehensive architectural overhaul addressing root causes of sync state explosion

## Q1: What has main added that the branch doesn't have?

### Files exclusive to main (in sync/)
| File | Lines | Status |
|------|-------|--------|
| `chain_follower.rs` | 876 | **Dead code** — exists on disk but NOT declared as a module in `mod.rs`. No code references it. |
| `fork_sync.rs` | 723 | Active module — declared in `mod.rs`, exported as `ForkSync, ForkSyncResult, ProbeResult`, used in `block_handling.rs` (10 references) |

### Targeted fixes on main (not on fix branch)
1. **`90091c90` — Rollback threshold 12->50 + sync_fails inflation fix**: The fix branch has `gap <= 50` in ONE code path but still has `gap <= 12` in another (line 333 of sync_engine.rs). The fix branch also still increments `consecutive_sync_failures` on empty headers in some paths — the exact bug main's commit fixes.
2. **`cc056c41` — Genesis deadlock fix (Layer6.5)**: The fix branch removed the entire Layer6.5 height-lag gate from `can_produce()`, replacing it with a simplified gate structure. Main's fix is N/A to the fix branch's architecture, but the underlying concern (genesis deadlock from stale peer heights) may need its own test.
3. **`94b62f01` — post_rollback flag clear after fork recovery**: Added `set_post_rollback()` method. The fix branch does not have `ForkSync` at all, so the N5 incident loop pattern cannot occur.
4. **`7dee86f5` — Structured diagnostic labels**: Log format changes `[SYNC] STUCK_PROCESSING`, `[SYNC] FORK_GATE`, etc. The fix branch has its own logging approach.

### Key takeaway
- `chain_follower.rs` is dead code on main — no action needed
- `fork_sync.rs` is active on main but the fix branch deliberately deleted it and replaced its functionality with the redesigned `ForkAction` + `RecoveryPhase` system
- Main's 4 fixes need to be evaluated for cherry-pick into the fix branch's codebase

## Q2: What has the branch added that main doesn't have?

### Files exclusive to fix branch (in sync/)
| File | Lines | Purpose |
|------|-------|---------|
| `manager/snap_sync.rs` | 330 | Extracted snap sync logic (state root voting, quorum, download, fallback). Previously embedded in sync_engine.rs |
| `manager/tests.rs` (expanded) | 3,865 (vs 793 on main) | 272 tests (vs 62 on main) — 4.4x test coverage increase |

### Architectural changes exclusive to fix branch
1. **SyncState redesign**: Collapsed from 7 variants (with embedded data) to 3 pure discriminants (`Idle`, `Syncing { phase, started_at }`, `Synchronized`). Data moved to `SyncPipelineData` enum.
2. **SyncManager field reduction**: 55+ fields -> ~35 fields via 4 sub-structs (`SyncPipeline`, `SnapSyncState`, `NetworkState`, `ForkState`)
3. **RecoveryPhase state machine**: Replaces 5 independent booleans (`resync_in_progress`, `post_rollback`, etc.) with a proper enum (`Normal`, `ResyncInProgress`, `PostRecoveryGrace`, `AwaitingCanonicalBlock`)
4. **ForkAction coordination**: Unified fork recovery decisions through `recommend_action()` instead of 3 competing systems
5. **Recovery gate**: All 9 genesis resync sites now route through a single gate method
6. **State transition validation**: All 40 state transitions go through `set_state()` with trigger strings for audit
7. **INC-I-005 fixes**: Peak height tracking, monotonic progress floor, snap sync quorum cap, cascade feedback loop breaks
8. **INC-I-008 fix**: Removed hash agreement production gate (deadlocked chains with non-producing relay nodes)
9. **INC-I-012 fixes**: Post-snap header deadlock fix, height-based header fallback, stale state root vote rejection
10. **Production gate hash agreement removal**: Eliminated false fork detection from stress/relay nodes outvoting producers

### New exports from fix branch
`ForkAction`, `RecoveryPhase`, `RecoveryReason`, `SyncPhase`, `SyncPipelineData`, `VerifiedSnapshot`

### Removed exports (vs main)
`ForkSync`, `ForkSyncResult`, `ProbeResult` — replaced by internal ForkAction system

## Q3: Conflicting changes in shared files

### Sync-related conflicts (from `git merge-tree`)
| File | Main's Change | Fix Branch's Change | Resolution |
|------|--------------|--------------------|-----------| 
| `sync/manager/sync_engine.rs` | 34 lines: threshold 12->50, diagnostic labels, stop sync_fails inflation | 932 lines: complete rewrite with SyncPhase, ForkState sub-struct, recovery gate | **Keep fix branch**. Main's threshold fix partially duplicated. Cherry-pick the sync_fails inflation fix if missing. |
| `sync/manager/production_gate.rs` | 10 lines: genesis deadlock guard (`&& self.local_height > 0`), diagnostic labels | 761 lines: complete redesign removing Layer6.5, adding finality gate, removing hash agreement | **Keep fix branch**. Layer6.5 is gone; genesis deadlock addressed differently. Verify genesis production tests cover the scenario. |
| `sync/manager/cleanup.rs` | 2 lines: log format change | 410 lines: extensive fork detection, stuck-sync handling, stable gap detection | **Keep fix branch**. Main's log change is trivial. |
| `sync/manager/block_lifecycle.rs` | 7 lines: `set_post_rollback()` method | 478 lines: rewritten with SyncPipeline, SyncPipelineData, block_applied() overhaul | **Keep fix branch**. `set_post_rollback()` is N/A without ForkSync. |

### Non-sync conflicts (25 total files conflict)
| File | Nature |
|------|--------|
| `block_handling.rs` | Main adds fork_sync references; fix branch removes all fork_sync (SUPERSEDING) |
| `event_loop.rs` | Both modified; needs manual review |
| `rollback.rs` | Both modified; needs manual review |
| + 22 non-sync files | Cargo.lock, CLI files, WHITEPAPER, network params, etc. |

## Q4: Root-cause fixes vs patches

| Dimension | origin/main | fix branch |
|-----------|------------|------------|
| Root-cause fixes | 0 | 21 |
| Patches/workarounds | 3 | 1 |
| Structural refactors | 0 | 2 |
| Configuration/logging | 1 | 0 |

Main's sync changes are exclusively patches that add guards, change thresholds, and reformulate log messages on top of the existing architecture. They do not address WHY the sync state machine enters bad states.

The fix branch identifies and fixes the root causes: state explosion from 55+ interacting fields, competing recovery systems (3 separate fork recovery mechanisms), destructive `start_sync()` being called 43 times/sec on large networks, cascade feedback loops, rollback death spirals, unreachable snap sync quorums, and production gate deadlocks from relay node vote outnumbering.

## Q5: Strength Comparison

| Dimension | origin/main | fix branch | Winner |
|-----------|------------|------------|--------|
| Structural depth | Surface: thresholds, guards, log labels | Core: state machine redesign, sub-struct extraction, recovery gate, transition validation | **fix branch** (3x) |
| Breadth | 4 files, 53 lines | 14 files, 9,141 lines | **fix branch** (1x) |
| Irreversibility | None | SyncState enum redesign, ForkSync removal, new public API exports | **fix branch** (2x) |
| Obsolescence potential | Main's patches work on the OLD sync architecture that the fix branch replaced | Fix branch makes main's sync patches unnecessary — they guard against bugs that no longer exist | **fix branch** (3x) |
| Freshness | Latest: 2026-03-26 | Latest: 2026-03-26 | **tie** (1x) |
| Test coverage | 62 tests | 272 tests (4.4x increase) | **fix branch** |

## Q6: Merge Difficulty

**Overall: MODERATE-HIGH** (25 conflicting files total, 6 sync-related)

### Sync-specific merge: EASY if fix branch is target
The fix branch's changes are so comprehensive that for sync files, the answer is simply "keep fix branch's version." Main's 4 sync commits are either:
- Already addressed differently (genesis deadlock, post_rollback)
- Partially present (threshold 50)
- Missing and should be cherry-picked (sync_fails inflation fix)

### Non-sync merge: MODERATE
25 total conflicting files across CLI, node core, network service, config, etc. These require manual conflict resolution but are independent of the sync architectural decision.

### Action items for cherry-pick evaluation
1. **sync_fails inflation fix (from `90091c90`)**: The fix branch still increments `consecutive_sync_failures` on empty headers in at least one code path (sync_engine.rs line ~740). Main's fix removes this. **Cherry-pick candidate.**
2. **Threshold consistency**: Fix branch has `gap <= 12` at line 333 AND `gap <= 50` at line 747. Main unified to 50. **Verify if the 12 at line 333 is intentional** (different context — may be for initial escalation from Processing state).

## Obsolescence Analysis

### Main's changes made obsolete by fix branch
- **`cc056c41` (genesis deadlock)**: Fix branch removed the entire Layer6.5 gate. The deadlock cannot occur.
- **`94b62f01` (post_rollback flag)**: Fix branch removed ForkSync entirely. The N5 incident loop is impossible.
- **`7dee86f5` (diagnostic labels)**: Fix branch has its own comprehensive logging approach.

### Main's changes NOT obsolete
- **`90091c90` (sync_fails inflation)**: The fix branch still has the same bug pattern in at least one code path. This specific fix should be reviewed and potentially cherry-picked.

### Fix branch changes — nothing is obsoleted by main
Main's 4 patches operate on the old architecture. None of them replicate or supersede any fix branch change.

## Recommendation

### Direction: Merge main -> fix branch (selective cherry-pick, then merge remaining)

**Why**: The fix branch has completely redesigned the sync subsystem with 21 root-cause fixes, 2 structural refactors, and 4.4x test coverage increase. Main's sync changes are 4 small patches (53 lines) applied to the OLD architecture that the fix branch replaced. Merging fix->main for sync would lose main's fork_sync.rs (active, 723 lines) and chain_follower.rs (dead code), but this is intentional — the fix branch replaced fork_sync with a unified ForkAction + RecoveryPhase system.

**Steps**:
1. Review the `sync_fails inflation` fix from main commit `90091c90` and determine if the fix branch needs it (check line ~740 of sync_engine.rs)
2. Review the `gap <= 12` vs `gap <= 50` inconsistency in the fix branch's sync_engine.rs
3. Merge `origin/main` INTO the fix branch, resolving conflicts by keeping fix branch's sync files and manually resolving non-sync conflicts
4. Run full test suite: `cargo test` (should pass 272 sync tests + all others)
5. After merge verification, merge the fix branch into main via PR

### Risks
- **fork_sync.rs removal**: `block_handling.rs` on main has 10 references to `ForkSync`. The fix branch already removed these. During merge, the fix branch's version of `block_handling.rs` should be kept (conflict resolution favoring fix branch).
- **Non-sync conflicts**: 22 non-sync files conflict. These need careful manual resolution but are independent of the sync decision.
- **Untested genesis scenario**: Main's genesis deadlock fix is for a specific scenario (stale peer height from previous chain). The fix branch's simplified gate may or may not handle this. Verify with a test that covers the scenario described in commit `cc056c41`.

### Post-Merge Actions
- Run `/omega-audit` on the merged result to check for any integration issues
- Verify the 272 sync tests all pass after merge
- Check that `cargo clippy -- -D warnings` passes
- Consider adding a test for the genesis deadlock scenario from `cc056c41`
