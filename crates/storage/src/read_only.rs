//! Read-only RocksDB opens for offline inspection of a stopped node's data.

use std::path::Path;

use crate::StorageError;

/// Opens every column family of the existing DB at `path` read-only.
///
/// RocksDB takes no `LOCK` and writes no file under `path`; any info log
/// goes to `log_dir`.
pub(crate) fn open_db(path: &Path, log_dir: &Path) -> Result<rocksdb::DB, StorageError> {
    let mut opts = rocksdb::Options::default();
    opts.set_max_open_files(256);
    opts.set_db_log_dir(log_dir);
    let cfs = rocksdb::DB::list_cf(&opts, path)?;
    Ok(rocksdb::DB::open_cf_for_read_only(&opts, path, cfs, false)?)
}
