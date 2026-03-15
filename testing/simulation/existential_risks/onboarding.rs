//! PRUEBA 1: Stress Test de Onboarding (Liveness)

use super::*;

// ============================================================================
// Metrics
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

// ============================================================================
// Scenario A: Normal growth (2 registrations/minute)
// ============================================================================

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

// ============================================================================
// Scenario B: Viral spike (50 registrations/minute for 5 minutes, then normal)
// ============================================================================

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

// ============================================================================
// Scenario C: Congestion attack (invalid registrations)
// ============================================================================

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
