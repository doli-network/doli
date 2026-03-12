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

use crypto::hash::hash_with_domain;
use crypto::signature::verify_hash;
use crypto::{Hash, PublicKey, Signature};

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

const TAG_SIGNATURE: u8 = 0x00;
const TAG_MULTISIG: u8 = 0x01;
const TAG_HASHLOCK: u8 = 0x02;
const TAG_TIMELOCK: u8 = 0x03;
const TAG_TIMELOCK_EXPIRY: u8 = 0x04;
const TAG_AND: u8 = 0x10;
const TAG_OR: u8 = 0x11;
const TAG_THRESHOLD: u8 = 0x12;

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
        }
    }
}

impl std::error::Error for ConditionError {}

// =============================================================================
// ENCODING
// =============================================================================

impl Condition {
    /// Encode this condition into extra_data bytes (with version prefix).
    pub fn encode(&self) -> Result<Vec<u8>, ConditionError> {
        // Validate before encoding
        self.validate()?;

        let mut buf = Vec::with_capacity(64);
        buf.push(CONDITION_VERSION);
        self.encode_inner(&mut buf);

        if buf.len() > crate::transaction::MAX_EXTRA_DATA_SIZE {
            return Err(ConditionError::EncodingTooLarge { size: buf.len() });
        }
        Ok(buf)
    }

    /// Encode condition body (without version prefix) — recursive.
    fn encode_inner(&self, buf: &mut Vec<u8>) {
        match self {
            Condition::Signature(pkh) => {
                buf.push(TAG_SIGNATURE);
                buf.extend_from_slice(pkh.as_bytes());
            }
            Condition::Multisig { threshold, keys } => {
                buf.push(TAG_MULTISIG);
                buf.push(*threshold);
                buf.push(keys.len() as u8);
                for key in keys {
                    buf.extend_from_slice(key.as_bytes());
                }
            }
            Condition::Hashlock(expected) => {
                buf.push(TAG_HASHLOCK);
                buf.extend_from_slice(expected.as_bytes());
            }
            Condition::Timelock(height) => {
                buf.push(TAG_TIMELOCK);
                buf.extend_from_slice(&height.to_le_bytes());
            }
            Condition::TimelockExpiry(height) => {
                buf.push(TAG_TIMELOCK_EXPIRY);
                buf.extend_from_slice(&height.to_le_bytes());
            }
            Condition::And(a, b) => {
                buf.push(TAG_AND);
                a.encode_inner(buf);
                b.encode_inner(buf);
            }
            Condition::Or(a, b) => {
                buf.push(TAG_OR);
                a.encode_inner(buf);
                b.encode_inner(buf);
            }
            Condition::Threshold { n, conditions } => {
                buf.push(TAG_THRESHOLD);
                buf.push(*n);
                buf.push(conditions.len() as u8);
                for cond in conditions {
                    cond.encode_inner(buf);
                }
            }
        }
    }

    /// Decode a condition from extra_data bytes (with version prefix).
    pub fn decode(bytes: &[u8]) -> Result<Self, ConditionError> {
        if bytes.is_empty() {
            return Err(ConditionError::BufferTooShort);
        }

        let version = bytes[0];
        if version != CONDITION_VERSION {
            return Err(ConditionError::UnsupportedVersion { version });
        }

        let (cond, consumed) = Self::decode_inner(&bytes[1..], 0)?;

        let remaining = bytes.len() - 1 - consumed;
        if remaining > 0 {
            return Err(ConditionError::TrailingBytes { remaining });
        }

        // Validate the decoded condition
        cond.validate()?;

        Ok(cond)
    }

    /// Decode a condition from the start of a byte slice, allowing trailing data.
    /// Returns (condition, total_bytes_consumed) including the version prefix.
    /// Use this for output types that append metadata after the condition (NFT, FungibleAsset, BridgeHTLC).
    pub fn decode_prefix(bytes: &[u8]) -> Result<(Self, usize), ConditionError> {
        if bytes.is_empty() {
            return Err(ConditionError::BufferTooShort);
        }

        let version = bytes[0];
        if version != CONDITION_VERSION {
            return Err(ConditionError::UnsupportedVersion { version });
        }

        let (cond, consumed) = Self::decode_inner(&bytes[1..], 0)?;
        cond.validate()?;
        Ok((cond, 1 + consumed))
    }

    /// Decode condition body (without version prefix) — recursive.
    /// Returns (condition, bytes_consumed).
    fn decode_inner(bytes: &[u8], depth: usize) -> Result<(Self, usize), ConditionError> {
        if depth > MAX_CONDITION_DEPTH {
            return Err(ConditionError::TooDeep { depth });
        }
        if bytes.is_empty() {
            return Err(ConditionError::BufferTooShort);
        }

        let tag = bytes[0];
        let rest = &bytes[1..];

        match tag {
            TAG_SIGNATURE => {
                if rest.len() < 32 {
                    return Err(ConditionError::BufferTooShort);
                }
                let hash = Hash::from_bytes(rest[..32].try_into().unwrap());
                Ok((Condition::Signature(hash), 1 + 32))
            }

            TAG_MULTISIG => {
                if rest.len() < 2 {
                    return Err(ConditionError::BufferTooShort);
                }
                let threshold = rest[0];
                let count = rest[1];
                if threshold == 0 {
                    return Err(ConditionError::ZeroThreshold);
                }
                if threshold > count {
                    return Err(ConditionError::InvalidThreshold {
                        threshold,
                        keys: count,
                    });
                }
                if count as usize > MAX_MULTISIG_KEYS {
                    return Err(ConditionError::TooManyKeys { count });
                }
                let keys_start = 2;
                let keys_end = keys_start + (count as usize) * 32;
                if rest.len() < keys_end {
                    return Err(ConditionError::BufferTooShort);
                }
                let mut keys = Vec::with_capacity(count as usize);
                for i in 0..count as usize {
                    let offset = keys_start + i * 32;
                    let hash = Hash::from_bytes(rest[offset..offset + 32].try_into().unwrap());
                    keys.push(hash);
                }
                Ok((Condition::Multisig { threshold, keys }, 1 + keys_end))
            }

            TAG_HASHLOCK => {
                if rest.len() < 32 {
                    return Err(ConditionError::BufferTooShort);
                }
                let hash = Hash::from_bytes(rest[..32].try_into().unwrap());
                Ok((Condition::Hashlock(hash), 1 + 32))
            }

            TAG_TIMELOCK => {
                if rest.len() < 8 {
                    return Err(ConditionError::BufferTooShort);
                }
                let height = u64::from_le_bytes(rest[..8].try_into().unwrap());
                Ok((Condition::Timelock(height), 1 + 8))
            }

            TAG_TIMELOCK_EXPIRY => {
                if rest.len() < 8 {
                    return Err(ConditionError::BufferTooShort);
                }
                let height = u64::from_le_bytes(rest[..8].try_into().unwrap());
                Ok((Condition::TimelockExpiry(height), 1 + 8))
            }

            TAG_AND => {
                let (a, consumed_a) = Self::decode_inner(rest, depth + 1)?;
                let (b, consumed_b) = Self::decode_inner(&rest[consumed_a..], depth + 1)?;
                Ok((
                    Condition::And(Box::new(a), Box::new(b)),
                    1 + consumed_a + consumed_b,
                ))
            }

            TAG_OR => {
                let (a, consumed_a) = Self::decode_inner(rest, depth + 1)?;
                let (b, consumed_b) = Self::decode_inner(&rest[consumed_a..], depth + 1)?;
                Ok((
                    Condition::Or(Box::new(a), Box::new(b)),
                    1 + consumed_a + consumed_b,
                ))
            }

            TAG_THRESHOLD => {
                if rest.len() < 2 {
                    return Err(ConditionError::BufferTooShort);
                }
                let n = rest[0];
                let count = rest[1];
                if n == 0 {
                    return Err(ConditionError::ZeroThreshold);
                }
                if n > count {
                    return Err(ConditionError::ThresholdExceedsCount { n, count });
                }
                if count as usize > MAX_THRESHOLD_CONDITIONS {
                    return Err(ConditionError::TooManyConditions { count });
                }
                let mut conditions = Vec::with_capacity(count as usize);
                let mut offset = 2;
                for _ in 0..count {
                    let (cond, consumed) = Self::decode_inner(&rest[offset..], depth + 1)?;
                    offset += consumed;
                    conditions.push(cond);
                }
                Ok((Condition::Threshold { n, conditions }, 1 + offset))
            }

            _ => Err(ConditionError::UnknownTag { tag }),
        }
    }

    /// Count the number of cryptographic operations (sig verifications + hash checks).
    pub fn ops_count(&self) -> usize {
        match self {
            Condition::Signature(_) => 1,
            Condition::Multisig { keys, .. } => keys.len(),
            Condition::Hashlock(_) => 1,
            Condition::Timelock(_) | Condition::TimelockExpiry(_) => 0,
            Condition::And(a, b) | Condition::Or(a, b) => a.ops_count() + b.ops_count(),
            Condition::Threshold { conditions, .. } => {
                conditions.iter().map(|c| c.ops_count()).sum()
            }
        }
    }

    /// Calculate the maximum depth of the condition tree.
    fn depth(&self) -> usize {
        match self {
            Condition::Signature(_)
            | Condition::Multisig { .. }
            | Condition::Hashlock(_)
            | Condition::Timelock(_)
            | Condition::TimelockExpiry(_) => 0,
            Condition::And(a, b) | Condition::Or(a, b) => 1 + a.depth().max(b.depth()),
            Condition::Threshold { conditions, .. } => {
                1 + conditions.iter().map(|c| c.depth()).max().unwrap_or(0)
            }
        }
    }

    /// Validate the condition tree (ops count, depth, structural integrity).
    pub fn validate(&self) -> Result<(), ConditionError> {
        let ops = self.ops_count();
        if ops > MAX_CONDITION_OPS {
            return Err(ConditionError::TooManyOperations { ops });
        }
        let depth = self.depth();
        if depth > MAX_CONDITION_DEPTH {
            return Err(ConditionError::TooDeep { depth });
        }
        // Structural validation for specific condition types
        self.validate_structure()
    }

    /// Validate structural invariants recursively.
    fn validate_structure(&self) -> Result<(), ConditionError> {
        match self {
            Condition::Multisig { threshold, keys } => {
                if *threshold == 0 {
                    return Err(ConditionError::ZeroThreshold);
                }
                if *threshold > keys.len() as u8 {
                    return Err(ConditionError::InvalidThreshold {
                        threshold: *threshold,
                        keys: keys.len() as u8,
                    });
                }
                if keys.len() > MAX_MULTISIG_KEYS {
                    return Err(ConditionError::TooManyKeys {
                        count: keys.len() as u8,
                    });
                }
                Ok(())
            }
            Condition::Threshold { n, conditions } => {
                if *n == 0 {
                    return Err(ConditionError::ZeroThreshold);
                }
                if *n > conditions.len() as u8 {
                    return Err(ConditionError::ThresholdExceedsCount {
                        n: *n,
                        count: conditions.len() as u8,
                    });
                }
                if conditions.len() > MAX_THRESHOLD_CONDITIONS {
                    return Err(ConditionError::TooManyConditions {
                        count: conditions.len() as u8,
                    });
                }
                for cond in conditions {
                    cond.validate_structure()?;
                }
                Ok(())
            }
            Condition::And(a, b) | Condition::Or(a, b) => {
                a.validate_structure()?;
                b.validate_structure()
            }
            _ => Ok(()),
        }
    }
}

// =============================================================================
// WITNESS DATA
// =============================================================================

/// Witness data for spending a conditioned output.
///
/// The witness is provided by the spender to satisfy the output's conditions.
/// It contains signatures, preimages, and branch choices for composite conditions.
#[derive(Clone, Debug, Default)]
pub struct Witness {
    /// Signatures indexed by pubkey_hash position in the condition.
    /// For single-sig: one entry. For multisig: threshold entries.
    pub signatures: Vec<WitnessSignature>,
    /// Preimage for hashlock conditions (32 bytes).
    pub preimage: Option<[u8; 32]>,
    /// Branch selection for Or conditions (false=left, true=right).
    pub or_branches: Vec<bool>,
}

/// A signature paired with the signer's public key.
#[derive(Clone, Debug)]
pub struct WitnessSignature {
    pub pubkey: PublicKey,
    pub signature: Signature,
}

// =============================================================================
// WITNESS ENCODING
// =============================================================================

/// Witness encoding tags.
const WITNESS_VERSION: u8 = 1;
const WITNESS_TAG_SIGS: u8 = 0x01;
const WITNESS_TAG_PREIMAGE: u8 = 0x02;
const WITNESS_TAG_BRANCHES: u8 = 0x03;

/// Maximum witness size (signatures + preimage + overhead).
/// 7 sigs × (32B pubkey + 64B sig) + 32B preimage + overhead ≈ 750 bytes.
pub const MAX_WITNESS_SIZE: usize = 1024;

impl Witness {
    /// Encode witness data into bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(128);
        buf.push(WITNESS_VERSION);

        // Signatures section
        if !self.signatures.is_empty() {
            buf.push(WITNESS_TAG_SIGS);
            buf.push(self.signatures.len() as u8);
            for ws in &self.signatures {
                buf.extend_from_slice(ws.pubkey.as_bytes());
                buf.extend_from_slice(ws.signature.as_bytes());
            }
        }

        // Preimage section
        if let Some(preimage) = &self.preimage {
            buf.push(WITNESS_TAG_PREIMAGE);
            buf.extend_from_slice(preimage);
        }

        // Or-branch selections
        if !self.or_branches.is_empty() {
            buf.push(WITNESS_TAG_BRANCHES);
            buf.push(self.or_branches.len() as u8);
            for &branch in &self.or_branches {
                buf.push(if branch { 1 } else { 0 });
            }
        }

        buf
    }

    /// Decode witness data from bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, ConditionError> {
        if bytes.is_empty() {
            // Empty witness is valid (for Normal/Bond outputs)
            return Ok(Self::default());
        }

        if bytes[0] != WITNESS_VERSION {
            return Err(ConditionError::UnsupportedVersion { version: bytes[0] });
        }

        let mut witness = Witness::default();
        let mut pos = 1;

        while pos < bytes.len() {
            let tag = bytes[pos];
            pos += 1;

            match tag {
                WITNESS_TAG_SIGS => {
                    if pos >= bytes.len() {
                        return Err(ConditionError::BufferTooShort);
                    }
                    let count = bytes[pos] as usize;
                    pos += 1;
                    for _ in 0..count {
                        if pos + 32 + 64 > bytes.len() {
                            return Err(ConditionError::BufferTooShort);
                        }
                        let pubkey_bytes: [u8; 32] = bytes[pos..pos + 32].try_into().unwrap();
                        pos += 32;
                        let sig_bytes: [u8; 64] = bytes[pos..pos + 64].try_into().unwrap();
                        pos += 64;
                        witness.signatures.push(WitnessSignature {
                            pubkey: PublicKey::from_bytes(pubkey_bytes),
                            signature: Signature::from_bytes(sig_bytes),
                        });
                    }
                }

                WITNESS_TAG_PREIMAGE => {
                    if pos + 32 > bytes.len() {
                        return Err(ConditionError::BufferTooShort);
                    }
                    let mut preimage = [0u8; 32];
                    preimage.copy_from_slice(&bytes[pos..pos + 32]);
                    witness.preimage = Some(preimage);
                    pos += 32;
                }

                WITNESS_TAG_BRANCHES => {
                    if pos >= bytes.len() {
                        return Err(ConditionError::BufferTooShort);
                    }
                    let count = bytes[pos] as usize;
                    pos += 1;
                    if pos + count > bytes.len() {
                        return Err(ConditionError::BufferTooShort);
                    }
                    for i in 0..count {
                        witness.or_branches.push(bytes[pos + i] != 0);
                    }
                    pos += count;
                }

                _ => {
                    // Unknown tag — skip to end (forward compat)
                    break;
                }
            }
        }

        Ok(witness)
    }
}

// =============================================================================
// EVALUATION
// =============================================================================

/// Context provided to the condition evaluator.
pub struct EvalContext<'a> {
    /// Current chain height at which the transaction is being validated.
    pub current_height: BlockHeight,
    /// The transaction's signing hash (for signature verification).
    pub signing_hash: &'a Hash,
}

/// Evaluate a condition tree against witness data.
///
/// Returns `true` if the condition is satisfied, `false` otherwise.
/// The witness's `or_branch_index` is used internally to track Or-branch consumption.
pub fn evaluate(
    condition: &Condition,
    witness: &Witness,
    ctx: &EvalContext,
    or_branch_idx: &mut usize,
) -> bool {
    match condition {
        Condition::Signature(expected_pkh) => {
            // Find a witness signature whose pubkey hashes to expected_pkh
            for ws in &witness.signatures {
                let actual_hash = hash_with_domain(ADDRESS_DOMAIN, ws.pubkey.as_bytes());
                if actual_hash.as_bytes()[..20] == expected_pkh.as_bytes()[..20]
                    && verify_hash(ctx.signing_hash, &ws.signature, &ws.pubkey).is_ok()
                {
                    return true;
                }
            }
            false
        }

        Condition::Multisig { threshold, keys } => {
            let mut satisfied = 0u8;
            for expected_pkh in keys {
                for ws in &witness.signatures {
                    let actual_hash = hash_with_domain(ADDRESS_DOMAIN, ws.pubkey.as_bytes());
                    if actual_hash.as_bytes()[..20] == expected_pkh.as_bytes()[..20]
                        && verify_hash(ctx.signing_hash, &ws.signature, &ws.pubkey).is_ok()
                    {
                        satisfied += 1;
                        break;
                    }
                }
                if satisfied >= *threshold {
                    return true;
                }
            }
            false
        }

        Condition::Hashlock(expected_hash) => {
            if let Some(preimage) = &witness.preimage {
                let computed = hash_with_domain(HASHLOCK_DOMAIN, preimage);
                computed == *expected_hash
            } else {
                false
            }
        }

        Condition::Timelock(min_height) => ctx.current_height >= *min_height,

        Condition::TimelockExpiry(max_height) => ctx.current_height >= *max_height,

        Condition::And(a, b) => {
            evaluate(a, witness, ctx, or_branch_idx) && evaluate(b, witness, ctx, or_branch_idx)
        }

        Condition::Or(a, b) => {
            // Use the next or_branch selection from witness to choose which branch
            let branch = if *or_branch_idx < witness.or_branches.len() {
                let b = witness.or_branches[*or_branch_idx];
                *or_branch_idx += 1;
                b
            } else {
                // No branch hint — try left first, then right
                if evaluate(a, witness, ctx, or_branch_idx) {
                    return true;
                }
                return evaluate(b, witness, ctx, or_branch_idx);
            };

            if branch {
                evaluate(b, witness, ctx, or_branch_idx)
            } else {
                evaluate(a, witness, ctx, or_branch_idx)
            }
        }

        Condition::Threshold { n, conditions } => {
            let mut satisfied = 0u8;
            for cond in conditions {
                if evaluate(cond, witness, ctx, or_branch_idx) {
                    satisfied += 1;
                    if satisfied >= *n {
                        return true;
                    }
                }
            }
            false
        }
    }
}

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

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_hash(byte: u8) -> Hash {
        Hash::from_bytes([byte; 32])
    }

    // ---- Encoding / Decoding roundtrips ----

    #[test]
    fn test_signature_roundtrip() {
        let cond = Condition::signature(dummy_hash(0xAA));
        let encoded = cond.encode().unwrap();
        let decoded = Condition::decode(&encoded).unwrap();
        assert_eq!(cond, decoded);
        assert_eq!(encoded.len(), 1 + 1 + 32); // version + tag + hash
    }

    #[test]
    fn test_multisig_roundtrip() {
        let cond = Condition::multisig(2, vec![dummy_hash(1), dummy_hash(2), dummy_hash(3)]);
        let encoded = cond.encode().unwrap();
        let decoded = Condition::decode(&encoded).unwrap();
        assert_eq!(cond, decoded);
        assert_eq!(encoded.len(), 1 + 1 + 2 + 3 * 32); // version + tag + params + keys
    }

    #[test]
    fn test_hashlock_roundtrip() {
        let cond = Condition::hashlock(dummy_hash(0xBB));
        let encoded = cond.encode().unwrap();
        let decoded = Condition::decode(&encoded).unwrap();
        assert_eq!(cond, decoded);
    }

    #[test]
    fn test_timelock_roundtrip() {
        let cond = Condition::timelock(50_000);
        let encoded = cond.encode().unwrap();
        let decoded = Condition::decode(&encoded).unwrap();
        assert_eq!(cond, decoded);
    }

    #[test]
    fn test_timelock_expiry_roundtrip() {
        let cond = Condition::timelock_expiry(100_000);
        let encoded = cond.encode().unwrap();
        let decoded = Condition::decode(&encoded).unwrap();
        assert_eq!(cond, decoded);
    }

    #[test]
    fn test_and_roundtrip() {
        let cond = Condition::And(
            Box::new(Condition::signature(dummy_hash(1))),
            Box::new(Condition::timelock(1000)),
        );
        let encoded = cond.encode().unwrap();
        let decoded = Condition::decode(&encoded).unwrap();
        assert_eq!(cond, decoded);
    }

    #[test]
    fn test_or_roundtrip() {
        let cond = Condition::Or(
            Box::new(Condition::hashlock(dummy_hash(0xCC))),
            Box::new(Condition::timelock_expiry(5000)),
        );
        let encoded = cond.encode().unwrap();
        let decoded = Condition::decode(&encoded).unwrap();
        assert_eq!(cond, decoded);
    }

    #[test]
    fn test_htlc_roundtrip() {
        let cond = Condition::htlc(dummy_hash(0xDD), 1000, 2000);
        let encoded = cond.encode().unwrap();
        let decoded = Condition::decode(&encoded).unwrap();
        assert_eq!(cond, decoded);
    }

    #[test]
    fn test_threshold_roundtrip() {
        let cond = Condition::Threshold {
            n: 2,
            conditions: vec![
                Condition::signature(dummy_hash(1)),
                Condition::signature(dummy_hash(2)),
                Condition::signature(dummy_hash(3)),
            ],
        };
        let encoded = cond.encode().unwrap();
        let decoded = Condition::decode(&encoded).unwrap();
        assert_eq!(cond, decoded);
    }

    #[test]
    fn test_vesting_roundtrip() {
        let cond = Condition::vesting(dummy_hash(0xEE), 50_000);
        let encoded = cond.encode().unwrap();
        let decoded = Condition::decode(&encoded).unwrap();
        assert_eq!(cond, decoded);
    }

    // ---- Validation errors ----

    #[test]
    fn test_multisig_threshold_exceeds_keys() {
        let cond = Condition::Multisig {
            threshold: 3,
            keys: vec![dummy_hash(1), dummy_hash(2)],
        };
        assert!(matches!(
            cond.encode(),
            Err(ConditionError::InvalidThreshold { .. })
        ));
    }

    #[test]
    fn test_multisig_zero_threshold() {
        let cond = Condition::Multisig {
            threshold: 0,
            keys: vec![dummy_hash(1)],
        };
        assert!(matches!(cond.encode(), Err(ConditionError::ZeroThreshold)));
    }

    #[test]
    fn test_too_deep() {
        // Build depth-5 nesting: And(And(And(And(And(sig, sig), sig), sig), sig)
        let mut cond = Condition::signature(dummy_hash(1));
        for _ in 0..5 {
            cond = Condition::And(
                Box::new(cond),
                Box::new(Condition::signature(dummy_hash(2))),
            );
        }
        assert!(matches!(cond.encode(), Err(ConditionError::TooDeep { .. })));
    }

    #[test]
    fn test_max_depth_ok() {
        // depth=4 should be fine
        let mut cond = Condition::signature(dummy_hash(1));
        for _ in 0..4 {
            cond = Condition::And(
                Box::new(cond),
                Box::new(Condition::signature(dummy_hash(2))),
            );
        }
        assert!(cond.encode().is_ok());
    }

    #[test]
    fn test_decode_bad_version() {
        let bytes = vec![0xFF, TAG_SIGNATURE];
        assert!(matches!(
            Condition::decode(&bytes),
            Err(ConditionError::UnsupportedVersion { version: 0xFF })
        ));
    }

    #[test]
    fn test_decode_unknown_tag() {
        let bytes = vec![CONDITION_VERSION, 0xFF];
        assert!(matches!(
            Condition::decode(&bytes),
            Err(ConditionError::UnknownTag { tag: 0xFF })
        ));
    }

    #[test]
    fn test_decode_truncated() {
        let bytes = vec![CONDITION_VERSION, TAG_SIGNATURE, 0x00]; // only 1 byte of 32
        assert!(matches!(
            Condition::decode(&bytes),
            Err(ConditionError::BufferTooShort)
        ));
    }

    #[test]
    fn test_trailing_bytes_rejected() {
        let cond = Condition::timelock(100);
        let mut encoded = cond.encode().unwrap();
        encoded.push(0xFF); // garbage
        assert!(matches!(
            Condition::decode(&encoded),
            Err(ConditionError::TrailingBytes { .. })
        ));
    }

    // ---- Ops count ----

    #[test]
    fn test_ops_count() {
        assert_eq!(Condition::signature(dummy_hash(1)).ops_count(), 1);
        assert_eq!(Condition::timelock(100).ops_count(), 0);
        assert_eq!(
            Condition::multisig(2, vec![dummy_hash(1), dummy_hash(2), dummy_hash(3)]).ops_count(),
            3
        );
        let htlc = Condition::htlc(dummy_hash(1), 100, 200);
        assert_eq!(htlc.ops_count(), 1); // hashlock=1, timelocks=0
    }

    // ---- Witness encoding/decoding ----

    #[test]
    fn test_witness_empty_roundtrip() {
        let w = Witness::default();
        let encoded = w.encode();
        let decoded = Witness::decode(&encoded).unwrap();
        assert!(decoded.signatures.is_empty());
        assert!(decoded.preimage.is_none());
        assert!(decoded.or_branches.is_empty());
    }

    #[test]
    fn test_witness_preimage_roundtrip() {
        let w = Witness {
            preimage: Some([0xAB; 32]),
            ..Default::default()
        };
        let encoded = w.encode();
        let decoded = Witness::decode(&encoded).unwrap();
        assert_eq!(decoded.preimage, Some([0xAB; 32]));
    }

    #[test]
    fn test_witness_or_branches_roundtrip() {
        let w = Witness {
            or_branches: vec![true, false, true],
            ..Default::default()
        };
        let encoded = w.encode();
        let decoded = Witness::decode(&encoded).unwrap();
        assert_eq!(decoded.or_branches, vec![true, false, true]);
    }

    // ---- Evaluation ----

    #[test]
    fn test_eval_timelock_satisfied() {
        let cond = Condition::timelock(100);
        let hash = dummy_hash(0);
        let ctx = EvalContext {
            current_height: 100,
            signing_hash: &hash,
        };
        assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));
    }

    #[test]
    fn test_eval_timelock_not_satisfied() {
        let cond = Condition::timelock(100);
        let hash = dummy_hash(0);
        let ctx = EvalContext {
            current_height: 99,
            signing_hash: &hash,
        };
        assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
    }

    #[test]
    fn test_eval_timelock_expiry_satisfied() {
        let cond = Condition::timelock_expiry(100);
        let hash = dummy_hash(0);
        let ctx = EvalContext {
            current_height: 100,
            signing_hash: &hash,
        };
        assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));
    }

    #[test]
    fn test_eval_timelock_expiry_not_satisfied() {
        // TimelockExpiry(100) means spendable at height >= 100
        // At height 99, should NOT be satisfied
        let cond = Condition::timelock_expiry(100);
        let hash = dummy_hash(0);
        let ctx = EvalContext {
            current_height: 99,
            signing_hash: &hash,
        };
        assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
    }

    #[test]
    fn test_eval_hashlock() {
        let preimage = [0x42u8; 32];
        let expected = hash_with_domain(HASHLOCK_DOMAIN, &preimage);
        let cond = Condition::Hashlock(expected);

        let hash = dummy_hash(0);
        let ctx = EvalContext {
            current_height: 0,
            signing_hash: &hash,
        };

        // With correct preimage
        let w = Witness {
            preimage: Some(preimage),
            ..Default::default()
        };
        assert!(evaluate(&cond, &w, &ctx, &mut 0));

        // With wrong preimage
        let w_bad = Witness {
            preimage: Some([0x43u8; 32]),
            ..Default::default()
        };
        assert!(!evaluate(&cond, &w_bad, &ctx, &mut 0));

        // With no preimage
        assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
    }

    #[test]
    fn test_eval_and() {
        // And(Timelock(50), TimelockExpiry(200)):
        // Timelock(50): height >= 50
        // TimelockExpiry(200): height >= 200
        // Combined: height >= 200 (both must be satisfied)
        let cond = Condition::And(
            Box::new(Condition::timelock(50)),
            Box::new(Condition::timelock_expiry(200)),
        );
        let hash = dummy_hash(0);

        // Height 30: timelock not met
        let ctx = EvalContext {
            current_height: 30,
            signing_hash: &hash,
        };
        assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));

        // Height 100: timelock met but expiry not yet reached
        let ctx = EvalContext {
            current_height: 100,
            signing_hash: &hash,
        };
        assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));

        // Height 200: both satisfied
        let ctx = EvalContext {
            current_height: 200,
            signing_hash: &hash,
        };
        assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));

        // Height 300: both satisfied
        let ctx = EvalContext {
            current_height: 300,
            signing_hash: &hash,
        };
        assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));
    }

    #[test]
    fn test_eval_or_with_branch_hint() {
        let cond = Condition::Or(
            Box::new(Condition::timelock(1000)), // left: not met at h=50
            Box::new(Condition::timelock(10)),   // right: met at h=50
        );
        let hash = dummy_hash(0);
        let ctx = EvalContext {
            current_height: 50,
            signing_hash: &hash,
        };

        // Branch hint: right (true)
        let w = Witness {
            or_branches: vec![true],
            ..Default::default()
        };
        assert!(evaluate(&cond, &w, &ctx, &mut 0));

        // Branch hint: left (false) — will fail
        let w = Witness {
            or_branches: vec![false],
            ..Default::default()
        };
        assert!(!evaluate(&cond, &w, &ctx, &mut 0));
    }

    #[test]
    fn test_eval_or_without_hint_tries_both() {
        let cond = Condition::Or(
            Box::new(Condition::timelock(1000)), // left: not met
            Box::new(Condition::timelock(10)),   // right: met
        );
        let hash = dummy_hash(0);
        let ctx = EvalContext {
            current_height: 50,
            signing_hash: &hash,
        };

        // No branch hints — should try left (fail) then right (succeed)
        assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));
    }

    #[test]
    fn test_eval_threshold() {
        let cond = Condition::Threshold {
            n: 2,
            conditions: vec![
                Condition::timelock(100), // met at h=150
                Condition::timelock(200), // not met at h=150
                Condition::timelock(50),  // met at h=150
            ],
        };
        let hash = dummy_hash(0);

        // Height 150: conditions[0] and conditions[2] satisfied → 2 >= 2
        let ctx = EvalContext {
            current_height: 150,
            signing_hash: &hash,
        };
        assert!(evaluate(&cond, &Witness::default(), &ctx, &mut 0));

        // Height 80: only conditions[2] satisfied → 1 < 2
        let ctx = EvalContext {
            current_height: 80,
            signing_hash: &hash,
        };
        assert!(!evaluate(&cond, &Witness::default(), &ctx, &mut 0));
    }

    // ---- Multisig validation invariants ----

    #[test]
    fn test_multisig_validate_errors() {
        // Threshold > keys
        let cond = Condition::Multisig {
            threshold: 4,
            keys: vec![dummy_hash(1), dummy_hash(2)],
        };
        assert!(cond.validate().is_err());

        // Zero threshold
        let cond = Condition::Multisig {
            threshold: 0,
            keys: vec![dummy_hash(1)],
        };
        assert!(cond.validate().is_err());
    }

    // ---- Threshold validation invariants ----

    #[test]
    fn test_threshold_validate_errors() {
        // n > count
        let cond = Condition::Threshold {
            n: 3,
            conditions: vec![Condition::timelock(100), Condition::timelock(200)],
        };
        assert!(cond.validate().is_err());

        // Zero n
        let cond = Condition::Threshold {
            n: 0,
            conditions: vec![Condition::timelock(100)],
        };
        assert!(cond.validate().is_err());
    }

    // ---- Encoding size checks ----

    #[test]
    fn test_max_multisig_fits() {
        // 7 keys = 1 (version) + 1 (tag) + 2 (params) + 7*32 (keys) = 228 bytes < 256
        let keys: Vec<Hash> = (0..7).map(dummy_hash).collect();
        let cond = Condition::multisig(5, keys);
        let encoded = cond.encode().unwrap();
        assert!(encoded.len() <= 256);
    }

    #[test]
    fn test_htlc_encoding_size() {
        let cond = Condition::htlc(dummy_hash(0xDD), 1000, 2000);
        let encoded = cond.encode().unwrap();
        // version(1) + Or(1) + And(1) + Hashlock(1+32) + Timelock(1+8) + TimelockExpiry(1+8) = 54
        assert!(encoded.len() < 64);
    }

    // ====================================================================
    // Integration tests with real crypto signatures
    // ====================================================================

    fn keypair_pubkey_hash(kp: &crypto::KeyPair) -> Hash {
        hash_with_domain(ADDRESS_DOMAIN, kp.public_key().as_bytes())
    }

    #[test]
    fn integration_signature_condition() {
        let kp = crypto::KeyPair::generate();
        let pkh = keypair_pubkey_hash(&kp);

        let cond = Condition::signature(pkh);
        let encoded = cond.encode().unwrap();
        let decoded = Condition::decode(&encoded).unwrap();
        assert_eq!(cond, decoded);

        // Build a witness with a real signature
        let tx_hash = Hash::from_bytes([0x42; 32]);
        let sig = crypto::signature::sign_hash(&tx_hash, kp.private_key());

        let witness = Witness {
            signatures: vec![WitnessSignature {
                pubkey: *kp.public_key(),
                signature: sig,
            }],
            ..Default::default()
        };

        let ctx = EvalContext {
            current_height: 100,
            signing_hash: &tx_hash,
        };

        let mut branch_idx = 0;
        assert!(evaluate(&cond, &witness, &ctx, &mut branch_idx));

        // Wrong signing hash should fail
        let wrong_hash = Hash::from_bytes([0x99; 32]);
        let ctx_wrong = EvalContext {
            current_height: 100,
            signing_hash: &wrong_hash,
        };
        let mut branch_idx = 0;
        assert!(!evaluate(&cond, &witness, &ctx_wrong, &mut branch_idx));
    }

    #[test]
    fn integration_multisig_2_of_3() {
        let kp1 = crypto::KeyPair::generate();
        let kp2 = crypto::KeyPair::generate();
        let kp3 = crypto::KeyPair::generate();

        let pkh1 = keypair_pubkey_hash(&kp1);
        let pkh2 = keypair_pubkey_hash(&kp2);
        let pkh3 = keypair_pubkey_hash(&kp3);

        let cond = Condition::multisig(2, vec![pkh1, pkh2, pkh3]);
        let encoded = cond.encode().unwrap();
        let decoded = Condition::decode(&encoded).unwrap();
        assert_eq!(cond, decoded);

        let tx_hash = Hash::from_bytes([0xAB; 32]);
        let sig1 = crypto::signature::sign_hash(&tx_hash, kp1.private_key());
        let sig3 = crypto::signature::sign_hash(&tx_hash, kp3.private_key());

        // 2-of-3 with sigs from kp1 and kp3
        let witness = Witness {
            signatures: vec![
                WitnessSignature {
                    pubkey: *kp1.public_key(),
                    signature: sig1,
                },
                WitnessSignature {
                    pubkey: *kp3.public_key(),
                    signature: sig3,
                },
            ],
            ..Default::default()
        };

        let ctx = EvalContext {
            current_height: 100,
            signing_hash: &tx_hash,
        };
        let mut idx = 0;
        assert!(evaluate(&cond, &witness, &ctx, &mut idx));

        // Only 1-of-3 should fail
        let witness_1 = Witness {
            signatures: vec![WitnessSignature {
                pubkey: *kp2.public_key(),
                signature: crypto::signature::sign_hash(&tx_hash, kp2.private_key()),
            }],
            ..Default::default()
        };
        let mut idx = 0;
        assert!(!evaluate(&cond, &witness_1, &ctx, &mut idx));
    }

    #[test]
    fn integration_hashlock_preimage() {
        let secret = [0x77u8; 32];
        let hash = hash_with_domain(HASHLOCK_DOMAIN, &secret);

        let cond = Condition::hashlock(hash);
        let encoded = cond.encode().unwrap();
        let decoded = Condition::decode(&encoded).unwrap();
        assert_eq!(cond, decoded);

        let tx_hash = Hash::from_bytes([0x00; 32]);
        let ctx = EvalContext {
            current_height: 1,
            signing_hash: &tx_hash,
        };

        // Correct preimage
        let witness = Witness {
            preimage: Some(secret),
            ..Default::default()
        };
        let mut idx = 0;
        assert!(evaluate(&cond, &witness, &ctx, &mut idx));

        // Wrong preimage
        let bad_witness = Witness {
            preimage: Some([0x88u8; 32]),
            ..Default::default()
        };
        let mut idx = 0;
        assert!(!evaluate(&cond, &bad_witness, &ctx, &mut idx));

        // No preimage
        let empty_witness = Witness::default();
        let mut idx = 0;
        assert!(!evaluate(&cond, &empty_witness, &ctx, &mut idx));
    }

    #[test]
    fn integration_htlc_claim_and_refund() {
        let secret = [0xCC; 32];
        let hash = hash_with_domain(HASHLOCK_DOMAIN, &secret);

        // Standard HTLC helper: Or(And(Hashlock, Timelock(100)), TimelockExpiry(200))
        // Left branch: receiver claims with preimage after lock_height
        // Right branch: refund window (anyone before expiry_height)
        let cond = Condition::htlc(hash, 100, 200);
        let encoded = cond.encode().unwrap();
        assert!(encoded.len() <= MAX_WITNESS_SIZE);

        let tx_hash = Hash::from_bytes([0xDD; 32]);

        // Claim path: preimage + height >= 100, branch(left)
        let claim_witness = Witness {
            preimage: Some(secret),
            or_branches: vec![false], // left branch (hashlock path)
            ..Default::default()
        };
        let ctx_after_lock = EvalContext {
            current_height: 150,
            signing_hash: &tx_hash,
        };
        let mut idx = 0;
        assert!(evaluate(&cond, &claim_witness, &ctx_after_lock, &mut idx));

        // Claim path fails before timelock
        let ctx_before_lock = EvalContext {
            current_height: 50,
            signing_hash: &tx_hash,
        };
        let mut idx = 0;
        assert!(!evaluate(&cond, &claim_witness, &ctx_before_lock, &mut idx));

        // Refund path: right branch, TimelockExpiry(200) means height >= 200
        let refund_witness = Witness {
            or_branches: vec![true], // right branch
            ..Default::default()
        };

        // Refund fails before expiry (height 150 < 200)
        let ctx_before_expiry = EvalContext {
            current_height: 150,
            signing_hash: &tx_hash,
        };
        let mut idx = 0;
        assert!(!evaluate(
            &cond,
            &refund_witness,
            &ctx_before_expiry,
            &mut idx
        ));

        // Refund succeeds after expiry (height 250 >= 200)
        let ctx_after_expiry = EvalContext {
            current_height: 250,
            signing_hash: &tx_hash,
        };
        let mut idx = 0;
        assert!(evaluate(
            &cond,
            &refund_witness,
            &ctx_after_expiry,
            &mut idx
        ));

        // Full HTLC with sender signature for refund:
        // Or(And(Hashlock, Timelock(100)), And(Signature(sender), Timelock(200)))
        let kp_sender = crypto::KeyPair::generate();
        let sender_pkh = keypair_pubkey_hash(&kp_sender);
        let cond_with_sig_refund = Condition::Or(
            Box::new(Condition::And(
                Box::new(Condition::Hashlock(hash)),
                Box::new(Condition::Timelock(100)),
            )),
            Box::new(Condition::And(
                Box::new(Condition::Signature(sender_pkh)),
                Box::new(Condition::Timelock(200)),
            )),
        );

        let sig = crypto::signature::sign_hash(&tx_hash, kp_sender.private_key());
        let sig_refund_witness = Witness {
            signatures: vec![WitnessSignature {
                pubkey: *kp_sender.public_key(),
                signature: sig,
            }],
            or_branches: vec![true], // right branch (refund)
            ..Default::default()
        };
        let ctx_after_refund_lock = EvalContext {
            current_height: 250,
            signing_hash: &tx_hash,
        };
        let mut idx = 0;
        assert!(evaluate(
            &cond_with_sig_refund,
            &sig_refund_witness,
            &ctx_after_refund_lock,
            &mut idx
        ));

        // Sender refund fails before refund timelock
        let ctx_too_early = EvalContext {
            current_height: 150,
            signing_hash: &tx_hash,
        };
        let mut idx = 0;
        assert!(!evaluate(
            &cond_with_sig_refund,
            &sig_refund_witness,
            &ctx_too_early,
            &mut idx
        ));
    }

    #[test]
    fn integration_vesting_schedule() {
        let kp = crypto::KeyPair::generate();
        let pkh = keypair_pubkey_hash(&kp);

        // Vesting: And(Signature(owner), Timelock(1000))
        let cond = Condition::vesting(pkh, 1000);
        let encoded = cond.encode().unwrap();
        let decoded = Condition::decode(&encoded).unwrap();
        assert_eq!(cond, decoded);

        let tx_hash = Hash::from_bytes([0xEE; 32]);
        let sig = crypto::signature::sign_hash(&tx_hash, kp.private_key());

        let witness = Witness {
            signatures: vec![WitnessSignature {
                pubkey: *kp.public_key(),
                signature: sig,
            }],
            ..Default::default()
        };

        // Before vesting period
        let ctx_early = EvalContext {
            current_height: 500,
            signing_hash: &tx_hash,
        };
        let mut idx = 0;
        assert!(!evaluate(&cond, &witness, &ctx_early, &mut idx));

        // After vesting period
        let ctx_vested = EvalContext {
            current_height: 1000,
            signing_hash: &tx_hash,
        };
        let mut idx = 0;
        assert!(evaluate(&cond, &witness, &ctx_vested, &mut idx));
    }

    #[test]
    fn integration_witness_encode_decode_roundtrip() {
        let kp1 = crypto::KeyPair::generate();
        let kp2 = crypto::KeyPair::generate();
        let tx_hash = Hash::from_bytes([0xFF; 32]);

        let sig1 = crypto::signature::sign_hash(&tx_hash, kp1.private_key());
        let sig2 = crypto::signature::sign_hash(&tx_hash, kp2.private_key());

        let witness = Witness {
            signatures: vec![
                WitnessSignature {
                    pubkey: *kp1.public_key(),
                    signature: sig1,
                },
                WitnessSignature {
                    pubkey: *kp2.public_key(),
                    signature: sig2,
                },
            ],
            preimage: Some([0xAA; 32]),
            or_branches: vec![true, false, true],
        };

        let encoded = witness.encode();
        let decoded = Witness::decode(&encoded).unwrap();

        assert_eq!(witness.signatures.len(), decoded.signatures.len());
        assert_eq!(witness.preimage, decoded.preimage);
        assert_eq!(witness.or_branches, decoded.or_branches);

        // Verify signatures still valid after roundtrip
        for (orig, dec) in witness.signatures.iter().zip(decoded.signatures.iter()) {
            assert_eq!(orig.pubkey, dec.pubkey);
            assert_eq!(orig.signature, dec.signature);
        }
    }

    #[test]
    fn integration_threshold_2_of_3_mixed() {
        // Threshold(2, [Signature(A), Hashlock(H), Timelock(50)])
        let kp = crypto::KeyPair::generate();
        let pkh = keypair_pubkey_hash(&kp);
        let secret = [0x55; 32];
        let hash = hash_with_domain(HASHLOCK_DOMAIN, &secret);

        let cond = Condition::Threshold {
            n: 2,
            conditions: vec![
                Condition::Signature(pkh),
                Condition::Hashlock(hash),
                Condition::Timelock(50),
            ],
        };
        assert!(cond.validate().is_ok());

        let tx_hash = Hash::from_bytes([0x11; 32]);
        let sig = crypto::signature::sign_hash(&tx_hash, kp.private_key());

        // Satisfy signature + hashlock (2 of 3)
        let witness = Witness {
            signatures: vec![WitnessSignature {
                pubkey: *kp.public_key(),
                signature: sig,
            }],
            preimage: Some(secret),
            ..Default::default()
        };
        let ctx = EvalContext {
            current_height: 10, // below timelock
            signing_hash: &tx_hash,
        };
        let mut idx = 0;
        assert!(evaluate(&cond, &witness, &ctx, &mut idx));

        // Satisfy signature + timelock (2 of 3)
        let witness_no_preimage = Witness {
            signatures: vec![WitnessSignature {
                pubkey: *kp.public_key(),
                signature: sig,
            }],
            ..Default::default()
        };
        let ctx_high = EvalContext {
            current_height: 100,
            signing_hash: &tx_hash,
        };
        let mut idx = 0;
        assert!(evaluate(&cond, &witness_no_preimage, &ctx_high, &mut idx));

        // Only timelock satisfied (1 of 3) — fail
        let witness_empty = Witness::default();
        let mut idx = 0;
        assert!(!evaluate(&cond, &witness_empty, &ctx_high, &mut idx));
    }
}
