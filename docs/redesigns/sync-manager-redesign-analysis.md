# Sync Manager Redesign -- Problem Scoping Analysis
INC-I-103 escalation. Scope: crates/network/src/sync/manager/

## 1. Behavior to PRESERVE (acceptance criteria for any redesign)

Each item is a hard invariant that any redesign must maintain:

P1. **Sync from behind**: When a node is N blocks behind the network tip (N > 0), the sync coordinator must drive it to the canonical tip. This includes header-first sync (small gaps), snap sync (large gaps), and block-by-block gossip catch-up (1-2 block gaps).

P2. **Production gate**: The sync coordinator must block block production when the node is syncing, in a minority fork, or lacks sufficient peers. The gate must NEVER permanently lock -- every blocked state must have a recovery path. (Anti-pattern from the production gate deadlock: irreversible ratchet on `consecutive_resync_count`.)

P3. **Orphan chase**: When an orphan block arrives (parent unknown), the coordinator must request the parent from the sender peer and cache the orphan. After the parent is applied, cached orphans whose `prev_hash` matches the new tip are drained and applied. (Stability Pillar 1, 14 lines, `block_handling.rs`.)

P4. **Fork detection and recovery**: When the node detects it is on a minority fork (peers report a different canonical hash at the same height), the coordinator must roll back to the common ancestor and re-apply the canonical chain. Rollback must NOT exceed finality depth (INC-I-081 guard).

P5. **Snap sync**: When the gap exceeds a threshold, the coordinator must request a state snapshot from a peer, apply it, and resume normal sync. Snap-synced nodes have incomplete block history; any function that rebuilds state from local blocks must handle this (CLAUDE.md invariant).

P6. **Peer status tracking**: The coordinator must track peer-reported chain tips (height, hash) to determine the network tip, detect forks, and select sync sources.

P7. **State root convergence**: After any sync recovery (rollback, snap sync, re-apply), the node's state root must match all peers on the canonical chain. The 3-state invariant (ChainState, UtxoSet, ProducerSet) must be identical across all nodes.

P8. **Production resumption after sync**: Once sync completes and the node is at the canonical tip with sufficient peers, production must resume within one slot (10 seconds). No lingering "blocked" states.

P9. **Graceful behavior under deploy restarts**: Rolling restarts (which create temporary chain divergences) must NOT trigger cascading rollbacks or snap syncs. The coordinator must tolerate brief periods where peers report different heights/hashes during the restart window.

## 2. Behavior currently UNDERSPECIFIED (must be defined by redesign)

U1. **When to rollback vs. when to wait.** The code has at least 3 paths that trigger rollback (`stuck_fork_signal`, `ORPHAN_FORK`, `anti-cascade-orphan`), each with different criteria. No unified contract defines the decision boundary between "I should roll back" and "I should wait for gossip." The 2026-04-19 postmortem showed this directly: unconditional rollback on orphan `prev_hash` mismatch caused a 190-rollback cascade.

U2. **Peer count accounting: transport vs. mesh vs. sync-enrolled.** INC-I-103 showed transport reporting 45 peers while the sync coordinator saw 1. There is no contract defining which peer set the coordinator uses for its decisions, or how transport peer counts map to sync peer counts.

U3. **Backoff and escalation policy.** The current escalation path is: header-first -> fork sync -> snap sync -> genesis resync. There is no documented contract for the conditions that trigger each escalation, the backoff between attempts, or the maximum number of retries before escalation. The 2026-03-14 postmortem showed a 1-block gap triggering a full snap sync.

U4. **Interaction between sync state machine and block production timer.** The production gate has 11+ layers (per the deadlock analysis). The contract between sync state transitions and production gate state transitions is implicit -- embedded in shared mutable fields (`last_block_received_via_gossip`, `consecutive_resync_count`, `fork_mismatch_detected`, etc.) rather than explicit state machine transitions.

U5. **What constitutes a "stuck" sync.** The `stuck_fork_signal` fires based on `last_applied_ago` thresholds, but the threshold values and the actions they trigger vary across code paths. No single definition of "stuck" exists.

U6. **Floor/ceiling behavior after rollback.** The `reset_sync_for_rollback REFUSED` bug (2026-04-16 postmortem) showed that the sync floor was not lowered after rollback. The contract for when floors reset is undefined.

## 3. Capability inventory (PRIOR-KNOWLEDGE-GATE)

**NOTE: Source code access was blocked by a stale `.diag_active` pipeline gate. File:line references below are drawn from postmortems and existing analysis documents. Items that could not be verified against current source are marked [unverified-against-current-source].**

### Recovery actions
From postmortems and existing analysis, the sync coordinator uses these recovery mechanisms (not a formal enum in all cases):
- **HeaderFirstSync** -- request headers from peers, download blocks, apply in sequence. Triggered when gap is small (<1000 blocks per 2026-03-12 postmortem).
- **ForkSync** (binary search) -- find common ancestor with majority chain, rollback, re-apply. Can span multiple ticks. Was interrupted by 1-second sync retry before the 2026-03-12 fix.
- **ShallowRollback** -- rollback 1 block via `rollback_one_block()`. Triggered by `stuck_fork_signal`. Finality guard added after INC-I-081.
- **SnapSync** -- request state snapshot from peer, wipe local state, apply snapshot. Triggered when gap exceeds threshold or after repeated failures.
- **GenesisResync** (`force_recover_from_peers` / `reset_local_state`) -- full state reset. Triggered after 10 consecutive empty headers (pre-fix) or 3+ apply failures with large gap.
- **OrphanChase** -- request `GetBlockByHeight(local_height+1)` from orphan sender. Not a formal RecoveryAction, integrated into block handling.
- **SilencePull** (`catch_up_request`) -- request blocks from random peer after 30s gossip silence. Proactive pull mechanism.

[unverified-against-current-source: There may be a formal `RecoveryAction` enum in `recovery.rs` with additional variants not documented in postmortems.]

### Sync states / phases
From postmortems and docs:
- **Idle** -- no active sync. Production allowed (subject to gate checks).
- **DownloadingHeaders** / **Syncing:Headers** -- header-first sync active.
- **ForkSyncActive** -- fork sync binary search in progress.
- **SnapSyncing** -- snap sync in progress.
- **HeaderFirstSync** -- referenced in INC-I-103 analysis as the state n9-n12 were stuck in.

Transitions: Idle -> DownloadingHeaders (when `should_sync()` and gap detected), DownloadingHeaders -> Idle (headers applied or failed), Idle -> ForkSyncActive (fork detected), ForkSyncActive -> Idle (resolved or failed), Idle/any -> SnapSyncing (escalation threshold).

[unverified-against-current-source: The exact `SyncState` enum and transition table may differ from what postmortems describe.]

### Peer registries
From INC-I-103 analysis and architecture docs, the sync manager reads/writes to:
- **Transport peers** (libp2p swarm) -- all TCP-connected peers. In INC-I-103: 45 peers. Read by sync manager for peer discovery.
- **Sync-enrolled peers** (sync coordinator's internal set) -- peers the coordinator considers active for sync. In INC-I-103: 1 peer. This is the set that determines `InsufficientPeers`.
- **Gossipsub mesh peers** -- peers in the gossipsub mesh. A subset of transport peers. Read by sync manager for block delivery status.
- **Blacklisted peers** -- peers temporarily excluded from sync source selection. 60s cooldown (per 2026-03-12 postmortem).
- **Status peers** -- peers that have responded to status protocol queries. Source of `network_tip_height`.

[unverified-against-current-source: There may be additional peer sets in `peers.rs` not referenced in postmortems.]

### Sources of `network_tip_height`
From postmortems:
- Peer status responses (status protocol)
- Gossip block heights (from received blocks)
- Potentially: direct peer height queries during sync

[unverified-against-current-source: exact code paths that write `network_tip_height` require source reading.]

### Gates on production
From the production gate deadlock analysis:
- 11+ layers of production gate checks in `production_gate.rs`
- Key layers: InsufficientPeers, BlockedSyncing, BlockedExplicit (RPC pause), resync grace period (Layer 5), circuit breaker (Layer 10.5), ahead-of-peers (Layer 10)
- Each layer can independently block production
- Meta-safety violation documented: all layers can fire simultaneously, locking all producers permanently

### Entry points into sync from outside the manager
From CLAUDE.md code map:
- `handle_network_event()` in `event_loop.rs` -- gossip block receipt, peer status updates
- `run_periodic_tasks()` in `periodic.rs` -- timed sync checks, cleanup
- `handle_new_block()` in `block_handling.rs` -- block receipt from gossip or sync
- `pauseProduction` / `resumeProduction` RPC -- explicit production control
- Network service events (`swarm_events.rs`, `behaviour_events.rs`) -- connection/disconnection, protocol events

## 4. Cascade-history map

| Incident | Trigger | Shared shape | Guard added | Guard prevented NEXT cascade? |
|----------|---------|-------------|-------------|-------------------------------|
| INC-I-014 (2026-03-26) | 103+ nodes connecting; libp2p pending connections bypass `with_max_established()` | Unbounded resource accumulation under sync pressure -- pending connections allocate ~1MB each with no cap | 1 conn/peer, 120s cooldown, total cap=50 (partial commits) | NO -- INC-I-016 showed transport-layer peers still diverge from mesh membership; the connection cap prevented RAM explosion but didn't prevent mesh quality degradation |
| INC-I-016 (2026-03-28/29) | Peer eviction with all gossipsub scores at 0.0 on seeds/relays; then 4-min gossip silence -> 24-slot gap -> unbounded exclusion cascade | Sync coordinator trusts gossipsub mesh membership without verifying actual message delivery; unbounded HashSet modifying scheduler inputs | LIFO age tiebreaker, 30s grace period, MAX_EXCLUSIONS_PER_BLOCK=3, max_excluded_total=active/3, gossip watchdog (3295741e, c725fa76) | PARTIALLY -- exclusion caps prevented mass exclusion, but didn't address root cause: transport peers != gossip mesh. INC-I-103 shows same symptom (45 transport, 1 sync peer) |
| INC-I-049 | evidence-not-found (db-query-blocked) | evidence-not-found | evidence-not-found | evidence-not-found |
| INC-I-050 | evidence-not-found (db-query-blocked) | evidence-not-found | evidence-not-found | evidence-not-found |
| INC-I-081 (2026-05-18) | Broken producer emitted invalid epoch-boundary block (missing EpochReward); fleet rejected; sync state machine amplified | ShallowRollback past finality + plan_reorg ancestor lookup failure + silent candidate drop = fleet-wide fork. The sync coordinator escalated a single invalid block into a fleet partition. | 5-commit hotfix: typed Result on epoch rewards, ShallowRollback finality guard, plan_reorg block_store fallback, direct-apply fallback, finality reset backstop | UNKNOWN for next cascade -- finality guard was structural, but INC-I-103 shows the same coordinator still enters HeaderFirstSync after shallow rollback and gets stuck |
| INC-I-089 | evidence-not-found (db-query-blocked) | evidence-not-found | evidence-not-found | evidence-not-found |
| INC-I-090 | evidence-not-found (db-query-blocked) | evidence-not-found | evidence-not-found | evidence-not-found |
| INC-I-103 (2026-05-31) | ai5 nodes (n9-n12) diverged after coordinated restart; binary skew (post-3215a5eb vs older fleet); 2 of 4 nodes on same divergent hash at h=327598 | Transport/sync peer count split (45 vs 1), production blocked InsufficientPeers, coordinator stuck in HeaderFirstSync after shallow rollback, RAM climbing ~8MB/min from orphan-pool/mempool/mcache accumulation under sync stall | investigation ongoing | N/A (latest incident) |

**Additional cascade events from postmortems (not formally INC-numbered but in scope):**

| Event | Trigger | Shared shape | Guard added | Prevented next? |
|-------|---------|-------------|-------------|-----------------|
| 2026-03-12 fork cascade | Deploy restart -> snap sync -> fork depth > MAX_SAFE_REORG_DEPTH=10 -> infinite rollback loop. Sync retry timer (1s) resetting fork_sync binary search mid-operation. | Safety limit prevents recovery; timer resets multi-tick operation | MAX_SAFE_REORG_DEPTH raised to 500; fork_sync protected from sync retry (5d1fc5c) | PARTIALLY -- deeper rollbacks now possible, but NT8 header exhaustion and NT10 rollback-without-forward-sync remained open |
| 2026-03-14 snap sync cascade | Peer reports Hash::ZERO -> interpreted as fork -> snap sync -> node becomes carrier -> cascade through network | Small gap (1-2 blocks) escalated to full state wipe; contagion via Hash::ZERO propagation | Hash::ZERO ignored in fork detection; small-gap empty headers = wait for gossip; small-gap apply failures = wait (dd77c7e) | YES for Hash::ZERO vector specifically. NO for the general pattern: small issues still escalate to disproportionate recovery actions |
| 2026-04-16 synmgr deploy cascade | Rolling deploy created competing fork chains; `reset_sync_for_rollback REFUSED` (floor not lowered after rollback) | Rollback succeeds but sync floor prevents re-requesting correct blocks; cascading rollbacks with no backoff | evidence-not-found for specific commit (floor fix); deploy spacing increased | PARTIALLY -- floor fix addressed the specific REFUSED bug, but 2026-04-19 showed a different rollback cascade |
| 2026-04-19 orphan fork rollback cascade | External producers 1 slot late -> fork blocks via gossip -> ORPHAN_FORK unconditional rollback on `prev_hash` mismatch | 190 rollbacks in 19 minutes from 3 circulating fork blocks; no verification of whether local chain or orphan is wrong before rollback | v6.17.3 reverted to v6.17.2; proposed fix (peer-majority verification) not yet implemented | NO -- v6.17.3 "fix" made it 10x worse; reverted. The ORPHAN_CHASE infinite loop remains in v6.17.2 |

## 5. Cross-incident pattern extraction

The shared substrate of fragility across these cascades is:

**DISPROPORTIONATE ESCALATION WITH NO BACKPRESSURE**

Specifically: the sync coordinator has a single escalation ladder (wait -> header sync -> fork sync -> rollback -> snap sync -> genesis resync) where each rung takes a more destructive action than the last, but:

1. **Escalation triggers conflate different problems.** A 1-block gossip delay, a Hash::ZERO snap-sync gap, a fork from a late external producer, and a genuine chain split all feed into the same counters (`consecutive_empty_headers`, `consecutive_fork_blocks`, `consecutive_apply_failures`). The counter hits a threshold -> next rung fires. The counter does not distinguish "normal transient condition" from "genuine structural divergence."

2. **No de-escalation path.** Once a counter increments, it either resets on full success or continues incrementing. There is no intermediate state ("things are improving, hold current position"). The `consecutive_resync_count` ratchet (production gate deadlock) is the purest example: it only increments, never decrements, and its backoff formula grows exponentially with no cap.

3. **Recovery actions create the preconditions for the next escalation.** Snap sync creates an incomplete block store, which reports Hash::ZERO for missing blocks, which triggers fork detection on other nodes. Rollback makes the node more behind, which makes more blocks look like orphans, which triggers more rollbacks. This self-reinforcing loop is the cascade mechanism.

4. **Multiple independent timers/signals can trigger the same destructive action concurrently.** `stuck_fork_signal`, `anti-cascade-orphan`, `ORPHAN_FORK`, sync retry timer -- each can independently trigger a rollback. They run on different cadences and don't coordinate. A node can receive multiple rollback signals per tick.

5. **The coordinator's peer model does not match the network's actual topology.** The coordinator tracks its own peer registries that can diverge from transport-layer and gossipsub-layer reality. INC-I-103: 45 transport peers, 1 sync peer. INC-I-016: 17 transport peers, 0 gossip mesh peers. The coordinator makes life-or-death decisions (snap sync, production block) based on its own registry, which may be stale or incomplete.

The shared architectural name: **escalation-without-discrimination**. The coordinator escalates aggressively but cannot distinguish severity, and its recovery actions are self-amplifying.

## 6. Architectural constraints any redesign must respect

### Consensus invariants
- Any change to block content (attestation bitfield, coinbase format, tx ordering, header fields) requires an activation height in `NetworkParams`. Rolling deploy creates mixed-version blocks -> fork.
- `CURRENT_PROTOCOL_VERSION` must NOT be bumped unless `EpochState` serialization format changes.
- Activation heights, once crossed on mainnet, are IMMUTABLE. New features get their own activation height.
- Sync coordinator changes that alter WHEN a node produces (but not WHAT it produces) do NOT require activation heights -- they are local policy. BUT: production timing affects which blocks exist at which heights, so the blast radius must be carefully evaluated.

### Storage invariants
- Rollback uses undo data kept for `UNDO_KEEP_DEPTH` (100 blocks; defined in `crates/core/src/consensus/constants.rs`). Rollback beyond undo data requires rebuild from blocks (fallback path in `rollback.rs` and `block_handling.rs::execute_reorg`).
- Finality is enforced: ShallowRollback must not go past finality depth (INC-I-081 guard).
- Block store gaps from snap sync are permanent unless backfilled.

### Snap-sync compatibility
- Snap-synced nodes have incomplete local block history (blocks before snapshot height are missing).
- Any function that rebuilds state from local blocks must handle incomplete history.
- `plan_reorg` ancestor lookup must fall back to `block_store` when local blocks are missing (INC-I-081 fix).
- The state root at the snap-sync height must match the source peer's state root exactly.

### Mainnet rolling-deploy constraint
- ~30 external producers exist beyond the 12 structural nodes. Synchronized stop-all is NOT possible.
- ALL feature activations MUST use activation heights + upgrade lead time -- same discipline as mainnet.
- Block content changes require synchronized deploy across the structural fleet, but external producers will be on old binaries during the transition window.

### Three-question consensus-shape checklist
Before merging any sync coordinator change:
1. Can any user-submittable transaction trigger this code path?
2. Can any producer-action or attestation pattern trigger it?
3. Is the new behavior bit-identical to the old behavior for ALL reachable inputs?

If (1) or (2) is YES and (3) is NO -> activation height required.

## 7. Acceptance criteria for the REDESIGN ITSELF

### Must
- M1. Every behavior in Section 1 (P1-P9) has an explicit test that verifies it.
- M2. No recovery action creates the preconditions for the next escalation (anti-amplification invariant).
- M3. Small-gap conditions (1-5 blocks behind) never trigger snap sync or genesis resync.
- M4. Production gate always has a recovery path. No combination of layer states can permanently lock production.
- M5. Rolling deploy with binary skew across the fleet does not trigger cascading rollbacks or snap syncs.
- M6. Rollback respects finality depth. No rollback path can bypass the finality guard.
- M7. Existing consensus invariants, activation height discipline, and deploy constraints from Section 6 are preserved.

### Should
- S1. Single peer registry: one authoritative peer set, no derived secondary registries that can diverge from transport/gossip reality.
- S2. Every escalation trigger has a documented discrimination criterion: what distinguishes "transient condition" from "genuine structural divergence."
- S3. Every counter/timer that can trigger a destructive action has a documented de-escalation path and a cap.
- S4. Sync state machine has a formal state diagram with explicit transition conditions, documented in code comments and in specs.
- S5. Each recovery action has exactly one handler. No multiple independent paths that can trigger the same destructive action concurrently.
- S6. Measurable: the coordinator logs enough structured data to reconstruct the escalation path post-incident.

### Could
- C1. Property-based tests for the sync state machine: random sequences of peer events, block arrivals, and failures never produce cascading escalation for small-gap conditions.
- C2. Separation of sync coordinator into testable pure-function decision engine + side-effectful executor.
- C3. Configurable escalation thresholds via `NetworkParams` or env vars.

### Won't
- W1. Snap-sync protocol redesign.
- W2. libp2p replacement.
- W3. Gossipsub parameter tuning (unless the redesign changes which peer set the coordinator uses).
- W4. Consensus rule changes.

## 8. Open questions for the design evaluators

Q1. How should the coordinator distinguish "transport peer" from "sync-capable peer" from "gossip-delivering peer"?
Q2. Should rollback be triggered by the sync coordinator at all, or should it be a separate subsystem?
Q3. What is the correct backoff function for sync escalation? Linear, exponential, or adaptive?
Q4. Should the coordinator have a "health self-assessment" distinguishing "behind but recovering" from "stuck and diverging"?
Q5. How should the redesign handle the transition from old coordinator to new on a live network with external producers?
Q6. Is the production gate's 11+ layer architecture worth preserving, or should it be collapsed?
Q7. Should the sync coordinator own its observability, or emit events for a separate subsystem?

## 9. Honesty checklist
- evidence-not-found cells included where DB query was blocked: YES (INC-I-049, 050, 089, 090).
- No fix proposed in this document: YES.
- Shared fragility named without naming "the" solution: YES.