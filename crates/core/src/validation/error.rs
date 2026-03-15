use thiserror::Error;

use crate::types::{Amount, BlockHeight};
use crypto::Hash;

/// Validation errors.
///
/// Each variant provides specific context about what validation check failed,
/// enabling precise error reporting and debugging.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// Block genesis_hash doesn't match our chain identity.
    /// This means the block was produced by a node with different genesis parameters.
    #[error("genesis hash mismatch: got={got}, expected={expected}")]
    GenesisHashMismatch {
        /// Genesis hash in the block header.
        got: crypto::Hash,
        /// Expected genesis hash from our consensus params.
        expected: crypto::Hash,
    },

    /// Block or transaction version is unsupported.
    #[error("invalid version: {0}")]
    InvalidVersion(u32),

    /// Block timestamp does not advance from previous block.
    #[error("invalid timestamp: block={block}, expected>={expected}")]
    InvalidTimestamp {
        /// Timestamp in the block header.
        block: u64,
        /// Minimum expected timestamp.
        expected: u64,
    },

    /// Block timestamp is too far in the future.
    #[error("timestamp too far in future: {0}")]
    TimestampTooFuture(u64),

    /// Block slot does not match its timestamp.
    #[error("invalid slot derivation: got={got}, expected={expected}")]
    InvalidSlot {
        /// Slot in the block header.
        got: u32,
        /// Expected slot based on timestamp.
        expected: u32,
    },

    /// Block slot does not advance from previous block.
    #[error("slot not advancing: got={got}, prev={prev}")]
    SlotNotAdvancing {
        /// Slot in the block header.
        got: u32,
        /// Slot of the previous block.
        prev: u32,
    },

    /// Block slot is too far in the future.
    #[error("slot too far in future: got={got}, current={current}, max_future={max_future}")]
    SlotTooFuture {
        /// Slot in the block header.
        got: u32,
        /// Current slot based on wall clock.
        current: u32,
        /// Maximum allowed future slots.
        max_future: u64,
    },

    /// Block slot is too far in the past.
    #[error("slot too far in past: got={got}, current={current}, max_past={max_past}")]
    SlotTooPast {
        /// Slot in the block header.
        got: u32,
        /// Current slot based on wall clock.
        current: u32,
        /// Maximum allowed past slots.
        max_past: u64,
    },

    /// Merkle root does not match transactions.
    #[error("invalid merkle root")]
    InvalidMerkleRoot,

    /// VDF proof is invalid.
    #[error("invalid VDF proof")]
    InvalidVdfProof,

    /// Producer is not authorized for this slot.
    #[error("invalid producer for slot")]
    InvalidProducer,

    /// Block exceeds maximum size.
    #[error("block too large: {size} > {max}")]
    BlockTooLarge {
        /// Actual block size in bytes.
        size: usize,
        /// Maximum allowed size.
        max: usize,
    },

    /// Block has no transactions (must have at least coinbase).
    #[error("missing coinbase transaction")]
    MissingCoinbase,

    /// Coinbase transaction is malformed.
    #[error("invalid coinbase: {0}")]
    InvalidCoinbase(String),

    /// Block-level validation failed (reward distribution, epoch rules).
    #[error("invalid block: {0}")]
    InvalidBlock(String),

    /// Regular transaction validation failed.
    #[error("invalid transaction: {0}")]
    InvalidTransaction(String),

    /// Same output spent twice within a block.
    #[error("double spend detected")]
    DoubleSpend,

    /// Transaction outputs exceed inputs.
    #[error("insufficient funds: inputs={inputs}, outputs={outputs}")]
    InsufficientFunds {
        /// Total input amount.
        inputs: Amount,
        /// Total output amount.
        outputs: Amount,
    },

    /// Signature verification failed for an input.
    #[error("invalid signature for input {index}")]
    InvalidSignature {
        /// Index of the input with invalid signature.
        index: usize,
    },

    /// Attempting to spend a locked output before maturity.
    #[error("output locked until height {lock_height}, current height {current_height}")]
    OutputLocked {
        /// Height at which the output becomes spendable.
        lock_height: BlockHeight,
        /// Current blockchain height.
        current_height: BlockHeight,
    },

    /// Referenced output does not exist.
    #[error("output not found: tx={tx_hash}, index={output_index}")]
    OutputNotFound {
        /// Transaction hash containing the output.
        tx_hash: Hash,
        /// Output index within the transaction.
        output_index: u32,
    },

    /// Output has already been spent.
    #[error("output already spent: tx={tx_hash}, index={output_index}")]
    OutputAlreadySpent {
        /// Transaction hash containing the output.
        tx_hash: Hash,
        /// Output index within the transaction.
        output_index: u32,
    },

    /// Amount calculation would overflow.
    #[error("amount overflow in {context}")]
    AmountOverflow {
        /// Context where the overflow occurred.
        context: String,
    },

    /// Amount exceeds the maximum possible supply.
    #[error("amount exceeds max supply: {amount} > {max}")]
    AmountExceedsSupply {
        /// The invalid amount.
        amount: Amount,
        /// Maximum supply.
        max: Amount,
    },

    /// Registration transaction validation failed.
    #[error("invalid registration: {0}")]
    InvalidRegistration(String),

    /// Public key hash mismatch.
    #[error("pubkey hash mismatch: expected {expected}, got {got}")]
    PubkeyHashMismatch {
        /// Expected hash from the output.
        expected: Hash,
        /// Actual hash of the provided pubkey.
        got: Hash,
    },

    /// Bond output requirements not met.
    #[error("invalid bond: {0}")]
    InvalidBond(String),

    /// Reward claim validation failed.
    #[error("invalid claim: {0}")]
    InvalidClaim(String),

    /// Bond claim validation failed.
    #[error("invalid bond claim: {0}")]
    InvalidBondClaim(String),

    /// Slash producer validation failed.
    #[error("invalid slash: {0}")]
    InvalidSlash(String),

    /// Add bond transaction validation failed.
    #[error("invalid add bond: {0}")]
    InvalidAddBond(String),

    /// Withdrawal request validation failed.
    #[error("invalid withdrawal request: {0}")]
    InvalidWithdrawalRequest(String),

    /// Claim withdrawal validation failed.
    #[error("invalid claim withdrawal: {0}")]
    InvalidClaimWithdrawal(String),

    /// MintAsset validation failed.
    #[error("invalid mint asset: {0}")]
    InvalidMintAsset(String),

    /// BurnAsset validation failed.
    #[error("invalid burn asset: {0}")]
    InvalidBurnAsset(String),

    /// Epoch reward transaction validation failed.
    #[error("invalid epoch reward: {0}")]
    InvalidEpochReward(String),

    /// Epoch rewards present in non-boundary block.
    #[error("unexpected epoch reward: rewards only allowed at epoch boundaries")]
    UnexpectedEpochReward,

    /// Missing required epoch rewards at boundary.
    #[error(
        "missing epoch reward: block at epoch boundary must include rewards for epoch {epoch}"
    )]
    MissingEpochReward {
        /// The epoch that should have been rewarded.
        epoch: u64,
    },

    /// Epoch reward distribution doesn't match expected.
    #[error("epoch reward mismatch: {reason}")]
    EpochRewardMismatch {
        /// Description of what doesn't match.
        reason: String,
    },

    /// Maintainer change transaction validation failed.
    #[error("invalid maintainer change: {0}")]
    InvalidMaintainerChange(String),

    /// Delegation transaction validation failed.
    #[error("invalid delegation: {0}")]
    InvalidDelegation(String),

    /// Protocol activation transaction validation failed.
    #[error("invalid protocol activation: {0}")]
    InvalidProtocolActivation(String),
}
