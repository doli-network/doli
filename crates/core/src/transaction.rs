//! Transaction types and operations

use crypto::{Hash, PublicKey, Signature};
use serde::{Deserialize, Serialize};

use crate::types::{Amount, BlockHeight};

/// Transaction type
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u32)]
pub enum TxType {
    /// Regular transfer transaction
    Transfer = 0,
    /// Producer registration transaction
    Registration = 1,
    /// Producer exit transaction (starts unbonding period)
    Exit = 2,
    /// Claim accumulated rewards
    ClaimReward = 3,
    /// Claim bond after unbonding period completes
    ClaimBond = 4,
    /// Slash a misbehaving producer (with evidence)
    SlashProducer = 5,
    /// Coinbase transaction (block reward to producer)
    Coinbase = 6,
    /// Add bonds to increase stake (bond stacking)
    AddBond = 7,
    /// Request withdrawal of bonds (instant, with vesting penalty)
    RequestWithdrawal = 8,
    /// Reserved -- unused. Withdrawal is instant via RequestWithdrawal.
    ClaimWithdrawal = 9,
    /// Epoch reward transaction (automatic weighted presence rewards at epoch boundary)
    ///
    /// This is the primary reward mechanism. At each epoch boundary, rewards are
    /// automatically distributed to all producers based on their weighted presence:
    /// - reward = Σ(block_reward × producer_weight / total_present_weight)
    /// - No manual claim needed - rewards go directly to producer wallets
    EpochReward = 10,
    /// Remove a maintainer from the auto-update system
    ///
    /// Requires 3/5 signatures from OTHER maintainers (target cannot sign own removal).
    /// Cannot reduce maintainer count below MIN_MAINTAINERS (3).
    RemoveMaintainer = 11,
    /// Add a new maintainer to the auto-update system
    ///
    /// Requires 3/5 signatures from current maintainers.
    /// Target must be a registered producer.
    /// Cannot exceed MAX_MAINTAINERS (5).
    AddMaintainer = 12,
    /// Delegate bond weight to a Tier 1/2 validator.
    ///
    /// The delegate receives the staker's weight for selection purposes.
    /// Rewards are split: delegate keeps DELEGATE_REWARD_PCT (10%),
    /// stakers receive STAKER_REWARD_PCT (90%).
    DelegateBond = 13,
    /// Revoke delegation (DELEGATION_UNBONDING_SLOTS delay applies).
    RevokeDelegation = 14,
    /// On-chain protocol activation (3/5 maintainer multisig).
    ///
    /// Schedules new consensus rules to activate at a future epoch boundary.
    /// All nodes switch simultaneously — deterministic, zero coordination.
    ProtocolActivation = 15,
}

impl TxType {
    pub fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::Transfer),
            1 => Some(Self::Registration),
            2 => Some(Self::Exit),
            3 => Some(Self::ClaimReward),
            4 => Some(Self::ClaimBond),
            5 => Some(Self::SlashProducer),
            6 => Some(Self::Coinbase),
            7 => Some(Self::AddBond),
            8 => Some(Self::RequestWithdrawal),
            9 => Some(Self::ClaimWithdrawal),
            10 => Some(Self::EpochReward),
            11 => Some(Self::RemoveMaintainer),
            12 => Some(Self::AddMaintainer),
            13 => Some(Self::DelegateBond),
            14 => Some(Self::RevokeDelegation),
            15 => Some(Self::ProtocolActivation),
            _ => None,
        }
    }
}

/// Output type
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum OutputType {
    /// Normal spendable output (default: single signature)
    Normal = 0,
    /// Bond output (time-locked, protocol-governed withdrawal)
    Bond = 1,
    /// Multisig output (threshold-of-N signatures, also used for escrow)
    Multisig = 2,
    /// Hashlock output (requires preimage reveal)
    Hashlock = 3,
    /// HTLC output (hashlock + timelock OR expiry refund)
    HTLC = 4,
    /// Vesting output (signature + timelock)
    Vesting = 5,
    /// NFT output (non-fungible token with metadata + covenant conditions)
    NFT = 6,
    /// Fungible asset output (user-issued token with fixed supply)
    FungibleAsset = 7,
    /// Bridge HTLC output (cross-chain atomic swap with target chain metadata)
    BridgeHTLC = 8,
}

impl OutputType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Normal),
            1 => Some(Self::Bond),
            2 => Some(Self::Multisig),
            3 => Some(Self::Hashlock),
            4 => Some(Self::HTLC),
            5 => Some(Self::Vesting),
            6 => Some(Self::NFT),
            7 => Some(Self::FungibleAsset),
            8 => Some(Self::BridgeHTLC),
            _ => None,
        }
    }

    /// Returns true if this output type uses covenant conditions in extra_data.
    pub fn is_conditioned(&self) -> bool {
        matches!(
            self,
            Self::Multisig
                | Self::Hashlock
                | Self::HTLC
                | Self::Vesting
                | Self::NFT
                | Self::FungibleAsset
                | Self::BridgeHTLC
        )
    }
}

/// Sighash type controlling what parts of the transaction an input's signature covers.
///
/// Modeled after Bitcoin's SIGHASH flags. Used for partial signing (PSBT-style)
/// where different parties sign different inputs of the same transaction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SighashType {
    /// Sign ALL inputs and ALL outputs (default, backwards-compatible).
    /// Both parties must have the complete transaction before signing.
    #[default]
    All = 0,
    /// Sign only THIS input + ALL outputs.
    /// Allows other parties to add their own inputs after the signer has signed.
    /// Used for NFT marketplace: seller signs their NFT input, buyer adds payment inputs later.
    AnyoneCanPay = 1,
}

impl SighashType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::All),
            1 => Some(Self::AnyoneCanPay),
            _ => None,
        }
    }
}

/// Transaction input (reference to a previous output)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    /// Hash of the transaction containing the output
    pub prev_tx_hash: Hash,
    /// Index of the output in that transaction
    pub output_index: u32,
    /// Signature proving ownership
    pub signature: Signature,
    /// Sighash type: what this input's signature covers.
    /// Default: All (backwards-compatible with v1 transactions).
    #[serde(default)]
    pub sighash_type: SighashType,
    /// Number of outputs this input's signature commits to (AnyoneCanPay only).
    /// 0 = all outputs (backward compat with pre-v3.7.1 transactions).
    /// N > 0 = sighash covers only the first N outputs, allowing the buyer
    /// to append additional outputs (e.g. change) without invalidating
    /// the seller's signature.
    #[serde(default)]
    pub committed_output_count: u32,
}

impl Input {
    /// Create a new input (default sighash: All)
    pub fn new(prev_tx_hash: Hash, output_index: u32) -> Self {
        Self {
            prev_tx_hash,
            output_index,
            signature: Signature::default(),
            sighash_type: SighashType::All,
            committed_output_count: 0,
        }
    }

    /// Create a new input with AnyoneCanPay sighash type.
    /// The signature covers only this input + all outputs (not other inputs).
    pub fn new_anyone_can_pay(prev_tx_hash: Hash, output_index: u32) -> Self {
        Self {
            prev_tx_hash,
            output_index,
            signature: Signature::default(),
            sighash_type: SighashType::AnyoneCanPay,
            committed_output_count: 0,
        }
    }

    /// Create an AnyoneCanPay input that commits to only the first N outputs.
    /// The buyer can append additional outputs (e.g. change) without
    /// invalidating the seller's signature.
    pub fn new_anyone_can_pay_partial(
        prev_tx_hash: Hash,
        output_index: u32,
        committed_output_count: u32,
    ) -> Self {
        Self {
            prev_tx_hash,
            output_index,
            signature: Signature::default(),
            sighash_type: SighashType::AnyoneCanPay,
            committed_output_count,
        }
    }

    /// Create an outpoint identifier
    pub fn outpoint(&self) -> (Hash, u32) {
        (self.prev_tx_hash, self.output_index)
    }

    /// Serialize for signing
    pub fn serialize_for_signing(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(self.prev_tx_hash.as_bytes());
        bytes.extend_from_slice(&self.output_index.to_le_bytes());
        bytes
    }
}

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

/// Bridge HTLC metadata version
pub const BRIDGE_HTLC_VERSION: u8 = 1;
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

    /// Create a bridge HTLC output for cross-chain atomic swaps.
    ///
    /// `extra_data` layout: `[condition_bytes][1B version][1B target_chain][1B addr_len][target_address]`
    /// The condition is a standard HTLC: `(Hashlock AND Timelock) OR TimelockExpiry`.
    /// The metadata identifies the target chain and recipient for the counterpart swap.
    pub fn bridge_htlc(
        amount: Amount,
        pubkey_hash: Hash,
        expected_hash: Hash,
        lock_height: BlockHeight,
        expiry_height: BlockHeight,
        target_chain: u8,
        target_address: &[u8],
    ) -> Result<Self, crate::conditions::ConditionError> {
        if lock_height >= expiry_height {
            return Err(crate::conditions::ConditionError::InvalidTimelockRange {
                lock: lock_height,
                expiry: expiry_height,
            });
        }
        let cond = crate::conditions::Condition::htlc(expected_hash, lock_height, expiry_height);
        let condition_bytes = cond.encode()?;
        let metadata_len = BRIDGE_HTLC_HEADER_SIZE + target_address.len();
        if condition_bytes.len() + metadata_len > MAX_EXTRA_DATA_SIZE {
            return Err(crate::conditions::ConditionError::EncodingTooLarge {
                size: MAX_EXTRA_DATA_SIZE + 1,
            });
        }
        let mut extra_data = condition_bytes;
        extra_data.push(BRIDGE_HTLC_VERSION);
        extra_data.push(target_chain);
        extra_data.push(target_address.len() as u8);
        extra_data.extend_from_slice(target_address);
        Ok(Self {
            output_type: OutputType::BridgeHTLC,
            amount,
            pubkey_hash,
            lock_until: 0,
            extra_data,
        })
    }

    /// Extract bridge HTLC metadata from extra_data.
    /// Returns (target_chain, target_address) or None.
    pub fn bridge_htlc_metadata(&self) -> Option<(u8, Vec<u8>)> {
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
        if meta[0] != BRIDGE_HTLC_VERSION {
            return None;
        }
        let target_chain = meta[1];
        let addr_len = meta[2] as usize;
        if meta.len() < 3 + addr_len {
            return None;
        }
        let target_address = meta[3..3 + addr_len].to_vec();
        Some((target_chain, target_address))
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

/// Registration data for producer registration transactions
///
/// Registration uses a chained VDF system for anti-Sybil protection.
/// Each registration must reference the previous registration's hash,
/// creating a sequential chain that prevents parallel registration attacks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationData {
    /// Producer's public key
    pub public_key: PublicKey,
    /// Target epoch
    pub epoch: u32,
    /// VDF output
    pub vdf_output: Vec<u8>,
    /// VDF proof
    pub vdf_proof: Vec<u8>,
    /// Hash of the previous registration transaction (Hash::ZERO for first registration)
    ///
    /// This creates a chain that prevents parallel registration attacks.
    /// An attacker cannot register multiple nodes simultaneously because
    /// each registration must wait for the previous one to be confirmed.
    pub prev_registration_hash: Hash,
    /// Global sequence number for this registration
    ///
    /// Starts at 0 for the first registration, increments by 1 for each
    /// subsequent registration. Used to verify registration ordering.
    pub sequence_number: u64,
    /// Number of bonds being staked (each bond = 1 bond_unit).
    ///
    /// This is consensus-critical: all nodes must agree on bond_count for
    /// deterministic producer selection (WHITEPAPER Section 7).
    /// Stored on-chain to avoid re-deriving from bond_amount / local_bond_unit,
    /// which would break consensus if nodes have different bond_unit configs.
    pub bond_count: u32,
    /// BLS12-381 public key for aggregate attestation signatures (48 bytes).
    ///
    /// Required for epoch reward qualification. Verified at registration time
    /// via proof-of-possession to prevent rogue public key attacks.
    #[serde(default)]
    pub bls_pubkey: Vec<u8>,
    /// BLS proof-of-possession: signature over the BLS pubkey using the `PoP` DST (96 bytes).
    ///
    /// Proves the registrant possesses the BLS secret key, preventing
    /// rogue public key attacks on aggregate signature verification.
    #[serde(default)]
    pub bls_pop: Vec<u8>,
}

/// Exit data for producer exit transactions
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitData {
    /// Producer's public key (to identify which producer is exiting)
    pub public_key: PublicKey,
}

/// Claim data for reward claim transactions
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimData {
    /// Producer's public key (to identify which producer is claiming)
    pub public_key: PublicKey,
}

/// Claim bond data for bond claim transactions (after unbonding)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimBondData {
    /// Producer's public key (to identify which producer is claiming their bond)
    pub public_key: PublicKey,
}

/// Evidence of producer misbehavior for slashing
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlashingEvidence {
    /// Producer created two different blocks for the same slot
    /// This is the ONLY slashable offense - it's unambiguously intentional
    ///
    /// The evidence includes full block headers so validators can verify:
    /// 1. Both headers have the same producer
    /// 2. Both headers have the same slot
    /// 3. Both headers have different hashes
    /// 4. Both headers have valid VDFs (proving the producer actually created them)
    DoubleProduction {
        /// First block header (complete, for VDF verification)
        block_header_1: crate::BlockHeader,
        /// Second block header (complete, for VDF verification)
        block_header_2: crate::BlockHeader,
    },
    // Note: Invalid blocks are NOT slashable. The network simply rejects them.
    // This follows Bitcoin's philosophy: natural consequences (lost slot/reward)
    // are sufficient for honest mistakes. Slashing is reserved for fraud.
}

/// Slash data for slash producer transactions
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlashData {
    /// Producer being slashed
    pub producer_pubkey: PublicKey,
    /// Evidence of misbehavior
    pub evidence: SlashingEvidence,
    /// Signature from the reporter (to prevent spam)
    pub reporter_signature: Signature,
}

// ==================== Bond Stacking Transactions ====================
//
// Producers can stake multiple bonds (up to 100) to increase their
// selection weight. Each bond is 1,000 DOLI and has its own vesting timer.

/// Add bond data for increasing stake
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddBondData {
    /// Producer's public key
    pub producer_pubkey: PublicKey,
    /// Number of bonds to add (each bond = 1,000 DOLI)
    pub bond_count: u32,
}

impl AddBondData {
    /// Create new add bond data
    pub fn new(producer_pubkey: PublicKey, bond_count: u32) -> Self {
        Self {
            producer_pubkey,
            bond_count,
        }
    }

    /// Calculate total amount required for a specific network
    ///
    /// Uses NetworkParams to get the correct bond_unit for the network.
    pub fn total_amount_for_network(&self, network: crate::Network) -> Amount {
        let params = crate::network_params::NetworkParams::load(network);
        self.bond_count as Amount * params.bond_unit
    }

    /// Calculate total amount required (mainnet default)
    ///
    /// **Deprecated**: Use `total_amount_for_network(network)` instead for
    /// network-aware calculations.
    #[deprecated(note = "Use total_amount_for_network(network) for network-aware bond calculation")]
    pub fn total_amount(&self) -> Amount {
        // Fallback to mainnet bond_unit for backward compatibility
        let params = crate::network_params::NetworkParams::load(crate::Network::Mainnet);
        self.bond_count as Amount * params.bond_unit
    }

    /// Serialize to bytes for storage in extra_data
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(self.producer_pubkey.as_bytes());
        bytes.extend_from_slice(&self.bond_count.to_le_bytes());
        bytes
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 36 {
            return None;
        }
        let pubkey_bytes: [u8; 32] = bytes[0..32].try_into().ok()?;
        let bond_count = u32::from_le_bytes(bytes[32..36].try_into().ok()?);
        Some(Self {
            producer_pubkey: PublicKey::from_bytes(pubkey_bytes),
            bond_count,
        })
    }
}

// ==================== Bond Delegation (Tier 3 → Tier 1/2) ====================

/// Delegate bond weight to a Tier 1/2 validator.
///
/// Stored in Transaction.extra_data. The delegator's bond weight is added
/// to the delegate's effective_weight for producer selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DelegateBondData {
    /// Public key of the delegator (the producer delegating weight)
    pub delegator: PublicKey,
    /// Public key of the delegate (Tier 1/2 validator receiving weight)
    pub delegate: PublicKey,
    /// Number of bonds to delegate
    pub bond_count: u32,
}

impl DelegateBondData {
    pub fn new(delegator: PublicKey, delegate: PublicKey, bond_count: u32) -> Self {
        Self {
            delegator,
            delegate,
            bond_count,
        }
    }

    /// Serialize to bytes for storage in extra_data
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(self.delegator.as_bytes());
        bytes.extend_from_slice(self.delegate.as_bytes());
        bytes.extend_from_slice(&self.bond_count.to_le_bytes());
        bytes
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 68 {
            return None;
        }
        let delegator_bytes: [u8; 32] = bytes[0..32].try_into().ok()?;
        let delegate_bytes: [u8; 32] = bytes[32..64].try_into().ok()?;
        let bond_count = u32::from_le_bytes(bytes[64..68].try_into().ok()?);
        Some(Self {
            delegator: PublicKey::from_bytes(delegator_bytes),
            delegate: PublicKey::from_bytes(delegate_bytes),
            bond_count,
        })
    }
}

/// Revoke a previously delegated bond.
///
/// DELEGATION_UNBONDING_SLOTS delay applies before weight is removed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokeDelegationData {
    /// Public key of the delegator revoking
    pub delegator: PublicKey,
    /// Public key of the delegate to revoke from
    pub delegate: PublicKey,
}

impl RevokeDelegationData {
    pub fn new(delegator: PublicKey, delegate: PublicKey) -> Self {
        Self {
            delegator,
            delegate,
        }
    }

    /// Serialize to bytes for storage in extra_data
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(self.delegator.as_bytes());
        bytes.extend_from_slice(self.delegate.as_bytes());
        bytes
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 64 {
            return None;
        }
        let delegator_bytes: [u8; 32] = bytes[0..32].try_into().ok()?;
        let delegate_bytes: [u8; 32] = bytes[32..64].try_into().ok()?;
        Some(Self {
            delegator: PublicKey::from_bytes(delegator_bytes),
            delegate: PublicKey::from_bytes(delegate_bytes),
        })
    }
}

/// Withdrawal request data for starting bond withdrawal
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WithdrawalRequestData {
    /// Producer's public key
    pub producer_pubkey: PublicKey,
    /// Number of bonds to withdraw
    pub bond_count: u32,
    /// Destination address for the withdrawal
    pub destination: Hash,
}

impl WithdrawalRequestData {
    /// Create new withdrawal request data
    pub fn new(producer_pubkey: PublicKey, bond_count: u32, destination: Hash) -> Self {
        Self {
            producer_pubkey,
            bond_count,
            destination,
        }
    }

    /// Serialize to bytes for storage in extra_data
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(self.producer_pubkey.as_bytes());
        bytes.extend_from_slice(&self.bond_count.to_le_bytes());
        bytes.extend_from_slice(self.destination.as_bytes());
        bytes
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 68 {
            return None;
        }
        let pubkey_bytes: [u8; 32] = bytes[0..32].try_into().ok()?;
        let bond_count = u32::from_le_bytes(bytes[32..36].try_into().ok()?);
        let dest_bytes: [u8; 32] = bytes[36..68].try_into().ok()?;
        Some(Self {
            producer_pubkey: PublicKey::from_bytes(pubkey_bytes),
            bond_count,
            destination: Hash::from_bytes(dest_bytes),
        })
    }
}

// ==================== Epoch Reward Distribution ====================
//
// For fair reward distribution, rewards accumulate in a pool during an epoch
// and are distributed equally to all producers at epoch boundaries.

/// Epoch reward data for fair distribution transactions
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochRewardData {
    /// The epoch number this reward is for
    pub epoch: u64,
    /// The recipient producer's public key
    pub recipient: PublicKey,
}

impl EpochRewardData {
    /// Create new epoch reward data
    pub fn new(epoch: u64, recipient: PublicKey) -> Self {
        Self { epoch, recipient }
    }

    /// Serialize to bytes for storage in extra_data
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = self.epoch.to_le_bytes().to_vec();
        bytes.extend_from_slice(self.recipient.as_bytes());
        bytes
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 40 {
            // 8 bytes for epoch + 32 bytes for public key
            return None;
        }
        let epoch = u64::from_le_bytes(bytes[0..8].try_into().ok()?);
        let pubkey_bytes: [u8; 32] = bytes[8..40].try_into().ok()?;
        Some(Self {
            epoch,
            recipient: PublicKey::from_bytes(pubkey_bytes),
        })
    }
}
// ==================== Proof of Time ====================
//
// In Proof of Time, there are NO multi-signature attestations.
// Each block has exactly one producer who receives 100% of the block reward.
//
// Time is proven by producing blocks with valid VDF when selected:
// - One producer per slot (10 seconds)
// - Selection based on bond count (deterministic round-robin)
// - VDF provides anti-grinding protection (~7s computation)
// - Producer receives full block reward via coinbase transaction
//
// This eliminates:
// - Attestation structs and signatures
// - Multi-signature overhead
// - Delegation incentives (giving keys away means losing all rewards)
//
// With EpochPool reward mode, rewards accumulate and are distributed
// fairly at epoch boundaries via EpochReward transactions.

/// A transaction
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    /// Protocol version
    pub version: u32,
    /// Transaction type
    pub tx_type: TxType,
    /// Inputs (references to previous outputs)
    pub inputs: Vec<Input>,
    /// Outputs (new coins being created)
    pub outputs: Vec<Output>,
    /// Extra data (type-specific)
    pub extra_data: Vec<u8>,
}

impl Transaction {
    /// Create a new transfer transaction
    pub fn new_transfer(inputs: Vec<Input>, outputs: Vec<Output>) -> Self {
        Self {
            version: 1,
            tx_type: TxType::Transfer,
            inputs,
            outputs,
            extra_data: Vec::new(),
        }
    }

    /// Create a coinbase transaction
    pub fn new_coinbase(amount: Amount, pubkey_hash: Hash, height: BlockHeight) -> Self {
        Self {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: Vec::new(),
            outputs: vec![Output::normal(amount, pubkey_hash)],
            extra_data: height.to_le_bytes().to_vec(),
        }
    }

    /// Create an epoch reward coinbase with multiple outputs.
    ///
    /// This is used at epoch boundaries to automatically distribute rewards
    /// to all producers who were present during the completed epoch.
    /// Each output pays the calculated reward to a producer's address.
    ///
    /// # Arguments
    /// * `outputs` - Vector of (amount, pubkey_hash) pairs for each producer
    /// * `height` - Block height (used as extra_data for uniqueness)
    /// * `epoch` - The completed epoch number (stored in extra_data)
    pub fn new_epoch_reward_coinbase(
        outputs: Vec<(Amount, Hash)>,
        height: BlockHeight,
        epoch: u64,
    ) -> Self {
        let tx_outputs: Vec<Output> = outputs
            .into_iter()
            .map(|(amount, pubkey_hash)| Output::normal(amount, pubkey_hash))
            .collect();

        // Store both height and epoch in extra_data for auditability
        let mut extra_data = height.to_le_bytes().to_vec();
        extra_data.extend_from_slice(&epoch.to_le_bytes());

        Self {
            version: 1,
            tx_type: TxType::EpochReward, // Use EpochReward type for automatic distribution
            inputs: Vec::new(),
            outputs: tx_outputs,
            extra_data,
        }
    }

    /// Check if this is a coinbase transaction
    ///
    /// A coinbase is a Transfer transaction with no inputs and one output.
    /// Note: ClaimReward transactions also have no inputs and one output,
    /// but they have a different tx_type.
    pub fn is_coinbase(&self) -> bool {
        self.tx_type == TxType::Transfer && self.inputs.is_empty() && self.outputs.len() == 1
    }

    /// Check if this is an epoch reward coinbase (automatic distribution at epoch boundaries)
    ///
    /// Epoch reward coinbase transactions have:
    /// - TxType::EpochReward
    /// - No inputs (minted coins)
    /// - One or more outputs (rewards distributed to present producers)
    pub fn is_epoch_reward_coinbase(&self) -> bool {
        self.tx_type == TxType::EpochReward && self.inputs.is_empty() && !self.outputs.is_empty()
    }

    /// Check if this is any type of reward-minting transaction (coinbase or epoch reward)
    ///
    /// Returns true for both regular single-output coinbase and epoch reward coinbase.
    /// Use this when validating blocks to check for reward transactions.
    pub fn is_reward_minting(&self) -> bool {
        self.is_coinbase() || self.is_epoch_reward_coinbase()
    }

    /// Check if this is an exit transaction
    pub fn is_exit(&self) -> bool {
        self.tx_type == TxType::Exit
    }

    /// Check if this is a registration transaction
    pub fn is_registration(&self) -> bool {
        self.tx_type == TxType::Registration
    }

    /// Check if this is a claim reward transaction
    pub fn is_claim_reward(&self) -> bool {
        self.tx_type == TxType::ClaimReward
    }

    /// Check if this is an epoch reward transaction
    pub fn is_epoch_reward(&self) -> bool {
        self.tx_type == TxType::EpochReward
    }

    /// Create an epoch reward transaction for fair distribution
    ///
    /// This transaction type is used at epoch boundaries to distribute
    /// accumulated rewards fairly among all producers who participated
    /// in the epoch.
    pub fn new_epoch_reward(
        epoch: u64,
        recipient_pubkey: PublicKey,
        amount: Amount,
        recipient_hash: Hash,
    ) -> Self {
        let data = EpochRewardData::new(epoch, recipient_pubkey);
        Self {
            version: 1,
            tx_type: TxType::EpochReward,
            inputs: Vec::new(), // Minted, no inputs
            outputs: vec![Output::normal(amount, recipient_hash)],
            extra_data: data.to_bytes(),
        }
    }

    /// Parse epoch reward data from extra_data
    pub fn epoch_reward_data(&self) -> Option<EpochRewardData> {
        if self.tx_type != TxType::EpochReward {
            return None;
        }
        EpochRewardData::from_bytes(&self.extra_data)
    }

    /// Create a registration transaction
    ///
    /// This creates a transaction to register as a block producer.
    /// Inputs must cover the bond amount. The bond is locked for the specified duration.
    ///
    /// # Arguments
    /// - `inputs`: UTXOs to spend for the bond
    /// - `public_key`: Producer's public key
    /// - `bond_amount`: Total amount to lock as bond
    /// - `lock_until`: Block height until which the bond is locked (must be >= current_height + blocks_per_era)
    ///
    /// Note: For simplicity, this creates a basic registration. For full
    /// registration with VDF proofs, use the node's registration flow.
    pub fn new_registration(
        inputs: Vec<Input>,
        public_key: PublicKey,
        bond_amount: Amount,
        lock_until: BlockHeight,
        bond_count: u32,
    ) -> Self {
        // Create registration data without VDF (for CLI use)
        // Full VDF registration happens through node's registration flow
        let reg_data = RegistrationData {
            public_key,
            epoch: 0,
            vdf_output: Vec::new(),
            vdf_proof: Vec::new(),
            prev_registration_hash: Hash::ZERO,
            sequence_number: 0,
            bond_count,
            bls_pubkey: Vec::new(),
            bls_pop: Vec::new(),
        };
        let extra_data = bincode::serialize(&reg_data).unwrap_or_default();

        // Create one Bond UTXO per bond (each with bond_unit amount)
        // so each bond has its own creation_slot for FIFO vesting
        let pubkey_hash =
            crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, public_key.as_bytes());
        let bond_unit = if bond_count > 0 {
            bond_amount / bond_count as u64
        } else {
            bond_amount
        };
        let outputs: Vec<Output> = (0..bond_count)
            .map(|_| Output::bond(bond_unit, pubkey_hash, lock_until, 0))
            .collect();

        Self {
            version: 1,
            tx_type: TxType::Registration,
            inputs,
            outputs,
            extra_data,
        }
    }

    /// Create an exit transaction
    pub fn new_exit(public_key: PublicKey) -> Self {
        let exit_data = ExitData { public_key };
        let extra_data = bincode::serialize(&exit_data).unwrap_or_default();

        Self {
            version: 1,
            tx_type: TxType::Exit,
            inputs: Vec::new(),  // No inputs needed for exit
            outputs: Vec::new(), // No outputs - bond released after cooldown
            extra_data,
        }
    }

    /// Parse exit data from extra_data
    pub fn exit_data(&self) -> Option<ExitData> {
        if self.tx_type != TxType::Exit {
            return None;
        }
        bincode::deserialize(&self.extra_data).ok()
    }

    /// Create a claim reward transaction
    ///
    /// The producer claims their accumulated rewards as a single UTXO.
    /// The output amount is determined by the node based on pending_rewards.
    pub fn new_claim_reward(public_key: PublicKey, amount: Amount, recipient_hash: Hash) -> Self {
        let claim_data = ClaimData { public_key };
        let extra_data = bincode::serialize(&claim_data).unwrap_or_default();

        Self {
            version: 1,
            tx_type: TxType::ClaimReward,
            inputs: Vec::new(), // No inputs - rewards come from pending balance
            outputs: vec![Output::normal(amount, recipient_hash)],
            extra_data,
        }
    }

    /// Parse claim data from extra_data
    pub fn claim_data(&self) -> Option<ClaimData> {
        if self.tx_type != TxType::ClaimReward {
            return None;
        }
        bincode::deserialize(&self.extra_data).ok()
    }

    /// Create a claim bond transaction
    ///
    /// After the unbonding period completes, the producer can claim their bond.
    /// The amount depends on whether it was a normal exit (100% returned) or
    /// early exit (proportional penalty applied).
    pub fn new_claim_bond(public_key: PublicKey, amount: Amount, recipient_hash: Hash) -> Self {
        let claim_bond_data = ClaimBondData { public_key };
        let extra_data = bincode::serialize(&claim_bond_data).unwrap_or_default();

        Self {
            version: 1,
            tx_type: TxType::ClaimBond,
            inputs: Vec::new(), // No inputs - bond comes from protocol
            outputs: vec![Output::normal(amount, recipient_hash)],
            extra_data,
        }
    }

    /// Check if this is a claim bond transaction
    pub fn is_claim_bond(&self) -> bool {
        self.tx_type == TxType::ClaimBond
    }

    /// Parse claim bond data from extra_data
    pub fn claim_bond_data(&self) -> Option<ClaimBondData> {
        if self.tx_type != TxType::ClaimBond {
            return None;
        }
        bincode::deserialize(&self.extra_data).ok()
    }

    /// Create a slash producer transaction
    ///
    /// Anyone can submit slashing evidence against a misbehaving producer.
    /// If valid, the producer's bond is burned (100%).
    pub fn new_slash_producer(slash_data: SlashData) -> Self {
        let extra_data = bincode::serialize(&slash_data).unwrap_or_default();

        Self {
            version: 1,
            tx_type: TxType::SlashProducer,
            inputs: Vec::new(),  // No inputs
            outputs: Vec::new(), // No outputs - bond is burned
            extra_data,
        }
    }

    /// Check if this is a slash producer transaction
    pub fn is_slash_producer(&self) -> bool {
        self.tx_type == TxType::SlashProducer
    }

    /// Parse slash data from extra_data
    pub fn slash_data(&self) -> Option<SlashData> {
        if self.tx_type != TxType::SlashProducer {
            return None;
        }
        bincode::deserialize(&self.extra_data).ok()
    }

    // ==================== Bond Stacking Transactions ====================

    /// Create an add bond transaction.
    ///
    /// Inputs: Normal UTXOs covering bond amount + fee
    /// Outputs: Bond UTXO (locked) + optional change
    ///
    /// The producer must already be registered.
    pub fn new_add_bond(
        inputs: Vec<Input>,
        producer_pubkey: PublicKey,
        bond_count: u32,
        bond_amount: Amount,
        lock_until: BlockHeight,
    ) -> Self {
        let bond_data = AddBondData::new(producer_pubkey, bond_count);
        let extra_data = bond_data.to_bytes();

        // Create one Bond UTXO per bond (each with bond_unit amount)
        // so each bond has its own creation_slot for FIFO vesting
        let pubkey_hash =
            crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, producer_pubkey.as_bytes());
        let bond_unit = if bond_count > 0 {
            bond_amount / bond_count as u64
        } else {
            bond_amount
        };
        let outputs: Vec<Output> = (0..bond_count)
            .map(|_| Output::bond(bond_unit, pubkey_hash, lock_until, 0))
            .collect();

        Self {
            version: 1,
            tx_type: TxType::AddBond,
            inputs,
            outputs,
            extra_data,
        }
    }

    /// Check if this is an add bond transaction
    pub fn is_add_bond(&self) -> bool {
        self.tx_type == TxType::AddBond
    }

    /// Parse add bond data from extra_data
    pub fn add_bond_data(&self) -> Option<AddBondData> {
        if self.tx_type != TxType::AddBond {
            return None;
        }
        AddBondData::from_bytes(&self.extra_data)
    }

    /// Create a withdrawal request transaction.
    ///
    /// Lock/unlock model: Bond UTXOs are consumed as inputs, Normal UTXO created as output.
    /// Penalty is implicitly burned (sum(bond inputs) − net_amount = burned).
    /// Bonds are removed from registry at next epoch boundary.
    pub fn new_request_withdrawal(
        inputs: Vec<Input>,
        producer_pubkey: PublicKey,
        bond_count: u32,
        destination: Hash,
        net_amount: Amount,
    ) -> Self {
        let withdrawal_data = WithdrawalRequestData::new(producer_pubkey, bond_count, destination);
        let extra_data = withdrawal_data.to_bytes();

        Self {
            version: 1,
            tx_type: TxType::RequestWithdrawal,
            inputs, // Bond UTXOs consumed (lock → unlock)
            outputs: vec![Output::normal(net_amount, destination)],
            extra_data,
        }
    }

    /// Check if this is a withdrawal request transaction
    pub fn is_request_withdrawal(&self) -> bool {
        self.tx_type == TxType::RequestWithdrawal
    }

    /// Parse withdrawal request data from extra_data
    pub fn withdrawal_request_data(&self) -> Option<WithdrawalRequestData> {
        if self.tx_type != TxType::RequestWithdrawal {
            return None;
        }
        WithdrawalRequestData::from_bytes(&self.extra_data)
    }

    /// Parse registration data from extra_data
    pub fn registration_data(&self) -> Option<RegistrationData> {
        if self.tx_type != TxType::Registration {
            return None;
        }
        bincode::deserialize(&self.extra_data).ok()
    }

    /// Returns true if this transaction type has no UTXO inputs by design.
    ///
    /// State-only txs (Exit, RequestWithdrawal, etc.) operate on producer state
    /// and are spam-protected by requiring a registered producer bond. They bypass
    /// UTXO-based fee accounting in the mempool.
    ///
    /// Registration and AddBond are NOT state-only — they consume UTXO inputs.
    pub fn is_state_only(&self) -> bool {
        matches!(
            self.tx_type,
            TxType::Exit
                | TxType::ClaimReward
                | TxType::ClaimBond
                | TxType::SlashProducer
                | TxType::DelegateBond
                | TxType::RevokeDelegation
                | TxType::AddMaintainer
                | TxType::RemoveMaintainer
        )
    }

    /// Compute the transaction hash
    pub fn hash(&self) -> Hash {
        use crypto::Hasher;

        let mut hasher = Hasher::new();
        hasher.update(&self.version.to_le_bytes());
        hasher.update(&(self.tx_type as u32).to_le_bytes());

        // Hash inputs (without signatures for tx hash)
        hasher.update(&(self.inputs.len() as u32).to_le_bytes());
        for input in &self.inputs {
            hasher.update(input.prev_tx_hash.as_bytes());
            hasher.update(&input.output_index.to_le_bytes());
            // Signature is NOT included in tx hash
        }

        // Hash outputs
        hasher.update(&(self.outputs.len() as u32).to_le_bytes());
        for output in &self.outputs {
            hasher.update(&output.serialize());
        }

        // Hash extra data
        hasher.update(&(self.extra_data.len() as u32).to_le_bytes());
        hasher.update(&self.extra_data);

        hasher.finalize()
    }

    /// Get the message to sign for a given input.
    ///
    /// Excludes extra_data (covenant witnesses) from the hash, analogous to
    /// Bitcoin SegWit: witnesses are committed in `hash()` for immutability,
    /// but excluded from the signing message to avoid a chicken-and-egg where
    /// the witness signature depends on a hash that includes the witness itself.
    pub fn signing_message(&self) -> Hash {
        use crypto::Hasher;

        let mut hasher = Hasher::new();
        hasher.update(&self.version.to_le_bytes());
        hasher.update(&(self.tx_type as u32).to_le_bytes());

        hasher.update(&(self.inputs.len() as u32).to_le_bytes());
        for input in &self.inputs {
            hasher.update(input.prev_tx_hash.as_bytes());
            hasher.update(&input.output_index.to_le_bytes());
        }

        hasher.update(&(self.outputs.len() as u32).to_le_bytes());
        for output in &self.outputs {
            hasher.update(&output.serialize());
        }

        // extra_data (covenant witnesses) intentionally excluded — SegWit-style
        hasher.finalize()
    }

    /// Get the signing message for a specific input, respecting its sighash type.
    ///
    /// BIP-143 style: each input's signing hash includes its own outpoint
    /// (prevTxHash || outputIndex), producing unique signatures per input.
    ///
    /// - `SighashType::All`: all inputs + all outputs + THIS input's outpoint.
    /// - `SighashType::AnyoneCanPay`: only THIS input + all outputs.
    ///   Allows other parties to add inputs after the signer has committed.
    pub fn signing_message_for_input(&self, input_index: usize) -> Hash {
        let input = &self.inputs[input_index];
        match input.sighash_type {
            SighashType::All => {
                use crypto::Hasher;
                let mut hasher = Hasher::new();
                hasher.update(&self.version.to_le_bytes());
                hasher.update(&(self.tx_type as u32).to_le_bytes());

                // All inputs (same as signing_message)
                hasher.update(&(self.inputs.len() as u32).to_le_bytes());
                for inp in &self.inputs {
                    hasher.update(inp.prev_tx_hash.as_bytes());
                    hasher.update(&inp.output_index.to_le_bytes());
                }

                // All outputs
                hasher.update(&(self.outputs.len() as u32).to_le_bytes());
                for output in &self.outputs {
                    hasher.update(&output.serialize());
                }

                // BIP-143: per-input outpoint for unique signing hash
                hasher.update(input.prev_tx_hash.as_bytes());
                hasher.update(&input.output_index.to_le_bytes());

                hasher.finalize()
            }
            SighashType::AnyoneCanPay => {
                use crypto::Hasher;
                let mut hasher = Hasher::new();
                hasher.update(&self.version.to_le_bytes());
                hasher.update(&(self.tx_type as u32).to_le_bytes());

                // ANYONECANPAY: hash only this single input
                hasher.update(&1u32.to_le_bytes()); // input count = 1
                hasher.update(input.prev_tx_hash.as_bytes());
                hasher.update(&input.output_index.to_le_bytes());

                // Determine how many outputs to commit to.
                // committed_output_count == 0 → all outputs (backward compat).
                // committed_output_count > 0 → first N outputs only, allowing
                // the buyer to append outputs (e.g. change) after signing.
                let output_count = if input.committed_output_count > 0 {
                    (input.committed_output_count as usize).min(self.outputs.len())
                } else {
                    self.outputs.len()
                };

                hasher.update(&(output_count as u32).to_le_bytes());
                for output in &self.outputs[..output_count] {
                    hasher.update(&output.serialize());
                }

                hasher.finalize()
            }
        }
    }

    /// Encode covenant witness data into extra_data for a Transfer transaction.
    ///
    /// Witness map format: for each input, `[u16 LE length][witness bytes]`.
    /// For inputs spending Normal/Bond outputs, length is 0.
    /// For inputs spending conditioned outputs, length > 0 with encoded Witness.
    ///
    /// This is the DOLI equivalent of Bitcoin's SegWit: witness data lives in
    /// extra_data (which IS part of the tx hash, preventing malleability).
    pub fn set_covenant_witnesses(&mut self, witnesses: &[Vec<u8>]) {
        assert_eq!(
            witnesses.len(),
            self.inputs.len(),
            "witness count must match input count"
        );
        let mut buf = Vec::new();
        for w in witnesses {
            buf.extend_from_slice(&(w.len() as u16).to_le_bytes());
            buf.extend_from_slice(w);
        }
        self.extra_data = buf;
    }

    /// Decode covenant witness data from extra_data.
    /// Returns a witness bytes slice for each input, or None if extra_data
    /// doesn't contain witness data (normal transfers, coinbase, etc.).
    pub fn get_covenant_witness(&self, input_index: usize) -> Option<&[u8]> {
        if self.extra_data.is_empty() || self.tx_type != TxType::Transfer {
            return None;
        }
        let mut pos = 0;
        let data = &self.extra_data;
        for i in 0.. {
            if pos + 2 > data.len() {
                return None;
            }
            let len = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
            pos += 2;
            if i == input_index {
                if len == 0 {
                    return Some(&[]);
                }
                if pos + len > data.len() {
                    return None;
                }
                return Some(&data[pos..pos + len]);
            }
            pos += len;
        }
        None // unreachable in practice
    }

    /// Calculate total input amount (requires UTXO lookup - returns 0 here)
    pub fn total_output(&self) -> Amount {
        self.outputs.iter().map(|o| o.amount).sum()
    }

    /// Serialize the transaction
    pub fn serialize(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap_or_default()
    }

    /// Deserialize a transaction
    pub fn deserialize(bytes: &[u8]) -> Option<Self> {
        bincode::deserialize(bytes).ok()
    }

    /// Get the size in bytes
    pub fn size(&self) -> usize {
        self.serialize().len()
    }

    // ==================== Maintainer Transactions ====================

    /// Create a remove maintainer transaction
    ///
    /// This transaction removes a maintainer from the auto-update system.
    /// Requires 3/5 signatures from OTHER maintainers (target cannot sign).
    ///
    /// # Arguments
    /// * `target` - Public key of the maintainer to remove
    /// * `signatures` - Signatures from at least 3 other maintainers
    /// * `reason` - Optional reason for removal (for transparency)
    pub fn new_remove_maintainer(
        target: PublicKey,
        signatures: Vec<crate::maintainer::MaintainerSignature>,
        reason: Option<String>,
    ) -> Self {
        let data = if let Some(r) = reason {
            crate::maintainer::MaintainerChangeData::with_reason(target, signatures, r)
        } else {
            crate::maintainer::MaintainerChangeData::new(target, signatures)
        };

        Self {
            version: 1,
            tx_type: TxType::RemoveMaintainer,
            inputs: Vec::new(),
            outputs: Vec::new(),
            extra_data: data.to_bytes(),
        }
    }

    /// Create an add maintainer transaction
    ///
    /// This transaction adds a new maintainer to the auto-update system.
    /// Requires 3/5 signatures from current maintainers.
    /// Target must be a registered producer.
    ///
    /// # Arguments
    /// * `target` - Public key of the producer to add as maintainer
    /// * `signatures` - Signatures from at least 3 current maintainers
    pub fn new_add_maintainer(
        target: PublicKey,
        signatures: Vec<crate::maintainer::MaintainerSignature>,
    ) -> Self {
        let data = crate::maintainer::MaintainerChangeData::new(target, signatures);

        Self {
            version: 1,
            tx_type: TxType::AddMaintainer,
            inputs: Vec::new(),
            outputs: Vec::new(),
            extra_data: data.to_bytes(),
        }
    }

    /// Check if this is a remove maintainer transaction
    pub fn is_remove_maintainer(&self) -> bool {
        self.tx_type == TxType::RemoveMaintainer
    }

    /// Check if this is an add maintainer transaction
    pub fn is_add_maintainer(&self) -> bool {
        self.tx_type == TxType::AddMaintainer
    }

    /// Check if this is any maintainer change transaction
    pub fn is_maintainer_change(&self) -> bool {
        self.is_remove_maintainer() || self.is_add_maintainer()
    }

    /// Parse maintainer change data from extra_data
    pub fn maintainer_change_data(&self) -> Option<crate::maintainer::MaintainerChangeData> {
        if !self.is_maintainer_change() {
            return None;
        }
        crate::maintainer::MaintainerChangeData::from_bytes(&self.extra_data)
    }

    /// Check if this is a delegate bond transaction
    pub fn is_delegate_bond(&self) -> bool {
        self.tx_type == TxType::DelegateBond
    }

    /// Check if this is a revoke delegation transaction
    pub fn is_revoke_delegation(&self) -> bool {
        self.tx_type == TxType::RevokeDelegation
    }

    /// Parse delegate bond data from extra_data
    pub fn delegate_bond_data(&self) -> Option<DelegateBondData> {
        if !self.is_delegate_bond() {
            return None;
        }
        DelegateBondData::from_bytes(&self.extra_data)
    }

    /// Parse revoke delegation data from extra_data
    pub fn revoke_delegation_data(&self) -> Option<RevokeDelegationData> {
        if !self.is_revoke_delegation() {
            return None;
        }
        RevokeDelegationData::from_bytes(&self.extra_data)
    }

    /// Create a new delegate bond transaction
    pub fn new_delegate_bond(data: DelegateBondData) -> Self {
        Self {
            version: 1,
            tx_type: TxType::DelegateBond,
            inputs: Vec::new(),
            outputs: Vec::new(),
            extra_data: data.to_bytes(),
        }
    }

    /// Create a new revoke delegation transaction
    pub fn new_revoke_delegation(data: RevokeDelegationData) -> Self {
        Self {
            version: 1,
            tx_type: TxType::RevokeDelegation,
            inputs: Vec::new(),
            outputs: Vec::new(),
            extra_data: data.to_bytes(),
        }
    }

    // ==================== Protocol Activation Transactions ====================

    /// Create a new protocol activation transaction
    ///
    /// Schedules new consensus rules to activate at a future epoch boundary.
    /// Requires 3/5 maintainer multisig.
    pub fn new_protocol_activation(data: crate::maintainer::ProtocolActivationData) -> Self {
        Self {
            version: 1,
            tx_type: TxType::ProtocolActivation,
            inputs: Vec::new(),
            outputs: Vec::new(),
            extra_data: data.to_bytes(),
        }
    }

    /// Check if this is a protocol activation transaction
    pub fn is_protocol_activation(&self) -> bool {
        self.tx_type == TxType::ProtocolActivation
    }

    /// Parse protocol activation data from extra_data
    pub fn protocol_activation_data(&self) -> Option<crate::maintainer::ProtocolActivationData> {
        if !self.is_protocol_activation() {
            return None;
        }
        crate::maintainer::ProtocolActivationData::from_bytes(&self.extra_data)
    }
}

#[cfg(test)]
mod tests {
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
}

/// Legacy (v3.5.0) structs for backward-compatible bincode deserialization.
///
/// v3.5.0 `Input` had no `sighash_type` field. Bincode is positional, so
/// `#[serde(default)]` does NOT work — the decoder reads past the end of the
/// old struct and misinterprets bytes from the next field as the enum discriminant.
///
/// These structs mirror the v3.5.0 layout exactly. After deserializing, call
/// `.into_current()` to convert to the current types with `SighashType::All`.
pub mod legacy {
    use super::*;

    /// v3.5.0 Input — no sighash_type field.
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct LegacyInput {
        pub prev_tx_hash: Hash,
        pub output_index: u32,
        pub signature: Signature,
    }

    impl LegacyInput {
        pub fn into_current(self) -> Input {
            Input {
                prev_tx_hash: self.prev_tx_hash,
                output_index: self.output_index,
                signature: self.signature,
                sighash_type: SighashType::All,
                committed_output_count: 0,
            }
        }
    }

    /// v3.5.0 Transaction — uses LegacyInput.
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct LegacyTransaction {
        pub version: u32,
        pub tx_type: TxType,
        pub inputs: Vec<LegacyInput>,
        pub outputs: Vec<Output>,
        pub extra_data: Vec<u8>,
    }

    impl LegacyTransaction {
        pub fn into_current(self) -> Transaction {
            Transaction {
                version: self.version,
                tx_type: self.tx_type,
                inputs: self.inputs.into_iter().map(|i| i.into_current()).collect(),
                outputs: self.outputs,
                extra_data: self.extra_data,
            }
        }
    }

    /// v3.5.0 Block — uses LegacyTransaction.
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct LegacyBlock {
        pub header: crate::block::BlockHeader,
        pub transactions: Vec<LegacyTransaction>,
        #[serde(default)]
        pub aggregate_bls_signature: Vec<u8>,
    }

    impl LegacyBlock {
        pub fn into_current(self) -> crate::block::Block {
            crate::block::Block {
                header: self.header,
                transactions: self
                    .transactions
                    .into_iter()
                    .map(|t| t.into_current())
                    .collect(),
                aggregate_bls_signature: self.aggregate_bls_signature,
            }
        }
    }

    /// v3.6.0 Input — has sighash_type but no committed_output_count.
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct LegacyInputV2 {
        pub prev_tx_hash: Hash,
        pub output_index: u32,
        pub signature: Signature,
        pub sighash_type: SighashType,
    }

    impl LegacyInputV2 {
        pub fn into_current(self) -> Input {
            Input {
                prev_tx_hash: self.prev_tx_hash,
                output_index: self.output_index,
                signature: self.signature,
                sighash_type: self.sighash_type,
                committed_output_count: 0,
            }
        }
    }

    /// v3.6.0 Transaction — uses LegacyInputV2.
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct LegacyTransactionV2 {
        pub version: u32,
        pub tx_type: TxType,
        pub inputs: Vec<LegacyInputV2>,
        pub outputs: Vec<Output>,
        pub extra_data: Vec<u8>,
    }

    impl LegacyTransactionV2 {
        pub fn into_current(self) -> Transaction {
            Transaction {
                version: self.version,
                tx_type: self.tx_type,
                inputs: self.inputs.into_iter().map(|i| i.into_current()).collect(),
                outputs: self.outputs,
                extra_data: self.extra_data,
            }
        }
    }

    /// v3.6.0 Block — uses LegacyTransactionV2.
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct LegacyBlockV2 {
        pub header: crate::block::BlockHeader,
        pub transactions: Vec<LegacyTransactionV2>,
        #[serde(default)]
        pub aggregate_bls_signature: Vec<u8>,
    }

    impl LegacyBlockV2 {
        pub fn into_current(self) -> crate::block::Block {
            crate::block::Block {
                header: self.header,
                transactions: self
                    .transactions
                    .into_iter()
                    .map(|t| t.into_current())
                    .collect(),
                aggregate_bls_signature: self.aggregate_bls_signature,
            }
        }
    }

    /// Deserialize a block from bincode, trying current format first, then legacy.
    pub fn deserialize_block_compat(data: &[u8]) -> Option<crate::block::Block> {
        // Try current format first (v3.7.1+: Input has committed_output_count)
        if let Ok(block) = bincode::deserialize::<crate::block::Block>(data) {
            return Some(block);
        }
        // Fallback: v3.6.0 format (Input has sighash_type but no committed_output_count)
        if let Ok(legacy) = bincode::deserialize::<LegacyBlockV2>(data) {
            return Some(legacy.into_current());
        }
        // Fallback: v3.5.0 format (Input has no sighash_type)
        if let Ok(legacy) = bincode::deserialize::<LegacyBlock>(data) {
            return Some(legacy.into_current());
        }
        None
    }
}
