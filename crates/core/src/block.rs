//! Block types and operations
//!
//! # Deterministic Scheduler Model
//!
//! Blocks no longer contain presence commitments. The deterministic scheduler
//! selects producers based on bond count, and 100% of the block reward goes
//! to the producer who creates the block.

use crypto::{Hash, Hasher, PublicKey};
use serde::{Deserialize, Serialize};
use vdf::{VdfOutput, VdfProof};

use crate::consensus::ConsensusParams;
use crate::transaction::Transaction;
use crate::types::{BlockHeight, Slot};

/// Block header
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockHeader {
    /// Protocol version (2 = genesis_hash added)
    pub version: u32,
    /// Hash of the previous block header
    pub prev_hash: Hash,
    /// Merkle root of transactions
    pub merkle_root: Hash,
    /// Legacy presence root field (always Hash::ZERO in deterministic scheduler model)
    ///
    /// This field is retained for backward compatibility with block hash calculation.
    /// In the deterministic scheduler model, presence commitments are no longer used.
    #[serde(default)]
    pub presence_root: Hash,
    /// Chain identity — BLAKE3 hash of genesis parameters (timestamp, network, slot_duration, message).
    /// Blocks with a mismatched genesis_hash are rejected immediately.
    /// Defaults to ZERO for deserialization of pre-v2 blocks (which are then rejected by validation).
    #[serde(default)]
    pub genesis_hash: Hash,
    /// Block timestamp (Unix seconds)
    pub timestamp: u64,
    /// Slot number (derived from timestamp)
    pub slot: Slot,
    /// Producer's public key
    pub producer: PublicKey,
    /// VDF computation output
    pub vdf_output: VdfOutput,
    /// VDF proof
    pub vdf_proof: VdfProof,
}

impl BlockHeader {
    /// Compute the block hash
    ///
    /// Includes presence_root to ensure presence data cannot be modified
    /// after block creation. This is critical for reward calculation integrity.
    pub fn hash(&self) -> Hash {
        let mut hasher = Hasher::new();
        hasher.update(&self.version.to_le_bytes());
        hasher.update(self.prev_hash.as_bytes());
        hasher.update(self.merkle_root.as_bytes());
        hasher.update(self.presence_root.as_bytes()); // Commit to presence data
        hasher.update(self.genesis_hash.as_bytes()); // Commit to chain identity
        hasher.update(&self.timestamp.to_le_bytes());
        hasher.update(&self.slot.to_le_bytes());
        hasher.update(self.producer.as_bytes());
        hasher.update(&self.vdf_output.value);
        hasher.finalize()
    }

    /// Compute the VDF input for this block
    pub fn vdf_input(&self) -> Hash {
        vdf::block_input(
            &self.prev_hash,
            &self.merkle_root,
            self.slot,
            &self.producer,
        )
    }

    /// Serialize the header
    pub fn serialize(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap_or_default()
    }

    /// Deserialize a header
    pub fn deserialize(bytes: &[u8]) -> Option<Self> {
        bincode::deserialize(bytes).ok()
    }

    /// Get approximate size in bytes
    pub fn size(&self) -> usize {
        // Fixed fields + variable VDF data
        // version(4) + prev_hash(32) + merkle_root(32) + presence_root(32) +
        // timestamp(8) + slot(4) + producer(32) + vdf_output + vdf_proof
        4 + 32 + 32 + 32 + 8 + 4 + 32 + self.vdf_output.value.len() + self.vdf_proof.pi.len()
    }

    /// For version >= 2 blocks, `presence_root` contains the attestation commitment
    /// (Merkle root of RegionAggregates from the previous slot).
    ///
    /// Returns `None` for version 1 blocks where presence_root is unused.
    pub fn attestation_commitment(&self) -> Option<Hash> {
        if self.version >= 2 {
            Some(self.presence_root)
        } else {
            None
        }
    }
}

/// A complete block
///
/// In the deterministic scheduler model, blocks contain only the header and
/// transactions. The producer receives 100% of the block reward via coinbase.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    /// Block header
    pub header: BlockHeader,
    /// Block transactions
    pub transactions: Vec<Transaction>,
    /// BLS aggregate signature over the attestation bitfield (96 bytes).
    ///
    /// Aggregates the BLS signatures of all producers whose bits are set in
    /// `header.presence_root`. Empty for pre-BLS blocks (backward compat).
    /// Stored in body (not header) to keep header hash stable.
    #[serde(default)]
    pub aggregate_bls_signature: Vec<u8>,
}

impl Block {
    /// Create a new block
    pub fn new(header: BlockHeader, transactions: Vec<Transaction>) -> Self {
        Self {
            header,
            transactions,
            aggregate_bls_signature: Vec::new(),
        }
    }

    /// Get the block hash
    pub fn hash(&self) -> Hash {
        self.header.hash()
    }

    /// Get the previous block hash
    pub fn prev_hash(&self) -> &Hash {
        &self.header.prev_hash
    }

    /// Get the slot
    pub fn slot(&self) -> Slot {
        self.header.slot
    }

    /// Get the timestamp
    pub fn timestamp(&self) -> u64 {
        self.header.timestamp
    }

    /// Get the producer
    pub fn producer(&self) -> &PublicKey {
        &self.header.producer
    }

    /// Check if this is a genesis block
    pub fn is_genesis(&self) -> bool {
        self.header.prev_hash.is_zero()
    }

    /// Compute merkle root of transactions
    pub fn compute_merkle_root(&self) -> Hash {
        compute_merkle_root(&self.transactions)
    }

    /// Verify that the stored merkle root matches transactions
    pub fn verify_merkle_root(&self) -> bool {
        self.header.merkle_root == self.compute_merkle_root()
    }

    /// Get the coinbase transaction (first tx)
    pub fn coinbase(&self) -> Option<&Transaction> {
        self.transactions.first()
    }

    /// Get total fees (requires UTXO lookup - placeholder)
    pub fn total_fees(&self) -> u64 {
        // Would need UTXO set to compute actual fees
        0
    }

    /// Serialize the block
    pub fn serialize(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap_or_default()
    }

    /// Deserialize a block
    pub fn deserialize(bytes: &[u8]) -> Option<Self> {
        bincode::deserialize(bytes).ok()
    }

    /// Get the size in bytes
    pub fn size(&self) -> usize {
        self.serialize().len()
    }
}

/// Compute the merkle root of a list of transactions
pub fn compute_merkle_root(transactions: &[Transaction]) -> Hash {
    if transactions.is_empty() {
        return crypto::hash::hash(b"");
    }

    let mut hashes: Vec<Hash> = transactions.iter().map(|tx| tx.hash()).collect();

    while hashes.len() > 1 {
        // Duplicate last hash if odd number
        if hashes.len() % 2 == 1 {
            hashes.push(*hashes.last().unwrap());
        }

        let mut next_level = Vec::with_capacity(hashes.len() / 2);
        for chunk in hashes.chunks(2) {
            let mut hasher = Hasher::new();
            hasher.update(chunk[0].as_bytes());
            hasher.update(chunk[1].as_bytes());
            next_level.push(hasher.finalize());
        }
        hashes = next_level;
    }

    hashes[0]
}

/// Block builder for creating new blocks
pub struct BlockBuilder {
    prev_hash: Hash,
    prev_slot: Slot,
    producer: PublicKey,
    transactions: Vec<Transaction>,
    params: ConsensusParams,
    genesis_hash: Hash,
    presence_root: Hash,
}

impl BlockBuilder {
    /// Create a new block builder
    pub fn new(prev_hash: Hash, prev_slot: Slot, producer: PublicKey) -> Self {
        let params = ConsensusParams::mainnet();
        let genesis_hash = params.genesis_hash;
        Self {
            prev_hash,
            prev_slot,
            producer,
            transactions: Vec::new(),
            params,
            genesis_hash,
            presence_root: Hash::ZERO,
        }
    }

    /// Set consensus params (also updates genesis_hash from params)
    pub fn with_params(mut self, params: ConsensusParams) -> Self {
        self.genesis_hash = params.genesis_hash;
        self.params = params;
        self
    }

    /// Set the presence_root (attestation bitfield commitment).
    pub fn with_presence_root(mut self, root: Hash) -> Self {
        self.presence_root = root;
        self
    }

    /// Add a transaction
    pub fn add_transaction(&mut self, tx: Transaction) -> &mut Self {
        self.transactions.push(tx);
        self
    }

    /// Add the coinbase transaction
    pub fn add_coinbase(&mut self, height: BlockHeight, pubkey_hash: Hash) -> &mut Self {
        let reward = self.params.block_reward(height);
        let coinbase = Transaction::new_coinbase(reward, pubkey_hash, height);
        self.transactions.insert(0, coinbase);
        self
    }

    /// Build the block header and return the finalized transaction list.
    ///
    /// The merkle root is computed from exactly the transactions returned,
    /// guaranteeing consistency between header and block body.
    ///
    /// Returns `None` if the timestamp would produce a slot <= prev_slot
    /// (slot monotonicity violation).
    pub fn build(self, timestamp: u64) -> Option<(BlockHeader, Vec<Transaction>)> {
        let slot = self.params.timestamp_to_slot(timestamp);

        // Enforce slot monotonicity: new slot must be > prev_slot
        // Exception: genesis block (prev_slot == 0) can have slot == 0
        if slot <= self.prev_slot && self.prev_slot > 0 {
            return None;
        }

        let merkle_root = compute_merkle_root(&self.transactions);

        let header = BlockHeader {
            version: 2,
            prev_hash: self.prev_hash,
            merkle_root,
            presence_root: self.presence_root,
            genesis_hash: self.genesis_hash,
            timestamp,
            slot,
            producer: self.producer,
            vdf_output: VdfOutput { value: Vec::new() },
            vdf_proof: VdfProof::empty(),
        };

        Some((header, self.transactions))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merkle_root_empty() {
        let root = compute_merkle_root(&[]);
        assert!(!root.is_zero());
    }

    #[test]
    fn test_merkle_root_single() {
        let tx = Transaction::new_coinbase(100, Hash::ZERO, 0);
        let root = compute_merkle_root(std::slice::from_ref(&tx));
        assert_eq!(root, tx.hash());
    }

    #[test]
    fn test_merkle_root_deterministic() {
        let tx1 = Transaction::new_coinbase(100, Hash::ZERO, 0);
        let tx2 = Transaction::new_coinbase(200, Hash::ZERO, 1);

        let root1 = compute_merkle_root(&[tx1.clone(), tx2.clone()]);
        let root2 = compute_merkle_root(&[tx1, tx2]);

        assert_eq!(root1, root2);
    }

    #[test]
    fn test_block_hash_deterministic() {
        let header = BlockHeader {
            version: 1,
            prev_hash: Hash::ZERO,
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: 1000,
            slot: 0,
            producer: PublicKey::from_bytes([0u8; 32]),
            vdf_output: VdfOutput {
                value: vec![1, 2, 3],
            },
            vdf_proof: VdfProof::empty(),
        };

        let hash1 = header.hash();
        let hash2 = header.hash();

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_block_hash_includes_presence_root() {
        let header1 = BlockHeader {
            version: 1,
            prev_hash: Hash::ZERO,
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: 1000,
            slot: 0,
            producer: PublicKey::from_bytes([0u8; 32]),
            vdf_output: VdfOutput {
                value: vec![1, 2, 3],
            },
            vdf_proof: VdfProof::empty(),
        };

        // Different presence_root should produce different hash
        let header2 = BlockHeader {
            presence_root: crypto::hash::hash(b"different"),
            ..header1.clone()
        };

        assert_ne!(
            header1.hash(),
            header2.hash(),
            "presence_root must affect block hash"
        );
    }

    #[test]
    fn test_block_builder_slot_monotonicity() {
        let producer = PublicKey::from_bytes([1u8; 32]);
        let params = ConsensusParams::mainnet();

        // Genesis case: prev_slot=0, slot=0 should work
        let builder = BlockBuilder::new(Hash::ZERO, 0, producer);
        let result = builder.build(params.genesis_time);
        assert!(result.is_some());
        let (header, _txs) = result.unwrap();
        assert_eq!(header.slot, 0);

        // Normal case: slot > prev_slot should work
        let builder = BlockBuilder::new(Hash::ZERO, 5, producer);
        // 6 slots after genesis = 6 * 60 seconds
        let timestamp = params.genesis_time + 6 * params.slot_duration;
        let result = builder.build(timestamp);
        assert!(result.is_some());
        let (header, _txs) = result.unwrap();
        assert!(header.slot > 5);

        // Violation: slot <= prev_slot should return None
        let builder = BlockBuilder::new(Hash::ZERO, 10, producer);
        // Only 5 slots after genesis, but prev_slot is 10
        let timestamp = params.genesis_time + 5 * params.slot_duration;
        let result = builder.build(timestamp);
        assert!(result.is_none(), "Should reject slot <= prev_slot");

        // Violation: same slot should return None
        let builder = BlockBuilder::new(Hash::ZERO, 10, producer);
        let timestamp = params.genesis_time + 10 * params.slot_duration;
        let result = builder.build(timestamp);
        assert!(result.is_none(), "Should reject slot == prev_slot");
    }
}
