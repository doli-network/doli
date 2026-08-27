//! The maintainer set, its signatures, and its authorization rules.

use crypto::{PublicKey, Signature};
use serde::{Deserialize, Serialize};

use super::{MAINTAINER_THRESHOLD, MAX_MAINTAINERS, MIN_MAINTAINERS};

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
    ///
    /// `threshold` is 0 here — NOT `calculate_threshold(0)` — because this value
    /// is persisted verbatim in `maintainer_state.bin` and the M1 versioned
    /// decoder round-trips it (`crates/storage/tests/maintainer_state_versioned_test.rs`).
    /// A zero threshold is never satisfiable regardless: every verifier refuses an
    /// un-authorizable set before counting (see [`Self::is_authorizable`]).
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

    /// Can this set authorize ANYTHING at all?
    ///
    /// INC-I-172 M2 / AUDIT-P1-010 (FM-02). **UNGATED** — this guard applies at
    /// every height, including below `maintainer_derivation_activation_height`.
    /// A set that is empty, or that carries a zero threshold, or that holds fewer
    /// keys than its own threshold, can never legitimately authorize anything, so
    /// there is no pre-activation behavior worth preserving: accepting zero
    /// signatures is not a rule, it is the absence of one. Mirrors
    /// `crates/updater/src/trust_root.rs::is_usable` so the governance path and
    /// the install path refuse the same roots.
    pub fn is_authorizable(&self) -> bool {
        !self.members.is_empty() && self.threshold >= 1 && self.members.len() >= self.threshold
    }

    /// Calculate the required threshold based on member count
    ///
    /// Threshold is always a majority:
    /// - 0 members: [`MAINTAINER_THRESHOLD`] required (unsatisfiable by construction)
    /// - 1 member: 1 required
    /// - 2 members: 2 required
    /// - 3 members: 2 required
    /// - 4 members: 3 required
    /// - 5 members: 3 required
    ///
    /// INC-I-172 M2 / AUDIT-P1-010: the 0 arm used to return 0, which made
    /// `valid_count >= self.threshold` VACUOUS — an empty set accepted a
    /// zero-signature `AddMaintainer`. This arm is **UNGATED**; see
    /// [`Self::is_authorizable`] for why no pre-activation parity is owed.
    pub fn calculate_threshold(member_count: usize) -> usize {
        match member_count {
            0 => MAINTAINER_THRESHOLD,
            1 => 1,
            2 => 2,
            3 => 2,
            4 => 3,
            5 => 3,
            n => (n / 2) + 1, // Simple majority for any size
        }
    }

    /// Count DISTINCT maintainers who signed `message`.
    ///
    /// INC-I-172 M2 / AUDIT-P0-010, REQ-172-012. This is the mainnet-live
    /// covenant k-of-n shape from `crates/core/src/conditions/eval.rs`
    /// (`Condition::Multisig`, live since covenant activation h=9150): outer loop
    /// over the EXPECTED member keys, inner loop over the witness signatures,
    /// `break` on the first match. Adopted rather than invented — a `HashSet`
    /// dedup would have been a second, separately-maintained shape.
    ///
    /// Each member can contribute at most 1 to the count, so padding the vector
    /// with duplicate entries from one compromised key cannot clear a k-of-n
    /// threshold.
    fn count_distinct_signers(
        &self,
        signatures: &[MaintainerSignature],
        message: &[u8],
        excluded: Option<&PublicKey>,
    ) -> usize {
        let mut satisfied = 0usize;
        for member in &self.members {
            if excluded == Some(member) {
                continue;
            }
            for sig in signatures {
                if &sig.pubkey == member && sig.verify(message) {
                    satisfied += 1;
                    break;
                }
            }
        }
        satisfied
    }

    /// Verify that a message has sufficient valid signatures from maintainers
    ///
    /// POST-activation semantics (and the default): `threshold` DISTINCT
    /// maintainers must have signed. Call sites that run at a chain height MUST
    /// use [`Self::verify_multisig_at`] instead, so history below
    /// `maintainer_derivation_activation_height` replays bit-identically.
    ///
    /// # Arguments
    /// * `signatures` - List of (pubkey, signature) pairs
    /// * `message` - The message that was signed
    ///
    /// # Returns
    /// `true` if at least `threshold` DISTINCT maintainers signed `message`
    pub fn verify_multisig(&self, signatures: &[MaintainerSignature], message: &[u8]) -> bool {
        if !self.is_authorizable() {
            return false;
        }
        self.count_distinct_signers(signatures, message, None) >= self.threshold
    }

    /// Verify multisig excluding a specific maintainer (for removal votes)
    ///
    /// When removing a maintainer, they cannot sign their own removal.
    /// This function verifies signatures from OTHER maintainers only, counting
    /// each remaining maintainer at most once.
    pub fn verify_multisig_excluding(
        &self,
        signatures: &[MaintainerSignature],
        message: &[u8],
        excluded: &PublicKey,
    ) -> bool {
        if !self.is_authorizable() {
            return false;
        }
        self.count_distinct_signers(signatures, message, Some(excluded)) >= self.threshold
    }

    /// PRE-activation `verify_multisig`: counts signature ENTRIES, not signers.
    ///
    /// Exists ONLY so a height gate can reproduce consensus history below
    /// `maintainer_derivation_activation_height`. **MUST NOT be called ungated** —
    /// use [`Self::verify_multisig_at`].
    ///
    /// The one deliberate divergence from the historical code is the
    /// [`Self::is_authorizable`] guard, which is UNGATED: an empty or
    /// zero-threshold set accepted ZERO signatures before M2 (FM-02) and that is
    /// not behavior worth replaying.
    pub fn verify_multisig_legacy(
        &self,
        signatures: &[MaintainerSignature],
        message: &[u8],
    ) -> bool {
        if !self.is_authorizable() {
            return false;
        }
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

    /// PRE-activation `verify_multisig_excluding`: counts entries, not signers.
    ///
    /// See [`Self::verify_multisig_legacy`]. **MUST NOT be called ungated** — use
    /// [`Self::verify_multisig_excluding_at`].
    pub fn verify_multisig_excluding_legacy(
        &self,
        signatures: &[MaintainerSignature],
        message: &[u8],
        excluded: &PublicKey,
    ) -> bool {
        if !self.is_authorizable() {
            return false;
        }
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

    /// Height-gated multisig verification — the ONLY form consensus paths may use.
    ///
    /// * `height < activation_height` → [`Self::verify_multisig_legacy`]
    /// * `height >= activation_height` → [`Self::verify_multisig`]
    ///
    /// `height` MUST be a chain-derived block height, never a per-process counter
    /// (INV-SYNC-012). `activation_height` is
    /// `NetworkParams::maintainer_derivation_activation_height`; it is passed in
    /// as a `u64` so `crates/core::maintainer` stays a leaf module.
    pub fn verify_multisig_at(
        &self,
        signatures: &[MaintainerSignature],
        message: &[u8],
        height: u64,
        activation_height: u64,
    ) -> bool {
        if height >= activation_height {
            self.verify_multisig(signatures, message)
        } else {
            self.verify_multisig_legacy(signatures, message)
        }
    }

    /// Height-gated removal-vote verification. See [`Self::verify_multisig_at`].
    pub fn verify_multisig_excluding_at(
        &self,
        signatures: &[MaintainerSignature],
        message: &[u8],
        excluded: &PublicKey,
        height: u64,
        activation_height: u64,
    ) -> bool {
        if height >= activation_height {
            self.verify_multisig_excluding(signatures, message, excluded)
        } else {
            self.verify_multisig_excluding_legacy(signatures, message, excluded)
        }
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
        self.members.len() >= super::INITIAL_MAINTAINER_COUNT
    }

    /// Check if the maintainer set needs more members during bootstrap
    pub fn needs_bootstrap_member(&self) -> bool {
        self.members.len() < super::INITIAL_MAINTAINER_COUNT
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
