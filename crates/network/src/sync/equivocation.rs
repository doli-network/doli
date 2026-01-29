//! Equivocation detection for double-signing
//!
//! This module detects when a producer creates two different blocks for the same slot,
//! which is the only slashable offense in DOLI. Double production cannot happen by
//! accident - it requires intentionally signing two different blocks for the same slot.
//!
//! The evidence includes full block headers with VDF proofs, which cryptographically
//! prove that the producer actually created both blocks. This prevents fabricated
//! evidence attacks where a malicious reporter could invent fake hashes.

use crypto::{signature, Hash, KeyPair, PublicKey};
use doli_core::transaction::{SlashData, SlashingEvidence};
use doli_core::{Block, BlockHeader, Transaction};
use std::collections::{HashMap, VecDeque};
use tracing::warn;

/// Maximum number of slots to track for equivocation detection
const MAX_TRACKED_SLOTS: usize = 1000;

/// Evidence of double production that can be used to create a slash transaction
///
/// Contains full block headers (not just hashes) so that validators can verify
/// the VDF proofs and confirm the producer actually created both blocks.
#[derive(Clone, Debug)]
pub struct EquivocationProof {
    /// Producer who committed the offense
    pub producer: PublicKey,
    /// First block header (complete, for VDF verification)
    pub block_header_1: BlockHeader,
    /// Second block header (complete, for VDF verification)
    pub block_header_2: BlockHeader,
    /// Slot where double production occurred
    pub slot: u32,
}

impl EquivocationProof {
    /// Convert to SlashData for creating a slash transaction
    ///
    /// The evidence includes full block headers with VDF proofs, allowing
    /// any validator to cryptographically verify the producer actually
    /// created both blocks.
    pub fn to_slash_data(&self, reporter_keypair: &KeyPair) -> SlashData {
        let evidence = SlashingEvidence::DoubleProduction {
            block_header_1: self.block_header_1.clone(),
            block_header_2: self.block_header_2.clone(),
        };

        // Sign the evidence to prove the reporter witnessed it
        let evidence_bytes = bincode::serialize(&evidence).unwrap_or_default();
        let reporter_signature = signature::sign(&evidence_bytes, reporter_keypair.private_key());

        SlashData {
            producer_pubkey: self.producer.clone(),
            evidence,
            reporter_signature,
        }
    }

    /// Create a slash transaction from this proof
    pub fn to_slash_transaction(&self, reporter_keypair: &KeyPair) -> Transaction {
        let slash_data = self.to_slash_data(reporter_keypair);
        Transaction::new_slash_producer(slash_data)
    }
}

/// Tracks block production to detect equivocation (double signing)
///
/// This detector keeps track of recently produced blocks by (producer, slot) pairs.
/// When it sees a second block from the same producer for the same slot with a
/// different hash, it creates an EquivocationProof that can be used to slash
/// the misbehaving producer.
///
/// The detector stores full block headers (not just hashes) so that the
/// equivocation evidence includes VDF proofs for cryptographic verification.
#[derive(Debug)]
pub struct EquivocationDetector {
    /// Map of (producer_pubkey, slot) -> first block header seen
    /// If a second different block is seen for the same key, equivocation is detected
    /// Stores full headers for VDF-verifiable evidence
    seen_blocks: HashMap<(PublicKey, u32), BlockHeader>,

    /// LRU order for eviction (oldest first)
    /// Stores (producer_pubkey, slot) tuples in order of insertion
    lru_order: VecDeque<(PublicKey, u32)>,

    /// Maximum number of entries to track
    max_tracked: usize,

    /// Detected equivocation proofs ready to be reported
    pending_proofs: Vec<EquivocationProof>,
}

impl Default for EquivocationDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl EquivocationDetector {
    /// Create a new equivocation detector
    pub fn new() -> Self {
        Self {
            seen_blocks: HashMap::new(),
            lru_order: VecDeque::new(),
            max_tracked: MAX_TRACKED_SLOTS,
            pending_proofs: Vec::new(),
        }
    }

    /// Check a block for equivocation
    ///
    /// Returns Some(EquivocationProof) if this block represents double production
    /// by its producer (i.e., we've already seen a different block from the same
    /// producer for the same slot).
    ///
    /// The proof includes full block headers with VDF proofs for cryptographic
    /// verification that the producer actually created both blocks.
    pub fn check_block(&mut self, block: &Block) -> Option<EquivocationProof> {
        let producer = block.header.producer.clone();
        let slot = block.header.slot;
        let block_hash = block.hash();
        let key = (producer.clone(), slot);

        // Check if we've already seen a block for this (producer, slot)
        if let Some(existing_header) = self.seen_blocks.get(&key) {
            if existing_header.hash() != block_hash {
                // EQUIVOCATION DETECTED!
                let proof = EquivocationProof {
                    producer: producer.clone(),
                    block_header_1: existing_header.clone(),
                    block_header_2: block.header.clone(),
                    slot,
                };

                warn!(
                    "EQUIVOCATION DETECTED: Producer {} created two blocks for slot {}: {} and {}",
                    crypto::hash::hash(producer.as_bytes()),
                    slot,
                    existing_header.hash(),
                    block_hash
                );

                // Store for later retrieval
                self.pending_proofs.push(proof.clone());

                return Some(proof);
            }
            // Same block seen twice, no equivocation
            return None;
        }

        // First time seeing a block for this (producer, slot)
        self.record_block(key, block.header.clone());
        None
    }

    /// Record a new block header in the tracker
    fn record_block(&mut self, key: (PublicKey, u32), header: BlockHeader) {
        // Evict oldest if at capacity
        while self.seen_blocks.len() >= self.max_tracked {
            if let Some(old_key) = self.lru_order.pop_front() {
                self.seen_blocks.remove(&old_key);
            } else {
                break;
            }
        }

        // Insert new entry
        self.seen_blocks.insert(key.clone(), header);
        self.lru_order.push_back(key);
    }

    /// Take all pending equivocation proofs
    ///
    /// This drains the internal list of detected proofs.
    pub fn take_pending_proofs(&mut self) -> Vec<EquivocationProof> {
        std::mem::take(&mut self.pending_proofs)
    }

    /// Check if there are pending proofs
    pub fn has_pending_proofs(&self) -> bool {
        !self.pending_proofs.is_empty()
    }

    /// Get the number of tracked (producer, slot) pairs
    pub fn tracked_count(&self) -> usize {
        self.seen_blocks.len()
    }

    /// Clear all tracked data
    pub fn clear(&mut self) {
        self.seen_blocks.clear();
        self.lru_order.clear();
        self.pending_proofs.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use doli_core::BlockHeader;
    use vdf::{VdfOutput, VdfProof};

    fn create_test_block(slot: u32, producer: &PublicKey, merkle_root: Hash) -> Block {
        let header = BlockHeader {
            version: 1,
            prev_hash: Hash::ZERO,
            merkle_root,
            timestamp: 0,
            slot,
            producer: producer.clone(),
            vdf_output: VdfOutput { value: vec![] },
            vdf_proof: VdfProof::empty(),
        };
        Block::new(header, vec![])
    }

    #[test]
    fn test_no_equivocation_same_block() {
        let mut detector = EquivocationDetector::new();
        let producer = PublicKey::from_bytes([1u8; 32]);

        let block = create_test_block(10, &producer, Hash::ZERO);

        // First time - no equivocation
        assert!(detector.check_block(&block).is_none());

        // Same block again - still no equivocation
        assert!(detector.check_block(&block).is_none());
    }

    #[test]
    fn test_no_equivocation_different_slots() {
        let mut detector = EquivocationDetector::new();
        let producer = PublicKey::from_bytes([1u8; 32]);

        let block1 = create_test_block(10, &producer, Hash::ZERO);
        let block2 = create_test_block(11, &producer, crypto::hash::hash(b"diff"));

        // Different slots - no equivocation
        assert!(detector.check_block(&block1).is_none());
        assert!(detector.check_block(&block2).is_none());
    }

    #[test]
    fn test_no_equivocation_different_producers() {
        let mut detector = EquivocationDetector::new();
        let producer1 = PublicKey::from_bytes([1u8; 32]);
        let producer2 = PublicKey::from_bytes([2u8; 32]);

        let block1 = create_test_block(10, &producer1, Hash::ZERO);
        let block2 = create_test_block(10, &producer2, crypto::hash::hash(b"diff"));

        // Same slot but different producers - no equivocation
        assert!(detector.check_block(&block1).is_none());
        assert!(detector.check_block(&block2).is_none());
    }

    #[test]
    fn test_detect_equivocation() {
        let mut detector = EquivocationDetector::new();
        let producer = PublicKey::from_bytes([1u8; 32]);

        // Same producer, same slot, different blocks
        let block1 = create_test_block(10, &producer, Hash::ZERO);
        let block2 = create_test_block(10, &producer, crypto::hash::hash(b"different"));

        // First block - no equivocation
        assert!(detector.check_block(&block1).is_none());

        // Second different block for same slot - EQUIVOCATION!
        let proof = detector.check_block(&block2);
        assert!(proof.is_some());

        let proof = proof.unwrap();
        assert_eq!(proof.producer, producer);
        assert_eq!(proof.slot, 10);
        assert_eq!(proof.block_header_1.hash(), block1.hash());
        assert_eq!(proof.block_header_2.hash(), block2.hash());

        // Should also be in pending proofs
        assert!(detector.has_pending_proofs());
        let pending = detector.take_pending_proofs();
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn test_proof_to_slash_transaction() {
        let producer = PublicKey::from_bytes([1u8; 32]);
        let reporter = KeyPair::generate();

        // Create test block headers
        let header1 = BlockHeader {
            version: 1,
            prev_hash: Hash::ZERO,
            merkle_root: Hash::ZERO,
            timestamp: 0,
            slot: 42,
            producer: producer.clone(),
            vdf_output: VdfOutput { value: vec![] },
            vdf_proof: VdfProof::empty(),
        };
        let header2 = BlockHeader {
            version: 1,
            prev_hash: Hash::ZERO,
            merkle_root: crypto::hash::hash(b"different"),
            timestamp: 0,
            slot: 42,
            producer: producer.clone(),
            vdf_output: VdfOutput { value: vec![] },
            vdf_proof: VdfProof::empty(),
        };

        let proof = EquivocationProof {
            producer: producer.clone(),
            block_header_1: header1.clone(),
            block_header_2: header2.clone(),
            slot: 42,
        };

        let slash_tx = proof.to_slash_transaction(&reporter);

        assert!(slash_tx.is_slash_producer());
        let slash_data = slash_tx.slash_data().unwrap();
        assert_eq!(slash_data.producer_pubkey, producer);

        if let SlashingEvidence::DoubleProduction {
            block_header_1,
            block_header_2,
        } = slash_data.evidence
        {
            assert_eq!(block_header_1.slot, 42);
            assert_eq!(block_header_2.slot, 42);
            assert_eq!(block_header_1.hash(), header1.hash());
            assert_eq!(block_header_2.hash(), header2.hash());
        } else {
            panic!("Expected DoubleProduction evidence");
        }
    }

    #[test]
    fn test_eviction() {
        let mut detector = EquivocationDetector::new();
        detector.max_tracked = 10;

        // Add more than max entries
        for i in 0..20u32 {
            let producer = PublicKey::from_bytes([i as u8; 32]);
            let block = create_test_block(i, &producer, Hash::ZERO);
            detector.check_block(&block);
        }

        // Should only have max_tracked entries
        assert!(detector.tracked_count() <= 10);
    }
}
