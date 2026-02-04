//! # TPoP Producer Node Implementation
//!
//! This module implements the producer side of Temporal Proof of Presence.
//!
//! ## Producer Responsibilities
//!
//! 1. **Continuous VDF Computation**: Maintain the presence chain 24/7
//! 2. **Proof Broadcasting**: Broadcast PresenceProof each slot
//! 3. **Block Production**: Produce blocks when eligible based on rank
//! 4. **Checkpoint Participation**: Submit presence updates at checkpoints

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, error, info};

use crypto::{Hash, KeyPair, PublicKey};
use vdf::{VdfOutput, VdfProof};

use crate::block::Block;
use crate::consensus::ConsensusParams;
use crate::types::{BlockHeight, Slot};

#[allow(deprecated)] // HEARTBEAT_VDF_ITERATIONS - telemetry only
use super::heartbeat::{hash_chain_vdf, HEARTBEAT_VDF_ITERATIONS};
use super::presence::{
    can_produce_at_time, producer_eligibility_offset, rank_producers_by_presence,
    EpochPresenceRecords, PresenceCheckpoint, PresenceProof, ProducerPresenceState, VdfLink,
    MAX_CHAIN_LENGTH,
};

/// State of the local presence chain being computed
#[derive(Debug)]
pub struct LocalPresenceChain {
    /// Producer's keypair
    keypair: KeyPair,

    /// Current VDF chain (since last checkpoint)
    chain: Vec<VdfLink>,

    /// VDF currently being computed (if any)
    computing: Option<VdfComputation>,

    /// Last checkpoint we're chaining from
    checkpoint_height: BlockHeight,
    checkpoint_hash: Hash,

    /// Our current presence state
    state: ProducerPresenceState,
}

/// In-progress VDF computation
#[derive(Debug)]
struct VdfComputation {
    target_slot: Slot,
    input_hash: Hash,
    started_at: Instant,
}

impl LocalPresenceChain {
    /// Create a new presence chain for a producer
    pub fn new(keypair: KeyPair, checkpoint: &PresenceCheckpoint) -> Self {
        let pubkey = keypair.public_key();

        // Get existing state from checkpoint or create new
        let state = checkpoint
            .get_producer_state(pubkey)
            .cloned()
            .unwrap_or_else(|| ProducerPresenceState::new(pubkey.clone(), 0));

        Self {
            keypair,
            chain: Vec::new(),
            computing: None,
            checkpoint_height: checkpoint.height,
            checkpoint_hash: checkpoint.hash(),
            state,
        }
    }

    /// Get the last VDF output (for chaining)
    fn last_output(&self) -> Vec<u8> {
        self.chain
            .last()
            .map(|link| link.output.value.clone())
            .unwrap_or_else(|| self.state.last_vdf_output.clone())
    }

    /// Get the last sequence number
    fn last_sequence(&self) -> u64 {
        self.chain
            .last()
            .map(|link| link.sequence)
            .unwrap_or(self.state.last_sequence)
    }

    /// Start computing VDF for a slot
    pub fn start_vdf_for_slot(&mut self, slot: Slot) {
        let prev_output = self.last_output();
        let input_hash = VdfLink::compute_input(&prev_output, slot, self.keypair.public_key());

        debug!(
            "Starting VDF computation for slot {} (input: {})",
            slot,
            &input_hash.to_hex()[..16]
        );

        self.computing = Some(VdfComputation {
            target_slot: slot,
            input_hash,
            started_at: Instant::now(),
        });
    }

    /// Complete a VDF computation (called when VDF finishes)
    pub fn complete_vdf(&mut self, output: VdfOutput, proof: VdfProof) -> Option<VdfLink> {
        let computation = self.computing.take()?;

        let elapsed = computation.started_at.elapsed();
        debug!(
            "VDF completed for slot {} in {:?}",
            computation.target_slot, elapsed
        );

        let link = VdfLink {
            sequence: self.last_sequence() + 1,
            slot: computation.target_slot,
            input_hash: computation.input_hash,
            output,
            proof,
        };

        // Add to chain
        self.chain.push(link.clone());

        // Prune if chain too long (keep last MAX_CHAIN_LENGTH / 2 on prune)
        if self.chain.len() > MAX_CHAIN_LENGTH {
            let keep_from = self.chain.len() - MAX_CHAIN_LENGTH / 2;
            self.chain = self.chain.split_off(keep_from);
        }

        Some(link)
    }

    /// Create a presence proof for the current slot
    pub fn create_proof(&self, slot: Slot) -> Option<PresenceProof> {
        // Check we have a chain that covers this slot
        if self.chain.is_empty() {
            return None;
        }

        // Find the link for this slot
        let last_link = self.chain.last()?;
        if last_link.slot != slot {
            return None;
        }

        Some(PresenceProof {
            producer: self.keypair.public_key().clone(),
            slot,
            vdf_chain: self.chain.clone(),
            checkpoint_height: self.checkpoint_height,
            checkpoint_hash: self.checkpoint_hash,
        })
    }

    /// Update state after a new checkpoint
    pub fn apply_checkpoint(&mut self, checkpoint: &PresenceCheckpoint) {
        // Update checkpoint reference
        self.checkpoint_height = checkpoint.height;
        self.checkpoint_hash = checkpoint.hash();

        // Update our state from checkpoint
        if let Some(state) = checkpoint.get_producer_state(self.keypair.public_key()) {
            self.state = state.clone();
        }

        // Clear chain (checkpoint is new anchor point)
        self.chain.clear();
    }

    /// Get our current presence score
    pub fn presence_score(&self) -> u64 {
        self.state.presence_score
    }

    /// Get our public key
    pub fn public_key(&self) -> &PublicKey {
        self.keypair.public_key()
    }

    /// Check if VDF computation is in progress
    pub fn is_computing(&self) -> bool {
        self.computing.is_some()
    }

    /// Get current target slot for VDF computation
    pub fn computing_for_slot(&self) -> Option<Slot> {
        self.computing.as_ref().map(|c| c.target_slot)
    }
}

/// The TPoP producer service
///
/// Runs continuously to:
/// 1. Compute VDFs for each slot
/// 2. Broadcast presence proofs
/// 3. Produce blocks when eligible
pub struct PresenceProducer {
    /// Local presence chain
    chain: Arc<RwLock<LocalPresenceChain>>,

    /// Consensus parameters
    params: ConsensusParams,

    /// Current checkpoint
    checkpoint: Arc<RwLock<PresenceCheckpoint>>,

    /// Epoch presence records
    epoch_records: Arc<RwLock<EpochPresenceRecords>>,
}

impl PresenceProducer {
    /// Create a new presence producer
    pub fn new(
        keypair: KeyPair,
        params: ConsensusParams,
        initial_checkpoint: PresenceCheckpoint,
    ) -> Self {
        let chain = LocalPresenceChain::new(keypair, &initial_checkpoint);

        Self {
            chain: Arc::new(RwLock::new(chain)),
            params,
            checkpoint: Arc::new(RwLock::new(initial_checkpoint)),
            epoch_records: Arc::new(RwLock::new(EpochPresenceRecords::new())),
        }
    }

    /// Run the producer loop
    ///
    /// This should be spawned as a separate task.
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Starting TPoP presence producer");

        let mut last_slot = 0u32;

        loop {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs();

            let current_slot = self.params.timestamp_to_slot(now);

            // New slot started
            if current_slot > last_slot {
                self.on_new_slot(current_slot).await?;
                last_slot = current_slot;
            }

            // Check if we should try to produce a block
            let slot_start = self.params.slot_to_timestamp(current_slot);
            let slot_offset = now.saturating_sub(slot_start);

            if let Some(_block) = self.try_produce_block(current_slot, slot_offset).await? {
                info!(
                    "Produced block for slot {} at offset {}s",
                    current_slot, slot_offset
                );
                // TODO: Broadcast block via network layer
            }

            // Sleep briefly before next check
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Handle a new slot starting
    #[allow(deprecated)] // Uses HEARTBEAT_VDF_ITERATIONS - telemetry only
    async fn on_new_slot(
        &self,
        slot: Slot,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        debug!("New slot: {}", slot);

        // Start VDF computation for this slot
        {
            let mut chain = self.chain.write().await;
            chain.start_vdf_for_slot(slot);
        }

        // Spawn VDF computation (this takes ~1 second with hash-chain VDF)
        let chain_clone = self.chain.clone();
        tokio::spawn(async move {
            // Get computation parameters
            let (input_hash, _target_slot) = {
                let chain = chain_clone.read().await;
                match &chain.computing {
                    Some(c) => (c.input_hash, c.target_slot),
                    None => return,
                }
            };

            // Compute hash-chain micro-VDF (blocking, but in spawned task)
            // Uses ~10M hash iterations for ~1 second
            let result = tokio::task::spawn_blocking(move || {
                let output_bytes = hash_chain_vdf(&input_hash, HEARTBEAT_VDF_ITERATIONS);
                let output = VdfOutput {
                    value: output_bytes.to_vec(),
                };
                let proof = VdfProof::empty(); // Hash chain doesn't need Wesolowski proof
                (output, proof)
            })
            .await;

            match result {
                Ok((output, proof)) => {
                    let mut chain = chain_clone.write().await;
                    if let Some(link) = chain.complete_vdf(output, proof) {
                        info!("VDF completed for slot {}", link.slot);
                    }
                }
                Err(e) => {
                    error!("VDF task panicked: {}", e);
                }
            }
        });

        Ok(())
    }

    /// Try to produce a block for the current slot
    async fn try_produce_block(
        &self,
        slot: Slot,
        slot_offset: u64,
    ) -> Result<Option<Block>, Box<dyn std::error::Error + Send + Sync>> {
        // Get our presence proof for this slot
        let proof = {
            let chain = self.chain.read().await;
            chain.create_proof(slot)
        };

        let _proof = match proof {
            Some(p) => p,
            None => {
                // VDF not yet complete for this slot
                return Ok(None);
            }
        };

        // Get current checkpoint and rankings
        let checkpoint = self.checkpoint.read().await;
        let rankings = rank_producers_by_presence(&checkpoint);

        // Find our rank
        let our_pubkey = {
            let chain = self.chain.read().await;
            chain.public_key().clone()
        };

        let our_rank = rankings
            .iter()
            .find(|(pk, _, _, _)| pk == &our_pubkey)
            .map(|(_, _, _, rank)| *rank);

        let our_rank = match our_rank {
            Some(r) => r,
            None => {
                // We're not in the rankings (new producer?)
                // Use a high rank that might be eligible late in the slot
                rankings.len()
            }
        };

        // Check if we're eligible at this time
        if !can_produce_at_time(our_rank, slot_offset) {
            debug!(
                "Not eligible yet: rank={}, offset={}s, need={}s",
                our_rank,
                slot_offset,
                producer_eligibility_offset(our_rank)
            );
            return Ok(None);
        }

        // We're eligible! Create the block
        info!(
            "Eligible to produce: rank={}, offset={}s",
            our_rank, slot_offset
        );

        // TODO: Build actual block with transactions
        // This will be integrated with the existing block building logic
        // For now, return None as we need more integration work

        Ok(None)
    }

    /// Update to a new checkpoint
    pub async fn apply_checkpoint(&self, checkpoint: PresenceCheckpoint) {
        // Update local chain
        {
            let mut chain = self.chain.write().await;
            chain.apply_checkpoint(&checkpoint);
        }

        // Update stored checkpoint
        {
            let mut stored = self.checkpoint.write().await;
            *stored = checkpoint;
        }

        // Reset epoch records
        {
            let mut records = self.epoch_records.write().await;
            *records = EpochPresenceRecords::new();
        }
    }

    /// Get current presence score
    pub async fn presence_score(&self) -> u64 {
        let chain = self.chain.read().await;
        chain.presence_score()
    }

    /// Get our public key
    pub async fn public_key(&self) -> PublicKey {
        let chain = self.chain.read().await;
        chain.public_key().clone()
    }

    /// Get current checkpoint
    pub async fn current_checkpoint(&self) -> PresenceCheckpoint {
        self.checkpoint.read().await.clone()
    }
}

// =============================================================================
// NETWORK INTEGRATION
// =============================================================================

/// Message types for presence proof broadcasting
#[derive(Debug, Clone)]
pub enum PresenceMessage {
    /// Broadcast a presence proof for a slot
    PresenceProof(PresenceProof),

    /// Request presence proof from a producer
    RequestProof { producer: PublicKey, slot: Slot },

    /// New checkpoint announced
    NewCheckpoint(PresenceCheckpoint),
}

/// Handler for incoming presence messages
pub struct PresenceMessageHandler {
    /// Received proofs for the current slot (waiting for block production)
    pending_proofs: Arc<RwLock<std::collections::HashMap<PublicKey, PresenceProof>>>,

    /// Current checkpoint
    checkpoint: Arc<RwLock<PresenceCheckpoint>>,
}

impl PresenceMessageHandler {
    pub fn new(initial_checkpoint: PresenceCheckpoint) -> Self {
        Self {
            pending_proofs: Arc::new(RwLock::new(std::collections::HashMap::new())),
            checkpoint: Arc::new(RwLock::new(initial_checkpoint)),
        }
    }

    /// Handle an incoming presence message
    pub async fn handle(&self, msg: PresenceMessage) -> Result<(), String> {
        match msg {
            PresenceMessage::PresenceProof(proof) => self.handle_proof(proof).await,
            PresenceMessage::RequestProof {
                producer: _,
                slot: _,
            } => {
                // TODO: Respond with proof if we have it
                Ok(())
            }
            PresenceMessage::NewCheckpoint(checkpoint) => self.handle_checkpoint(checkpoint).await,
        }
    }

    async fn handle_proof(&self, proof: PresenceProof) -> Result<(), String> {
        // Verify the proof
        let checkpoint = self.checkpoint.read().await;
        if !proof.verify(&checkpoint) {
            return Err("Invalid presence proof".to_string());
        }
        drop(checkpoint);

        // Store for later use in block production
        let mut proofs = self.pending_proofs.write().await;
        proofs.insert(proof.producer.clone(), proof);

        Ok(())
    }

    async fn handle_checkpoint(&self, checkpoint: PresenceCheckpoint) -> Result<(), String> {
        // Verify checkpoint chains from previous
        let current = self.checkpoint.read().await;
        if checkpoint.prev_checkpoint_hash != current.hash() {
            return Err("Checkpoint doesn't chain from current".to_string());
        }
        drop(current);

        // Update checkpoint
        let mut stored = self.checkpoint.write().await;
        *stored = checkpoint;

        // Clear pending proofs (new checkpoint = new epoch)
        let mut proofs = self.pending_proofs.write().await;
        proofs.clear();

        Ok(())
    }

    /// Get proofs for block production
    pub async fn get_pending_proofs(&self) -> std::collections::HashMap<PublicKey, PresenceProof> {
        self.pending_proofs.read().await.clone()
    }

    /// Get current checkpoint
    pub async fn current_checkpoint(&self) -> PresenceCheckpoint {
        self.checkpoint.read().await.clone()
    }
}

// =============================================================================
// BOOTSTRAP MODE
// =============================================================================

/// Bootstrap presence producer (no bond required)
///
/// During bootstrap, producers build their presence chains without
/// the bond requirement. After bootstrap, they must register with bond
/// to continue producing.
pub struct BootstrapPresenceProducer {
    inner: PresenceProducer,
    bootstrap_start_height: BlockHeight,
    bootstrap_end_height: BlockHeight,
}

impl BootstrapPresenceProducer {
    /// Create a bootstrap producer
    pub fn new(
        keypair: KeyPair,
        params: ConsensusParams,
        bootstrap_end_height: BlockHeight,
    ) -> Self {
        // Create genesis checkpoint
        let genesis_checkpoint = PresenceCheckpoint::new(0, 0, Hash::ZERO, Vec::new());

        let inner = PresenceProducer::new(keypair, params, genesis_checkpoint);

        Self {
            inner,
            bootstrap_start_height: 0,
            bootstrap_end_height,
        }
    }

    /// Check if we're still in bootstrap mode
    pub fn is_bootstrap(&self, current_height: BlockHeight) -> bool {
        current_height < self.bootstrap_end_height
    }

    /// Get bootstrap start height
    pub fn bootstrap_start_height(&self) -> BlockHeight {
        self.bootstrap_start_height
    }

    /// Get bootstrap end height
    pub fn bootstrap_end_height(&self) -> BlockHeight {
        self.bootstrap_end_height
    }

    /// Run the bootstrap producer
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Same as regular producer, but eligibility is more permissive
        self.inner.run().await
    }

    /// Calculate bootstrap score at end of bootstrap period
    ///
    /// This becomes the initial presence_score for post-bootstrap production
    pub async fn finalize_bootstrap_score(&self) -> u64 {
        let chain = self.inner.chain.read().await;
        let slots_present = chain.chain.len() as u64;

        // Bootstrap score = slots present + chain bonus
        let chain_bonus = (slots_present / 10).min(100);
        slots_present.saturating_add(chain_bonus)
    }

    /// Get the inner producer for direct access
    pub fn inner(&self) -> &PresenceProducer {
        &self.inner
    }
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_keypair() -> KeyPair {
        KeyPair::generate()
    }

    fn test_checkpoint() -> PresenceCheckpoint {
        PresenceCheckpoint::new(0, 0, Hash::ZERO, Vec::new())
    }

    #[test]
    fn test_local_chain_creation() {
        let keypair = test_keypair();
        let checkpoint = test_checkpoint();

        let chain = LocalPresenceChain::new(keypair, &checkpoint);

        assert!(chain.chain.is_empty());
        assert_eq!(chain.presence_score(), 0);
    }

    #[test]
    fn test_vdf_link_chaining() {
        let keypair = test_keypair();
        let checkpoint = test_checkpoint();

        let mut chain = LocalPresenceChain::new(keypair, &checkpoint);

        // Start first VDF
        chain.start_vdf_for_slot(1);
        assert!(chain.computing.is_some());

        // Simulate completion (in real code, VDF would actually compute)
        let fake_output = VdfOutput {
            value: vec![1, 2, 3],
        };
        let fake_proof = VdfProof::empty();

        let link = chain.complete_vdf(fake_output.clone(), fake_proof.clone());
        assert!(link.is_some());
        assert_eq!(chain.chain.len(), 1);
        assert_eq!(chain.last_sequence(), 1);

        // Start second VDF
        chain.start_vdf_for_slot(2);
        let link2 = chain.complete_vdf(
            VdfOutput {
                value: vec![4, 5, 6],
            },
            VdfProof::empty(),
        );
        assert!(link2.is_some());
        assert_eq!(chain.chain.len(), 2);
        assert_eq!(chain.last_sequence(), 2);

        // Verify chaining
        let l1 = &chain.chain[0];
        let l2 = &chain.chain[1];
        assert_eq!(l2.sequence, l1.sequence + 1);
    }

    #[test]
    fn test_proof_creation() {
        let keypair = test_keypair();
        let checkpoint = test_checkpoint();

        let mut chain = LocalPresenceChain::new(keypair, &checkpoint);

        // No proof without VDF
        assert!(chain.create_proof(1).is_none());

        // Complete VDF for slot 1
        chain.start_vdf_for_slot(1);
        chain.complete_vdf(
            VdfOutput {
                value: vec![1, 2, 3],
            },
            VdfProof::empty(),
        );

        // Now can create proof for slot 1
        let proof = chain.create_proof(1);
        assert!(proof.is_some());

        let proof = proof.unwrap();
        assert_eq!(proof.slot, 1);
        assert_eq!(proof.vdf_chain.len(), 1);

        // Can't create proof for slot 2 (no VDF yet)
        assert!(chain.create_proof(2).is_none());
    }

    #[test]
    fn test_chain_pruning() {
        let keypair = test_keypair();
        let checkpoint = test_checkpoint();

        let mut chain = LocalPresenceChain::new(keypair, &checkpoint);

        // Add MAX_CHAIN_LENGTH + 10 links
        for slot in 1..=(MAX_CHAIN_LENGTH as u32 + 10) {
            chain.start_vdf_for_slot(slot);
            chain.complete_vdf(
                VdfOutput {
                    value: vec![slot as u8],
                },
                VdfProof::empty(),
            );
        }

        // Chain should have been pruned
        assert!(chain.chain.len() <= MAX_CHAIN_LENGTH);
        assert!(chain.chain.len() >= MAX_CHAIN_LENGTH / 2);
    }
}
