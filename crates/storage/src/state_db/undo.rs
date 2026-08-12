//! Undo log operations for block rollback

use doli_core::types::BlockHeight;

use super::types::{MaintainerUndoSnapshot, StateDb, UndoData, CF_UNDO};
use crate::StorageError;

/// Key discriminator for the INC-I-174 maintainer snapshot records in `cf_undo`.
///
/// A `UndoData` key is `height.to_le_bytes()` — exactly 8 bytes. A maintainer snapshot
/// key is this byte followed by the same 8 bytes — exactly 9. RocksDB point lookups
/// compare the whole key, and the two families can never collide because their LENGTHS
/// differ, whatever the height. That is what lets INC-I-174 add per-height maintainer
/// undo state without touching the `UndoData` encoding (see the append-hostility note on
/// [`UndoData`]).
///
/// `0x4D` is ASCII `M`, chosen only for greppability in a hexdump.
pub(super) const MAINTAINER_UNDO_KEY_PREFIX: u8 = 0x4D;

/// The `cf_undo` key of the maintainer snapshot for `height`.
fn maintainer_undo_key(height: BlockHeight) -> [u8; 9] {
    let mut key = [0u8; 9];
    key[0] = MAINTAINER_UNDO_KEY_PREFIX;
    key[1..].copy_from_slice(&height.to_le_bytes());
    key
}

/// The block height a `cf_undo` key belongs to, for EITHER record family.
///
/// The two pruners iterate the whole CF and used to filter on `key.len() == 8`, which
/// would have silently skipped every maintainer record and leaked it past
/// `UNDO_KEEP_DEPTH`. Both now route through this function, so a key shape can only be
/// added in one place.
pub(super) fn undo_key_height(key: &[u8]) -> Option<BlockHeight> {
    match key.len() {
        8 => Some(BlockHeight::from_le_bytes(key.try_into().ok()?)),
        9 if key[0] == MAINTAINER_UNDO_KEY_PREFIX => {
            Some(BlockHeight::from_le_bytes(key[1..9].try_into().ok()?))
        }
        _ => None,
    }
}

impl StateDb {
    // ==================== Undo Log ====================

    /// Store undo data for a block height.
    pub fn put_undo(&self, height: BlockHeight, undo: &UndoData) -> Result<(), StorageError> {
        let cf = self.db.cf_handle(CF_UNDO).unwrap();
        let key = height.to_le_bytes();
        let value = bincode::serialize(undo).expect("UndoData serialization");
        self.db.put_cf(cf, key, value)?;
        Ok(())
    }

    /// Get undo data for a block height.
    pub fn get_undo(&self, height: BlockHeight) -> Option<UndoData> {
        let cf = self.db.cf_handle(CF_UNDO).unwrap();
        let key = height.to_le_bytes();
        let bytes = self.db.get_cf(cf, key).ok()??;
        bincode::deserialize(&bytes).ok()
    }

    // ==================== Maintainer Undo Snapshots (INC-I-174) ====================

    /// Store the pre-block maintainer trust root for `height`.
    ///
    /// Written ONLY for a block that carries an `AddMaintainer` / `RemoveMaintainer`.
    /// Absence is the "the maintainer set did not change at this height" sentinel and
    /// costs zero bytes (INC-I-071 `cf_undo` bloat discipline).
    ///
    /// The production writer is [`crate::BlockBatch::put_maintainer_undo`], which puts
    /// the record in the SAME `WriteBatch` as the block commit. This direct variant
    /// exists for tests and for repair tooling.
    pub fn put_maintainer_undo(
        &self,
        height: BlockHeight,
        snapshot: &MaintainerUndoSnapshot,
    ) -> Result<(), StorageError> {
        let cf = self.db.cf_handle(CF_UNDO).unwrap();
        let value =
            bincode::serialize(snapshot).map_err(|e| StorageError::Serialization(e.to_string()))?;
        self.db.put_cf(cf, maintainer_undo_key(height), value)?;
        Ok(())
    }

    /// Read the pre-block maintainer trust root for `height`.
    ///
    /// `None` means "no record". Like [`Self::get_undo`], that collapses "never written"
    /// with "unreadable" — so the rewind path in `bins/node/src/node/maintainer_rewind/`
    /// never treats `None` as proof that the block changed nothing. It asks the BLOCK,
    /// and counts + announces the case where a rotation has no restorable snapshot
    /// (REQ-174-005).
    pub fn get_maintainer_undo(&self, height: BlockHeight) -> Option<MaintainerUndoSnapshot> {
        let cf = self.db.cf_handle(CF_UNDO).unwrap();
        let bytes = self.db.get_cf(cf, maintainer_undo_key(height)).ok()??;
        bincode::deserialize(&bytes).ok()
    }

    /// Delete the maintainer snapshot for `height`. Idempotent.
    ///
    /// **Deliberately has no production caller, and must not grow one on the rewind path.**
    /// It is the primitive the REJECTED alternative for INC-I-174 reviewer F1 would have
    /// needed: deleting the record for every rewound height so no fossil can survive a
    /// reorg. That variant was costed and refused on two grounds — it adds `depth` durable
    /// writes to the exact non-atomic rollback sequence AUDIT-P1-201 already records as
    /// half-applied-on-abort, and it FAILS OPEN, because one missed delete silently
    /// restores a trust root that exists on no chain. The shipped fix is a read-side
    /// cross-check in `plan_maintainer_rewind` (`bins/node/src/node/maintainer_rewind/`):
    /// a record at `h` is authority only while the block NOW at `h` carries a rotation.
    /// That is self-healing and has no durable side effect.
    ///
    /// Kept, not removed, because offline repair tooling and the schema tests
    /// (`crates/storage/tests/inc_i_174_undo_schema.rs`) need a way to remove one record
    /// without walking the whole CF — the two pruners are range operations only.
    pub fn delete_maintainer_undo(&self, height: BlockHeight) -> Result<(), StorageError> {
        let cf = self.db.cf_handle(CF_UNDO).unwrap();
        self.db.delete_cf(cf, maintainer_undo_key(height))?;
        Ok(())
    }

    /// Prune undo data older than `keep_height`.
    /// Called after apply_block to keep only the last N blocks of undo data.
    ///
    /// Deletes only the single entry that just expired (O(1) per block).
    /// Tombstone reclamation is handled by RocksDB's `periodic_compaction_seconds`
    /// (set on cf_undo in `open.rs`), running in background threads — not by a
    /// synchronous `compact_range_cf` in the apply event loop. INC-I-108.
    ///
    /// INC-I-174: the maintainer snapshot for the same height expires with it. Two
    /// point deletes, still O(1) per block — INV-STORAGE-108 holds.
    pub fn prune_undo_before(&self, keep_height: BlockHeight) {
        if keep_height == 0 {
            return;
        }
        let cf = self.db.cf_handle(CF_UNDO).unwrap();

        // Delete only the single entry that just aged out (keep_height - 1).
        // Since prune is called every block with monotonically increasing
        // keep_height, at most one entry expires per call.
        let expired = keep_height - 1;
        let key = expired.to_le_bytes();
        let _ = self.db.delete_cf(cf, key);
        let _ = self.db.delete_cf(cf, maintainer_undo_key(expired));
    }

    /// Delete all undo data above the given height (for truncation).
    ///
    /// INC-I-174: covers BOTH record families via [`undo_key_height`]. Filtering on
    /// `key.len() == 8` would have left every maintainer snapshot above `keep_height`
    /// in place, so a later re-apply of that height would find a stale pre-block root.
    pub fn prune_undo_above(&self, keep_height: BlockHeight) {
        let cf = self.db.cf_handle(CF_UNDO).unwrap();
        let mut batch = rocksdb::WriteBatch::default();
        let mut count = 0u64;
        for (key, _) in self
            .db
            .iterator_cf(cf, rocksdb::IteratorMode::Start)
            .flatten()
        {
            if let Some(h) = undo_key_height(&key) {
                if h > keep_height {
                    batch.delete_cf(cf, &key);
                    count += 1;
                }
            }
        }
        if count > 0 {
            let _ = self.db.write(batch);
            let start = (keep_height + 1).to_le_bytes();
            let end = u64::MAX.to_le_bytes();
            self.db
                .compact_range_cf(cf, Some(&start[..]), Some(&end[..]));
        }
    }

    /// Check whether every height in `[from, to]` (inclusive) has undo data.
    /// Returns `true` iff `get_undo(h)` is `Some` for all h in `[from, to]`.
    /// An empty range (`from > to`) returns `true`.
    ///
    /// Used by the checkpoint guardian (INC-I-136 M2, REQ-GUARD-003 F4) to
    /// refuse a `healthy` tag when undo data is missing in the rollback window.
    ///
    /// O(to - from + 1) point lookups against cf_undo.
    pub fn has_undo_data(&self, from: u64, to: u64) -> bool {
        if from > to {
            return true;
        }
        for h in from..=to {
            if self.get_undo(h).is_none() {
                return false;
            }
        }
        true
    }

    /// One-shot bulk delete of cf_undo entries STRICTLY BELOW `keep_height`.
    ///
    /// Mirrors `prune_undo_above`. Intended for catch-up after `UNDO_KEEP_DEPTH`
    /// is reduced — the per-block `prune_undo_before` walks forward only and
    /// cannot reclaim historical entries below the new horizon. Idempotent.
    /// Returns the number of entries deleted.
    ///
    /// INC-I-174: covers BOTH record families via [`undo_key_height`].
    pub fn prune_undo_below(&self, keep_height: BlockHeight) -> u64 {
        if keep_height == 0 {
            return 0;
        }
        let cf = self.db.cf_handle(CF_UNDO).unwrap();
        let mut batch = rocksdb::WriteBatch::default();
        let mut count = 0u64;
        for (key, _) in self
            .db
            .iterator_cf(cf, rocksdb::IteratorMode::Start)
            .flatten()
        {
            if let Some(h) = undo_key_height(&key) {
                if h < keep_height {
                    batch.delete_cf(cf, &key);
                    count += 1;
                }
            }
        }
        if count > 0 {
            let _ = self.db.write(batch);
            let start = 0u64.to_le_bytes();
            let end = keep_height.to_le_bytes();
            self.db
                .compact_range_cf(cf, Some(&start[..]), Some(&end[..]));
        }
        count
    }
}
