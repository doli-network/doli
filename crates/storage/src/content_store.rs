//! Content-addressed blob storage with reference counting.
//!
//! Deduplicates large data (NFT images, documents) by storing each unique
//! content blob once, keyed by its BLAKE3 hash. Multiple UTXOs can reference
//! the same content without duplicating storage.
//!
//! Used by archivers to efficiently store NFT collections where many tokens
//! share the same base image or metadata template.

use crypto::{hash::hash, Hash};
use rocksdb::{Options, DB};
use std::path::Path;

use crate::StorageError;

const CF_CONTENT: &str = "content"; // content_hash → raw bytes
const CF_REFCOUNT: &str = "refcount"; // content_hash → u64 ref count

/// Content-addressed blob store with reference counting.
pub struct ContentStore {
    db: DB,
}

impl ContentStore {
    /// Open or create a content store at the given path.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let cfs = vec![
            rocksdb::ColumnFamilyDescriptor::new(CF_CONTENT, Options::default()),
            rocksdb::ColumnFamilyDescriptor::new(CF_REFCOUNT, Options::default()),
        ];

        let db = DB::open_cf_descriptors(&opts, path, cfs)?;
        Ok(Self { db })
    }

    /// Store content and increment reference count.
    /// If content already exists (same hash), only increments the ref count.
    /// Returns the content hash.
    pub fn put(&self, data: &[u8]) -> Result<Hash, StorageError> {
        let content_hash = hash(data);
        let key = content_hash.as_bytes();

        let cf_content = self.db.cf_handle(CF_CONTENT).unwrap();
        let cf_refcount = self.db.cf_handle(CF_REFCOUNT).unwrap();

        // Check if content already exists
        let current_refs = self.get_refcount(&content_hash)?;

        if current_refs == 0 {
            // New content — store it
            self.db.put_cf(cf_content, key, data)?;
        }

        // Increment ref count
        let new_refs = current_refs + 1;
        self.db.put_cf(cf_refcount, key, new_refs.to_le_bytes())?;

        Ok(content_hash)
    }

    /// Get content by hash. Returns None if not found.
    pub fn get(&self, content_hash: &Hash) -> Result<Option<Vec<u8>>, StorageError> {
        let cf_content = self.db.cf_handle(CF_CONTENT).unwrap();
        Ok(self.db.get_cf(cf_content, content_hash.as_bytes())?)
    }

    /// Decrement reference count. Deletes content when ref count reaches 0.
    /// Returns the new ref count (0 = deleted).
    pub fn release(&self, content_hash: &Hash) -> Result<u64, StorageError> {
        let key = content_hash.as_bytes();
        let cf_content = self.db.cf_handle(CF_CONTENT).unwrap();
        let cf_refcount = self.db.cf_handle(CF_REFCOUNT).unwrap();

        let current_refs = self.get_refcount(content_hash)?;
        if current_refs == 0 {
            return Ok(0);
        }

        let new_refs = current_refs - 1;
        if new_refs == 0 {
            // Last reference — delete content
            self.db.delete_cf(cf_content, key)?;
            self.db.delete_cf(cf_refcount, key)?;
        } else {
            self.db.put_cf(cf_refcount, key, new_refs.to_le_bytes())?;
        }

        Ok(new_refs)
    }

    /// Get the reference count for a content hash.
    pub fn get_refcount(&self, content_hash: &Hash) -> Result<u64, StorageError> {
        let cf_refcount = self.db.cf_handle(CF_REFCOUNT).unwrap();
        match self.db.get_cf(cf_refcount, content_hash.as_bytes())? {
            Some(bytes) if bytes.len() == 8 => Ok(u64::from_le_bytes(bytes.try_into().unwrap())),
            _ => Ok(0),
        }
    }

    /// Check if content exists (ref count > 0).
    pub fn contains(&self, content_hash: &Hash) -> Result<bool, StorageError> {
        Ok(self.get_refcount(content_hash)? > 0)
    }

    /// Total number of unique content blobs stored.
    pub fn len(&self) -> usize {
        let cf_refcount = self.db.cf_handle(CF_REFCOUNT).unwrap();
        self.db
            .iterator_cf(cf_refcount, rocksdb::IteratorMode::Start)
            .count()
    }

    /// Check if store is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_put_get_release() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(dir.path()).unwrap();

        let data = b"hello world NFT image data";
        let hash = store.put(data).unwrap();
        assert_eq!(store.get_refcount(&hash).unwrap(), 1);

        let retrieved = store.get(&hash).unwrap().unwrap();
        assert_eq!(retrieved, data);

        store.release(&hash).unwrap();
        assert_eq!(store.get_refcount(&hash).unwrap(), 0);
        assert!(store.get(&hash).unwrap().is_none());
    }

    #[test]
    fn test_deduplication() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(dir.path()).unwrap();

        let data = b"same image used by 100 NFTs";

        // Store same content 3 times
        let h1 = store.put(data).unwrap();
        let h2 = store.put(data).unwrap();
        let h3 = store.put(data).unwrap();

        assert_eq!(h1, h2);
        assert_eq!(h2, h3);
        assert_eq!(store.get_refcount(&h1).unwrap(), 3);
        assert_eq!(store.len(), 1); // Only 1 unique blob

        // Release 2 references — content still exists
        store.release(&h1).unwrap();
        store.release(&h1).unwrap();
        assert_eq!(store.get_refcount(&h1).unwrap(), 1);
        assert!(store.get(&h1).unwrap().is_some());

        // Release last reference — content deleted
        store.release(&h1).unwrap();
        assert_eq!(store.get_refcount(&h1).unwrap(), 0);
        assert!(store.get(&h1).unwrap().is_none());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn test_different_content() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(dir.path()).unwrap();

        let h1 = store.put(b"image1").unwrap();
        let h2 = store.put(b"image2").unwrap();

        assert_ne!(h1, h2);
        assert_eq!(store.len(), 2);
        assert_eq!(store.get_refcount(&h1).unwrap(), 1);
        assert_eq!(store.get_refcount(&h2).unwrap(), 1);
    }
}
