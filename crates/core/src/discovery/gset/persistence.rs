//! Disk persistence for ProducerGSet.
//!
//! Handles atomic writes and re-verified loads for producer announcement data.

use std::io::{self, Write};

use tracing::warn;

use super::ProducerGSet;

impl ProducerGSet {
    /// Persist the current state to disk.
    ///
    /// Uses atomic write (write to temp file, then rename) to prevent corruption.
    /// This is a no-op if no storage path was configured.
    ///
    /// # Errors
    ///
    /// Returns an IO error if the write fails.
    pub fn persist_to_disk(&self) -> io::Result<()> {
        let Some(ref path) = self.storage_path else {
            return Ok(());
        };

        // Serialize all announcements
        let announcements: Vec<_> = self.producers.values().cloned().collect();
        let data = bincode::serialize(&announcements)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        // Write to temp file first (atomic write pattern)
        let temp_path = path.with_extension("tmp");
        let mut file = std::fs::File::create(&temp_path)?;
        file.write_all(&data)?;
        file.sync_all()?;

        // Atomic rename
        std::fs::rename(&temp_path, path)?;

        Ok(())
    }

    /// Load state from disk, re-verifying all signatures.
    ///
    /// Only announcements that pass verification are loaded.
    /// This is called automatically by `new_with_persistence`.
    pub(super) fn load_from_disk(&mut self) {
        let Some(ref path) = self.storage_path else {
            return;
        };

        if !path.exists() {
            return;
        }

        // Read and deserialize
        let data = match std::fs::read(path) {
            Ok(data) => data,
            Err(e) => {
                warn!("Failed to read producer gset from disk: {}", e);
                return;
            }
        };

        let announcements: Vec<super::super::ProducerAnnouncement> =
            match bincode::deserialize(&data) {
                Ok(anns) => anns,
                Err(e) => {
                    warn!("Failed to deserialize producer gset: {}", e);
                    return;
                }
            };

        // Re-verify each announcement and add only valid ones
        // Note: We skip timestamp checks during load since these are persisted announcements
        let mut loaded = 0;
        let mut rejected = 0;

        for ann in announcements {
            // Verify signature
            if !ann.verify() {
                rejected += 1;
                continue;
            }

            // Check network ID
            if ann.network_id != self.network_id {
                rejected += 1;
                continue;
            }

            // Check genesis hash
            if ann.genesis_hash != self.genesis_hash {
                rejected += 1;
                continue;
            }

            // Add to state (no timestamp check for persisted data)
            if let Some(&current_seq) = self.sequences.get(&ann.pubkey) {
                if ann.sequence <= current_seq {
                    continue; // Skip older sequences
                }
            }

            self.sequences.insert(ann.pubkey, ann.sequence);
            self.producers.insert(ann.pubkey, ann);
            loaded += 1;
        }

        if rejected > 0 {
            warn!(
                "Loaded {} producers from disk, rejected {} invalid",
                loaded, rejected
            );
        }
    }
}
