//! Veto voting system
//!
//! Producers can vote to veto any proposed update during the 7-day period.
//! If >= 33% of active producers veto, the update is rejected.

use crypto::{PublicKey, Signature as CryptoSignature};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// A vote on a proposed release
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Vote {
    /// Approve the update (or abstain - same effect)
    Approve,
    /// Veto the update
    Veto,
}

/// A signed vote message from a producer
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VoteMessage {
    /// Version being voted on
    pub version: String,

    /// The vote
    pub vote: Vote,

    /// Producer's public key (hex-encoded)
    pub producer_id: String,

    /// Unix timestamp of the vote
    pub timestamp: u64,

    /// Signature over "version:vote:timestamp" (hex-encoded)
    pub signature: String,
}

impl VoteMessage {
    /// Create a new vote message (unsigned)
    pub fn new(version: String, vote: Vote, producer_id: String) -> Self {
        Self {
            version,
            vote,
            producer_id,
            timestamp: crate::current_timestamp(),
            signature: String::new(),
        }
    }

    /// Get the message bytes for signing
    pub fn message_bytes(&self) -> Vec<u8> {
        let vote_str = match self.vote {
            Vote::Approve => "approve",
            Vote::Veto => "veto",
        };
        format!("{}:{}:{}", self.version, vote_str, self.timestamp).into_bytes()
    }

    /// Verify the signature on this vote
    pub fn verify(&self, expected_producer: &str) -> bool {
        if self.producer_id != expected_producer {
            return false;
        }

        // Parse public key
        let pubkey = match PublicKey::from_hex(&self.producer_id) {
            Ok(pk) => pk,
            Err(_) => return false,
        };

        // Parse signature
        let sig = match CryptoSignature::from_hex(&self.signature) {
            Ok(s) => s,
            Err(_) => return false,
        };

        // Verify
        crypto::signature::verify(&self.message_bytes(), &sig, &pubkey).is_ok()
    }
}

/// Tracks votes for a specific release
///
/// Supports both count-based and weight-based veto calculations.
/// Weight-based voting gives more power to senior producers (anti-Sybil).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoteTracker {
    /// Version being tracked
    version: String,

    /// Set of producer IDs that have vetoed
    vetos: HashSet<String>,

    /// Set of producer IDs that have approved
    approvals: HashSet<String>,

    /// Weight of each producer (by ID)
    /// If empty, falls back to count-based calculation
    #[serde(default)]
    producer_weights: std::collections::HashMap<String, u64>,
}

impl Default for VoteTracker {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl VoteTracker {
    /// Create a new vote tracker for a release
    pub fn new(version: String) -> Self {
        Self {
            version,
            vetos: HashSet::new(),
            approvals: HashSet::new(),
            producer_weights: std::collections::HashMap::new(),
        }
    }

    /// Create a vote tracker with weighted voting
    ///
    /// # Arguments
    /// - `version`: The release version being voted on
    /// - `weights`: Map of producer IDs to their seniority weights
    pub fn with_weights(version: String, weights: std::collections::HashMap<String, u64>) -> Self {
        Self {
            version,
            vetos: HashSet::new(),
            approvals: HashSet::new(),
            producer_weights: weights,
        }
    }

    /// Set producer weights for weighted voting
    pub fn set_weights(&mut self, weights: std::collections::HashMap<String, u64>) {
        self.producer_weights = weights;
    }

    /// Record a vote from a producer
    ///
    /// Returns true if this is a new vote, false if already voted
    pub fn record_vote(&mut self, producer_id: String, vote: Vote) -> bool {
        // Check if already voted
        if self.vetos.contains(&producer_id) || self.approvals.contains(&producer_id) {
            return false;
        }

        match vote {
            Vote::Veto => self.vetos.insert(producer_id),
            Vote::Approve => self.approvals.insert(producer_id),
        }
    }

    /// Get the number of veto votes
    pub fn veto_count(&self) -> usize {
        self.vetos.len()
    }

    /// Get the number of approval votes
    pub fn approval_count(&self) -> usize {
        self.approvals.len()
    }

    /// Get total votes cast
    pub fn total_votes(&self) -> usize {
        self.vetos.len() + self.approvals.len()
    }

    /// Calculate if update should be rejected (count-based, legacy)
    ///
    /// Prefer `should_reject_weighted` for Sybil-resistant voting.
    pub fn should_reject(&self, total_producers: usize) -> bool {
        if total_producers == 0 {
            return false;
        }

        let veto_percent = (self.vetos.len() * 100) / total_producers;
        veto_percent >= crate::VETO_THRESHOLD_PERCENT as usize
    }

    /// Calculate if update should be rejected using weighted voting
    ///
    /// Uses producer seniority weights for anti-Sybil resistance.
    /// A whale with many new nodes has less voting power than
    /// established producers.
    ///
    /// # Arguments
    /// - `total_weight`: Total weight of all active producers
    ///
    /// # Returns
    /// `true` if veto weight >= 40% of total weight
    pub fn should_reject_weighted(&self, total_weight: u64) -> bool {
        if total_weight == 0 {
            return false;
        }

        let veto_weight = self.veto_weight();
        let veto_percent = (veto_weight * 100) / total_weight;
        veto_percent >= crate::VETO_THRESHOLD_PERCENT as u64
    }

    /// Get the total weight of veto votes
    pub fn veto_weight(&self) -> u64 {
        self.vetos
            .iter()
            .map(|id| self.producer_weights.get(id).copied().unwrap_or(1))
            .sum()
    }

    /// Get the total weight of approval votes
    pub fn approval_weight(&self) -> u64 {
        self.approvals
            .iter()
            .map(|id| self.producer_weights.get(id).copied().unwrap_or(1))
            .sum()
    }

    /// Get current veto percentage (count-based, legacy)
    pub fn veto_percent(&self, total_producers: usize) -> u8 {
        if total_producers == 0 {
            return 0;
        }
        ((self.vetos.len() * 100) / total_producers) as u8
    }

    /// Get current veto percentage (weight-based)
    pub fn veto_percent_weighted(&self, total_weight: u64) -> u8 {
        if total_weight == 0 {
            return 0;
        }
        ((self.veto_weight() * 100) / total_weight) as u8
    }

    /// Get the version being tracked
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Get list of producers who vetoed
    pub fn veto_producers(&self) -> &HashSet<String> {
        &self.vetos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vote_tracker() {
        let mut tracker = VoteTracker::new("1.0.0".into());

        // Record some votes
        assert!(tracker.record_vote("producer1".into(), Vote::Veto));
        assert!(tracker.record_vote("producer2".into(), Vote::Approve));
        assert!(tracker.record_vote("producer3".into(), Vote::Veto));

        // Duplicate vote should return false
        assert!(!tracker.record_vote("producer1".into(), Vote::Approve));

        assert_eq!(tracker.veto_count(), 2);
        assert_eq!(tracker.approval_count(), 1);
        assert_eq!(tracker.total_votes(), 3);

        // With 10 producers, 2 vetos = 20%
        assert_eq!(tracker.veto_percent(10), 20);
        assert!(!tracker.should_reject(10));

        // With 5 producers, 2 vetos = 40%
        assert_eq!(tracker.veto_percent(5), 40);
        assert!(tracker.should_reject(5));
    }

    #[test]
    fn test_vote_message() {
        let msg = VoteMessage::new("1.0.0".into(), Vote::Veto, "abc123".into());

        assert_eq!(msg.version, "1.0.0");
        assert_eq!(msg.vote, Vote::Veto);

        let bytes = msg.message_bytes();
        assert!(bytes.starts_with(b"1.0.0:veto:"));
    }

    #[test]
    fn test_weighted_voting() {
        // Set up weights: senior producers have more weight
        let mut weights = std::collections::HashMap::new();
        weights.insert("senior1".to_string(), 4); // 4-year veteran
        weights.insert("senior2".to_string(), 3); // 3-year veteran
        weights.insert("junior1".to_string(), 1); // New producer
        weights.insert("junior2".to_string(), 1); // New producer
        weights.insert("junior3".to_string(), 1); // New producer

        let mut tracker = VoteTracker::with_weights("1.0.0".into(), weights);

        // Total weight = 4 + 3 + 1 + 1 + 1 = 10

        // Junior votes veto (weight 1)
        tracker.record_vote("junior1".into(), Vote::Veto);
        assert_eq!(tracker.veto_weight(), 1);
        assert!(!tracker.should_reject_weighted(10)); // 10% < 33%

        // Another junior votes veto (total weight 2)
        tracker.record_vote("junior2".into(), Vote::Veto);
        assert_eq!(tracker.veto_weight(), 2);
        assert!(!tracker.should_reject_weighted(10)); // 20% < 33%

        // Senior producer votes veto (weight 4, total 6)
        tracker.record_vote("senior1".into(), Vote::Veto);
        assert_eq!(tracker.veto_weight(), 6);
        assert!(tracker.should_reject_weighted(10)); // 60% >= 33%
    }

    #[test]
    fn test_weighted_vs_count_difference() {
        // Scenario: 1 senior (weight 4) vs 3 juniors (weight 1 each)
        let mut weights = std::collections::HashMap::new();
        weights.insert("senior".to_string(), 4);
        weights.insert("junior1".to_string(), 1);
        weights.insert("junior2".to_string(), 1);
        weights.insert("junior3".to_string(), 1);

        let mut tracker = VoteTracker::with_weights("1.0.0".into(), weights);
        let total_weight = 7; // 4 + 1 + 1 + 1
        let total_count = 4;

        // Senior vetoes alone
        tracker.record_vote("senior".into(), Vote::Veto);

        // Count-based: 1/4 = 25% - NOT rejected
        assert!(!tracker.should_reject(total_count));

        // Weight-based: 4/7 = 57% - REJECTED
        assert!(tracker.should_reject_weighted(total_weight));

        // This is the anti-Sybil protection: a whale's 3 new nodes
        // can't outvote a single established producer
    }

    #[test]
    fn test_veto_percent_weighted() {
        let mut weights = std::collections::HashMap::new();
        weights.insert("p1".to_string(), 2);
        weights.insert("p2".to_string(), 3);
        weights.insert("p3".to_string(), 5);

        let mut tracker = VoteTracker::with_weights("1.0.0".into(), weights);
        let total_weight = 10;

        tracker.record_vote("p1".into(), Vote::Veto);
        assert_eq!(tracker.veto_percent_weighted(total_weight), 20);

        tracker.record_vote("p2".into(), Vote::Veto);
        assert_eq!(tracker.veto_percent_weighted(total_weight), 50);
    }
}
