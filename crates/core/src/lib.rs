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

pub mod block;
pub mod consensus;
pub mod discovery;
pub mod genesis;
pub mod network;
pub mod tpop;
pub mod transaction;
pub mod types;
pub mod validation;

pub use block::{Block, BlockHeader};
pub use consensus::{
    allowed_producer_rank,
    allowed_producer_rank_ms,
    get_producer_rank,
    is_producer_eligible,
    select_producer_for_slot,
    withdrawal_penalty_rate,
    BondEntry,
    BondError,
    BondsMaturitySummary,
    ConsensusParams,
    PendingWithdrawal,
    // Producer state and scoring
    PresenceScore,
    ProducerBonds,
    ProducerState,
    BLOCK_REWARD_POOL,
    // Bond stacking system
    BOND_UNIT,
    ELIGIBLE_PRODUCER_POOL,
    EPOCH_REWARD_POOL,
    GENESIS_TIME,
    INITIAL_PRESENCE_SCORE,
    MAX_BONDS_PER_PRODUCER,
    MAX_FALLBACK_PRODUCERS,
    MAX_FUTURE_SLOTS,
    MAX_PAST_SLOTS,
    MAX_PRESENCE_SCORE,
    MIN_PRESENCE_RATE,
    MIN_PRESENCE_SCORE,
    // Producer window parameters (milliseconds for precision)
    PRIMARY_WINDOW_MS,
    PRIMARY_WINDOW_SECS,
    SCORE_MISS_PENALTY,
    SCORE_PRODUCE_BONUS,
    SECONDARY_WINDOW_MS,
    SECONDARY_WINDOW_SECS,
    SIGNATURE_WINDOW_MS,
    // Reward epoch constants
    SLOTS_PER_REWARD_EPOCH,
    // Proof of Time parameters
    SLOT_DURATION,
    TERTIARY_WINDOW_MS,
    TERTIARY_WINDOW_SECS,
    VDF_DISCRIMINANT_BITS,
    WITHDRAWAL_DELAY_SLOTS,
    YEAR_IN_SLOTS,
};
pub use genesis::{
    generate_genesis_block, genesis_hash, verify_genesis_block, GenesisConfig, GenesisError,
};
pub use network::Network;
pub use transaction::{
    // Bond stacking transactions
    AddBondData,
    ClaimWithdrawalData,
    ExitData,
    Input,
    Output,
    OutputType,
    RegistrationData,
    Transaction,
    TxType,
    WithdrawalRequestData,
};
pub use types::{coins_to_units, units_to_coins, Amount, BlockHeight, Epoch, Era, Slot};
pub use validation::{
    validate_block, validate_header, validate_producer_eligibility, validate_transaction,
    validate_transaction_with_utxos, UtxoInfo, UtxoProvider, ValidationContext, ValidationError,
};

// TPoP (Temporal Proof of Presence) exports - telemetry module, not consensus
pub use tpop::{
    calculate_heartbeat_score,
    // Functions
    calculate_presence_score,
    can_produce_at_time,
    rank_producers_by_presence,
    validate_heartbeat_timing,
    BootstrapPresenceProducer,
    HeartbeatCollector,
    HeartbeatError,
    PresenceCheckpoint,
    // Legacy trait
    PresenceConsensus,
    // Heartbeat types (primary API for micro-VDF)
    PresenceHeartbeat,
    PresenceMessage,
    PresenceMessageHandler,
    // Producer types
    PresenceProducer,
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
    HEARTBEAT_GRACE_PERIOD_SECS,
    HEARTBEAT_VDF_ITERATIONS,
};

// Discovery module exports (producer discovery system)
pub use discovery::{
    decode_announcement, decode_digest, decode_producer_set, encode_announcement, encode_digest,
    encode_producer_set, is_legacy_bincode_format, AdaptiveGossip, MergeResult,
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
