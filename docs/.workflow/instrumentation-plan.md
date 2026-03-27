# Instrumentation Assessment: INC-I-010 and INC-I-014 Log Coverage

## Detected Framework
- Language: Rust
- Framework: `tracing` crate (`info!`, `warn!`, `error!`, `debug!`, `trace!`)
- Format: Unstructured string interpolation with `[PREFIX]` tags (e.g., `[SYNC_STATE]`, `[SNAP_SYNC]`, `[MEM-CONN]`)
- Existing patterns: Prefixed messages, no structured key-value fields, `Debug` trait for types

## Assessment Methodology

For each code file relevant to the two incidents, I assessed:
1. What IS logged (current coverage)
2. What is NOT logged but would have been needed to diagnose the incident
3. Severity of each gap: CRITICAL / HIGH / MEDIUM

---

## INC-I-010: Fork-stuck node cannot recover

**Root cause chain**: Snap sync restores ProducerSet but block store is empty. `rebuild_scheduled_from_blocks` calls `reset_scheduled_flags()` (all=true) since no blocks exist. Network has different scheduled set from real block history. Scheduler denominator mismatch causes "invalid producer for slot" rejection. Additionally: snap disabled (threshold=u64::MAX), progressive rollback deadlocked, `clear_snap_sync` fired after 1 block instead of waiting for Synchronized.

---

### File: `crates/network/src/sync/manager/mod.rs`

**Current coverage:**
- `set_state()` logs state transitions with old/new label and trigger (debug level)
- Invalid state transitions logged with warn
- `update_local_tip()` logs synchronization at info level
- Resync completion logged with height and grace period

**GAPS:**

| # | What is missing | Severity | Why needed for INC-I-010 |
|---|----------------|----------|--------------------------|
| 1 | `set_state()` does not log `local_height`, `local_hash`, `recovery_phase`, or `consecutive_empty_headers` alongside the transition | CRITICAL | During INC-I-010, the state machine cycled through transitions but the logs never showed WHY -- no context about what drove each transition. A log showing `Idle -> Syncing:Headers (trigger: add_peer) local_h=240 recovery=Normal empty_headers=0` would have immediately revealed the cycle. |
| 2 | `best_peer_for_recovery()` does not log which peer was selected, what the min_acceptable threshold was, or whether it fell back | HIGH | INC-I-010 involved sync downloading from stuck peers. Without logging the selection logic, the investigation had to guess which peers were being selected and why. |
| 3 | No log when `is_valid_transition()` returns true but the transition is a no-op (same state) | MEDIUM | Would help distinguish "transition fired but was already in that state" from "transition fired and state changed". |

---

### File: `crates/network/src/sync/manager/snap_sync.rs`

**Current coverage:**
- Quorum voting: stale vote rejection, quorum reached, snapshot received
- Download errors: peer failover, alternate peer retry, fallback
- Root mismatch between quorum and response noted

**GAPS:**

| # | What is missing | Severity | Why needed for INC-I-010 |
|---|----------------|----------|--------------------------|
| 4 | `take_snap_snapshot()` does not log the ProducerSet state being installed -- specifically how many producers are active and what their scheduled flags are | CRITICAL | This is THE root cause of INC-I-010. The snap sync installed a ProducerSet where all scheduled=true, but the network had a different scheduled set. A log like `[SNAP_SYNC] Installed ProducerSet: N active producers, M scheduled, scheduled_count_at_height(H)=K` would have immediately identified the divergence. |
| 5 | `take_snap_snapshot()` sets `recovery_phase = AwaitingCanonicalBlock` but does not log whether it was previously `ResyncInProgress` or something else | HIGH | The transition from ResyncInProgress to AwaitingCanonicalBlock via complete_resync() is a critical lifecycle event. Logging the before/after recovery_phase at this point would have shown the deadlock path. |
| 6 | No log when snap sync is SKIPPED because `snap.threshold == u64::MAX` | HIGH | INC-I-010 had snap disabled. The node silently bypassed snap sync with no trace in logs, making it invisible that the operator's `--no-snap-sync` flag was preventing recovery. |

---

### File: `crates/network/src/sync/manager/cleanup.rs`

**Current coverage:**
- Extensive: stale requests, stale peers, snap timeouts, stall detection, stuck sync, body stall retries, blacklist management, height offset detection, idle-behind-retries, post-recovery grace timeout, AwaitingCanonicalBlock timeout

**GAPS:**

| # | What is missing | Severity | Why needed for INC-I-010 |
|---|----------------|----------|--------------------------|
| 7 | The cleanup log at the top does not include `recovery_phase`, `consecutive_empty_headers`, `consecutive_apply_failures`, `fork.stuck_fork_signal`, or `snap.threshold` | HIGH | These are the exact variables that determine whether the node recovers or gets stuck. The existing log shows state/peers/pending_requests which is necessary but not sufficient. |
| 8 | No log when `should_sync()` returns false -- the node sits in Idle but nothing explains WHY it is not syncing | HIGH | INC-I-010 likely had periods where the node was Idle and not syncing. Without logging the `should_sync()` decision (what height comparison failed, what peer state prevented it), the investigation had to infer from indirect evidence. |

---

### File: `crates/network/src/sync/manager/production_gate.rs`

**Current coverage:**
- `can_produce()` logs slot, local_h, peers, state on every call
- `can_produce()` logs AUTHORIZED on success
- Each blocker returns a typed enum but only the start/end states are logged
- `start_resync()`, `complete_resync()`, `block_production()`, `unblock_production()` all logged
- `signal_stuck_fork()` logs when ignored due to non-Normal recovery phase

**GAPS:**

| # | What is missing | Severity | Why needed for INC-I-010 |
|---|----------------|----------|--------------------------|
| 9 | `can_produce()` does not log the SPECIFIC blocker reason when production is denied -- the return value (e.g., `BlockedSyncing`, `BlockedAwaitingCanonicalBlock`) is not logged | CRITICAL | INC-I-010 involved production being blocked but the operator could not tell WHY from the logs. The enum variant was returned to the caller but never logged in the sync manager. The caller (Node) may or may not have logged it. |
| 10 | `needs_genesis_resync()` returns false silently when `snap.threshold == u64::MAX` AND `fork.needs_genesis_resync` is false | HIGH | The `--no-snap-sync` deadlock happened because this function silently returned false. The warn log only fires when `fork.needs_genesis_resync` is true AND threshold is u64::MAX. When neither condition triggers, there is zero trace. |
| 11 | `is_deep_fork_detected()` has no logging at all -- it is a pure boolean check | HIGH | This function has 5 conditions that must all be true. When it returns false, there is no way to know WHICH condition failed. A debug-level log showing each condition's value would have identified why deep fork detection never triggered in INC-I-010. |

---

### File: `crates/network/src/sync/manager/block_lifecycle.rs`

**Current coverage:**
- `block_applied_with_weight()` logs sync progress every 5s, processing complete transitions, sync completion
- `block_apply_failed()` logs failure count, apply failure escalation (stuck fork or genesis resync)
- `reset_local_state()` logs refusal (height floor), genesis reset, and sync trigger

**GAPS:**

| # | What is missing | Severity | Why needed for INC-I-010 |
|---|----------------|----------|--------------------------|
| 12 | `block_apply_failed()` does not log the REASON the block failed -- only the failure count | CRITICAL | INC-I-010 had blocks failing with "invalid producer for slot" but this function only logged "Block apply failure #N". The actual error message was logged elsewhere (in the caller) but not correlated with the sync manager's reaction. |
| 13 | `reset_sync_for_rollback()` does not log the consecutive_empty_headers value or what state it is resetting from | HIGH | INC-I-010 had the counter being reset, preventing escalation. A log showing the counter value before and after reset would have been the smoking gun. |

---

### File: `crates/network/src/sync/fork_recovery.rs`

**Current coverage:**
- Start: logs orphan block hash and parent
- Failover: logs per-request timeout and peer switch
- Cancel: logs reason
- Connection: logs connection point and block count
- Max depth: logs escalation warning

**GAPS:**

| # | What is missing | Severity | Why needed for INC-I-010 |
|---|----------------|----------|--------------------------|
| 14 | `handle_block()` does not log when a block IS received -- only when it is unexpected or missing | MEDIUM | During investigation, knowing the chain walk progress (block N received, next parent = X) would speed up tracing. |
| 15 | `cancel()` does not log the current depth, peer, or how long the recovery session lasted | MEDIUM | Context-free cancellation messages ("Fork recovery cancelled: session timeout") are hard to correlate without depth/peer/duration. |

---

### File: `crates/network/src/sync/reorg/mod.rs`

**Current coverage:**
- Reorg decision: logs weight comparison (lighter ignored), common ancestor not found, finality rejection, switching to heavier chain
- Tie-break: logs equal-weight decisions

**GAPS:**

| # | What is missing | Severity | Why needed for INC-I-010 |
|---|----------------|----------|--------------------------|
| 16 | `plan_reorg()` does not log the rollback depth, new chain length, or weight delta when it returns `Some(result)` | MEDIUM | The function returns the result silently. The caller (Node) logs it, but having it at the source (reorg handler) would provide redundant confirmation. |
| 17 | No log when `check_reorg_weighted()` finds unknown parent and returns None | MEDIUM | The `debug!("Unknown parent...")` fires but at debug level, likely filtered in production. For fork recovery, this is a decision point worth info-level logging. |

---

### File: `bins/node/src/node/apply_block/mod.rs`

**Current coverage:**
- Duplicate block detection (already applied, poisoned block)
- "Applying block X at height Y" at info level
- Validation errors propagated via `?` (logged by caller)

**GAPS:**

| # | What is missing | Severity | Why needed for INC-I-010 |
|---|----------------|----------|--------------------------|
| 18 | `validate_block_for_apply()` error is returned but NOT logged before propagation -- the `?` operator exits without context | CRITICAL | When "invalid producer for slot" occurs, the error propagates up through `?` operators. The apply_block function logs "Applying block X at height Y" BEFORE validation, so the log shows the attempt but not the failure. The caller gets the error but may not log it with the same context (height, hash, slot, producer). |
| 19 | No log after reactive scheduling mutations showing the resulting scheduled set summary | HIGH | INC-I-010 root cause was divergent scheduled flags. A log like `"Reactive scheduling: N now scheduled out of M active"` after the scheduling block would have shown the divergence immediately after snap sync. |

---

### File: `bins/node/src/node/apply_block/state_update.rs`

**Current coverage:**
- `clear_snap_sync()` is called but NOT logged
- Protocol activation logged
- Genesis timestamp logged

**GAPS:**

| # | What is missing | Severity | Why needed for INC-I-010 |
|---|----------------|----------|--------------------------|
| 20 | `clear_snap_sync()` has no log | CRITICAL | INC-I-010 had `clear_snap_sync` firing after 1 block instead of waiting for Synchronized. This silent state mutation was invisible in logs. A log like `"Clearing snap sync marker at height H (sync_state=X)"` would have immediately shown it fired too early. |

---

### File: `bins/node/src/node/rollback.rs`

**Current coverage:**
- Rolling back from height X to Y logged
- Undo-based rollback logged
- No undo data fallback logged
- Block 1 missing edge case logged
- Completion logged

**GAPS:**

| # | What is missing | Severity | Why needed for INC-I-010 |
|---|----------------|----------|--------------------------|
| 21 | `resolve_shallow_fork()` does not log the full decision context when `has_fork_evidence` is false | MEDIUM | When the function exits early (no evidence), there is no log. During INC-I-010, this would have shown that the counter never reached threshold because it was being reset. |
| 22 | `rebuild_scheduled_from_blocks()` is called during rollback but its effect (how many producers changed scheduling state) is not logged | HIGH | This is part of the root cause chain -- after rollback, the scheduled flags are rebuilt from blocks. Logging the before/after scheduled count would show whether the rebuild converged with the network. |

---

### File: `crates/storage/src/producer/set_core.rs`

**Current coverage:**
- `apply_pending_updates()` has warn-level logs for failed deferred operations
- NO other scheduled flag operations are logged

**GAPS:**

| # | What is missing | Severity | Why needed for INC-I-010 |
|---|----------------|----------|--------------------------|
| 23 | `reset_scheduled_flags()` has no logging at all | CRITICAL | This is the function that sets ALL producers to scheduled=true -- the direct cause of the scheduler denominator mismatch in INC-I-010. It is called from 4 places. A single info-level log showing how many producers were reset would have been the clearest diagnostic signal. |
| 24 | `unschedule_producer()` and `schedule_producer()` have no logging | HIGH | These are the functions that change individual scheduled flags. They are called frequently from apply_block. At debug level, logging each change would create a complete audit trail of scheduling decisions. |
| 25 | `scheduled_producers_at_height()` does not log its result count | MEDIUM | This function determines the scheduler denominator. Logging the count at debug level when it differs from active producer count would flag scheduling divergence. |

---

## INC-I-014: RAM explosion at 103+ nodes

**Root cause chain**: (1) idle_timeout=86400s causing zombie connections. (2) Connection limit bypass via pending connections. (3) Eviction churn loop -- nodes exceed max_peers, constant connect/evict cycles allocate transient yamux buffers never reclaimed.

---

### File: `crates/network/src/service/mod.rs`

**Current coverage:**
- `[MEM-BOOT]` log at startup: max_peers, conn_limit, pending_limit, yamux_window, idle_timeout, event_buf, cmd_buf

**GAPS:**

| # | What is missing | Severity | Why needed for INC-I-014 |
|---|----------------|----------|--------------------------|
| 26 | No periodic log of RSS/heap memory usage | HIGH | INC-I-014 was a RAM explosion. The connection budget log (`[MEM-CONN-BUDGET]`) shows connection counts but not actual memory. Without memory metrics, the 86GB explosion was detected only when the OS killed the process. Even a simple RSS check every 60s would have shown the growth curve. |

---

### File: `crates/network/src/service/swarm_events.rs`

**Current coverage:**
- ConnectionEstablished: debug-level log with peer, num_established, total connections (in/out)
- Milestone log every 10 connections at info level
- Eviction cooldown check (silent disconnect on cooldown)
- Eviction: info-level log with peer table size, evicted peer score, new peer, cooldown count, bootstrap count
- ConnectionClosed: debug-level log with peer, cause, remaining connections
- Bootstrap peer disconnect logged

**GAPS:**

| # | What is missing | Severity | Why needed for INC-I-014 |
|---|----------------|----------|--------------------------|
| 27 | `ConnectionEstablished` with `num_established > 1` (duplicate connection) is logged at debug only -- this is exactly the condition that caused per-peer connection doubling | HIGH | INC-I-014 had max_established_per_peer=2, meaning each peer consumed 2 connections. The debug log fires but is filtered in production. An info-level log for duplicate connections would have quantified the problem. |
| 28 | No log when a connection is REJECTED by `connection_limits` behaviour -- the rejection happens inside libp2p and is not surfaced | CRITICAL | The core of INC-I-014 was that pending connections bypassed `with_max_established()`. The connection_limits behaviour rejects connections, but this rejection is not logged by the application. A `SwarmEvent::IncomingConnectionError` handler with the limit reason would have shown the bypass. |
| 29 | Eviction cooldown silent disconnect (line 84) has no log at all -- by design to avoid flooding | HIGH | While the no-log-per-event design is correct for steady state, the AGGREGATE count of cooldown rejections per cleanup cycle is only logged during rate_limit_cleanup (every 5 min). During INC-I-014's rapid churn, 5 minutes was too infrequent. A counter that logs a summary every 10s during high churn would have shown the eviction storm. |
| 30 | No log when a new peer is rejected because peers.len() >= max_peers AND no evictable peer exists (all producers) | MEDIUM | When all peers are producers, eviction cannot free a slot. The new peer gets silently dropped. This edge case would be valuable for diagnosing producer-mesh saturation. |

---

### File: `crates/network/src/service/swarm_loop.rs`

**Current coverage:**
- `[MEM-CONN-BUDGET]` every 60s: peers, established (in/out), pending (in/out), bootstrap, eviction_cooldown
- `[EVICTION]` cooldown purge count logged at 5min intervals
- `[DHT]` bootstrap logged
- `[BOOTSTRAP]` expired peer disconnect logged

**GAPS:**

| # | What is missing | Severity | Why needed for INC-I-014 |
|---|----------------|----------|--------------------------|
| 31 | The `[MEM-CONN-BUDGET]` 60s interval was too coarse for a 3-minute RAM explosion. The pending counts ARE included but by the time 2 budget logs fired, the process was already at 86GB. | HIGH | An adaptive interval (log at 10s if pending > N or established > conn_limit*0.8) or a warn-level alert when pending connections exceed a threshold would have caught the ramp in time to react. |
| 32 | No log showing eviction churn RATE -- how many evictions per minute | HIGH | INC-I-014 had constant evict/reconnect cycles. The individual eviction logs exist but without a rate metric (evictions/min), the pattern was only visible by grepping and counting log lines. |
| 33 | No log of Yamux buffer memory or connection-level resource usage | HIGH | The 1MB per Yamux connection was the memory multiplier. Without a periodic log of estimated Yamux memory (connections * window_size), the RAM growth was invisible until OOM. |

---

### File: `crates/network/src/rate_limit.rs`

**Current coverage:**
- Rate limit exceeded: debug-level per-peer and global logs
- No logging for cleanup, LRU eviction, or capacity management

**GAPS:**

| # | What is missing | Severity | Why needed for INC-I-014 |
|---|----------------|----------|--------------------------|
| 34 | Rate limit rejections are at debug level only | MEDIUM | During INC-I-014, rate limiting may have been triggered by the connection churn but was invisible in production logs. Info-level logging for the first rejection per peer per minute would provide signal. |
| 35 | `cleanup()` does not log how many stale peers were removed or the current tracked_peers count | MEDIUM | Memory pressure from rate limiter state (HashMap with per-peer buckets) is invisible. A cleanup summary log would show the growth. |

---

## Summary Table

| Area | File | Log Count | Coverage | Critical Gaps | High Gaps | Medium Gaps | Verdict |
|------|------|-----------|----------|---------------|-----------|-------------|---------|
| Sync state machine | `sync/manager/mod.rs` | 5 | PARTIAL | 0 | 1 | 2 | Missing context on transitions |
| Snap sync | `sync/manager/snap_sync.rs` | 11 | GOOD | 1 | 2 | 0 | Critical blind spot on installed state |
| Cleanup/recovery | `sync/manager/cleanup.rs` | 23 | GOOD | 0 | 2 | 0 | Good volume, missing decision context |
| Production gate | `sync/manager/production_gate.rs` | 27 | GOOD | 1 | 2 | 0 | Volume good, blocker reason critical |
| Block lifecycle | `sync/manager/block_lifecycle.rs` | 13 | PARTIAL | 1 | 1 | 0 | Key escalation paths under-logged |
| Fork recovery | `sync/fork_recovery.rs` | 5 | MINIMAL | 0 | 0 | 2 | Adequate for its scope |
| Reorg handler | `sync/reorg/mod.rs` | 8 | PARTIAL | 0 | 0 | 2 | Fork choice decisions logged, planning not |
| apply_block | `node/apply_block/mod.rs` | 7 | PARTIAL | 1 | 1 | 0 | Happy path logged, error path silent |
| State update | `node/apply_block/state_update.rs` | 8 | GOOD | 1 | 0 | 0 | One critical silent mutation |
| Rollback | `node/rollback.rs` | 11 | GOOD | 0 | 1 | 1 | Good for happy path |
| ProducerSet | `storage/producer/set_core.rs` | 2 | MINIMAL | 1 | 1 | 1 | Almost no observability |
| Connection mgmt | `service/swarm_events.rs` | 12 | GOOD | 1 | 2 | 1 | Good for steady state, blind to churn |
| Swarm loop | `service/swarm_loop.rs` | ~9 | GOOD | 0 | 2 | 0 | Budget log exists, frequency too low |
| Rate limiter | `rate_limit.rs` | 7 | MINIMAL | 0 | 0 | 2 | Debug-only, invisible in production |
| **TOTALS** | | **~148** | | **7** | **15** | **11** | |

---

## Final Verdict

### INC-I-010: Can we diagnose from current logs? **NO**

**Missing pieces that would have been needed:**

1. **CRITICAL**: `reset_scheduled_flags()` is completely silent. The root cause (all producers set to scheduled=true after snap sync) left zero trace in logs. Without this, investigators had to reason backwards from "invalid producer for slot" errors to hypothesize the scheduling divergence.

2. **CRITICAL**: `clear_snap_sync()` fires silently. The premature clearing (after 1 block instead of waiting for Synchronized) was invisible. Investigators had to read code to discover this bug.

3. **CRITICAL**: `validate_block_for_apply()` failures exit via `?` without logging. The "invalid producer for slot" error propagated upward but apply_block never logged what failed or why.

4. **CRITICAL**: `can_produce()` denial reason not logged. When production was blocked, operators saw the `[CAN_PRODUCE]` entry but not WHY it was blocked. The blocker enum variant was returned but not printed.

5. **CRITICAL**: Snap sync installed ProducerSet content never summarized. The node installed a divergent scheduled set silently.

6. **HIGH**: `is_deep_fork_detected()` has zero logging. The 5 conditions that must all be true for deep fork escalation were completely opaque. When deep fork detection failed to trigger, there was no way to know which condition was the gate.

**Estimated investigation time saved with proper logging**: 4-6 hours. The actual investigation required reading code and reasoning about state machine interactions. With the CRITICAL logs above, the root cause chain would have been visible in a single log trace.

### INC-I-014: Can we diagnose from current logs? **PARTIAL**

**What IS diagnosable:**
- Connection counts are logged every 60s via `[MEM-CONN-BUDGET]`
- Eviction events are logged per-occurrence
- Boot configuration (max_peers, conn_limit, pending_limit, idle_timeout) is logged at startup

**Missing pieces:**

1. **CRITICAL**: Connection limit rejections from libp2p's `connection_limits` behaviour are not surfaced. The pending connection bypass was invisible because rejected connections generate a SwarmEvent that the handler ignores (falls through to `_ => {}`).

2. **HIGH**: No eviction churn rate metric. Individual evictions are logged, but detecting the churn LOOP required manual log analysis.

3. **HIGH**: No Yamux memory estimation. The 1MB/connection multiplier was invisible without external monitoring.

4. **HIGH**: 60s logging interval was too coarse for a 3-minute RAM explosion.

**Estimated investigation time saved with proper logging**: 2-3 hours.

---

## Recommendations for Instrumentation (not implemented -- assessment only)

### Highest-impact additions (would cover both incidents):

1. **ProducerSet scheduled mutations**: Log `reset_scheduled_flags()`, `schedule_producer()`, `unschedule_producer()` with counts
2. **apply_block validation failure**: Log the specific error before `?` propagation with height/hash/slot/producer context
3. **can_produce() blocker**: Log the denial reason (enum variant) at warn level
4. **clear_snap_sync**: Log with current sync state and height
5. **Connection limit rejections**: Surface libp2p connection_limits rejections as warn-level events
6. **Churn rate tracking**: Periodic summary of evictions/connections per interval
7. **is_deep_fork_detected() decision trace**: Debug-level log showing each condition's value

These 7 additions would have made both INC-I-010 and INC-I-014 diagnosable from logs alone, without code reading.
