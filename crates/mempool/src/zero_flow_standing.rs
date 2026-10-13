//! Delegation standing at admission (INC-I-237 M12.1, AUDIT-P1-515): the
//! epoch-boundary preconditions of `delegate_bonds_capped` / `revoke_delegation`,
//! read on the applied ProducerSet plus the delegator's queued updates.

use crypto::PublicKey;
use storage::{PendingProducerUpdate, ProducerSet};

/// Delegator and delegate as the next epoch boundary would see them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Standing {
    delegator_known: bool,
    delegator_active: bool,
    is_self: bool,
    available: u64,
    delegate_active: bool,
}

impl Standing {
    /// `bonds` is Some for a DelegateBond; `queued` is the delegator's queued updates in queue order.
    pub(crate) fn of(
        ps: &ProducerSet,
        delegator: &PublicKey,
        delegate: &PublicKey,
        bonds: Option<u32>,
        queued: &[&PendingProducerUpdate],
    ) -> Self {
        let applied = ps.get_by_pubkey(delegator);
        let mut s = Standing {
            delegator_known: applied.is_some(),
            delegator_active: applied.is_some_and(|p| p.is_active()),
            is_self: delegator == delegate,
            available: applied.map_or(0, |p| u64::from(p.bond_count)),
            delegate_active: false,
        };
        for update in queued {
            match update {
                PendingProducerUpdate::Register { info, .. } => {
                    s.delegator_known = true;
                    s.delegator_active |= info.is_active();
                    s.available += u64::from(info.bond_count);
                }
                PendingProducerUpdate::AddBond { outpoints, .. } => {
                    s.available += outpoints.len() as u64;
                }
                _ => {}
            }
        }
        let pending = applied.map_or(0, |p| u64::from(p.withdrawal_pending_count));
        s.available = s.available.saturating_sub(pending);
        if bonds.is_some() && s.delegator_active && !s.is_self {
            s.delegate_active = active_at_boundary(ps, delegate);
        }
        s
    }

    /// Why the boundary turns the tx into a no-op, if it does.
    pub(crate) fn no_op(&self, bonds: Option<u32>, delegated: bool) -> Option<&'static str> {
        let Some(bonds) = bonds else {
            return if !self.delegator_known {
                Some("has an unregistered delegator")
            } else if !delegated {
                Some("revokes no effective delegation")
            } else {
                None
            };
        };
        if !self.delegator_active {
            Some("has an unregistered or inactive delegator")
        } else if self.is_self {
            Some("delegates to its own delegator")
        } else if !self.delegate_active {
            Some("names an unregistered or inactive delegate")
        } else if u64::from(bonds) > self.available {
            Some("exceeds the delegator's available bonds")
        } else if delegated {
            Some("has a delegator with an effective delegation")
        } else {
            None
        }
    }
}

fn active_at_boundary(ps: &ProducerSet, key: &PublicKey) -> bool {
    ps.get_by_pubkey(key).is_some_and(|p| p.is_active())
        || ps
            .pending_updates_for(key)
            .iter()
            .any(|u| matches!(u, PendingProducerUpdate::Register { info, .. } if info.is_active()))
}
