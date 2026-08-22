//! INC-I-180 M2 / S2 — the decidable subset of the M1 withdrawal rule table,
//! evaluated against ONE transaction and CURRENT state.
//!
//! R4 lives in block composition, which admission cannot see. The `in_block_*`
//! terms are not evaluated either: `in_block_addbond` is zero and
//! `in_block_withdrawn` is REPLACED by `in_mempool_withdrawn`. Admission
//! therefore substitutes mempool-wide state for block-local state, and can
//! OVER-reject only when that substitute raises the block's allowance or
//! exceeds the block's debit — the `[AddBond(P,+n), RequestWithdrawal(P,d)]`
//! window is one instance, a resident same-producer withdrawal the block does
//! not carry is another. General rule, not an enumeration. Bounded, not poison:
//! it lasts until the resident confirms or expires, and costs no fee and no
//! input. Admission is NOT contained in builder-skip.

use crypto::PublicKey;
use doli_core::transaction::OutputType;
use doli_core::{BlockHeight, Transaction};
use storage::{Outpoint, UtxoSet};

use crate::holdings::HoldingsLookup;

/// `in_mempool_withdrawn` stands in for the gate's `in_block_withdrawn`: bonds
/// already claimed by same-producer withdrawals this mempool holds. Pass 0 to
/// evaluate a resident transaction on its own (`revalidate`), which is what
/// keeps a pre-existing pair from evicting both of its members.
pub(crate) fn check(
    tx: &Transaction,
    utxo: &UtxoSet,
    lookup: HoldingsLookup,
    in_mempool_withdrawn: u32,
    height: BlockHeight,
) -> Result<(), String> {
    let Some(wd) = tx.withdrawal_request_data() else {
        return Ok(());
    };
    let pk = wd.producer_pubkey;
    let pk_hash = crypto::hash::hash(pk.as_bytes());

    let holdings = match lookup {
        HoldingsLookup::Unavailable => return Ok(()),
        HoldingsLookup::Unregistered => {
            return Err(format!(
                "[ECON_WITHDRAWAL_UNKNOWN_PRODUCER] RequestWithdrawal at height={} for \
                 unregistered producer={} ({} bonds)",
                height, pk_hash, wd.bond_count
            ))
        }
        HoldingsLookup::Found(h) => h,
    };

    let allowance = holdings.allowance_with(0, in_mempool_withdrawn);
    if wd.bond_count > allowance {
        return Err(format!(
            "[ECON_WITHDRAWAL_OVER_HOLDINGS] RequestWithdrawal at height={} producer={} \
             requests {} bonds but allowance is {} (held={}, pending_addbond={}, \
             withdrawal_pending={}, in_mempool_withdrawn={})",
            height,
            pk_hash,
            wd.bond_count,
            allowance,
            holdings.bond_count,
            holdings.pending_addbond,
            holdings.withdrawal_pending,
            in_mempool_withdrawn
        ));
    }

    let owner = address_of(&pk);
    let (bond_inputs, all_bond_inputs) = bond_input_split(tx, utxo, &owner);
    let mismatch = || {
        format!(
            "[ECON_WITHDRAWAL_BOND_COUNT_MISMATCH] RequestWithdrawal at height={} \
             producer={} declares {} bonds but spends {} Bond UTXO inputs OWNED BY IT \
             of {} Bond inputs total (of {} inputs)",
            height,
            pk_hash,
            wd.bond_count,
            bond_inputs,
            all_bond_inputs,
            tx.inputs.len()
        )
    };
    if all_bond_inputs != bond_inputs {
        return Err(mismatch());
    }

    if wd.bond_count == allowance && wd.bond_count > 0 {
        let owned = u32::try_from(utxo.get_bond_entries(&owner).len()).unwrap_or(u32::MAX);
        if bond_inputs != owned {
            return Err(format!(
                "[ECON_WITHDRAWAL_INCOMPLETE_DRAIN] RequestWithdrawal at height={} \
                 producer={} declares its full allowance of {} bonds but spends {} of \
                 the {} Bond UTXOs it owns",
                height, pk_hash, wd.bond_count, bond_inputs, owned
            ));
        }
    } else if wd.bond_count != bond_inputs {
        return Err(mismatch());
    }
    Ok(())
}

/// Bonds the named producer has already committed through OTHER withdrawals in
/// this mempool.
pub(crate) fn resident_withdrawn<'a>(
    entries: impl Iterator<Item = &'a Transaction>,
    pk: &PublicKey,
) -> u32 {
    entries
        .filter_map(|tx| tx.withdrawal_request_data())
        .filter(|wd| wd.producer_pubkey == *pk)
        .fold(0u32, |acc, wd| acc.saturating_add(wd.bond_count))
}

pub(crate) fn address_of(pk: &PublicKey) -> crypto::Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes())
}

/// `(inputs resolving as Bond owned by `owner`, ALL inputs resolving as Bond)`.
fn bond_input_split(tx: &Transaction, utxo: &UtxoSet, owner: &crypto::Hash) -> (u32, u32) {
    let (mut owned, mut all_bonds) = (0u32, 0u32);
    for inp in &tx.inputs {
        let Some(entry) = utxo.get(&Outpoint::new(inp.prev_tx_hash, inp.output_index)) else {
            continue;
        };
        if entry.output.output_type != OutputType::Bond {
            continue;
        }
        all_bonds = all_bonds.saturating_add(1);
        if entry.output.pubkey_hash == *owner {
            owned = owned.saturating_add(1);
        }
    }
    (owned, all_bonds)
}
