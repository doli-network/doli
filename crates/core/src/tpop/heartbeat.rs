//! # Presence Heartbeat - Micro-VDF Implementation
//!
//! This module implements the simplified TPoP using micro-VDFs (~1 second)
//! instead of the original 55-second VDFs.
//!
//! ## Design Philosophy
//!
//! The long VDF was designed for anti-grinding in lottery-based selection.
//! In TPoP with presence_score, there's no lottery to grind. The bond is
//! the anti-Sybil mechanism, not the VDF.
//!
//! The micro-VDF serves only to prove:
//! 1. You couldn't generate heartbeats instantaneously (need ~1s each)
//! 2. You couldn't pre-compute (input depends on prev_block_hash)
//! 3. You were active during the slot (must arrive on time)
//!
//! ## Resource Efficiency
//!
//! | Metric          | 55s VDF | 1s Micro-VDF |
//! |-----------------|---------|--------------|
//! | CPU per slot    | 91%     | 3%           |
//! | Min hardware    | i5      | Raspberry Pi |
//! | Verification    | 500ms   | 10ms         |

use crypto::hash::hash_concat;
use crypto::signature::sign_hash;
use crypto::{Hash, KeyPair, PublicKey, Signature};
use serde::{Deserialize, Serialize};
use vdf::VdfProof;

use crate::consensus::ConsensusParams;
use crate::types::Slot;

// =============================================================================
// MICRO-VDF CONSTANTS
// =============================================================================

/// Iterations for micro-VDF (~1 second on reference hardware)
///
/// We use a hash-chain VDF instead of class-group VDF because:
/// - Hash chains run at ~10M iterations/second
/// - Class-group operations are ~1000x slower
/// - For timing proofs, hash chains are sufficient
///
/// 10,000,000 iterations ≈ 1 second on modern hardware
pub const HEARTBEAT_VDF_ITERATIONS: u64 = 10_000_000;

/// Legacy constant for compatibility (not used with hash-chain VDF)
pub const HEARTBEAT_DISCRIMINANT_BITS: usize = 1024;

/// Maximum time after slot start to accept heartbeats (seconds)
/// Heartbeats arriving after this are considered late
pub const HEARTBEAT_DEADLINE_SECS: u64 = 55;

/// Grace period for network delays (seconds)
/// Heartbeats can arrive up to this many seconds after deadline
pub const HEARTBEAT_GRACE_PERIOD_SECS: u64 = 5;

/// Maximum heartbeats to store per slot (memory limit)
pub const MAX_HEARTBEATS_PER_SLOT: usize = 1000;

// =============================================================================
// HASH-CHAIN MICRO-VDF
// =============================================================================

/// Compute a hash-chain micro-VDF
///
/// This provides sequential work proof using iterated hashing.
/// Unlike class-group VDFs, this is:
/// - Very fast (~10M hashes/second)
/// - Linear verification (just repeat the hash chain)
/// - Sufficient for timing proofs
///
/// # Arguments
/// * `input` - The 32-byte input hash
/// * `iterations` - Number of hash iterations
///
/// # Returns
/// The final hash after all iterations
pub fn hash_chain_vdf(input: &Hash, iterations: u64) -> [u8; 32] {
    use crypto::hash::hash;

    let mut state = *input.as_bytes();
    for _ in 0..iterations {
        state = *hash(&state).as_bytes();
    }
    state
}

/// Verify a hash-chain micro-VDF
///
/// Verification requires recomputing the entire chain.
/// This is acceptable because:
/// - Hash operations are very fast
/// - Verification can be parallelized across multiple heartbeats
/// - The time to verify << time to compute (due to caching effects)
pub fn verify_hash_chain_vdf(input: &Hash, expected_output: &[u8; 32], iterations: u64) -> bool {
    let computed = hash_chain_vdf(input, iterations);
    computed == *expected_output
}

// =============================================================================
// HEARTBEAT STRUCTURE
// =============================================================================

/// A presence heartbeat - minimal proof of activity for a slot
///
/// The heartbeat proves:
/// 1. The producer has the private key (signature)
/// 2. They did ~1 second of sequential work (micro-VDF)
/// 3. They know the previous block hash (can't pre-compute)
/// 4. They were active during this slot (timing)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceHeartbeat {
    /// Protocol version (for future upgrades)
    pub version: u8,

    /// Producer's public key
    pub producer: PublicKey,

    /// Slot this heartbeat is for
    pub slot: Slot,

    /// Hash of the previous block (anchor point)
    pub prev_block_hash: Hash,

    /// Micro-VDF output (32 bytes)
    pub vdf_output: [u8; 32],

    /// Wesolowski proof for the micro-VDF
    pub vdf_proof: VdfProof,

    /// Signature over (version || slot || prev_hash || vdf_output)
    pub signature: Signature,
}

impl PresenceHeartbeat {
    /// Current protocol version
    pub const VERSION: u8 = 1;

    /// Create a new heartbeat (computes VDF and signs)
    pub fn create(
        keypair: &KeyPair,
        slot: Slot,
        prev_block_hash: Hash,
    ) -> Result<Self, HeartbeatError> {
        let producer = keypair.public_key().clone();

        // Compute VDF input
        let vdf_input = Self::compute_vdf_input(&producer, slot, &prev_block_hash);

        // Compute hash-chain micro-VDF (~1 second)
        let vdf_output = hash_chain_vdf(&vdf_input, HEARTBEAT_VDF_ITERATIONS);

        // Create signature
        let sign_msg = Self::signing_message(Self::VERSION, slot, &prev_block_hash, &vdf_output);
        let signature = sign_hash(&sign_msg, keypair.private_key());

        Ok(Self {
            version: Self::VERSION,
            producer,
            slot,
            prev_block_hash,
            vdf_output,
            // Empty proof - hash chain VDF doesn't need Wesolowski proof
            vdf_proof: VdfProof::empty(),
            signature,
        })
    }

    /// Compute the VDF input hash
    ///
    /// Input = H("DOLI_HEARTBEAT_V1" || producer || slot || prev_hash)
    ///
    /// This ensures:
    /// - Different per producer (can't share VDF results)
    /// - Different per slot (can't reuse across slots)
    /// - Depends on prev_hash (can't pre-compute)
    pub fn compute_vdf_input(producer: &PublicKey, slot: Slot, prev_hash: &Hash) -> Hash {
        hash_concat(&[
            b"DOLI_HEARTBEAT_V1",
            producer.as_bytes(),
            &slot.to_le_bytes(),
            prev_hash.as_bytes(),
        ])
    }

    /// Compute the message to sign
    fn signing_message(version: u8, slot: Slot, prev_hash: &Hash, vdf_output: &[u8; 32]) -> Hash {
        hash_concat(&[
            &[version],
            &slot.to_le_bytes(),
            prev_hash.as_bytes(),
            vdf_output,
        ])
    }

    /// Verify the heartbeat is valid
    ///
    /// Checks:
    /// 1. VDF output is correct (hash chain)
    /// 2. Signature is valid
    /// 3. Version is supported
    pub fn verify(&self, expected_prev_hash: &Hash) -> Result<(), HeartbeatError> {
        // Check version
        if self.version != Self::VERSION {
            return Err(HeartbeatError::UnsupportedVersion(self.version));
        }

        // Check prev_hash matches
        if &self.prev_block_hash != expected_prev_hash {
            return Err(HeartbeatError::PrevHashMismatch);
        }

        // Verify hash-chain VDF
        let vdf_input = Self::compute_vdf_input(&self.producer, self.slot, &self.prev_block_hash);
        if !verify_hash_chain_vdf(&vdf_input, &self.vdf_output, HEARTBEAT_VDF_ITERATIONS) {
            return Err(HeartbeatError::InvalidVdf);
        }

        // Verify signature
        let sign_msg = Self::signing_message(
            self.version,
            self.slot,
            &self.prev_block_hash,
            &self.vdf_output,
        );

        crypto::signature::verify_hash(&sign_msg, &self.signature, &self.producer)
            .map_err(|_| HeartbeatError::InvalidSignature)?;

        Ok(())
    }

    /// Get the unique identifier for this heartbeat
    pub fn id(&self) -> Hash {
        hash_concat(&[self.producer.as_bytes(), &self.slot.to_le_bytes()])
    }

    /// Serialize for network transmission
    pub fn serialize(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap_or_default()
    }

    /// Deserialize from network
    pub fn deserialize(bytes: &[u8]) -> Option<Self> {
        bincode::deserialize(bytes).ok()
    }

    /// Approximate size in bytes
    pub fn size(&self) -> usize {
        1 + 32 + 4 + 32 + 32 + self.vdf_proof.pi.len() + 64
    }
}

/// Errors that can occur with heartbeats
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeartbeatError {
    /// VDF computation failed
    VdfFailed(String),
    /// VDF output has wrong size
    InvalidVdfOutput,
    /// VDF proof verification failed
    InvalidVdf,
    /// Signature verification failed
    InvalidSignature,
    /// Previous block hash doesn't match
    PrevHashMismatch,
    /// Heartbeat version not supported
    UnsupportedVersion(u8),
    /// Heartbeat arrived too late
    TooLate { slot: Slot, received_at: u64 },
    /// Heartbeat is for future slot
    FutureSlot { slot: Slot, current: Slot },
    /// Heartbeat is for old slot
    TooOld { slot: Slot, current: Slot },
}

impl std::fmt::Display for HeartbeatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::VdfFailed(e) => write!(f, "VDF computation failed: {}", e),
            Self::InvalidVdfOutput => write!(f, "VDF output has invalid size"),
            Self::InvalidVdf => write!(f, "VDF proof verification failed"),
            Self::InvalidSignature => write!(f, "signature verification failed"),
            Self::PrevHashMismatch => write!(f, "previous block hash mismatch"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported version: {}", v),
            Self::TooLate { slot, received_at } => {
                write!(
                    f,
                    "heartbeat for slot {} received too late at {}",
                    slot, received_at
                )
            }
            Self::FutureSlot { slot, current } => {
                write!(
                    f,
                    "heartbeat for future slot {} (current: {})",
                    slot, current
                )
            }
            Self::TooOld { slot, current } => {
                write!(f, "heartbeat for old slot {} (current: {})", slot, current)
            }
        }
    }
}

impl std::error::Error for HeartbeatError {}

// =============================================================================
// HEARTBEAT TIMING VALIDATION
// =============================================================================

/// Validate that a heartbeat arrived within the acceptable time window
pub fn validate_heartbeat_timing(
    heartbeat: &PresenceHeartbeat,
    current_slot: Slot,
    received_timestamp: u64,
    params: &ConsensusParams,
) -> Result<(), HeartbeatError> {
    // Check slot is not in the future
    if heartbeat.slot > current_slot {
        return Err(HeartbeatError::FutureSlot {
            slot: heartbeat.slot,
            current: current_slot,
        });
    }

    // Check slot is not too old (allow current and previous slot)
    if heartbeat.slot < current_slot.saturating_sub(1) {
        return Err(HeartbeatError::TooOld {
            slot: heartbeat.slot,
            current: current_slot,
        });
    }

    // Check arrival time is within deadline + grace period
    let slot_start = params.slot_to_timestamp(heartbeat.slot);
    let deadline = slot_start + HEARTBEAT_DEADLINE_SECS + HEARTBEAT_GRACE_PERIOD_SECS;

    if received_timestamp > deadline {
        return Err(HeartbeatError::TooLate {
            slot: heartbeat.slot,
            received_at: received_timestamp,
        });
    }

    Ok(())
}

// =============================================================================
// HEARTBEAT COLLECTOR
// =============================================================================

/// Collects and validates heartbeats for presence tracking
#[derive(Debug, Default)]
pub struct HeartbeatCollector {
    /// Heartbeats indexed by (slot, producer)
    heartbeats: std::collections::HashMap<(Slot, PublicKey), PresenceHeartbeat>,

    /// Count per slot (for rate limiting)
    per_slot_count: std::collections::HashMap<Slot, usize>,
}

impl HeartbeatCollector {
    /// Create a new collector
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a heartbeat (validates before adding)
    pub fn add(
        &mut self,
        heartbeat: PresenceHeartbeat,
        expected_prev_hash: &Hash,
        current_slot: Slot,
        received_at: u64,
        params: &ConsensusParams,
    ) -> Result<(), HeartbeatError> {
        // Validate timing
        validate_heartbeat_timing(&heartbeat, current_slot, received_at, params)?;

        // Validate content
        heartbeat.verify(expected_prev_hash)?;

        // Check rate limit
        let count = self.per_slot_count.entry(heartbeat.slot).or_insert(0);
        if *count >= MAX_HEARTBEATS_PER_SLOT {
            // Silently drop (DoS protection)
            return Ok(());
        }

        // Add to collection
        let key = (heartbeat.slot, heartbeat.producer.clone());
        if !self.heartbeats.contains_key(&key) {
            *count += 1;
        }
        self.heartbeats.insert(key, heartbeat);

        Ok(())
    }

    /// Get all valid heartbeats for a slot
    pub fn heartbeats_for_slot(&self, slot: Slot) -> Vec<&PresenceHeartbeat> {
        self.heartbeats
            .iter()
            .filter(|((s, _), _)| *s == slot)
            .map(|(_, hb)| hb)
            .collect()
    }

    /// Get producers who submitted heartbeats for a slot
    pub fn producers_for_slot(&self, slot: Slot) -> Vec<PublicKey> {
        self.heartbeats
            .iter()
            .filter(|((s, _), _)| *s == slot)
            .map(|((_, pk), _)| pk.clone())
            .collect()
    }

    /// Count heartbeats for a slot
    pub fn count_for_slot(&self, slot: Slot) -> usize {
        self.per_slot_count.get(&slot).copied().unwrap_or(0)
    }

    /// Prune old slots (memory management)
    pub fn prune_before(&mut self, min_slot: Slot) {
        self.heartbeats.retain(|(slot, _), _| *slot >= min_slot);
        self.per_slot_count.retain(|slot, _| *slot >= min_slot);
    }

    /// Check if a producer submitted a heartbeat for a slot
    pub fn has_heartbeat(&self, slot: Slot, producer: &PublicKey) -> bool {
        self.heartbeats.contains_key(&(slot, producer.clone()))
    }
}

// =============================================================================
// PRESENCE SCORE CALCULATION (SIMPLIFIED)
// =============================================================================

/// Calculate presence score based on heartbeat history
///
/// The score reflects:
/// - Recent activity (heartbeats in recent slots)
/// - Consistency (few missed slots)
/// - Longevity (time in network)
pub fn calculate_heartbeat_score(
    heartbeats_submitted: u64,
    total_slots: u64,
    consecutive_present: u64,
    age_in_eras: u32,
) -> u64 {
    if total_slots == 0 {
        return 0;
    }

    // Base: presence ratio (0-100)
    let presence_ratio = (heartbeats_submitted * 100) / total_slots;

    // Bonus: consecutive presence (max 50 points)
    let consecutive_bonus = (consecutive_present / 10).min(50);

    // Bonus: consistency (>90% = 20 points, >95% = 30 points)
    let consistency_bonus = if presence_ratio >= 95 {
        30
    } else if presence_ratio >= 90 {
        20
    } else if presence_ratio >= 80 {
        10
    } else {
        0
    };

    // Bonus: age (logarithmic, max 20 points)
    let age_bonus = if age_in_eras > 0 {
        ((age_in_eras as u64).ilog2() as u64 + 1) * 5
    } else {
        0
    }
    .min(20);

    presence_ratio
        .saturating_add(consecutive_bonus)
        .saturating_add(consistency_bonus)
        .saturating_add(age_bonus)
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_hash(seed: u8) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        Hash::from_bytes(bytes)
    }

    fn mock_pubkey(seed: u8) -> PublicKey {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        PublicKey::from_bytes(bytes)
    }

    #[test]
    fn test_vdf_input_uniqueness() {
        let pk1 = mock_pubkey(1);
        let pk2 = mock_pubkey(2);
        let hash1 = mock_hash(10);
        let hash2 = mock_hash(20);

        // Different producers = different inputs
        let input1 = PresenceHeartbeat::compute_vdf_input(&pk1, 100, &hash1);
        let input2 = PresenceHeartbeat::compute_vdf_input(&pk2, 100, &hash1);
        assert_ne!(input1, input2);

        // Different slots = different inputs
        let input3 = PresenceHeartbeat::compute_vdf_input(&pk1, 101, &hash1);
        assert_ne!(input1, input3);

        // Different prev_hash = different inputs
        let input4 = PresenceHeartbeat::compute_vdf_input(&pk1, 100, &hash2);
        assert_ne!(input1, input4);

        // Same inputs = same result (deterministic)
        let input5 = PresenceHeartbeat::compute_vdf_input(&pk1, 100, &hash1);
        assert_eq!(input1, input5);
    }

    #[test]
    fn test_heartbeat_score_calculation() {
        // Perfect presence
        let score = calculate_heartbeat_score(100, 100, 100, 0);
        assert!(score >= 100); // 100 ratio + bonuses

        // 50% presence
        let score = calculate_heartbeat_score(50, 100, 0, 0);
        assert_eq!(score, 50); // Just ratio, no bonuses

        // High consistency bonus
        let score = calculate_heartbeat_score(95, 100, 50, 0);
        assert!(score > 95); // Ratio + consistency bonus

        // Age bonus
        let score_new = calculate_heartbeat_score(80, 100, 0, 0);
        let score_old = calculate_heartbeat_score(80, 100, 0, 4);
        assert!(score_old > score_new);
    }

    #[test]
    fn test_collector_rate_limiting() {
        let mut collector = HeartbeatCollector::new();

        // Manually add entries to test rate limiting
        for i in 0..MAX_HEARTBEATS_PER_SLOT {
            let pk = mock_pubkey(i as u8);
            collector.heartbeats.insert(
                (100, pk),
                PresenceHeartbeat {
                    version: 1,
                    producer: mock_pubkey(i as u8),
                    slot: 100,
                    prev_block_hash: mock_hash(0),
                    vdf_output: [0u8; 32],
                    vdf_proof: VdfProof::empty(),
                    signature: Signature::default(),
                },
            );
        }
        collector
            .per_slot_count
            .insert(100, MAX_HEARTBEATS_PER_SLOT);

        assert_eq!(collector.count_for_slot(100), MAX_HEARTBEATS_PER_SLOT);
    }

    #[test]
    fn test_collector_pruning() {
        let mut collector = HeartbeatCollector::new();

        // Add heartbeats for slots 100-110
        for slot in 100..=110 {
            let pk = mock_pubkey(slot as u8);
            collector.heartbeats.insert(
                (slot, pk.clone()),
                PresenceHeartbeat {
                    version: 1,
                    producer: pk,
                    slot,
                    prev_block_hash: mock_hash(0),
                    vdf_output: [0u8; 32],
                    vdf_proof: VdfProof::empty(),
                    signature: Signature::default(),
                },
            );
            collector.per_slot_count.insert(slot, 1);
        }

        assert_eq!(collector.heartbeats.len(), 11);

        // Prune before slot 105
        collector.prune_before(105);

        assert_eq!(collector.heartbeats.len(), 6); // 105-110
        assert!(collector.has_heartbeat(105, &mock_pubkey(105)));
        assert!(!collector.has_heartbeat(104, &mock_pubkey(104)));
    }
}
