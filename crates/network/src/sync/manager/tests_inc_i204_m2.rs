//! INC-I-204 M2 / REQ-FORK-013 — branch-aware `best_peer`. TESTS-FIRST (RED).
//!
//! `best_peer()` selects a sync source on height + blacklist only, with no ancestry
//! term, so a wedged node re-draws sources from peers that cannot serve connecting
//! headers. M2 adds "prefer peers last observed on OUR branch, fall back to the full
//! eligible set".
//!
//! TRAP T4: every peer eligible for `best_peer` is AHEAD of us, and
//! `recent_canonical_hashes` only spans `[local_height-199, local_height]`, so a
//! naive "filter to peers agreeing right now" classifies 100% of candidates as
//! non-agreeing, empties the candidate set, and the node syncs from nobody. The
//! unfiltered fallback is therefore MANDATORY, and (c) is its regression test.
//!
//! Preserved invariants: INV-SYNC-005 (re-opened — canonical-membership filtering
//! of sync sources), INV-SYNC-009 (request chokepoint untouched), INC-I-014 (load
//! distribution), INC-I-017 (`sync_epoch` seeding / anti-thundering-herd).
//!
//! OUTPUT CONTRACT — `fn SyncManager::best_peer(&self) -> Option<PeerId>`
//!   O1 mutable params: none (`&self`).       O2 receiver mutation: none.
//!   O3 return value: `Option<PeerId>` — the ONLY observable output.
//!   O4 persistent store: none.  O5 statics: none.  O6 events/channels: none.
//!   PATHS: P1 eligible set empty → `None`.
//!          P2 eligible non-empty, no fresh Agreeing verdict → seeded index over
//!             the FULL eligible set (today's behavior, unchanged).
//!          P3 eligible non-empty, >= 1 fresh Agreeing verdict → seeded index over
//!             the Agreeing partition.
//!   INPUT PARTITIONS (one cell each):
//!          IP-A one Agreeing among a Divergent cohort            → (a)  P3
//!          IP-B no classifiable observation on any peer          → (b)  P2
//!          IP-C every eligible peer Divergent (minority branch)  → (c)  P2
//!          IP-D the only Agreeing verdict is stale               → (d)  P2
//!          IP-E several Agreeing peers (load distribution)       → (e), (f) P3
//!          IP-F a blacklisted peer, whatever its verdict         → (h)  P2/P3
//!          IP-G nobody ahead of us                               → (i)  P1
//!   MATRIX: O3 × {P1, P2, P3} = 3 cells, all asserted.
//!
//! OUTPUT CONTRACT — `fn SyncManager::checkpoint_health(&self) -> (usize,usize,usize)`
//!   O3 tuple is the only output; pinned by (g) so extracting the classifier out of
//!   `checkpoint_health` cannot change what it reports.
//!
//! FAIL EVIDENCE (pre-fix, decision.rs unmodified): (a) MUST FAIL.
//! (b), (c), (d), (e), (f), (g), (h), (i) are PASS-locks — green before AND after.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use crypto::Hash;
use libp2p::PeerId;

use crate::sync::manager::{SyncConfig, SyncManager};

/// Ring top / `local_height`. Well under the 200-entry ring cap, so heights
/// `1..=RING` are all inside the classifiable window.
const RING: u64 = 40;

/// Trial count. `best_peer` indexes a `HashMap`-derived `Vec` with a
/// `sync_epoch`-seeded index, so a single call can land on the right peer by
/// accident. Every branch-verdict assertion is repeated over independent trials.
const TRIALS: u64 = 60;

/// Our canonical hash at `h` — what `recent_canonical_hashes` holds.
fn our_hash(h: u64) -> Hash {
    crypto::hash::hash(format!("inc_i204_m2_canonical_{h}").as_bytes())
}

/// A hash that is never on our canonical chain.
fn foreign_hash(tag: &str) -> Hash {
    crypto::hash::hash(format!("inc_i204_m2_foreign_{tag}").as_bytes())
}

/// A manager whose `recent_canonical_hashes` ring holds `1..=local_height`.
fn mgr_with_ring(local_height: u64) -> SyncManager {
    let mut mgr = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    for h in 1..=local_height {
        mgr.update_local_tip(h, our_hash(h), h as u32);
    }
    mgr
}

/// A peer whose LAST CLASSIFIABLE observation was `obs_hash` at `obs_h` (inside our
/// ring), and which then advanced to `ahead_h` (outside the ring → unclassifiable,
/// so the earlier verdict is the only one available) making it eligible.
///
/// The classifiable observation is delivered through BOTH status entry points
/// (`add_peer` then `update_peer`) so the test does not depend on which one the
/// implementation records from.
fn peer_observed_at(
    mgr: &mut SyncManager,
    obs_h: u64,
    obs_hash: Hash,
    ahead_h: u64,
    tag: &str,
) -> PeerId {
    let p = PeerId::random();
    mgr.add_peer(p, obs_h, obs_hash, obs_h as u32);
    mgr.update_peer(p, obs_h, obs_hash, obs_h as u32);
    mgr.update_peer(p, ahead_h, foreign_hash(tag), ahead_h as u32);
    p
}

/// Eligible peer whose last classifiable observation matched our canonical chain.
fn agreeing_ahead(mgr: &mut SyncManager, obs_h: u64, ahead_h: u64, tag: &str) -> PeerId {
    peer_observed_at(mgr, obs_h, our_hash(obs_h), ahead_h, tag)
}

/// Eligible peer whose last classifiable observation contradicted our chain at that
/// height — the wedged-cohort shape measured in INC-I-204.
fn divergent_ahead(mgr: &mut SyncManager, obs_h: u64, ahead_h: u64, tag: &str) -> PeerId {
    let wrong = foreign_hash(&format!("divergent_{tag}"));
    peer_observed_at(mgr, obs_h, wrong, ahead_h, tag)
}

/// Eligible peer that never reported a height inside our ring — no verdict is
/// derivable from any observation it made (trap T4's default population).
fn unclassified_ahead(mgr: &mut SyncManager, ahead_h: u64, tag: &str) -> PeerId {
    let p = PeerId::random();
    mgr.add_peer(p, ahead_h, foreign_hash(tag), ahead_h as u32);
    mgr.update_peer(
        p,
        ahead_h + 1,
        foreign_hash(&format!("{tag}_next")),
        (ahead_h + 1) as u32,
    );
    p
}

/// Push every peer observation into the past.
///
/// DEVELOPER CONTRACT (M2): the branch verdict recorded per peer carries a
/// timestamp and is honored only within a freshness bound tied to the status/ping
/// cadence. That bound MUST be reachable from a unit test — either the verdict is
/// aged through an existing peer timestamp, or the implementation exposes a
/// `#[cfg(test)]` ageing/clock seam and this helper calls it. A verdict that no
/// test can age is a verdict that can pin a peer choice forever; test (d) exists to
/// make that unimplementable.
fn age_all_observations(mgr: &mut SyncManager, by: Duration) {
    let past = Instant::now() - by;
    for status in mgr.peers.values_mut() {
        status.last_status_response = past;
    }
}

/// The full eligible set as today's predicate computes it (height + blacklist).
/// Used to prove fallback selections stay inside the unfiltered set.
fn eligible_today(mgr: &SyncManager) -> HashSet<PeerId> {
    mgr.peers
        .iter()
        .filter(|(pid, s)| {
            s.best_height > mgr.local_height && !mgr.fork.header_blacklisted_peers.contains_key(pid)
        })
        .map(|(pid, _)| *pid)
        .collect()
}

// ===========================================================================
// (a) INCIDENT SHAPE — MUST FAIL pre-fix. P3 / IP-A.
// ===========================================================================

/// REQ-FORK-013 — Decision: a failure here reveals that peer selection is still
/// blind to ancestry, so a wedged node keeps re-drawing sources from the losing
/// branch and header-first sync can never converge.
///
/// Covers O3 on P3: one Agreeing-and-ahead peer among a Divergent-and-ahead cohort
/// must be selected on EVERY trial, not on average.
#[test]
fn t4_incident_shape_advancing_peer_wins_over_wedged_cohort() {
    for epoch in 0..TRIALS {
        let mut mgr = mgr_with_ring(RING);

        let mut wedged = Vec::new();
        for i in 0..4u64 {
            wedged.push(divergent_ahead(
                &mut mgr,
                RING - 1 - i,
                RING + 10 + i,
                &format!("wedged_{i}"),
            ));
        }
        let advancing = agreeing_ahead(&mut mgr, RING - 1, RING + 5, "advancing");
        mgr.pipeline.sync_epoch = epoch;

        let chosen = mgr
            .best_peer()
            .expect("(a) eligible set is non-empty — best_peer must return Some");

        assert_eq!(
            chosen, advancing,
            "(a) trial epoch={epoch}: best_peer chose {chosen}, a member of the wedged \
             divergent cohort ({wedged:?}), over the only peer last observed on OUR \
             branch ({advancing}). REQ-FORK-013: the agreeing peer must win on every \
             trial, not on a seeded-index coin flip."
        );
    }
}

// ===========================================================================
// (b) NO VERDICT ANYWHERE — the trap T4 default population. P2 / IP-B.
// ===========================================================================

/// REQ-FORK-013 — Decision: a failure here reveals that the branch filter is applied
/// without a fallback, so the ordinary case (nobody classifiable) selects nobody and
/// sync stops network-wide.
///
/// Covers O3 on P2 twice: single-eligible-peer determinism, then multi-peer
/// membership in the FULL eligible set with INC-I-014 spread preserved.
#[test]
fn t4_all_peers_unclassified_falls_back_to_previous_selection() {
    // Variant 1 — exactly one eligible peer makes the seeded index 0 for every
    // epoch, so the expected selection is deterministic and pinnable.
    let mut solo = mgr_with_ring(RING);
    let only = unclassified_ahead(&mut solo, RING + 7, "solo");
    for epoch in 0..TRIALS {
        solo.pipeline.sync_epoch = epoch;
        assert_eq!(
            solo.best_peer(),
            Some(only),
            "(b) variant 1, epoch={epoch}: the single eligible peer must be selected \
             exactly as the pre-change implementation selects it."
        );
    }

    // Variant 2 — several eligible peers, none classifiable. Selection must stay
    // inside the unfiltered eligible set and must still spread across it.
    let mut many = mgr_with_ring(RING);
    for i in 0..5u64 {
        unclassified_ahead(&mut many, RING + 11 + i * 3, &format!("no_verdict_{i}"));
    }
    let eligible = eligible_today(&many);
    assert_eq!(eligible.len(), 5, "(b) fixture: 5 eligible peers expected");

    let mut seen = HashSet::new();
    for epoch in 0..TRIALS {
        many.pipeline.sync_epoch = epoch;
        let chosen = many
            .best_peer()
            .expect("(b) variant 2: fallback must never return None with 5 peers ahead");
        assert!(
            eligible.contains(&chosen),
            "(b) variant 2, epoch={epoch}: {chosen} is not in the unfiltered eligible set"
        );
        seen.insert(chosen);
    }
    assert!(
        seen.len() > 1,
        "(b) variant 2: INC-I-014 — fallback selected only {} distinct peer(s) across \
         {TRIALS} epochs; the seeded index must still distribute load.",
        seen.len()
    );
}

// ===========================================================================
// (c) T4 REGRESSION — local node on the minority branch. P2 / IP-C.
// ===========================================================================

/// REQ-FORK-013 / trap T4 — Decision: a failure here reveals the strict-filter
/// deadlock: a node whose peers are all on the other branch would select no sync
/// source at all and could never leave the minority branch it is stuck on.
///
/// Covers O3 on P2: every eligible peer Divergent must still yield `Some(_)`.
#[test]
fn t4_local_on_minority_branch_never_deadlocks() {
    for epoch in 0..TRIALS {
        let mut mgr = mgr_with_ring(RING);
        let mut cohort = HashSet::new();
        for i in 0..5u64 {
            cohort.insert(divergent_ahead(
                &mut mgr,
                RING - 1 - i,
                RING + 4 + i,
                &format!("minority_{i}"),
            ));
        }
        mgr.pipeline.sync_epoch = epoch;

        let chosen = mgr.best_peer();
        assert!(
            chosen.is_some(),
            "(c) trap T4, epoch={epoch}: every eligible peer is on the other branch and \
             best_peer returned None — the node would sync from nobody. The unfiltered \
             fallback is mandatory."
        );
        assert!(
            cohort.contains(&chosen.unwrap()),
            "(c) epoch={epoch}: fallback returned a peer outside the eligible cohort"
        );
    }
}

// ===========================================================================
// (d) STALE VERDICT — must not pin the choice. P2 / IP-D.
// ===========================================================================

/// REQ-FORK-013 — Decision: a failure here reveals that a branch verdict never
/// expires, so one old observation permanently pins the node to a single sync
/// source even after that peer has moved to another branch.
///
/// Covers O3 on P2: with the only Agreeing verdict aged past the freshness bound,
/// selection must fall back to the full eligible set and keep spreading.
/// See `age_all_observations` for the DEVELOPER CONTRACT this test enforces.
#[test]
fn t4_stale_verdict_does_not_pin_peer_choice() {
    let mut mgr = mgr_with_ring(RING);
    let stale_agreeing = agreeing_ahead(&mut mgr, RING - 1, RING + 4, "stale_agreeing");
    for i in 0..4u64 {
        unclassified_ahead(&mut mgr, RING + 9 + i * 3, &format!("no_verdict_{i}"));
    }

    // Push every observation an hour into the past — far beyond any freshness bound
    // derivable from the status/ping cadence.
    age_all_observations(&mut mgr, Duration::from_secs(3600));

    let eligible = eligible_today(&mgr);
    assert_eq!(eligible.len(), 5, "(d) fixture: 5 eligible peers expected");

    let mut seen = HashSet::new();
    for epoch in 0..TRIALS {
        mgr.pipeline.sync_epoch = epoch;
        let chosen = mgr
            .best_peer()
            .expect("(d): fallback must never return None with 5 peers ahead");
        assert!(
            eligible.contains(&chosen),
            "(d) epoch={epoch}: {chosen} is outside the unfiltered eligible set"
        );
        seen.insert(chosen);
    }

    assert!(
        seen.len() > 1,
        "(d): the only Agreeing verdict is stale, yet selection collapsed to {} peer(s) \
         across {TRIALS} epochs (stale_agreeing={stale_agreeing}). An expired verdict \
         must be treated as absent, not as a pin.",
        seen.len()
    );
}

// ===========================================================================
// (e)-(i) REGRESSION LOCKS — green before AND after.
// ===========================================================================

/// INC-I-014 — Decision: a failure here reveals that branch preference collapsed
/// sync onto one peer, recreating the serial-bottleneck hammering that made sync
/// time explode at 120+ nodes.
///
/// Covers O3 on P3: load distribution must hold INSIDE the preferred partition.
#[test]
fn inc_i_014_load_distribution_preserved() {
    let mut mgr = mgr_with_ring(RING);
    for i in 0..6u64 {
        agreeing_ahead(
            &mut mgr,
            RING - 1 - i,
            RING + 6 + i,
            &format!("agreeing_{i}"),
        );
    }

    let mut seen = HashSet::new();
    for epoch in 0..TRIALS {
        mgr.pipeline.sync_epoch = epoch;
        seen.insert(
            mgr.best_peer()
                .expect("(e): 6 peers are ahead — best_peer must return Some"),
        );
    }

    assert!(
        seen.len() > 1,
        "(e) INC-I-014: only {} distinct peer(s) selected across {TRIALS} epochs with 6 \
         eligible agreeing peers — load distribution was lost.",
        seen.len()
    );
}

/// INC-I-017 — Decision: a failure here reveals that the `sync_epoch` term stopped
/// varying the index, so every node at the same height picks the same source and
/// saturates its rate limit (thundering herd).
///
/// Covers O3 on P3: index is a pure function of `(local_height, sync_epoch)` modulo
/// the chosen set size — it varies with the epoch and repeats with that period.
#[test]
fn inc_i_017_sync_epoch_seeding_preserved() {
    let mut mgr = mgr_with_ring(RING);
    let mut peers = HashSet::new();
    for i in 0..5u64 {
        peers.insert(agreeing_ahead(
            &mut mgr,
            RING - 1 - i,
            RING + 6 + i,
            &format!("seeded_{i}"),
        ));
    }
    let set_len = peers.len() as u64;

    let mut by_epoch = Vec::new();
    for epoch in 0..(set_len * 3) {
        mgr.pipeline.sync_epoch = epoch;
        by_epoch.push(
            mgr.best_peer()
                .expect("(f): 5 peers are ahead — best_peer must return Some"),
        );
    }

    assert!(
        by_epoch.iter().collect::<HashSet<_>>().len() > 1,
        "(f) INC-I-017: identical local_height with {} different sync_epoch values \
         selected a single peer — the epoch term no longer seeds the index.",
        set_len * 3
    );

    for epoch in 0..set_len {
        assert_eq!(
            by_epoch[epoch as usize],
            by_epoch[(epoch + set_len) as usize],
            "(f) epoch={epoch}: selection must repeat with period {set_len} — the index \
             must stay `(local_height * K + sync_epoch) % chosen_set.len()`."
        );
    }
}

/// REQ-FORK-013 — Decision: a failure here reveals that extracting the per-peer
/// comparison out of `checkpoint_health` changed what checkpoint tagging reports,
/// silently altering fork detection downstream.
///
/// Covers checkpoint_health O3: agreeing / same-height-mismatch / not-in-ring /
/// stale-h0 peers in one scenario.
#[test]
fn checkpoint_health_tuple_unchanged() {
    let mut mgr = mgr_with_ring(RING);

    let agree = PeerId::random();
    mgr.add_peer(agree, RING - 2, our_hash(RING - 2), (RING - 2) as u32);

    let same_height_fork = PeerId::random();
    mgr.add_peer(
        same_height_fork,
        RING - 2,
        foreign_hash("ch_fork"),
        (RING - 2) as u32,
    );

    let ahead_of_ring = PeerId::random();
    mgr.add_peer(
        ahead_of_ring,
        RING + 100,
        foreign_hash("ch_ahead"),
        (RING + 100) as u32,
    );

    // Stale h=0 connection: skipped entirely while local_height > 10.
    let stale_zero = PeerId::random();
    mgr.add_peer(stale_zero, 0, foreign_hash("ch_zero"), 0);

    assert_eq!(
        mgr.checkpoint_health(),
        (3, 1, 3),
        "(g): checkpoint_health must report (counted=3, agreeing=1, unique_chain_tips=3) \
         — 4 peers minus the skipped h=0 connection, one on our chain, and 1 (ours) + 2 \
         distinct divergent hashes (same-height fork + out-of-ring peer)."
    );
}

/// REQ-FORK-013 — Decision: a failure here reveals that branch preference was
/// applied before the blacklist, resurrecting a peer that already proved it cannot
/// serve connecting headers.
///
/// Covers O3 on P2/P3: the blacklist term dominates any branch verdict.
#[test]
fn blacklisted_peer_still_excluded() {
    for epoch in 0..TRIALS {
        let mut mgr = mgr_with_ring(RING);
        let banned = agreeing_ahead(&mut mgr, RING - 1, RING + 5, "banned_agreeing");
        let mut others = HashSet::new();
        for i in 0..3u64 {
            others.insert(divergent_ahead(
                &mut mgr,
                RING - 2 - i,
                RING + 8 + i,
                &format!("open_{i}"),
            ));
        }
        mgr.fork
            .header_blacklisted_peers
            .insert(banned, Instant::now());
        mgr.pipeline.sync_epoch = epoch;

        let chosen = mgr
            .best_peer()
            .expect("(h): 3 non-blacklisted peers are ahead — best_peer must return Some");
        assert_ne!(
            chosen, banned,
            "(h) epoch={epoch}: a header-blacklisted peer was selected because its branch \
             verdict was Agreeing. The blacklist term must not be weakened."
        );
        assert!(
            others.contains(&chosen),
            "(h) epoch={epoch}: {chosen} is outside the non-blacklisted eligible set"
        );
    }
}

/// REQ-FORK-013 — Decision: a failure here reveals that the empty-eligible-set case
/// started returning a peer, which would start a sync against a node that is not
/// ahead of us.
///
/// Covers O3 on P1: no peers, peers not ahead, and ahead-but-blacklisted.
#[test]
fn no_eligible_peers_returns_none() {
    let mut mgr = mgr_with_ring(RING);
    assert_eq!(mgr.best_peer(), None, "(i): no peers at all → None");

    // Peers at exactly our height and behind it are not eligible.
    let at_tip = PeerId::random();
    mgr.add_peer(at_tip, RING, our_hash(RING), RING as u32);
    let behind = PeerId::random();
    mgr.add_peer(behind, RING - 5, our_hash(RING - 5), (RING - 5) as u32);
    assert_eq!(
        mgr.best_peer(),
        None,
        "(i): peers at or below local_height are not eligible → None"
    );

    // The only peer ahead is blacklisted.
    let ahead = agreeing_ahead(&mut mgr, RING - 1, RING + 9, "ahead_but_banned");
    mgr.fork
        .header_blacklisted_peers
        .insert(ahead, Instant::now());
    assert_eq!(
        mgr.best_peer(),
        None,
        "(i): the only ahead peer is header-blacklisted → None"
    );
}
