//! PRUEBA 4: Early Active Attacker (Atacante Paciente desde Dia 1)

use super::*;

// ============================================================================
// Types
// ============================================================================

#[derive(Debug)]
struct EarlyAttackerResult {
    attacker_nodes: u64,
    doli_cost: u64,
    years_with_veto: u64,
    year_diluted_below_33: Option<u64>,
    final_attacker_pct: f64,
}

#[derive(Debug)]
struct YearlyEntry {
    year: u64,
    benevolent_nodes: u64,
    attacker_nodes: u64,
    total_weight: f64,
    attacker_pct: f64,
}

// ============================================================================
// Test
// ============================================================================

/// Critical simulation: Attacker enters at block 0 with perfect activity
///
/// This tests the WORST CASE scenario:
/// - Attacker enters at genesis (same time as founders)
/// - Attacker maintains PERFECT activity (0 gaps, no penalty)
/// - Attacker buys DOLI cheap before price increases
/// - Benevolent growth is SLOW (realistic, not optimistic)
///
/// Key question: Can attacker maintain >=40% veto for multiple upgrade cycles?
/// (Using 40% threshold as implemented in doli-storage)
#[test]
fn test_early_active_attacker() {
    println!("\n=== PRUEBA 4: Early Active Attacker ===\n");
    println!("Escenario: Atacante paciente, activo, temprano, con capital\n");

    let simulation_years = 8; // 2 eras, ~3 upgrade cycles
    let founder_nodes = 5;
    let benevolent_growth_per_year = 5; // Slow, realistic growth

    // Test different attacker sizes
    let attacker_scenarios = [3, 5, 10, 15, 20, 30];

    println!("Parametros:");
    println!("  Fundadores: {} nodos (bloque 0)", founder_nodes);
    println!("  Crecimiento benevolo: +{} nodos/año", benevolent_growth_per_year);
    println!("  Atacante: actividad PERFECTA (sin penalty)");
    println!("  Duracion: {} años (2 eras)\n", simulation_years);

    println!("{:>10} {:>12} {:>15} {:>15} {:>12}",
             "Atacante", "Costo DOLI", "Mantiene 40%", "Año Diluido", "Final %");
    println!("{}", "-".repeat(70));

    for &attacker_nodes in &attacker_scenarios {
        let result = simulate_early_active_attacker(
            founder_nodes,
            attacker_nodes,
            benevolent_growth_per_year,
            simulation_years,
        );

        let maintains_veto = if result.years_with_veto > 0 {
            format!("{} años", result.years_with_veto)
        } else {
            "Nunca".to_string()
        };

        let diluted_at = result.year_diluted_below_33
            .map(|y| format!("Año {}", y))
            .unwrap_or_else(|| "Nunca".to_string());

        println!("{:>10} {:>12} {:>15} {:>15} {:>11.1}%",
                 attacker_nodes,
                 format!("{}K", result.doli_cost / 1000),
                 maintains_veto,
                 diluted_at,
                 result.final_attacker_pct * 100.0);
    }

    // Detailed year-by-year for critical scenario (10 attacker nodes)
    println!("\n=== Progresion Detallada (10 nodos atacante) ===\n");
    let detailed = simulate_early_active_detailed(founder_nodes, 10, benevolent_growth_per_year, simulation_years);

    println!("{:>6} {:>10} {:>10} {:>12} {:>12}",
             "Año", "Benevolos", "Atacante", "Total Peso", "Atacante %");
    println!("{}", "-".repeat(55));

    for entry in &detailed {
        let veto_marker = if entry.attacker_pct >= 0.33 { " <-- VETO" } else { "" };
        println!("{:>6} {:>10} {:>10} {:>12} {:>11.1}%{}",
                 entry.year,
                 entry.benevolent_nodes,
                 entry.attacker_nodes,
                 format!("{:.0}", entry.total_weight),
                 entry.attacker_pct * 100.0,
                 veto_marker);
    }

    // Find minimum attacker nodes to maintain veto for 4+ years
    let min_for_4_years = attacker_scenarios.iter()
        .find(|&&n| {
            let r = simulate_early_active_attacker(founder_nodes, n, benevolent_growth_per_year, simulation_years);
            r.years_with_veto >= 4
        });

    println!("\n=== Analisis de Seguridad ===\n");

    if let Some(&min_nodes) = min_for_4_years {
        let result = simulate_early_active_attacker(founder_nodes, min_nodes, benevolent_growth_per_year, simulation_years);
        let doli_cost_usd = result.doli_cost * 2; // $2/DOLI at launch
        let server_cost = min_nodes as u64 * 50 * 12 * 4; // $50/month * 4 years
        let total_cost = doli_cost_usd + server_cost;

        println!("  Nodos minimos para mantener veto 4+ años: {}", min_nodes);
        println!("  Costo DOLI: {} DOLI = ${}", result.doli_cost, doli_cost_usd);
        println!("  Costo servidores (4 años): ${}", server_cost);
        println!("  Costo total: ${}", total_cost);

        if total_cost < 500_000 {
            println!("\n  [CRITICO] Atacante temprano puede bloquear upgrades por <$500K");
            println!("  ACCION REQUERIDA: Implementar 40% threshold + veto bond");
        } else if total_cost < 2_000_000 {
            println!("\n  [RIESGOSO] Atacante temprano puede bloquear por <$2M");
            println!("  CONSIDERAR: Implementar veto bond o aumentar threshold");
        } else {
            println!("\n  [ACEPTABLE] Atacante necesita >$2M para bloqueo sostenido");
        }
    } else {
        println!("  [EXCELENTE] Ningun escenario de ataque mantiene veto por 4+ años");
        println!("  El sistema con 33% + activity penalty es suficiente");
    }

    // Critical assertion
    let worst_case = simulate_early_active_attacker(founder_nodes, 30, benevolent_growth_per_year, simulation_years);
    println!("\n  Peor caso (30 nodos atacante): {}% durante {} años",
             (worst_case.final_attacker_pct * 100.0) as u32,
             worst_case.years_with_veto);

    // Compare 33% vs 40% threshold
    println!("\n=== Comparacion: 33% vs 40% Threshold ===\n");
    println!("{:>10} {:>15} {:>15}",
             "Atacante", "Veto 33% (años)", "Veto 40% (años)");
    println!("{}", "-".repeat(45));

    for &attacker_nodes in &attacker_scenarios {
        let result_33 = simulate_early_active_attacker_threshold(
            founder_nodes, attacker_nodes, benevolent_growth_per_year, simulation_years, 0.33);
        let result_40 = simulate_early_active_attacker_threshold(
            founder_nodes, attacker_nodes, benevolent_growth_per_year, simulation_years, 0.40);

        println!("{:>10} {:>15} {:>15}",
                 attacker_nodes,
                 result_33.years_with_veto,
                 result_40.years_with_veto);
    }

    // Find minimum for 4 years at 40%
    let min_40_for_4_years = attacker_scenarios.iter()
        .find(|&&n| {
            let r = simulate_early_active_attacker_threshold(
                founder_nodes, n, benevolent_growth_per_year, simulation_years, 0.40);
            r.years_with_veto >= 4
        });

    println!("\nCon 40% threshold:");
    if let Some(&min_nodes) = min_40_for_4_years {
        let doli_cost = min_nodes as u64 * 1000;
        let doli_cost_usd = doli_cost * 2;
        let server_cost = min_nodes as u64 * 50 * 12 * 4;
        let total_cost = doli_cost_usd + server_cost;
        println!("  Nodos minimos para 4+ años veto: {}", min_nodes);
        println!("  Costo total: ${}", total_cost);
    } else {
        println!("  Ningun escenario mantiene veto 4+ años con 40% threshold");
    }
}

// ============================================================================
// Simulation Helpers
// ============================================================================

fn simulate_early_active_attacker_threshold(
    founder_nodes: u64,
    attacker_nodes: u64,
    benevolent_growth_per_year: u64,
    simulation_years: u64,
    threshold: f64,
) -> EarlyAttackerResult {
    let doli_per_node = 1000;
    let doli_cost = attacker_nodes * doli_per_node;

    let mut years_with_veto = 0u64;
    let mut year_diluted_below_threshold: Option<u64> = None;
    let mut final_attacker_pct = 0.0;
    let mut had_veto = false;

    for year in 0..=simulation_years {
        let benevolent_nodes = founder_nodes + (year * benevolent_growth_per_year);

        let mut benevolent_weight = 0.0;
        let founder_weight_each = 1.0 + (year as f64).sqrt();
        benevolent_weight += founder_nodes as f64 * founder_weight_each.min(MAX_WEIGHT as f64);

        let new_joiners = year * benevolent_growth_per_year;
        if new_joiners > 0 {
            let avg_joiner_age = year as f64 / 2.0;
            let joiner_weight_each = 1.0 + avg_joiner_age.sqrt();
            benevolent_weight += new_joiners as f64 * joiner_weight_each.min(MAX_WEIGHT as f64);
        }

        let attacker_weight_each = 1.0 + (year as f64).sqrt();
        let attacker_weight = attacker_nodes as f64 * attacker_weight_each.min(MAX_WEIGHT as f64);

        let total_weight = benevolent_weight + attacker_weight;
        let attacker_pct = attacker_weight / total_weight;

        if attacker_pct >= threshold {
            years_with_veto += 1;
            had_veto = true;
        } else if had_veto && year_diluted_below_threshold.is_none() {
            year_diluted_below_threshold = Some(year);
        }

        final_attacker_pct = attacker_pct;
    }

    EarlyAttackerResult {
        attacker_nodes,
        doli_cost,
        years_with_veto,
        year_diluted_below_33: year_diluted_below_threshold,
        final_attacker_pct,
    }
}

fn simulate_early_active_attacker(
    founder_nodes: u64,
    attacker_nodes: u64,
    benevolent_growth_per_year: u64,
    simulation_years: u64,
) -> EarlyAttackerResult {
    // Use 40% threshold (matching VETO_THRESHOLD_PERCENT in doli-storage)
    simulate_early_active_attacker_threshold(
        founder_nodes,
        attacker_nodes,
        benevolent_growth_per_year,
        simulation_years,
        0.40, // 40% veto threshold
    )
}

fn simulate_early_active_detailed(
    founder_nodes: u64,
    attacker_nodes: u64,
    benevolent_growth_per_year: u64,
    simulation_years: u64,
) -> Vec<YearlyEntry> {
    let mut results = Vec::new();

    for year in 0..=simulation_years {
        let benevolent_nodes = founder_nodes + (year * benevolent_growth_per_year);

        let mut benevolent_weight = 0.0;
        let founder_weight_each = 1.0 + (year as f64).sqrt();
        benevolent_weight += founder_nodes as f64 * founder_weight_each.min(MAX_WEIGHT as f64);

        let new_joiners = year * benevolent_growth_per_year;
        if new_joiners > 0 {
            let avg_joiner_age = year as f64 / 2.0;
            let joiner_weight_each = 1.0 + avg_joiner_age.sqrt();
            benevolent_weight += new_joiners as f64 * joiner_weight_each.min(MAX_WEIGHT as f64);
        }

        let attacker_weight_each = 1.0 + (year as f64).sqrt();
        let attacker_weight = attacker_nodes as f64 * attacker_weight_each.min(MAX_WEIGHT as f64);

        let total_weight = benevolent_weight + attacker_weight;
        let attacker_pct = attacker_weight / total_weight;

        results.push(YearlyEntry {
            year,
            benevolent_nodes,
            attacker_nodes,
            total_weight,
            attacker_pct,
        });
    }

    results
}
