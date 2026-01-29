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

use crate::block::{Block, BlockHeader};
use crate::consensus::{
    is_producer_eligible, max_block_size, select_producer_for_slot, ConsensusParams, RewardMode,
    MAX_DRIFT, MAX_FUTURE_SLOTS, MAX_PAST_SLOTS, NETWORK_MARGIN, TOTAL_SUPPLY,
};
use crate::network::Network;
use crate::tpop::heartbeat::verify_hash_chain_vdf;
use crate::transaction::{
    AddBondData, ClaimBondData, ClaimData, ClaimWithdrawalData, ExitData, Input, Output,
    OutputType, RegistrationData, SlashData, Transaction, TxType, WithdrawalRequestData,
};
use crate::types::{Amount, BlockHeight};
use crypto::{Hash, PublicKey};

/// Validation errors.
///
/// Each variant provides specific context about what validation check failed,
/// enabling precise error reporting and debugging.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ValidationError {
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
    /// Registration chain state for chained VDF anti-Sybil verification.
    pub registration_chain: RegistrationChainState,
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
            registration_chain: RegistrationChainState::default(),
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
        self.active_producers = producers.iter().map(|(pk, _)| pk.clone()).collect();
        self.active_producers_weighted = producers;
        self
    }

    /// Set registration chain state for chained VDF validation.
    #[must_use]
    pub fn with_registration_chain(mut self, last_hash: Hash, sequence: u64) -> Self {
        self.registration_chain = RegistrationChainState::new(last_hash, sequence);
        self
    }
}

/// Validate a block header
pub fn validate_header(
    header: &BlockHeader,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    // 1. Version check
    if header.version != 1 {
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

    // Check not too far in the past (but must be after prev_slot which is already checked)
    if current_slot as u64 > header.slot as u64 + MAX_PAST_SLOTS {
        return Err(ValidationError::SlotTooPast {
            got: header.slot,
            current: current_slot,
            max_past: MAX_PAST_SLOTS,
        });
    }

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

    // 4. Validate rewards based on mode
    match ctx.params.reward_mode {
        RewardMode::DirectCoinbase => {
            // Legacy mode: must have coinbase as first transaction
            if block.transactions.is_empty() {
                return Err(ValidationError::MissingCoinbase);
            }

            let coinbase = &block.transactions[0];
            validate_coinbase(coinbase, ctx)?;

            // Validate other transactions (no coinbases after first)
            for (i, tx) in block.transactions.iter().skip(1).enumerate() {
                if tx.is_coinbase() {
                    return Err(ValidationError::InvalidTransaction(format!(
                        "transaction {} looks like coinbase",
                        i + 1
                    )));
                }
                validate_transaction(tx, ctx)?;
            }
        }
        RewardMode::EpochPool => {
            // Epoch pool mode: validate block rewards (coinbase/epoch_reward rules)
            validate_block_rewards(block, ctx)?;

            // Validate all transactions
            for tx in &block.transactions {
                validate_transaction(tx, ctx)?;
            }
        }
    }

    // 5. Check for double spends within block
    check_internal_double_spend(block)?;

    // 6. Validate VDF (if not in bootstrap)
    if !ctx.params.is_bootstrap(ctx.current_height) {
        validate_vdf(&block.header, ctx.network)?;
    }

    // 7. Validate producer eligibility (if not in bootstrap)
    validate_producer_eligibility(&block.header, ctx)?;

    Ok(())
}

/// Validate the coinbase transaction
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

/// Validate block reward transactions for epoch distribution mode.
///
/// In EpochPool mode:
/// - At epoch boundary: expect EpochReward txs, NO coinbase
/// - Non-boundary: NO rewards at all (pool accumulates)
///
/// In DirectCoinbase mode: this function does nothing (handled by validate_coinbase).
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

        // Calculate total distributed
        let total_distributed: Amount = epoch_rewards
            .iter()
            .map(|tx| tx.outputs.first().map(|o| o.amount).unwrap_or(0))
            .sum();

        // Expected pool for this epoch
        let expected_pool = ctx.params.total_epoch_reward(ctx.current_height);

        if total_distributed != expected_pool {
            return Err(ValidationError::InvalidBlock(format!(
                "epoch rewards {} != expected pool {}",
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
        // Non-boundary: NO rewards at all (pool accumulates)
        if block
            .transactions
            .iter()
            .any(|tx| tx.is_coinbase() || tx.is_epoch_reward())
        {
            return Err(ValidationError::InvalidBlock(
                "rewards only distributed at epoch boundary in EpochPool mode".to_string(),
            ));
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
    //    add_bond, request_withdrawal, claim_withdrawal, or epoch_reward)
    //    Note: add_bond requires inputs but handles its own error for specificity
    if tx.inputs.is_empty()
        && !tx.is_coinbase()
        && !tx.is_exit()
        && !tx.is_claim_reward()
        && !tx.is_claim_bond()
        && !tx.is_slash_producer()
        && !tx.is_add_bond()
        && !tx.is_request_withdrawal()
        && !tx.is_claim_withdrawal()
        && !tx.is_epoch_reward()
    {
        return Err(ValidationError::InvalidTransaction(
            "transaction must have inputs".to_string(),
        ));
    }

    // 3. Must have at least one output (unless exit, slash_producer, add_bond, or request_withdrawal)
    //    Note: claim_withdrawal and epoch_reward require exactly one output but handle their own errors
    if tx.outputs.is_empty()
        && !tx.is_exit()
        && !tx.is_slash_producer()
        && !tx.is_add_bond()
        && !tx.is_request_withdrawal()
        && !tx.is_claim_withdrawal()
        && !tx.is_epoch_reward()
    {
        return Err(ValidationError::InvalidTransaction(
            "transaction must have outputs".to_string(),
        ));
    }

    // 4. Validate all outputs
    let total_output = validate_outputs(&tx.outputs)?;

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
            validate_claim_withdrawal_data(tx)?;
        }
        TxType::EpochReward => {
            validate_epoch_reward_data(tx)?;
        }
    }

    Ok(())
}

/// Validate transaction outputs and compute safe total.
///
/// Returns the total output amount, or an error if any output is invalid
/// or if the total would overflow.
fn validate_outputs(outputs: &[Output]) -> Result<Amount, ValidationError> {
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
            }
            OutputType::Bond => {
                // Bond outputs must have a future lock time
                if output.lock_until == 0 {
                    return Err(ValidationError::InvalidBond(format!(
                        "bond output {} has zero lock_until",
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

    // Verify VDF proof for registration
    // (The actual VDF verification happens here)
    validate_registration_vdf(&reg_data)?;

    // Verify registration chain (anti-Sybil: prevents parallel registration)
    validate_registration_chain(&reg_data, ctx)?;

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
fn validate_registration_vdf(reg_data: &RegistrationData) -> Result<(), ValidationError> {
    // Create VDF input using the standard function
    let input = vdf::registration_input(&reg_data.public_key, reg_data.epoch);

    // Parse VDF output and proof
    let vdf_output = vdf::VdfOutput {
        value: reg_data.vdf_output.clone(),
    };

    let vdf_proof = vdf::VdfProof::from_bytes(&reg_data.vdf_proof).ok_or_else(|| {
        ValidationError::InvalidRegistration("invalid VDF proof format".to_string())
    })?;

    // Verify the VDF proof using base registration difficulty
    // Note: In production, the difficulty should be looked up based on current network size
    vdf::verify(&input, &vdf_output, &vdf_proof, vdf::T_REGISTER_BASE)
        .map_err(|_| ValidationError::InvalidRegistration("VDF verification failed".to_string()))
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
                    "double production evidence must have same producer in both headers".to_string(),
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

    // Must have no outputs (funds go into bond state)
    if !tx.outputs.is_empty() {
        return Err(ValidationError::InvalidAddBond(
            "add bond transaction must have no outputs".to_string(),
        ));
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
    // - Input amount matches bond_count * BOND_UNIT

    Ok(())
}

/// Validate withdrawal request transaction data.
///
/// Structural validation for RequestWithdrawal transactions:
/// - Must have no inputs (state-only operation)
/// - Must have no outputs (funds locked until claim)
/// - Must have valid withdrawal request data
///
/// Note: Producer bond holdings and FIFO calculation done at node level.
fn validate_withdrawal_request_data(tx: &Transaction) -> Result<(), ValidationError> {
    // Must have no inputs
    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidWithdrawalRequest(
            "withdrawal request must have no inputs".to_string(),
        ));
    }

    // Must have no outputs (funds locked in pending state)
    if !tx.outputs.is_empty() {
        return Err(ValidationError::InvalidWithdrawalRequest(
            "withdrawal request must have no outputs".to_string(),
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

    // Note: These validations are done at node level:
    // - Producer is registered
    // - Producer has enough bonds to withdraw
    // - FIFO calculation of net/penalty amounts

    Ok(())
}

/// Validate claim withdrawal transaction data.
///
/// Structural validation for ClaimWithdrawal transactions:
/// - Must have no inputs (funds come from pending withdrawal)
/// - Must have exactly one output (the net amount to destination)
/// - Must have valid claim withdrawal data
///
/// Note: 7-day delay check and matching pending withdrawal done at node level.
fn validate_claim_withdrawal_data(tx: &Transaction) -> Result<(), ValidationError> {
    // Must have no inputs
    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidClaimWithdrawal(
            "claim withdrawal must have no inputs".to_string(),
        ));
    }

    // Must have exactly one output (the net amount)
    if tx.outputs.len() != 1 {
        return Err(ValidationError::InvalidClaimWithdrawal(
            "claim withdrawal must have exactly one output".to_string(),
        ));
    }

    // Output must be a normal output
    if tx.outputs[0].output_type != OutputType::Normal {
        return Err(ValidationError::InvalidClaimWithdrawal(
            "claim withdrawal output must be normal type".to_string(),
        ));
    }

    // Parse claim withdrawal data from extra_data
    let _claim_data = ClaimWithdrawalData::from_bytes(&tx.extra_data).ok_or_else(|| {
        ValidationError::InvalidClaimWithdrawal("invalid claim withdrawal data".to_string())
    })?;

    // Note: These validations are done at node level:
    // - Pending withdrawal exists at withdrawal_index
    // - 7-day delay has passed
    // - Output amount matches pending withdrawal net_amount
    // - Output destination matches pending withdrawal destination

    Ok(())
}

/// Validate epoch reward transaction data
///
/// Basic validation of EpochReward transactions:
/// - Must have no inputs (minted)
/// - Must have exactly one output
/// - Output must be Normal type
/// - Must have valid EpochRewardData
fn validate_epoch_reward_data(tx: &Transaction) -> Result<(), ValidationError> {
    use crate::transaction::EpochRewardData;

    // Must have no inputs (minted)
    if !tx.inputs.is_empty() {
        return Err(ValidationError::InvalidEpochReward(
            "epoch reward must have no inputs".to_string(),
        ));
    }

    // Must have exactly one output
    if tx.outputs.len() != 1 {
        return Err(ValidationError::InvalidEpochReward(
            "epoch reward must have exactly one output".to_string(),
        ));
    }

    // Output must be Normal type
    if tx.outputs[0].output_type != OutputType::Normal {
        return Err(ValidationError::InvalidEpochReward(
            "epoch reward output must be Normal type".to_string(),
        ));
    }

    // Must have valid EpochRewardData
    let _epoch_data = EpochRewardData::from_bytes(&tx.extra_data).ok_or_else(|| {
        ValidationError::InvalidEpochReward("invalid epoch reward data".to_string())
    })?;

    // Note: Block-level validation (in Milestone 4) will also verify:
    // - Epoch number matches current epoch
    // - Only valid at epoch boundaries
    // - Total distributed equals expected pool

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

    // Get the signing message (tx hash)
    let signing_hash = tx.signing_message();

    // Validate each input
    let mut total_input: Amount = 0;

    for (i, input) in tx.inputs.iter().enumerate() {
        // Look up the UTXO
        let utxo = utxo_provider
            .get_utxo(&input.prev_tx_hash, input.output_index)
            .ok_or_else(|| ValidationError::OutputNotFound {
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

        // Check lock time
        if !utxo.output.is_spendable_at(ctx.current_height) {
            return Err(ValidationError::OutputLocked {
                lock_height: utxo.output.lock_until,
                current_height: ctx.current_height,
            });
        }

        // Verify signature
        verify_input_signature(input, &signing_hash, &utxo, i)?;

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

    Ok(())
}

/// Verify the signature on a transaction input.
fn verify_input_signature(
    input: &Input,
    signing_hash: &Hash,
    utxo: &UtxoInfo,
    input_index: usize,
) -> Result<(), ValidationError> {
    // We need the public key to verify the signature
    let pubkey = utxo
        .pubkey
        .as_ref()
        .ok_or_else(|| ValidationError::InvalidSignature { index: input_index })?;

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
/// Uses hash-chain VDF (10M SHA-256 iterations, ~700ms) which is:
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

/// Validate that the block producer is eligible for the slot.
///
/// This checks that:
/// 1. The producer is in the top 3 eligible producers for the slot
/// 2. The block timestamp falls within the producer's allowed window
///
/// During bootstrap (height < BOOTSTRAP_BLOCKS), any producer is allowed.
pub fn validate_producer_eligibility(
    header: &BlockHeader,
    ctx: &ValidationContext,
) -> Result<(), ValidationError> {
    // During bootstrap, anyone can produce
    if ctx.params.is_bootstrap(ctx.current_height) {
        return Ok(());
    }

    // If no active producers are set, skip validation
    // (this allows backward compatibility during testing)
    if ctx.active_producers.is_empty() {
        return Ok(());
    }

    // Select eligible producers for this slot using consecutive tickets
    // Selection is independent of prev_hash (anti-grinding protection)
    let eligible = select_producer_for_slot(header.slot, &ctx.active_producers_weighted);

    if eligible.is_empty() {
        return Err(ValidationError::InvalidProducer);
    }

    // Calculate time offset within the slot
    let slot_start = ctx.params.slot_to_timestamp(header.slot);
    let slot_offset = header.timestamp.saturating_sub(slot_start);

    // Check if producer is eligible at this time
    if !is_producer_eligible(&header.producer, &eligible, slot_offset) {
        return Err(ValidationError::InvalidProducer);
    }

    Ok(())
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
            version: 1,
            prev_hash: Hash::ZERO,
            merkle_root: Hash::ZERO,
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
            version: 1,
            prev_hash: Hash::ZERO,
            merkle_root: Hash::ZERO,
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
            version: 1,
            prev_hash: Hash::ZERO,
            merkle_root: Hash::ZERO,
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
        let pubkey = keypair.public_key().clone();
        let pubkey_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());

        // Create a previous transaction output
        let prev_tx_hash = crypto::hash::hash(b"prev_tx");
        let prev_output = Output::normal(1000, pubkey_hash);

        // Set up the mock UTXO provider
        let mut utxo_provider = MockUtxoProvider::new();
        utxo_provider.add_utxo(prev_tx_hash, 0, prev_output, pubkey.clone());

        // Create the spending transaction
        let new_pubkey_hash = crypto::hash::hash(b"recipient");
        let mut tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![Input::new(prev_tx_hash, 0)],
            outputs: vec![Output::normal(900, new_pubkey_hash)],
            extra_data: vec![],
        };

        // Sign the transaction
        let signing_hash = tx.signing_message();
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
        let pubkey = keypair.public_key().clone();
        let pubkey_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());

        let prev_tx_hash = crypto::hash::hash(b"prev_tx");
        let prev_output = Output::normal(100, pubkey_hash); // Only 100

        let mut utxo_provider = MockUtxoProvider::new();
        utxo_provider.add_utxo(prev_tx_hash, 0, prev_output, pubkey.clone());

        let new_pubkey_hash = crypto::hash::hash(b"recipient");
        let mut tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![Input::new(prev_tx_hash, 0)],
            outputs: vec![Output::normal(200, new_pubkey_hash)], // Needs 200
            extra_data: vec![],
        };

        // Sign the transaction
        let signing_hash = tx.signing_message();
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
        let owner_pubkey = owner_keypair.public_key().clone();
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

        // Sign with wrong key
        let signing_hash = tx.signing_message();
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
        let pubkey = keypair.public_key().clone();
        let pubkey_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());

        let prev_tx_hash = crypto::hash::hash(b"prev_tx");
        // Output locked until height 1000
        let prev_output = Output::bond(1000, pubkey_hash, 1000);

        let mut utxo_provider = MockUtxoProvider::new();
        utxo_provider.add_utxo(prev_tx_hash, 0, prev_output, pubkey.clone());

        let new_pubkey_hash = crypto::hash::hash(b"recipient");
        let mut tx = Transaction {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: vec![Input::new(prev_tx_hash, 0)],
            outputs: vec![Output::normal(900, new_pubkey_hash)],
            extra_data: vec![],
        };

        let signing_hash = tx.signing_message();
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
            let result = validate_outputs(&outputs);
            prop_assert!(result.is_ok());
            prop_assert_eq!(result.unwrap(), amounts.iter().sum::<u64>());
        }

        /// Zero amount outputs fail validation
        #[test]
        fn prop_zero_amount_fails(seed: [u8; 32]) {
            let mut hash = seed;
            hash[0] = 1; // Non-zero
            let outputs = vec![Output::normal(0, Hash::from_bytes(hash))];
            let result = validate_outputs(&outputs);
            prop_assert!(result.is_err());
        }

        /// Zero pubkey_hash fails validation
        #[test]
        fn prop_zero_pubkey_hash_fails(amount in 1u64..1_000_000_000u64) {
            let outputs = vec![Output::normal(amount, Hash::ZERO)];
            let result = validate_outputs(&outputs);
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
            };
            let result = validate_outputs(&[output]);
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
            };
            let result = validate_outputs(&[output]);
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
            let pubkey = keypair.public_key().clone();
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

            let signing_hash = tx.signing_message();
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

            let owner_pubkey = owner_keypair.public_key().clone();
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

            // Sign with wrong key
            let signing_hash = tx.signing_message();
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
                    timestamp: 0,
                    slot: 0,
                    producer: crypto::PublicKey::from_bytes([0u8; 32]),
                    vdf_output: vdf::VdfOutput { value: vec![] },
                    vdf_proof: vdf::VdfProof::empty(),
                        },
                transactions: vec![tx1, tx2],
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
        let pubkey = keypair.public_key().clone();

        // Create an input (required for AddBond)
        let prev_tx_hash = crypto::hash::hash(b"prev_tx");
        let input = Input::new(prev_tx_hash, 0);

        let tx = Transaction::new_add_bond(vec![input], pubkey.clone(), 5);

        assert!(tx.is_add_bond());
        assert_eq!(tx.tx_type, TxType::AddBond);
        assert_eq!(tx.inputs.len(), 1);
        assert!(tx.outputs.is_empty());

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
        let pubkey = keypair.public_key().clone();

        // Create AddBond with no inputs (invalid)
        let tx = Transaction::new_add_bond(vec![], pubkey, 5);

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidAddBond(msg)) if msg.contains("must have inputs")
        ));
    }

    #[test]
    fn test_add_bond_has_outputs() {
        let ctx = test_context();
        let keypair = crypto::KeyPair::generate();
        let pubkey = keypair.public_key().clone();
        let pubkey_hash = crypto::hash::hash(b"test");

        // Create AddBond with output (invalid)
        let bond_data = crate::transaction::AddBondData::new(pubkey, 5);
        let tx = Transaction {
            version: 1,
            tx_type: TxType::AddBond,
            inputs: vec![Input::new(crypto::hash::hash(b"prev"), 0)],
            outputs: vec![Output::normal(100, pubkey_hash)], // Should have no outputs
            extra_data: bond_data.to_bytes(),
        };

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidAddBond(msg)) if msg.contains("must have no outputs")
        ));
    }

    #[test]
    fn test_add_bond_zero_bond_count() {
        let ctx = test_context();
        let keypair = crypto::KeyPair::generate();
        let pubkey = keypair.public_key().clone();

        // Create AddBond with zero bond count (invalid)
        let tx =
            Transaction::new_add_bond(vec![Input::new(crypto::hash::hash(b"prev"), 0)], pubkey, 0);

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidAddBond(msg)) if msg.contains("bond count must be positive")
        ));
    }

    #[test]
    fn test_add_bond_data_serialization() {
        let keypair = crypto::KeyPair::generate();
        let pubkey = keypair.public_key().clone();

        let tx = Transaction::new_add_bond(
            vec![Input::new(crypto::hash::hash(b"prev"), 0)],
            pubkey.clone(),
            10,
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
        let pubkey = keypair.public_key().clone();
        let destination = crypto::hash::hash(b"destination");

        let tx = Transaction::new_request_withdrawal(pubkey.clone(), 3, destination);

        assert!(tx.is_request_withdrawal());
        assert_eq!(tx.tx_type, TxType::RequestWithdrawal);
        assert!(tx.inputs.is_empty());
        assert!(tx.outputs.is_empty());

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
    fn test_request_withdrawal_with_inputs() {
        let ctx = test_context();
        let keypair = crypto::KeyPair::generate();
        let pubkey = keypair.public_key().clone();
        let destination = crypto::hash::hash(b"destination");

        // Create RequestWithdrawal with inputs (invalid)
        let withdrawal_data =
            crate::transaction::WithdrawalRequestData::new(pubkey, 3, destination);
        let tx = Transaction {
            version: 1,
            tx_type: TxType::RequestWithdrawal,
            inputs: vec![Input::new(crypto::hash::hash(b"prev"), 0)], // Should be empty
            outputs: vec![],
            extra_data: withdrawal_data.to_bytes(),
        };

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidWithdrawalRequest(msg)) if msg.contains("must have no inputs")
        ));
    }

    #[test]
    fn test_request_withdrawal_with_outputs() {
        let ctx = test_context();
        let keypair = crypto::KeyPair::generate();
        let pubkey = keypair.public_key().clone();
        let destination = crypto::hash::hash(b"destination");

        // Create RequestWithdrawal with outputs (invalid)
        let withdrawal_data =
            crate::transaction::WithdrawalRequestData::new(pubkey, 3, destination);
        let tx = Transaction {
            version: 1,
            tx_type: TxType::RequestWithdrawal,
            inputs: vec![],
            outputs: vec![Output::normal(100, destination)], // Should be empty
            extra_data: withdrawal_data.to_bytes(),
        };

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidWithdrawalRequest(msg)) if msg.contains("must have no outputs")
        ));
    }

    #[test]
    fn test_request_withdrawal_zero_bond_count() {
        let ctx = test_context();
        let keypair = crypto::KeyPair::generate();
        let pubkey = keypair.public_key().clone();
        let destination = crypto::hash::hash(b"destination");

        // Create RequestWithdrawal with zero bond count (invalid)
        let tx = Transaction::new_request_withdrawal(pubkey, 0, destination);

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
        let pubkey = keypair.public_key().clone();

        // Create RequestWithdrawal with zero destination (invalid)
        let tx = Transaction::new_request_withdrawal(pubkey, 3, Hash::ZERO);

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidWithdrawalRequest(msg)) if msg.contains("destination cannot be zero")
        ));
    }

    #[test]
    fn test_request_withdrawal_data_serialization() {
        let keypair = crypto::KeyPair::generate();
        let pubkey = keypair.public_key().clone();
        let destination = crypto::hash::hash(b"destination");

        let tx = Transaction::new_request_withdrawal(pubkey.clone(), 7, destination);
        let bytes = tx.serialize();
        let recovered = Transaction::deserialize(&bytes).unwrap();

        assert_eq!(tx.tx_type, recovered.tx_type);
        let recovered_data = recovered.withdrawal_request_data().unwrap();
        assert_eq!(recovered_data.producer_pubkey, pubkey);
        assert_eq!(recovered_data.bond_count, 7);
        assert_eq!(recovered_data.destination, destination);
    }

    // ==========================================================================
    // TX Type 9: ClaimWithdrawal Validation Tests
    // ==========================================================================

    #[test]
    fn test_claim_withdrawal_transaction() {
        let ctx = test_context();
        let keypair = crypto::KeyPair::generate();
        let pubkey = keypair.public_key().clone();
        let destination = crypto::hash::hash(b"destination");
        let net_amount = 500_000_000;

        let tx = Transaction::new_claim_withdrawal(pubkey.clone(), 0, net_amount, destination);

        assert!(tx.is_claim_withdrawal());
        assert_eq!(tx.tx_type, TxType::ClaimWithdrawal);
        assert!(tx.inputs.is_empty());
        assert_eq!(tx.outputs.len(), 1);
        assert_eq!(tx.outputs[0].amount, net_amount);
        assert_eq!(tx.outputs[0].output_type, OutputType::Normal);

        // Verify claim data can be parsed
        let claim_data = tx.claim_withdrawal_data().unwrap();
        assert_eq!(claim_data.producer_pubkey, pubkey);
        assert_eq!(claim_data.withdrawal_index, 0);

        // Structural validation should pass
        let result = validate_transaction(&tx, &ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_claim_withdrawal_with_inputs() {
        let ctx = test_context();
        let keypair = crypto::KeyPair::generate();
        let pubkey = keypair.public_key().clone();
        let destination = crypto::hash::hash(b"destination");

        // Create ClaimWithdrawal with inputs (invalid)
        let claim_data = crate::transaction::ClaimWithdrawalData::new(pubkey, 0);
        let tx = Transaction {
            version: 1,
            tx_type: TxType::ClaimWithdrawal,
            inputs: vec![Input::new(crypto::hash::hash(b"prev"), 0)], // Should be empty
            outputs: vec![Output::normal(500_000_000, destination)],
            extra_data: claim_data.to_bytes(),
        };

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidClaimWithdrawal(msg)) if msg.contains("must have no inputs")
        ));
    }

    #[test]
    fn test_claim_withdrawal_no_outputs() {
        let ctx = test_context();
        let keypair = crypto::KeyPair::generate();
        let pubkey = keypair.public_key().clone();

        // Create ClaimWithdrawal with no outputs (invalid)
        let claim_data = crate::transaction::ClaimWithdrawalData::new(pubkey, 0);
        let tx = Transaction {
            version: 1,
            tx_type: TxType::ClaimWithdrawal,
            inputs: vec![],
            outputs: vec![], // Should have exactly 1 output
            extra_data: claim_data.to_bytes(),
        };

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidClaimWithdrawal(msg)) if msg.contains("exactly one output")
        ));
    }

    #[test]
    fn test_claim_withdrawal_multiple_outputs() {
        let ctx = test_context();
        let keypair = crypto::KeyPair::generate();
        let pubkey = keypair.public_key().clone();
        let destination = crypto::hash::hash(b"destination");

        // Create ClaimWithdrawal with multiple outputs (invalid)
        let claim_data = crate::transaction::ClaimWithdrawalData::new(pubkey, 0);
        let tx = Transaction {
            version: 1,
            tx_type: TxType::ClaimWithdrawal,
            inputs: vec![],
            outputs: vec![
                Output::normal(250_000_000, destination),
                Output::normal(250_000_000, destination),
            ], // Should have exactly 1 output
            extra_data: claim_data.to_bytes(),
        };

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidClaimWithdrawal(msg)) if msg.contains("exactly one output")
        ));
    }

    #[test]
    fn test_claim_withdrawal_wrong_output_type() {
        let ctx = test_context();
        let keypair = crypto::KeyPair::generate();
        let pubkey = keypair.public_key().clone();
        let destination = crypto::hash::hash(b"destination");

        // Create ClaimWithdrawal with bond output type (invalid)
        let claim_data = crate::transaction::ClaimWithdrawalData::new(pubkey, 0);
        let tx = Transaction {
            version: 1,
            tx_type: TxType::ClaimWithdrawal,
            inputs: vec![],
            outputs: vec![Output::bond(500_000_000, destination, 1000)], // Should be Normal
            extra_data: claim_data.to_bytes(),
        };

        let result = validate_transaction(&tx, &ctx);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidClaimWithdrawal(msg)) if msg.contains("must be normal type")
        ));
    }

    #[test]
    fn test_claim_withdrawal_data_serialization() {
        let keypair = crypto::KeyPair::generate();
        let pubkey = keypair.public_key().clone();
        let destination = crypto::hash::hash(b"destination");

        let tx = Transaction::new_claim_withdrawal(pubkey.clone(), 5, 1_000_000_000, destination);
        let bytes = tx.serialize();
        let recovered = Transaction::deserialize(&bytes).unwrap();

        assert_eq!(tx.tx_type, recovered.tx_type);
        let recovered_data = recovered.claim_withdrawal_data().unwrap();
        assert_eq!(recovered_data.producer_pubkey, pubkey);
        assert_eq!(recovered_data.withdrawal_index, 5);
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
    /// - Mainnet: 2048-bit discriminant, higher iterations for security
    /// - Testnet: 512-bit discriminant, lower iterations for practical testing
    /// - Devnet: 256-bit discriminant, uses fast hash-chain VDF
    #[test]
    fn test_vdf_validation_uses_network_specific_parameters() {
        // VDF is enabled for all networks (using hash-chain VDF for devnet)
        assert!(Network::Mainnet.vdf_enabled());
        assert!(Network::Testnet.vdf_enabled());
        assert!(Network::Devnet.vdf_enabled()); // Uses fast hash-chain VDF

        // Each network has different VDF iterations (class-group VDF params, not used for devnet)
        assert!(Network::Mainnet.vdf_iterations() > Network::Testnet.vdf_iterations());
        assert!(Network::Testnet.vdf_iterations() > Network::Devnet.vdf_iterations());

        // Testnet should have minimal iterations for practical testing
        assert_eq!(Network::Testnet.vdf_iterations(), 10);

        // Devnet has single iteration (uses hash-chain VDF instead)
        assert_eq!(Network::Devnet.vdf_iterations(), 1);

        // Each network has different discriminant sizes
        assert!(
            Network::Mainnet.vdf_discriminant_bits() > Network::Testnet.vdf_discriminant_bits()
        );
        assert!(Network::Testnet.vdf_discriminant_bits() > Network::Devnet.vdf_discriminant_bits());

        // Mainnet uses production-grade 2048-bit discriminant
        assert_eq!(Network::Mainnet.vdf_discriminant_bits(), 2048);

        // Testnet uses 512-bit discriminant for faster testing
        assert_eq!(Network::Testnet.vdf_discriminant_bits(), 512);

        // Devnet uses 256-bit discriminant (uses hash-chain VDF)
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
        let mut tx =
            Transaction::new_epoch_reward(1, keypair.public_key().clone(), 1000, pubkey_hash);

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
    fn test_validate_epoch_reward_one_output() {
        let keypair = crypto::KeyPair::generate();
        let pubkey_hash = crypto::hash::hash(b"recipient");
        let mut tx =
            Transaction::new_epoch_reward(1, keypair.public_key().clone(), 1000, pubkey_hash);

        // Add extra output with valid pubkey_hash (invalid for epoch reward - must have exactly 1 output)
        let extra_pubkey_hash = crypto::hash::hash(b"another_recipient");
        tx.outputs.push(Output::normal(500, extra_pubkey_hash));

        let ctx = test_context();
        let result = validate_transaction(&tx, &ctx);

        assert!(matches!(
            result,
            Err(ValidationError::InvalidEpochReward(_))
        ));
    }

    #[test]
    fn test_validate_epoch_reward_normal_output_type() {
        let keypair = crypto::KeyPair::generate();
        let pubkey_hash = crypto::hash::hash(b"recipient");
        let mut tx =
            Transaction::new_epoch_reward(1, keypair.public_key().clone(), 1000, pubkey_hash);

        // Change to bond output (invalid)
        tx.outputs[0].output_type = OutputType::Bond;
        tx.outputs[0].lock_until = 1000;

        let ctx = test_context();
        let result = validate_transaction(&tx, &ctx);

        assert!(matches!(
            result,
            Err(ValidationError::InvalidEpochReward(_))
        ));
    }

    #[test]
    fn test_validate_epoch_reward_invalid_data() {
        let pubkey_hash = crypto::hash::hash(b"recipient");
        let tx = Transaction {
            version: 1,
            tx_type: TxType::EpochReward,
            inputs: Vec::new(),
            outputs: vec![Output::normal(1000, pubkey_hash)],
            extra_data: vec![0u8; 10], // Invalid - too short for EpochRewardData
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
        let keypair = crypto::KeyPair::generate();
        let pubkey_hash = crypto::hash::hash(b"recipient");
        let tx = Transaction::new_epoch_reward(1, keypair.public_key().clone(), 1000, pubkey_hash);

        let ctx = test_context();
        let result = validate_transaction(&tx, &ctx);

        // Should pass basic validation (block-level validation is separate)
        assert!(result.is_ok());
    }

    #[test]
    fn test_epoch_reward_skips_utxo_validation() {
        let keypair = crypto::KeyPair::generate();
        let pubkey_hash = crypto::hash::hash(b"recipient");
        let tx = Transaction::new_epoch_reward(1, keypair.public_key().clone(), 1000, pubkey_hash);

        let ctx = test_context();
        let utxo_provider = MockUtxoProvider::new(); // Empty UTXO set

        // Should pass even with no UTXOs (it's minted, like coinbase)
        let result = validate_transaction_with_utxos(&tx, &ctx, &utxo_provider);
        assert!(result.is_ok());
    }

    // ==========================================================================
    // Block Rewards Validation Tests (EpochPool Mode)
    // ==========================================================================

    /// Create a context with EpochPool reward mode for testing
    fn epoch_pool_context_at_slot(slot: u32) -> ValidationContext {
        let mut params = ConsensusParams::mainnet();
        params.reward_mode = RewardMode::EpochPool;

        ValidationContext::new(params, Network::Mainnet, GENESIS_TIME + (slot as u64 * 60), slot as u64)
            .with_prev_block(slot.saturating_sub(1), GENESIS_TIME + ((slot as u64 - 1) * 60), Hash::ZERO)
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
            producer: keypair.public_key().clone(),
            vdf_output: VdfOutput { value: vec![] },
            vdf_proof: VdfProof::default(),
        };

        Block {
            header,
            transactions,
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
            Transaction::new_epoch_reward(0, keypair.public_key().clone(), 1000, pubkey_hash);
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
    fn test_validate_block_rewards_total_must_match_pool() {
        let params = ConsensusParams::mainnet();
        let boundary_slot = params.slots_per_reward_epoch; // First epoch boundary
        let expected_epoch = params.slot_to_reward_epoch(boundary_slot);

        let ctx = epoch_pool_context_at_slot(boundary_slot);

        // Create block with incorrect total (half of expected)
        let keypair = crypto::KeyPair::generate();
        let pubkey_hash = crypto::hash::hash(b"recipient");
        let expected_pool = ctx.params.total_epoch_reward(boundary_slot as u64);
        let epoch_reward = Transaction::new_epoch_reward(
            expected_epoch as u64,
            keypair.public_key().clone(),
            expected_pool / 2, // Wrong amount!
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
            keypair.public_key().clone(),
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
            keypair.public_key().clone(),
            expected_pool,
            pubkey_hash,
        );
        let block = create_test_block_at_slot(boundary_slot, vec![epoch_reward]);

        let result = validate_block_rewards(&block, &ctx);
        assert!(matches!(result, Err(ValidationError::InvalidEpochReward(_))));
    }
}
