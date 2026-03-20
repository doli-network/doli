# Architecture: Single-Proposer-Per-Slot Migration

## Scope

Migration from multi-rank fallback producer selection (5 ranks, 2s windows) to single-proposer-per-slot with attestation-based fork choice. Three phases: (1) single proposer, (2) attestation fork choice, (3) graceful empty slots.

## Overview

```
CURRENT (v1):
  Slot S → scheduler returns [P0, P1, P2, P3, P4]
  0-2s: P0 can produce  |  2-4s: P1 can produce  |  4-6s: P2  |  6-8s: P3  |  8-10s: P4
  Result: up to 5 competing blocks → weight-based fork choice → fragmentation at scale

PROPOSED (v2):
  Slot S → scheduler returns [P0]   (single proposer)
  0-10s: P0 produces or slot is empty
  Fork choice: attestation weight from presence_root bitfields
  Emergency: after 10 consecutive empty slots, fallback producers activated
```

## Protocol Version Gating

All changes are gated behind `is_protocol_active(2, state.active_protocol_version)`. This is the existing protocol activation mechanism (3/5 maintainer multisig via `ProtocolActivation` transaction).

**Transition strategy**: Deploy new binary to all nodes first. Then activate protocol v2 via maintainer transaction. All nodes switch atomically at the activation block height.

**Backward compatibility**: v1 nodes will continue to produce rank 1-4 blocks. v2 nodes will:
- Accept rank-0 blocks from v1 nodes (valid under both protocols)
- Reject rank 1-4 blocks from v1 nodes (invalid under v2)
- This is a **soft fork**: v2 rules are strictly more restrictive than v1

## Milestones

| ID | Name | Scope (Modules) | Scope (Requirements) | Est. Size | Dependencies |
|----|------|-----------------|---------------------|-----------|-------------|
| M1 | Single Proposer Selection | consensus/selection, consensus/constants, scheduler, network_params | REQ-SP-001 to REQ-SP-007, REQ-SP-009 | L | None |
| M2 | Attestation Fork Choice | finality, sync/reorg, block_handling | REQ-SP-008, REQ-SP-011 | M | M1 |
| M3 | Cleanup and Metrics | constants (deprecated removal), metrics | REQ-SP-010, REQ-SP-012, REQ-SP-013, REQ-SP-014 | S | M1, M2 |

---

## Modules

### Module 1: Consensus Selection (Phase 1 — Single Proposer)

- **Responsibility**: Select exactly one proposer per slot when protocol v2 is active
- **Public interface**: Modified `select_producer_for_slot()`, `is_producer_eligible_ms()`, `DeterministicScheduler::select_producer()`
- **Dependencies**: `consensus/constants`, `network_params`, `chain_state.active_protocol_version`
- **Implementation order**: 1

#### Exact Changes

**File: `crates/core/src/consensus/constants.rs`**

Add new constant for emergency fallback threshold:

```rust
/// Consecutive empty slots before emergency fallback activates (protocol v2+).
/// 10 empty slots = 100 seconds of no blocks. This is much more tolerant than
/// v1's 3-slot equalization, since single-proposer means empty slots are expected.
pub const EMERGENCY_FALLBACK_SLOTS: u64 = 10;

/// Protocol version that activates single-proposer mode.
pub const SINGLE_PROPOSER_PROTOCOL_VERSION: u32 = 2;
```

**File: `crates/core/src/consensus/selection.rs`**

Modify `select_producer_for_slot()` to accept a protocol version parameter:

```rust
/// Select producers for a slot.
///
/// Protocol v1: Returns up to MAX_FALLBACK_PRODUCERS with evenly-distributed offsets.
/// Protocol v2+: Returns exactly 1 producer (the primary). No fallbacks.
pub fn select_producer_for_slot(
    slot: Slot,
    producers_with_bonds: &[(crypto::PublicKey, u64)],
) -> Vec<crypto::PublicKey> {
    // ... existing code unchanged (v1 behavior)
}

/// Select the single proposer for a slot (protocol v2+).
///
/// Returns exactly one producer: slot % total_tickets mapped to the ticket holder.
/// No fallback producers. If this producer doesn't produce, the slot is empty.
pub fn select_single_proposer(
    slot: Slot,
    producers_with_bonds: &[(crypto::PublicKey, u64)],
) -> Option<crypto::PublicKey> {
    if producers_with_bonds.is_empty() {
        return None;
    }
    let mut sorted: Vec<_> = producers_with_bonds.to_vec();
    sorted.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

    let total_tickets: u64 = sorted.iter().map(|(_, bonds)| (*bonds).max(1)).sum();
    if total_tickets == 0 {
        return None;
    }

    let ticket = (slot as u64) % total_tickets;
    let mut cumulative: u64 = 0;
    for (pk, bonds) in &sorted {
        let tickets = (*bonds).max(1);
        if ticket < cumulative + tickets {
            return Some(*pk);
        }
        cumulative += tickets;
    }
    None
}
```

Modify eligibility check for v2:

```rust
/// Check if a producer is eligible for a slot (protocol v2+).
///
/// In v2, only the single proposer is eligible, and they are eligible
/// for the entire slot duration (no time windows).
pub fn is_single_proposer_eligible(
    producer: &crypto::PublicKey,
    proposer: &crypto::PublicKey,
) -> bool {
    producer == proposer
}
```

**File: `crates/core/src/scheduler.rs`**

Add single-proposer method to `DeterministicScheduler`:

```rust
impl DeterministicScheduler {
    /// Select the single proposer for a slot (protocol v2+).
    /// Returns only rank 0, ignoring all fallback ranks.
    pub fn select_single_proposer(&self, slot: Slot) -> Option<&PublicKey> {
        self.select_producer(slot, 0)
    }
}
```

**File: `bins/node/src/node/production/scheduling.rs`**

In `resolve_epoch_eligibility()` (line 408-489), gate on protocol version:

```rust
pub(super) fn resolve_epoch_eligibility(
    &mut self,
    current_slot: u32,
    height: u64,
    active_with_weights: &[(PublicKey, u64)],
) -> Vec<PublicKey> {
    // ... existing liveness filter code ...

    // Build scheduler (existing code)
    let scheduler = /* ... existing scheduler build/cache ... */;

    // PROTOCOL V2: Single proposer — no fallback ranks
    let protocol_version = self.chain_state_version(); // helper to read active_protocol_version
    if consensus::is_protocol_active(
        consensus::SINGLE_PROPOSER_PROTOCOL_VERSION,
        protocol_version,
    ) {
        // Single proposer: only rank 0
        if let Some(pk) = scheduler.select_single_proposer(current_slot) {
            return vec![pk.clone()];
        }
        return Vec::new();
    }

    // V1: existing multi-rank code
    let mut eligible: Vec<PublicKey> = Vec::with_capacity(self.config.network.params().max_fallback_ranks);
    for rank in 0..self.config.network.params().max_fallback_ranks {
        if let Some(pk) = scheduler.select_producer(current_slot, rank).cloned() {
            if !eligible.contains(&pk) {
                eligible.push(pk);
            }
        }
    }
    eligible
}
```

**File: `bins/node/src/node/production/mod.rs`**

In `try_produce_block()`:

1. **Remove emergency equalization for v2** (lines 237-284). In v2, the `slot_gap > 3` emergency equalization is disabled. Instead, the emergency fallback mechanism (REQ-SP-006) uses `EMERGENCY_FALLBACK_SLOTS`:

```rust
// V2: Emergency fallback after EMERGENCY_FALLBACK_SLOTS consecutive empty slots
let protocol_v2 = consensus::is_protocol_active(
    consensus::SINGLE_PROPOSER_PROTOCOL_VERSION,
    self.chain_state_version(),
);

let weights_for_scheduler = if protocol_v2 {
    if slot_gap > consensus::EMERGENCY_FALLBACK_SLOTS {
        // Emergency: allow top-3 live producers at equal weight
        warn!("Emergency fallback: {} consecutive empty slots", slot_gap);
        let emergency: Vec<(PublicKey, u64)> = active_with_weights
            .iter()
            .filter_map(|(pk, _)| {
                let live = self.producer_liveness.get(pk)
                    .map(|&h| height.saturating_sub(h) < 50)
                    .unwrap_or(false);
                if live { Some((*pk, 1u64)) } else { None }
            })
            .take(3)
            .collect();
        if emergency.is_empty() {
            active_with_weights.iter().map(|(pk, _)| (*pk, 1u64)).collect()
        } else {
            emergency
        }
    } else {
        active_with_weights.clone()
    }
} else {
    // V1: existing 3-slot emergency equalization
    // ... existing code ...
};
```

2. **Remove time-window eligibility check for v2**. In v2, if we are the single proposer, we can produce immediately at slot start (no 2s window waiting):

```rust
let is_eligible = if let Some(score) = our_bootstrap_rank {
    // Bootstrap mode: unchanged
    // ...
} else if protocol_v2 {
    // V2: we're eligible if we're the single proposer (already filtered by resolve_epoch_eligibility)
    eligible.contains(&our_pubkey)
} else {
    // V1: ms-precision sequential eligibility check
    consensus::is_producer_eligible_ms(&our_pubkey, &eligible, slot_offset_ms)
};
```

3. **Produce at slot start in v2**. No need to wait for fallback window. The proposer can start VDF immediately when the slot begins, reducing latency:

```rust
if protocol_v2 && eligible.len() == 1 && eligible[0] == our_pubkey {
    // We are THE proposer. Produce immediately at slot start.
    // No fallback timing needed.
}
```

**File: `bins/node/src/node/validation_checks.rs`**

In `check_producer_eligibility()`:

1. Gate the emergency equalization on protocol version (lines 57-88):

```rust
let protocol_v2 = consensus::is_protocol_active(
    consensus::SINGLE_PROPOSER_PROTOCOL_VERSION,
    self.chain_state.read().await.active_protocol_version,
);

let weighted: Vec<(PublicKey, u64)> = if !protocol_v2 && slot_gap > 3 /* ... */ {
    // V1 emergency equalization (existing code)
} else if protocol_v2 && slot_gap > consensus::EMERGENCY_FALLBACK_SLOTS {
    // V2 emergency fallback (same logic as production side)
} else {
    active_with_weights
};
```

**File: `crates/core/src/validation/producer.rs`**

In `validate_producer_eligibility()` (line 200-252):

```rust
pub fn validate_producer_eligibility(
    header: &BlockHeader,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    if ctx.network.is_in_genesis(ctx.current_height) {
        return validate_bootstrap_producer(header, ctx);
    }

    if !ctx.active_producers_weighted.is_empty() {
        // PROTOCOL V2: Single proposer validation
        if is_protocol_active(SINGLE_PROPOSER_PROTOCOL_VERSION, ctx.protocol_version) {
            let proposer = select_single_proposer(header.slot, &ctx.active_producers_weighted);
            match proposer {
                Some(pk) if pk == header.producer => return Ok(()),
                Some(_) => {
                    // Check emergency fallback (slot gap from prev_slot)
                    let slot_gap = (header.slot as u64).saturating_sub(ctx.prev_slot as u64);
                    if slot_gap > EMERGENCY_FALLBACK_SLOTS {
                        // During emergency: accept any active producer
                        if ctx.active_producers_weighted.iter().any(|(pk, _)| *pk == header.producer) {
                            return Ok(());
                        }
                    }
                    return Err(ValidationError::InvalidProducer);
                }
                None => return Err(ValidationError::InvalidProducer),
            }
        }

        // V1: existing multi-rank validation
        let eligible = select_producer_for_slot(header.slot, &ctx.active_producers_weighted);
        // ... existing time-window check ...
    }
    // ... rest of existing code ...
}
```

The `ValidationContext` needs a new field:

```rust
pub struct ValidationContext {
    // ... existing fields ...
    pub protocol_version: u32,  // NEW: from chain_state.active_protocol_version
}
```

#### Failure Modes

| Failure | Cause | Detection | Recovery | Impact |
|---------|-------|-----------|----------|--------|
| Proposer offline | Node crash, network partition | Slot expires with no block (10s timeout) | Next slot's proposer produces. Emergency fallback after 10 empty slots. | 1 empty slot (10s delay). Normal behavior. |
| Protocol version mismatch | Partial binary upgrade | v2 node rejects rank 1-4 blocks from v1 node | Deploy all nodes before activating v2. v1 blocks from rank 0 are valid under both versions. | Fork between v1 and v2 nodes if activated prematurely. Resolved by upgrade. |
| Emergency fallback abuse | Attacker with majority weight offline to trigger fallback | Monitor consecutive empty slots. Fallback requires 10+ empty slots. | Fallback uses equal weight, preventing single-entity dominance. | 100+ seconds of degraded chain progress. |
| Stale proposer | Proposer is registered but hasn't produced in liveness window | Liveness filter already in place. Stale producers removed from scheduler. | Re-entry mechanism (REENTRY_INTERVAL) gives stale producers periodic chances. | No impact -- already handled by liveness system. |

#### Security Considerations

- **Trust boundary**: The protocol version is set by on-chain maintainer consensus (3/5 multisig). No single entity can activate v2.
- **Sensitive data**: No new sensitive data introduced.
- **Attack surface**: Single proposer means an attacker who controls the proposer for a slot can censor transactions for 10 seconds. This is identical to Ethereum and is mitigated by the fact that proposer assignment rotates every slot.
- **Mitigations**: Emergency fallback prevents extended censorship (>100s). Inactivity leak reduces bond weight of persistently offline producers, reducing their future slot allocation.

#### Performance Budget

- **Selection latency**: O(log n) binary search in ticket space (existing `DeterministicScheduler`). Target: < 1us for 500 producers.
- **Validation latency**: Reduced from 5 comparisons (rank 0-4) to 1 comparison (single proposer). Net improvement.
- **Block propagation**: Improved -- no competing blocks means gossip carries only 1 block per slot, reducing bandwidth by up to 80%.
- **Memory**: No change -- scheduler cache is already O(n) where n = producer count.

---

### Module 2: Attestation Fork Choice (Phase 2)

- **Responsibility**: Replace weight-based fork choice with attestation-weighted fork choice
- **Public interface**: Modified `ReorgHandler`, `FinalityTracker` integration in fork choice
- **Dependencies**: Module 1 (single proposer), `attestation.rs`, `finality.rs`
- **Implementation order**: 2

#### Exact Changes

**File: `crates/network/src/sync/reorg.rs`**

The current `ReorgHandler` tracks `accumulated_weight` per block based on the producer's `effective_weight`. This must change to use attestation weight.

Add attestation weight to `BlockWeight`:

```rust
pub struct BlockWeight {
    pub prev_hash: Hash,
    pub producer_weight: u64,       // kept for v1 compatibility
    pub accumulated_weight: u64,    // v1: producer weight chain. v2: attestation weight chain.
    pub height: u64,
    pub attestation_weight: u64,    // NEW: sum of attesting producer bond weights
}
```

Add new method `record_block_with_attestation_weight()`:

```rust
/// Record a block with its attestation weight (protocol v2+).
///
/// In v2 fork choice, the "weight" of a chain is the sum of attestation
/// weights, not producer weights. A block's attestation weight comes from
/// the presence_root bitfield in the NEXT block (which commits attestations
/// for the previous block).
pub fn record_block_with_attestation_weight(
    &mut self,
    hash: Hash,
    prev_hash: Hash,
    attestation_weight: u64,
) {
    // Attestation weight is recorded when the NEXT block arrives
    // (because that block's presence_root encodes who attested the previous block)
    self.record_block_internal(hash, prev_hash, attestation_weight, true);
}
```

Modify `handle_new_block_weighted()` to use attestation weight when available:

```rust
/// Handle a new fork block with protocol-aware weight comparison.
///
/// Protocol v1: Uses producer bond weight.
/// Protocol v2+: Uses attestation weight from presence_root bitfields.
pub fn handle_new_block_attestation_weighted(
    &mut self,
    block: Block,
    attestation_weight: u64,
) -> Option<ReorgResult> {
    // Same logic as handle_new_block_weighted but uses attestation_weight
    // for accumulated chain weight comparison
}
```

**File: `bins/node/src/node/block_handling.rs`**

In `handle_new_block()` (line 56-71), change the weight source:

```rust
// Current (v1):
let producer_weight = producers.get_by_pubkey(&block.header.producer)
    .map(|p| p.effective_weight(current_height + 1))
    .unwrap_or(1);
let reorg_result = self.sync_manager.write().await
    .handle_new_block_weighted(block.clone(), producer_weight);

// New (v2):
let protocol_v2 = self.chain_state.read().await.active_protocol_version >= 2;
let reorg_result = if protocol_v2 {
    // Decode attestation bitfield from the block's presence_root
    let producers = self.producer_set.read().await;
    let active_count = producers.active_count();
    let attested_indices = attestation::decode_attestation_bitfield(
        &block.header.presence_root, active_count
    );
    let attestation_weight: u64 = attested_indices.iter()
        .filter_map(|&idx| {
            producers.producer_at_index(idx)
                .map(|p| p.effective_weight(current_height + 1) as u64)
        })
        .sum();
    drop(producers);
    self.sync_manager.write().await
        .handle_new_block_attestation_weighted(block.clone(), attestation_weight)
} else {
    // V1: existing producer weight
    let producer_weight = /* existing code */;
    self.sync_manager.write().await
        .handle_new_block_weighted(block.clone(), producer_weight)
};
```

In `execute_fork_sync_reorg()` (lines 586-612), similarly gate the weight computation:

```rust
// V2: Use attestation weight for chain comparison
let new_chain_weight: i64 = if protocol_v2 {
    result.canonical_blocks.iter()
        .map(|b| {
            let active_count = producers.active_count();
            let attested = attestation::decode_attestation_bitfield(
                &b.header.presence_root, active_count
            );
            attested.len() as i64 // Each attestation = 1 vote (simplified)
        })
        .sum()
} else {
    // V1: existing producer weight code
};
```

**How attestation weight flows**:

1. Block B(N) is produced by proposer P at slot S
2. Other producers receive B(N) and attest it via gossip (existing attestation system)
3. Block B(N+1) producer collects attestations and encodes them in `presence_root` bitfield
4. When B(N+1) arrives, the attestation weight for B(N) is decoded from B(N+1)'s `presence_root`
5. Fork choice uses this attestation weight to compare competing chains

This means attestation weight is **one block delayed** -- which is exactly how it works in Ethereum (block N's attestations are included in block N+1).

#### Failure Modes

| Failure | Cause | Detection | Recovery | Impact |
|---------|-------|-----------|----------|--------|
| Low attestation participation | Many producers offline | Attestation weight < 67% of total | Chain continues with lower finality confidence. Finality tracker delays checkpoint. | Blocks take longer to finalize but chain doesn't halt. |
| Attestation forgery | Malicious presence_root | Ed25519/BLS signature verification on attestations | Reject blocks with invalid attestation bitfields. | Invalid blocks rejected by honest nodes. |
| Fork with split attestations | Network partition | Each side accumulates attestation weight independently | When partition heals, the side with more attestation weight wins. | Resolved naturally by fork choice when partition heals. |

#### Security Considerations

- **Trust boundary**: Attestation data comes from gossip (untrusted). Each attestation is Ed25519-signed and optionally BLS-signed.
- **Attack surface**: An attacker could create blocks with fabricated `presence_root` bitfields. Mitigation: aggregate BLS signature in block body (`aggregate_bls_signature` field) proves the attestations are genuine.
- **Mitigations**: Verify BLS aggregate signature before trusting attestation bitfield for fork choice. For the initial implementation, fall back to producer bond weight if BLS verification fails.

#### Performance Budget

- **Fork choice latency**: O(p) where p = producer count (decode bitfield + sum weights). Target: < 1ms for 256 producers.
- **Memory**: Additional `attestation_weight` field per tracked block in ReorgHandler. ~8 bytes per block. Negligible.
- **Bandwidth**: No change -- attestation bitfield is already in the block header's `presence_root`.

---

### Module 3: Graceful Empty Slots (Phase 1, integrated)

- **Responsibility**: Handle empty slots without triggering emergency recovery
- **Public interface**: Modified slot advancement logic, epoch boundary calculation
- **Dependencies**: Module 1 (single proposer)
- **Implementation order**: Implemented as part of Module 1

#### Exact Changes

**Key insight**: DOLI already tracks epochs by block count, not slot count (`BLOCKS_PER_REWARD_EPOCH = 360`). Empty slots do NOT affect epoch boundaries. This is already correct.

**What changes**:

1. **Remove the `slot_gap > 3` emergency equalization** in production (Module 1 change). In v2, empty slots are normal and don't trigger emergency mode until `slot_gap > EMERGENCY_FALLBACK_SLOTS (10)`.

2. **Slot gap logging**: Change log level from `warn!` to `debug!` for small gaps (1-5 slots). Only `warn!` for gaps > 5 slots, and `error!` for gaps > EMERGENCY_FALLBACK_SLOTS.

3. **Solo production watchdog**: The existing solo production circuit breaker should be aware that consecutive empty slots don't mean we're isolated -- they mean the proposer was offline. Only trigger if we ourselves produce N consecutive blocks with no gossip blocks arriving at all.

4. **Chain stall detection**: The existing stale chain detector uses slot-height gap. In v2, a large slot-height gap is normal (empty slots accumulate). The stale chain detector should use **peer height comparison** instead of slot-height gap. This is already partially the case (the defense-in-depth check at production/mod.rs lines 94-138 compares heights, not slots).

#### Failure Modes

| Failure | Cause | Detection | Recovery | Impact |
|---------|-------|-----------|----------|--------|
| Extended empty slots (>10) | Multiple proposers offline in sequence | Slot gap counter exceeds EMERGENCY_FALLBACK_SLOTS | Emergency fallback activates: top-3 live producers can fill slot | Temporarily degraded to multi-producer mode. Self-corrects when normal proposers come back online. |
| Epoch reward timing shift | Many empty slots delay block production | Epochs defined by block count, not time. No timing shift. | None needed. | Epoch calendar shifts in wall-clock time but rewards are correct per-block. |

---

## Failure Modes (system-level)

| Scenario | Affected Modules | Detection | Recovery Strategy | Degraded Behavior |
|----------|-----------------|-----------|-------------------|-------------------|
| Partial v2 activation | All | v1 nodes produce rank 1-4 blocks that v2 rejects | Ensure all nodes on v2 binary before activation | Temporary fork until all nodes upgraded |
| Network partition | Fork choice | Height/slot divergence between peer groups | Attestation-weighted fork choice resolves when partition heals | Each partition operates independently; heavier-attested side wins |
| Majority proposers offline | Production | 10+ consecutive empty slots | Emergency fallback activates | Chain progresses slowly with emergency producers |
| All proposers offline | Production | No blocks for extended period | Manual intervention: restart nodes | Chain halted (same as current behavior) |

## Security Model

### Trust Boundaries

- **Protocol version**: Trusted (set by 3/5 maintainer multisig, verified on-chain)
- **Proposer selection**: Trusted (deterministic from on-chain bond data, no external input)
- **Attestation data**: Semi-trusted (signed by Ed25519/BLS, but bitfield in presence_root must be verified against aggregate signature)
- **Peer block data**: Untrusted (validated before applying)

### Data Classification

| Data | Classification | Storage | Access Control |
|------|---------------|---------|---------------|
| Protocol version | Public | ChainState (on-chain) | Read by all nodes, set by maintainer multisig |
| Proposer schedule | Public | Derived from on-chain bonds | Deterministic, verifiable by any node |
| Attestation bitfield | Public | Block header (presence_root) | Set by block producer, verified by validators |
| BLS aggregate signature | Public | Block body | Set by block producer, verified by validators |

### Attack Surface

- **Proposer censorship (10s window)**: Single proposer can censor transactions for 1 slot. Mitigated by slot rotation (each slot has a different proposer). Risk: LOW.
- **Emergency fallback abuse**: Attacker who controls >50% of bond weight goes offline to trigger emergency fallback, then uses the emergency mode to produce without competition. Mitigated: emergency mode gives equal weight to all live producers, not bond-weighted. Risk: LOW.
- **Attestation withholding**: Producers refuse to attest to manipulate fork choice. Mitigated: attestation rate affects epoch reward qualification (existing system). Non-attesting producers lose rewards. Risk: MEDIUM.

## Graceful Degradation

| Dependency | Normal Behavior | Degraded Behavior | User Impact |
|-----------|----------------|-------------------|-------------|
| Proposer availability | 1 block per slot (10s) | Empty slots (10s gaps) | Transactions delayed by 1+ slots |
| Attestation participation | >67% attestation weight | <67% attestation weight | Blocks take longer to finalize |
| Emergency fallback | Not active | Active (>10 empty slots) | Multiple producers competing (v1-like behavior) |

## Performance Budgets

| Operation | Latency (p50) | Latency (p99) | Memory | Notes |
|-----------|---------------|---------------|--------|-------|
| Proposer selection | < 1us | < 5us | O(n) scheduler cache | Binary search in ticket space |
| Attestation decode | < 100us | < 500us | 32 bytes per block | Bitfield decode + weight sum |
| Fork choice comparison | < 1ms | < 5ms | O(depth) per chain | Attestation weight accumulation |
| Block validation (v2) | 10% faster than v1 | 15% faster than v1 | No change | Fewer eligibility checks |

## Data Flow

```
Slot S begins
  └─ Proposer P (only 1) selected by scheduler
       ├─ P online → produces block B(S)
       │    ├─ B(S) broadcast via gossip
       │    ├─ Other producers receive B(S)
       │    ├─ Other producers attest B(S) via ATTESTATION_TOPIC gossip
       │    ├─ Attestations collected by next proposer P' for slot S+1
       │    └─ P' encodes attestation bitfield in B(S+1).presence_root
       │
       └─ P offline → slot S is empty
            ├─ No block produced or broadcast
            ├─ All nodes observe slot S passes with no block
            ├─ Slot S+1 begins, new proposer P' produces B(S+1)
            │   └─ B(S+1).prev_hash = B(S-1).hash (skips empty slot)
            └─ If 10+ consecutive empty → emergency fallback
```

## Design Decisions

| Decision | Alternatives Considered | Justification |
|----------|------------------------|---------------|
| Protocol v2 soft fork (not hard fork) | Hard fork at specific height | Soft fork is safer: v1 rank-0 blocks are valid under both versions. Old nodes can still follow the chain (they just see some blocks rejected). |
| Emergency fallback at 10 slots (not 3) | 3 slots (current), 5 slots, 20 slots | 10 slots (100s) is tolerant enough for normal producer churn but fast enough to recover from extended outages. 3 was too aggressive for single-proposer (every offline producer causes empty slot). |
| Attestation-weighted fork choice (not pure longest chain) | Longest chain, heaviest producer chain, GHOST | Attestation weight reflects network consensus better than producer weight. A chain attested by 80% of validators is more trustworthy than one produced by high-weight producers but with few attestations. |
| New function `select_single_proposer()` (not modifying existing) | Modify `select_producer_for_slot()` in place | Keeps v1 path untouched for backward compatibility. Reduces risk of breaking existing behavior during transition. |
| Equal weight in emergency fallback (not bond-weighted) | Bond-weighted emergency | Equal weight prevents a whale from using emergency fallback to monopolize production. During emergency, fairness matters more than weight. |

## External Dependencies

No new external dependencies. All changes use existing crates:
- `crypto` -- Ed25519, BLS, hash functions (existing)
- `bincode` -- serialization (existing)
- `serde` -- serialization traits (existing)

## Requirement Traceability

| Requirement ID | Architecture Section | Module(s) | File(s) |
|---------------|---------------------|-----------|---------|
| REQ-SP-001 | Module 1: Consensus Selection | consensus/selection, scheduler | `crates/core/src/consensus/selection.rs`, `crates/core/src/scheduler.rs` |
| REQ-SP-002 | Protocol Version Gating | consensus/constants, chain_state | `crates/core/src/consensus/constants.rs`, `crates/storage/src/chain_state.rs` |
| REQ-SP-003 | Module 1: Consensus Selection | validation/producer | `crates/core/src/validation/producer.rs` |
| REQ-SP-004 | Module 1: Consensus Selection | production/scheduling, production/mod | `bins/node/src/node/production/scheduling.rs`, `bins/node/src/node/production/mod.rs` |
| REQ-SP-005 | Module 3: Graceful Empty Slots | production/mod, validation_checks | `bins/node/src/node/production/mod.rs`, `bins/node/src/node/validation_checks.rs` |
| REQ-SP-006 | Module 1: Consensus Selection | consensus/constants, production/mod, validation/producer | `crates/core/src/consensus/constants.rs`, `bins/node/src/node/production/mod.rs`, `crates/core/src/validation/producer.rs` |
| REQ-SP-007 | Module 1: Consensus Selection | production/scheduling | `bins/node/src/node/production/scheduling.rs` |
| REQ-SP-008 | Module 2: Attestation Fork Choice | sync/reorg, block_handling, finality | `crates/network/src/sync/reorg.rs`, `bins/node/src/node/block_handling.rs`, `crates/core/src/finality.rs` |
| REQ-SP-009 | Protocol Version Gating | All modules | Gated by `is_protocol_active(2, ...)` everywhere |
| REQ-SP-010 | Module 3: Cleanup | consensus/constants | `crates/core/src/consensus/constants.rs` |
| REQ-SP-011 | Module 2: Attestation Fork Choice | sync/reorg, block_handling | `crates/network/src/sync/reorg.rs`, `bins/node/src/node/block_handling.rs` |
| REQ-SP-012 | Module 3: Graceful Empty Slots | rewards | Already correct: epochs defined by block count |
| REQ-SP-013 | Module 3: Cleanup | scheduler | `crates/core/src/scheduler.rs` |
| REQ-SP-014 | Module 3: Cleanup | rpc/methods, periodic | `crates/rpc/src/methods.rs`, `bins/node/src/node/periodic.rs` |

## Test Plan Outline

### Unit Tests (per module)

**Module 1**:
- `test_select_single_proposer_returns_one` -- verify only 1 producer returned
- `test_select_single_proposer_deterministic` -- same slot + same producers = same proposer
- `test_single_proposer_different_from_v1_fallbacks` -- rank 0 is same, but no ranks 1-4
- `test_emergency_fallback_threshold` -- verify 10-slot gap triggers emergency
- `test_v1_v2_rank0_identical` -- v1 rank 0 == v2 single proposer (soft fork compatibility)
- `test_protocol_version_gating` -- v1 behavior when protocol < 2, v2 behavior when >= 2
- `test_bootstrap_mode_unaffected` -- genesis phase uses bootstrap regardless of protocol version

**Module 2**:
- `test_attestation_fork_choice_heavier_wins` -- chain with more attestation weight selected
- `test_attestation_fork_choice_tiebreak` -- equal attestation weight falls back to hash comparison
- `test_attestation_weight_from_presence_root` -- correct decode of bitfield into weight
- `test_fork_choice_v1_fallback` -- v1 nodes use producer weight (backward compat)

**Module 3**:
- `test_empty_slot_no_panic` -- chain handles missing slot gracefully
- `test_epoch_boundary_with_empty_slots` -- epoch rewards correct despite empty slots
- `test_consecutive_empty_slots_emergency` -- 10+ empty slots activates emergency
- `test_solo_production_watchdog_not_triggered_by_empty_slots` -- empty slots != isolation

### Integration Tests

- Deploy 5-node devnet, activate protocol v2, take 1 producer offline, verify empty slots and chain progress
- Deploy mixed v1/v2 network, verify v2 nodes reject rank 1-4 blocks but accept rank 0
- Simulate network partition, verify attestation-weighted fork choice resolves correctly

### Migration Test

- Start chain at v1, produce 100 blocks with fallback behavior
- Activate v2 via ProtocolActivation transaction
- Verify immediate switch to single-proposer behavior
- Verify no fork at activation boundary
