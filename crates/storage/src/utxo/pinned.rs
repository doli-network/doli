//! A UTXO view held across MANY reads — the M3 [F3] state-transfer session.
//!
//! `canonical_range` pins a fresh RocksDB snapshot per call, which is enough for
//! one fold and not enough for a multi-message session: the set may move between
//! chunks and the reassembled image would not hash to the manifest. Here the pin
//! is created once and held for the whole closure, so the borrow checker keeps it
//! from escaping and the caller cannot accidentally serve two chunks from two
//! different versions.

use crypto::Hash;

use super::canonical;
use super::set::UtxoSet;
use crate::StorageError;

/// Canonical reads served from one pinned view.
pub trait PinnedUtxoRows {
    /// BLAKE3 over the whole canonical image, and its byte length.
    fn canonical_digest_and_len(&self) -> Result<(Hash, u64), StorageError>;

    /// Entry count — the value the canonical image's LE header carries.
    fn canonical_row_count(&self) -> Result<u64, StorageError>;

    /// Canonical BODY bytes from the first key `>= start_key`, up to `max_bytes`,
    /// with the cursor of the next entry (`None` when exhausted).
    fn canonical_range(
        &self,
        start_key: Option<&[u8]>,
        max_bytes: usize,
    ) -> Result<(Vec<u8>, Option<Vec<u8>>), StorageError>;
}

impl<T: canonical::CanonicalRows> PinnedUtxoRows for T {
    fn canonical_digest_and_len(&self) -> Result<(Hash, u64), StorageError> {
        canonical::digest_and_len(self)
    }

    fn canonical_row_count(&self) -> Result<u64, StorageError> {
        self.row_count()
    }

    fn canonical_range(
        &self,
        start_key: Option<&[u8]>,
        max_bytes: usize,
    ) -> Result<(Vec<u8>, Option<Vec<u8>>), StorageError> {
        canonical::range(self, start_key, max_bytes)
    }
}

impl UtxoSet {
    /// Run `f` with ONE pinned view of this set, held for the whole closure.
    pub fn with_pinned_view<R>(&self, f: impl FnOnce(&dyn PinnedUtxoRows) -> R) -> R {
        match self {
            UtxoSet::InMemory(store) => f(store),
            UtxoSet::RocksDb(sdb) => f(&sdb.utxo_rows()),
        }
    }

    /// An OWNED handle that can be moved to a session worker and pinned there.
    ///
    /// The two backends are asymmetric and cannot be made otherwise: RocksDB
    /// hands back a cheap `Arc` clone whose pin is a storage snapshot, while the
    /// in-memory backend's pin is a live `BTreeMap` the writer still owns, so the
    /// only honest freeze is a copy — affordable precisely because that backend
    /// already holds the whole set in RAM.
    pub fn pinnable_handle(&self) -> Result<UtxoSet, StorageError> {
        match self {
            UtxoSet::RocksDb(sdb) => Ok(UtxoSet::RocksDb(sdb.clone())),
            UtxoSet::InMemory(_) => UtxoSet::deserialize_canonical(&self.serialize_canonical()),
        }
    }
}
