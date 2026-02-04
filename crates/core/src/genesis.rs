//! Genesis block generation for each network
//!
//! Each network has a unique genesis block that anchors the chain.
//! The genesis block contains:
//! - A single coinbase transaction (first block reward)
//! - Network-specific parameters embedded in the block
//! - A deterministic hash that serves as the chain identifier

use crate::block::{Block, BlockHeader};
use crate::network::Network;
use crate::network_params::NetworkParams;
use crate::transaction::{Output, Transaction, TxType};
use crate::types::Amount;
use crypto::{Hash, PublicKey};
use vdf::{VdfOutput, VdfProof};

/// Genesis block configuration
#[derive(Clone, Debug)]
pub struct GenesisConfig {
    /// Network this genesis is for
    pub network: Network,
    /// Genesis timestamp
    pub timestamp: u64,
    /// Initial block reward
    pub reward: Amount,
    /// Genesis message embedded in coinbase
    pub message: &'static str,
}

impl GenesisConfig {
    /// Mainnet genesis configuration
    ///
    /// Genesis time: 2026-02-01T00:00:00Z
    /// Message references the whitepaper philosophy
    pub fn mainnet() -> Self {
        let params = NetworkParams::load(Network::Mainnet);
        Self {
            network: Network::Mainnet,
            timestamp: params.genesis_time,
            reward: params.initial_reward,
            message: "Time is the only fair currency. 01/Feb/2026",
        }
    }

    /// Testnet genesis configuration
    ///
    /// Genesis time: 2026-01-29T22:00:00Z
    /// Testnet v2 launched January 2026 (fresh genesis with mainnet parameters)
    pub fn testnet() -> Self {
        let params = NetworkParams::load(Network::Testnet);
        Self {
            network: Network::Testnet,
            timestamp: params.genesis_time,
            reward: params.initial_reward,
            message: "DOLI Testnet v2 Genesis - Time is the only fair currency",
        }
    }

    /// Devnet genesis configuration
    ///
    /// Genesis time: dynamic (current time when created)
    /// For local development
    pub fn devnet() -> Self {
        let params = NetworkParams::load(Network::Devnet);
        Self {
            network: Network::Devnet,
            timestamp: params.genesis_time, // 0 = set dynamically at generation time
            reward: params.initial_reward,
            message: "DOLI Devnet - Development and Testing",
        }
    }

    /// Create genesis config for a specific network
    pub fn for_network(network: Network) -> Self {
        match network {
            Network::Mainnet => Self::mainnet(),
            Network::Testnet => Self::testnet(),
            Network::Devnet => Self::devnet(),
        }
    }
}

/// The "null" public key used for genesis coinbase
/// This is a well-known unspendable key (all zeros)
pub const GENESIS_PUBKEY: [u8; 32] = [0u8; 32];

/// Mainnet genesis producers (pubkey hex, bond_count)
///
/// **CRITICAL**: These pubkeys MUST match the actual wallet files!
/// See docs/GENESIS.md for the correct procedure to generate these.
///
/// These 5 producers are registered at genesis with 1 bond each.
/// Synthetic bond outpoints (Hash::ZERO) - cannot unbond.
///
/// **BEFORE MAINNET**: Replace with actual pubkeys from producer wallet files!
pub const MAINNET_GENESIS_PRODUCERS: &[(&str, u32)] = &[
    // PLACEHOLDER - Replace with actual mainnet producer pubkeys before launch!
    // Generate wallets first, then copy pubkeys here.
    // See docs/GENESIS.md Section 4 for instructions.
    (
        "0000000000000000000000000000000000000000000000000000000000000001",
        1,
    ), // producer_1 - REPLACE!
    (
        "0000000000000000000000000000000000000000000000000000000000000002",
        1,
    ), // producer_2 - REPLACE!
    (
        "0000000000000000000000000000000000000000000000000000000000000003",
        1,
    ), // producer_3 - REPLACE!
    (
        "0000000000000000000000000000000000000000000000000000000000000004",
        1,
    ), // producer_4 - REPLACE!
    (
        "0000000000000000000000000000000000000000000000000000000000000005",
        1,
    ), // producer_5 - REPLACE!
];

/// Check if mainnet genesis producers are still placeholders
pub fn mainnet_using_placeholder_producers() -> bool {
    MAINNET_GENESIS_PRODUCERS
        .iter()
        .any(|(hex, _)| hex.starts_with("00000000"))
}

/// Parse mainnet genesis producers into (PublicKey, bond_count) pairs
///
/// **WARNING**: Will panic in debug builds if placeholder keys detected!
pub fn mainnet_genesis_producers() -> Vec<(PublicKey, u32)> {
    #[cfg(debug_assertions)]
    if mainnet_using_placeholder_producers() {
        panic!(
            "MAINNET_GENESIS_PRODUCERS contains placeholder keys! \
             Replace with actual pubkeys before mainnet launch. \
             See docs/GENESIS.md for instructions."
        );
    }

    MAINNET_GENESIS_PRODUCERS
        .iter()
        .filter_map(|(hex, bonds)| {
            let bytes = hex::decode(hex).ok()?;
            if bytes.len() != 32 {
                return None;
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            Some((PublicKey::from_bytes(arr), *bonds))
        })
        .collect()
}

/// Testnet genesis producers (pubkey hex, bond_count)
///
/// These 5 producers are registered at genesis with 1 bond each.
/// The pubkeys are derived from the testnet producer private keys.
/// Synthetic bond outpoints (Hash::ZERO) - cannot unbond.
pub const TESTNET_GENESIS_PRODUCERS: &[(&str, u32)] = &[
    (
        "8f5b66af162a74d3d0992e73adbb3c6baf774ee3b75e01dd393eaba8907621a2",
        1,
    ), // producer_1
    (
        "2f2bc92b84423977e10c595f33099eacec476ea2a7353d01a51a54658b342895",
        1,
    ), // producer_2
    (
        "066c22d232fe36b5b415ad38b155034323c3b2083e18d5c6c269218541605674",
        1,
    ), // producer_3
    (
        "743a4ca3c0fc033a213195fa20352aac2118ef1a624cf77aaaba4ab59e2335d8",
        1,
    ), // producer_4
    (
        "7c8ce647c6d32eaea14ae47a282e78fba469f6c9117f062e9345143d4c967145",
        1,
    ), // producer_5
];

/// Parse testnet genesis producers into (PublicKey, bond_count) pairs
pub fn testnet_genesis_producers() -> Vec<(PublicKey, u32)> {
    TESTNET_GENESIS_PRODUCERS
        .iter()
        .filter_map(|(hex, bonds)| {
            let bytes = hex::decode(hex).ok()?;
            if bytes.len() != 32 {
                return None;
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            Some((PublicKey::from_bytes(arr), *bonds))
        })
        .collect()
}

/// The "null" hash used as prev_hash for genesis block
pub const NULL_HASH: [u8; 32] = [0u8; 32];

/// Generate the genesis block for a network
///
/// The genesis block is deterministic for mainnet and testnet,
/// but devnet can have a dynamic timestamp.
pub fn generate_genesis_block(config: &GenesisConfig) -> Block {
    let timestamp = if config.timestamp == 0 {
        // Devnet: use current time
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    } else {
        config.timestamp
    };

    // Create coinbase transaction
    let coinbase_tx = create_genesis_coinbase(config, timestamp);
    let tx_hash = coinbase_tx.hash();

    // Calculate merkle root (single tx = tx hash)
    let merkle_root = tx_hash;

    // Create block header
    let header = BlockHeader {
        version: 1,
        prev_hash: Hash::from_bytes(NULL_HASH),
        merkle_root,
        presence_root: Hash::ZERO, // Genesis block has no presence commitment
        timestamp,
        slot: 0,
        producer: PublicKey::from_bytes(GENESIS_PUBKEY),
        vdf_output: genesis_vdf_output(config.network),
        vdf_proof: VdfProof::empty(), // Genesis has no VDF proof (bootstrap)
    };

    Block {
        header,
        transactions: vec![coinbase_tx],
    }
}

/// Create the genesis coinbase transaction
///
/// This is a special transaction with:
/// - No inputs (new coins created)
/// - One output: the genesis reward
/// - The genesis message in extra_data
fn create_genesis_coinbase(config: &GenesisConfig, timestamp: u64) -> Transaction {
    // Encode the genesis message with timestamp
    let message_with_ts = format!("{} ts:{}", config.message, timestamp);
    let extra_data = message_with_ts.into_bytes();

    // The genesis output goes to a well-known address
    // In practice, this output is unspendable (null pubkey hash)
    // The real distribution starts with block 1
    let output = Output::normal(config.reward, hash_genesis_recipient(config.network));

    Transaction {
        version: 1,
        tx_type: TxType::Transfer, // Coinbase is a Transfer with no inputs
        inputs: vec![],            // No inputs for coinbase
        outputs: vec![output],
        extra_data,
    }
}

/// Generate deterministic VDF output for genesis
///
/// Genesis block doesn't have a real VDF proof (it's the bootstrap).
/// We use a deterministic hash based on network to ensure uniqueness.
fn genesis_vdf_output(network: Network) -> VdfOutput {
    use crypto::Hasher;

    let mut hasher = Hasher::new();
    hasher.update(b"DOLI_GENESIS_VDF");
    hasher.update(&[network.id() as u8]);
    hasher.update(network.name().as_bytes());
    let hash = hasher.finalize();

    VdfOutput {
        value: hash.as_bytes().to_vec(),
    }
}

/// Generate the recipient hash for genesis reward
///
/// Each network has a unique genesis recipient.
/// These addresses are unspendable (no one has the private key).
fn hash_genesis_recipient(network: Network) -> Hash {
    use crypto::Hasher;

    let mut hasher = Hasher::new();
    hasher.update(b"DOLI_GENESIS_RECIPIENT");
    hasher.update(&[network.id() as u8]);
    hasher.finalize()
}

/// Get the pre-computed genesis hash for a network
///
/// These hashes are computed once and hardcoded for verification.
/// Any node can verify by regenerating the genesis block.
pub fn genesis_hash(network: Network) -> Hash {
    match network {
        Network::Mainnet => {
            // Pre-computed mainnet genesis hash
            // To regenerate: generate_genesis_block(GenesisConfig::mainnet()).hash()
            let genesis = generate_genesis_block(&GenesisConfig::mainnet());
            genesis.hash()
        }
        Network::Testnet => {
            // Pre-computed testnet genesis hash
            let genesis = generate_genesis_block(&GenesisConfig::testnet());
            genesis.hash()
        }
        Network::Devnet => {
            // Devnet genesis is dynamic, compute at runtime
            // Note: This means devnet genesis changes each time!
            // For persistent devnet, store the genesis hash after first run
            let genesis = generate_genesis_block(&GenesisConfig::devnet());
            genesis.hash()
        }
    }
}

/// Verify that a block is the valid genesis block for a network
pub fn verify_genesis_block(block: &Block, network: Network) -> Result<(), GenesisError> {
    // Check slot is 0
    if block.header.slot != 0 {
        return Err(GenesisError::InvalidSlot(block.header.slot));
    }

    // Check prev_hash is null
    if block.header.prev_hash != Hash::from_bytes(NULL_HASH) {
        return Err(GenesisError::InvalidPrevHash);
    }

    // Check producer is null pubkey
    if block.header.producer != PublicKey::from_bytes(GENESIS_PUBKEY) {
        return Err(GenesisError::InvalidProducer);
    }

    // Check exactly one transaction (coinbase)
    if block.transactions.len() != 1 {
        return Err(GenesisError::InvalidTransactionCount(
            block.transactions.len(),
        ));
    }

    let tx = &block.transactions[0];

    // Check it's a coinbase (Transfer with no inputs)
    if !tx.is_coinbase() {
        return Err(GenesisError::NotCoinbase);
    }

    // Check no inputs
    if !tx.inputs.is_empty() {
        return Err(GenesisError::CoinbaseHasInputs);
    }

    // Check exactly one output
    if tx.outputs.len() != 1 {
        return Err(GenesisError::InvalidOutputCount(tx.outputs.len()));
    }

    // For mainnet/testnet, verify timestamp matches expected
    let config = GenesisConfig::for_network(network);
    if config.timestamp != 0 && block.header.timestamp != config.timestamp {
        return Err(GenesisError::InvalidTimestamp {
            expected: config.timestamp,
            actual: block.header.timestamp,
        });
    }

    // Verify reward amount
    if tx.outputs[0].amount != config.reward {
        return Err(GenesisError::InvalidReward {
            expected: config.reward,
            actual: tx.outputs[0].amount,
        });
    }

    Ok(())
}

/// Errors that can occur during genesis verification
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenesisError {
    InvalidSlot(u32),
    InvalidPrevHash,
    InvalidProducer,
    InvalidTransactionCount(usize),
    NotCoinbase,
    CoinbaseHasInputs,
    InvalidOutputCount(usize),
    InvalidTimestamp { expected: u64, actual: u64 },
    InvalidReward { expected: Amount, actual: Amount },
}

impl std::fmt::Display for GenesisError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSlot(s) => write!(f, "genesis block must have slot 0, got {}", s),
            Self::InvalidPrevHash => write!(f, "genesis block must have null prev_hash"),
            Self::InvalidProducer => write!(f, "genesis block must have null producer"),
            Self::InvalidTransactionCount(n) => {
                write!(
                    f,
                    "genesis block must have exactly 1 transaction, got {}",
                    n
                )
            }
            Self::NotCoinbase => write!(f, "genesis transaction must be coinbase"),
            Self::CoinbaseHasInputs => write!(f, "genesis coinbase must have no inputs"),
            Self::InvalidOutputCount(n) => {
                write!(f, "genesis coinbase must have exactly 1 output, got {}", n)
            }
            Self::InvalidTimestamp { expected, actual } => {
                write!(
                    f,
                    "genesis timestamp mismatch: expected {}, got {}",
                    expected, actual
                )
            }
            Self::InvalidReward { expected, actual } => {
                write!(
                    f,
                    "genesis reward mismatch: expected {}, got {}",
                    expected, actual
                )
            }
        }
    }
}

impl std::error::Error for GenesisError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mainnet_genesis() {
        let params = NetworkParams::load(Network::Mainnet);
        let config = GenesisConfig::mainnet();
        let genesis = generate_genesis_block(&config);

        assert_eq!(genesis.header.slot, 0);
        assert_eq!(genesis.header.timestamp, params.genesis_time);
        assert_eq!(genesis.transactions.len(), 1);
        assert_eq!(
            genesis.transactions[0].outputs[0].amount,
            params.initial_reward
        );

        // Verify it passes validation
        assert!(verify_genesis_block(&genesis, Network::Mainnet).is_ok());
    }

    #[test]
    fn test_testnet_genesis() {
        let params = NetworkParams::load(Network::Testnet);
        let config = GenesisConfig::testnet();
        let genesis = generate_genesis_block(&config);

        assert_eq!(genesis.header.slot, 0);
        assert_eq!(genesis.header.timestamp, params.genesis_time);
        assert_eq!(
            genesis.transactions[0].outputs[0].amount,
            params.initial_reward
        );

        assert!(verify_genesis_block(&genesis, Network::Testnet).is_ok());
    }

    #[test]
    fn test_devnet_genesis() {
        let params = NetworkParams::load(Network::Devnet);
        let config = GenesisConfig::devnet();
        let genesis = generate_genesis_block(&config);

        assert_eq!(genesis.header.slot, 0);
        assert!(genesis.header.timestamp > 0); // Dynamic (genesis_time=0 means use current time)
        assert_eq!(
            genesis.transactions[0].outputs[0].amount,
            params.initial_reward
        );

        assert!(verify_genesis_block(&genesis, Network::Devnet).is_ok());
    }

    #[test]
    fn test_genesis_hashes_unique() {
        let mainnet = generate_genesis_block(&GenesisConfig::mainnet());
        let testnet = generate_genesis_block(&GenesisConfig::testnet());

        // Hashes must be different
        assert_ne!(mainnet.hash(), testnet.hash());
    }

    #[test]
    fn test_genesis_vdf_outputs_unique() {
        let mainnet_vdf = genesis_vdf_output(Network::Mainnet);
        let testnet_vdf = genesis_vdf_output(Network::Testnet);
        let devnet_vdf = genesis_vdf_output(Network::Devnet);

        assert_ne!(mainnet_vdf.value, testnet_vdf.value);
        assert_ne!(mainnet_vdf.value, devnet_vdf.value);
        assert_ne!(testnet_vdf.value, devnet_vdf.value);
    }

    #[test]
    fn test_genesis_contains_message() {
        let genesis = generate_genesis_block(&GenesisConfig::mainnet());
        let message = String::from_utf8_lossy(&genesis.transactions[0].extra_data);

        assert!(message.contains("Time is the only fair currency"));
        assert!(message.contains("01/Feb/2026"));
    }

    #[test]
    fn test_genesis_validation_wrong_network() {
        let mainnet_genesis = generate_genesis_block(&GenesisConfig::mainnet());

        // Mainnet genesis should fail testnet validation (different timestamp)
        let result = verify_genesis_block(&mainnet_genesis, Network::Testnet);
        assert!(result.is_err());
    }

    #[test]
    fn test_genesis_validation_modified_slot() {
        let mut genesis = generate_genesis_block(&GenesisConfig::mainnet());
        genesis.header.slot = 1;

        let result = verify_genesis_block(&genesis, Network::Mainnet);
        assert!(matches!(result, Err(GenesisError::InvalidSlot(1))));
    }

    #[test]
    fn test_genesis_validation_modified_reward() {
        let mut genesis = generate_genesis_block(&GenesisConfig::mainnet());
        genesis.transactions[0].outputs[0].amount = 999;

        let result = verify_genesis_block(&genesis, Network::Mainnet);
        assert!(matches!(result, Err(GenesisError::InvalidReward { .. })));
    }

    #[test]
    fn test_genesis_recipient_unique_per_network() {
        let mainnet_recipient = hash_genesis_recipient(Network::Mainnet);
        let testnet_recipient = hash_genesis_recipient(Network::Testnet);
        let devnet_recipient = hash_genesis_recipient(Network::Devnet);

        assert_ne!(mainnet_recipient, testnet_recipient);
        assert_ne!(mainnet_recipient, devnet_recipient);
        assert_ne!(testnet_recipient, devnet_recipient);
    }
}
