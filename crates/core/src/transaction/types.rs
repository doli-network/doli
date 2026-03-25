use crypto::{Hash, Signature};
use serde::{Deserialize, Serialize};

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
    /// Reserved — DO NOT REUSE. Tombstone for wire compat (was ClaimWithdrawal).
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
    /// Mint new units of a fungible asset (issuer-only, requires matching asset_id).
    MintAsset = 17,
    /// Burn units of a fungible asset (holder burns own tokens, provably destroyed).
    BurnAsset = 18,
    /// Create a new AMM pool with initial liquidity
    CreatePool = 19,
    /// Add liquidity to an existing pool
    AddLiquidity = 20,
    /// Remove liquidity from a pool (burn LP shares)
    RemoveLiquidity = 21,
    /// Swap assets through a pool
    Swap = 22,
    /// Create a collateralized loan
    CreateLoan = 24,
    /// Repay a loan and recover collateral
    RepayLoan = 25,
    /// Liquidate an undercollateralized loan
    LiquidateLoan = 26,
    /// Deposit DOLI into lending pool
    LendingDeposit = 27,
    /// Withdraw DOLI + interest from lending pool
    LendingWithdraw = 28,
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
            17 => Some(Self::MintAsset),
            18 => Some(Self::BurnAsset),
            19 => Some(Self::CreatePool),
            20 => Some(Self::AddLiquidity),
            21 => Some(Self::RemoveLiquidity),
            22 => Some(Self::Swap),
            24 => Some(Self::CreateLoan),
            25 => Some(Self::RepayLoan),
            26 => Some(Self::LiquidateLoan),
            27 => Some(Self::LendingDeposit),
            28 => Some(Self::LendingWithdraw),
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
    /// AMM pool output (reserves + TWAP state)
    Pool = 9,
    /// Liquidity provider share (transferable)
    LPShare = 10,
    /// Lending collateral (locked loan collateral)
    Collateral = 11,
    /// Lending pool deposit receipt (depositor provides DOLI, earns interest)
    LendingDeposit = 12,
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
            9 => Some(Self::Pool),
            10 => Some(Self::LPShare),
            11 => Some(Self::Collateral),
            12 => Some(Self::LendingDeposit),
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

    /// Returns true if `amount` on this output type is denominated in native DOLI.
    ///
    /// Non-native types store token units, LP shares, or zero (Pool) in the
    /// `amount` field.  Summing those as DOLI would corrupt supply calculations,
    /// balance queries, and fee accounting.
    pub fn is_native_amount(&self) -> bool {
        matches!(
            self,
            Self::Normal
                | Self::Bond
                | Self::Multisig
                | Self::Hashlock
                | Self::HTLC
                | Self::Vesting
                | Self::BridgeHTLC
                | Self::NFT
                | Self::LendingDeposit
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
