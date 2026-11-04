// covers: manager/recovery.rs (classify Rules 1/1b) manager/sync_engine/response.rs (handle_headers_response)
//! INC-I-294 M-2: below the INC-I-288 gate, a depth-1 rollback of Rules 1/1b needs a
//! live `HeaderDivergence` for the current tip. Mainnet gate 881_800; below = tip
//! 881_799 (one peer at 881_800), above = tip 881_800 (one peer at 881_801).
//
// OUTPUT CONTRACT: RecoveryCoordinator::classify(&self, &ctx) -> RecoveryAction (O3 only;
//   &self, no mutation). SyncManager::handle_response(Headers) -> O2 fork.use_height_based_headers,
//   O2 coordinator evidence (HeaderDivergence), O6 next_request() request.
// PARTITIONS
//   gate:     B = tip 881_799 (Rules 1/1b live) | A = tip 881_800 (Rules 1/1b off)
//   proof:    P0 none | Ps stale (> EVIDENCE_TTL) | Po other tip 881_798 | Pf fresh 881_799
//   evidence: E1 2x OrphanGossip, apply 5 s ago | E2 StuckFork, apply 5 s ago
//             E3 StuckFork, apply >= STALE_TIP_SECS (wedge shape) | E0 none
//   finality: F+ allows (None) | F- refuses (Some(tip))
// MATRIX (classify)
//   B  E1|E2     P0|Ps|Po  F+  Rule 3             -> HeaderFirstSync
//   B  E3        P0|Ps|Po  F+  terminal           -> Wedged{NoActionableRung}
//   B  E1|E2|E3  Pf        F+  Rule 1/1b          -> ShallowRollback{1}
//   B  E0        Pf        F+  Rule 3             -> HeaderFirstSync (a proof is not a counter)
//   B  E1|E2     P0|Pf     F-  on_rollback_refused -> None
//   B  E3        P0|Pf     F-  on_rollback_refused -> SiblingFetch{881_799} (first attempt)
//   A  E1|E2     P0|Pf     F+  Rule 3             -> HeaderFirstSync
//   A  E3        P0|Pf     F+  terminal           -> Wedged{NoActionableRung}
//   A  E1        P0|Pf     F-  Rule 3             -> HeaderFirstSync
//   A  E3        P0|Pf     F-  terminal           -> Wedged{FinalityConflict}
// MATRIX (sync engine, tip 881_799)
//   empty reply, gap 1|3          -> flag true; next request GetHeadersByHeight(881_799)
//   empty reply, gap 1, counter 0|PARK-2 before -> flag true; counter PARK-1 -> PARK, flag
//     false, next_request() parks (None, counter reset to 0)
//   first header breaks (prev != local_hash) -> HeaderDivergence{peer, 881_799} (live)
//   header extends local_hash     -> no HeaderDivergence
//   empty reply                   -> no HeaderDivergence
//   break after one valid header, or first-header break once expected != local_hash -> none
//   (observed through classify with 2 fresh orphans: proof -> ShallowRollback{1}, none -> HeaderFirstSync)

use std::time::{Duration, Instant};

use crypto::{Hash, KeyPair};
use doli_core::{BlockHeader, Network, NetworkParams};
use libp2p::PeerId;
use vdf::{VdfOutput, VdfProof};

use super::recovery::{
    thresholds, RecoveryAction, RecoveryContext, RecoveryCoordinator, RecoveryEvidence, WedgeReason,
};
use super::sync_engine::PARK_EMPTY_HEADERS;
use crate::protocols::{SyncRequest, SyncResponse};
use crate::sync::manager::{
    RecoveryPhase, SyncConfig, SyncManager, SyncPhase, SyncPipelineData, SyncState,
};

const TIP_BELOW: u64 = 881_799;
const TIP_ABOVE: u64 = 881_800;
const HFS: RecoveryAction = RecoveryAction::HeaderFirstSync;
const BLIND: RecoveryAction = RecoveryAction::ShallowRollback { depth: 1 };
/// Ten EVIDENCE_TTLs (120 s each).
const STALE_AGE: Duration = Duration::from_secs(1_200);

fn mainnet_gate() -> u64 {
    let gate = NetworkParams::defaults(Network::Mainnet).inc_i_288_fork_choice_activation_height;
    assert_eq!(gate, 881_800, "mainnet inc_i_288 pin moved");
    gate
}

fn mainnet_config() -> SyncConfig {
    let p = NetworkParams::defaults(Network::Mainnet);
    SyncConfig {
        inc_i_288_fork_choice_activation_height: mainnet_gate(),
        inc_i_236_block_signature_activation_height: p.inc_i_236_block_signature_activation_height,
        inc_i_178_attestation_bls_activation_height: p.inc_i_178_attestation_bls_activation_height,
        ..SyncConfig::default()
    }
}

#[derive(Clone, Copy, Debug)]
enum Ev {
    Orphans,
    StuckRecent,
    StuckWedge,
}

const ALL_EV: [Ev; 3] = [Ev::Orphans, Ev::StuckRecent, Ev::StuckWedge];

fn last_applied(ev: Ev) -> u64 {
    match ev {
        Ev::Orphans | Ev::StuckRecent => 5,
        Ev::StuckWedge => thresholds::STALE_TIP_SECS + 50,
    }
}

/// One peer at `tip + 1` (gap 1), no rollback yet, budget free.
fn ctx(tip: u64, ev: Ev, finality: Option<u64>) -> RecoveryContext {
    RecoveryContext {
        local_height: tip,
        network_tip_height: tip + 1,
        peer_count: 10,
        last_applied_secs: last_applied(ev),
        shallow_rollback_count: 0,
        snap_attempts: 0,
        last_rollback_local_height: None,
        last_rollback_time: None,
        in_grace_period: false,
        last_finality_height: finality,
    }
}

/// Mainnet-gated coordinator holding `ev`, reported now.
fn coord(ev: Ev) -> RecoveryCoordinator {
    let mut c = RecoveryCoordinator::new();
    c.set_fork_choice_gate(mainnet_gate());
    match ev {
        Ev::Orphans => {
            c.report(RecoveryEvidence::OrphanGossip { slot: 11, gap: 1 });
            c.report(RecoveryEvidence::OrphanGossip { slot: 11, gap: 1 });
        }
        Ev::StuckRecent | Ev::StuckWedge => c.report(RecoveryEvidence::StuckFork { gap: 1 }),
    }
    c
}

/// No valid proof, finality allows: the ladder continues past Rules 1/1b.
fn no_proof_result(ev: Ev) -> RecoveryAction {
    match ev {
        Ev::Orphans | Ev::StuckRecent => HFS,
        Ev::StuckWedge => RecoveryAction::Wedged {
            reason: WedgeReason::NoActionableRung,
        },
    }
}

// INC-I-294 M-2 (Must) — Decision: a failure means a fresh orphan/stuck-fork counter alone still rolls back a mainnet tip below 881_800 at a one-block gap.
#[test]
fn inc_i_294_m2_fresh_counters_below_gate_without_header_proof_do_not_roll_back() {
    for ev in ALL_EV {
        let got = coord(ev).classify(&ctx(TIP_BELOW, ev, None));
        assert_eq!(
            got,
            no_proof_result(ev),
            "M-2 {ev:?} at tip {TIP_BELOW}, gap 1, no header proof: a depth-1 rollback needs proof"
        );
    }
}

/// Tip `local`, one peer at `local + gap`, header download pending on that peer.
fn mgr_awaiting_headers(local: u64, gap: u64) -> (SyncManager, PeerId) {
    let mut m = SyncManager::new(mainnet_config(), Hash::ZERO);
    let local_hash = crypto::hash::hash(b"inc-i-294 local tip");
    m.update_local_tip(local, local_hash, local as u32);
    m.recovery_phase = RecoveryPhase::Normal;
    let peer = PeerId::random();
    let peer_h = local + gap;
    m.add_peer(
        peer,
        peer_h,
        crypto::hash::hash(b"inc-i-294 peer tip"),
        peer_h as u32,
    );
    m.state = SyncState::Syncing {
        phase: SyncPhase::DownloadingHeaders,
        started_at: Instant::now(),
    };
    m.pipeline_data = SyncPipelineData::Headers {
        target_slot: peer_h as u32,
        peer,
        headers_count: 0,
    };
    (m, peer)
}

// INC-I-294 M-2 (Must) — Decision: a failure means an empty reply at gap <= 3 leaves the next request hash-based, so the node never sees the peer's header at tip+1 and can never earn the proof a true depth-1 fork needs.
#[test]
fn inc_i_294_m2_small_gap_empty_reply_switches_to_height_based_headers() {
    for gap in [1u64, 3] {
        let (mut m, peer) = mgr_awaiting_headers(TIP_BELOW, gap);
        assert!(
            !m.fork.use_height_based_headers,
            "setup gap {gap}: flag off"
        );
        m.handle_response(peer, SyncResponse::Headers(vec![]));
        assert!(
            m.fork.use_height_based_headers,
            "M-2 gap {gap}: an empty reply at gap <= 3 must arm GetHeadersByHeight"
        );
        m.start_sync();
        let next = m.next_request();
        assert!(
            matches!(
                &next,
                Some((to, SyncRequest::GetHeadersByHeight { start_height, .. }))
                    if *to == peer && *start_height == TIP_BELOW
            ),
            "M-2 gap {gap}: next request must be GetHeadersByHeight({TIP_BELOW}): {next:?}"
        );
        assert!(
            !m.fork.use_height_based_headers,
            "M-2 gap {gap}: the flag is one-shot"
        );
    }
}

// INC-I-294 M-2 (Must) — Decision: a failure means the gap <= 3 height flag bypasses the dispatch park, so the empty-headers counter can climb past PARK_EMPTY_HEADERS toward the cleanup >= 20 arm.
#[test]
fn inc_i_294_m2_small_gap_height_flag_respects_the_park_cap() {
    for before in [0, PARK_EMPTY_HEADERS - 2] {
        let (mut m, peer) = mgr_awaiting_headers(TIP_BELOW, 1);
        m.fork.consecutive_empty_headers = before;
        m.handle_response(peer, SyncResponse::Headers(vec![]));
        assert_eq!(
            m.fork.consecutive_empty_headers,
            before + 1,
            "counter {before}"
        );
        assert!(
            m.fork.use_height_based_headers,
            "M-2 counter {before} -> {}: below the park cap the flag is set",
            before + 1
        );
    }

    let (mut m, peer) = mgr_awaiting_headers(TIP_BELOW, 1);
    m.fork.consecutive_empty_headers = PARK_EMPTY_HEADERS - 1;
    m.handle_response(peer, SyncResponse::Headers(vec![]));
    assert_eq!(m.fork.consecutive_empty_headers, PARK_EMPTY_HEADERS);
    assert!(
        !m.fork.use_height_based_headers,
        "M-2 counter reaches PARK_EMPTY_HEADERS: the flag must stay off"
    );
    m.start_sync();
    let next = m.next_request();
    assert!(
        next.is_none(),
        "M-2 at the park cap: next_request() parks, no GetHeadersByHeight: {next:?}"
    );
    assert_eq!(
        m.fork.consecutive_empty_headers, 0,
        "M-2: the gap <= 3 park resets the counter"
    );
}

// ---- step 2: tests that need RecoveryEvidence::HeaderDivergence ----

fn proof(tip_height: u64) -> RecoveryEvidence {
    RecoveryEvidence::HeaderDivergence {
        peer: PeerId::random(),
        tip_height,
    }
}

/// `coord(ev)` plus a header proof reported now.
fn coord_with_proof(ev: Ev, tip_height: u64) -> RecoveryCoordinator {
    let mut c = coord(ev);
    c.report(proof(tip_height));
    c
}

// INC-I-294 M-2 (Must) — Decision: a failure means the gate also blocks a true depth-1 fork that a fetched header proves, or the proof check moved ahead of the finality guard and replaced its None/SiblingFetch answer.
#[test]
fn inc_i_294_m2_fresh_header_proof_for_current_tip_still_heals_a_true_depth_one_fork() {
    for ev in ALL_EV {
        let got = coord_with_proof(ev, TIP_BELOW).classify(&ctx(TIP_BELOW, ev, None));
        assert_eq!(
            got, BLIND,
            "M-2 {ev:?} + fresh proof for {TIP_BELOW}, finality allows"
        );
    }

    let mut alone = RecoveryCoordinator::new();
    alone.set_fork_choice_gate(mainnet_gate());
    alone.report(proof(TIP_BELOW));
    let got = alone.classify(&ctx(TIP_BELOW, Ev::Orphans, None));
    assert_eq!(got, HFS, "M-2: a proof alone is not minor-fork evidence");

    let refused = Some(TIP_BELOW);
    for (ev, want) in [
        (Ev::Orphans, RecoveryAction::None),
        (Ev::StuckRecent, RecoveryAction::None),
        (
            Ev::StuckWedge,
            RecoveryAction::SiblingFetch { height: TIP_BELOW },
        ),
    ] {
        let with = coord_with_proof(ev, TIP_BELOW).classify(&ctx(TIP_BELOW, ev, refused));
        assert_eq!(
            with, want,
            "M-2 {ev:?} + proof, finality refuses: guard answer unchanged"
        );
        let without = coord(ev).classify(&ctx(TIP_BELOW, ev, refused));
        assert_eq!(
            without, want,
            "M-2 {ev:?} no proof, finality refuses: guard answer unchanged"
        );
    }
}

// INC-I-294 M-2 (Must) — Decision: a failure means a proof recorded against an earlier tip still approves rolling back the current one, so the tip_height match is missing.
#[test]
fn inc_i_294_m2_header_proof_for_an_older_tip_does_not_roll_back_the_current_tip() {
    for ev in ALL_EV {
        let got = coord_with_proof(ev, TIP_BELOW - 1).classify(&ctx(TIP_BELOW, ev, None));
        assert_eq!(
            got,
            no_proof_result(ev),
            "M-2 {ev:?}: proof for {} is not proof for {TIP_BELOW}",
            TIP_BELOW - 1
        );
    }
}

// INC-I-294 M-2 (Must) — Decision: a failure means the proof check reads raw evidence instead of live(now), so a proof older than EVIDENCE_TTL (the M-1 defect class) still approves a rollback.
#[test]
fn inc_i_294_m2_stale_header_proof_does_not_roll_back() {
    let stale = Instant::now()
        .checked_sub(STALE_AGE)
        .expect("monotonic clock is older than ten evidence lifetimes");
    for ev in ALL_EV {
        let mut c = coord(ev);
        c.report_at(stale, proof(TIP_BELOW));
        let got = c.classify(&ctx(TIP_BELOW, ev, None));
        assert_eq!(
            got,
            no_proof_result(ev),
            "M-2 {ev:?}: a proof older than EVIDENCE_TTL is no proof"
        );
    }
}

/// The node-side INC-I-293 cell inputs (`classify_at`), optionally with a fresh proof.
fn sync_classify_at(tip: u64, ev: Ev, with_proof: bool) -> RecoveryAction {
    let p = NetworkParams::defaults(Network::Mainnet);
    let config = SyncConfig {
        inc_i_147_activation_height: p.inc_i_147_activation_height,
        inc_i_204_fork_choice_activation_height: p.inc_i_204_fork_choice_activation_height,
        inc_i_288_fork_choice_activation_height: mainnet_gate(),
        ..SyncConfig::default()
    };
    let mut sm = SyncManager::new(config, Hash::ZERO);
    sm.update_local_tip(tip, crypto::hash::hash(b"tip"), 10);
    sm.add_peer(
        PeerId::random(),
        tip + 1,
        crypto::hash::hash(b"peer tip"),
        11,
    );
    match ev {
        Ev::Orphans => {
            sm.report_orphan_gossip(11, 1);
            sm.report_orphan_gossip(11, 1);
        }
        Ev::StuckRecent | Ev::StuckWedge => sm.report_stuck_fork(1),
    }
    if with_proof {
        sm.recovery.report(proof(tip));
    }
    sm.classify_and_dispatch(0).0
}

// INC-I-294 M-2 (Must) — Decision: a failure means the header-proof change leaks above 881_800, where Rules 1/1b do not exist and any result change is a mainnet behaviour change at the pin.
#[test]
fn inc_i_294_m2_above_gate_rules_unchanged() {
    for ev in [Ev::Orphans, Ev::StuckRecent] {
        for with_proof in [false, true] {
            let got = sync_classify_at(TIP_ABOVE, ev, with_proof);
            assert!(
                !matches!(got, RecoveryAction::ShallowRollback { .. }),
                "BLIND-ON {ev:?} proof={with_proof}: tip {TIP_ABOVE} still rolls back blind: {got:?}"
            );
        }
    }
    let refused = Some(TIP_ABOVE);
    for (ev, finality, want) in [
        (Ev::Orphans, None, HFS),
        (Ev::StuckRecent, None, HFS),
        (
            Ev::StuckWedge,
            None,
            RecoveryAction::Wedged {
                reason: WedgeReason::NoActionableRung,
            },
        ),
        (Ev::Orphans, refused, HFS),
        (
            Ev::StuckWedge,
            refused,
            RecoveryAction::Wedged {
                reason: WedgeReason::FinalityConflict,
            },
        ),
    ] {
        let c = ctx(TIP_ABOVE, ev, finality);
        let without = coord(ev).classify(&c);
        let with = coord_with_proof(ev, TIP_ABOVE).classify(&c);
        assert_eq!(
            without, want,
            "above gate {ev:?} finality={finality:?}, no proof"
        );
        assert_eq!(
            with, without,
            "above gate {ev:?} finality={finality:?}: a proof changes nothing"
        );
    }
}

fn signed_header(prev_hash: Hash, slot: u32) -> BlockHeader {
    let kp = KeyPair::from_seed([0x6e; 32]);
    let mut h = BlockHeader {
        version: 1,
        prev_hash,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 1_700_000_000 + slot as u64,
        slot,
        producer: *kp.public_key(),
        vdf_output: VdfOutput { value: vec![0; 32] },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    h.vdf_proof.pi =
        crypto::signature::sign(h.signing_message().as_bytes(), kp.private_key()).to_vec();
    h
}

/// Deliver `replies` in order, then classify with two fresh orphan reports.
fn classify_after_replies(replies: impl Fn(Hash) -> Vec<Vec<BlockHeader>>) -> RecoveryAction {
    let (mut m, peer) = mgr_awaiting_headers(TIP_BELOW, 1);
    let local = m.local_hash;
    for headers in replies(local) {
        m.handle_response(peer, SyncResponse::Headers(headers));
    }
    m.report_orphan_gossip(11, 1);
    m.report_orphan_gossip(11, 1);
    m.classify_and_dispatch(0).0
}

// INC-I-294 M-2 (Must) — Decision: a failure means the sync engine never turns a sibling header at tip+1 into proof (a true depth-1 fork cannot heal below the gate), or it reports proof for replies that extend our tip or break only deeper.
#[test]
fn inc_i_294_m2_sibling_header_at_tip_plus_one_reports_header_divergence() {
    let other_parent = crypto::hash::hash(b"inc-i-294 peer 881799");
    let slot = TIP_BELOW as u32 + 1;

    let got = classify_after_replies(|_| vec![vec![signed_header(other_parent, slot)]]);
    assert_eq!(
        got, BLIND,
        "(a) first header breaks at our tip: proof for {TIP_BELOW}"
    );

    let got = classify_after_replies(|local| vec![vec![signed_header(local, slot)]]);
    assert_eq!(got, HFS, "(b) header extends our tip: no proof");

    let got = classify_after_replies(|_| vec![vec![]]);
    assert_eq!(got, HFS, "(c) empty reply: no proof");

    let got = classify_after_replies(|local| {
        vec![vec![
            signed_header(local, slot),
            signed_header(other_parent, slot + 1),
        ]]
    });
    assert_eq!(
        got, HFS,
        "(d) break after one valid header: not proof against our tip"
    );

    let got = classify_after_replies(|local| {
        vec![
            vec![signed_header(local, slot)],
            vec![signed_header(other_parent, slot + 1)],
        ]
    });
    assert_eq!(
        got, HFS,
        "(e) first-header break while expected != local_hash: no proof"
    );
}
