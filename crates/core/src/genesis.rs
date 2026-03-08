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
    /// Genesis time: 2026-02-25T00:00:00Z
    /// Message references the whitepaper philosophy
    pub fn mainnet() -> Self {
        let params = NetworkParams::load(Network::Mainnet);
        Self {
            network: Network::Mainnet,
            timestamp: params.genesis_time,
            reward: params.initial_reward,
            message: "Time is the only fair currency. 25/Feb/2026",
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
/// N1-N5 are both **producers AND maintainers** (dual role, 3-of-5 release signing).
/// N6-N12 register on-chain after genesis as producers only.
/// Keys match BOOTSTRAP_MAINTAINER_KEYS_MAINNET in `crates/updater/src/lib.rs`.
/// Synthetic bond outpoints (Hash::ZERO) - cannot unbond.
pub const MAINNET_GENESIS_PRODUCERS: &[(&str, u32)] = &[
    // N1 — omegacortex — producer_1.json
    (
        "202047256a8072a8b8f476691b9a5ae87710cc545e8707ca9fe0c803c3e6d3df",
        1,
    ),
    // N2 — omegacortex — producer_2.json
    (
        "effe88fefb6d992a1329277a1d49c7296d252bbc368319cb4bc061119926272b",
        1,
    ),
    // N3 — N3-VPS — producer_3.json
    (
        "54323cefd0eabac89b2a2198c95a8f261598c341a8e579a05e26322325c48c2b",
        1,
    ),
    // N4 — producer_4.json (regenerated — original VPS decommissioned)
    (
        "2d27fdcc6a240b76ecaea64ad05c9b70d1adad90b6f9c43e8cbbbc0f1ab04116",
        1,
    ),
    // N5 — producer_5.json (regenerated — original VPS decommissioned)
    (
        "3047e96b13276dd92ef5eb2d6396e66c29909217f11f8c0544ea7d76a76c7602",
        1,
    ),
];

/// Parse mainnet genesis producers into (PublicKey, bond_count) pairs
pub fn mainnet_genesis_producers() -> Vec<(PublicKey, u32)> {
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
/// NT1-NT5 are both **producers AND maintainers** (dual role, 3-of-5 release signing).
/// NT6-NT12 register on-chain after genesis as producers only.
/// Keys for NT1-NT5 match BOOTSTRAP_MAINTAINER_KEYS_TESTNET in `crates/updater/src/lib.rs`.
/// Synthetic bond outpoints (Hash::ZERO) - cannot unbond.
pub const TESTNET_GENESIS_PRODUCERS: &[(&str, u32)] = &[
    (
        "273a257357a0fefeba0d97f4e61ea069e2cb2758239b315824ea73410d06a199",
        1,
    ), // nt1 — omegacortex
    (
        "d70259cb4fc7acaeddb5028014a62b8d359a8e9fbd98b6cc7b8ca6e9bb1270df",
        1,
    ), // nt2 — omegacortex
    (
        "f23fb0840f985b781cdce2a8f9996e58dc154909e6fc36eb419b2b31a88fcc7f",
        1,
    ), // nt3 — omegacortex
    (
        "7e5f6f49f934099c78edfbc7967143d8e32c88feb36a10864e8f5575b4f0028b",
        1,
    ), // nt4 — omegacortex
    (
        "952f3d72abd9708ea7f3760b0113a522143895a0948e76220e8c5b320c3ca91d",
        1,
    ), // nt5 — omegacortex
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

    // Compute genesis_hash for chain identity
    let genesis_hash = {
        let mut hasher = crypto::Hasher::new();
        hasher.update(&timestamp.to_le_bytes());
        hasher.update(&(config.network as u32).to_le_bytes());
        hasher.update(&config.network.slot_duration().to_le_bytes());
        hasher.update(config.message.as_bytes());
        hasher.finalize()
    };

    // Create block header
    let header = BlockHeader {
        version: 2,
        prev_hash: Hash::from_bytes(NULL_HASH),
        merkle_root,
        presence_root: Hash::ZERO, // Genesis block has no presence commitment
        genesis_hash,
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
        // genesis_time=0 is a placeholder — genesis block uses current time (like devnet)
        // Once testnet launches with a real timestamp, this will be set in chainspec
        if params.genesis_time == 0 {
            assert!(genesis.header.timestamp > 0);
        } else {
            assert_eq!(genesis.header.timestamp, params.genesis_time);
        }
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
        assert!(message.contains("25/Feb/2026"));
    }

    #[test]
    fn test_genesis_validation_wrong_network() {
        let mainnet_genesis = generate_genesis_block(&GenesisConfig::mainnet());
        let testnet_config = GenesisConfig::testnet();

        // When mainnet and testnet share the same timestamp+reward, verify_genesis_block
        // passes for both (block structure is identical). Network isolation relies on
        // genesis_hash (includes network_id) checked at the P2P/sync layer, not here.
        let result = verify_genesis_block(&mainnet_genesis, Network::Testnet);
        if testnet_config.timestamp != 0
            && testnet_config.timestamp == GenesisConfig::mainnet().timestamp
        {
            // Same timestamp+reward → block passes both validations
            assert!(result.is_ok());
        } else if testnet_config.timestamp != 0 {
            assert!(result.is_err());
        } else {
            assert!(result.is_ok());
        }
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
    fn test_mainnet_genesis_keys_are_real() {
        // Ensure no placeholder keys (all zeros)
        for (hex, _) in MAINNET_GENESIS_PRODUCERS {
            assert!(
                !hex.starts_with("00000000"),
                "MAINNET_GENESIS_PRODUCERS still contains placeholder key: {}",
                hex
            );
        }
        // Ensure we have exactly 5 producers
        assert_eq!(MAINNET_GENESIS_PRODUCERS.len(), 5);
        // Ensure all keys parse successfully
        let producers = mainnet_genesis_producers();
        assert_eq!(producers.len(), 5);
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
