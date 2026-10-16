//! INC-I-248 M1 — the pure peer-quorum rule for a node's maintainer trust root.
//!
//! REQ-248-002 (Must) — Decision: a failure means a snap-joined node adopts a set the peer quorum did not agree on (a lagging, ahead, empty or minority answer) or never adopts the canonical one.
//! REQ-248-004 (Must) — Decision: a failure means a replay node with a usable root replaces its history-derived set on a non-unanimous vote.
//!
//! OUTPUT CONTRACT: fn select_quorum_maintainer_state(local, answers, quorum) -> QuorumVerdict
//!   O3 return: QuorumVerdict (Adopt{set, agreeing} | Keep(KeepReason))
//!   O1/O2/O4/O5/O6: none (pure function)
//! PATHS: R0 answer filter (tip, last_updated, empty set); R1 NoAnswers; R2 Tie;
//!   R3 NoQuorum; R4 AlreadyCanonical; R5 PeersDisagree; R6 Adopt (group-minimum last_updated).
//! INPUT PARTITIONS: local = snap-synced | replay usable | replay unusable | empty root;
//!   answers = none | below quorum | unanimous | majority + dissent | tie | empty sets |
//!   lagging peer tip | set changed above local tip | mixed last_updated in one group;
//!   boundaries tip == local tip and last_updated == local tip count.
//! MATRIX: 7 paths x O3 (variant + payload) = 7 cells, each asserted on the full verdict.

use crypto::PublicKey;
use doli_core::maintainer::{
    maintainer_set_digest, select_quorum_maintainer_state, KeepReason, LocalRootView,
    PeerMaintainerAnswer, QuorumVerdict,
};
use doli_core::MaintainerSet;

const GENESIS: [u8; 32] = [0x48; 32];
const LOCAL_TIP: u64 = 25_000;
const QUORUM: usize = 3;
const CANONICAL_LAST_UPDATED: u64 = 20_310;
const PRODUCER_LAST_UPDATED: u64 = 15_087;

fn pk(seed: u8) -> PublicKey {
    *crypto::KeyPair::from_seed([seed; 32]).public_key()
}

fn set_of(seeds: &[u8], last_updated: u64) -> MaintainerSet {
    MaintainerSet::with_members(seeds.iter().map(|s| pk(*s)).collect(), last_updated)
}

fn canonical() -> MaintainerSet {
    set_of(&[101, 102, 103, 104, 105], CANONICAL_LAST_UPDATED)
}

fn producers() -> MaintainerSet {
    set_of(&[1, 2, 3, 4, 5], PRODUCER_LAST_UPDATED)
}

fn digest(set: &MaintainerSet) -> [u8; 32] {
    maintainer_set_digest(set, &GENESIS)
}

fn answer_at(set: &MaintainerSet, tip_height: u64) -> PeerMaintainerAnswer {
    PeerMaintainerAnswer {
        set: set.clone(),
        last_derived_height: tip_height,
        digest: digest(set),
        tip_height,
    }
}

fn answers(set: &MaintainerSet, n: usize) -> Vec<PeerMaintainerAnswer> {
    (0..n).map(|_| answer_at(set, LOCAL_TIP)).collect()
}

fn snap_local() -> LocalRootView {
    LocalRootView {
        digest: None,
        snap_synced: true,
        usable: false,
        tip_height: LOCAL_TIP,
    }
}

fn replay_local(usable: bool) -> LocalRootView {
    LocalRootView {
        digest: Some(digest(&producers())),
        snap_synced: false,
        usable,
        tip_height: LOCAL_TIP,
    }
}

fn adopt(set: MaintainerSet, agreeing: usize) -> QuorumVerdict {
    QuorumVerdict::Adopt { set, agreeing }
}

fn majority_with_dissent() -> Vec<PeerMaintainerAnswer> {
    let mut v = answers(&canonical(), 3);
    v.push(answer_at(&producers(), LOCAL_TIP));
    v
}

#[test]
fn inc_i_248_select_quorum_no_answers() {
    assert_eq!(
        select_quorum_maintainer_state(&snap_local(), &[], QUORUM),
        QuorumVerdict::Keep(KeepReason::NoAnswers)
    );
    assert_eq!(
        select_quorum_maintainer_state(&replay_local(true), &[], QUORUM),
        QuorumVerdict::Keep(KeepReason::NoAnswers)
    );
}

#[test]
fn inc_i_248_select_quorum_below_quorum() {
    assert_eq!(
        select_quorum_maintainer_state(&snap_local(), &answers(&canonical(), 2), QUORUM),
        QuorumVerdict::Keep(KeepReason::NoQuorum {
            largest: 2,
            quorum: QUORUM
        })
    );
    let local = LocalRootView {
        digest: Some(digest(&canonical())),
        ..snap_local()
    };
    assert_eq!(
        select_quorum_maintainer_state(&local, &answers(&canonical(), 2), QUORUM),
        QuorumVerdict::Keep(KeepReason::NoQuorum {
            largest: 2,
            quorum: QUORUM
        }),
        "the quorum check runs before the already-canonical check"
    );
}

#[test]
fn inc_i_248_select_quorum_quorum_adopt() {
    assert_eq!(
        select_quorum_maintainer_state(&snap_local(), &answers(&canonical(), 3), QUORUM),
        adopt(canonical(), 3)
    );
}

#[test]
fn inc_i_248_select_quorum_snap_node_adopts_with_one_dissent() {
    let local = LocalRootView {
        snap_synced: true,
        ..replay_local(true)
    };
    assert_eq!(
        select_quorum_maintainer_state(&local, &majority_with_dissent(), QUORUM),
        adopt(canonical(), 3)
    );
}

#[test]
fn inc_i_248_select_quorum_replay_usable_keeps_on_dissent() {
    assert_eq!(
        select_quorum_maintainer_state(&replay_local(true), &majority_with_dissent(), QUORUM),
        QuorumVerdict::Keep(KeepReason::PeersDisagree {
            agreeing: 3,
            dissenting: 1
        })
    );
}

#[test]
fn inc_i_248_select_quorum_replay_usable_adopts_on_unanimous() {
    assert_eq!(
        select_quorum_maintainer_state(&replay_local(true), &answers(&canonical(), 3), QUORUM),
        adopt(canonical(), 3)
    );
}

#[test]
fn inc_i_248_select_quorum_replay_unusable_adopts_on_dissent() {
    assert_eq!(
        select_quorum_maintainer_state(&replay_local(false), &majority_with_dissent(), QUORUM),
        adopt(canonical(), 3)
    );
}

#[test]
fn inc_i_248_select_quorum_empty_local_root_adopts() {
    let local = LocalRootView {
        digest: None,
        snap_synced: false,
        usable: false,
        tip_height: LOCAL_TIP,
    };
    assert_eq!(
        select_quorum_maintainer_state(&local, &majority_with_dissent(), QUORUM),
        adopt(canonical(), 3)
    );
}

#[test]
fn inc_i_248_select_quorum_tie() {
    let mut v = answers(&canonical(), 3);
    v.extend(answers(&producers(), 3));
    assert_eq!(
        select_quorum_maintainer_state(&snap_local(), &v, QUORUM),
        QuorumVerdict::Keep(KeepReason::Tie)
    );
    let mut small = answers(&canonical(), 2);
    small.extend(answers(&producers(), 2));
    assert_eq!(
        select_quorum_maintainer_state(&snap_local(), &small, QUORUM),
        QuorumVerdict::Keep(KeepReason::Tie),
        "the tie check runs before the quorum check"
    );
}

#[test]
fn inc_i_248_select_quorum_empty_peer_sets_ignored() {
    let empty = MaintainerSet::new();
    let mut mixed = answers(&empty, 3);
    mixed.extend(answers(&canonical(), 2));
    assert_eq!(
        select_quorum_maintainer_state(&snap_local(), &mixed, QUORUM),
        QuorumVerdict::Keep(KeepReason::NoQuorum {
            largest: 2,
            quorum: QUORUM
        })
    );
    assert_eq!(
        select_quorum_maintainer_state(&snap_local(), &answers(&empty, 3), QUORUM),
        QuorumVerdict::Keep(KeepReason::NoAnswers)
    );
    assert_eq!(
        select_quorum_maintainer_state(&replay_local(false), &answers(&empty, 5), QUORUM),
        QuorumVerdict::Keep(KeepReason::NoAnswers)
    );
}

#[test]
fn inc_i_248_select_quorum_already_canonical() {
    let local = LocalRootView {
        digest: Some(digest(&canonical())),
        ..snap_local()
    };
    assert_eq!(
        select_quorum_maintainer_state(&local, &answers(&canonical(), 3), QUORUM),
        QuorumVerdict::Keep(KeepReason::AlreadyCanonical)
    );
    let replay = LocalRootView {
        digest: Some(digest(&canonical())),
        ..replay_local(true)
    };
    assert_eq!(
        select_quorum_maintainer_state(&replay, &majority_with_dissent(), QUORUM),
        QuorumVerdict::Keep(KeepReason::AlreadyCanonical),
        "the already-canonical check runs before the dissent check"
    );
}

#[test]
fn inc_i_248_select_quorum_peer_tip_below_local_tip_ignored() {
    let lagging: Vec<_> = (0..3)
        .map(|_| answer_at(&canonical(), LOCAL_TIP - 1))
        .collect();
    assert_eq!(
        select_quorum_maintainer_state(&snap_local(), &lagging, QUORUM),
        QuorumVerdict::Keep(KeepReason::NoAnswers)
    );

    let mut partial = lagging.clone();
    partial.extend(answers(&canonical(), 2));
    assert_eq!(
        select_quorum_maintainer_state(&snap_local(), &partial, QUORUM),
        QuorumVerdict::Keep(KeepReason::NoQuorum {
            largest: 2,
            quorum: QUORUM
        })
    );

    assert_eq!(
        select_quorum_maintainer_state(&snap_local(), &answers(&canonical(), 3), QUORUM),
        adopt(canonical(), 3),
        "an answer at exactly the local tip counts"
    );

    let mut stale_majority: Vec<_> = (0..4)
        .map(|_| answer_at(&producers(), LOCAL_TIP - 1))
        .collect();
    stale_majority.extend((0..3).map(|_| answer_at(&canonical(), LOCAL_TIP + 100)));
    assert_eq!(
        select_quorum_maintainer_state(&snap_local(), &stale_majority, QUORUM),
        adopt(canonical(), 3),
        "lagging peers never form or block a group"
    );
}

#[test]
fn inc_i_248_select_quorum_last_updated_above_local_tip_ignored() {
    let ahead = set_of(&[101, 102, 103, 104, 106], LOCAL_TIP + 1);
    let ahead_answers: Vec<_> = (0..3).map(|_| answer_at(&ahead, LOCAL_TIP + 500)).collect();
    assert_eq!(
        select_quorum_maintainer_state(&snap_local(), &ahead_answers, QUORUM),
        QuorumVerdict::Keep(KeepReason::NoAnswers)
    );

    let mut mixed = ahead_answers.clone();
    mixed.extend((0..3).map(|_| answer_at(&canonical(), LOCAL_TIP + 500)));
    assert_eq!(
        select_quorum_maintainer_state(&snap_local(), &mixed, QUORUM),
        adopt(canonical(), 3),
        "a set changed above the local tip never forms or ties a group"
    );

    let at_tip = set_of(&[101, 102, 103, 104, 105], LOCAL_TIP);
    let boundary: Vec<_> = (0..3)
        .map(|_| answer_at(&at_tip, LOCAL_TIP + 500))
        .collect();
    assert_eq!(
        select_quorum_maintainer_state(&snap_local(), &boundary, QUORUM),
        adopt(at_tip, 3),
        "a set changed at exactly the local tip counts"
    );
}

#[test]
fn inc_i_248_select_quorum_adopted_last_updated_is_group_minimum() {
    let local = LocalRootView {
        tip_height: 25_000,
        ..snap_local()
    };
    let with =
        |last_updated: u64| answer_at(&set_of(&[101, 102, 103, 104, 105], last_updated), 25_000);
    let group = [with(20_010), with(20_000), with(24_000)];
    assert!(
        group.iter().all(|a| a.digest == group[0].digest),
        "fixture: the digest must exclude last_updated"
    );

    let mut expected = canonical();
    expected.last_updated = 20_000;
    for order in [[0, 1, 2], [2, 0, 1], [1, 2, 0]] {
        let ordered: Vec<_> = order.iter().map(|i| group[*i].clone()).collect();
        assert_eq!(
            select_quorum_maintainer_state(&local, &ordered, QUORUM),
            adopt(expected.clone(), 3),
            "adopted last_updated must be the group minimum for order {order:?}"
        );
    }
}
