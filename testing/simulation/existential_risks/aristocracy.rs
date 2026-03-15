//! PRUEBA 2: Simulacion de Elite (Aristocracia)

use super::*;

// ============================================================================
// Metrics
// ============================================================================

#[derive(Debug, Clone, Default)]
struct AristocracyMetrics {
    /// Gini coefficient of weight distribution
    gini: f64,
    /// Percentage of weight controlled by top 5%
    top5_concentration: f64,
    /// Percentage of weight controlled by founders
    founders_percentage: f64,
    /// Average age of producers in top 33% by weight
    top33_avg_age_years: f64,
    /// Number of producers
    total_producers: usize,
    /// Total weight
    total_weight: u64,
}

// ============================================================================
// Test
// ============================================================================

#[test]
fn test_aristocracy_simulation() {
    println!("\n=== PRUEBA 2: Simulacion de Elite (Aristocracia) ===\n");

    let network = Network::Devnet;
    let mut producers = ProducerSet::new();

    // Parameters
    let founder_count = 10;
    let new_producers_per_year = 10;
    let exit_rate_per_year = 0.05; // 5%
    let simulation_years = 20;

    // Register founders at block 0
    println!("Registrando {} fundadores en bloque 0...", founder_count);
    for i in 0..founder_count {
        let keypair = KeyPair::generate();
        let info = ProducerInfo::new(
            keypair.public_key().clone(),
            0, // registered_at block 0
            1000_000_000,
            (Hash::ZERO, i as u32),
            0, // era 0
        );
        producers.register(info, 0).unwrap();
    }

    // Track metrics by year
    let mut yearly_metrics: Vec<AristocracyMetrics> = Vec::new();

    for year in 0..=simulation_years {
        let current_block = year as u64 * DEVNET_BLOCKS_PER_YEAR;

        // Add new producers (not in year 0)
        if year > 0 {
            for i in 0..new_producers_per_year {
                let keypair = KeyPair::generate();
                let info = ProducerInfo::new(
                    keypair.public_key().clone(),
                    current_block,
                    1000_000_000,
                    (Hash::ZERO, (year * 100 + i) as u32),
                    0,
                );
                let _ = producers.register(info, current_block);
            }

            // Simulate exits (5% of producers)
            let exit_count = (producers.active_count() as f64 * exit_rate_per_year) as usize;
            // In real simulation, we'd select random producers to exit
            // For simplicity, we skip actual exits as they'd complicate tracking
        }

        // Calculate metrics
        let metrics = calculate_aristocracy_metrics(&producers, current_block, network, founder_count);

        println!("Ano {}: {} productores, peso total {}",
                 year, metrics.total_producers, metrics.total_weight);
        println!("  Gini: {:.3}, Top 5%: {:.1}%, Fundadores: {:.1}%",
                 metrics.gini, metrics.top5_concentration * 100.0, metrics.founders_percentage * 100.0);

        yearly_metrics.push(metrics);
    }

    // Final analysis
    println!("\n=== Analisis de Aristocracia ===\n");

    let final_metrics = yearly_metrics.last().unwrap();

    // Gini evaluation
    let gini_status = if final_metrics.gini < thresholds::GINI_HEALTHY {
        ("SALUDABLE", AlertLevel::Green)
    } else if final_metrics.gini < thresholds::GINI_CONCERNING {
        ("PREOCUPANTE", AlertLevel::Yellow)
    } else {
        ("CRITICO", AlertLevel::Red)
    };

    // Top 5% concentration evaluation
    let top5_status = if final_metrics.top5_concentration < thresholds::TOP5_HEALTHY {
        ("SALUDABLE", AlertLevel::Green)
    } else if final_metrics.top5_concentration < thresholds::TOP5_CONCERNING {
        ("PREOCUPANTE", AlertLevel::Yellow)
    } else {
        ("CRITICO", AlertLevel::Red)
    };

    // Founders evaluation
    let founders_status = if final_metrics.founders_percentage < thresholds::FOUNDERS_HEALTHY {
        ("SALUDABLE", AlertLevel::Green)
    } else if final_metrics.founders_percentage < thresholds::FOUNDERS_CONCERNING {
        ("PREOCUPANTE", AlertLevel::Yellow)
    } else {
        ("CRITICO", AlertLevel::Red)
    };

    println!("Metricas finales (Ano {}):", simulation_years);
    println!("  Coeficiente Gini: {:.3} - {} {:?}",
             final_metrics.gini, gini_status.0, gini_status.1);
    println!("  Top 5% controla: {:.1}% - {} {:?}",
             final_metrics.top5_concentration * 100.0, top5_status.0, top5_status.1);
    println!("  Fundadores controlan: {:.1}% - {} {:?}",
             final_metrics.founders_percentage * 100.0, founders_status.0, founders_status.1);

    // Evolution summary
    println!("\nEvolucion del poder de fundadores:");
    for (year, metrics) in yearly_metrics.iter().enumerate() {
        let bar_len = (metrics.founders_percentage * 50.0) as usize;
        let bar: String = "█".repeat(bar_len);
        println!("  Ano {:2}: {:5.1}% {}", year, metrics.founders_percentage * 100.0, bar);
    }

    // Assertions for healthy network
    assert!(
        final_metrics.gini < thresholds::GINI_CONCERNING,
        "Gini ({:.3}) indica concentracion excesiva de poder",
        final_metrics.gini
    );
}

// ============================================================================
// Helpers
// ============================================================================

fn calculate_aristocracy_metrics(
    producers: &ProducerSet,
    current_block: u64,
    network: Network,
    founder_count: usize,
) -> AristocracyMetrics {
    let active = producers.active_producers();
    let total_producers = active.len();

    if total_producers == 0 {
        return AristocracyMetrics::default();
    }

    // Calculate weights
    let mut weights: Vec<(u64, bool)> = active.iter().map(|p| {
        let weight = producer_weight_for_network(p.registered_at, current_block, network);
        let is_founder = p.registered_at == 0;
        (weight, is_founder)
    }).collect();

    weights.sort_by(|a, b| b.0.cmp(&a.0)); // Sort descending

    let total_weight: u64 = weights.iter().map(|(w, _)| *w).sum();

    // Gini coefficient
    let gini = calculate_gini(&weights.iter().map(|(w, _)| *w).collect::<Vec<_>>());

    // Top 5% concentration
    let top5_count = (total_producers as f64 * 0.05).ceil() as usize;
    let top5_weight: u64 = weights.iter().take(top5_count).map(|(w, _)| *w).sum();
    let top5_concentration = top5_weight as f64 / total_weight as f64;

    // Founders percentage
    let founders_weight: u64 = weights.iter()
        .filter(|(_, is_founder)| *is_founder)
        .map(|(w, _)| *w)
        .sum();
    let founders_percentage = founders_weight as f64 / total_weight as f64;

    // Top 33% average age
    let top33_count = (total_producers as f64 * 0.33).ceil() as usize;
    // This would require tracking registration times, simplified here
    let top33_avg_age_years = 0.0; // Placeholder

    AristocracyMetrics {
        gini,
        top5_concentration,
        founders_percentage,
        top33_avg_age_years,
        total_producers,
        total_weight,
    }
}
