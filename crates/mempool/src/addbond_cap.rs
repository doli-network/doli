//! INC-I-203 — the shared `MAX_BONDS_PER_PRODUCER` verdict (REQ-BOND-004).
//!
//! One function, so the block builder, `add_transaction` and `revalidate`
//! cannot drift apart. It CALLS `check_addbond_cap`; a second copy of the
//! expression is the INC-I-180 `allowance_with` lesson repeated.

use doli_core::transaction::{Transaction, TxType};
use doli_core::validation::{check_addbond_cap, count_bond_outputs};

use crate::holdings::HoldingsLookup;

/// `Err` carries the bracketed `[ADDBOND_CAP_EXCEEDED]` code the fleet greps;
/// the caller skips (builder) or refuses (admission).
///
/// REQ-BOND-005: below `activation_height` the gate is a no-op, so this is one
/// too. REQ-BOND-006: only `Unavailable` — no source answered at all — still
/// fails open, because over-rejection there is censorship. `Unregistered` IS an
/// answer and is evaluated with `current = 0`, exactly as
/// `validate_block_economics` reads an absent key's `bond_count`.
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
    let (current, pending) = match holdings {
        HoldingsLookup::Unavailable => return Ok(()),
        HoldingsLookup::Unregistered { pending_addbond } => (0, *pending_addbond),
        HoldingsLookup::Found(h) => (h.bond_count, h.pending_addbond),
    };
    let Some(ab) = tx.add_bond_data() else {
        return Ok(());
    };
    let requested = count_bond_outputs(tx);
    check_addbond_cap(
        current,
        pending.saturating_add(in_block_prior),
        requested,
        height,
        activation_height,
    )
    .map_err(|e| {
        format!(
            "[{}] producer={:?} current={} pending={} in_block_prior={} requested={} max={}",
            e.error_code(),
            ab.producer_pubkey,
            current,
            pending,
            in_block_prior,
            requested,
            doli_core::MAX_BONDS_PER_PRODUCER
        )
    })
}
