//! ProducerSet persistence: load, save, serialize_canonical

use std::path::Path;

use crypto::Hash;
use doli_core::consensus::BOND_UNIT as CORE_BOND_UNIT;

use super::types::{ProducerInfo, ProducerSet};
use crate::StorageError;

impl ProducerSet {
    /// Load producer set from file.
    ///
    /// Tries JSON first (current format), then bincode (legacy), then starts fresh.
    /// This ensures backward compatibility across version upgrades — bincode is
    /// positional and breaks when fields are added, while JSON with `#[serde(default)]`
    /// handles missing fields gracefully.
    pub fn load(path: &Path) -> Result<Self, StorageError> {
        if !path.exists() {
            return Ok(Self::new());
        }

        let data = std::fs::read(path)?;

        // Try JSON first (current format)
        if let Ok(mut set) = serde_json::from_slice::<Self>(&data) {
            set.rebuild_unbonding_index();
            set.migrate_bond_entries(CORE_BOND_UNIT);
            return Ok(set);
        }

        // Try bincode (legacy format from older versions)
        if let Ok(mut set) = bincode::deserialize::<Self>(&data) {
            tracing::info!(
                "Migrated producers.bin from bincode to JSON ({} producers)",
                set.total_count()
            );
            set.rebuild_unbonding_index();
            set.migrate_bond_entries(CORE_BOND_UNIT);
            // Re-save as JSON so future loads use the new format
            if let Err(e) = set.save(path) {
                tracing::warn!("Failed to re-save migrated producer set as JSON: {}", e);
            }
            return Ok(set);
        }

        tracing::warn!("Could not deserialize producers.bin (JSON or bincode), starting fresh");
        Ok(Self::new())
    }

    /// Migrate all producers that have empty bond_entries.
    pub fn migrate_bond_entries(&mut self, bond_unit: u64) {
        for info in self.producers.values_mut() {
            info.migrate_bond_entries(bond_unit);
        }
    }

    /// Save producer set to file (atomic: write to temp file, then rename).
    ///
    /// Uses JSON format — backward compatible with `#[serde(default)]` fields,
    /// so future version upgrades won't break deserialization.
    pub fn save(&self, path: &Path) -> Result<(), StorageError> {
        let data =
            serde_json::to_vec(self).map_err(|e| StorageError::Serialization(e.to_string()))?;
        let tmp = path.with_extension("bin.tmp");
        std::fs::write(&tmp, &data)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Serialize the producer set in canonical (deterministic) order.
    ///
    /// HashMap iteration order is non-deterministic in Rust. This method
    /// sorts entries by key (Hash) to produce identical bytes on every node
    /// for the same logical state. Used for state root computation.
    ///
    /// Format: `[8-byte LE producer_count][sorted (key, info) pairs][8-byte LE exit_count][sorted (key, height) pairs]`
    pub fn serialize_canonical(&self) -> Vec<u8> {
        let mut buf = Vec::new();

        // Producers: sort by key hash for deterministic order
        let mut producers: Vec<(&Hash, &ProducerInfo)> = self.producers.iter().collect();
        producers.sort_by_key(|(k, _)| *k);

        buf.extend_from_slice(&(producers.len() as u64).to_le_bytes());
        for (key, info) in &producers {
            buf.extend_from_slice(key.as_bytes());
            // Clone and sort Vec fields for deterministic serialization.
            // Insertion order should already be consistent (same chain = same order),
            // but explicit sorting is defense-in-depth against non-determinism.
            let mut info_sorted = (*info).clone();
            info_sorted.additional_bonds.sort_by_key(|(h, i)| (*h, *i));
            info_sorted
                .received_delegations
                .sort_by_key(|(h, c)| (*h, *c));
            info_sorted.bond_entries.sort_by_key(|e| e.creation_slot);
            let info_bytes = bincode::serialize(&info_sorted).unwrap_or_default();
            buf.extend_from_slice(&(info_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(&info_bytes);
        }

        // Exit history: sort by key hash for deterministic order
        let mut exits: Vec<(&Hash, &u64)> = self.exit_history.iter().collect();
        exits.sort_by_key(|(k, _)| *k);

        buf.extend_from_slice(&(exits.len() as u64).to_le_bytes());
        for (key, height) in &exits {
            buf.extend_from_slice(key.as_bytes());
            buf.extend_from_slice(&height.to_le_bytes());
        }

        buf
    }
}
