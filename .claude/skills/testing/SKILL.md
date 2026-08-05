# testing — DOLI Test Infrastructure
<!-- OUTPUT CONTRACT: N/A — skill/reference file, not a test file -->
<!-- INPUT PARTITIONS: N/A — skill/reference file, not a test file -->
<!-- @INDEX
INTEGRATION-TESTS   16-142
E2E-TESTS           143-151
FUZZ-TARGETS        152-166
NODE-TESTS          167-239
SIMULATION-TESTS    240-277
BENCHMARKS          278-289
OTHER-CRATE-TESTS   290-300
TEST-UTILITIES      301-349
PATTERNS            350-452
@/INDEX -->

## INTEGRATION-TESTS

All live in `testing/integration/`. **CRITICAL DRIFT (verified 2026-07-09): only 7 of the 13 `.rs` files in this directory are wired into `testing/integration/Cargo.toml` `[[test]]` entries.** Cargo does NOT autodiscover tests here (files sit at the crate root, not in a `tests/` subdir) — a file not listed in Cargo.toml is **never compiled or run**, regardless of its content. Verified by reading `testing/integration/Cargo.toml` directly.

### WIRED (run by `cargo test -p integration-tests`)
`epoch_rewards.rs`, `staggered_validator_rewards.rs`, `two_producer_pop.rs`, `bond_stacking.rs`, `equivocation_slashing.rs`, `presence_manipulation_test.rs`, `mempool_poison.rs`.

### ORPHANED (exist on disk, NOT in Cargo.toml, NOT run by any `cargo test`)
`two_node_sync.rs`, `reorg_test.rs`, `partition_heal.rs`, `attack_reorg_test.rs`, `malicious_peer.rs`, `mempool_stress.rs`. Two of these are additionally **broken against current code** (would fail to compile if re-added):
- `malicious_peer.rs:32-44` constructs `BlockHeader { .. }` WITHOUT the `presence_root` field (added by the presence-manipulation fix) — missing-field compile error.
- `attack_reorg_test.rs` calls `common::create_transfer(&attacker, tx_hash, idx, &recipient_hash, amount, &change_hash, fee)` (7 args) but the real `create_transfer` in `testing/common/mod.rs:257` takes `(inputs: Vec<(Hash,u32)>, outputs: Vec<(Amount,Hash)>, keypair)` (3 args) — signature mismatch, compile error.
Treat these 6 files as **historical reference only**. Do not assume their assertions reflect current behavior; do not cite them as passing regression coverage.

### epoch_rewards.rs (WIRED, 644 lines)
Pool-First epoch reward distribution unit tests (no async, no node).

| Function | Line | What it tests |
|---|---|---|
| `test_fair_share_calculation_even_split` | 26 | 300 DOLI / 3 producers = 100 DOLI each, remainder=0 |
| `test_fair_share_calculation_with_remainder` | 39 | 100_000_000_001 / 3 = 33_333_333_333 + remainder=2, first gets dust |
| `test_fair_share_single_producer` | 60 | Single producer gets 100% of pool |
| `test_fair_share_many_producers` | 73 | 1000 DOLI / 100 producers = 10 DOLI each |
| `test_epoch_reward_transaction_creation` | 94 | `TxType::EpochReward`, no inputs, 1 output, correct amount and pubkey_hash |
| `test_epoch_reward_transaction_data` | 111 | `epoch_reward_data()` returns correct epoch and recipient |
| `test_epoch_reward_has_correct_type` | 128 | type=EpochReward, `is_epoch_reward()=true`, `is_coinbase()=false` |
| `test_epoch_reward_utxo_maturity` | 145 | Not spendable < 6 confirmations; spendable at exactly 6 |
| `test_coinbase_maturity_unchanged` | 172 | Coinbase also requires 6 confirmations |
| `test_regular_tx_no_maturity` | 194 | Regular output spendable at same height |
| `test_pool_accumulation_over_epoch` | 218 | pool = block_reward * epoch_length |
| `test_epoch_total_matches_distribution` | 234 | first_producer + others = pool exactly |
| `test_producer_sorting_deterministic` | 261 | Sorting by pubkey bytes is deterministic |
| `test_first_producer_gets_remainder` | 279 | Remainder assigned to index-0 producer (sorted by pubkey) |
| `test_reward_mode_epoch_pool_is_default` | 314 | `RewardMode::default() == RewardMode::EpochPool` |
| `test_consensus_params_reward_mode` | 320 | All networks (mainnet/testnet/devnet) use EpochPool |
| `test_epoch_boundary_detection` | 332 | Slot 0 is NOT boundary; multiples of epoch_length are |
| `test_epoch_reward_minimum_amount` | 357 | Amount=1 base unit is valid |
| `test_epoch_reward_large_epoch_number` | 374 | `u64::MAX` epoch number survives round-trip |
| `test_reward_maturity_constant` | 386 | `REWARD_MATURITY == 6` |
| `test_utxo_set_add_epoch_reward` | 396 | `add_transaction` marks is_epoch_reward=true, height=100 |
| `test_utxo_set_epoch_reward_balance` | 423 | `get_balance` returns 0 before maturity, 100 DOLI after |
| `test_producer_block_after_empty_boundary` | 449 | Empty boundary slot → first block of next epoch carries rewards |
| `test_empty_epoch_produces_no_rewards` | 481 | pool=0 when epoch has zero blocks |
| `test_multi_epoch_catchup_order` | 504 | last_rewarded=2 at epoch 5 → rewards epoch 3, then 4, then 5 |
| `test_epoch_reward_slot_range_calculation` | 542 | Epoch N covers slots [N*epoch_len, (N+1)*epoch_len) |
| `test_proportional_rewards_rounding` | 581 | 7/10/2/10/1/10 split: 70M/20M/10M; total=pool |
| `test_epoch_rewards_with_odd_distribution` | 622 | 100 / 3 = 33+33+34 (last gets dust) |

### attack_reorg_test.rs (ORPHANED — see drift note above, contains real bugfix-shaped scenarios but not compiled)
Security scenario tests: double-spend, selfish mining, long-range attack, nothing-at-stake, eclipse, Sybil, rapid/concurrent reorg. 936 lines, 15 `#[tokio::test]` fns starting at lines 31, 151, 196, 254, 325, 379, 417, 504, 539, 588, 629, 681, 742, 819, 876.

### equivocation_slashing.rs (WIRED, 490 lines)
Full equivocation detection → proof → slash TX → ProducerSet update pipeline. CRITICAL: must pass before genesis.

| Function | Line | What it tests |
|---|---|---|
| `test_equivocation_detection_basic` | 82 | Two different blocks for same slot triggers `EquivocationDetector::check_block` → `Some(proof)` |
| `test_same_block_twice_not_equivocation` | 123 | Same block twice → `None`; `has_pending_proofs() == false` |
| `test_different_slots_not_equivocation` | 145 | Blocks at slots 10 and 11 → `None` |
| `test_different_producers_same_slot_not_equivocation` | 167 | Different producers at same slot → `None` |
| `test_proof_contains_vdf_verifiable_headers` | 191 | Proof has full `BlockHeader` structs with correct slot/producer fields |
| `test_proof_to_slash_transaction` | 224 | `proof.to_slash_transaction(&reporter)` creates `SlashProducer` TX with evidence |
| `test_slashing_updates_producer_status` | 273 | `slash_producer()` → `ProducerStatus::Slashed`; honest producer unaffected |
| `test_slashed_producer_cannot_reregister` | 310 | Slashed status persists; re-registration blocked |
| `test_multiple_equivocations_detected` | 336 | Two equivocations in different slots → 2 pending proofs |
| `test_equivocation_slashing_e2e` | 369 | Full pipeline: detect → proof → slash TX → ProducerSet update → 2 active remain |
| `test_detector_eviction_memory_bounded` | 472 | After 2000 entries, `tracked_count() <= 1000` |

### mempool_poison.rs (WIRED, 365 lines)
Mempool purge-by-error-pattern (testnet incident 2026-03-25: NFT re-injection froze production). Helper `make_signed_nft_tx(keypair, funding_hash)` builds a fully-structured EncryptedContent TX (condition=34B + ciphertext_len=4B + wrapped_key=80B + nonce=12B + content_hash=32B).

| Function | Line | What it tests |
|---|---|---|
| `test_poison_nft_purged_by_error_pattern` | 119 | NFT TX purged when error contains "token_id ... already exists" |
| `test_poison_purge_preserves_normal_txs` | 136 | NFT-pattern purge does not remove unrelated Transfer TXs |
| `test_poison_10_purge_cycles_no_crash` | 153 | 10 add+purge cycles don't crash or leak |
| `test_poison_pool_pattern_targets_create_pool` | 188 | Pool "already exists" pattern on empty mempool is a no-op |
| `test_poison_registration_pattern` | 203 | Registration "already registered" pattern is a no-op on empty mempool |
| `test_poison_mixed_mempool_selective_purge` | 217 | Mixed normal+NFT mempool: only NFT purged, normal survives |
| `test_poison_pattern_specificity` | 271 | Unrelated error strings never crash/mutate an empty mempool |
| `test_poison_regossip_repurge` | 289 | Re-add after purge + re-purge is safe (0 or ≤1 remaining) |
| `test_poison_purge_idempotent` | 316 | 100x purge calls on empty mempool is a no-op each time |
| `test_poison_full_lifecycle` | 336 | add → simulate apply_block rejection error → purge → verify gone |

### presence_manipulation_test.rs (WIRED, 289 lines)
Verifies `presence_root` is part of `BlockHeader::hash()` (DOLI_NETWORK_BUG.md fix): `test_presence_root_affects_block_hash`(36), `test_presence_root_zero_vs_nonzero`(59), `test_total_weight_manipulation_detected`(82), `test_weighted_reward_calculation`(115), `test_presence_commitment_v2_determinism`(164), `test_block_hash_includes_all_presence_components`(182), `test_doli_network_bug_scenario`(221).

### bond_stacking.rs (WIRED, 769 lines — 33 `#[test]` fns, not just constants)
Bond stacking: `BOND_UNIT=1_000_000_000`, `MAX_BONDS_PER_PRODUCER=3_000`, `YEAR_IN_SLOTS=3_153_600`, vesting Q1-Q4 penalty schedule (75/50/25/0%), FIFO withdrawal, per-bond `StoredBondEntry`/`bond_entries` tracking, `withdrawal_pending_count` double-withdrawal guard.
Constants/basics: lines 39-153 (`test_bond_unit_constant`, `test_max_bonds_constant`, `test_year_in_slots_constant`, `test_vesting_constants`, `test_withdrawal_penalty_schedule`, `test_bond_entry`, `test_bond_withdrawal_amounts`, `test_bond_is_vested`).
`ProducerBonds` API: lines 157-422 (`test_producer_bonds_add`, `test_producer_bonds_max_limit`, `test_producer_bonds_fifo_withdrawal`, `test_bonds_maturity_summary`, `test_total_withdrawal_penalty`, `test_add_bond_data_serialization`, `test_withdrawal_request_data_serialization`, `test_add_bond_transaction`, `test_request_withdrawal_transaction`, `test_bond_error_display`, `test_realistic_bond_scenario`).
`StoredBondEntry` / `ProducerInfo` per-bond tracking: lines 428-768 (`test_stored_bond_entry_creation`(428), `test_producer_info_initializes_bond_entries`(439), `test_bond_entries_migration`(461), `test_add_bonds_with_creation_slot`(503), `test_calculate_withdrawal_fifo_order`(529), `test_withdrawal_mixed_age_penalties`(554), `test_apply_withdrawal_reduces_bond_count`(582), `test_withdrawal_insufficient_bonds`(605), `test_withdrawal_pending_prevents_double`(622), `test_withdrawal_pending_resets_at_epoch`(645), `test_withdrawal_all_vested_zero_penalty`(666), `test_withdrawal_all_q1_max_penalty`(687), `test_full_withdrawal_lifecycle`(706), `test_new_with_bonds_initializes_entries`(752)).

### two_producer_pop.rs (WIRED, 409 lines)
Simplified PoP model, 2 producers alternating, uses real `generate_genesis_block(&GenesisConfig::devnet())`.

| Function | Line | What it tests |
|---|---|---|
| `test_two_producers_alternating` | 24 | 10 blocks alternating; each producer's `presence_score` = INITIAL + 5×PRODUCE_BONUS |
| `test_producer_misses_slots` | 98 | Producer 1 misses slot 3; Producer 2 fallback-produces; score penalty applied |
| `test_presence_rate_calculation` | 190 | `presence_rate()` = produced/(produced+missed)*100, new producer defaults to 100 |
| `test_minimum_presence_threshold` | 226 | `meets_minimum()` true ≥ MIN_PRESENCE_RATE (90% ok, 40% fails) |
| `test_producer_activity` | 260 | `is_active(slot, threshold)` — active within threshold, inactive after |
| `test_genesis_block_valid` | 282 | `verify_genesis_block` passes for devnet/testnet/mainnet configs |
| `test_full_pop_chain` | 306 | 100-slot simulation, 95% vs 70% reliability, more-reliable producer earns more + higher score |

### staggered_validator_rewards.rs (WIRED, 663 lines)
Local `EpochRewardTracker` helper (not production code) simulating Pool-First proportional-to-blocks-produced distribution for producers joining mid-epoch.

| Function | Line | What it tests |
|---|---|---|
| `test_producer_joins_mid_epoch_no_immediate_rewards` | 108 | P2 joins mid-epoch-0, still gets proportional epoch-0 reward for blocks produced; P3 (joins epoch-1) excluded from epoch-0 |
| `test_producer_joins_but_no_blocks_no_rewards` | 332 | Producer who joined but produced 0 blocks gets 0 reward |
| `test_ten_producers_fair_distribution` | 387 | 10 producers staggered join over 3 epochs; reward proportional to blocks each epoch |
| `test_proportional_rewards_unequal_blocks` | 528 | 8/21/1 block split over 30 → ~27%/70%/3.3% of pool (bug-fix regression anchor) |

### mempool_stress.rs (ORPHANED, 476 lines)
10k-scale mempool stress: sequential/concurrent/batched adds, full-rejection at 10k cap, clear, varying tx sizes, throughput, memory footprint, churn, concurrent read/write, duplicate handling, burst traffic. 12 `#[tokio::test]` fns at lines 52, 79, 117, 156, 184, 214, 262, 299, 331, 367, 406, 437.

### two_node_sync.rs, reorg_test.rs, partition_heal.rs, malicious_peer.rs (ORPHANED)
Historical reference only (see drift note). Function/line inventory (still accurate against the file content, NOT against live behavior):
- `two_node_sync.rs` (368 lines): `test_two_nodes_sync_basic`(22), `test_incremental_sync`(65), `test_large_sync_gap`(120), `test_duplicate_block_handling`(158), `test_sync_multiple_producers`(187), `test_utxo_sync`(241), `test_reject_future_blocks`(276), `test_concurrent_block_additions`(302), `test_chain_tip_tracking`(341).
- `reorg_test.rs` (434 lines): `test_single_block_reorg`(20), `test_deep_reorg_10_blocks`(67), `test_very_deep_reorg`(116), `test_utxo_consistency_during_reorg`(163), `test_multiple_sequential_reorgs`(212), `test_reorg_different_producers`(281), `test_reorg_chain_integrity`(322), `test_reorg_equal_length`(366), `test_empty_revert`(409).
- `partition_heal.rs` (474 lines): `test_partition_separate_chains`(20), `test_heal_longer_partition_chain`(95), `test_three_way_partition`(158), `test_partition_utxo_reconciliation`(244), `test_partition_large_length_difference`(325), `test_mock_peer_partition`(383), `test_gradual_healing`(433).
- `malicious_peer.rs` (505 lines, CONFIRMED BROKEN — see drift note): `test_reject_wrong_prev_hash`(151), `test_bad_merkle_root`(175), `test_tx_overflow_amount`(194), `test_tx_duplicate_inputs`(206), `test_mempool_rejects_bad_tx`(219), `test_unknown_producer_block`(243), `test_future_dated_block`(279), `test_empty_block`(312), `test_excessive_coinbase_reward`(343), `test_corrupted_block_data`(360), `test_corrupted_tx_data`(385), `test_zero_amount_output`(409), `test_too_many_outputs`(428), `test_slot_timestamp_mismatch`(451), `test_rapid_invalid_submissions`(484).

---

## E2E-TESTS

All live in `testing/e2e/`. **NOT A CARGO PACKAGE — `testing/e2e/Cargo.toml` does not exist and `testing/e2e` is NOT a workspace member.** These files cannot be compiled or run by any `cargo test` invocation today. Confirmed additionally broken: both files construct `storage::UtxoEntry { output, height, is_coinbase }` (3 fields) but the current `UtxoEntry` struct requires a 4th field `is_epoch_reward` (see `testing/integration/epoch_rewards.rs:154-159` and `testing/common/mod.rs:132-137` for the current 4-field shape). Treat as historical design reference only.

- `full_cycle.rs` (447 lines): `test_genesis_to_1000_blocks`(27), `test_full_cycle_with_reorg`(69), `test_full_cycle_with_transactions`(145), `test_era_transition_simulation`(228, `#[ignore]`), `test_block_production_simulation`(266), `test_chain_state_consistency`(309), `test_genesis_block`(367), `test_performance_metrics`(397).
- `wallet_flow.rs` (598 lines): local `TestWallet` struct (address/balance/add_utxo/remove_utxo/select_utxos_for_amount/create_send_transaction). Tests: `test_wallet_basic_flow`(113), `test_wallet_receive_coinbase`(127), `test_wallet_send_coins`(164), `test_wallet_multiple_transactions`(255), `test_wallet_reorg_handling`(360), `test_wallet_immediate_spend`(427), `test_wallet_utxo_consolidation`(478), `test_wallet_balance_accuracy`(529), `test_wallet_signature_verification`(568).

---

## FUZZ-TARGETS

Isolated cargo-fuzz crate at `testing/fuzz/` (own `Cargo.toml`, `[workspace] members=["."]` — deliberately excluded from the main workspace, this is correct/expected for cargo-fuzz). Run via `cargo fuzz run <target>` from `testing/fuzz/`. No `#[test]`, uses `libfuzzer_sys::fuzz_target!`.

| Bin name | Target file | Status | Invariants checked |
|---|---|---|---|
| `fuzz_merkle` | `targets/merkle.rs` | OK | `compute_merkle_root` deterministic, no panic, tx_type%3→Transfer/Registration/Exit, ≤10 outputs |
| `fuzz_tx_deserialize` | `targets/tx_deserialize.rs` | OK | `Transaction::deserialize` never panics; round-trip serialize→deserialize preserves hash |
| `fuzz_block_deserialize` | `targets/block_deserialize.rs` | OK | `Block::deserialize` never panics on arbitrary bytes |
| `fuzz_hash` | `targets/hash.rs` | OK | `hash::hash` deterministic; `Hasher` incremental == single-call; `to_hex` is 64 chars |
| `fuzz_signature` | `targets/signature.rs` | OK | `signature::verify` never panics for arbitrary pubkey/sig/message |
| `fuzz_vdf_verify` | `targets/vdf_verify.rs` | **BROKEN — file does not exist** | `Cargo.toml:38-43` declares this `[[bin]]` but `targets/vdf_verify.rs` is missing (confirmed via direct Read, also tried `targets/vdf.rs` — also missing). `cargo fuzz build` (all targets) fails; individual `cargo fuzz run fuzz_merkle` etc. still work by targeting one bin. |

---

## NODE-TESTS

All live in `bins/node/tests/`. Standard Cargo convention (`tests/*.rs` under a package with no `autotests=false`) — **all files here ARE auto-discovered and run** by `cargo test -p doli-node`. Uses real `Node::new_for_test()` with real RocksDB, real ProducerSet, real SyncManager. No mocks. No networking — blocks injected via `apply_block(block, ValidationMode::Light|Full|Replay)`.

### test_network.rs (2530 lines) — core test infrastructure + 22 tests
`TestNetwork` struct (line 26): simulated P2P network of real `Node::new_for_test()` instances wrapped in `Arc<Mutex<Node>>`, full-mesh topology.
Key methods: `new(n_nodes, n_producers)`(46), `build_block`(95), `build_chain`(131), `apply_to_node`(155), `propagate`(167, parallel), `produce_and_propagate`(195), `produce_blocks`(225), `partition`/`heal`(239/249), `height`/`hash`(254/261), `is_synced`(268), `gossip_with_sync`(653), `gossip_propagate`(803), `gossip_with_eligibility`(843), `gossip_realistic`(1063, uses `check_producer_eligibility` — the real gossip gate), `backfill_from_leader`(1147), `backfill_with_eligibility`(1177), `sync_from_leader`(1216, full ancestor-find+rollback+replay), `height_distribution`(1351), `build_block_with_gap`(2277, populates `missed_producers`), `epoch_list`/`set_epoch_list_all`(2332/2338).
`ClusterNetwork` (496): runs N sequential clusters of M nodes each, dropping between clusters to stay under macOS RocksDB `fopen()` FD ceiling (~1259 nodes via `fopen`, confirmed at line 461-465 comment).
Tests (fn name @ line): `test_network_creates_and_syncs`(305), `test_network_partition_and_heal`(317), `test_network_10_nodes`(366), `test_network_scale_ceiling`(374), `test_network_50_nodes`(398), `test_network_100_nodes`(410), `test_network_500_nodes`(423,`#[ignore]`), `test_network_1000_nodes`(436,`#[ignore]`), `test_network_5000_nodes`(449,`#[ignore]`), `test_network_10000_nodes`(466,`#[ignore]`,macOS FD-capped), `test_cluster_10x100`(609), `test_cluster_10x500`(618,`#[ignore]`), `test_cluster_10x1000`(627,`#[ignore]`), `test_cluster_100x1000`(636,`#[ignore]`), `test_gossip_divergence_and_recovery`(891), `test_gossip_convergence_50_nodes`(951), `test_realistic_gossip_20_nodes_100_blocks`(1369), `test_realistic_gossip_100_nodes_200_blocks`(1581,`#[ignore]`), `test_realistic_gossip_500_nodes`(1664,`#[ignore]`), `test_realistic_gossip_1000_nodes`(1736,`#[ignore]`), `test_realistic_gossip_10k_clustered`(1808,`#[ignore]`), `test_scheduler_slot_coverage_100_nodes`(1944), `test_scheduler_slot_coverage_500_nodes_varied_bonds`(2120,`#[ignore]`), `test_onchain_liveness_exclusion_deterministic`(2355), `test_onchain_liveness_10k_nodes`(2438).
Run the heavy `#[ignore]` scale tests explicitly: `cargo test -p doli-node --test test_network -- --ignored --test-threads=1`.

### fork_recovery.rs (550 lines) — 10 tests on real Node
Helpers: `make_node(n_producers)`(23), `build_block`(33), `build_chain`(69), `apply_chain`(90, panics on error), `devnet_genesis_hash`(100).

| Function | Line | What it tests |
|---|---|---|
| `test_fork_recovery_with_divergent_bonds` | 108 | Fork with bond_snapshot divergence; canonical chain applied after rollback |
| `test_cumulative_rollback_resets_on_sync` | 151 | 49 rollbacks → depth=49; apply synced block → depth resets to 0 |
| `test_recovery_from_20_block_fork` | 190 | 20-block fork rolled back; 25-block canonical applied; height=35 |
| `test_recovery_with_scheduler_divergence` | 229 | bond_snapshot divergence; canonical 8-block chain applies despite it |
| `test_recovery_after_rollback_cap` | 269 | 50 rollbacks hit cap; 51st refused; synced block resets cap to 0 |
| `test_no_refork_after_recovery` | 310 | After recovery, 100 more blocks apply cleanly; `shallow_rollback_count==0` |
| `test_recovery_under_load` | 351 | 50-block fork reverted; 60-block canonical applied during/after recovery |
| `test_multiple_nodes_recover_independently` | 388 | 3 nodes each fork independently (5/7/3 blocks); all converge to 20-block canonical |
| `test_recovery_preserves_mempool` | 461 | 10 system TXs in mempool survive fork+rollback |
| `test_post_snap_gossip_validation_mode` | 512 | `snap_sync_height=Some(2)` → cleared at epoch boundary (h=4, devnet bpe=4) |

### epoch_reward_explicit_inputs.rs
Tests `EPOCH_REWARD_EXPLICIT_INPUTS_HEIGHT` (25,560) hard fork: pre-activation EpochReward has empty inputs (pool consumed by side-effect); post-activation has explicit sorted pool UTXO inputs. Tests 1-6 fast unit tests; 7-8 build full chain to activation height (~60s each). Uses `MockUtxoProvider` with `add_pool_utxo`/`add_non_pool_utxo`.

### epoch_state_regression.rs
`EpochState` refactor (INC-I-035) regression: `derive_at_boundary` producer_list/active_list correctness, `accumulate_block` per-block tracking, `UndoData` round-trip (rollback restores exact `epoch_state`), multi-epoch accumulator rotation. Helper `make_node`(24).

### checkpoint_rotation.rs
Auto-checkpoint rotation (INC-I-020): keeps 5 HIGHEST-height checkpoints, not lexicographically-last (`h526` sorts after `h4535` lexicographically — the bug). Includes `bins/node/tests/test_network.rs` via `#[allow(dead_code)] mod test_network;`(12).

### recover_replay.rs — 5 tests on `ValidationMode::Replay`
Disaster-recovery replay path. Output Contract documented in-file (lines 3-16): chain_state, utxo_set, producer_set, epoch_state, block_store, state_db, mempool(must stay empty), network(must be None), sync_manager.

| Function | What it tests |
|---|---|
| `replay_produces_identical_state` | 20-block chain normal → snapshot → wipe state_db → replay → state matches exactly |
| `replay_skips_dedup_check` | Blocks already in store are NOT skipped in Replay mode |
| `replay_suppresses_side_effects` | network=None, mempool empty, recovery_mode=false after replay |
| `replay_produces_undo_data` | Undo data exists after replay (broken `recover` produced none) |
| `replay_handles_epoch_boundaries` | 2+ epoch chain produces correct `bond_snapshot`/`producer_list` |

### m_rc9_silent_vec_regression.rs (INC-I-034)
Reproduces 2026-04-16 mainnet cascade: `calculate_epoch_rewards` (`bins/node/src/node/rewards.rs:41-69` as of the incident) silently skipped missing blocks in `block_store` via `if let Ok(Some(block))`, undercounting attestation and desyncing EpochReward outputs. Output Contract: `Vec<(u64, Hash)>` — 4 outputs × 6 paths = 24 assertion cells. Paths: `complete_body_bitfield`, `gap_in_middle`, `many_missing_mainnet`.

### m_rc10_apply_after_reject_regression.rs (INC-I-034)
Reproduces 2026-04-16 05:11 UTC santiago cascade: same block REJECTED by one validation path (`ECON_EPOCH_NOT_BOUNDARY`) and half-APPLIED by another (`apply_block/mod.rs:75-83` logs "Applied" before transaction processing runs) — apply-after-reject desync.

### m_rc11_fork_guard_backfill_regression.rs (INC-I-034, REQ-REDESIGN-011)
Reproduces the FORK_GUARD block_store gap from the same 2026-04-16 cascade: `execute_reorg` calling `get_block_by_height(target_height)` on a missing block returns `Ok(None)` and silently substitutes `genesis_hash` as the common ancestor — wrong rollback point.

### inc_i_053_epoch_startup_gate.rs
Tests `Node::should_defer_epoch_production()` — restart grace period preventing mass-restart forks (epoch mode had no sync-before-produce guard, unlike bootstrap mode's `scheduling.rs:90-167`). P1: `first_peer_connected=None`→false. P2: `Some(recent)`→true (defer). P3: old enough→false (grace expired).

### inc_i_061_delegator_reward_address.rs
`calculate_epoch_rewards` must use `hash_with_domain(ADDRESS_DOMAIN, pubkey)` (wallet address) for reward outputs, NOT `hash(pubkey)` (ProducerSet key) — delegator rewards were going to unreachable addresses. P1: delegation A→B, both addresses correct. P2: no-delegation regression anchor. Uses `DOLI_BLOCKS_PER_REWARD_EPOCH=36` env var + `std::sync::Once`.

### inc_i_064_supply_conservation.rs (INC-I-064)
Output Contract in-file: O1 apply_block Err on non-Replay spend failure, O2 Light-mode economics rejection, O3 Replay tolerates spend failures, O4 fee-paying TXs accepted (fees burn, deflationary), O5 correct UTXO delta=coinbase. Paths: Full | Light | Replay modes. Matrix includes `spend_failure_propagates_in_light_mode` etc.

### inc_i_027_utxo_restore_selfheal.rs [UNCLEAR — could not verify]
Referenced by the prior skill (tests `init_utxo_set` rebuild-on-mismatch behavior). **Could not confirm this file still exists** in this session — `Read` failed both at the documented path and a plausible rename (`inc_i_027_utxo_selfheal.rs`), and directory listing was unavailable (see PATTERNS note on tooling). Verify with `ls bins/node/tests/inc_i_027*` before citing.

### INC-I-138 (self-fork recovery starvation + SnapSync escalation, committed `6f6714a4`)
No dedicated test file could be located under plausible names (`inc_i_138_self_fork_starvation.rs`, `inc_i_138_snapsync_escalation.rs` both confirmed absent). The fix likely lives inside `fork_recovery.rs` or `bins/node/src/node/fork_recovery.rs` directly — **directory listing was unavailable this session (see PATTERNS)**; re-verify with a working `rg`/`fd`/`ls` before assuming no regression test exists for this incident.

---

## SIMULATION-TESTS

All live in `testing/simulation/existential_risks/`. **Not a separate Cargo package** — no `Cargo.toml` in `testing/simulation/` or the `existential_risks/` subdir, and neither is a workspace member. Same status as `testing/e2e/`: **cannot be compiled/run by any `cargo test` today.** Treat as design-validation reference. Pure Rust, no async, real `ProducerSet`/`RegistrationQueue`/`KeyPair`.

**CORRECTION vs prior skill**: the file→"Prueba N" numbering had drifted; re-verified directly from file headers 2026-07-09.

### mod.rs (117 lines) — shared infrastructure
Declares 7 submodules: `aristocracy`, `early_attacker`, `economics`, `infrastructure`, `infiltration`, `onboarding`, `producer_lifecycle` (line 15-21).
Constants: `DEVNET_BLOCKS_PER_YEAR=12`(41), `DEVNET_BLOCKS_PER_MONTH=1`(42), `MAX_REGISTRATIONS_PER_BLOCK=5`(46).
`mod thresholds`(49-69): `QUEUE_WAIT_ALERT=10`, `QUEUE_WAIT_CRITICAL=60`, `FEE_MULTIPLIER_ALERT=5.0`, `FEE_MULTIPLIER_CRITICAL=100.0`, `ABANDONMENT_RATE_CRITICAL=0.50`, `GINI_HEALTHY=0.3`, `GINI_CONCERNING=0.5`, `TOP5_HEALTHY=0.20`, `TOP5_CONCERNING=0.35`, `FOUNDERS_HEALTHY=0.15`, `FOUNDERS_CONCERNING=0.25`, `ATTACK_COST_CRITICAL=$500K`, `ATTACK_COST_RISKY=$2M`, `ATTACK_COST_ACCEPTABLE=$10M`.
Helpers: `calculate_gini(values)`(86), `calculate_fee_multiplier(queue_length)`(113, uses `fee_multiplier_x100` from `doli_core::consensus`).

### onboarding.rs — PRUEBA 1: Liveness stress test
`LivenessMetrics` struct. Tests viral-growth registration queue behavior against `QUEUE_WAIT_*`/`FEE_MULTIPLIER_*`/`ABANDONMENT_RATE_CRITICAL` thresholds.

### aristocracy.rs — PRUEBA 2: Power concentration ("Elite simulation")
`AristocracyMetrics` struct (gini, top5_concentration, founders_percentage, top33_avg_age_years). Uses `producer_weight_for_network`/`MAX_WEIGHT` from `storage`.

### infiltration.rs — PRUEBA 3: Slow infiltration attack
`AttackSimulationResult` struct. Tests gradual attacker accumulation toward 33% stake threshold.

### early_attacker.rs — PRUEBA 4: Early active attacker
`EarlyAttackerResult` struct. Tests whether founders/early-joiners have unfair long-term seniority advantage.

### infrastructure.rs — PRUEBA 5, 6, 7 (three simulations in one file)
PRUEBA 5: VDF Verification Throughput (P0-Critical). PRUEBA 6: Fork Choice Under Partition. PRUEBA 7: Network Liveness Simulation.
**Correction**: prior skill described this file as "Prueba 6: Infrastructure concentration" only — actual content is VDF throughput + fork choice + liveness, not infrastructure/hosting concentration.

### producer_lifecycle.rs — PRUEBA 8, 9
PRUEBA 8: Zombie Producer Behavior (`test_zombie_producer_behavior`, line 13) — uses `ActivityStatus`, `INACTIVITY_THRESHOLD`, `REACTIVATION_THRESHOLD` from `storage`. PRUEBA 9: Producer Exit/Cancel Flow.
**Correction**: prior skill described this file generically as "full producer lifecycle: register→activation→active→withdrawal→exit" — actual content is specifically zombie/reactivation and exit/cancel, not the bond withdrawal flow (that's covered in `bond_stacking.rs`).

### economics.rs — PRUEBA 10, 11 + Dashboard Summary
PRUEBA 10: Reward Distribution Fairness (P1-Important). PRUEBA 11: Era Transition / Halving. Plus a Dashboard Summary section.
**Correction**: prior skill described this file as "Prueba 3: Economic security / attack cost" — that content is NOT here; it's `infiltration.rs` (Prueba 3) that covers attack-cost-adjacent slow-infiltration economics. `economics.rs` is reward-fairness + halving.

---

## BENCHMARKS

`testing/benchmarks/src/main.rs` (workspace member, standalone binary `vdf-benchmark`, not `cargo test`).

Commands (clap `Cli`/`Commands` enum, lines 11-34):
- `vdf-benchmark compute [--iterations N] [--t-value T]` — benchmarks hash-chain VDF at T iterations
- `vdf-benchmark full` — runs full benchmark suite

Key function: `hash_chain_vdf(input: &Hash, t: u64) -> Hash` (in `vdf` crate) — `state = BLAKE3(state)` repeated t times (NOT the production consensus VDF path — DOLI does not use VDF in production per `feedback_no_vdf.md`; this is purely a benchmark harness measuring hash-chain cost). Default T: `T_BLOCK` from `vdf` crate = 800_000 iterations ≈ 55ms.

---

## OTHER-CRATE-TESTS

`crates/*/tests/**/*.rs` outside `bins/node/tests/` — **this session's tooling could not enumerate these directories** (see PATTERNS note). Only one file was confirmed via an explicit CLAUDE.md cross-reference:

### crates/channels/tests/inc_i_092_close_covenant.rs (INC-I-092 RC-C ground-truth)
Answers: does a correctly-signed channel cooperative-close PASS the consensus covenant evaluator (`validate_transaction_with_utxos`)? Funding output is 2-of-2 multisig (`channels/src/conditions.rs::funding_condition`) — cooperative close requires BOTH parties' signatures by design. Output Contract documented in-file (lines 16-30): O1 = `Result<(), ValidationError>` for a Transfer spending a 2-of-2 Multisig funding UTXO. P1: both sign → `cooperative_close_with_both_sigs_passes_consensus`. P2: one signs → `cooperative_close_with_one_sig_fails_consensus`. Confirms the MPTX007 stress-test failures were single-party USAGE artifacts, not a consensus/witness-encoding bug (see CLAUDE.md "If You Touch" RC-C note).

**Gap**: `crates/core/tests/`, `crates/storage/tests/`, `crates/network/tests/`, `crates/mempool/tests/`, `crates/rpc/tests/`, `crates/updater/tests/`, `crates/wallet/tests/`, `crates/bridge/tests/` were NOT enumerated this session (tooling failure — see PATTERNS). Re-run with working `Glob`/`Grep` to complete this section.

---

## TEST-UTILITIES

### common/mod.rs (`testing/common/mod.rs`, 400 lines) — shared test infrastructure
Included via `#[path = "../common/mod.rs"] mod common;` from `testing/integration/*.rs`, `testing/e2e/*.rs`, and `testing/simulation/existential_risks/mod.rs` (`#[path = "../../common/mod.rs"]`).

**`TestNodeConfig`** (line 20):
```rust
pub struct TestNodeConfig {
    pub data_dir: PathBuf,    // temp_dir/node_{port_offset}
    pub listen_port: u16,     // 30300 + port_offset
    pub rpc_port: u16,        // 8500 + port_offset
    pub bootstrap_nodes: Vec<String>,
    pub producer_key: Option<KeyPair>,
}
```
`TestNodeConfig::new(temp_dir, port_offset)`(29), `.with_producer_key(keypair)`(39), `.with_bootstrap(addr)`(44).

**`TestNode`** (line 51) — in-memory test node (NOT the production `Node`):
```rust
pub struct TestNode {
    pub config: TestNodeConfig,
    pub chain_state: Arc<RwLock<ChainState>>,
    pub utxo_set: Arc<RwLock<UtxoSet>>,
    pub blocks: Arc<RwLock<HashMap<Hash, Block>>>,
    pub mempool: Arc<RwLock<Vec<Transaction>>>,
    pub keypair: KeyPair,
    pub params: ConsensusParams,  // ConsensusParams::mainnet()
}
```
Methods (all async): `height()`(81), `best_hash()`(86), `get_block(hash)`(91), `add_block(block)`(96, validates prev_hash except genesis, updates UTXO set + chain_state), `revert_blocks(count)`(156), `add_to_mempool(tx)`(197, 10k cap), `mempool_size()`(207), `clear_mempool()`(212).

**Block construction helpers**: `create_test_block(height, prev_hash, producer, transactions)`(218, `ConsensusParams::mainnet()`, timestamp=`genesis_time + slot*slot_duration`), `create_coinbase(height, recipient, amount)`(252), `create_transfer(inputs: Vec<(Hash,u32)>, outputs: Vec<(Amount,Hash)>, keypair)`(257, signs per-input via `signing_message_for_input(i)`). **NOTE**: `create_transfer`'s real signature takes exactly these 3 args — some orphaned integration tests (`attack_reorg_test.rs`) call it with a different 7-arg shape and would not compile if re-wired (see INTEGRATION-TESTS drift note).

**Chain generation**: `generate_test_chain(length, producer, initial_reward)`(310) → `Vec<Block>`, linked chain with coinbase outputs.

**Async utilities**: `wait_for(condition_fn, timeout, poll_interval)`(286) → polls until timeout; `init_test_logging()`(302), sets `tracing_subscriber` with `doli=debug` filter once.

**`MockPeer`** (line 335): `id`, `connected`, `blocks_sent`, `blocks_received`. Methods: `connect()`(352), `disconnect()`(356), `send_block(block)`(360, only if connected), `receive_block(block)`(366).

**Self-test in common/mod.rs**: `test_test_node_basics`(378), `test_generate_chain`(387).

### Node::new_for_test (production Node, `bins/node/src/node/init.rs`)
`Node::new_for_test(data_dir: PathBuf, producers: Vec<KeyPair>) -> Result<Node, _>` — creates a real node with devnet params, real RocksDB, registered producers, genesis block applied. Used by every file in `bins/node/tests/`. Async — always `.await`.

### TestNetwork (production-grade harness, `bins/node/tests/test_network.rs:26`)
Wraps real `Node::new_for_test()` instances in `Arc<Mutex<Node>>` for parallel gossip simulation. See NODE-TESTS section above for full API. This is the harness to reach for when a test needs MULTIPLE real nodes exchanging blocks (vs. `common::TestNode`, which is a single-node in-memory stub used by the orphaned/legacy `testing/integration` and `testing/e2e` files).

---

## PATTERNS

### Pattern 1: Wired integration test setup (testing/integration/, only the 7 WIRED files)
```rust
#[path = "../common/mod.rs"]
mod common;
use common::{create_coinbase, create_test_block, generate_test_chain,
             init_test_logging, TestNode, TestNodeConfig};

#[tokio::test]
async fn test_foo() {
    init_test_logging();
    let temp_dir = TempDir::new().unwrap();
    let config = TestNodeConfig::new(&temp_dir, /* port_offset */ 0);
    let node = Arc::new(TestNode::new(config));
    // ...
}
```
Use unique port offsets per test to avoid clashes. **Before adding a new file here, add its `[[test]]` entry to `testing/integration/Cargo.toml` — files are NOT auto-discovered.**

### Pattern 2: Real node tests (bins/node/tests/)
```rust
async fn make_node(n_producers: usize) -> (Node, Vec<KeyPair>, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let node = Node::new_for_test(temp.path().to_path_buf(), producers.clone()).await.unwrap();
    (node, producers, temp)
}

fn build_block(height, slot, prev_hash, producer, params) -> Block {
    // coinbase → reward_pool_pubkey_hash()
    // ChainSpec::devnet().genesis_hash()
    // VdfOutput { value: vec![0u8; 32] }, VdfProof::empty()
    // BlockHeader needs: version, prev_hash, merkle_root, presence_root,
    // genesis_hash, timestamp, slot, producer, vdf_output, vdf_proof,
    // missed_producers, data_root, fork_id  — ALL fields required, see
    // malicious_peer.rs drift note for what happens when a field is missing.
}

async fn apply_chain(node: &mut Node, blocks: &[Block]) {
    for block in blocks {
        node.apply_block(block.clone(), ValidationMode::Light).await.unwrap();
    }
}
```

### Pattern 3: Fork + rollback + canonical
```rust
let base = build_chain(1, 1, Hash::ZERO, &producers[0], N, &params);
apply_chain(&mut node, &base).await;
let fork = build_chain(N+1, N+1, base.last().hash(), &producers[0], M, &params);
apply_chain(&mut node, &fork).await;
for _ in 0..M { node.rollback_one_block().await.unwrap(); }
let canonical = build_chain(N+1, N+1, base.last().hash(), &producers[1], K, &params);
apply_chain(&mut node, &canonical).await;
assert_eq!(node.chain_state.read().await.best_height, N + K);
assert_eq!(node.chain_state.read().await.best_hash, canonical.last().hash());
```

### Pattern 4: Output Contract Checklist (mandatory for new tests, per CLAUDE.md protocol #21)
Before writing assertions, list ALL observable outputs, ALL paths, ALL input partitions. See `.claude/protocols/output-contract.md`. Real in-tree examples: `bins/node/tests/inc_i_064_supply_conservation.rs` (O1-O5 × 3 modes), `bins/node/tests/recover_replay.rs` (9 observable outputs listed in header comment), `crates/channels/tests/inc_i_092_close_covenant.rs` (O1 × P1/P2 × IP1/IP2).

### Pattern 5: TestNetwork multi-node propagation (bins/node/tests/test_network.rs)
```rust
let net = TestNetwork::new(3 /* nodes */, 2 /* producers */).await;
net.produce_blocks(10, 0).await;               // producer index 0
net.partition(&[0], &[1, 2]);
net.heal();
assert!(net.is_synced().await);
```
For realistic (lossy) gossip, use `net.gossip_realistic(&block, delivery_probability)` which routes through `check_producer_eligibility` — the actual production gossip gate — not just `apply_block` directly.

### Pattern 6: Equivocation detection
```rust
let mut detector = EquivocationDetector::new();
assert!(detector.check_block(&block_a).is_none());   // first block → no equivocation
let proof = detector.check_block(&block_b);           // same slot, different hash → Some
let slash_tx = proof.unwrap().to_slash_transaction(&reporter);
assert!(slash_tx.is_slash_producer());
let pending = detector.take_pending_proofs();         // drains the queue
```

### Pattern 7: Epoch reward calculation invariants
- Pool = sum of `block_reward(height)` for each block in epoch.
- Fair share = pool / n_producers (integer division); remainder → index-0 producer (sorted by pubkey bytes).
- Total distributed must EXACTLY equal pool (no dust lost).
- EpochReward UTXOs require `REWARD_MATURITY=6` confirmations before spendable (same as coinbase).

### Pattern 8: UTXO maturity in tests
- `entry.is_spendable_at_for_network(height, Network::Mainnet)` — use devnet in fast tests.
- Coinbase: `created_height + 6 <= check_height`. EpochReward: same 6-block maturity. Regular output: spendable immediately.
- `UtxoEntry` requires 4 fields: `output, height, is_coinbase, is_epoch_reward` — omitting the 4th is a compile error against current `storage::UtxoEntry` (see E2E-TESTS drift note).

### Pattern 9: Environment tooling gap encountered this session (2026-07-09)
The `Glob` and `Grep` tools both failed with `ENOENT: posix_spawn 'rg'` for the entire session — `ripgrep` was not resolvable in `PATH`. All file discovery in this update was done via targeted `Read` calls against paths named in the prior skill file, `CLAUDE.md`, and `MEMORY.md`; new/renamed files that aren't referenced by name anywhere could not be discovered. **If re-running this analysis, first confirm `rg` is on `PATH`** (or use a shell tool directly) before trusting any "file does not exist" conclusion in this document — several are asserted from failed `Read` attempts on guessed paths, not from an authoritative directory listing.

### Key invariants to verify after any reorg
1. `node.chain_state.best_height` and `best_hash` match the last applied block.
2. UTXO count reflects only UTXOs created by blocks still in chain.
3. `cumulative_rollback_depth` resets to 0 after applying a synced block.
4. `shallow_rollback_count` resets to 0 after clean continuation.
5. `snap_sync_height` is `None` after epoch boundary passes.
