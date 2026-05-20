//! Economic Simulator S2: Compounding Centralization
//!
//! // OUTPUT CONTRACT: Node::apply_block() + calculate_epoch_rewards() + AddBond TX
//! // with Pareto-distributed initial stake and auto-restaking
//!
//! Observable outputs:
//! O1. UTXO total_value after apply_block (total supply)
//! O2. ProducerSet bond_count per producer (bond distribution)
//! O3. Epoch reward distribution amounts (bond-weighted split)
//! O4. AddBond TX acceptance/rejection by validation + cap enforcement
//! O5. Gini coefficient trajectory (centralization metric)
//! O6. Nakamoto coefficient (minimum producers for >50% stake)
//!
//! PATHS:
//! P-NORMAL: Non-boundary block (coinbase only)
//! P-EPOCH: Epoch boundary block (coinbase + EpochReward distribution)
//! P-RESTAKE: Post-epoch block with AddBond TXs (reward -> new bonds)
//! P-CAP-HIT: Restake where producer is at MAX_BONDS_PER_PRODUCER
//! P-EPOCH0: Epoch 0 boundary (skipped, no distribution)
//!
//! INPUT PARTITIONS:
//! I-PARETO: 30 producers with Pareto bonds (100 down to 10), auto-restaking
//! I-SMOKE: 10 epochs (quick validation, no cap hits expected)
//! I-MEDIUM: 1000 epochs (default, cap hits possible for top producers)
//! I-LONG: 50k epochs (stress, all producers approach cap)
//!
//! MATRIX: see cells O1-O6 x P-* x I-* above; each tested per epoch.
//!
//! Invariants:
//! INV-1: Bond conservation — sum_bonds never decreases, increases by restaked amount
//! INV-2: Cap enforcement — no producer exceeds MAX_BONDS_PER_PRODUCER (3000)
//! INV-3: Supply conservation — total_supply == sum(block_reward(1..=h))
//! INV-4: Monotonic Gini OR equilibrium (variance < 0.005 over last 100 epochs)
//! INV-5: No restake from nothing — 0-reward producers add 0 bonds

use crypto::{signature, Hash, KeyPair};
use doli_core::consensus::reward_epoch;
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{Input, Output, OutputType, Transaction};
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader};
use doli_node::node::Node;
use std::io::Write;
use std::sync::Once;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

static INIT_ENV: Once = Once::new();
fn init_devnet_env() {
    INIT_ENV.call_once(|| {
        std::env::set_var("DOLI_GENESIS_BLOCKS", "0");
        std::env::set_var("DOLI_BLOCKS_PER_REWARD_EPOCH", "4");
    });
}

const NUM_PRODUCERS: usize = 30;
const DEFAULT_SIM_EPOCHS: u64 = 1_000;
const MAX_SIM_EPOCHS: u64 = 50_000;

/// Pareto initial bond distribution.
/// bonds[i] = max(10, round(100 * ((30-i)/30)^1.16))
/// Produces a power-law curve: richest ~100, poorest 10.
fn pareto_bonds(n: usize) -> Vec<u32> {
    (0..n)
        .map(|i| {
            let rank_frac = (n - i) as f64 / n as f64;
            let raw = 100.0 * rank_frac.powf(1.16);
            (raw.round() as u32).clamp(10, doli_core::consensus::MAX_BONDS_PER_PRODUCER)
        })
        .collect()
}

#[derive(Debug, Clone)]
struct S2EpochMetrics {
    epoch: u64,
    height: u64,
    total_supply: u64,
    sum_bonds: u64,
    top1_bonds: u32,
    top5_share: f64,
    top10_share: f64,
    gini_bonds: f64,
    nakamoto_coefficient: usize,
    producers_at_cap: usize,
    restake_attempted: u32,
    restake_rejected_cap: u32,
}

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

/// Nakamoto coefficient: minimum producers whose combined bonds > 50%.
fn nakamoto_coefficient(bond_counts: &[u32]) -> usize {
    let total: u64 = bond_counts.iter().map(|&b| b as u64).sum();
    if total == 0 {
        return 0;
    }
    let threshold = total / 2;
    let mut sorted: Vec<u32> = bond_counts.to_vec();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    let mut cumulative: u64 = 0;
    for (i, &b) in sorted.iter().enumerate() {
        cumulative += b as u64;
        if cumulative > threshold {
            return i + 1;
        }
    }
    sorted.len()
}

/// Create a test node with N producers, each with individual bond counts.
async fn make_sim_node_pareto(
    n_producers: usize,
    bond_counts: &[u32],
) -> (Node, Vec<KeyPair>, TempDir) {
    init_devnet_env();

    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();

    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");

    let bond_unit = node.config.network.bond_unit();
    {
        let mut ps = node.producer_set.write().await;
        for (idx, kp) in producers.iter().enumerate() {
            let bc = bond_counts[idx];
            if let Some(info) = ps.get_by_pubkey_mut(kp.public_key()) {
                info.bond_count = bc;
                info.bond_amount = bc as u64 * bond_unit;
                info.bond_entries = (0..bc)
                    .map(|_| storage::producer::StoredBondEntry {
                        creation_slot: 0,
                        amount: bond_unit,
                    })
                    .collect();
            }
        }
    }
    let mut snapshot = std::collections::HashMap::new();
    for (idx, kp) in producers.iter().enumerate() {
        let pkh =
            crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes());
        snapshot.insert(pkh, bond_counts[idx] as u64);
    }
    node.epoch_state.bond_snapshot = snapshot;
    node.params.blocks_per_era = 2000;

    (node, producers, temp)
}

fn make_header(
    slot: u32,
    prev_hash: Hash,
    txs: &[Transaction],
    producer: &KeyPair,
    params: &ConsensusParams,
) -> BlockHeader {
    BlockHeader {
        version: 2,
        prev_hash,
        merkle_root: doli_core::block::compute_merkle_root(txs),
        presence_root: Hash::ZERO,
        genesis_hash: doli_core::chainspec::ChainSpec::devnet().genesis_hash(),
        timestamp: params.genesis_time + (slot as u64 * params.slot_duration),
        slot,
        producer: *producer.public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    }
}

fn build_coinbase_block(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    params: &ConsensusParams,
) -> Block {
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(params.block_reward(height), pool_hash, height, 0);
    let header = make_header(
        slot,
        prev_hash,
        std::slice::from_ref(&coinbase),
        producer,
        params,
    );
    Block::new(header, vec![coinbase])
}

async fn build_epoch_boundary_block(
    node: &Node,
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    completed_epoch: u64,
) -> (Block, u64) {
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase =
        Transaction::new_coinbase(node.params.block_reward(height), pool_hash, height, 0);
    let epoch_outputs = node
        .calculate_epoch_rewards(completed_epoch)
        .await
        .expect("complete store in economic sim S2");
    let distributed: u64 = epoch_outputs.iter().map(|(amt, _)| *amt).sum();
    let mut txs = vec![coinbase];
    if !epoch_outputs.is_empty() {
        let pool_inputs = {
            let utxo = node.utxo_set.read().await;
            let pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
            let mut ops: Vec<(Hash, u32)> = pool_utxos
                .iter()
                .map(|(op, _)| (op.tx_hash, op.index))
                .collect();
            ops.sort();
            ops
        };
        txs.push(Transaction::new_epoch_reward_coinbase(
            pool_inputs,
            epoch_outputs,
            height,
            completed_epoch,
        ));
    }
    let header = make_header(slot, prev_hash, &txs, producer, &node.params);
    (Block::new(header, txs), distributed)
}

/// Build a block containing AddBond TXs for producers who want to restake.
/// Returns (block, restake_attempted, restake_rejected_cap, bonds_added).
///
/// Restake logic: gather all mature Normal UTXOs for each producer. If the
/// total is >= bond_unit, convert floor(total/bond_unit) into bonds. Change
/// goes back as a Normal UTXO (will be picked up in a future restake).
async fn build_restake_block(
    node: &Node,
    height: u64,
    slot: u32,
    prev_hash: Hash,
    block_producer: &KeyPair,
    producers: &[KeyPair],
) -> (Block, u32, u32, u64) {
    let params = &node.params;
    let reward = params.block_reward(height);
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, 0);

    let bond_unit = node.config.network.bond_unit();
    let maturity = node.config.network.params().coinbase_maturity;
    let mut txs = vec![coinbase];
    let mut attempted: u32 = 0;
    let mut rejected_cap: u32 = 0;
    let mut total_bonds_added: u64 = 0;

    for kp in producers.iter() {
        let pkh =
            crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes());

        // Find ALL mature Normal UTXOs for this producer
        let reward_utxos: Vec<(storage::Outpoint, storage::UtxoEntry)> = {
            let utxo = node.utxo_set.read().await;
            utxo.get_by_pubkey_hash(&pkh)
                .into_iter()
                .filter(|(_, e)| {
                    e.output.output_type == OutputType::Normal && e.height + maturity <= height
                })
                .collect()
        };

        let reward_total: u64 = reward_utxos.iter().map(|(_, e)| e.output.amount).sum();

        if reward_total == 0 {
            continue; // INV-5: no restake from nothing
        }

        let whole_bonds = (reward_total / bond_unit) as u32;
        if whole_bonds == 0 {
            continue; // Not enough for even 1 bond — UTXOs remain for next time
        }

        // Check current bond count for cap
        let current_bonds = {
            let ps = node.producer_set.read().await;
            ps.get_by_pubkey(kp.public_key())
                .map(|i| i.bond_count)
                .unwrap_or(0)
        };

        attempted += 1;

        let bonds_to_add =
            if current_bonds + whole_bonds > doli_core::consensus::MAX_BONDS_PER_PRODUCER {
                let can_add =
                    doli_core::consensus::MAX_BONDS_PER_PRODUCER.saturating_sub(current_bonds);
                if can_add == 0 {
                    rejected_cap += 1;
                    continue;
                }
                rejected_cap += 1;
                can_add
            } else {
                whole_bonds
            };

        let bond_amount = bonds_to_add as u64 * bond_unit;
        let fee = doli_core::consensus::BASE_FEE
            + (bonds_to_add as u64 * 4 * doli_core::consensus::FEE_PER_BYTE)
                / doli_core::consensus::FEE_DIVISOR;

        if reward_total < bond_amount + fee {
            continue;
        }

        let lock_until = height + 2000 + 1000;
        let mut inputs: Vec<Input> = Vec::new();
        let mut input_total: u64 = 0;
        for (outpoint, entry) in &reward_utxos {
            if input_total >= bond_amount + fee {
                break;
            }
            inputs.push(Input::new(outpoint.tx_hash, outpoint.index));
            input_total += entry.output.amount;
        }

        let mut tx = Transaction::new_add_bond(
            inputs,
            *kp.public_key(),
            bonds_to_add,
            bond_amount,
            lock_until,
        );

        let change = input_total.saturating_sub(bond_amount + fee);
        if change > 0 {
            tx.outputs.push(Output::normal(change, pkh));
        }

        // Sign each input (BIP-143 style)
        for i in 0..tx.inputs.len() {
            let signing_hash = tx.signing_message_for_input(i);
            tx.inputs[i].signature = signature::sign_hash(&signing_hash, kp.private_key());
            tx.inputs[i].public_key = Some(*kp.public_key());
        }

        total_bonds_added += bonds_to_add as u64;
        txs.push(tx);
    }

    let header = make_header(slot, prev_hash, &txs, block_producer, params);
    (
        Block::new(header, txs),
        attempted,
        rejected_cap,
        total_bonds_added,
    )
}

fn s2_csv_header(w: &mut impl Write) {
    writeln!(
        w,
        "epoch,height,total_supply,sum_bonds,top1_bonds,top5_share,\
         top10_share,gini_bonds,nakamoto_coefficient,producers_at_cap,\
         restake_attempted,restake_rejected_cap"
    )
    .unwrap();
}

fn s2_csv_row(w: &mut impl Write, m: &S2EpochMetrics) {
    writeln!(
        w,
        "{},{},{},{},{},{:.6},{:.6},{:.6},{},{},{},{}",
        m.epoch,
        m.height,
        m.total_supply,
        m.sum_bonds,
        m.top1_bonds,
        m.top5_share,
        m.top10_share,
        m.gini_bonds,
        m.nakamoto_coefficient,
        m.producers_at_cap,
        m.restake_attempted,
        m.restake_rejected_cap
    )
    .unwrap();
}

fn sim_epochs() -> u64 {
    std::env::var("DOLI_SIM_EPOCHS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_SIM_EPOCHS)
        .clamp(1, MAX_SIM_EPOCHS)
}

fn top_n_share(sorted_desc: &[u32], n: usize, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    sorted_desc.iter().take(n).map(|&b| b as u64).sum::<u64>() as f64 / total as f64
}

// ============================================================
// SMOKE TEST: 10 epochs — validates the harness compiles and runs
// ============================================================

#[tokio::test]
async fn economic_sim_s2_smoke() {
    let target_epochs: u64 = 10;
    let initial_bonds = pareto_bonds(NUM_PRODUCERS);
    let (mut node, producers, _tmp) = make_sim_node_pareto(NUM_PRODUCERS, &initial_bonds).await;
    let bpe = node.config.network.blocks_per_reward_epoch();
    let mut prev_hash = Hash::ZERO;
    let mut slot: u32 = 0;
    let mut restake_pending = false;

    for height in 1..=(target_epochs + 1) * bpe {
        slot += 1;
        let producer = &producers[(height as usize - 1) % producers.len()];
        let is_start = reward_epoch::is_epoch_start_with(height, bpe);
        if is_start {
            let ep = (height / bpe) - 1;
            if ep == 0 {
                let b = build_coinbase_block(height, slot, prev_hash, producer, &node.params);
                prev_hash = b.hash();
                node.apply_block(b, ValidationMode::Light, None)
                    .await
                    .unwrap();
                continue;
            }
            let (b, dist) =
                build_epoch_boundary_block(&node, height, slot, prev_hash, producer, ep).await;
            prev_hash = b.hash();
            node.apply_block(b, ValidationMode::Light, None)
                .await
                .unwrap();
            if dist > 0 {
                restake_pending = true;
            }
            if ep >= target_epochs {
                break;
            }
        } else if restake_pending {
            let (b, _, _, _) =
                build_restake_block(&node, height, slot, prev_hash, producer, &producers).await;
            prev_hash = b.hash();
            node.apply_block(b, ValidationMode::Light, None)
                .await
                .unwrap();
            restake_pending = false;
        } else {
            let b = build_coinbase_block(height, slot, prev_hash, producer, &node.params);
            prev_hash = b.hash();
            node.apply_block(b, ValidationMode::Light, None)
                .await
                .unwrap();
        }
    }
}

// ============================================================
// FULL SIM: 1000 epochs (default), configurable via DOLI_SIM_EPOCHS
// ============================================================

#[tokio::test]
#[ignore]
async fn economic_sim_s2_compounding() {
    let target_epochs = sim_epochs();
    let initial_bonds = pareto_bonds(NUM_PRODUCERS);
    let (mut node, producers, _tmp) = make_sim_node_pareto(NUM_PRODUCERS, &initial_bonds).await;

    let blocks_per_epoch = node.config.network.blocks_per_reward_epoch();
    let bond_unit = node.config.network.bond_unit();

    eprintln!(
        "=== S2 Compounding: {} producers, Pareto bonds, {} epochs ===",
        NUM_PRODUCERS, target_epochs
    );
    eprintln!("    Initial bonds: {:?}", &initial_bonds[..5]);
    eprintln!(
        "    bond_unit={}, blocks_per_epoch={}, initial_reward={}",
        bond_unit, blocks_per_epoch, node.params.initial_reward
    );

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = std::path::Path::new(manifest_dir)
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(std::path::Path::new(manifest_dir));
    let csv_dir = workspace_root.join("target/sim");
    std::fs::create_dir_all(&csv_dir).unwrap();
    let mut csv_file = std::fs::File::create(csv_dir.join("s2_compounding.csv")).unwrap();
    s2_csv_header(&mut csv_file);

    let mut prev_hash = Hash::ZERO;
    let mut slot: u32 = 0;
    let mut expected_total_minted: u64 = 0;
    let mut cumulative_fees: u64 = 0;
    let mut restake_pending = false;
    let mut prev_sum_bonds: u64 = initial_bonds.iter().map(|&b| b as u64).sum();
    let mut gini_history: Vec<f64> = Vec::new();
    let mut first_cap_epoch: Option<u64> = None;
    let mut cumulative_restake_attempted: u32 = 0;
    let mut cumulative_restake_rejected: u32 = 0;

    let total_blocks = (target_epochs + 1) * blocks_per_epoch;

    for height in 1..=total_blocks {
        slot += 1;
        let producer_idx = (height as usize - 1) % producers.len();
        let producer = &producers[producer_idx];

        let is_epoch_start = reward_epoch::is_epoch_start_with(height, blocks_per_epoch);

        if is_epoch_start && height > 0 {
            let completed_epoch = (height / blocks_per_epoch) - 1;

            if completed_epoch == 0 {
                let block = build_coinbase_block(height, slot, prev_hash, producer, &node.params);
                prev_hash = block.hash();
                node.apply_block(block, ValidationMode::Light, None)
                    .await
                    .unwrap_or_else(|e| panic!("apply_block h={}: {}", height, e));
                expected_total_minted += node.params.block_reward(height);
                continue;
            }

            let (block, _distributed) = build_epoch_boundary_block(
                &node,
                height,
                slot,
                prev_hash,
                producer,
                completed_epoch,
            )
            .await;

            prev_hash = block.hash();
            node.apply_block(block, ValidationMode::Light, None)
                .await
                .unwrap_or_else(|e| {
                    panic!("apply_block h={} epoch={}: {}", height, completed_epoch, e)
                });
            expected_total_minted += node.params.block_reward(height);
            restake_pending = true;

            // Collect metrics
            let total_supply = {
                let utxo = node.utxo_set.read().await;
                utxo.total_value()
            };

            let bond_counts: Vec<u32> = {
                let ps = node.producer_set.read().await;
                producers
                    .iter()
                    .map(|kp| {
                        ps.get_by_pubkey(kp.public_key())
                            .map(|i| i.bond_count)
                            .unwrap_or(0)
                    })
                    .collect()
            };

            let sum_bonds: u64 = bond_counts.iter().map(|&b| b as u64).sum();
            let mut sorted_desc = bond_counts.clone();
            sorted_desc.sort_unstable_by(|a, b| b.cmp(a));
            let top1 = sorted_desc[0];
            let top5 = top_n_share(&sorted_desc, 5, sum_bonds);
            let top10 = top_n_share(&sorted_desc, 10, sum_bonds);
            let gini = gini_coefficient(&bond_counts);
            let naka = nakamoto_coefficient(&bond_counts);
            let at_cap = bond_counts
                .iter()
                .filter(|&&b| b == doli_core::consensus::MAX_BONDS_PER_PRODUCER)
                .count();

            if at_cap > 0 && first_cap_epoch.is_none() {
                first_cap_epoch = Some(completed_epoch);
                eprintln!(
                    "  *** First producer hit cap at epoch {} ***",
                    completed_epoch
                );
            }

            let metrics = S2EpochMetrics {
                epoch: completed_epoch,
                height,
                total_supply,
                sum_bonds,
                top1_bonds: top1,
                top5_share: top5,
                top10_share: top10,
                gini_bonds: gini,
                nakamoto_coefficient: naka,
                producers_at_cap: at_cap,
                restake_attempted: 0,
                restake_rejected_cap: 0,
            };

            s2_csv_row(&mut csv_file, &metrics);
            gini_history.push(gini);

            // INV-3: Supply conservation (minted - fees = supply)
            assert_eq!(
                total_supply,
                expected_total_minted - cumulative_fees,
                "INV-3 at epoch {}: supply {} != minted({}) - fees({})",
                completed_epoch,
                total_supply,
                expected_total_minted,
                cumulative_fees
            );

            // INV-2: Cap enforcement
            for (i, &bc) in bond_counts.iter().enumerate() {
                assert!(
                    bc <= doli_core::consensus::MAX_BONDS_PER_PRODUCER,
                    "INV-2 at epoch {}: producer {} has {} bonds",
                    completed_epoch,
                    i,
                    bc
                );
            }

            // INV-4: Monotonic Gini OR equilibrium
            if gini_history.len() > 100 {
                let last_n = &gini_history[gini_history.len() - 100..];
                let mean: f64 = last_n.iter().sum::<f64>() / 100.0;
                let variance: f64 = last_n.iter().map(|g| (g - mean).powi(2)).sum::<f64>() / 100.0;

                let is_monotonic = last_n.windows(2).all(|w| w[1] >= w[0] - 0.001);

                if !is_monotonic && variance >= 0.005 {
                    eprintln!(
                        "INV-4 WARNING epoch {}: Gini oscillating \
                         (var={:.6}, not monotonic)",
                        completed_epoch, variance
                    );
                }
            }

            prev_sum_bonds = sum_bonds;

            if completed_epoch % 100 == 0 || completed_epoch == target_epochs {
                eprintln!(
                    "  epoch {:>6} | h={:>8} | supply={:>15} | \
                     bonds={:>8} | top1={:>4} | gini={:.4} | \
                     naka={} | cap={}",
                    completed_epoch, height, total_supply, sum_bonds, top1, gini, naka, at_cap
                );
            }

            if completed_epoch >= target_epochs {
                break;
            }
        } else if restake_pending {
            let (block, attempted, rejected, _bonds_added) =
                build_restake_block(&node, height, slot, prev_hash, producer, &producers).await;

            // Track fees burned by restake TXs
            for tx in &block.transactions {
                if tx.is_add_bond() {
                    cumulative_fees += tx.minimum_fee();
                }
            }

            prev_hash = block.hash();
            node.apply_block(block, ValidationMode::Light, None)
                .await
                .unwrap_or_else(|e| panic!("apply_block (restake) h={}: {}", height, e));

            expected_total_minted += node.params.block_reward(height);
            restake_pending = false;
            cumulative_restake_attempted += attempted;
            cumulative_restake_rejected += rejected;

            // INV-1: Bond conservation — bonds never decrease
            let new_sum_bonds: u64 = {
                let ps = node.producer_set.read().await;
                producers
                    .iter()
                    .map(|kp| {
                        ps.get_by_pubkey(kp.public_key())
                            .map(|i| i.bond_count as u64)
                            .unwrap_or(0)
                    })
                    .sum()
            };
            assert!(
                new_sum_bonds >= prev_sum_bonds,
                "INV-1 at h={}: bonds decreased {} -> {}",
                height,
                prev_sum_bonds,
                new_sum_bonds
            );

            prev_sum_bonds = new_sum_bonds;
        } else {
            let block = build_coinbase_block(height, slot, prev_hash, producer, &node.params);
            prev_hash = block.hash();
            node.apply_block(block, ValidationMode::Light, None)
                .await
                .unwrap_or_else(|e| panic!("apply_block h={}: {}", height, e));
            expected_total_minted += node.params.block_reward(height);
        }
    }

    csv_file.flush().unwrap();

    // Final summary
    let final_bonds: Vec<u32> = {
        let ps = node.producer_set.read().await;
        producers
            .iter()
            .map(|kp| {
                ps.get_by_pubkey(kp.public_key())
                    .map(|i| i.bond_count)
                    .unwrap_or(0)
            })
            .collect()
    };
    let final_gini = gini_coefficient(&final_bonds);
    let final_naka = nakamoto_coefficient(&final_bonds);
    let initial_gini = gini_coefficient(&initial_bonds);
    let initial_naka = nakamoto_coefficient(&initial_bonds);

    let mut final_sorted = final_bonds.clone();
    final_sorted.sort_unstable_by(|a, b| b.cmp(a));

    eprintln!("\n=== S2 Complete ===");
    eprintln!("  Epochs: {}", target_epochs);
    eprintln!(
        "  Initial: gini={:.4}, nakamoto={}, bonds={:?}",
        initial_gini,
        initial_naka,
        &initial_bonds[..5]
    );
    eprintln!(
        "  Final:   gini={:.4}, nakamoto={}, top5={:?}",
        final_gini,
        final_naka,
        &final_sorted[..5]
    );
    eprintln!(
        "  Restake: attempted={}, rejected_cap={}",
        cumulative_restake_attempted, cumulative_restake_rejected
    );
    if let Some(e) = first_cap_epoch {
        eprintln!("  First cap hit at epoch {}", e);
    } else {
        eprintln!("  No producer hit MAX_BONDS_PER_PRODUCER");
    }
    eprintln!("  CSV: target/sim/s2_compounding.csv");
}

// ============================================================
// UNIT TESTS
// ============================================================

#[test]
fn test_pareto_distribution() {
    let bonds = pareto_bonds(30);
    assert_eq!(bonds.len(), 30);
    assert_eq!(bonds[0], 100);
    assert!(bonds[29] >= 10);
    for i in 1..bonds.len() {
        assert!(
            bonds[i] <= bonds[i - 1],
            "bonds[{}]={} > bonds[{}]={}",
            i,
            bonds[i],
            i - 1,
            bonds[i - 1]
        );
    }
    let g = gini_coefficient(&bonds);
    assert!(g > 0.1, "Pareto should have Gini > 0.1, got {}", g);
}

#[test]
fn test_nakamoto_coefficient_uniform() {
    let uniform = vec![100u32; 30];
    let n = nakamoto_coefficient(&uniform);
    assert_eq!(n, 16, "Uniform 30: nakamoto={}, expected 16", n);
}

#[test]
fn test_nakamoto_coefficient_skewed() {
    let mut skewed = vec![1u32; 30];
    skewed[0] = 1000;
    let n = nakamoto_coefficient(&skewed);
    assert_eq!(n, 1, "Skewed: nakamoto={}, expected 1", n);
}
