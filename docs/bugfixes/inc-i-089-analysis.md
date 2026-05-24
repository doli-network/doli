# INC-I-089 Analysis — Producer Self-Fork on Restart (Gossip vs Scheduler Race)

## 1. Structural Verification

| # | Claim | Verdict | Evidence |
|---|-------|---------|----------|
| 1 | `production_gate.rs` exists at `crates/network/src/sync/manager/production_gate.rs` | **VERIFIED TRUE** | 783 lines implementing the production gate |
| 2 | `[SNAP_SYNC] Production gated: awaiting first canonical gossip block` emits from snap_sync.rs | **DIVERGENT FROM CLAIM** — log emits from `snap_sync.rs:272-275`, NOT from `production_gate.rs`. The gate CHECK is in `production_gate.rs:57-62`, the CLEAR in `production_gate.rs:289-296`, the SET and LOG in `snap_sync.rs:272-275`. |
| 3 | `try_produce_block()` consults gate via `handle_production_authorization()` → `SyncManager::can_produce()` → `ProductionAuthorization` enum | **VERIFIED TRUE** | `production/mod.rs:102` → `gates.rs:24` → `production_gate.rs:30` |
| 4 | Gate constructed in `Node::new()` (`init.rs`) | **VERIFIED TRUE** | `init.rs:691` creates `SyncManager::new()` → `mod.rs:189` sets `recovery_phase: RecoveryPhase::Normal`. Configuration at `init.rs:693-758`. |
| 5 | What clears the gate | **VERIFIED TRUE** | `block_handling.rs:501` after gossip block extends tip + `apply_block()` succeeds; AND 60s timeout in `cleanup.rs:563-569` |

**Critical structural fact**: On normal restart, `recovery_phase = Normal` — NO startup lockout exists today. Only the snap-sync code path sets `AwaitingCanonicalBlock`. The fix is to ALSO set it during normal restart in `init.rs`.

## 2. Architecture Context

**Module Boundaries:**
- `SyncManager` (crates/network/src/sync/manager/) — owns `recovery_phase`, `can_produce()`, all gate state. Shared via `Arc<RwLock<SyncManager>>`.
- `Node` (bins/node/src/node/) — owns production timer, calls `handle_production_authorization()`, owns `first_peer_connected`.
- `production/mod.rs` — `try_produce_block()`, outer chain of guards.
- `production/gates.rs` — `handle_production_authorization()`, delegates to SyncManager.
- `block_handling.rs` — gossip block processing, calls `clear_awaiting_canonical_block()` on success.

**Data Flow:**
```
production_timer tick → try_produce_block()
  → VERSION CHECK → HARDFORK CHECK → SLOT CHECKS
  → handle_production_authorization()
      → sync_manager.write().can_produce(slot)
          → Check explicit block
          → Check syncing/resync
          → Check AwaitingCanonicalBlock ← THE GATE
          → Check bootstrap/min_peers
          → Check gossip watchdog (non-blocking)
          → Return Authorized or Blocked*
  → BEHIND-PEERS CHECK
  → SCHEDULER ELIGIBILITY
  → should_defer_epoch_production() ← INC-I-053 timer (epoch only)
  → BUILD BLOCK → APPLY → BROADCAST
```

**Trust Boundary:** Gate decision lives entirely in `SyncManager::can_produce()`. Node layer is consumer. Unlock signal: `handle_new_block()` → `apply_block()` success → `clear_awaiting_canonical_block()`. This is the gossip-RX → gate-state trust path.

**Architectural Constraints:**
- `RecoveryPhase` is a single enum — only ONE phase active at a time.
- Setting `AwaitingCanonicalBlock` at startup is safe (no other recovery phase expected at construction time).
- `clear_awaiting_canonical_block()` is idempotent.
- 60s cleanup timeout is a safety net for both snap-sync and (after fix) startup.

## 3. Root Cause Confirmation

Forward-reasoning trace from actual code:

1. **Node starts** → `Node::new()` → `SyncManager::new()` → `recovery_phase = Normal` (`mod.rs:189`)
2. **Chain state loaded from disk** → `sm.update_local_tip(height, hash, slot)` at `init.rs:732`
3. **Event loop starts** → `run_event_loop()` → production_timer ticks every 1s
4. **Peer connects** → `network_events.rs:22-23` → `first_peer_connected = Some(Instant::now())`
5. **Status exchange** → peer reports tip height, `SyncManager` updates `best_peer_height()`
6. **Gossip block arrives** at h=22089 → `handle_new_block()` → classified as `ExtendsTip` → `apply_block()` succeeds → local tip = h=22089
7. **INC-I-053 grace**: if elapsed > 15s since `first_peer_connected`, grace expired
8. **Production timer fires** → `try_produce_block()`:
   - `can_produce()` → `recovery_phase = Normal` → NOT blocked
   - Behind-peers check: peer reported h=22089, we're producing h=22090, `network_tip_height = 22089`, `22089 > 22090-1` = false → guard doesn't fire
   - Scheduler: n1 is designated for current slot
   - `should_defer_epoch_production()`: if elapsed > 15s → returns false
   - **RESULT: n1 produces h=22090 on local tip h=22089 — SELF-FORK**
9. **Canonical h=22090 arrives** microseconds later → same parent, different content → `Orphan` → fork begins

**Confirmed**: No semantic "have I seen a peer-produced block since restart?" barrier exists. Only INC-I-053's fixed 15s timer, which is demonstrably insufficient on fast local networks.

## 4. Existing Snap-Sync Gate Map

| Aspect | Detail |
|--------|--------|
| **Entry condition** | `snap_sync.rs:272` — set when snap sync completes and state is applied |
| **Variant** | `RecoveryPhase::AwaitingCanonicalBlock { started: Instant }` |
| **Block condition** | `production_gate.rs:57-62` — returns `BlockedAwaitingCanonicalBlock` |
| **Unlock #1 (semantic)** | `production_gate.rs:289-296` — `clear_awaiting_canonical_block()` sets `Normal` |
| **Unlock call site** | `block_handling.rs:501` — after gossip block extends tip and apply succeeds |
| **Unlock #2 (timeout)** | `cleanup.rs:563-569` — 60s timeout clears the gate |
| **Test coverage** | `tests.rs:1304` timeout fires after 60s; `tests.rs:1334` no premature timeout; `tests.rs:1359` post-snap empty headers |
| **API** | `can_produce(slot) → ProductionAuthorization` enum; `clear_awaiting_canonical_block()` method; `is_awaiting_canonical_block() → bool` |

## 5. Fix Plan Skeleton (Extension Points)

1. **SET**: In `init.rs` after `sm.update_local_tip()` when `state.best_height > 0`:
   ```rust
   sm.recovery_phase = RecoveryPhase::AwaitingCanonicalBlock { started: Instant::now() };
   ```
   Engages gate ONLY when restart has prior chain data (skip fresh genesis).

2. **BLOCK**: Already handled — `production_gate.rs:57-62`.

3. **UNLOCK (gossip)**: Already handled — `block_handling.rs:501`.

4. **UNLOCK (timeout/safety)**: Already handled — `cleanup.rs:563-569` at 60s. Either reuse 60s (SSF) or parameterize per-cause timeout.

5. **NAMED CONSTANT** (required by user constraint): `POST_RESTART_LOCKOUT_SLOTS: u32 = 3` placed next to other production-gate constants.

**Minimal change surface**: ~3-5 LOC in `init.rs` to set the gate + 1 constant definition + (optional) cleanup tuning.

## 6. Requirements

| ID | Requirement | Priority | Acceptance Criteria |
|----|------------|----------|---------------------|
| REQ-INC-089-001 | Production gate blocks production on normal restart until gossip block from peer proves canonical alignment | **Must** | On restart with height > 0, production blocked; cleared by first gossip block that extends local tip; no self-fork during lockout |
| REQ-INC-089-002 | Safety timer unlocks production for single-producer / no-peer scenarios after N slots | **Must** | If no gossip arrives within N*slot_duration, production resumes; N is named constant (default=3, ~30s); single-producer testnet produces after timeout |
| REQ-INC-089-003 | Existing snap-sync gate behavior unchanged (regression-free) | **Must** | All existing snap-sync gate tests pass unchanged; `AwaitingCanonicalBlock` still set after snap sync; 60s timeout still works |
| REQ-INC-089-004 | Fresh genesis start (height=0) not affected by startup lockout | **Must** | New node at height=0 does NOT enter `AwaitingCanonicalBlock`; bootstrap mode guards remain functional |
| REQ-INC-089-005 | Startup lockout logged for observability | **Should** | Log emitted when startup lockout engages; log emitted when lockout clears (gossip or timeout) |
| REQ-INC-089-006 | No protocol/consensus changes | **Must** | No CURRENT_PROTOCOL_VERSION bump, no EPOCH_STATE_FORMAT_VERSION bump, no NetworkParams activation height, no HardForkSchedule entry; rolling-deploy safe |

## 7. Impact Analysis

| File/Module | Impact | Risk |
|-------------|--------|------|
| `bins/node/src/node/init.rs` | Set `AwaitingCanonicalBlock` on restart | Low — additive in existing config block |
| `crates/network/src/sync/manager/production_gate.rs` | Add named constant; no behavior change to existing fn | None |
| `crates/network/src/sync/manager/cleanup.rs` | Optional: adjust timeout from 60s | Low |
| `bins/node/src/node/block_handling.rs` | No change needed (existing clear call) | None |
| `bins/node/src/node/production/mod.rs` | `should_defer_epoch_production()` becomes partially redundant (leave as defense-in-depth) | None |
| `bins/node/tests/inc_i_053_epoch_startup_gate.rs` | Remains valid | None |
| `crates/network/src/sync/manager/tests.rs` | New tests added; existing tests unaffected (use `RecoveryPhase::Normal` directly) | None |

**Regression Risk Areas:**
- `new_for_test()` doesn't set height > 0 → new gate won't engage in tests → safe.
- If any integration test constructs SyncManager with height > 0 AND expects immediate production → would fail. **Must verify via full test suite.**
- Snap-sync tests that set `AwaitingCanonicalBlock` manually are unaffected.

## 8. Specs/Docs Drift

| Source | Drift |
|--------|-------|
| `docs/bugfixes/inc-i-053-epoch-sync-gate.md` | States "15s timer gives time for gossip blocks" — now proven insufficient for single-node rolling-restart. Should note `AwaitingCanonicalBlock` startup lockout supersedes the timer as primary defense. |
| `.claude/skills/node/SKILL.md:303` | Lists "should_defer_epoch_production" as the startup guard. After fix, should also mention `AwaitingCanonicalBlock` startup lockout. |

## Triage Verdict

```
━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.95, code-verified)
Reasoning: All structural claims verified. Root cause confirmed via forward-reasoning against actual code. Fix is a ~3-line gate SET in init.rs reusing an existing mechanism with existing unlock paths and existing tests. No architectural deviation from prompt.
━━━━━━━━━━━━━━━━━━━━━━━━
```

**Milestone breakdown**: Single milestone **M1** = reproduction test + fix combined.
