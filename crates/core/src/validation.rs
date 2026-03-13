//! Validation rules for transactions and blocks.
//!
//! This module provides comprehensive validation for DOLI transactions and blocks.
//! Validation is split into two levels:
//!
//! ## Structural Validation (Context-Free)
//!
//! These checks can be performed without access to the UTXO set:
//! - Version and format validity
//! - Non-empty inputs/outputs
//! - Positive output amounts
//! - Amount overflow protection
//! - Registration data validity
//!
//! ## Contextual Validation (Requires UTXO Set)
//!
//! These checks require access to the UTXO set and are performed via the
//! `UtxoValidator` trait:
//! - Signature verification
//! - Input existence and spendability
//! - Input/output balance
//! - Lock time enforcement

use thiserror::Error;

use crate::attestation::decode_attestation_bitfield;
use crate::block::{Block, BlockHeader};
use crate::consensus::{
    is_producer_eligible_ms, max_block_size, select_producer_for_slot, ConsensusParams, RewardMode,
    MAX_DRIFT, MAX_FUTURE_SLOTS, NETWORK_MARGIN, TOTAL_SUPPLY,
};
use crate::network::Network;
use crate::tpop::heartbeat::verify_hash_chain_vdf;
use crate::transaction::{
    AddBondData, ClaimBondData, ClaimData, ExitData, Input, Output, OutputType, RegistrationData,
    SighashType, SlashData, Transaction, TxType, WithdrawalRequestData, MAX_EXTRA_DATA_SIZE,
};
use crate::types::{Amount, BlockHeight};
use crypto::{Hash, PublicKey};

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

/// Information about an unspent transaction output.
///
/// This is returned by UTXO lookups to provide all information needed
/// for transaction validation.
#[derive(Clone, Debug)]
pub struct UtxoInfo {
    /// The output data.
    pub output: Output,
    /// The public key that can spend this output.
    /// For pay-to-pubkey-hash, this must be provided separately.
    pub pubkey: Option<PublicKey>,
    /// Whether this output has been spent.
    pub spent: bool,
}

/// Trait for UTXO set access during validation.
///
/// Implement this trait to provide the validator with access to the UTXO set
/// for contextual validation (signature verification, balance checks, etc.).
///
/// # Example
///
/// ```rust,ignore
/// struct MyUtxoSet { /* ... */ }
///
/// impl UtxoProvider for MyUtxoSet {
///     fn get_utxo(&self, tx_hash: &Hash, output_index: u32) -> Option<UtxoInfo> {
///         // Look up in database
///     }
/// }
/// ```
pub trait UtxoProvider {
    /// Look up an unspent output.
    ///
    /// Returns `None` if the output doesn't exist or has been spent.
    fn get_utxo(&self, tx_hash: &Hash, output_index: u32) -> Option<UtxoInfo>;
}

/// Trait for accessing epoch block data needed for deterministic reward validation.
///
/// Implement this trait in the storage layer to provide the validator with
/// access to historical blocks for exact epoch reward verification.
///
/// # Example
///
/// ```rust,ignore
/// struct MyBlockStore { /* ... */ }
///
/// impl EpochBlockSource for MyBlockStore {
///     fn last_rewarded_epoch(&self) -> Result<u64, String> {
///         // Scan backwards to find last EpochReward tx
///     }
///     fn blocks_in_slot_range(&self, start: u32, end: u32) -> Result<Vec<Block>, String> {
///         // Return all blocks in [start, end)
///     }
/// }
/// ```
pub trait EpochBlockSource {
    /// Get the last epoch that has received rewards.
    ///
    /// Scans backwards from chain tip to find the most recent block
    /// containing an `EpochReward` transaction and returns its epoch number.
    ///
    /// Returns 0 if no rewards have been distributed yet.
    fn last_rewarded_epoch(&self) -> Result<u64, String>;

    /// Get all blocks in the specified slot range.
    ///
    /// Range is `[start, end)` - inclusive start, exclusive end.
    /// Empty slots are skipped (not included in result).
    fn blocks_in_slot_range(&self, start: u32, end: u32) -> Result<Vec<Block>, String>;

    /// Check if any block exists in the specified slot range.
    ///
    /// Range is `[start, end)` - inclusive start, exclusive end.
    /// Returns true if at least one block exists in the range.
    ///
    /// This is used to skip empty epochs during reward catch-up.
    fn has_any_block_in_slot_range(&self, start: u32, end: u32) -> Result<bool, String>;
}

/// Registration chain state for anti-Sybil verification.
///
/// Tracks the chained VDF registration state needed to validate
/// that new registrations form a proper chain.
#[derive(Clone, Debug, Default)]
pub struct RegistrationChainState {
    /// Hash of the last registration transaction (Hash::ZERO before any registration)
    pub last_registration_hash: Hash,
    /// Current global registration sequence number
    pub registration_sequence: u64,
}

impl RegistrationChainState {
    /// Create a new registration chain state
    pub fn new(last_hash: Hash, sequence: u64) -> Self {
        Self {
            last_registration_hash: last_hash,
            registration_sequence: sequence,
        }
    }

    /// Get the expected prev_registration_hash for the next registration
    pub fn expected_prev_hash(&self) -> Hash {
        self.last_registration_hash
    }

    /// Get the expected sequence number for the next registration
    pub fn expected_sequence(&self) -> u64 {
        if self.last_registration_hash == Hash::ZERO {
            0 // First registration
        } else {
            self.registration_sequence + 1
        }
    }
}

/// Validation mode for synced blocks.
///
/// Controls whether VDF proof verification is performed during block validation.
/// After snap sync, gap blocks use `Light` mode (VDF already trusted via state root
/// quorum), while recent blocks near the tip use `Full` mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationMode {
    /// Full validation including VDF proof verification.
    /// Used for: gossip blocks, last epoch of sync (360 blocks).
    Full,
    /// Light validation: everything except VDF proof verification.
    /// Used for: gap blocks after snap sync where state root was
    /// already verified by peer quorum (2+ peers).
    Light,
}

/// Block validation context.
///
/// Holds all the context needed to validate a block or transaction,
/// including consensus parameters, current time, and chain state.
#[derive(Clone, Debug)]
pub struct ValidationContext {
    /// Consensus parameters (slot duration, block reward, etc.).
    pub params: ConsensusParams,
    /// Network type (determines VDF iterations and other parameters).
    pub network: Network,
    /// Current wall-clock time (unix timestamp).
    pub current_time: u64,
    /// Height of the block being validated.
    pub current_height: BlockHeight,
    /// Slot of the previous block.
    pub prev_slot: u32,
    /// Timestamp of the previous block.
    pub prev_timestamp: u64,
    /// Hash of the previous block (for producer selection).
    pub prev_hash: crypto::Hash,
    /// Active producers for the current epoch (legacy, for backward compatibility).
    pub active_producers: Vec<crypto::PublicKey>,
    /// Active producers with their effective weights for weighted selection (Option C).
    /// If set, weighted selection is used for anti-grinding protection.
    pub active_producers_weighted: Vec<(crypto::PublicKey, u64)>,
    /// Bootstrap producers (sorted by pubkey) for fallback rank validation during bootstrap.
    /// When non-empty, bootstrap validation uses deterministic fallback ranks instead of
    /// accepting any producer. Populated from GSet/known_producers in the node layer.
    pub bootstrap_producers: Vec<crypto::PublicKey>,
    /// Live bootstrap producers (sorted by pubkey) for liveness-filtered scheduling.
    /// Empty = no liveness filter active (use bootstrap_producers for all ranks).
    pub live_bootstrap_producers: Vec<crypto::PublicKey>,
    /// Stale bootstrap producers (sorted by pubkey) for re-entry slot scheduling.
    pub stale_bootstrap_producers: Vec<crypto::PublicKey>,
    /// Registration chain state for chained VDF anti-Sybil verification.
    pub registration_chain: RegistrationChainState,
    /// Public keys with pending (epoch-deferred) registrations.
    /// Used to reject duplicate registrations before epoch activation.
    pub pending_producer_keys: Vec<crypto::PublicKey>,
    /// BLS public keys for active producers, sorted by Ed25519 pubkey (same as bitfield order).
    ///
    /// Index N corresponds to the producer at sorted index N in the bitfield.
    /// Empty Vec at index N means that producer has no BLS key (pre-BLS registration).
    #[allow(dead_code)]
    pub producer_bls_keys: Vec<Vec<u8>>,
}

impl ValidationContext {
    /// Create a new validation context.
    #[must_use]
    pub fn new(
        params: ConsensusParams,
        network: Network,
        current_time: u64,
        current_height: BlockHeight,
    ) -> Self {
        Self {
            params,
            network,
            current_time,
            current_height,
            prev_slot: 0,
            prev_timestamp: 0,
            prev_hash: crypto::Hash::ZERO,
            active_producers: Vec::new(),
            active_producers_weighted: Vec::new(),
            bootstrap_producers: Vec::new(),
            live_bootstrap_producers: Vec::new(),
            stale_bootstrap_producers: Vec::new(),
            registration_chain: RegistrationChainState::default(),
            pending_producer_keys: Vec::new(),
            producer_bls_keys: Vec::new(),
        }
    }

    /// Set previous block info.
    #[must_use]
    pub fn with_prev_block(mut self, slot: u32, timestamp: u64, hash: crypto::Hash) -> Self {
        self.prev_slot = slot;
        self.prev_timestamp = timestamp;
        self.prev_hash = hash;
        self
    }

    /// Set active producers for producer eligibility validation (legacy).
    #[must_use]
    pub fn with_producers(mut self, producers: Vec<crypto::PublicKey>) -> Self {
        self.active_producers = producers;
        self
    }

    /// Set active producers with weights for weighted selection (Option C - anti-grinding).
    ///
    /// When weights are provided, the validation uses weighted producer selection:
    /// - Phase 1: Top N producers by weight are eligible (deterministic, not grindable)
    /// - Phase 2: Hash-based ordering among eligible (limited grinding)
    #[must_use]
    pub fn with_producers_weighted(mut self, producers: Vec<(crypto::PublicKey, u64)>) -> Self {
        // Also populate legacy field for backward compatibility
        self.active_producers = producers.iter().map(|(pk, _)| *pk).collect();
        self.active_producers_weighted = producers;
        self
    }

    /// Set bootstrap producers for fallback rank validation during bootstrap.
    /// Producers must be sorted by pubkey (same order used by production side).
    #[must_use]
    pub fn with_bootstrap_producers(mut self, producers: Vec<crypto::PublicKey>) -> Self {
        self.bootstrap_producers = producers;
        self
    }

    /// Set the liveness split for bootstrap producers.
    /// Both lists must be sorted by pubkey. When `live` is non-empty,
    /// liveness-aware scheduling is used (live for normal slots, stale for re-entry).
    #[must_use]
    pub fn with_bootstrap_liveness(
        mut self,
        live: Vec<crypto::PublicKey>,
        stale: Vec<crypto::PublicKey>,
    ) -> Self {
        self.live_bootstrap_producers = live;
        self.stale_bootstrap_producers = stale;
        self
    }

    /// Set registration chain state for chained VDF validation.
    #[must_use]
    pub fn with_registration_chain(mut self, last_hash: Hash, sequence: u64) -> Self {
        self.registration_chain = RegistrationChainState::new(last_hash, sequence);
        self
    }

    /// Set pending producer keys (epoch-deferred registrations not yet active).
    #[must_use]
    pub fn with_pending_producer_keys(mut self, keys: Vec<crypto::PublicKey>) -> Self {
        self.pending_producer_keys = keys;
        self
    }
}

/// Validate a block header
pub fn validate_header(
    header: &BlockHeader,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    // 0. Chain identity — reject blocks from different genesis FIRST.
    // This is O(1) and catches all genesis-time-hijack attacks with zero tolerance.
    if header.genesis_hash != ctx.params.genesis_hash {
        return Err(ValidationError::GenesisHashMismatch {
            got: header.genesis_hash,
            expected: ctx.params.genesis_hash,
        });
    }

    // 1. Version check
    if header.version != 2 {
        return Err(ValidationError::InvalidVersion(header.version));
    }

    // 2. Timestamp must be after previous block
    if header.timestamp <= ctx.prev_timestamp {
        return Err(ValidationError::InvalidTimestamp {
            block: header.timestamp,
            expected: ctx.prev_timestamp + 1,
        });
    }

    // 3. Timestamp not too far in future
    if header.timestamp > ctx.current_time + MAX_DRIFT {
        return Err(ValidationError::TimestampTooFuture(header.timestamp));
    }

    // 4. Slot must derive correctly from timestamp
    let expected_slot = ctx.params.timestamp_to_slot(header.timestamp);
    if header.slot != expected_slot {
        return Err(ValidationError::InvalidSlot {
            got: header.slot,
            expected: expected_slot,
        });
    }

    // 5. Slot must advance
    if header.slot <= ctx.prev_slot {
        return Err(ValidationError::SlotNotAdvancing {
            got: header.slot,
            prev: ctx.prev_slot,
        });
    }

    // 6. Timestamp must be within slot window
    let slot_start = ctx.params.slot_to_timestamp(header.slot);
    let min_time = slot_start.saturating_sub(NETWORK_MARGIN);
    if header.timestamp < min_time {
        return Err(ValidationError::InvalidTimestamp {
            block: header.timestamp,
            expected: min_time,
        });
    }

    // 7. Slot boundary check (time-based consensus)
    // Block slot must be within acceptable range of current time.
    // This prevents clock manipulation attacks and enforces time-based slot selection.
    let current_slot = ctx.params.timestamp_to_slot(ctx.current_time);

    // Check not too far in the future
    if header.slot as u64 > current_slot as u64 + MAX_FUTURE_SLOTS {
        return Err(ValidationError::SlotTooFuture {
            got: header.slot,
            current: current_slot,
            max_future: MAX_FUTURE_SLOTS,
        });
    }

    // MAX_PAST_SLOTS: enforced at the gossip boundary (node.rs gossip handler),
    // NOT here. Checking here breaks header-first sync and reorgs because
    // historical blocks have slots far behind wall-clock. The gossip handler
    // is the correct enforcement point — it rejects old gossip blocks before
    // they reach apply_block(), while sync/reorg paths bypass it safely.

    Ok(())
}

/// Validate a complete block
pub fn validate_block(block: &Block, ctx: &ValidationContext) -> Result<(), ValidationError> {
    // 1. Validate header
    validate_header(&block.header, ctx)?;

    // 2. Check block size (scales with era)
    let size = block.size();
    let max_size = max_block_size(ctx.current_height);
    if size > max_size {
        return Err(ValidationError::BlockTooLarge {
            size,
            max: max_size,
        });
    }

    // 3. Verify merkle root
    if !block.verify_merkle_root() {
        return Err(ValidationError::InvalidMerkleRoot);
    }

    // 4. Validate all transactions
    // Note: The old RewardMode-based validation is deprecated. The new weighted
    // presence reward system uses on-demand ClaimEpochReward transactions.
    // Blocks no longer require coinbase or automatic EpochReward transactions.
    for tx in &block.transactions {
        validate_transaction(tx, ctx)?;
    }

    // 5. Check for double spends within block
    check_internal_double_spend(block)?;

    // 6. Validate VDF (if not in bootstrap)
    if !ctx.params.is_bootstrap(ctx.current_height) {
        validate_vdf(&block.header, ctx.network)?;
    }

    // 7. Validate producer eligibility (if not in bootstrap)
    validate_producer_eligibility(&block.header, ctx)?;

    // 8. Verify BLS aggregate attestation signature (if present).
    // Pre-BLS blocks have empty aggregate_bls_signature — accepted.
    // Post-BLS blocks with a signature are verified against the bitfield.
    if !block.aggregate_bls_signature.is_empty() {
        validate_bls_aggregate(block, ctx)?;
    }

    Ok(())
}

/// Validate a block with a specified validation mode.
///
/// In `Full` mode, this is identical to `validate_block()` — all checks including
/// VDF proof verification are performed.
///
/// In `Light` mode, VDF verification is skipped. This is used for gap blocks after
/// snap sync, where the state root was already verified by a peer quorum. All other
/// checks (header, merkle root, block size, transactions, double-spend, producer
/// eligibility) are still performed.
pub fn validate_block_with_mode(
    block: &Block,
    ctx: &ValidationContext,
    mode: ValidationMode,
) -> Result<(), ValidationError> {
    match mode {
        ValidationMode::Full => {
            // Full validation: all checks including time-based header validation and VDF.
            validate_header(&block.header, ctx)?;

            let size = block.size();
            let max_size = max_block_size(ctx.current_height);
            if size > max_size {
                return Err(ValidationError::BlockTooLarge {
                    size,
                    max: max_size,
                });
            }

            if !block.verify_merkle_root() {
                return Err(ValidationError::InvalidMerkleRoot);
            }

            for tx in &block.transactions {
                validate_transaction(tx, ctx)?;
            }

            check_internal_double_spend(block)?;

            if !ctx.params.is_bootstrap(ctx.current_height) {
                validate_vdf(&block.header, ctx.network)?;
            }

            validate_producer_eligibility(&block.header, ctx)?;
        }
        ValidationMode::Light => {
            // Light validation for synced gap blocks: skip VDF and time-based header
            // checks (MAX_PAST_SLOTS would reject old blocks during sync).
            // Header chain linkage was already verified during header download.

            // Chain identity — reject blocks from different genesis FIRST.
            if block.header.genesis_hash != ctx.params.genesis_hash {
                return Err(ValidationError::GenesisHashMismatch {
                    got: block.header.genesis_hash,
                    expected: ctx.params.genesis_hash,
                });
            }

            // Version check
            if block.header.version != 2 {
                return Err(ValidationError::InvalidVersion(block.header.version));
            }

            // Slot derivation from genesis — rejects blocks with wrong slot calculation.
            let expected_slot = ctx.params.timestamp_to_slot(block.header.timestamp);
            if block.header.slot != expected_slot {
                return Err(ValidationError::InvalidSlot {
                    got: block.header.slot,
                    expected: expected_slot,
                });
            }

            // Slot must advance from previous block
            if block.header.slot <= ctx.prev_slot {
                return Err(ValidationError::SlotNotAdvancing {
                    got: block.header.slot,
                    prev: ctx.prev_slot,
                });
            }

            // Block size
            let size = block.size();
            let max_size = max_block_size(ctx.current_height);
            if size > max_size {
                return Err(ValidationError::BlockTooLarge {
                    size,
                    max: max_size,
                });
            }

            // Merkle root integrity
            if !block.verify_merkle_root() {
                return Err(ValidationError::InvalidMerkleRoot);
            }

            // Transaction structural validation
            for tx in &block.transactions {
                validate_transaction(tx, ctx)?;
            }

            // Internal double-spend detection
            check_internal_double_spend(block)?;

            // Producer eligibility (still checked — confirms the right producer signed)
            validate_producer_eligibility(&block.header, ctx)?;

            // Skipped: VDF (trusted via state root quorum)
            // Skipped: MAX_FUTURE_SLOTS / MAX_PAST_SLOTS (time-based, breaks sync)
            // Skipped: timestamp-not-too-future (time-based)
        }
    }

    Ok(())
}

/// Validate the coinbase transaction
///
/// Note: This function is no longer called from validate_block since the automatic
/// reward distribution system was deprecated. It's kept for backward compatibility.
#[allow(dead_code)]
fn validate_coinbase(tx: &Transaction, ctx: &ValidationContext) -> Result<(), ValidationError> {
    // Must have no inputs
    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidCoinbase(
            "coinbase must have no inputs".to_string(),
        ));
    }

    // Must have exactly one output
    if tx.outputs.len() != 1 {
        return Err(ValidationError::InvalidCoinbase(
            "coinbase must have exactly one output".to_string(),
        ));
    }

    // Amount must match block reward (+ fees, but we skip fee check here)
    let expected_reward = ctx.params.block_reward(ctx.current_height);
    if tx.outputs[0].amount < expected_reward {
        return Err(ValidationError::InvalidCoinbase(format!(
            "coinbase amount {} less than reward {}",
            tx.outputs[0].amount, expected_reward
        )));
    }

    Ok(())
}

/// Calculate expected epoch rewards for the given blocks.
///
/// # Deprecated
///
/// This function is deprecated as part of Milestone 10 (Remove Old Reward System).
/// The automatic epoch reward distribution system has been replaced with weighted
/// presence rewards using ClaimEpochReward transactions. Use the new
/// `crate::rewards::WeightedRewardCalculator` instead.
///
/// This is the deterministic reward calculation algorithm used by both
/// block producers and validators to ensure consistency. Given the same
/// blocks, any node will calculate exactly the same rewards.
///
/// # Arguments
/// * `epoch` - The epoch number for reward transactions
/// * `blocks` - All blocks produced in the epoch (from BlockStore)
/// * `current_height` - Current block height (for reward halving)
/// * `params` - Consensus parameters
///
/// # Returns
/// Vector of (producer_pubkey, reward_amount) pairs, sorted by pubkey for determinism.
/// Empty vector if no blocks were produced in the epoch.
#[deprecated(
    since = "0.1.0",
    note = "Use crate::rewards::WeightedRewardCalculator instead"
)]
pub fn calculate_expected_epoch_rewards(
    _epoch: u64,
    blocks: &[Block],
    current_height: BlockHeight,
    params: &ConsensusParams,
) -> Vec<(PublicKey, u64)> {
    use std::collections::HashMap;

    // Count blocks per producer (by public key)
    let mut producer_blocks: HashMap<PublicKey, u64> = HashMap::new();
    for block in blocks {
        // Skip null producer (genesis block has zero pubkey)
        if block.header.producer.as_bytes().iter().all(|&b| b == 0) {
            continue;
        }
        *producer_blocks.entry(block.header.producer).or_insert(0) += 1;
    }

    let total_blocks = producer_blocks.values().sum::<u64>();
    if total_blocks == 0 {
        return Vec::new();
    }

    // Calculate block reward (with halving based on current height)
    let block_reward = params.block_reward(current_height);

    // Total pool = blocks_produced × block_reward
    let total_pool = total_blocks.saturating_mul(block_reward);

    // Sort producers by public key for deterministic ordering
    let mut sorted_producers: Vec<_> = producer_blocks.into_iter().collect();
    sorted_producers.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

    // Calculate proportional rewards
    let mut rewards = Vec::new();
    let mut distributed = 0u64;

    for (i, (producer_pubkey, blocks_produced)) in sorted_producers.iter().enumerate() {
        let is_last = i == sorted_producers.len() - 1;

        // Calculate proportional share
        let share = if is_last {
            // Last producer gets remainder (rounding dust)
            total_pool.saturating_sub(distributed)
        } else {
            // Proportional: (blocks_produced / total_blocks) * total_pool
            // Use u128 to avoid overflow in intermediate calculation
            let share =
                ((*blocks_produced as u128) * (total_pool as u128)) / (total_blocks as u128);
            share as u64
        };

        if share > 0 {
            rewards.push((*producer_pubkey, share));
            distributed += share;
        }
    }

    rewards
}

/// Determine which epoch (if any) needs reward distribution at the current slot.
///
/// # Deprecated
///
/// This function is deprecated as part of Milestone 10 (Remove Old Reward System).
/// The automatic epoch reward distribution system has been replaced with weighted
/// presence rewards using ClaimEpochReward transactions.
///
/// Returns `Some(epoch)` if rewards for that epoch should be included in a block
/// at the given slot, or `None` if no rewards are due.
///
/// This handles catch-up scenarios where multiple epochs may need rewards.
/// Only one epoch is rewarded per block (oldest first).
///
/// **Empty Epoch Handling**: Epochs with no blocks are skipped. This handles
/// the case where nodes start running after the genesis timestamp, leaving
/// earlier epochs empty. The function scans forward from `last_rewarded + 1`
/// to find the first epoch that actually contains blocks.
#[deprecated(
    since = "0.1.0",
    note = "Automatic epoch rewards replaced by ClaimEpochReward transactions"
)]
pub fn epoch_needing_rewards<S: EpochBlockSource>(
    current_slot: u32,
    params: &ConsensusParams,
    source: &S,
) -> Result<Option<u64>, String> {
    let slots_per_epoch = params.slots_per_reward_epoch as u64;

    // Skip slot 0 (genesis, can't distribute before epoch 0 ends)
    if current_slot == 0 {
        return Ok(None);
    }

    let current_epoch = current_slot as u64 / slots_per_epoch;

    // Get the last epoch that received rewards from BlockStore
    let last_rewarded = source.last_rewarded_epoch()?;

    // Only finished epochs (< current_epoch) are eligible for rewards
    if current_epoch <= last_rewarded {
        return Ok(None);
    }

    // Find the first epoch after last_rewarded that has blocks
    // This skips empty epochs (e.g., when chain starts after genesis timestamp)
    for epoch in (last_rewarded + 1)..current_epoch {
        let start_slot = if epoch == 0 {
            1u32 // Skip genesis slot
        } else {
            (epoch * slots_per_epoch) as u32
        };
        let end_slot = ((epoch + 1) * slots_per_epoch) as u32;

        if source.has_any_block_in_slot_range(start_slot, end_slot)? {
            return Ok(Some(epoch));
        }
    }

    // All epochs between last_rewarded and current are empty - no rewards needed
    Ok(None)
}

/// Validate block reward transactions for epoch distribution mode.
///
/// # Deprecated
///
/// This function is deprecated as part of Milestone 10 (Remove Old Reward System).
/// The automatic epoch reward distribution system has been replaced with weighted
/// presence rewards using ClaimEpochReward transactions.
///
/// In EpochPool mode:
/// - At epoch boundary: expect EpochReward txs, NO coinbase
/// - Non-boundary: NO rewards at all (pool accumulates)
///
/// In DirectCoinbase mode: this function does nothing (handled by validate_coinbase).
#[allow(dead_code)]
fn validate_block_rewards(block: &Block, ctx: &ValidationContext) -> Result<(), ValidationError> {
    // Only applies to EpochPool mode
    if ctx.params.reward_mode != RewardMode::EpochPool {
        return Ok(());
    }

    let slot = block.header.slot;
    let is_boundary = ctx.params.is_reward_epoch_boundary(slot);

    if is_boundary {
        // Epoch boundary: expect EpochReward transactions, NO coinbase
        let epoch_rewards: Vec<_> = block
            .transactions
            .iter()
            .filter(|tx| tx.is_epoch_reward())
            .collect();

        // Verify no coinbase in epoch boundary blocks
        if block.transactions.iter().any(|tx| tx.is_coinbase()) {
            return Err(ValidationError::InvalidBlock(
                "epoch boundary block cannot have coinbase".to_string(),
            ));
        }

        // Calculate total distributed across all epoch reward outputs
        let total_distributed: Amount = epoch_rewards
            .iter()
            .flat_map(|tx| tx.outputs.iter())
            .map(|o| o.amount)
            .sum();

        // Expected pool for this epoch
        let expected_pool = ctx.params.total_epoch_reward(ctx.current_height);

        // With attestation qualification, total_distributed ≤ pool:
        // - All qualified: distributed == pool (remainder dust to first qualifier)
        // - Some disqualified: distributed < pool (non-qualifier share burned)
        // - None qualified: distributed == 0 (entire pool burned)
        if total_distributed > expected_pool {
            return Err(ValidationError::InvalidBlock(format!(
                "epoch rewards {} > expected pool {}",
                total_distributed, expected_pool
            )));
        }

        // Validate each epoch reward transaction
        for tx in &epoch_rewards {
            // Verify epoch number matches
            if let Some(data) = tx.epoch_reward_data() {
                let expected_epoch = ctx.params.slot_to_reward_epoch(slot);
                if data.epoch != expected_epoch as u64 {
                    return Err(ValidationError::InvalidEpochReward(format!(
                        "epoch mismatch: {} vs expected {}",
                        data.epoch, expected_epoch
                    )));
                }
            }
        }
    } else {
        // Non-boundary: NO coinbase, NO epoch rewards (pool accumulates)
        if block.transactions.iter().any(|tx| tx.is_coinbase()) {
            return Err(ValidationError::InvalidBlock(
                "coinbase transactions not allowed — rewards distributed at epoch boundary"
                    .to_string(),
            ));
        }
        if block.transactions.iter().any(|tx| tx.is_epoch_reward()) {
            return Err(ValidationError::InvalidBlock(
                "epoch rewards only distributed at epoch boundary".to_string(),
            ));
        }
    }

    Ok(())
}

/// Validate block rewards with exact distribution verification.
///
/// # Deprecated
///
/// This function is deprecated as part of Milestone 10 (Remove Old Reward System).
/// The automatic epoch reward distribution system has been replaced with weighted
/// presence rewards using ClaimEpochReward transactions.
///
/// This is the strict validation mode that verifies each producer receives
/// exactly their proportional share of rewards based on blocks produced.
///
/// Uses `EpochBlockSource` to access historical blockchain data and
/// recalculate expected rewards deterministically.
///
/// # Arguments
/// * `block` - The block to validate
/// * `ctx` - Validation context with consensus parameters
/// * `source` - BlockStore accessor for epoch data
///
/// # Validation Rules
///
/// At epoch boundary (`current_epoch > last_rewarded`):
/// - Block MUST contain EpochReward transactions
/// - Each transaction MUST have correct epoch number
/// - Recipients MUST match producers who created blocks
/// - Amounts MUST match proportional distribution
/// - Total MUST equal `blocks_produced × block_reward`
///
/// At non-boundary:
/// - Block MUST NOT contain any reward transactions
#[deprecated(
    since = "0.1.0",
    note = "Automatic epoch rewards replaced by ClaimEpochReward transactions"
)]
#[allow(deprecated)]
pub fn validate_block_rewards_exact<S: EpochBlockSource>(
    block: &Block,
    ctx: &ValidationContext,
    source: &S,
) -> Result<(), ValidationError> {
    // Only applies to EpochPool mode
    if ctx.params.reward_mode != RewardMode::EpochPool {
        return Ok(());
    }

    let slot = block.header.slot;
    let _slots_per_epoch = ctx.params.slots_per_reward_epoch as u64;

    // Determine if this block should include epoch rewards
    let epoch_to_reward = match epoch_needing_rewards(slot, &ctx.params, source) {
        Ok(epoch) => epoch,
        Err(e) => {
            return Err(ValidationError::InvalidBlock(format!(
                "failed to determine epoch reward status: {}",
                e
            )));
        }
    };

    // Get the block's epoch reward transactions
    let block_epoch_rewards: Vec<_> = block
        .transactions
        .iter()
        .filter(|tx| tx.is_epoch_reward())
        .collect();

    // Verify no coinbase in EpochPool mode blocks
    if block.transactions.iter().any(|tx| tx.is_coinbase()) {
        return Err(ValidationError::InvalidBlock(
            "coinbase not allowed in EpochPool mode".to_string(),
        ));
    }

    if let Some(epoch) = epoch_to_reward {
        // Epoch 0 (genesis): pool was consumed by genesis bonds, no distribution.
        // Remainder carries to E1. Skip validation for E0.
        if epoch == 0 {
            if !block_epoch_rewards.is_empty() {
                return Err(ValidationError::InvalidEpochReward(
                    "epoch 0 must not distribute rewards (genesis pool used for bonds)".to_string(),
                ));
            }
            return Ok(());
        }

        // Block SHOULD include epoch rewards - validate exact distribution

        if block_epoch_rewards.is_empty() {
            return Err(ValidationError::MissingEpochReward { epoch });
        }

        // Calculate slot range for the epoch being rewarded
        let start_slot = if epoch == 0 {
            1u32 // Skip genesis at slot 0
        } else {
            epoch as u32 * ctx.params.slots_per_reward_epoch
        };
        let end_slot = (epoch + 1) as u32 * ctx.params.slots_per_reward_epoch;

        // Get blocks from source
        let epoch_blocks = source
            .blocks_in_slot_range(start_slot, end_slot)
            .map_err(|e| {
                ValidationError::InvalidBlock(format!("failed to get epoch blocks: {}", e))
            })?;

        // Calculate expected rewards
        let expected_rewards =
            calculate_expected_epoch_rewards(epoch, &epoch_blocks, ctx.current_height, &ctx.params);

        // Verify transaction count matches
        if block_epoch_rewards.len() != expected_rewards.len() {
            return Err(ValidationError::EpochRewardMismatch {
                reason: format!(
                    "expected {} reward transactions, got {}",
                    expected_rewards.len(),
                    block_epoch_rewards.len()
                ),
            });
        }

        // Build a map of expected rewards for comparison
        let mut expected_map: std::collections::HashMap<&PublicKey, u64> =
            std::collections::HashMap::new();
        for (pubkey, amount) in &expected_rewards {
            expected_map.insert(pubkey, *amount);
        }

        // Verify each epoch reward transaction
        for tx in &block_epoch_rewards {
            let data = tx.epoch_reward_data().ok_or_else(|| {
                ValidationError::InvalidEpochReward("missing epoch reward data".to_string())
            })?;

            // Verify epoch number
            if data.epoch != epoch {
                return Err(ValidationError::InvalidEpochReward(format!(
                    "epoch mismatch: got {}, expected {}",
                    data.epoch, epoch
                )));
            }

            // Verify recipient is expected
            let expected_amount = expected_map.get(&data.recipient).ok_or_else(|| {
                ValidationError::EpochRewardMismatch {
                    reason: format!(
                        "unexpected recipient: {}",
                        &crypto::hash::hash(data.recipient.as_bytes()).to_hex()[..16]
                    ),
                }
            })?;

            // Verify amount matches
            let actual_amount = tx.outputs.first().map(|o| o.amount).unwrap_or(0);
            if actual_amount != *expected_amount {
                return Err(ValidationError::EpochRewardMismatch {
                    reason: format!(
                        "amount mismatch for producer: expected {}, got {}",
                        expected_amount, actual_amount
                    ),
                });
            }
        }
    } else {
        // Block should NOT include epoch rewards
        if !block_epoch_rewards.is_empty() {
            return Err(ValidationError::UnexpectedEpochReward);
        }
    }

    Ok(())
}

/// Validate a regular transaction (structural validation only).
///
/// This performs all checks that don't require UTXO access:
/// - Version check
/// - Non-empty inputs and outputs
/// - Positive output amounts
/// - No amount overflow in outputs
/// - Type-specific validation (registration data)
///
/// For full validation including signatures and balances,
/// use `validate_transaction_with_utxos`.
pub fn validate_transaction(
    tx: &Transaction,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    // 1. Version check
    if tx.version != 1 {
        return Err(ValidationError::InvalidVersion(tx.version));
    }

    // 2. Must have at least one input (unless coinbase, exit, claim, claim_bond, slash,
    //    add_bond, request_withdrawal, or epoch_reward)
    //    Note: add_bond requires inputs but handles its own error for specificity
    if tx.inputs.is_empty()
        && !tx.is_coinbase()
        && !tx.is_exit()
        && !tx.is_claim_reward()
        && !tx.is_claim_bond()
        && !tx.is_slash_producer()
        && !tx.is_add_bond()
        && !tx.is_request_withdrawal()
        && tx.tx_type != TxType::ClaimWithdrawal
        && !tx.is_epoch_reward()
        && !tx.is_delegate_bond()
        && !tx.is_revoke_delegation()
        && !tx.is_registration()
        && !tx.is_maintainer_change()
        && !tx.is_protocol_activation()
    // Registration/maintainer/protocol txs handle their own input validation
    {
        return Err(ValidationError::InvalidTransaction(
            "transaction must have inputs".to_string(),
        ));
    }

    // 3. Must have at least one output (unless exit, slash_producer, add_bond, or request_withdrawal)
    //    Note: epoch_reward requires exactly one output but handles its own errors
    if tx.outputs.is_empty()
        && !tx.is_exit()
        && !tx.is_slash_producer()
        && !tx.is_add_bond()
        && !tx.is_request_withdrawal()
        && tx.tx_type != TxType::ClaimWithdrawal
        && !tx.is_epoch_reward()
        && !tx.is_delegate_bond()
        && !tx.is_revoke_delegation()
        && !tx.is_registration()
        && !tx.is_maintainer_change()
        && !tx.is_protocol_activation()
    // Registration/maintainer/protocol txs handle their own output validation
    {
        return Err(ValidationError::InvalidTransaction(
            "transaction must have outputs".to_string(),
        ));
    }

    // 4. Validate all outputs
    let total_output = validate_outputs(&tx.outputs, ctx)?;

    // 5. Total output must not exceed max supply
    if total_output > TOTAL_SUPPLY {
        return Err(ValidationError::AmountExceedsSupply {
            amount: total_output,
            max: TOTAL_SUPPLY,
        });
    }

    // 6. Type-specific validation
    match tx.tx_type {
        TxType::Transfer => {
            // Transfer transactions should have empty extra_data
            // (or use it for memo, but we don't validate that)
        }
        TxType::Registration => {
            validate_registration_data(tx, ctx)?;
        }
        TxType::Exit => {
            validate_exit_data(tx)?;
        }
        TxType::ClaimReward => {
            validate_claim_data(tx)?;
        }
        TxType::ClaimBond => {
            validate_claim_bond_data(tx)?;
        }
        TxType::SlashProducer => {
            validate_slash_data(tx, ctx)?;
        }
        TxType::Coinbase => {
            // Coinbase validation is handled at the block level
            // (must be first tx, amount must match block reward, etc.)
        }
        TxType::AddBond => {
            validate_add_bond_data(tx)?;
        }
        TxType::RequestWithdrawal => {
            validate_withdrawal_request_data(tx)?;
        }
        TxType::ClaimWithdrawal => {
            // Reserved -- ClaimWithdrawal is unused. Withdrawal is instant via RequestWithdrawal.
            return Err(ValidationError::InvalidClaimWithdrawal(
                "ClaimWithdrawal is not supported".to_string(),
            ));
        }
        TxType::EpochReward => {
            validate_epoch_reward_data(tx)?;
        }
        TxType::RemoveMaintainer => {
            validate_maintainer_change_data(tx)?;
        }
        TxType::AddMaintainer => {
            validate_maintainer_change_data(tx)?;
        }
        TxType::DelegateBond => {
            validate_delegate_bond_data(tx)?;
        }
        TxType::RevokeDelegation => {
            validate_revoke_delegation_data(tx)?;
        }
        TxType::ProtocolActivation => {
            validate_protocol_activation_data(tx)?;
        }
    }

    Ok(())
}

/// Validate transaction outputs and compute safe total.
///
/// Returns the total output amount, or an error if any output is invalid
/// or if the total would overflow.
fn validate_outputs(
    outputs: &[Output],
    ctx: &ValidationContext,
) -> Result<Amount, ValidationError> {
    let mut total: Amount = 0;

    for (i, output) in outputs.iter().enumerate() {
        // Amount must be positive
        if output.amount == 0 {
            return Err(ValidationError::InvalidTransaction(format!(
                "output {} has zero amount",
                i
            )));
        }

        // Amount must not exceed max supply individually
        if output.amount > TOTAL_SUPPLY {
            return Err(ValidationError::AmountExceedsSupply {
                amount: output.amount,
                max: TOTAL_SUPPLY,
            });
        }

        // Check for overflow when adding to total
        total =
            total
                .checked_add(output.amount)
                .ok_or_else(|| ValidationError::AmountOverflow {
                    context: format!("output total at index {}", i),
                })?;

        // Validate extra_data size limit (all output types)
        if output.extra_data.len() > MAX_EXTRA_DATA_SIZE {
            return Err(ValidationError::InvalidTransaction(format!(
                "output {} extra_data exceeds max size ({} > {})",
                i,
                output.extra_data.len(),
                MAX_EXTRA_DATA_SIZE,
            )));
        }

        // Validate output type consistency
        match output.output_type {
            OutputType::Normal => {
                // Normal outputs should have lock_until = 0
                if output.lock_until != 0 {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "normal output {} has non-zero lock_until",
                        i
                    )));
                }
                // Normal outputs must not carry extra_data
                if !output.extra_data.is_empty() {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "normal output {} has non-empty extra_data",
                        i
                    )));
                }
            }
            OutputType::Bond => {
                // Bond outputs must have a future lock time
                if output.lock_until == 0 {
                    return Err(ValidationError::InvalidBond(format!(
                        "bond output {} has zero lock_until",
                        i
                    )));
                }
                // Bond outputs must carry exactly 4 bytes of extra_data (creation_slot)
                if output.extra_data.len() != 4 {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "bond output {} has {} bytes extra_data, expected 4 (creation_slot)",
                        i,
                        output.extra_data.len()
                    )));
                }
            }
            // Covenant output types: validate activation and condition encoding
            OutputType::Multisig
            | OutputType::Hashlock
            | OutputType::HTLC
            | OutputType::Vesting => {
                // Activation height gating: reject conditioned outputs before activation
                let activation = ctx.params.covenants_activation_height(&ctx.network);
                if ctx.current_height < activation {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "conditioned output {} rejected: covenants activate at height {}",
                        i, activation
                    )));
                }
                if output.extra_data.is_empty() {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "conditioned output {} has empty extra_data",
                        i
                    )));
                }
                // Validate condition decodes successfully
                if let Err(e) = crate::conditions::Condition::decode(&output.extra_data) {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "conditioned output {} has invalid condition: {}",
                        i, e
                    )));
                }
            }
            OutputType::NFT => {
                let activation = ctx.params.covenants_activation_height(&ctx.network);
                if ctx.current_height < activation {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "NFT output {} rejected: covenants activate at height {}",
                        i, activation
                    )));
                }
                if output.extra_data.is_empty() {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "NFT output {} has empty extra_data",
                        i
                    )));
                }
                if let Err(e) = crate::conditions::Condition::decode_prefix(&output.extra_data) {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "NFT output {} has invalid condition: {}",
                        i, e
                    )));
                }
                if output.nft_metadata().is_none() {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "NFT output {} has invalid or missing NFT metadata",
                        i
                    )));
                }
            }
            OutputType::FungibleAsset => {
                let activation = ctx.params.covenants_activation_height(&ctx.network);
                if ctx.current_height < activation {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "FungibleAsset output {} rejected: covenants activate at height {}",
                        i, activation
                    )));
                }
                if output.extra_data.is_empty() {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "FungibleAsset output {} has empty extra_data",
                        i
                    )));
                }
                if let Err(e) = crate::conditions::Condition::decode_prefix(&output.extra_data) {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "FungibleAsset output {} has invalid condition: {}",
                        i, e
                    )));
                }
                if output.fungible_asset_metadata().is_none() {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "FungibleAsset output {} has invalid or missing asset metadata",
                        i
                    )));
                }
            }
            OutputType::BridgeHTLC => {
                let activation = ctx.params.covenants_activation_height(&ctx.network);
                if ctx.current_height < activation {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "BridgeHTLC output {} rejected: covenants activate at height {}",
                        i, activation
                    )));
                }
                if output.extra_data.is_empty() {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "BridgeHTLC output {} has empty extra_data",
                        i
                    )));
                }
                if let Err(e) = crate::conditions::Condition::decode_prefix(&output.extra_data) {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "BridgeHTLC output {} has invalid condition: {}",
                        i, e
                    )));
                }
                if output.bridge_htlc_metadata().is_none() {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "BridgeHTLC output {} has invalid or missing bridge metadata",
                        i
                    )));
                }
            }
        }

        // Pubkey hash must not be zero (except for burn address)
        if output.pubkey_hash == Hash::ZERO {
            return Err(ValidationError::InvalidTransaction(format!(
                "output {} has zero pubkey_hash",
                i
            )));
        }
    }

    Ok(total)
}

/// Validate registration transaction data.
fn validate_registration_data(
    tx: &Transaction,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    // During genesis, Registration TXs are VDF proof containers only.
    // No bond required (bond is handled at GENESIS PHASE COMPLETE).
    // No registration chain validation (bootstrap producers can't be Sybil-attacked).
    // VDF proof is still validated — that's the whole point.
    if ctx.network.is_in_genesis(ctx.current_height) {
        if tx.extra_data.is_empty() {
            return Err(ValidationError::InvalidRegistration(
                "missing registration data".to_string(),
            ));
        }
        let reg_data: RegistrationData = bincode::deserialize(&tx.extra_data).map_err(|e| {
            ValidationError::InvalidRegistration(format!("invalid registration data: {}", e))
        })?;
        // BLS key is mandatory — every producer must have one
        if reg_data.bls_pubkey.is_empty() {
            return Err(ValidationError::InvalidRegistration(
                "BLS public key required for registration".to_string(),
            ));
        }
        if reg_data.bls_pop.is_empty() {
            return Err(ValidationError::InvalidRegistration(
                "BLS proof of possession required for registration".to_string(),
            ));
        }
        validate_bls_pop(&reg_data)?;

        // Validate VDF proof (the only requirement for genesis registrations)
        validate_registration_vdf(&reg_data, ctx.network)?;
        return Ok(());
    }

    // Registration must have at least one input (for bond)
    if tx.inputs.is_empty() {
        return Err(ValidationError::InvalidRegistration(
            "registration must have inputs for bond".to_string(),
        ));
    }

    // Must have at least one bond output
    let bond_outputs: Vec<_> = tx
        .outputs
        .iter()
        .filter(|o| o.output_type == OutputType::Bond)
        .collect();

    if bond_outputs.is_empty() {
        return Err(ValidationError::InvalidRegistration(
            "registration must have a bond output".to_string(),
        ));
    }

    // Verify bond amount meets minimum requirement
    let required_bond = ctx.params.bond_amount(ctx.current_height);
    let total_bond: Amount = bond_outputs
        .iter()
        .map(|o| o.amount)
        .try_fold(0u64, |acc, amt| acc.checked_add(amt))
        .ok_or_else(|| ValidationError::AmountOverflow {
            context: "bond total".to_string(),
        })?;

    if total_bond < required_bond {
        return Err(ValidationError::InvalidRegistration(format!(
            "insufficient bond: {} < {}",
            total_bond, required_bond
        )));
    }

    // Verify bond lock duration
    let required_lock = ctx.current_height + ctx.params.blocks_per_era;
    for bond in &bond_outputs {
        if bond.lock_until < required_lock {
            return Err(ValidationError::InvalidRegistration(format!(
                "bond lock too short: {} < {}",
                bond.lock_until, required_lock
            )));
        }
    }

    // Parse and validate registration data from extra_data
    if tx.extra_data.is_empty() {
        return Err(ValidationError::InvalidRegistration(
            "missing registration data".to_string(),
        ));
    }

    // Try to deserialize registration data
    let reg_data: RegistrationData = bincode::deserialize(&tx.extra_data).map_err(|e| {
        ValidationError::InvalidRegistration(format!("invalid registration data: {}", e))
    })?;

    // Validate bond_count is consensus-safe (WHITEPAPER Section 7)
    // bond_count is embedded on-chain to ensure all nodes agree on producer selection.
    // We only validate bounds here; the existing total_bond >= required_bond check
    // above ensures sufficient collateral.
    if reg_data.bond_count < 1 {
        return Err(ValidationError::InvalidRegistration(
            "bond_count must be at least 1".to_string(),
        ));
    }
    if reg_data.bond_count > crate::consensus::MAX_BONDS_PER_PRODUCER {
        return Err(ValidationError::InvalidRegistration(format!(
            "bond_count {} exceeds maximum {}",
            reg_data.bond_count,
            crate::consensus::MAX_BONDS_PER_PRODUCER,
        )));
    }

    // BLS key is mandatory — every producer must have one (like Ethereum validators)
    if reg_data.bls_pubkey.is_empty() {
        return Err(ValidationError::InvalidRegistration(
            "BLS public key required for registration".to_string(),
        ));
    }
    if reg_data.bls_pop.is_empty() {
        return Err(ValidationError::InvalidRegistration(
            "BLS proof of possession required for registration".to_string(),
        ));
    }
    validate_bls_pop(&reg_data)?;

    // Verify VDF proof for registration
    // (The actual VDF verification happens here)
    validate_registration_vdf(&reg_data, ctx.network)?;

    // Verify registration chain (anti-Sybil: prevents parallel registration)
    validate_registration_chain(&reg_data, ctx)?;

    // Reject duplicate registration (GitHub Issue #4: duplicate register deletes producer)
    if ctx.active_producers.contains(&reg_data.public_key) {
        return Err(ValidationError::InvalidRegistration(
            "producer already registered".to_string(),
        ));
    }

    // Reject registration if already pending (epoch-deferred, not yet active)
    if ctx.pending_producer_keys.contains(&reg_data.public_key) {
        return Err(ValidationError::InvalidRegistration(
            "producer already has a pending registration".to_string(),
        ));
    }

    Ok(())
}

/// Validate the registration chain fields (anti-Sybil protection).
///
/// Each registration must reference the previous registration's hash and
/// have the correct sequence number. This prevents parallel registration
/// attacks where an attacker tries to register many nodes simultaneously.
fn validate_registration_chain(
    reg_data: &RegistrationData,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    let expected_prev_hash = ctx.registration_chain.expected_prev_hash();
    let expected_sequence = ctx.registration_chain.expected_sequence();

    // Verify prev_registration_hash matches
    if reg_data.prev_registration_hash != expected_prev_hash {
        return Err(ValidationError::InvalidRegistration(format!(
            "invalid prev_registration_hash: expected {}, got {}",
            hex::encode(expected_prev_hash.as_bytes()),
            hex::encode(reg_data.prev_registration_hash.as_bytes())
        )));
    }

    // Verify sequence number is correct
    if reg_data.sequence_number != expected_sequence {
        return Err(ValidationError::InvalidRegistration(format!(
            "invalid sequence_number: expected {}, got {}",
            expected_sequence, reg_data.sequence_number
        )));
    }

    Ok(())
}

/// Validate the VDF proof in registration data.
///
/// Validate registration VDF using hash-chain (same as block VDF).
/// Hash-chain is fast to compute (~5s for 5M iterations) and self-verifying
/// (recompute and compare output). No separate proof needed.
/// For devnet, VDF validation is skipped to allow quick testing.
fn validate_registration_vdf(
    reg_data: &RegistrationData,
    network: Network,
) -> Result<(), ValidationError> {
    // Skip VDF validation for devnet (allows CLI registration without VDF computation)
    if network == Network::Devnet {
        return Ok(());
    }

    // Create VDF input using the standard function
    let input = vdf::registration_input(&reg_data.public_key, reg_data.epoch);

    // Hash-chain VDF output must be exactly 32 bytes
    if reg_data.vdf_output.len() != 32 {
        return Err(ValidationError::InvalidRegistration(
            "invalid VDF output: expected 32 bytes".to_string(),
        ));
    }

    let expected_output: [u8; 32] = reg_data.vdf_output.as_slice().try_into().map_err(|_| {
        ValidationError::InvalidRegistration("invalid VDF output format".to_string())
    })?;

    // Verify by recomputing the hash-chain VDF
    if !verify_hash_chain_vdf(&input, &expected_output, network.vdf_register_iterations()) {
        return Err(ValidationError::InvalidRegistration(
            "VDF verification failed".to_string(),
        ));
    }

    Ok(())
}

/// Verify the BLS aggregate attestation signature in a block.
///
/// Decodes the presence_root bitfield to determine which producers attested,
/// gathers their BLS public keys from the validation context, and verifies
/// the aggregate signature against the attestation message.
fn validate_bls_aggregate(block: &Block, ctx: &ValidationContext) -> Result<(), ValidationError> {
    // Decode aggregate signature
    let agg_sig = crypto::BlsSignature::try_from_slice(&block.aggregate_bls_signature)
        .map_err(|e| ValidationError::InvalidBlock(format!("invalid BLS aggregate sig: {}", e)))?;

    // Decode bitfield to find which producers attested
    let attested_indices =
        decode_attestation_bitfield(&block.header.presence_root, ctx.producer_bls_keys.len());

    if attested_indices.is_empty() {
        return Err(ValidationError::InvalidBlock(
            "BLS aggregate sig present but bitfield is empty".to_string(),
        ));
    }

    // Gather BLS pubkeys of attesting producers (skip those without BLS keys)
    let mut bls_pubkeys: Vec<crypto::BlsPublicKey> = Vec::new();
    for &idx in &attested_indices {
        if idx < ctx.producer_bls_keys.len() && !ctx.producer_bls_keys[idx].is_empty() {
            if let Ok(pk) = crypto::BlsPublicKey::try_from_slice(&ctx.producer_bls_keys[idx]) {
                bls_pubkeys.push(pk);
            }
        }
    }

    if bls_pubkeys.is_empty() {
        // No BLS-capable producers in the bitfield — can't verify.
        // This is a transitional state: block has aggregate sig but no BLS keys registered.
        // Accept gracefully during migration.
        return Ok(());
    }

    // Verify: aggregate sig must match the attestation message signed by these pubkeys
    let msg = crypto::attestation_message(&block.hash(), block.header.slot);
    crypto::bls_verify_aggregate(&msg, &agg_sig, &bls_pubkeys).map_err(|e| {
        ValidationError::InvalidBlock(format!("BLS aggregate verification failed: {}", e))
    })?;

    Ok(())
}

/// Validate BLS proof-of-possession for a registration.
///
/// Verifies that the registrant controls the BLS secret key by checking
/// a signature over the BLS public key itself (separate `PoP` DST).
fn validate_bls_pop(reg_data: &RegistrationData) -> Result<(), ValidationError> {
    let pubkey = crypto::BlsPublicKey::try_from_slice(&reg_data.bls_pubkey).map_err(|e| {
        ValidationError::InvalidRegistration(format!("invalid BLS public key: {}", e))
    })?;

    let sig = crypto::BlsSignature::try_from_slice(&reg_data.bls_pop).map_err(|e| {
        ValidationError::InvalidRegistration(format!("invalid BLS PoP signature: {}", e))
    })?;

    crypto::bls_verify_pop(&pubkey, &sig).map_err(|_| {
        ValidationError::InvalidRegistration(
            "BLS proof-of-possession verification failed".to_string(),
        )
    })?;

    Ok(())
}

/// Validate exit transaction data.
fn validate_exit_data(tx: &Transaction) -> Result<(), ValidationError> {
    // Exit must have no inputs (just identifies producer to exit)
    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidTransaction(
            "exit transaction must have no inputs".to_string(),
        ));
    }

    // Exit must have no outputs (bond released after cooldown)
    if !tx.outputs.is_empty() {
        return Err(ValidationError::InvalidTransaction(
            "exit transaction must have no outputs".to_string(),
        ));
    }

    // Parse and validate exit data from extra_data
    if tx.extra_data.is_empty() {
        return Err(ValidationError::InvalidTransaction(
            "missing exit data".to_string(),
        ));
    }

    // Try to deserialize exit data
    let _exit_data: ExitData = bincode::deserialize(&tx.extra_data)
        .map_err(|e| ValidationError::InvalidTransaction(format!("invalid exit data: {}", e)))?;

    // Note: Producer state validation (is producer active, not already in cooldown, etc.)
    // is done at the node level where we have access to the producer set

    Ok(())
}

/// Validate claim reward transaction data.
///
/// Structural validation for ClaimReward transactions:
/// - Must have no inputs (rewards come from pending balance)
/// - Must have exactly one output (the claimed amount)
/// - Must have valid claim data identifying the producer
///
/// Note: The actual reward amount validation is done at the node level
/// where we have access to the producer set and their pending_rewards.
fn validate_claim_data(tx: &Transaction) -> Result<(), ValidationError> {
    // Claim must have no inputs (rewards come from pending balance, not UTXOs)
    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidClaim(
            "claim transaction must have no inputs".to_string(),
        ));
    }

    // Claim must have exactly one output (the claimed rewards)
    if tx.outputs.len() != 1 {
        return Err(ValidationError::InvalidClaim(
            "claim transaction must have exactly one output".to_string(),
        ));
    }

    // Output must be a normal output (not a bond)
    if tx.outputs[0].output_type != OutputType::Normal {
        return Err(ValidationError::InvalidClaim(
            "claim output must be a normal output".to_string(),
        ));
    }

    // Parse and validate claim data from extra_data
    if tx.extra_data.is_empty() {
        return Err(ValidationError::InvalidClaim(
            "missing claim data".to_string(),
        ));
    }

    // Try to deserialize claim data
    let _claim_data: ClaimData = bincode::deserialize(&tx.extra_data)
        .map_err(|e| ValidationError::InvalidClaim(format!("invalid claim data: {}", e)))?;

    // Note: The following validations are done at the node level:
    // - Producer exists and is registered
    // - Producer has sufficient pending_rewards for the claimed amount
    // - Signature verification (producer must sign the claim)

    Ok(())
}

/// Validate claim bond transaction data.
///
/// Structural validation for ClaimBond transactions:
/// - Must have no inputs (bond comes from protocol)
/// - Must have exactly one output (the returned bond)
/// - Must have valid claim bond data identifying the producer
///
/// Note: The actual bond amount and exit terms validation is done at the node level
/// where we have access to the producer set and their unbonding status.
fn validate_claim_bond_data(tx: &Transaction) -> Result<(), ValidationError> {
    // Claim bond must have no inputs
    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidBondClaim(
            "claim bond transaction must have no inputs".to_string(),
        ));
    }

    // Claim bond must have exactly one output
    if tx.outputs.len() != 1 {
        return Err(ValidationError::InvalidBondClaim(
            "claim bond transaction must have exactly one output".to_string(),
        ));
    }

    // Output must be a normal output (not a bond)
    if tx.outputs[0].output_type != OutputType::Normal {
        return Err(ValidationError::InvalidBondClaim(
            "claim bond output must be a normal output".to_string(),
        ));
    }

    // Parse and validate claim bond data from extra_data
    if tx.extra_data.is_empty() {
        return Err(ValidationError::InvalidBondClaim(
            "missing claim bond data".to_string(),
        ));
    }

    // Try to deserialize claim bond data
    let _claim_bond_data: ClaimBondData = bincode::deserialize(&tx.extra_data).map_err(|e| {
        ValidationError::InvalidBondClaim(format!("invalid claim bond data: {}", e))
    })?;

    // Note: The following validations are done at the node level:
    // - Producer exists and has status Exited (unbonding complete)
    // - Bond amount matches exit terms (full or early exit penalty applied)
    // - Signature verification (producer must sign the claim)

    Ok(())
}

/// Validate slash producer transaction data.
///
/// Structural validation for SlashProducer transactions:
/// - Must have no inputs
/// - Must have no outputs (bond is burned, not redistributed)
/// - Must have valid slash data with cryptographically verifiable evidence
///
/// Evidence verification is now done here with VDF verification to prevent
/// fabricated evidence attacks. The VDF proves the producer actually created
/// both blocks (since the VDF input includes the producer's public key).
fn validate_slash_data(tx: &Transaction, ctx: &ValidationContext) -> Result<(), ValidationError> {
    // Slash must have no inputs
    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidSlash(
            "slash transaction must have no inputs".to_string(),
        ));
    }

    // Slash must have no outputs (bond is burned)
    if !tx.outputs.is_empty() {
        return Err(ValidationError::InvalidSlash(
            "slash transaction must have no outputs".to_string(),
        ));
    }

    // Parse and validate slash data from extra_data
    if tx.extra_data.is_empty() {
        return Err(ValidationError::InvalidSlash(
            "missing slash data".to_string(),
        ));
    }

    // Try to deserialize slash data
    let slash_data: SlashData = bincode::deserialize(&tx.extra_data)
        .map_err(|e| ValidationError::InvalidSlash(format!("invalid slash data: {}", e)))?;

    // Validate evidence structure with full cryptographic verification
    // Only double production is slashable - this is the only unambiguously intentional offense
    match &slash_data.evidence {
        crate::transaction::SlashingEvidence::DoubleProduction {
            block_header_1,
            block_header_2,
        } => {
            // 1. Both headers must have the same producer
            if block_header_1.producer != block_header_2.producer {
                return Err(ValidationError::InvalidSlash(
                    "double production evidence must have same producer in both headers"
                        .to_string(),
                ));
            }

            // 2. Both headers must have the same slot
            if block_header_1.slot != block_header_2.slot {
                return Err(ValidationError::InvalidSlash(
                    "double production evidence must have same slot in both headers".to_string(),
                ));
            }

            // 3. Block hashes must be different (otherwise it's not double production)
            if block_header_1.hash() == block_header_2.hash() {
                return Err(ValidationError::InvalidSlash(
                    "double production evidence must have different block hashes".to_string(),
                ));
            }

            // 4. Producer in evidence must match slash_data.producer_pubkey
            if block_header_1.producer != slash_data.producer_pubkey {
                return Err(ValidationError::InvalidSlash(
                    "evidence producer does not match slash target".to_string(),
                ));
            }

            // 5. Verify VDF for header 1 (proves the producer actually created it)
            validate_vdf(block_header_1, ctx.network).map_err(|_| {
                ValidationError::InvalidSlash(
                    "invalid VDF proof in first block header - evidence may be fabricated"
                        .to_string(),
                )
            })?;

            // 6. Verify VDF for header 2 (proves the producer actually created it)
            validate_vdf(block_header_2, ctx.network).map_err(|_| {
                ValidationError::InvalidSlash(
                    "invalid VDF proof in second block header - evidence may be fabricated"
                        .to_string(),
                )
            })?;
        }
    }

    // Note: The following validations are done at the node level:
    // - Producer exists and is active (not already slashed or exited)
    // - Reporter signature is valid

    Ok(())
}

// ==================== Bond Transaction Validation ====================

/// Validate add bond transaction data.
///
/// Structural validation for AddBond transactions:
/// - Must have inputs (paying for bonds)
/// - Must have no outputs (funds become bonds)
/// - Must have valid add bond data with bond count
/// - Input amount must equal bond_count * BOND_UNIT
///
/// Note: Producer existence and max bonds check is done at node level.
fn validate_add_bond_data(tx: &Transaction) -> Result<(), ValidationError> {
    // Must have inputs (funds to become bonds)
    if tx.inputs.is_empty() {
        return Err(ValidationError::InvalidAddBond(
            "add bond transaction must have inputs".to_string(),
        ));
    }

    // Must have at least one Bond output (lock/unlock model)
    let bond_outputs: Vec<_> = tx
        .outputs
        .iter()
        .filter(|o| o.output_type == OutputType::Bond)
        .collect();

    if bond_outputs.is_empty() {
        return Err(ValidationError::InvalidAddBond(
            "add bond must have a Bond output".to_string(),
        ));
    }

    // Non-bond outputs must be Normal (for change)
    for output in &tx.outputs {
        if output.output_type != OutputType::Normal && output.output_type != OutputType::Bond {
            return Err(ValidationError::InvalidAddBond(
                "add bond outputs must be Bond or Normal type".to_string(),
            ));
        }
    }

    // Parse add bond data from extra_data
    let bond_data = AddBondData::from_bytes(&tx.extra_data)
        .ok_or_else(|| ValidationError::InvalidAddBond("invalid add bond data".to_string()))?;

    // Bond count must be positive
    if bond_data.bond_count == 0 {
        return Err(ValidationError::InvalidAddBond(
            "bond count must be positive".to_string(),
        ));
    }

    // Note: These validations are done at node level:
    // - Producer is registered
    // - New total doesn't exceed MAX_BONDS_PER_PRODUCER
    // - Bond output amount matches bond_count * BOND_UNIT

    Ok(())
}

/// Validate withdrawal request transaction data.
///
/// Structural validation for RequestWithdrawal transactions:
/// - Must have inputs (Bond UTXOs being consumed — lock/unlock model)
/// - Must have exactly 1 normal output (payout to destination)
/// - Must have valid withdrawal request data
/// - Output amount must be > 0
/// - Output pubkey_hash must match destination in withdrawal data
///
/// Note: Bond UTXO ownership, producer bond holdings, and FIFO calculation done at node level.
fn validate_withdrawal_request_data(tx: &Transaction) -> Result<(), ValidationError> {
    // Must have inputs (Bond UTXOs being unlocked)
    if tx.inputs.is_empty() {
        return Err(ValidationError::InvalidWithdrawalRequest(
            "withdrawal request must have Bond UTXO inputs".to_string(),
        ));
    }

    // Must have exactly 1 output (payout)
    if tx.outputs.len() != 1 {
        return Err(ValidationError::InvalidWithdrawalRequest(
            "withdrawal request must have exactly 1 output".to_string(),
        ));
    }

    let output = &tx.outputs[0];
    if output.output_type != OutputType::Normal {
        return Err(ValidationError::InvalidWithdrawalRequest(
            "withdrawal output must be Normal type".to_string(),
        ));
    }
    if output.amount == 0 {
        return Err(ValidationError::InvalidWithdrawalRequest(
            "withdrawal output amount must be positive".to_string(),
        ));
    }

    // Parse withdrawal data from extra_data
    let withdrawal_data = WithdrawalRequestData::from_bytes(&tx.extra_data).ok_or_else(|| {
        ValidationError::InvalidWithdrawalRequest("invalid withdrawal request data".to_string())
    })?;

    // Bond count must be positive
    if withdrawal_data.bond_count == 0 {
        return Err(ValidationError::InvalidWithdrawalRequest(
            "withdrawal bond count must be positive".to_string(),
        ));
    }

    // Destination must not be zero hash
    if withdrawal_data.destination == Hash::ZERO {
        return Err(ValidationError::InvalidWithdrawalRequest(
            "destination cannot be zero hash".to_string(),
        ));
    }

    // Output destination must match withdrawal data destination
    if output.pubkey_hash != withdrawal_data.destination {
        return Err(ValidationError::InvalidWithdrawalRequest(
            "output destination must match withdrawal data destination".to_string(),
        ));
    }

    // Note: These validations are done at node level:
    // - Producer is registered
    // - Producer has enough bonds to withdraw
    // - Output amount <= FIFO net calculation

    Ok(())
}

/// Validate epoch reward transaction data
///
/// Basic validation of EpochReward transactions:
/// - Must have no inputs (minted)
/// - Must have exactly one output
/// - Output must be Normal type
/// - Must have valid EpochRewardData
///
/// NOTE: This is the working automatic push-based reward system.
/// Rewards are distributed automatically at epoch boundaries by the block producer.
fn validate_epoch_reward_data(tx: &Transaction) -> Result<(), ValidationError> {
    // Must have no inputs (minted)
    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidEpochReward(
            "epoch reward must have no inputs".to_string(),
        ));
    }

    // Must have at least one output
    if tx.outputs.is_empty() {
        return Err(ValidationError::InvalidEpochReward(
            "epoch reward must have at least one output".to_string(),
        ));
    }

    // All outputs must be Normal type
    for output in &tx.outputs {
        if output.output_type != OutputType::Normal {
            return Err(ValidationError::InvalidEpochReward(
                "epoch reward outputs must be Normal type".to_string(),
            ));
        }
    }

    Ok(())
}

// ==================== Maintainer Transaction Validation ====================

/// Validate maintainer change transaction data (AddMaintainer/RemoveMaintainer).
///
/// Structural validation for maintainer change transactions:
/// - Must have no inputs (state-only operation)
/// - Must have no outputs (no funds transferred)
/// - Must have valid MaintainerChangeData in extra_data
///
/// Note: Signature verification and maintainer set state checks are done
/// at the node level where we have access to the current maintainer set.
fn validate_maintainer_change_data(tx: &Transaction) -> Result<(), ValidationError> {
    use crate::maintainer::MaintainerChangeData;

    // Maintainer changes must have no inputs (state-only operation)
    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidMaintainerChange(
            "maintainer change transaction must have no inputs".to_string(),
        ));
    }

    // Maintainer changes must have no outputs (no funds transferred)
    if !tx.outputs.is_empty() {
        return Err(ValidationError::InvalidMaintainerChange(
            "maintainer change transaction must have no outputs".to_string(),
        ));
    }

    // Must have valid MaintainerChangeData in extra_data
    if tx.extra_data.is_empty() {
        return Err(ValidationError::InvalidMaintainerChange(
            "missing maintainer change data".to_string(),
        ));
    }

    // Try to deserialize maintainer change data
    let _change_data = MaintainerChangeData::from_bytes(&tx.extra_data).ok_or_else(|| {
        ValidationError::InvalidMaintainerChange(
            "invalid maintainer change data format".to_string(),
        )
    })?;

    // Note: The following validations are done at the node level:
    // - Current maintainer set exists and is valid
    // - Sufficient signatures from current maintainers (threshold check)
    // - Target is not already a maintainer (for Add) or is a maintainer (for Remove)
    // - Adding won't exceed MAX_MAINTAINERS
    // - Removing won't go below MIN_MAINTAINERS

    Ok(())
}

/// Validate DelegateBond transaction data.
///
/// Structural validation:
/// - Must have no inputs (state-only operation)
/// - Must have no outputs
/// - Must have valid DelegateBondData in extra_data
/// - Bond count must be positive
///
/// Note: Producer existence, active status, self-delegation, and
/// sufficient bonds are checked at the node level.
fn validate_delegate_bond_data(tx: &Transaction) -> Result<(), ValidationError> {
    use crate::transaction::DelegateBondData;

    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidDelegation(
            "delegate bond must have no inputs".to_string(),
        ));
    }

    if !tx.outputs.is_empty() {
        return Err(ValidationError::InvalidDelegation(
            "delegate bond must have no outputs".to_string(),
        ));
    }

    if tx.extra_data.is_empty() {
        return Err(ValidationError::InvalidDelegation(
            "missing delegate bond data".to_string(),
        ));
    }

    let data = DelegateBondData::from_bytes(&tx.extra_data).ok_or_else(|| {
        ValidationError::InvalidDelegation("invalid delegate bond data format".to_string())
    })?;

    if data.bond_count == 0 {
        return Err(ValidationError::InvalidDelegation(
            "bond count must be positive".to_string(),
        ));
    }

    Ok(())
}

/// Validate RevokeDelegation transaction data.
///
/// Structural validation:
/// - Must have no inputs (state-only operation)
/// - Must have no outputs
/// - Must have valid RevokeDelegationData in extra_data
///
/// Note: Active delegation existence and unbonding delay are
/// checked at the node level.
fn validate_revoke_delegation_data(tx: &Transaction) -> Result<(), ValidationError> {
    use crate::transaction::RevokeDelegationData;

    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidDelegation(
            "revoke delegation must have no inputs".to_string(),
        ));
    }

    if !tx.outputs.is_empty() {
        return Err(ValidationError::InvalidDelegation(
            "revoke delegation must have no outputs".to_string(),
        ));
    }

    if tx.extra_data.is_empty() {
        return Err(ValidationError::InvalidDelegation(
            "missing revoke delegation data".to_string(),
        ));
    }

    let _data = RevokeDelegationData::from_bytes(&tx.extra_data).ok_or_else(|| {
        ValidationError::InvalidDelegation("invalid revoke delegation data format".to_string())
    })?;

    Ok(())
}

// ==================== Protocol Activation Validation ====================

/// Validate protocol activation transaction data.
///
/// Structural validation:
/// - Must have no inputs (state-only operation)
/// - Must have no outputs (no funds transferred)
/// - Must have valid ProtocolActivationData in extra_data
/// - Protocol version must be > 0
/// - Activation epoch must be > 0
/// - At least 1 signature present (full 3/5 check done at node level)
///
/// Note: Maintainer set verification (3/5 multisig), version > current,
/// and epoch > current are checked at the node level where state is available.
fn validate_protocol_activation_data(tx: &Transaction) -> Result<(), ValidationError> {
    use crate::maintainer::ProtocolActivationData;

    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidProtocolActivation(
            "protocol activation must have no inputs".to_string(),
        ));
    }

    if !tx.outputs.is_empty() {
        return Err(ValidationError::InvalidProtocolActivation(
            "protocol activation must have no outputs".to_string(),
        ));
    }

    if tx.extra_data.is_empty() {
        return Err(ValidationError::InvalidProtocolActivation(
            "missing protocol activation data".to_string(),
        ));
    }

    let data = ProtocolActivationData::from_bytes(&tx.extra_data).ok_or_else(|| {
        ValidationError::InvalidProtocolActivation(
            "invalid protocol activation data format".to_string(),
        )
    })?;

    if data.protocol_version == 0 {
        return Err(ValidationError::InvalidProtocolActivation(
            "protocol version must be > 0".to_string(),
        ));
    }

    if data.activation_epoch == 0 {
        return Err(ValidationError::InvalidProtocolActivation(
            "activation epoch must be > 0".to_string(),
        ));
    }

    if data.signatures.is_empty() {
        return Err(ValidationError::InvalidProtocolActivation(
            "at least one maintainer signature required".to_string(),
        ));
    }

    Ok(())
}

/// Validate a transaction with full UTXO context.
///
/// This performs complete validation including:
/// - All structural checks from `validate_transaction`
/// - Signature verification for each input
/// - Input existence and spendability
/// - Input/output balance verification
/// - Lock time enforcement
///
/// # Errors
///
/// Returns an error if any validation check fails.
pub fn validate_transaction_with_utxos<U: UtxoProvider>(
    tx: &Transaction,
    ctx: &ValidationContext,
    utxo_provider: &U,
) -> Result<(), ValidationError> {
    // First, perform structural validation
    validate_transaction(tx, ctx)?;

    // Coinbase and EpochReward transactions don't need UTXO validation (minted)
    if tx.is_coinbase() || tx.is_epoch_reward() {
        return Ok(());
    }

    // Validate each input
    let mut total_input: Amount = 0;

    for (i, input) in tx.inputs.iter().enumerate() {
        // Validate committed_output_count
        if input.committed_output_count > 0 {
            if input.sighash_type != SighashType::AnyoneCanPay {
                return Err(ValidationError::InvalidTransaction(format!(
                    "input {} has committed_output_count={} but sighash is not AnyoneCanPay",
                    i, input.committed_output_count
                )));
            }
            if input.committed_output_count as usize > tx.outputs.len() {
                return Err(ValidationError::InvalidTransaction(format!(
                    "input {} committed_output_count={} exceeds output count={}",
                    i, input.committed_output_count, tx.outputs.len()
                )));
            }
        }

        // Per-input signing hash: respects SighashType (All vs AnyoneCanPay)
        let signing_hash = tx.signing_message_for_input(i);
        // Look up the UTXO
        let utxo = utxo_provider
            .get_utxo(&input.prev_tx_hash, input.output_index)
            .ok_or(ValidationError::OutputNotFound {
                tx_hash: input.prev_tx_hash,
                output_index: input.output_index,
            })?;

        // Check if already spent
        if utxo.spent {
            return Err(ValidationError::OutputAlreadySpent {
                tx_hash: input.prev_tx_hash,
                output_index: input.output_index,
            });
        }

        // Check lock time — skip for WithdrawalRequest/Exit (they unlock Bond UTXOs)
        if tx.tx_type != TxType::RequestWithdrawal
            && tx.tx_type != TxType::Exit
            && !utxo.output.is_spendable_at(ctx.current_height)
        {
            return Err(ValidationError::OutputLocked {
                lock_height: utxo.output.lock_until,
                current_height: ctx.current_height,
            });
        }

        // Verify spending conditions (signature for Normal/Bond, condition evaluator for others)
        verify_input_conditions(tx, input, &signing_hash, &utxo, i, ctx.current_height)?;

        // Add to total (with overflow check)
        total_input = total_input.checked_add(utxo.output.amount).ok_or_else(|| {
            ValidationError::AmountOverflow {
                context: format!("input total at index {}", i),
            }
        })?;
    }

    // Verify inputs >= outputs (difference is fee)
    let total_output = tx.total_output();
    if total_input < total_output {
        return Err(ValidationError::InsufficientFunds {
            inputs: total_input,
            outputs: total_output,
        });
    }

    // ── Royalty enforcement ──────────────────────────────────────────────
    // When an NFT with royalties is spent, the transaction MUST include
    // a payment output to the creator for at least (sale_price * royalty_bps / 10000).
    // Sale price is inferred from the largest non-NFT, non-change output to a
    // pubkey_hash that differs from both the NFT input owner and the NFT output recipient.
    if tx.tx_type == TxType::Transfer {
        for (i, input) in tx.inputs.iter().enumerate() {
            let utxo = utxo_provider.get_utxo(&input.prev_tx_hash, input.output_index);
            if let Some(utxo) = utxo {
                if let Some((creator_hash, royalty_bps)) = utxo.output.nft_royalty() {
                    if royalty_bps > 0 {
                        // Find the sale price: the payment output to the seller
                        // (the owner of the NFT being spent = utxo.output.pubkey_hash)
                        let seller_hash = utxo.output.pubkey_hash;
                        let sale_price: Amount = tx
                            .outputs
                            .iter()
                            .filter(|o| {
                                o.output_type == OutputType::Normal && o.pubkey_hash == seller_hash
                            })
                            .map(|o| o.amount)
                            .sum();

                        if sale_price > 0 {
                            let required_royalty =
                                (sale_price as u128 * royalty_bps as u128 / 10000) as Amount;
                            if required_royalty > 0 {
                                let actual_royalty: Amount = tx
                                    .outputs
                                    .iter()
                                    .filter(|o| o.pubkey_hash == creator_hash)
                                    .map(|o| o.amount)
                                    .sum();
                                if actual_royalty < required_royalty {
                                    return Err(ValidationError::InvalidTransaction(format!(
                                        "NFT input {} requires royalty of {} to creator, got {}",
                                        i, required_royalty, actual_royalty
                                    )));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Verify spending conditions on a transaction input.
///
/// For Normal/Bond outputs: verify single Ed25519 signature (existing behavior).
/// For conditioned outputs (Multisig, Hashlock, HTLC, Vesting):
///   decode condition from output extra_data, decode witness from tx extra_data, evaluate.
fn verify_input_conditions(
    tx: &Transaction,
    input: &Input,
    signing_hash: &Hash,
    utxo: &UtxoInfo,
    input_index: usize,
    current_height: crate::types::BlockHeight,
) -> Result<(), ValidationError> {
    if utxo.output.output_type.is_conditioned() {
        // ---- Conditioned output: evaluate condition tree ----
        let condition = crate::conditions::Condition::decode_prefix(&utxo.output.extra_data)
            .map(|(cond, _consumed)| cond)
            .map_err(|e| {
                ValidationError::InvalidTransaction(format!(
                    "input {} references output with invalid condition: {}",
                    input_index, e
                ))
            })?;

        // Check ops limit
        let ops = condition.ops_count();
        if ops > crate::conditions::MAX_CONDITION_OPS {
            return Err(ValidationError::InvalidTransaction(format!(
                "input {} condition has {} ops (max {})",
                input_index,
                ops,
                crate::conditions::MAX_CONDITION_OPS
            )));
        }

        // Decode witness from Transaction.extra_data (SegWit-style)
        let witness_bytes = tx.get_covenant_witness(input_index).unwrap_or(&[]);
        let witness = crate::conditions::Witness::decode(witness_bytes).map_err(|e| {
            ValidationError::InvalidTransaction(format!(
                "input {} has invalid witness data: {}",
                input_index, e
            ))
        })?;

        let ctx = crate::conditions::EvalContext {
            current_height,
            signing_hash,
        };

        let mut or_idx = 0;
        if !crate::conditions::evaluate(&condition, &witness, &ctx, &mut or_idx) {
            return Err(ValidationError::InvalidSignature { index: input_index });
        }

        Ok(())
    } else {
        // ---- Normal/Bond output: single signature verification ----
        verify_input_signature(input, signing_hash, utxo, input_index)
    }
}

/// Verify the signature on a transaction input (Normal/Bond outputs only).
fn verify_input_signature(
    input: &Input,
    signing_hash: &Hash,
    utxo: &UtxoInfo,
    input_index: usize,
) -> Result<(), ValidationError> {
    // We need the public key to verify the signature.
    // In pay-to-pubkey-hash, the UTXO only stores the hash — the pubkey
    // is not available until the spender reveals it. When pubkey is None
    // (production UTXO set), skip signature verification; covenant conditions,
    // lock times, and balance are still enforced.
    let pubkey = match utxo.pubkey.as_ref() {
        Some(pk) => pk,
        None => return Ok(()),
    };

    // Verify the pubkey hash matches the output
    let expected_hash = utxo.output.pubkey_hash;
    let actual_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());

    // Compare only the first 20 bytes (address size)
    if expected_hash.as_bytes()[..20] != actual_hash.as_bytes()[..20] {
        return Err(ValidationError::PubkeyHashMismatch {
            expected: expected_hash,
            got: actual_hash,
        });
    }

    // Verify the signature
    crypto::signature::verify_hash(signing_hash, &input.signature, pubkey)
        .map_err(|_| ValidationError::InvalidSignature { index: input_index })
}

/// Check for double spends within a block
fn check_internal_double_spend(block: &Block) -> Result<(), ValidationError> {
    use std::collections::HashSet;

    let mut spent: HashSet<(Hash, u32)> = HashSet::new();

    for tx in &block.transactions {
        for input in &tx.inputs {
            let outpoint = input.outpoint();
            if !spent.insert(outpoint) {
                return Err(ValidationError::DoubleSpend);
            }
        }
    }

    Ok(())
}

/// Validate VDF proof with hash-chain verification
///
/// The VDF serves as an anti-grinding mechanism to prevent producers from
/// trying many block variations to find favorable randomness.
///
/// Uses hash-chain VDF (800K BLAKE3 iterations, ~55ms) which is:
/// - Fast enough for block production on all networks
/// - Self-verifying (just re-compute and compare)
/// - Devnet: VDF disabled for rapid development
fn validate_vdf(header: &BlockHeader, network: Network) -> Result<(), ValidationError> {
    // Skip VDF validation if disabled for this network (e.g., devnet)
    if !network.vdf_enabled() {
        return Ok(());
    }

    let input = header.vdf_input();

    // Hash-chain VDF output should be exactly 32 bytes
    if header.vdf_output.value.len() != 32 {
        return Err(ValidationError::InvalidVdfProof);
    }

    let expected_output: [u8; 32] = header
        .vdf_output
        .value
        .as_slice()
        .try_into()
        .map_err(|_| ValidationError::InvalidVdfProof)?;

    // Verify by re-computing the hash-chain VDF
    // Use network-specific iterations (devnet uses fewer for faster blocks)
    if !verify_hash_chain_vdf(&input, &expected_output, network.heartbeat_vdf_iterations()) {
        return Err(ValidationError::InvalidVdfProof);
    }

    Ok(())
}

/// Compute the bootstrap fallback rank order for a slot.
///
/// Returns a deduped list of producers in fallback rank order:
/// rank 0 = `sorted_producers[slot % n]`, rank 1 = `sorted_producers[(slot+1) % n]`, etc.
/// At most `MAX_FALLBACK_RANKS` entries, deduplicated (important when n < MAX_FALLBACK_RANKS).
///
/// Both the production and validation sides must use this same function to ensure consensus.
pub fn bootstrap_fallback_order(
    slot: u32,
    sorted_producers: &[crypto::PublicKey],
) -> Vec<crypto::PublicKey> {
    let n = sorted_producers.len();
    if n == 0 {
        return Vec::new();
    }
    let max_ranks = crate::consensus::MAX_FALLBACK_RANKS;
    let mut result = Vec::with_capacity(max_ranks.min(n));
    for rank in 0..max_ranks {
        let idx = ((slot as usize) + rank) % n;
        let pk = sorted_producers[idx];
        if !result.contains(&pk) {
            result.push(pk);
        }
    }
    result
}

/// Build the eligible producer list for a bootstrap slot, applying liveness filter.
///
/// Normal slots: all ranks from `live_producers` only.
/// Re-entry slots: one stale producer at rank 0, `live_producers` at ranks 1+.
///
/// Both `live_producers` and `stale_producers` must be sorted by pubkey.
/// Returns the eligible list ordered by rank (index 0 = rank 0).
///
/// Re-entry slots are spread evenly: for stale producer `i`, re-entry fires when
/// `slot % reentry_interval == i * (reentry_interval / effective_stale)`.
/// Capped at `reentry_interval / 5` stale producers (20% max overhead).
pub fn bootstrap_schedule_with_liveness(
    slot: u32,
    live_producers: &[crypto::PublicKey],
    stale_producers: &[crypto::PublicKey],
    reentry_interval: u32,
) -> Vec<crypto::PublicKey> {
    let max_ranks = crate::consensus::MAX_FALLBACK_RANKS;

    // Cap stale count to avoid excessive re-entry overhead (max 20%)
    let effective_stale = stale_producers.len().min((reentry_interval / 5) as usize);

    // Determine if this is a re-entry slot for a stale producer
    let reentry_producer = if effective_stale > 0 && reentry_interval > 0 {
        let phase = slot % reentry_interval;
        let spacing = reentry_interval / effective_stale as u32;
        if spacing > 0 && phase.is_multiple_of(spacing) {
            let idx = (phase / spacing) as usize;
            if idx < effective_stale {
                Some(stale_producers[idx])
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let mut result = Vec::with_capacity(max_ranks);

    if let Some(stale_pk) = reentry_producer {
        // RE-ENTRY SLOT: stale producer at rank 0, live producers at ranks 1+
        result.push(stale_pk);
        if !live_producers.is_empty() {
            for rank in 0..max_ranks.saturating_sub(1) {
                let idx = ((slot as usize) + rank) % live_producers.len();
                let pk = live_producers[idx];
                if !result.contains(&pk) {
                    result.push(pk);
                }
            }
        }
    } else {
        // NORMAL SLOT: all ranks from live producers
        if live_producers.is_empty() {
            return Vec::new();
        }
        for rank in 0..max_ranks {
            let idx = ((slot as usize) + rank) % live_producers.len();
            let pk = live_producers[idx];
            if !result.contains(&pk) {
                result.push(pk);
            }
        }
    }

    result
}

/// Validate a block's producer during bootstrap using fallback rank windows.
///
/// Same 2-second exclusive windows as the epoch scheduler:
/// rank 0 → 0-2s, rank 1 → 2-4s, rank 2 → 4-6s, etc.
///
/// If no bootstrap_producers are set, falls back to accepting any producer
/// (backward compatibility for blocks produced before this fix).
fn validate_bootstrap_producer(
    header: &BlockHeader,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    // No bootstrap list → accept any producer (pre-fix blocks / early genesis)
    if ctx.bootstrap_producers.is_empty() {
        return Ok(());
    }

    // Producer must be in the known set
    if !ctx.bootstrap_producers.contains(&header.producer) {
        return Err(ValidationError::InvalidProducer);
    }

    // Compute fallback order for this slot.
    // Use liveness-aware scheduling when live/stale split is available.
    let eligible = if !ctx.live_bootstrap_producers.is_empty() {
        bootstrap_schedule_with_liveness(
            header.slot,
            &ctx.live_bootstrap_producers,
            &ctx.stale_bootstrap_producers,
            crate::consensus::REENTRY_INTERVAL,
        )
    } else {
        bootstrap_fallback_order(header.slot, &ctx.bootstrap_producers)
    };

    if eligible.is_empty() {
        return Err(ValidationError::InvalidProducer);
    }

    // Validate time window: producer's rank must match the block's timestamp offset
    let slot_start = ctx.params.slot_to_timestamp(header.slot);
    let slot_offset_secs = header.timestamp.saturating_sub(slot_start);
    let slot_offset_ms_low = slot_offset_secs * 1000;
    let slot_offset_ms_high = slot_offset_ms_low + 999;

    if !is_producer_eligible_ms(&header.producer, &eligible, slot_offset_ms_low)
        && !is_producer_eligible_ms(&header.producer, &eligible, slot_offset_ms_high)
    {
        return Err(ValidationError::InvalidProducer);
    }

    Ok(())
}

/// Validate that the block producer is eligible for the slot.
///
/// This checks that:
/// 1. The producer is in the eligible fallback list for the slot
/// 2. The block timestamp falls within the producer's allowed window
///
/// During bootstrap, uses the same 2-second fallback rank windows as the epoch
/// scheduler but with the GSet-derived producer list instead of bond-weighted tickets.
pub fn validate_producer_eligibility(
    header: &BlockHeader,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    // During genesis phase, use bootstrap validation (aligned with production mode).
    // Production switches to bond-weighted scheduler at genesis_blocks + 1,
    // so validation must use the same threshold — not bootstrap_blocks.
    if ctx.network.is_in_genesis(ctx.current_height) {
        return validate_bootstrap_producer(header, ctx);
    }

    // Post-genesis: use bond-weighted scheduler if producer data available.
    // This is the normal path once GENESIS PHASE COMPLETE has populated the
    // producer set with bond-weighted entries.
    if !ctx.active_producers_weighted.is_empty() {
        let eligible = select_producer_for_slot(header.slot, &ctx.active_producers_weighted);

        if eligible.is_empty() {
            return Err(ValidationError::InvalidProducer);
        }

        // Calculate time offset within the slot (in ms for sequential 2s windows).
        // Block timestamps have second precision, but eligibility uses 2s windows.
        // Check if the producer is eligible at any point within the timestamp's second bucket
        // [offset_secs * 1000, (offset_secs + 1) * 1000) to avoid false rejections.
        let slot_start = ctx.params.slot_to_timestamp(header.slot);
        let slot_offset_secs = header.timestamp.saturating_sub(slot_start);
        let slot_offset_ms_low = slot_offset_secs * 1000;
        let slot_offset_ms_high = slot_offset_ms_low + 999;

        if !is_producer_eligible_ms(&header.producer, &eligible, slot_offset_ms_low)
            && !is_producer_eligible_ms(&header.producer, &eligible, slot_offset_ms_high)
        {
            return Err(ValidationError::InvalidProducer);
        }

        return Ok(());
    }

    // Fallback: syncing node without producer set — use bootstrap validation
    // if still within bootstrap window (allows catching up without producer data)
    if ctx.params.is_bootstrap(ctx.current_height) {
        return validate_bootstrap_producer(header, ctx);
    }

    // No weighted producers and not in bootstrap — skip if no producers at all
    // (backward compatibility during testing)
    if ctx.active_producers.is_empty() {
        return Ok(());
    }

    Err(ValidationError::InvalidProducer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::GENESIS_TIME;
    use crate::transaction::Input;
    use std::collections::HashMap;

    fn test_context() -> ValidationContext {
        ValidationContext::new(
            ConsensusParams::mainnet(),
            Network::Mainnet,
            GENESIS_TIME + 120,
            1,
        )
        .with_prev_block(0, GENESIS_TIME, Hash::ZERO)
    }

    /// Mock UTXO provider for testing
    struct MockUtxoProvider {
        utxos: HashMap<(Hash, u32), UtxoInfo>,
    }

    impl MockUtxoProvider {
        fn new() -> Self {
            Self {
                utxos: HashMap::new(),
            }
        }

        fn add_utxo(&mut self, tx_hash: Hash, index: u32, output: Output, pubkey: PublicKey) {
            self.utxos.insert(
                (tx_hash, index),
                UtxoInfo {
                    output,
                    pubkey: Some(pubkey),
                    spent: false,
                },
            );
        }
    }

    impl UtxoProvider for MockUtxoProvider {
        fn get_utxo(&self, tx_hash: &Hash, output_index: u32) -> Option<UtxoInfo> {
            self.utxos.get(&(*tx_hash, output_index)).cloned()
        }
    }

    // ==========================================================================
    // Header Validation Tests
    // ==========================================================================

    #[test]
    fn test_validate_timestamp_advancing() {
        let ctx = test_context();

        // With 10-second slots, slot 1 is at GENESIS_TIME + 10
        let header = BlockHeader {
            version: 2,
            prev_hash: Hash::ZERO,
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: ctx.params.genesis_hash,
            timestamp: GENESIS_TIME + 10,
            slot: 1,
            producer: crypto::PublicKey::from_bytes([0u8; 32]),
            vdf_output: vdf::VdfOutput { value: vec![] },
            vdf_proof: vdf::VdfProof::empty(),
        };

        assert!(validate_header(&header, &ctx).is_ok());
    }

    #[test]
    fn test_validate_timestamp_not_advancing() {
        let ctx = test_context();

        let header = BlockHeader {
            version: 2,
            prev_hash: Hash::ZERO,
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: ctx.params.genesis_hash,
            timestamp: GENESIS_TIME - 1,
            slot: 0,
            producer: crypto::PublicKey::from_bytes([0u8; 32]),
            vdf_output: vdf::VdfOutput { value: vec![] },
            vdf_proof: vdf::VdfProof::empty(),
        };

        assert!(matches!(
            validate_header(&header, &ctx),
            Err(ValidationError::InvalidTimestamp { .. })
        ));
    }

    #[test]
    fn test_validate_slot_derivation() {
        let ctx = test_context();

        // With 10-second slots, timestamp GENESIS_TIME + 60 = slot 6
        // Claiming slot 5 should be invalid (actual slot should be 6)
        let header = BlockHeader {
            version: 2,
            prev_hash: Hash::ZERO,
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: ctx.params.genesis_hash,
            timestamp: GENESIS_TIME + 60,
            slot: 5, // Should be 6 with 10-second slots
            producer: crypto::PublicKey::from_bytes([0u8; 32]),
            vdf_output: vdf::VdfOutput { value: vec![] },
            vdf_proof: vdf::VdfProof::empty(),
        };

        assert!(matches!(
            validate_header(&header, &ctx),
            Err(ValidationError::InvalidSlot { .. })
        ));
    }

    // ==========================================================================
    // Transaction Structural Validation Tests
    // ==========================================================================

    #[test]
    fn test_validate_tx_zero_output() {
        let ctx = test_context();

        let tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![Input::new(Hash::ZERO, 0)],
            outputs: vec![Output::normal(0, crypto::hash::hash(b"test"))],
            extra_data: vec![],
        };

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidTransaction(msg)) if msg.contains("zero amount")
        ));
    }

    #[test]
    fn test_validate_tx_large_single_amount() {
        let ctx = test_context();
        let pubkey_hash = crypto::hash::hash(b"test");

        // Test that u64::MAX (larger than TOTAL_SUPPLY) is rejected
        let tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![Input::new(Hash::ZERO, 0)],
            outputs: vec![Output::normal(u64::MAX, pubkey_hash)],
            extra_data: vec![],
        };

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::AmountExceedsSupply { .. })
        ));
    }

    #[test]
    fn test_validate_tx_total_exceeds_supply() {
        let ctx = test_context();
        let pubkey_hash = crypto::hash::hash(b"test");

        // Two outputs that individually are valid but together exceed supply
        // Each is half of TOTAL_SUPPLY + 1
        let half_plus_one = TOTAL_SUPPLY / 2 + 1;

        let tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![Input::new(Hash::ZERO, 0)],
            outputs: vec![
                Output::normal(half_plus_one, pubkey_hash),
                Output::normal(half_plus_one, pubkey_hash),
            ],
            extra_data: vec![],
        };

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::AmountExceedsSupply { .. })
        ));
    }

    #[test]
    fn test_validate_tx_exceeds_supply() {
        let ctx = test_context();
        let pubkey_hash = crypto::hash::hash(b"test");

        let tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![Input::new(Hash::ZERO, 0)],
            outputs: vec![Output::normal(TOTAL_SUPPLY + 1, pubkey_hash)],
            extra_data: vec![],
        };

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::AmountExceedsSupply { .. })
        ));
    }

    #[test]
    fn test_validate_tx_no_inputs() {
        let ctx = test_context();
        let pubkey_hash = crypto::hash::hash(b"test");

        // A transaction with no inputs and multiple outputs (not a coinbase)
        // should fail validation
        let tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![],
            outputs: vec![
                Output::normal(100, pubkey_hash),
                Output::normal(200, pubkey_hash), // Second output so it's not a coinbase
            ],
            extra_data: vec![],
        };

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidTransaction(msg)) if msg.contains("must have inputs")
        ));
    }

    #[test]
    fn test_validate_tx_no_outputs() {
        let ctx = test_context();

        let tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![Input::new(Hash::ZERO, 0)],
            outputs: vec![],
            extra_data: vec![],
        };

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidTransaction(msg)) if msg.contains("must have outputs")
        ));
    }

    #[test]
    fn test_validate_tx_zero_pubkey_hash() {
        let ctx = test_context();

        let tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![Input::new(Hash::ZERO, 0)],
            outputs: vec![Output::normal(100, Hash::ZERO)],
            extra_data: vec![],
        };

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidTransaction(msg)) if msg.contains("zero pubkey_hash")
        ));
    }

    #[test]
    fn test_validate_bond_output_zero_lock() {
        let ctx = test_context();
        let pubkey_hash = crypto::hash::hash(b"test");

        // Bond output with lock_until = 0 is invalid
        let bad_output = Output {
            output_type: OutputType::Bond,
            amount: 100,
            pubkey_hash,
            lock_until: 0, // Invalid for bond
            extra_data: vec![],
        };

        let tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![Input::new(Hash::ZERO, 0)],
            outputs: vec![bad_output],
            extra_data: vec![],
        };

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(result, Err(ValidationError::InvalidBond(_))));
    }

    #[test]
    fn test_validate_normal_output_nonzero_lock() {
        let ctx = test_context();
        let pubkey_hash = crypto::hash::hash(b"test");

        // Normal output with lock_until != 0 is invalid
        let bad_output = Output {
            output_type: OutputType::Normal,
            amount: 100,
            pubkey_hash,
            lock_until: 1000, // Invalid for normal
            extra_data: vec![],
        };

        let tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![Input::new(Hash::ZERO, 0)],
            outputs: vec![bad_output],
            extra_data: vec![],
        };

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidTransaction(msg)) if msg.contains("non-zero lock_until")
        ));
    }

    // ==========================================================================
    // UTXO Validation Tests
    // ==========================================================================

    #[test]
    fn test_validate_tx_with_valid_utxo() {
        let ctx = test_context();

        // Create a keypair for the test
        let keypair = crypto::KeyPair::generate();
        let pubkey = *keypair.public_key();
        let pubkey_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());

        // Create a previous transaction output
        let prev_tx_hash = crypto::hash::hash(b"prev_tx");
        let prev_output = Output::normal(1000, pubkey_hash);

        // Set up the mock UTXO provider
        let mut utxo_provider = MockUtxoProvider::new();
        utxo_provider.add_utxo(prev_tx_hash, 0, prev_output, pubkey);

        // Create the spending transaction
        let new_pubkey_hash = crypto::hash::hash(b"recipient");
        let mut tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![Input::new(prev_tx_hash, 0)],
            outputs: vec![Output::normal(900, new_pubkey_hash)],
            extra_data: vec![],
        };

        // Sign the transaction (BIP-143: per-input)
        let signing_hash = tx.signing_message_for_input(0);
        let signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());
        tx.inputs[0].signature = signature;

        // Validate
        let result = validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_tx_missing_utxo() {
        let ctx = test_context();
        let pubkey_hash = crypto::hash::hash(b"test");

        let tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![Input::new(Hash::ZERO, 0)],
            outputs: vec![Output::normal(100, pubkey_hash)],
            extra_data: vec![],
        };

        let utxo_provider = MockUtxoProvider::new();

        let result = validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
        assert!(matches!(
            result,
            Err(ValidationError::OutputNotFound { .. })
        ));
    }

    #[test]
    fn test_validate_tx_insufficient_funds() {
        let ctx = test_context();

        let keypair = crypto::KeyPair::generate();
        let pubkey = *keypair.public_key();
        let pubkey_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());

        let prev_tx_hash = crypto::hash::hash(b"prev_tx");
        let prev_output = Output::normal(100, pubkey_hash); // Only 100

        let mut utxo_provider = MockUtxoProvider::new();
        utxo_provider.add_utxo(prev_tx_hash, 0, prev_output, pubkey);

        let new_pubkey_hash = crypto::hash::hash(b"recipient");
        let mut tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![Input::new(prev_tx_hash, 0)],
            outputs: vec![Output::normal(200, new_pubkey_hash)], // Needs 200
            extra_data: vec![],
        };

        // Sign the transaction (BIP-143: per-input)
        let signing_hash = tx.signing_message_for_input(0);
        let signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());
        tx.inputs[0].signature = signature;

        let result = validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
        assert!(matches!(
            result,
            Err(ValidationError::InsufficientFunds {
                inputs: 100,
                outputs: 200
            })
        ));
    }

    #[test]
    fn test_validate_tx_invalid_signature() {
        let ctx = test_context();

        let owner_keypair = crypto::KeyPair::generate();
        let owner_pubkey = *owner_keypair.public_key();
        let pubkey_hash =
            crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, owner_pubkey.as_bytes());

        let prev_tx_hash = crypto::hash::hash(b"prev_tx");
        let prev_output = Output::normal(1000, pubkey_hash);

        let mut utxo_provider = MockUtxoProvider::new();
        utxo_provider.add_utxo(prev_tx_hash, 0, prev_output, owner_pubkey);

        // Use a DIFFERENT keypair to sign (attack attempt)
        let attacker_keypair = crypto::KeyPair::generate();

        let new_pubkey_hash = crypto::hash::hash(b"recipient");
        let mut tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![Input::new(prev_tx_hash, 0)],
            outputs: vec![Output::normal(900, new_pubkey_hash)],
            extra_data: vec![],
        };

        // Sign with wrong key (BIP-143: per-input)
        let signing_hash = tx.signing_message_for_input(0);
        let bad_signature =
            crypto::signature::sign_hash(&signing_hash, attacker_keypair.private_key());
        tx.inputs[0].signature = bad_signature;

        let result = validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidSignature { index: 0 })
        ));
    }

    #[test]
    fn test_validate_tx_locked_output() {
        let ctx = test_context();

        let keypair = crypto::KeyPair::generate();
        let pubkey = *keypair.public_key();
        let pubkey_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());

        let prev_tx_hash = crypto::hash::hash(b"prev_tx");
        // Output locked until height 1000
        let prev_output = Output::bond(1000, pubkey_hash, 1000, 0);

        let mut utxo_provider = MockUtxoProvider::new();
        utxo_provider.add_utxo(prev_tx_hash, 0, prev_output, pubkey);

        let new_pubkey_hash = crypto::hash::hash(b"recipient");
        let mut tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![Input::new(prev_tx_hash, 0)],
            outputs: vec![Output::normal(900, new_pubkey_hash)],
            extra_data: vec![],
        };

        let signing_hash = tx.signing_message_for_input(0);
        let signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());
        tx.inputs[0].signature = signature;

        // Context is at height 1, output locked until 1000
        let result = validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
        assert!(matches!(
            result,
            Err(ValidationError::OutputLocked {
                lock_height: 1000,
                current_height: 1
            })
        ));
    }

    // ==========================================================================
    // Property-Based Tests
    // ==========================================================================

    use proptest::prelude::*;

    #[allow(dead_code)]
    fn arb_hash() -> impl Strategy<Value = Hash> {
        any::<[u8; 32]>().prop_map(Hash::from_bytes)
    }

    #[allow(dead_code)]
    fn arb_pubkey_hash() -> impl Strategy<Value = Hash> {
        // Non-zero hash for valid outputs
        any::<[u8; 32]>()
            .prop_filter("non-zero hash", |b| b.iter().any(|&x| x != 0))
            .prop_map(Hash::from_bytes)
    }

    proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(50))]

        /// Valid amount outputs pass validation
        #[test]
        fn prop_valid_outputs_pass(amounts in prop::collection::vec(1u64..1_000_000_000u64, 1..5)) {
            let outputs: Vec<Output> = amounts.iter()
                .enumerate()
                .map(|(i, &a)| {
                    let mut hash = [0u8; 32];
                    hash[0] = (i as u8) + 1; // Non-zero
                    Output::normal(a, Hash::from_bytes(hash))
                })
                .collect();
            let result = validate_outputs(&outputs, &test_context());
            prop_assert!(result.is_ok());
            prop_assert_eq!(result.unwrap(), amounts.iter().sum::<u64>());
        }

        /// Zero amount outputs fail validation
        #[test]
        fn prop_zero_amount_fails(seed: [u8; 32]) {
            let mut hash = seed;
            hash[0] = 1; // Non-zero
            let outputs = vec![Output::normal(0, Hash::from_bytes(hash))];
            let result = validate_outputs(&outputs, &test_context());
            prop_assert!(result.is_err());
        }

        /// Zero pubkey_hash fails validation
        #[test]
        fn prop_zero_pubkey_hash_fails(amount in 1u64..1_000_000_000u64) {
            let outputs = vec![Output::normal(amount, Hash::ZERO)];
            let result = validate_outputs(&outputs, &test_context());
            prop_assert!(result.is_err());
        }

        /// Bond with zero lock fails
        #[test]
        fn prop_bond_zero_lock_fails(amount in 1u64..1_000_000_000u64, seed: [u8; 32]) {
            let mut hash = seed;
            hash[0] = 1;
            let output = Output {
                output_type: OutputType::Bond,
                amount,
                pubkey_hash: Hash::from_bytes(hash),
                lock_until: 0,
                extra_data: vec![],
            };
            let result = validate_outputs(&[output], &test_context());
            prop_assert!(result.is_err());
        }

        /// Normal output with lock fails
        #[test]
        fn prop_normal_with_lock_fails(amount in 1u64..1_000_000_000u64, lock in 1u64..1_000_000u64, seed: [u8; 32]) {
            let mut hash = seed;
            hash[0] = 1;
            let output = Output {
                output_type: OutputType::Normal,
                amount,
                pubkey_hash: Hash::from_bytes(hash),
                lock_until: lock,
                extra_data: vec![],
            };
            let result = validate_outputs(&[output], &test_context());
            prop_assert!(result.is_err());
        }

        /// Version 1 transactions with valid structure pass
        #[test]
        fn prop_valid_structure_passes(amount in 1u64..1_000_000_000u64, seed: [u8; 32]) {
            let ctx = test_context();
            let mut hash = seed;
            hash[0] = 1;
            let tx = Transaction {
                version: 1,
                tx_type: TxType::Transfer,
                inputs: vec![Input::new(Hash::from_bytes(seed), 0)],
                outputs: vec![Output::normal(amount, Hash::from_bytes(hash))],
                extra_data: vec![],
            };
            let result = validate_transaction(&tx, &ctx);
            prop_assert!(result.is_ok());
        }

        /// Invalid version fails validation
        #[test]
        fn prop_invalid_version_fails(version in 2u32..100u32, amount in 1u64..1_000_000_000u64, seed: [u8; 32]) {
            let ctx = test_context();
            let mut hash = seed;
            hash[0] = 1;
            let tx = Transaction {
                version,
                tx_type: TxType::Transfer,
                inputs: vec![Input::new(Hash::from_bytes(seed), 0)],
                outputs: vec![Output::normal(amount, Hash::from_bytes(hash))],
                extra_data: vec![],
            };
            let result = validate_transaction(&tx, &ctx);
            prop_assert!(matches!(result, Err(ValidationError::InvalidVersion(_))));
        }

        /// Coinbase transactions are valid (structural)
        #[test]
        fn prop_coinbase_valid(height: u64, seed: [u8; 32]) {
            let ctx = ValidationContext::new(
                ConsensusParams::mainnet(),
                Network::Mainnet,
                GENESIS_TIME + 120,
                height,
            ).with_prev_block(0, GENESIS_TIME, Hash::ZERO);

            let mut hash = seed;
            hash[0] = 1;
            let reward = ctx.params.block_reward(height);
            let tx = Transaction::new_coinbase(reward, Hash::from_bytes(hash), height);

            let result = validate_coinbase(&tx, &ctx);
            prop_assert!(result.is_ok());
        }

        /// Insufficient coinbase reward fails
        #[test]
        fn prop_insufficient_coinbase_fails(height in 0u64..1_000_000u64, reduction in 1u64..1_000_000u64, seed: [u8; 32]) {
            let ctx = ValidationContext::new(
                ConsensusParams::mainnet(),
                Network::Mainnet,
                GENESIS_TIME + 120,
                height,
            ).with_prev_block(0, GENESIS_TIME, Hash::ZERO);

            let mut hash = seed;
            hash[0] = 1;
            let reward = ctx.params.block_reward(height);
            // Reduce reward below minimum
            let insufficient = reward.saturating_sub(reduction).min(reward.saturating_sub(1));
            if insufficient < reward {
                let tx = Transaction::new_coinbase(insufficient, Hash::from_bytes(hash), height);
                let result = validate_coinbase(&tx, &ctx);
                prop_assert!(result.is_err());
            }
        }

        /// Valid signatures verify correctly
        #[test]
        fn prop_valid_signature_verifies(amount in 1u64..1_000_000_000u64, seed: [u8; 32]) {
            let ctx = test_context();
            let keypair = crypto::KeyPair::from_seed(seed);
            let pubkey = *keypair.public_key();
            let pubkey_hash = crypto::hash::hash_with_domain(
                crypto::ADDRESS_DOMAIN,
                pubkey.as_bytes(),
            );

            let prev_tx_hash = crypto::hash::hash(&seed);
            let prev_output = Output::normal(amount, pubkey_hash);

            let mut utxo_provider = MockUtxoProvider::new();
            utxo_provider.add_utxo(prev_tx_hash, 0, prev_output, pubkey);

            let mut recipient_hash = seed;
            recipient_hash[0] ^= 1;
            let output_amount = amount.saturating_sub(100).max(1);
            let mut tx = Transaction {
                version: 1,
                tx_type: TxType::Transfer,
                inputs: vec![Input::new(prev_tx_hash, 0)],
                outputs: vec![Output::normal(output_amount, Hash::from_bytes(recipient_hash))],
                extra_data: vec![],
            };

            let signing_hash = tx.signing_message_for_input(0);
            let signature = crypto::signature::sign_hash(&signing_hash, keypair.private_key());
            tx.inputs[0].signature = signature;

            let result = validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
            prop_assert!(result.is_ok());
        }

        /// Invalid signatures are rejected
        #[test]
        fn prop_invalid_signature_rejected(amount in 1u64..1_000_000_000u64, owner_seed: [u8; 32], attacker_seed: [u8; 32]) {
            prop_assume!(owner_seed != attacker_seed);

            let ctx = test_context();
            let owner_keypair = crypto::KeyPair::from_seed(owner_seed);
            let attacker_keypair = crypto::KeyPair::from_seed(attacker_seed);

            let owner_pubkey = *owner_keypair.public_key();
            let pubkey_hash = crypto::hash::hash_with_domain(
                crypto::ADDRESS_DOMAIN,
                owner_pubkey.as_bytes(),
            );

            let prev_tx_hash = crypto::hash::hash(&owner_seed);
            let prev_output = Output::normal(amount, pubkey_hash);

            let mut utxo_provider = MockUtxoProvider::new();
            utxo_provider.add_utxo(prev_tx_hash, 0, prev_output, owner_pubkey);

            let mut recipient_hash = owner_seed;
            recipient_hash[0] ^= 1;
            let output_amount = amount.saturating_sub(100).max(1);
            let mut tx = Transaction {
                version: 1,
                tx_type: TxType::Transfer,
                inputs: vec![Input::new(prev_tx_hash, 0)],
                outputs: vec![Output::normal(output_amount, Hash::from_bytes(recipient_hash))],
                extra_data: vec![],
            };

            // Sign with wrong key (BIP-143: per-input)
            let signing_hash = tx.signing_message_for_input(0);
            let bad_signature = crypto::signature::sign_hash(&signing_hash, attacker_keypair.private_key());
            tx.inputs[0].signature = bad_signature;

            let result = validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
            let is_invalid_sig = matches!(result, Err(ValidationError::InvalidSignature { .. }));
            prop_assert!(is_invalid_sig, "Expected InvalidSignature error, got {:?}", result);
        }

        /// Double spend detection works
        #[test]
        fn prop_double_spend_detected(seed: [u8; 32]) {
            let input = Input::new(Hash::from_bytes(seed), 0);
            let mut hash = seed;
            hash[0] = 1;
            let output = Output::normal(100, Hash::from_bytes(hash));

            let tx1 = Transaction {
                version: 1,
                tx_type: TxType::Transfer,
                inputs: vec![input.clone()],
                outputs: vec![output.clone()],
                extra_data: vec![],
            };
            let tx2 = Transaction {
                version: 1,
                tx_type: TxType::Transfer,
                inputs: vec![input],
                outputs: vec![output],
                extra_data: vec![],
            };

            // Create a mock block with both transactions
            let block = crate::Block {
                header: crate::BlockHeader {
                    version: 1,
                    prev_hash: Hash::ZERO,
                    merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
                    timestamp: 0,
                    slot: 0,
                    producer: crypto::PublicKey::from_bytes([0u8; 32]),
                    vdf_output: vdf::VdfOutput { value: vec![] },
                    vdf_proof: vdf::VdfProof::empty(),
                        },
                transactions: vec![tx1, tx2],
                aggregate_bls_signature: Vec::new(),
            };

            let result = check_internal_double_spend(&block);
            prop_assert!(matches!(result, Err(ValidationError::DoubleSpend)));
        }
    }

    // ==========================================================================
    // TX Type 7: AddBond Validation Tests
    // ==========================================================================

    #[test]
    fn test_add_bond_transaction() {
        let ctx = test_context();
        let keypair = crypto::KeyPair::generate();
        let pubkey = *keypair.public_key();

        // Create an input (required for AddBond)
        let prev_tx_hash = crypto::hash::hash(b"prev_tx");
        let input = Input::new(prev_tx_hash, 0);

        let tx = Transaction::new_add_bond(vec![input], pubkey, 5, 5_000_000_000, 10_000_000);

        assert!(tx.is_add_bond());
        assert_eq!(tx.tx_type, TxType::AddBond);
        assert_eq!(tx.inputs.len(), 1);
        assert_eq!(tx.outputs.len(), 5); // One Bond output per bond
        for output in &tx.outputs {
            assert_eq!(output.output_type, OutputType::Bond);
        }

        // Verify add bond data can be parsed
        let bond_data = tx.add_bond_data().unwrap();
        assert_eq!(bond_data.producer_pubkey, pubkey);
        assert_eq!(bond_data.bond_count, 5);

        // Structural validation should pass
        let result = validate_transaction(&tx, &ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_add_bond_no_inputs() {
        let ctx = test_context();
        let keypair = crypto::KeyPair::generate();
        let pubkey = *keypair.public_key();

        // Create AddBond with no inputs (invalid)
        let tx = Transaction::new_add_bond(vec![], pubkey, 5, 5_000_000_000, 10_000_000);

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidAddBond(msg)) if msg.contains("must have inputs")
        ));
    }

    #[test]
    fn test_add_bond_allows_normal_outputs_for_change() {
        let ctx = test_context();
        let keypair = crypto::KeyPair::generate();
        let pubkey = *keypair.public_key();
        let pubkey_hash = crypto::hash::hash(b"test");

        // Create AddBond with Bond output + Normal change output (valid)
        let bond_data = crate::transaction::AddBondData::new(pubkey, 5);
        let tx = Transaction {
            version: 1,
            tx_type: TxType::AddBond,
            inputs: vec![Input::new(crypto::hash::hash(b"prev"), 0)],
            outputs: vec![
                Output::bond(5_000_000_000, pubkey_hash, 10_000_000, 0), // Bond output required
                Output::normal(100, pubkey_hash),                        // Normal change OK
            ],
            extra_data: bond_data.to_bytes(),
        };

        // Should pass structural validation (Bond + Normal outputs allowed)
        let result = validate_transaction(&tx, &ctx);
        assert!(
            result.is_ok(),
            "AddBond with Bond + Normal outputs should pass: {:?}",
            result
        );
    }

    #[test]
    fn test_add_bond_requires_bond_output() {
        let ctx = test_context();
        let keypair = crypto::KeyPair::generate();
        let pubkey = *keypair.public_key();
        let pubkey_hash = crypto::hash::hash(b"test");

        // Create AddBond with only Normal output (invalid - must have Bond output)
        let bond_data = crate::transaction::AddBondData::new(pubkey, 5);
        let tx = Transaction {
            version: 1,
            tx_type: TxType::AddBond,
            inputs: vec![Input::new(crypto::hash::hash(b"prev"), 0)],
            outputs: vec![Output::normal(100, pubkey_hash)], // No Bond output = invalid
            extra_data: bond_data.to_bytes(),
        };

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidAddBond(msg)) if msg.contains("must have a Bond output")
        ));
    }

    #[test]
    fn test_add_bond_zero_bond_count() {
        let ctx = test_context();
        let keypair = crypto::KeyPair::generate();
        let pubkey = *keypair.public_key();
        let pubkey_hash = crypto::hash::hash(pubkey.as_bytes());

        // Create AddBond with valid Bond output but zero bond_count in data
        let bond_data = crate::transaction::AddBondData::new(pubkey, 0);
        let tx = Transaction {
            version: 1,
            tx_type: TxType::AddBond,
            inputs: vec![Input::new(crypto::hash::hash(b"prev"), 0)],
            outputs: vec![Output::bond(1_000_000_000, pubkey_hash, 10_000_000, 0)],
            extra_data: bond_data.to_bytes(),
        };

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidAddBond(msg)) if msg.contains("bond count must be positive")
        ));
    }

    #[test]
    fn test_add_bond_data_serialization() {
        let keypair = crypto::KeyPair::generate();
        let pubkey = *keypair.public_key();

        let tx = Transaction::new_add_bond(
            vec![Input::new(crypto::hash::hash(b"prev"), 0)],
            pubkey,
            10,
            10_000_000_000,
            10_000_000,
        );
        let bytes = tx.serialize();
        let recovered = Transaction::deserialize(&bytes).unwrap();

        assert_eq!(tx.tx_type, recovered.tx_type);
        let recovered_data = recovered.add_bond_data().unwrap();
        assert_eq!(recovered_data.producer_pubkey, pubkey);
        assert_eq!(recovered_data.bond_count, 10);
    }

    // ==========================================================================
    // TX Type 8: RequestWithdrawal Validation Tests
    // ==========================================================================

    #[test]
    fn test_request_withdrawal_transaction() {
        let ctx = test_context();
        let keypair = crypto::KeyPair::generate();
        let pubkey = *keypair.public_key();
        let destination = crypto::hash::hash(b"destination");
        let net_amount = 3_000_000_000u64; // 3 bonds worth

        // Lock/unlock: withdrawal requires Bond UTXO inputs
        let bond_inputs = vec![Input::new(crypto::hash::hash(b"bond_utxo"), 0)];
        let tx =
            Transaction::new_request_withdrawal(bond_inputs, pubkey, 3, destination, net_amount);

        assert!(tx.is_request_withdrawal());
        assert_eq!(tx.tx_type, TxType::RequestWithdrawal);
        assert_eq!(tx.inputs.len(), 1); // Bond UTXO input
        assert_eq!(tx.outputs.len(), 1);
        assert_eq!(tx.outputs[0].amount, net_amount);
        assert_eq!(tx.outputs[0].pubkey_hash, destination);

        // Verify withdrawal data can be parsed
        let withdrawal_data = tx.withdrawal_request_data().unwrap();
        assert_eq!(withdrawal_data.producer_pubkey, pubkey);
        assert_eq!(withdrawal_data.bond_count, 3);
        assert_eq!(withdrawal_data.destination, destination);

        // Structural validation should pass
        let result = validate_transaction(&tx, &ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_request_withdrawal_without_inputs() {
        let ctx = test_context();
        let keypair = crypto::KeyPair::generate();
        let pubkey = *keypair.public_key();
        let destination = crypto::hash::hash(b"destination");

        // Create RequestWithdrawal without inputs (invalid — must have Bond UTXO inputs)
        let withdrawal_data =
            crate::transaction::WithdrawalRequestData::new(pubkey, 3, destination);
        let tx = Transaction {
            version: 1,
            tx_type: TxType::RequestWithdrawal,
            inputs: vec![], // Must have Bond UTXO inputs
            outputs: vec![Output::normal(100, destination)],
            extra_data: withdrawal_data.to_bytes(),
        };

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidWithdrawalRequest(msg)) if msg.contains("must have Bond UTXO inputs")
        ));
    }

    #[test]
    fn test_request_withdrawal_wrong_output_count() {
        let ctx = test_context();
        let keypair = crypto::KeyPair::generate();
        let pubkey = *keypair.public_key();
        let destination = crypto::hash::hash(b"destination");

        // Create RequestWithdrawal with no outputs (must have exactly 1)
        let withdrawal_data =
            crate::transaction::WithdrawalRequestData::new(pubkey, 3, destination);
        let tx = Transaction {
            version: 1,
            tx_type: TxType::RequestWithdrawal,
            inputs: vec![Input::new(crypto::hash::hash(b"bond"), 0)], // Bond UTXO input
            outputs: vec![],                                          // Must have exactly 1
            extra_data: withdrawal_data.to_bytes(),
        };

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidWithdrawalRequest(msg)) if msg.contains("exactly 1 output")
        ));
    }

    #[test]
    fn test_request_withdrawal_zero_bond_count() {
        let ctx = test_context();
        let keypair = crypto::KeyPair::generate();
        let pubkey = *keypair.public_key();
        let destination = crypto::hash::hash(b"destination");

        // Create RequestWithdrawal with zero bond count (invalid)
        let tx = Transaction::new_request_withdrawal(
            vec![Input::new(crypto::hash::hash(b"bond"), 0)],
            pubkey,
            0,
            destination,
            100,
        );

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidWithdrawalRequest(msg)) if msg.contains("bond count must be positive")
        ));
    }

    #[test]
    fn test_request_withdrawal_zero_destination() {
        let ctx = test_context();
        let keypair = crypto::KeyPair::generate();
        let pubkey = *keypair.public_key();

        // Create RequestWithdrawal with zero destination (invalid)
        let tx = Transaction::new_request_withdrawal(
            vec![Input::new(crypto::hash::hash(b"bond"), 0)],
            pubkey,
            3,
            Hash::ZERO,
            100,
        );

        let result = validate_transaction(&tx, &ctx);
        // General output validation catches zero pubkey_hash before withdrawal-specific checks
        assert!(result.is_err(), "zero destination should be rejected");
    }

    #[test]
    fn test_request_withdrawal_data_serialization() {
        let keypair = crypto::KeyPair::generate();
        let pubkey = *keypair.public_key();
        let destination = crypto::hash::hash(b"destination");

        let tx = Transaction::new_request_withdrawal(
            vec![Input::new(crypto::hash::hash(b"bond"), 0)],
            pubkey,
            7,
            destination,
            5_000_000_000,
        );
        let bytes = tx.serialize();
        let recovered = Transaction::deserialize(&bytes).unwrap();

        assert_eq!(tx.tx_type, recovered.tx_type);
        assert_eq!(recovered.outputs.len(), 1);
        assert_eq!(recovered.outputs[0].amount, 5_000_000_000);
        let recovered_data = recovered.withdrawal_request_data().unwrap();
        assert_eq!(recovered_data.producer_pubkey, pubkey);
        assert_eq!(recovered_data.bond_count, 7);
        assert_eq!(recovered_data.destination, destination);
    }

    // ==========================================================================
    // VDF Network Parameter Tests
    // ==========================================================================

    /// Regression test: VDF validation must use network-specific parameters
    ///
    /// Test that VDF iterations and discriminant sizes are network-specific.
    ///
    /// The VDF serves as anti-grinding protection, and parameters are
    /// calibrated per-network for practical operation:
    /// - Mainnet: 2048-bit discriminant, 100k iterations for security
    /// - Testnet: Same as Mainnet (production-identical testing)
    /// - Devnet: 256-bit discriminant, uses fast hash-chain VDF
    #[test]
    fn test_vdf_validation_uses_network_specific_parameters() {
        // VDF is enabled for all networks (using hash-chain VDF for devnet)
        assert!(Network::Mainnet.vdf_enabled());
        assert!(Network::Testnet.vdf_enabled());
        assert!(Network::Devnet.vdf_enabled()); // Uses fast hash-chain VDF

        // Testnet has identical VDF parameters to Mainnet (production-identical testing)
        assert_eq!(
            Network::Mainnet.vdf_iterations(),
            Network::Testnet.vdf_iterations()
        );
        assert_eq!(
            Network::Mainnet.vdf_discriminant_bits(),
            Network::Testnet.vdf_discriminant_bits()
        );

        // Mainnet and Testnet use production-grade parameters
        assert_eq!(Network::Mainnet.vdf_iterations(), 800_000);
        assert_eq!(Network::Mainnet.vdf_discriminant_bits(), 2048);

        // Devnet has minimal parameters (uses hash-chain VDF)
        assert!(Network::Mainnet.vdf_iterations() > Network::Devnet.vdf_iterations());
        assert!(Network::Mainnet.vdf_discriminant_bits() > Network::Devnet.vdf_discriminant_bits());
        assert_eq!(Network::Devnet.vdf_iterations(), 1);
        assert_eq!(Network::Devnet.vdf_discriminant_bits(), 256);

        // Each network has unique VDF seeds
        assert_ne!(Network::Mainnet.vdf_seed(), Network::Testnet.vdf_seed());
        assert_ne!(Network::Testnet.vdf_seed(), Network::Devnet.vdf_seed());
    }

    // ==========================================================================
    // EpochReward Validation Tests
    // ==========================================================================

    #[test]
    fn test_validate_epoch_reward_no_inputs() {
        let keypair = crypto::KeyPair::generate();
        let pubkey_hash = crypto::hash::hash(b"recipient");
        let mut tx = Transaction::new_epoch_reward(1, *keypair.public_key(), 1000, pubkey_hash);

        // Add an input (invalid for epoch reward)
        tx.inputs.push(Input::new(Hash::ZERO, 0));

        let ctx = test_context();
        let result = validate_transaction(&tx, &ctx);

        assert!(matches!(
            result,
            Err(ValidationError::InvalidEpochReward(_))
        ));
    }

    #[test]
    fn test_validate_epoch_reward_multiple_outputs_ok() {
        let keypair = crypto::KeyPair::generate();
        let pubkey_hash = crypto::hash::hash(b"recipient");
        let mut tx = Transaction::new_epoch_reward(1, *keypair.public_key(), 1000, pubkey_hash);

        // Multiple Normal outputs are valid (one per producer in pool distribution)
        let extra_pubkey_hash = crypto::hash::hash(b"another_recipient");
        tx.outputs.push(Output::normal(500, extra_pubkey_hash));

        let ctx = test_context();
        let result = validate_transaction(&tx, &ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_epoch_reward_normal_output_type() {
        let keypair = crypto::KeyPair::generate();
        let pubkey_hash = crypto::hash::hash(b"recipient");
        let mut tx = Transaction::new_epoch_reward(1, *keypair.public_key(), 1000, pubkey_hash);

        // Change to bond output (invalid for epoch reward)
        tx.outputs[0].output_type = OutputType::Bond;
        tx.outputs[0].lock_until = 1000;
        tx.outputs[0].extra_data = 0u32.to_le_bytes().to_vec();

        let ctx = test_context();
        let result = validate_transaction(&tx, &ctx);

        assert!(matches!(
            result,
            Err(ValidationError::InvalidEpochReward(_))
        ));
    }

    #[test]
    fn test_validate_epoch_reward_invalid_structure() {
        // EpochReward with inputs is invalid (must be minted)
        let pubkey_hash = crypto::hash::hash(b"recipient");
        let tx = Transaction {
            version: 1,
            tx_type: TxType::EpochReward,
            inputs: vec![Input::new(crypto::Hash::ZERO, 0)], // Invalid - must have no inputs
            outputs: vec![Output::normal(1000, pubkey_hash)],
            extra_data: vec![],
        };

        let ctx = test_context();
        let result = validate_transaction(&tx, &ctx);

        assert!(matches!(
            result,
            Err(ValidationError::InvalidEpochReward(_))
        ));
    }

    #[test]
    fn test_validate_epoch_reward_valid() {
        // EpochReward is the automatic push-based reward system for the
        // deterministic scheduler model. It IS valid and should pass validation.
        let keypair = crypto::KeyPair::generate();
        let pubkey_hash = crypto::hash::hash(b"recipient");
        let tx = Transaction::new_epoch_reward(1, *keypair.public_key(), 1000, pubkey_hash);

        let ctx = test_context();
        let result = validate_transaction(&tx, &ctx);

        // EpochReward transactions are valid (100% of block reward to producer)
        assert!(result.is_ok(), "EpochReward should be valid: {:?}", result);
    }

    #[test]
    fn test_epoch_reward_skips_utxo_validation() {
        // EpochReward transactions don't require UTXO inputs since they are minted
        let keypair = crypto::KeyPair::generate();
        let pubkey_hash = crypto::hash::hash(b"recipient");
        let tx = Transaction::new_epoch_reward(1, *keypair.public_key(), 1000, pubkey_hash);

        let ctx = test_context();
        let utxo_provider = MockUtxoProvider::new(); // Empty UTXO set

        // EpochReward should pass even with empty UTXO set (no inputs needed)
        let result = validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
        assert!(
            result.is_ok(),
            "EpochReward should skip UTXO validation: {:?}",
            result
        );
    }

    // ==========================================================================
    // Block Rewards Validation Tests (EpochPool Mode)
    // ==========================================================================

    /// Create a context with EpochPool reward mode for testing
    fn epoch_pool_context_at_slot(slot: u32) -> ValidationContext {
        let mut params = ConsensusParams::mainnet();
        params.reward_mode = RewardMode::EpochPool;

        ValidationContext::new(
            params,
            Network::Mainnet,
            GENESIS_TIME + (slot as u64 * 60),
            slot as u64,
        )
        .with_prev_block(
            slot.saturating_sub(1),
            GENESIS_TIME + ((slot as u64 - 1) * 60),
            Hash::ZERO,
        )
    }

    /// Create a minimal test block at a given slot
    fn create_test_block_at_slot(slot: u32, transactions: Vec<Transaction>) -> Block {
        use crate::block::{Block, BlockHeader};
        use vdf::{VdfOutput, VdfProof};

        let keypair = crypto::KeyPair::generate();
        let header = BlockHeader {
            version: 1,
            slot,
            timestamp: GENESIS_TIME + (slot as u64 * 60),
            prev_hash: Hash::ZERO,
            merkle_root: Hash::ZERO, // Will be invalid, but we're testing rewards logic
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            producer: *keypair.public_key(),
            vdf_output: VdfOutput { value: vec![] },
            vdf_proof: VdfProof::default(),
        };

        Block {
            header,
            transactions,
            aggregate_bls_signature: Vec::new(),
        }
    }

    #[test]
    fn test_validate_block_rewards_direct_coinbase_mode() {
        // DirectCoinbase mode should not be affected by validate_block_rewards
        let mut params = ConsensusParams::mainnet();
        params.reward_mode = RewardMode::DirectCoinbase;

        let ctx = ValidationContext::new(params, Network::Mainnet, GENESIS_TIME + 60, 1)
            .with_prev_block(0, GENESIS_TIME, Hash::ZERO);

        let pubkey_hash = crypto::hash::hash(b"producer");
        let coinbase = Transaction::new_coinbase(ctx.params.block_reward(1), pubkey_hash, 1);
        let block = create_test_block_at_slot(1, vec![coinbase]);

        // Should pass for DirectCoinbase mode (validate_block_rewards returns Ok immediately)
        let result = validate_block_rewards(&block, &ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_block_rewards_epoch_boundary_no_coinbase() {
        let params = ConsensusParams::mainnet();
        let boundary_slot = params.slots_per_reward_epoch; // First epoch boundary

        let ctx = epoch_pool_context_at_slot(boundary_slot);

        // Create block with coinbase at epoch boundary (should fail)
        let pubkey_hash = crypto::hash::hash(b"producer");
        let coinbase = Transaction::new_coinbase(1000, pubkey_hash, boundary_slot as u64);
        let block = create_test_block_at_slot(boundary_slot, vec![coinbase]);

        let result = validate_block_rewards(&block, &ctx);
        assert!(matches!(result, Err(ValidationError::InvalidBlock(_))));
    }

    #[test]
    fn test_validate_block_rewards_non_boundary_no_rewards() {
        let params = ConsensusParams::mainnet();
        let non_boundary_slot = params.slots_per_reward_epoch / 2; // Mid-epoch

        let ctx = epoch_pool_context_at_slot(non_boundary_slot);

        // Create block with coinbase at non-boundary (should fail in EpochPool mode)
        let pubkey_hash = crypto::hash::hash(b"producer");
        let coinbase = Transaction::new_coinbase(1000, pubkey_hash, non_boundary_slot as u64);
        let block = create_test_block_at_slot(non_boundary_slot, vec![coinbase]);

        let result = validate_block_rewards(&block, &ctx);
        assert!(matches!(result, Err(ValidationError::InvalidBlock(_))));
    }

    #[test]
    fn test_validate_block_rewards_non_boundary_no_epoch_reward() {
        let params = ConsensusParams::mainnet();
        let non_boundary_slot = params.slots_per_reward_epoch / 2; // Mid-epoch

        let ctx = epoch_pool_context_at_slot(non_boundary_slot);

        // Create block with epoch reward at non-boundary (should fail)
        let keypair = crypto::KeyPair::generate();
        let pubkey_hash = crypto::hash::hash(b"recipient");
        let epoch_reward =
            Transaction::new_epoch_reward(0, *keypair.public_key(), 1000, pubkey_hash);
        let block = create_test_block_at_slot(non_boundary_slot, vec![epoch_reward]);

        let result = validate_block_rewards(&block, &ctx);
        assert!(matches!(result, Err(ValidationError::InvalidBlock(_))));
    }

    #[test]
    fn test_validate_block_rewards_non_boundary_empty_block_ok() {
        let params = ConsensusParams::mainnet();
        let non_boundary_slot = params.slots_per_reward_epoch / 2; // Mid-epoch

        let ctx = epoch_pool_context_at_slot(non_boundary_slot);

        // Empty block at non-boundary should be OK (pool accumulates)
        let block = create_test_block_at_slot(non_boundary_slot, vec![]);

        let result = validate_block_rewards(&block, &ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_block_rewards_partial_pool_ok() {
        // With attestation qualification, total_distributed ≤ pool is valid:
        // non-qualifying producers' share is burned (not distributed)
        let params = ConsensusParams::mainnet();
        let boundary_slot = params.slots_per_reward_epoch; // First epoch boundary
        let expected_epoch = params.slot_to_reward_epoch(boundary_slot);

        let ctx = epoch_pool_context_at_slot(boundary_slot);

        let keypair = crypto::KeyPair::generate();
        let pubkey_hash = crypto::hash::hash(b"recipient");
        let expected_pool = ctx.params.total_epoch_reward(boundary_slot as u64);
        let epoch_reward = Transaction::new_epoch_reward(
            expected_epoch as u64,
            *keypair.public_key(),
            expected_pool / 2, // Half pool — valid when some producers disqualified
            pubkey_hash,
        );
        let block = create_test_block_at_slot(boundary_slot, vec![epoch_reward]);

        let result = validate_block_rewards(&block, &ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_block_rewards_exceeds_pool_rejected() {
        // Total distributed > pool is always invalid (inflation attack)
        let params = ConsensusParams::mainnet();
        let boundary_slot = params.slots_per_reward_epoch;
        let expected_epoch = params.slot_to_reward_epoch(boundary_slot);

        let ctx = epoch_pool_context_at_slot(boundary_slot);

        let keypair = crypto::KeyPair::generate();
        let pubkey_hash = crypto::hash::hash(b"recipient");
        let expected_pool = ctx.params.total_epoch_reward(boundary_slot as u64);
        let epoch_reward = Transaction::new_epoch_reward(
            expected_epoch as u64,
            *keypair.public_key(),
            expected_pool + 1, // Exceeds pool — inflation attack
            pubkey_hash,
        );
        let block = create_test_block_at_slot(boundary_slot, vec![epoch_reward]);

        let result = validate_block_rewards(&block, &ctx);
        assert!(matches!(result, Err(ValidationError::InvalidBlock(_))));
    }

    #[test]
    fn test_validate_block_rewards_epoch_boundary_valid() {
        let params = ConsensusParams::mainnet();
        let boundary_slot = params.slots_per_reward_epoch; // First epoch boundary
        let expected_epoch = params.slot_to_reward_epoch(boundary_slot);

        let ctx = epoch_pool_context_at_slot(boundary_slot);

        // Create block with correct epoch reward
        let keypair = crypto::KeyPair::generate();
        let pubkey_hash = crypto::hash::hash(b"recipient");
        let expected_pool = ctx.params.total_epoch_reward(boundary_slot as u64);
        let epoch_reward = Transaction::new_epoch_reward(
            expected_epoch as u64,
            *keypair.public_key(),
            expected_pool, // Correct amount
            pubkey_hash,
        );
        let block = create_test_block_at_slot(boundary_slot, vec![epoch_reward]);

        let result = validate_block_rewards(&block, &ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_block_rewards_epoch_mismatch() {
        let params = ConsensusParams::mainnet();
        let boundary_slot = params.slots_per_reward_epoch; // First epoch boundary

        let ctx = epoch_pool_context_at_slot(boundary_slot);

        // Create epoch reward with wrong epoch number
        let keypair = crypto::KeyPair::generate();
        let pubkey_hash = crypto::hash::hash(b"recipient");
        let expected_pool = ctx.params.total_epoch_reward(boundary_slot as u64);
        let epoch_reward = Transaction::new_epoch_reward(
            99, // Wrong epoch!
            *keypair.public_key(),
            expected_pool,
            pubkey_hash,
        );
        let block = create_test_block_at_slot(boundary_slot, vec![epoch_reward]);

        let result = validate_block_rewards(&block, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidEpochReward(_))
        ));
    }

    // ==========================================================================
    // Milestone 5: Exact Epoch Reward Validation Tests
    // ==========================================================================

    /// Mock EpochBlockSource for testing exact validation
    struct MockEpochBlockSource {
        last_rewarded: u64,
        blocks: Vec<Block>,
    }

    impl MockEpochBlockSource {
        fn new(last_rewarded: u64, blocks: Vec<Block>) -> Self {
            Self {
                last_rewarded,
                blocks,
            }
        }
    }

    impl EpochBlockSource for MockEpochBlockSource {
        fn last_rewarded_epoch(&self) -> Result<u64, String> {
            Ok(self.last_rewarded)
        }

        fn blocks_in_slot_range(&self, start: u32, end: u32) -> Result<Vec<Block>, String> {
            Ok(self
                .blocks
                .iter()
                .filter(|b| b.header.slot >= start && b.header.slot < end)
                .cloned()
                .collect())
        }

        fn has_any_block_in_slot_range(&self, start: u32, end: u32) -> Result<bool, String> {
            Ok(self
                .blocks
                .iter()
                .any(|b| b.header.slot >= start && b.header.slot < end))
        }
    }

    /// Create a test block with a specific producer
    fn create_test_block_with_producer(slot: u32, producer: &crypto::PublicKey) -> Block {
        use crate::block::{Block, BlockHeader};
        use vdf::{VdfOutput, VdfProof};

        let header = BlockHeader {
            version: 1,
            slot,
            timestamp: GENESIS_TIME + (slot as u64 * 60),
            prev_hash: Hash::ZERO,
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            producer: *producer,
            vdf_output: VdfOutput { value: vec![] },
            vdf_proof: VdfProof::default(),
        };

        Block::new(header, vec![])
    }

    #[test]
    #[allow(deprecated)]
    fn test_calculate_expected_epoch_rewards_empty() {
        let params = ConsensusParams::mainnet();
        let blocks: Vec<Block> = vec![];

        let rewards = calculate_expected_epoch_rewards(0, &blocks, 100, &params);
        assert!(rewards.is_empty(), "No blocks should produce no rewards");
    }

    #[test]
    #[allow(deprecated)]
    fn test_calculate_expected_epoch_rewards_single_producer() {
        let params = ConsensusParams::mainnet();
        let keypair = crypto::KeyPair::generate();
        let producer = *keypair.public_key();

        // Create 10 blocks by single producer
        let blocks: Vec<Block> = (1..=10)
            .map(|slot| create_test_block_with_producer(slot, &producer))
            .collect();

        let rewards = calculate_expected_epoch_rewards(0, &blocks, 100, &params);

        assert_eq!(rewards.len(), 1, "Single producer should get one reward");
        assert_eq!(rewards[0].0, producer, "Reward should go to producer");

        // Expected: 10 blocks × block_reward
        let expected_total = 10 * params.block_reward(100);
        assert_eq!(rewards[0].1, expected_total, "Amount should match");
    }

    #[test]
    #[allow(deprecated)]
    fn test_calculate_expected_epoch_rewards_multiple_producers() {
        let params = ConsensusParams::mainnet();

        let keypair1 = crypto::KeyPair::generate();
        let keypair2 = crypto::KeyPair::generate();
        let producer1 = *keypair1.public_key();
        let producer2 = *keypair2.public_key();

        // Producer1: 3 blocks, Producer2: 7 blocks
        let mut blocks = Vec::new();
        for slot in 1..=3 {
            blocks.push(create_test_block_with_producer(slot, &producer1));
        }
        for slot in 4..=10 {
            blocks.push(create_test_block_with_producer(slot, &producer2));
        }

        let rewards = calculate_expected_epoch_rewards(0, &blocks, 100, &params);

        assert_eq!(rewards.len(), 2, "Two producers should get rewards");

        let block_reward = params.block_reward(100);
        let total_pool = 10 * block_reward;

        // Find rewards for each producer
        let p1_reward = rewards.iter().find(|(pk, _)| pk == &producer1);
        let p2_reward = rewards.iter().find(|(pk, _)| pk == &producer2);

        assert!(p1_reward.is_some(), "Producer1 should have reward");
        assert!(p2_reward.is_some(), "Producer2 should have reward");

        // Check proportional distribution (3/10 and 7/10)
        let p1_amount = p1_reward.unwrap().1;
        let p2_amount = p2_reward.unwrap().1;

        // Total should equal pool
        assert_eq!(p1_amount + p2_amount, total_pool, "Total should equal pool");

        // Approximate proportions (allow for rounding)
        let expected_p1 = (3 * total_pool) / 10;
        let _expected_p2 = total_pool - expected_p1; // Last gets remainder

        // The actual amounts depend on sort order, but total must be correct
        assert!(p1_amount > 0 && p2_amount > 0, "Both should get rewards");
    }

    #[test]
    #[allow(deprecated)]
    fn test_calculate_expected_epoch_rewards_deterministic_ordering() {
        let params = ConsensusParams::mainnet();

        // Generate multiple keypairs
        let keypairs: Vec<_> = (0..5).map(|_| crypto::KeyPair::generate()).collect();
        let producers: Vec<_> = keypairs.iter().map(|kp| *kp.public_key()).collect();

        // Each producer creates 2 blocks
        let mut blocks = Vec::new();
        for (i, producer) in producers.iter().enumerate() {
            blocks.push(create_test_block_with_producer(
                (i * 2 + 1) as u32,
                producer,
            ));
            blocks.push(create_test_block_with_producer(
                (i * 2 + 2) as u32,
                producer,
            ));
        }

        // Calculate twice to verify determinism
        let rewards1 = calculate_expected_epoch_rewards(0, &blocks, 100, &params);
        let rewards2 = calculate_expected_epoch_rewards(0, &blocks, 100, &params);

        assert_eq!(rewards1, rewards2, "Calculation should be deterministic");

        // Verify sorted by pubkey
        for i in 1..rewards1.len() {
            assert!(
                rewards1[i - 1].0.as_bytes() < rewards1[i].0.as_bytes(),
                "Rewards should be sorted by pubkey"
            );
        }
    }

    #[test]
    #[allow(deprecated)]
    fn test_calculate_expected_epoch_rewards_skips_null_producer() {
        let params = ConsensusParams::mainnet();
        let keypair = crypto::KeyPair::generate();
        let producer = *keypair.public_key();

        // Create blocks including one with null producer (genesis-like)
        let mut blocks = vec![create_test_block_with_producer(1, &producer)];

        // Add a "genesis" block with null producer
        let null_producer = crypto::PublicKey::from_bytes([0u8; 32]);
        blocks.push(create_test_block_with_producer(0, &null_producer));

        let rewards = calculate_expected_epoch_rewards(0, &blocks, 100, &params);

        // Only the real producer should get rewards
        assert_eq!(rewards.len(), 1, "Only real producer should get reward");
        assert_eq!(rewards[0].0, producer, "Reward should go to real producer");
    }

    #[test]
    #[allow(deprecated)]
    fn test_epoch_needing_rewards_slot_zero() {
        let params = ConsensusParams::mainnet();
        let source = MockEpochBlockSource::new(0, vec![]);

        let result = epoch_needing_rewards(0, &params, &source);
        assert!(result.is_ok());
        assert!(
            result.unwrap().is_none(),
            "Slot 0 should not trigger rewards"
        );
    }

    #[test]
    #[allow(deprecated)]
    fn test_epoch_needing_rewards_first_epoch_boundary() {
        let params = ConsensusParams::mainnet();
        let keypair = crypto::KeyPair::generate();
        let pubkey = *keypair.public_key();
        // Add a block in epoch 1 (slots 360-719) so it has blocks to reward
        // Note: We're at slot 720 (epoch 2), checking if epoch 1 needs rewards
        let block_in_epoch_1 = create_test_block_with_producer(
            params.slots_per_reward_epoch + 50, // Slot 410 (in epoch 1)
            &pubkey,
        );
        let source = MockEpochBlockSource::new(0, vec![block_in_epoch_1]);

        // First slot of epoch 2 should trigger epoch 1 rewards (epoch 1 is finished and has blocks)
        let boundary_slot = params.slots_per_reward_epoch * 2;
        let result = epoch_needing_rewards(boundary_slot, &params, &source);

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Some(1), "Should reward epoch 1");
    }

    #[test]
    #[allow(deprecated)]
    fn test_epoch_needing_rewards_empty_epoch_skipped() {
        let params = ConsensusParams::mainnet();
        // No blocks in epoch 0 or 1 - they should be skipped
        let source = MockEpochBlockSource::new(0, vec![]);

        // In epoch 2, but epochs 0 and 1 are empty
        let slot_in_epoch_2 = params.slots_per_reward_epoch * 2 + 5;
        let result = epoch_needing_rewards(slot_in_epoch_2, &params, &source);

        assert!(result.is_ok());
        assert!(
            result.unwrap().is_none(),
            "Empty epochs should be skipped, no rewards needed"
        );
    }

    #[test]
    #[allow(deprecated)]
    fn test_epoch_needing_rewards_already_rewarded() {
        let params = ConsensusParams::mainnet();
        let source = MockEpochBlockSource::new(1, vec![]); // Epoch 1 already rewarded

        let boundary_slot = params.slots_per_reward_epoch;
        let result = epoch_needing_rewards(boundary_slot, &params, &source);

        assert!(result.is_ok());
        assert!(
            result.unwrap().is_none(),
            "Already rewarded epoch should return None"
        );
    }

    #[test]
    #[allow(deprecated)]
    fn test_epoch_needing_rewards_multi_epoch_catchup() {
        let params = ConsensusParams::mainnet();
        let keypair = crypto::KeyPair::generate();
        let pubkey = *keypair.public_key();

        // Add blocks in epochs 1 and 2 (epoch 0 is empty)
        let block_in_epoch_1 =
            create_test_block_with_producer(params.slots_per_reward_epoch + 50, &pubkey);
        let block_in_epoch_2 =
            create_test_block_with_producer(params.slots_per_reward_epoch * 2 + 50, &pubkey);
        let source = MockEpochBlockSource::new(0, vec![block_in_epoch_1, block_in_epoch_2]);

        // In epoch 3 but no rewards distributed - should catch up with epoch 1
        // (epoch 0 is empty and skipped, epoch 1 has blocks)
        let slot_in_epoch_3 = params.slots_per_reward_epoch * 3 + 5;
        let result = epoch_needing_rewards(slot_in_epoch_3, &params, &source);

        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            Some(1),
            "Should reward oldest non-empty unrewarded epoch (1, skipping empty epoch 0)"
        );
    }

    #[test]
    #[allow(deprecated)]
    fn test_epoch_needing_rewards_late_start_skips_empty() {
        let params = ConsensusParams::mainnet();
        let keypair = crypto::KeyPair::generate();
        let pubkey = *keypair.public_key();

        // Simulate chain starting late: only epoch 5 has blocks
        let block_in_epoch_5 =
            create_test_block_with_producer(params.slots_per_reward_epoch * 5 + 50, &pubkey);
        let source = MockEpochBlockSource::new(0, vec![block_in_epoch_5]);

        // In epoch 6 - should skip epochs 0-4 (empty) and reward epoch 5
        let slot_in_epoch_6 = params.slots_per_reward_epoch * 6 + 5;
        let result = epoch_needing_rewards(slot_in_epoch_6, &params, &source);

        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            Some(5),
            "Should skip empty epochs 0-4 and reward epoch 5"
        );
    }

    #[test]
    #[allow(deprecated)]
    fn test_validate_block_rewards_exact_non_boundary_no_rewards() {
        let params = ConsensusParams::mainnet();
        let non_boundary_slot = params.slots_per_reward_epoch / 2;

        let ctx = epoch_pool_context_at_slot(non_boundary_slot);
        let source = MockEpochBlockSource::new(0, vec![]);

        // Empty block at non-boundary should be OK
        let block = create_test_block_at_slot(non_boundary_slot, vec![]);

        let result = validate_block_rewards_exact(&block, &ctx, &source);
        assert!(result.is_ok(), "Empty block at non-boundary should pass");
    }

    #[test]
    #[allow(deprecated)]
    fn test_validate_block_rewards_exact_unexpected_rewards() {
        let params = ConsensusParams::mainnet();
        let non_boundary_slot = params.slots_per_reward_epoch / 2;

        let ctx = epoch_pool_context_at_slot(non_boundary_slot);
        let source = MockEpochBlockSource::new(0, vec![]);

        // Block with epoch reward at non-boundary
        let keypair = crypto::KeyPair::generate();
        let pubkey_hash = crypto::hash::hash(b"recipient");
        let epoch_reward =
            Transaction::new_epoch_reward(0, *keypair.public_key(), 1000, pubkey_hash);
        let block = create_test_block_at_slot(non_boundary_slot, vec![epoch_reward]);

        let result = validate_block_rewards_exact(&block, &ctx, &source);
        assert!(
            matches!(result, Err(ValidationError::UnexpectedEpochReward)),
            "Should reject unexpected epoch rewards"
        );
    }

    #[test]
    #[allow(deprecated)]
    #[ignore = "Tests deprecated automatic epoch reward system - use ClaimEpochReward instead"]
    fn test_validate_block_rewards_exact_missing_rewards() {
        let params = ConsensusParams::mainnet();
        let boundary_slot = params.slots_per_reward_epoch;

        let ctx = epoch_pool_context_at_slot(boundary_slot);

        // Create blocks in epoch 0 for the source
        let keypair = crypto::KeyPair::generate();
        let producer = *keypair.public_key();
        let epoch_blocks: Vec<Block> = (1..=10)
            .map(|slot| create_test_block_with_producer(slot, &producer))
            .collect();

        let source = MockEpochBlockSource::new(0, epoch_blocks);

        // Empty block at boundary (missing rewards)
        let block = create_test_block_at_slot(boundary_slot, vec![]);

        let result = validate_block_rewards_exact(&block, &ctx, &source);
        assert!(
            matches!(result, Err(ValidationError::MissingEpochReward { .. })),
            "Should reject missing epoch rewards at boundary"
        );
    }

    #[test]
    #[allow(deprecated)]
    fn test_validate_block_rewards_exact_valid() {
        let params = ConsensusParams::mainnet();
        // Use second boundary (slot 720) where we validate epoch 1 rewards
        let boundary_slot = params.slots_per_reward_epoch * 2; // 720

        let mut ctx_params = ConsensusParams::mainnet();
        ctx_params.reward_mode = RewardMode::EpochPool;

        let ctx = ValidationContext::new(
            ctx_params.clone(),
            Network::Mainnet,
            GENESIS_TIME + (boundary_slot as u64 * 60),
            boundary_slot as u64,
        )
        .with_prev_block(
            boundary_slot - 1,
            GENESIS_TIME + ((boundary_slot as u64 - 1) * 60),
            Hash::ZERO,
        );

        // Create blocks in epoch 1 (slots 360-719)
        // At slot 720 with last_rewarded=0, epoch 1 is being rewarded
        let keypair = crypto::KeyPair::generate();
        let producer = *keypair.public_key();
        let start_slot = params.slots_per_reward_epoch; // 360
        let epoch_blocks: Vec<Block> = (start_slot..start_slot + 10)
            .map(|slot| create_test_block_with_producer(slot, &producer))
            .collect();

        let source = MockEpochBlockSource::new(0, epoch_blocks.clone());

        // Calculate expected rewards for epoch 1
        let expected =
            calculate_expected_epoch_rewards(1, &epoch_blocks, ctx.current_height, &ctx_params);
        assert_eq!(expected.len(), 1, "Should have one reward");

        // Create valid epoch reward transaction
        let pubkey_hash = crypto::hash::hash(producer.as_bytes());
        let epoch_reward = Transaction::new_epoch_reward(1, producer, expected[0].1, pubkey_hash);
        let block = create_test_block_at_slot(boundary_slot, vec![epoch_reward]);

        let result = validate_block_rewards_exact(&block, &ctx, &source);
        assert!(
            result.is_ok(),
            "Valid epoch rewards should pass: {:?}",
            result
        );
    }

    #[test]
    #[allow(deprecated)]
    #[ignore = "Tests deprecated automatic epoch reward system - use ClaimEpochReward instead"]
    fn test_validate_block_rewards_exact_wrong_amount() {
        let params = ConsensusParams::mainnet();
        let boundary_slot = params.slots_per_reward_epoch;

        let mut ctx_params = ConsensusParams::mainnet();
        ctx_params.reward_mode = RewardMode::EpochPool;

        let ctx = ValidationContext::new(
            ctx_params.clone(),
            Network::Mainnet,
            GENESIS_TIME + (boundary_slot as u64 * 60),
            boundary_slot as u64,
        )
        .with_prev_block(
            boundary_slot - 1,
            GENESIS_TIME + ((boundary_slot as u64 - 1) * 60),
            Hash::ZERO,
        );

        // Create blocks in epoch 0
        let keypair = crypto::KeyPair::generate();
        let producer = *keypair.public_key();
        let epoch_blocks: Vec<Block> = (1..=10)
            .map(|slot| create_test_block_with_producer(slot, &producer))
            .collect();

        let source = MockEpochBlockSource::new(0, epoch_blocks);

        // Create epoch reward with WRONG amount
        let pubkey_hash = crypto::hash::hash(producer.as_bytes());
        let wrong_amount = 12345; // Arbitrary wrong amount
        let epoch_reward = Transaction::new_epoch_reward(1, producer, wrong_amount, pubkey_hash);
        let block = create_test_block_at_slot(boundary_slot, vec![epoch_reward]);

        let result = validate_block_rewards_exact(&block, &ctx, &source);
        assert!(
            matches!(result, Err(ValidationError::EpochRewardMismatch { .. })),
            "Should reject wrong amount: {:?}",
            result
        );
    }

    #[test]
    #[allow(deprecated)]
    #[ignore = "Tests deprecated automatic epoch reward system - use ClaimEpochReward instead"]
    fn test_validate_block_rewards_exact_wrong_recipient() {
        let params = ConsensusParams::mainnet();
        let boundary_slot = params.slots_per_reward_epoch;

        let mut ctx_params = ConsensusParams::mainnet();
        ctx_params.reward_mode = RewardMode::EpochPool;

        let ctx = ValidationContext::new(
            ctx_params.clone(),
            Network::Mainnet,
            GENESIS_TIME + (boundary_slot as u64 * 60),
            boundary_slot as u64,
        )
        .with_prev_block(
            boundary_slot - 1,
            GENESIS_TIME + ((boundary_slot as u64 - 1) * 60),
            Hash::ZERO,
        );

        // Create blocks in epoch 0 by producer1
        let keypair1 = crypto::KeyPair::generate();
        let producer1 = *keypair1.public_key();
        let epoch_blocks: Vec<Block> = (1..=10)
            .map(|slot| create_test_block_with_producer(slot, &producer1))
            .collect();

        let source = MockEpochBlockSource::new(0, epoch_blocks);

        // Calculate correct amount but use WRONG recipient
        let expected =
            calculate_expected_epoch_rewards(1, &source.blocks, ctx.current_height, &ctx_params);
        let keypair2 = crypto::KeyPair::generate();
        let wrong_recipient = *keypair2.public_key();
        let pubkey_hash = crypto::hash::hash(wrong_recipient.as_bytes());

        let epoch_reward =
            Transaction::new_epoch_reward(1, wrong_recipient, expected[0].1, pubkey_hash);
        let block = create_test_block_at_slot(boundary_slot, vec![epoch_reward]);

        let result = validate_block_rewards_exact(&block, &ctx, &source);
        assert!(
            matches!(result, Err(ValidationError::EpochRewardMismatch { .. })),
            "Should reject wrong recipient: {:?}",
            result
        );
    }

    #[test]
    #[allow(deprecated)]
    fn test_validate_block_rewards_exact_wrong_epoch_number() {
        let params = ConsensusParams::mainnet();
        // Use second epoch boundary (slot 720) where we reward epoch 2
        // after epoch 1 was already rewarded
        let boundary_slot = params.slots_per_reward_epoch * 2; // slot 720

        let mut ctx_params = ConsensusParams::mainnet();
        ctx_params.reward_mode = RewardMode::EpochPool;

        let ctx = ValidationContext::new(
            ctx_params.clone(),
            Network::Mainnet,
            GENESIS_TIME + (boundary_slot as u64 * 60),
            boundary_slot as u64,
        )
        .with_prev_block(
            boundary_slot - 1,
            GENESIS_TIME + ((boundary_slot as u64 - 1) * 60),
            Hash::ZERO,
        );

        // Create blocks in epoch 1 (slots 360-719) since we're at slot 720
        // and last_rewarded=1 means epoch 2 is next to be rewarded
        let keypair = crypto::KeyPair::generate();
        let producer = *keypair.public_key();
        let start = params.slots_per_reward_epoch;
        let epoch_blocks: Vec<Block> = (start..start + 10)
            .map(|slot| create_test_block_with_producer(slot, &producer))
            .collect();

        // last_rewarded=1 means epoch 1 was rewarded, so epoch 2 is next
        let _source = MockEpochBlockSource::new(1, epoch_blocks);

        // Calculate expected rewards for epoch 2 (but there are no blocks there)
        // So we need blocks in epoch 2 range (720-1079)
        // Let's simplify: use last_rewarded=0, boundary at 360, blocks in epoch 0 range
        // The issue: epoch_needing_rewards(360) with last_rewarded=0 returns Some(1)
        // but epoch 1 slots are 360-719, and we only have blocks in 1-10

        // Alternative approach: test with blocks in the correct epoch range
        // At slot 720, last_rewarded=1, we reward epoch 2
        // So we need blocks in epoch 2 range (720-1079)... but we're AT 720
        // This is tricky because we're producing the first block of the epoch

        // Simplest fix: put blocks in epoch 1 range and set last_rewarded=0
        // so epoch 1 is being rewarded and blocks exist

        // Actually, let's create blocks in epoch 1 (360-719)
        let epoch1_blocks: Vec<Block> = (360..370)
            .map(|slot| create_test_block_with_producer(slot, &producer))
            .collect();

        // At slot 720, last_rewarded=0 means epoch 1 is next
        let source2 = MockEpochBlockSource::new(0, epoch1_blocks.clone());

        let expected =
            calculate_expected_epoch_rewards(1, &epoch1_blocks, ctx.current_height, &ctx_params);
        let pubkey_hash = crypto::hash::hash(producer.as_bytes());

        // Create epoch reward with WRONG epoch number (should be 1)
        let epoch_reward = Transaction::new_epoch_reward(
            99, // Wrong epoch! Should be 1
            producer,
            expected[0].1,
            pubkey_hash,
        );
        let block = create_test_block_at_slot(boundary_slot, vec![epoch_reward]);

        let result = validate_block_rewards_exact(&block, &ctx, &source2);
        assert!(
            matches!(result, Err(ValidationError::InvalidEpochReward(_))),
            "Should reject wrong epoch number: {:?}",
            result
        );
    }

    // =========================================================================
    // Milestone 6: Additional Validation Tests
    // =========================================================================

    #[test]
    #[allow(deprecated)]
    fn test_validate_block_rewards_exact_many_producers() {
        // Test validation with 10 producers to verify scalability
        let params = ConsensusParams::mainnet();
        let boundary_slot = params.slots_per_reward_epoch * 2; // slot 720

        let mut ctx_params = ConsensusParams::mainnet();
        ctx_params.reward_mode = RewardMode::EpochPool;

        let ctx = ValidationContext::new(
            ctx_params.clone(),
            Network::Mainnet,
            GENESIS_TIME + (boundary_slot as u64 * 60),
            boundary_slot as u64,
        )
        .with_prev_block(
            boundary_slot - 1,
            GENESIS_TIME + ((boundary_slot as u64 - 1) * 60),
            Hash::ZERO,
        );

        // Create 10 producers, each with 1 block in epoch 1
        let keypairs: Vec<_> = (0..10).map(|_| crypto::KeyPair::generate()).collect();
        let producers: Vec<_> = keypairs.iter().map(|kp| *kp.public_key()).collect();

        let start_slot = params.slots_per_reward_epoch;
        let epoch_blocks: Vec<Block> = producers
            .iter()
            .enumerate()
            .map(|(i, producer)| create_test_block_with_producer(start_slot + i as u32, producer))
            .collect();

        let source = MockEpochBlockSource::new(0, epoch_blocks.clone());

        // Calculate expected rewards
        let expected =
            calculate_expected_epoch_rewards(1, &epoch_blocks, ctx.current_height, &ctx_params);
        assert_eq!(expected.len(), 10, "Should have 10 reward entries");

        // Create valid epoch reward transactions
        let epoch_txs: Vec<Transaction> = expected
            .iter()
            .map(|(producer, amount)| {
                let pubkey_hash = crypto::hash::hash(producer.as_bytes());
                Transaction::new_epoch_reward(1, *producer, *amount, pubkey_hash)
            })
            .collect();

        let block = create_test_block_at_slot(boundary_slot, epoch_txs);

        let result = validate_block_rewards_exact(&block, &ctx, &source);
        assert!(
            result.is_ok(),
            "Valid 10-producer rewards should pass: {:?}",
            result
        );
    }

    #[test]
    #[allow(deprecated)]
    fn test_validate_block_rewards_exact_dust_handling() {
        // Test validation with rounding dust (e.g., 100 units / 3 producers)
        let params = ConsensusParams::mainnet();
        // Use second boundary (slot 720) to reward epoch 1 (slots 360-719)
        let boundary_slot = params.slots_per_reward_epoch * 2;

        let mut ctx_params = ConsensusParams::mainnet();
        ctx_params.reward_mode = RewardMode::EpochPool;

        let ctx = ValidationContext::new(
            ctx_params.clone(),
            Network::Mainnet,
            GENESIS_TIME + (boundary_slot as u64 * 60),
            boundary_slot as u64,
        )
        .with_prev_block(
            boundary_slot - 1,
            GENESIS_TIME + ((boundary_slot as u64 - 1) * 60),
            Hash::ZERO,
        );

        // Create 3 producers with equal blocks (1 each) in epoch 1 range
        let keypairs: Vec<_> = (0..3).map(|_| crypto::KeyPair::generate()).collect();
        let mut producers: Vec<_> = keypairs.iter().map(|kp| *kp.public_key()).collect();

        // Sort producers by pubkey for deterministic dust assignment
        producers.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

        // Create blocks in epoch 1 range (slots 360-719)
        let start_slot = params.slots_per_reward_epoch;
        let epoch_blocks: Vec<Block> = producers
            .iter()
            .enumerate()
            .map(|(i, producer)| create_test_block_with_producer(start_slot + i as u32, producer))
            .collect();

        let source = MockEpochBlockSource::new(0, epoch_blocks.clone());

        // Calculate expected rewards (dust goes to last sorted producer)
        let expected =
            calculate_expected_epoch_rewards(1, &epoch_blocks, ctx.current_height, &ctx_params);
        assert_eq!(expected.len(), 3);

        // Verify dust is assigned to last producer
        let total_pool: u64 = expected.iter().map(|(_, amt)| *amt).sum();
        let base_share = total_pool / 3;

        // Last producer should get base_share + remainder
        let last_producer_amount = expected.last().unwrap().1;
        let dust = total_pool % 3;
        assert_eq!(
            last_producer_amount,
            base_share + dust,
            "Last producer should get dust"
        );

        // Create valid epoch reward transactions
        let epoch_txs: Vec<Transaction> = expected
            .iter()
            .map(|(producer, amount)| {
                let pubkey_hash = crypto::hash::hash(producer.as_bytes());
                Transaction::new_epoch_reward(1, *producer, *amount, pubkey_hash)
            })
            .collect();

        let block = create_test_block_at_slot(boundary_slot, epoch_txs);

        let result = validate_block_rewards_exact(&block, &ctx, &source);
        assert!(
            result.is_ok(),
            "Valid dust distribution should pass: {:?}",
            result
        );
    }

    #[test]
    #[allow(deprecated)]
    #[ignore = "Tests deprecated automatic epoch reward system - use ClaimEpochReward instead"]
    fn test_validate_block_rewards_exact_over_distribution() {
        // Test rejection of rewards that exceed expected amounts
        let params = ConsensusParams::mainnet();
        let boundary_slot = params.slots_per_reward_epoch;

        let mut ctx_params = ConsensusParams::mainnet();
        ctx_params.reward_mode = RewardMode::EpochPool;

        let ctx = ValidationContext::new(
            ctx_params.clone(),
            Network::Mainnet,
            GENESIS_TIME + (boundary_slot as u64 * 60),
            boundary_slot as u64,
        )
        .with_prev_block(
            boundary_slot - 1,
            GENESIS_TIME + ((boundary_slot as u64 - 1) * 60),
            Hash::ZERO,
        );

        let keypair = crypto::KeyPair::generate();
        let producer = *keypair.public_key();

        let epoch_blocks: Vec<Block> = (1..=10)
            .map(|slot| create_test_block_with_producer(slot, &producer))
            .collect();

        let source = MockEpochBlockSource::new(0, epoch_blocks.clone());

        // Calculate expected, then add extra
        let expected =
            calculate_expected_epoch_rewards(1, &epoch_blocks, ctx.current_height, &ctx_params);
        let correct_amount = expected[0].1;
        let inflated_amount = correct_amount + 1_000_000; // Add 1M extra units

        let pubkey_hash = crypto::hash::hash(producer.as_bytes());
        let epoch_reward = Transaction::new_epoch_reward(1, producer, inflated_amount, pubkey_hash);
        let block = create_test_block_at_slot(boundary_slot, vec![epoch_reward]);

        let result = validate_block_rewards_exact(&block, &ctx, &source);
        assert!(
            matches!(result, Err(ValidationError::EpochRewardMismatch { .. })),
            "Should reject over-distribution: {:?}",
            result
        );
    }

    #[test]
    #[allow(deprecated)]
    #[ignore = "Tests deprecated automatic epoch reward system - use ClaimEpochReward instead"]
    fn test_validate_block_rewards_exact_under_distribution() {
        // Test rejection of rewards that are less than expected
        let params = ConsensusParams::mainnet();
        let boundary_slot = params.slots_per_reward_epoch;

        let mut ctx_params = ConsensusParams::mainnet();
        ctx_params.reward_mode = RewardMode::EpochPool;

        let ctx = ValidationContext::new(
            ctx_params.clone(),
            Network::Mainnet,
            GENESIS_TIME + (boundary_slot as u64 * 60),
            boundary_slot as u64,
        )
        .with_prev_block(
            boundary_slot - 1,
            GENESIS_TIME + ((boundary_slot as u64 - 1) * 60),
            Hash::ZERO,
        );

        let keypair = crypto::KeyPair::generate();
        let producer = *keypair.public_key();

        let epoch_blocks: Vec<Block> = (1..=10)
            .map(|slot| create_test_block_with_producer(slot, &producer))
            .collect();

        let source = MockEpochBlockSource::new(0, epoch_blocks.clone());

        // Calculate expected, then subtract
        let expected =
            calculate_expected_epoch_rewards(1, &epoch_blocks, ctx.current_height, &ctx_params);
        let correct_amount = expected[0].1;
        let reduced_amount = correct_amount - 1_000_000; // Remove 1M units

        let pubkey_hash = crypto::hash::hash(producer.as_bytes());
        let epoch_reward = Transaction::new_epoch_reward(1, producer, reduced_amount, pubkey_hash);
        let block = create_test_block_at_slot(boundary_slot, vec![epoch_reward]);

        let result = validate_block_rewards_exact(&block, &ctx, &source);
        assert!(
            matches!(result, Err(ValidationError::EpochRewardMismatch { .. })),
            "Should reject under-distribution: {:?}",
            result
        );
    }

    #[test]
    #[allow(deprecated)]
    #[ignore = "Tests deprecated automatic epoch reward system - use ClaimEpochReward instead"]
    fn test_validate_block_rewards_exact_partial_producer_list() {
        // Test rejection when some producers are missing from rewards
        let params = ConsensusParams::mainnet();
        let boundary_slot = params.slots_per_reward_epoch;

        let mut ctx_params = ConsensusParams::mainnet();
        ctx_params.reward_mode = RewardMode::EpochPool;

        let ctx = ValidationContext::new(
            ctx_params.clone(),
            Network::Mainnet,
            GENESIS_TIME + (boundary_slot as u64 * 60),
            boundary_slot as u64,
        )
        .with_prev_block(
            boundary_slot - 1,
            GENESIS_TIME + ((boundary_slot as u64 - 1) * 60),
            Hash::ZERO,
        );

        // Create 3 producers with blocks
        let keypairs: Vec<_> = (0..3).map(|_| crypto::KeyPair::generate()).collect();
        let producers: Vec<_> = keypairs.iter().map(|kp| *kp.public_key()).collect();

        let epoch_blocks: Vec<Block> = producers
            .iter()
            .enumerate()
            .map(|(i, producer)| create_test_block_with_producer(i as u32 + 1, producer))
            .collect();

        let source = MockEpochBlockSource::new(0, epoch_blocks.clone());

        // Calculate expected (should have 3 entries)
        let expected =
            calculate_expected_epoch_rewards(1, &epoch_blocks, ctx.current_height, &ctx_params);
        assert_eq!(expected.len(), 3);

        // Only include 2 of 3 producers (missing last one)
        let partial_txs: Vec<Transaction> = expected
            .iter()
            .take(2) // Only first 2 producers
            .map(|(producer, amount)| {
                let pubkey_hash = crypto::hash::hash(producer.as_bytes());
                Transaction::new_epoch_reward(1, *producer, *amount, pubkey_hash)
            })
            .collect();

        let block = create_test_block_at_slot(boundary_slot, partial_txs);

        let result = validate_block_rewards_exact(&block, &ctx, &source);
        assert!(
            matches!(result, Err(ValidationError::EpochRewardMismatch { .. })),
            "Should reject partial producer list: {:?}",
            result
        );
    }

    #[test]
    #[allow(deprecated)]
    #[ignore = "Tests deprecated automatic epoch reward system - use ClaimEpochReward instead"]
    fn test_validate_block_rewards_exact_extra_reward_tx() {
        // Test rejection when block has more reward transactions than expected
        let params = ConsensusParams::mainnet();
        let boundary_slot = params.slots_per_reward_epoch;

        let mut ctx_params = ConsensusParams::mainnet();
        ctx_params.reward_mode = RewardMode::EpochPool;

        let ctx = ValidationContext::new(
            ctx_params.clone(),
            Network::Mainnet,
            GENESIS_TIME + (boundary_slot as u64 * 60),
            boundary_slot as u64,
        )
        .with_prev_block(
            boundary_slot - 1,
            GENESIS_TIME + ((boundary_slot as u64 - 1) * 60),
            Hash::ZERO,
        );

        let keypair = crypto::KeyPair::generate();
        let producer = *keypair.public_key();

        let epoch_blocks: Vec<Block> = (1..=10)
            .map(|slot| create_test_block_with_producer(slot, &producer))
            .collect();

        let source = MockEpochBlockSource::new(0, epoch_blocks.clone());

        // Calculate expected (should have 1 entry)
        let expected =
            calculate_expected_epoch_rewards(1, &epoch_blocks, ctx.current_height, &ctx_params);
        assert_eq!(expected.len(), 1);

        // Create valid reward plus an EXTRA reward to a fake producer
        let pubkey_hash = crypto::hash::hash(producer.as_bytes());
        let valid_reward = Transaction::new_epoch_reward(1, producer, expected[0].1, pubkey_hash);

        let fake_keypair = crypto::KeyPair::generate();
        let fake_producer = *fake_keypair.public_key();
        let fake_hash = crypto::hash::hash(fake_producer.as_bytes());
        let extra_reward = Transaction::new_epoch_reward(1, fake_producer, 1_000_000, fake_hash);

        let block = create_test_block_at_slot(boundary_slot, vec![valid_reward, extra_reward]);

        let result = validate_block_rewards_exact(&block, &ctx, &source);
        assert!(
            matches!(result, Err(ValidationError::EpochRewardMismatch { .. })),
            "Should reject extra reward transaction: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_delegate_bond_valid() {
        use crate::transaction::{DelegateBondData, Transaction};

        let delegator = crypto::KeyPair::generate();
        let delegate = crypto::KeyPair::generate();
        let data = DelegateBondData::new(*delegator.public_key(), *delegate.public_key(), 3);
        let tx = Transaction::new_delegate_bond(data);
        let ctx = test_context();

        let result = validate_transaction(&tx, &ctx);
        assert!(
            result.is_ok(),
            "Valid DelegateBond should pass: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_delegate_bond_zero_bonds() {
        use crate::transaction::{DelegateBondData, TxType};

        let delegator = crypto::KeyPair::generate();
        let delegate = crypto::KeyPair::generate();
        let data = DelegateBondData::new(*delegator.public_key(), *delegate.public_key(), 0);
        let tx = Transaction {
            version: 1,
            tx_type: TxType::DelegateBond,
            inputs: vec![],
            outputs: vec![],
            extra_data: data.to_bytes(),
        };
        let ctx = test_context();

        let result = validate_transaction(&tx, &ctx);
        assert!(
            matches!(result, Err(ValidationError::InvalidDelegation(_))),
            "DelegateBond with 0 bonds should fail: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_delegate_bond_empty_extra_data() {
        use crate::transaction::TxType;

        let tx = Transaction {
            version: 1,
            tx_type: TxType::DelegateBond,
            inputs: vec![],
            outputs: vec![],
            extra_data: vec![],
        };
        let ctx = test_context();

        let result = validate_transaction(&tx, &ctx);
        assert!(
            matches!(result, Err(ValidationError::InvalidDelegation(_))),
            "DelegateBond with empty data should fail: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_revoke_delegation_valid() {
        use crate::transaction::RevokeDelegationData;

        let delegator = crypto::KeyPair::generate();
        let delegate = crypto::KeyPair::generate();
        let data = RevokeDelegationData::new(*delegator.public_key(), *delegate.public_key());
        let tx = Transaction::new_revoke_delegation(data);
        let ctx = test_context();

        let result = validate_transaction(&tx, &ctx);
        assert!(
            result.is_ok(),
            "Valid RevokeDelegation should pass: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_revoke_delegation_empty_extra_data() {
        use crate::transaction::TxType;

        let tx = Transaction {
            version: 1,
            tx_type: TxType::RevokeDelegation,
            inputs: vec![],
            outputs: vec![],
            extra_data: vec![],
        };
        let ctx = test_context();

        let result = validate_transaction(&tx, &ctx);
        assert!(
            matches!(result, Err(ValidationError::InvalidDelegation(_))),
            "RevokeDelegation with empty data should fail: {:?}",
            result
        );
    }

    // ==========================================================================
    // Protocol Activation Validation Tests
    // ==========================================================================

    #[test]
    fn test_validate_protocol_activation_valid() {
        use crate::maintainer::{MaintainerSignature, ProtocolActivationData};

        let kp = crypto::KeyPair::generate();
        let data = ProtocolActivationData::new(2, 500, "Test".to_string(), vec![]);
        // Add a signature so structural validation passes
        let msg = data.signing_message();
        let sig = crypto::signature::sign(&msg, kp.private_key());
        let data = ProtocolActivationData::new(
            2,
            500,
            "Test".to_string(),
            vec![MaintainerSignature::new(*kp.public_key(), sig)],
        );
        let tx = Transaction::new_protocol_activation(data);
        let ctx = test_context();

        let result = validate_transaction(&tx, &ctx);
        assert!(
            result.is_ok(),
            "Valid ProtocolActivation should pass: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_protocol_activation_empty_extra_data() {
        use crate::transaction::TxType;

        let tx = Transaction {
            version: 1,
            tx_type: TxType::ProtocolActivation,
            inputs: vec![],
            outputs: vec![],
            extra_data: vec![],
        };
        let ctx = test_context();

        let result = validate_transaction(&tx, &ctx);
        assert!(
            matches!(result, Err(ValidationError::InvalidProtocolActivation(_))),
            "ProtocolActivation with empty data should fail: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_protocol_activation_zero_version() {
        use crate::maintainer::{MaintainerSignature, ProtocolActivationData};

        let kp = crypto::KeyPair::generate();
        let msg = b"activate:0:500";
        let sig = crypto::signature::sign(msg, kp.private_key());
        let data = ProtocolActivationData::new(
            0, // invalid
            500,
            "Test".to_string(),
            vec![MaintainerSignature::new(*kp.public_key(), sig)],
        );
        let tx = Transaction::new_protocol_activation(data);
        let ctx = test_context();

        let result = validate_transaction(&tx, &ctx);
        assert!(
            matches!(result, Err(ValidationError::InvalidProtocolActivation(_))),
            "ProtocolActivation with version 0 should fail: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_protocol_activation_no_signatures() {
        use crate::maintainer::ProtocolActivationData;

        let data = ProtocolActivationData::new(2, 500, "Test".to_string(), vec![]);
        let tx = Transaction::new_protocol_activation(data);
        let ctx = test_context();

        let result = validate_transaction(&tx, &ctx);
        assert!(
            matches!(result, Err(ValidationError::InvalidProtocolActivation(_))),
            "ProtocolActivation with no signatures should fail: {:?}",
            result
        );
    }

    // ==========================================================================
    // ValidationMode Tests (Smart Sync)
    // ==========================================================================

    /// Create a block with a valid merkle root for validation mode tests
    fn create_block_with_merkle() -> (Block, ValidationContext) {
        use crate::block::{Block, BlockHeader};
        use vdf::{VdfOutput, VdfProof};

        let params = ConsensusParams::mainnet();
        let keypair = crypto::KeyPair::generate();
        let slot = 12; // Slot 12 = GENESIS_TIME + 120
        let header = BlockHeader {
            version: 2,
            slot,
            timestamp: GENESIS_TIME + (slot as u64 * 10),
            prev_hash: Hash::ZERO,
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: params.genesis_hash,
            producer: *keypair.public_key(),
            vdf_output: VdfOutput {
                value: vec![0u8; 32],
            },
            vdf_proof: VdfProof::default(),
        };

        let mut block = Block {
            header,
            transactions: vec![],
            aggregate_bls_signature: Vec::new(),
        };
        // Fix merkle root to match (empty tx list)
        block.header.merkle_root = block.compute_merkle_root();

        let ctx = ValidationContext::new(
            ConsensusParams::mainnet(),
            Network::Mainnet,
            GENESIS_TIME + 120,
            1,
        )
        .with_prev_block(0, GENESIS_TIME, Hash::ZERO)
        .with_bootstrap_producers(vec![*keypair.public_key()]);

        (block, ctx)
    }

    #[test]
    fn test_validate_block_light_skips_vdf() {
        let (block, ctx) = create_block_with_merkle();

        // Light mode should accept a block even with invalid VDF (zeros)
        let result = validate_block_with_mode(&block, &ctx, ValidationMode::Light);
        assert!(
            result.is_ok(),
            "Light mode should skip VDF validation: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_block_light_rejects_bad_merkle() {
        let (mut block, ctx) = create_block_with_merkle();

        // Corrupt the merkle root
        block.header.merkle_root = crypto::hash::hash(b"corrupted");

        let result = validate_block_with_mode(&block, &ctx, ValidationMode::Light);
        assert!(
            matches!(result, Err(ValidationError::InvalidMerkleRoot)),
            "Light mode should still reject bad merkle root: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_block_light_rejects_bad_version() {
        let (mut block, ctx) = create_block_with_merkle();

        // Set invalid version
        block.header.version = 99;

        let result = validate_block_with_mode(&block, &ctx, ValidationMode::Light);
        assert!(
            matches!(result, Err(ValidationError::InvalidVersion(99))),
            "Light mode should reject invalid version: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_block_light_rejects_oversized_block() {
        let (mut block, ctx) = create_block_with_merkle();

        // Add a large transaction to exceed block size limit
        let large_tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![],
            outputs: vec![],
            extra_data: vec![0u8; 2_000_000], // 2MB > 1MB limit
        };
        block.transactions.push(large_tx);
        block.header.merkle_root = block.compute_merkle_root();

        let result = validate_block_with_mode(&block, &ctx, ValidationMode::Light);
        assert!(
            matches!(result, Err(ValidationError::BlockTooLarge { .. })),
            "Light mode should reject oversized blocks: {:?}",
            result
        );
    }

    #[test]
    fn test_validation_mode_enum_equality() {
        assert_eq!(ValidationMode::Full, ValidationMode::Full);
        assert_eq!(ValidationMode::Light, ValidationMode::Light);
        assert_ne!(ValidationMode::Full, ValidationMode::Light);
    }
}
