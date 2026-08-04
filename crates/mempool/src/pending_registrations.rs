//! Pending producer-registration key derivation for mempool admission.
//!
//! INC-I-147 (INV-VALIDATION-001). `ValidationContext::pending_producer_keys`
//! (`crates/core/src/validation/types.rs:146`) has exactly ONE consumer,
//! `crates/core/src/validation/registration.rs:173`, which rejects a second
//! `Registration` for a pubkey that already has an accepted-but-not-yet-active
//! one. Because the field defaults to `Vec::new()` (`types.rs:264`), "the
//! caller never populated it" was byte-identical to "no pending registrations
//! exist", so the check silently evaluated `Vec::new().contains(&pk) == false`
//! at every mempool admission.
//!
//! A registration is "pending" in exactly TWO places, and admission parity with
//! block validation requires BOTH:
//!
//! 1. **Mined but not epoch-flushed** — a `Register` sitting in
//!    `ProducerSet::pending_updates`. `apply_block` defers producer mutations
//!    to the epoch boundary (`apply_block/tx_processing.rs`), so
//!    `ProducerSet::producers` and `pending_updates` are DISJOINT for up to a
//!    whole epoch. Block validation reads this via
//!    `ProducerSet::pending_registration_keys()`
//!    (`crates/storage/src/producer/set_core.rs:250`) at
//!    `bins/node/src/node/validation_checks.rs:187`. The node republishes the
//!    same value into the mempool after every `apply_block`
//!    (`Node::refresh_mempool_producer_snapshot`).
//!
//! 2. **Mempool-resident** — a second registration can arrive while the first
//!    is still in the mempool, before either is mined. Nothing outside the
//!    mempool can observe that, so it is derived here from the entries.
//!
//! Source 2 is why this is not a one-line snapshot read: the ProducerSet knows
//! nothing about the first registration until it mines.

use std::collections::HashMap;

use crypto::{Hash, PublicKey};
use doli_core::{Transaction, TxType};

use crate::entry::MempoolEntry;

/// The registration pubkey a transaction would create, or `None` if it is not a
/// well-formed `Registration`.
///
/// A `Registration` whose `extra_data` does not deserialize yields `None`: it
/// can never become a duplicate, because `validate_registration_data_inner`
/// rejects it on the same `bincode::deserialize` at `registration.rs:122`.
pub(crate) fn registration_key_of(tx: &Transaction) -> Option<PublicKey> {
    if tx.tx_type != TxType::Registration {
        return None;
    }
    tx.registration_data().map(|data| data.public_key)
}

/// Registration pubkeys held by the mempool itself (source 2), excluding
/// `exclude` — a transaction must never count itself as its own prior pending
/// registration.
///
/// Mirrors the traversal style of `Mempool::remove_registration_txs`: filter
/// `entries` on `TxType::Registration`, then read the pubkey through
/// `Transaction::registration_data()`
/// (`crates/core/src/transaction/core.rs:449`).
pub(crate) fn resident_registration_keys(
    entries: &HashMap<Hash, MempoolEntry>,
    exclude: Option<&Hash>,
) -> Vec<PublicKey> {
    entries
        .iter()
        .filter(|(hash, _)| exclude != Some(*hash))
        .filter_map(|(_, entry)| registration_key_of(&entry.tx))
        .collect()
}

/// Union of source 1 (`node_pending`, republished from
/// `ProducerSet::pending_registration_keys()`) and source 2
/// (`resident_registration_keys`).
///
/// Duplicates between the two sources are harmless: the sole consumer is a
/// `contains()` test.
pub(crate) fn pending_registration_keys(
    entries: &HashMap<Hash, MempoolEntry>,
    node_pending: &[PublicKey],
    exclude: Option<&Hash>,
) -> Vec<PublicKey> {
    let mut keys = node_pending.to_vec();
    keys.extend(resident_registration_keys(entries, exclude));
    keys
}
