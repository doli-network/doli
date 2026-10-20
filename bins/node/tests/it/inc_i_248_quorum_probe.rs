//! INC-I-248 M1 — the maintainer quorum probe state machine (collect, evaluate, retry, stop).
//!
//! REQ-248-002 (Must) — Decision: a failure means a snap-joined node never evaluates its answers, retries forever, keeps collecting after a final verdict, or accepts frames it never asked for.
//!
//! OUTPUT CONTRACT: MaintainerQuorumProbe::{new, arm, record, on_tick, on_verdict}
//!   O2 receiver: phase_name(), attempts(), is_collecting(), answers()
//!   O3 return: record -> bool; on_tick -> ProbeStep
//!   O1/O4/O5/O6: none
//! PATHS: idle; collecting (record accept / same-peer replace, tick Wait | Evaluate);
//!   transient verdict (NoAnswers | NoQuorum | Tie | NoMajority) -> retry-wait (tick Wait | Rearm)
//!   or gave-up at MAX attempts; final verdict (Adopt | NotEligible) -> done (REL-M1).
//! INPUT PARTITIONS: elapsed 0 | WINDOW-1s | WINDOW | RETRY-1s | RETRY; attempts 1 | 2 | MAX;
//!   same peer twice | distinct peers.
//! MATRIX: 5 phases x (phase_name, attempts, is_collecting, answers, record, on_tick) = 30 cells.
//!
// covers: crates/core/src/maintainer/quorum.rs, crates/core/src/maintainer/mod.rs, bins/node/src/node/maintainer_quorum.rs

use std::time::{Duration, Instant};

use doli_core::maintainer::{KeepReason, PeerMaintainerAnswer, QuorumVerdict};
use doli_core::MaintainerSet;
use doli_node::node::maintainer_quorum::{
    MaintainerQuorumProbe, ProbeReason, ProbeStep, MAINTAINER_QUORUM_MAX_ATTEMPTS,
    MAINTAINER_QUORUM_RETRY_SECS, MAINTAINER_QUORUM_WINDOW_SECS,
};
use network::PeerId;

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

fn answer(tip_height: u64) -> PeerMaintainerAnswer {
    let set = MaintainerSet::with_members(
        vec![*crypto::KeyPair::from_seed([101; 32]).public_key()],
        tip_height,
    );
    PeerMaintainerAnswer {
        set,
        last_derived_height: tip_height,
        digest: [tip_height as u8; 32],
        tip_height,
    }
}

fn transient_verdicts() -> Vec<QuorumVerdict> {
    vec![
        QuorumVerdict::Keep(KeepReason::NoAnswers),
        QuorumVerdict::Keep(KeepReason::NoQuorum {
            largest: 2,
            quorum: 3,
        }),
        QuorumVerdict::Keep(KeepReason::Tie),
        QuorumVerdict::Keep(KeepReason::NoMajority {
            largest: 3,
            counted: 7,
        }),
    ]
}

fn final_verdicts() -> Vec<QuorumVerdict> {
    vec![
        QuorumVerdict::Adopt {
            set: answer(7).set,
            agreeing: 3,
        },
        QuorumVerdict::Keep(KeepReason::NotEligible),
    ]
}

fn armed(now: Instant) -> MaintainerQuorumProbe {
    let mut p = MaintainerQuorumProbe::new();
    p.arm(ProbeReason::PostSnapInstall, now);
    p
}

#[test]
fn inc_i_248_probe_constants_pinned() {
    assert_eq!(MAINTAINER_QUORUM_WINDOW_SECS, 15);
    assert_eq!(MAINTAINER_QUORUM_RETRY_SECS, 60);
    assert_eq!(MAINTAINER_QUORUM_MAX_ATTEMPTS, 3);
}

#[test]
fn inc_i_248_probe_idle_tick_is_idle() {
    let now = Instant::now();
    let mut p = MaintainerQuorumProbe::new();
    assert_eq!(p.phase_name(), "idle");
    assert_eq!(p.attempts(), 0);
    assert!(!p.is_collecting());
    assert!(p.answers().is_empty());
    assert_eq!(p.on_tick(now), ProbeStep::Idle);
    assert_eq!(p.on_tick(now + secs(3_600)), ProbeStep::Idle);
    assert_eq!(p.phase_name(), "idle");
    let d = MaintainerQuorumProbe::default();
    assert_eq!(d.phase_name(), "idle");
    assert_eq!(d.attempts(), 0);
}

#[test]
fn inc_i_248_probe_record_while_idle_dropped() {
    let mut p = MaintainerQuorumProbe::new();
    assert!(!p.record(PeerId::random(), answer(1)));
    assert!(p.answers().is_empty());
    assert_eq!(p.phase_name(), "idle");
}

#[test]
fn inc_i_248_probe_arm_starts_collecting() {
    let p = armed(Instant::now());
    assert_eq!(p.phase_name(), "collecting");
    assert_eq!(p.attempts(), 1);
    assert!(p.is_collecting());
    assert!(p.answers().is_empty());
}

#[test]
fn inc_i_248_probe_record_while_collecting_same_peer_replaces() {
    let mut p = armed(Instant::now());
    let (p1, p2) = (PeerId::random(), PeerId::random());
    assert!(p.record(p1, answer(1)));
    assert!(p.record(p2, answer(2)));
    assert_eq!(p.answers().len(), 2);
    assert!(p.record(p1, answer(3)));
    let got = p.answers();
    assert_eq!(
        got.len(),
        2,
        "a second answer from the same peer must replace the first"
    );
    assert!(got.contains(&answer(3)));
    assert!(got.contains(&answer(2)));
    assert!(!got.contains(&answer(1)));
}

#[test]
fn inc_i_248_probe_window_wait_then_evaluate() {
    let t0 = Instant::now();
    let mut p = armed(t0);
    assert_eq!(p.on_tick(t0), ProbeStep::Wait);
    assert_eq!(
        p.on_tick(t0 + secs(MAINTAINER_QUORUM_WINDOW_SECS - 1)),
        ProbeStep::Wait
    );
    assert_eq!(
        p.on_tick(t0 + secs(MAINTAINER_QUORUM_WINDOW_SECS)),
        ProbeStep::Evaluate
    );
    assert_eq!(p.attempts(), 1);
}

#[test]
fn inc_i_248_probe_transient_verdict_retries_then_rearms() {
    for verdict in transient_verdicts() {
        let t0 = Instant::now();
        let mut p = armed(t0);
        assert!(p.record(PeerId::random(), answer(1)));
        let t1 = t0 + secs(MAINTAINER_QUORUM_WINDOW_SECS);
        p.on_verdict(&verdict, t1);
        assert_eq!(p.phase_name(), "retry-wait", "{verdict:?}");
        assert_eq!(p.attempts(), 1, "{verdict:?}");
        assert!(!p.is_collecting(), "{verdict:?}");
        assert!(
            !p.record(PeerId::random(), answer(2)),
            "{verdict:?}: record while retry-wait"
        );

        assert_eq!(
            p.on_tick(t1 + secs(MAINTAINER_QUORUM_RETRY_SECS - 1)),
            ProbeStep::Wait,
            "{verdict:?}"
        );
        assert_eq!(
            p.on_tick(t1 + secs(MAINTAINER_QUORUM_RETRY_SECS)),
            ProbeStep::Rearm,
            "{verdict:?}"
        );
        assert_eq!(
            p.phase_name(),
            "retry-wait",
            "{verdict:?}: Rearm must not re-arm by itself"
        );

        let t2 = t1 + secs(MAINTAINER_QUORUM_RETRY_SECS);
        p.arm(ProbeReason::PostSnapInstall, t2);
        assert_eq!(p.phase_name(), "collecting", "{verdict:?}");
        assert_eq!(p.attempts(), 2, "{verdict:?}");
        assert!(
            p.answers().is_empty(),
            "{verdict:?}: arm must clear answers"
        );
        assert_eq!(
            p.on_tick(t2),
            ProbeStep::Wait,
            "{verdict:?}: window restarts at arm"
        );
    }
}

#[test]
fn inc_i_248_probe_gives_up_after_max_attempts() {
    for verdict in transient_verdicts() {
        let mut now = Instant::now();
        let mut p = MaintainerQuorumProbe::new();
        for attempt in 1..=MAINTAINER_QUORUM_MAX_ATTEMPTS {
            p.arm(ProbeReason::Startup, now);
            assert_eq!(p.attempts(), attempt, "{verdict:?}");
            now += secs(MAINTAINER_QUORUM_WINDOW_SECS);
            p.on_verdict(&verdict, now);
            let expected = if attempt < MAINTAINER_QUORUM_MAX_ATTEMPTS {
                "retry-wait"
            } else {
                "gave-up"
            };
            assert_eq!(p.phase_name(), expected, "{verdict:?} attempt {attempt}");
            now += secs(MAINTAINER_QUORUM_RETRY_SECS);
        }
        assert_eq!(p.on_tick(now), ProbeStep::Idle, "{verdict:?}");
        assert_eq!(p.on_tick(now + secs(3_600)), ProbeStep::Idle, "{verdict:?}");
        assert!(!p.is_collecting(), "{verdict:?}");
        assert!(!p.record(PeerId::random(), answer(1)), "{verdict:?}");
        assert_eq!(p.attempts(), MAINTAINER_QUORUM_MAX_ATTEMPTS, "{verdict:?}");
    }
}

#[test]
fn inc_i_248_probe_final_verdict_done() {
    for verdict in final_verdicts() {
        let t0 = Instant::now();
        let mut p = armed(t0);
        assert!(p.record(PeerId::random(), answer(1)));
        let t1 = t0 + secs(1);
        p.on_verdict(&verdict, t1);
        assert_eq!(p.phase_name(), "done", "{verdict:?}");
        assert_eq!(p.attempts(), 1, "{verdict:?}");
        assert!(!p.is_collecting(), "{verdict:?}");
        assert!(
            !p.record(PeerId::random(), answer(2)),
            "{verdict:?}: record after done"
        );
        assert_eq!(p.on_tick(t1), ProbeStep::Idle, "{verdict:?}");
        assert_eq!(
            p.on_tick(t1 + secs(2 * MAINTAINER_QUORUM_RETRY_SECS)),
            ProbeStep::Idle,
            "{verdict:?}: done never re-arms"
        );
        assert_eq!(p.phase_name(), "done", "{verdict:?}");
    }
}
