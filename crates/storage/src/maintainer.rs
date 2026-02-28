//! Maintainer set persistence — caches the chain-derived MaintainerSet.
//!
//! The MaintainerSet is derived deterministically from the blockchain,
//! but re-deriving from genesis on every restart is wasteful. This module
//! caches the derived set with the height it was derived at, so on restart
//! we only need to replay from `last_derived_height` to chain tip.
//!
//! At 150K nodes this matters: startup time stays constant regardless of
//! chain length because we only replay the delta.

use std::path::{Path, PathBuf};

use doli_core::maintainer::MaintainerSet;
use serde::{Deserialize, Serialize};

use crate::StorageError;

const MAINTAINER_STATE_FILE: &str = "maintainer_state.bin";

/// Cached maintainer set with derivation height.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaintainerState {
    /// The cached maintainer set
    pub set: MaintainerSet,
    /// Block height at which this set was derived
    pub last_derived_height: u64,
}

impl Default for MaintainerState {
    fn default() -> Self {
        Self {
            set: MaintainerSet::new(),
            last_derived_height: 0,
        }
    }
}

impl MaintainerState {
    /// Load cached state from disk, or return default if not found.
    pub fn load(data_dir: &Path) -> Result<Self, StorageError> {
        let path = Self::file_path(data_dir);
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = std::fs::read(&path)?;
        bincode::deserialize(&data).map_err(|e| StorageError::Serialization(e.to_string()))
    }

    /// Save cached state to disk.
    pub fn save(&self, data_dir: &Path) -> Result<(), StorageError> {
        let path = Self::file_path(data_dir);
        let data =
            bincode::serialize(self).map_err(|e| StorageError::Serialization(e.to_string()))?;
        std::fs::write(&path, data)?;
        Ok(())
    }

    /// Update the cached set and persist.
    pub fn update(
        &mut self,
        set: MaintainerSet,
        height: u64,
        data_dir: &Path,
    ) -> Result<(), StorageError> {
        self.set = set;
        self.last_derived_height = height;
        self.save(data_dir)
    }

    fn file_path(data_dir: &Path) -> PathBuf {
        data_dir.join(MAINTAINER_STATE_FILE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_maintainer_state_default() {
        let state = MaintainerState::default();
        assert_eq!(state.last_derived_height, 0);
        assert!(state.set.members.is_empty());
    }

    #[test]
    fn test_maintainer_state_save_load() {
        let dir = tempfile::tempdir().unwrap();

        let state = MaintainerState {
            last_derived_height: 42,
            ..Default::default()
        };
        state.save(dir.path()).unwrap();

        let loaded = MaintainerState::load(dir.path()).unwrap();
        assert_eq!(loaded.last_derived_height, 42);
    }

    #[test]
    fn test_maintainer_state_load_missing() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = MaintainerState::load(dir.path()).unwrap();
        assert_eq!(loaded.last_derived_height, 0);
    }

    #[test]
    fn test_maintainer_state_update() {
        let dir = tempfile::tempdir().unwrap();

        let mut state = MaintainerState::default();
        let set = MaintainerSet::new();
        state.update(set, 100, dir.path()).unwrap();

        let loaded = MaintainerState::load(dir.path()).unwrap();
        assert_eq!(loaded.last_derived_height, 100);
    }
}
