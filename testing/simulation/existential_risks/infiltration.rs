//! PRUEBA 3: Ataque de Infiltracion Lenta

use super::*;

// ============================================================================
// Types
// ============================================================================

#[derive(Debug, Clone)]
struct AttackSimulationResult {
    /// Attacker nodes
    attacker_nodes: u64,
    /// Total DOLI cost for attacker
    doli_cost: u64,
    /// Total server cost (4 years) in USD
    server_cost_usd: u64,
    /// Year when attacker reaches 33% (None if never)
    year_reaches_veto: Option<u64>,
    /// Final attacker percentage
    final_attacker_pct: f64,
    /// Assessment
    assessment: SecurityAssessment,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SecurityAssessment {
    Critical,    // < $500K to capture
    Risky,       // $500K - $2M
    Acceptable,  // $2M - $10M
    Secure,      // > $10M
}

// ============================================================================
// Test
// ============================================================================

#[test]
fn test_slow_infiltration_attack() {
    println!("\n=== PRUEBA 3: Ataque de Infiltracion Lenta ===\n");

    let network = Network::Devnet;

    // Test different attacker sizes
    let attacker_scenarios = [50, 100, 200, 300, 500];
    let mut results: Vec<AttackSimulationResult> = Vec::new();

    for &attacker_nodes in &attacker_scenarios {
        let result = simulate_slow_attack(attacker_nodes, network);
        results.push(result);
    }

    // Print results table
    println!("Resultados de Simulacion de Ataque:\n");
    println!("{:>10} {:>12} {:>12} {:>15} {:>10} {:>15}",
             "Nodos", "Costo DOLI", "Servidores", "Alcanza 33%", "Final %", "Evaluacion");
    println!("{}", "-".repeat(75));

    for result in &results {
        let veto_year = result.year_reaches_veto
            .map(|y| format!("Ano {}", y))
            .unwrap_or_else(|| "Nunca".to_string());

        println!("{:>10} {:>12} {:>12} {:>15} {:>9.1}% {:>15?}",
                 result.attacker_nodes,
                 format!("{}K", result.doli_cost / 1000),
                 format!("${}K", result.server_cost_usd / 1000),
                 veto_year,
                 result.final_attacker_pct * 100.0,
                 result.assessment);
    }

    // Find minimum nodes to reach veto
    let min_veto_nodes = results.iter()
        .filter(|r| r.year_reaches_veto.is_some())
        .min_by_key(|r| r.attacker_nodes)
        .map(|r| r.attacker_nodes);

    println!("\n=== Analisis de Seguridad ===\n");

    if let Some(min_nodes) = min_veto_nodes {
        let attack_result = results.iter().find(|r| r.attacker_nodes == min_nodes).unwrap();
        let total_cost = attack_result.doli_cost + attack_result.server_cost_usd;

        println!("Costo minimo para alcanzar veto: {} nodos", min_nodes);
        println!("  DOLI requerido: {} DOLI", attack_result.doli_cost);
        println!("  Servidores (4 anos): ${}", attack_result.server_cost_usd);
        println!("  Costo total: ${}", total_cost);
        println!("  Alcanza 33% en: Ano {}", attack_result.year_reaches_veto.unwrap());

        match attack_result.assessment {
            SecurityAssessment::Critical => {
                println!("\n  [CRITICO] Red capturable por ${} - INACEPTABLE", total_cost);
                println!("  Accion requerida: aumentar costo de registro o tiempo de maduracion");
            }
            SecurityAssessment::Risky => {
                println!("\n  [RIESGOSO] Ataque costaria ${} - vulnerable a estados-nacion", total_cost);
                println!("  Considerar: mecanismos adicionales de deteccion de Sybil");
            }
            SecurityAssessment::Acceptable => {
                println!("\n  [ACEPTABLE] Ataque costaria ${} - comparable a redes PoS", total_cost);
            }
            SecurityAssessment::Secure => {
                println!("\n  [SEGURO] Ataque costaria mas de $10M - economia saludable");
            }
        }
    } else {
        println!("  [EXCELENTE] Ningun escenario de ataque alcanza 33% en 10 anos");
    }

    // Detailed yearly breakdown for critical scenario
    println!("\n=== Progresion de Ataque (300 nodos) ===\n");
    let detailed = simulate_slow_attack_detailed(300, network);

    for (year, pct) in detailed {
        let bar_len = (pct * 100.0) as usize;
        let bar: String = "█".repeat(bar_len);
        let threshold = if pct >= 0.33 { " <-- VETO" } else { "" };
        println!("  Ano {:2}: {:5.1}% {}{}", year, pct * 100.0, bar, threshold);
    }
}

// ============================================================================
// Simulation Helpers
// ============================================================================

fn simulate_slow_attack(attacker_nodes: u64, network: Network) -> AttackSimulationResult {
    let simulation_years = 10;
    let initial_honest_nodes = 500;
    let honest_growth_rate = 0.10; // 10% per year

    let doli_per_node = 1000; // 1000 DOLI bond
    let server_cost_per_month = 50; // $50/month per server

    let doli_cost = attacker_nodes * doli_per_node;
    let server_cost_usd = attacker_nodes * server_cost_per_month * 12 * 4; // 4 years

    let mut year_reaches_veto: Option<u64> = None;
    let mut final_attacker_pct = 0.0;

    let mut honest_nodes = initial_honest_nodes as f64;

    // Activity gap penalty for dormant nodes:
    // - Each week without activity = 1 gap
    // - Each gap = 10% weight reduction
    // - Max penalty = 50%
    //
    // Honest nodes actively produce blocks, so they have 0 gaps.
    // Dormant attacker nodes ("register and wait") accumulate max gaps quickly.
    const DORMANT_PENALTY: f64 = 0.50; // 50% weight reduction for dormant nodes

    for year in 0..=simulation_years {
        // Calculate honest network EFFECTIVE weight
        // Honest nodes are active, so they have full weight (no gap penalty)
        let avg_honest_age_months = (year as f64 / 2.0) * 12.0;
        let avg_honest_weight = 1.0 + (avg_honest_age_months / 12.0).sqrt();
        let honest_weight = honest_nodes * avg_honest_weight.min(MAX_WEIGHT as f64);

        // Attacker EFFECTIVE weight (dormant, so 50% penalty)
        // They have seniority weight but can't USE it for veto because they're dormant
        let attacker_age_months = year as f64 * 12.0;
        let attacker_base_weight = 1.0 + (attacker_age_months / 12.0).sqrt();
        let attacker_effective_weight = attacker_base_weight.min(MAX_WEIGHT as f64) * (1.0 - DORMANT_PENALTY);
        let attacker_weight = attacker_nodes as f64 * attacker_effective_weight;

        let total_weight = honest_weight + attacker_weight;
        let attacker_pct = attacker_weight / total_weight;

        if attacker_pct >= 0.33 && year_reaches_veto.is_none() {
            year_reaches_veto = Some(year as u64);
        }

        final_attacker_pct = attacker_pct;

        // Honest network grows
        honest_nodes *= 1.0 + honest_growth_rate;
    }

    let total_cost = (doli_cost * 2) + server_cost_usd; // Assume $2/DOLI
    let assessment = if total_cost < thresholds::ATTACK_COST_CRITICAL {
        SecurityAssessment::Critical
    } else if total_cost < thresholds::ATTACK_COST_RISKY {
        SecurityAssessment::Risky
    } else if total_cost < thresholds::ATTACK_COST_ACCEPTABLE {
        SecurityAssessment::Acceptable
    } else {
        SecurityAssessment::Secure
    };

    AttackSimulationResult {
        attacker_nodes,
        doli_cost,
        server_cost_usd,
        year_reaches_veto,
        final_attacker_pct,
        assessment,
    }
}

fn simulate_slow_attack_detailed(attacker_nodes: u64, network: Network) -> Vec<(u64, f64)> {
    let simulation_years = 10;
    let initial_honest_nodes = 500;
    let honest_growth_rate = 0.10;

    // 50% penalty for dormant attackers (max activity gap penalty)
    const DORMANT_PENALTY: f64 = 0.50;

    let mut results = Vec::new();
    let mut honest_nodes = initial_honest_nodes as f64;

    for year in 0..=simulation_years {
        // Honest nodes have full effective weight (active, no gaps)
        let avg_honest_age_months = (year as f64 / 2.0) * 12.0;
        let avg_honest_weight = 1.0 + (avg_honest_age_months / 12.0).sqrt();
        let honest_weight = honest_nodes * avg_honest_weight.min(MAX_WEIGHT as f64);

        // Dormant attackers have 50% weight penalty
        let attacker_age_months = year as f64 * 12.0;
        let attacker_base_weight = 1.0 + (attacker_age_months / 12.0).sqrt();
        let attacker_effective_weight = attacker_base_weight.min(MAX_WEIGHT as f64) * (1.0 - DORMANT_PENALTY);
        let attacker_weight = attacker_nodes as f64 * attacker_effective_weight;

        let total_weight = honest_weight + attacker_weight;
        let attacker_pct = attacker_weight / total_weight;

        results.push((year as u64, attacker_pct));
        honest_nodes *= 1.0 + honest_growth_rate;
    }

    results
}
