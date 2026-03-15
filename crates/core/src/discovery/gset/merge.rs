//! Merge operations for ProducerGSet.
//!
//! Contains the core CRDT merge logic: single-announcement merge with
//! cryptographic verification and batch merge with DoS protection.

use std::time::Instant;

use tracing::{info, warn};

use super::super::{
    MergeResult, ProducerAnnouncement, ProducerSetError, MAX_ANNOUNCEMENT_AGE_SECS,
    MAX_FUTURE_TIMESTAMP_SECS,
};
use super::{MergeOneResult, ProducerGSet};

impl ProducerGSet {
    /// Merge a single announcement into the set.
    ///
    /// Returns:
    /// - `Ok(MergeOneResult::NewProducer)` if a new producer was added
    /// - `Ok(MergeOneResult::SequenceUpdate)` if an existing producer's sequence was updated
    /// - `Ok(MergeOneResult::Duplicate)` if it was a duplicate or older sequence
    /// - `Err(_)` if validation failed
    ///
    /// # Validation Steps
    ///
    /// 1. Verify signature
    /// 2. Check network_id matches
    /// 3. Check timestamp bounds (not stale, not future)
    /// 4. Check sequence is newer than known
    ///
    /// # Arguments
    ///
    /// * `announcement` - The producer announcement to merge
    ///
    /// # Errors
    ///
    /// - `InvalidSignature` - Signature verification failed
    /// - `NetworkMismatch` - Wrong network ID
    /// - `StaleAnnouncement` - Timestamp > 1 hour old
    /// - `FutureTimestamp` - Timestamp > 5 minutes in future
    pub fn merge_one(
        &mut self,
        announcement: ProducerAnnouncement,
    ) -> Result<MergeOneResult, ProducerSetError> {
        let pubkey_hex = hex::encode(&announcement.pubkey.as_bytes()[..8]);

        // Step 1: Verify signature
        if !announcement.verify() {
            info!("GSet merge_one {}: REJECT invalid_signature", pubkey_hex);
            return Err(ProducerSetError::InvalidSignature);
        }

        // Step 2: Check network ID
        if announcement.network_id != self.network_id {
            return Err(ProducerSetError::NetworkMismatch {
                expected: self.network_id,
                got: announcement.network_id,
            });
        }

        // Step 2b: Check genesis hash (prevents cross-genesis contamination)
        if announcement.genesis_hash != self.genesis_hash {
            info!(
                "GSet merge_one {}: REJECT genesis_hash_mismatch expected={} got={}",
                pubkey_hex,
                hex::encode(&self.genesis_hash.as_bytes()[..8]),
                hex::encode(&announcement.genesis_hash.as_bytes()[..8]),
            );
            return Err(ProducerSetError::GenesisHashMismatch);
        }

        // Step 3: Check timestamp bounds
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_secs();

        // Check for stale announcement (> 1 hour old)
        if announcement.timestamp + MAX_ANNOUNCEMENT_AGE_SECS < now {
            info!(
                "GSet merge_one {}: REJECT stale ts={} now={} age={}s",
                pubkey_hex,
                announcement.timestamp,
                now,
                now.saturating_sub(announcement.timestamp)
            );
            return Err(ProducerSetError::StaleAnnouncement);
        }

        // Check for future timestamp (> 5 minutes ahead)
        if announcement.timestamp > now + MAX_FUTURE_TIMESTAMP_SECS {
            return Err(ProducerSetError::FutureTimestamp);
        }

        // Step 4: Check sequence version vector
        let is_new_producer = !self.sequences.contains_key(&announcement.pubkey);
        if let Some(&current_sequence) = self.sequences.get(&announcement.pubkey) {
            // If we already have a higher or equal sequence, this is a duplicate
            if announcement.sequence <= current_sequence {
                info!(
                    "GSet merge_one {}: DUPLICATE incoming_seq={} stored_seq={} ts={}",
                    pubkey_hex, announcement.sequence, current_sequence, announcement.timestamp
                );
                return Ok(MergeOneResult::Duplicate);
            }
        }

        info!(
            "GSet merge_one {}: {} incoming_seq={} stored_seq={:?} ts={}",
            pubkey_hex,
            if is_new_producer { "NEW" } else { "SEQ_UPDATE" },
            announcement.sequence,
            self.sequences.get(&announcement.pubkey),
            announcement.timestamp
        );

        // All checks passed - merge the announcement
        self.sequences
            .insert(announcement.pubkey, announcement.sequence);
        self.producers.insert(announcement.pubkey, announcement);
        self.last_modified = Instant::now();

        // Auto-persist if configured
        if self.storage_path.is_some() {
            if let Err(e) = self.persist_to_disk() {
                warn!("Failed to persist producer gset: {}", e);
            }
        }

        if is_new_producer {
            Ok(MergeOneResult::NewProducer)
        } else {
            Ok(MergeOneResult::SequenceUpdate)
        }
    }

    /// Merge multiple announcements into the set.
    ///
    /// Returns statistics about the merge operation.
    ///
    /// # Arguments
    ///
    /// * `announcements` - A vector of announcements to merge
    #[must_use]
    pub fn merge(&mut self, announcements: Vec<ProducerAnnouncement>) -> MergeResult {
        let mut result = MergeResult::default();
        // Rate limit: bail early if too many consecutive rejections (DoS protection)
        let max_rejections = 50;
        let mut consecutive_rejections: u32 = 0;
        let total_count = announcements.len();

        for ann in announcements {
            let pubkey_prefix = hex::encode(&ann.pubkey.as_bytes()[..4]);
            match self.merge_one(ann) {
                Ok(MergeOneResult::NewProducer) => {
                    result.added += 1;
                    result.new_producers += 1;
                    consecutive_rejections = 0;
                }
                Ok(MergeOneResult::SequenceUpdate) => {
                    result.added += 1;
                    consecutive_rejections = 0;
                }
                Ok(MergeOneResult::Duplicate) => {
                    result.duplicates += 1;
                    consecutive_rejections = 0;
                }
                Err(e) => {
                    consecutive_rejections += 1;
                    if consecutive_rejections >= max_rejections {
                        warn!(
                            "GSet merge: aborting after {} consecutive rejections (DoS protection)",
                            max_rejections
                        );
                        result.rejected +=
                            total_count - result.added - result.duplicates - result.rejected;
                        break;
                    }
                    warn!("GSet rejected announcement from {}: {}", pubkey_prefix, e);
                    result.rejected += 1;
                }
            }
        }

        result
    }
}
