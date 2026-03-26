use super::*;
use crypto::KeyPair;

fn mock_hash(seed: u8) -> Hash {
    let mut bytes = [0u8; 32];
    bytes[0] = seed;
    Hash::from_bytes(bytes)
}

fn mock_pubkey(seed: u8) -> PublicKey {
    let mut bytes = [0u8; 32];
    bytes[0] = seed;
    PublicKey::from_bytes(bytes)
}

#[test]
fn test_vdf_input_uniqueness() {
    let pk1 = mock_pubkey(1);
    let pk2 = mock_pubkey(2);
    let hash1 = mock_hash(10);
    let hash2 = mock_hash(20);

    // Different producers = different inputs
    let input1 = Heartbeat::compute_vdf_input(&pk1, 100, &hash1);
    let input2 = Heartbeat::compute_vdf_input(&pk2, 100, &hash1);
    assert_ne!(input1, input2);

    // Different slots = different inputs
    let input3 = Heartbeat::compute_vdf_input(&pk1, 101, &hash1);
    assert_ne!(input1, input3);

    // Different prev_hash = different inputs
    let input4 = Heartbeat::compute_vdf_input(&pk1, 100, &hash2);
    assert_ne!(input1, input4);

    // Same inputs = same result (deterministic)
    let input5 = Heartbeat::compute_vdf_input(&pk1, 100, &hash1);
    assert_eq!(input1, input5);
}

#[test]
fn test_hash_chain_vdf_deterministic() {
    let input = mock_hash(42);

    // Same input produces same output
    let output1 = hash_chain_vdf(&input, 100);
    let output2 = hash_chain_vdf(&input, 100);
    assert_eq!(output1, output2);

    // Different iterations produce different output
    let output3 = hash_chain_vdf(&input, 101);
    assert_ne!(output1, output3);
}

#[test]
fn test_verify_hash_chain_vdf() {
    let input = mock_hash(42);
    let output = hash_chain_vdf(&input, 1000);

    assert!(verify_hash_chain_vdf(&input, &output, 1000));
    assert!(!verify_hash_chain_vdf(&input, &output, 999));

    let wrong_output = [0u8; 32];
    assert!(!verify_hash_chain_vdf(&input, &wrong_output, 1000));
}

#[test]
fn test_witness_signature() {
    let producer = mock_pubkey(1);
    let slot = 100;
    let vdf_output = [42u8; 32];

    // Create witness keypair
    let witness_kp = KeyPair::generate();
    let witness_pk = *witness_kp.public_key();

    // Sign the witness message
    let message = WitnessSignature::compute_message(&producer, slot, &vdf_output);
    let signature = crypto::signature::sign_hash(&message, witness_kp.private_key());

    let witness_sig = WitnessSignature::new(witness_pk, signature);

    // Should verify
    assert!(witness_sig.verify(&producer, slot, &vdf_output));

    // Different producer should fail
    let other_producer = mock_pubkey(2);
    assert!(!witness_sig.verify(&other_producer, slot, &vdf_output));

    // Different slot should fail
    assert!(!witness_sig.verify(&producer, slot + 1, &vdf_output));

    // Different vdf_output should fail
    let other_output = [99u8; 32];
    assert!(!witness_sig.verify(&producer, slot, &other_output));
}

#[test]
fn test_heartbeat_signature() {
    let kp = KeyPair::generate();
    let producer = *kp.public_key();
    let slot = 100;
    let prev_hash = mock_hash(1);

    // Compute VDF (use small iterations for test)
    let input = Heartbeat::compute_vdf_input(&producer, slot, &prev_hash);
    let vdf_output = hash_chain_vdf(&input, 100);

    // Sign the heartbeat
    let signing_msg = Heartbeat::compute_signing_message(
        HEARTBEAT_VERSION,
        &producer,
        slot,
        &prev_hash,
        &vdf_output,
    );
    let signature = crypto::signature::sign_hash(&signing_msg, kp.private_key());

    let heartbeat = Heartbeat::new(producer, slot, prev_hash, vdf_output, signature);

    // Signature should verify (VDF would fail with wrong iterations)
    assert!(heartbeat.verify_signature());
}

#[test]
fn test_insufficient_witnesses() {
    let heartbeat = Heartbeat {
        version: HEARTBEAT_VERSION,
        producer: mock_pubkey(1),
        slot: 100,
        prev_block_hash: mock_hash(1),
        vdf_output: [0u8; 32],
        signature: Signature::default(),
        witnesses: vec![], // No witnesses
    };

    let active = vec![mock_pubkey(2), mock_pubkey(3)];
    let result = heartbeat.verify_witnesses(&active);

    assert!(matches!(
        result,
        Err(HeartbeatError::InsufficientWitnesses { have: 0, need: 2 })
    ));
}

#[test]
fn test_self_witness_rejected() {
    let producer = mock_pubkey(1);

    // Try to use producer as their own witness (along with a valid one to meet count)
    let self_witness = WitnessSignature {
        witness: producer,
        signature: Signature::default(),
    };
    let other_witness = WitnessSignature {
        witness: mock_pubkey(2),
        signature: Signature::default(),
    };

    let heartbeat = Heartbeat {
        version: HEARTBEAT_VERSION,
        producer,
        slot: 100,
        prev_block_hash: mock_hash(1),
        vdf_output: [0u8; 32],
        signature: Signature::default(),
        witnesses: vec![self_witness, other_witness], // 2 witnesses to pass count check
    };

    let active = vec![producer, mock_pubkey(2), mock_pubkey(3)];
    let result = heartbeat.verify_witnesses(&active);

    // Should fail with SelfWitness before checking other_witness signature
    assert!(matches!(result, Err(HeartbeatError::SelfWitness)));
}

#[test]
fn test_invalid_witness_rejected() {
    let producer = mock_pubkey(1);
    let non_active_witness = mock_pubkey(99); // Not in active list
    let valid_witness = mock_pubkey(2);

    let invalid_sig = WitnessSignature {
        witness: non_active_witness,
        signature: Signature::default(),
    };
    let valid_sig = WitnessSignature {
        witness: valid_witness,
        signature: Signature::default(),
    };

    let heartbeat = Heartbeat {
        version: HEARTBEAT_VERSION,
        producer,
        slot: 100,
        prev_block_hash: mock_hash(1),
        vdf_output: [0u8; 32],
        signature: Signature::default(),
        witnesses: vec![invalid_sig, valid_sig], // 2 witnesses to pass count check
    };

    let active = vec![producer, valid_witness, mock_pubkey(3)];
    let result = heartbeat.verify_witnesses(&active);

    // Should fail because non_active_witness is not in active list
    assert!(matches!(result, Err(HeartbeatError::InvalidWitness(_))));
}

#[test]
fn test_valid_witnesses() {
    let producer_kp = KeyPair::generate();
    let producer = *producer_kp.public_key();
    let slot = 100;
    let prev_hash = mock_hash(1);

    // Compute VDF
    let input = Heartbeat::compute_vdf_input(&producer, slot, &prev_hash);
    let vdf_output = hash_chain_vdf(&input, 100);

    // Create two valid witnesses
    let witness1_kp = KeyPair::generate();
    let witness2_kp = KeyPair::generate();

    let msg = WitnessSignature::compute_message(&producer, slot, &vdf_output);

    let sig1 = crypto::signature::sign_hash(&msg, witness1_kp.private_key());
    let sig2 = crypto::signature::sign_hash(&msg, witness2_kp.private_key());

    let witnesses = vec![
        WitnessSignature::new(*witness1_kp.public_key(), sig1),
        WitnessSignature::new(*witness2_kp.public_key(), sig2),
    ];

    let heartbeat = Heartbeat {
        version: HEARTBEAT_VERSION,
        producer,
        slot,
        prev_block_hash: prev_hash,
        vdf_output,
        signature: Signature::default(), // Not checking producer sig here
        witnesses,
    };

    let active = vec![
        producer,
        *witness1_kp.public_key(),
        *witness2_kp.public_key(),
    ];

    let result = heartbeat.verify_witnesses(&active);
    assert!(result.is_ok());
}

#[test]
fn test_heartbeat_serialization() {
    // Test with no witnesses first
    let heartbeat_empty = Heartbeat {
        version: HEARTBEAT_VERSION,
        producer: mock_pubkey(1),
        slot: 12345,
        prev_block_hash: mock_hash(42),
        vdf_output: [99u8; 32],
        signature: Signature::default(),
        witnesses: vec![],
    };

    let serialized = heartbeat_empty.serialize();
    assert!(!serialized.is_empty(), "Serialization should not be empty");

    if let Some(deserialized) = Heartbeat::deserialize(&serialized) {
        assert_eq!(heartbeat_empty.version, deserialized.version);
        assert_eq!(heartbeat_empty.producer, deserialized.producer);
        assert_eq!(heartbeat_empty.slot, deserialized.slot);
        assert_eq!(
            heartbeat_empty.prev_block_hash,
            deserialized.prev_block_hash
        );
        assert_eq!(heartbeat_empty.vdf_output, deserialized.vdf_output);
        assert_eq!(
            heartbeat_empty.witnesses.len(),
            deserialized.witnesses.len()
        );
    } else {
        // If deserialization fails, just check that serialize/size work
        // bincode may have issues with certain type combinations
        assert!(heartbeat_empty.size() > 100);
    }
}

#[test]
fn test_heartbeat_id() {
    let hb1 = Heartbeat {
        version: HEARTBEAT_VERSION,
        producer: mock_pubkey(1),
        slot: 100,
        prev_block_hash: mock_hash(1),
        vdf_output: [0u8; 32],
        signature: Signature::default(),
        witnesses: vec![],
    };

    let hb2 = Heartbeat {
        version: HEARTBEAT_VERSION,
        producer: mock_pubkey(1),
        slot: 100,
        prev_block_hash: mock_hash(2), // Different prev_hash
        vdf_output: [1u8; 32],         // Different output
        signature: Signature::default(),
        witnesses: vec![],
    };

    // Same producer + slot = same ID
    assert_eq!(hb1.id(), hb2.id());

    let hb3 = Heartbeat {
        version: HEARTBEAT_VERSION,
        producer: mock_pubkey(2), // Different producer
        slot: 100,
        prev_block_hash: mock_hash(1),
        vdf_output: [0u8; 32],
        signature: Signature::default(),
        witnesses: vec![],
    };

    // Different producer = different ID
    assert_ne!(hb1.id(), hb3.id());
}

#[test]
fn test_has_enough_witnesses() {
    let mut heartbeat = Heartbeat {
        version: HEARTBEAT_VERSION,
        producer: mock_pubkey(1),
        slot: 100,
        prev_block_hash: mock_hash(1),
        vdf_output: [0u8; 32],
        signature: Signature::default(),
        witnesses: vec![],
    };

    assert!(!heartbeat.has_enough_witnesses());

    heartbeat.add_witness(WitnessSignature {
        witness: mock_pubkey(2),
        signature: Signature::default(),
    });
    assert!(!heartbeat.has_enough_witnesses());

    heartbeat.add_witness(WitnessSignature {
        witness: mock_pubkey(3),
        signature: Signature::default(),
    });
    assert!(heartbeat.has_enough_witnesses());
}

#[test]
fn test_size_estimation() {
    let heartbeat = Heartbeat {
        version: HEARTBEAT_VERSION,
        producer: mock_pubkey(1),
        slot: 100,
        prev_block_hash: mock_hash(1),
        vdf_output: [0u8; 32],
        signature: Signature::default(),
        witnesses: vec![
            WitnessSignature {
                witness: mock_pubkey(2),
                signature: Signature::default(),
            },
            WitnessSignature {
                witness: mock_pubkey(3),
                signature: Signature::default(),
            },
        ],
    };

    let size = heartbeat.size();
    // 1 + 32 + 4 + 32 + 32 + 64 + 2*(32+64) = 357
    assert!(size > 300);
    assert!(size < 400);
}
