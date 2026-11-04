// covers: manager/recovery.rs (evidence_summary) manager/block_lifecycle.rs (classify_and_dispatch [COORDINATOR] line)
//! INC-I-294 M-4: the coordinator renders its live evidence per kind (count, min/max age,
//! expired count) and the `[COORDINATOR]` INFO line ends with that summary.
//
// OUTPUT CONTRACT: RecoveryCoordinator::evidence_summary(&self, now: Instant) -> String (O3 only, &self)
//   SyncManager::classify_and_dispatch(&mut self, u32) -> O6 INFO `[COORDINATOR]` event text
// PATHS: S1 mixed live + expired | S2 every kind live once + TTL boundary | S3 classify_and_dispatch
//   with action != None (HeaderFirstSync above the mainnet inc_i_288 gate)
// MATRIX
//   S1 -> {orphan:2 age=5s..40s, stuck_fork:1 age=12s, header_proof:1 age=3s, expired:3}; expired last;
//         expired kinds (apply_fail, empty_headers) absent from the kinds list
//   S2 -> six snake_case kinds, `kind:1 age=Ns`; age 120 s live (stale_tip), 121 s expired
//   S1,S2 -> no Rust variant name in the text
//   S3 -> line = byte-identical old prefix + " ev=[orphan:2 age=0s..."; ends with ']'
// INPUT PARTITIONS: ages min == max | min < max; age == EVIDENCE_TTL | > EVIDENCE_TTL;
//   expired entry of a kind that also has live entries (orphan) | expired-only kind

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crypto::Hash;
use doli_core::{Network, NetworkParams};
use libp2p::PeerId;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;

use super::recovery::{RecoveryAction, RecoveryCoordinator, RecoveryEvidence};
use crate::sync::manager::{RecoveryPhase, SyncConfig, SyncManager};

const VARIANTS: [&str; 7] = [
    "StuckFork",
    "ShallowRollback",
    "OrphanGossip",
    "EmptyHeaders",
    "HeaderDivergence",
    "ApplyFailure",
    "StaleTip",
];

fn s(secs: u64) -> Duration {
    Duration::from_secs(secs)
}

fn orphan() -> RecoveryEvidence {
    RecoveryEvidence::OrphanGossip { slot: 11, gap: 1 }
}

fn proof() -> RecoveryEvidence {
    RecoveryEvidence::HeaderDivergence {
        peer: PeerId::random(),
        tip_height: 10,
        tip_hash: Hash::ZERO,
    }
}

/// Items between `ev=[` and `]`, in order.
fn items(summary: &str) -> Vec<String> {
    let inner = summary
        .strip_prefix("ev=[")
        .and_then(|r| r.strip_suffix(']'))
        .unwrap_or_else(|| panic!("summary must be `ev=[...]`; got {summary:?}"));
    inner.split(", ").map(str::to_string).collect()
}

fn no_variant_names(text: &str) {
    for v in VARIANTS {
        assert!(
            !text.contains(v),
            "summary must not carry Rust name {v}; got {text:?}"
        );
    }
}

// INC-I-294 M-4 (Must) — Decision: a failure means a rollback log still cannot show which evidence (kind, count, age) drove it, so a one-peer stale-evidence trigger stays hidden.
#[test]
fn inc_i_294_m4_summary_counts_live_kinds_with_age_bounds_and_expired() {
    let t0 = Instant::now();
    let mut c = RecoveryCoordinator::new();
    c.report_at(t0, RecoveryEvidence::ApplyFailure { height: 9 });
    c.report_at(
        t0 + s(10),
        RecoveryEvidence::EmptyHeaders {
            peer: PeerId::random(),
            gap: 1,
        },
    );
    c.report_at(t0 + s(60), orphan());
    c.report_at(t0 + s(160), orphan());
    c.report_at(t0 + s(188), RecoveryEvidence::StuckFork { gap: 1 });
    c.report_at(t0 + s(195), orphan());
    c.report_at(t0 + s(197), proof());
    let summary = c.evidence_summary(t0 + s(200));
    let got = items(&summary);
    let want: BTreeSet<String> = [
        "orphan:2 age=5s..40s",
        "stuck_fork:1 age=12s",
        "header_proof:1 age=3s",
        "expired:3",
    ]
    .iter()
    .map(|x| x.to_string())
    .collect();
    assert_eq!(
        got.iter().cloned().collect::<BTreeSet<_>>(),
        want,
        "summary {summary:?}"
    );
    assert_eq!(got.len(), want.len(), "no duplicate items: {summary:?}");
    assert_eq!(
        got.last().map(String::as_str),
        Some("expired:3"),
        "expired last: {summary:?}"
    );
    no_variant_names(&summary);
}

// INC-I-294 M-4 (Must) — Decision: a failure means a kind is mislabelled or the TTL edge disagrees with classify's `live` (120 s live, 121 s expired), so the log would contradict the decision it explains.
#[test]
fn inc_i_294_m4_summary_names_every_kind_snake_case_and_honours_ttl_edge() {
    let t0 = Instant::now();
    let now = t0 + s(500);
    let mut c = RecoveryCoordinator::new();
    c.report_at(now - s(121), RecoveryEvidence::StuckFork { gap: 2 });
    c.report_at(
        now - s(120),
        RecoveryEvidence::StaleTip {
            last_applied_secs: 40,
            gap: 2,
        },
    );
    c.report_at(now - s(5), proof());
    c.report_at(now - s(4), RecoveryEvidence::StuckFork { gap: 2 });
    c.report_at(now - s(3), RecoveryEvidence::ApplyFailure { height: 7 });
    c.report_at(now - s(2), orphan());
    c.report_at(
        now - s(1),
        RecoveryEvidence::EmptyHeaders {
            peer: PeerId::random(),
            gap: 2,
        },
    );
    let summary = c.evidence_summary(now);
    let got: BTreeSet<String> = items(&summary).into_iter().collect();
    let want: BTreeSet<String> = [
        "empty_headers:1 age=1s",
        "orphan:1 age=2s",
        "apply_fail:1 age=3s",
        "stale_tip:1 age=120s",
        "stuck_fork:1 age=4s",
        "header_proof:1 age=5s",
        "expired:1",
    ]
    .iter()
    .map(|x| x.to_string())
    .collect();
    assert_eq!(got, want, "summary {summary:?}");
    no_variant_names(&summary);
}

/// A second live dispatcher keeps tracing-core out of its one-dispatcher mode, where an
/// uncaptured thread's first hit of a callsite caches `never` for every thread.
fn keep_interest_rebuilt() {
    static KEEP: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();
    KEEP.get_or_init(|| tracing::Dispatch::new(tracing_subscriber::registry()));
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<String>>>);

struct Msg(String);

impl tracing::field::Visit for Msg {
    fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
        if f.name() == "message" {
            self.0 = format!("{v:?}");
        }
    }
}

impl<S: tracing::Subscriber> Layer<S> for Logs {
    fn on_event(&self, e: &tracing::Event<'_>, _: Context<'_, S>) {
        if *e.metadata().level() > tracing::Level::INFO {
            return;
        }
        let mut m = Msg(String::new());
        e.record(&mut m);
        self.0.lock().unwrap().push(m.0);
    }
}

// INC-I-294 M-4 (Must) — Decision: a failure means the [COORDINATOR] line lost its byte-identical prefix (gauntlet-gs024 and the troubleshooting guide grep it) or still carries no evidence.
#[test]
fn inc_i_294_m4_coordinator_line_keeps_prefix_and_ends_with_summary() {
    let p = NetworkParams::defaults(Network::Mainnet);
    let gate = p.inc_i_288_fork_choice_activation_height;
    let config = SyncConfig {
        inc_i_147_activation_height: p.inc_i_147_activation_height,
        inc_i_204_fork_choice_activation_height: p.inc_i_204_fork_choice_activation_height,
        inc_i_288_fork_choice_activation_height: gate,
        ..SyncConfig::default()
    };
    let mut m = SyncManager::new(config, Hash::ZERO);
    m.update_local_tip(gate, crypto::hash::hash(b"m4 tip"), 10);
    m.recovery_phase = RecoveryPhase::Normal;
    m.add_peer(
        PeerId::random(),
        gate + 1,
        crypto::hash::hash(b"m4 peer"),
        11,
    );
    m.report_orphan_gossip(11, 1);
    m.report_orphan_gossip(11, 1);
    let logs = Logs::default();
    let action = {
        keep_interest_rebuilt();
        let _g =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
        m.classify_and_dispatch(0).0
    };
    assert_eq!(
        action,
        RecoveryAction::HeaderFirstSync,
        "FIXTURE: an action is logged"
    );
    let lines: Vec<String> = logs
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|l| l.contains("[COORDINATOR]"))
        .cloned()
        .collect();
    assert_eq!(
        lines.len(),
        1,
        "FIXTURE: one [COORDINATOR] INFO line; got {lines:?}"
    );
    let line = &lines[0];
    let rest = line
        .strip_prefix("[COORDINATOR] action=")
        .unwrap_or_else(|| panic!("prefix changed: {line:?}"));
    let name: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    let after = &rest[name.len()..];
    assert!(
        !name.is_empty()
            && after.starts_with(" gap=")
            && after[5..].starts_with(|c: char| c.is_ascii_digit()),
        "prefix must match ^\\[COORDINATOR\\] action=[A-Za-z]+ gap=[0-9]+; got {line:?}"
    );
    assert!(
        line.starts_with("[COORDINATOR] action=HeaderFirstSync gap=1 last_applied="),
        "prefix head byte-identical: {line:?}"
    );
    assert!(
        line.contains("s peers=1 shallow_rb=0 last_rb_h=None snap_attempts=0 grace=false ev=["),
        "summary appended after `grace=false` with one space: {line:?}"
    );
    let ev = &line[line.find(" ev=[").expect("ev=[") + 1..];
    assert!(ev.ends_with(']'), "the summary ends the line: {line:?}");
    assert!(
        ev.starts_with("ev=[orphan:2 age=0s"),
        "live orphans rendered: {line:?}"
    );
    no_variant_names(ev);
}
