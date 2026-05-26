use crypto::Hash;
use serde::{Deserialize, Serialize};

use crate::types::{Amount, BlockHeight};

use super::types::OutputType;

/// Base maximum size of extra_data in an output (bytes) — Era 0.
/// Reserved for conditions, metadata, and embedded content (e.g. NFT images, programs).
/// Normal and Bond outputs must have empty extra_data.
/// 512 KB enables: on-chain images, complex conditions, zero-knowledge proofs,
/// programmable UTXOs, and cases not yet imagined.
/// Doubles each era (~4 years), same schedule as block size growth.
/// The field is variable — unused bytes cost nothing.
pub const BASE_EXTRA_DATA_SIZE: usize = 524_288; // 512 KB

/// Maximum extra_data size cap (Era 4+).
pub const MAX_EXTRA_DATA_SIZE_CAP: usize = 8_388_608; // 8 MB

/// Legacy alias — use `max_extra_data_size(height)` for height-aware validation.
/// Kept for condition encoding checks (which don't have height context).
pub const MAX_EXTRA_DATA_SIZE: usize = BASE_EXTRA_DATA_SIZE;

/// Calculate max extra_data size for a given block height.
///
/// Doubles every era (~4 years), same schedule as block size:
/// - Era 0: 512 KB
/// - Era 1: 1 MB
/// - Era 2: 2 MB
/// - Era 3: 4 MB
/// - Era 4+: 8 MB (capped)
#[must_use]
pub fn max_extra_data_size(height: crate::types::BlockHeight) -> usize {
    let era = height / crate::consensus::BLOCKS_PER_ERA;
    if era >= 4 {
        MAX_EXTRA_DATA_SIZE_CAP
    } else {
        BASE_EXTRA_DATA_SIZE << era
    }
}

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
/// Bridge target chain: Binance Smart Chain (EVM-compatible, keccak256)
pub const BRIDGE_CHAIN_BSC: u8 = 6;
/// Bridge HTLC header: 1B version + 1B target_chain + 1B addr_len
pub const BRIDGE_HTLC_HEADER_SIZE: usize = 3;

/// Pool metadata version
pub const POOL_VERSION: u8 = 1;
/// Pool extra_data: 1B version + 32B pool_id + 32B asset_b_id + 8B reserve_a + 8B reserve_b + 8B total_lp + 16B cumulative_price + 4B last_slot + 2B fee_bps + 4B creation_slot + 1B status
pub const POOL_METADATA_SIZE: usize = 116;
/// Pool domain for deterministic ID
/// AMM Foundations M2 (D2, 2026-05-25): domain bumped to V2 to encode the
/// fee_bps inclusion in [`Output::compute_pool_id`]. Any pre-existing V1
/// artifact is provably non-collidable with a V2 pool_id by domain
/// separation. **IRREVERSIBLE** once `amm_activation_height` is ever
/// crossed — never change.
pub const POOL_ID_DOMAIN: &[u8] = b"DOLI_POOL_V2";
/// Default pool fee: 0.3% = 30 basis points
pub const POOL_DEFAULT_FEE_BPS: u16 = 30;
/// Maximum pool fee: 10% = 1000 basis points
pub const POOL_MAX_FEE_BPS: u16 = 1000;

/// LPShare metadata version
pub const LP_SHARE_VERSION: u8 = 1;
/// LPShare metadata size (after condition prefix): 1B version + 32B pool_id
pub const LP_SHARE_METADATA_SIZE: usize = 33;

/// Decoded pool metadata from extra_data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolMetadata {
    pub pool_id: Hash,
    pub asset_b_id: Hash,
    pub reserve_a: Amount,
    pub reserve_b: Amount,
    pub total_lp_shares: Amount,
    pub cumulative_price: u128,
    pub last_update_slot: u32,
    pub fee_bps: u16,
    pub creation_slot: u32,
    pub status: u8,
}

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
    /// Interpretation depends on output_type. Max 512 KB (era 0), grows per era.
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

    /// Create an HTLC output with signed refund (AUDIT-BRIDGE-001).
    ///
    /// `refund_pubkey_hash` is the creator/sender who can reclaim after expiry.
    /// The refund branch requires their signature, preventing front-running.
    pub fn htlc(
        amount: Amount,
        pubkey_hash: Hash,
        expected_hash: Hash,
        lock_height: BlockHeight,
        expiry_height: BlockHeight,
        refund_pubkey_hash: Hash,
    ) -> Result<Self, crate::conditions::ConditionError> {
        let cond = crate::conditions::Condition::htlc_signed_refund(
            expected_hash,
            lock_height,
            expiry_height,
            refund_pubkey_hash,
        );
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

    /// Create an EncryptedContent output.
    ///
    /// `extra_data` layout: `[ciphertext_len(4 LE) | ciphertext | wrapped_key(80) | nonce(12) | content_hash(32)]`
    pub fn encrypted_content(
        amount: Amount,
        pubkey_hash: Hash,
        ciphertext: &[u8],
        wrapped_key: &[u8; 80],
        nonce: &[u8; 12],
        content_hash: &[u8; 32],
    ) -> Self {
        let ciphertext_len = ciphertext.len() as u32;
        let mut extra_data = Vec::with_capacity(4 + ciphertext.len() + 80 + 12 + 32);
        extra_data.extend_from_slice(&ciphertext_len.to_le_bytes());
        extra_data.extend_from_slice(ciphertext);
        extra_data.extend_from_slice(wrapped_key);
        extra_data.extend_from_slice(nonce);
        extra_data.extend_from_slice(content_hash);
        Self {
            output_type: OutputType::EncryptedContent,
            amount,
            pubkey_hash,
            lock_until: 0,
            extra_data,
        }
    }

    /// Construct an `OraclePrice` system UTXO — Phase 2.1 Oracle M5.
    ///
    /// Per-pair singleton output created by `apply_block` at the
    /// epoch boundary (M6 aggregator). User transactions never call
    /// this — only the in-node epoch-boundary code path does. The
    /// validation arm `OutputType::OraclePrice` hard-rejects any user
    /// tx that emits one of these (`[ERRTX-ORACLE004]`).
    ///
    /// extra_data layout (50 bytes, fixed):
    ///   offset  0  u64 LE   price_cents
    ///   offset  8  u64 LE   last_update_height
    ///   offset 16  u16 LE   contributor_count
    ///   offset 18  [u8;32]  pair_id
    ///
    /// The `pubkey_hash` field is set to the deterministic system
    /// address `oracle_price_address(pair_id)` so the UTXO is always
    /// looked up at the same outpoint key, regardless of which
    /// producer minted the epoch-boundary block.
    ///
    /// `amount = 0` (price lives in extra_data; no DOLI is locked).
    /// `lock_until = 0` (system-spent only, not user-spendable).
    ///
    /// Spec: `specs/oracle-structural-anchored-economics.md` §1.2.
    pub fn oracle_price(
        pair_id: Hash,
        price_cents: u64,
        last_update_height: u64,
        contributor_count: u16,
    ) -> Self {
        let mut extra_data = Vec::with_capacity(Self::ORACLE_PRICE_EXTRA_DATA_SIZE);
        extra_data.extend_from_slice(&price_cents.to_le_bytes());
        extra_data.extend_from_slice(&last_update_height.to_le_bytes());
        extra_data.extend_from_slice(&contributor_count.to_le_bytes());
        extra_data.extend_from_slice(pair_id.as_bytes());
        debug_assert_eq!(extra_data.len(), Self::ORACLE_PRICE_EXTRA_DATA_SIZE);
        Self {
            output_type: OutputType::OraclePrice,
            amount: 0,
            pubkey_hash: Self::oracle_price_address(&pair_id),
            lock_until: 0,
            extra_data,
        }
    }

    /// Fixed `extra_data` size for `OraclePrice` outputs (M5 spec §1.2).
    ///
    /// 8 (price_cents) + 8 (last_update_height) + 2 (contributor_count)
    /// + 32 (pair_id) = 50 bytes.
    pub const ORACLE_PRICE_EXTRA_DATA_SIZE: usize = 8 + 8 + 2 + 32;

    /// Deterministic system address for the per-pair `OraclePrice`
    /// UTXO. Equal to `hash_with_domain(b"ORACLE_PRICE", pair_id)`.
    ///
    /// Mirrors `crate::consensus::reward_pool_address()`, which uses
    /// the same `hash_with_domain` pattern (consensus/constants.rs:44).
    /// The domain prefix is critical: without it, any 32-byte
    /// preimage colliding with `pair_id` would map to the same
    /// address as some other system pool.
    ///
    /// Used by M6's aggregator to look up the previous epoch's
    /// `OraclePrice` UTXO and consume-and-recreate it with the new
    /// median.
    pub fn oracle_price_address(pair_id: &Hash) -> Hash {
        crypto::hash::hash_with_domain(b"ORACLE_PRICE", pair_id.as_bytes())
    }

    /// Decode the `extra_data` of an `OraclePrice` output into its
    /// four fixed fields. Returns `None` if the output is not of
    /// type `OraclePrice` or if `extra_data` is not exactly 50
    /// bytes long.
    pub fn parse_oracle_price(&self) -> Option<(u64, u64, u16, Hash)> {
        if self.output_type != OutputType::OraclePrice
            || self.extra_data.len() != Self::ORACLE_PRICE_EXTRA_DATA_SIZE
        {
            return None;
        }
        let price_cents = u64::from_le_bytes(self.extra_data[0..8].try_into().ok()?);
        let last_update_height = u64::from_le_bytes(self.extra_data[8..16].try_into().ok()?);
        let contributor_count = u16::from_le_bytes(self.extra_data[16..18].try_into().ok()?);
        let pair_id_bytes: [u8; 32] = self.extra_data[18..50].try_into().ok()?;
        Some((
            price_cents,
            last_update_height,
            contributor_count,
            Hash::from_bytes(pair_id_bytes),
        ))
    }

    /// Parse EncryptedContent extra_data layout.
    /// Returns (ciphertext, wrapped_key, nonce, content_hash) or None if malformed.
    #[allow(clippy::type_complexity)]
    pub fn parse_encrypted_content(&self) -> Option<(&[u8], [u8; 80], [u8; 12], [u8; 32])> {
        if self.output_type != OutputType::EncryptedContent || self.extra_data.len() < 128 {
            return None;
        }
        let ct_len = u32::from_le_bytes(self.extra_data[0..4].try_into().ok()?) as usize;
        let offset = 4 + ct_len;
        if self.extra_data.len() < offset + 80 + 12 + 32 {
            return None;
        }
        let ciphertext = &self.extra_data[4..4 + ct_len];
        let mut wrapped_key = [0u8; 80];
        wrapped_key.copy_from_slice(&self.extra_data[offset..offset + 80]);
        let mut nonce = [0u8; 12];
        nonce.copy_from_slice(&self.extra_data[offset + 80..offset + 92]);
        let mut content_hash = [0u8; 32];
        content_hash.copy_from_slice(&self.extra_data[offset + 92..offset + 124]);
        Some((ciphertext, wrapped_key, nonce, content_hash))
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
        let cond = crate::conditions::Condition::htlc_signed_refund(
            expected_hash,
            lock_height,
            expiry_height,
            pubkey_hash,
        );
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
            BRIDGE_CHAIN_BSC => "BSC",
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

    /// Serialize for hashing.
    ///
    /// ENCODING SCHEMES FOR LENGTH-PREFIXED DATA
    ///
    /// Output::serialize() (hash-only, never deserialized):
    ///   extra_data.len() < 65536  -> u16 LE (backward-compatible with all existing blocks)
    ///   extra_data.len() >= 65536 -> u32 LE (for large NFTs >64KB)
    ///   No ambiguity because this is never deserialized -- bytes go into BLAKE3.
    ///
    /// Covenant witnesses (serialized AND deserialized -- see set_covenant_witnesses):
    ///   witness.len() < 65535   -> u16 LE (backward-compatible)
    ///   witness.len() >= 65535  -> escape marker 0xFFFF + u32 LE
    ///   0xFFFF is unambiguous because real u16 lengths max at 65534.
    pub fn serialize(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(self.output_type as u8);
        bytes.extend_from_slice(&self.amount.to_le_bytes());
        bytes.extend_from_slice(self.pubkey_hash.as_bytes());
        bytes.extend_from_slice(&self.lock_until.to_le_bytes());
        // extra_data: length-prefixed, backward-compatible encoding.
        // ≤64KB: u16 LE (matches all existing blocks).
        // >64KB: escape marker 0xFFFF + u32 LE (no existing blocks affected).
        if self.extra_data.len() > 65535 {
            bytes.extend_from_slice(&0xFFFFu16.to_le_bytes()); // escape marker
            bytes.extend_from_slice(&(self.extra_data.len() as u32).to_le_bytes());
        } else {
            bytes.extend_from_slice(&(self.extra_data.len() as u16).to_le_bytes());
        }
        bytes.extend_from_slice(&self.extra_data);
        bytes
    }

    /// Compute a deterministic pool ID for a given asset pair AND fee tier.
    ///
    /// AMM Foundations M2 (D2, 2026-05-25): `fee_bps` is included in the
    /// hash so that the same asset pair can host multiple pools at
    /// different fee tiers (e.g. 5/30/100 bps). Each (pair, fee_bps) tuple
    /// derives to a DIFFERENT `pool_id`, generalising the per-pool
    /// singleton invariant (INV-DEFI-010) to a per-(pair, fee_bps)
    /// singleton.
    ///
    /// Canonical payload layout (PINNED — IRREVERSIBLE once
    /// `amm_activation_height` is ever crossed):
    ///
    ///   `pool_id = BLAKE3(POOL_ID_DOMAIN ‖ fee_bps_le ‖ lo_asset ‖ hi_asset)`
    ///
    /// where `(lo_asset, hi_asset) = sort_by_raw_bytes(asset_a, asset_b)`
    /// and `POOL_ID_DOMAIN = b"DOLI_POOL_V2"`. The asset sort makes the
    /// function commutative in `asset_a`/`asset_b`. The V2 domain bump
    /// guarantees domain separation from any pre-existing V1 artifact.
    ///
    /// Spec: `specs/defi-foundations-economics.md` §0 D2.
    pub fn compute_pool_id(asset_a: &Hash, asset_b: &Hash, fee_bps: u16) -> Hash {
        use crypto::hash::hash_with_domain;
        let (lo, hi) = if asset_a.as_bytes() < asset_b.as_bytes() {
            (asset_a, asset_b)
        } else {
            (asset_b, asset_a)
        };
        let mut data = Vec::with_capacity(2 + 64);
        data.extend_from_slice(&fee_bps.to_le_bytes());
        data.extend_from_slice(lo.as_bytes());
        data.extend_from_slice(hi.as_bytes());
        hash_with_domain(POOL_ID_DOMAIN, &data)
    }

    /// Create a pool output.
    ///
    /// `asset_a` = DOLI (Hash::ZERO), `asset_b` = FungibleAsset ID.
    /// `pubkey_hash` = deterministic pool address (same as pool_id for simplicity).
    #[allow(clippy::too_many_arguments)]
    pub fn pool(
        pool_id: Hash,
        asset_b_id: Hash,
        reserve_a: Amount,
        reserve_b: Amount,
        total_lp_shares: Amount,
        cumulative_price: u128,
        last_update_slot: u32,
        fee_bps: u16,
        creation_slot: u32,
    ) -> Self {
        let mut extra_data = Vec::with_capacity(POOL_METADATA_SIZE);
        extra_data.push(POOL_VERSION);
        extra_data.extend_from_slice(pool_id.as_bytes());
        extra_data.extend_from_slice(asset_b_id.as_bytes());
        extra_data.extend_from_slice(&reserve_a.to_le_bytes());
        extra_data.extend_from_slice(&reserve_b.to_le_bytes());
        extra_data.extend_from_slice(&total_lp_shares.to_le_bytes());
        extra_data.extend_from_slice(&cumulative_price.to_le_bytes());
        extra_data.extend_from_slice(&last_update_slot.to_le_bytes());
        extra_data.extend_from_slice(&fee_bps.to_le_bytes());
        extra_data.extend_from_slice(&creation_slot.to_le_bytes());
        extra_data.push(0u8); // status: active
        Self {
            output_type: OutputType::Pool,
            amount: 0,            // reserves tracked in extra_data
            pubkey_hash: pool_id, // pool address = pool_id
            lock_until: 0,
            extra_data,
        }
    }

    /// Extract pool metadata from extra_data.
    /// Returns None if not a Pool output.
    #[allow(clippy::type_complexity)]
    pub fn pool_metadata(&self) -> Option<PoolMetadata> {
        if self.output_type != OutputType::Pool || self.extra_data.len() < POOL_METADATA_SIZE {
            return None;
        }
        let d = &self.extra_data;
        if d[0] != POOL_VERSION {
            return None;
        }
        let pool_id = Hash::from_bytes({
            let mut buf = [0u8; 32];
            buf.copy_from_slice(&d[1..33]);
            buf
        });
        let asset_b_id = Hash::from_bytes({
            let mut buf = [0u8; 32];
            buf.copy_from_slice(&d[33..65]);
            buf
        });
        let reserve_a = u64::from_le_bytes(d[65..73].try_into().ok()?);
        let reserve_b = u64::from_le_bytes(d[73..81].try_into().ok()?);
        let total_lp_shares = u64::from_le_bytes(d[81..89].try_into().ok()?);
        let cumulative_price = u128::from_le_bytes(d[89..105].try_into().ok()?);
        let last_update_slot = u32::from_le_bytes(d[105..109].try_into().ok()?);
        let fee_bps = u16::from_le_bytes(d[109..111].try_into().ok()?);
        let creation_slot = u32::from_le_bytes(d[111..115].try_into().ok()?);
        let status = d[115];
        Some(PoolMetadata {
            pool_id,
            asset_b_id,
            reserve_a,
            reserve_b,
            total_lp_shares,
            cumulative_price,
            last_update_slot,
            fee_bps,
            creation_slot,
            status,
        })
    }

    /// Create an LP share output with default `Condition::Signature(owner)`.
    ///
    /// `extra_data` layout: `[condition_bytes][1B version][32B pool_id]`.
    /// The default condition wraps `Condition::Signature(owner)` so existing
    /// call sites (CLI, pool validation, tests) stay ergonomic — the spending
    /// path is identical to a single-sig Normal output.
    ///
    /// For custom conditions (AmountGuard, Timelock, etc.), use
    /// `lp_share_with_condition()`.
    pub fn lp_share(share_amount: Amount, pool_id: Hash, owner: Hash) -> Self {
        let condition = crate::conditions::Condition::Signature(owner);
        // encode() cannot fail for a simple Signature condition
        let condition_bytes = condition
            .encode()
            .expect("Signature condition always encodes");
        let mut extra_data = Vec::with_capacity(condition_bytes.len() + LP_SHARE_METADATA_SIZE);
        extra_data.extend_from_slice(&condition_bytes);
        extra_data.push(LP_SHARE_VERSION);
        extra_data.extend_from_slice(pool_id.as_bytes());
        Self {
            output_type: OutputType::LPShare,
            amount: share_amount,
            pubkey_hash: owner,
            lock_until: 0,
            extra_data,
        }
    }

    /// Create an LP share output with a custom spending condition.
    ///
    /// `extra_data` layout: `[condition_bytes][1B version][32B pool_id]`.
    /// Use this for LPShares that need AmountGuard, Timelock, or other
    /// compound conditions beyond simple signature ownership.
    pub fn lp_share_with_condition(
        share_amount: Amount,
        pool_id: Hash,
        owner: Hash,
        condition: &crate::conditions::Condition,
    ) -> Result<Self, crate::conditions::ConditionError> {
        let condition_bytes = condition.encode()?;
        let metadata_len = LP_SHARE_METADATA_SIZE;
        if condition_bytes.len() + metadata_len > MAX_EXTRA_DATA_SIZE {
            return Err(crate::conditions::ConditionError::EncodingTooLarge {
                size: MAX_EXTRA_DATA_SIZE + 1,
            });
        }
        let mut extra_data = Vec::with_capacity(condition_bytes.len() + metadata_len);
        extra_data.extend_from_slice(&condition_bytes);
        extra_data.push(LP_SHARE_VERSION);
        extra_data.extend_from_slice(pool_id.as_bytes());
        Ok(Self {
            output_type: OutputType::LPShare,
            amount: share_amount,
            pubkey_hash: owner,
            lock_until: 0,
            extra_data,
        })
    }

    /// Extract LP share metadata. Returns the pool_id or None.
    ///
    /// Skips the condition prefix (via `Condition::decode_prefix`) then reads
    /// `[1B version][32B pool_id]` from the remaining bytes.
    pub fn lp_share_metadata(&self) -> Option<Hash> {
        if self.output_type != OutputType::LPShare || self.extra_data.is_empty() {
            return None;
        }
        let cond_len = match crate::conditions::Condition::decode_prefix(&self.extra_data) {
            Ok((_, len)) => len,
            Err(_) => return None,
        };
        let meta = &self.extra_data[cond_len..];
        if meta.len() < LP_SHARE_METADATA_SIZE {
            return None;
        }
        if meta[0] != LP_SHARE_VERSION {
            return None;
        }
        let pool_id = Hash::from_bytes({
            let mut buf = [0u8; 32];
            buf.copy_from_slice(&meta[1..33]);
            buf
        });
        Some(pool_id)
    }

    // ==================== EncryptedContent v1 (MIME + Royalties) ====================

    /// EncryptedContent metadata version for MIME + royalties extension.
    pub const EC_METADATA_VERSION_V1: u8 = 1;

    /// Maximum MIME type length in bytes.
    pub const EC_MAX_MIME_LEN: usize = 127;

    /// Minimum v1 extension size: version(1) + mime_len(1) + creator_hash(32) + royalty_bps(2) = 36.
    pub const EC_V1_MIN_EXTENSION: usize = 36;

    /// Create an EncryptedContent v1 output with MIME type and royalties.
    ///
    /// `extra_data` layout:
    /// `[ciphertext_len(4 LE) | ciphertext | wrapped_key(80) | nonce(12) | content_hash(32)
    ///  | metadata_version(1) | mime_len(1) | mime_bytes(N) | creator_hash(32) | royalty_bps(2)]`
    #[allow(clippy::too_many_arguments)]
    pub fn encrypted_content_v1(
        amount: Amount,
        pubkey_hash: Hash,
        ciphertext: &[u8],
        wrapped_key: &[u8; 80],
        nonce: &[u8; 12],
        content_hash: &[u8; 32],
        mime_type: &[u8],
        creator_hash: Hash,
        royalty_bps: u16,
    ) -> Self {
        assert!(mime_type.len() <= Self::EC_MAX_MIME_LEN);
        assert!(royalty_bps <= MAX_ROYALTY_BPS);
        let ciphertext_len = ciphertext.len() as u32;
        let total = 4 + ciphertext.len() + 80 + 12 + 32 + 1 + 1 + mime_type.len() + 32 + 2;
        let mut extra_data = Vec::with_capacity(total);
        extra_data.extend_from_slice(&ciphertext_len.to_le_bytes());
        extra_data.extend_from_slice(ciphertext);
        extra_data.extend_from_slice(wrapped_key);
        extra_data.extend_from_slice(nonce);
        extra_data.extend_from_slice(content_hash);
        // v1 extension
        extra_data.push(Self::EC_METADATA_VERSION_V1);
        extra_data.push(mime_type.len() as u8);
        extra_data.extend_from_slice(mime_type);
        extra_data.extend_from_slice(creator_hash.as_bytes());
        extra_data.extend_from_slice(&royalty_bps.to_le_bytes());
        Self {
            output_type: OutputType::EncryptedContent,
            amount,
            pubkey_hash,
            lock_until: 0,
            extra_data,
        }
    }

    /// Parse EncryptedContent v1 metadata extension.
    /// Returns `Some((mime_type, creator_hash, royalty_bps))` if v1 extension present, None for v0.
    pub fn parse_encrypted_content_v1(&self) -> Option<(Vec<u8>, Hash, u16)> {
        if self.output_type != OutputType::EncryptedContent || self.extra_data.len() < 128 {
            return None;
        }
        let ct_len = u32::from_le_bytes(self.extra_data[0..4].try_into().ok()?) as usize;
        let v0_end = 4 + ct_len + 80 + 12 + 32;
        if self.extra_data.len() <= v0_end {
            return None; // v0 — no extension
        }
        if self.extra_data[v0_end] != Self::EC_METADATA_VERSION_V1 {
            return None; // unknown version
        }
        if self.extra_data.len() < v0_end + 2 {
            return None;
        }
        let mime_len = self.extra_data[v0_end + 1] as usize;
        let ext_start = v0_end + 2;
        if self.extra_data.len() < ext_start + mime_len + 32 + 2 {
            return None;
        }
        let mime_type = self.extra_data[ext_start..ext_start + mime_len].to_vec();
        let mut creator_bytes = [0u8; 32];
        creator_bytes
            .copy_from_slice(&self.extra_data[ext_start + mime_len..ext_start + mime_len + 32]);
        let creator_hash = Hash::from_bytes(creator_bytes);
        let bps_start = ext_start + mime_len + 32;
        let royalty_bps =
            u16::from_le_bytes([self.extra_data[bps_start], self.extra_data[bps_start + 1]]);
        Some((mime_type, creator_hash, royalty_bps))
    }

    /// Extract royalty info from an EncryptedContent output.
    /// Returns `Some((creator_hash, royalty_bps))` if this content has royalties (v1).
    pub fn encrypted_content_royalty(&self) -> Option<(Hash, u16)> {
        self.parse_encrypted_content_v1()
            .map(|(_, creator, bps)| (creator, bps))
    }
}

// OUTPUT CONTRACT: fn Output::encrypted_content(amount, pubkey_hash, ciphertext, wrapped_key, nonce, content_hash)
//   O1: return.output_type — OutputType::EncryptedContent
//   O2: return.extra_data — [ct_len(4 LE) | ciphertext | wrapped_key(80) | nonce(12) | content_hash(32)]
//   O3: return.amount — amount passthrough
//   O4: return.pubkey_hash — pubkey_hash passthrough
// PATHS: P1 = v0 construction (always)
//
// OUTPUT CONTRACT: fn Output::encrypted_content_v1(... + mime_type, creator_hash, royalty_bps)
//   O1-O4: same as v0
//   O5: return.extra_data tail — [metadata_version(1) | mime_len(1) | mime_bytes(N) | creator_hash(32) | royalty_bps(2 LE)]
// PATHS: P1 = v1 construction (always)
//
// OUTPUT CONTRACT: fn Output::parse_encrypted_content(&self) -> Option<(ciphertext, wrapped_key, nonce, content_hash)>
//   O1: return — Some(tuple) if valid EncryptedContent, None otherwise
// PATHS: P1 = v0 output, P2 = v1 output (backward compat), P3 = non-EC output (None), P4 = truncated (None)
//
// OUTPUT CONTRACT: fn Output::parse_encrypted_content_v1(&self) -> Option<(mime, creator_hash, royalty_bps)>
//   O1: return — Some(tuple) if v1 extension present, None for v0/non-EC
// PATHS: P1 = v1 output (Some), P2 = v0 output (None), P3 = non-EC (None), P4 = empty MIME (Some with empty vec)
//
// OUTPUT CONTRACT: fn Output::encrypted_content_royalty(&self) -> Option<(creator_hash, royalty_bps)>
//   O1: return — delegates to parse_encrypted_content_v1, strips mime
// PATHS: P1 = v1 with royalty (Some), P2 = v0 (None)
//
// MATRIX:
//   v0×parse_v0=Some(4 fields)  | v0×parse_v1=None          | v0×royalty=None
//   v1×parse_v0=Some(4 fields)  | v1×parse_v1=Some(3 fields)| v1×royalty=Some(2 fields)
//   empty_mime×parse_v1=Some     | various_mime×parse_v1=Some (roundtrip)
#[cfg(test)]
mod tests {
    use super::*;

    fn sample_v0_output() -> Output {
        let ciphertext = vec![0xABu8; 64];
        let wrapped_key = [0x11u8; 80];
        let nonce = [0x22u8; 12];
        let content_hash = [0x33u8; 32];
        let pubkey_hash = Hash::from_bytes([0x44u8; 32]);
        Output::encrypted_content(
            1,
            pubkey_hash,
            &ciphertext,
            &wrapped_key,
            &nonce,
            &content_hash,
        )
    }

    fn sample_v1_output() -> Output {
        let ciphertext = vec![0xABu8; 64];
        let wrapped_key = [0x11u8; 80];
        let nonce = [0x22u8; 12];
        let content_hash = [0x33u8; 32];
        let pubkey_hash = Hash::from_bytes([0x44u8; 32]);
        let creator_hash = Hash::from_bytes([0x55u8; 32]);
        Output::encrypted_content_v1(
            1,
            pubkey_hash,
            &ciphertext,
            &wrapped_key,
            &nonce,
            &content_hash,
            b"image/png",
            creator_hash,
            500, // 5%
        )
    }

    #[test]
    fn v0_output_parses_correctly() {
        let output = sample_v0_output();
        let parsed = output.parse_encrypted_content();
        assert!(parsed.is_some(), "v0 output must parse");
        let (ct, wk, n, ch) = parsed.unwrap();
        assert_eq!(ct, &[0xABu8; 64]);
        assert_eq!(wk, [0x11u8; 80]);
        assert_eq!(n, [0x22u8; 12]);
        assert_eq!(ch, [0x33u8; 32]);
    }

    #[test]
    fn v0_output_has_no_v1_extension() {
        let output = sample_v0_output();
        assert!(
            output.parse_encrypted_content_v1().is_none(),
            "v0 output must not parse as v1"
        );
        assert!(
            output.encrypted_content_royalty().is_none(),
            "v0 output has no royalty"
        );
    }

    #[test]
    fn v1_output_still_parses_as_v0() {
        let output = sample_v1_output();
        let parsed = output.parse_encrypted_content();
        assert!(
            parsed.is_some(),
            "v1 output must still parse as v0 (backward compat)"
        );
        let (ct, wk, n, ch) = parsed.unwrap();
        assert_eq!(ct, &[0xABu8; 64]);
        assert_eq!(wk, [0x11u8; 80]);
        assert_eq!(n, [0x22u8; 12]);
        assert_eq!(ch, [0x33u8; 32]);
    }

    #[test]
    fn v1_output_parses_mime_and_royalty() {
        let output = sample_v1_output();
        let parsed = output.parse_encrypted_content_v1();
        assert!(parsed.is_some(), "v1 output must parse v1 extension");
        let (mime, creator, bps) = parsed.unwrap();
        assert_eq!(mime, b"image/png");
        assert_eq!(creator, Hash::from_bytes([0x55u8; 32]));
        assert_eq!(bps, 500);
    }

    #[test]
    fn v1_royalty_extraction() {
        let output = sample_v1_output();
        let royalty = output.encrypted_content_royalty();
        assert!(royalty.is_some());
        let (creator, bps) = royalty.unwrap();
        assert_eq!(creator, Hash::from_bytes([0x55u8; 32]));
        assert_eq!(bps, 500);
    }

    #[test]
    fn v1_zero_length_mime() {
        let ciphertext = vec![0xABu8; 32];
        let wrapped_key = [0x11u8; 80];
        let nonce = [0x22u8; 12];
        let content_hash = [0x33u8; 32];
        let pubkey_hash = Hash::from_bytes([0x44u8; 32]);
        let creator_hash = Hash::from_bytes([0x55u8; 32]);
        let output = Output::encrypted_content_v1(
            1,
            pubkey_hash,
            &ciphertext,
            &wrapped_key,
            &nonce,
            &content_hash,
            b"", // empty MIME
            creator_hash,
            0, // no royalty
        );
        let parsed = output.parse_encrypted_content_v1();
        assert!(parsed.is_some());
        let (mime, creator, bps) = parsed.unwrap();
        assert!(mime.is_empty());
        assert_eq!(creator, creator_hash);
        assert_eq!(bps, 0);
    }

    #[test]
    fn v1_roundtrip_various_mime_types() {
        let mimes: &[&[u8]] = &[
            b"image/jpeg",
            b"image/png",
            b"text/plain",
            b"application/pdf",
            b"audio/mpeg",
            b"video/mp4",
        ];
        for mime in mimes {
            let output = Output::encrypted_content_v1(
                1,
                Hash::from_bytes([0x44u8; 32]),
                &[0u8; 16],
                &[0u8; 80],
                &[0u8; 12],
                &[0u8; 32],
                mime,
                Hash::from_bytes([0x55u8; 32]),
                250,
            );
            let parsed = output.parse_encrypted_content_v1().unwrap();
            assert_eq!(&parsed.0, mime);
        }
    }
}
