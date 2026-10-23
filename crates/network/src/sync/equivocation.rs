//! Equivocation detection for double-signing
//!
//! This module detects when a producer creates two different blocks for the same slot,
//! which is the only slashable offense in DOLI. Double production cannot happen by
//! accident - it requires intentionally signing two different blocks for the same slot.
//!
//! The evidence includes full block headers. Authorship is proven by the producer's
//! block signature (`vdf_proof.pi`, INC-I-236, enforced from the AH), not by the VDF.

use crypto::{signature, Hash, KeyPair, PublicKey};
use doli_core::consensus::MAX_FUTURE_SLOTS;
use doli_core::transaction::{SlashData, SlashingEvidence};
use doli_core::validation::double_production_evidence_fits;
use doli_core::{Block, BlockHeader, Transaction};
use std::collections::{HashMap, VecDeque};
use tracing::warn;

/// Maximum number of entries to track for equivocation detection.
/// At 360 producers per epoch, 10K entries covers ~28 slots.
const MAX_TRACKED_ENTRIES: usize = 10_000;

/// Number of slots to retain in the sliding window (1 epoch).
/// Entries older than this are cleaned up to bound memory.
const SLIDING_WINDOW_SLOTS: u32 = 360;

/// Most proofs held until taken; further offences are still returned.
const MAX_PENDING_EQUIVOCATION_PROOFS: usize = 64;

/// `admit` calls allowed for a (producer, slot) after its first proof.
pub const MAX_EVIDENCE_CHECKS_AFTER_PROOF: u32 = 4;

/// Evidence of double production that can be used to create a slash transaction
///
/// Contains full block headers (not just hashes) so that validators can verify
/// each header's producer block signature (INC-I-236); the VDF does not prove authorship.
#[derive(Clone, Debug)]
pub struct EquivocationProof {
    /// Producer who committed the offense
    pub producer: PublicKey,
    /// First block header (complete, for signature verification)
    pub block_header_1: BlockHeader,
    /// Second block header (complete, for signature verification)
    pub block_header_2: BlockHeader,
    /// Slot where double production occurred
    pub slot: u32,
}

impl EquivocationProof {
    /// Convert to SlashData for creating a slash transaction
    ///
    /// The evidence includes full block headers; validators check each header's
    /// producer block signature (INC-I-236) to confirm authorship.
    pub fn to_slash_data(&self, reporter_keypair: &KeyPair) -> SlashData {
        let evidence = SlashingEvidence::DoubleProduction {
            block_header_1: self.block_header_1.clone(),
            block_header_2: self.block_header_2.clone(),
        };

        // Sign the evidence to prove the reporter witnessed it
        let evidence_bytes = bincode::serialize(&evidence).unwrap_or_default();
        let reporter_signature = signature::sign(&evidence_bytes, reporter_keypair.private_key());

        SlashData {
            producer_pubkey: self.producer,
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
/// Keyed by (producer, slot). Only headers the caller's `admit` predicate
/// accepts are stored or paired; the smaller-encoded header is kept so a pair
/// that fits a SlashData can still form.
#[derive(Debug)]
pub struct EquivocationDetector {
    /// (producer, slot) -> the stored admitted header
    seen_blocks: HashMap<(PublicKey, u32), Tracked>,

    /// LRU order for eviction (oldest first)
    lru_order: VecDeque<(PublicKey, u32)>,

    /// Maximum number of entries to track (secondary cap)
    max_tracked: usize,

    /// Detected equivocation proofs, at most one per (producer, slot)
    pending_proofs: Vec<EquivocationProof>,

    /// No tracked entry has a slot below this.
    oldest_tracked_slot: u32,

    /// Set at the first `note_gate` crossing; `clear()` keeps it.
    gate_crossed: bool,
}

/// The stored header for one (producer, slot).
#[derive(Debug)]
struct Tracked {
    header: BlockHeader,
    hash: Hash,
    size: u64,
    /// `admit` calls since the first proof; `None` before it.
    checks_after_proof: Option<u32>,
}

impl Tracked {
    fn new(header: &BlockHeader, hash: Hash) -> Self {
        Self {
            size: bincode::serialized_size(header).unwrap_or(u64::MAX),
            header: header.clone(),
            hash,
            checks_after_proof: None,
        }
    }
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
            max_tracked: MAX_TRACKED_ENTRIES,
            pending_proofs: Vec::new(),
            oldest_tracked_slot: 0,
            gate_crossed: false,
        }
    }

    /// `check_block_with` admitting every header, with the header's slot as the wall slot.
    pub fn check_block(&mut self, block: &Block) -> Option<EquivocationProof> {
        self.check_block_with(block, block.header.slot, |_| true)
    }

    /// Check a block for equivocation. Returns a proof when an admitted header
    /// differs from the stored admitted header for its (producer, slot) and the
    /// pair fits a SlashData. `admit` is not called for the stored header, nor
    /// after `MAX_EVIDENCE_CHECKS_AFTER_PROOF` calls once (producer, slot) proved.
    /// A header past `wall_slot + MAX_FUTURE_SLOTS` is skipped; the window is
    /// bounded by `wall_slot` only.
    pub fn check_block_with(
        &mut self,
        block: &Block,
        wall_slot: u32,
        admit: impl FnOnce(&BlockHeader) -> bool,
    ) -> Option<EquivocationProof> {
        if u64::from(block.header.slot) > u64::from(wall_slot).saturating_add(MAX_FUTURE_SLOTS) {
            return None;
        }
        let proof = self.check_admitted(block, admit);
        self.cleanup_before_slot(wall_slot.saturating_sub(SLIDING_WINDOW_SLOTS));
        proof
    }

    /// Drop every stored header once, at the first `prospective_height >= gate`.
    pub fn note_gate(&mut self, prospective_height: u64, gate: u64) {
        if !self.gate_crossed && prospective_height >= gate {
            self.clear();
            self.gate_crossed = true;
        }
    }

    fn check_admitted(
        &mut self,
        block: &Block,
        admit: impl FnOnce(&BlockHeader) -> bool,
    ) -> Option<EquivocationProof> {
        let header = &block.header;
        let key = (header.producer, header.slot);
        let hash = block.hash();

        if let Some(stored) = self.seen_blocks.get_mut(&key) {
            if stored.hash == hash {
                return None;
            }
            if let Some(n) = stored.checks_after_proof.as_mut() {
                if *n >= MAX_EVIDENCE_CHECKS_AFTER_PROOF {
                    return None;
                }
                *n += 1;
            }
        }
        if !admit(header) {
            return None;
        }
        let candidate = Tracked::new(header, hash);
        let Some(stored) = self.seen_blocks.get_mut(&key) else {
            self.record_block(key, candidate);
            return None;
        };

        let proof = double_production_evidence_fits(&key.0, &stored.header, header).then(|| {
            EquivocationProof {
                producer: key.0,
                block_header_1: stored.header.clone(),
                block_header_2: header.clone(),
                slot: key.1,
            }
        });
        if proof.is_some() && stored.checks_after_proof.is_none() {
            stored.checks_after_proof = Some(0);
        }
        if candidate.size < stored.size {
            let checks = stored.checks_after_proof;
            *stored = Tracked {
                checks_after_proof: checks,
                ..candidate
            };
        }
        let proof = proof?;

        warn!(
            "EQUIVOCATION DETECTED: Producer {} created two blocks for slot {}: {} and {}",
            crypto::hash::hash(proof.producer.as_bytes()),
            proof.slot,
            proof.block_header_1.hash(),
            hash
        );
        self.queue_pending(&proof);
        Some(proof)
    }

    /// Queue `proof` unless its (producer, slot) is pending or the cap is reached.
    fn queue_pending(&mut self, proof: &EquivocationProof) {
        let queued = self
            .pending_proofs
            .iter()
            .any(|p| p.producer == proof.producer && p.slot == proof.slot);
        if !queued && self.pending_proofs.len() < MAX_PENDING_EQUIVOCATION_PROOFS {
            self.pending_proofs.push(proof.clone());
        }
    }

    /// Record a new block header in the tracker
    fn record_block(&mut self, key: (PublicKey, u32), tracked: Tracked) {
        // Evict oldest if at capacity
        while self.seen_blocks.len() >= self.max_tracked {
            if let Some(old_key) = self.lru_order.pop_front() {
                self.seen_blocks.remove(&old_key);
            } else {
                break;
            }
        }

        self.seen_blocks.insert(key, tracked);
        self.lru_order.push_back(key);
        self.oldest_tracked_slot = self.oldest_tracked_slot.min(key.1);
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
        self.oldest_tracked_slot = 0;
    }

    /// Remove every entry with slot < min_slot, whatever its arrival order.
    pub fn cleanup_before_slot(&mut self, min_slot: u32) {
        if min_slot <= self.oldest_tracked_slot {
            return;
        }
        let seen = &mut self.seen_blocks;
        self.lru_order.retain(|key| {
            let keep = key.1 >= min_slot;
            if !keep {
                seen.remove(key);
            }
            keep
        });
        self.oldest_tracked_slot = min_slot;
    }
}

#[cfg(test)]
#[path = "equivocation_tests.rs"]
mod tests;
