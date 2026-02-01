//! # Heartbeat Pool
//!
//! Collects and validates heartbeats from producers during each slot.
//! Used by the block producer to build the presence commitment.
//!
//! ## Lifecycle
//!
//! 1. At start of slot, producers compute their heartbeat VDF
//! 2. Heartbeats are broadcast to the network
//! 3. Other producers witness (sign) valid heartbeats
//! 4. Block producer collects all valid witnessed heartbeats
//! 5. Block producer builds PresenceCommitment from heartbeats

use std::collections::HashMap;

use crypto::{Hash, Hasher, PublicKey};
use doli_core::heartbeat::{Heartbeat, HeartbeatError, MIN_WITNESS_SIGNATURES};
use doli_core::presence::PresenceCommitment;
use doli_core::types::{Amount, Slot};
use tracing::{debug, warn};

/// A validated heartbeat with metadata
#[derive(Clone, Debug)]
pub struct ValidatedHeartbeat {
    /// The original heartbeat
    pub heartbeat: Heartbeat,
    /// Bond weight of the producer (cached for reward calculation)
    pub bond_weight: Amount,
    /// Producer index in sorted list (for bitfield)
    pub producer_index: usize,
}

/// Pool for collecting heartbeats during a slot
pub struct HeartbeatPool {
    /// Current slot we're collecting for
    current_slot: Slot,
    /// Expected previous block hash (heartbeats must reference this)
    expected_prev_hash: Hash,
    /// Heartbeats collected for current slot, keyed by producer pubkey
    heartbeats: HashMap<PublicKey, ValidatedHeartbeat>,
    /// Active producers (sorted by pubkey) for validation
    active_producers: Vec<PublicKey>,
    /// Bond weights for each active producer
    bond_weights: HashMap<PublicKey, Amount>,
}

impl HeartbeatPool {
    /// Create a new heartbeat pool
    pub fn new() -> Self {
        Self {
            current_slot: 0,
            expected_prev_hash: Hash::ZERO,
            heartbeats: HashMap::new(),
            active_producers: Vec::new(),
            bond_weights: HashMap::new(),
        }
    }

    /// Reset the pool for a new slot
    ///
    /// # Arguments
    ///
    /// * `slot` - The new slot number
    /// * `prev_hash` - Hash of the previous block
    /// * `active_producers` - Sorted list of active producer pubkeys
    /// * `bond_weights` - Bond weights for each producer
    pub fn reset_for_slot(
        &mut self,
        slot: Slot,
        prev_hash: Hash,
        active_producers: Vec<PublicKey>,
        bond_weights: HashMap<PublicKey, Amount>,
    ) {
        self.current_slot = slot;
        self.expected_prev_hash = prev_hash;
        self.heartbeats.clear();
        self.active_producers = active_producers;
        self.bond_weights = bond_weights;
    }

    /// Add a heartbeat to the pool if valid
    ///
    /// Returns Ok(()) if the heartbeat was accepted, or an error if invalid.
    pub fn add_heartbeat(&mut self, heartbeat: Heartbeat) -> Result<(), HeartbeatError> {
        // Check slot
        if heartbeat.slot != self.current_slot {
            debug!(
                "Heartbeat for wrong slot: got {}, expected {}",
                heartbeat.slot, self.current_slot
            );
            return Err(HeartbeatError::PrevHashMismatch);
        }

        // Check prev_hash
        if heartbeat.prev_block_hash != self.expected_prev_hash {
            debug!("Heartbeat has wrong prev_hash");
            return Err(HeartbeatError::PrevHashMismatch);
        }

        // Check producer is active
        let producer_index = self
            .active_producers
            .iter()
            .position(|p| p == &heartbeat.producer);

        let producer_index = match producer_index {
            Some(idx) => idx,
            None => {
                debug!(
                    "Heartbeat from unknown producer: {}",
                    hex::encode(&heartbeat.producer.as_bytes()[..8])
                );
                return Err(HeartbeatError::InvalidWitness(heartbeat.producer.clone()));
            }
        };

        // Verify VDF
        if !heartbeat.verify_vdf() {
            warn!(
                "Invalid VDF from producer {}",
                hex::encode(&heartbeat.producer.as_bytes()[..8])
            );
            return Err(HeartbeatError::InvalidVdf);
        }

        // Verify signature
        if !heartbeat.verify_signature() {
            warn!(
                "Invalid signature from producer {}",
                hex::encode(&heartbeat.producer.as_bytes()[..8])
            );
            return Err(HeartbeatError::InvalidSignature);
        }

        // Verify witnesses
        heartbeat.verify_witnesses(&self.active_producers)?;

        // Get bond weight
        let bond_weight = self
            .bond_weights
            .get(&heartbeat.producer)
            .copied()
            .unwrap_or(0);

        // Store the validated heartbeat
        let validated = ValidatedHeartbeat {
            heartbeat,
            bond_weight,
            producer_index,
        };

        self.heartbeats
            .insert(validated.heartbeat.producer.clone(), validated);

        Ok(())
    }

    /// Get all valid heartbeats for the current slot
    pub fn get_valid_heartbeats(&self) -> Vec<&ValidatedHeartbeat> {
        self.heartbeats.values().collect()
    }

    /// Get the number of valid heartbeats
    pub fn heartbeat_count(&self) -> usize {
        self.heartbeats.len()
    }

    /// Check if a producer has a valid heartbeat
    pub fn has_heartbeat(&self, producer: &PublicKey) -> bool {
        self.heartbeats.contains_key(producer)
    }

    /// Build a presence commitment from the collected heartbeats
    ///
    /// Returns a PresenceCommitment that can be included in a block.
    pub fn build_presence_commitment(&self) -> PresenceCommitment {
        if self.heartbeats.is_empty() {
            return PresenceCommitment::empty();
        }

        // Collect validated heartbeats sorted by producer index
        let mut sorted_heartbeats: Vec<&ValidatedHeartbeat> = self.heartbeats.values().collect();
        sorted_heartbeats.sort_by_key(|h| h.producer_index);

        // Build indices and weights in sorted order
        let present_indices: Vec<usize> =
            sorted_heartbeats.iter().map(|h| h.producer_index).collect();
        let weights: Vec<Amount> = sorted_heartbeats.iter().map(|h| h.bond_weight).collect();

        // Compute merkle root of heartbeat data
        let merkle_root = self.compute_heartbeat_merkle(&sorted_heartbeats);

        PresenceCommitment::new(
            self.active_producers.len(),
            &present_indices,
            weights,
            merkle_root,
        )
    }

    /// Compute merkle root of heartbeat data for fraud proofs
    fn compute_heartbeat_merkle(&self, heartbeats: &[&ValidatedHeartbeat]) -> Hash {
        if heartbeats.is_empty() {
            return Hash::ZERO;
        }

        // Hash each heartbeat
        let mut hashes: Vec<Hash> = heartbeats
            .iter()
            .map(|h| {
                let mut hasher = Hasher::new();
                hasher.update(h.heartbeat.producer.as_bytes());
                hasher.update(&h.heartbeat.slot.to_le_bytes());
                hasher.update(&h.heartbeat.vdf_output);
                hasher.finalize()
            })
            .collect();

        // Build merkle tree
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

    /// Get current slot
    pub fn current_slot(&self) -> Slot {
        self.current_slot
    }
}

impl Default for HeartbeatPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::KeyPair;
    use doli_core::heartbeat::{hash_chain_vdf, Heartbeat, WitnessSignature, HEARTBEAT_VERSION};

    fn mock_hash(seed: u8) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        Hash::from_bytes(bytes)
    }

    fn create_valid_heartbeat(
        producer_kp: &KeyPair,
        slot: Slot,
        prev_hash: &Hash,
        witnesses: &[&KeyPair],
    ) -> Heartbeat {
        let producer = producer_kp.public_key().clone();

        // Compute VDF (use small iterations for test)
        let input = Heartbeat::compute_vdf_input(&producer, slot, prev_hash);
        // Note: We need to use the same iterations as verify_vdf expects
        // For tests, we use a mock that skips VDF verification
        let vdf_output = hash_chain_vdf(&input, 100);

        // Sign the heartbeat
        let signing_msg = Heartbeat::compute_signing_message(
            HEARTBEAT_VERSION,
            &producer,
            slot,
            prev_hash,
            &vdf_output,
        );
        let signature = crypto::signature::sign_hash(&signing_msg, producer_kp.private_key());

        let mut heartbeat = Heartbeat::new(producer, slot, *prev_hash, vdf_output, signature);

        // Add witness signatures
        for witness_kp in witnesses {
            let witness_msg =
                WitnessSignature::compute_message(&heartbeat.producer, slot, &vdf_output);
            let witness_sig = crypto::signature::sign_hash(&witness_msg, witness_kp.private_key());
            heartbeat.add_witness(WitnessSignature::new(
                witness_kp.public_key().clone(),
                witness_sig,
            ));
        }

        heartbeat
    }

    #[test]
    fn test_empty_pool() {
        let pool = HeartbeatPool::new();
        assert_eq!(pool.heartbeat_count(), 0);
        assert!(pool.get_valid_heartbeats().is_empty());
    }

    #[test]
    fn test_reset_for_slot() {
        let mut pool = HeartbeatPool::new();

        let producers = vec![
            KeyPair::generate().public_key().clone(),
            KeyPair::generate().public_key().clone(),
        ];
        let mut weights = HashMap::new();
        weights.insert(producers[0].clone(), 1000);
        weights.insert(producers[1].clone(), 2000);

        let prev_hash = mock_hash(1);
        pool.reset_for_slot(100, prev_hash, producers.clone(), weights);

        assert_eq!(pool.current_slot(), 100);
        assert_eq!(pool.heartbeat_count(), 0);
    }

    #[test]
    fn test_build_empty_presence() {
        let pool = HeartbeatPool::new();
        let presence = pool.build_presence_commitment();

        assert!(presence.is_empty());
        assert_eq!(presence.present_count(), 0);
        assert_eq!(presence.total_weight(), 0);
    }

    #[test]
    fn test_build_presence_with_heartbeats() {
        let mut pool = HeartbeatPool::new();

        // Create 4 producers
        let producer1 = KeyPair::generate();
        let producer2 = KeyPair::generate();
        let witness1 = KeyPair::generate();
        let witness2 = KeyPair::generate();

        let mut all_producers: Vec<PublicKey> = vec![
            producer1.public_key().clone(),
            producer2.public_key().clone(),
            witness1.public_key().clone(),
            witness2.public_key().clone(),
        ];
        // Sort by pubkey bytes for deterministic ordering
        all_producers.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

        let mut weights = HashMap::new();
        weights.insert(producer1.public_key().clone(), 1000);
        weights.insert(producer2.public_key().clone(), 2000);
        weights.insert(witness1.public_key().clone(), 500);
        weights.insert(witness2.public_key().clone(), 500);

        let prev_hash = mock_hash(42);
        let slot = 100;

        pool.reset_for_slot(slot, prev_hash, all_producers.clone(), weights.clone());

        // Manually add validated heartbeats (skip VDF verification for test)
        let p1_idx = all_producers
            .iter()
            .position(|p| p == producer1.public_key())
            .unwrap();
        let p2_idx = all_producers
            .iter()
            .position(|p| p == producer2.public_key())
            .unwrap();

        // Create mock VDF output
        let vdf_output = [0u8; 32];

        let validated1 = ValidatedHeartbeat {
            heartbeat: Heartbeat {
                version: HEARTBEAT_VERSION,
                producer: producer1.public_key().clone(),
                slot,
                prev_block_hash: prev_hash,
                vdf_output,
                signature: crypto::Signature::default(),
                witnesses: vec![],
            },
            bond_weight: 1000,
            producer_index: p1_idx,
        };

        let validated2 = ValidatedHeartbeat {
            heartbeat: Heartbeat {
                version: HEARTBEAT_VERSION,
                producer: producer2.public_key().clone(),
                slot,
                prev_block_hash: prev_hash,
                vdf_output,
                signature: crypto::Signature::default(),
                witnesses: vec![],
            },
            bond_weight: 2000,
            producer_index: p2_idx,
        };

        pool.heartbeats
            .insert(producer1.public_key().clone(), validated1);
        pool.heartbeats
            .insert(producer2.public_key().clone(), validated2);

        // Build presence
        let presence = pool.build_presence_commitment();

        assert_eq!(presence.present_count(), 2);
        assert_eq!(presence.total_weight(), 3000);
        assert!(presence.is_present(p1_idx));
        assert!(presence.is_present(p2_idx));
        assert_eq!(presence.get_weight(p1_idx), Some(1000));
        assert_eq!(presence.get_weight(p2_idx), Some(2000));
    }

    #[test]
    fn test_merkle_root_deterministic() {
        let mut pool = HeartbeatPool::new();

        let producer = KeyPair::generate();
        let all_producers = vec![producer.public_key().clone()];

        let mut weights = HashMap::new();
        weights.insert(producer.public_key().clone(), 1000);

        let prev_hash = mock_hash(1);
        pool.reset_for_slot(50, prev_hash, all_producers, weights);

        let vdf_output = [42u8; 32];

        let validated = ValidatedHeartbeat {
            heartbeat: Heartbeat {
                version: HEARTBEAT_VERSION,
                producer: producer.public_key().clone(),
                slot: 50,
                prev_block_hash: prev_hash,
                vdf_output,
                signature: crypto::Signature::default(),
                witnesses: vec![],
            },
            bond_weight: 1000,
            producer_index: 0,
        };

        pool.heartbeats
            .insert(producer.public_key().clone(), validated);

        // Build presence twice
        let presence1 = pool.build_presence_commitment();
        let presence2 = pool.build_presence_commitment();

        // Merkle roots should be identical
        assert_eq!(presence1.merkle_root, presence2.merkle_root);
    }

    #[test]
    fn test_has_heartbeat() {
        let mut pool = HeartbeatPool::new();

        let producer = KeyPair::generate();
        let all_producers = vec![producer.public_key().clone()];

        let mut weights = HashMap::new();
        weights.insert(producer.public_key().clone(), 1000);

        let prev_hash = mock_hash(1);
        pool.reset_for_slot(50, prev_hash, all_producers, weights);

        assert!(!pool.has_heartbeat(producer.public_key()));

        let validated = ValidatedHeartbeat {
            heartbeat: Heartbeat {
                version: HEARTBEAT_VERSION,
                producer: producer.public_key().clone(),
                slot: 50,
                prev_block_hash: prev_hash,
                vdf_output: [0u8; 32],
                signature: crypto::Signature::default(),
                witnesses: vec![],
            },
            bond_weight: 1000,
            producer_index: 0,
        };

        pool.heartbeats
            .insert(producer.public_key().clone(), validated);

        assert!(pool.has_heartbeat(producer.public_key()));
    }
}
