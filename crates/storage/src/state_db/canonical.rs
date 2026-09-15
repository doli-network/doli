//! `cf_utxo` as canonical rows: a pinned RocksDB view for the streaming fold.

use crate::utxo::canonical::{self, CanonicalRows, EmitEntry, OnUndecodable};
use crate::utxo::UtxoEntry;
use crate::StorageError;

use super::types::{StateDb, CF_UTXO};

/// One `cf_utxo` view, pinned by a RocksDB snapshot.
///
/// The snapshot is what lets the fold count rows and then hash them without the
/// two passes seeing different sets.
pub(crate) struct UtxoRows<'a> {
    snapshot: rocksdb::Snapshot<'a>,
    cf: &'a rocksdb::ColumnFamily,
    hint: usize,
}

impl StateDb {
    /// Pin `cf_utxo` for one canonical fold.
    pub(crate) fn utxo_rows(&self) -> UtxoRows<'_> {
        UtxoRows {
            snapshot: self.db.snapshot(),
            cf: self.db.cf_handle(CF_UTXO).unwrap(),
            hint: self.utxo_len(),
        }
    }

    /// Produce canonical UTXO bytes for state root computation.
    ///
    /// RocksDB iterates in lexicographic key order — no sorting needed.
    /// Values are re-encoded to the canonical 61-byte-base format.
    pub fn serialize_canonical_utxo(&self) -> Vec<u8> {
        canonical::materialize(&self.utxo_rows())
    }
}

impl CanonicalRows for UtxoRows<'_> {
    fn size_hint(&self) -> usize {
        self.hint
    }

    fn row_count(&self) -> Result<u64, StorageError> {
        let mut iter = self.snapshot.raw_iterator_cf(self.cf);
        iter.seek_to_first();
        let mut count = 0u64;
        while iter.valid() {
            count += 1;
            iter.next();
        }
        iter.status()
            .map_err(|e| StorageError::Database(format!("[STOR033] cf_utxo iterator: {e}")))?;
        Ok(count)
    }

    fn for_each_entry(
        &self,
        start: Option<&[u8]>,
        policy: OnUndecodable,
        f: EmitEntry<'_>,
    ) -> Result<(), StorageError> {
        let mode = match start {
            Some(key) => rocksdb::IteratorMode::From(key, rocksdb::Direction::Forward),
            None => rocksdb::IteratorMode::Start,
        };

        for row in self.snapshot.iterator_cf(self.cf, mode) {
            let (key, raw) = row
                .map_err(|e| StorageError::Database(format!("[STOR033] cf_utxo iterator: {e}")))?;
            match bincode::deserialize::<UtxoEntry>(&raw) {
                Ok(entry) => {
                    if f(&key, &entry).is_break() {
                        break;
                    }
                }
                Err(e) if policy == OnUndecodable::Skip => {
                    tracing::debug!("[CF_UTXO] skipping undecodable value: {}", e);
                }
                Err(e) => {
                    return Err(StorageError::Serialization(format!(
                        "[STOR034] undecodable cf_utxo value at key {}: {e}",
                        key.iter().map(|b| format!("{b:02x}")).collect::<String>()
                    )))
                }
            }
        }
        Ok(())
    }
}
