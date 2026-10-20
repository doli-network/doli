//! Peer-quorum selection of a maintainer trust root (INC-I-248).
//!
//! Pure rule: no storage, no network. The caller decodes and validates each
//! answer and drops any whose digest does not match its body under the LOCAL
//! genesis hash before calling [`select_quorum_maintainer_state`].

use super::MaintainerSet;

/// One peer's maintainer state, already validated by the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerMaintainerAnswer {
    pub set: MaintainerSet,
    pub last_derived_height: u64,
    pub digest: [u8; 32],
    pub tip_height: u64,
}

/// What the local node holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalRootView {
    /// `None` when the local set is empty.
    pub digest: Option<[u8; 32]>,
    pub last_derived_height: u64,
}

/// Why the local root is kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeepReason {
    NotEligible,
    NoAnswers,
    NoQuorum { largest: usize, quorum: usize },
    Tie,
    NoMajority { largest: usize, counted: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuorumVerdict {
    Adopt { set: MaintainerSet, agreeing: usize },
    Keep(KeepReason),
}

/// Only an EMPTY root stamped at a snap height may be replaced by the quorum.
pub fn quorum_eligible(local: &LocalRootView) -> bool {
    local.digest.is_none() && local.last_derived_height > 0
}

/// Select the maintainer set a peer quorum agrees on, or why the local one is kept.
pub fn select_quorum_maintainer_state(
    local: &LocalRootView,
    answers: &[PeerMaintainerAnswer],
    quorum: usize,
) -> QuorumVerdict {
    if !quorum_eligible(local) {
        return QuorumVerdict::Keep(KeepReason::NotEligible);
    }
    let valid: Vec<&PeerMaintainerAnswer> = answers
        .iter()
        .filter(|a| a.tip_height >= local.last_derived_height && !a.set.members.is_empty())
        .collect();
    if valid.is_empty() {
        return QuorumVerdict::Keep(KeepReason::NoAnswers);
    }

    let mut groups: Vec<([u8; 32], Vec<&PeerMaintainerAnswer>)> = Vec::new();
    for a in &valid {
        match groups.iter_mut().find(|(d, _)| *d == a.digest) {
            Some((_, members)) => members.push(a),
            None => groups.push((a.digest, vec![a])),
        }
    }
    let largest = groups.iter().map(|(_, g)| g.len()).max().unwrap_or(0);
    if groups.iter().filter(|(_, g)| g.len() == largest).count() > 1 {
        return QuorumVerdict::Keep(KeepReason::Tie);
    }
    if largest < quorum {
        return QuorumVerdict::Keep(KeepReason::NoQuorum { largest, quorum });
    }
    let counted = valid.len();
    if largest * 2 <= counted {
        return QuorumVerdict::Keep(KeepReason::NoMajority { largest, counted });
    }
    let best = groups
        .into_iter()
        .find(|(_, g)| g.len() == largest)
        .map(|(_, g)| g)
        .unwrap_or_default();
    let Some(first) = best.first() else {
        return QuorumVerdict::Keep(KeepReason::NoAnswers);
    };
    let mut set = first.set.clone();
    set.last_updated = best
        .iter()
        .map(|a| a.set.last_updated)
        .min()
        .unwrap_or(set.last_updated);
    QuorumVerdict::Adopt {
        set,
        agreeing: largest,
    }
}
