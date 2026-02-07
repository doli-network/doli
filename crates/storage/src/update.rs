//! Update state persistence — survives restarts.
//!
//! Stores pending releases, vote state, and update history so that
//! a node restart doesn't lose in-progress votes or pending updates.
//!
//! Uses simple file-based persistence (bincode) consistent with
//! ProducerSet and MaintainerState in this crate.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::StorageError;

const UPDATE_STATE_FILE: &str = "update_state.bin";

/// Persisted vote record for a single producer on a specific version.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedVote {
    /// Producer public key (hex)
    pub producer_id: String,
    /// "veto" or "approve"
    pub vote: String,
    /// Unix timestamp when vote was cast
    pub timestamp: u64,
    /// Vote weight (u64, scaled by 100 for 2-decimal precision)
    pub weight: u64,
}

/// Persisted release info (subset of updater::Release for storage).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedRelease {
    pub version: String,
    pub binary_sha256: String,
    pub published_at: u64,
    pub changelog: String,
}

/// Record of an applied update.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateHistoryEntry {
    pub version: String,
    pub applied_at: u64,
    /// "applied", "rolled_back", "vetoed"
    pub outcome: String,
}

/// Complete persisted update state.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UpdateState {
    /// Pending release (if any) — version → release info
    pub pending_releases: HashMap<String, PersistedRelease>,
    /// Votes per version — version → list of votes
    pub votes: HashMap<String, Vec<PersistedVote>>,
    /// History of applied/rolled-back updates (most recent last)
    pub history: Vec<UpdateHistoryEntry>,
}

impl UpdateState {
    /// Load state from disk, or return default if not found.
    pub fn load(data_dir: &Path) -> Result<Self, StorageError> {
        let path = Self::file_path(data_dir);
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = std::fs::read(&path)?;
        bincode::deserialize(&data).map_err(|e| StorageError::Serialization(e.to_string()))
    }

    /// Save state to disk.
    pub fn save(&self, data_dir: &Path) -> Result<(), StorageError> {
        let path = Self::file_path(data_dir);
        let data =
            bincode::serialize(self).map_err(|e| StorageError::Serialization(e.to_string()))?;
        std::fs::write(&path, data)?;
        Ok(())
    }

    /// Add a pending release.
    pub fn add_pending_release(&mut self, release: PersistedRelease) {
        self.pending_releases
            .insert(release.version.clone(), release);
    }

    /// Remove a pending release (after apply, veto, or expiry).
    pub fn remove_pending_release(&mut self, version: &str) {
        self.pending_releases.remove(version);
    }

    /// Record a vote for a version.
    pub fn record_vote(&mut self, version: &str, vote: PersistedVote) {
        self.votes
            .entry(version.to_string())
            .or_default()
            .push(vote);
    }

    /// Get votes for a version.
    pub fn get_votes(&self, version: &str) -> &[PersistedVote] {
        self.votes.get(version).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Record an update outcome in history.
    pub fn record_history(&mut self, entry: UpdateHistoryEntry) {
        self.history.push(entry);
    }

    /// Clear votes for a version (after resolution).
    pub fn clear_votes(&mut self, version: &str) {
        self.votes.remove(version);
    }

    fn file_path(data_dir: &Path) -> PathBuf {
        data_dir.join(UPDATE_STATE_FILE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_state_default() {
        let state = UpdateState::default();
        assert!(state.pending_releases.is_empty());
        assert!(state.votes.is_empty());
        assert!(state.history.is_empty());
    }

    #[test]
    fn test_update_state_save_load() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = UpdateState::default();

        state.add_pending_release(PersistedRelease {
            version: "1.0.1".into(),
            binary_sha256: "abc123".into(),
            published_at: 1000,
            changelog: "fix bug".into(),
        });

        state.record_vote(
            "1.0.1",
            PersistedVote {
                producer_id: "prod1".into(),
                vote: "veto".into(),
                timestamp: 1001,
                weight: 400,
            },
        );

        state.save(dir.path()).unwrap();

        let loaded = UpdateState::load(dir.path()).unwrap();
        assert_eq!(loaded.pending_releases.len(), 1);
        assert!(loaded.pending_releases.contains_key("1.0.1"));
        assert_eq!(loaded.get_votes("1.0.1").len(), 1);
        assert_eq!(loaded.get_votes("1.0.1")[0].producer_id, "prod1");
    }

    #[test]
    fn test_update_state_load_missing() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = UpdateState::load(dir.path()).unwrap();
        assert!(loaded.pending_releases.is_empty());
    }

    #[test]
    fn test_update_state_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = UpdateState::default();

        state.record_history(UpdateHistoryEntry {
            version: "1.0.0".into(),
            applied_at: 500,
            outcome: "applied".into(),
        });
        state.record_history(UpdateHistoryEntry {
            version: "1.0.1".into(),
            applied_at: 1000,
            outcome: "rolled_back".into(),
        });

        state.save(dir.path()).unwrap();
        let loaded = UpdateState::load(dir.path()).unwrap();
        assert_eq!(loaded.history.len(), 2);
        assert_eq!(loaded.history[1].outcome, "rolled_back");
    }

    #[test]
    fn test_update_state_remove_release() {
        let mut state = UpdateState::default();
        state.add_pending_release(PersistedRelease {
            version: "2.0.0".into(),
            binary_sha256: "def".into(),
            published_at: 2000,
            changelog: "major".into(),
        });
        assert_eq!(state.pending_releases.len(), 1);

        state.remove_pending_release("2.0.0");
        assert!(state.pending_releases.is_empty());
    }

    #[test]
    fn test_update_state_clear_votes() {
        let mut state = UpdateState::default();
        state.record_vote(
            "1.0.1",
            PersistedVote {
                producer_id: "p1".into(),
                vote: "approve".into(),
                timestamp: 100,
                weight: 200,
            },
        );
        assert_eq!(state.get_votes("1.0.1").len(), 1);

        state.clear_votes("1.0.1");
        assert_eq!(state.get_votes("1.0.1").len(), 0);
    }
}
