//! # doli-core
//!
//! Core types, validation, and consensus rules for the DOLI protocol.
//!
//! This crate is the foundation of the DOLI blockchain, providing all the
//! fundamental data structures and validation logic needed to process
//! blocks and transactions.
//!
//! ## Overview
//!
//! DOLI uses a UTXO (Unspent Transaction Output) model similar to Bitcoin,
//! combined with Proof of Time (PoT) consensus using VDF.
//!
//! ## Proof of Time
//!
//! Unlike other consensus mechanisms:
//! - **Proof of Work**: Parallelizable computation (more hardware = more blocks)
//! - **Proof of Stake**: Capital-based selection (more stake = higher chance)
//! - **Proof of Time**: VDF-based sequential computation (time is the scarce resource)
//!
//! Key properties:
//! - One producer per slot (10 seconds)
//! - Selection based on bond count (deterministic round-robin)
//! - VDF provides anti-grinding protection (~7s computation)
//! - 100% of block reward goes to the producer
//!
//! ## Modules
//!
//! - [`block`] - Block and header structures
//! - [`consensus`] - Proof of Time parameters and rules
//! - [`transaction`] - Transaction types including transfers and producer registration
//! - [`types`] - Core numeric types (Amount, Slot, Epoch, Era)
//! - [`validation`] - Block and transaction validation rules
//!
//! ## Time Structure
//!
//! DOLI organizes time hierarchically:
//!
//! | Unit  | Duration    | Description                              |
//! |-------|-------------|------------------------------------------|
//! | Slot  | 10 seconds  | Single block production opportunity      |
//! | Epoch | 1 hour      | 360 slots, producer set is stable        |
//! | Era   | ~4 years    | 12,614,400 blocks, emission halving      |
//!
//! ## Transaction Types
//!
//! ```text
//! TxType::Transfer  - Standard value transfer between addresses
//! TxType::Register  - Producer registration (requires VDF proof)
//! TxType::Coinbase  - Block reward to producer
//! ```
//!
//! ## Example Usage
//!
//! ```rust
//! use doli_core::{Amount, BlockHeight, Slot};
//! use doli_core::consensus::ConsensusParams;
//! use doli_core::types::{coins_to_units, units_to_coins};
//!
//! // Access consensus parameters
//! let params = ConsensusParams::mainnet();
//!
//! // Work with amounts (in base units, 1 DOLI = 10^8 units)
//! let amount: Amount = coins_to_units(100); // 100 DOLI
//! assert_eq!(amount, 10_000_000_000);
//!
//! // Convert back to coins
//! assert_eq!(units_to_coins(amount), 100);
//!
//! // Slots represent time in the protocol
//! let slot: Slot = 42;
//! let epoch = params.slot_to_epoch(slot);
//! ```
//!
//! ## Validation
//!
//! All blocks and transactions undergo rigorous validation:
//!
//! - **Syntactic validation**: Structure and format checks
//! - **Contextual validation**: UTXO existence, balance checks
//! - **Consensus validation**: VDF proofs, timing rules, producer eligibility
//!
//! ## Security Considerations
//!
//! This crate implements the following security measures:
//!
//! ### Overflow Protection
//! - All amount calculations use checked or saturating arithmetic
//! - Total output validation prevents amounts exceeding total supply
//! - Era/epoch calculations handle edge cases at type boundaries
//!
//! ### Double-Spend Prevention
//! - UTXO model ensures each output can only be spent once
//! - Signature verification confirms ownership before spending
//! - Transaction hash excludes signatures to prevent malleability
//!
//! ### Input Validation
//! - Zero amounts are rejected (prevents dust attacks)
//! - Zero pubkey hashes are rejected (prevents burn-to-anyone)
//! - Version numbers are validated for forward compatibility
//! - Bond outputs require positive lock heights
//!
//! ### Timing Security
//! - Slot numbers are derived from timestamps, not freely chosen
//! - VDF proofs anchor blocks to real-world time
//! - Producer selection depends on previous block hash (unpredictable)
//!
//! ### Economic Security
//! - Bond outputs enforce time-locked collateral for producers
//! - Slashing conditions deter equivocation
//! - Registration VDF limits identity creation rate
//!
//! For complete security documentation, see `SECURITY.md` in the repository root.

pub mod attestation;
pub mod block;
pub mod chainspec;
pub mod conditions;
pub mod config_validation;
pub mod consensus;
pub mod discovery;
pub mod finality;
pub mod genesis;
pub mod heartbeat;
pub mod lending;
pub mod maintainer;
pub mod network;
pub mod network_params;
pub mod pool;
pub mod presence;
pub mod rewards;
pub mod scheduler;
pub mod tpop;
pub mod transaction;
pub mod types;
pub mod validation;

pub use attestation::{
    attestation_minute, attestation_minutes_per_epoch, attestation_qualification_threshold,
    decode_attestation_bitfield, encode_attestation_bitfield, validate_attestation_bitfield,
    Attestation, AttestationError, MinuteAttestationTracker, RegionAggregate,
    ATTESTATION_MINUTES_PER_EPOCH, ATTESTATION_QUALIFICATION_THRESHOLD,
};
pub use block::{Block, BlockBuilder, BlockHeader};
pub use conditions::{
    evaluate as evaluate_condition, Condition, ConditionError, EvalContext, Witness,
    WitnessSignature as ConditionWitnessSignature, CONDITION_VERSION, HASHLOCK_DOMAIN,
    MAX_CONDITION_DEPTH, MAX_CONDITION_OPS, MAX_MULTISIG_KEYS, MAX_THRESHOLD_CONDITIONS,
    MAX_WITNESS_SIZE,
};
#[allow(deprecated)]
pub use consensus::{
    allowed_producer_rank,
    allowed_producer_rank_ms,
    calculate_exit_with_quarter,
    // Tiered architecture
    compute_tier1_set,
    eligible_rank_at_ms,
    get_producer_rank,
    is_producer_eligible,
    is_producer_eligible_ms,
    is_protocol_active,
    is_rank_eligible_at_ms,
    producer_region,
    producer_tier,
    select_producer_for_slot,
    withdrawal_penalty_rate,
    withdrawal_penalty_rate_with_quarter,
    BondEntry,
    BondError,
    BondsMaturitySummary,
    ConsensusParams,
    // Producer state and scoring
    PresenceScore,
    ProducerBonds,
    ProducerState,
    WithdrawalResult,
    // Block-height based reward epochs
    BLOCKS_PER_REWARD_EPOCH,
    BLOCK_REWARD_POOL,
    // Bond stacking system
    BOND_UNIT,
    DELEGATE_REWARD_PCT,
    DELEGATION_UNBONDING_SLOTS,
    ELIGIBLE_PRODUCER_POOL,
    // Sequential fallback window constants
    EPOCH_REWARD_POOL,
    FALLBACK_TIMEOUT_MS,
    GENESIS_TIME,
    INITIAL_PRESENCE_SCORE,
    // Protocol versioning
    INITIAL_PROTOCOL_VERSION,
    MAX_BONDS_PER_PRODUCER,
    MAX_DRIFT_MS,
    MAX_FALLBACK_PRODUCERS,
    MAX_FALLBACK_RANKS,
    MAX_FUTURE_SLOTS,
    MAX_PAST_SLOTS,
    MAX_PRESENCE_SCORE,
    MIN_PRESENCE_RATE,
    MIN_PRESENCE_SCORE,
    NUM_REGIONS,
    // Deprecated window constants (kept for backward compatibility)
    PRIMARY_WINDOW_MS,
    PRIMARY_WINDOW_SECS,
    SCORE_MISS_PENALTY,
    SCORE_PRODUCE_BONUS,
    SECONDARY_WINDOW_MS,
    SECONDARY_WINDOW_SECS,
    SIGNATURE_WINDOW_MS,
    // Reward epoch constants (slot-based, legacy)
    SLOTS_PER_REWARD_EPOCH,
    // Proof of Time parameters
    SLOT_DURATION,
    STAKER_REWARD_PCT,
    TERTIARY_WINDOW_MS,
    TERTIARY_WINDOW_SECS,
    TIER1_MAX_VALIDATORS,
    TIER2_MAX_ATTESTORS,
    VDF_DISCRIMINANT_BITS,
    VESTING_PERIOD_SLOTS,
    VESTING_QUARTER_SLOTS,
    YEAR_IN_SLOTS,
};
pub use finality::{
    FinalityCheckpoint, FinalityTracker, FINALITY_THRESHOLD_PCT, FINALITY_TIMEOUT_SLOTS,
};

// Block-height based reward epoch utilities
pub use consensus::reward_epoch;

// Presence commitment for weighted presence rewards
pub use presence::PresenceCommitment;

// Weighted presence reward calculation
pub use rewards::{
    complete_epoch_range, complete_epoch_range_with, complete_epochs_at_height,
    complete_epochs_at_height_with, epoch_boundaries, epoch_boundaries_with, is_epoch_complete,
    is_epoch_complete_with, BlockSource, ClaimableSummary, RewardError, WeightedRewardCalculation,
    WeightedRewardCalculator,
};

// Deterministic slot scheduler (replaces heartbeat/presence system)
pub use scheduler::{DeterministicScheduler, ScheduledProducer, SchedulerStats, MAX_FALLBACK_RANK};

// Heartbeat VDF and witness system for presence proofs (consensus-affecting)
// Note: Use `heartbeat::` prefix to access these types to avoid conflict with tpop telemetry
pub use heartbeat::{
    hash_chain_vdf, verify_hash_chain_vdf, Heartbeat, WitnessSignature, HEARTBEAT_VERSION,
    MIN_WITNESS_SIGNATURES,
};
// HeartbeatError and HEARTBEAT_VDF_ITERATIONS are exported from tpop (telemetry)
// For consensus heartbeat error, use heartbeat::HeartbeatError directly

pub use genesis::{
    generate_genesis_block, genesis_hash, verify_genesis_block, GenesisConfig, GenesisError,
};
pub use lending::{
    collateral_value_from_twap, compute_depositor_earnings, compute_interest, compute_ltv_bps,
    compute_total_debt, is_liquidatable, verify_creation_ltv, SLOTS_PER_YEAR,
};
pub use maintainer::{
    derive_maintainer_set, BlockchainReader, MaintainerChange, MaintainerChangeData,
    MaintainerError, MaintainerSet, MaintainerSignature, ProtocolActivationData,
    INITIAL_MAINTAINER_COUNT, MAINTAINER_THRESHOLD, MAX_MAINTAINERS, MIN_MAINTAINERS,
};
pub use network::Network;
pub use network_params::NetworkParams;
pub use pool::{
    compute_initial_lp_shares, compute_lp_shares, compute_remove_liquidity, compute_swap,
    compute_twap_price, update_twap, verify_invariant,
};
pub use transaction::{
    // Bond stacking transactions
    AddBondData,
    // Collateral/lending types
    CollateralMetadata,
    // Delegation transactions
    DelegateBondData,
    ExitData,
    Input,
    LendingDepositMetadata,
    Output,
    OutputType,
    // Pool types
    PoolMetadata,
    RegistrationData,
    RevokeDelegationData,
    SighashType,
    Transaction,
    TxType,
    WithdrawalRequestData,
    // Collateral/lending constants
    COLLATERAL_DEFAULT_INTEREST_BPS,
    COLLATERAL_DEFAULT_LIQUIDATION_BPS,
    COLLATERAL_MAX_INTEREST_BPS,
    COLLATERAL_MAX_LTV_BPS,
    COLLATERAL_METADATA_SIZE,
    COLLATERAL_MIN_LIQUIDATION_BPS,
    LENDING_DEPOSIT_METADATA_SIZE,
    LENDING_DEPOSIT_VERSION,
    LENDING_POOL_ID_DOMAIN,
    LOAN_ID_DOMAIN,
    MAX_EXTRA_DATA_SIZE,
    MAX_ROYALTY_BPS,
    NFT_METADATA_VERSION_ROYALTY,
    // Pool constants
    POOL_DEFAULT_FEE_BPS,
    POOL_ID_DOMAIN,
    POOL_MAX_FEE_BPS,
    POOL_METADATA_SIZE,
    POOL_VERSION,
};
pub use types::{coins_to_units, units_to_coins, Amount, BlockHeight, Epoch, Era, Slot};
pub use validation::{
    bootstrap_fallback_order, validate_block, validate_block_with_mode, validate_header,
    validate_producer_eligibility, validate_transaction, validate_transaction_with_utxos, UtxoInfo,
    UtxoProvider, ValidationContext, ValidationError, ValidationMode,
};

// TPoP (Temporal Proof of Presence) exports - telemetry module, not consensus
// Some constants are deprecated - use NetworkParams for network-aware code
#[allow(deprecated)]
pub use tpop::{
    calculate_heartbeat_score,
    // Functions
    calculate_presence_score,
    can_produce_at_time,
    rank_producers_by_presence,
    validate_heartbeat_timing,
    HeartbeatCollector,
    HeartbeatError,
    PresenceCheckpoint,
    // Legacy trait
    PresenceConsensus,
    // Heartbeat types (primary API for micro-VDF)
    PresenceHeartbeat,
    PresenceProof,
    ProducerPresenceState,
    // Simplified presence state
    SimplePresenceState,
    SimpleProducerState,
    SlotStats,
    // Configuration and metrics
    TpopConfig,
    TpopConsensus,
    TpopMetrics,
    TpopMigrationConfig,
    // Legacy core types
    VdfLink,
    HEARTBEAT_DEADLINE_SECS,
    HEARTBEAT_DISCRIMINANT_BITS,
    // Deprecated constants - use NetworkParams instead for network-aware code
    HEARTBEAT_GRACE_PERIOD_SECS,
    HEARTBEAT_VDF_ITERATIONS,
};

// Discovery module exports (producer discovery system)
pub use discovery::{
    decode_announcement, decode_digest, decode_producer_set, encode_announcement, encode_digest,
    encode_producer_set, is_legacy_bincode_format, AdaptiveGossip, EpochSnapshot, MergeResult,
    ProducerAnnouncement, ProducerBloomFilter, ProducerGSet, ProducerSetError, ProtoError,
    MAX_ANNOUNCEMENT_AGE_SECS, MAX_FUTURE_TIMESTAMP_SECS, PRODUCER_ANNOUNCEMENT_DOMAIN,
};

// Re-export the proto module for direct access to protobuf types
pub use discovery::proto;

/// Protocol version.
///
/// This version is included in block headers and used for
/// backwards compatibility checks during network upgrades.
pub const PROTOCOL_VERSION: u32 = 1;
