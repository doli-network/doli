//! Persistent swap database (JSON file).
//!
//! Simple file-based storage for active swaps. Writes are atomic
//! (write to temp file, then rename).

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::swap::SwapRecord;

/// File-based swap store.
pub struct SwapStore {
    path: PathBuf,
    swaps: Vec<SwapRecord>,
}

impl SwapStore {
    /// Load or create a swap store at the given path.
    pub fn open(path: &Path) -> Result<Self> {
        let swaps = if path.exists() {
            let data = std::fs::read_to_string(path)?;
            if data.trim().is_empty() {
                Vec::new()
            } else {
                serde_json::from_str(&data)?
            }
        } else {
            Vec::new()
        };
        Ok(Self {
            path: path.to_path_buf(),
            swaps,
        })
    }

    /// Save all swaps to disk (atomic write).
    pub fn save(&self) -> Result<()> {
        let data = serde_json::to_string_pretty(&self.swaps)?;
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, &data)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// Get all active (non-terminal) swaps.
    pub fn active_swaps(&self) -> Vec<&SwapRecord> {
        self.swaps.iter().filter(|s| !s.is_terminal()).collect()
    }

    /// Get all swaps.
    pub fn all_swaps(&self) -> &[SwapRecord] {
        &self.swaps
    }

    /// Find a swap by ID.
    pub fn find(&self, id: &str) -> Option<&SwapRecord> {
        self.swaps.iter().find(|s| s.id == id)
    }

    /// Find a swap by ID (mutable).
    pub fn find_mut(&mut self, id: &str) -> Option<&mut SwapRecord> {
        self.swaps.iter_mut().find(|s| s.id == id)
    }

    /// Find a swap by DOLI hash.
    pub fn find_by_doli_hash(&self, hash: &str) -> Option<&SwapRecord> {
        self.swaps.iter().find(|s| s.doli_hash == hash)
    }

    /// Add a new swap record. Returns false if ID already exists.
    pub fn add(&mut self, swap: SwapRecord) -> bool {
        if self.find(&swap.id).is_some() {
            return false;
        }
        self.swaps.push(swap);
        true
    }

    /// Number of active swaps.
    pub fn active_count(&self) -> usize {
        self.swaps.iter().filter(|s| !s.is_terminal()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swap::{SwapRole, SwapState};
    use tempfile::NamedTempFile;

    #[test]
    fn test_store_roundtrip() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        // Create and save
        {
            let mut store = SwapStore::open(&path).unwrap();
            let swap = SwapRecord::new(
                "abc123".to_string(),
                0,
                1000,
                "hash123".to_string(),
                100,
                200,
                "creator".to_string(),
                1,
                "bc1qtest".to_string(),
                SwapRole::Initiator,
            );
            assert!(store.add(swap));
            store.save().unwrap();
        }

        // Reload and verify
        {
            let store = SwapStore::open(&path).unwrap();
            assert_eq!(store.all_swaps().len(), 1);
            assert_eq!(store.all_swaps()[0].doli_tx_hash, "abc123");
            assert_eq!(store.all_swaps()[0].state, SwapState::DoliLocked);
        }
    }

    #[test]
    fn test_no_duplicate_ids() {
        let tmp = NamedTempFile::new().unwrap();
        let mut store = SwapStore::open(tmp.path()).unwrap();

        let swap1 = SwapRecord::new(
            "abc".to_string(),
            0,
            100,
            "h".to_string(),
            1,
            2,
            "c".to_string(),
            1,
            "addr".to_string(),
            SwapRole::Initiator,
        );
        let swap2 = SwapRecord::new(
            "abc".to_string(),
            0,
            200,
            "h2".to_string(),
            1,
            2,
            "c".to_string(),
            1,
            "addr".to_string(),
            SwapRole::Initiator,
        );

        assert!(store.add(swap1));
        assert!(!store.add(swap2)); // duplicate ID
        assert_eq!(store.all_swaps().len(), 1);
    }
}
