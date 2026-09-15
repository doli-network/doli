//! M4 [F3]: the `StateDb` side of the snap-sync staging seam.
//!
//! `crates/network` has no `storage` dependency, so the session client reaches
//! the staging column family through `network::sync::UtxoChunkSink` and this
//! type supplies the implementation (memory.db decision 130).

use std::sync::Arc;

use network::sync::UtxoChunkSink;
use storage::StateDb;

/// Routes verified snap-sync chunk bodies into `cf_utxo_staging`.
pub struct StateDbChunkSink(Arc<StateDb>);

impl StateDbChunkSink {
    pub fn new(db: Arc<StateDb>) -> Self {
        Self(db)
    }
}

impl UtxoChunkSink for StateDbChunkSink {
    fn stage(&self, body: &[u8]) -> Result<(), String> {
        self.0.stage_utxo_bytes(body).map_err(|e| e.to_string())
    }

    fn clear(&self) -> Result<(), String> {
        self.0.clear_staged_utxos().map_err(|e| e.to_string())
    }

    /// `StateDb::staged_utxo_len` is `usize`; the trait is `u64` so the network
    /// crate never sees a platform-width type.
    fn staged_len(&self) -> u64 {
        self.0.staged_utxo_len() as u64
    }
}
