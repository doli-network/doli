use crypto::{Hash, Signature};

use crate::types::Amount;

use super::*;

#[test]
fn test_coinbase() {
    let tx = Transaction::new_coinbase(500_000_000, Hash::ZERO, 0);

    assert!(tx.is_coinbase());
    assert_eq!(tx.inputs.len(), 0);
    assert_eq!(tx.outputs.len(), 1);
    assert_eq!(tx.outputs[0].amount, 500_000_000);
}

#[test]
fn test_tx_hash_deterministic() {
    let tx = Transaction::new_coinbase(500_000_000, Hash::ZERO, 100);

    let hash1 = tx.hash();
    let hash2 = tx.hash();

    assert_eq!(hash1, hash2);
}

#[test]
fn test_output_spendability() {
    let normal = Output::normal(100, Hash::ZERO);
    assert!(normal.is_spendable_at(0));
    assert!(normal.is_spendable_at(100));

    let bond = Output::bond(100, Hash::ZERO, 1000, 0);
    assert!(!bond.is_spendable_at(0));
    assert!(!bond.is_spendable_at(999));
    assert!(bond.is_spendable_at(1000));
    assert!(bond.is_spendable_at(1001));
}

#[test]
fn test_serialization_roundtrip() {
    let tx = Transaction::new_coinbase(500_000_000, Hash::ZERO, 42);
    let bytes = tx.serialize();
    let recovered = Transaction::deserialize(&bytes).unwrap();

    assert_eq!(tx, recovered);
}

#[test]
fn test_tx_type_conversion() {
    assert_eq!(TxType::from_u32(0), Some(TxType::Transfer));
    assert_eq!(TxType::from_u32(1), Some(TxType::Registration));
    assert_eq!(TxType::from_u32(2), Some(TxType::Exit));
    assert_eq!(TxType::from_u32(3), Some(TxType::ClaimReward));
    assert_eq!(TxType::from_u32(4), Some(TxType::ClaimBond));
    assert_eq!(TxType::from_u32(5), Some(TxType::SlashProducer));
    assert_eq!(TxType::from_u32(6), Some(TxType::Coinbase));
    assert_eq!(TxType::from_u32(7), Some(TxType::AddBond));
    assert_eq!(TxType::from_u32(8), Some(TxType::RequestWithdrawal));
    assert_eq!(TxType::from_u32(9), Some(TxType::ClaimWithdrawal));
    assert_eq!(TxType::from_u32(10), Some(TxType::EpochReward));
    assert_eq!(TxType::from_u32(11), Some(TxType::RemoveMaintainer));
    assert_eq!(TxType::from_u32(12), Some(TxType::AddMaintainer));
    assert_eq!(TxType::from_u32(13), Some(TxType::DelegateBond));
    assert_eq!(TxType::from_u32(14), Some(TxType::RevokeDelegation));
    assert_eq!(TxType::from_u32(15), Some(TxType::ProtocolActivation));
    assert_eq!(TxType::from_u32(16), None);
    assert_eq!(TxType::from_u32(17), Some(TxType::MintAsset));
    assert_eq!(TxType::from_u32(18), Some(TxType::BurnAsset));
    assert_eq!(TxType::from_u32(19), None);
    assert_eq!(TxType::from_u32(u32::MAX), None);
}

#[test]
fn test_output_type_conversion() {
    assert_eq!(OutputType::from_u8(0), Some(OutputType::Normal));
    assert_eq!(OutputType::from_u8(1), Some(OutputType::Bond));
    assert_eq!(OutputType::from_u8(2), Some(OutputType::Multisig));
    assert_eq!(OutputType::from_u8(3), Some(OutputType::Hashlock));
    assert_eq!(OutputType::from_u8(4), Some(OutputType::HTLC));
    assert_eq!(OutputType::from_u8(5), Some(OutputType::Vesting));
    assert_eq!(OutputType::from_u8(6), Some(OutputType::NFT));
    assert_eq!(OutputType::from_u8(7), Some(OutputType::FungibleAsset));
    assert_eq!(OutputType::from_u8(8), Some(OutputType::BridgeHTLC));
    assert_eq!(OutputType::from_u8(9), None);
    assert_eq!(OutputType::from_u8(u8::MAX), None);
}

#[test]
fn test_input_outpoint() {
    let hash = crypto::hash::hash(b"test");
    let input = Input::new(hash, 42);
    assert_eq!(input.outpoint(), (hash, 42));
}

#[test]
fn test_transfer_not_coinbase() {
    let hash = crypto::hash::hash(b"prev");
    let tx = Transaction::new_transfer(
        vec![Input::new(hash, 0)],
        vec![Output::normal(100, Hash::ZERO)],
    );
    assert!(!tx.is_coinbase());
}

#[test]
fn test_exit_transaction() {
    let keypair = crypto::KeyPair::generate();
    let pubkey = keypair.public_key();

    let tx = Transaction::new_exit(*pubkey);

    assert!(tx.is_exit());
    assert!(!tx.is_coinbase());
    assert!(!tx.is_registration());
    assert_eq!(tx.tx_type, TxType::Exit);
    assert!(tx.inputs.is_empty());
    assert!(tx.outputs.is_empty());

    // Verify exit data can be parsed
    let exit_data = tx.exit_data().unwrap();
    assert_eq!(exit_data.public_key, *pubkey);
}

#[test]
fn test_exit_data_serialization() {
    let keypair = crypto::KeyPair::generate();
    let pubkey = keypair.public_key();

    let tx = Transaction::new_exit(*pubkey);
    let bytes = tx.serialize();
    let recovered = Transaction::deserialize(&bytes).unwrap();

    assert_eq!(tx.tx_type, recovered.tx_type);
    let recovered_data = recovered.exit_data().unwrap();
    assert_eq!(recovered_data.public_key, *pubkey);
}

#[test]
fn test_claim_reward_transaction() {
    let keypair = crypto::KeyPair::generate();
    let pubkey = keypair.public_key();
    let recipient_hash = crypto::hash::hash(b"recipient");

    let tx = Transaction::new_claim_reward(*pubkey, 500_000_000, recipient_hash);

    assert!(tx.is_claim_reward());
    assert!(!tx.is_coinbase());
    assert!(!tx.is_exit());
    assert!(!tx.is_registration());
    assert_eq!(tx.tx_type, TxType::ClaimReward);
    assert!(tx.inputs.is_empty());
    assert_eq!(tx.outputs.len(), 1);
    assert_eq!(tx.outputs[0].amount, 500_000_000);

    // Verify claim data can be parsed
    let claim_data = tx.claim_data().unwrap();
    assert_eq!(claim_data.public_key, *pubkey);
}

#[test]
fn test_claim_data_serialization() {
    let keypair = crypto::KeyPair::generate();
    let pubkey = keypair.public_key();
    let recipient_hash = crypto::hash::hash(b"recipient");

    let tx = Transaction::new_claim_reward(*pubkey, 1_000_000_000, recipient_hash);
    let bytes = tx.serialize();
    let recovered = Transaction::deserialize(&bytes).unwrap();

    assert_eq!(tx.tx_type, recovered.tx_type);
    let recovered_data = recovered.claim_data().unwrap();
    assert_eq!(recovered_data.public_key, *pubkey);
}

#[test]
fn test_claim_bond_transaction() {
    let keypair = crypto::KeyPair::generate();
    let pubkey = keypair.public_key();
    let recipient_hash = crypto::hash::hash(b"recipient");

    let tx = Transaction::new_claim_bond(*pubkey, 100_000_000_000, recipient_hash);

    assert!(tx.is_claim_bond());
    assert!(!tx.is_coinbase());
    assert!(!tx.is_claim_reward());
    assert!(!tx.is_exit());
    assert_eq!(tx.tx_type, TxType::ClaimBond);
    assert!(tx.inputs.is_empty());
    assert_eq!(tx.outputs.len(), 1);
    assert_eq!(tx.outputs[0].amount, 100_000_000_000);

    // Verify claim bond data can be parsed
    let claim_bond_data = tx.claim_bond_data().unwrap();
    assert_eq!(claim_bond_data.public_key, *pubkey);
}

#[test]
fn test_claim_bond_serialization() {
    let keypair = crypto::KeyPair::generate();
    let pubkey = keypair.public_key();
    let recipient_hash = crypto::hash::hash(b"recipient");

    let tx = Transaction::new_claim_bond(*pubkey, 50_000_000_000, recipient_hash);
    let bytes = tx.serialize();
    let recovered = Transaction::deserialize(&bytes).unwrap();

    assert_eq!(tx.tx_type, recovered.tx_type);
    let recovered_data = recovered.claim_bond_data().unwrap();
    assert_eq!(recovered_data.public_key, *pubkey);
}

#[test]
fn test_slash_producer_transaction() {
    use crate::BlockHeader;
    use vdf::{VdfOutput, VdfProof};

    let producer_keypair = crypto::KeyPair::generate();

    // Create test block headers with same producer and slot but different content
    let header1 = BlockHeader {
        version: 1,
        prev_hash: Hash::ZERO,
        merkle_root: crypto::hash::hash(b"block1"),
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 0,
        slot: 12345,
        producer: *producer_keypair.public_key(),
        vdf_output: VdfOutput { value: vec![] },
        vdf_proof: VdfProof::empty(),
    };
    let header2 = BlockHeader {
        version: 1,
        prev_hash: Hash::ZERO,
        merkle_root: crypto::hash::hash(b"block2"),
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 0,
        slot: 12345,
        producer: *producer_keypair.public_key(),
        vdf_output: VdfOutput { value: vec![] },
        vdf_proof: VdfProof::empty(),
    };

    let evidence = SlashingEvidence::DoubleProduction {
        block_header_1: header1,
        block_header_2: header2,
    };

    let slash_data = SlashData {
        producer_pubkey: *producer_keypair.public_key(),
        evidence,
        reporter_signature: Signature::default(),
    };

    let tx = Transaction::new_slash_producer(slash_data.clone());

    assert!(tx.is_slash_producer());
    assert!(!tx.is_coinbase());
    assert!(!tx.is_claim_reward());
    assert!(!tx.is_exit());
    assert_eq!(tx.tx_type, TxType::SlashProducer);
    assert!(tx.inputs.is_empty());
    assert!(tx.outputs.is_empty()); // No outputs - bond is burned

    // Verify slash data can be parsed
    let parsed_data = tx.slash_data().unwrap();
    assert_eq!(parsed_data.producer_pubkey, slash_data.producer_pubkey);
}

#[test]
fn test_slash_producer_serialization() {
    use crate::BlockHeader;
    use vdf::{VdfOutput, VdfProof};

    let producer_keypair = crypto::KeyPair::generate();

    // Create test block headers with same producer and slot but different content
    let header1 = BlockHeader {
        version: 1,
        prev_hash: Hash::ZERO,
        merkle_root: crypto::hash::hash(b"block_a"),
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 0,
        slot: 99999,
        producer: *producer_keypair.public_key(),
        vdf_output: VdfOutput { value: vec![] },
        vdf_proof: VdfProof::empty(),
    };
    let header2 = BlockHeader {
        version: 1,
        prev_hash: Hash::ZERO,
        merkle_root: crypto::hash::hash(b"block_b"),
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 0,
        slot: 99999,
        producer: *producer_keypair.public_key(),
        vdf_output: VdfOutput { value: vec![] },
        vdf_proof: VdfProof::empty(),
    };

    let evidence = SlashingEvidence::DoubleProduction {
        block_header_1: header1,
        block_header_2: header2,
    };

    let slash_data = SlashData {
        producer_pubkey: *producer_keypair.public_key(),
        evidence,
        reporter_signature: Signature::default(),
    };

    let tx = Transaction::new_slash_producer(slash_data);
    let bytes = tx.serialize();
    let recovered = Transaction::deserialize(&bytes).unwrap();

    assert_eq!(tx.tx_type, recovered.tx_type);
    let recovered_data = recovered.slash_data().unwrap();

    // Check evidence type matches
    match recovered_data.evidence {
        SlashingEvidence::DoubleProduction {
            block_header_1,
            block_header_2,
        } => {
            assert_eq!(block_header_1.slot, 99999);
            assert_eq!(block_header_2.slot, 99999);
        }
    }
}

// ==================== EpochReward Transaction Tests ====================

#[test]
fn test_tx_type_epoch_reward_value() {
    assert_eq!(TxType::EpochReward as u32, 10);
}

#[test]
fn test_epoch_reward_data_serialization() {
    let keypair = crypto::KeyPair::generate();
    let data = EpochRewardData::new(42, *keypair.public_key());

    let bytes = data.to_bytes();
    let parsed = EpochRewardData::from_bytes(&bytes).unwrap();

    assert_eq!(data.epoch, parsed.epoch);
    assert_eq!(data.recipient, parsed.recipient);
}

#[test]
fn test_epoch_reward_data_from_bytes_short() {
    // Less than 40 bytes should return None
    let short_bytes = vec![0u8; 39];
    assert!(EpochRewardData::from_bytes(&short_bytes).is_none());
}

#[test]
fn test_new_epoch_reward_transaction() {
    let keypair = crypto::KeyPair::generate();
    let pubkey_hash =
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, keypair.public_key().as_bytes());

    let tx = Transaction::new_epoch_reward(
        5,                     // epoch
        *keypair.public_key(), // recipient
        1_000_000,             // amount
        pubkey_hash,           // recipient hash
    );

    assert!(tx.is_epoch_reward());
    assert!(!tx.is_coinbase());
    assert!(tx.inputs.is_empty());
    assert_eq!(tx.outputs.len(), 1);
    assert_eq!(tx.outputs[0].amount, 1_000_000);
    assert_eq!(tx.outputs[0].output_type, OutputType::Normal);

    let data = tx.epoch_reward_data().unwrap();
    assert_eq!(data.epoch, 5);
    assert_eq!(data.recipient, *keypair.public_key());
}

#[test]
fn test_epoch_reward_is_not_coinbase() {
    let keypair = crypto::KeyPair::generate();
    let pubkey_hash = Hash::ZERO;
    let tx = Transaction::new_epoch_reward(1, *keypair.public_key(), 1000, pubkey_hash);

    assert!(!tx.is_coinbase());
    assert!(tx.is_epoch_reward());
}

#[test]
fn test_epoch_reward_hash_deterministic() {
    let keypair = crypto::KeyPair::generate();
    let pubkey_hash = Hash::ZERO;

    let tx1 = Transaction::new_epoch_reward(1, *keypair.public_key(), 1000, pubkey_hash);
    let tx2 = Transaction::new_epoch_reward(1, *keypair.public_key(), 1000, pubkey_hash);

    assert_eq!(tx1.hash(), tx2.hash());
}

#[test]
fn test_epoch_reward_serialization_roundtrip() {
    let keypair = crypto::KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(b"recipient");

    let tx = Transaction::new_epoch_reward(
        100,                   // epoch
        *keypair.public_key(), // recipient
        50_000_000,            // amount
        pubkey_hash,           // recipient hash
    );

    let bytes = tx.serialize();
    let recovered = Transaction::deserialize(&bytes).unwrap();

    assert_eq!(tx.tx_type, recovered.tx_type);
    assert_eq!(tx, recovered);

    let recovered_data = recovered.epoch_reward_data().unwrap();
    assert_eq!(recovered_data.epoch, 100);
    assert_eq!(recovered_data.recipient, *keypair.public_key());
}

#[test]
fn test_epoch_reward_data_none_for_non_epoch_reward() {
    let tx = Transaction::new_coinbase(1000, Hash::ZERO, 0);
    assert!(tx.epoch_reward_data().is_none());
}

// ==================== Maintainer Transaction Tests ====================

#[test]
fn test_remove_maintainer_transaction() {
    let target = crypto::KeyPair::generate();

    let tx = Transaction::new_remove_maintainer(
        *target.public_key(),
        vec![], // Empty sigs for test - real tx would have 3+ sigs
        Some("Inactive for 6 months".to_string()),
    );

    assert!(tx.is_remove_maintainer());
    assert!(tx.is_maintainer_change());
    assert!(!tx.is_add_maintainer());
    assert_eq!(tx.tx_type, TxType::RemoveMaintainer);
    assert!(tx.inputs.is_empty());
    assert!(tx.outputs.is_empty());

    // Verify data can be parsed
    let data = tx.maintainer_change_data().unwrap();
    assert_eq!(data.target, *target.public_key());
    assert_eq!(data.reason, Some("Inactive for 6 months".to_string()));
}

#[test]
fn test_add_maintainer_transaction() {
    let target = crypto::KeyPair::generate();

    let tx = Transaction::new_add_maintainer(
        *target.public_key(),
        vec![], // Empty sigs for test
    );

    assert!(tx.is_add_maintainer());
    assert!(tx.is_maintainer_change());
    assert!(!tx.is_remove_maintainer());
    assert_eq!(tx.tx_type, TxType::AddMaintainer);
    assert!(tx.inputs.is_empty());
    assert!(tx.outputs.is_empty());

    // Verify data can be parsed
    let data = tx.maintainer_change_data().unwrap();
    assert_eq!(data.target, *target.public_key());
    assert!(data.reason.is_none());
}

#[test]
fn test_maintainer_tx_serialization_roundtrip() {
    let target = crypto::KeyPair::generate();

    let tx = Transaction::new_remove_maintainer(
        *target.public_key(),
        vec![],
        Some("Test removal".to_string()),
    );

    let bytes = tx.serialize();
    let recovered = Transaction::deserialize(&bytes).unwrap();

    assert_eq!(tx.tx_type, recovered.tx_type);
    assert_eq!(tx, recovered);

    let recovered_data = recovered.maintainer_change_data().unwrap();
    assert_eq!(recovered_data.target, *target.public_key());
}

#[test]
fn test_maintainer_change_data_none_for_other_tx_types() {
    let tx = Transaction::new_coinbase(1000, Hash::ZERO, 0);
    assert!(tx.maintainer_change_data().is_none());

    let keypair = crypto::KeyPair::generate();
    let tx = Transaction::new_exit(*keypair.public_key());
    assert!(tx.maintainer_change_data().is_none());
}

#[test]
fn test_delegate_bond_transaction() {
    let delegator = crypto::KeyPair::generate();
    let delegate = crypto::KeyPair::generate();
    let data = DelegateBondData::new(*delegator.public_key(), *delegate.public_key(), 5);
    let tx = Transaction::new_delegate_bond(data);

    assert!(tx.is_delegate_bond());
    assert!(!tx.is_revoke_delegation());
    assert_eq!(tx.tx_type, TxType::DelegateBond);
    assert!(tx.inputs.is_empty());
    assert!(tx.outputs.is_empty());

    let parsed = tx.delegate_bond_data().unwrap();
    assert_eq!(parsed.delegator, *delegator.public_key());
    assert_eq!(parsed.delegate, *delegate.public_key());
    assert_eq!(parsed.bond_count, 5);
}

#[test]
fn test_revoke_delegation_transaction() {
    let delegator = crypto::KeyPair::generate();
    let delegate = crypto::KeyPair::generate();
    let data = RevokeDelegationData::new(*delegator.public_key(), *delegate.public_key());
    let tx = Transaction::new_revoke_delegation(data);

    assert!(tx.is_revoke_delegation());
    assert!(!tx.is_delegate_bond());
    assert_eq!(tx.tx_type, TxType::RevokeDelegation);
    assert!(tx.inputs.is_empty());
    assert!(tx.outputs.is_empty());

    let parsed = tx.revoke_delegation_data().unwrap();
    assert_eq!(parsed.delegator, *delegator.public_key());
    assert_eq!(parsed.delegate, *delegate.public_key());
}

#[test]
fn test_delegate_bond_data_serialization() {
    let delegator = crypto::KeyPair::generate();
    let delegate = crypto::KeyPair::generate();
    let data = DelegateBondData::new(*delegator.public_key(), *delegate.public_key(), 42);
    let bytes = data.to_bytes();
    let recovered = DelegateBondData::from_bytes(&bytes).unwrap();
    assert_eq!(data, recovered);
}

#[test]
fn test_delegate_bond_data_too_short() {
    assert!(DelegateBondData::from_bytes(&[0u8; 67]).is_none());
    assert!(DelegateBondData::from_bytes(&[]).is_none());
}

#[test]
fn test_revoke_delegation_data_serialization() {
    let delegator = crypto::KeyPair::generate();
    let delegate = crypto::KeyPair::generate();
    let data = RevokeDelegationData::new(*delegator.public_key(), *delegate.public_key());
    let bytes = data.to_bytes();
    let recovered = RevokeDelegationData::from_bytes(&bytes).unwrap();
    assert_eq!(data, recovered);
}

// ==================== Protocol Activation Tests ====================

#[test]
fn test_protocol_activation_transaction() {
    use crate::maintainer::ProtocolActivationData;

    let data = ProtocolActivationData::new(2, 500, "Enable finality".to_string(), vec![]);
    let tx = Transaction::new_protocol_activation(data);

    assert!(tx.is_protocol_activation());
    assert_eq!(tx.tx_type, TxType::ProtocolActivation);
    assert!(tx.inputs.is_empty());
    assert!(tx.outputs.is_empty());

    let parsed = tx.protocol_activation_data().unwrap();
    assert_eq!(parsed.protocol_version, 2);
    assert_eq!(parsed.activation_epoch, 500);
    assert_eq!(parsed.description, "Enable finality");
}

#[test]
fn test_protocol_activation_serialization_roundtrip() {
    use crate::maintainer::ProtocolActivationData;

    let data = ProtocolActivationData::new(3, 1000, "New rules".to_string(), vec![]);
    let tx = Transaction::new_protocol_activation(data);

    let bytes = tx.serialize();
    let recovered = Transaction::deserialize(&bytes).unwrap();

    assert_eq!(tx.tx_type, recovered.tx_type);
    assert_eq!(tx, recovered);

    let recovered_data = recovered.protocol_activation_data().unwrap();
    assert_eq!(recovered_data.protocol_version, 3);
    assert_eq!(recovered_data.activation_epoch, 1000);
}

#[test]
fn test_protocol_activation_data_none_for_other_types() {
    let tx = Transaction::new_coinbase(1000, Hash::ZERO, 0);
    assert!(tx.protocol_activation_data().is_none());
}

#[test]
fn test_tx_type_from_u32_protocol_activation() {
    assert_eq!(TxType::from_u32(15), Some(TxType::ProtocolActivation));
    assert_eq!(TxType::from_u32(16), None);
    assert_eq!(TxType::from_u32(17), Some(TxType::MintAsset));
    assert_eq!(TxType::from_u32(18), Some(TxType::BurnAsset));
    assert_eq!(TxType::from_u32(19), None);
}

// Property-based tests
use proptest::prelude::*;

#[allow(dead_code)]
fn arb_hash() -> impl Strategy<Value = Hash> {
    any::<[u8; 32]>().prop_map(Hash::from_bytes)
}

#[allow(dead_code)]
fn arb_output() -> impl Strategy<Value = Output> {
    (
        1u64..=u64::MAX / 2,
        arb_hash(),
        any::<bool>(),
        0u64..1_000_000u64,
    )
        .prop_map(|(amount, pubkey_hash, is_bond, lock)| {
            if is_bond {
                Output::bond(amount, pubkey_hash, lock.max(1), 0)
            } else {
                Output::normal(amount, pubkey_hash)
            }
        })
}

#[allow(dead_code)]
fn arb_input() -> impl Strategy<Value = Input> {
    (arb_hash(), 0u32..1000u32).prop_map(|(hash, idx)| Input::new(hash, idx))
}

proptest! {
    /// Transaction hash is deterministic
    #[test]
    fn prop_tx_hash_deterministic(amount in 1u64..u64::MAX/2, height: u64, seed: [u8; 32]) {
        let pubkey_hash = Hash::from_bytes(seed);
        let tx = Transaction::new_coinbase(amount, pubkey_hash, height);
        prop_assert_eq!(tx.hash(), tx.hash());
    }

    /// Different transactions have different hashes (with high probability)
    #[test]
    fn prop_different_tx_different_hash(amount1 in 1u64..u64::MAX/2, amount2 in 1u64..u64::MAX/2, height1: u64, height2: u64) {
        prop_assume!(amount1 != amount2 || height1 != height2);
        let pubkey_hash = Hash::ZERO;
        let tx1 = Transaction::new_coinbase(amount1, pubkey_hash, height1);
        let tx2 = Transaction::new_coinbase(amount2, pubkey_hash, height2);
        prop_assert_ne!(tx1.hash(), tx2.hash());
    }

    /// Serialization roundtrip preserves transaction
    #[test]
    fn prop_tx_serialization_roundtrip(amount in 1u64..u64::MAX/2, height: u64, seed: [u8; 32]) {
        let pubkey_hash = Hash::from_bytes(seed);
        let tx = Transaction::new_coinbase(amount, pubkey_hash, height);
        let bytes = tx.serialize();
        let recovered = Transaction::deserialize(&bytes);
        prop_assert!(recovered.is_some());
        prop_assert_eq!(tx, recovered.unwrap());
    }

    /// total_output sums correctly
    #[test]
    fn prop_total_output_sums(amounts in prop::collection::vec(1u64..1_000_000u64, 1..10)) {
        let outputs: Vec<Output> = amounts.iter()
            .map(|&a| Output::normal(a, Hash::ZERO))
            .collect();
        let tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![Input::new(Hash::ZERO, 0)],
            outputs,
            extra_data: vec![],
        };
        let expected: Amount = amounts.iter().sum();
        prop_assert_eq!(tx.total_output(), expected);
    }

    /// Output spendability: normal outputs always spendable
    #[test]
    fn prop_normal_always_spendable(amount in 1u64..u64::MAX/2, height: u64) {
        let output = Output::normal(amount, Hash::ZERO);
        prop_assert!(output.is_spendable_at(height));
    }

    /// Output spendability: bond outputs respect lock time
    #[test]
    fn prop_bond_respects_lock(amount in 1u64..u64::MAX/2, lock_height in 1u64..u64::MAX/2) {
        let output = Output::bond(amount, Hash::ZERO, lock_height, 0);
        // Not spendable before lock
        if lock_height > 0 {
            prop_assert!(!output.is_spendable_at(lock_height - 1));
        }
        // Spendable at and after lock
        prop_assert!(output.is_spendable_at(lock_height));
        if lock_height < u64::MAX {
            prop_assert!(output.is_spendable_at(lock_height + 1));
        }
    }

    /// Input serialization is deterministic
    #[test]
    fn prop_input_serialize_deterministic(seed: [u8; 32], idx: u32) {
        let hash = Hash::from_bytes(seed);
        let input = Input::new(hash, idx);
        prop_assert_eq!(input.serialize_for_signing(), input.serialize_for_signing());
    }

    /// Output serialization is deterministic
    #[test]
    fn prop_output_serialize_deterministic(amount in 1u64..u64::MAX/2, seed: [u8; 32]) {
        let output = Output::normal(amount, Hash::from_bytes(seed));
        prop_assert_eq!(output.serialize(), output.serialize());
    }

    /// Coinbase detection: empty inputs + single output = coinbase
    #[test]
    fn prop_coinbase_detection(amount in 1u64..u64::MAX/2, height: u64, seed: [u8; 32]) {
        let tx = Transaction::new_coinbase(amount, Hash::from_bytes(seed), height);
        prop_assert!(tx.is_coinbase());
        prop_assert!(tx.inputs.is_empty());
        prop_assert_eq!(tx.outputs.len(), 1);
    }

    /// Transfer with inputs is not coinbase
    #[test]
    fn prop_transfer_not_coinbase(seed: [u8; 32], idx: u32) {
        let hash = Hash::from_bytes(seed);
        let tx = Transaction::new_transfer(
            vec![Input::new(hash, idx)],
            vec![Output::normal(100, Hash::ZERO)],
        );
        prop_assert!(!tx.is_coinbase());
    }
}

#[test]
fn test_sighash_all_per_input_unique() {
    // BIP-143: each input gets a unique signing hash due to outpoint inclusion
    let tx = Transaction::new_transfer(
        vec![
            Input::new(Hash::ZERO, 0),
            Input::new(Hash::ZERO, 1),
            Input::new(Hash::from_bytes([1u8; 32]), 0),
        ],
        vec![Output::normal(100, Hash::ZERO)],
    );
    let h0 = tx.signing_message_for_input(0);
    let h1 = tx.signing_message_for_input(1);
    let h2 = tx.signing_message_for_input(2);
    // All three must be different
    assert_ne!(h0, h1);
    assert_ne!(h0, h2);
    assert_ne!(h1, h2);
}

#[test]
fn test_anyone_can_pay_differs_from_all() {
    // AnyoneCanPay should produce a DIFFERENT hash than All
    let tx = Transaction::new_transfer(
        vec![
            Input::new_anyone_can_pay(Hash::ZERO, 0),
            Input::new(Hash::ZERO, 1),
        ],
        vec![Output::normal(100, Hash::ZERO)],
    );
    let acp_hash = tx.signing_message_for_input(0);
    let all_hash = tx.signing_message_for_input(1);
    assert_ne!(acp_hash, all_hash);
}

#[test]
fn test_anyone_can_pay_stable_after_adding_inputs() {
    // The key PSBT property: seller signs with AnyoneCanPay,
    // then buyer adds inputs — seller's hash should NOT change.
    let outputs = vec![
        Output::normal(50, Hash::ZERO),  // NFT to buyer
        Output::normal(100, Hash::ZERO), // Payment to seller
    ];

    // Step 1: partial TX with only seller's input
    let tx_partial = Transaction::new_transfer(
        vec![Input::new_anyone_can_pay(Hash::ZERO, 0)],
        outputs.clone(),
    );
    let seller_hash_before = tx_partial.signing_message_for_input(0);

    // Step 2: full TX with buyer's inputs added
    let tx_full = Transaction::new_transfer(
        vec![
            Input::new_anyone_can_pay(Hash::ZERO, 0), // seller's input (same)
            Input::new(Hash::from_bytes([1u8; 32]), 0), // buyer's input 1
            Input::new(Hash::from_bytes([2u8; 32]), 1), // buyer's input 2
        ],
        outputs,
    );
    let seller_hash_after = tx_full.signing_message_for_input(0);

    // Seller's AnyoneCanPay hash must be identical before and after buyer adds inputs
    assert_eq!(seller_hash_before, seller_hash_after);
}

#[test]
fn test_anyone_can_pay_changes_if_outputs_change() {
    // Security: if outputs change, AnyoneCanPay hash MUST change
    let tx1 = Transaction::new_transfer(
        vec![Input::new_anyone_can_pay(Hash::ZERO, 0)],
        vec![Output::normal(100, Hash::ZERO)],
    );
    let tx2 = Transaction::new_transfer(
        vec![Input::new_anyone_can_pay(Hash::ZERO, 0)],
        vec![Output::normal(200, Hash::ZERO)], // different amount
    );
    assert_ne!(
        tx1.signing_message_for_input(0),
        tx2.signing_message_for_input(0)
    );
}

#[test]
fn test_nft_royalty_roundtrip() {
    let creator = Hash::from_bytes([42u8; 32]);
    let owner = Hash::from_bytes([1u8; 32]);
    let token_id = Hash::from_bytes([2u8; 32]);
    let content = b"test";
    let cond = crate::conditions::Condition::signature(owner);

    let output = Output::nft_with_royalty(
        0, owner, token_id, content, &cond, creator, 500, // 5%
    )
    .unwrap();

    // Should be able to extract royalty
    let (extracted_creator, extracted_bps) = output.nft_royalty().unwrap();
    assert_eq!(extracted_creator, creator);
    assert_eq!(extracted_bps, 500);

    // Should also extract metadata normally
    let (extracted_token_id, extracted_content) = output.nft_metadata().unwrap();
    assert_eq!(extracted_token_id, token_id);
    assert_eq!(extracted_content, content);
}

#[test]
fn test_nft_no_royalty() {
    let owner = Hash::from_bytes([1u8; 32]);
    let token_id = Hash::from_bytes([2u8; 32]);
    let content = b"test";
    let cond = crate::conditions::Condition::signature(owner);

    let output = Output::nft(0, owner, token_id, content, &cond).unwrap();

    // No royalty on v1 NFT
    assert!(output.nft_royalty().is_none());

    // But metadata should still work
    assert!(output.nft_metadata().is_some());
}

#[test]
fn test_sighash_type_serialization_backwards_compat() {
    // A v1 transaction (SighashType::All) should serialize and deserialize correctly
    let tx = Transaction::new_transfer(
        vec![Input::new(Hash::ZERO, 0)],
        vec![Output::normal(100, Hash::ZERO)],
    );
    let bytes = tx.serialize();
    let tx2 = Transaction::deserialize(&bytes).unwrap();
    assert_eq!(tx2.inputs[0].sighash_type, SighashType::All);
}

#[test]
fn test_sighash_anyone_can_pay_serialization() {
    let tx = Transaction::new_transfer(
        vec![Input::new_anyone_can_pay(Hash::ZERO, 0)],
        vec![Output::normal(100, Hash::ZERO)],
    );
    let bytes = tx.serialize();
    let tx2 = Transaction::deserialize(&bytes).unwrap();
    assert_eq!(tx2.inputs[0].sighash_type, SighashType::AnyoneCanPay);
}

#[test]
fn test_committed_output_count_allows_appended_outputs() {
    // Seller creates partial TX with 2 outputs, commits to 2
    let seller_input = Input::new_anyone_can_pay_partial(Hash::ZERO, 0, 2);
    let tx_at_sign = Transaction::new_transfer(
        vec![seller_input],
        vec![
            Output::normal(100, Hash::ZERO), // NFT → buyer
            Output::normal(50, Hash::ZERO),  // payment → seller
        ],
    );
    let sighash_at_sign = tx_at_sign.signing_message_for_input(0);

    // Buyer appends a change output — sighash must remain the same
    let buyer_input = Input::new_anyone_can_pay_partial(Hash::ZERO, 0, 2);
    let tx_with_change = Transaction::new_transfer(
        vec![buyer_input],
        vec![
            Output::normal(100, Hash::ZERO), // NFT → buyer (same)
            Output::normal(50, Hash::ZERO),  // payment → seller (same)
            Output::normal(30, Hash::ZERO),  // change → buyer (appended)
        ],
    );
    let sighash_with_change = tx_with_change.signing_message_for_input(0);

    assert_eq!(
        sighash_at_sign, sighash_with_change,
        "Appending outputs must not change sighash when committed_output_count is set"
    );
}

#[test]
fn test_committed_output_count_zero_means_all() {
    // committed_output_count=0 (backward compat) hashes ALL outputs
    let input_old = Input::new_anyone_can_pay(Hash::ZERO, 0); // count=0
    let tx2 = Transaction::new_transfer(
        vec![input_old],
        vec![
            Output::normal(100, Hash::ZERO),
            Output::normal(50, Hash::ZERO),
        ],
    );
    let hash_all = tx2.signing_message_for_input(0);

    // Same outputs with committed_output_count=2 should produce same hash
    let input_explicit = Input::new_anyone_can_pay_partial(Hash::ZERO, 0, 2);
    let tx3 = Transaction::new_transfer(
        vec![input_explicit],
        vec![
            Output::normal(100, Hash::ZERO),
            Output::normal(50, Hash::ZERO),
        ],
    );
    let hash_explicit = tx3.signing_message_for_input(0);

    assert_eq!(
        hash_all, hash_explicit,
        "committed_output_count=N should match count=0 when N equals total outputs"
    );
}

#[test]
fn test_committed_output_count_serialization_roundtrip() {
    let tx = Transaction::new_transfer(
        vec![Input::new_anyone_can_pay_partial(Hash::ZERO, 0, 3)],
        vec![Output::normal(100, Hash::ZERO)],
    );
    let bytes = tx.serialize();
    let tx2 = Transaction::deserialize(&bytes).unwrap();
    assert_eq!(tx2.inputs[0].sighash_type, SighashType::AnyoneCanPay);
    assert_eq!(tx2.inputs[0].committed_output_count, 3);
}
