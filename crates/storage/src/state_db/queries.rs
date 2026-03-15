//! Read-only query methods on StateDb

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use crypto::Hash;
use doli_core::types::{Amount, BlockHeight};

use crate::chain_state::ChainState;
use crate::producer::{PendingProducerUpdate, ProducerInfo, ProducerSet};
use crate::utxo::{Outpoint, UtxoEntry};

use super::types::{
    LastApplied, StateDb, CF_EXIT_HISTORY, CF_META, CF_PRODUCERS, CF_UTXO, CF_UTXO_BY_PUBKEY,
    META_CHAIN_STATE, META_LAST_APPLIED, META_PENDING_UPDATES,
};

impl StateDb {
    /// Check if the database has any state (non-empty).
    pub fn has_state(&self) -> bool {
        let cf = self.db.cf_handle(CF_META).unwrap();
        self.db
            .get_cf(cf, META_CHAIN_STATE)
            .ok()
            .flatten()
            .is_some()
    }

    // ==================== Read Methods ====================

    /// Get a UTXO by outpoint.
    pub fn get_utxo(&self, outpoint: &Outpoint) -> Option<UtxoEntry> {
        let cf = self.db.cf_handle(CF_UTXO).unwrap();
        let key = outpoint.to_bytes();
        match self.db.get_cf(cf, &key) {
            Ok(Some(bytes)) => bincode::deserialize(&bytes).ok(),
            _ => None,
        }
    }

    /// Check if a UTXO exists.
    pub fn contains_utxo(&self, outpoint: &Outpoint) -> bool {
        let cf = self.db.cf_handle(CF_UTXO).unwrap();
        let key = outpoint.to_bytes();
        self.db.get_cf(cf, &key).ok().flatten().is_some()
    }

    /// Get all UTXOs for a given pubkey hash via secondary index prefix scan.
    pub fn get_utxos_by_pubkey(&self, pubkey_hash: &Hash) -> Vec<(Outpoint, UtxoEntry)> {
        let cf_by_pk = self.db.cf_handle(CF_UTXO_BY_PUBKEY).unwrap();
        let cf_utxo = self.db.cf_handle(CF_UTXO).unwrap();
        let prefix = pubkey_hash.as_bytes();

        let mut results = Vec::new();
        let iter = self.db.prefix_iterator_cf(cf_by_pk, prefix);
        for item in iter.flatten() {
            let (key, _) = item;
            if key.len() != 68 || &key[..32] != prefix {
                break;
            }
            if let Some(outpoint) = Outpoint::from_bytes(&key[32..68]) {
                let op_key = outpoint.to_bytes();
                if let Ok(Some(val)) = self.db.get_cf(cf_utxo, &op_key) {
                    if let Ok(entry) = bincode::deserialize::<UtxoEntry>(&val) {
                        results.push((outpoint, entry));
                    }
                }
            }
        }
        results
    }

    /// Get number of UTXOs (O(1) via cached counter).
    pub fn utxo_len(&self) -> usize {
        self.utxo_count.load(Ordering::Relaxed) as usize
    }

    /// Get total value in the UTXO set.
    pub fn utxo_total_value(&self) -> Amount {
        let cf = self.db.cf_handle(CF_UTXO).unwrap();
        let mut total: Amount = 0;
        for (_, value) in self
            .db
            .iterator_cf(cf, rocksdb::IteratorMode::Start)
            .flatten()
        {
            if let Ok(entry) = bincode::deserialize::<UtxoEntry>(&value) {
                total += entry.output.amount;
            }
        }
        total
    }

    /// Get spendable balance for a pubkey hash at a given height with custom maturity.
    pub fn get_balance_with_maturity(
        &self,
        pubkey_hash: &Hash,
        height: BlockHeight,
        maturity: BlockHeight,
    ) -> Amount {
        self.get_utxos_by_pubkey(pubkey_hash)
            .iter()
            .filter(|(_, entry)| entry.is_spendable_at_with_maturity(height, maturity))
            .map(|(_, entry)| entry.output.amount)
            .sum()
    }

    /// Get immature balance for a pubkey hash with custom maturity.
    pub fn get_immature_balance_with_maturity(
        &self,
        pubkey_hash: &Hash,
        height: BlockHeight,
        maturity: BlockHeight,
    ) -> Amount {
        self.get_utxos_by_pubkey(pubkey_hash)
            .iter()
            .filter(|(_, entry)| {
                (entry.is_coinbase || entry.is_epoch_reward)
                    && !entry.is_spendable_at_with_maturity(height, maturity)
            })
            .map(|(_, entry)| entry.output.amount)
            .sum()
    }

    /// Get a producer by pubkey hash.
    pub fn get_producer(&self, pubkey_hash: &Hash) -> Option<ProducerInfo> {
        let cf = self.db.cf_handle(CF_PRODUCERS).unwrap();
        match self.db.get_cf(cf, pubkey_hash.as_bytes()) {
            Ok(Some(bytes)) => bincode::deserialize(&bytes).ok(),
            _ => None,
        }
    }

    /// Iterate all producers. Returns (pubkey_hash, ProducerInfo) pairs.
    pub fn iter_producers(&self) -> Vec<(Hash, ProducerInfo)> {
        let cf = self.db.cf_handle(CF_PRODUCERS).unwrap();
        let mut results = Vec::new();
        for (key, value) in self
            .db
            .iterator_cf(cf, rocksdb::IteratorMode::Start)
            .flatten()
        {
            if key.len() == 32 {
                let hash = Hash::from_bytes(key[..32].try_into().unwrap());
                if let Ok(info) = bincode::deserialize::<ProducerInfo>(&value) {
                    results.push((hash, info));
                }
            }
        }
        results
    }

    /// Get exit history entry for a pubkey hash.
    pub fn get_exit_height(&self, pubkey_hash: &Hash) -> Option<u64> {
        let cf = self.db.cf_handle(CF_EXIT_HISTORY).unwrap();
        match self.db.get_cf(cf, pubkey_hash.as_bytes()) {
            Ok(Some(bytes)) if bytes.len() >= 8 => {
                Some(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
            }
            _ => None,
        }
    }

    /// Iterate all exit history entries.
    pub fn iter_exit_history(&self) -> Vec<(Hash, u64)> {
        let cf = self.db.cf_handle(CF_EXIT_HISTORY).unwrap();
        let mut results = Vec::new();
        for (key, value) in self
            .db
            .iterator_cf(cf, rocksdb::IteratorMode::Start)
            .flatten()
        {
            if key.len() == 32 && value.len() >= 8 {
                let hash = Hash::from_bytes(key[..32].try_into().unwrap());
                let height = u64::from_le_bytes(value[..8].try_into().unwrap());
                results.push((hash, height));
            }
        }
        results
    }

    /// Load ChainState from cf_meta.
    ///
    /// Returns `None` only when the key does not exist (genuine first run).
    /// Panics if the key exists but cannot be deserialized — this prevents
    /// silent state loss from format mismatches (the v1.0.29 incident).
    pub fn get_chain_state(&self) -> Option<ChainState> {
        let cf = self.db.cf_handle(CF_META).unwrap();
        match self.db.get_cf(cf, META_CHAIN_STATE) {
            Ok(Some(bytes)) if bytes.is_empty() => None,
            Ok(Some(bytes)) => {
                let cs = Self::deserialize_chain_state(&bytes).expect(
                    "ChainState exists in DB but failed to deserialize — refusing to start. \
                             This likely means a binary with new ChainState fields was deployed \
                             without a migration path. DO NOT ignore this error.",
                );
                Some(cs)
            }
            Ok(None) => None,
            Err(e) => {
                panic!("RocksDB read error for chain_state: {e}");
            }
        }
    }

    /// Deserialize ChainState bytes, handling format versions.
    ///
    /// Format detection:
    /// - `0x01` prefix → versioned format (v1.0.30+), strip prefix, bincode the rest
    /// - Anything else → legacy unversioned bincode (v1.0.28/v1.0.29)
    pub(super) fn deserialize_chain_state(bytes: &[u8]) -> Result<ChainState, String> {
        const FORMAT_V1: u8 = 0x01;

        if bytes.first() == Some(&FORMAT_V1) {
            // Versioned format: skip the 1-byte prefix
            return bincode::deserialize::<ChainState>(&bytes[1..])
                .map_err(|e| format!("v1 format: {e}"));
        }

        // Legacy: try raw bincode (v1.0.29 format — has all current fields)
        if let Ok(cs) = bincode::deserialize::<ChainState>(bytes) {
            return Ok(cs);
        }

        // Legacy migration: v1.0.28 format lacks active_protocol_version (u32)
        // and pending_protocol_activation (Option<(u32, u64)>).
        // Append defaults: 1u32 LE + 0x00 (None).
        let mut extended = bytes.to_vec();
        extended.extend_from_slice(&1u32.to_le_bytes());
        extended.push(0x00);
        bincode::deserialize::<ChainState>(&extended).map_err(|e| format!("legacy migration: {e}"))
    }

    /// Load pending producer updates from cf_meta.
    pub fn get_pending_updates(&self) -> Vec<PendingProducerUpdate> {
        let cf = self.db.cf_handle(CF_META).unwrap();
        match self.db.get_cf(cf, META_PENDING_UPDATES) {
            Ok(Some(bytes)) => bincode::deserialize(&bytes).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// Load the last_applied consistency canary.
    pub fn get_last_applied(&self) -> Option<LastApplied> {
        let cf = self.db.cf_handle(CF_META).unwrap();
        match self.db.get_cf(cf, META_LAST_APPLIED) {
            Ok(Some(bytes)) => LastApplied::from_bytes(&bytes),
            _ => None,
        }
    }

    /// Produce canonical UTXO bytes for state root computation.
    ///
    /// RocksDB iterates in lexicographic key order — no sorting needed.
    /// Values are re-encoded to canonical 59-byte format.
    pub fn serialize_canonical_utxo(&self) -> Vec<u8> {
        let cf = self.db.cf_handle(CF_UTXO).unwrap();
        let count = self.utxo_len() as u64;

        let mut buf = Vec::with_capacity(8 + (count as usize) * 95);
        buf.extend_from_slice(&count.to_le_bytes());

        for (key, value) in self
            .db
            .iterator_cf(cf, rocksdb::IteratorMode::Start)
            .flatten()
        {
            if let Ok(entry) = bincode::deserialize::<UtxoEntry>(&value) {
                buf.extend_from_slice(&key);
                buf.extend_from_slice(&entry.serialize_canonical_bytes());
            }
        }
        buf
    }

    /// Load the full ProducerSet from the database.
    ///
    /// Rebuilds the in-memory ProducerSet from cf_producers, cf_exit_history,
    /// and the pending_updates stored in cf_meta.
    pub fn load_producer_set(&self) -> ProducerSet {
        let mut producers = HashMap::new();
        let cf_prod = self.db.cf_handle(CF_PRODUCERS).unwrap();
        for (key, value) in self
            .db
            .iterator_cf(cf_prod, rocksdb::IteratorMode::Start)
            .flatten()
        {
            if key.len() == 32 {
                let hash = Hash::from_bytes(key[..32].try_into().unwrap());
                if let Ok(info) = bincode::deserialize::<ProducerInfo>(&value) {
                    producers.insert(hash, info);
                }
            }
        }

        let mut exit_history = HashMap::new();
        let cf_exit = self.db.cf_handle(CF_EXIT_HISTORY).unwrap();
        for (key, value) in self
            .db
            .iterator_cf(cf_exit, rocksdb::IteratorMode::Start)
            .flatten()
        {
            if key.len() == 32 && value.len() >= 8 {
                let hash = Hash::from_bytes(key[..32].try_into().unwrap());
                let height = u64::from_le_bytes(value[..8].try_into().unwrap());
                exit_history.insert(hash, height);
            }
        }

        let pending_updates = self.get_pending_updates();

        ProducerSet::from_parts(producers, exit_history, pending_updates)
    }
}
