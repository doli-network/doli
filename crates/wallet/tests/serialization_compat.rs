//! Cross-crate serialization compatibility tests.
//!
//! These tests verify that transactions built by the wallet crate's TxBuilder
//! produce byte-identical output to `bincode::serialize()` on doli-core's
//! Transaction struct. This is critical: if the wire format doesn't match,
//! nodes will reject every transaction the GUI sends.
//!
//! NOTE: doli-core is added as a DEV-dependency only. The runtime wallet crate
//! does NOT depend on doli-core (architectural constraint: no VDF/GMP).

use crypto::{hash::hash, signature, KeyPair};
use doli_core::{Input, Output, OutputType, SighashType, Transaction, TxType};
use wallet::Utxo;

/// Build a Transfer tx via TxBuilder, then build the identical tx via doli-core,
/// serialize both, and compare byte-for-byte.
#[test]
fn test_m1_transfer_serialization_matches_core() {
    // Deterministic keypair from fixed seed
    let seed = [42u8; 32];
    let kp = KeyPair::from_seed(seed);

    let tx_hash_bytes = [0xAAu8; 32];
    let recipient_hash = [0xBBu8; 32];
    let sender_hash = [0xCCu8; 32];
    let amount: u64 = 500_000_000; // 5 DOLI
    let change: u64 = 300_000_000; // 3 DOLI

    // --- Build via wallet TxBuilder ---
    let mut wallet_builder = wallet::TxBuilder::new(wallet::TxType::Transfer);
    wallet_builder.add_input(tx_hash_bytes, 0);
    wallet_builder.add_output(amount, recipient_hash, 0, 0, Vec::new());
    wallet_builder.add_output(change, sender_hash, 0, 0, Vec::new());
    let wallet_hex = wallet_builder.sign_and_build(&kp).unwrap();
    let wallet_bytes = hex::decode(&wallet_hex).unwrap();

    // --- Build via doli-core ---
    // First, compute the same signing message the wallet computes
    let signing_bytes = wallet_builder.build_for_signing().unwrap();
    let signing_hash = hash(&signing_bytes);
    let sig = signature::sign(signing_hash.as_bytes(), kp.private_key());

    let core_tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input {
            prev_tx_hash: crypto::Hash::from_bytes(tx_hash_bytes),
            output_index: 0,
            signature: sig,
            sighash_type: SighashType::All,
            committed_output_count: 0,
        }],
        outputs: vec![
            Output {
                output_type: OutputType::Normal,
                amount,
                pubkey_hash: crypto::Hash::from_bytes(recipient_hash),
                lock_until: 0,
                extra_data: Vec::new(),
            },
            Output {
                output_type: OutputType::Normal,
                amount: change,
                pubkey_hash: crypto::Hash::from_bytes(sender_hash),
                lock_until: 0,
                extra_data: Vec::new(),
            },
        ],
        extra_data: Vec::new(),
    };

    let core_bytes = bincode::serialize(&core_tx).unwrap();

    assert_eq!(
        wallet_bytes,
        core_bytes,
        "Wallet TxBuilder output must be byte-identical to bincode::serialize(Transaction).\n\
         wallet len={}, core len={}\n\
         wallet first 64 bytes: {:02x?}\n\
         core   first 64 bytes: {:02x?}",
        wallet_bytes.len(),
        core_bytes.len(),
        &wallet_bytes[..wallet_bytes.len().min(64)],
        &core_bytes[..core_bytes.len().min(64)],
    );
}

/// Build an AddBond tx via the convenience builder and compare with core.
#[test]
fn test_m1_add_bond_serialization_matches_core() {
    let seed = [99u8; 32];
    let kp = KeyPair::from_seed(seed);

    let utxo_hash = "dd".repeat(32); // 64 hex chars = 32 bytes
    let utxos = vec![Utxo {
        tx_hash: utxo_hash.clone(),
        output_index: 0,
        amount: 50_000_000_000, // 500 DOLI
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];

    let sender_hash = [0xEEu8; 32];
    let fee = 1000u64;
    let bond_count = 2u32;

    let mut wallet_builder =
        wallet::TxBuilder::build_add_bond(&utxos, bond_count, sender_hash, fee).unwrap();

    // Sign with wallet
    let wallet_hex = wallet_builder.sign_and_build(&kp).unwrap();
    let wallet_bytes = hex::decode(&wallet_hex).unwrap();

    // Compute the same signing message
    let signing_bytes = wallet_builder.build_for_signing().unwrap();
    let signing_hash = hash(&signing_bytes);
    let sig = signature::sign(signing_hash.as_bytes(), kp.private_key());

    let utxo_hash_bytes: [u8; 32] = hex::decode(&utxo_hash).unwrap().try_into().unwrap();

    // Expected: 2 bond outputs + 1 change output
    let bond_cost = bond_count as u64 * wallet::BOND_UNIT;
    let total_needed = bond_cost + fee;
    let change_amount = 50_000_000_000u64 - total_needed;

    let core_tx = Transaction {
        version: 1,
        tx_type: TxType::AddBond,
        inputs: vec![Input {
            prev_tx_hash: crypto::Hash::from_bytes(utxo_hash_bytes),
            output_index: 0,
            signature: sig,
            sighash_type: SighashType::All,
            committed_output_count: 0,
        }],
        outputs: {
            let mut outs = Vec::new();
            for _ in 0..bond_count {
                outs.push(Output {
                    output_type: OutputType::Bond,
                    amount: wallet::BOND_UNIT,
                    pubkey_hash: crypto::Hash::from_bytes(sender_hash),
                    lock_until: u64::MAX,
                    extra_data: Vec::new(),
                });
            }
            outs.push(Output {
                output_type: OutputType::Normal,
                amount: change_amount,
                pubkey_hash: crypto::Hash::from_bytes(sender_hash),
                lock_until: 0,
                extra_data: Vec::new(),
            });
            outs
        },
        extra_data: Vec::new(),
    };

    let core_bytes = bincode::serialize(&core_tx).unwrap();

    assert_eq!(
        wallet_bytes,
        core_bytes,
        "AddBond TxBuilder output must match core bincode.\n\
         wallet len={}, core len={}",
        wallet_bytes.len(),
        core_bytes.len(),
    );
}

/// Verify that Hash fields in the wire format include the u64 length prefix
/// that bincode 1.x writes for serialize_bytes().
#[test]
fn test_m1_hash_field_has_length_prefix() {
    let seed = [7u8; 32];
    let kp = KeyPair::from_seed(seed);

    let tx_hash = [0x11u8; 32];
    let recipient = [0x22u8; 32];

    let mut builder = wallet::TxBuilder::new(wallet::TxType::Transfer);
    builder.add_input(tx_hash, 0);
    builder.add_output(100, recipient, 0, 0, Vec::new());

    let hex_out = builder.sign_and_build(&kp).unwrap();
    let bytes = hex::decode(&hex_out).unwrap();

    // Layout: version(4) + tx_type(4) + vec_len(8) = 16 bytes before first input
    // First input starts at offset 16:
    //   Hash length prefix (8 bytes, u64 LE = 32)
    let hash_len_prefix = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
    assert_eq!(
        hash_len_prefix, 32,
        "Hash field must have u64 LE length prefix of 32"
    );

    // After hash (24+32=56), output_index (4 bytes) at offset 56
    // Then signature length prefix at offset 60
    let sig_len_prefix = u64::from_le_bytes(bytes[60..68].try_into().unwrap());
    assert_eq!(
        sig_len_prefix, 64,
        "Signature field must have u64 LE length prefix of 64"
    );
}

/// Verify the deserialization round-trip: wallet -> bytes -> core deserialize.
#[test]
fn test_m1_wallet_tx_deserializes_in_core() {
    let seed = [55u8; 32];
    let kp = KeyPair::from_seed(seed);

    let utxos = vec![Utxo {
        tx_hash: "ff".repeat(32),
        output_index: 2,
        amount: 1_000_000_000, // 10 DOLI
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 42,
        spendable: true,
    }];

    let recipient = [0x33u8; 32];
    let sender = [0x44u8; 32];

    let mut builder =
        wallet::TxBuilder::build_transfer(&utxos, recipient, 500_000_000, 1000, sender).unwrap();
    let hex_out = builder.sign_and_build(&kp).unwrap();
    let bytes = hex::decode(&hex_out).unwrap();

    // This is the critical test: can doli-core deserialize what the wallet produced?
    let deserialized: Transaction = bincode::deserialize(&bytes)
        .expect("doli-core must be able to deserialize wallet-built transactions");

    assert_eq!(deserialized.version, 1);
    assert_eq!(deserialized.tx_type, TxType::Transfer);
    assert_eq!(deserialized.inputs.len(), 1);
    assert_eq!(deserialized.outputs.len(), 2); // recipient + change

    // Verify amounts
    assert_eq!(deserialized.outputs[0].amount, 500_000_000);
    assert_eq!(
        deserialized.outputs[0].pubkey_hash,
        crypto::Hash::from_bytes(recipient)
    );
    assert_eq!(deserialized.outputs[0].output_type, OutputType::Normal);

    // Verify change
    let expected_change = 1_000_000_000 - 500_000_000 - 1000;
    assert_eq!(deserialized.outputs[1].amount, expected_change);
    assert_eq!(
        deserialized.outputs[1].pubkey_hash,
        crypto::Hash::from_bytes(sender)
    );
}
