//! INC-I-237 M5 (AUDIT-P3-417) — the gossip-relay-verdict specs and the
//! redesign analysis name no production host, host log file, log-rotation
//! state or external-producer count (project rule; two prior incidents).
//!
//! OUTPUT CONTRACT: specs/gossip-relay-verdict-*.md +
//!   docs/redesigns/gossip-relay-verdict-redesign-analysis.md (read-only scan);
//!   contract doc §1i.  O4: document text; O1-O3, O5, O6: none
//! PATHS: P1 forbidden token present -> fail with file:line; P2 vacuous read
//! INPUT PARTITIONS: token {host alias word, host log file, log-rotation word,
//!   external-producer count}; case {any}; planted lines {each token, each
//!   approved replacement}
//! The guard stores no operational fact: it matches generic patterns only.
//! MATRIX: every document x every token, plus the planted falsifier rows.

use std::path::PathBuf;

/// Any mention of log rotation in these documents (the words alone carry no
/// operational state; the documents have no reason to discuss it).
const LOG_ROTATION_WORDS: [&str; 2] = ["logrotate", "log rotation"];

/// A number directly followed by " external" (an external-producer count).
fn external_count(l: &str) -> bool {
    l.match_indices(" external").any(|(i, _)| {
        l[..i]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_digit())
    })
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn targets() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(root().join("specs"))
        .expect("specs/")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("gossip-relay-verdict-") && n.ends_with(".md"))
        })
        .collect();
    v.sort();
    assert!(
        v.len() >= 2,
        "vacuous: gossip-relay-verdict specs not found: {v:?}"
    );
    v.push(root().join("docs/redesigns/gossip-relay-verdict-redesign-analysis.md"));
    v
}

/// Host alias: the two-letter prefix plus one digit 1-5, as a whole word.
fn host_word(l: &str) -> bool {
    let b = l.as_bytes();
    (0..b.len().saturating_sub(2)).any(|i| {
        b[i] == b'a'
            && b[i + 1] == b'i'
            && (b'1'..=b'5').contains(&b[i + 2])
            && (i == 0 || !b[i - 1].is_ascii_alphanumeric())
            && b.get(i + 3).is_none_or(|c| !c.is_ascii_alphanumeric())
    })
}

/// `n<digits>.log`.
fn node_log(l: &str) -> bool {
    l.match_indices(".log").any(|(i, _)| {
        let head = &l[..i];
        let stem = head.trim_end_matches(|c: char| c.is_ascii_digit());
        stem.len() < head.len() && stem.ends_with('n')
    })
}

fn forbidden(line: &str) -> Option<String> {
    let l = line.to_lowercase();
    if host_word(&l) {
        return Some("host alias".into());
    }
    if node_log(&l) {
        return Some("host log file".into());
    }
    if external_count(&l) {
        return Some("external-producer count".into());
    }
    LOG_ROTATION_WORDS
        .iter()
        .find(|w| l.contains(*w))
        .map(|w| format!("log-rotation word `{w}`"))
}

// REQ-GRV-019 (AUDIT-P3-417) — Decision: a failure means a spec about to be pushed names production hosts, host log files, log-rotation state or fleet counts, the disclosure the project rule forbids.
#[test]
fn m5_audit_p3_417_gossip_relay_docs_disclose_no_host_or_fleet_detail() {
    let mut planted: Vec<String> = LOG_ROTATION_WORDS
        .iter()
        .map(|w| format!("x {} y", w.to_uppercase()))
        .collect();
    let host = format!("{}{}", "ai", 4);
    let log = format!("{}{}.log", 'n', 7);
    let count = format!("{}{}", 1, 2);
    planted.push(format!("about {count} external producers"));
    planted.push(format!("see {host} z"));
    planted.push(format!("[E8] {log}:14"));
    for planted in &planted {
        assert!(forbidden(planted).is_some(), "guard misses a planted row");
    }
    for approved in [
        "rolling-deploy safe with external producers on mixed versions",
        "production host density, swap and MemoryMax",
        "[E8] production node log (RUN 570 evidence)",
        "tai3 value",
        "events.log",
    ] {
        assert_eq!(
            forbidden(approved),
            None,
            "guard flags approved `{approved}`"
        );
    }
    let mut hits = Vec::new();
    for p in targets() {
        let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p:?}: {e}"));
        assert!(text.lines().count() > 10, "{p:?}: vacuous document");
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        for (n, l) in text.lines().enumerate() {
            if let Some(what) = forbidden(l) {
                hits.push(format!(
                    "{name}:{}: [{what}] {}",
                    n + 1,
                    l.chars().take(120).collect::<String>()
                ));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "AUDIT-P3-417 disclosure:\n{}",
        hits.join("\n")
    );
}
