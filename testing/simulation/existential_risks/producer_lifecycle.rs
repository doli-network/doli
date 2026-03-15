//! PRUEBA 8: Zombie Producer Behavior
//! PRUEBA 9: Producer Exit/Cancel Flow

use super::*;

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
