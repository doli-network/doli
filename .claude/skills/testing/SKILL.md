# testing — DOLI Test Infrastructure
<!-- OUTPUT CONTRACT: N/A — skill/reference file, not a test file -->
<!-- INPUT PARTITIONS: N/A — skill/reference file, not a test file -->
<!-- @INDEX
INTEGRATION-TESTS: lines 15-169
E2E-TESTS: lines 170-194
FUZZ-TARGETS: lines 195-214
NODE-TESTS: lines 215-345
SIMULATION-TESTS: lines 346-383
BENCHMARKS: lines 384-396
TEST-UTILITIES: lines 397-468
PATTERNS: lines 469-582
-->

## INTEGRATION-TESTS

All live in `testing/integration/`. Each file includes common via `#[path = "../common/mod.rs"] mod common;`.

### two_node_sync.rs
Tests basic block propagation and sync between two TestNode instances.

| Function | Line | What it tests |
|---|---|---|
| `test_two_nodes_sync_basic` | 22 | Node B syncs all 20 blocks from Node A; asserts height and best_hash equality |
| `test_incremental_sync` | 64 | Both share 10 blocks; Node A adds 5 more; Node B catches up one at a time |
| `test_large_sync_gap` | 120 | Node A has 100 blocks, Node B has 10; syncs remaining 90 |
| `test_duplicate_block_handling` | 157 | Adding same block twice returns `Err("already exists")`; chain state unchanged |
| `test_sync_multiple_producers` | 186 | 15 blocks from 3 rotating producers; both nodes converge on same tip |
| `test_utxo_sync` | 240 | 10 coinbase blocks on both nodes; asserts `utxo_set.len() == 10` on each |
| `test_reject_future_blocks` | 275 | Documents that production would reject far-future slot blocks |
| `test_concurrent_block_additions` | 301 | 50 blocks added via concurrent tokio tasks; asserts `height > 0` |
| `test_chain_tip_tracking` | 340 | Each block updates `best_hash` to the new block hash |

### reorg_test.rs
Tests chain reorganization mechanics on a single TestNode.

| Function | Line | What it tests |
|---|---|---|
| `test_single_block_reorg` | 20 | Revert 1 block; add competing block; tip changes to competing |
| `test_deep_reorg_10_blocks` | 66 | Revert 10; build competing chain of 12; node follows longer chain |
| `test_very_deep_reorg` | 115 | Revert 15; build competing chain of 20; confirms height=34 |
| `test_utxo_consistency_during_reorg` | 162 | UTXOs count correctly: 10 → 5 (after revert) → 12 (after new chain) |
| `test_multiple_sequential_reorgs` | 211 | Three successive reorgs; each produces distinct tip hash |
| `test_reorg_different_producers` | 280 | After reorg, tip block's `producer` field matches the new producer's pubkey |
| `test_reorg_chain_integrity` | 321 | `prev_hash` links verified through all blocks after reorg |
| `test_reorg_equal_length` | 365 | Reorg to same-length chain produces new tip != old tip |
| `test_empty_revert` | 408 | `revert_blocks(0)` returns empty vec; state unchanged |

### partition_heal.rs
Simulates network partitions: two groups build divergent chains, then one heals.

| Function | Line | What it tests |
|---|---|---|
| `test_partition_separate_chains` | 20 | Node A builds 10, Node B builds 8 after common genesis; B reverts and adopts A's chain |
| `test_heal_longer_partition_chain` | 94 | B has longer chain (20) than A (10) after fork; A reverts and syncs B's chain |
| `test_three_way_partition` | 157 | 3 nodes diverge (5/8/12 blocks); all converge on C's longest (12) |
| `test_partition_utxo_reconciliation` | 243 | UTXOs reconcile correctly: both nodes reach 10 after heal |
| `test_partition_large_length_difference` | 324 | B has 3 blocks vs A's 100; B reverts and fully syncs |
| `test_mock_peer_partition` | 382 | MockPeer connect/disconnect; blocks sent while disconnected are lost |
| `test_gradual_healing` | 431 | Node re-syncs 15 blocks one at a time; height increments verified each block |

### epoch_rewards.rs
Unit and integration tests for the Pool-First epoch reward distribution.

| Function | Line | What it tests |
|---|---|---|
| `test_fair_share_calculation_even_split` | 26 | 300 DOLI / 3 producers = 100 DOLI each, remainder=0 |
| `test_fair_share_calculation_with_remainder` | 39 | 100_000_000_001 / 3 = 33_333_333_333 + remainder=2, first gets dust |
| `test_fair_share_single_producer` | 59 | Single producer gets 100% of pool |
| `test_fair_share_many_producers` | 72 | 1000 DOLI / 100 producers = 10 DOLI each |
| `test_epoch_reward_transaction_creation` | 94 | `TxType::EpochReward`, no inputs, 1 output, correct amount and pubkey_hash |
| `test_epoch_reward_transaction_data` | 111 | `epoch_reward_data()` returns correct epoch and recipient |
| `test_epoch_reward_has_correct_type` | 128 | type=EpochReward(10), `is_epoch_reward()=true`, `is_coinbase()=false` |
| `test_epoch_reward_utxo_maturity` | 145 | Not spendable < 6 confirmations; spendable at exactly 6 |
| `test_coinbase_maturity_unchanged` | 172 | Coinbase also requires 6 confirmations |
| `test_regular_tx_no_maturity` | 193 | Regular output spendable at same height |
| `test_pool_accumulation_over_epoch` | 218 | pool = block_reward * epoch_length |
| `test_epoch_total_matches_distribution` | 233 | first_producer + others = pool exactly |
| `test_producer_sorting_deterministic` | 261 | Sorting by pubkey bytes is deterministic |
| `test_first_producer_gets_remainder` | 279 | Remainder assigned to index-0 producer (sorted by pubkey) |
| `test_reward_mode_epoch_pool_is_default` | 314 | `RewardMode::default() == RewardMode::EpochPool` |
| `test_consensus_params_reward_mode` | 320 | All networks (mainnet/testnet/devnet) use EpochPool |
| `test_epoch_boundary_detection` | 332 | Slot 0 is NOT boundary; multiples of epoch_length are |
| `test_epoch_reward_minimum_amount` | 357 | Amount=1 base unit is valid |
| `test_epoch_reward_large_epoch_number` | 374 | `u64::MAX` epoch number survives round-trip |
| `test_reward_maturity_constant` | 386 | `REWARD_MATURITY == 6` |
| `test_utxo_set_add_epoch_reward` | 396 | `add_transaction` marks is_epoch_reward=true, height=100 |
| `test_utxo_set_epoch_reward_balance` | 422 | `get_balance` returns 0 before maturity, 100 DOLI after |
| `test_producer_block_after_empty_boundary` | 449 | Empty boundary slot → first block of next epoch carries rewards |
| `test_empty_epoch_produces_no_rewards` | 481 | pool=0 when epoch has zero blocks |
| `test_multi_epoch_catchup_order` | 503 | last_rewarded=2 at epoch 5 → rewards epoch 3, then 4, then 5 |
| `test_epoch_reward_slot_range_calculation` | 541 | Epoch N covers slots [N*epoch_len, (N+1)*epoch_len) |
| `test_proportional_rewards_rounding` | 581 | 7/10/2/10/1/10 split: 70M/20M/10M; total=pool |
| `test_epoch_rewards_with_odd_distribution` | 622 | 100 / 3 = 33+33+34 (last gets dust) |

### attack_reorg_test.rs
Security tests: double-spend, selfish mining, long-range attack, nothing-at-stake, eclipse, Sybil.

| Function | Line | What it tests |
|---|---|---|
| `test_double_spend_attack_via_reorg` | 31 | 1-confirmation TX wiped by 3-block reorg; illustrates confirmation requirement |
| `test_confirmation_depth_protection` | 151 | 10-confirmation TX survives shallow reorg; attacker needs 12 blocks |
| `test_selfish_mining_withholding` | 196 | Attacker's withheld block vs honest block; first-seen/hash tiebreaker documented |
| `test_selfish_mining_with_lead` | 253 | 3-block secret lead beats honest 2-block chain |
| `test_long_range_attack_from_genesis` | 324 | Attacker rewrites 40 blocks; checkpoint need documented |
| `test_checkpoint_prevents_long_range_attack` | 378 | `config.checkpoint_height` blocks revert to checkpoint, not past it |
| `test_nothing_at_stake_multiple_forks` | 416 | `detect_equivocation()` helper verifies same height/producer/different hash |
| `test_future_timestamp_attack` | 503 | Block with timestamp+3600s → `validate_block_timestamp` returns err/false |
| `test_past_timestamp_attack` | 538 | Block with timestamp before parent → validation error |
| `test_finality_after_confirmations` | 587 | 100-confirmation depth documented; VDF sequential time cost calculated |
| `test_shallow_reorg_preserves_finality` | 628 | Reorg of 3 blocks leaves block 10 (10 confirmations) in chain |
| `test_sybil_reorg_resistance` | 681 | 10 sybil identities can build longer chain but still at VDF cost |
| `test_eclipse_attack_with_reorg` | 741 | Eclipsed node accepts fake chain; heals to longer honest chain |
| `test_rapid_reorg_attempts` | 818 | 50 rapid reorgs (depth 1-5); node remains functional |
| `test_concurrent_reorg_attempts` | 875 | 5 attackers forking from different points; only one chain is canonical |

### equivocation_slashing.rs
Tests the full equivocation detection and slashing pipeline.

| Function | Line | What it tests |
|---|---|---|
| `test_equivocation_detection_basic` | 82 | Two different blocks for same slot triggers `EquivocationDetector::check_block` → `Some(proof)` |
| `test_same_block_twice_not_equivocation` | 123 | Same block twice → `None`; `has_pending_proofs() == false` |
| `test_different_slots_not_equivocation` | 145 | Blocks at slots 10 and 11 → `None` |
| `test_different_producers_same_slot_not_equivocation` | 167 | Different producers at same slot → `None` |
| `test_proof_contains_vdf_verifiable_headers` | 191 | Proof has full `BlockHeader` structs with correct slot/producer fields |
| `test_proof_to_slash_transaction` | 223 | `proof.to_slash_transaction(&reporter)` creates `SlashProducer` TX with evidence |
| `test_slashing_updates_producer_status` | 270 | `slash_producer()` → `ProducerStatus::Slashed`; honest producer unaffected |
| `test_slashed_producer_cannot_reregister` | 307 | Slashed status persists; re-registration blocked |
| `test_multiple_equivocations_detected` | 333 | Two equivocations in different slots → 2 pending proofs |
| `test_equivocation_slashing_e2e` | 366 | Full pipeline: detect → proof → slash TX → ProducerSet update → 2 active remain |
| `test_detector_eviction_memory_bounded` | 469 | After 2000 entries, `tracked_count() <= 1000` |

### mempool_poison.rs
Validates mempool purge-by-error-pattern (testnet incident 2026-03-25: NFT re-injection froze production).

Key helper: `make_signed_nft_tx(keypair, funding_hash)` — builds a fully-structured EncryptedContent TX with correct extra_data encoding (condition=34B + ciphertext_len=4B + wrapped_key=80B + nonce=12B + content_hash=32B).

### mempool_stress.rs (testing/integration/)
| Function | Line | What it tests |
|---|---|---|
| `test_mempool_10k_sequential` | 52 | 10,000 TXs added sequentially; `mempool_size() == 10_000`; logs per-TX time |
| `test_mempool_10k_concurrent` | 78+ | 10,000 TXs added via concurrent tokio tasks |

### presence_manipulation_test.rs
| Function | Line | What it tests |
|---|---|---|
| `test_presence_root_affects_block_hash` | 36 | Two headers differing only in `presence_root` → different `header.hash()` |
| `test_presence_root_zero_vs_nonzero` | 58+ | Legacy blocks (presence_root=ZERO) still produce distinct hashes from non-zero |

### bond_stacking.rs
Tests bond stacking system: constants, vesting, FIFO withdrawal, penalty calculation.
- Constants: `BOND_UNIT=1_000_000_000`, `MAX_BONDS_PER_PRODUCER=3_000`, `YEAR_IN_SLOTS=3_153_600`

### two_producer_pop.rs
Tests simplified PoP (Proof of Presence) model with 2 producers alternating.
- Uses `ProducerState::new()`, `INITIAL_PRESENCE_SCORE`, `SCORE_PRODUCE_BONUS`, `SCORE_MISS_PENALTY`
- Uses `generate_genesis_block(&GenesisConfig::devnet())`

### staggered_validator_rewards.rs
Tests that producers joining mid-epoch receive no rewards for that epoch.
Uses `EpochRewardTracker` (local helper struct) to simulate proportional reward distribution tracking.

### malicious_peer.rs
Tests node resilience against peers sending blocks with: wrong prev_hash, bad merkle root, bad version.
Helpers: `create_block_wrong_prev_hash`, `create_block_bad_merkle`, `create_block_bad_version`.

---

## E2E-TESTS

All live in `testing/e2e/`. Include common via `#[path = "../common/mod.rs"] mod common;`.

### full_cycle.rs
| Function | Line | What it tests |
|---|---|---|
| `test_genesis_to_1000_blocks` | 27 | 1000 blocks applied; height=999; UTXOs=1000; logs throughput |
| `test_full_cycle_with_reorg` | 68+ | 500 blocks, then reorg, then continue; full chain integrity |

### wallet_flow.rs
Uses local `TestWallet` struct with UTXO tracking and coin selection.

`TestWallet` API:
- `TestWallet::new()` — generates a `KeyPair`, derives `pubkey_hash`
- `wallet.address()` → `Hash` (pubkey_hash)
- `wallet.balance()` — sums all tracked UTXO amounts
- `wallet.add_utxo(outpoint, entry)` / `remove_utxo(outpoint)`
- `wallet.select_utxos_for_amount(target)` — greedy coin selection
- `wallet.create_send_transaction(recipient, amount, fee)` — signs inputs

Tests cover: receive from coinbase, send to another wallet, verify change output, multi-hop payment chains.

---

## FUZZ-TARGETS

All live in `testing/fuzz/targets/`. Use `libfuzzer_sys::fuzz_target!`. No `#[test]` — run with `cargo fuzz`.

| File | Input | Invariants checked |
|---|---|---|
| `merkle.rs` | `Vec<FuzzTransaction>` (up to 100, via `Arbitrary`) | `compute_merkle_root` does not panic; deterministic across two calls |
| `tx_deserialize.rs` | `&[u8]` raw bytes | `Transaction::deserialize` never panics; round-trip `serialize`→`deserialize` preserves hash |
| `block_deserialize.rs` | `&[u8]` raw bytes | `Block::deserialize` never panics |
| `hash.rs` | `&[u8]` raw bytes | `hash::hash` deterministic; incremental `Hasher` matches single call; `to_hex` is 64 chars |
| `signature.rs` | `FuzzInput { message, pubkey_bytes:[u8;32], sig_bytes:[u8;64] }` | `signature::verify` never panics for arbitrary pubkey/sig/message |

`FuzzTransaction` (merkle.rs):
```
struct FuzzTransaction { version: u32, tx_type: u8, num_outputs: u8, extra_data: Vec<u8> }
```
Maps tx_type % 3 → Transfer/Registration/Exit; num_outputs capped at 10.

---

## NODE-TESTS

All live in `bins/node/tests/`. Use real `Node::new_for_test()` with real RocksDB. No mocks. No networking. Blocks injected via `apply_block(block, ValidationMode::Light)`.

### Core test infrastructure (test_network.rs)

**`TestNetwork`** — simulated P2P network of real DOLI nodes:
```rust
pub struct TestNetwork {
    pub nodes: Vec<Arc<Mutex<Node>>>,
    _temps: Vec<TempDir>,
    pub producers: Vec<KeyPair>,
    pub params: ConsensusParams,
    pub connections: HashMap<usize, HashSet<usize>>,
    pub partitions: HashSet<(usize, usize)>,
    pub genesis_hash: Hash,
}
```

Key methods:
- `TestNetwork::new(n_nodes, n_producers)` — full-mesh topology; all nodes share same producer set
- `build_block(height, slot, prev_hash, producer)` — coinbase to reward pool, devnet genesis_hash
- `build_chain(start_height, start_slot, prev_hash, producer, count)` → `Vec<Block>`
- `apply_to_node(node_id, block)` → `Result<(), String>`
- `propagate(source, block)` — parallel apply to all connected peers; returns accepted count
- `produce_and_propagate(producer_idx)` — build+apply+propagate; returns `(Block, accepted_count)`
- `produce_blocks(count, producer_idx)` → final height
- `partition(group_a, group_b)` — disconnects cross-group links
- `heal()` — clears all partition links
- `height(node_id)` / `hash(node_id)` — async queries
- `is_synced()` — parallel check; all nodes same height+hash

### fork_recovery.rs — 10 tests on real Node

Common helpers:
- `make_node(n_producers)` → `(Node, Vec<KeyPair>, TempDir)` — calls `Node::new_for_test`
- `build_block(height, slot, prev_hash, producer, params)` — coinbase to pool, devnet genesis_hash
- `build_chain(start_height, start_slot, prev_hash, producer, count, params)` → `Vec<Block>`
- `apply_chain(node, blocks)` — sequential apply with Light validation, panics on error
- `devnet_genesis_hash()` → `ChainSpec::devnet().genesis_hash()`

| Function | Line | What it tests |
|---|---|---|
| `test_fork_recovery_with_divergent_bonds` | 108 | Fork with bond_snapshot divergence; canonical chain applied after rollback |
| `test_cumulative_rollback_resets_on_sync` | 151 | 49 rollbacks → depth=49; apply synced block → depth resets to 0 |
| `test_recovery_from_20_block_fork` | 190 | 20-block fork rolled back; 25-block canonical applied; height=35 |
| `test_recovery_with_scheduler_divergence` | 229 | bond_snapshot divergence; canonical 8-block chain applies despite it |
| `test_recovery_after_rollback_cap` | 268 | 50 rollbacks hit cap; 51st refused; synced block resets cap to 0 |
| `test_no_refork_after_recovery` | 310 | After recovery, 100 more blocks apply cleanly; `shallow_rollback_count==0` |
| `test_recovery_under_load` | 351 | 50-block fork reverted; 60-block canonical applied during/after recovery |
| `test_multiple_nodes_recover_independently` | 388 | 3 nodes each fork independently (5/7/3 blocks); all converge to 20-block canonical |
| `test_recovery_preserves_mempool` | 460 | 10 system TXs in mempool survive fork+rollback |
| `test_post_snap_gossip_validation_mode` | 512 | `snap_sync_height=Some(2)` → cleared at epoch boundary (h=4, devnet bpe=4) |

### epoch_reward_explicit_inputs.rs
Tests `EPOCH_REWARD_EXPLICIT_INPUTS_HEIGHT` (25,560) hard fork:
- Pre-activation: EpochReward has empty inputs, pool consumed by side-effect
- Post-activation: EpochReward has explicit sorted pool UTXO inputs

Uses `MockUtxoProvider` struct with `add_pool_utxo` / `add_non_pool_utxo` helpers.
Tests 1-6 are fast unit tests; tests 7-8 build full chain to activation height.

### epoch_state_regression.rs
Tests `EpochState` refactor (INC-I-035):
- `derive_at_boundary` produces correct `producer_list`/`active_list`
- `accumulate_block` per-block tracking matches expected attestation state
- `UndoData` round-trip: rollback restores exact `epoch_state`
- Multi-epoch accumulator rotation is correct

### checkpoint_rotation.rs
Tests auto-checkpoint rotation (INC-I-020): keeps 5 HIGHEST-height checkpoints (not lexicographically-last). Bug: `h526` sorted after `h4535` lexicographically. Uses `test_network` module via `#[allow(dead_code)] mod test_network;`.

### recover_replay.rs — 5 tests on `ValidationMode::Replay`
Tests disaster recovery replay path (`apply_block` with `ValidationMode::Replay`):

| Function | What it tests |
|---|---|
| `replay_produces_identical_state` | 20-block chain normal → snapshot → wipe state_db → replay → state matches exactly |
| `replay_skips_dedup_check` | Blocks in store are NOT skipped in Replay mode |
| `replay_suppresses_side_effects` | network=None, mempool empty, recovery_mode=false after replay |
| `replay_produces_undo_data` | Undo data exists after replay (broken `recover` produced none) |
| `replay_handles_epoch_boundaries` | 2+ epoch chain produces correct `bond_snapshot`/`producer_list` |

### m_rc9_silent_vec_regression.rs (INC-I-034)
Tests `calculate_epoch_rewards` for silent block_store gaps (2026-04-16 mainnet cascade).
Output Contract: `Vec<(u64, Hash)>` — 4 outputs × 6 paths = 24 assertion cells.
Paths: `complete_body_bitfield`, `gap_in_middle`, `many_missing_mainnet`.

### m_rc10_apply_after_reject_regression.rs (INC-I-034)
Reproduces apply-after-reject desync (2026-04-16 05:11 UTC cascade):
- Same block accepted by Light mode, rejected by Full mode
- `producer_liveness` mutated before tx processing fails → not reverted

### m_rc11_fork_guard_backfill_regression.rs (INC-I-034, REQ-REDESIGN-011)
Reproduces FORK_GUARD block_store gap (2026-04-16):
- `execute_reorg` uses `get_block_by_height(target_height)` → returns `Ok(None)` for missing block
- Silently substitutes `genesis_hash` for common ancestor → wrong rollback point

### inc_i_027_utxo_restore_selfheal.rs
Tests `init_utxo_set` (exported from `doli_node::node`):
- `rebuilt_when_len_mismatches_state_db` — stale utxo_store rebuilt from state_db
- `used_as_is_when_matches_state_db` — no rebuild when counts and contents match
- `migrated_when_empty` — existing empty-migration path unchanged

Helper: `make_utxo(tag, amount)` → `(Outpoint, UtxoEntry)`.

### inc_i_053_epoch_startup_gate.rs
Tests `Node::should_defer_epoch_production()` — 3 path tests:
- P1: `first_peer_connected=None` → false (no defer)
- P2: `first_peer_connected=Some(Instant::now())` → true (defer, recent)
- P3: `first_peer_connected` old enough → false (grace expired)

### inc_i_061_delegator_reward_address.rs
Tests `calculate_epoch_rewards` uses `hash_with_domain(ADDRESS_DOMAIN, pubkey)` (wallet address), NOT `hash(pubkey)` (ProducerSet key).
- P1: delegation A→B; delegator 90% and delegatee 10%+own go to wallet addresses
- P2: no delegation; all addresses correct (regression anchor)

Uses `DOLI_BLOCKS_PER_REWARD_EPOCH=36` env var; `std::sync::Once` for env init.

### inc_i_064_supply_conservation.rs
Tests supply conservation (INC-I-064):
- P-COINBASE: coinbase-only block; delta = +coinbase exactly
- P-FEE-TX: fee-paying TX; delta < +coinbase (fees burned)
- P-INFLATE: inflated output; MUST reject
- P-EPOCH: EpochReward TX; zero-sum redistribution
- P-BAD-SPEND: non-existent UTXO; spend_transaction fails first

Defects verified: silent `let _ = utxo.spend_transaction(tx)` (P0); ECON_EPOCH_INPUTS_MISMATCH gated behind Full mode only (P1); no post-block conservation invariant (P2).

---

## SIMULATION-TESTS

All live in `testing/simulation/existential_risks/`. Tests use real `ProducerSet`, `RegistrationQueue`, and `KeyPair`. No async. Pure Rust unit tests.

### mod.rs — shared infrastructure
Constants: `DEVNET_BLOCKS_PER_YEAR=12`, `DEVNET_BLOCKS_PER_MONTH=1`, `MAX_REGISTRATIONS_PER_BLOCK=5`.
Alert thresholds: `GINI_HEALTHY=0.3`, `GINI_CONCERNING=0.5`, `TOP5_HEALTHY=0.20`, `ATTACK_COST_CRITICAL=$500K`.
Helpers:
- `calculate_gini(values: &[u64])` → Gini coefficient
- `calculate_fee_multiplier(queue_length)` → uses `fee_multiplier_x100` table

### onboarding.rs (Prueba 1: Liveness)
`LivenessMetrics`: avg_wait_blocks, max_wait_blocks, max_fee_multiplier, abandonment_rate, recovery_blocks.
- `test_onboarding_normal_growth` — 2 reg/min over 240 blocks; queue stays healthy
- Spike scenarios (viral growth) test queue wait time and fee multiplier alert thresholds

### aristocracy.rs (Prueba 2: Power concentration)
`AristocracyMetrics`: gini, top5_concentration, founders_percentage, top33_avg_age_years.
- `test_aristocracy_simulation` — 10 founders + 10 new/year over 20 years; measures Gini drift
- Uses `producer_weight_for_network` and `MAX_WEIGHT` from storage

### economics.rs (Prueba 3: Economic security / attack cost)
Tests attack cost floor and economic incentive alignment.

### infiltration.rs (Prueba 4: Slow infiltration)
Tests gradual attacker accumulation — how many registrations needed to reach 33% stake.

### early_attacker.rs (Prueba 5: Early attacker advantage)
Tests whether founders have unfair long-term advantage due to seniority weighting.

### infrastructure.rs (Prueba 6: Infrastructure concentration)
Tests risk when multiple producers share same infrastructure.

### producer_lifecycle.rs
Tests full producer lifecycle: register → ACTIVATION_DELAY → active → withdrawal → exit.

---

## BENCHMARKS

`testing/benchmarks/src/main.rs` — standalone binary `vdf-benchmark` (not `cargo test`).

Commands:
- `vdf-benchmark compute [--iterations N] [--t-value T]` — benchmarks hash-chain VDF at T iterations
- `vdf-benchmark full` — runs full benchmark suite

Key function: `hash_chain_vdf(input: &Hash, t: u64) -> Hash` — `state = BLAKE3(state)` repeated t times (NOT the production VDF, just benchmark harness).
Default T: `T_BLOCK = 800_000` iterations ≈ 55ms.

---

## TEST-UTILITIES

### common/mod.rs — shared test infrastructure

**`TestNodeConfig`**:
```rust
pub struct TestNodeConfig {
    pub data_dir: PathBuf,    // temp_dir/node_{port_offset}
    pub listen_port: u16,     // 30300 + port_offset
    pub rpc_port: u16,        // 8500 + port_offset
    pub bootstrap_nodes: Vec<String>,
    pub producer_key: Option<KeyPair>,
}
```
- `TestNodeConfig::new(temp_dir, port_offset)` — derives ports from offset
- `.with_producer_key(keypair)` — chainable builder
- `.with_bootstrap(addr)` — chainable builder

**`TestNode`** — in-memory test node (NOT the production Node):
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
Key methods (all async):
- `height()` → `BlockHeight`
- `best_hash()` → `Hash`
- `get_block(hash)` → `Option<Block>`
- `add_block(block)` → `Result<(), String>` — validates prev_hash (except genesis), updates UTXO set, updates chain_state
- `revert_blocks(count)` → `Result<Vec<Block>, String>` — removes UTXOs created by reverted blocks
- `add_to_mempool(tx)` / `mempool_size()` / `clear_mempool()`

**Block construction helpers**:
- `create_test_block(height, prev_hash, producer, transactions)` → `Block` — uses `ConsensusParams::mainnet()`, timestamp from `genesis_time + slot * slot_duration`
- `create_coinbase(height, recipient, amount)` → `Transaction::new_coinbase(amount, recipient, height, 0)`
- `create_transfer(inputs, outputs, keypair)` → signed `Transaction::new_transfer()`; signs per-input via `signing_message_for_input(i)`

**Chain generation**:
- `generate_test_chain(length, producer, initial_reward)` → `Vec<Block>` — linked chain with coinbase outputs

**Async utilities**:
- `wait_for(condition_fn, timeout, poll_interval)` → `bool` — polls async condition until timeout
- `init_test_logging()` — sets up `tracing_subscriber` with `doli=debug` filter (once)

**`MockPeer`** — peer connection simulator:
```rust
pub struct MockPeer {
    pub id: String,
    pub connected: bool,
    pub blocks_sent: Vec<Block>,
    pub blocks_received: Vec<Block>,
}
```
Methods: `connect()`, `disconnect()`, `send_block(block)` (only if connected), `receive_block(block)`.

**Self-test in common/mod.rs**:
- `test_test_node_basics` (line 378) — height=0, mempool_size=0 after init
- `test_generate_chain` (line 387) — 10-block chain; `chain[i].header.prev_hash == chain[i-1].hash()`

**Node::new_for_test (production Node)**:
- `Node::new_for_test(data_dir: PathBuf, producers: Vec<KeyPair>)` — creates a real node with devnet params, real RocksDB, registered producers, genesis block applied
- Located at `bins/node/src/node/init.rs`
- Used by all `bins/node/tests/` files

---

## PATTERNS

### Pattern 1: Standard integration test setup
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

Use unique port offsets per test to avoid clashes.

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
}

async fn apply_chain(node: &mut Node, blocks: &[Block]) {
    for block in blocks {
        node.apply_block(block.clone(), ValidationMode::Light).await.unwrap();
    }
}
```

### Pattern 3: Fork + rollback + canonical
```rust
// 1. Apply base chain
let base = build_chain(1, 1, Hash::ZERO, &producers[0], N, &params);
apply_chain(&mut node, &base).await;

// 2. Apply fork
let fork = build_chain(N+1, N+1, base.last().hash(), &producers[0], M, &params);
apply_chain(&mut node, &fork).await;

// 3. Rollback M blocks
for _ in 0..M { node.rollback_one_block().await.unwrap(); }

// 4. Apply canonical
let canonical = build_chain(N+1, N+1, base.last().hash(), &producers[1], K, &params);
apply_chain(&mut node, &canonical).await;

assert_eq!(node.chain_state.read().await.best_height, N + K);
assert_eq!(node.chain_state.read().await.best_hash, canonical.last().hash());
```

### Pattern 4: Output Contract Checklist (mandatory for new tests)
Before writing assertions, list ALL observable outputs, ALL paths, ALL input partitions.
See `.claude/protocols/output-contract.md`. Example from inc_i_064:
- Outputs: (O1) apply_block returns Err, (O2) validation rejects, (O3) conservation detects inflation, (O4) Replay mode tolerates
- Paths: P-COINBASE, P-FEE-TX, P-INFLATE, P-EPOCH, P-BAD-SPEND
- Matrix: 4 outputs × 5 paths = 20 assertion cells

### Pattern 5: TestNetwork propagation
```rust
let mut net = TestNetwork::new(3 /* nodes */, 2 /* producers */).await;
// Produce and propagate blocks
net.produce_blocks(10, 0).await; // producer index 0
// Partition
net.partition(&[0], &[1, 2]);
// Each partition produces independently
// Heal
net.heal();
// Verify convergence
assert!(net.is_synced().await);
```

### Pattern 6: Equivocation detection
```rust
let mut detector = EquivocationDetector::new();
assert!(detector.check_block(&block_a).is_none());   // first block → no equivocation
let proof = detector.check_block(&block_b);           // same slot, different hash → Some
let slash_tx = proof.unwrap().to_slash_transaction(&reporter);
assert!(slash_tx.is_slash_producer());
let pending = detector.take_pending_proofs();         // drains the queue
```

### Pattern 7: Epoch reward calculation
- Pool = sum of `block_reward(height)` for each block in epoch
- Fair share = pool / n_producers (integer division)
- Remainder = pool % n_producers → goes to index-0 producer (sorted by pubkey bytes)
- Total distributed must EXACTLY equal pool (no dust lost)
- EpochReward UTXOs require `REWARD_MATURITY=6` confirmations before spendable

### Pattern 8: UTXO maturity in tests
- `entry.is_spendable_at_for_network(height, Network::Mainnet)` — use devnet in fast tests
- Coinbase: `created_height + 6 <= check_height`
- EpochReward: same 6-block maturity (`REWARD_MATURITY`)
- Regular output: spendable immediately (same height)

### Key invariants to verify after any reorg
1. `node.chain_state.best_height` and `best_hash` match the last applied block
2. UTXO count reflects only UTXOs created by blocks still in chain
3. `cumulative_rollback_depth` resets to 0 after applying a synced block
4. `shallow_rollback_count` resets to 0 after clean continuation
5. `snap_sync_height` is `None` after epoch boundary passes
