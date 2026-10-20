//! INC-I-248 REL-M1 — the pure quorum rule after the release gate: eligibility, the fixed
//! snap-height filter, strict majority, the retired-key predicate and its per-network height.
//!
// covers: crates/core/src/maintainer/quorum.rs, crates/core/src/maintainer/retired.rs, crates/core/src/maintainer/mod.rs, crates/core/src/network_params/mod.rs, crates/core/src/network_params/defaults.rs, crates/core/src/network_params/env_loader.rs, crates/core/src/network_params/chainspec_loader.rs, crates/core/src/consensus/params.rs, crates/core/src/consensus/stress.rs
//!
//! REQ-248-007 (Must) — Decision: a failure means the peer quorum can still replace a usable or replayed root, adopt on a minority or tie, drop honest answers after a new block, or treat a retired key as live.
//!
// OUTPUT CONTRACT: fn quorum_eligible(local: &LocalRootView) -> bool
//   O3 return: true iff digest None AND last_derived_height > 0; O1/O2/O4/O5/O6 none (pure)
// PATHS: E1 EMPTY@S -> true; E2 EMPTY@0 -> false; E3 non-empty@any -> false
// INPUT PARTITIONS: S = 1 | 25_000 | u64::MAX; non-empty at 0 | S
// OUTPUT CONTRACT: fn select_quorum_maintainer_state(local, answers, quorum) -> QuorumVerdict
//   O3 return: Adopt{set (group-minimum last_updated), agreeing} | Keep(KeepReason); pure
// PATHS: R0 NotEligible; R1 filter (empty set, tip < S) -> NoAnswers; R2 Tie; R3 NoQuorum;
//   R4 NoMajority; R5 Adopt
// INPUT PARTITIONS: local = EMPTY@S | EMPTY@0 | canonical | producer seed | retired five;
//   answers = 7 honest + 3 forged (both orders) | 3v3 | 3v2v2 | 4v2v2 | 5v2v2 | 3v2 | 2v2 | 2v1v1;
//   peer tip = S-1 | S | S+500; last_updated = 0 | <S | >S
// OUTPUT CONTRACT: fn retired_genesis_maintainer_keys() -> Vec<PublicKey>; fn holds_retired_key_at(set, h, retired_after) -> bool
//   O3 return only; PATHS: h > retired_after & holds -> true; h == retired_after | MAX | no retired key | empty -> false
// OUTPUT CONTRACT: NetworkParams::defaults(n).retired_genesis_maintainer_keys_height; ConsensusParams::for_network(n).retired_genesis_maintainer_keys_height
//   O3 return: mainnet 331_457, testnet u64::MAX, devnet u64::MAX
// MATRIX: eligible 6 cells; select 7 paths x O3 = 19 asserted verdicts; retired 7 cells; params 6 cells

use crypto::PublicKey;
use doli_core::consensus::ConsensusParams;
use doli_core::genesis::MAINNET_GENESIS_PRODUCERS;
use doli_core::maintainer::{
    holds_retired_key_at, maintainer_set_digest, quorum_eligible, retired_genesis_maintainer_keys,
    select_quorum_maintainer_state, KeepReason, LocalRootView, PeerMaintainerAnswer, QuorumVerdict,
};
use doli_core::network_params::NetworkParams;
use doli_core::{MaintainerSet, Network};

const GENESIS: [u8; 32] = [0x48; 32];
const S: u64 = 25_000;
const QUORUM: usize = 3;
const HONEST_LAST_UPDATED: u64 = 20_310;
const FORGED_LAST_UPDATED: u64 = 0;
const MAINNET_RETIREMENT: u64 = 331_457;

fn pk(seed: u8) -> PublicKey {
    *crypto::KeyPair::from_seed([seed; 32]).public_key()
}

fn set_of(seeds: &[u8], last_updated: u64) -> MaintainerSet {
    MaintainerSet::with_members(seeds.iter().map(|s| pk(*s)).collect(), last_updated)
}

fn honest() -> MaintainerSet {
    set_of(&[101, 102, 103, 104, 105], HONEST_LAST_UPDATED)
}

fn forged() -> MaintainerSet {
    set_of(&[201, 202, 203], FORGED_LAST_UPDATED)
}

fn other() -> MaintainerSet {
    set_of(&[1, 2, 3, 4, 5], 15_087)
}

fn retired_five() -> Vec<PublicKey> {
    MAINNET_GENESIS_PRODUCERS
        .iter()
        .map(|(hex, _)| PublicKey::from_hex(hex).expect("retired key hex"))
        .collect()
}

fn digest(set: &MaintainerSet) -> [u8; 32] {
    maintainer_set_digest(set, &GENESIS)
}

fn at(set: &MaintainerSet, tip: u64) -> PeerMaintainerAnswer {
    PeerMaintainerAnswer {
        set: set.clone(),
        last_derived_height: tip,
        digest: digest(set),
        tip_height: tip,
    }
}

fn n_at(set: &MaintainerSet, n: usize, tip: u64) -> Vec<PeerMaintainerAnswer> {
    (0..n).map(|_| at(set, tip)).collect()
}

fn empty_at(height: u64) -> LocalRootView {
    LocalRootView {
        digest: None,
        last_derived_height: height,
    }
}

fn holding(set: &MaintainerSet, height: u64) -> LocalRootView {
    LocalRootView {
        digest: Some(digest(set)),
        last_derived_height: height,
    }
}

fn mix(groups: &[(&MaintainerSet, usize)], tip: u64) -> Vec<PeerMaintainerAnswer> {
    groups.iter().flat_map(|(s, n)| n_at(s, *n, tip)).collect()
}

fn adopt(set: MaintainerSet, agreeing: usize) -> QuorumVerdict {
    QuorumVerdict::Adopt { set, agreeing }
}

fn keep(reason: KeepReason) -> QuorumVerdict {
    QuorumVerdict::Keep(reason)
}

// REQ-248-007 — Decision: a failure means a replay or usable root (or a fresh EMPTY@0 node) is still a quorum target.
#[test]
fn inc_i_248_rel_m1_quorum_eligible_only_empty_at_snap_height() {
    assert!(quorum_eligible(&empty_at(S)), "E1: EMPTY@S");
    assert!(quorum_eligible(&empty_at(1)), "E1: EMPTY@1");
    assert!(quorum_eligible(&empty_at(u64::MAX)), "E1: EMPTY@MAX");
    assert!(!quorum_eligible(&empty_at(0)), "E2: EMPTY@0 (QA-248-R01)");
    assert!(!quorum_eligible(&holding(&honest(), S)), "E3: canonical@S");
    assert!(!quorum_eligible(&holding(&other(), 0)), "E3: non-empty@0");
}

// REQ-248-007 — Decision: a failure means forged answers (high tips, low last_updated) still replace a non-eligible root.
#[test]
fn inc_i_248_rel_m1_quorum_not_eligible_ignores_any_answers() {
    let retired = MaintainerSet::with_members(retired_five(), 0);
    let colluding = n_at(&forged(), 5, S + 6_000);
    let mut honest_and_forged = n_at(&forged(), 3, S + 500);
    honest_and_forged.extend(n_at(&honest(), 7, S + 500));
    let locals = [
        ("canonical", holding(&honest(), S + 5_000)),
        ("producer seed", holding(&other(), S + 5_000)),
        ("retired five", holding(&retired, 480_001)),
        ("EMPTY@0", empty_at(0)),
    ];
    for (name, local) in &locals {
        for (case, answers) in [
            ("5 colluding", &colluding),
            ("7 honest + 3 forged", &honest_and_forged),
        ] {
            assert_eq!(
                select_quorum_maintainer_state(local, answers, QUORUM),
                keep(KeepReason::NotEligible),
                "{name} / {case}"
            );
        }
        assert_eq!(
            select_quorum_maintainer_state(local, &[], QUORUM),
            keep(KeepReason::NotEligible),
            "{name} / no answers: NotEligible precedes NoAnswers"
        );
    }
}

// REQ-248-007 — Decision: a failure means an honest answer from the window start is dropped after the node applies a block, or a forged last_updated still filters.
#[test]
fn inc_i_248_rel_m1_quorum_filter_uses_fixed_snap_height() {
    let local = empty_at(S);
    assert_eq!(
        select_quorum_maintainer_state(&local, &n_at(&honest(), 3, S), QUORUM),
        adopt(honest(), 3),
        "peer tip == S counts"
    );
    assert_eq!(
        select_quorum_maintainer_state(&local, &n_at(&honest(), 3, S - 1), QUORUM),
        keep(KeepReason::NoAnswers),
        "peer tip == S - 1 is dropped"
    );
    let ahead = set_of(&[101, 102, 103, 104, 106], S + 1);
    assert_eq!(
        select_quorum_maintainer_state(&local, &n_at(&ahead, 3, S + 500), QUORUM),
        adopt(ahead.clone(), 3),
        "last_updated above S is not filtered any more"
    );
    let mut lagging_majority = n_at(&other(), 4, S - 1);
    lagging_majority.extend(n_at(&honest(), 3, S + 500));
    assert_eq!(
        select_quorum_maintainer_state(&local, &lagging_majority, QUORUM),
        adopt(honest(), 3),
        "answers below S neither form nor block a group"
    );
}

// REQ-248-007 — Decision: a failure means a quorum-sized minority, a tie or exactly half of the counted answers is adopted.
#[test]
fn inc_i_248_rel_m1_quorum_strict_majority_required() {
    let local = empty_at(S);
    let tip = S + 100;
    let o = other();
    let (h, f) = (honest(), forged());
    for answers in [mix(&[(&f, 3), (&h, 7)], tip), mix(&[(&h, 7), (&f, 3)], tip)] {
        assert_eq!(
            select_quorum_maintainer_state(&local, &answers, QUORUM),
            adopt(honest(), 7),
            "7 honest + 3 forged"
        );
    }
    let cases = [
        (mix(&[(&h, 3), (&f, 3)], tip), keep(KeepReason::Tie)),
        (
            mix(&[(&h, 3), (&f, 2), (&o, 2)], tip),
            keep(KeepReason::NoMajority {
                largest: 3,
                counted: 7,
            }),
        ),
        (
            mix(&[(&f, 2), (&h, 4), (&o, 2)], tip),
            keep(KeepReason::NoMajority {
                largest: 4,
                counted: 8,
            }),
        ),
        (mix(&[(&o, 2), (&f, 2), (&h, 5)], tip), adopt(honest(), 5)),
        (mix(&[(&f, 2), (&h, 3)], tip), adopt(honest(), 3)),
    ];
    for (answers, want) in cases {
        assert_eq!(
            select_quorum_maintainer_state(&local, &answers, QUORUM),
            want,
            "{} answers",
            answers.len()
        );
    }
}

// REQ-248-007 — Decision: a failure means the keep reasons are reordered, so the probe retries or stops on the wrong verdict.
#[test]
fn inc_i_248_rel_m1_quorum_keep_reason_precedence() {
    let local = empty_at(S);
    let (h, f, o) = (honest(), forged(), other());
    let empty = MaintainerSet::new();
    assert_eq!(
        select_quorum_maintainer_state(&local, &n_at(&empty, 5, S), QUORUM),
        keep(KeepReason::NoAnswers),
        "empty peer sets never count"
    );
    assert_eq!(
        select_quorum_maintainer_state(&local, &mix(&[(&h, 2), (&f, 2)], S), QUORUM),
        keep(KeepReason::Tie),
        "Tie precedes NoQuorum"
    );
    assert_eq!(
        select_quorum_maintainer_state(&local, &mix(&[(&h, 2), (&f, 1), (&o, 1)], S), QUORUM),
        keep(KeepReason::NoQuorum {
            largest: 2,
            quorum: QUORUM
        }),
        "NoQuorum precedes NoMajority"
    );
}

// REQ-248-007 — Decision: a failure means a retired genesis key is treated as live on mainnet, or as retired on testnet/devnet or at the boundary.
#[test]
fn inc_i_248_rel_m1_retired_keys_predicate() {
    let mut got: Vec<String> = retired_genesis_maintainer_keys()
        .iter()
        .map(|k| k.to_hex())
        .collect();
    got.sort();
    let mut want: Vec<String> = retired_five().iter().map(|k| k.to_hex()).collect();
    want.sort();
    assert_eq!(got, want, "retired keys == MAINNET_GENESIS_PRODUCERS");

    let all = MaintainerSet::with_members(retired_five(), 0);
    let mut members: Vec<PublicKey> = [101, 102, 103, 104].iter().map(|s| pk(*s)).collect();
    members.push(retired_five()[3]);
    let one = MaintainerSet::with_members(members, 20_000);
    let r = MAINNET_RETIREMENT;
    assert!(holds_retired_key_at(&all, r + 1, r), "all five above");
    assert!(
        holds_retired_key_at(&one, r + 1, r),
        "one retired key above"
    );
    assert!(
        !holds_retired_key_at(&all, r, r),
        "at the retirement height"
    );
    assert!(
        !holds_retired_key_at(&all, u64::MAX, u64::MAX),
        "never retired"
    );
    assert!(!holds_retired_key_at(&honest(), r + 1, r), "no retired key");
    assert!(
        !holds_retired_key_at(&MaintainerSet::new(), r + 1, r),
        "empty set"
    );
    assert!(!holds_retired_key_at(&one, 0, r), "height 0");
}

// REQ-248-007 — Decision: a failure means the retirement height is live on testnet/devnet (resets their on-chain sets) or missing on mainnet.
#[test]
fn inc_i_248_rel_m1_params_retirement_height() {
    for (network, want) in [
        (Network::Mainnet, MAINNET_RETIREMENT),
        (Network::Testnet, u64::MAX),
        (Network::Devnet, u64::MAX),
    ] {
        assert_eq!(
            NetworkParams::defaults(network).retired_genesis_maintainer_keys_height,
            want,
            "{network:?} NetworkParams"
        );
        assert_eq!(
            ConsensusParams::for_network(network).retired_genesis_maintainer_keys_height,
            want,
            "{network:?} ConsensusParams (the node reads self.params)"
        );
    }
}
