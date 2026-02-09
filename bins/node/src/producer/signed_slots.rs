//! Signed slots tracking database
//!
//! Persists which slots we have signed to prevent double-signing
//! after a quick restart. This is a critical safety measure.

use std::path::Path;
use tracing::{debug, warn};

use super::constants::SIGNED_SLOTS_DB_DIR;
use super::errors::ProducerStartupError;

/// Database for tracking which slots we have signed
///
/// This prevents double-signing if the node is quickly restarted.
/// The database persists across restarts.
pub struct SignedSlotsDb {
    db: sled::Db,
}

impl SignedSlotsDb {
    /// Open the signed slots database
    pub fn open(data_dir: &Path) -> Result<Self, ProducerStartupError> {
        let db_path = data_dir.join(SIGNED_SLOTS_DB_DIR);
        let db = sled::open(&db_path).map_err(|e| {
            ProducerStartupError::SignedSlotsDbFailed(format!(
                "Failed to open {:?}: {}",
                db_path, e
            ))
        })?;

        Ok(Self { db })
    }

    /// Check if we can sign this slot and mark it as signed
    ///
    /// MUST be called BEFORE signing/producing a block.
    /// Returns an error if the slot was already signed.
    ///
    /// This is atomic: if it returns Ok, the slot is now marked as signed.
    pub fn check_and_mark(&self, slot: u64) -> Result<(), ProducerStartupError> {
        let key = slot.to_be_bytes();

        // Atomic check-and-insert using compare-and-swap
        // If key exists, returns the existing value
        // If key doesn't exist, inserts [1] and returns None
        let result = self
            .db
            .compare_and_swap(key, None::<&[u8]>, Some(&[1u8]))
            .map_err(|e| {
                ProducerStartupError::SignedSlotsDbFailed(format!("Database error: {}", e))
            })?;

        match result {
            Ok(()) => {
                // Successfully inserted - we can sign this slot
                debug!("Marked slot {} as signed", slot);

                // CRITICAL: Flush to disk BEFORE returning
                // If we crash after signing but before flush, we might double-sign
                self.db.flush().map_err(|e| {
                    ProducerStartupError::SignedSlotsDbFailed(format!("Flush failed: {}", e))
                })?;

                Ok(())
            }
            Err(_) => {
                // Key already exists - slot was already signed!
                warn!(
                    "BLOCKED: Attempted to sign slot {} which was already signed!",
                    slot
                );
                Err(ProducerStartupError::SlotAlreadySigned { slot })
            }
        }
    }

    /// Check if a slot was already signed (read-only)
    #[allow(dead_code)]
    pub fn was_signed(&self, slot: u64) -> bool {
        let key = slot.to_be_bytes();
        self.db.contains_key(key).unwrap_or(false)
    }

    /// Clear old entries to prevent unbounded growth
    ///
    /// Keeps only slots from the last N slots.
    /// Called periodically during normal operation.
    #[allow(dead_code)]
    pub fn prune(&self, current_slot: u64, keep_slots: u64) -> Result<usize, String> {
        let cutoff = current_slot.saturating_sub(keep_slots);
        let mut removed = 0;

        // Iterate and remove old entries
        for entry in self.db.iter() {
            match entry {
                Ok((key, _)) => {
                    if key.len() == 8 {
                        let slot = u64::from_be_bytes(key.as_ref().try_into().unwrap_or([0; 8]));
                        if slot < cutoff && self.db.remove(&key).is_ok() {
                            removed += 1;
                        }
                    }
                }
                Err(e) => {
                    return Err(format!("Iteration error: {}", e));
                }
            }
        }

        if removed > 0 {
            debug!(
                "Pruned {} old signed slot entries (cutoff slot: {})",
                removed, cutoff
            );
        }

        Ok(removed)
    }

    /// Get the number of tracked slots
    #[allow(dead_code)]
    pub fn count(&self) -> usize {
        self.db.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_signed_slots_prevents_double_sign() {
        let temp_dir = TempDir::new().unwrap();
        let db = SignedSlotsDb::open(temp_dir.path()).unwrap();

        // First sign should succeed
        assert!(db.check_and_mark(100).is_ok());

        // Second sign of same slot should fail
        assert!(matches!(
            db.check_and_mark(100),
            Err(ProducerStartupError::SlotAlreadySigned { slot: 100 })
        ));

        // Different slot should succeed
        assert!(db.check_and_mark(101).is_ok());
    }

    #[test]
    fn test_signed_slots_persists_across_restart() {
        let temp_dir = TempDir::new().unwrap();

        // Sign a slot
        {
            let db = SignedSlotsDb::open(temp_dir.path()).unwrap();
            assert!(db.check_and_mark(200).is_ok());
        }

        // Reopen and verify it's still marked
        {
            let db = SignedSlotsDb::open(temp_dir.path()).unwrap();
            assert!(db.was_signed(200));
            assert!(matches!(
                db.check_and_mark(200),
                Err(ProducerStartupError::SlotAlreadySigned { slot: 200 })
            ));
        }
    }

    #[test]
    fn test_signed_slots_prune() {
        let temp_dir = TempDir::new().unwrap();
        let db = SignedSlotsDb::open(temp_dir.path()).unwrap();

        // Sign several slots
        for slot in 100..110 {
            assert!(db.check_and_mark(slot).is_ok());
        }

        assert_eq!(db.count(), 10);

        // Prune keeping only last 5 slots (current_slot=109, keep=5)
        // Should remove slots 100-103 (slots < 104)
        let removed = db.prune(109, 5).unwrap();
        assert_eq!(removed, 4);
        assert_eq!(db.count(), 6);

        // Verify old slots are gone but recent ones remain
        assert!(!db.was_signed(100));
        assert!(!db.was_signed(103));
        assert!(db.was_signed(104));
        assert!(db.was_signed(109));
    }
}
