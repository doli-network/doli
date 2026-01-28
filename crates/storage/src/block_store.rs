//! Block storage

use std::path::Path;

use crypto::Hash;
use doli_core::{Block, BlockHeader};

use crate::StorageError;

/// Column family names
const CF_HEADERS: &str = "headers";
const CF_BODIES: &str = "bodies";
const CF_HEIGHT_INDEX: &str = "height_index";
const CF_SLOT_INDEX: &str = "slot_index";

/// Block store
pub struct BlockStore {
    db: rocksdb::DB,
}

impl BlockStore {
    /// Open or create a block store
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let cfs = vec![CF_HEADERS, CF_BODIES, CF_HEIGHT_INDEX, CF_SLOT_INDEX];

        let db = rocksdb::DB::open_cf(&opts, path, cfs)?;

        Ok(Self { db })
    }

    /// Store a block
    pub fn put_block(&self, block: &Block, height: u64) -> Result<(), StorageError> {
        let hash = block.hash();
        let hash_bytes = hash.as_bytes();

        // Store header
        let header_bytes = bincode::serialize(&block.header)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let cf_headers = self.db.cf_handle(CF_HEADERS).unwrap();
        self.db.put_cf(cf_headers, hash_bytes, &header_bytes)?;

        // Store body
        let body_bytes = bincode::serialize(&block.transactions)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let cf_bodies = self.db.cf_handle(CF_BODIES).unwrap();
        self.db.put_cf(cf_bodies, hash_bytes, &body_bytes)?;

        // Update height index
        let cf_height = self.db.cf_handle(CF_HEIGHT_INDEX).unwrap();
        self.db
            .put_cf(cf_height, height.to_le_bytes(), hash_bytes)?;

        // Update slot index
        let cf_slot = self.db.cf_handle(CF_SLOT_INDEX).unwrap();
        self.db
            .put_cf(cf_slot, block.slot().to_le_bytes(), hash_bytes)?;

        Ok(())
    }

    /// Get a block by hash
    pub fn get_block(&self, hash: &Hash) -> Result<Option<Block>, StorageError> {
        let cf_headers = self.db.cf_handle(CF_HEADERS).unwrap();
        let cf_bodies = self.db.cf_handle(CF_BODIES).unwrap();

        let header_bytes = match self.db.get_cf(cf_headers, hash.as_bytes())? {
            Some(b) => b,
            None => return Ok(None),
        };

        let body_bytes = match self.db.get_cf(cf_bodies, hash.as_bytes())? {
            Some(b) => b,
            None => return Ok(None),
        };

        let header: BlockHeader = bincode::deserialize(&header_bytes)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let transactions = bincode::deserialize(&body_bytes)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        Ok(Some(Block::new(header, transactions)))
    }

    /// Get a header by hash
    pub fn get_header(&self, hash: &Hash) -> Result<Option<BlockHeader>, StorageError> {
        let cf_headers = self.db.cf_handle(CF_HEADERS).unwrap();

        let bytes = match self.db.get_cf(cf_headers, hash.as_bytes())? {
            Some(b) => b,
            None => return Ok(None),
        };

        let header: BlockHeader =
            bincode::deserialize(&bytes).map_err(|e| StorageError::Serialization(e.to_string()))?;

        Ok(Some(header))
    }

    /// Get block hash by height
    pub fn get_hash_by_height(&self, height: u64) -> Result<Option<Hash>, StorageError> {
        let cf_height = self.db.cf_handle(CF_HEIGHT_INDEX).unwrap();

        let bytes = match self.db.get_cf(cf_height, height.to_le_bytes())? {
            Some(b) => b,
            None => return Ok(None),
        };

        if bytes.len() != 32 {
            return Err(StorageError::Serialization("invalid hash length".into()));
        }

        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Some(Hash::from_bytes(arr)))
    }

    /// Get block by height
    pub fn get_block_by_height(&self, height: u64) -> Result<Option<Block>, StorageError> {
        let hash = match self.get_hash_by_height(height)? {
            Some(h) => h,
            None => return Ok(None),
        };
        self.get_block(&hash)
    }

    /// Check if block exists
    pub fn has_block(&self, hash: &Hash) -> Result<bool, StorageError> {
        let cf_headers = self.db.cf_handle(CF_HEADERS).unwrap();
        Ok(self.db.get_cf(cf_headers, hash.as_bytes())?.is_some())
    }
}
