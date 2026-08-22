//! INC-I-180 M2 / S2 — the node's half of the mempool holdings channel.
//!
//! One `pending_updates` pass, then one producer pass: calling
//! `pending_addbond_count` per producer would be O(producers x pending).

use crypto::PublicKey;
use mempool::ProducerHoldings;
use storage::{PendingProducerUpdate, ProducerSet};

pub fn holdings_of_every_producer(set: &ProducerSet) -> Vec<(PublicKey, ProducerHoldings)> {
    let mut queued_bonds: std::collections::HashMap<PublicKey, u32> =
        std::collections::HashMap::new();
    for (pk, updates) in set.pending_updates_by_pubkey() {
        let bonds: u32 = updates
            .iter()
            .filter_map(|u| match u {
                PendingProducerUpdate::AddBond { outpoints, .. } => {
                    Some(u32::try_from(outpoints.len()).unwrap_or(u32::MAX))
                }
                _ => None,
            })
            .fold(0u32, |acc, n| acc.saturating_add(n));
        if bonds > 0 {
            queued_bonds.insert(pk, bonds);
        }
    }

    set.all_producers()
        .iter()
        .map(|info| {
            (
                info.public_key,
                ProducerHoldings {
                    bond_count: info.bond_count,
                    pending_addbond: queued_bonds.get(&info.public_key).copied().unwrap_or(0),
                    withdrawal_pending: info.withdrawal_pending_count,
                },
            )
        })
        .collect()
}
