//! M3 [F3] staged UTXO transfer landing zone: chunks land in `cf_utxo_staging`,
//! `promote.rs` moves them into the live family, a separate write clears them.
//!
//! `cf_utxo_staging` is deliberately absent from `deletable_cf_names()`: the
//! promotion streams OUT of the rows a wipe would otherwise delete.

use crate::utxo::canonical;
use crate::StorageError;

use super::types::{StateDb, CF_META, CF_UTXO_STAGING, META_UTXO_STAGING_RESIDUAL};

impl StateDb {
    /// Column families a whole-state replace wipes before rewriting.
    ///
    /// The replace path reads its delete set from here, so a caller asserting
    /// against this list is reading the real one, not a second copy.
    pub fn deletable_cf_names() -> &'static [&'static str] {
        use super::types::{CF_EXIT_HISTORY, CF_PRODUCERS, CF_UTXO, CF_UTXO_BY_PUBKEY};
        &[CF_UTXO, CF_UTXO_BY_PUBKEY, CF_PRODUCERS, CF_EXIT_HISTORY]
    }

    /// Append one verified chunk body to the staging family.
    ///
    /// `body` is canonical BODY bytes with no count header. A chunk boundary may
    /// fall anywhere, including inside a record, so the tail that does not yet
    /// form a whole record is carried in `CF_META` until the next call.
    pub fn stage_utxo_bytes(&self, body: &[u8]) -> Result<(), StorageError> {
        let cf_stage = self.db.cf_handle(CF_UTXO_STAGING).unwrap();
        let cf_meta = self.db.cf_handle(CF_META).unwrap();

        let mut buf = self
            .db
            .get_cf(cf_meta, META_UTXO_STAGING_RESIDUAL)
            .map_err(|e| StorageError::Database(e.to_string()))?
            .unwrap_or_default();
        buf.extend_from_slice(body);

        let mut batch = rocksdb::WriteBatch::default();
        let mut pos = 0usize;
        while let Some((outpoint, entry, len)) = canonical::parse_record(&buf, pos)? {
            let value = bincode::serialize(&entry)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            batch.put_cf(cf_stage, outpoint.to_bytes(), &value);
            pos += len;
        }

        if pos == buf.len() {
            batch.delete_cf(cf_meta, META_UTXO_STAGING_RESIDUAL);
        } else {
            batch.put_cf(cf_meta, META_UTXO_STAGING_RESIDUAL, &buf[pos..]);
        }

        self.db
            .write(batch)
            .map_err(|e| StorageError::Database(e.to_string()))
    }

    /// Rows currently held in the staging family.
    pub fn staged_utxo_len(&self) -> usize {
        let cf_stage = self.db.cf_handle(CF_UTXO_STAGING).unwrap();
        self.db
            .iterator_cf(cf_stage, rocksdb::IteratorMode::Start)
            .flatten()
            .count()
    }

    /// Discard everything in the staging family, including any residual tail.
    pub fn clear_staged_utxos(&self) -> Result<(), StorageError> {
        let cf_stage = self.db.cf_handle(CF_UTXO_STAGING).unwrap();
        let cf_meta = self.db.cf_handle(CF_META).unwrap();

        let mut batch = rocksdb::WriteBatch::default();
        for (key, _) in self
            .db
            .iterator_cf(cf_stage, rocksdb::IteratorMode::Start)
            .flatten()
        {
            batch.delete_cf(cf_stage, &key);
        }
        batch.delete_cf(cf_meta, META_UTXO_STAGING_RESIDUAL);

        self.db
            .write(batch)
            .map_err(|e| StorageError::Database(e.to_string()))
    }
}
