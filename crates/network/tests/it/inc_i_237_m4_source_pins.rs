//! INC-I-237 M4 — source-shape pins: R6 app SeenCache deleted, INV-NETWORK-002
//! queue half derived from the lane budgets, one gossipsub config builder,
//! caller-less seam fns deleted, module budget, no gossipsub type in bins/node.
//!
//! covers: staleness relay_decision config publish event_sink event_lanes swarm_loop swarm_events behaviour_events gossip_intake mod command_handling types
//!
//! OUTPUT CONTRACT: non-test source of crates/network/src/{service,gossip} (+ bins/node/src)
//!   O1: none   O2: none   O3: none   O5: none   O6: none
//!   O4: the source files themselves (read-only scan)
//! PATHS: P1 forbidden token present; P2 forbidden fn definition present;
//!   P3 call-site argument is a literal; P4 file over budget; P5 builder count != 1
//! INPUT PARTITIONS: file kind {non-test .rs, tests.rs / *_tests.rs / tests/ dir};
//!   token position {code line, comment line}; scan target {non-empty, vacuous}
//! MATRIX: every forbidden item x P1-P5 asserted with offending file:line lists;
//!   every scan asserts a non-vacuous target (file count, anchor present).

use std::path::{Path, PathBuf};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo() -> PathBuf {
    crate_dir().join("../..")
}

fn is_test_file(p: &Path) -> bool {
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name == "tests.rs"
        || name.ends_with("_tests.rs")
        || p.components().any(|c| c.as_os_str() == "tests")
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {dir:?}: {e}"));
    for e in entries {
        let p = e.expect("dir entry").path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Offset of the first `#[cfg(test)]` that opens an inline `mod x { .. }` (not `mod x;`).
fn inline_test_mod(src: &str) -> Option<usize> {
    src.match_indices("#[cfg(test)]")
        .map(|(i, _)| i)
        .find(|&i| {
            let decl = src[i..]
                .lines()
                .skip(1)
                .map(str::trim)
                .find(|l| !l.starts_with("#["))
                .unwrap_or("");
            decl.starts_with("mod ") && decl.ends_with('{')
        })
}

fn non_test_sources(rel: &[&str]) -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    for r in rel {
        rs_files(&repo().join(r), &mut files);
    }
    files.sort();
    files
        .into_iter()
        .filter(|p| !is_test_file(p))
        .map(|p| {
            let all = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p:?}: {e}"));
            let cut = inline_test_mod(&all).map_or(all.as_str(), |i| &all[..i]);
            (p, cut.to_string())
        })
        .collect()
}

fn code_lines(src: &str) -> impl Iterator<Item = (usize, &str)> {
    src.lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.trim()))
        .filter(|(_, l)| !l.starts_with("//"))
}

fn has_word(line: &str, word: &str) -> bool {
    line.match_indices(word).any(|(i, _)| {
        let ok = |c: Option<char>| c.is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        ok(line[..i].chars().next_back()) && ok(line[i + word.len()..].chars().next())
    })
}

fn rel(p: &Path) -> String {
    p.strip_prefix(repo()).unwrap_or(p).display().to_string()
}

const NETWORK_SEAM: [&str; 2] = ["crates/network/src/service", "crates/network/src/gossip"];

// REQ-GRV-013 — Decision: a failure means the app SeenCache (R6) or its tx alias still exists in network source, so a second dedup authority can suppress or diverge from the vendor duplicate cache.
#[test]
fn req_grv_013_r6_app_seen_cache_is_gone_from_network_source() {
    let files = non_test_sources(&["crates/network/src"]);
    assert!(files.len() >= 20, "vacuous scan: {} files", files.len());
    let forbidden = [
        "SeenCache",
        "seen_key",
        "classify_transaction_gossip",
        "SEEN_CACHE_TTL_SECS",
        "SEEN_CACHE_CAPACITY",
        "TX_SEEN_TTL_SECS",
    ];
    let mut hits = Vec::new();
    for (p, src) in &files {
        for (n, l) in src.lines().enumerate() {
            for w in forbidden {
                if has_word(l, w) {
                    hits.push(format!("{}:{} {w}", rel(p), n + 1));
                }
            }
        }
    }
    assert!(hits.is_empty(), "R6 leftovers:\n{}", hits.join("\n"));
}

/// Argument text of every `name(` call in `src` (balanced parentheses).
fn call_args(src: &str, name: &str) -> Vec<String> {
    let pat = format!("{name}(");
    let mut out = Vec::new();
    for (i, _) in src.match_indices(&pat) {
        let before = src[..i].trim_end();
        let line_start = src[..i].rfind('\n').map_or(0, |n| n + 1);
        if before.ends_with("fn") || src[line_start..i].trim_start().starts_with("//") {
            continue;
        }
        let mut depth = 1;
        let body = &src[i + pat.len()..];
        let end = body
            .char_indices()
            .find(|&(_, c)| {
                depth += match c {
                    '(' => 1,
                    ')' => -1,
                    _ => 0,
                };
                depth == 0
            })
            .map(|(j, _)| j)
            .expect("balanced call");
        out.push(body[..end].split_whitespace().collect::<Vec<_>>().join(" "));
    }
    out
}

// REQ-GRV-013 — Decision: a failure means INV-NETWORK-002's bounded-queue half is still a hard-coded literal, so an unbounded lane budget would pass the construction-time hardening gate.
#[test]
fn req_grv_013_hardening_gate_reads_the_real_lane_budgets() {
    let mut calls = Vec::new();
    for (p, src) in non_test_sources(&["crates/network/src", "bins/node/src"]) {
        for args in call_args(&src, "assert_gossip_hardening_invariant") {
            calls.push((rel(&p), args));
        }
    }
    assert!(!calls.is_empty(), "no production hardening-gate call site");
    for (file, args) in &calls {
        let second = args
            .split_once(',')
            .map(|(_, s)| s.trim().trim_end_matches(','));
        assert!(
            !matches!(second, Some("true") | Some("false")),
            "{file}: literal queue argument in assert_gossip_hardening_invariant({args})"
        );
        assert!(
            second.is_some_and(|s| s.contains("is_bounded")),
            "{file}: queue argument must come from LaneBudgets::is_bounded(), got ({args})"
        );
    }
}

// REQ-GRV-020 — Decision: a failure means two gossipsub configs still exist, so the test builder can drift from production (it already lacks the topic scores) and tests prove the wrong config.
#[test]
fn req_grv_020_one_gossipsub_config_builder() {
    let path = crate_dir().join("src/gossip/config.rs");
    let src = std::fs::read_to_string(&path).expect("config.rs");
    let count = |tok: &str| code_lines(&src).filter(|(_, l)| l.contains(tok)).count();
    assert_eq!(
        count("ConfigBuilder::default()"),
        1,
        "gossipsub ConfigBuilder sites"
    );
    assert_eq!(count("|message: &Message|"), 1, "message-id fn definitions");
}

// REQ-GRV-013 — Decision: a failure means the production gossipsub no longer takes its duplicate-cache lifetime from GOSSIP_DUPLICATE_CACHE_TIME, so the R6 dominance pin no longer guards the running config.
#[test]
fn req_grv_013_production_gossipsub_wires_the_duplicate_cache_constant() {
    let src = std::fs::read_to_string(crate_dir().join("src/gossip/config.rs")).expect("config.rs");
    let start = src
        .find("pub fn new_gossipsub(")
        .expect("new_gossipsub exists");
    let body = &src[start..];
    let end = body.find("\n}\n").expect("fn end");
    assert!(
        body[..end].contains("GOSSIP_DUPLICATE_CACHE_TIME"),
        "new_gossipsub must pass GOSSIP_DUPLICATE_CACHE_TIME (directly or via its one builder)"
    );
    assert!(
        code_lines(&src).any(|(_, l)| l.contains(".duplicate_cache_time(")),
        "the builder sets duplicate_cache_time"
    );
}

const CALLER_LESS: [&str; 10] = [
    "broadcast_transaction",
    "broadcast_producers",
    "send_producer_delta",
    "get_peer",
    "get_peers",
    "publish_block",
    "publish_transaction",
    "publish_producer",
    "publish_vote",
    "publish_attestation",
];

// REQ-GRV-020 — Decision: a failure means a seam fn with no production caller survives, so a future caller can revive an unjudged publish path (broadcast_transaction re-publish) or a deprecated wire format.
#[test]
fn req_grv_020_caller_less_seam_fns_are_deleted() {
    let files = non_test_sources(&NETWORK_SEAM);
    assert!(files.len() >= 15, "vacuous scan: {} files", files.len());
    let mut hits = Vec::new();
    for (p, src) in &files {
        for (n, l) in code_lines(src) {
            for name in CALLER_LESS {
                if has_word(l, &format!("fn {name}")) {
                    hits.push(format!("{}:{n} {name}", rel(p)));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "caller-less fns still defined:\n{}",
        hits.join("\n")
    );
}

// REQ-GRV-021 — Decision: a failure means a seam module exceeds the 500-line budget (800 for test files), so the next change lands in an oversize file that reviewers cannot hold.
#[test]
fn req_grv_021_seam_files_within_module_budget() {
    let mut files = Vec::new();
    for r in NETWORK_SEAM {
        rs_files(&repo().join(r), &mut files);
    }
    assert!(files.len() >= 20, "vacuous scan: {} files", files.len());
    let mut over = Vec::new();
    for p in &files {
        let lines = std::fs::read_to_string(p).expect("read").lines().count();
        let cap = if is_test_file(p) { 800 } else { 500 };
        if lines > cap {
            over.push(format!("{} {lines} > {cap}", rel(p)));
        }
    }
    assert!(over.is_empty(), "over budget:\n{}", over.join("\n"));
}

// REQ-GRV-018 — Decision: a failure means a gossipsub type crossed into bins/node, so a libp2p-gossipsub >= 0.48 upgrade (W06) would need node rework.
#[test]
fn req_grv_018_no_gossipsub_type_in_node_source() {
    let files = non_test_sources(&["bins/node/src"]);
    assert!(files.len() >= 50, "vacuous scan: {} files", files.len());
    let anchor = files.iter().any(|(_, s)| s.contains("gossipsub"));
    assert!(
        anchor,
        "scanner sees no 'gossipsub' text at all — wrong root?"
    );
    let hits: Vec<String> = files
        .iter()
        .flat_map(|(p, src)| {
            code_lines(src)
                .filter(|(_, l)| l.contains("gossipsub::") || l.contains("libp2p::gossipsub"))
                .map(|(n, l)| format!("{}:{n} {l}", rel(p)))
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        hits.is_empty(),
        "gossipsub types in bins/node:\n{}",
        hits.join("\n")
    );
}
