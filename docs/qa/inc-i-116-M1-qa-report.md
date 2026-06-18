# QA Report: INC-I-116 M1 -- Epoch-Boundary Liveness Prune Redesign

## Summary

All 8 acceptance criteria verified. The gated activation-height change to epoch-boundary liveness pruning is correctly implemented with pre-activation bit-identity preserved, lockstep between canonical and rebuild paths, all 3 construction sites threaded, and no version bumps.

**Verdict: PASS**

## Filter Verification Table

| Filter | Description | Applies to M1 | Result | Evidence |
|--------|-------------|---------------|--------|----------|
| FILTER-01 | Determinism | Yes | PASS | `derive_at_boundary` is a pure function taking `(prev: &EpochState, input: &EpochDerivationInput)`. Zero `self.config`, `state_db`, or node-local references. |
| FILTER-02 | Serialization stability | No (EpochState struct unchanged) | N/A | -- |
| FILTER-03 | Rebuild parity | Yes | PASS | Both `derive_at_boundary()` and `rewards.rs` rebuild path implement identical gated floor: pre-AH uses `effective_active * 2/3` condition; post-AH uses `MIN_PRODUCERS_FLOOR`. |
| FILTER-04 | Rollback safety | No (no apply_block changes) | N/A | -- |
| FILTER-05 | Absolute floor | Yes | PASS | Post-activation: `new_list.len() < MIN_PRODUCERS_FLOOR` triggers fallback. Test `test_post_activation_absolute_floor_fires` verifies with 2/57 attested (below floor=3), falls back to all 57. |
| FILTER-06 | Activation height gate | Yes | PASS | `if input.height >= input.epoch_prune_activation_height` gates the branch. Mainnet=u64::MAX (never fires). Testnet/devnet=0 (always fires). |
| FILTER-07 | No version bump | Yes | PASS | `EpochState` struct unchanged (7 fields, no additions). `CURRENT_PROTOCOL_VERSION=8`, `EPOCH_STATE_FORMAT_VERSION=1`, `MIN_PEER_PROTOCOL_VERSION=1` -- none modified. |
| FILTER-08 | Bit-identity | Yes | PASS | Pre-activation `else` branch is character-identical to the removed code (only a comment `// Mass event...` removed, which has no execution effect). Test `test_pre_activation_floor_is_identical_to_current_behavior` passes with u64::MAX. |
| FILTER-09 | Bond snapshot unchanged | No (no bond changes) | N/A | -- |
| FILTER-10 | Coinbase unchanged | No (no coinbase changes) | N/A | -- |
| FILTER-11 | Block validation unchanged | No (no validation changes) | N/A | -- |
| FILTER-12 | Tier system unchanged | No (no tier changes) | N/A | -- |
| FILTER-13 | Accumulator rotation unchanged | No (rotation code untouched) | N/A | -- |
| FILTER-14 | Rewards ordering | Yes | PASS | Rewards rebuild: attestation filter applied first (line ~836), then gated floor logic (line ~919). Matches canonical `derive_at_boundary` ordering (step 2 before step 3). |

## INC-I-075 Three-Question Checklist

| Question | Answer | Justification |
|----------|--------|---------------|
| Q1: Can any user-submittable transaction trigger this code path? | NO | The prune logic runs at epoch boundary derivation only. |
| Q2: Can any producer-action or attestation pattern trigger it? | YES | Attestation presence/absence determines which producers are pruned. |
| Q3: Is the new behavior bit-identical to the old behavior for ALL reachable inputs? | NO | Post-activation produces a different producer_list for the same inputs. |
| Verdict | Activation height REQUIRED | Present: `epoch_prune_activation_height` in NetworkParams. |

## Test Results

```
running 22 tests
test epoch_state::tests::test_accumulate_block_no_attestation ... ok
test epoch_state::tests::test_accumulate_block_with_attestation ... ok
test epoch_state::tests::test_accumulate_block_increments_count ... ok
test epoch_state::tests::test_derive_at_boundary_attestation_filter ... ok
test epoch_state::tests::test_derive_accumulator_rotation ... ok
test epoch_state::tests::test_derive_at_boundary_epoch_1 ... ok
test epoch_state::tests::test_derive_deadlock_safety_floor ... ok
test epoch_state::tests::test_derive_empty_accum_uses_all_producers ... ok
test epoch_state::tests::test_genesis_creates_empty_state ... ok
test epoch_state::tests::test_ghost_exclusion_grace_period_for_new_registrations ... ok
test epoch_state::tests::test_ghost_exclusion_inactive_before_activation ... ok
test epoch_state::tests::test_ghost_exclusion_mass_event_saves_real_producers ... ok
test epoch_state::tests::test_ghost_exclusion_prevents_deadlock_floor_override ... ok
test epoch_state::tests::test_hash_deterministic ... ok
test epoch_state::tests::test_hash_differs_on_change ... ok
test epoch_state::tests::test_post_activation_absolute_floor_fires ... ok
test epoch_state::tests::test_post_activation_zero_attested_uses_fallback ... ok
test epoch_state::tests::test_post_activation_prunes_absent_producers ... ok
test epoch_state::tests::test_pruned_producer_reappears_on_attestation ... ok
test epoch_state::tests::test_serialize_deserialize_round_trip_empty ... ok
test epoch_state::tests::test_serialize_deserialize_round_trip_populated ... ok
test epoch_state::tests::test_pre_activation_floor_is_identical_to_current_behavior ... ok

test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 933 filtered out
```

Build: `cargo build --release -p doli-core -p doli-node` -- clean.
Clippy: `cargo clippy -p doli-core -p doli-node -- -D warnings` -- clean.

## Construction Site Threading

All 3 construction sites correctly thread `epoch_prune_activation_height` from `self.config.network.params()`:

| Site | File | Line | Source |
|------|------|------|--------|
| post_commit | `bins/node/src/node/apply_block/post_commit.rs` | 308-312 | `self.config.network.params().epoch_prune_activation_height` |
| fork_recovery | `bins/node/src/node/fork_recovery.rs` | 718-722 | `self.config.network.params().epoch_prune_activation_height` |
| rewards (rebuild) | `bins/node/src/node/rewards.rs` | 919 | `self.config.network.params().epoch_prune_activation_height` |

## NetworkParams Defaults

| Network | Value | Correct |
|---------|-------|---------|
| Mainnet | `u64::MAX` | Yes (frozen, operator pins before deploy) |
| Testnet | `0` | Yes (always active) |
| Devnet | `0` | Yes (always active) |
| env_loader | Mainnet locked, testnet/devnet overridable via `DOLI_EPOCH_PRUNE_ACTIVATION_HEIGHT` | Yes |

## REQ-PRUNE-003 (Re-inclusion)

Test `test_pruned_producer_reappears_on_attestation` verifies the two-epoch flow:
- Epoch N+1: producer absent, gets pruned (8 attested >= floor 3, absent producer excluded)
- Epoch N+2: producer re-attests during epoch N+1, appears in attested_union, re-included
- No on-chain transaction required for re-inclusion

PASS.

## Blocking Issues

None.

## Non-Blocking Observations

- **OBS-001**: The pre-activation branch removed one inline comment (`// Mass event -- include all non-ghost producers`) that existed in the original code. This has zero execution impact but is noted for completeness. The comment was arguably descriptive and its removal is acceptable since the branch now has the header comment `// Pre-activation: VERBATIM proportional floor...` which serves the same documentation purpose.

## Final Verdict

**PASS** -- All 8 acceptance criteria met. No blocking issues. All 22 epoch_state tests pass. Build and clippy clean. Pre-activation behavior is byte-identical to current. Activation height correctly gates the new absolute floor. Approved for review.
