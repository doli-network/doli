# Scheduler Comparison: main vs fix/sync-state-explosion-root-causes

## Divergence point: 103fc31a

---

## Question 1: Is the claim of "mutually exclusive algorithms" accurate?

**YES, the claim is accurate.** The two branches implement fundamentally different scheduling algorithms for post-genesis (epoch) block production and validation.

### Main branch: Flat round-robin with excluded_producers filter

**Production** (`scheduling.rs:resolve_epoch_eligibility`):
```
slot % len(sorted_active - excluded_producers) → single producer
```
- Ignores bond weights entirely (weights are present but unused in selection)
- Filters out producers via `excluded_producers: HashSet<PublicKey>` (local Node field)
- Returns exactly 1 producer per slot (no fallback ranks)
- Deadlock safety: if all excluded, falls back to full list

**Validation** (`validation/producer.rs:validate_producer_eligibility`):
```
slot % len(sorted_active - excluded_producers) → expected producer
```
- Same flat round-robin algorithm
- Receives `excluded_producers` via `ValidationContext.with_excluded_producers()`
- Single expected producer per slot

### Source branch: Bond-weighted DeterministicScheduler with ticket ranges

**Production** (`scheduling.rs:resolve_epoch_eligibility`):
```
DeterministicScheduler.select_producer(slot, rank) → up to max_fallback_ranks producers
```
- Bond weights are USED: each bond = 1 ticket, `slot % total_tickets` selects producer from cumulative ticket ranges
- Cached per epoch (invalidated on epoch/count/bond changes)
- Returns multiple eligible producers (primary + fallback ranks)
- Deduplication across ranks

**Validation** (`validation/producer.rs:validate_producer_eligibility`):
```
select_producer_for_slot(slot, producers_with_bonds) → eligible list
```
- Same bond-weighted ticket algorithm (the standalone function, not DeterministicScheduler struct, but equivalent logic)
- Uses time-window eligibility: `is_producer_eligible_ms()` with 2s exclusive windows per rank
- Checks both [offset*1000, offset*1000+999] to handle second-precision timestamps

**These algorithms produce DIFFERENT results for the same inputs.** A chain produced by one cannot be validated by the other (except in degenerate cases where all producers have equal bonds and no exclusions).

---

## Question 2: Does main truly replace the DeterministicScheduler with round-robin?

**YES.**

Evidence:
1. `scheduling.rs` on main: the entire `DeterministicScheduler` usage block (epoch cache lookup, `ScheduledProducer::new`, `scheduler.select_producer()`, rank iteration with dedup) is **deleted** and replaced with `slot % sorted.len()` after filtering `excluded_producers`.

2. The `cached_scheduler` field still exists on `Node` (`mod.rs:154`) and is still set to `None` on invalidation events (rollback, epoch boundary, reorg), but it is **never rebuilt or read** in the production path. This is dead code.

3. `select_producer_for_slot()` still exists in `consensus/selection.rs` but is annotated `#[deprecated(note = "Use DeterministicScheduler::select_producer() for consensus-critical code")]`. Ironically, main uses neither -- it uses its own inline round-robin. This deprecated function is not called from any production or validation hot path on main.

4. Validation on main (`validation/producer.rs`) uses the same flat round-robin: `slot % sorted.len()` with `excluded_producers` filtering. It does NOT call `select_producer_for_slot()` or use time-window eligibility.

---

## Question 3: Does source keep the DeterministicScheduler and add genesis hardcoded producers?

**YES to both.**

### DeterministicScheduler retained
Source branch's `resolve_epoch_eligibility` still builds and caches a `DeterministicScheduler`:
- Keyed by `(epoch, active_count, total_bonds)`
- Creates `ScheduledProducer` entries from `(pk, bonds)` tuples
- Calls `scheduler.select_producer(current_slot, rank)` for each rank up to `max_fallback_ranks`
- Deduplicates across ranks

### Genesis hardcoded producers added
Source branch adds a new code path at the top of `resolve_bootstrap_eligibility` for genesis mode:
- Testnet: calls `doli_core::genesis::testnet_genesis_producers()`
- Mainnet: calls `doli_core::genesis::mainnet_genesis_producers()`
- These return hardcoded `(PublicKey, _)` pairs, identical on all nodes
- Motivation (from comments): "The on-chain ProducerSet may be empty and the GSet has DIFFERENT contents on different nodes (anti-entropy hasn't converged). Both cause divergent schedulers -> competing blocks -> forks."
- Devnet falls through to on-chain/GSet/known_producers chain

### Additional source branch changes
- `last_producer_list_change` changed from `Option<Instant>` to `RwLock<Option<Instant>>` (concurrency fix)
- Producer weights derived from `scheduled_producers_at_height()` with `scheduled` flag, weight = `1u64` per producer
- Self-produced blocks use `ValidationMode::Full` (not `Light`)
- Removed `seen_blocks_for_slot` gossip check from early block existence check
- Removed propagation delay logic (1s wait after eligibility)

---

## Question 4: What is the epoch_bond_snapshot on main and does it affect compatibility?

### What it is
Main introduces `epoch_bond_snapshot: HashMap<Hash, u64>` and `epoch_bond_snapshot_epoch: u64` on the Node struct. The `bond_weights_for_scheduling()` method:
1. If snapshot is empty (first epoch): falls back to UTXO-derived bond counts (same as old code)
2. If snapshot exists: uses the snapshot as the bond weight source

The snapshot is presumably populated at epoch boundaries (I did not trace the population path, as it was not in the requested diffs).

### Compatibility impact
**It does not affect compatibility between branches in a meaningful way** because main's `resolve_epoch_eligibility` ignores the bond weights entirely. The round-robin uses `slot % len(sorted)` -- the weights are passed in but only the pubkeys are extracted. The weights could be 1, 100, or 999 -- the result is the same.

However, the epoch_bond_snapshot IS used in the scheduler fingerprint debug logging on main (line 252: `self.epoch_bond_snapshot_epoch`), suggesting it was intended for future use or for the bootstrap path.

**Summary**: On main, bond weights are computed but thrown away by the epoch scheduler. On source, bond weights are the core of the scheduling algorithm.

---

## Question 5: Which approach is safer for a PoS chain?

This question requires factual analysis, not opinion. Here are the properties of each:

### Main (flat round-robin with exclusion)

**Properties:**
- Equal scheduling regardless of stake -- a producer with 1 bond gets the same slots as one with 100 bonds
- Slot assignment: `slot % N` where N = active non-excluded producers
- Exclusion tracking via local `HashSet` on each node

**Determinism risks:**
- `excluded_producers` is a local `HashSet<PublicKey>` field. It is populated in `apply_block/post_commit.rs` by detecting missed slots and cleared on rollback. If two nodes have different chain tips (even briefly during reorgs), they have different `excluded_producers` sets, producing different `N` values, producing different `slot % N` results.
- This is the exact same class of bug that the source branch's comments describe: "producer_liveness is LOCAL state -- different on each node because they've applied different blocks."
- The deadlock safety fallback (all excluded -> full list) changes `N` non-deterministically across nodes.

**Fork-choice impact:**
- Returns exactly 1 eligible producer per slot (no fallback). If that producer is offline, the slot is empty. This simplifies fork choice but reduces liveness.

### Source (bond-weighted DeterministicScheduler)

**Properties:**
- Scheduling proportional to stake -- more bonds = more tickets = more slots
- Slot assignment: `slot % total_tickets` maps into cumulative ticket ranges
- Exclusion via `scheduled` flag on `ProducerInfo` (on-chain state in ProducerSet)

**Determinism risks:**
- The `scheduled` flag is on `ProducerInfo` which is part of the `ProducerSet` (persisted state). Modified via `unschedule_producer`/`schedule_producer` in `apply_block`. If all nodes apply the same blocks, they have the same `scheduled` flags. However, during reorgs, the `scheduled` flags must be correctly rolled back -- if ProducerSet rollback does not restore `scheduled` flags, divergence occurs.
- The `DeterministicScheduler` is cached by `(epoch, active_count, total_bonds)`. If a rollback changes `active_count` but not `total_bonds` (or vice versa), the stale cache could persist.
- `select_producer_for_slot` (used in validation) sorts by pubkey and uses the same ticket algorithm -- this is deterministic given the same inputs.

**Fork-choice impact:**
- Returns multiple eligible producers with fallback ranks. Uses 2s exclusive time windows. More complex but higher liveness (if rank 0 misses, rank 1 produces after 2s delay).

### Critical observation

**Both branches have the same fundamental vulnerability to exclusion-state divergence during reorgs.** Main tracks exclusions in a local HashSet; source tracks them in ProducerSet's `scheduled` flag. Neither approach is inherently safe -- both require correct rollback behavior to maintain cross-node determinism.

**The bond-weighted approach (source) aligns with standard PoS design** where stake influences scheduling probability. The flat round-robin (main) is a simpler model but violates the PoS principle that more stake = more responsibility = more slots.

**The time-window fallback (source) improves liveness** by allowing rank-1 producers to step in after 2 seconds. Main's single-producer-per-slot design has no fallback mechanism -- missed slots are simply empty until the exclusion filter kicks in.

---

## Summary Table

| Aspect | Main | Source |
|--------|------|--------|
| Epoch scheduler algorithm | `slot % N` (flat round-robin) | `slot % total_tickets` (bond-weighted) |
| Bond weight usage | Computed but IGNORED in selection | Core selection mechanism |
| Exclusion tracking | Local `HashSet<PublicKey>` on Node | `scheduled: bool` on `ProducerInfo` (ProducerSet) |
| Fallback ranks | None (1 producer per slot) | Up to `max_fallback_ranks` with 2s time windows |
| Validation alignment | Round-robin matches production | `select_producer_for_slot` + time windows matches production |
| Genesis handling | Same as base (GSet/on-chain fallback) | Hardcoded genesis producers (deterministic) |
| `DeterministicScheduler` | Dead code (field exists, never used) | Active (cached per epoch) |
| `select_producer_for_slot` | Deprecated, unused | Active in validation path |
| Self-block validation mode | `ValidationMode::Light` | `ValidationMode::Full` |
| New Node fields | `excluded_producers`, `epoch_bond_snapshot`, `epoch_bond_snapshot_epoch` | `scheduled` flag on ProducerInfo |
| Cross-node determinism risk | `excluded_producers` diverges during reorgs | `scheduled` flag must be correctly rolled back |
