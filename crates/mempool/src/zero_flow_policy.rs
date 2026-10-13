//! Zero-flow admission policy (INC-I-237 M11/M12.1): canonical maintainer payloads,
//! state-effect preconditions, authorization, delegation standing and the INC-I-246
//! delegation stop-gap. Admission-only: block validation and apply are untouched.

use crypto::{Hash, PublicKey};
use doli_core::maintainer::MaintainerChangeData;
use doli_core::network::Network;
use doli_core::{MaintainerSet, Transaction, TxType};
use storage::{PendingProducerUpdate, ProducerSet};

use crate::pool::MempoolError;
use crate::zero_flow_auth::{authorized, ZeroFlowSnapshot};
use crate::zero_flow_newness::ChangeKey;
use crate::zero_flow_standing::Standing;

/// A delegation tx against its delegator's delegation as the next epoch boundary would see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DelegationView {
    named: PublicKey,
    bonds: Option<u32>,
    effective: Option<(PublicKey, u32)>,
    received: Option<u64>,
    standing: Standing,
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
    let queued = ps.pending_updates_for(delegator);
    for &update in &queued {
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
        standing: Standing::of(ps, delegator, delegate, bonds, &queued),
    }
}

/// Every zero-flow refusal after structural validation: payload and effect, authorization, standing.
pub(crate) fn check(
    tx: &Transaction,
    snapshot: Option<&ZeroFlowSnapshot>,
    current_height: u64,
    network: Network,
    genesis: &Hash,
) -> Result<(), MempoolError> {
    let mined_at = current_height.saturating_add(1);
    pre_auth(tx, snapshot)?;
    let maintainers = snapshot.and_then(|s| s.maintainers.as_ref());
    if !authorized(tx, mined_at, network, genesis, maintainers) {
        return Err(MempoolError::InvalidTransaction(format!(
            "[MPTX010] {:?} authorization refused at height {}",
            tx.tx_type, mined_at
        )));
    }
    post_auth(tx, snapshot, mined_at, network)
}

pub(crate) fn refuse(code: &str, tx: &Transaction, why: &str) -> Result<(), MempoolError> {
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

/// Newness key: a maintainer change's signed content (action, target); a delegation tx's delegator.
pub(crate) fn change_key(tx: &Transaction) -> Option<ChangeKey> {
    match tx.tx_type {
        TxType::DelegateBond => tx
            .delegate_bond_data()
            .map(|d| ChangeKey::Delegator(d.delegator)),
        TxType::RevokeDelegation => tx
            .revoke_delegation_data()
            .map(|d| ChangeKey::Delegator(d.delegator)),
        _ => change_data(tx)
            .map(|d| ChangeKey::Maintainer(tx.tx_type == TxType::AddMaintainer, d.target)),
    }
}

/// Checks that run before the multisig: canonical payload bytes, then the state-effect precondition.
fn pre_auth(tx: &Transaction, snapshot: Option<&ZeroFlowSnapshot>) -> Result<(), MempoolError> {
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

/// Checks that run after the multisig: delegation standing, then the INC-I-246 stop-gap.
fn post_auth(
    tx: &Transaction,
    snapshot: Option<&ZeroFlowSnapshot>,
    height: u64,
    network: Network,
) -> Result<(), MempoolError> {
    if !matches!(tx.tx_type, TxType::DelegateBond | TxType::RevokeDelegation) {
        return Ok(());
    }
    let Some(view) = snapshot.and_then(|s| s.delegation.as_ref()) else {
        return refuse("MPTX016", tx, "has no producer-set view to check standing");
    };
    if let Some(why) = view.standing.no_op(view.bonds, view.effective.is_some()) {
        return refuse("MPTX016", tx, why);
    }
    delegation_stop_gap(tx, view, height, network)
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
