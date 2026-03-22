# Milestone Progress — Sync Manager Redesign (RUN_ID=44)

## Previous (INC-001 fixes — completed 2026-03-19)
| Milestone | Status |
|-----------|--------|
| M1: Fix Rollback Loop | COMPLETE |
| M2: Fix Production Gate | COMPLETE |
| M3: Reduce Empty Slots | COMPLETE |

## Current Redesign (approved 2026-03-22, M3 deferred)

| Milestone | Status | PRs |
|-----------|--------|-----|
| M1: Dead Code Removal + Data Grouping | COMPLETE | PR1-PR3 (PR4 deferred — thresholds move with M3 sub-structs) |
| M2: Fork Coordination + Idempotent Sync | COMPLETE | PR5-PR7 |
| M3: Production Gate + Cleanup | DEFERRED | defer until network stable |

| PR | Description | Status |
|----|-------------|--------|
| PR1 | Delete chain_follower.rs (876 lines dead code) | COMPLETE |
| PR2 | Remove 4 dead fields + merge production_blocked | COMPLETE |
| PR3 | Extract SyncPipeline/SnapSyncState/NetworkState/ForkState sub-structs | COMPLETE |
| PR4 | Extract SyncConfig thresholds | DEFERRED (thresholds belong to M3 domains) |
| PR5 | start_sync() idempotent guard clause (INC-I-004/005 root cause fix) | COMPLETE |
| PR6 | ForkState sub-struct (15 fields) + ForkAction enum | COMPLETE |
| PR7 | State transition logging (40 transitions, unique triggers) | COMPLETE |

## Metrics

| Metric | Before | After | Target |
|--------|--------|-------|--------|
| SyncManager direct fields | 55 | 31 | ≤20 (M3 gets remaining ~13 production gate fields) |
| Dead code lines | 876 | 0 | 0 |
| Sub-structs | 0 | 4 (SyncPipeline, SnapSyncState, NetworkState, ForkState) | 5 (ProductionGate in M3) |
| State transition logging | 0 | 40 with unique triggers | All |
| start_sync() idempotent | No (43 calls/sec destructive reset) | Yes (guard clause) | Yes |
| ForkAction coordination | No (3 systems, no coordinator) | Enum defined, not yet wired | Wired into Node |
| Tests passing | 146 | 146 | 146 |
