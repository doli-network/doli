//! Veto voting system
//!
//! Producers can vote to veto any proposed update during the veto period
//! (`VETO_PERIOD`, or the network-specific `UpdateParams::veto_period_secs`).
//! If >= 40% of active producers veto, the update is rejected.
//!
//! Veto counting is by HEAD COUNT. The seniority-weighted variant that used to live
//! here was never reachable from production code — its only callers were `#[cfg(test)]`
//! — while four documents and one log line told operators a "7-day seniority-weighted
//! veto" was protecting them. It was deleted in INC-I-172 M1 (F8) so the code and the
//! documentation say the same thing.

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
    #[serde(alias = "producerId")]
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
/// Veto calculation is count-based: one registered producer, one vote.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoteTracker {
    /// Version being tracked
    version: String,

    /// Set of producer IDs that have vetoed
    vetos: HashSet<String>,

    /// Set of producer IDs that have approved
    approvals: HashSet<String>,
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
        }
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

    /// Calculate if the update should be rejected: veto head count >= 40% of producers.
    pub fn should_reject(&self, total_producers: usize) -> bool {
        if total_producers == 0 {
            return false;
        }

        let veto_percent = (self.vetos.len() * 100) / total_producers;
        veto_percent >= crate::VETO_THRESHOLD_PERCENT as usize
    }

    /// Get current veto percentage
    pub fn veto_percent(&self, total_producers: usize) -> u8 {
        if total_producers == 0 {
            return 0;
        }
        ((self.vetos.len() * 100) / total_producers) as u8
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
}
