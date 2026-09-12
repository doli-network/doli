//! INC-I-217 M7 — the stateful half of a BLS key rotation.
//!
//! covers: REQ-ROT-005, REQ-ROT-008, REQ-ROT-SEC-004, REQ-ROT-SEC-005,
//! REQ-ROT-SEC-006
//!
//! Shape and cryptography are block-invalidating at `validation/block.rs`
//! (M5), so nothing here reads `bls_pop` or `signature`.

use crypto::hash::hash as crypto_hash;
use crypto::PublicKey;
use doli_core::transaction::RotateBlsData;

use super::types::{PendingProducerUpdate, ProducerSet, ProducerStatus};

/// The four stateful facts a rotation verdict needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RotationInputs {
    /// `Active` in the map, or carried by a queued `Register`.
    pub sender_registered: bool,
    /// The key this rotation replaces, from the map or the queued `Register`.
    pub current_bls: Option<Vec<u8>>,
    /// The target key is held by another producer or reserved by the queue.
    pub key_in_use: bool,
    /// A `RotateBlsKey` for this sender is already queued.
    pub already_pending: bool,
}

/// Why a rotation was skipped. A skip is never block-invalidating (D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotateSkip {
    /// Sender is not an `Active` producer and has no queued `Register`.
    NotProducer,
    /// The target key is the sender's own current key.
    SameKey,
    /// The sender already has a rotation queued for this boundary.
    AlreadyPending,
    /// The target key belongs to, or is reserved for, someone else.
    KeyInUse,
}

/// Pure: no I/O and no receiver. Evaluation order is a wire contract —
/// `NotProducer` -> `SameKey` -> `AlreadyPending` -> `KeyInUse`. M8's rebuild
/// path calls this same function, so the two apply paths cannot drift.
pub fn rotate_verdict(data: &RotateBlsData, inputs: &RotationInputs) -> Result<(), RotateSkip> {
    if !inputs.sender_registered {
        return Err(RotateSkip::NotProducer);
    }
    if inputs.current_bls.as_deref() == Some(&data.new_bls_pubkey[..]) {
        return Err(RotateSkip::SameKey);
    }
    if inputs.already_pending {
        return Err(RotateSkip::AlreadyPending);
    }
    if inputs.key_in_use {
        return Err(RotateSkip::KeyInUse);
    }
    Ok(())
}

/// The ONE apply. On `Ok` it queues exactly one `RotateBlsKey`; every `Err`
/// leaves the set byte-identical.
pub fn apply_rotation(
    producers: &mut ProducerSet,
    data: &RotateBlsData,
    height: u64,
) -> Result<(), RotateSkip> {
    let inputs = producers.resolve_rotation_inputs(data);
    if let Err(reason) = rotate_verdict(data, &inputs) {
        tracing::warn!(
            "[BLS_ROTATE] skipped producer={:.8} reason={:?} h={}",
            crypto_hash(&data.producer),
            reason,
            height
        );
        return Err(reason);
    }
    tracing::info!(
        "[BLS_ROTATE] queued producer={:.8} new={:.8} h={}",
        crypto_hash(&data.producer),
        crypto_hash(&data.new_bls_pubkey),
        height
    );
    producers.queue_update(PendingProducerUpdate::RotateBlsKey {
        pubkey: PublicKey::from_bytes(data.producer),
        new_bls_pubkey: data.new_bls_pubkey.to_vec(),
        height,
    });
    Ok(())
}

impl ProducerSet {
    /// One O(n) pass over the map and the queue (D8). No reverse index: a
    /// second source of truth would have to be rebuilt identically on
    /// snap-sync, restart and rebuild.
    pub fn resolve_rotation_inputs(&self, data: &RotateBlsData) -> RotationInputs {
        let sender = PublicKey::from_bytes(data.producer);
        let target = &data.new_bls_pubkey[..];

        let (sender_registered, current_bls) = match self.get_by_pubkey(&sender) {
            Some(p) => (
                matches!(p.status, ProducerStatus::Active),
                Some(p.bls_pubkey.clone()),
            ),
            None => match self.queued_register_key(&sender) {
                Some(bls) => (true, Some(bls)),
                None => (false, None),
            },
        };

        // The sender's own current key is `SameKey`, never `KeyInUse`.
        let held_by_other = self
            .producers
            .values()
            .any(|p| p.public_key != sender && p.bls_pubkey.as_slice() == target);

        let mut reserved_by_queue = false;
        let mut already_pending = false;
        for update in &self.pending_updates {
            match update {
                PendingProducerUpdate::RotateBlsKey {
                    pubkey,
                    new_bls_pubkey,
                    ..
                } => {
                    reserved_by_queue |= new_bls_pubkey.as_slice() == target;
                    already_pending |= *pubkey == sender;
                }
                PendingProducerUpdate::Register { info, .. } => {
                    reserved_by_queue |= info.bls_pubkey.as_slice() == target;
                }
                _ => {}
            }
        }

        RotationInputs {
            sender_registered,
            current_bls,
            key_in_use: held_by_other || reserved_by_queue,
            already_pending,
        }
    }

    /// `bls_pubkey` of a queued `Register` for `pubkey`, if the queue holds one.
    fn queued_register_key(&self, pubkey: &PublicKey) -> Option<Vec<u8>> {
        self.pending_updates.iter().find_map(|u| match u {
            PendingProducerUpdate::Register { info, .. } if info.public_key == *pubkey => {
                Some(info.bls_pubkey.clone())
            }
            _ => None,
        })
    }
}
