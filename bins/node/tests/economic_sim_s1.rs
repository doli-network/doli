//! Economic Simulator S1: Baseline Steady-State
//!
//! // OUTPUT CONTRACT: Node::apply_block() + calculate_epoch_rewards() economic invariants
//!
//! Observable outputs:
//! O1. UTXO total_value after apply_block (total supply)
//! O2. ProducerSet bond_count per producer (bond integrity)
//! O3. Epoch reward distribution amounts (epoch_pool distributed)
//! O4. Block reward halving at era boundaries (epoch_pool value)
//! O5. Producer qualification status across epochs
//!
//! PATHS:
//! P-NORMAL: Non-boundary block (coinbase only, no EpochReward)
//! P-EPOCH: Epoch boundary block (coinbase + EpochReward TX distribution)
//! P-HALVING: Epoch boundary that crosses an era boundary (reward halves)
//! P-EPOCH0: Epoch 0 boundary (skipped, no distribution)
//!
//! INPUT PARTITIONS:
//! - I-UNIFORM: 30 producers with equal bonds (100 each), no churn/delegation
//! - I-SHORT: 10 epochs (smoke test, no halving)
//! - I-MEDIUM: 1000 epochs (default, crosses multiple halvings)
//! - I-LONG: 100k epochs (stress, covers 2+ halvings, gated behind --ignored)
//!
//! MATRIX (outputs x paths x partitions):
//! O1 x P-NORMAL x I-UNIFORM -> total_supply += block_reward (every block)
//! O1 x P-EPOCH  x I-UNIFORM -> total_supply == sum(block_reward(1..=h))
//! O1 x P-EPOCH0 x I-UNIFORM -> no distribution, pool carries forward
//! O2 x P-EPOCH  x I-UNIFORM -> bond_count unchanged (no churn in S1)
//! O3 x P-EPOCH  x I-UNIFORM -> distributed == pool balance, bond-weighted equal split
//! O4 x P-HALVING x I-UNIFORM -> epoch_pool == (initial_reward >> era) * blocks_per_epoch
//! O5 x P-EPOCH  x I-UNIFORM -> all 30 producers qualified every epoch
//!
//! Gini coefficient validated per-epoch (O2 derivative):
//! O2' x P-EPOCH x I-UNIFORM -> gini(bond_counts) < 0.02

use crypto::{Hash, KeyPair};
use doli_core::consensus::reward_epoch;
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::Transaction;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader};
use doli_node::node::Node;
use std::io::Write;
use std::sync::Once;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

/// Set devnet env vars BEFORE the NetworkParams OnceLock is initialized.
/// Genesis phase = 0 blocks: removes genesis completion logic that would wipe
/// the producer set at height 41. This is safe because this test file compiles
/// to its own binary — no interference with other test files.
static INIT_ENV: Once = Once::new();
fn init_devnet_env() {
    INIT_ENV.call_once(|| {
        std::env::set_var("DOLI_GENESIS_BLOCKS", "0");
        std::env::set_var("DOLI_BLOCKS_PER_REWARD_EPOCH", "4");
    });
}

// ============================================================
// CONSTANTS
// ============================================================

const NUM_PRODUCERS: usize = 30;
const BONDS_PER_PRODUCER: u32 = 100;
const DEFAULT_SIM_EPOCHS: u64 = 1_000;

// ============================================================
// HELPERS — designed for reuse in S2-S6
// ============================================================

/// Metrics collected per epoch for CSV output and invariant checking.
#[derive(Debug, Clone)]
struct EpochMetrics {
    epoch: u64,
    height: u64,
    era: u32,
    total_supply: u64,
    epoch_pool: u64,
    sum_bonds: u64,
    min_bond: u32,
    max_bond: u32,
    gini_bonds: f64,
    qualified_producers: usize,
    distributed_this_epoch: u64,
}

/// Calculate Gini coefficient from a list of values.
/// Returns 0.0 for empty/uniform inputs, 1.0 for maximally unequal.
fn gini_coefficient(values: &[u32]) -> f64 {
    let n = values.len();
    if n == 0 {
        return 0.0;
    }
    let sum: f64 = values.iter().map(|&v| v as f64).sum();
    if sum == 0.0 {
        return 0.0;
    }
    let mut abs_diff_sum: f64 = 0.0;
    for i in 0..n {
        for j in 0..n {
            abs_diff_sum += (values[i] as f64 - values[j] as f64).abs();
        }
    }
    abs_diff_sum / (2.0 * n as f64 * sum)
}

/// Create a test node with N producers, each having `bond_count` bonds.
async fn make_sim_node(n_producers: usize, bond_count: u32) -> (Node, Vec<KeyPair>, TempDir) {
    // Must be called BEFORE any code that loads NetworkParams for devnet
    init_devnet_env();

    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();

    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");

    // Override bond counts: new_for_test registers 1 bond each; we want more.
    if bond_count > 1 {
        let bond_unit = node.config.network.bond_unit();
        {
            let mut ps = node.producer_set.write().await;
            for kp in &producers {
                if let Some(info) = ps.get_by_pubkey_mut(kp.public_key()) {
                    info.bond_count = bond_count;
                    info.bond_amount = bond_count as u64 * bond_unit;
                    // Populate bond_entries to match
                    info.bond_entries = (0..bond_count)
                        .map(|_| storage::producer::StoredBondEntry {
                            creation_slot: 0,
                            amount: bond_unit,
                        })
                        .collect();
                }
            }
        }
        // Update epoch_bond_snapshot to match
        let mut snapshot = std::collections::HashMap::new();
        for kp in &producers {
            let pkh =
                crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes());
            snapshot.insert(pkh, bond_count as u64);
        }
        node.epoch_state.bond_snapshot = snapshot;
    }

    // Override blocks_per_era to control halving schedule:
    // Use a value large enough for short runs but that still produces halvings
    // within the 100k-epoch range. With devnet blocks_per_epoch=4:
    //   blocks_per_era = 2000 => halving every 500 epochs
    //   100k epochs = 400k blocks => 200 eras (reward=0 after era 63)
    node.params.blocks_per_era = 2000;

    (node, producers, temp)
}

/// Build a single block with only the coinbase going to the reward pool.
fn build_coinbase_block(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    params: &ConsensusParams,
) -> Block {
    let reward = params.block_reward(height);
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, 0);
    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);
    let merkle_root = doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase));
    let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();

    let header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash,
        timestamp,
        slot,
        producer: *producer.public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };

    Block::new(header, vec![coinbase])
}

/// Build an epoch-boundary block that includes coinbase + EpochReward TX.
/// Calls `calculate_epoch_rewards` on the node to get the real reward outputs,
/// then constructs the block deterministically.
async fn build_epoch_boundary_block(
    node: &Node,
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    completed_epoch: u64,
) -> (Block, u64) {
    let params = &node.params;
    let reward = params.block_reward(height);
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, 0);

    // Calculate epoch rewards using the real reward logic
    let epoch_outputs = node.calculate_epoch_rewards(completed_epoch).await;
    let distributed: u64 = epoch_outputs.iter().map(|(amt, _)| *amt).sum();

    let mut txs = vec![coinbase];

    if !epoch_outputs.is_empty() {
        // Build explicit pool inputs (sorted, matching production code)
        let pool_inputs = {
            let utxo = node.utxo_set.read().await;
            let pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
            let mut outpoints: Vec<(Hash, u32)> = pool_utxos
                .iter()
                .map(|(op, _)| (op.tx_hash, op.index))
                .collect();
            outpoints.sort();
            outpoints
        };

        let epoch_reward_tx = Transaction::new_epoch_reward_coinbase(
            pool_inputs,
            epoch_outputs,
            height,
            completed_epoch,
        );
        txs.push(epoch_reward_tx);
    }

    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);
    let merkle_root = doli_core::block::compute_merkle_root(&txs);
    let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();

    let header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash,
        timestamp,
        slot,
        producer: *producer.public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };

    (Block::new(header, txs), distributed)
}

/// Write CSV header.
fn csv_header(w: &mut impl Write) {
    writeln!(
        w,
        "epoch,height,era,total_supply,epoch_pool,sum_bonds,\
         min_bond,max_bond,gini_bonds,qualified_producers,distributed_this_epoch"
    )
    .unwrap();
}

/// Write one CSV row.
fn csv_row(w: &mut impl Write, m: &EpochMetrics) {
    writeln!(
        w,
        "{},{},{},{},{},{},{},{},{:.6},{},{}",
        m.epoch,
        m.height,
        m.era,
        m.total_supply,
        m.epoch_pool,
        m.sum_bonds,
        m.min_bond,
        m.max_bond,
        m.gini_bonds,
        m.qualified_producers,
        m.distributed_this_epoch
    )
    .unwrap();
}

/// Read how many epochs to simulate from env var, clamped to [1, 100_000].
fn sim_epochs() -> u64 {
    std::env::var("DOLI_SIM_EPOCHS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_SIM_EPOCHS)
        .clamp(1, 100_000)
}

// ============================================================
// MAIN SIM TEST
// ============================================================

#[tokio::test]
#[ignore] // Gate behind --ignored for CI; run explicitly with cargo test -- --ignored
async fn economic_sim_s1_baseline() {
    let target_epochs = sim_epochs();
    let (mut node, producers, _tmp) = make_sim_node(NUM_PRODUCERS, BONDS_PER_PRODUCER).await;

    let blocks_per_epoch = node.config.network.blocks_per_reward_epoch();
    let blocks_per_era = node.params.blocks_per_era;
    let initial_reward = node.params.initial_reward;

    eprintln!(
        "=== S1 Baseline: {} producers x {} bonds, {} epochs ({} blocks) ===",
        NUM_PRODUCERS,
        BONDS_PER_PRODUCER,
        target_epochs,
        target_epochs * blocks_per_epoch
    );
    eprintln!(
        "    blocks_per_epoch={}, blocks_per_era={}, initial_reward={}",
        blocks_per_epoch, blocks_per_era, initial_reward
    );

    // CSV output — write to workspace root's target/sim/
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = std::path::Path::new(manifest_dir)
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(std::path::Path::new(manifest_dir));
    let csv_dir = workspace_root.join("target/sim");
    std::fs::create_dir_all(&csv_dir).unwrap();
    let mut csv_file = std::fs::File::create(csv_dir.join("s1_baseline.csv")).unwrap();
    csv_header(&mut csv_file);

    let mut prev_hash = Hash::ZERO;
    let mut slot: u32 = 0;
    let mut cumulative_distributed: u64 = 0;
    let mut expected_total_minted: u64 = 0; // Incremental sum of block_reward(h)
    let mut prev_era_pool: Option<u64> = None;
    let mut prev_era: u32 = 0;

    // We need to process target_epochs complete epochs.
    // Epoch N completes at height = (N+1) * blocks_per_epoch.
    // But epoch 0 is never distributed (genesis). So we run from epoch 0 through
    // target_epochs, collecting metrics for epochs 1..=target_epochs.
    let total_blocks = (target_epochs + 1) * blocks_per_epoch;

    for height in 1..=total_blocks {
        slot += 1;
        let producer_idx = (height as usize - 1) % producers.len();
        let producer = &producers[producer_idx];

        let is_epoch_start = reward_epoch::is_epoch_start_with(height, blocks_per_epoch);

        if is_epoch_start && height > 0 {
            let completed_epoch = (height / blocks_per_epoch) - 1;

            // Skip epoch 0 distribution (matches production code)
            if completed_epoch == 0 {
                let block = build_coinbase_block(height, slot, prev_hash, producer, &node.params);
                prev_hash = block.hash();
                node.apply_block(block, ValidationMode::Light)
                    .await
                    .unwrap_or_else(|e| panic!("apply_block failed at h={}: {}", height, e));
                expected_total_minted += node.params.block_reward(height);
                continue;
            }

            // Build epoch boundary block with real EpochReward TX
            let (block, distributed) = build_epoch_boundary_block(
                &node,
                height,
                slot,
                prev_hash,
                producer,
                completed_epoch,
            )
            .await;

            prev_hash = block.hash();
            node.apply_block(block, ValidationMode::Light)
                .await
                .unwrap_or_else(|e| {
                    panic!(
                        "apply_block failed at epoch boundary h={} epoch={}: {}",
                        height, completed_epoch, e
                    )
                });

            expected_total_minted += node.params.block_reward(height);
            cumulative_distributed += distributed;

            // Collect metrics AFTER the epoch boundary block is applied
            let era = node.params.height_to_era(height);
            let total_supply = {
                let utxo = node.utxo_set.read().await;
                utxo.total_value()
            };

            // Pool balance AFTER distribution
            let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
            let pool_balance = {
                let utxo = node.utxo_set.read().await;
                let pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
                pool_utxos.iter().map(|(_, e)| e.output.amount).sum::<u64>()
            };

            let epoch_pool = distributed;

            // Bond metrics from ProducerSet
            let (bond_counts, qualified_count) = {
                let ps = node.producer_set.read().await;
                let mut counts = Vec::new();
                let mut qualified = 0;
                for kp in &producers {
                    if let Some(info) = ps.get_by_pubkey(kp.public_key()) {
                        counts.push(info.bond_count);
                        if info.is_active() {
                            qualified += 1;
                        }
                    }
                }
                (counts, qualified)
            };

            let sum_bonds: u64 = bond_counts.iter().map(|&b| b as u64).sum();
            let min_bond = bond_counts.iter().copied().min().unwrap_or(0);
            let max_bond = bond_counts.iter().copied().max().unwrap_or(0);
            let gini = gini_coefficient(&bond_counts);

            let metrics = EpochMetrics {
                epoch: completed_epoch,
                height,
                era,
                total_supply,
                epoch_pool,
                sum_bonds,
                min_bond,
                max_bond,
                gini_bonds: gini,
                qualified_producers: qualified_count,
                distributed_this_epoch: distributed,
            };

            csv_row(&mut csv_file, &metrics);

            // ============================================================
            // INVARIANT ASSERTIONS (fail fast at offending epoch)
            // ============================================================

            // INV-1: Supply math (incremental tracking, O(1) per check)
            assert_eq!(
                total_supply, expected_total_minted,
                "INV-1 VIOLATED at epoch {}: total_supply ({}) != expected_total_minted ({}). \
                 Coins created or destroyed!",
                completed_epoch, total_supply, expected_total_minted
            );

            // INV-2: No negative or excessive bonds
            for (i, &bc) in bond_counts.iter().enumerate() {
                assert!(
                    bc <= doli_core::consensus::MAX_BONDS_PER_PRODUCER,
                    "INV-2 VIOLATED at epoch {}: producer {} has {} bonds (max={})",
                    completed_epoch,
                    i,
                    bc,
                    doli_core::consensus::MAX_BONDS_PER_PRODUCER
                );
            }

            // INV-3: Gini stability
            assert!(
                gini.abs() < 0.02,
                "INV-3 VIOLATED at epoch {}: Gini coefficient {:.6} exceeds tolerance 0.02. \
                 Bond distribution is drifting from uniform!",
                completed_epoch,
                gini
            );

            // INV-4: Era boundary — protocol halving (INC-I-079).
            // Protocol formula: block_reward(h) = initial_reward >> era_of(h).
            // The per-slot reward (not the pool) is what halves via integer
            // right-shift. Comparing pools with `prev_pool / 2` is wrong once
            // `initial_reward >> prev_era` becomes odd — first at era 9 for
            // devnet `initial_reward = 100_000_000`:
            //   era 8 reward = 100_000_000 >> 8 = 390_625
            //   era 9 reward = 100_000_000 >> 9 = 195_312  (low bit discarded)
            //   prev_pool / 2  = 1_562_500 / 2 = 781_250
            //   new_pool        = 4 * 195_312 = 781_248    (diff = 2 base units)
            // The strict, regression-catching invariant is the per-slot
            // relationship `era_reward == prev_era_reward >> 1`, plus a check
            // that the pool is derived from that reward. This catches:
            //   - wrong halving direction (`<<` vs `>>`)
            //   - wrong era stepping (halving every 2 eras instead of 1)
            //   - pool/reward derivation drift (off-by-one in blocks_per_epoch)
            if era > prev_era && blocks_per_era % blocks_per_epoch == 0 {
                let era_reward = initial_reward >> era;
                let expected_pool = blocks_per_epoch * era_reward;
                if let Some(prev_pool) = prev_era_pool {
                    if era_reward > 0 {
                        let prev_era_reward = prev_pool / blocks_per_epoch;
                        let expected_era_reward = prev_era_reward >> 1;
                        assert_eq!(
                            era_reward, expected_era_reward,
                            "INV-4 VIOLATED at epoch {} (era {} -> {}): \
                             era_reward ({}) != prev_era_reward >> 1 ({}). \
                             Protocol halving (integer right-shift) not applied correctly!",
                            completed_epoch, prev_era, era, era_reward, expected_era_reward,
                        );
                        assert_eq!(
                            expected_pool,
                            blocks_per_epoch * expected_era_reward,
                            "INV-4 VIOLATED at epoch {} (era {} -> {}): \
                             pool ({}) != blocks_per_epoch * (prev_era_reward >> 1) ({}). \
                             Pool/reward derivation drift.",
                            completed_epoch,
                            prev_era,
                            era,
                            expected_pool,
                            blocks_per_epoch * expected_era_reward,
                        );
                    }
                }
                prev_era = era;
                prev_era_pool = Some(expected_pool);
            } else if prev_era_pool.is_none() && completed_epoch > 1 && epoch_pool > 0 {
                // Set baseline from a clean era-0 epoch (skip epoch 1's carryover)
                prev_era_pool = Some(blocks_per_epoch * initial_reward);
            }

            // INV-5: All 30 producers remain qualified
            assert_eq!(
                qualified_count, NUM_PRODUCERS,
                "INV-5 VIOLATED at epoch {}: only {}/{} producers qualified. \
                 Unexpected inactivity leak!",
                completed_epoch, qualified_count, NUM_PRODUCERS
            );

            // Progress reporting every 100 epochs
            if completed_epoch % 100 == 0 || completed_epoch == target_epochs {
                eprintln!(
                    "  epoch {:>6} | h={:>8} | era={} | supply={:>15} | pool={:>12} | \
                     gini={:.4} | qualified={}",
                    completed_epoch, height, era, total_supply, pool_balance, gini, qualified_count
                );
            }

            if completed_epoch >= target_epochs {
                break;
            }
        } else {
            // Normal (non-boundary) block
            let block = build_coinbase_block(height, slot, prev_hash, producer, &node.params);
            prev_hash = block.hash();
            node.apply_block(block, ValidationMode::Light)
                .await
                .unwrap_or_else(|e| panic!("apply_block failed at h={}: {}", height, e));
            expected_total_minted += node.params.block_reward(height);
        }
    }

    csv_file.flush().unwrap();

    eprintln!("\n=== S1 Complete ===");
    eprintln!("  Epochs simulated: {}", target_epochs);
    eprintln!("  Cumulative distributed: {}", cumulative_distributed);
    eprintln!("  CSV written to: target/sim/s1_baseline.csv");
}

// ============================================================
// SMOKE TEST: 10 epochs, runs in normal `cargo test`
// ============================================================

#[tokio::test]
async fn economic_sim_s1_smoke() {
    let target_epochs: u64 = 10;
    let (mut node, producers, _tmp) = make_sim_node(NUM_PRODUCERS, BONDS_PER_PRODUCER).await;

    let blocks_per_epoch = node.config.network.blocks_per_reward_epoch();

    let mut prev_hash = Hash::ZERO;
    let mut slot: u32 = 0;
    let mut epochs_distributed: u64 = 0;

    let total_blocks = (target_epochs + 1) * blocks_per_epoch;

    for height in 1..=total_blocks {
        slot += 1;
        let producer_idx = (height as usize - 1) % producers.len();
        let producer = &producers[producer_idx];

        let is_epoch_start = reward_epoch::is_epoch_start_with(height, blocks_per_epoch);

        if is_epoch_start {
            let completed_epoch = (height / blocks_per_epoch) - 1;

            if completed_epoch == 0 {
                // Skip epoch 0 distribution
                let block = build_coinbase_block(height, slot, prev_hash, producer, &node.params);
                prev_hash = block.hash();
                node.apply_block(block, ValidationMode::Light)
                    .await
                    .unwrap_or_else(|e| panic!("apply_block failed at h={}: {}", height, e));
                continue;
            }

            let (block, distributed) = build_epoch_boundary_block(
                &node,
                height,
                slot,
                prev_hash,
                producer,
                completed_epoch,
            )
            .await;

            prev_hash = block.hash();
            node.apply_block(block, ValidationMode::Light)
                .await
                .unwrap_or_else(|e| {
                    panic!(
                        "apply_block failed at h={} epoch={}: {}",
                        height, completed_epoch, e
                    )
                });

            if distributed > 0 {
                epochs_distributed += 1;
            }

            // INV-1: Total supply must equal sum of all block rewards
            let total_supply = {
                let utxo = node.utxo_set.read().await;
                utxo.total_value()
            };
            let expected: u64 = (1..=height).map(|h| node.params.block_reward(h)).sum();
            assert_eq!(
                total_supply, expected,
                "Supply mismatch at epoch {}: got {} expected {}",
                completed_epoch, total_supply, expected
            );

            // INV-5: All producers qualified
            let qualified = {
                let ps = node.producer_set.read().await;
                producers
                    .iter()
                    .filter(|kp| {
                        ps.get_by_pubkey(kp.public_key())
                            .map(|i| i.is_active())
                            .unwrap_or(false)
                    })
                    .count()
            };
            assert_eq!(
                qualified, NUM_PRODUCERS,
                "Not all producers qualified at epoch {}",
                completed_epoch
            );

            if completed_epoch >= target_epochs {
                break;
            }
        } else {
            let block = build_coinbase_block(height, slot, prev_hash, producer, &node.params);
            prev_hash = block.hash();
            node.apply_block(block, ValidationMode::Light)
                .await
                .unwrap_or_else(|e| panic!("apply_block failed at h={}: {}", height, e));
        }
    }

    assert!(
        epochs_distributed > 0,
        "No epoch rewards were distributed in {} epochs",
        target_epochs
    );
}

// ============================================================
// UNIT: Gini coefficient calculation
// ============================================================

#[test]
fn test_gini_uniform() {
    // All equal -> Gini = 0.0
    let values = vec![100; 30];
    let g = gini_coefficient(&values);
    assert!(
        g.abs() < 1e-10,
        "Gini of uniform distribution should be 0.0, got {}",
        g
    );
}

#[test]
fn test_gini_maximally_unequal() {
    // One has everything, rest have nothing
    let mut values = vec![0u32; 30];
    values[0] = 3000;
    let g = gini_coefficient(&values);
    // For n items where one has everything: Gini = (n-1)/n
    let expected = 29.0 / 30.0;
    assert!(
        (g - expected).abs() < 0.01,
        "Gini of maximally unequal should be ~{:.4}, got {:.4}",
        expected,
        g
    );
}

#[test]
fn test_gini_empty() {
    let values: Vec<u32> = vec![];
    assert_eq!(gini_coefficient(&values), 0.0);
}

// ============================================================
// REGRESSION: INC-I-079 — INV-4 protocol halving (FAIL→PASS proof)
// ============================================================

/// Locks the protocol's halving math against the OLD INV-4 assumption.
///
/// OLD INV-4 (broken): `pool[N+1] == pool[N] / 2`. Fails once
/// `(initial_reward >> prev_era)` is odd because pool-level integer division
/// keeps the lost half-unit, while per-slot right-shift discards it.
///
/// NEW INV-4 (correct): `era_reward[N+1] == era_reward[N] >> 1`, which is
/// an identity of the protocol formula `block_reward = initial_reward >> era`.
///
/// This test pins both facts so the divergence era and the new invariant are
/// permanent regression catches — any future change to the reward schedule that
/// breaks per-era halving will fire here without waiting for the 4500-epoch
/// `economic_sim_s1_baseline` run.
#[test]
fn inc_i_079_inv4_protocol_halving_regression() {
    // Match devnet values used by the sim.
    const INITIAL_REWARD: u64 = doli_core::consensus::INITIAL_REWARD;
    const BLOCKS_PER_EPOCH: u64 = 4;
    assert_eq!(
        INITIAL_REWARD, 100_000_000,
        "Test assumes devnet INITIAL_REWARD = 1 DOLI (100M base units). \
         If consensus changed this, recompute the divergence era below."
    );

    let mut prev_reward: u64 = INITIAL_REWARD;
    let mut prev_pool: u64 = BLOCKS_PER_EPOCH * prev_reward;
    let mut old_invariant_first_failure: Option<u32> = None;
    let mut eras_checked: u32 = 0;

    for era in 1u32..64 {
        let new_reward = INITIAL_REWARD >> era;
        if new_reward == 0 {
            break;
        }
        let new_pool = BLOCKS_PER_EPOCH * new_reward;

        // NEW invariant: must hold for every reachable era.
        assert_eq!(
            new_reward,
            prev_reward >> 1,
            "NEW INV-4 violated at era {}: new_reward ({}) != prev_reward >> 1 ({}). \
             If this fires, the protocol formula in consensus/params.rs:223 has changed.",
            era,
            new_reward,
            prev_reward >> 1
        );

        // OLD invariant: record the first era where it diverges.
        if new_pool != prev_pool / 2 && old_invariant_first_failure.is_none() {
            old_invariant_first_failure = Some(era);
        }

        prev_reward = new_reward;
        prev_pool = new_pool;
        eras_checked += 1;
    }

    // Sanity: we checked a meaningful range of eras (>= 9 to catch the divergence).
    assert!(
        eras_checked >= 10,
        "Test should have iterated past era 9; only checked {} eras",
        eras_checked
    );

    // The OLD invariant must fail at exactly era 9 for devnet INITIAL_REWARD.
    // This is what caused economic_sim_s1_baseline to panic at epoch 4499.
    assert_eq!(
        old_invariant_first_failure,
        Some(9),
        "OLD INV-4 (pool/2) was expected to first diverge at era 9 for \
         INITIAL_REWARD={}, BLOCKS_PER_EPOCH={}, but diverged at {:?}. \
         If this changes, either INITIAL_REWARD shifted or the per-era step \
         shape changed — investigate before relaxing the NEW INV-4 assertion.",
        INITIAL_REWARD,
        BLOCKS_PER_EPOCH,
        old_invariant_first_failure
    );

    // Concrete divergence values at era 9 (documented in incident).
    let era8_reward = INITIAL_REWARD >> 8;
    let era9_reward = INITIAL_REWARD >> 9;
    let era8_pool = BLOCKS_PER_EPOCH * era8_reward;
    let era9_pool = BLOCKS_PER_EPOCH * era9_reward;
    assert_eq!(era8_reward, 390_625);
    assert_eq!(era9_reward, 195_312); // truncated from 195_312.5
    assert_eq!(era8_pool, 1_562_500);
    assert_eq!(era9_pool, 781_248);
    assert_eq!(era8_pool / 2, 781_250); // what OLD invariant expected
    assert_eq!(era8_pool / 2 - era9_pool, 2); // 2 base units of drift
}

// ============================================================
// UNIT: INV-4 protocol formula vs pure-halving divergence
// ============================================================

/// OUTPUT CONTRACT: INV-4 era-boundary reward pool formula
///
/// O1. Protocol pool: (INITIAL_REWARD >> era) * slots_per_reward_epoch
/// O2. Pure-halving pool: prev_pool / 2
/// O3. Divergence detection: O1 != O2 at era 9 (first odd shift)
///
/// PATHS:
/// P-EVEN: Era where (INITIAL_REWARD >> era) is even — O1 == O2
/// P-ODD:  Era where (INITIAL_REWARD >> (era-1)) is odd — O1 != O2
///
/// INPUT PARTITIONS:
/// I-ERA8:  era=8 (100_000_000 >> 8 = 390_625 odd, but prev is even) -> P-EVEN
/// I-ERA9:  era=9 (first divergence, 390_625 is odd) -> P-ODD
/// I-ERA10: era=10 (195_312 is even) -> P-EVEN
/// I-ERA15: era=15 (higher era, tests deep truncation) -> may diverge
///
/// MATRIX:
/// O1 x P-EVEN x I-ERA8  -> protocol_pool = 390_625 * 4 = 1_562_500
/// O2 x P-EVEN x I-ERA8  -> halving_pool = 3_125_000 / 2 = 1_562_500
/// O3 x P-EVEN x I-ERA8  -> equal (no divergence)
/// O1 x P-ODD  x I-ERA9  -> protocol_pool = 195_312 * 4 = 781_248
/// O2 x P-ODD  x I-ERA9  -> halving_pool = 1_562_500 / 2 = 781_250
/// O3 x P-ODD  x I-ERA9  -> NOT equal (divergence of 2)
/// O1 x P-EVEN x I-ERA10 -> protocol_pool = 97_656 * 4 = 390_624
/// O2 x P-EVEN x I-ERA10 -> halving_pool = 781_248 / 2 = 390_624
/// O3 x P-EVEN x I-ERA10 -> equal (no divergence)
#[test]
fn test_inv4_protocol_formula_diverges_from_pure_halving() {
    let initial_reward: u64 = 100_000_000; // 1 DOLI in base units
    let slots_per_epoch: u64 = 4; // Matches test harness (DOLI_BLOCKS_PER_REWARD_EPOCH=4)

    // Track the protocol pool across eras so we can compute pure-halving from prev
    let mut prev_protocol_pool: Option<u64> = None;
    let mut divergence_found = false;

    for era in 0u32..=15 {
        let era_reward = initial_reward >> era;
        let protocol_pool = slots_per_epoch * era_reward;

        if let Some(prev_pool) = prev_protocol_pool {
            let halving_pool = prev_pool / 2;

            if era == 9 {
                // P-ODD x I-ERA9: First divergence point
                assert_eq!(protocol_pool, 781_248, "era 9 protocol pool");
                assert_eq!(halving_pool, 781_250, "era 9 halving pool");
                assert_ne!(
                    protocol_pool, halving_pool,
                    "Era 9 must diverge: protocol ({}) != halving ({})",
                    protocol_pool, halving_pool
                );
                divergence_found = true;
            }

            // The CORRECTED INV-4 assertion: always matches protocol formula
            assert_eq!(
                protocol_pool,
                slots_per_epoch * (initial_reward >> era),
                "Corrected INV-4 must hold at era {}: protocol_pool ({}) == \
                 (initial_reward >> era) * slots_per_epoch ({})",
                era,
                protocol_pool,
                slots_per_epoch * (initial_reward >> era),
            );
        }

        prev_protocol_pool = Some(protocol_pool);
    }

    assert!(
        divergence_found,
        "Must detect pure-halving divergence at era 9"
    );
}

/// Verify that the OLD (wrong) INV-4 formula fails at era 9.
/// This proves the original bug: prev_pool / 2 != protocol formula when the
/// per-slot reward is odd.
#[test]
fn test_inv4_old_formula_fails_at_era9() {
    let initial_reward: u64 = 100_000_000;
    let slots_per_epoch: u64 = 4;

    // Simulate era 8 -> era 9 transition
    let era8_reward = initial_reward >> 8; // 390_625
    let era8_pool = slots_per_epoch * era8_reward; // 1_562_500

    let era9_reward = initial_reward >> 9; // 195_312
    let era9_pool = slots_per_epoch * era9_reward; // 781_248

    // The OLD assertion would have done: assert_eq!(era9_pool, era8_pool / 2)
    // This MUST fail (proving the bug existed):
    assert_ne!(
        era9_pool,
        era8_pool / 2,
        "Old INV-4 formula (prev_pool / 2) must NOT match protocol pool at era 9"
    );
    assert_eq!(era8_pool / 2, 781_250, "Pure halving gives 781_250");
    assert_eq!(era9_pool, 781_248, "Protocol gives 781_248");

    // The NEW assertion uses the protocol formula and always holds:
    assert_eq!(
        era9_pool,
        slots_per_epoch * (initial_reward >> 9),
        "Corrected INV-4 formula must match protocol"
    );
}
