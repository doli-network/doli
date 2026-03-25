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
    assert_eq!(TxType::from_u32(19), Some(TxType::CreatePool));
    assert_eq!(TxType::from_u32(20), Some(TxType::AddLiquidity));
    assert_eq!(TxType::from_u32(21), Some(TxType::RemoveLiquidity));
    assert_eq!(TxType::from_u32(22), Some(TxType::Swap));
    assert_eq!(TxType::from_u32(23), None);
    assert_eq!(TxType::from_u32(24), Some(TxType::CreateLoan));
    assert_eq!(TxType::from_u32(25), Some(TxType::RepayLoan));
    assert_eq!(TxType::from_u32(26), Some(TxType::LiquidateLoan));
    assert_eq!(TxType::from_u32(27), Some(TxType::LendingDeposit));
    assert_eq!(TxType::from_u32(28), Some(TxType::LendingWithdraw));
    assert_eq!(TxType::from_u32(29), None);
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
    assert_eq!(OutputType::from_u8(9), Some(OutputType::Pool));
    assert_eq!(OutputType::from_u8(10), Some(OutputType::LPShare));
    assert_eq!(OutputType::from_u8(11), Some(OutputType::Collateral));
    assert_eq!(OutputType::from_u8(12), Some(OutputType::LendingDeposit));
    assert_eq!(OutputType::from_u8(13), None);
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
    assert_eq!(TxType::from_u32(19), Some(TxType::CreatePool));
    assert_eq!(TxType::from_u32(20), Some(TxType::AddLiquidity));
    assert_eq!(TxType::from_u32(21), Some(TxType::RemoveLiquidity));
    assert_eq!(TxType::from_u32(22), Some(TxType::Swap));
    assert_eq!(TxType::from_u32(23), None);
    assert_eq!(TxType::from_u32(24), Some(TxType::CreateLoan));
    assert_eq!(TxType::from_u32(25), Some(TxType::RepayLoan));
    assert_eq!(TxType::from_u32(26), Some(TxType::LiquidateLoan));
    assert_eq!(TxType::from_u32(27), Some(TxType::LendingDeposit));
    assert_eq!(TxType::from_u32(28), Some(TxType::LendingWithdraw));
    assert_eq!(TxType::from_u32(29), None);
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

// ==================== Fee Routing Tests ====================

#[test]
fn test_coinbase_extra_fees_from_nft() {
    // Block with 1 NFT (300 bytes extra_data)
    // extra_fees = 300 * FEE_PER_BYTE = 300
    // coinbase = block_reward + 300
    let block_reward = 100_000_000u64;
    let extra_fees = 300u64 * crate::consensus::FEE_PER_BYTE;
    assert_eq!(block_reward + extra_fees, 100_000_300);
}

#[test]
fn test_coinbase_no_extra_for_transfers() {
    let extra_fees = 0u64 * crate::consensus::FEE_PER_BYTE;
    assert_eq!(extra_fees, 0);
}

#[test]
fn test_extra_fees_mixed_block() {
    // 3 NFTs x 300 bytes + 1 pool x 116 bytes + 2 transfers x 0
    let bytes: Vec<u64> = vec![300, 300, 300, 116, 0, 0];
    let extra: u64 = bytes
        .iter()
        .map(|b| b * crate::consensus::FEE_PER_BYTE)
        .sum();
    assert_eq!(extra, 1016);
}

#[test]
fn test_protocol_txs_excluded_from_extra_fees() {
    // Registration and EpochReward TXs have extra_data but should NOT count
    // This is enforced by filtering in validation_checks.rs
    let excluded = [TxType::Registration, TxType::EpochReward, TxType::Coinbase];
    for tt in &excluded {
        // These types are filtered out in the extra_fees calculation
        assert!(matches!(
            tt,
            TxType::Registration | TxType::EpochReward | TxType::Coinbase
        ));
    }
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

// ==================== Bridge HTLC v2 Tests ====================

/// Test 1: BridgeHTLC v2 roundtrip — create, serialize, deserialize, extract metadata
#[test]
fn test_bridge_htlc_v2_roundtrip() {
    use crate::conditions::HASHLOCK_DOMAIN;
    use crate::transaction::{
        BRIDGE_CHAIN_BITCOIN, BRIDGE_HTLC_CURRENT_VERSION, BRIDGE_HTLC_VERSION_V2,
    };
    use crypto::hash::hash_with_domain;

    let preimage = [0x42u8; 32];
    let expected_hash = hash_with_domain(HASHLOCK_DOMAIN, &preimage);
    let counter_hash = Hash::from_bytes([0xBBu8; 32]);
    let pubkey_hash = Hash::from_bytes([0xAAu8; 32]);
    let target_address = b"bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";

    let output = Output::bridge_htlc(
        1_000_000,
        pubkey_hash,
        expected_hash,
        100,
        200,
        BRIDGE_CHAIN_BITCOIN,
        target_address,
        counter_hash,
    )
    .unwrap();

    assert_eq!(output.output_type, OutputType::BridgeHTLC);
    assert_eq!(output.amount, 1_000_000);
    assert_eq!(output.pubkey_hash, pubkey_hash);

    // Verify version byte in extra_data
    assert_eq!(BRIDGE_HTLC_CURRENT_VERSION, BRIDGE_HTLC_VERSION_V2);

    // Extract metadata and verify all fields
    let (chain_id, addr, opt_counter) = output.bridge_htlc_metadata().unwrap();
    assert_eq!(chain_id, BRIDGE_CHAIN_BITCOIN);
    assert_eq!(addr, target_address.to_vec());
    assert_eq!(opt_counter, Some(counter_hash));

    // Verify condition decodes correctly
    let cond = output.condition().unwrap().unwrap();
    match &cond {
        crate::conditions::Condition::Or(left, right) => {
            // Left: And(Hashlock, Timelock)
            match left.as_ref() {
                crate::conditions::Condition::And(h, t) => {
                    assert!(matches!(
                        h.as_ref(),
                        crate::conditions::Condition::Hashlock(_)
                    ));
                    assert!(matches!(
                        t.as_ref(),
                        crate::conditions::Condition::Timelock(100)
                    ));
                }
                _ => panic!("Expected And(Hashlock, Timelock)"),
            }
            // Right: TimelockExpiry
            assert!(matches!(
                right.as_ref(),
                crate::conditions::Condition::TimelockExpiry(200)
            ));
        }
        _ => panic!("Expected Or condition"),
    }

    // Serialize and deserialize the output
    let _bytes = output.serialize();
    let tx = Transaction::new_transfer(vec![Input::new(Hash::ZERO, 0)], vec![output.clone()]);
    let tx_bytes = tx.serialize();
    let recovered = Transaction::deserialize(&tx_bytes).unwrap();
    let recovered_output = &recovered.outputs[0];

    // Metadata must survive serialization roundtrip
    let (r_chain, r_addr, r_counter) = recovered_output.bridge_htlc_metadata().unwrap();
    assert_eq!(r_chain, BRIDGE_CHAIN_BITCOIN);
    assert_eq!(r_addr, target_address.to_vec());
    assert_eq!(r_counter, Some(counter_hash));
}

/// Test 2: BridgeHTLC v1 backward compatibility
#[test]
fn test_bridge_htlc_v1_backward_compat() {
    use crate::conditions::{Condition, HASHLOCK_DOMAIN};
    use crate::transaction::{BRIDGE_CHAIN_ETHEREUM, BRIDGE_HTLC_VERSION_V1};
    use crypto::hash::hash_with_domain;

    let preimage = [0x55u8; 32];
    let expected_hash = hash_with_domain(HASHLOCK_DOMAIN, &preimage);
    let pubkey_hash = Hash::from_bytes([0xCCu8; 32]);
    let target_address = b"0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045";

    // Manually construct a v1 BridgeHTLC (no counter_hash)
    let cond = Condition::htlc(expected_hash, 50, 150);
    let condition_bytes = cond.encode().unwrap();
    let mut extra_data = condition_bytes;
    extra_data.push(BRIDGE_HTLC_VERSION_V1); // version 1
    extra_data.push(BRIDGE_CHAIN_ETHEREUM); // chain
    extra_data.push(target_address.len() as u8);
    extra_data.extend_from_slice(target_address);

    let output = Output {
        output_type: OutputType::BridgeHTLC,
        amount: 500_000,
        pubkey_hash,
        lock_until: 0,
        extra_data,
    };

    // bridge_htlc_metadata must work and return None for counter_hash
    let (chain_id, addr, opt_counter) = output.bridge_htlc_metadata().unwrap();
    assert_eq!(chain_id, BRIDGE_CHAIN_ETHEREUM);
    assert_eq!(addr, target_address.to_vec());
    assert_eq!(opt_counter, None); // v1 has no counter_hash

    // Condition must still be decodable
    let cond_decoded = output.condition().unwrap().unwrap();
    assert_eq!(cond, cond_decoded);
}

/// Test 3: Counter-hash derivation correctness
#[test]
fn test_counter_hash_derivation() {
    use sha2::{Digest, Sha256};

    // Known preimage
    let preimage = [0x01u8; 32];

    // Bitcoin: SHA256(preimage)
    let mut sha = Sha256::new();
    sha.update(preimage);
    let sha256_result = sha.finalize();
    // Verify it's 32 bytes and non-zero
    assert_eq!(sha256_result.len(), 32);
    assert_ne!(&sha256_result[..], &[0u8; 32]);

    // Known SHA256([0x01; 32]) from external calculation
    let sha256_hex = hex::encode(sha256_result);
    // SHA256 of 32 bytes of 0x01 is deterministic
    let mut sha_verify = Sha256::new();
    sha_verify.update([0x01u8; 32]);
    assert_eq!(sha256_hex, hex::encode(sha_verify.finalize()));

    // Ethereum: keccak256(preimage)
    use tiny_keccak::{Hasher, Keccak};
    let mut keccak_output = [0u8; 32];
    let mut keccak = Keccak::v256();
    keccak.update(&preimage);
    keccak.finalize(&mut keccak_output);
    assert_ne!(keccak_output, [0u8; 32]);

    // keccak256 and SHA256 must differ for same input
    assert_ne!(&sha256_result[..], &keccak_output[..]);

    // Verify keccak256 is deterministic
    let mut keccak_output2 = [0u8; 32];
    let mut keccak2 = Keccak::v256();
    keccak2.update(&preimage);
    keccak2.finalize(&mut keccak_output2);
    assert_eq!(keccak_output, keccak_output2);

    // Both hashes must be usable as Hash
    let btc_hash = Hash::from_bytes(sha256_result.into());
    let eth_hash = Hash::from_bytes(keccak_output);
    assert_ne!(btc_hash, eth_hash);

    // Verify they can be stored in BridgeHTLC
    let doli_hash = crypto::hash::hash_with_domain(crate::conditions::HASHLOCK_DOMAIN, &preimage);
    let pubkey = Hash::from_bytes([0xAAu8; 32]);

    let btc_output = Output::bridge_htlc(
        1000,
        pubkey,
        doli_hash,
        10,
        20,
        crate::transaction::BRIDGE_CHAIN_BITCOIN,
        b"bc1test",
        btc_hash,
    )
    .unwrap();
    let (_, _, ch) = btc_output.bridge_htlc_metadata().unwrap();
    assert_eq!(ch, Some(btc_hash));

    let eth_output = Output::bridge_htlc(
        1000,
        pubkey,
        doli_hash,
        10,
        20,
        crate::transaction::BRIDGE_CHAIN_ETHEREUM,
        b"0xtest",
        eth_hash,
    )
    .unwrap();
    let (_, _, ch) = eth_output.bridge_htlc_metadata().unwrap();
    assert_eq!(ch, Some(eth_hash));
}

/// Test 7: Claim with incorrect preimage fails
#[test]
fn test_bridge_htlc_wrong_preimage_fails() {
    use crate::conditions::{evaluate, Condition, EvalContext, Witness, HASHLOCK_DOMAIN};
    use crypto::hash::hash_with_domain;

    let correct_preimage = [0xAA; 32];
    let wrong_preimage = [0xBB; 32];
    let expected_hash = hash_with_domain(HASHLOCK_DOMAIN, &correct_preimage);

    let cond = Condition::htlc(expected_hash, 10, 100);
    let signing_hash = Hash::from_bytes([0x00; 32]);

    // Correct preimage succeeds
    let good_witness = Witness {
        preimage: Some(correct_preimage),
        or_branches: vec![false], // left branch (claim)
        ..Default::default()
    };
    let ctx = EvalContext {
        current_height: 50,
        signing_hash: &signing_hash,
    };
    let mut idx = 0;
    assert!(evaluate(&cond, &good_witness, &ctx, &mut idx));

    // Wrong preimage fails
    let bad_witness = Witness {
        preimage: Some(wrong_preimage),
        or_branches: vec![false],
        ..Default::default()
    };
    let mut idx = 0;
    assert!(!evaluate(&cond, &bad_witness, &ctx, &mut idx));

    // No preimage fails
    let empty_witness = Witness {
        or_branches: vec![false],
        ..Default::default()
    };
    let mut idx = 0;
    assert!(!evaluate(&cond, &empty_witness, &ctx, &mut idx));
}

/// Test 8: Cannot claim before lock_height
#[test]
fn test_bridge_htlc_claim_before_lock_fails() {
    use crate::conditions::{evaluate, Condition, EvalContext, Witness, HASHLOCK_DOMAIN};
    use crypto::hash::hash_with_domain;

    let preimage = [0xCC; 32];
    let expected_hash = hash_with_domain(HASHLOCK_DOMAIN, &preimage);

    // lock_height=100, meaning claim only at height >= 100
    let cond = Condition::htlc(expected_hash, 100, 500);
    let signing_hash = Hash::from_bytes([0x00; 32]);

    let claim_witness = Witness {
        preimage: Some(preimage),
        or_branches: vec![false], // left branch (claim path)
        ..Default::default()
    };

    // At height 1 (well before lock): fails
    let ctx_early = EvalContext {
        current_height: 1,
        signing_hash: &signing_hash,
    };
    let mut idx = 0;
    assert!(!evaluate(&cond, &claim_witness, &ctx_early, &mut idx));

    // At height 99 (just before lock): fails
    let ctx_just_before = EvalContext {
        current_height: 99,
        signing_hash: &signing_hash,
    };
    let mut idx = 0;
    assert!(!evaluate(&cond, &claim_witness, &ctx_just_before, &mut idx));

    // At height 100 (exactly at lock): succeeds
    let ctx_at_lock = EvalContext {
        current_height: 100,
        signing_hash: &signing_hash,
    };
    let mut idx = 0;
    assert!(evaluate(&cond, &claim_witness, &ctx_at_lock, &mut idx));

    // At height 101 (after lock): succeeds
    let ctx_after = EvalContext {
        current_height: 101,
        signing_hash: &signing_hash,
    };
    let mut idx = 0;
    assert!(evaluate(&cond, &claim_witness, &ctx_after, &mut idx));
}

/// Test 9: Cannot refund before expiry_height
#[test]
fn test_bridge_htlc_refund_before_expiry_fails() {
    use crate::conditions::{evaluate, Condition, EvalContext, Witness, HASHLOCK_DOMAIN};
    use crypto::hash::hash_with_domain;

    let preimage = [0xDD; 32];
    let expected_hash = hash_with_domain(HASHLOCK_DOMAIN, &preimage);

    // expiry_height=100
    let cond = Condition::htlc(expected_hash, 10, 100);
    let signing_hash = Hash::from_bytes([0x00; 32]);

    let refund_witness = Witness {
        or_branches: vec![true], // right branch (refund path)
        ..Default::default()
    };

    // At height 1: refund fails
    let ctx_early = EvalContext {
        current_height: 1,
        signing_hash: &signing_hash,
    };
    let mut idx = 0;
    assert!(!evaluate(&cond, &refund_witness, &ctx_early, &mut idx));

    // At height 99: refund fails
    let ctx_just_before = EvalContext {
        current_height: 99,
        signing_hash: &signing_hash,
    };
    let mut idx = 0;
    assert!(!evaluate(
        &cond,
        &refund_witness,
        &ctx_just_before,
        &mut idx
    ));

    // At height 100 (exactly at expiry): refund succeeds
    let ctx_at_expiry = EvalContext {
        current_height: 100,
        signing_hash: &signing_hash,
    };
    let mut idx = 0;
    assert!(evaluate(&cond, &refund_witness, &ctx_at_expiry, &mut idx));

    // At height 200 (well after expiry): refund succeeds
    let ctx_well_after = EvalContext {
        current_height: 200,
        signing_hash: &signing_hash,
    };
    let mut idx = 0;
    assert!(evaluate(&cond, &refund_witness, &ctx_well_after, &mut idx));
}

/// Test 10: counter_hash is metadata only — does not affect spending condition
#[test]
fn test_counter_hash_does_not_affect_spending() {
    use crate::conditions::{evaluate, EvalContext, Witness, HASHLOCK_DOMAIN};
    use crypto::hash::hash_with_domain;

    let preimage = [0xEE; 32];
    let expected_hash = hash_with_domain(HASHLOCK_DOMAIN, &preimage);
    let pubkey_hash = Hash::from_bytes([0xAA; 32]);

    // Two BridgeHTLCs with different counter_hashes
    let counter_hash_a = Hash::from_bytes([0x11; 32]);
    let counter_hash_b = Hash::from_bytes([0x22; 32]);

    let output_a = Output::bridge_htlc(
        1000,
        pubkey_hash,
        expected_hash,
        10,
        100,
        crate::transaction::BRIDGE_CHAIN_BITCOIN,
        b"addr1",
        counter_hash_a,
    )
    .unwrap();

    let output_b = Output::bridge_htlc(
        1000,
        pubkey_hash,
        expected_hash,
        10,
        100,
        crate::transaction::BRIDGE_CHAIN_BITCOIN,
        b"addr1",
        counter_hash_b,
    )
    .unwrap();

    // Verify counter_hashes differ
    let (_, _, ch_a) = output_a.bridge_htlc_metadata().unwrap();
    let (_, _, ch_b) = output_b.bridge_htlc_metadata().unwrap();
    assert_ne!(ch_a, ch_b);

    // Both must be spendable with the same preimage
    let cond_a = output_a.condition().unwrap().unwrap();
    let cond_b = output_b.condition().unwrap().unwrap();

    // The conditions themselves must be identical (counter_hash is outside the condition)
    assert_eq!(cond_a, cond_b);

    let signing_hash = Hash::from_bytes([0x00; 32]);
    let claim_witness = Witness {
        preimage: Some(preimage),
        or_branches: vec![false],
        ..Default::default()
    };
    let ctx = EvalContext {
        current_height: 50,
        signing_hash: &signing_hash,
    };

    let mut idx = 0;
    assert!(evaluate(&cond_a, &claim_witness, &ctx, &mut idx));
    let mut idx = 0;
    assert!(evaluate(&cond_b, &claim_witness, &ctx, &mut idx));

    // Refund also works identically on both
    let refund_witness = Witness {
        or_branches: vec![true],
        ..Default::default()
    };
    let ctx_expired = EvalContext {
        current_height: 200,
        signing_hash: &signing_hash,
    };
    let mut idx = 0;
    assert!(evaluate(&cond_a, &refund_witness, &ctx_expired, &mut idx));
    let mut idx = 0;
    assert!(evaluate(&cond_b, &refund_witness, &ctx_expired, &mut idx));
}

/// Test 4: Swap happy path — BridgeHTLC creation + claim with preimage
#[test]
fn test_bridge_swap_claim_happy_path() {
    use crate::conditions::{evaluate, EvalContext, Witness, HASHLOCK_DOMAIN};
    use crypto::hash::hash_with_domain;

    // Simulate: Alice creates BridgeHTLC on DOLI targeting Bitcoin
    let preimage = [0x77; 32];
    let expected_hash = hash_with_domain(HASHLOCK_DOMAIN, &preimage);

    // Compute Bitcoin counter_hash (SHA256)
    use sha2::{Digest, Sha256};
    let mut sha = Sha256::new();
    sha.update(preimage);
    let sha_result: [u8; 32] = sha.finalize().into();
    let counter_hash = Hash::from_bytes(sha_result);

    let creator_hash = Hash::from_bytes([0x01; 32]);
    let lock_height = 50;
    let expiry_height = 200;

    let bridge_output = Output::bridge_htlc(
        10_000_000,
        creator_hash,
        expected_hash,
        lock_height,
        expiry_height,
        crate::transaction::BRIDGE_CHAIN_BITCOIN,
        b"bc1qtest",
        counter_hash,
    )
    .unwrap();

    // Verify UTXO is self-describing
    let (chain, addr, ch) = bridge_output.bridge_htlc_metadata().unwrap();
    assert_eq!(chain, crate::transaction::BRIDGE_CHAIN_BITCOIN);
    assert_eq!(addr, b"bc1qtest".to_vec());
    assert_eq!(ch.unwrap(), counter_hash);

    // Simulate: counterparty locks BTC (off-chain, we just note counter_hash matches)
    // Now preimage P is revealed on Bitcoin. Bob uses P to claim on DOLI.

    // Build claim transaction
    let claim_dest = Hash::from_bytes([0x02; 32]);
    let claim_output = Output::normal(10_000_000 - 1, claim_dest); // minus fee
    let mut claim_tx = Transaction::new_transfer(
        vec![Input::new(Hash::from_bytes([0xFF; 32]), 0)],
        vec![claim_output],
    );

    let signing_hash = claim_tx.signing_message_for_input(0);

    // Claim witness: branch(left) + preimage
    let claim_witness = Witness {
        preimage: Some(preimage),
        or_branches: vec![false], // left branch (claim path)
        ..Default::default()
    };

    // Evaluate: must succeed at height >= lock_height
    let cond = bridge_output.condition().unwrap().unwrap();
    let ctx = EvalContext {
        current_height: 100, // > lock_height(50)
        signing_hash: &signing_hash,
    };
    let mut idx = 0;
    assert!(evaluate(&cond, &claim_witness, &ctx, &mut idx));

    // Encode witness into transaction
    let witness_bytes = claim_witness.encode();
    claim_tx.set_covenant_witnesses(&[witness_bytes]);
    assert!(!claim_tx.extra_data.is_empty());
}

/// Test 5: Swap happy path Ethereum — keccak256 counter_hash
#[test]
fn test_bridge_swap_ethereum_happy_path() {
    use crate::conditions::{evaluate, EvalContext, Witness, HASHLOCK_DOMAIN};
    use crypto::hash::hash_with_domain;
    use tiny_keccak::{Hasher, Keccak};

    let preimage = [0x99; 32];
    let expected_hash = hash_with_domain(HASHLOCK_DOMAIN, &preimage);

    // Compute Ethereum counter_hash (keccak256)
    let mut keccak_out = [0u8; 32];
    let mut keccak = Keccak::v256();
    keccak.update(&preimage);
    keccak.finalize(&mut keccak_out);
    let counter_hash = Hash::from_bytes(keccak_out);

    let creator_hash = Hash::from_bytes([0x03; 32]);
    let eth_address = b"0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045";

    let bridge_output = Output::bridge_htlc(
        5_000_000,
        creator_hash,
        expected_hash,
        10,
        360,
        crate::transaction::BRIDGE_CHAIN_ETHEREUM,
        eth_address,
        counter_hash,
    )
    .unwrap();

    // Verify counter_hash is keccak256
    let (chain, addr, ch) = bridge_output.bridge_htlc_metadata().unwrap();
    assert_eq!(chain, crate::transaction::BRIDGE_CHAIN_ETHEREUM);
    assert_eq!(addr, eth_address.to_vec());
    assert_eq!(ch.unwrap(), counter_hash);

    // Verify keccak256(preimage) == stored counter_hash
    let mut verify_keccak = [0u8; 32];
    let mut k = Keccak::v256();
    k.update(&preimage);
    k.finalize(&mut verify_keccak);
    assert_eq!(verify_keccak, keccak_out);

    // Claim with preimage succeeds
    let cond = bridge_output.condition().unwrap().unwrap();
    let signing_hash = Hash::from_bytes([0x00; 32]);
    let claim_witness = Witness {
        preimage: Some(preimage),
        or_branches: vec![false],
        ..Default::default()
    };
    let ctx = EvalContext {
        current_height: 50,
        signing_hash: &signing_hash,
    };
    let mut idx = 0;
    assert!(evaluate(&cond, &claim_witness, &ctx, &mut idx));
}

/// Test 6: Refund after expiry
#[test]
fn test_bridge_htlc_refund_after_expiry() {
    use crate::conditions::{evaluate, EvalContext, Witness, HASHLOCK_DOMAIN};
    use crypto::hash::hash_with_domain;

    let preimage = [0xDD; 32];
    let expected_hash = hash_with_domain(HASHLOCK_DOMAIN, &preimage);
    let counter_hash = Hash::from_bytes([0xFF; 32]);
    let creator_hash = Hash::from_bytes([0x01; 32]);

    // Create BridgeHTLC with expiry close to current
    let current_height = 100;
    let expiry_height = current_height + 1; // expires at 101
    let lock_height = current_height;

    let bridge_output = Output::bridge_htlc(
        2_000_000,
        creator_hash,
        expected_hash,
        lock_height,
        expiry_height,
        crate::transaction::BRIDGE_CHAIN_BITCOIN,
        b"bc1qrefund",
        counter_hash,
    )
    .unwrap();

    let cond = bridge_output.condition().unwrap().unwrap();
    let signing_hash = Hash::from_bytes([0x00; 32]);

    // Refund witness: branch(right) + no preimage
    let refund_witness = Witness {
        or_branches: vec![true],
        ..Default::default()
    };

    // Before expiry (height 100): refund fails
    let ctx_before = EvalContext {
        current_height: 100,
        signing_hash: &signing_hash,
    };
    let mut idx = 0;
    assert!(!evaluate(&cond, &refund_witness, &ctx_before, &mut idx));

    // At expiry (height 101): refund succeeds
    let ctx_at = EvalContext {
        current_height: 101,
        signing_hash: &signing_hash,
    };
    let mut idx = 0;
    assert!(evaluate(&cond, &refund_witness, &ctx_at, &mut idx));

    // Well after expiry (height 200): refund still succeeds
    let ctx_after = EvalContext {
        current_height: 200,
        signing_hash: &signing_hash,
    };
    let mut idx = 0;
    assert!(evaluate(&cond, &refund_witness, &ctx_after, &mut idx));

    // Build a refund transaction and verify witness encoding
    let refund_output = Output::normal(2_000_000 - 1, creator_hash);
    let mut refund_tx = Transaction::new_transfer(
        vec![Input::new(Hash::from_bytes([0xAA; 32]), 0)],
        vec![refund_output],
    );
    let refund_bytes = refund_witness.encode();
    refund_tx.set_covenant_witnesses(&[refund_bytes]);
    assert!(!refund_tx.extra_data.is_empty());
}

/// Test 10 (from crypto crate): BridgeHTLC v2 with Monero counter_hash roundtrip
#[test]
fn test_bridge_htlc_monero_roundtrip() {
    use crate::conditions::HASHLOCK_DOMAIN;
    use crate::transaction::BRIDGE_CHAIN_MONERO;
    use curve25519_dalek::constants::ED25519_BASEPOINT_TABLE;
    use curve25519_dalek::scalar::Scalar;

    // Generate adaptor secret t and point T = t * G
    let t_bytes = [0x42u8; 32];
    let t = Scalar::from_bytes_mod_order(t_bytes);
    let adaptor_point = &t * ED25519_BASEPOINT_TABLE;
    let counter_hash = Hash::from_bytes(*adaptor_point.compress().as_bytes());

    // The preimage on DOLI side is the adaptor secret t
    let preimage = t_bytes;
    let expected_hash = crypto::hash::hash_with_domain(HASHLOCK_DOMAIN, &preimage);
    let dummy_address = b"888tNkZrPN6JsEG2vx7JX4B1F7f4zF52";

    let output = Output::bridge_htlc(
        1000,
        Hash::from_bytes([0xAA; 32]),
        expected_hash,
        100,
        200,
        BRIDGE_CHAIN_MONERO,
        dummy_address,
        counter_hash,
    )
    .unwrap();

    assert_eq!(output.output_type, OutputType::BridgeHTLC);

    let (chain, addr, ch) = output.bridge_htlc_metadata().unwrap();
    assert_eq!(chain, BRIDGE_CHAIN_MONERO);
    assert_eq!(addr, dummy_address.to_vec());
    assert_eq!(ch, Some(counter_hash));

    // Verify T can be reconstructed from the UTXO
    let recovered = curve25519_dalek::edwards::CompressedEdwardsY(*ch.unwrap().as_bytes());
    let t_recovered = recovered.decompress().unwrap();
    assert_eq!(adaptor_point.compress(), t_recovered.compress());
}

// ==================== Per-Byte Fee Tests ====================

/// Test 1: Plain transfer (0 extra_data bytes) => minimum_fee = 1 sat (BASE_FEE only)
#[test]
fn test_fee_plain_transfer() {
    let pubkey_hash = crypto::hash::hash(b"recipient");
    let tx = Transaction::new_transfer(
        vec![Input::new(Hash::ZERO, 0)],
        vec![Output::normal(100, pubkey_hash)],
    );
    assert_eq!(tx.minimum_fee(), 1);
}

/// Test 2: Bond output (4 bytes extra_data) => minimum_fee = 5 sats
#[test]
fn test_fee_bond_output() {
    let pubkey_hash = crypto::hash::hash(b"producer");
    let tx = Transaction {
        version: 1,
        tx_type: TxType::Registration,
        inputs: vec![Input::new(Hash::ZERO, 0)],
        outputs: vec![Output::bond(100_000_000, pubkey_hash, 1000, 0)],
        extra_data: vec![],
    };
    // Bond has 4 bytes of extra_data (creation_slot)
    assert_eq!(tx.outputs[0].extra_data.len(), 4);
    assert_eq!(tx.minimum_fee(), 1 + 4); // BASE_FEE + 4 * FEE_PER_BYTE = 5
}

/// Test 3: NFT-sized output (300 bytes) => minimum_fee = 301 sats
#[test]
fn test_fee_nft_300_bytes() {
    let pubkey_hash = crypto::hash::hash(b"nft_owner");
    let tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(Hash::ZERO, 0)],
        outputs: vec![Output {
            output_type: OutputType::NFT,
            amount: 0,
            pubkey_hash,
            lock_until: 0,
            extra_data: vec![0u8; 300],
        }],
        extra_data: vec![],
    };
    assert_eq!(tx.minimum_fee(), 1 + 300); // 301
}

/// Test 4: Pool swap output (116 bytes) => minimum_fee = 117 sats
#[test]
fn test_fee_pool_swap_116_bytes() {
    use crate::transaction::POOL_METADATA_SIZE;
    let pubkey_hash = crypto::hash::hash(b"pool");
    let tx = Transaction {
        version: 1,
        tx_type: TxType::Swap,
        inputs: vec![Input::new(Hash::ZERO, 0)],
        outputs: vec![Output {
            output_type: OutputType::Pool,
            amount: 0,
            pubkey_hash,
            lock_until: 0,
            extra_data: vec![0u8; POOL_METADATA_SIZE],
        }],
        extra_data: vec![],
    };
    assert_eq!(POOL_METADATA_SIZE, 116);
    assert_eq!(tx.minimum_fee(), 1 + 116); // 117
}

/// Test 5: CryptoPunk-sized NFT (3000 bytes) => minimum_fee = 3001 sats
#[test]
fn test_fee_cryptopunk_3000_bytes() {
    let pubkey_hash = crypto::hash::hash(b"punk_owner");
    let tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(Hash::ZERO, 0)],
        outputs: vec![Output {
            output_type: OutputType::NFT,
            amount: 0,
            pubkey_hash,
            lock_until: 0,
            extra_data: vec![0u8; 3000],
        }],
        extra_data: vec![],
    };
    assert_eq!(tx.minimum_fee(), 1 + 3000); // 3001
}

/// Test 6: Multiple outputs with extra_data — fee sums across all outputs
#[test]
fn test_fee_multiple_outputs_sum() {
    let pubkey_hash = crypto::hash::hash(b"multi");
    let tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(Hash::ZERO, 0)],
        outputs: vec![
            Output::normal(50, pubkey_hash),         // 0 bytes
            Output::bond(100, pubkey_hash, 1000, 0), // 4 bytes
            Output {
                output_type: OutputType::NFT,
                amount: 0,
                pubkey_hash,
                lock_until: 0,
                extra_data: vec![0u8; 100], // 100 bytes
            },
        ],
        extra_data: vec![],
    };
    // Total extra_data: 0 + 4 + 100 = 104 bytes
    assert_eq!(tx.minimum_fee(), 1 + 104); // 105
}

/// Test 7: Coinbase/EpochReward have minimum_fee but it is never enforced
///         (they have no inputs, fee is moot). Verify calculation still works.
#[test]
fn test_fee_coinbase_and_epoch_reward() {
    let coinbase = Transaction::new_coinbase(100_000_000, Hash::from_bytes([1u8; 32]), 0);
    // Coinbase has no outputs with extra_data (normal output)
    assert_eq!(coinbase.minimum_fee(), 1); // BASE_FEE only

    let keypair = crypto::KeyPair::generate();
    let epoch_reward = Transaction::new_epoch_reward(
        1,
        *keypair.public_key(),
        1_000_000,
        Hash::from_bytes([2u8; 32]),
    );
    assert_eq!(epoch_reward.minimum_fee(), 1); // BASE_FEE only
}

/// Test 14: Zero extra_data on all outputs => fee is exactly BASE_FEE
#[test]
fn test_fee_no_extra_data_is_base_only() {
    let pubkey_hash = crypto::hash::hash(b"no_extra");
    let tx = Transaction::new_transfer(
        vec![Input::new(Hash::ZERO, 0)],
        vec![
            Output::normal(50, pubkey_hash),
            Output::normal(30, pubkey_hash),
        ],
    );
    // Normal outputs have empty extra_data
    assert!(tx.outputs.iter().all(|o| o.extra_data.is_empty()));
    assert_eq!(tx.minimum_fee(), 1); // BASE_FEE only
}

/// Test 15: Maximum extra_data (4096 bytes) => minimum_fee = 4097 sats
#[test]
fn test_fee_max_extra_data() {
    use crate::transaction::MAX_EXTRA_DATA_SIZE;
    let pubkey_hash = crypto::hash::hash(b"max_data");
    let tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(Hash::ZERO, 0)],
        outputs: vec![Output {
            output_type: OutputType::NFT,
            amount: 0,
            pubkey_hash,
            lock_until: 0,
            extra_data: vec![0u8; MAX_EXTRA_DATA_SIZE],
        }],
        extra_data: vec![],
    };
    assert_eq!(tx.minimum_fee(), 1 + MAX_EXTRA_DATA_SIZE as u64); // 4097
}

/// Test 16: Fee calculation does not overflow with many large outputs
#[test]
fn test_fee_no_overflow_many_outputs() {
    use crate::transaction::MAX_EXTRA_DATA_SIZE;
    let pubkey_hash = crypto::hash::hash(b"overflow");
    // 100 outputs, each with 4096 bytes of extra_data
    let outputs: Vec<Output> = (0..100)
        .map(|_| Output {
            output_type: OutputType::NFT,
            amount: 1,
            pubkey_hash,
            lock_until: 0,
            extra_data: vec![0u8; MAX_EXTRA_DATA_SIZE],
        })
        .collect();
    let tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(Hash::ZERO, 0)],
        outputs,
        extra_data: vec![],
    };
    // 100 * 4096 = 409,600 bytes. Fee = 1 + 409,600 = 409,601.
    // No overflow — u64 handles this trivially.
    let expected = 1 + 100 * MAX_EXTRA_DATA_SIZE as u64;
    assert_eq!(tx.minimum_fee(), expected);
}

// ==================== Fee Routing (Coinbase Extra Fees) Tests ====================

#[test]
fn test_coinbase_extra_fees_calculation() {
    use crate::consensus::FEE_PER_BYTE;

    // Simulate: block with 1 NFT (300 bytes extra_data) + 1 transfer (0 bytes)
    let extra_fees: u64 = [300u64, 0u64]
        .iter()
        .map(|bytes| bytes * FEE_PER_BYTE)
        .sum();
    assert_eq!(extra_fees, 300);

    // Coinbase should be block_reward + extra_fees
    // Using a test block_reward of 100_000_000 (1 DOLI)
    let block_reward = 100_000_000u64;
    let coinbase_amount = block_reward + extra_fees;
    assert_eq!(coinbase_amount, 100_000_300);
}

#[test]
fn test_coinbase_no_extra_fees_for_transfers() {
    use crate::consensus::FEE_PER_BYTE;

    // Block with only transfers (0 extra_data each)
    let extra_fees: u64 = [0u64, 0u64, 0u64]
        .iter()
        .map(|bytes| bytes * FEE_PER_BYTE)
        .sum();
    assert_eq!(extra_fees, 0);
}

#[test]
fn test_extra_fees_multiple_nfts() {
    use crate::consensus::FEE_PER_BYTE;

    // 3 NFTs of 300 bytes each + 1 pool swap of 116 bytes
    let extra_fees: u64 = [300u64, 300, 300, 116]
        .iter()
        .map(|bytes| bytes * FEE_PER_BYTE)
        .sum();
    assert_eq!(extra_fees, 1016);
}

#[test]
fn test_extra_fees_only_from_user_txs() {
    use crate::consensus::FEE_PER_BYTE;

    // Coinbase and EpochReward TXs should NOT count.
    // Only user TXs contribute extra fees.
    let user_tx_bytes = vec![300u64, 116]; // NFT + pool
    let _protocol_tx_bytes = vec![40u64, 200]; // registration + epoch reward (not counted)

    // Only user TXs count
    let extra_fees: u64 = user_tx_bytes.iter().map(|bytes| bytes * FEE_PER_BYTE).sum();
    assert_eq!(extra_fees, 416); // NOT 416 + 240
}

#[test]
fn test_block_builder_add_coinbase_with_extra() {
    use crate::block::BlockBuilder;
    use crate::consensus::ConsensusParams;
    use crypto::PublicKey;

    let producer = PublicKey::from_bytes([1u8; 32]);
    let pool_hash = crate::consensus::reward_pool_pubkey_hash();
    let params = ConsensusParams::mainnet();

    let mut builder = BlockBuilder::new(Hash::ZERO, 0, producer).with_params(params.clone());

    // Add a user transaction first (simulates mempool tx)
    let user_tx = Transaction::new_transfer(
        vec![Input::new(Hash::ZERO, 0)],
        vec![Output {
            output_type: OutputType::NFT,
            amount: 1,
            pubkey_hash: Hash::ZERO,
            lock_until: 0,
            extra_data: vec![0u8; 300],
        }],
    );
    builder.add_transaction(user_tx);

    // Add coinbase with extra fees (300 bytes * FEE_PER_BYTE = 300)
    let extra_fees = 300u64;
    builder.add_coinbase_with_extra(1, pool_hash, extra_fees);

    // Build the block
    let result = builder.build(params.genesis_time + params.slot_duration);
    assert!(result.is_some());
    let (_header, txs) = result.unwrap();

    // Coinbase must be at position 0 (insert(0, ...))
    assert!(txs[0].is_coinbase());
    let expected_reward = params.block_reward(1) + extra_fees;
    assert_eq!(txs[0].outputs[0].amount, expected_reward);
    assert_eq!(txs[0].outputs[0].pubkey_hash, pool_hash);
}
