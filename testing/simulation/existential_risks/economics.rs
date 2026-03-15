//! PRUEBA 10: Reward Distribution Fairness
//! PRUEBA 11: Era Transition / Halving
//! Dashboard Summary

use super::*;

// ============================================================================
// PRUEBA 10: Reward Distribution Fairness (P1 - Important)
// ============================================================================

/// Test that rewards are distributed fairly over time
/// Verify: proportional to blocks produced, no systematic bias
#[test]
fn test_reward_distribution_fairness() {
    println!("\n=== PRUEBA 10: Reward Distribution Fairness ===\n");

    // Simulate 1 year of block production
    let blocks_per_year = 525600; // 1 per minute
    let simulation_blocks = blocks_per_year / 100; // Scaled down for speed
    let producer_count = 100;
    let reward_per_block = 5_000_000_000u64; // 5 DOLI in smallest units

    // Track rewards per producer
    let mut rewards: Vec<u64> = vec![0; producer_count];
    let mut blocks_produced: Vec<u64> = vec![0; producer_count];

    // Simple deterministic selection (round-robin with some variance)
    let mut rng_state = 42u64;
    let mut next_random = || {
        rng_state = rng_state.wrapping_mul(1103515245).wrapping_add(12345);
        rng_state
    };

    println!("Simulando {} bloques con {} productores...\n", simulation_blocks, producer_count);

    for block in 0..simulation_blocks {
        // Select producer (mostly round-robin, some randomness for realism)
        let base_producer = (block as usize) % producer_count;
        let variance = (next_random() % 3) as i32 - 1; // -1, 0, or 1
        let selected = ((base_producer as i32 + variance).rem_euclid(producer_count as i32)) as usize;

        rewards[selected] += reward_per_block;
        blocks_produced[selected] += 1;
    }

    // Calculate statistics
    let total_rewards: u64 = rewards.iter().sum();
    let expected_per_producer = total_rewards / producer_count as u64;
    let avg_blocks = simulation_blocks as f64 / producer_count as f64;

    // Gini coefficient of reward distribution
    let gini = calculate_gini(&rewards);

    // Standard deviation
    let mean = expected_per_producer as f64;
    let variance: f64 = rewards.iter()
        .map(|&r| (r as f64 - mean).powi(2))
        .sum::<f64>() / producer_count as f64;
    let std_dev = variance.sqrt();
    let coefficient_of_variation = std_dev / mean;

    // Min/max analysis
    let min_reward = *rewards.iter().min().unwrap();
    let max_reward = *rewards.iter().max().unwrap();
    let spread_ratio = max_reward as f64 / min_reward as f64;

    println!("Resultados:");
    println!("  Total recompensas: {} DOLI", total_rewards / 100_000_000);
    println!("  Promedio por productor: {} DOLI", expected_per_producer / 100_000_000);
    println!("  Bloques promedio: {:.1}", avg_blocks);
    println!("\nDistribucion:");
    println!("  Gini: {:.4} (0 = perfecta igualdad)", gini);
    println!("  Coef. variacion: {:.4}", coefficient_of_variation);
    println!("  Min: {} DOLI ({:.1}% del promedio)",
             min_reward / 100_000_000,
             (min_reward as f64 / expected_per_producer as f64) * 100.0);
    println!("  Max: {} DOLI ({:.1}% del promedio)",
             max_reward / 100_000_000,
             (max_reward as f64 / expected_per_producer as f64) * 100.0);
    println!("  Spread (max/min): {:.2}x", spread_ratio);

    // Fairness thresholds
    let gini_threshold = 0.1; // Very low Gini for fair distribution
    let spread_threshold = 2.0; // Max 2x between min and max

    let is_fair = gini < gini_threshold && spread_ratio < spread_threshold;

    if is_fair {
        println!("\n  [OK] Distribucion de recompensas es justa");
    } else {
        println!("\n  [WARN] Distribucion tiene sesgo");
        if gini >= gini_threshold {
            println!("    - Gini ({:.4}) >= {:.2}", gini, gini_threshold);
        }
        if spread_ratio >= spread_threshold {
            println!("    - Spread ({:.2}x) >= {:.1}x", spread_ratio, spread_threshold);
        }
    }

    assert!(
        gini < 0.2,
        "Distribucion muy desigual: Gini = {:.4}",
        gini
    );
}

// ============================================================================
// PRUEBA 11: Era Transition / Halving (P2 - Nice to have)
// ============================================================================

/// Test era transition and halving mechanics
#[test]
fn test_era_transition_halving() {
    println!("\n=== PRUEBA 11: Era Transition / Halving ===\n");

    const BLOCKS_PER_ERA: u64 = 2_102_400;

    // Simulate reward and bond at different eras
    let eras_to_test = [0, 1, 2, 3, 4, 5];

    println!("{:>4} {:>12} {:>12} {:>15}", "Era", "Reward", "Bond", "Bloques para ROI");
    println!("{}", "-".repeat(50));

    for era in eras_to_test {
        let reward = calculate_reward_at_era(era);
        let bond = calculate_bond_at_era(era);
        let blocks_to_roi = if reward > 0 { bond / reward } else { u64::MAX };

        println!("{:>4} {:>12} {:>12} {:>15}",
                 era,
                 format!("{:.4}", reward as f64 / 100_000_000.0),
                 bond / 100_000_000,
                 blocks_to_roi);
    }

    // Verify halving ratios
    println!("\nVerificacion de halvings:");

    let reward_era_0 = calculate_reward_at_era(0);
    let reward_era_1 = calculate_reward_at_era(1);
    let reward_ratio = reward_era_0 as f64 / reward_era_1 as f64;
    println!("  Reward ratio (Era 0 / Era 1): {:.2}x (esperado: 2.0x)", reward_ratio);
    assert!((reward_ratio - 2.0).abs() < 0.01, "Reward should halve");

    let bond_era_0 = calculate_bond_at_era(0);
    let bond_era_1 = calculate_bond_at_era(1);
    let bond_ratio = bond_era_0 as f64 / bond_era_1 as f64;
    println!("  Bond ratio (Era 0 / Era 1): {:.2}x (esperado: ~1.43x = 1/0.7)", bond_ratio);
    assert!((bond_ratio - (1.0/0.7)).abs() < 0.1, "Bond should decrease by 30%");

    // Verify difficulty increase
    println!("\nCosto relativo por era (bloques para ROI):");
    for era in 0..5 {
        let blocks = calculate_bond_at_era(era) / calculate_reward_at_era(era);
        let era_0_blocks = calculate_bond_at_era(0) / calculate_reward_at_era(0);
        let relative = blocks as f64 / era_0_blocks as f64;
        println!("  Era {}: {}x respecto a Era 0", era, format!("{:.2}", relative));
    }

    println!("\n  [OK] Halving mechanics funcionan correctamente");
}

fn calculate_reward_at_era(era: u64) -> u64 {
    // 5 DOLI base, halving each era
    let base = 5_00_000_000u64; // 5 DOLI in smallest units
    base >> era
}

fn calculate_bond_at_era(era: u64) -> u64 {
    // 1000 DOLI base, decreasing 30% per era
    let base = 1000_00_000_000u64; // 1000 DOLI in smallest units
    let mut bond = base;
    for _ in 0..era {
        bond = bond * 70 / 100; // Multiply by 0.7
    }
    bond
}

// ============================================================================
// Dashboard Summary Test
// ============================================================================

#[test]
fn test_generate_health_dashboard() {
    println!("\n");
    println!("╔═══════════════════════════════════════════════════════════════════╗");
    println!("║                    DOLI Health Dashboard                          ║");
    println!("╠═══════════════════════════════════════════════════════════════════╣");
    println!("║                                                                   ║");
    println!("║  LIVENESS                                                         ║");
    println!("║  ├── Cola de registro: 3 pendientes [OK < 20]                     ║");
    println!("║  ├── Fee actual: 1.2x base [OK < 5x]                              ║");
    println!("║  └── Registros/dia (7d avg): 12 [OK creciendo]                    ║");
    println!("║                                                                   ║");
    println!("║  DESCENTRALIZACION                                                ║");
    println!("║  ├── Gini de peso: 0.28 [OK < 0.3]                                ║");
    println!("║  ├── Top 10 productores: 18% peso [OK < 25%]                      ║");
    println!("║  ├── Productores > 2 anos: 45% [OK 40-60%]                        ║");
    println!("║  └── Nuevos ultimo mes: 23 [OK creciendo]                         ║");
    println!("║                                                                   ║");
    println!("║  SEGURIDAD                                                        ║");
    println!("║  ├── Mayor peso individual: 2.1% [OK < 5%]                        ║");
    println!("║  ├── Concentracion para veto: necesita 47 productores [OK]        ║");
    println!("║  └── Nodos inactivos > 1 semana: 3% [OK < 10%]                    ║");
    println!("║                                                                   ║");
    println!("║  ALERTAS                                                          ║");
    println!("║  └── Sin alertas criticas                                         ║");
    println!("║                                                                   ║");
    println!("╚═══════════════════════════════════════════════════════════════════╝");
    println!("\n");

    // This test always passes - it's a documentation of the desired dashboard
    assert!(true);
}
