# Requirements: Single-Proposer-Per-Slot Migration

## Context

DOLI currently allows up to 5 producers (fallback ranks 0-4) to produce competing blocks for the same slot, each within a 2-second exclusive window. This causes network fragmentation when scaling beyond ~22 nodes because multiple valid blocks propagate simultaneously, leading to forks that are resolved by weight-based fork choice (which depends on local state).

The solution: adopt a single-proposer-per-slot model (similar to Ethereum). One deterministic proposer per slot, attestation-based fork choice, and empty slots when the proposer is offline.

## MoSCoW Priorities

### Must Have

| ID | Requirement | Acceptance Criteria |
|----|------------|-------------------|
| REQ-SP-001 | Single proposer selection per slot | `select_producer_for_slot()` returns exactly 1 producer. `MAX_FALLBACK_RANKS` effectively becomes 1 for protocol v2. |
| REQ-SP-002 | Protocol version gating | Changes activate only when `active_protocol_version >= 2`. Nodes at v1 continue using old rules. |
| REQ-SP-003 | Validation accepts only rank-0 blocks (v2) | `validate_producer_eligibility()` rejects blocks from non-primary producers when protocol v2 is active. |
| REQ-SP-004 | Production stops producing fallback blocks (v2) | `try_produce_block()` only produces if we are rank 0 for the slot. No waiting for fallback windows. |
| REQ-SP-005 | Empty slot tolerance | Chain advances normally when proposer misses a slot. No panic, no emergency equalization for gaps <= EMERGENCY_FALLBACK_THRESHOLD. |
| REQ-SP-006 | Emergency fallback for extended outages | After N consecutive empty slots (N >= 10), a backup mechanism allows additional producers to fill the slot. |
| REQ-SP-007 | Genesis/bootstrap mode preserved | Bootstrap mode (genesis phase) continues to work with simplified scheduling. Single-proposer applies only post-genesis. |
| REQ-SP-008 | Attestation-weighted fork choice | `execute_reorg()` and `handle_new_block()` use attestation weight (not producer bond weight) to choose between competing chains. |
| REQ-SP-009 | Backward compatibility during transition | Old nodes (v1) can still connect and sync. Their rank 1-4 blocks are simply ignored by v2 nodes. No hard fork required. |

### Should Have

| ID | Requirement | Acceptance Criteria |
|----|------------|-------------------|
| REQ-SP-010 | Remove deprecated fallback window constants | Clean up `PRIMARY_WINDOW_MS`, `SECONDARY_WINDOW_MS`, `TERTIARY_WINDOW_MS` and related deprecated code. |
| REQ-SP-011 | Attestation weight accumulation per block | Each block's fork-choice score includes attestation weight from subsequent blocks' `presence_root` bitfields. |
| REQ-SP-012 | Epoch reward calculation handles empty slots | Epoch boundaries defined by block count (not slot count). Empty slots do not affect reward distribution. |

### Could Have

| ID | Requirement | Acceptance Criteria |
|----|------------|-------------------|
| REQ-SP-013 | Proposer lookahead (predictable schedule) | Nodes can compute the proposer for the next N slots in advance for pre-computation. |
| REQ-SP-014 | Metrics for empty slot tracking | RPC/metrics expose consecutive empty slots, total empty slots per epoch, proposer hit rate. |

### Won't Have (this phase)

| ID | Requirement | Rationale |
|----|------------|-----------|
| REQ-SP-015 | Committee-based attestation | Full committee rotation is a future upgrade. Current bitfield attestation is sufficient. |
| REQ-SP-016 | Proposer-builder separation | PBS is a separate concern for a future milestone. |

## Affected Files

### Core consensus
- `crates/core/src/consensus/constants.rs` -- fallback constants
- `crates/core/src/consensus/selection.rs` -- `select_producer_for_slot()`, eligibility functions
- `crates/core/src/consensus/mod.rs` -- re-exports
- `crates/core/src/scheduler.rs` -- `DeterministicScheduler::select_producer()`
- `crates/core/src/network_params/defaults.rs` -- `max_fallback_ranks`
- `crates/core/src/network_params/mod.rs` -- `max_fallback_ranks` field

### Validation
- `crates/core/src/validation/producer.rs` -- `validate_producer_eligibility()`, `bootstrap_fallback_order()`
- `bins/node/src/node/validation_checks.rs` -- `check_producer_eligibility()`

### Production
- `bins/node/src/node/production/mod.rs` -- `try_produce_block()`
- `bins/node/src/node/production/scheduling.rs` -- `resolve_epoch_eligibility()`, `resolve_bootstrap_eligibility()`

### Fork choice
- `bins/node/src/node/block_handling.rs` -- `handle_new_block()`, `execute_reorg()`, `execute_fork_sync_reorg()`
- `crates/network/src/sync/reorg.rs` -- `ReorgHandler`, weight comparison
- `crates/core/src/finality.rs` -- `FinalityTracker`

### Attestation
- `crates/core/src/attestation.rs` -- attestation types and bitfield encoding
- `bins/node/src/node/production/assembly.rs` -- attestation inclusion
- `bins/node/src/node/apply_block/post_commit.rs` -- epoch snapshot

### Gossip
- `crates/network/src/gossip/config.rs` -- mesh parameters (unchanged, but relevant)
