/// Transaction types matching `doli-core::transaction::TxType`.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxType {
    /// Standard coin transfer
    Transfer = 0,
    /// Producer registration
    Registration = 1,
    /// Producer exit
    ProducerExit = 2,
    /// Coinbase (block reward) -- not user-constructible
    Coinbase = 3,
    /// NFT mint
    NftMint = 4,
    /// NFT transfer
    NftTransfer = 5,
    /// Reward claim
    RewardClaim = 6,
    /// Add bonds
    AddBond = 7,
    /// Request withdrawal
    RequestWithdrawal = 8,
    /// Reserved — DO NOT REUSE (wire compat tombstone)
    ClaimWithdrawal = 9,
    /// Slashing evidence
    SlashingEvidence = 10,
    /// Token issuance
    TokenIssuance = 11,
    /// Bridge lock (HTLC)
    BridgeLock = 12,
    /// Delegate bond
    DelegateBond = 13,
    /// Revoke delegation
    RevokeDelegation = 14,
}

impl TxType {
    /// Map wallet TxType to doli-core TxType u32 variant index for bincode serialization.
    ///
    /// Core TxType (repr(u32)): Transfer=0, Registration=1, Exit=2, ClaimReward=3,
    /// ClaimBond=4, SlashProducer=5, Coinbase=6, AddBond=7, RequestWithdrawal=8,
    /// ClaimWithdrawal=9(tombstone), EpochReward=10, RemoveMaintainer=11, AddMaintainer=12,
    /// DelegateBond=13, RevokeDelegation=14, ProtocolActivation=15
    pub(crate) fn to_core_type_id(self) -> u32 {
        match self {
            TxType::Transfer => 0,
            TxType::Registration => 1,
            TxType::ProducerExit => 2,
            TxType::Coinbase => 6,
            TxType::NftMint => 0,     // NFT uses Transfer type in core
            TxType::NftTransfer => 0, // NFT uses Transfer type in core
            TxType::RewardClaim => 3, // ClaimReward in core
            TxType::AddBond => 7,
            TxType::RequestWithdrawal => 8,
            TxType::ClaimWithdrawal => 9,
            TxType::SlashingEvidence => 5, // SlashProducer in core
            TxType::TokenIssuance => 0,    // Uses Transfer type in core
            TxType::BridgeLock => 0,       // Uses Transfer type in core
            TxType::DelegateBond => 13,
            TxType::RevokeDelegation => 14,
        }
    }
}

/// Protocol version for all wallet-constructed transactions.
pub(crate) const TX_VERSION: u32 = 1;

/// A transaction input referencing a UTXO.
#[derive(Clone, Debug)]
pub struct TxInput {
    /// Previous transaction hash (32 bytes)
    pub prev_tx_hash: [u8; 32],
    /// Output index in previous transaction
    pub output_index: u32,
    /// Signature (filled during signing)
    pub signature: Option<Vec<u8>>,
    /// Public key of signer
    pub public_key: Option<Vec<u8>>,
}

/// A transaction output.
#[derive(Clone, Debug)]
pub struct TxOutput {
    /// Amount in base units
    pub amount: u64,
    /// Recipient public key hash (32 bytes)
    pub pubkey_hash: [u8; 32],
    /// Output type (0 = normal, 1 = bond, etc.)
    pub output_type: u8,
    /// Lock until height (0 = no lock)
    pub lock_until: u64,
    /// Extra data (e.g. BLS key for registration)
    pub extra_data: Vec<u8>,
}
