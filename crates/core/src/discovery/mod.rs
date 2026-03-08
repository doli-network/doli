//! Producer Discovery Module
//!
//! This module implements a production-grade producer discovery system using:
//! - Cryptographically signed `ProducerAnnouncement` with replay protection
//! - G-Set CRDT (`ProducerGSet`) for eventual consistency
//! - Bloom filter for efficient delta synchronization
//! - Adaptive gossip intervals based on network conditions
//!
//! ## Architecture
//!
//! ```text
//! ProducerAnnouncement (signed, versioned)
//!         │
//!         ▼
//! ProducerGSet (CRDT with version vectors)
//!         │
//!         ├──▶ Persistence (disk storage with re-verification)
//!         │
//!         └──▶ ProducerBloomFilter (delta sync optimization)
//!                     │
//!                     ▼
//!              AdaptiveGossip (smart interval control)
//! ```

mod announcement;
mod bloom;
mod gossip;
mod gset;
pub mod proto;
pub mod snapshot;

pub use announcement::ProducerAnnouncement;
pub use bloom::ProducerBloomFilter;
pub use gossip::AdaptiveGossip;
pub use gset::{MergeOneResult, ProducerGSet};
pub use proto::{
    decode_announcement, decode_digest, decode_producer_set, encode_announcement, encode_digest,
    encode_producer_set, is_legacy_bincode_format, ProtoError,
};
pub use snapshot::EpochSnapshot;

use thiserror::Error;

/// Domain separation tag for producer announcement signing.
pub const PRODUCER_ANNOUNCEMENT_DOMAIN: &[u8] = b"DOLI_PRODUCER_ANN_V1";

/// Maximum age of an announcement in seconds (1 hour).
pub const MAX_ANNOUNCEMENT_AGE_SECS: u64 = 3600;

/// Maximum future timestamp tolerance in seconds (5 minutes).
pub const MAX_FUTURE_TIMESTAMP_SECS: u64 = 300;

/// Errors that can occur during producer set operations.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProducerSetError {
    /// Announcement signature verification failed.
    #[error("invalid signature: announcement signature verification failed")]
    InvalidSignature,

    /// Announcement timestamp is too old (> 1 hour).
    #[error("stale announcement: timestamp is more than 1 hour old")]
    StaleAnnouncement,

    /// Announcement timestamp is too far in the future (> 5 minutes).
    #[error("future timestamp: announcement timestamp is more than 5 minutes in the future")]
    FutureTimestamp,

    /// Announcement is for a different network.
    #[error("network mismatch: expected network {expected}, got {got}")]
    NetworkMismatch {
        /// Expected network ID.
        expected: u32,
        /// Received network ID.
        got: u32,
    },

    /// Announcement is for a different genesis (same network, different chain).
    #[error("genesis hash mismatch: announcement signed for different genesis")]
    GenesisHashMismatch,

    /// Announcement has an older sequence number than currently known.
    #[error("sequence regression: current sequence is {current}, received {received}")]
    SequenceRegression {
        /// Current known sequence number.
        current: u64,
        /// Received sequence number.
        received: u64,
    },
}

/// Result of merging announcements into a producer set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeResult {
    /// Number of announcements successfully merged (new producers + sequence updates).
    pub added: usize,
    /// Number of truly new producers added (not seen before).
    pub new_producers: usize,
    /// Number of invalid/stale announcements rejected.
    pub rejected: usize,
    /// Number of already known announcements (duplicates or older sequence).
    pub duplicates: usize,
}

impl MergeResult {
    /// Returns true if no announcements were processed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added == 0 && self.rejected == 0 && self.duplicates == 0
    }

    /// Returns the total number of announcements processed.
    #[must_use]
    pub fn total(&self) -> usize {
        self.added + self.rejected + self.duplicates
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display_messages() {
        let err = ProducerSetError::InvalidSignature;
        assert!(err.to_string().contains("signature"));

        let err = ProducerSetError::StaleAnnouncement;
        assert!(
            err.to_string().contains("old") || err.to_string().contains("stale"),
            "Error message should contain 'old' or 'stale': {}",
            err
        );

        let err = ProducerSetError::FutureTimestamp;
        assert!(err.to_string().contains("future"));

        let err = ProducerSetError::NetworkMismatch {
            expected: 1,
            got: 2,
        };
        let msg = err.to_string();
        assert!(msg.contains('1') && msg.contains('2'));

        let err = ProducerSetError::SequenceRegression {
            current: 5,
            received: 3,
        };
        let msg = err.to_string();
        assert!(msg.contains('5') && msg.contains('3'));
    }

    #[test]
    fn test_error_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ProducerSetError>();
    }

    #[test]
    fn test_merge_result_default() {
        let result = MergeResult::default();
        assert_eq!(result.added, 0);
        assert_eq!(result.rejected, 0);
        assert!(result.is_empty());
    }

    #[test]
    fn test_merge_result_total() {
        let result = MergeResult {
            added: 5,
            new_producers: 3,
            rejected: 2,
            duplicates: 3,
        };
        assert_eq!(result.total(), 10);
        assert!(!result.is_empty());
    }
}
