# Architecture: Sync Manager Redesign

*Scope: `crates/network/src/sync/`, `bins/node/src/node/periodic.rs`, `bins/node/src/node/rollback.rs`*
*Requirements source: `docs/redesigns/syncmanager-redesign-analysis.md`*
*Reasoning trace: `docs/.workflow/architecture-reasoning.md`*

---

## Design Space Analysis

### Existing Patterns

| Pattern | Where | Status |
|---------|-------|--------|
| Enum with data (SyncState, RecoveryPhase, ProductionAuthorization) | mod.rs | Idiomatic Rust, keep |
| Sub-struct for config (SyncConfig) | mod.rs | Already exists, extend pattern |
| Sub-struct for peer state (PeerSyncStatus) | mod.rs | Already exists, extend pattern |
| Request/response model (next_request/handle_response) | sync_engine.rs | Wire protocol, do not change |
| Impl block per file (cleanup, production_gate, block_lifecycle, snap_sync, sync_engine) | manager/ | Already exists, extend pattern |
| ForkSync as Option<> parallel state machine | block_lifecycle.rs | Keep pattern |
| ForkRecoveryTracker as owned sub-struct | fork_recovery.rs | Keep pattern |

### Tech Stack Constraints

1. **Rust ownership**: Sub-structs in SyncManager are owned. Methods that need cross-struct data pass it as parameters or take `&mut self` on the parent.
2. **Single-threaded event loop**: All SyncManager calls are synchronous from Node's event loop. No concurrency concerns within SyncManager.
3. **Wire protocol frozen**: SyncRequest/SyncResponse enums do not change. Internal restructuring only.
4. **81 call sites in Node**: The public API surface of SyncManager is large. Changes must preserve API or provide clear migration path.

### Architecture Constraint Table (from incidents)

| Constraint | Source | Impact |
|-----------|--------|--------|
| start_sync() must not nuke in-flight fork_sync requests | INC-001: fork sync loop | Decision 3: guard clause |
| consecutive_empty_headers must not be reset by gossip | INC-I-004: sync loop | Decision 2: counter semantics preserved |
| can_produce() must be side-effect-free | 2026-03-15 production gate deadlock | Decision 4: no mutations in gate |
| Snap sync cascade: Hash::ZERO from a single peer must not trigger chain reaction | INC from 2026-03-14 | Keep existing fix, don't regress |
| Fork sync cooldown prevents infinite reorg loops | INC-001 | Decision 2: preserve cooldown/breaker |

### Anti-Overengineering Gate

**Q0 (Subtraction):** YES. Dead code (chain_follower.rs, Layer 7), dead fields (max_heights_behind, max_heights_ahead), and redundant production gate layers can be removed.

**Q1 (Does this need solving?):** YES. 55 fields on one struct caused the INC-I-004 sync loop, the production gate deadlock, and 3 other incidents. This is not cosmetic.

**Q2 (At this scale?):** YES. Network of 12-44 nodes. The complexity is already causing production incidents.

**Q3 (Simplest solution?):** The design below removes ~800 lines of code (chain_follower.rs + dead fields + dead Layer 7 code), adds ~5 sub-structs (no new files or modules), and preserves all behavior. This is the minimal structural change that addresses the requirements.

---

## Overview

The redesign applies **subtraction first**: remove dead code, dead fields, and redundant checks. Then **group** remaining fields into typed sub-structs, and **decompose** the cleanup god function into named functions.

```
BEFORE (current):
  SyncManager (55 fields, 1 struct, god-function cleanup)
  + ForkRecoveryTracker (fork_recovery.rs)
  + ForkSync (fork_sync.rs)
  + resolve_shallow_fork (Node-side, periodic.rs/rollback.rs)
  + chain_follower.rs (DEAD CODE)

AFTER (proposed):
  SyncManager (~10 top-level fields, 5 sub-structs)
  ├── SyncPipeline (headers, bodies, pending requests, downloaders)
  ├── ProductionGate (all production authorization state)
  ├── ForkState (fork detection/recovery coordination)
  ├── SnapSyncState (snap sync config + runtime state)
  └── NetworkState (network tip, timing, gossip tracking)
  + ForkRecoveryTracker (unchanged)
  + ForkSync (unchanged)
  - chain_follower.rs (DELETED)
  - resolve_shallow_fork (ABSORBED into ForkState coordination)
```

### What Gets DELETED

| Item | Lines | Reason |
|------|-------|--------|
| `chain_follower.rs` | 795 | Dead code. Not in mod.rs, not referenced anywhere. |
| `max_heights_behind` field | ~5 | Set in constructor, never read. Dead since Layer 6 uses slots only. |
| `max_heights_ahead` field | ~5 | Set by setter, never read. Layer 7 (ahead check) was removed. |
| Layer 7 dead comments/logging in `production_gate.rs` | ~15 | Comments for removed layer. Noise. |
| `has_connected_to_peer` field | ~3 | Derivable from `first_peer_status_received.is_some()`. |
| `production_block_reason` field | ~3 | Merge into `production_blocked: Option<String>` (None = not blocked). |
| **Total deleted** | **~826 lines** | |

---

## Modules

### Module 1: SyncPipeline (sub-struct)

**Responsibility**: Manages the header-first sync pipeline state: pending headers, bodies, blocks, requests, downloaders, and the sync epoch counter.

**Fields** (moved from SyncManager):
```rust
pub(crate) struct SyncPipeline {
    pub pending_headers: VecDeque<BlockHeader>,
    pub headers_needing_bodies: VecDeque<Hash>,
    pub pending_blocks: HashMap<Hash, Block>,
    pub pending_requests: HashMap<SyncRequestId, PendingRequest>,
    pub next_request_id: u64,
    pub sync_epoch: u64,
    pub header_downloader: HeaderDownloader,
    pub body_downloader: BodyDownloader,
    pub body_stall_retries: u32,
}
```

**Methods**: `clear()`, `bump_epoch()`, `register_request()`, `has_pending_work()`.

**Dependencies**: HeaderDownloader, BodyDownloader (existing, unchanged).

**Implementation order**: M1 (foundational — other sub-structs don't depend on this).

#### Failure Modes
| Failure | Cause | Detection | Recovery | Impact |
|---------|-------|-----------|----------|--------|
| Stale requests from old epoch | start_sync() bumped epoch | Request tagged with epoch, response discarded if mismatch | Automatic | None — existing behavior preserved |
| Body download stall | Parallel responses arrive out of order | body_stall_retries counter | Soft retry (rebuild needed list), hard reset after 3 | Sync restart |

#### Performance Budget
- **Latency target**: clear() < 1ms (HashMap/VecDeque clear is O(n) where n is allocated capacity, typically < 1000)
- **Memory budget**: Bounded by max_headers_per_request (500) * header_size + max_concurrent_body_requests (8) * block_size

---

### Module 2: ProductionGate (sub-struct)

**Responsibility**: Holds all state needed for production authorization decisions. Includes recovery phase, bootstrap state, gossip tracking, resync tracking.

**Fields** (moved from SyncManager):
```rust
pub(crate) struct ProductionGate {
    // Explicit block
    pub production_blocked: Option<String>,  // None = not blocked, Some(reason) = blocked
    // Recovery
    pub recovery_phase: RecoveryPhase,
    pub last_resync_completed: Option<Instant>,
    pub consecutive_resync_count: u32,
    pub blocks_since_resync_completed: u32,
    // Thresholds (from SyncConfig or separate GateConfig)
    pub resync_grace_period_secs: u64,
    pub max_grace_cap_secs: u64,
    pub max_slots_behind: u32,
    pub min_peers_for_production: usize,
    pub bootstrap_grace_period_secs: u64,
    pub peer_loss_timeout_secs: u64,
    // Gossip tracking
    pub last_block_received_via_gossip: Option<Instant>,
    pub gossip_activity_timeout_secs: u64,
    pub max_solo_production_secs: u64,
    // Bootstrap
    pub first_peer_status_received: Option<Instant>,
    pub last_peer_status_received: Option<Instant>,
    pub peers_lost_at: Option<Instant>,
    // Fork detection (from gate perspective)
    pub fork_mismatch_detected: bool,
    // Tier
    pub tier: u8,
    // Height lag tracking
    pub behind_since: Option<Instant>,
}
```

**Methods**: `can_produce(context) -> ProductionAuthorization`, `update_state(context)`, `start_resync()`, `complete_resync()`, `block_production(reason)`, `unblock_production()`, `is_in_bootstrap_phase(local_height, peers_empty)`.

**Key change**: `can_produce()` and `update_production_state()` take a `GateContext` struct as parameter instead of reading 10+ fields from `self`:
```rust
pub(crate) struct GateContext {
    pub local_height: u64,
    pub local_hash: Hash,
    pub local_slot: u32,
    pub peers: &HashMap<PeerId, PeerSyncStatus>,  // or summary
    pub sync_state: &SyncState,
    pub fork_sync_active: bool,
    pub best_peer_height: u64,
    pub best_peer_slot: u32,
    pub network_tip_height: u64,
    pub network_tip_slot: u32,
    pub last_finalized_height: Option<u64>,
}
```

**Merged layers**:
- Layer 8 (consecutive_sync_failures check) REMOVED from can_produce(). The persistent `fork_mismatch_detected` flag (set by update_production_state) already covers this. `consecutive_sync_failures` stays as a field on ForkState for fork detection logic but is NOT checked in the production gate.
- Layer 10 + 10.5 merged into single gossip health check.
- Layer 7 dead code deleted.

**Result**: ~9 check phases (down from 13). Still above the "5" target, but each remaining check is non-redundant and safety-critical.

**Dependencies**: GateContext (constructed by SyncManager from its own state).

**Implementation order**: M2 (depends on M1 only for shared types).

#### Failure Modes
| Failure | Cause | Detection | Recovery | Impact |
|---------|-------|-----------|----------|--------|
| Grace period deadlock | Exponential backoff disables production for 480s+ | max_grace_cap_secs (60s hard cap) | Automatic cap | Already fixed (PGD-002) |
| Fork oscillation | Layer 9 detects/forgets fork repeatedly | Persistent fork_mismatch_detected flag | Flag cleared only on successful resync | Already fixed |
| Bootstrap deadlock | All peers at height 0, no chain evidence | bootstrap_grace_period_secs timeout | Produce after timeout expires | Already fixed |

#### Security Considerations
- **Trust boundary**: Production gate is the final safety check before producing a block. It must never produce on a known fork.
- **Attack surface**: A malicious peer reporting false height/hash could cause production to be blocked or unblocked incorrectly. Mitigation: majority voting (Layer 9), minimum peer count (Layer 5.5).

---

### Module 3: ForkState (sub-struct)

**Responsibility**: Coordinates all fork detection and recovery state. Single point of truth for "are we on a fork, and what are we doing about it?"

**Fields** (moved from SyncManager):
```rust
pub(crate) struct ForkState {
    // Fork detection
    pub consecutive_empty_headers: u32,
    pub consecutive_sync_failures: u32,
    pub max_sync_failures_before_fork_detection: u32,
    pub consecutive_apply_failures: u32,
    pub needs_genesis_resync: bool,
    // Fork sync coordination
    pub fork_sync: Option<ForkSync>,
    pub last_fork_sync_rejection: Instant,
    pub fork_sync_cooldown_secs: u64,
    pub consecutive_fork_syncs: u32,
    pub last_fork_sync_at: Option<Instant>,
    pub recently_held_tips: Vec<(Hash, Instant)>,
    // Fork recovery (orphan chain walk)
    pub fork_recovery: ForkRecoveryTracker,
    // Stall detection
    pub stable_gap_since: Option<(u64, Instant)>,
    // Blacklists
    pub header_blacklisted_peers: HashMap<PeerId, Instant>,
}
```

**Methods**: `signal_stuck_fork()`, `start_fork_sync()`, `is_fork_sync_active()`, `is_fork_sync_breaker_tripped()`, `reset_fork_sync_breaker()`, `mark_fork_sync_rejected()`, `record_held_tip()`, `is_recently_held_tip()`, `cleanup_blacklists()`.

**Key change**: The decision logic currently split between:
1. SyncManager.cleanup() (stuck detection, escalation)
2. Node.resolve_shallow_fork() (rollback vs binary search decision)
3. SyncManager.signal_stuck_fork() (sets RecoveryPhase)

...is consolidated into ForkState methods. The Node still executes rollbacks (block_store access needed), but the DECISION (rollback vs fork_sync vs snap sync) is made by `ForkState::recommend_action()`:

```rust
pub enum ForkAction {
    None,
    RollbackOne,           // gap <= 12, rollback count < limit
    StartForkSync,         // gap <= 1000, binary search
    NeedsGenesisResync,    // gap > 1000 or fork_sync bottomed out
}

pub fn recommend_action(&self, gap: u64, local_height: u64, has_peer: bool) -> ForkAction { ... }
```

Node's `resolve_shallow_fork()` simplifies from ~70 lines of decision logic to:
```rust
match fork_state.recommend_action(gap, local_height, has_peer) {
    ForkAction::RollbackOne => self.rollback_one_block().await,
    ForkAction::StartForkSync => fork_state.start_fork_sync(...),
    ForkAction::NeedsGenesisResync => { fork_state.needs_genesis_resync = true; },
    ForkAction::None => {},
}
```

**Dependencies**: ForkSync (existing, unchanged), ForkRecoveryTracker (existing, unchanged).

**Implementation order**: M2 (can be done in parallel with ProductionGate).

---

### Module 4: SnapSyncState (sub-struct)

**Responsibility**: All snap sync configuration and runtime state.

**Fields** (moved from SyncManager):
```rust
pub(crate) struct SnapSyncState {
    pub threshold: u64,
    pub quorum: usize,
    pub root_timeout: Duration,
    pub download_timeout: Duration,
    pub blacklisted_peers: HashSet<PeerId>,
    pub attempts: u8,
    pub fresh_node_wait_start: Option<Instant>,
    pub store_floor: u64,
}
```

**Methods**: `should_snap_sync(gap, peer_count)`, `record_attempt()`, `blacklist_peer()`, `reset()`.

**Dependencies**: None.

**Implementation order**: M1 (simple data grouping, no logic changes).

---

### Module 5: NetworkState (sub-struct)

**Responsibility**: Network tip tracking, gossip timing, block application counters.

**Fields** (moved from SyncManager):
```rust
pub(crate) struct NetworkState {
    pub network_tip_height: u64,
    pub network_tip_slot: u32,
    pub last_block_seen: Instant,
    pub last_block_applied: Instant,
    pub last_sync_activity: Instant,
    pub blocks_applied_counter: u64,
    pub last_progress_log: Instant,
    pub idle_behind_retries: u32,
}
```

**Methods**: `note_block_applied()`, `note_sync_activity()`, `update_tip(height, slot)`.

**Dependencies**: None.

**Implementation order**: M1 (simple data grouping).

---

### Resulting SyncManager Struct

After all sub-struct extractions:

```rust
pub struct SyncManager {
    // Core
    config: SyncConfig,
    state: SyncState,
    local_height: u64,
    local_hash: Hash,
    local_slot: u32,
    peers: HashMap<PeerId, PeerSyncStatus>,
    // Sub-structs
    pipeline: SyncPipeline,
    gate: ProductionGate,
    fork: ForkState,
    snap: SnapSyncState,
    network: NetworkState,
    // Owned subsystems (not sub-structs — they have their own modules)
    reorg_handler: ReorgHandler,
    finality_tracker: FinalityTracker,
}
```

**Direct fields: 13** (down from 55). Target was <=20. ACHIEVED.

---

## Failure Modes (system-level)

| Scenario | Affected Modules | Detection | Recovery Strategy | Degraded Behavior |
|----------|-----------------|-----------|-------------------|-------------------|
| All peers disconnect | NetworkState, ProductionGate | peers.is_empty() + peers_lost_at timeout | Wait peer_loss_timeout_secs, then resume solo production | Solo production (may fork) |
| Stuck on deep fork (gap > 1000) | ForkState, SnapSyncState | consecutive_empty_headers >= 10 OR stuck 120s | Snap sync from quorum peers | Node restarts from snap sync point |
| Stuck on shallow fork (gap <= 12) | ForkState | consecutive_empty_headers >= 3 | Rollback 1 block per tick, then ForkSync binary search | Brief production pause |
| Body download stall | SyncPipeline | last_sync_activity > 120s | Soft retry (rebuild needed list), hard reset after 3 | Sync restart |
| Production gate deadlock | ProductionGate | Grace period exceeds max_grace_cap_secs | Hard cap at 60s | Production resumes after cap |

## Security Model

### Trust Boundaries
- **Peer status data**: Untrusted. Peers can report false height/hash/slot. Mitigated by majority voting in production gate Layer 9 and snap sync quorum.
- **Block data**: Validated by consensus rules before application. SyncManager only coordinates download, not validation.
- **State snapshots**: Verified by computing state root from downloaded data and comparing to quorum-agreed root.

### Attack Surface
- **Single-peer poisoning**: A peer reports inflated height to trigger unnecessary snap sync. Mitigation: consensus_target_hash() requires >= 2 peers agreeing. Snap sync requires quorum of 5.
- **Fork oscillation**: Alternating peer status causes can_produce() to oscillate. Mitigation: persistent fork_mismatch_detected flag, fork_sync cooldown, recently_held_tips ring buffer.
- **Hash::ZERO injection**: Peer reports Hash::ZERO as best_hash. Mitigation: Hash::ZERO explicitly skipped in Layer 9 comparison (existing fix from 2026-03-14 cascade).

---

## Graceful Degradation

| Dependency | Normal Behavior | Degraded Behavior | User Impact |
|-----------|----------------|-------------------|-------------|
| All peers lost | Sync from peers, validate via gossip | Solo production after timeout | May create short fork |
| Best peer on wrong chain | Sync from majority | Fork detection triggers rollback/resync | Brief production pause |
| Snap sync quorum unreachable | Snap sync in seconds | Fall back to header-first sync (hours) | Slow initial sync |

---

## Performance Budgets

| Operation | Latency (p50) | Latency (p99) | Memory | Notes |
|-----------|---------------|---------------|--------|-------|
| can_produce() | < 10us | < 100us | 0 alloc | Read-only query over sub-structs |
| cleanup() | < 1ms | < 5ms | 0 alloc | Timer checks + peer iteration |
| start_sync() | < 100us | < 1ms | Allocs for HashMap/VecDeque clears | Now idempotent — no work if already syncing |
| Fork recovery (gap 51-1000) | < 120s total | < 120s | O(log N) blocks | Binary search via ForkSync |

---

## Data Flow

```
Node event_loop
  │
  ├── handle_network_event(StatusResponse)
  │   └── sync_manager.update_peer() → may call start_sync()
  │                                      ↓ (guarded: skip if syncing)
  │                                    pipeline.clear() + state = DownloadingHeaders
  │
  ├── run_periodic_tasks()
  │   ├── get_blocks_to_apply() → returns ordered blocks from pipeline.pending_blocks
  │   ├── cleanup() → dispatches to:
  │   │   ├── pipeline: cleanup_request_timeouts(), cleanup_body_timeouts()
  │   │   ├── peers: cleanup_stale_peers()
  │   │   ├── snap: cleanup_snap_timeouts()
  │   │   ├── fork: detect_stuck_sync(), detect_stuck_fork(), detect_height_offset()
  │   │   ├── network: retry_idle_sync()
  │   │   └── gate: cleanup_recovery_grace()
  │   └── resolve_shallow_fork() → fork.recommend_action() → rollback / start_fork_sync
  │
  ├── try_produce_block()
  │   ├── gate.update_production_state(context)
  │   └── gate.can_produce(context) → ProductionAuthorization
  │
  └── handle_new_block() / block_applied()
      └── sync_manager.block_applied_with_weight()
          ├── network.note_block_applied()
          ├── fork: reset counters
          ├── gate: track recovery blocks
          └── state transition check (syncing → synchronized)
```

---

## Design Decision Hypotheses

### DDH-1: Field Grouping via Subtract + Sub-structs
**conf(0.85, observed)**

- **Hypothesis**: Grouping 55 fields into 5 typed sub-structs (SyncPipeline, ProductionGate, ForkState, SnapSyncState, NetworkState) plus eliminating ~7 dead fields reduces SyncManager to 13 direct fields while preserving all behavior.
- **Evidence**: Field clusters identified from actual code access patterns. Each cluster's fields are read/written together in the same methods. Cross-cluster access is limited and can be bridged with method parameters (GateContext pattern).
- **Risk**: Methods that currently read `self.consecutive_sync_failures` and `self.fork_mismatch_detected` in the same function will need to reference `self.fork.consecutive_sync_failures` and `self.gate.fork_mismatch_detected`. This is verbose but not complex.
- **Kill condition**: If more than 5 methods need to read fields from 3+ sub-structs in a single function body, the grouping is wrong. Re-evaluate cluster boundaries.

### DDH-2: Fork Recovery Coordination in ForkState
**conf(0.80, observed)**

- **Hypothesis**: Moving fork recovery decision logic from Node's resolve_shallow_fork() into ForkState.recommend_action() eliminates interference between fork recovery systems without merging them.
- **Evidence**: The three systems handle different triggers (orphan blocks, empty headers, stuck sync). They don't need to be merged — they need ONE coordinator that prevents interference. ForkState becomes that coordinator.
- **Risk**: Node still executes rollbacks (needs block_store). The interface between Node and ForkState must be clean: ForkState decides, Node executes.
- **Kill condition**: If recommend_action() needs to know about block_store contents (not just height/gap), the abstraction is leaking.

### DDH-3: Idempotent start_sync() via Guard Clause
**conf(0.80, observed)**

- **Hypothesis**: Adding `if self.state.is_syncing() { return; }` at the top of start_sync() prevents the 43-calls-per-second problem on a 44-node network without any other changes.
- **Evidence**: update_peer() already guards with `matches!(self.state, SyncState::Idle | SyncState::Synchronized)`. The guard clause adds defense-in-depth for other call sites. The destructive reset (clear downloaders, bump epoch) only runs when starting from Idle.
- **Risk**: A legitimate restart (e.g., current sync peer disconnected) might be blocked. Mitigation: remove_peer() already resets state to Idle when the sync peer disconnects, so the next call to start_sync() will proceed.
- **Kill condition**: If there's a code path where start_sync() MUST restart an active sync (not just resume), the guard clause is wrong.

### DDH-4: Production Gate Layer Subtraction
**conf(0.75, observed)**

- **Hypothesis**: Merging Layer 8 + 8.5 and Layer 10 + 10.5, plus deleting Layer 7 dead code, reduces production gate from 13 to ~9 check phases without losing any safety.
- **Evidence**: Layer 8.5 (persistent fork flag) was introduced BECAUSE Layer 8 (consecutive failures count) oscillated. Layer 8.5 subsumes Layer 8 for production gate purposes. Layer 10 and 10.5 are the same check (gossip health) applied to different height relationships — they merge naturally.
- **Risk**: Layer 8 (consecutive_sync_failures) is also used by fork detection logic, not just production gate. It stays as a field on ForkState; it's just removed from can_produce() where it's redundant with fork_mismatch_detected.
- **Kill condition**: If there's a scenario where fork_mismatch_detected is false but consecutive_sync_failures >= 3 should block production, the merge is wrong. Analysis: update_production_state() sets fork_mismatch_detected when disagree > agree. consecutive_sync_failures >= 3 without fork_mismatch_detected means we had 3 empty header responses but all peers agree on our hash. This happens during normal sync (peer has our blocks but doesn't recognize our tip hash due to a reorg). In this case, blocking production is WRONG — we should just resync. So removing the check from can_produce() is correct.

### DDH-5: cleanup() Decomposition into Named Functions
**conf(0.80, observed)**

- **Hypothesis**: Splitting cleanup() into 8-10 named functions called in sequence from a dispatch function preserves all behavior while making each concern independently readable and testable.
- **Evidence**: The 12 responsibilities in cleanup() are independent — they read shared state but don't interact. Each can be a standalone function.
- **Risk**: Order might matter for some cleanup operations. Analysis: the only ordering dependency is "stale peer removal before sync retry" (removing a peer might change should_sync() result). This ordering is preserved by calling functions in the same order as the current code.

---

## Milestones

| ID | Name | Scope (Modules) | Scope (Requirements) | Est. Size | Dependencies |
|----|------|-----------------|---------------------|-----------|-------------|
| M1 | Dead Code Removal + Data Grouping | chain_follower.rs deletion, SyncPipeline, SnapSyncState, NetworkState sub-structs, dead field removal | REQ-SYNC-001, REQ-SYNC-005, REQ-SYNC-006 | M | None |
| M2 | Fork Coordination + Idempotent Sync | ForkState sub-struct, ForkAction enum, start_sync() guard clause, resolve_shallow_fork simplification | REQ-SYNC-002, REQ-SYNC-003, REQ-SYNC-007, REQ-SYNC-013 | L | M1 |
| M3 | Production Gate + Cleanup | ProductionGate sub-struct, layer merges, cleanup() decomposition, GateContext | REQ-SYNC-004, REQ-SYNC-008, REQ-SYNC-012 | M | M1 |

### Migration Path

Each milestone is a series of standalone PRs that preserve behavior:

#### M1: Dead Code Removal + Data Grouping (4 PRs)

**PR 1: Delete chain_follower.rs**
- Delete `crates/network/src/sync/chain_follower.rs` (795 lines)
- Verify: not in mod.rs, not referenced anywhere
- All tests pass

**PR 2: Remove dead fields + merge production_blocked/reason**
- Remove `max_heights_behind`, `max_heights_ahead`, `has_connected_to_peer` (derive from first_peer_status_received)
- Merge `production_blocked: bool` + `production_block_reason: Option<String>` into `production_blocked: Option<String>`
- Update all call sites (grep for each field name)
- All tests pass

**PR 3: Extract SyncPipeline, SnapSyncState, NetworkState sub-structs**
- Create sub-structs in mod.rs (no new files)
- Move fields, update all references from `self.field` to `self.pipeline.field` etc.
- This is a mechanical refactor — use `replace_all` on field names
- All tests pass

**PR 4: Extract SyncConfig thresholds (REQ-SYNC-005)**
- Move magic thresholds from SyncManager::new() defaults into SyncConfig
- Add doc comments with rationale for each threshold
- All tests pass

#### M2: Fork Coordination + Idempotent Sync (3 PRs)

**PR 5: start_sync() guard clause (REQ-SYNC-003)**
- Add `if self.state.is_syncing() { return; }` at top of start_sync()
- All tests pass
- Test: call start_sync() 43 times in a row, verify sync_epoch increments only once

**PR 6: Extract ForkState sub-struct + ForkAction enum**
- Move fork-related fields into ForkState sub-struct
- Add `recommend_action()` method
- Update Node's resolve_shallow_fork() to use recommend_action()
- All tests pass

**PR 7: State transition logging (REQ-SYNC-007)**
- Add `set_state()` method that logs old -> new state transition with trigger source
- Replace all direct `self.state = ...` assignments with `self.set_state(new_state, "trigger")`
- All tests pass

#### M3: Production Gate + Cleanup (3 PRs)

**PR 8: Extract ProductionGate sub-struct**
- Move production gate fields into ProductionGate sub-struct
- Add GateContext parameter to can_produce() and update_production_state()
- All tests pass

**PR 9: Merge redundant production gate layers**
- Remove Layer 8 (consecutive_sync_failures) from can_produce() — covered by fork_mismatch_detected
- Merge Layer 10 + 10.5 into single gossip health check
- Delete Layer 7 dead comments/logging
- All tests pass

**PR 10: Decompose cleanup() (REQ-SYNC-004)**
- Split into named functions: cleanup_request_timeouts(), cleanup_stale_peers(), cleanup_snap_timeouts(), detect_stuck_sync(), cleanup_blacklists(), retry_idle_sync(), detect_stuck_fork(), detect_height_offset(), cleanup_recovery_grace()
- cleanup() becomes a dispatcher calling each in order
- All tests pass

**Total: 10 PRs** (within REQ-SYNC-011 limit of <= 10).

---

## Requirement Traceability

| Requirement ID | Architecture Section | Module(s) | Milestone |
|---------------|---------------------|-----------|-----------|
| REQ-SYNC-001 | DDH-1: Field Grouping | SyncPipeline, ProductionGate, ForkState, SnapSyncState, NetworkState | M1 |
| REQ-SYNC-002 | DDH-2: Fork Recovery Coordination | ForkState, ForkAction enum | M2 |
| REQ-SYNC-003 | DDH-3: Idempotent start_sync() | SyncManager.start_sync() | M2 |
| REQ-SYNC-004 | DDH-5: cleanup() Decomposition | cleanup.rs (same file, named functions) | M3 |
| REQ-SYNC-005 | M1 PR 4: SyncConfig thresholds | SyncConfig | M1 |
| REQ-SYNC-006 | "What Gets DELETED" section | chain_follower.rs | M1 |
| REQ-SYNC-007 | M2 PR 7: State transition logging | SyncManager.set_state() | M2 |
| REQ-SYNC-008 | DDH-4: Production Gate Layer Subtraction | ProductionGate | M3 |
| REQ-SYNC-009 | Out of scope for this redesign (Could priority) | — | — |
| REQ-SYNC-010 | All milestones: "All tests pass" on every PR | All | M1, M2, M3 |
| REQ-SYNC-011 | Milestones section: 10 PRs | All | M1, M2, M3 |
| REQ-SYNC-012 | M2 PR 7 + DDH-2 ForkAction enum | ForkState, SyncManager.set_state() | M2 |
| REQ-SYNC-013 | DDH-2: ForkState.recommend_action() handles gap 51-1000 | ForkState | M2 |

---

*Architecture completed 2026-03-22. Architect: OMEGA workflow RUN_ID=44, scope: syncmanager.*
