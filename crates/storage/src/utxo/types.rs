use crypto::Hash;
use doli_core::network::Network;
use doli_core::network_params::NetworkParams;
use doli_core::transaction::{Output, OutputType};
use doli_core::types::BlockHeight;
use serde::{Deserialize, Serialize};

/// Unique ID index prefixes — one byte type tag + 32-byte ID
pub const UID_PREFIX_NFT: u8 = 0x01;
pub const UID_PREFIX_ASSET: u8 = 0x02;
pub const UID_PREFIX_POOL: u8 = 0x03;
pub const UID_PREFIX_CHANNEL: u8 = 0x04;

/// Build a 33-byte unique index key from prefix + hash
pub fn uid_key(prefix: u8, id: &Hash) -> [u8; 33] {
    let mut key = [0u8; 33];
    key[0] = prefix;
    key[1..33].copy_from_slice(id.as_bytes());
    key
}

/// An entry in the UTXO set
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UtxoEntry {
    /// The output
    pub output: doli_core::transaction::Output,
    /// Block height when created
    pub height: BlockHeight,
    /// Whether this is a coinbase output
    pub is_coinbase: bool,
    /// Whether this is an epoch reward output
    #[serde(default)] // For backward compatibility with existing UTXOs
    pub is_epoch_reward: bool,
}

/// Default reward maturity constant (mainnet default: 100 blocks)
///
/// **Deprecated**: Use `reward_maturity_for_network(network)` for network-aware calculations.
/// Devnet uses 10 blocks for faster testing.
#[deprecated(note = "Use reward_maturity_for_network(network) for network-aware calculations")]
pub const DEFAULT_REWARD_MATURITY: BlockHeight = 6;

/// Get reward maturity for a specific network
pub fn reward_maturity_for_network(network: Network) -> BlockHeight {
    NetworkParams::load(network).coinbase_maturity
}

impl UtxoEntry {
    /// Canonical serialization for state root computation.
    ///
    /// Fixed-field encoding immune to bincode struct evolution.
    /// Format: `[1B output_type][8B amount][32B pubkey_hash][8B lock_until]
    ///          [8B height][1B is_coinbase][1B is_epoch_reward]
    ///          [2B extra_data_len (u16 LE)][NB extra_data]`
    ///
    /// Base size: 61 bytes (59 + 2 for length=0 when extra_data is empty).
    pub fn serialize_canonical_bytes(&self) -> Vec<u8> {
        let extra_len = self.output.extra_data.len();
        let mut buf = Vec::with_capacity(61 + extra_len);
        buf.push(self.output.output_type as u8);
        buf.extend_from_slice(&self.output.amount.to_le_bytes());
        buf.extend_from_slice(self.output.pubkey_hash.as_bytes());
        buf.extend_from_slice(&self.output.lock_until.to_le_bytes());
        buf.extend_from_slice(&self.height.to_le_bytes());
        buf.push(self.is_coinbase as u8);
        buf.push(self.is_epoch_reward as u8);
        buf.extend_from_slice(&(extra_len as u16).to_le_bytes());
        buf.extend_from_slice(&self.output.extra_data);
        buf
    }

    /// Reconstruct a `UtxoEntry` from canonical encoding.
    ///
    /// Returns `None` if the bytes are too short or the output type is unknown.
    pub fn deserialize_canonical_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 61 {
            return None;
        }
        let output_type = OutputType::from_u8(bytes[0])?;
        let amount = u64::from_le_bytes(bytes[1..9].try_into().ok()?);
        let pubkey_hash = Hash::from_bytes(bytes[9..41].try_into().ok()?);
        let lock_until = u64::from_le_bytes(bytes[41..49].try_into().ok()?);
        let height = u64::from_le_bytes(bytes[49..57].try_into().ok()?);
        let is_coinbase = bytes[57] != 0;
        let is_epoch_reward = bytes[58] != 0;
        let extra_len = u16::from_le_bytes(bytes[59..61].try_into().ok()?) as usize;
        let extra_data = if extra_len > 0 {
            if bytes.len() < 61 + extra_len {
                return None;
            }
            bytes[61..61 + extra_len].to_vec()
        } else {
            Vec::new()
        };
        Some(UtxoEntry {
            output: Output {
                output_type,
                amount,
                pubkey_hash,
                lock_until,
                extra_data,
            },
            height,
            is_coinbase,
            is_epoch_reward,
        })
    }

    /// Size of this entry's canonical serialization in bytes.
    pub fn canonical_byte_size(&self) -> usize {
        61 + self.output.extra_data.len()
    }

    /// Check if the UTXO is spendable at the given height for a specific network
    pub fn is_spendable_at_for_network(&self, height: BlockHeight, network: Network) -> bool {
        self.is_spendable_at_with_maturity(height, reward_maturity_for_network(network))
    }

    /// Check if the UTXO is spendable at the given height with mainnet default maturity (100 blocks)
    ///
    /// **Deprecated**: Use `is_spendable_at_for_network()` for network-aware calculations.
    #[deprecated(
        note = "Use is_spendable_at_for_network(height, network) for network-aware calculations"
    )]
    pub fn is_spendable_at(&self, height: BlockHeight) -> bool {
        #[allow(deprecated)]
        self.is_spendable_at_with_maturity(height, DEFAULT_REWARD_MATURITY)
    }

    /// Check if the UTXO is spendable at the given height with custom maturity
    pub fn is_spendable_at_with_maturity(
        &self,
        height: BlockHeight,
        maturity: BlockHeight,
    ) -> bool {
        // Check time lock
        if !self.output.is_spendable_at(height) {
            return false;
        }

        // Coinbase AND EpochReward require maturity confirmations
        if self.is_coinbase || self.is_epoch_reward {
            let confirmations = height.saturating_sub(self.height);
            return confirmations >= maturity;
        }

        true
    }
}

/// Outpoint identifier
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Outpoint {
    pub tx_hash: Hash,
    pub index: u32,
}

impl Outpoint {
    pub fn new(tx_hash: Hash, index: u32) -> Self {
        Self { tx_hash, index }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(36);
        bytes.extend_from_slice(self.tx_hash.as_bytes());
        bytes.extend_from_slice(&self.index.to_le_bytes());
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 36 {
            return None;
        }
        let mut hash_arr = [0u8; 32];
        hash_arr.copy_from_slice(&bytes[0..32]);
        let index = u32::from_le_bytes(bytes[32..36].try_into().ok()?);
        Some(Self {
            tx_hash: Hash::from_bytes(hash_arr),
            index,
        })
    }
}
