# Redesign Analysis: Epoch-Boundary Liveness Pruning (INC-I-116)

> **Problem-scoping analysis for design evaluators. Proposal-only -- no implementation.**

## Scope

**Affected modules/files:**
- `crates/core/src/scheduler.rs` -- DeterministicScheduler (ticket-based leader selection)
- `crates/core/src/epoch_state/mod.rs` -- EpochState::derive_at_boundary (canonical epoch derivation)
- `crates/core/src/consensus/constants.rs` -- scheduler/liveness constants
- `crates/core/src/network_params/mod.rs` + `defaults.rs` -- activation height (new field required)
- `bins/node/src/node/apply_block/post_commit.rs` -- epoch boundary state refresh
- `bins/node/src/node/apply_block/mod.rs` -- producer_liveness tracker (node-local)
- `bins/node/src/node/rewards.rs` -- rebuild_producer_liveness + epoch reward validation

**Incident:** INC-I-116 (OMEGA DB id=131, high severity, domain=consensus-scheduler, OPEN)

## Summary (plain language)

DOLI's block scheduler gives every registered producer a share of slots proportional to their bonds. When absent producers miss their slots, those slots are lost -- the chain produces nothing during those windows. The existing ghost-exclusion mechanism only adjusts a safety floor denominator; it does NOT remove absent producers from the schedule. After an epoch boundary, all 57 registered producers remain in the schedule even if only 12 are actually online, yielding ~21% slot utilization instead of ~100%.

The redesign must add a deterministic epoch-boundary mechanism that prunes producers who demonstrably failed a liveness threshold from the NEXT epoch's leader schedule, re-including them when they resume participating.

## 1. Current Scheduler Architecture

### 1.1 Ticket Assignment and Leader Selection

**File:** `crates/core/src/scheduler.rs`

The `DeterministicScheduler` (line 91) is a pure function:

1. **Input:** `Vec<ScheduledProducer>` (pubkey + bond_units)
2. **Sort:** producers sorted by pubkey bytes (line 108) -- deterministic ordering
3. **Ticket boundaries:** cumulative sum of bond_units per producer (line 114-119). Each producer owns a contiguous ticket range.
4. **Selection:** `slot % total_bonds` yields a ticket number; binary search in `ticket_boundaries` finds the owning producer (lines 181-184)
5. **Fallback:** `MAX_FALLBACK_RANKS = 2` (constants.rs:602). Rank 1 offset = `total_bonds / 2`. Each rank gets an exclusive 2-second window within a 10-second slot. Only 2 producers can attempt any given slot.

**Key insight:** If the primary (rank 0) and fallback (rank 1) for a slot are BOTH absent, the slot is dead. With 45/57 producers absent, the probability of both ranks being absent for any given slot is high because ticket ranges are contiguous -- large absent producers can own entire swathes of the ticket space.

### 1.2 Where the Scheduler Gets Its Input

The scheduler does NOT directly read the ProducerSet. The `EpochState` mediates:

**Post-epoch-boundary (normal operation):**
`post_commit.rs:260-265` builds the input:
```
ProducerSet.active_producers_for_scheduling_at_height()
  -> filters: is_active() AND past ACTIVATION_DELAY AND selection_weight > 0
  -> passed as `active_producers` in EpochDerivationInput
```

`EpochState::derive_at_boundary()` (epoch_state/mod.rs:141) then filters through the attestation filter, which is the critical path where pruning should attach.

**Bootstrap (genesis epoch 0):**
Uses `known_producers` with a separate `producer_liveness` filter (production/scheduling.rs:348-366). This IS liveness-aware but is node-local and non-consensus.

### 1.3 Fallback Coverage Gap

With `MAX_FALLBACK_RANKS = 2`, each slot has exactly 2 candidates (primary + 1 fallback). If 45 of 57 producers are absent (~79% of registrations), the expected probability that BOTH ranks are absent for a random slot is approximately `(45/57)^2 ~ 62%`. But because tickets are contiguous, it's worse: a single absent producer with many bonds creates a contiguous dead zone. The observed ~21-24% utilization (12/57 ~= 21%) confirms this matches roughly `live_weight / total_weight` = 21728/27360 = 79.4% of WEIGHT but only ~21% of SLOTS because the ticket assignment is contiguous, not interleaved.

## 2. Epoch-Boundary Mutation Pipeline

### 2.1 What Changes at an Epoch Boundary Today

**File:** `apply_block/state_update.rs:152-226` (`track_finality_and_apply_deferred`)

At each epoch boundary (when `height % blocks_per_epoch == 0`):

1. **Deferred producer mutations applied** (state_update.rs:194-217):
   - `ProducerSet::apply_pending_updates_with_cap()` flushes all queued `PendingProducerUpdate` entries
   - Mutation types: Register, Exit, Slash, AddBond, DelegateBond, RevokeDelegation, RequestWithdrawal
   - These were queued during the epoch by `queue_update()` (set_core.rs:74)

2. **Maintainer set bootstrap** (state_update.rs:223)

**File:** `apply_block/post_commit.rs:153-367` (`post_commit_actions` at epoch boundary)

3. **EpochSnapshot built** (post_commit.rs:157-183): merkle root over all active producers
4. **Bond snapshot rebuilt** (post_commit.rs:186-246): `{pubkey_hash -> selection_weight}` from current ProducerSet
5. **Active producers extracted** (post_commit.rs:249-271): `active_producers_for_scheduling_at_height()` filters by active status + ACTIVATION_DELAY + weight > 0
6. **`EpochState::derive_at_boundary()` called** (post_commit.rs:312): THE canonical derivation
7. **New EpochState persisted** (post_commit.rs:323-332): producer_list, active_list, attestation accumulators, bond snapshot, epoch state
8. **Attestation accumulators rotated** (inside derive_at_boundary, epoch_state/mod.rs:280-289): `[0] -> [1] -> [2]`, new `[0]` starts empty

### 2.2 What Does NOT Change at the Boundary

- **ProducerSet.producers map** is NOT pruned for liveness. A producer with `status=Active` remains Active regardless of whether they produced any blocks or attested.
- **`is_active()`** (info.rs:205-210) only checks `ProducerStatus::Active | Unbonding`. It does NOT check `last_activity`, `activity_status`, or any liveness signal.
- **`active_producers_for_scheduling_at_height()`** (set_core.rs:355-370) filters by `is_active()` + weight > 0. No liveness filter.

### 2.3 The Attestation Filter in `derive_at_boundary` (the Closest Existing Primitive)

`epoch_state/mod.rs:152-223`:

1. Build `attested_union` = union of `attested_sets[0..3]` (3-epoch lookback)
2. Filter `active_producers` to only those in `attested_union` -> `new_list`
3. **Deadlock safety floor** (line 209): if `new_list.len() < effective_active * 2/3`, OVERRIDE new_list to ALL non-ghost active producers

**This is WHERE the bug lives.** The attestation filter DOES conceptually prune absent producers. But the 2/3 deadlock safety floor OVERRIDES it whenever the pruned set is too small. When 45/57 are absent, the attested set might be ~12, which is well below `57 * 2/3 = 38`, so the floor fires and ALL producers are included.

The ghost exclusion (lines 178-207) adjusts the denominator: it subtracts producers absent for >3 epochs from `effective_active`. But this only helps the floor calculation -- it does NOT directly remove them from the schedule. Even with ghost exclusion active, if 45 producers disappeared in the CURRENT epoch (not yet 3 epochs old), `ghost_count = 0` and the floor is `57 * 2/3 = 38` -- still overriding the attestation filter.

## 3. Existing Liveness Tracking

### 3.1 `producer_liveness` (Node-Local, Non-Consensus)

**Type:** `HashMap<PublicKey, u64>` (pubkey -> last block height produced)
**Location:** `bins/node/src/node/mod.rs:178`

**Fed by:**
- `apply_block/mod.rs:140`: `self.producer_liveness.insert(block.header.producer, height)` -- every applied block
- `init.rs:546-564`: rebuilt from block_store at startup (last `LIVENESS_WINDOW_MIN=500` blocks)
- `rewards.rs:529-544`: rebuilt after rollback (same window)

**Used by (all node-local, non-consensus):**
- `validation_checks.rs:72`: bootstrap validation liveness split (live vs stale producers for scheduling)
- `production/scheduling.rs:350`: bootstrap round-robin liveness filter
- Both use it to partition producers into live/stale for the BOOTSTRAP scheduler only

**Critically:** `producer_liveness` is NOT an input to `EpochState::derive_at_boundary()`. It is NOT persisted in the EpochState. It is NOT available to snap-synced nodes (they rebuild it from whatever blocks they have locally). It is **purely observational** and **NOT consensus-safe**.

### 3.2 `ProducerInfo.last_activity` (Persisted but Unused for Scheduling)

**Location:** `crates/storage/src/producer/types.rs:119`
**Type:** `u64` (block height of last activity)

This field exists on every `ProducerInfo` but is NOT updated by the epoch pool reward system (it was part of the old Pull/Claim model). The `activity_status()` method (info.rs:684) computes `Active/RecentlyInactive/Dormant` from it, but this status is used ONLY for governance power, NOT for scheduling.

### 3.3 `ActivityStatus` (Governance Only, Not Scheduling)

**Location:** `crates/storage/src/producer/types.rs:37-56`

Three states: Active, RecentlyInactive, Dormant. Used for:
- Governance quorum denominator (only Active producers count)
- Governance voting eligibility

NOT used for scheduling at all.

### 3.4 `EpochState.attested_sets` and `attestation_accum` (Consensus-Safe, Already Used)

**Location:** `crates/core/src/epoch_state/mod.rs:79-87`

- `attested_sets: [HashSet<PublicKey>; 3]` -- producers who attested in each of 3 epochs
- `attestation_accum: [HashMap<PublicKey, HashSet<u32>>; 3]` -- per-producer minute-level attestation tracking
- `blocks_produced: HashMap<PublicKey, u32>` -- blocks produced in current epoch

**These ARE consensus inputs.** They are:
- Fed deterministically by `accumulate_block()` from block attestation bitfields
- Rotated at epoch boundary by `derive_at_boundary()`
- Persisted atomically with the block commit (post_commit.rs:145)
- Available to snap-synced nodes (persisted in EpochState)

**These are the ONLY deterministic liveness signals available at the epoch boundary.**

## 4. Ghost Exclusion Deep Dive (INC-I-016 / INC-I-046)

### 4.1 What It Does

Ghost exclusion (epoch_state/mod.rs:178-223) does NOT remove producers from the schedule. It adjusts the 2/3 safety floor denominator:

```
effective_active = active_count - ghost_count
```

A producer is a "ghost" if:
1. NOT in `attested_union` (absent from ALL 3 lookback epochs)
2. Registered for > `GHOST_EXCLUSION_GRACE_EPOCHS = 3` epochs

### 4.2 Why It Cannot Prune a Large Absent Cohort

Three reasons:

1. **Grace period:** New absences don't count as ghosts until 3 epochs have passed. A sudden partition (like INC-I-114 Fork A) creates 45 absent producers who are NOT ghosts yet.

2. **Denominator-only:** Even when ghosts ARE subtracted from `effective_active`, the floor logic is: if `new_list.len() < effective_active * 2/3`, include ALL non-ghost producers. So a producer absent for <3 epochs is NEVER excluded -- it's included via the floor override.

3. **The floor is a SAFETY mechanism:** It exists to prevent an attestation bitfield bug from shrinking the producer set to zero. It correctly protects against false positives (producers wrongly classified as absent due to bitfield decode bugs). But it also prevents the system from actually pruning absent producers.

### 4.3 The Original `excluded_producers` System (INC-I-016 — REMOVED)

The incident description mentions "MAX_EXCLUSIONS_PER_BLOCK=3, max_excluded_total ~= active/3". This was a per-block exclusion mechanism from an earlier version. **It no longer exists in the codebase.** There is no `excluded_producers` field on the Node struct (the comment at mod.rs:157 is a vestigial fragment that flows into the `epoch_state` field description). The per-block exclusion was removed, likely as part of the epoch state consolidation.

## 5. Capability Inventory (PRIOR-KNOWLEDGE-GATE)

### 5.1 Producer-Set Mutation Types (7 total)

All deferred to epoch boundary via `PendingProducerUpdate`:

| # | Type | What It Does |
|---|------|-------------|
| 1 | Register | Add new producer with bonds |
| 2 | Exit | Start unbonding period |
| 3 | Slash | 100% bond burn, permanent exclusion |
| 4 | AddBond | Increase bond count |
| 5 | DelegateBond | Transfer scheduling weight to another producer |
| 6 | RevokeDelegation | Undo delegation |
| 7 | RequestWithdrawal | FIFO bond withdrawal with vesting penalty |

### 5.2 Epoch-Boundary Operations (8 total)

| # | Operation | Location |
|---|-----------|----------|
| 1 | Flush pending producer mutations | state_update.rs:194 |
| 2 | Build EpochSnapshot (merkle root) | post_commit.rs:157 |
| 3 | Rebuild bond snapshot | post_commit.rs:186 |
| 4 | Extract active producers | post_commit.rs:249 |
| 5 | Run derive_at_boundary | post_commit.rs:312 |
| 6 | Persist new EpochState | post_commit.rs:323 |
| 7 | Aggregate oracle prices | post_commit.rs:350 |
| 8 | Rotate attestation accumulators | epoch_state/mod.rs:280 |

### 5.3 Liveness Signals (5 total)

| # | Signal | Consensus-Safe? | Available to Snap-Synced? |
|---|--------|-----------------|--------------------------|
| 1 | `EpochState.attested_sets[0..3]` | YES (deterministic from bitfields) | YES (persisted in EpochState) |
| 2 | `EpochState.attestation_accum[0..3]` | YES (deterministic from bitfields) | YES (persisted in EpochState) |
| 3 | `EpochState.blocks_produced` | YES (deterministic from blocks) | YES (persisted in EpochState) |
| 4 | `Node.producer_liveness` | NO (node-local, rebuilt from block_store) | NO (depends on local block history) |
| 5 | `ProducerInfo.last_activity` | NO (persisted but not updated by epoch pool) | Partially (stale data) |

### 5.4 Activation Heights in NetworkParams (19 total)

| # | Field | Status |
|---|-------|--------|
| 1 | `inc_i_026_scheduler_activation_height` | Active (mainnet=0) |
| 2 | `fork_id_activation_height` | Active (mainnet=0) |
| 3 | `full_bitfield_decode_height` | Active |
| 4 | `rewards_epoch_list_fix_height` | Active |
| 5 | `encrypted_content_activation_height` | Active |
| 6 | `encrypted_content_v2_activation_height` | Active (mainnet=100_000) |
| 7 | `epoch_state_reorg_activation_height` | Active |
| 8 | `security_audit_activation_height` | Active (mainnet=27_547) |
| 9 | `ghost_exclusion_activation_height` | Active (mainnet=18_152) |
| 10 | `inc_i_068_weight_filter_activation_height` | Active (mainnet=197_800) |
| 11 | `received_delegation_cap_activation_height` | Active (mainnet=254_344) |
| 12 | `delegation_auth_activation_height` | Active (mainnet=254_344) |
| 13 | `addbond_cap_enforcement_activation_height` | Active (mainnet=254_344) |
| 14 | `defi_activation_height` | Disabled (u64::MAX, tombstoned) |
| 15 | `amm_activation_height` | Active (mainnet=375_640) |
| 16 | `oracle_activation_height` | Disabled (u64::MAX) |
| 17 | `large_block_activation_height` | Active (mainnet=375_640) |
| 18 | `inc_i_092_activation_height` | Active (mainnet=375_640) |
| 19 | `inc_i_096_activation_height` | Active (mainnet=375_640) |

### 5.5 Scheduler-Relevant Constants

| Constant | Value | Location |
|----------|-------|----------|
| `MAX_FALLBACK_RANKS` | 2 | constants.rs:602 |
| `GHOST_EXCLUSION_GRACE_EPOCHS` | 3 | constants.rs:147 |
| `MIN_ATTESTATION_MINUTES` | 30 | constants.rs:98 |
| `ACTIVE_PRODUCERS_CAP` | 50 | constants.rs:74 |
| `LIVENESS_WINDOW_MIN` | 500 | constants.rs:222 |
| `ACTIVATION_DELAY` | 10 blocks | constants.rs (producer) |

## 6. What Is Tangled / Coupled / Misplaced

### 6.1 The Core Coupling Problem

`derive_at_boundary()` has TWO conflicting responsibilities:
1. **Attestation-filter** the producer list (remove non-attesting producers)
2. **Deadlock-protect** with a 2/3 floor (ensure the filtered list isn't dangerously small)

These conflict when a large cohort is absent: the attestation filter correctly identifies them, but the floor overrides the filter and includes them anyway.

### 6.2 Ghost Exclusion Is a Denominator Adjustment, Not a Pruning Mechanism

Ghost exclusion adjusts `effective_active` (the denominator of the 2/3 check) but only for producers absent >3 epochs. It NEVER directly removes producers from `new_list`. The gap: there is no mechanism for removing producers absent for <3 epochs from the schedule.

### 6.3 `producer_liveness` Is the Right Signal in the Wrong Place

The node-local `producer_liveness` HashMap tracks exactly what we need (who produced recently) but is:
- Not deterministic (depends on local block history, which varies for snap-synced nodes)
- Not a consensus input (not in EpochState)
- Not available at `derive_at_boundary()` time

Meanwhile, `EpochState.blocks_produced` and `attested_sets[0]` carry the SAME information in a consensus-safe form, but `derive_at_boundary()` only uses them for the tier promotion filter (ACTIVE_PRODUCERS_CAP path), not for the main producer list.

### 6.4 Minimal Seam for Attaching a Deterministic Prune

The cleanest attachment point is inside `derive_at_boundary()` (epoch_state/mod.rs), between step 2 (attestation filter) and step 3 (deadlock floor). The function already receives all necessary inputs:
- `prev.attested_sets` -- who attested in the last 3 epochs
- `prev.blocks_produced` -- who produced blocks in the just-completed epoch
- `input.active_producers` -- all registered producers

A liveness prune would filter `input.active_producers` BEFORE building `new_list`, ensuring absent producers are excluded from BOTH the filtered list AND the floor calculation.

## 7. Determinism and Snap-Sync Analysis

### 7.1 Deterministic-Safe Liveness Signals

These are available at every epoch boundary, agree across all nodes (including snap-synced), and are persisted in EpochState:

| Signal | What It Represents | Determinism |
|--------|-------------------|-------------|
| `prev.attested_sets[0]` | Producers who attested in the just-completed epoch | BIT-IDENTICAL -- derived from bitfields in applied blocks |
| `prev.attested_sets[1]` | Producers who attested in the epoch before that | BIT-IDENTICAL |
| `prev.attested_sets[2]` | Producers who attested 2 epochs ago | BIT-IDENTICAL |
| `prev.blocks_produced` | Blocks each producer produced in the just-completed epoch | BIT-IDENTICAL -- counted from applied blocks |
| `prev.attestation_accum[0]` | Per-producer minute-level attestation detail | BIT-IDENTICAL |

### 7.2 UNSAFE Liveness Signals (Must NOT Be Used)

| Signal | Why Unsafe |
|--------|-----------|
| `Node.producer_liveness` | Node-local HashMap, rebuilt from local block_store. Snap-synced nodes have incomplete history -> different map -> different prune decisions -> FORK. |
| `ProducerInfo.last_activity` | Stale field (not updated by epoch pool). Different nodes may have different values after rollback/restore. |
| Any wall-clock / network-derived signal | Non-deterministic by definition. |

### 7.3 Snap-Sync Specific Concerns

A snap-synced node receives:
- EpochState (including attested_sets, attestation_accum, blocks_produced)
- ProducerSet (including all ProducerInfo fields)
- UtxoSet
- ChainState

It does NOT have:
- Full block history (only blocks from sync floor onward)
- Full `producer_liveness` (rebuilt from available blocks only)

Therefore, any prune computation MUST derive exclusively from EpochState fields. The existing `attested_sets` and `blocks_produced` satisfy this requirement.

## 8. Requirements

### REQ-PRUNE-001: Deterministic Epoch-Boundary Liveness Prune

| Field | Value |
|-------|-------|
| **ID** | REQ-PRUNE-001 |
| **Priority** | Must |
| **Description** | At each epoch boundary, `derive_at_boundary()` must deterministically remove producers who failed a liveness threshold over the prior epoch from the next epoch's `producer_list` (and therefore from the scheduler input). |
| **Acceptance Criteria** | - [ ] Given producers A (produced 10 blocks in epoch N) and B (produced 0 blocks, 0 attestations in epoch N), when epoch N+1 boundary is reached, then B is NOT in `epoch_state.producer_list` for epoch N+1 |
| | - [ ] Given a snap-synced node and a full node at the same height, both compute identical `producer_list` at the epoch boundary |
| | - [ ] Given the same chain of blocks applied in the same order, any two nodes produce bit-identical `EpochState` after `derive_at_boundary()` |
| | - [ ] The prune decision derives ONLY from fields persisted in EpochState (attested_sets, blocks_produced, attestation_accum) -- no node-local state |

### REQ-PRUNE-002: Activation Height Gate

| Field | Value |
|-------|-------|
| **ID** | REQ-PRUNE-002 |
| **Priority** | Must |
| **Description** | The liveness prune behavior must be gated behind a new activation height in NetworkParams (`epoch_prune_activation_height`). Pre-activation: behavior is identical to current (no prune). Post-activation: prune is applied. Per INC-I-075 three-question checklist: Q1=NO (no user tx triggers it), Q2=YES (producer attestation pattern triggers it), Q3=NO (new behavior differs from old) -> activation height REQUIRED. |
| **Acceptance Criteria** | - [ ] New field `epoch_prune_activation_height: u64` exists in NetworkParams |
| | - [ ] Mainnet default is `u64::MAX` (disabled until explicitly pinned) |
| | - [ ] Testnet and devnet defaults are `0` (active from genesis for testing) |
| | - [ ] Pre-activation height: `derive_at_boundary()` produces identical output to current implementation |
| | - [ ] Once crossed on mainnet, the height is immutable (per INC-I-054 rule) |

### REQ-PRUNE-003: Re-Inclusion of Resumed Producers

| Field | Value |
|-------|-------|
| **ID** | REQ-PRUNE-003 |
| **Priority** | Must |
| **Description** | A pruned producer who resumes participating (attesting/producing) must be re-included in the schedule at the next epoch boundary after they demonstrate liveness. Pruning is NOT permanent exclusion. |
| **Acceptance Criteria** | - [ ] Given producer P was pruned at epoch N boundary, and P attests during epoch N, then P is included in `producer_list` at epoch N+1 boundary |
| | - [ ] Re-inclusion does not require any on-chain transaction from P |
| | - [ ] P retains their bond weight and scheduling tickets upon re-inclusion |

### REQ-PRUNE-004: Forward-Only Activation (No Genesis Reset)

| Field | Value |
|-------|-------|
| **ID** | REQ-PRUNE-004 |
| **Priority** | Must |
| **Description** | The prune mechanism activates at a future height. No genesis reset, no retroactive state change, no protocol version bump (EpochState format unchanged if prune uses existing fields). |
| **Acceptance Criteria** | - [ ] Existing chain state is unmodified below the activation height |
| | - [ ] `CURRENT_PROTOCOL_VERSION` is NOT bumped (unless EpochState serialization format changes) |
| | - [ ] `EPOCH_STATE_FORMAT_VERSION` is NOT bumped (prune uses existing EpochState fields) |

### REQ-PRUNE-005: Deadlock Safety Floor Interaction

| Field | Value |
|-------|-------|
| **ID** | REQ-PRUNE-005 |
| **Priority** | Must |
| **Description** | The prune mechanism must interact correctly with the 2/3 deadlock safety floor. A prune that would reduce the schedule below a minimum viable producer count must be bounded. |
| **Acceptance Criteria** | - [ ] If pruning would leave fewer than `MIN_PRODUCERS_FLOOR` (design parameter, e.g., 3) producers in the schedule, prune is capped to retain at least that many |
| | - [ ] The floor denominator uses the PRUNED set size, not the full registered set size |
| | - [ ] An attacker who partitions honest producers cannot use liveness prune to reduce the schedule below the floor |

### REQ-PRUNE-006: Bond-Weight Preservation

| Field | Value |
|-------|-------|
| **ID** | REQ-PRUNE-006 |
| **Priority** | Must |
| **Description** | Pruning a producer from the schedule does NOT affect their bond, registration status, delegation state, or economic position. Pruning is schedule-only. |
| **Acceptance Criteria** | - [ ] A pruned producer's `ProducerStatus` remains `Active` |
| | - [ ] A pruned producer's bonds are not slashed or modified |
| | - [ ] A pruned producer's delegations (incoming and outgoing) are not affected |
| | - [ ] A pruned producer continues to appear in `getProducers` RPC (with a status field indicating schedule exclusion) |
| | - [ ] Epoch rewards are distributed only to producers who are in the schedule AND produced blocks (existing behavior -- pruned producers simply aren't in the schedule) |

### REQ-PRUNE-007: Graceful Throughput Degradation

| Field | Value |
|-------|-------|
| **ID** | REQ-PRUNE-007 |
| **Priority** | Should |
| **Description** | With L of N registered producers live, slot utilization should approach L/L (100% of live capacity) rather than L/N (diluted by absent producers). |
| **Acceptance Criteria** | - [ ] Given 12 of 57 producers are live and pruning is active, slot utilization exceeds 80% (was ~21% without pruning) |
| | - [ ] The scheduler's `total_bonds` reflects only the bonds of live (non-pruned) producers |
| | - [ ] Ticket ranges are contiguous among live producers only |

### REQ-PRUNE-008: Liveness Signal as Clean Consensus Input

| Field | Value |
|-------|-------|
| **ID** | REQ-PRUNE-008 |
| **Priority** | Should |
| **Description** | The liveness signal used for pruning should be a first-class input to `derive_at_boundary()`, not a side-channel or post-hoc adjustment. |
| **Acceptance Criteria** | - [ ] The prune decision is computed entirely inside `derive_at_boundary()` from its `EpochDerivationInput` and `prev: &EpochState` |
| | - [ ] No new node-local state is required for the prune computation |
| | - [ ] The prune computation is documented as part of the `derive_at_boundary()` contract |

### REQ-PRUNE-009: Decoupling from Ghost Exclusion

| Field | Value |
|-------|-------|
| **ID** | REQ-PRUNE-009 |
| **Priority** | Should |
| **Description** | The epoch-boundary prune is architecturally distinct from ghost exclusion. Ghost exclusion adjusts the safety floor denominator for chronically absent producers (>3 epochs). Epoch prune removes recently-absent producers from the schedule. Both can coexist. |
| **Acceptance Criteria** | - [ ] Ghost exclusion logic (epoch_state/mod.rs:178-223) is not modified |
| | - [ ] A producer can be ghost-excluded AND liveness-pruned (they are independent filters) |
| | - [ ] The prune fires BEFORE the ghost exclusion denominator adjustment |

### REQ-PRUNE-010: Interaction with Tier System

| Field | Value |
|-------|-------|
| **ID** | REQ-PRUNE-010 |
| **Priority** | Should |
| **Description** | The prune must compose correctly with the tier system (ACTIVE_PRODUCERS_CAP, tier promotion). A pruned producer should not occupy a tier slot. |
| **Acceptance Criteria** | - [ ] Pruned producers are excluded before the tier cap is applied |
| | - [ ] The ACTIVE_PRODUCERS_CAP counts only non-pruned producers |

### REQ-PRUNE-011: Observability

| Field | Value |
|-------|-------|
| **ID** | REQ-PRUNE-011 |
| **Priority** | Could |
| **Description** | Operators should be able to observe prune decisions via logs and RPC. |
| **Acceptance Criteria** | - [ ] Epoch boundary log includes count of pruned producers and their identities |
| | - [ ] `getProducers` RPC response includes a field indicating whether a producer is schedule-pruned |

### REQ-PRUNE-012: Simplicity Constraint

| Field | Value |
|-------|-------|
| **ID** | REQ-PRUNE-012 |
| **Priority** | Could |
| **Description** | Prefer the simplest liveness threshold that addresses the problem. Avoid complex multi-signal composite metrics. |
| **Acceptance Criteria** | - [ ] The liveness threshold can be expressed in one sentence |
| | - [ ] The threshold uses at most 2 of the 3 available signals (attested_sets, blocks_produced, attestation_accum) |

### REQ-PRUNE-013: Economic Penalties for Absence

| Field | Value |
|-------|-------|
| **ID** | REQ-PRUNE-013 |
| **Priority** | Won't |
| **Description** | Slashing, bond reduction, or any economic penalty for absent producers is explicitly OUT OF SCOPE for this redesign. Pruning is a scheduling optimization, not a punishment mechanism. |

### REQ-PRUNE-014: Bond Mechanics Changes

| Field | Value |
|-------|-------|
| **ID** | REQ-PRUNE-014 |
| **Priority** | Won't |
| **Description** | Changes to bond stacking, withdrawal, delegation, or any economic parameter are OUT OF SCOPE. |

## Architecture Context

### Module Boundaries

| Module | Responsibility | Depends On | Depended By |
|--------|---------------|------------|-------------|
| `crates/core/src/scheduler.rs` | Pure ticket-based selection from `Vec<ScheduledProducer>` | `consensus::MAX_FALLBACK_RANKS` | production, validation |
| `crates/core/src/epoch_state/mod.rs` | Canonical epoch derivation; produces producer_list, active_list, bond_snapshot | Nothing (pure function) | apply_block/post_commit, rewards, validation, fork_recovery |
| `bins/node/src/node/apply_block/post_commit.rs` | Calls derive_at_boundary, builds EpochDerivationInput | ProducerSet, EpochState | Block persistence pipeline |
| `crates/storage/src/producer/set_core.rs` | ProducerSet queries (active_producers_at_height) | ProducerInfo | post_commit, validation, rewards, RPC |
| `crates/core/src/network_params/` | Activation height definitions | Nothing | Everything that gates on heights |

### Data Flows Through Affected Area

```
Block applied
  -> accumulate_block() updates attested_sets[0], blocks_produced, attestation_accum[0]
  -> [per block, persisted in EpochState]

Epoch boundary (height % blocks_per_epoch == 0)
  -> ProducerSet.apply_pending_updates() [flushes deferred mutations]
  -> active_producers_for_scheduling_at_height() [extracts eligible producers]
  -> derive_at_boundary(prev_epoch_state, input) [THE derivation]
       -> attestation filter (attested_union from prev.attested_sets)
       -> deadlock floor (2/3 of effective_active)    <-- BUG: overrides filter
       -> ghost exclusion (denominator adjustment)
       -> tier system (ACTIVE_PRODUCERS_CAP)
       -> accumulator rotation
  -> new EpochState persisted
  -> scheduler uses new producer_list + bond_snapshot for next epoch
```

### Architectural Constraints and Invariants

| Constraint | Why It Exists | What Breaks If Violated |
|-----------|--------------|----------------------|
| `derive_at_boundary()` is a pure function | Determinism guarantee -- same inputs = same outputs on all nodes | Fork: different nodes compute different schedules |
| EpochState is the sole scheduler input | Snap-sync safety -- all nodes have identical EpochState after sync | Fork: snap-synced nodes diverge from full nodes |
| Deferred mutations only at epoch boundary | Prevents scheduler divergence between forks mid-epoch | Fork: nodes on different fork branches see different producer sets |
| 2/3 deadlock floor | Prevents attestation bugs from shrinking schedule to zero | Liveness collapse: too few producers scheduled |
| Activation height gate (INC-I-075) | Rolling deploy safety -- mixed binary versions agree on behavior | Fork: old binary and new binary compute different schedules |

### Blast Radius

**Direct impact:**
- `epoch_state/mod.rs::derive_at_boundary()` -- core change location
- `network_params/mod.rs` + `defaults.rs` -- new activation height field
- `consensus/constants.rs` -- possible new threshold constants
- `apply_block/post_commit.rs` -- may need to pass additional data in EpochDerivationInput

**Indirect impact:**
- `rewards.rs::calculate_expected_epoch_rewards()` -- has its own copy of attestation filter logic; must be kept in sync
- `validation_checks.rs` -- validates blocks against the schedule; schedule change affects what blocks are valid
- `fork_recovery.rs` -- rebuilds EpochState; must handle activation height
- `production/assembly.rs` -- builds blocks using the schedule
- `production/scheduling.rs` -- bootstrap scheduling (separate path, likely unaffected)
- `rpc/methods/producer.rs` -- getProducers response may need new field

### Brittleness Check

| Signal | Applies? | Details |
|--------|----------|---------|
| Cross-module blast radius (3+ modules) | YES | epoch_state, network_params, consensus, post_commit, rewards |
| Invariant gaps | YES | No invariant enforces "only live producers in schedule" |
| Data flow reversal | NO | Data flows in the same direction (blocks -> accumulator -> derivation) |
| Shared mutable state | NO | derive_at_boundary is a pure function |
| Contract absence | PARTIAL | derive_at_boundary's contract doesn't document the floor override behavior |

```
--- BRITTLENESS CHECK ---
Signals detected: 2.5/5
Details: Cross-module blast radius, invariant gap, partial contract absence
Verdict: LOCALIZED (borderline)
---
```

The change is architecturally well-scoped (the seam exists inside derive_at_boundary) but affects multiple downstream consumers of the schedule.

## Impact Analysis

### Existing Code Affected

| File/Module | How Affected | Risk |
|------------|-------------|------|
| `epoch_state/mod.rs:derive_at_boundary()` | Core logic change: new prune step before floor | HIGH -- this is the single consensus derivation function |
| `network_params/mod.rs` | New `epoch_prune_activation_height` field | LOW -- additive, no existing behavior changes |
| `network_params/defaults.rs` | Default values for new field | LOW -- mechanical |
| `consensus/constants.rs` | New threshold constants | LOW -- additive |
| `rewards.rs:calculate_expected_epoch_rewards()` | Has parallel attestation filter logic; must match | MEDIUM -- drift risk (known existing gap, see MEMORY.md) |
| `apply_block/post_commit.rs` | May need to add activation height to EpochDerivationInput | LOW -- already passes network params |
| `fork_recovery.rs` | Rebuilds EpochState at epoch boundaries; must use same activation height | MEDIUM -- subtle interaction |

### What Breaks If This Changes

| Module/Function | What Happens | Mitigation |
|----------------|-------------|-----------|
| `derive_at_boundary()` output changes | Different producer_list -> different scheduler -> blocks from "wrong" producer rejected | Activation height gate ensures all nodes switch atomically |
| Rewards calculation | If rewards.rs doesn't apply the same prune, expected vs actual rewards diverge | Synchronize the filter logic (or unify the code path) |
| Fork recovery epoch rebuild | If recovery doesn't gate on activation height, rebuilt EpochState differs | Pass activation height through EpochDerivationInput |

### Regression Risk Areas

| Area | Why It Might Break |
|------|-------------------|
| Attestation bitfield decode | Prune changes producer_list -> changes bitfield encoding order -> all decoders must agree |
| State root | Different producer_list -> different EpochState hash -> different state root -> snap sync divergence |
| Bond snapshot | If pruned producers are excluded from bond_snapshot, rewards pool calculation changes |

## Traceability Matrix

| Requirement ID | Priority | Test IDs | Architecture Section | Implementation Module |
|---------------|----------|----------|---------------------|---------------------|
| REQ-PRUNE-001 | Must | test_post_activation_prunes_absent_producers | derive_at_boundary | epoch_state/mod.rs |
| REQ-PRUNE-002 | Must | test_pre_activation_floor_is_identical_to_current_behavior, test_post_activation_prunes_absent_producers | network_params | network_params/ |
| REQ-PRUNE-003 | Must | test_pruned_producer_reappears_on_attestation | derive_at_boundary | epoch_state/mod.rs |
| REQ-PRUNE-004 | Must | test_pre_activation_floor_is_identical_to_current_behavior (no version bump verified structurally) | activation gate | network_params/ |
| REQ-PRUNE-005 | Must | test_post_activation_absolute_floor_fires, test_post_activation_zero_attested_uses_fallback | derive_at_boundary | epoch_state/mod.rs |
| REQ-PRUNE-006 | Must | (verified structurally: prune filters input.active_producers, does not modify ProducerSet) | producer set | producer types |
| REQ-PRUNE-007 | Should | (M1 scope: covered by test_post_activation_prunes_absent_producers verifying attested-only list) | scheduler | scheduler.rs |
| REQ-PRUNE-008 | Should | (verified structurally: all prune logic inside derive_at_boundary, no node-local state) | derive_at_boundary | epoch_state/mod.rs |
| REQ-PRUNE-009 | Should | (verified structurally: ghost exclusion code unchanged, prune is a separate gate) | derive_at_boundary | epoch_state/mod.rs |
| REQ-PRUNE-010 | Should | (deferred to M2: tier interaction tested when extraction lands) | derive_at_boundary | epoch_state/mod.rs |
| REQ-PRUNE-011 | Could | (deferred: observability is not part of M1 scope) | post_commit, RPC | post_commit.rs, rpc/ |
| REQ-PRUNE-012 | Could | (verified structurally: threshold = "in attested_union", one sentence) | derive_at_boundary | epoch_state/mod.rs |
| REQ-PRUNE-013 | Won't | N/A | N/A | N/A |
| REQ-PRUNE-014 | Won't | N/A | N/A | N/A |

## Specs Drift Detected

- `bins/node/src/node/mod.rs:157` -- Vestigial comment "Producers excluded from round-robin for missing their slot" is stale. The `excluded_producers` field it describes no longer exists. The comment should be removed (it flows into the `epoch_state` description).

## Assumptions

| # | Assumption (technical) | Explanation (plain language) | Confirmed |
|---|----------------------|---------------------------|-----------|
| 1 | `EpochState.attested_sets` and `blocks_produced` are deterministic across all nodes | These fields are derived from applied blocks, which are identical on all nodes following the same chain | Yes -- verified in code (accumulate_block is pure function of block data) |
| 2 | Snap-synced nodes receive the full EpochState including all 3 attested_sets | The snapshot includes the serialized EpochState | Yes -- post_commit.rs:145 persists after every block |
| 3 | No existing code path depends on absent producers remaining in the schedule | The schedule is consumed for production/validation only; no reward logic depends on absent producers being scheduled | Yes -- rewards are based on blocks_produced, not schedule membership |
| 4 | The `derive_at_boundary()` function signature can accept new fields in EpochDerivationInput without breaking serialization | EpochDerivationInput is a struct passed by reference, not serialized | Yes -- it's a function parameter, not a persisted type |
| 5 | Adding a new field to NetworkParams does not require a protocol version bump | NetworkParams is programmatic (defaults.rs), not wire-serialized | Yes -- per CLAUDE.md: "Change requires new binary on ALL nodes simultaneously" but no version bump |

## Identified Risks

| Risk | Mitigation |
|------|-----------|
| Rewards divergence: `calculate_expected_epoch_rewards()` in rewards.rs has a parallel attestation filter that must match the prune logic | The redesign should document that rewards.rs must be updated in lockstep, or ideally unified with derive_at_boundary |
| Attestation bitfield encoder/decoder order depends on producer_list; pruning changes producer_list | The bitfield encode/decode already handles dynamic producer_list (it uses the frozen epoch list). Prune changes which producers are IN that list, which is the intended effect |
| Adversarial pruning: an attacker partitions honest producers from the network, causing them to be pruned, then controls the schedule | REQ-PRUNE-005 minimum floor prevents total takeover. Additionally, the 3-epoch lookback in attested_union provides inertia |
| Flapping: a producer at the liveness threshold boundary alternates between pruned and included each epoch | Design evaluators should consider hysteresis (e.g., prune threshold != re-inclusion threshold) |
| CURRENT_PROTOCOL_VERSION bump: if the EpochState serialization format changes (new fields), this triggers delete_epoch_state() | The redesign should NOT add new fields to EpochState if possible -- use existing attested_sets/blocks_produced |

## Out of Scope (Won't)

- **Economic penalties for absence** (REQ-PRUNE-013): No slashing, bond reduction, or reward penalty for pruned producers. Pruning is a scheduling optimization.
- **Bond mechanics changes** (REQ-PRUNE-014): Bond stacking, withdrawal, delegation unchanged.
- **Increasing MAX_FALLBACK_RANKS**: While more fallback ranks would reduce dead slot probability, it's an orthogonal change and increases block contention.
- **Changing the bootstrap scheduler**: The genesis/bootstrap scheduling path has its own liveness filter and is unaffected.

## Open Questions for Design Evaluators

1. **Liveness threshold**: What is the minimum liveness signal to avoid pruning? Options:
   - Present in `attested_sets[0]` (attested at least once in the just-completed epoch)
   - Produced >= 1 block in `blocks_produced` in the just-completed epoch
   - Present in `attested_union` (attested in any of the last 3 epochs -- current attestation filter behavior)
   - Some minimum number of attestation minutes in `attestation_accum[0]`

2. **Hysteresis / flapping prevention**: Should the prune threshold differ from the re-inclusion threshold? E.g., prune if `blocks_produced == 0 AND attested_sets[0] == false`, but re-include if `attested_sets[0] == true` (lower bar for re-entry).

3. **Interaction with the 2/3 deadlock floor**: Should the prune REPLACE the current floor behavior, MODIFY it, or LAYER on top?
   - Replace: prune first, then apply the floor to the pruned set
   - Modify: change the floor denominator to use pruned count
   - Layer: prune independently, floor independently, take the larger set

4. **Minimum viable producer count**: What is the absolute floor below which no pruning occurs? (e.g., never prune below 3 producers). This is REQ-PRUNE-005's design parameter.

5. **New-registration grace period**: Should newly registered producers (epoch N) be exempt from pruning at epoch N+1? They may not have had a full epoch to demonstrate liveness. (Analogous to `GHOST_EXCLUSION_GRACE_EPOCHS` but shorter.)

6. **Bond snapshot interaction**: Should pruned producers be excluded from the `bond_snapshot`? If yes, total_bonds decreases, changing all ticket assignments. If no, pruned producers' bonds are "dead weight" in the snapshot but have no ticket ranges (because they're not in producer_list).

7. **Partition attack surface**: If an attacker can isolate honest producers from the network for 1 epoch, they get pruned. How does this interact with the 2/3 floor? What if the attacker controls 34% of bonds -- can they prune the other 66% and take over?

8. **Epoch length sensitivity**: With 360 blocks per epoch (~60 minutes), a producer who goes offline for 1 hour gets pruned. Is this too aggressive? Should the lookback window be longer (e.g., use attested_union across 2-3 epochs instead of just the current epoch)?

9. **Block content vs consensus rules (CLAUDE.md #0b)**: Does this change block CONTENT (what a producer puts INTO a block)? The prune changes `producer_list` which changes attestation bitfield encoding (block header field). This means Q2 from #0b = YES: different `producer_list` -> different `presence_root` -> different block content. This requires either an activation height (already required per Q3=NO) OR synchronized deploy. Since an activation height is already required, this is covered.

10. **Rewards.rs synchronization**: `calculate_expected_epoch_rewards()` has its own parallel attestation filter logic (rewards.rs:883-929). If the prune changes who is in the schedule, does the expected reward calculation need to match? The reward validator computes expected rewards independently -- will it still agree?

## The Stupidest Thing That Could Work (SSF Baseline)

**In `derive_at_boundary()`, after building `attested_union`, filter `input.active_producers` to only those present in `attested_union` OR registered within the last epoch (grace period), and compute the 2/3 floor using the filtered count instead of the full count.**

This is exactly what the attestation filter already does (lines 152-171) EXCEPT the floor denominator currently uses `active_count` (all registered). The fix: change the floor denominator from `active_count` to `attested_union.len()` (or `effective_active` after ghost exclusion). One line change in the floor computation (line 209): `new_list.len() < effective_active * 2 / 3` becomes `new_list.len() < pruned_base * 2 / 3` where `pruned_base` is the attested set size.

This works because the attestation filter already does the pruning -- the bug is that the floor OVERRIDES it by using the wrong denominator.
