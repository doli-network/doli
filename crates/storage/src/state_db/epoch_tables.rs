//! INC-I-289 L-2: the epoch table of each recent boundary. Local only: `cf_meta`,
//! never part of the state root. Key = `epoch_table:` ++ epoch start (8B BE).

use crypto::{Hash, PublicKey};
use serde::{Deserialize, Serialize};

use super::types::{StateDb, CF_META};
use crate::producer::ProducerSet;

const PREFIX: &[u8] = b"epoch_table:";

/// The weighing inputs of one epoch, frozen when its opening boundary applied.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochTable {
    pub epoch_start: u64,
    /// The opening boundary block; `None` when the node does not hold it.
    pub boundary_hash: Option<Hash>,
    /// The bit decode list (`EpochState::producer_list`).
    pub producer_list: Vec<PublicKey>,
    /// `selection_weight_at(epoch_start, ..)` per producer.
    pub bonds: Vec<(PublicKey, u64)>,
    /// The applied ProducerSet (bincode): BLS keys, status, the active set.
    pub producers: Vec<u8>,
}

impl EpochTable {
    pub fn new(
        epoch_start: u64,
        boundary_hash: Option<Hash>,
        producer_list: Vec<PublicKey>,
        set: &ProducerSet,
        bond_of: impl Fn(&crate::producer::ProducerInfo) -> u64,
    ) -> Self {
        let mut bonds: Vec<(PublicKey, u64)> = (set.all_producers().iter())
            .map(|p| (p.public_key, bond_of(p)))
            .collect();
        bonds.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        Self {
            epoch_start,
            boundary_hash,
            producer_list,
            bonds,
            producers: bincode::serialize(set).unwrap_or_default(),
        }
    }

    pub fn producer_set(&self) -> Option<ProducerSet> {
        ProducerSet::from_snapshot_bytes(&self.producers).ok()
    }

    /// A digest of the whole table (restart and snap checks).
    pub fn digest(&self) -> Hash {
        crypto::hash::hash(&bincode::serialize(self).unwrap_or_default())
    }
}

fn key(start: u64) -> Vec<u8> {
    [PREFIX, &start.to_be_bytes()[..]].concat()
}

impl StateDb {
    pub fn put_epoch_table(&self, table: &EpochTable) {
        let cf = self.db.cf_handle(CF_META).unwrap();
        if let Ok(bytes) = bincode::serialize(table) {
            let _ = self.db.put_cf(cf, key(table.epoch_start), bytes);
        }
    }

    pub fn get_epoch_table(&self, epoch_start: u64) -> Option<EpochTable> {
        let cf = self.db.cf_handle(CF_META).unwrap();
        let bytes = self.db.get_cf(cf, key(epoch_start)).ok()??;
        bincode::deserialize(&bytes).ok()
    }

    /// Stored epoch starts, ascending.
    pub fn epoch_table_starts(&self) -> Vec<u64> {
        let cf = self.db.cf_handle(CF_META).unwrap();
        let mode = rocksdb::IteratorMode::From(PREFIX, rocksdb::Direction::Forward);
        (self.db.iterator_cf(cf, mode))
            .map_while(|row| row.ok())
            .take_while(|(k, _)| k.starts_with(PREFIX))
            .filter_map(|(k, _)| Some(u64::from_be_bytes(k[PREFIX.len()..].try_into().ok()?)))
            .collect()
    }

    pub fn delete_epoch_table(&self, epoch_start: u64) {
        let cf = self.db.cf_handle(CF_META).unwrap();
        let _ = self.db.delete_cf(cf, key(epoch_start));
    }

    /// Drops every table older than `keep_from`, but never one of the `min_keep` newest.
    pub fn prune_epoch_tables(&self, keep_from: u64, min_keep: usize) {
        let starts = self.epoch_table_starts();
        let protected = starts.len().saturating_sub(min_keep);
        for s in starts
            .into_iter()
            .take(protected)
            .filter(|s| *s < keep_from)
        {
            self.delete_epoch_table(s);
        }
    }

    pub fn clear_epoch_tables(&self) {
        for s in self.epoch_table_starts() {
            self.delete_epoch_table(s);
        }
    }
}
