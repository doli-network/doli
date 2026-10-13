//! Zero-flow admission auth (INC-I-237 M8): refuse what apply would refuse.
//!
//! Mirrors the apply-time checks in `apply_block/tx_processing.rs` (delegations)
//! and `apply_block/governance.rs` (maintainer changes). Admission-only: block
//! validation and apply are untouched. M11 adds the pre-lock state snapshot.

use std::sync::Arc;

use crypto::Hash;
use doli_core::maintainer::{
    signing_message_at, MaintainerChangeData, MAINTAINER_AUTH_VALID_BEFORE_UNSET,
};
use doli_core::network::Network;
use doli_core::{MaintainerSet, Transaction, TxType};
use storage::{MaintainerState, ProducerSet};
use tokio::sync::RwLock;

use crate::zero_flow_policy::{delegation_view, DelegationView};

/// Zero-flow types whose authorization admission verifies; only these may authorize a gossip forward.
pub fn is_authenticated(tx_type: TxType) -> bool {
    matches!(
        tx_type,
        TxType::DelegateBond
            | TxType::RevokeDelegation
            | TxType::AddMaintainer
            | TxType::RemoveMaintainer
    )
}

/// State one zero-flow admission reads, taken BEFORE the mempool write lock (INC-I-237 M11).
#[derive(Clone, Debug, Default)]
pub struct ZeroFlowSnapshot {
    pub(crate) maintainers: Option<MaintainerSet>,
    pub(crate) delegation: Option<DelegationView>,
}

/// Snapshot for `tx`: the current maintainer set and the delegation view.
pub async fn snapshot_for(
    tx: &Transaction,
    maintainer_state: Option<&Arc<RwLock<MaintainerState>>>,
    producers: Option<&Arc<RwLock<ProducerSet>>>,
) -> ZeroFlowSnapshot {
    let mut snap = ZeroFlowSnapshot::default();
    let parties = match tx.tx_type {
        TxType::AddMaintainer | TxType::RemoveMaintainer => {
            if let Some(s) = maintainer_state {
                snap.maintainers = Some(s.read().await.set.clone());
            }
            None
        }
        TxType::DelegateBond => tx
            .delegate_bond_data()
            .map(|d| (d.delegator, d.delegate, Some(d.bond_count))),
        TxType::RevokeDelegation => tx
            .revoke_delegation_data()
            .map(|d| (d.delegator, d.delegate, None)),
        _ => return snap,
    };
    if let (Some(ps), Some((from, to, bonds))) = (producers, parties) {
        snap.delegation = Some(delegation_view(&*ps.read().await, &from, &to, bonds));
    }
    snap
}

/// Apply's authorization predicate for `tx` mined at `height`; types it does not cover pass.
pub(crate) fn authorized(
    tx: &Transaction,
    height: u64,
    network: Network,
    genesis_hash: &Hash,
    maintainers: Option<&MaintainerSet>,
) -> bool {
    let params = network.params();
    let delegation_ok = |msg: Hash, sig: &crypto::Signature, delegator: &crypto::PublicKey| {
        height < params.delegation_auth_activation_height
            || crypto::signature::verify_hash(&msg, sig, delegator).is_ok()
    };
    match tx.tx_type {
        TxType::DelegateBond => tx
            .delegate_bond_data()
            .is_some_and(|d| delegation_ok(d.signing_message(), &d.signature, &d.delegator)),
        TxType::RevokeDelegation => tx
            .revoke_delegation_data()
            .is_some_and(|d| delegation_ok(d.signing_message(), &d.signature, &d.delegator)),
        TxType::AddMaintainer | TxType::RemoveMaintainer => {
            match (
                maintainers,
                MaintainerChangeData::from_bytes(&tx.extra_data),
            ) {
                (Some(set), Some(d)) => {
                    let is_add = tx.tx_type == TxType::AddMaintainer;
                    let message = signing_message_at(
                        genesis_hash.as_bytes(),
                        is_add,
                        &d.target,
                        MAINTAINER_AUTH_VALID_BEFORE_UNSET,
                        height,
                        params.inc_i_176_auth_binding_activation_height,
                    );
                    let gate = params.maintainer_derivation_activation_height;
                    if is_add {
                        set.verify_multisig_at(&d.signatures, &message, height, gate)
                    } else {
                        set.verify_multisig_excluding_at(
                            &d.signatures,
                            &message,
                            &d.target,
                            height,
                            gate,
                        )
                    }
                }
                // No current set attached: admission cannot authenticate, so it refuses.
                _ => false,
            }
        }
        _ => true,
    }
}
