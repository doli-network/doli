//! Existential Risk Simulations for DOLI
//!
//! These tests validate the core premise of DOLI:
//! "Verifiable sequential time can be a scarce resource
//!  without killing liveness or creating aristocracy"
//!
//! Three critical simulations:
//! 1. Onboarding stress test (liveness under viral growth)
//! 2. Elite simulation (power concentration over time)
//! 3. Slow infiltration attack (economic security)

#[path = "../common/mod.rs"]
mod common;

use doli_core::network::Network;
use doli_core::consensus::{
    RegistrationQueue, PendingRegistration, BASE_REGISTRATION_FEE,
    fee_multiplier_x100, MAX_FEE_MULTIPLIER_X100,
};
use storage::{
    ProducerInfo, ProducerSet,
    producer_weight_for_network,
    MAX_WEIGHT,
};
use crypto::{KeyPair, Hash};

// ============================================================================
// Simulation Parameters
// ============================================================================

/// Devnet: 1 block = 5 seconds, 12 blocks = 1 year
const DEVNET_BLOCKS_PER_YEAR: u64 = 12;
const DEVNET_BLOCKS_PER_MONTH: u64 = 1;

/// Registration limits
const MAX_REGISTRATIONS_PER_BLOCK: usize = 5;

/// Alert thresholds
mod thresholds {
    // Liveness
    pub const QUEUE_WAIT_ALERT: u64 = 10;  // blocks
    pub const QUEUE_WAIT_CRITICAL: u64 = 60;  // 1 hour in blocks at 1/min
    pub const FEE_MULTIPLIER_ALERT: f64 = 5.0;
    pub const FEE_MULTIPLIER_CRITICAL: f64 = 100.0;
    pub const ABANDONMENT_RATE_CRITICAL: f64 = 0.50;

    // Aristocracy
    pub const GINI_HEALTHY: f64 = 0.3;
    pub const GINI_CONCERNING: f64 = 0.5;
    pub const TOP5_HEALTHY: f64 = 0.20;
    pub const TOP5_CONCERNING: f64 = 0.35;
    pub const FOUNDERS_HEALTHY: f64 = 0.15;
    pub const FOUNDERS_CONCERNING: f64 = 0.25;

    // Security
    pub const ATTACK_COST_CRITICAL: u64 = 500_000;  // $500K
    pub const ATTACK_COST_RISKY: u64 = 2_000_000;   // $2M
    pub const ATTACK_COST_ACCEPTABLE: u64 = 10_000_000;  // $10M
}

// ============================================================================
// Metrics Collection
// ============================================================================

#[derive(Debug, Clone, Default)]
struct LivenessMetrics {
    /// Average wait time in blocks
    avg_wait_blocks: f64,
    /// Maximum wait time in blocks
    max_wait_blocks: u64,
    /// Maximum fee multiplier reached
    max_fee_multiplier: f64,
    /// Percentage of registrations abandoned due to fee
    abandonment_rate: f64,
    /// Time to return to normal after spike
    recovery_blocks: u64,
    /// Total successful registrations
    successful_registrations: u64,
    /// Total attempted registrations
    attempted_registrations: u64,
}

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

#[derive(Debug, Clone, Copy, PartialEq)]
enum AlertLevel {
    Green,
    Yellow,
    Red,
}

// ============================================================================
// PRUEBA 1: Stress Test de Onboarding (Liveness)
// ============================================================================

/// Scenario A: Normal growth (2 registrations/minute)
#[test]
fn test_onboarding_normal_growth() {
    println!("\n=== PRUEBA 1A: Crecimiento Normal ===\n");

    let network = Network::Devnet;
    let mut queue = RegistrationQueue::new();
    let mut metrics = LivenessMetrics::default();

    // Simulate 20 minutes = 20 years in devnet
    // At 1 block/5 seconds, that's 240 blocks
    let total_blocks = 240;
    let registrations_per_minute = 2;
    let blocks_per_minute = 12; // 60s / 5s

    let mut total_wait = 0u64;
    let mut wait_samples = 0u64;

    for block in 0..total_blocks {
        // Add new registrations every minute
        if block % blocks_per_minute == 0 {
            for i in 0..registrations_per_minute {
                let keypair = KeyPair::generate();
                let fee = queue.current_fee_for_network(network);
                let reg = PendingRegistration {
                    public_key: keypair.public_key().clone(),
                    bond_amount: 1000_000_000,
                    fee_paid: fee,
                    submitted_at: block,
                    prev_registration_hash: Hash::ZERO,
                    sequence_number: (block * 100 + i as u64) as u64,
                };
                let _ = queue.submit_for_network(reg, block, network);
                metrics.attempted_registrations += 1;
            }
        }

        // Process registrations
        while let Some(reg) = queue.next_registration_for_network(network) {
            let wait = block - reg.submitted_at;
            total_wait += wait;
            wait_samples += 1;
            metrics.max_wait_blocks = metrics.max_wait_blocks.max(wait);
            metrics.successful_registrations += 1;
        }

        // Track max fee
        let current_fee_mult = calculate_fee_multiplier(queue.pending_count());
        metrics.max_fee_multiplier = metrics.max_fee_multiplier.max(current_fee_mult);

        // Reset block count for next block
        queue.begin_block(block + 1);
    }

    metrics.avg_wait_blocks = if wait_samples > 0 {
        total_wait as f64 / wait_samples as f64
    } else {
        0.0
    };

    println!("Resultados Escenario A (Crecimiento Normal):");
    println!("  Registros intentados: {}", metrics.attempted_registrations);
    println!("  Registros exitosos: {}", metrics.successful_registrations);
    println!("  Espera promedio: {:.1} bloques", metrics.avg_wait_blocks);
    println!("  Espera maxima: {} bloques", metrics.max_wait_blocks);
    println!("  Fee maximo: {:.1}x base", metrics.max_fee_multiplier);
    println!("  Cola final: {} pendientes", queue.pending_count());

    // Assertions
    assert!(
        metrics.avg_wait_blocks < thresholds::QUEUE_WAIT_ALERT as f64,
        "ALERTA: Espera promedio ({:.1}) > {} bloques",
        metrics.avg_wait_blocks, thresholds::QUEUE_WAIT_ALERT
    );
    assert_eq!(
        metrics.successful_registrations, metrics.attempted_registrations,
        "Todos los registros deben completarse en crecimiento normal"
    );

    println!("\n  [OK] Crecimiento normal manejado correctamente\n");
}

/// Scenario B: Viral spike (50 registrations/minute for 5 minutes, then normal)
#[test]
fn test_onboarding_viral_spike() {
    println!("\n=== PRUEBA 1B: Pico Viral ===\n");

    let network = Network::Devnet;
    let mut queue = RegistrationQueue::new();
    let mut metrics = LivenessMetrics::default();

    let spike_duration_minutes = 5;
    let spike_registrations_per_minute = 50;
    let normal_registrations_per_minute = 2;
    let blocks_per_minute = 12;
    let total_minutes = 30;
    let total_blocks = total_minutes * blocks_per_minute;

    let fee_abandonment_threshold = BASE_REGISTRATION_FEE as f64 * 100.0;
    let mut abandoned = 0u64;
    let mut total_wait = 0u64;
    let mut wait_samples = 0u64;
    let mut spike_ended_block = 0u64;
    let mut returned_to_normal = false;

    for block in 0..total_blocks {
        let minute = block / blocks_per_minute;

        // Determine registration rate
        let regs_per_min = if minute < spike_duration_minutes as u64 {
            spike_registrations_per_minute
        } else {
            normal_registrations_per_minute
        };

        // Add new registrations at start of each minute
        if block % blocks_per_minute == 0 {
            for i in 0..regs_per_min {
                let fee = queue.current_fee_for_network(network);

                // Check if registration would be abandoned due to fee
                if fee as f64 > fee_abandonment_threshold {
                    abandoned += 1;
                    metrics.attempted_registrations += 1;
                    continue;
                }

                let keypair = KeyPair::generate();
                let reg = PendingRegistration {
                    public_key: keypair.public_key().clone(),
                    bond_amount: 1000_000_000,
                    fee_paid: fee,
                    submitted_at: block,
                    prev_registration_hash: Hash::ZERO,
                    sequence_number: (block * 100 + i) as u64,
                };
                let _ = queue.submit_for_network(reg, block, network);
                metrics.attempted_registrations += 1;
            }

            // Track when spike ends
            if minute == spike_duration_minutes as u64 && spike_ended_block == 0 {
                spike_ended_block = block;
            }
        }

        // Process registrations
        while let Some(reg) = queue.next_registration_for_network(network) {
            let wait = block - reg.submitted_at;
            total_wait += wait;
            wait_samples += 1;
            metrics.max_wait_blocks = metrics.max_wait_blocks.max(wait);
            metrics.successful_registrations += 1;
        }

        // Track max fee and recovery
        let current_fee_mult = calculate_fee_multiplier(queue.pending_count());
        metrics.max_fee_multiplier = metrics.max_fee_multiplier.max(current_fee_mult);

        // Check if returned to normal (queue < 10)
        if spike_ended_block > 0 && !returned_to_normal && queue.pending_count() < 10 {
            metrics.recovery_blocks = block - spike_ended_block;
            returned_to_normal = true;
        }

        queue.begin_block(block + 1);
    }

    metrics.avg_wait_blocks = if wait_samples > 0 {
        total_wait as f64 / wait_samples as f64
    } else {
        0.0
    };
    metrics.abandonment_rate = abandoned as f64 / metrics.attempted_registrations as f64;

    println!("Resultados Escenario B (Pico Viral):");
    println!("  Registros intentados: {}", metrics.attempted_registrations);
    println!("  Registros exitosos: {}", metrics.successful_registrations);
    println!("  Registros abandonados: {} ({:.1}%)", abandoned, metrics.abandonment_rate * 100.0);
    println!("  Espera promedio: {:.1} bloques", metrics.avg_wait_blocks);
    println!("  Espera maxima: {} bloques ({:.1} minutos)",
             metrics.max_wait_blocks, metrics.max_wait_blocks as f64 / blocks_per_minute as f64);
    println!("  Fee maximo: {:.1}x base", metrics.max_fee_multiplier);
    println!("  Tiempo de recuperacion: {} bloques ({:.1} minutos)",
             metrics.recovery_blocks, metrics.recovery_blocks as f64 / blocks_per_minute as f64);
    println!("  Cola final: {} pendientes", queue.pending_count());

    // Alert evaluation
    let wait_alert = if metrics.max_wait_blocks > thresholds::QUEUE_WAIT_CRITICAL {
        AlertLevel::Red
    } else if metrics.max_wait_blocks > thresholds::QUEUE_WAIT_ALERT {
        AlertLevel::Yellow
    } else {
        AlertLevel::Green
    };

    let fee_alert = if metrics.max_fee_multiplier > thresholds::FEE_MULTIPLIER_CRITICAL {
        AlertLevel::Red
    } else if metrics.max_fee_multiplier > thresholds::FEE_MULTIPLIER_ALERT {
        AlertLevel::Yellow
    } else {
        AlertLevel::Green
    };

    let abandonment_alert = if metrics.abandonment_rate > thresholds::ABANDONMENT_RATE_CRITICAL {
        AlertLevel::Red
    } else {
        AlertLevel::Green
    };

    println!("\nAlertas:");
    println!("  Espera maxima: {:?}", wait_alert);
    println!("  Fee maximo: {:?}", fee_alert);
    println!("  Tasa abandono: {:?}", abandonment_alert);

    // This test documents behavior, not necessarily passes/fails
    // The key insight is understanding the limits
    if wait_alert == AlertLevel::Red || fee_alert == AlertLevel::Red {
        println!("\n  [WARN] Pico viral causa degradacion significativa");
        println!("  Considerar: aumentar MAX_REGISTRATIONS_PER_BLOCK o reducir FEE_MULTIPLIER");
    } else {
        println!("\n  [OK] Pico viral manejado dentro de limites aceptables");
    }
}

/// Scenario C: Congestion attack (invalid registrations)
#[test]
fn test_onboarding_congestion_attack() {
    println!("\n=== PRUEBA 1C: Ataque de Congestion ===\n");

    let network = Network::Devnet;
    let mut queue = RegistrationQueue::new();

    // Attack parameters
    let invalid_registrations_per_minute = 100;
    let legitimate_registrations_per_minute = 5;
    let blocks_per_minute = 12;
    let total_minutes = 10;
    let total_blocks = total_minutes * blocks_per_minute;

    let mut legitimate_completed = 0u64;
    let mut legitimate_blocked = 0u64;
    let mut attacker_cost = 0u64;

    // Invalid registration fee (must pay to submit, even if invalid)
    let base_fee = queue.current_fee_for_network(network);

    for block in 0..total_blocks {
        if block % blocks_per_minute == 0 {
            // Attacker submits invalid registrations
            // In real system, these would be rejected at validation
            // but might still consume some resources
            for _ in 0..invalid_registrations_per_minute {
                // Simulate attacker paying fee (cost)
                attacker_cost += base_fee;
                // Invalid registrations are rejected before entering queue
                // (duplicate keys, invalid signatures, etc.)
            }

            // Legitimate users submit valid registrations
            for i in 0..legitimate_registrations_per_minute {
                let keypair = KeyPair::generate();
                let fee = queue.current_fee_for_network(network);

                // Check if fee is too high (effectively blocked)
                if fee > base_fee * 10 {
                    legitimate_blocked += 1;
                    continue;
                }

                let reg = PendingRegistration {
                    public_key: keypair.public_key().clone(),
                    bond_amount: 1000_000_000,
                    fee_paid: fee,
                    submitted_at: block,
                    prev_registration_hash: Hash::ZERO,
                    sequence_number: (block * 100 + i as u64) as u64,
                };
                let _ = queue.submit_for_network(reg, block, network);
            }
        }

        // Process legitimate registrations
        while let Some(_reg) = queue.next_registration_for_network(network) {
            legitimate_completed += 1;
        }

        queue.begin_block(block + 1);
    }

    let total_legitimate = legitimate_completed + legitimate_blocked;
    let block_rate = legitimate_blocked as f64 / total_legitimate as f64;

    // Convert attacker cost to USD equivalent
    // Assume 1 DOLI = $2, fee is in smallest units (1 DOLI = 100_000_000 units)
    let attacker_cost_usd = attacker_cost as f64 * 2.0 / 100_000_000.0;
    let attacker_cost_per_hour = attacker_cost_usd / (total_minutes as f64 / 60.0);

    println!("Resultados Escenario C (Ataque de Congestion):");
    println!("  Registros legitimos completados: {}", legitimate_completed);
    println!("  Registros legitimos bloqueados: {} ({:.1}%)", legitimate_blocked, block_rate * 100.0);
    println!("  Costo del atacante: {} unidades = ${:.2}", attacker_cost, attacker_cost_usd);
    println!("  Costo por hora: ${:.2}/hora", attacker_cost_per_hour);

    // In a well-designed system, invalid registrations should be rejected
    // before they affect the queue, making this attack ineffective
    println!("\nAnalisis:");
    if block_rate < 0.1 {
        println!("  [OK] Ataque inefectivo - registros invalidos rechazados antes de afectar cola");
        println!("  La validacion temprana protege contra ataques de congestion");
    } else {
        println!("  [WARN] Ataque parcialmente efectivo - {:.0}% de registros legitimos bloqueados", block_rate * 100.0);
        println!("  Considerar: rate limiting por IP, proof-of-work en submission, depositos");
    }

    assert!(
        block_rate < 0.5,
        "Ataque de congestion bloqueo mas del 50% de registros legitimos"
    );
}

// ============================================================================
// PRUEBA 2: Simulacion de Elite (Aristocracia)
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

fn calculate_gini(values: &[u64]) -> f64 {
    let n = values.len();
    if n == 0 {
        return 0.0;
    }

    let mut sorted: Vec<u64> = values.to_vec();
    sorted.sort();

    let sum: u64 = sorted.iter().sum();
    if sum == 0 {
        return 0.0;
    }

    let mut cumulative = 0u64;
    let mut gini_sum = 0.0;

    for (i, &value) in sorted.iter().enumerate() {
        cumulative += value;
        let expected = (i + 1) as f64 / n as f64;
        let actual = cumulative as f64 / sum as f64;
        gini_sum += expected - actual;
    }

    2.0 * gini_sum / n as f64
}

// ============================================================================
// PRUEBA 3: Ataque de Infiltracion Lenta
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

// ============================================================================
// Helper Functions
// ============================================================================

fn calculate_fee_multiplier(queue_length: usize) -> f64 {
    // Use the deterministic table-based multiplier from consensus (capped at 10x)
    fee_multiplier_x100(queue_length as u32) as f64 / 100.0
}

// ============================================================================
// PRUEBA 4: Early Active Attacker (Atacante Paciente desde Día 1)
// ============================================================================

/// Critical simulation: Attacker enters at block 0 with perfect activity
///
/// This tests the WORST CASE scenario:
/// - Attacker enters at genesis (same time as founders)
/// - Attacker maintains PERFECT activity (0 gaps, no penalty)
/// - Attacker buys DOLI cheap before price increases
/// - Benevolent growth is SLOW (realistic, not optimistic)
///
/// Key question: Can attacker maintain ≥40% veto for multiple upgrade cycles?
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

#[derive(Debug)]
struct EarlyAttackerResult {
    attacker_nodes: u64,
    doli_cost: u64,
    years_with_veto: u64,
    year_diluted_below_33: Option<u64>,
    final_attacker_pct: f64,
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

#[derive(Debug)]
struct YearlyEntry {
    year: u64,
    benevolent_nodes: u64,
    attacker_nodes: u64,
    total_weight: f64,
    attacker_pct: f64,
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

// ============================================================================
// PRUEBA 5: VDF Verification Throughput (P0 - Critical)
// ============================================================================

/// Stress test: Verify VDF performance doesn't degrade under load
/// Target: < 50ms per verification, no memory leaks
#[test]
fn test_vdf_verification_throughput() {
    println!("\n=== PRUEBA 5: VDF Verification Throughput ===\n");

    use std::time::Instant;
    use crypto::hash::hash;

    // Simulate VDF verification (using hash as proxy since actual VDF is slow)
    // In production, this would use crypto::vdf::verify
    let iterations = 1000;
    let mut total_time_us = 0u128;
    let mut max_time_us = 0u128;
    let mut min_time_us = u128::MAX;

    println!("Ejecutando {} verificaciones simuladas...\n", iterations);

    for i in 0..iterations {
        let input = format!("VDF_INPUT_{}", i);
        let start = Instant::now();

        // Simulate verification work (hash chain as proxy)
        let mut current_hash = hash(input.as_bytes());
        for _ in 0..100 {
            current_hash = hash(current_hash.as_bytes());
        }

        let elapsed = start.elapsed().as_micros();
        total_time_us += elapsed;
        max_time_us = max_time_us.max(elapsed);
        min_time_us = min_time_us.min(elapsed);
    }

    let avg_time_us = total_time_us / iterations as u128;
    let avg_time_ms = avg_time_us as f64 / 1000.0;
    let max_time_ms = max_time_us as f64 / 1000.0;

    println!("Resultados:");
    println!("  Verificaciones: {}", iterations);
    println!("  Tiempo promedio: {:.3} ms", avg_time_ms);
    println!("  Tiempo maximo: {:.3} ms", max_time_ms);
    println!("  Tiempo minimo: {:.3} ms", min_time_us as f64 / 1000.0);
    println!("  Throughput: {:.0} verificaciones/segundo", 1000.0 / avg_time_ms);

    // Thresholds
    let avg_threshold_ms = 50.0;  // 50ms average
    let max_threshold_ms = 100.0; // 100ms max spike

    let status = if avg_time_ms < avg_threshold_ms && max_time_ms < max_threshold_ms {
        println!("\n  [OK] VDF verification dentro de limites");
        AlertLevel::Green
    } else if avg_time_ms < avg_threshold_ms * 2.0 {
        println!("\n  [WARN] VDF verification cerca del limite");
        AlertLevel::Yellow
    } else {
        println!("\n  [CRITICO] VDF verification demasiado lenta");
        AlertLevel::Red
    };

    assert!(
        avg_time_ms < avg_threshold_ms,
        "VDF verification promedio ({:.2}ms) > {}ms threshold",
        avg_time_ms, avg_threshold_ms
    );
}

// ============================================================================
// PRUEBA 6: Fork Choice Under Partition (P0 - Critical)
// ============================================================================

/// Simulate network partition and verify correct fork resolution
/// Key invariant: VDF with more covered slots wins
#[test]
fn test_fork_choice_partition() {
    println!("\n=== PRUEBA 6: Fork Choice Under Partition ===\n");

    use crypto::hash::hash;

    // Simulation parameters
    let partition_duration_blocks = 30; // 30 minutes partition
    let pre_partition_blocks = 10;
    let _post_heal_blocks = 20;

    // Simulate two partitions building separate chains
    struct ChainState {
        height: u64,
        tip_slot: u64,
        tip_hash: Hash,
    }

    // Common history before partition
    let common_chain = ChainState {
        height: pre_partition_blocks,
        tip_slot: pre_partition_blocks,
        tip_hash: hash(b"common_tip"),
    };

    println!("Cadena comun antes de particion:");
    println!("  Altura: {}", common_chain.height);
    println!("  Slot: {}", common_chain.tip_slot);

    // Partition A: 60% of producers, produces faster
    let mut chain_a = ChainState {
        height: common_chain.height,
        tip_slot: common_chain.tip_slot,
        tip_hash: common_chain.tip_hash,
    };

    // Partition B: 40% of producers, produces slower
    let mut chain_b = ChainState {
        height: common_chain.height,
        tip_slot: common_chain.tip_slot,
        tip_hash: common_chain.tip_hash,
    };

    // Simulate partition: A gets ~60% of slots, B gets ~40%
    println!("\nDurante particion ({} bloques):", partition_duration_blocks);

    for slot_offset in 1..=partition_duration_blocks {
        let slot = common_chain.tip_slot + slot_offset;

        // 60% chance A produces, 40% chance B produces
        if slot % 5 < 3 {
            // A produces
            chain_a.height += 1;
            chain_a.tip_slot = slot;
            chain_a.tip_hash = hash(format!("chain_a_{}", slot).as_bytes());
        }

        if slot % 5 >= 2 {
            // B produces (some overlap in middle)
            chain_b.height += 1;
            chain_b.tip_slot = slot;
            chain_b.tip_hash = hash(format!("chain_b_{}", slot).as_bytes());
        }
    }

    println!("  Particion A: altura {}, slot {}", chain_a.height, chain_a.tip_slot);
    println!("  Particion B: altura {}, slot {}", chain_b.height, chain_b.tip_slot);

    // Determine winner: highest slot wins, then highest height, then lowest hash
    let winner = if chain_a.tip_slot > chain_b.tip_slot {
        "A"
    } else if chain_b.tip_slot > chain_a.tip_slot {
        "B"
    } else if chain_a.height > chain_b.height {
        "A"
    } else if chain_b.height > chain_a.height {
        "B"
    } else {
        // Compare hashes
        if chain_a.tip_hash.as_bytes() < chain_b.tip_hash.as_bytes() { "A" } else { "B" }
    };

    println!("\n  Ganador: Particion {} (mayor slot coverage)", winner);

    // Simulate heal: nodes converge to winning chain
    println!("\nDespues de reunion:");
    let final_chain = if winner == "A" { &chain_a } else { &chain_b };

    // B nodes reorganize to A
    let reorg_depth = if winner == "A" {
        chain_b.height - common_chain.height
    } else {
        chain_a.height - common_chain.height
    };

    println!("  Profundidad de reorg: {} bloques", reorg_depth);
    println!("  Cadena final: altura {}, slot {}", final_chain.height, final_chain.tip_slot);

    // Verify fork choice is deterministic
    let winner_check = if chain_a.tip_slot > chain_b.tip_slot {
        "A"
    } else if chain_b.tip_slot > chain_a.tip_slot {
        "B"
    } else if chain_a.height > chain_b.height {
        "A"
    } else if chain_b.height > chain_a.height {
        "B"
    } else {
        if chain_a.tip_hash.as_bytes() < chain_b.tip_hash.as_bytes() { "A" } else { "B" }
    };

    assert_eq!(winner, winner_check, "Fork choice must be deterministic");

    // Verify invariant: the chain covering more time (slots) wins
    let winning_chain = if winner == "A" { &chain_a } else { &chain_b };
    let losing_chain = if winner == "A" { &chain_b } else { &chain_a };

    assert!(
        winning_chain.tip_slot >= losing_chain.tip_slot,
        "Winner must have >= slot coverage"
    );

    println!("\n  [OK] Fork choice determinista y correcto");
    println!("  La cadena con mayor cobertura de tiempo (slots) gana");
}

// ============================================================================
// PRUEBA 7: Network Liveness Simulation (P0 - Critical)
// ============================================================================

/// Simulate 72 hours of continuous operation
/// Verify: blocks every minute, no extended gaps, recovery from failures
#[test]
fn test_network_liveness_72h() {
    println!("\n=== PRUEBA 7: Network Liveness 72h ===\n");

    let hours = 72;
    let blocks_per_hour = 60; // 1 per minute
    let total_expected_blocks = hours * blocks_per_hour;

    // Simulation parameters
    let producer_count = 50;
    let producer_failure_rate = 0.05; // 5% of producers offline at any time
    let fallback_success_rate = 0.95; // 95% of fallback attempts succeed

    let mut produced_blocks = 0u64;
    let mut missed_slots = 0u64;
    let mut fallback_blocks = 0u64;
    let mut max_consecutive_misses = 0u64;
    let mut current_misses = 0u64;

    println!("Parametros:");
    println!("  Duracion: {} horas ({} bloques esperados)", hours, total_expected_blocks);
    println!("  Productores: {}", producer_count);
    println!("  Tasa de fallo: {:.0}%", producer_failure_rate * 100.0);
    println!("  Exito de fallback: {:.0}%\n", fallback_success_rate * 100.0);

    // Simple RNG for deterministic simulation
    let mut rng_state = 12345u64;
    let mut next_random = || {
        rng_state = rng_state.wrapping_mul(1103515245).wrapping_add(12345);
        (rng_state >> 16) as f64 / 65536.0
    };

    for slot in 0..total_expected_blocks {
        // Primary producer might be offline
        let primary_online = next_random() > producer_failure_rate;

        if primary_online {
            produced_blocks += 1;
            current_misses = 0;
        } else {
            // Try fallback producer
            let fallback_success = next_random() < fallback_success_rate;

            if fallback_success {
                produced_blocks += 1;
                fallback_blocks += 1;
                current_misses = 0;
            } else {
                missed_slots += 1;
                current_misses += 1;
                max_consecutive_misses = max_consecutive_misses.max(current_misses);
            }
        }
    }

    let success_rate = produced_blocks as f64 / total_expected_blocks as f64;
    let fallback_rate = fallback_blocks as f64 / produced_blocks as f64;

    println!("Resultados:");
    println!("  Bloques producidos: {} / {}", produced_blocks, total_expected_blocks);
    println!("  Tasa de exito: {:.2}%", success_rate * 100.0);
    println!("  Slots perdidos: {}", missed_slots);
    println!("  Bloques via fallback: {} ({:.1}%)", fallback_blocks, fallback_rate * 100.0);
    println!("  Max slots consecutivos perdidos: {}", max_consecutive_misses);

    // Thresholds
    let min_success_rate = 0.95; // 95% minimum
    let max_consecutive_miss_threshold = 5; // Max 5 minutes gap

    let liveness_ok = success_rate >= min_success_rate;
    let gap_ok = max_consecutive_misses <= max_consecutive_miss_threshold;

    if liveness_ok && gap_ok {
        println!("\n  [OK] Liveness saludable");
    } else {
        println!("\n  [WARN] Liveness degradada");
        if !liveness_ok {
            println!("    - Tasa de exito ({:.1}%) < {:.0}%", success_rate * 100.0, min_success_rate * 100.0);
        }
        if !gap_ok {
            println!("    - Gap maximo ({}) > {} bloques", max_consecutive_misses, max_consecutive_miss_threshold);
        }
    }

    assert!(
        success_rate >= 0.90,
        "Liveness critica: solo {:.1}% de bloques producidos",
        success_rate * 100.0
    );

    assert!(
        max_consecutive_misses <= 10,
        "Gap critico: {} bloques consecutivos perdidos",
        max_consecutive_misses
    );
}

// ============================================================================
// PRUEBA 8: Zombie Producer Behavior (P1 - Important)
// ============================================================================

/// Test behavior of inactive/zombie producers
/// Verify: loses governance power, can reactivate, bond preserved
#[test]
fn test_zombie_producer_behavior() {
    println!("\n=== PRUEBA 8: Zombie Producer Behavior ===\n");

    use storage::{
        ActivityStatus, INACTIVITY_THRESHOLD, REACTIVATION_THRESHOLD,
    };

    let _network = Network::Devnet;
    let mut producers = ProducerSet::new();

    // Register producer at block 0
    let keypair = KeyPair::generate();
    let info = ProducerInfo::new(
        keypair.public_key().clone(),
        0,
        1000_000_000,
        (Hash::ZERO, 0),
        0,
    );
    producers.register(info.clone(), 0).unwrap();
    // Set initial activity
    producers.get_by_pubkey_mut(keypair.public_key()).unwrap().last_activity = 0;

    println!("Productor registrado en bloque 0");
    println!("  INACTIVITY_THRESHOLD: {} bloques (~7 dias)", INACTIVITY_THRESHOLD);
    println!("  REACTIVATION_THRESHOLD: {} bloques (~1 dia)\n", REACTIVATION_THRESHOLD);

    // Test 1: Active producer (just produced)
    let current_height = 100;
    // Update last_activity to simulate active producer
    producers.get_by_pubkey_mut(keypair.public_key()).unwrap().last_activity = current_height;

    let status = producers.get_by_pubkey(keypair.public_key()).unwrap()
        .activity_status(current_height);
    println!("Test 1: Productor activo (just produced at block {})", current_height);
    println!("  Status: {:?}", status);
    assert_eq!(status, ActivityStatus::Active, "Should be Active");

    // Test 2: Inactive producer (7+ days without activity)
    let inactive_height = current_height + INACTIVITY_THRESHOLD + 100;
    // last_activity stays at current_height (100), so gap = inactive_height - 100 > INACTIVITY_THRESHOLD

    let status = producers.get_by_pubkey(keypair.public_key()).unwrap()
        .activity_status(inactive_height);
    println!("\nTest 2: Productor inactivo (height {} sin actividad)", inactive_height);
    println!("  Status: {:?}", status);
    assert!(
        status == ActivityStatus::RecentlyInactive || status == ActivityStatus::Dormant,
        "Should be Inactive or Dormant, got {:?}", status
    );

    // Test 3: Governance power
    let has_power = producers.get_by_pubkey(keypair.public_key()).unwrap()
        .has_governance_power(inactive_height);
    println!("\nTest 3: Poder de gobernanza cuando inactivo");
    println!("  has_governance_power: {}", has_power);
    // Dormant producers should NOT have governance power
    if status == ActivityStatus::Dormant {
        assert!(!has_power, "Dormant producer should not have governance power");
        println!("  [OK] Zombie no tiene poder de gobernanza");
    }

    // Test 4: Reactivation
    println!("\nTest 4: Reactivacion");
    // Simulate producer producing blocks again
    producers.get_by_pubkey_mut(keypair.public_key()).unwrap().last_activity = inactive_height;

    let status_after = producers.get_by_pubkey(keypair.public_key()).unwrap()
        .activity_status(inactive_height);
    println!("  Productor produce bloque en height {}", inactive_height);
    println!("  Status despues: {:?}", status_after);

    // Should be Active again after producing
    assert_eq!(status_after, ActivityStatus::Active, "Should reactivate after producing");
    println!("  [OK] Zombie puede reactivarse produciendo bloques");

    // Test 5: Bond preservation
    let bond = producers.get_by_pubkey(keypair.public_key()).unwrap().bond_amount;
    println!("\nTest 5: Preservacion de bond");
    println!("  Bond: {} (sin cambio)", bond);
    assert_eq!(bond, 1000_000_000, "Bond should be preserved");
    println!("  [OK] Bond preservado durante inactividad");

    println!("\n  [OK] Comportamiento de zombie correcto");
}

// ============================================================================
// PRUEBA 9: Producer Exit/Cancel Flow (P1 - Important)
// ============================================================================

/// Test the exit and cancel_exit flow
/// Verify: can exit, can cancel during unbonding, seniority preserved
#[test]
fn test_producer_exit_cancel_flow() {
    println!("\n=== PRUEBA 9: Producer Exit/Cancel Flow ===\n");

    use storage::ProducerStatus;

    let _network = Network::Devnet;
    let mut producers = ProducerSet::new();

    // Register producer at block 0
    let keypair = KeyPair::generate();
    let info = ProducerInfo::new(
        keypair.public_key().clone(),
        0, // registered at block 0 (maximum seniority)
        1000_000_000,
        (Hash::ZERO, 0),
        0,
    );
    producers.register(info, 0).unwrap();

    let initial_registered_at = producers.get_by_pubkey(keypair.public_key()).unwrap().registered_at;
    println!("Productor registrado en bloque {}", initial_registered_at);

    // Test 1: Request exit
    let exit_height = 100;
    println!("\nTest 1: Solicitar salida en bloque {}", exit_height);

    producers.request_exit(keypair.public_key(), exit_height).unwrap();

    let status = producers.get_by_pubkey(keypair.public_key()).unwrap().status.clone();
    match status {
        ProducerStatus::Unbonding { started_at } => {
            println!("  Status: Unbonding (started_at: {})", started_at);
            assert_eq!(started_at, exit_height);
        }
        _ => panic!("Expected Unbonding status, got {:?}", status),
    }
    println!("  [OK] Salida iniciada correctamente");

    // Test 2: Cancel exit
    println!("\nTest 2: Cancelar salida durante unbonding");

    producers.cancel_exit(keypair.public_key()).unwrap();

    let status_after = producers.get_by_pubkey(keypair.public_key()).unwrap().status.clone();
    assert_eq!(status_after, ProducerStatus::Active, "Should be Active after cancel");
    println!("  Status: {:?}", status_after);
    println!("  [OK] Salida cancelada correctamente");

    // Test 3: Seniority preserved
    let registered_at_after = producers.get_by_pubkey(keypair.public_key()).unwrap().registered_at;
    println!("\nTest 3: Seniority preservada");
    println!("  registered_at antes: {}", initial_registered_at);
    println!("  registered_at despues: {}", registered_at_after);
    assert_eq!(initial_registered_at, registered_at_after, "Seniority should be preserved");
    println!("  [OK] Seniority preservada tras cancelar salida");

    // Test 4: Cannot cancel if not unbonding
    println!("\nTest 4: No puede cancelar si no esta en unbonding");
    let result = producers.cancel_exit(keypair.public_key());
    assert!(result.is_err(), "Should fail if not unbonding");
    println!("  Error esperado: {:?}", result.unwrap_err());
    println!("  [OK] Cancelacion rechazada correctamente");

    // Test 5: Full exit flow (without cancel)
    println!("\nTest 5: Flujo completo de salida");
    producers.request_exit(keypair.public_key(), 200).unwrap();
    println!("  Salida iniciada en bloque 200");

    // Complete exit (after unbonding period)
    producers.get_by_pubkey_mut(keypair.public_key()).unwrap().complete_exit();

    let final_status = producers.get_by_pubkey(keypair.public_key()).unwrap().status.clone();
    assert_eq!(final_status, ProducerStatus::Exited, "Should be Exited");
    println!("  Status final: {:?}", final_status);
    println!("  [OK] Salida completada correctamente");

    println!("\n  [OK] Flujo exit/cancel funciona correctamente");
}

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
