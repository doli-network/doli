use crate::consensus::ConsensusParams;
use crate::network::Network;
use crate::transaction::Output;
use crate::types::BlockHeight;
use crypto::{Hash, PublicKey};

/// Information about an unspent transaction output.
///
/// This is returned by UTXO lookups to provide all information needed
/// for transaction validation.
#[derive(Clone, Debug)]
pub struct UtxoInfo {
    /// The output data.
    pub output: Output,
    /// The public key that can spend this output.
    /// For pay-to-pubkey-hash, this must be provided separately.
    pub pubkey: Option<PublicKey>,
    /// Whether this output has been spent.
    pub spent: bool,
}

/// Trait for UTXO set access during validation.
///
/// Implement this trait to provide the validator with access to the UTXO set
/// for contextual validation (signature verification, balance checks, etc.).
///
/// # Example
///
/// ```rust,ignore
/// struct MyUtxoSet { /* ... */ }
///
/// impl UtxoProvider for MyUtxoSet {
///     fn get_utxo(&self, tx_hash: &Hash, output_index: u32) -> Option<UtxoInfo> {
///         // Look up in database
///     }
/// }
/// ```
pub trait UtxoProvider {
    /// Look up an unspent output.
    ///
    /// Returns `None` if the output doesn't exist or has been spent.
    fn get_utxo(&self, tx_hash: &Hash, output_index: u32) -> Option<UtxoInfo>;
}

/// Registration chain state for anti-Sybil verification.
///
/// Tracks the chained VDF registration state needed to validate
/// that new registrations form a proper chain.
#[derive(Clone, Debug, Default)]
pub struct RegistrationChainState {
    /// Hash of the last registration transaction (Hash::ZERO before any registration)
    pub last_registration_hash: Hash,
    /// Current global registration sequence number
    pub registration_sequence: u64,
}

impl RegistrationChainState {
    /// Create a new registration chain state
    pub fn new(last_hash: Hash, sequence: u64) -> Self {
        Self {
            last_registration_hash: last_hash,
            registration_sequence: sequence,
        }
    }

    /// Get the expected prev_registration_hash for the next registration
    pub fn expected_prev_hash(&self) -> Hash {
        self.last_registration_hash
    }

    /// Get the expected sequence number for the next registration
    pub fn expected_sequence(&self) -> u64 {
        if self.last_registration_hash == Hash::ZERO {
            0 // First registration
        } else {
            self.registration_sequence + 1
        }
    }
}

/// Validation mode for synced blocks.
///
/// Controls whether VDF proof verification is performed during block validation.
/// After snap sync, gap blocks use `Light` mode (VDF already trusted via state root
/// quorum), while recent blocks near the tip use `Full` mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationMode {
    /// Full validation including VDF proof verification.
    /// Used for: gossip blocks, last epoch of sync (360 blocks).
    Full,
    /// Light validation: everything except VDF proof verification.
    /// Used for: gap blocks after snap sync where state root was
    /// already verified by peer quorum (2+ peers).
    Light,
}

/// Block validation context.
///
/// Holds all the context needed to validate a block or transaction,
/// including consensus parameters, current time, and chain state.
#[derive(Clone, Debug)]
pub struct ValidationContext {
    /// Consensus parameters (slot duration, block reward, etc.).
    pub params: ConsensusParams,
    /// Network type (determines VDF iterations and other parameters).
    pub network: Network,
    /// Current wall-clock time (unix timestamp).
    pub current_time: u64,
    /// Height of the block being validated.
    pub current_height: BlockHeight,
    /// Slot of the previous block.
    pub prev_slot: u32,
    /// Timestamp of the previous block.
    pub prev_timestamp: u64,
    /// Hash of the previous block (for producer selection).
    pub prev_hash: crypto::Hash,
    /// Active producers for the current epoch (legacy, for backward compatibility).
    pub active_producers: Vec<crypto::PublicKey>,
    /// Active producers with their effective weights for weighted selection (Option C).
    /// If set, weighted selection is used for anti-grinding protection.
    pub active_producers_weighted: Vec<(crypto::PublicKey, u64)>,
    /// Bootstrap producers (sorted by pubkey) for fallback rank validation during bootstrap.
    /// When non-empty, bootstrap validation uses deterministic fallback ranks instead of
    /// accepting any producer. Populated from GSet/known_producers in the node layer.
    pub bootstrap_producers: Vec<crypto::PublicKey>,
    /// Live bootstrap producers (sorted by pubkey) for liveness-filtered scheduling.
    /// Empty = no liveness filter active (use bootstrap_producers for all ranks).
    pub live_bootstrap_producers: Vec<crypto::PublicKey>,
    /// Stale bootstrap producers (sorted by pubkey) for re-entry slot scheduling.
    pub stale_bootstrap_producers: Vec<crypto::PublicKey>,
    /// Registration chain state for chained VDF anti-Sybil verification.
    pub registration_chain: RegistrationChainState,
    /// Public keys with pending (epoch-deferred) registrations.
    /// Used to reject duplicate registrations before epoch activation.
    pub pending_producer_keys: Vec<crypto::PublicKey>,
    /// BLS public keys for active producers, sorted by Ed25519 pubkey (same as bitfield order).
    ///
    /// Index N corresponds to the producer at sorted index N in the bitfield.
    /// Empty Vec at index N means that producer has no BLS key (pre-BLS registration).
    #[allow(dead_code)]
    pub producer_bls_keys: Vec<Vec<u8>>,
}

impl ValidationContext {
    /// Create a new validation context.
    #[must_use]
    pub fn new(
        params: ConsensusParams,
        network: Network,
        current_time: u64,
        current_height: BlockHeight,
    ) -> Self {
        Self {
            params,
            network,
            current_time,
            current_height,
            prev_slot: 0,
            prev_timestamp: 0,
            prev_hash: crypto::Hash::ZERO,
            active_producers: Vec::new(),
            active_producers_weighted: Vec::new(),
            bootstrap_producers: Vec::new(),
            live_bootstrap_producers: Vec::new(),
            stale_bootstrap_producers: Vec::new(),
            registration_chain: RegistrationChainState::default(),
            pending_producer_keys: Vec::new(),
            producer_bls_keys: Vec::new(),
        }
    }

    /// Set previous block info.
    #[must_use]
    pub fn with_prev_block(mut self, slot: u32, timestamp: u64, hash: crypto::Hash) -> Self {
        self.prev_slot = slot;
        self.prev_timestamp = timestamp;
        self.prev_hash = hash;
        self
    }

    /// Set active producers for producer eligibility validation (legacy).
    #[must_use]
    pub fn with_producers(mut self, producers: Vec<crypto::PublicKey>) -> Self {
        self.active_producers = producers;
        self
    }

    /// Set active producers with weights for weighted selection (Option C - anti-grinding).
    ///
    /// When weights are provided, the validation uses weighted producer selection:
    /// - Phase 1: Top N producers by weight are eligible (deterministic, not grindable)
    /// - Phase 2: Hash-based ordering among eligible (limited grinding)
    #[must_use]
    pub fn with_producers_weighted(mut self, producers: Vec<(crypto::PublicKey, u64)>) -> Self {
        // Also populate legacy field for backward compatibility
        self.active_producers = producers.iter().map(|(pk, _)| *pk).collect();
        self.active_producers_weighted = producers;
        self
    }

    /// Set bootstrap producers for fallback rank validation during bootstrap.
    /// Producers must be sorted by pubkey (same order used by production side).
    #[must_use]
    pub fn with_bootstrap_producers(mut self, producers: Vec<crypto::PublicKey>) -> Self {
        self.bootstrap_producers = producers;
        self
    }

    /// Set the liveness split for bootstrap producers.
    /// Both lists must be sorted by pubkey. When `live` is non-empty,
    /// liveness-aware scheduling is used (live for normal slots, stale for re-entry).
    #[must_use]
    pub fn with_bootstrap_liveness(
        mut self,
        live: Vec<crypto::PublicKey>,
        stale: Vec<crypto::PublicKey>,
    ) -> Self {
        self.live_bootstrap_producers = live;
        self.stale_bootstrap_producers = stale;
        self
    }

    /// Set registration chain state for chained VDF validation.
    #[must_use]
    pub fn with_registration_chain(mut self, last_hash: Hash, sequence: u64) -> Self {
        self.registration_chain = RegistrationChainState::new(last_hash, sequence);
        self
    }

    /// Set pending producer keys (epoch-deferred registrations not yet active).
    #[must_use]
    pub fn with_pending_producer_keys(mut self, keys: Vec<crypto::PublicKey>) -> Self {
        self.pending_producer_keys = keys;
        self
    }
}
