//! INC-I-180 M2 / S2 — the producer-holdings channel for withdrawal admission.
//!
//! Two sources, tried in order: the live `ProducerSet` handle is authoritative,
//! the published snapshot answers while the live handle is contended.
//! `Unavailable` from both means the check does not run — over-rejection here
//! is censorship, while under-rejection is still caught by the builder and by
//! consensus.

use std::sync::{Arc, RwLock};

use crypto::PublicKey;
use storage::ProducerSet;

/// The three allowance terms `validate_block_economics` reads per producer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProducerHoldings {
    /// Flushed bonds (`ProducerInfo::bond_count`).
    pub bond_count: u32,
    /// Queued-but-unflushed `AddBond` bonds (`pending_addbond_count`).
    pub pending_addbond: u32,
    /// Bonds already committed to a withdrawal (`withdrawal_pending_count`).
    pub withdrawal_pending: u32,
}

impl ProducerHoldings {
    /// The gate's allowance (`validation_checks.rs`, R1), term for term AND
    /// operation for operation: every credit added before any debit.
    ///
    /// `saturating_sub` does not commute with `saturating_add` across the
    /// clamp, so re-expressing these terms in a second order raises the
    /// allowance when `withdrawal_pending > bond_count + pending_addbond` AND
    /// `in_block_addbond > 0` — the deficit alone clamps both orders to 0. The
    /// builder and this crate call it; the gate holds the same expression
    /// inline, and the two are locked by the two
    /// `inc_i180_m2_the_gate_allowance_equals_the_shared_function*` rows.
    pub fn allowance_with(&self, in_block_addbond: u32, in_block_withdrawn: u32) -> u32 {
        self.bond_count
            .saturating_add(self.pending_addbond)
            .saturating_add(in_block_addbond)
            .saturating_sub(self.withdrawal_pending)
            .saturating_sub(in_block_withdrawn)
    }

    /// The M1 allowance with every `in_block_*` term at zero.
    pub fn allowance(&self) -> u32 {
        self.allowance_with(0, 0)
    }
}

/// The node-published holdings snapshot, shared by `Arc`.
pub type HoldingsSnapshot = Arc<RwLock<Vec<(PublicKey, ProducerHoldings)>>>;

/// Resolution of one producer against whatever source is wired.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldingsLookup {
    /// No source is wired, or every wired source is contended right now.
    Unavailable,
    /// The source answered: this key is not a registered producer (R0). Carries
    /// the gate's `pending` term, which it reads for absent keys too.
    Unregistered { pending_addbond: u32 },
    /// The source answered with the producer's holdings.
    Found(ProducerHoldings),
}

/// Both channels the mempool can resolve a producer through.
#[derive(Clone, Default)]
pub(crate) struct HoldingsSources {
    /// Node-owned live set, read with `try_read`. Never blocks, so no admission
    /// call site can deadlock against an `apply_block` writer.
    pub(crate) live: Option<Arc<tokio::sync::RwLock<ProducerSet>>>,
    /// Node-published snapshot, and the channel embedders wire directly.
    pub(crate) snapshot: Option<HoldingsSnapshot>,
}

impl HoldingsSources {
    pub(crate) fn lookup(&self, pk: &PublicKey) -> HoldingsLookup {
        if let Some(set) = &self.live {
            if let Ok(guard) = set.try_read() {
                return of_producer_set(&guard, pk);
            }
        }
        if let Some(snapshot) = &self.snapshot {
            let Ok(guard) = snapshot.read() else {
                return HoldingsLookup::Unavailable;
            };
            // Absence from a POPULATED snapshot is the R0 condition. Absence
            // from an EMPTY one is no answer at all: only `Node::new` seeds it,
            // so under `new_for_test` / `new_for_replay` a contended `try_read`
            // would otherwise make every producer read `Unregistered` and
            // reject — fail-CLOSED, the opposite of the direction this layer
            // documents. A chain with a genuinely empty producer set cannot
            // carry a withdrawal the builder or the gate would accept either.
            if guard.is_empty() {
                return HoldingsLookup::Unavailable;
            }
            // A snapshot cannot answer `pending` for a key it does not carry; 0
            // keeps the filter a strict subset of the gate.
            return match guard.iter().find(|(k, _)| k == pk) {
                Some((_, h)) => HoldingsLookup::Found(*h),
                None => HoldingsLookup::Unregistered { pending_addbond: 0 },
            };
        }
        HoldingsLookup::Unavailable
    }
}

/// Read one producer's three allowance terms out of a live `ProducerSet`.
pub fn of_producer_set(set: &ProducerSet, pk: &PublicKey) -> HoldingsLookup {
    match set.get_by_pubkey(pk) {
        Some(info) => HoldingsLookup::Found(ProducerHoldings {
            bond_count: info.bond_count,
            pending_addbond: set.pending_addbond_count(pk),
            withdrawal_pending: info.withdrawal_pending_count,
        }),
        None => HoldingsLookup::Unregistered {
            pending_addbond: set.pending_addbond_count(pk),
        },
    }
}
