//! Staged-install promotion: bounded sub-batches into the live UTXO family.
//!
//! Decision 136. This path runs only under an armed `rebuild_in_progress`
//! marker, which is what makes a partial write safe; rollback and reorg replay
//! keep the single-batch `atomic_replace` because they arm no marker.

use std::sync::atomic::Ordering;

use crate::chain_state::ChainState;
use crate::producer::ProducerSet;
use crate::utxo::{Outpoint, UtxoEntry};
use crate::StorageError;

use super::types::{
    LastApplied, StateDb, CF_EXIT_HISTORY, CF_META, CF_PRODUCERS, CF_UTXO, CF_UTXO_BY_PUBKEY,
    CF_UTXO_STAGING, META_CHAIN_STATE, META_LAST_APPLIED, META_PENDING_UPDATES,
    PROMOTE_BATCH_MAX_BYTES,
};

/// Above every key in the four replaced families — the widest is 68 bytes.
const KEY_UPPER_BOUND: [u8; 69] = [0xFF; 69];

fn db_err(e: impl std::fmt::Display) -> StorageError {
    StorageError::Database(e.to_string())
}

fn ser_err(e: impl std::fmt::Display) -> StorageError {
    StorageError::Serialization(e.to_string())
}

impl StateDb {
    /// Install the staged set: bounded sub-batches, then the state labels, then a
    /// SEPARATE write that clears staging.
    ///
    /// The clear cannot join the promotion: its delete set would then overlap the
    /// rows being streamed out of, and the live set would land as a prefix.
    pub fn promote_staged_utxos(
        &self,
        cs: &ChainState,
        ps: &ProducerSet,
    ) -> Result<(), StorageError> {
        self.clear_live_utxo_families()?;
        let rows = self.write_staged_rows_in_sub_batches()?;
        self.write_state_labels(cs, ps)?;
        self.utxo_count.store(rows, Ordering::Relaxed);
        self.clear_staged_utxos()
    }

    /// Test seam: abort `promote_staged_utxos` after this many committed
    /// sub-batches, standing in for a crash between them. `None` disarms.
    #[doc(hidden)]
    pub fn fault_inject_promote_abort_after(&self, batches: Option<u64>) {
        self.promote_abort_after
            .store(batches.unwrap_or(u64::MAX), Ordering::Relaxed);
    }

    /// Phase 1 — drop the old live rows with two range tombstones, never a
    /// delete per key.
    fn clear_live_utxo_families(&self) -> Result<(), StorageError> {
        let mut batch = rocksdb::WriteBatch::default();
        for name in [CF_UTXO, CF_UTXO_BY_PUBKEY] {
            let cf = self.db.cf_handle(name).unwrap();
            batch.delete_range_cf(cf, [].as_slice(), KEY_UPPER_BOUND.as_slice());
        }
        self.db.write(batch).map_err(db_err)?;
        self.utxo_count.store(0, Ordering::Relaxed);
        Ok(())
    }

    /// Phase 2 — stream staging into the live families, committing whenever the
    /// pending payload reaches [`PROMOTE_BATCH_MAX_BYTES`].
    fn write_staged_rows_in_sub_batches(&self) -> Result<u64, StorageError> {
        let cf_stage = self.db.cf_handle(CF_UTXO_STAGING).unwrap();
        let cf_utxo = self.db.cf_handle(CF_UTXO).unwrap();
        let cf_by_pk = self.db.cf_handle(CF_UTXO_BY_PUBKEY).unwrap();

        let staged = self
            .db
            .iterator_cf(cf_stage, rocksdb::IteratorMode::Start)
            .flatten()
            .filter_map(|(k, v)| {
                let outpoint = Outpoint::from_bytes(&k)?;
                let entry: UtxoEntry = bincode::deserialize(&v).ok()?;
                Some((outpoint, entry))
            });

        let mut batch = rocksdb::WriteBatch::default();
        let mut pending = 0usize;
        let mut committed = 0u64;
        let mut rows = 0u64;

        for (outpoint, entry) in staged {
            let key = outpoint.to_bytes();
            let value = bincode::serialize(&entry).map_err(ser_err)?;

            let mut idx_key = Vec::with_capacity(68);
            idx_key.extend_from_slice(entry.output.pubkey_hash.as_bytes());
            idx_key.extend_from_slice(&key);

            pending += key.len() + value.len() + idx_key.len() + 1;
            batch.put_cf(cf_utxo, key, &value);
            batch.put_cf(cf_by_pk, &idx_key, [0u8]);
            rows += 1;

            if pending >= PROMOTE_BATCH_MAX_BYTES {
                self.commit_sub_batch(std::mem::take(&mut batch), rows)?;
                pending = 0;
                committed += 1;
                self.abort_if_armed(committed)?;
            }
        }

        if pending > 0 {
            self.commit_sub_batch(std::mem::take(&mut batch), rows)?;
            committed += 1;
            self.abort_if_armed(committed)?;
        }

        Ok(rows)
    }

    /// Phase 3 — the tip label and the producer families, after the UTXO rows and
    /// before the caller's root re-derivation.
    fn write_state_labels(&self, cs: &ChainState, ps: &ProducerSet) -> Result<(), StorageError> {
        let cf_prod = self.db.cf_handle(CF_PRODUCERS).unwrap();
        let cf_exit = self.db.cf_handle(CF_EXIT_HISTORY).unwrap();
        let cf_meta = self.db.cf_handle(CF_META).unwrap();

        let mut batch = rocksdb::WriteBatch::default();
        batch.delete_range_cf(cf_prod, [].as_slice(), KEY_UPPER_BOUND.as_slice());
        batch.delete_range_cf(cf_exit, [].as_slice(), KEY_UPPER_BOUND.as_slice());

        let (producers, exit_history, pending_updates) = ps.as_parts();
        for (hash, info) in producers {
            let value = bincode::serialize(info).map_err(ser_err)?;
            batch.put_cf(cf_prod, hash.as_bytes(), &value);
        }
        for (hash, height) in exit_history {
            batch.put_cf(cf_exit, hash.as_bytes(), height.to_le_bytes());
        }

        let cs_bytes = bincode::serialize(cs).map_err(ser_err)?;
        batch.put_cf(cf_meta, META_CHAIN_STATE, &cs_bytes);

        let pending_bytes = bincode::serialize(pending_updates).map_err(ser_err)?;
        batch.put_cf(cf_meta, META_PENDING_UPDATES, &pending_bytes);

        let la = LastApplied {
            height: cs.best_height,
            hash: cs.best_hash,
            slot: cs.best_slot,
        };
        batch.put_cf(cf_meta, META_LAST_APPLIED, la.to_bytes());

        self.db.write(batch).map_err(db_err)
    }

    fn commit_sub_batch(&self, batch: rocksdb::WriteBatch, rows: u64) -> Result<(), StorageError> {
        self.db.write(batch).map_err(db_err)?;
        self.utxo_count.store(rows, Ordering::Relaxed);
        Ok(())
    }

    fn abort_if_armed(&self, committed: u64) -> Result<(), StorageError> {
        if committed >= self.promote_abort_after.load(Ordering::Relaxed) {
            return Err(StorageError::Database(format!(
                "[STOR040] staged promotion aborted after {committed} committed sub-batches"
            )));
        }
        Ok(())
    }
}
