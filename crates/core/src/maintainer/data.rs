//! Transaction payloads for maintainer governance and protocol activation.

use crypto::PublicKey;
use serde::{Deserialize, Serialize};

use super::MaintainerSignature;

/// Data for maintainer change transactions (Add/Remove)
///
/// # INC-I-176 M1a — this payload moves ZERO bytes
///
/// The field list, their order and their types are FROZEN at exactly what they
/// were before INC-I-176, and the bincode encoding is byte-identical to that
/// shape for every input. That is not a style preference, it is a measured
/// constraint: `MaintainerChangeData::from_bytes` is consumed FATALLY and
/// WITHOUT a height gate at
/// `crates/core/src/validation/tx_types.rs::validate_maintainer_change_data`,
/// and a real `add_maintainer` payload of this exact shape is already in
/// testnet history (block 136_690, txid `62a3bfbd…bc81`, 385-byte
/// `extra_data`). Any field added, removed, reordered or re-typed here makes
/// that block undecodable, so every node re-validating it from genesis stops —
/// forever, in both deploy directions, and a synchronized deploy does not
/// repair it.
///
/// What INC-I-176 M1a DOES change lives entirely in the *signing message*, not
/// in this payload: [`super::signing_message_legacy`] is now the ONE owner of
/// the legacy format (REQ-176-030) and [`super::signing_message`] is the bound
/// form that M2 dispatches to by height.
///
/// The payload swap itself (delete the write-only `reason`, add a signed
/// `valid_before`, canonicalize the signature order) is DEFERRED to milestone
/// **M2.5**, where an activation height and an explicit format discriminator
/// make it safe. It is deferred, not abandoned.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaintainerChangeData {
    /// Public key of the maintainer being added or removed
    pub target: PublicKey,
    /// Signatures from current maintainers authorizing this change
    ///
    // M2.5: canonical (ascending-by-signer-pubkey) ordering of this vector
    // moves to M2.5, together with the payload change. It is deliberately NOT
    // done in M1a: `extra_data` feeds the txid, so sorting would emit
    // different BYTES for the same caller input — a behaviour change with no
    // gate — and it is not an adversarial control anyway, because
    // `sendTransaction` accepts any ordering off the wire (security audit F3).
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
    ///
    /// UNCHANGED SEMANTICS, byte for byte. INC-I-176 M1a made this a thin
    /// delegate to [`super::signing_message_legacy`], the ONE owner of that
    /// format — which is what satisfies REQ-176-030 ("exactly ONE
    /// implementation of the signed message"), not deletion. The method cannot
    /// be deleted: `derivation.rs` and the read-only updater test at
    /// `crates/updater/tests/inc_i_172_m2_release_sign_arg_validation.rs` both
    /// reach the format through it.
    ///
    /// Note what this format does NOT cover: the genesis hash and any validity
    /// window. That blindness IS the pre-INC-I-176 defect, kept intact here
    /// because it is frozen consensus history. The bound form is
    /// [`super::signing_message`], selected by height in
    /// [`super::signing_message_at`]; M2 wires that dispatch into production.
    pub fn signing_message(&self, is_add: bool) -> Vec<u8> {
        super::signing_message_legacy(is_add, &self.target)
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

/// Data for protocol activation transactions (on-chain consensus upgrade)
///
/// When maintainers want to activate new consensus rules, they create a
/// ProtocolActivation transaction with a maintainer quorum. The activation is
/// scheduled for a future epoch, giving all nodes time to process it.
/// At the target epoch boundary, ALL nodes switch simultaneously.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolActivationData {
    /// Protocol version to activate (must be > current active version)
    pub protocol_version: u32,
    /// Epoch at which activation occurs (must be in the future)
    pub activation_epoch: u64,
    /// Human-readable description of consensus changes
    pub description: String,
    /// Signatures from current maintainers authorizing activation
    pub signatures: Vec<MaintainerSignature>,
}

impl ProtocolActivationData {
    /// Create new protocol activation data
    pub fn new(
        protocol_version: u32,
        activation_epoch: u64,
        description: String,
        signatures: Vec<MaintainerSignature>,
    ) -> Self {
        Self {
            protocol_version,
            activation_epoch,
            description,
            signatures,
        }
    }

    /// Get the message bytes that should be signed for this activation
    ///
    /// Format: "activate:{version}:{epoch}"
    pub fn signing_message(&self) -> Vec<u8> {
        format!(
            "activate:{}:{}",
            self.protocol_version, self.activation_epoch
        )
        .into_bytes()
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
