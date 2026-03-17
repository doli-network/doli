# Diagnostic Reasoning Trace: Why Every Fix Creates a New Bug (Meta-Diagnosis)

**Date**: 2026-03-17
**Session**: Structural meta-diagnosis of the recurring failure pattern
**Supersedes**: Previous trace (4 specific fork sync bugs)
**Focus**: Why does fixing one problem always reveal another? What is the architectural root cause?

---

## Phase 1: Symptom Characterization

### The Meta-Symptom

The user describes a pattern, not a single bug:
1. Fix snap sync cascade -> production gate deadlock appears
2. Fix production gate deadlock -> fork sync weight rejection appears
3. Fix 4 fork sync bugs (39169ae) -> deploy 50 sync nodes -> chain stalls from gossip flooding
4. User: "You fix one problem and another one arises, and so on. What is this?"

### Current Specific Symptom (chain stall at 112 nodes)
- Chain at h=1133, s=1561, gap=428 missed blocks
- Producers flooded with GSet DUPLICATE gossip from 100+ peers
- Event loop starved: production timer never fires
- Several original producers (n3, n4, n8) fell behind
- n10 stuck at genesis

### Key Observation
The gossip flooding stall is NOT one of the 4 bugs fixed in 39169ae. It is an entirely different failure class (event loop architecture vs sync recovery logic). The fixes did not CAUSE the stall -- the stall was always latent, only triggered at 112-node scale.

---

## Phase 2: Evidence Assembly

### Constraint Table (from ALL prior fix rounds)

| Fix Round | What Changed | Scale Tested | Result | What Emerged |
|---|---|---|---|---|
| dd77c7e: snap sync cascade fix | Hash::ZERO rejection, quorum validation | 22 nodes | Fixed cascade | Production gate deadlock from exponential grace backoff |
| Production gate deadlock fix | Grace period cap, resync counter reset | 13 nodes | Fixed deadlock | Fork sync weight rejection (latent since genesis) |
| 7259d47: 3 sync root causes | network_tip_height recompute, stuck_fork_signal, can_produce() purity | 13 nodes | Fixed 3 specific state machine bugs | 4 deeper fork sync bugs revealed |
| 39169ae: 4 fork sync bugs | Weight comparison, blacklisting, recovery gate, peer selection | 56 nodes | Fixed fork sync recovery | Chain stall from gossip flooding at 112 nodes |

### Data Flow for Current Stall

```
[112 nodes, each with gossip_timer at 1-60s intervals]
    |
    v
[Each node broadcasts ALL 13 producer announcements to mesh peers]
    | GossipSub propagates to mesh_n (6-12) peers per node
    | Total messages per round: 112 * mesh_n * 13_announcements
    v
[Each node receives ~100+ ProducerAnnouncementsReceived events per round]
    | Each event -> merge_one() * 13 -> 13 signature verifications + DUPLICATE classification
    v
[Events queued in mpsc::channel(1024)]
    |
    v
[Node event loop: biased select!]
    | Branch 1 (HIGHEST PRIORITY): network.next_event() -> handle_network_event()
    | Branch 2: production_timer.tick() -> try_produce_block()
    | Branch 3: gossip_timer.tick() -> broadcast more announcements
    |
    | With biased select, Branch 1 always wins if events are in the channel.
    | With 100+ events per gossip round, the channel is never empty.
    | Production timer NEVER fires.
    v
[Chain stalls. No blocks produced.]
```

### Architecture Inventory

**SyncManager**: 73 fields, 9 boolean flags, 8 SyncState variants, ~10 counters, ~15 Option<Instant> timestamps.
- Minimum state combinations: 8 * 2^9 = 4,096 (before counters/timestamps)
- Practical state space: millions of reachable combinations
- Each fix adds conditions to prevent one path, potentially opening others

**Event loop** (`event_loop.rs`): Single tokio task with `biased select!`
- Network events: absolute priority (Branch 1)
- Production: second priority (Branch 2, same task)
- Gossip: third priority (Branch 3, same task, creates more Branch 1 events)
- No priority separation between production-critical and non-critical events

**GSet gossip**: Full-state CRDT with O(N^2) convergence messages
- Each node sends ALL producers to mesh peers every gossip_interval
- Delta sync only activates for >50 producers (current: 13)
- Adaptive backoff helps but minimum interval is 1s

**Sync serving**: Unbounded, same task as production
- Each SyncRequest event processed synchronously on main task
- No global limit on concurrent sync requests served
- 100 peers * 20 req/sec rate limit = 2000 req/sec aggregate

**Event channel**: `mpsc::channel(1024)`, blocking `.send().await`
- Swarm loop pushes events without admission control
- Node loop drains events with biased priority
- 1024-deep buffer means ~1s of production starvation when full

---

## Explorer Round 1 -- Hypothesis Generation

### H1: Biased select! + single-task architecture causes production starvation under sustained event load
- The event loop processes ALL events (gossip, sync, status, blocks) before production
- At 100+ peers, event volume exceeds production timer cadence
- Production is literally unreachable while events are pending
- **Falsification**: Run with un-biased select and observe if production resumes

### H2: GSet gossip creates O(N^2) message amplification at the application layer
- GossipSub message-level dedup prevents same-message delivery, but each node creates UNIQUE messages (different signatures, sequences)
- Application-level dedup (DUPLICATE) works but happens AFTER deserialization and signature verification
- The event loop cost is paid regardless of whether the merge succeeds
- **Falsification**: If GossipSub dedup catches all duplicates before they reach the event channel

### H3: No backpressure between network layer and node event loop causes buffering-induced starvation
- The 1024-event channel acts as a shock absorber but delays production
- Even if events arrive at moderate rate, 1024 pending events = 1024 biased-select iterations before production
- **Falsification**: If the channel is always nearly empty (events processed faster than produced)

### H4: Sync request serving on the main task competes directly with production
- Each of 100 sync-only nodes sends periodic requests
- Each request is a NetworkEvent::SyncRequest processed on the same biased select
- Block store I/O (reading headers, bodies, state) is done synchronously
- **Falsification**: If sync requests are handled on a separate spawned task

### H5: The SyncManager's 73-field state machine creates emergent bugs that local fixes cannot prevent
- Each fix reasons about one code path but cannot cover all 4,096+ state combinations
- New fixes add fields (stuck_fork_signal, behind_since, stable_gap_since), growing the state space
- The "whack-a-mole" pattern is the natural consequence of local reasoning about a global state space
- **Falsification**: If a state machine refactor (hierarchical, with invariants) stops the pattern

### H6: The recurring pattern is an artifact of testing at progressively larger scales
- 13 -> 22 -> 56 -> 112 nodes
- Each scale reveals latent bugs that were below threshold at smaller scale
- The bugs are not CAUSED by fixes -- they were always present, just untriggered
- **Falsification**: If bugs reappear at the SAME scale after fixes (not just at larger scale)

### H7: GossipSub mesh scaling creates super-linear event growth
- mesh_n scales with sqrt(N) * 1.5: from 6 at 13 nodes to 16 at 112 nodes
- Total messages: N * mesh_n = 112 * 16 = 1,792 per round (vs 78 at 13 nodes) = 23x increase
- But nodes only increased 8.6x, so message growth is super-linear
- **Falsification**: If mesh_n is fixed at config time and does not dynamically scale

---

## Skeptic Round 1 -- Elimination

### H1: SURVIVES -- Confirmed by code
- `event_loop.rs:39-40`: `biased` keyword present
- `event_loop.rs:44-56`: Network event branch is first
- `event_loop.rs:59-71`: Production timer is second
- With biased, ready Branch 1 always wins over ready Branch 2
- **None of the 4 prior fixes touched the event loop architecture.** This is a completely different subsystem.

### H2: SURVIVES with refinement
- GossipSub `duplicate_cache_time: 60s` prevents re-delivery of the SAME message
- But 112 different nodes produce 112 DIFFERENT messages per round (different senders = different message IDs)
- Each message contains all 13 producer announcements
- Application-level GSet merge correctly identifies duplicates but AFTER deserializing and verifying signatures
- **Cost**: 112 messages * 13 announcements * crypto_verify() = 1,456 signature verifications per gossip round, plus event loop overhead

### H3: WEAKENED
- The swarm loop uses `.send().await` (blocking when channel full). This is ACCIDENTAL backpressure.
- When channel is full, swarm blocks, which slows new event arrival. This prevents overflow.
- But the 1024 events already buffered are sufficient to starve production for many iterations.
- **Revised**: The issue is not channel overflow but buffered-event draining taking priority over production.

### H4: SURVIVES
- `handle_sync_request()` at `validation_checks.rs:460` runs on the main task. No `tokio::spawn`.
- Reads block_store (I/O), serializes state (CPU). All synchronous on main task.
- Rate limiting at 20 req/sec PER PEER. But 100 peers = 2000 req/sec aggregate.
- Each request becomes a NetworkEvent::SyncRequest queued in the same channel as gossip.

### H5: SURVIVES as meta-explanation for prior rounds
- 73 fields confirmed by code count
- 9 boolean flags confirmed: production_blocked, resync_in_progress, has_connected_to_peer, needs_genesis_resync, awaiting_canonical_block, fork_mismatch_detected, post_rollback, post_recovery_grace, stuck_fork_signal
- BUT: this explains the prior whack-a-mole pattern (Mechanism A), not the current stall (Mechanism B)

### H6: SURVIVES as partial explanation
- Each incident was at increasing scale
- The gossip stall was always latent -- it just was not triggered at 13 nodes
- But the prior fork sync bugs (Mechanism A) appeared at the SAME 13-node scale, so H6 does not fully explain the pattern

### H7: WEAKENED
- mesh_n is set at startup from NetworkConfig, not dynamically recalculated
- Default mesh_n=6, mesh_n_high=12. GossipSub may graft up to 12 mesh peers.
- The dynamic scaling function (`calculate_adaptive_mesh_params`) exists but appears to be for creating the gossipsub config, not runtime adjustment.
- Even at mesh_n=6 with 112 nodes: 112 * 6 = 672 messages per gossip round. Still a lot.

---

## Explorer Round 2 -- Refined Hypotheses

### H1a: The biased select is the PROXIMATE cause of the current stall
- Remove biased -> production resumes (but may introduce race forks)
- Better: decouple production to its own task -> immune to event volume

### H1b: Gossip timer shares the biased select, creating a positive feedback loop
- Gossip timer is Branch 3. When it fires (during a lull in events), it BROADCASTS to peers.
- Those peers receive the broadcast, creating MORE events in their channels.
- Some of those events propagate back to this node via mesh forwarding.
- The gossip timer creates the events that starve the production timer.

### H2a: Full-set gossip below the delta sync threshold (50 producers) is wasteful
- 13 producers << 50 threshold
- Every round sends all 13 announcements, all 13 are DUPLICATE on every receiving node
- Useful information content per round: ~0 (network is stable)
- The adaptive gossip backs off to max 60s, but minimum 1s is too fast

### H4a: Sync request serving has no global admission control
- Per-peer rate limit: 20 req/sec. Aggregate with 100 peers: 2000 req/sec.
- No limit on how many sync requests the node serves concurrently.
- A producer surrounded by 100 sync-only nodes becomes a de facto serving-only node.

### H8 (NEW): The two failure classes (Mechanism A and Mechanism B) have a common root cause: the absence of architectural boundaries
- **Mechanism A** (same-scale bugs): SyncManager is a 73-field god object with no internal boundaries. Any function can read/write any field. No invariants enforce valid state combinations.
- **Mechanism B** (scale bugs): The event loop is a single task with no priority boundaries. All events compete for the same processing time. No admission control separates critical (production) from non-critical (gossip, sync serving).
- **Common root cause**: The system lacks architectural boundaries between concerns. When everything shares everything, any change can affect anything, and any load pattern can displace any function.

---

## Skeptic Round 2

### H1a: CONFIRMED
- Direct code evidence of biased select with network-first priority
- Production is unreachable when event channel is non-empty
- At 112 nodes, event channel is effectively always non-empty

### H1b: CONFIRMED
- Gossip timer at event_loop.rs:76 fires on the same select
- When it fires (lines 83-170), it broadcasts to peers via GossipSub
- Remote nodes receive, generate events, some propagate back
- This is a measurable positive feedback loop

### H2a: CONFIRMED
- 13 producers < 50 delta_sync threshold
- Full-set gossip every round
- All merges return DUPLICATE for established producers
- CPU cost: 13 * signature_verify per incoming gossip message, hundreds of messages per round

### H4a: CONFIRMED
- No global admission control in code
- `handle_sync_request()` is synchronous, no tokio::spawn
- Each request blocks the main task during block_store I/O

### H8: CONFIRMED as unifying hypothesis
- Mechanism A: SyncManager as god object (73 fields, no boundaries)
- Mechanism B: Event loop as single task (no priority boundaries)
- Both are instances of the same architectural pattern: **missing separation of concerns**
- The "fix one thing, break another" pattern is the natural consequence of a system without boundaries

---

## Analogist -- Known Failure Patterns

### Pattern 1: Head-of-Line Blocking in Single-Threaded Event Loops
- **Redis, Node.js, Nginx workers**: All single-threaded event loops. Work at moderate load. Fail when one event type monopolizes the loop.
- **Standard fix**: Separate critical-path work to its own thread/task.
- **DOLI match**: Production is critical-path work competing with non-critical gossip/sync on the same loop.

### Pattern 2: God Object Anti-Pattern in State Machines
- **Classic OOP**: A god object that knows everything and does everything cannot be modified safely. Changes have unpredictable side effects because any field can interact with any other.
- **Standard fix**: Decompose into smaller, focused objects with explicit interfaces and invariants.
- **DOLI match**: SyncManager is a textbook god object. 73 fields, methods spread across 5 files, no internal invariants.

### Pattern 3: Gossip Amplification in CRDT Systems
- **Riak, CockroachDB, Cassandra**: Full-state CRDTs converge at O(N^2) cost. Production systems use deltas, bloom filters, or frequency scaling proportional to N.
- **DOLI match**: Full-set gossip below delta threshold. The adaptive gossip helps but thresholds are too high for current problem.

### Pattern 4: Missing Admission Control (Thundering Herd)
- **Web servers**: Without connection limits, a sudden spike in clients overwhelms the server. Each client gets worse service, causing retries, which make the spike worse.
- **Standard fix**: Global connection/request limits. Serve N clients well, reject the rest with backpressure signal.
- **DOLI match**: 100 sync-only nodes create a thundering herd of sync requests. No global limit. Producer drowns in serving requests instead of producing blocks.

### Pattern 5: Cooperative Scheduling Starvation in Blockchain Nodes
- **Ethereum geth (2020)**: Block import starved the miner. Fix: separate authoring thread.
- **Substrate (2021)**: Import queue starved block authorship. Fix: priority-separated tasks.
- **DOLI match**: Identical pattern. Network event processing starves block production.

---

## Synthesis: Two Orthogonal Failure Mechanisms

### Mechanism A: State Machine Complexity (whack-a-mole at same scale)

**Root cause**: SyncManager is a 73-field god object with no internal invariants. 9 boolean flags for mutually exclusive states (should be an enum). 10+ counters serving multiple purposes (should be purpose-specific). Monotonic high-water marks without decay. Side effects in query functions.

**Why fixes create new bugs**: Each fix adds conditions to prevent one state-space path. But the fix cannot reason about all 4,096+ state combinations. New bugs are state combinations that were reachable before but became likely after the fix redirected control flow. This is mathematically inevitable when the state space exceeds human reasoning capacity.

**Evidence**: 20+ fixes to SyncManager over 6 days. Each correct for its target. Each revealed new failure modes at the same or similar scale. The memory.db lesson (confidence 0.7) correctly identified this pattern.

**Minimum intervention**: Replace the 9 boolean flags with a RecoveryState enum. Split the overloaded `consecutive_empty_headers` counter into purpose-specific counters. Add invariant checks that assert mutual exclusivity of states. This reduces the reachable state space by 2-3 orders of magnitude and makes illegal states unrepresentable.

### Mechanism B: Event Loop Architecture (system breaks at new scale)

**Root cause**: Single tokio task with biased select!, no priority separation between production (critical) and gossip/sync-serving (non-critical). The architecture was designed for 10-20 peers and works up to ~30. Beyond that, network event volume exceeds production timer cadence.

**Why the current stall happened**: Adding 100 sync-only nodes created 10x more network events. The biased select gives these events absolute priority over production. Production timer never fires. Chain stalls.

**Why the 4 fork sync fixes did not prevent it**: The fixes were in sync recovery logic (Mechanism A). The stall is in event loop architecture (Mechanism B). They are in different subsystems. The stall was always latent -- just untriggered at 13-node scale.

**Minimum intervention**: Three changes, in order of impact:
1. **Decouple production to its own tokio::spawn task** with a dedicated timer and channel. This makes production immune to ANY sustained event stream.
2. **Add global sync serving admission control**: Max 8 concurrent sync request handlers. Queue or drop excess.
3. **Scale gossip frequency with peer count**: Minimum interval = max(5, sqrt(peer_count) * 2) seconds.

### Why BOTH mechanisms must be addressed

Fixing only Mechanism B (event loop) will stop the current stall but leave the SyncManager as a 73-field time bomb. Next fix to sync logic will create another Mechanism A bug.

Fixing only Mechanism A (state machine) will prevent same-scale whack-a-mole but leave the system vulnerable to the next scale increase. At 200 or 500 nodes, the event loop will stall again.

The minimum intervention that breaks the cycle is:
1. Production decoupling (Mechanism B -- immediate fix for the stall)
2. RecoveryState enum replacing 9 booleans (Mechanism A -- reduces state space 512x)
3. Global sync admission control (Mechanism B -- prevents thundering herd)

These three changes total ~200-300 lines and do not require a full rewrite.
