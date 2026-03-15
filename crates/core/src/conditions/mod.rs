//! Programmable Output Conditions
//!
//! Declarative spending conditions for UTXO outputs. Conditions are composable
//! predicates serialized into `extra_data` — no VM, no gas, no Turing-complete
//! scripting. Each condition resolves to a fixed number of cryptographic operations.
//!
//! # Design
//!
//! An output's spending conditions are encoded in its `extra_data` field.
//! When a transaction attempts to spend a conditioned output, the node decodes
//! the condition tree and evaluates it against the witness data provided in the
//! spending input. All evaluation is deterministic and bounded.
//!
//! # Encoding Format
//!
//! ```text
//! Byte 0:    Version (0x01)
//! Byte 1:    Condition tag
//! Bytes 2+:  Condition-specific payload (recursive for composites)
//! ```
//!
//! # Condition Types
//!
//! | Tag  | Condition          | Payload                                    |
//! |------|--------------------|--------------------------------------------|
//! | 0x00 | Signature          | 32B pubkey_hash                            |
//! | 0x01 | Multisig           | 1B threshold + 1B count + N×32B keys       |
//! | 0x02 | Hashlock           | 32B expected_hash                          |
//! | 0x03 | Timelock           | 8B min_height (LE)                         |
//! | 0x04 | TimelockExpiry     | 8B max_height (LE)                         |
//! | 0x10 | And                | [condition_a] [condition_b]                |
//! | 0x11 | Or                 | [condition_a] [condition_b]                |
//! | 0x12 | Threshold          | 1B n + 1B count + N×[condition]            |

mod encoding;
mod eval;
#[cfg(test)]
mod tests;
mod witness;

pub use eval::{evaluate, EvalContext};
pub use witness::{Witness, WitnessSignature, MAX_WITNESS_SIZE};

use crypto::hash::hash_with_domain;
use crypto::Hash;

use crate::types::BlockHeight;

// =============================================================================
// CONSTANTS
// =============================================================================

/// Current condition encoding version.
pub const CONDITION_VERSION: u8 = 1;

/// Maximum number of cryptographic operations in a single condition tree.
/// Prevents DoS via deeply nested or wide condition trees.
pub const MAX_CONDITION_OPS: usize = 20;

/// Maximum recursion depth for nested conditions (And/Or/Threshold).
pub const MAX_CONDITION_DEPTH: usize = 4;

/// Maximum number of keys in a multisig condition.
/// Limited by MAX_EXTRA_DATA_SIZE (256 bytes): 2B header + 2B params + N×32B = 256 → N=7.
pub const MAX_MULTISIG_KEYS: usize = 7;

/// Maximum number of sub-conditions in a Threshold.
pub const MAX_THRESHOLD_CONDITIONS: usize = 5;

/// Domain separator for hashlock preimage hashing.
pub const HASHLOCK_DOMAIN: &[u8] = b"DOLI_HASHLOCK";

/// Domain separator for address derivation (must match validation.rs).
pub const ADDRESS_DOMAIN: &[u8] = crypto::ADDRESS_DOMAIN;

// =============================================================================
// CONDITION TAGS
// =============================================================================

pub(crate) const TAG_SIGNATURE: u8 = 0x00;
pub(crate) const TAG_MULTISIG: u8 = 0x01;
pub(crate) const TAG_HASHLOCK: u8 = 0x02;
pub(crate) const TAG_TIMELOCK: u8 = 0x03;
pub(crate) const TAG_TIMELOCK_EXPIRY: u8 = 0x04;
pub(crate) const TAG_AND: u8 = 0x10;
pub(crate) const TAG_OR: u8 = 0x11;
pub(crate) const TAG_THRESHOLD: u8 = 0x12;

// =============================================================================
// CONDITION AST
// =============================================================================

/// A declarative spending condition.
///
/// Conditions are composable predicates: each returns true or false.
/// An output is spendable when all its conditions are satisfied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Condition {
    /// Requires a valid signature from the key matching this pubkey_hash.
    /// Default condition for Normal outputs (implicit when extra_data is empty).
    Signature(Hash),

    /// Requires threshold-of-N signatures from the given set of pubkey_hashes.
    Multisig { threshold: u8, keys: Vec<Hash> },

    /// Requires revealing a preimage whose BLAKE3 hash matches `expected_hash`.
    Hashlock(Hash),

    /// Spendable only at or after `min_height`.
    Timelock(BlockHeight),

    /// Spendable only at or before `max_height` (expires after).
    TimelockExpiry(BlockHeight),

    /// Both conditions must be satisfied.
    And(Box<Condition>, Box<Condition>),

    /// At least one condition must be satisfied.
    Or(Box<Condition>, Box<Condition>),

    /// At least `n` of the sub-conditions must be satisfied.
    Threshold { n: u8, conditions: Vec<Condition> },
}

// =============================================================================
// ERRORS
// =============================================================================

/// Errors that can occur during condition encoding, decoding, or evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConditionError {
    /// The extra_data is too short to contain a valid condition.
    BufferTooShort,
    /// Unsupported condition encoding version.
    UnsupportedVersion { version: u8 },
    /// Unknown condition tag byte.
    UnknownTag { tag: u8 },
    /// Multisig threshold exceeds key count.
    InvalidThreshold { threshold: u8, keys: u8 },
    /// Too many keys in multisig (exceeds MAX_MULTISIG_KEYS).
    TooManyKeys { count: u8 },
    /// Too many sub-conditions in Threshold (exceeds MAX_THRESHOLD_CONDITIONS).
    TooManyConditions { count: u8 },
    /// Threshold N exceeds condition count.
    ThresholdExceedsCount { n: u8, count: u8 },
    /// Condition tree exceeds MAX_CONDITION_OPS.
    TooManyOperations { ops: usize },
    /// Condition tree exceeds MAX_CONDITION_DEPTH.
    TooDeep { depth: usize },
    /// Encoded condition exceeds MAX_EXTRA_DATA_SIZE.
    EncodingTooLarge { size: usize },
    /// Unexpected trailing bytes after decoding.
    TrailingBytes { remaining: usize },
    /// Zero threshold in multisig or threshold condition.
    ZeroThreshold,
    /// HTLC lock height must be less than expiry height.
    InvalidTimelockRange {
        lock: crate::types::BlockHeight,
        expiry: crate::types::BlockHeight,
    },
    /// Invalid public key bytes in witness data.
    InvalidPublicKey,
}

impl std::fmt::Display for ConditionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BufferTooShort => write!(f, "buffer too short for condition"),
            Self::UnsupportedVersion { version } => {
                write!(f, "unsupported condition version: {}", version)
            }
            Self::UnknownTag { tag } => write!(f, "unknown condition tag: 0x{:02x}", tag),
            Self::InvalidThreshold { threshold, keys } => {
                write!(f, "threshold {} exceeds key count {}", threshold, keys)
            }
            Self::TooManyKeys { count } => {
                write!(
                    f,
                    "too many multisig keys: {} (max {})",
                    count, MAX_MULTISIG_KEYS
                )
            }
            Self::TooManyConditions { count } => write!(
                f,
                "too many threshold conditions: {} (max {})",
                count, MAX_THRESHOLD_CONDITIONS
            ),
            Self::ThresholdExceedsCount { n, count } => {
                write!(f, "threshold {} exceeds condition count {}", n, count)
            }
            Self::TooManyOperations { ops } => {
                write!(f, "condition has {} ops (max {})", ops, MAX_CONDITION_OPS)
            }
            Self::TooDeep { depth } => {
                write!(
                    f,
                    "condition depth {} exceeds max {}",
                    depth, MAX_CONDITION_DEPTH
                )
            }
            Self::EncodingTooLarge { size } => {
                write!(f, "encoded condition {} bytes exceeds max 256", size)
            }
            Self::TrailingBytes { remaining } => {
                write!(f, "unexpected {} trailing bytes after condition", remaining)
            }
            Self::ZeroThreshold => write!(f, "threshold must be at least 1"),
            Self::InvalidTimelockRange { lock, expiry } => {
                write!(
                    f,
                    "lock height {} must be less than expiry height {}",
                    lock, expiry
                )
            }
            Self::InvalidPublicKey => write!(f, "invalid public key bytes in witness"),
        }
    }
}

impl std::error::Error for ConditionError {}

// =============================================================================
// CONVENIENCE CONSTRUCTORS
// =============================================================================

impl Condition {
    /// Create a simple signature condition.
    pub fn signature(pubkey_hash: Hash) -> Self {
        Condition::Signature(pubkey_hash)
    }

    /// Create a multisig condition.
    pub fn multisig(threshold: u8, keys: Vec<Hash>) -> Self {
        Condition::Multisig { threshold, keys }
    }

    /// Create a hashlock condition from a preimage hash.
    pub fn hashlock(expected_hash: Hash) -> Self {
        Condition::Hashlock(expected_hash)
    }

    /// Create a hashlock condition by hashing the preimage.
    pub fn hashlock_from_preimage(preimage: &[u8; 32]) -> Self {
        let hash = hash_with_domain(HASHLOCK_DOMAIN, preimage);
        Condition::Hashlock(hash)
    }

    /// Create a timelock condition (spendable at or after min_height).
    pub fn timelock(min_height: BlockHeight) -> Self {
        Condition::Timelock(min_height)
    }

    /// Create a timelock expiry condition (spendable at or after max_height).
    pub fn timelock_expiry(max_height: BlockHeight) -> Self {
        Condition::TimelockExpiry(max_height)
    }

    /// Create an HTLC: (Hashlock AND Timelock) OR TimelockExpiry.
    /// The receiver can claim with the preimage after lock_height.
    /// The sender can reclaim after expiry_height.
    pub fn htlc(expected_hash: Hash, lock_height: BlockHeight, expiry_height: BlockHeight) -> Self {
        Condition::Or(
            Box::new(Condition::And(
                Box::new(Condition::Hashlock(expected_hash)),
                Box::new(Condition::Timelock(lock_height)),
            )),
            Box::new(Condition::TimelockExpiry(expiry_height)),
        )
    }

    /// Create a vesting condition: signature + timelock.
    pub fn vesting(pubkey_hash: Hash, unlock_height: BlockHeight) -> Self {
        Condition::And(
            Box::new(Condition::Signature(pubkey_hash)),
            Box::new(Condition::Timelock(unlock_height)),
        )
    }
}
