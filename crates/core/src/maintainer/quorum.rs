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
    pub snap_synced: bool,
    pub usable: bool,
    pub tip_height: u64,
}

/// Why the local root is kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeepReason {
    NoAnswers,
    NoQuorum { largest: usize, quorum: usize },
    Tie,
    AlreadyCanonical,
    PeersDisagree { agreeing: usize, dissenting: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuorumVerdict {
    Adopt { set: MaintainerSet, agreeing: usize },
    Keep(KeepReason),
}

/// An answer counts only when the peer is at or above the local tip, its set
/// did not change above the local tip, and its set is not empty.
fn counts(local: &LocalRootView, a: &PeerMaintainerAnswer) -> bool {
    a.tip_height >= local.tip_height
        && a.set.last_updated <= local.tip_height
        && !a.set.members.is_empty()
}

/// Select the maintainer set a peer quorum agrees on, or why the local one is kept.
pub fn select_quorum_maintainer_state(
    local: &LocalRootView,
    answers: &[PeerMaintainerAnswer],
    quorum: usize,
) -> QuorumVerdict {
    let valid: Vec<&PeerMaintainerAnswer> = answers.iter().filter(|a| counts(local, a)).collect();
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
    let best_at = groups
        .iter()
        .position(|(_, g)| g.len() == largest)
        .unwrap_or(0);
    let (best_digest, best) = groups.swap_remove(best_at);
    if local.digest == Some(best_digest) {
        return QuorumVerdict::Keep(KeepReason::AlreadyCanonical);
    }
    let dissenting = valid.len() - largest;
    if !local.snap_synced && local.usable && dissenting > 0 {
        return QuorumVerdict::Keep(KeepReason::PeersDisagree {
            agreeing: largest,
            dissenting,
        });
    }
    let mut set = best[0].set.clone();
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
