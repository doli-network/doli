use crypto::Hash;
use serde::{Deserialize, Serialize};

use crate::types::{Amount, BlockHeight};

use super::types::OutputType;

/// Maximum size of extra_data in an output (bytes).
/// Reserved for future contract types (scripts, conditions, metadata).
/// Normal and Bond outputs must have empty extra_data.
pub const MAX_EXTRA_DATA_SIZE: usize = 256;

/// NFT metadata version (without royalties)
pub const NFT_METADATA_VERSION: u8 = 1;
/// NFT metadata version with royalties
pub const NFT_METADATA_VERSION_ROYALTY: u8 = 2;
/// NFT metadata header size: 1B version + 32B token_id
pub const NFT_METADATA_HEADER_SIZE: usize = 33;
/// NFT royalty metadata: 32B creator_pubkey_hash + 2B royalty_bps (basis points, 0-10000)
pub const NFT_ROYALTY_SIZE: usize = 34;
/// Maximum royalty in basis points (50% = 5000 bps)
pub const MAX_ROYALTY_BPS: u16 = 5000;

/// Fungible asset metadata version
pub const FUNGIBLE_ASSET_VERSION: u8 = 1;
/// Fungible asset header size: 1B version + 32B asset_id + 8B total_supply + 1B ticker_len
pub const FUNGIBLE_ASSET_HEADER_SIZE: usize = 42;
/// Maximum ticker length
pub const MAX_TICKER_LEN: usize = 12;

/// Bridge HTLC metadata version v1 (no counter_hash)
pub const BRIDGE_HTLC_VERSION_V1: u8 = 1;
/// Bridge HTLC metadata version v2 (with counter_hash)
pub const BRIDGE_HTLC_VERSION_V2: u8 = 2;
/// Current version for newly created BridgeHTLC outputs
pub const BRIDGE_HTLC_CURRENT_VERSION: u8 = BRIDGE_HTLC_VERSION_V2;
/// Size of the counter_hash field in v2 BridgeHTLC metadata
pub const BRIDGE_HTLC_COUNTER_HASH_SIZE: usize = 32;
/// Bridge target chain identifiers
pub const BRIDGE_CHAIN_BITCOIN: u8 = 1;
pub const BRIDGE_CHAIN_ETHEREUM: u8 = 2;
pub const BRIDGE_CHAIN_MONERO: u8 = 3;
pub const BRIDGE_CHAIN_LITECOIN: u8 = 4;
pub const BRIDGE_CHAIN_CARDANO: u8 = 5;
/// Bridge HTLC header: 1B version + 1B target_chain + 1B addr_len
pub const BRIDGE_HTLC_HEADER_SIZE: usize = 3;

/// Transaction output
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    /// Type of output
    pub output_type: OutputType,
    /// Amount in base units
    pub amount: Amount,
    /// Hash of the recipient's public key
    pub pubkey_hash: Hash,
    /// Lock until height (0 for normal, >0 for bonds)
    pub lock_until: BlockHeight,
    /// Extensible data for future output types (empty for Normal/Bond).
    /// Interpretation depends on output_type. Max 256 bytes.
    #[serde(default)]
    pub extra_data: Vec<u8>,
}

impl Output {
    /// Create a normal output
    pub fn normal(amount: Amount, pubkey_hash: Hash) -> Self {
        Self {
            output_type: OutputType::Normal,
            amount,
            pubkey_hash,
            lock_until: 0,
            extra_data: Vec::new(),
        }
    }

    /// Create a bond output with creation_slot encoded in extra_data (4 bytes LE)
    pub fn bond(
        amount: Amount,
        pubkey_hash: Hash,
        lock_until: BlockHeight,
        creation_slot: u32,
    ) -> Self {
        Self {
            output_type: OutputType::Bond,
            amount,
            pubkey_hash,
            lock_until,
            extra_data: creation_slot.to_le_bytes().to_vec(),
        }
    }

    /// Extract creation_slot from a Bond output's extra_data (4 bytes LE)
    pub fn bond_creation_slot(&self) -> Option<u32> {
        if self.output_type == OutputType::Bond && self.extra_data.len() == 4 {
            Some(u32::from_le_bytes([
                self.extra_data[0],
                self.extra_data[1],
                self.extra_data[2],
                self.extra_data[3],
            ]))
        } else {
            None
        }
    }

    /// Create a conditioned output. The condition is encoded into extra_data.
    /// `pubkey_hash` is the primary recipient (for display/indexing purposes).
    pub fn conditioned(
        output_type: OutputType,
        amount: Amount,
        pubkey_hash: Hash,
        condition: &crate::conditions::Condition,
    ) -> Result<Self, crate::conditions::ConditionError> {
        let extra_data = condition.encode()?;
        Ok(Self {
            output_type,
            amount,
            pubkey_hash,
            lock_until: 0,
            extra_data,
        })
    }

    /// Create a multisig output.
    pub fn multisig(
        amount: Amount,
        primary_pubkey_hash: Hash,
        threshold: u8,
        keys: Vec<Hash>,
    ) -> Result<Self, crate::conditions::ConditionError> {
        let cond = crate::conditions::Condition::multisig(threshold, keys);
        Self::conditioned(OutputType::Multisig, amount, primary_pubkey_hash, &cond)
    }

    /// Create a hashlock output.
    pub fn hashlock(
        amount: Amount,
        pubkey_hash: Hash,
        expected_hash: Hash,
    ) -> Result<Self, crate::conditions::ConditionError> {
        let cond = crate::conditions::Condition::hashlock(expected_hash);
        Self::conditioned(OutputType::Hashlock, amount, pubkey_hash, &cond)
    }

    /// Create an HTLC output.
    pub fn htlc(
        amount: Amount,
        pubkey_hash: Hash,
        expected_hash: Hash,
        lock_height: BlockHeight,
        expiry_height: BlockHeight,
    ) -> Result<Self, crate::conditions::ConditionError> {
        let cond = crate::conditions::Condition::htlc(expected_hash, lock_height, expiry_height);
        Self::conditioned(OutputType::HTLC, amount, pubkey_hash, &cond)
    }

    /// Create a vesting output (signature + timelock).
    pub fn vesting(
        amount: Amount,
        pubkey_hash: Hash,
        unlock_height: BlockHeight,
    ) -> Result<Self, crate::conditions::ConditionError> {
        let cond = crate::conditions::Condition::vesting(pubkey_hash, unlock_height);
        Self::conditioned(OutputType::Vesting, amount, pubkey_hash, &cond)
    }

    /// Create an NFT output.
    ///
    /// `extra_data` layout: `[condition_bytes][1B version][32B token_id][content_hash/URI]`
    /// The condition controls who can transfer/burn the NFT.
    /// `token_id` is globally unique: BLAKE3("DOLI_NFT" || creator_pubkey_hash || creation_nonce).
    /// `amount` is 0 for pure NFTs, >0 for semi-fungible tokens carrying value.
    pub fn nft(
        amount: Amount,
        pubkey_hash: Hash,
        token_id: Hash,
        content_hash: &[u8],
        condition: &crate::conditions::Condition,
    ) -> Result<Self, crate::conditions::ConditionError> {
        let condition_bytes = condition.encode()?;
        let metadata_len = 1 + 32 + content_hash.len();
        if condition_bytes.len() + metadata_len > MAX_EXTRA_DATA_SIZE {
            return Err(crate::conditions::ConditionError::EncodingTooLarge {
                size: MAX_EXTRA_DATA_SIZE + 1,
            });
        }
        let mut extra_data = condition_bytes;
        extra_data.push(NFT_METADATA_VERSION);
        extra_data.extend_from_slice(token_id.as_bytes());
        extra_data.extend_from_slice(content_hash);
        Ok(Self {
            output_type: OutputType::NFT,
            amount,
            pubkey_hash,
            lock_until: 0,
            extra_data,
        })
    }

    /// Compute a deterministic NFT token ID.
    /// `token_id = BLAKE3("DOLI_NFT" || creator_pubkey_hash || nonce)`
    pub fn compute_nft_token_id(creator_pubkey_hash: &Hash, nonce: &[u8]) -> Hash {
        use crypto::hash::hash_with_domain;
        let mut data = Vec::with_capacity(32 + nonce.len());
        data.extend_from_slice(creator_pubkey_hash.as_bytes());
        data.extend_from_slice(nonce);
        hash_with_domain(b"DOLI_NFT", &data)
    }

    /// Extract NFT metadata from an NFT output's extra_data.
    /// Returns (condition_bytes, token_id, content_hash) or None if not an NFT.
    pub fn nft_metadata(&self) -> Option<(Hash, Vec<u8>)> {
        if self.output_type != OutputType::NFT || self.extra_data.is_empty() {
            return None;
        }
        // Decode condition prefix to find where metadata starts
        let cond_len = match crate::conditions::Condition::decode_prefix(&self.extra_data) {
            Ok((_, len)) => len,
            Err(_) => return None,
        };
        let meta = &self.extra_data[cond_len..];
        if meta.len() < NFT_METADATA_HEADER_SIZE {
            return None;
        }
        if meta[0] != NFT_METADATA_VERSION && meta[0] != NFT_METADATA_VERSION_ROYALTY {
            return None;
        }
        let token_id = Hash::from_bytes({
            let mut buf = [0u8; 32];
            buf.copy_from_slice(&meta[1..33]);
            buf
        });
        let rest = &meta[33..];
        // For v2 (royalty), strip royalty bytes from content_hash
        if meta[0] == NFT_METADATA_VERSION_ROYALTY && rest.len() >= NFT_ROYALTY_SIZE {
            let content_hash = rest[NFT_ROYALTY_SIZE..].to_vec();
            Some((token_id, content_hash))
        } else {
            let content_hash = rest.to_vec();
            Some((token_id, content_hash))
        }
    }

    /// Create an NFT output with royalty.
    ///
    /// `extra_data` layout: `[condition_bytes][1B version=2][32B token_id][32B creator_hash][2B royalty_bps][content_hash]`
    /// `royalty_bps` is in basis points (100 = 1%, 500 = 5%, max 5000 = 50%).
    /// The creator_hash and royalty_bps are immutable — they travel with the NFT forever.
    pub fn nft_with_royalty(
        amount: Amount,
        pubkey_hash: Hash,
        token_id: Hash,
        content_hash: &[u8],
        condition: &crate::conditions::Condition,
        creator_pubkey_hash: Hash,
        royalty_bps: u16,
    ) -> Result<Self, crate::conditions::ConditionError> {
        if royalty_bps > MAX_ROYALTY_BPS {
            return Err(crate::conditions::ConditionError::EncodingTooLarge {
                size: MAX_EXTRA_DATA_SIZE + 1,
            });
        }
        let condition_bytes = condition.encode()?;
        let metadata_len = 1 + 32 + NFT_ROYALTY_SIZE + content_hash.len();
        if condition_bytes.len() + metadata_len > MAX_EXTRA_DATA_SIZE {
            return Err(crate::conditions::ConditionError::EncodingTooLarge {
                size: MAX_EXTRA_DATA_SIZE + 1,
            });
        }
        let mut extra_data = condition_bytes;
        extra_data.push(NFT_METADATA_VERSION_ROYALTY);
        extra_data.extend_from_slice(token_id.as_bytes());
        extra_data.extend_from_slice(creator_pubkey_hash.as_bytes());
        extra_data.extend_from_slice(&royalty_bps.to_le_bytes());
        extra_data.extend_from_slice(content_hash);
        Ok(Self {
            output_type: OutputType::NFT,
            amount,
            pubkey_hash,
            lock_until: 0,
            extra_data,
        })
    }

    /// Extract royalty info from an NFT output.
    /// Returns `Some((creator_pubkey_hash, royalty_bps))` if this NFT has royalties.
    pub fn nft_royalty(&self) -> Option<(Hash, u16)> {
        if self.output_type != OutputType::NFT || self.extra_data.is_empty() {
            return None;
        }
        let cond_len = match crate::conditions::Condition::decode_prefix(&self.extra_data) {
            Ok((_, len)) => len,
            Err(_) => return None,
        };
        let meta = &self.extra_data[cond_len..];
        if meta.len() < NFT_METADATA_HEADER_SIZE + NFT_ROYALTY_SIZE {
            return None;
        }
        if meta[0] != NFT_METADATA_VERSION_ROYALTY {
            return None;
        }
        // After version(1) + token_id(32): creator_hash(32) + royalty_bps(2)
        let royalty_start = 33;
        let creator_hash = Hash::from_bytes({
            let mut buf = [0u8; 32];
            buf.copy_from_slice(&meta[royalty_start..royalty_start + 32]);
            buf
        });
        let bps = u16::from_le_bytes([meta[royalty_start + 32], meta[royalty_start + 33]]);
        Some((creator_hash, bps))
    }

    /// Create a fungible asset output.
    ///
    /// `extra_data` layout: `[condition_bytes][1B version][32B asset_id][8B total_supply LE][1B ticker_len][ticker]`
    /// `asset_id` is globally unique: BLAKE3("DOLI_ASSET" || genesis_tx_hash || output_index LE).
    /// `amount` = units of this asset held in this UTXO.
    /// `total_supply` = fixed at issuance (genesis output carries full supply).
    pub fn fungible_asset(
        amount: Amount,
        pubkey_hash: Hash,
        asset_id: Hash,
        total_supply: Amount,
        ticker: &str,
        condition: &crate::conditions::Condition,
    ) -> Result<Self, crate::conditions::ConditionError> {
        if ticker.len() > MAX_TICKER_LEN || ticker.is_empty() {
            return Err(crate::conditions::ConditionError::EncodingTooLarge {
                size: MAX_EXTRA_DATA_SIZE + 1,
            });
        }
        let condition_bytes = condition.encode()?;
        let metadata_len = 1 + 32 + 8 + 1 + ticker.len();
        if condition_bytes.len() + metadata_len > MAX_EXTRA_DATA_SIZE {
            return Err(crate::conditions::ConditionError::EncodingTooLarge {
                size: MAX_EXTRA_DATA_SIZE + 1,
            });
        }
        let mut extra_data = condition_bytes;
        extra_data.push(FUNGIBLE_ASSET_VERSION);
        extra_data.extend_from_slice(asset_id.as_bytes());
        extra_data.extend_from_slice(&total_supply.to_le_bytes());
        extra_data.push(ticker.len() as u8);
        extra_data.extend_from_slice(ticker.as_bytes());
        Ok(Self {
            output_type: OutputType::FungibleAsset,
            amount,
            pubkey_hash,
            lock_until: 0,
            extra_data,
        })
    }

    /// Compute a deterministic fungible asset ID.
    /// `asset_id = BLAKE3("DOLI_ASSET" || genesis_tx_hash || output_index LE)`
    pub fn compute_asset_id(genesis_tx_hash: &Hash, output_index: u32) -> Hash {
        use crypto::hash::hash_with_domain;
        let mut data = Vec::with_capacity(36);
        data.extend_from_slice(genesis_tx_hash.as_bytes());
        data.extend_from_slice(&output_index.to_le_bytes());
        hash_with_domain(b"DOLI_ASSET", &data)
    }

    /// Extract fungible asset metadata from extra_data.
    /// Returns (asset_id, total_supply, ticker) or None.
    pub fn fungible_asset_metadata(&self) -> Option<(Hash, Amount, String)> {
        if self.output_type != OutputType::FungibleAsset || self.extra_data.is_empty() {
            return None;
        }
        let cond_len = match crate::conditions::Condition::decode_prefix(&self.extra_data) {
            Ok((_, len)) => len,
            Err(_) => return None,
        };
        let meta = &self.extra_data[cond_len..];
        if meta.len() < FUNGIBLE_ASSET_HEADER_SIZE {
            return None;
        }
        if meta[0] != FUNGIBLE_ASSET_VERSION {
            return None;
        }
        let asset_id = Hash::from_bytes({
            let mut buf = [0u8; 32];
            buf.copy_from_slice(&meta[1..33]);
            buf
        });
        let total_supply = u64::from_le_bytes({
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&meta[33..41]);
            buf
        });
        let ticker_len = meta[41] as usize;
        if meta.len() < 42 + ticker_len {
            return None;
        }
        let ticker = String::from_utf8(meta[42..42 + ticker_len].to_vec()).ok()?;
        Some((asset_id, total_supply, ticker))
    }

    /// Create a bridge HTLC output for cross-chain atomic swaps (v2 with counter_hash).
    ///
    /// `extra_data` layout v2: `[condition_bytes][1B version=2][1B target_chain][1B addr_len][target_address][32B counter_hash]`
    /// The condition is a standard HTLC: `(Hashlock AND Timelock) OR TimelockExpiry`.
    /// The metadata identifies the target chain, recipient, and counter-chain hash for the swap.
    /// `counter_hash` is the hash the target chain understands (SHA256 for Bitcoin, keccak256 for Ethereum).
    #[allow(clippy::too_many_arguments)]
    pub fn bridge_htlc(
        amount: Amount,
        pubkey_hash: Hash,
        expected_hash: Hash,
        lock_height: BlockHeight,
        expiry_height: BlockHeight,
        target_chain: u8,
        target_address: &[u8],
        counter_hash: Hash,
    ) -> Result<Self, crate::conditions::ConditionError> {
        if lock_height >= expiry_height {
            return Err(crate::conditions::ConditionError::InvalidTimelockRange {
                lock: lock_height,
                expiry: expiry_height,
            });
        }
        let cond = crate::conditions::Condition::htlc(expected_hash, lock_height, expiry_height);
        let condition_bytes = cond.encode()?;
        let metadata_len =
            BRIDGE_HTLC_HEADER_SIZE + target_address.len() + BRIDGE_HTLC_COUNTER_HASH_SIZE;
        if condition_bytes.len() + metadata_len > MAX_EXTRA_DATA_SIZE {
            return Err(crate::conditions::ConditionError::EncodingTooLarge {
                size: MAX_EXTRA_DATA_SIZE + 1,
            });
        }
        let mut extra_data = condition_bytes;
        extra_data.push(BRIDGE_HTLC_CURRENT_VERSION);
        extra_data.push(target_chain);
        extra_data.push(target_address.len() as u8);
        extra_data.extend_from_slice(target_address);
        extra_data.extend_from_slice(counter_hash.as_bytes());
        Ok(Self {
            output_type: OutputType::BridgeHTLC,
            amount,
            pubkey_hash,
            lock_until: 0,
            extra_data,
        })
    }

    /// Extract bridge HTLC metadata from extra_data.
    /// Returns (target_chain, target_address, counter_hash) or None.
    /// Handles both v1 (no counter_hash) and v2 (with counter_hash) layouts.
    pub fn bridge_htlc_metadata(&self) -> Option<(u8, Vec<u8>, Option<Hash>)> {
        if self.output_type != OutputType::BridgeHTLC || self.extra_data.is_empty() {
            return None;
        }
        let cond_len = match crate::conditions::Condition::decode_prefix(&self.extra_data) {
            Ok((_cond, consumed)) => consumed,
            Err(_) => return None,
        };
        let meta = &self.extra_data[cond_len..];
        if meta.len() < BRIDGE_HTLC_HEADER_SIZE {
            return None;
        }
        let version = meta[0];
        let target_chain = meta[1];
        let addr_len = meta[2] as usize;
        if meta.len() < 3 + addr_len {
            return None;
        }
        let target_address = meta[3..3 + addr_len].to_vec();
        match version {
            BRIDGE_HTLC_VERSION_V1 => Some((target_chain, target_address, None)),
            BRIDGE_HTLC_VERSION_V2 => {
                let hash_start = 3 + addr_len;
                if meta.len() < hash_start + BRIDGE_HTLC_COUNTER_HASH_SIZE {
                    return None;
                }
                let mut buf = [0u8; 32];
                buf.copy_from_slice(&meta[hash_start..hash_start + 32]);
                Some((target_chain, target_address, Some(Hash::from_bytes(buf))))
            }
            _ => None,
        }
    }

    /// Human-readable name for a bridge target chain ID.
    pub fn bridge_chain_name(chain_id: u8) -> &'static str {
        match chain_id {
            BRIDGE_CHAIN_BITCOIN => "Bitcoin",
            BRIDGE_CHAIN_ETHEREUM => "Ethereum",
            BRIDGE_CHAIN_MONERO => "Monero",
            BRIDGE_CHAIN_LITECOIN => "Litecoin",
            BRIDGE_CHAIN_CARDANO => "Cardano",
            _ => "Unknown",
        }
    }

    /// Decode the spending condition from extra_data (for conditioned output types).
    /// Returns None for Normal/Bond outputs.
    pub fn condition(
        &self,
    ) -> Option<Result<crate::conditions::Condition, crate::conditions::ConditionError>> {
        if self.output_type.is_conditioned() && !self.extra_data.is_empty() {
            Some(
                crate::conditions::Condition::decode_prefix(&self.extra_data)
                    .map(|(cond, _consumed)| cond),
            )
        } else {
            None
        }
    }

    /// Check if the output is spendable at a given height
    pub fn is_spendable_at(&self, height: BlockHeight) -> bool {
        height >= self.lock_until
    }

    /// Serialize for hashing
    pub fn serialize(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(self.output_type as u8);
        bytes.extend_from_slice(&self.amount.to_le_bytes());
        bytes.extend_from_slice(self.pubkey_hash.as_bytes());
        bytes.extend_from_slice(&self.lock_until.to_le_bytes());
        // extra_data: length-prefixed (u16 LE) + raw bytes
        bytes.extend_from_slice(&(self.extra_data.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&self.extra_data);
        bytes
    }
}
