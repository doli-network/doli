//! JSON-based channel persistence (atomic writes).
//!
//! Follows the bridge crate's `SwapStore` pattern: load from JSON,
//! mutate in memory, atomic write (temp file → rename).

use std::path::{Path, PathBuf};

use crate::channel::ChannelRecord;
use crate::error::Result;
use crate::types::ChannelId;

/// File-based channel store.
pub struct ChannelStore {
    path: PathBuf,
    channels: Vec<ChannelRecord>,
}

impl ChannelStore {
    /// Load or create a channel store at the given path.
    pub fn open(path: &Path) -> Result<Self> {
        let channels = if path.exists() {
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
            channels,
        })
    }

    /// Save all channels to disk (atomic write).
    pub fn save(&self) -> Result<()> {
        let data = serde_json::to_string_pretty(&self.channels)?;
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, &data)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// Get all non-terminal channels.
    pub fn active_channels(&self) -> Vec<&ChannelRecord> {
        self.channels.iter().filter(|c| !c.is_terminal()).collect()
    }

    /// Get all channels.
    pub fn all_channels(&self) -> &[ChannelRecord] {
        &self.channels
    }

    /// Find a channel by ID.
    pub fn find(&self, id: &ChannelId) -> Option<&ChannelRecord> {
        self.channels.iter().find(|c| c.channel_id == *id)
    }

    /// Find a channel by ID (mutable).
    pub fn find_mut(&mut self, id: &ChannelId) -> Option<&mut ChannelRecord> {
        self.channels.iter_mut().find(|c| c.channel_id == *id)
    }

    /// Find a channel by funding outpoint.
    pub fn find_by_funding(&self, tx_hash: &[u8; 32], output_index: u32) -> Option<&ChannelRecord> {
        self.channels.iter().find(|c| {
            c.funding_outpoint.tx_hash == *tx_hash
                && c.funding_outpoint.output_index == output_index
        })
    }

    /// Add a new channel record. Returns false if ID already exists.
    pub fn add(&mut self, channel: ChannelRecord) -> bool {
        if self.find(&channel.channel_id).is_some() {
            return false;
        }
        self.channels.push(channel);
        true
    }

    /// Number of active channels.
    pub fn active_count(&self) -> usize {
        self.channels.iter().filter(|c| !c.is_terminal()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::ChannelRecord;
    use crate::types::{ChannelBalance, ChannelId, ChannelState, FundingOutpoint};
    use tempfile::TempDir;

    fn make_test_channel(id_byte: u8) -> ChannelRecord {
        ChannelRecord {
            channel_id: ChannelId([id_byte; 32]),
            state: ChannelState::Active,
            local_pubkey_hash: [1u8; 32],
            remote_pubkey_hash: [2u8; 32],
            funding_outpoint: FundingOutpoint {
                tx_hash: [id_byte; 32],
                output_index: 0,
            },
            capacity: 1_000_000,
            balance: ChannelBalance::new(500_000, 500_000),
            commitment_number: 0,
            channel_seed: [0u8; 32],
            revocation_store: Default::default(),
            dispute_window: 144,
            htlcs: Vec::new(),
            funding_confirmations: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            close_tx_hash: None,
            penalty_tx_hash: None,
        }
    }

    #[test]
    fn store_open_creates_empty() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("channels.json");
        let store = ChannelStore::open(&path).unwrap();
        assert_eq!(store.all_channels().len(), 0);
    }

    #[test]
    fn store_save_and_reload() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("channels.json");

        {
            let mut store = ChannelStore::open(&path).unwrap();
            store.add(make_test_channel(1));
            store.add(make_test_channel(2));
            store.save().unwrap();
        }

        let store = ChannelStore::open(&path).unwrap();
        assert_eq!(store.all_channels().len(), 2);
    }

    #[test]
    fn store_rejects_duplicate_id() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("channels.json");
        let mut store = ChannelStore::open(&path).unwrap();

        assert!(store.add(make_test_channel(1)));
        assert!(!store.add(make_test_channel(1))); // duplicate
    }

    #[test]
    fn store_find_by_funding() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("channels.json");
        let mut store = ChannelStore::open(&path).unwrap();
        store.add(make_test_channel(1));

        assert!(store.find_by_funding(&[1u8; 32], 0).is_some());
        assert!(store.find_by_funding(&[1u8; 32], 1).is_none());
        assert!(store.find_by_funding(&[9u8; 32], 0).is_none());
    }
}
