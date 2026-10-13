//! Zero-flow admission policy (INC-I-237 M11): canonical maintainer payloads,
//! state-effect preconditions, the INC-I-246 delegation stop-gap and the
//! canonical mined check. Admission-only: block validation and apply are untouched.

use crypto::PublicKey;
use doli_core::maintainer::MaintainerChangeData;
use doli_core::network::Network;
use doli_core::{MaintainerSet, Transaction, TxType};
use storage::{BlockStore, PendingProducerUpdate, ProducerSet};

use crate::pool::MempoolError;
use crate::zero_flow_auth::ZeroFlowSnapshot;
use crate::zero_flow_newness::ChangeKey;

/// A delegation tx against its delegator's delegation as the next epoch boundary would see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DelegationView {
    named: PublicKey,
    bonds: Option<u32>,
    effective: Option<(PublicKey, u32)>,
    received: Option<u64>,
}

/// `bonds` is Some for a DelegateBond, None for a RevokeDelegation naming `delegate`.
pub(crate) fn delegation_view(
    ps: &ProducerSet,
    delegator: &PublicKey,
    delegate: &PublicKey,
    bonds: Option<u32>,
) -> DelegationView {
    let mut effective = ps
        .get_by_pubkey(delegator)
        .and_then(|i| i.delegated_to.map(|to| (to, i.delegated_bonds)));
    for update in ps.pending_updates_for(delegator) {
        match update {
            PendingProducerUpdate::DelegateBond {
                delegate,
                bond_count,
                ..
            } if effective.is_none() => effective = Some((*delegate, *bond_count)),
            PendingProducerUpdate::RevokeDelegation { .. } => effective = None,
            _ => {}
        }
    }
    let received = ps.get_by_pubkey(delegate).map(|p| {
        p.received_delegations
            .iter()
            .map(|(_, c)| u64::from(*c))
            .sum()
    });
    DelegationView {
        named: *delegate,
        bonds,
        effective,
        received,
    }
}

/// True iff the tx index places `tx` at a height whose canonical block carries it.
pub(crate) fn mined_in_chain(tx: &Transaction, blocks: &BlockStore) -> bool {
    let txid = tx.hash();
    let Ok(Some(height)) = blocks.get_tx_block_height(&txid) else {
        return false;
    };
    blocks
        .get_block_by_height(height)
        .ok()
        .flatten()
        .is_some_and(|b| b.transactions.iter().any(|t| t.hash() == txid))
}

fn refuse(code: &str, tx: &Transaction, why: &str) -> Result<(), MempoolError> {
    Err(MempoolError::InvalidTransaction(format!(
        "[{code}] {:?} {why}",
        tx.tx_type
    )))
}

fn change_data(tx: &Transaction) -> Option<MaintainerChangeData> {
    matches!(tx.tx_type, TxType::AddMaintainer | TxType::RemoveMaintainer)
        .then(|| MaintainerChangeData::from_bytes(&tx.extra_data))
        .flatten()
}

/// Newness key of a maintainer change: its signed content (action, target).
pub(crate) fn change_key(tx: &Transaction) -> Option<ChangeKey> {
    change_data(tx).map(|d| (tx.tx_type == TxType::AddMaintainer, d.target))
}

/// Checks that run before the multisig: canonical payload bytes, then the state-effect precondition.
pub(crate) fn pre_auth(
    tx: &Transaction,
    snapshot: Option<&ZeroFlowSnapshot>,
) -> Result<(), MempoolError> {
    if !matches!(tx.tx_type, TxType::AddMaintainer | TxType::RemoveMaintainer) {
        return Ok(());
    }
    let Some(d) = change_data(tx).filter(|d| d.to_bytes() == tx.extra_data) else {
        return refuse("MPTX012", tx, "payload is not canonical");
    };
    let sigs = &d.signatures;
    if (1..sigs.len()).any(|i| sigs[..i].iter().any(|s| s.pubkey == sigs[i].pubkey)) {
        return refuse("MPTX012", tx, "repeats a signature entry");
    }
    match snapshot.and_then(|s| s.maintainers.as_ref()) {
        Some(set) if !effect_possible(tx.tx_type, set, &d.target) => {
            refuse("MPTX011", tx, "has no effect on the current maintainer set")
        }
        _ => Ok(()),
    }
}

fn effect_possible(tx_type: TxType, set: &MaintainerSet, target: &PublicKey) -> bool {
    if tx_type == TxType::AddMaintainer {
        set.can_add() && !set.is_maintainer(target)
    } else {
        set.can_remove() && set.is_maintainer(target)
    }
}

/// Checks that run after the multisig: the INC-I-246 delegation stop-gap, then the mined check.
pub(crate) fn post_auth(
    tx: &Transaction,
    snapshot: Option<&ZeroFlowSnapshot>,
    height: u64,
    network: Network,
) -> Result<(), MempoolError> {
    let Some(snap) = snapshot else {
        return Ok(());
    };
    if let Some(view) = &snap.delegation {
        delegation_stop_gap(tx, view, height, network)?;
    }
    if snap.mined_in_chain {
        return refuse("MPTX014", tx, "is already mined in the canonical chain");
    }
    Ok(())
}

fn delegation_stop_gap(
    tx: &Transaction,
    view: &DelegationView,
    height: u64,
    network: Network,
) -> Result<(), MempoolError> {
    let Some(bonds) = view.bonds else {
        return match view.effective {
            Some((current, _)) if current != view.named => {
                refuse("MPTX013", tx, "names a delegate other than the current one")
            }
            _ => Ok(()),
        };
    };
    if view.effective == Some((view.named, bonds)) {
        return refuse("MPTX013", tx, "repeats the current delegation");
    }
    let p = network.params();
    let cap = if height >= p.received_delegation_cap_activation_height
        && p.received_delegation_cap != u64::MAX
    {
        p.received_delegation_cap
    } else {
        0
    };
    match view.received {
        Some(total) if cap > 0 && total.saturating_add(u64::from(bonds)) > cap => {
            refuse("MPTX013", tx, "exceeds the delegate's received cap")
        }
        _ => Ok(()),
    }
}
