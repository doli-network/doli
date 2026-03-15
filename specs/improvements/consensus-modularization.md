# Consensus Module Modularization

> REQ doc for splitting `crates/core/src/consensus.rs` (4,242 lines) into sub-modules.
> Pattern: follows `validation.rs` modularization (commit `f15b409`).

## Scope

**Target**: `crates/core/src/consensus.rs` → `crates/core/src/consensus/` directory
**Goal**: Max ~500 lines per sub-module. Zero behavioral changes.
**Safety net**: `cargo build --all-targets && cargo clippy -- -D warnings && cargo fmt --check && cargo test`

## Proposed Sub-modules

| # | Module | Domain | ~Lines |
|---|--------|--------|--------|
| 1 | `mod.rs` | Re-exports + mod declarations | ~100 |
| 2 | `constants.rs` | All 68+ `pub const` + `is_protocol_active`, `reward_pool_pubkey_hash`, `max_block_size` | ~380 |
| 3 | `exit.rs` | PenaltyDestination, RewardMode, ExitTerms, SlashResult, calculate_exit*, calculate_slash, withdrawal_penalty_rate* | ~200 |
| 4 | `producer_state.rs` | PresenceScore type alias, score constants, ProducerState struct+impl | ~110 |
| 5 | `bonds.rs` | BondEntry, WithdrawalResult, ProducerBonds, BondsMaturitySummary, BondError | ~310 |
| 6 | `vdf.rs` | VDF constants + t_block(), construct_vdf_input(), registration VDF constants | ~85 |
| 7 | `registration.rs` | Fee constants/functions, PendingRegistration, RegistrationQueue | ~320 |
| 8 | `selection.rs` | select_producer_for_slot, eligible_rank*, allowed_producer_rank*, is_producer_eligible*, get_producer_rank, deprecated scaled functions | ~200 |
| 9 | `tiers.rs` | compute_tier1_set, producer_region, producer_tier | ~80 |
| 10 | `params.rs` | ConsensusParams struct + all impl blocks + Default | ~320 |
| 11 | `stress.rs` | StressTestParams + ConsensusParams::for_stress_test | ~150 |
| 12 | `reward_epoch.rs` | pub mod reward_epoch contents | ~200 |
| 13 | `tests.rs` | All #[cfg(test)] tests | ~1,870 |

## Requirements

| ID | Requirement | Priority |
|----|------------|----------|
| REQ-CON-001 | All public items re-exported from mod.rs — zero consumer changes | Must |
| REQ-CON-002 | `cargo build --all-targets` passes | Must |
| REQ-CON-003 | `cargo clippy -- -D warnings` passes | Must |
| REQ-CON-004 | `cargo fmt --check` passes | Must |
| REQ-CON-005 | `cargo test` passes (all existing tests) | Must |
| REQ-CON-006 | No changes to files outside `crates/core/src/consensus*` | Must |
| REQ-CON-007 | Each sub-module under ~500 lines (soft limit) | Should |

## Execution Order

1. `consensus.rs` → `consensus/mod.rs` (rename, all content)
2. Extract `constants.rs` (least coupled)
3. Extract `vdf.rs` (self-contained)
4. Extract `exit.rs` (depends on constants for VESTING_QUARTER_SLOTS)
5. Extract `producer_state.rs` (depends on constants for score values)
6. Extract `bonds.rs` (depends on constants + exit for withdrawal_penalty_rate*)
7. Extract `registration.rs` (depends on constants)
8. Extract `selection.rs` (depends on constants for MAX_FALLBACK_*)
9. Extract `tiers.rs` (depends on constants for TIER1_MAX_*)
10. Extract `params.rs` (depends on many constants + exit for RewardMode)
11. Extract `stress.rs` (depends on params)
12. Extract `reward_epoch.rs` (depends on BLOCKS_PER_REWARD_EPOCH)
13. Extract `tests.rs` (depends on everything via `super::*`)
