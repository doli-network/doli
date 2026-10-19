use std::path::Path;

mod catch_up;
mod fence;
mod import;
mod manifest;
mod write;

pub use catch_up::archive_finalized;
pub use fence::fence_stale_archive;
pub use import::{
    backfill_from_archive, backfill_heights_from_archive, force_backfill_from_archive,
    restore_from_archive, TargetedBackfillReport,
};
pub use manifest::{manifest_genesis_hash, manifest_height};

/// A block ready to be archived.
pub struct ArchiveBlock {
    pub height: u64,
    pub hash: crypto::Hash,
    pub data: Vec<u8>,
}

/// Outcome of one catch-up or live archive call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CatchUpReport {
    /// `.block` files read (the spot-check at H reads none).
    pub files_read: u64,
    /// `.block` + `.blake3` pairs written.
    pub files_written: u64,
    /// Highest contiguous height verified or self-written for the genesis.
    pub verified_height: u64,
}

/// Archives blocks to a filesystem directory.
///
/// Each block is stored as `{height:010}.block` with a BLAKE3 checksum sidecar
/// (`{height:010}.blake3`). `manifest.json` holds `latest_height` (high-water
/// mark), `genesis_hash` and `verified_height` (contiguous verified height).
pub struct BlockArchiver;

impl BlockArchiver {
    /// Catch up the archive to `tip` from the BlockStore.
    ///
    /// `Some(genesis)`: bounded, genesis-verified catch-up
    /// ([`Self::catch_up_bounded`]); returns the pairs written.
    /// `None`: presence-only fill of missing files; `verified_height` is kept.
    pub fn catch_up(
        dir: &Path,
        block_store: &crate::BlockStore,
        tip: u64,
        expected_genesis_hash: Option<&crypto::Hash>,
    ) -> Result<u64, std::io::Error> {
        match expected_genesis_hash {
            Some(g) => Self::catch_up_bounded(dir, block_store, tip, g).map(|r| r.files_written),
            None => catch_up::catch_up_presence(dir, block_store, tip),
        }
    }

    /// Verifies and fills heights above the trusted `verified_height` H up to
    /// `tip`. An untrusted or missing H costs one full check from height 1.
    pub fn catch_up_bounded(
        dir: &Path,
        block_store: &crate::BlockStore,
        tip: u64,
        expected_genesis: &crypto::Hash,
    ) -> std::io::Result<CatchUpReport> {
        catch_up::catch_up_bounded(dir, block_store, tip, expected_genesis)
    }
}
