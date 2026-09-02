//! INC-I-203 — the shared `MAX_BONDS_PER_PRODUCER` verdict (REQ-BOND-004).
//!
//! One function, so the block builder, `add_transaction` and `revalidate`
//! cannot drift apart. It CALLS `check_addbond_cap`; a second copy of the
//! expression is the INC-I-180 `allowance_with` lesson repeated.

use doli_core::transaction::{OutputType, Transaction, TxType};
use doli_core::validation::check_addbond_cap;

use crate::holdings::HoldingsLookup;

/// The gate's `requested` term (`validation_checks.rs:1206-1211`).
fn requested_bonds(tx: &Transaction) -> u32 {
    u32::try_from(
        tx.outputs
            .iter()
            .filter(|o| o.output_type == OutputType::Bond)
            .count(),
    )
    .unwrap_or(u32::MAX)
}

/// `Err` carries the bracketed `[ADDBOND_CAP_EXCEEDED]` code the fleet greps;
/// the caller skips (builder) or refuses (admission).
///
/// REQ-BOND-005: below `activation_height` the gate is a no-op, so this is one
/// too. REQ-BOND-006: anything other than `Found` is no answer at all and the
/// check does not run — over-rejection here is censorship, while
/// under-rejection is still caught by the gate.
pub fn addbond_cap_verdict(
    tx: &Transaction,
    holdings: &HoldingsLookup,
    in_block_prior: u32,
    height: u64,
    activation_height: u64,
) -> Result<(), String> {
    if tx.tx_type != TxType::AddBond || height < activation_height {
        return Ok(());
    }
    let HoldingsLookup::Found(h) = holdings else {
        return Ok(());
    };
    let Some(ab) = tx.add_bond_data() else {
        return Ok(());
    };
    let requested = requested_bonds(tx);
    check_addbond_cap(
        h.bond_count,
        h.pending_addbond.saturating_add(in_block_prior),
        requested,
        height,
        activation_height,
    )
    .map_err(|e| {
        format!(
            "[{}] producer={:?} current={} pending={} in_block_prior={} requested={} max={}",
            e.error_code(),
            ab.producer_pubkey,
            h.bond_count,
            h.pending_addbond,
            in_block_prior,
            requested,
            doli_core::MAX_BONDS_PER_PRODUCER
        )
    })
}
