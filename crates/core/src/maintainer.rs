//! Maintainer Bootstrap System
//!
//! The DOLI auto-update system uses a decentralized maintainer set derived from
//! the blockchain itself. Unlike other blockchains that hardcode maintainer keys
//! in configuration files, DOLI derives its maintainer set from the first 5
//! registered producers.
//!
//! # Bootstrap Process
//!
//! 1. The first 5 producers to register become automatic maintainers
//! 2. After bootstrap, the maintainer set can be modified via on-chain transactions
//! 3. All changes require 3/5 multisig from current maintainers
//!
//! # Why This Design?
//!
//! | Aspect          | Hardcoded Keys      | DOLI Bootstrap       |
//! |-----------------|---------------------|----------------------|
//! | Source of truth | External config     | Blockchain itself    |
//! | Verification    | Trust the config    | Anyone can verify    |
//! | Changes         | Requires hard fork  | On-chain transactions|
//! | Auditability    | Check config        | Deterministic        |
//!
//! # Security Model
//!
//! - 3 of 5 signatures required for any action
//! - Minimum 3 maintainers must remain
//! - Maximum 5 maintainers allowed
//! - Slashed producers are automatically removed from maintainer set

use crypto::{PublicKey, Signature};
use serde::{Deserialize, Serialize};

// Re-export for backwards compatibility
pub use crate::protocol_activation::ProtocolActivationData;

// ============================================================================
// Constants
// ============================================================================

/// Number of initial maintainers derived from first N registrations
pub const INITIAL_MAINTAINER_COUNT: usize = 5;

/// Required signatures for any maintainer action (3 of 5)
pub const MAINTAINER_THRESHOLD: usize = 3;

/// Minimum maintainers allowed (cannot remove below this)
pub const MIN_MAINTAINERS: usize = 3;

/// Maximum maintainers allowed (cannot add above this)
pub const MAX_MAINTAINERS: usize = 5;

// ============================================================================
// Types
// ============================================================================

/// The set of maintainers who can sign software releases
///
/// This is derived deterministically from the blockchain by:
/// 1. Taking the first 5 registered producers as initial maintainers
/// 2. Processing any AddMaintainer/RemoveMaintainer transactions
///
/// Any node can independently verify the maintainer set by replaying
/// the blockchain from genesis.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaintainerSet {
    /// Current maintainer public keys (max 5)
    pub members: Vec<PublicKey>,
    /// Required signatures (dynamically calculated based on member count)
    pub threshold: usize,
    /// Block height of last change (for caching/verification)
    pub last_updated: u64,
}

impl Default for MaintainerSet {
    fn default() -> Self {
        Self::new()
    }
}

impl MaintainerSet {
    /// Create an empty maintainer set
    pub fn new() -> Self {
        Self {
            members: Vec::new(),
            threshold: 0,
            last_updated: 0,
        }
    }

    /// Create a maintainer set with initial members
    pub fn with_members(members: Vec<PublicKey>, last_updated: u64) -> Self {
        let threshold = Self::calculate_threshold(members.len());
        Self {
            members,
            threshold,
            last_updated,
        }
    }

    /// Check if a public key is a maintainer
    pub fn is_maintainer(&self, pubkey: &PublicKey) -> bool {
        self.members.contains(pubkey)
    }

    /// Check if we can remove a maintainer (must stay above MIN_MAINTAINERS)
    pub fn can_remove(&self) -> bool {
        self.members.len() > MIN_MAINTAINERS
    }

    /// Check if we can add a maintainer (must stay at or below MAX_MAINTAINERS)
    pub fn can_add(&self) -> bool {
        self.members.len() < MAX_MAINTAINERS
    }

    /// Get the current member count
    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    /// Calculate the required threshold based on member count
    ///
    /// Threshold is always a majority:
    /// - 1 member: 1 required
    /// - 2 members: 2 required
    /// - 3 members: 2 required
    /// - 4 members: 3 required
    /// - 5 members: 3 required
    pub fn calculate_threshold(member_count: usize) -> usize {
        match member_count {
            0 => 0,
            1 => 1,
            2 => 2,
            3 => 2,
            4 => 3,
            5 => 3,
            n => (n / 2) + 1, // Simple majority for any size
        }
    }

    /// Verify that a message has sufficient valid signatures from maintainers
    ///
    /// # Arguments
    /// * `signatures` - List of (pubkey, signature) pairs
    /// * `message` - The message that was signed
    ///
    /// # Returns
    /// `true` if at least `threshold` valid signatures from maintainers are present
    pub fn verify_multisig(&self, signatures: &[MaintainerSignature], message: &[u8]) -> bool {
        let valid_count = signatures
            .iter()
            .filter(|sig| {
                // Must be a current maintainer
                if !self.is_maintainer(&sig.pubkey) {
                    return false;
                }
                // Signature must be valid
                sig.verify(message)
            })
            .count();

        valid_count >= self.threshold
    }

    /// Verify multisig excluding a specific maintainer (for removal votes)
    ///
    /// When removing a maintainer, they cannot sign their own removal.
    /// This function verifies signatures from OTHER maintainers only.
    pub fn verify_multisig_excluding(
        &self,
        signatures: &[MaintainerSignature],
        message: &[u8],
        excluded: &PublicKey,
    ) -> bool {
        let valid_count = signatures
            .iter()
            .filter(|sig| {
                // Cannot be the excluded maintainer
                if &sig.pubkey == excluded {
                    return false;
                }
                // Must be a current maintainer
                if !self.is_maintainer(&sig.pubkey) {
                    return false;
                }
                // Signature must be valid
                sig.verify(message)
            })
            .count();

        valid_count >= self.threshold
    }

    /// Add a new maintainer
    ///
    /// # Errors
    /// Returns error if:
    /// - Already at MAX_MAINTAINERS
    /// - Target is already a maintainer
    pub fn add_maintainer(
        &mut self,
        pubkey: PublicKey,
        height: u64,
    ) -> Result<(), MaintainerError> {
        if !self.can_add() {
            return Err(MaintainerError::MaxMaintainersReached);
        }
        if self.is_maintainer(&pubkey) {
            return Err(MaintainerError::AlreadyMaintainer);
        }

        self.members.push(pubkey);
        self.threshold = Self::calculate_threshold(self.members.len());
        self.last_updated = height;
        Ok(())
    }

    /// Remove a maintainer
    ///
    /// # Errors
    /// Returns error if:
    /// - Would go below MIN_MAINTAINERS
    /// - Target is not a maintainer
    pub fn remove_maintainer(
        &mut self,
        pubkey: &PublicKey,
        height: u64,
    ) -> Result<(), MaintainerError> {
        if !self.can_remove() {
            return Err(MaintainerError::MinMaintainersRequired);
        }
        if !self.is_maintainer(pubkey) {
            return Err(MaintainerError::NotMaintainer);
        }

        self.members.retain(|m| m != pubkey);
        self.threshold = Self::calculate_threshold(self.members.len());
        self.last_updated = height;
        Ok(())
    }

    /// Force remove a maintainer (for slashing - bypasses minimum check)
    ///
    /// This is used when a maintainer is slashed for double-production.
    /// Network security takes precedence over maintainer set stability.
    pub fn force_remove_maintainer(&mut self, pubkey: &PublicKey, height: u64) -> bool {
        if !self.is_maintainer(pubkey) {
            return false;
        }

        self.members.retain(|m| m != pubkey);
        self.threshold = Self::calculate_threshold(self.members.len());
        self.last_updated = height;
        true
    }

    /// Check if the maintainer set is fully bootstrapped (has 5 members)
    pub fn is_fully_bootstrapped(&self) -> bool {
        self.members.len() >= INITIAL_MAINTAINER_COUNT
    }

    /// Check if the maintainer set needs more members during bootstrap
    pub fn needs_bootstrap_member(&self) -> bool {
        self.members.len() < INITIAL_MAINTAINER_COUNT
    }
}

/// A maintainer's signature on a message
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaintainerSignature {
    /// Maintainer's public key
    pub pubkey: PublicKey,
    /// Signature over the message
    pub signature: Signature,
}

impl MaintainerSignature {
    /// Create a new maintainer signature
    pub fn new(pubkey: PublicKey, signature: Signature) -> Self {
        Self { pubkey, signature }
    }

    /// Verify this signature against a message
    pub fn verify(&self, message: &[u8]) -> bool {
        crypto::signature::verify(message, &self.signature, &self.pubkey).is_ok()
    }
}

/// Data for maintainer change transactions (Add/Remove)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaintainerChangeData {
    /// Public key of the maintainer being added or removed
    pub target: PublicKey,
    /// Signatures from current maintainers authorizing this change
    pub signatures: Vec<MaintainerSignature>,
    /// Optional reason for the change (for transparency)
    pub reason: Option<String>,
}

impl MaintainerChangeData {
    /// Create new maintainer change data
    pub fn new(target: PublicKey, signatures: Vec<MaintainerSignature>) -> Self {
        Self {
            target,
            signatures,
            reason: None,
        }
    }

    /// Create new maintainer change data with reason
    pub fn with_reason(
        target: PublicKey,
        signatures: Vec<MaintainerSignature>,
        reason: String,
    ) -> Self {
        Self {
            target,
            signatures,
            reason: Some(reason),
        }
    }

    /// Get the message bytes that should be signed for this change
    ///
    /// For AddMaintainer: "add:{target_pubkey_hex}"
    /// For RemoveMaintainer: "remove:{target_pubkey_hex}"
    pub fn signing_message(&self, is_add: bool) -> Vec<u8> {
        let action = if is_add { "add" } else { "remove" };
        format!("{}:{}", action, self.target.to_hex()).into_bytes()
    }

    /// Serialize to bytes for storage in transaction extra_data
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap_or_default()
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        bincode::deserialize(bytes).ok()
    }
}

/// Errors that can occur during maintainer operations
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaintainerError {
    /// Cannot add: already at maximum maintainers
    MaxMaintainersReached,
    /// Cannot remove: would go below minimum maintainers
    MinMaintainersRequired,
    /// Target is already a maintainer
    AlreadyMaintainer,
    /// Target is not a maintainer
    NotMaintainer,
    /// Insufficient valid signatures
    InsufficientSignatures { found: usize, required: usize },
    /// Target must be a registered producer
    NotRegisteredProducer,
    /// Maintainer was slashed
    MaintainerSlashed,
}

impl std::fmt::Display for MaintainerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MaxMaintainersReached => {
                write!(
                    f,
                    "Cannot add maintainer: maximum of {} reached",
                    MAX_MAINTAINERS
                )
            }
            Self::MinMaintainersRequired => {
                write!(
                    f,
                    "Cannot remove maintainer: minimum of {} required",
                    MIN_MAINTAINERS
                )
            }
            Self::AlreadyMaintainer => write!(f, "Target is already a maintainer"),
            Self::NotMaintainer => write!(f, "Target is not a maintainer"),
            Self::InsufficientSignatures { found, required } => {
                write!(
                    f,
                    "Insufficient signatures: {}/{} required",
                    found, required
                )
            }
            Self::NotRegisteredProducer => {
                write!(
                    f,
                    "Target must be a registered producer to become maintainer"
                )
            }
            Self::MaintainerSlashed => write!(f, "Maintainer was slashed for misbehavior"),
        }
    }
}

impl std::error::Error for MaintainerError {}

// ============================================================================
// Blockchain Derivation
// ============================================================================

/// Trait for reading registration transactions from the blockchain
///
/// This is used by `derive_maintainer_set` to scan the chain and build
/// the maintainer set deterministically.
pub trait BlockchainReader {
    /// Get all registration public keys in order (earliest first)
    fn get_registrations_in_order(&self) -> Vec<(u64, PublicKey)>;

    /// Get all maintainer change transactions in order
    fn get_maintainer_changes(&self) -> Vec<(u64, MaintainerChange)>;

    /// Get all slashed producers
    fn get_slashed_producers(&self) -> Vec<PublicKey>;
}

/// A maintainer change event from the blockchain
#[derive(Clone, Debug)]
pub enum MaintainerChange {
    /// Add a new maintainer
    Add(MaintainerChangeData),
    /// Remove an existing maintainer
    Remove(MaintainerChangeData),
}

/// Derive the current maintainer set from the blockchain
///
/// This function deterministically computes the maintainer set by:
/// 1. Taking the first 5 registered producers as initial maintainers
/// 2. Processing all AddMaintainer/RemoveMaintainer transactions in order
/// 3. Removing any slashed producers
///
/// Any node can independently verify the maintainer set by calling this
/// function with the same blockchain state.
pub fn derive_maintainer_set<R: BlockchainReader>(reader: &R) -> MaintainerSet {
    let mut maintainer_set = MaintainerSet::new();

    // Step 1: Bootstrap from first 5 registrations
    let registrations = reader.get_registrations_in_order();
    for (height, pubkey) in registrations.into_iter().take(INITIAL_MAINTAINER_COUNT) {
        let _ = maintainer_set.add_maintainer(pubkey, height);
    }

    // Step 2: Process maintainer change transactions
    let changes = reader.get_maintainer_changes();
    for (height, change) in changes {
        match change {
            MaintainerChange::Add(data) => {
                // Verify signatures before applying
                let message = data.signing_message(true);
                if maintainer_set.verify_multisig(&data.signatures, &message) {
                    let _ = maintainer_set.add_maintainer(data.target, height);
                }
            }
            MaintainerChange::Remove(data) => {
                // Verify signatures (excluding the target) before applying
                let message = data.signing_message(false);
                if maintainer_set.verify_multisig_excluding(
                    &data.signatures,
                    &message,
                    &data.target,
                ) {
                    let _ = maintainer_set.remove_maintainer(&data.target, height);
                }
            }
        }
    }

    // Step 3: Remove slashed producers from maintainer set
    let slashed = reader.get_slashed_producers();
    for pubkey in slashed {
        maintainer_set.force_remove_maintainer(&pubkey, maintainer_set.last_updated);
    }

    maintainer_set
}

#[cfg(test)]
#[path = "maintainer_tests.rs"]
mod tests;
