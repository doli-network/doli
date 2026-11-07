//! INC-I-295: every read of peer height, network tip or peer count on the production
//! path is named in an allow-list with an exact count. Source reads only; no node.
//!
//! covers: production_gate gates scheduling production_asks
//
// OUTPUT CONTRACT: fn scan_reads(source: &str) -> Vec<Read>
//   O3: return — one (enclosing fn, token) per token occurrence in code text
//   O1, O2, O4..O7: none (pure)
// OUTPUT CONTRACT: fn compare(found: &Counts, listed: &Counts) -> Vec<Drift>
//   O3: return — one Drift per (file, fn, token) whose count differs from the list
// PATHS: P1 equal -> empty | P2 more reads than listed -> NEW READ
//   P3 fewer reads than listed -> DROPPED READ | P4 comment text -> not a read
// INPUT PARTITIONS:
//   A: the real sources                         -> P1
//   B: one more read in an allowed fn           -> P2
//   C: one read in a new fn, for each token     -> P2
//   D: one read in a new file                   -> P2
//   E: an allowed read removed                  -> P3
//   F: a read in a `//` line, `///` line, trailing text -> P4
//   G: a `//` inside a string, then a read      -> P2
// MATRIX: A in `production_stop_reads_match_the_allow_list`; B..G in
//   `census_scanner_flags_an_injected_line`, each by the rendered text of `compare`.
// REQ-PASK-SEC-003 — Decision: a grep hit in the names and messages of this file would reveal a public-detail leak; checked by grep, no runtime test.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const PRODUCTION_DIR: &str = "bins/node/src/node/production";
const GATE_FILE: &str = "crates/network/src/sync/manager/production_gate.rs";
const GATES: &str = "bins/node/src/node/production/gates.rs";
const MOD: &str = "bins/node/src/node/production/mod.rs";
const ASSEMBLY: &str = "bins/node/src/node/production/assembly.rs";
const SCHEDULING: &str = "bins/node/src/node/production/scheduling.rs";
const OLD_SLOT_READ: &str = "has_block_for_slot";

const TOKENS: [&str; 6] = [
    "best_peer_height",
    "network_tip",
    "peer_count",
    "peers.len",
    "peers.is_empty",
    ".best_height",
];

const PEER_HEIGHT: &str = TOKENS[0];
const NET_TIP: &str = TOKENS[1];
const PEER_COUNT: &str = TOKENS[2];
const PEERS_LEN: &str = TOKENS[3];
const PEERS_EMPTY: &str = TOKENS[4];
const BEST_HEIGHT: &str = TOKENS[5];

const CAN_PRODUCE: &str = "can_produce";
const HEIGHT_RULE: &str = "peer_height_gate";
const TRY_PRODUCE: &str = "try_produce_block";
const BOOTSTRAP: &str = "resolve_bootstrap_eligibility";

type Allowed = (&'static str, &'static str, &'static str, usize);

const ALLOW_LIST: &[Allowed] = &[
    (ASSEMBLY, "drain_pending_events", BEST_HEIGHT, 1),
    (GATES, "handle_production_authorization", PEER_COUNT, 2),
    (GATES, HEIGHT_RULE, PEER_HEIGHT, 2),
    (MOD, TRY_PRODUCE, BEST_HEIGHT, 4),
    (MOD, TRY_PRODUCE, PEER_HEIGHT, 1),
    (MOD, TRY_PRODUCE, NET_TIP, 9),
    (SCHEDULING, BOOTSTRAP, PEER_HEIGHT, 9),
    (SCHEDULING, BOOTSTRAP, PEER_COUNT, 10),
    (GATE_FILE, CAN_PRODUCE, BEST_HEIGHT, 2),
    (GATE_FILE, CAN_PRODUCE, PEER_HEIGHT, 1),
    (GATE_FILE, CAN_PRODUCE, NET_TIP, 3),
    (GATE_FILE, CAN_PRODUCE, PEER_COUNT, 1),
    (GATE_FILE, CAN_PRODUCE, PEERS_EMPTY, 1),
    (GATE_FILE, CAN_PRODUCE, PEERS_LEN, 6),
];

/// The functions in `production/` that may read peer height, each with its reason.
const PEER_HEIGHT_READERS: [(&str, &str); 3] = [
    (HEIGHT_RULE, "the pure rule; the value is its input"),
    (TRY_PRODUCE, "the one caller of the pure rule"),
    (
        BOOTSTRAP,
        "reached on testnet and devnet only while `in_genesis` or the active list is empty",
    ),
];

/// Files that must exist at every tag, then files that a tag may not have.
const STALE_TEXT_REQUIRED: [&str; 2] = [
    ".claude/skills/node/SKILL.md",
    "specs/attestation-bls-architecture.md",
];
const STALE_TEXT_OPTIONAL: [&str; 5] = [
    "specs/protocol.md",
    "specs/engine-parts.md",
    "docs/architecture.md",
    "docs/troubleshooting.md",
    "docs/cli.md",
];
const REMOVED_GATE_NAME: &str = "BEHIND_TIP_SUPPRESS";
const REMOVED_GATE_CLAIM: &str = "silently discarded";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Read {
    func: String,
    token: &'static str,
}

/// (file, fn, token) -> reads.
type Counts = BTreeMap<(String, String, String), usize>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Drift {
    file: String,
    func: String,
    token: String,
    listed: usize,
    found: usize,
}

impl Drift {
    fn render(&self) -> String {
        let kind = if self.found > self.listed {
            "NEW READ"
        } else {
            "DROPPED READ"
        };
        format!(
            "{kind}: {} fn {} token `{}`: listed {}, found {}",
            self.file, self.func, self.token, self.listed, self.found
        )
    }
}

/// Splits one source line: code with string text, and code with string text removed.
/// `//` starts a comment only outside a string. `in_string` carries over lines.
fn split_line(line: &str, in_string: &mut bool) -> (String, String) {
    let chars: Vec<char> = line.chars().collect();
    let (mut code, mut bare) = (String::new(), String::new());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if *in_string {
            code.push(c);
            if c == '\\' {
                if let Some(next) = chars.get(i + 1) {
                    code.push(*next);
                }
                i += 1;
            } else if c == '"' {
                *in_string = false;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            break;
        } else if c == '\'' && chars.get(i + 2) == Some(&'\'') {
            code.extend(&chars[i..=i + 2]);
            i += 2;
        } else if c == '"' {
            code.push(c);
            *in_string = true;
        } else {
            code.push(c);
            bare.push(c);
        }
        i += 1;
    }
    (code, bare)
}

/// The text of `source` with every comment removed, line by line.
fn code_only(source: &str) -> String {
    let mut in_string = false;
    source
        .lines()
        .map(|l| split_line(l, &mut in_string).0)
        .collect::<Vec<_>>()
        .join("\n")
}

fn fn_name(bare: &str) -> Option<String> {
    let mut rest = bare;
    while let Some(at) = rest.find("fn ") {
        let before = rest[..at].chars().next_back();
        let after = &rest[at + 3..];
        if !before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            let name: String = after
                .trim_start()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                return Some(name);
            }
        }
        rest = after;
    }
    None
}

/// One `Read` per token occurrence on a non-comment line, named by its enclosing fn.
/// A read in a signature belongs to that fn; a read outside any fn is `<no fn>`.
fn scan_reads(source: &str) -> Vec<Read> {
    let mut reads = Vec::new();
    let mut in_string = false;
    let mut open: Vec<(String, i64)> = Vec::new();
    let mut pending: Option<String> = None;
    let (mut depth, mut nest) = (0i64, 0i64);
    for line in source.lines() {
        let (code, bare) = split_line(line, &mut in_string);
        if let Some(name) = fn_name(&bare) {
            pending = Some(name);
        }
        let func = pending
            .clone()
            .or_else(|| open.last().map(|(name, _)| name.clone()))
            .unwrap_or_else(|| "<no fn>".to_string());
        for token in TOKENS {
            for _ in code.matches(token) {
                reads.push(Read {
                    func: func.clone(),
                    token,
                });
            }
        }
        for c in bare.chars() {
            match c {
                '(' | '[' => nest += 1,
                ')' | ']' => nest -= 1,
                ';' if nest == 0 => pending = None,
                '{' => {
                    if let Some(name) = pending.take() {
                        open.push((name, depth));
                    }
                    depth += 1;
                }
                '}' => {
                    depth -= 1;
                    if open.last().is_some_and(|(_, d)| *d == depth) {
                        open.pop();
                    }
                }
                _ => {}
            }
        }
    }
    reads
}

fn count_reads(sources: &[(String, String)]) -> Counts {
    let mut counts = Counts::new();
    for (file, text) in sources {
        for read in scan_reads(text) {
            let key = (file.clone(), read.func, read.token.to_string());
            *counts.entry(key).or_insert(0) += 1;
        }
    }
    counts
}

fn listed_counts(allowed: &[Allowed]) -> Counts {
    let mut listed = Counts::new();
    for (file, func, token, count) in allowed {
        let key = (file.to_string(), func.to_string(), token.to_string());
        *listed.entry(key).or_insert(0) += count;
    }
    listed
}

fn compare(found: &Counts, listed: &Counts) -> Vec<Drift> {
    let mut keys: Vec<_> = found.keys().chain(listed.keys()).cloned().collect();
    keys.sort();
    keys.dedup();
    keys.into_iter()
        .filter_map(|key| {
            let found = found.get(&key).copied().unwrap_or(0);
            let listed = listed.get(&key).copied().unwrap_or(0);
            let (file, func, token) = key;
            (found != listed).then_some(Drift {
                file,
                func,
                token,
                listed,
                found,
            })
        })
        .collect()
}

fn render_all(drift: &[Drift]) -> String {
    drift
        .iter()
        .map(Drift::render)
        .collect::<Vec<_>>()
        .join("\n")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read_repo(rel: &str) -> String {
    fs::read_to_string(repo_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn walk_rs(dir: &Path, rel: &str, out: &mut Vec<String>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read dir {rel}: {e}"));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        let child = format!("{rel}/{name}");
        if entry.path().is_dir() {
            walk_rs(&entry.path(), &child, out);
        } else if name.ends_with(".rs") {
            out.push(child);
        }
    }
}

/// Every .rs file under `production/`, by a recursive walk.
fn production_sources() -> Vec<(String, String)> {
    let mut files = Vec::new();
    walk_rs(
        &repo_root().join(PRODUCTION_DIR),
        PRODUCTION_DIR,
        &mut files,
    );
    files.sort();
    assert!(
        files.iter().any(|f| f == MOD) && files.iter().any(|f| f == GATES),
        "fixture: the walk must reach the production module"
    );
    files
        .into_iter()
        .map(|f| {
            let text = read_repo(&f);
            (f, text)
        })
        .collect()
}

/// A method body: from the line holding `marker` to the first closing `    }` line.
fn method_text(source: &str, marker: &str) -> String {
    let mut lines = source.lines();
    let first = lines
        .by_ref()
        .find(|l| l.contains(marker))
        .unwrap_or_else(|| panic!("no `{marker}` in the source"));
    let mut body = vec![first];
    for line in lines {
        body.push(line);
        if line == "    }" {
            return body.join("\n");
        }
    }
    panic!("no closing line for `{marker}`");
}

fn real_sources() -> Vec<(String, String)> {
    let mut sources = production_sources();
    let body = method_text(&read_repo(GATE_FILE), "pub fn can_produce");
    sources.push((GATE_FILE.to_string(), body));
    sources
}

fn with_replaced(
    sources: &[(String, String)],
    file: &str,
    from: &str,
    to: &str,
) -> Vec<(String, String)> {
    sources
        .iter()
        .map(|(f, text)| {
            if f != file {
                return (f.clone(), text.clone());
            }
            assert!(text.contains(from), "fixture: {file} must hold `{from}`");
            (f.clone(), text.replacen(from, to, 1))
        })
        .collect()
}

fn with_appended(sources: &[(String, String)], file: &str, extra: &str) -> Vec<(String, String)> {
    with_replaced(
        sources,
        file,
        "use super::*;",
        &format!("use super::*;\n{extra}"),
    )
}

fn peer_height_reads_outside_named(counts: &Counts) -> Vec<String> {
    counts
        .keys()
        .filter(|(file, func, token)| {
            token == PEER_HEIGHT
                && file.starts_with(PRODUCTION_DIR)
                && !PEER_HEIGHT_READERS.iter().any(|(name, _)| name == func)
        })
        .map(|(file, func, _)| format!("{file} fn {func}"))
        .collect()
}

/// The text between the `{` at `open` and its matching `}`.
fn braced(text: &str, open: usize) -> &str {
    let mut depth = 0i64;
    for (i, c) in text[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &text[open + 1..open + i];
                }
            }
            _ => {}
        }
    }
    panic!("no closing brace");
}

// REQ-PASK-007 — Decision: a failure means a read of peer data was added to or removed from the production path, so the stop census no longer describes the code.
#[test]
fn production_stop_reads_match_the_allow_list() {
    let drift = compare(&count_reads(&real_sources()), &listed_counts(ALLOW_LIST));
    assert!(
        drift.is_empty(),
        "the production path reads differ from the allow-list:\n{}",
        render_all(&drift)
    );
}

// REQ-PASK-007 — Decision: a failure means the census cannot see a new read, so its pass says nothing about the production path.
#[test]
fn census_scanner_flags_an_injected_line() {
    let sources = real_sources();
    let real = count_reads(&sources);
    let tip_line = "let tip = next_height.saturating_sub(1);";
    assert_eq!(
        compare(&count_reads(&sources), &real),
        vec![],
        "fixture: real is clean"
    );

    // B: one more read in an allowed fn.
    let injected = format!("{tip_line}\n    let _read = best_peer_height;");
    let drift = compare(
        &count_reads(&with_replaced(&sources, GATES, tip_line, &injected)),
        &real,
    );
    assert_eq!(drift.len(), 1, "B: {}", render_all(&drift));
    assert!(drift[0].render().starts_with("NEW READ"), "B: {drift:?}");
    assert_eq!(drift[0].func, HEIGHT_RULE, "B: names the fn");
    assert_eq!(drift[0].token, PEER_HEIGHT, "B: names the token");

    // C: one read in a new fn, for each token.
    for token in TOKENS {
        let read = token.trim_start_matches('.');
        let extra = format!("fn new_stop(s: &Sync) -> bool {{\n    s.{read} > 0\n}}");
        let drift = compare(&count_reads(&with_appended(&sources, GATES, &extra)), &real);
        assert_eq!(drift.len(), 1, "C {token}: {}", render_all(&drift));
        assert_eq!(drift[0].func, "new_stop", "C {token}: names the new fn");
        assert_eq!(drift[0].token, token, "C {token}: names the token");
        assert!(drift[0].render().starts_with("NEW READ"), "C {token}");
    }

    // C: a one-line fn, and a read in a signature, are named by that fn.
    let extra = "fn one_line(s: &Sync) -> u64 { s.best_peer_height() }";
    let drift = compare(&count_reads(&with_appended(&sources, GATES, extra)), &real);
    assert_eq!(drift.len(), 1, "one line: {}", render_all(&drift));
    assert_eq!(drift[0].func, "one_line", "one line: names the fn");
    let extra = "fn signature(\n    peer_count: usize,\n) -> usize {\n    0\n}";
    let drift = compare(&count_reads(&with_appended(&sources, GATES, extra)), &real);
    assert_eq!(drift.len(), 1, "signature: {}", render_all(&drift));
    assert_eq!(drift[0].func, "signature", "signature: names the fn");

    // D: one read in a new file.
    let mut grown = sources.clone();
    let new_file = format!("{PRODUCTION_DIR}/deep/new_stop.rs");
    let text = "fn in_new_file(s: &Sync) -> u64 {\n    s.network_tip_height\n}\n";
    grown.push((new_file.clone(), text.to_string()));
    let drift = compare(&count_reads(&grown), &real);
    assert_eq!(drift.len(), 1, "D: {}", render_all(&drift));
    assert_eq!(drift[0].file, new_file, "D: names the file");
    assert!(drift[0].render().starts_with("NEW READ"), "D: {drift:?}");

    // E: an allowed read removed.
    let removed = "let gap = best_peer_height.saturating_sub(tip);";
    let drift = compare(
        &count_reads(&with_replaced(&sources, GATES, removed, "let gap = 0;")),
        &real,
    );
    assert_eq!(drift.len(), 1, "E: {}", render_all(&drift));
    assert!(
        drift[0].render().starts_with("DROPPED READ"),
        "E: {drift:?}"
    );
    assert_eq!(drift[0].func, HEIGHT_RULE, "E: names the fn");

    // F: comment text is not a read.
    let comments = format!(
        "{tip_line} // best_peer_height\n    // network_tip peer_count\n    /// peers.len .best_height"
    );
    let drift = compare(
        &count_reads(&with_replaced(&sources, GATES, tip_line, &comments)),
        &real,
    );
    assert_eq!(drift, vec![], "F: {}", render_all(&drift));

    // G: `//` in a string does not hide a read after it.
    let hidden = format!("{tip_line}\n    let _s = (\"//\", peer_count);");
    let drift = compare(
        &count_reads(&with_replaced(&sources, GATES, tip_line, &hidden)),
        &real,
    );
    assert_eq!(drift.len(), 1, "G: {}", render_all(&drift));
    assert_eq!(drift[0].token, PEER_COUNT, "G: names the token");
}

// REQ-PASK-005 — Decision: a failure means the dead gossip set can again stop a build, because a caller of the old read came back.
#[test]
fn has_block_for_slot_has_no_caller_in_production() {
    let callers: Vec<String> = production_sources()
        .into_iter()
        .filter(|(_, text)| code_only(text).contains(OLD_SLOT_READ))
        .map(|(file, _)| file)
        .collect();
    assert_eq!(callers, Vec::<String>::new(), "callers of the old read");

    let call = format!("let a = store.{OLD_SLOT_READ}(1);");
    assert!(code_only(&call).contains(OLD_SLOT_READ), "a call is seen");
    let comment = format!("// {OLD_SLOT_READ}(1)");
    assert!(
        !code_only(&comment).contains(OLD_SLOT_READ),
        "a comment is not"
    );
}

// REQ-PASK-006 — Decision: expected behaviour is that the peer rule takes no sync phase and no peer count as input.
#[test]
fn peer_height_gate_takes_no_phase_and_no_peer_count() {
    let source = code_only(&read_repo(GATES));
    let marker = "pub fn peer_height_gate(";
    let start = source.find(marker).expect("the rule is in gates.rs") + marker.len();
    let end = start + source[start..].find(')').expect("parameter list end");
    let params = &source[start..end];
    for banned in ["phase", "SyncPhase", "peer_count", "peers"] {
        assert!(!params.contains(banned), "input `{banned}` in: {params}");
    }
    let names: Vec<&str> = params
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| p.split(':').next().unwrap_or(p).trim())
        .collect();
    assert_eq!(
        names,
        ["next_height", "best_peer_height", "gate", "latch_holds"],
        "the inputs of the peer rule"
    );
}

// REQ-PASK-007 — Decision: a failure means peer height reached a new function on the production path, or the bootstrap path is no longer limited to its named state.
#[test]
fn bootstrap_path_reads_peer_height_only_in_its_named_function() {
    let sources = production_sources();
    let counts = count_reads(&sources);
    assert_eq!(
        peer_height_reads_outside_named(&counts),
        Vec::<String>::new(),
        "peer height read outside the named functions"
    );
    for (name, reason) in PEER_HEIGHT_READERS {
        assert!(!reason.is_empty(), "{name}: no reason");
        assert!(
            counts.keys().any(|(_, f, t)| f == name && t == PEER_HEIGHT),
            "{name}: listed but reads nothing"
        );
    }

    let planted = with_appended(
        &sources,
        ASSEMBLY,
        "fn planted(s: &Sync) -> u64 { s.best_peer_height() }",
    );
    assert_eq!(
        peer_height_reads_outside_named(&count_reads(&planted)),
        [format!("{ASSEMBLY} fn planted")],
        "a planted read must be reported"
    );

    let caller = code_only(&read_repo(MOD));
    let rule = caller
        .find("let use_bootstrap = in_genesis || active_with_weights.is_empty();")
        .expect("the bootstrap path has its named state");
    let branch = rule
        + caller[rule..]
            .find("if use_bootstrap {")
            .expect("its branch");
    let body = braced(&caller, branch + "if use_bootstrap ".len());
    assert!(
        body.contains(BOOTSTRAP),
        "the bootstrap reads are reached only from the branch of the named state"
    );
    let calls = caller.matches(BOOTSTRAP).count();
    assert_eq!(calls, 1, "one call site of the bootstrap reads");
}

// REQ-PASK-007 — Decision: a failure means a mainnet node past the transition block reads peer height on the bootstrap path before it leaves.
#[test]
fn mainnet_none_arm_comes_before_the_peer_read() {
    let source = read_repo(SCHEDULING);
    let body = code_only(&method_text(&source, &format!("pub async fn {BOOTSTRAP}")));

    let arm = body
        .find("Network::Mainnet if !in_genesis")
        .expect("the mainnet arm");
    let arm_open = arm + body[arm..].find('{').expect("mainnet arm body");
    assert_eq!(
        braced(&body, arm_open).trim(),
        "None",
        "the mainnet arm body"
    );

    let next = body
        .find("Network::Mainnet | Network::Testnet | Network::Devnet =>")
        .expect("the next arm");
    let read = body.find("best_peer_height()").expect("the peer read");
    assert!(arm_open < next, "the mainnet arm comes first");
    assert!(next < read, "the first peer read comes after the None arm");

    let next_open = next + body[next..].find('{').expect("next arm body");
    let next_end = next_open + braced(&body, next_open).len();
    assert!(
        next_open < read && read < next_end,
        "the first peer read is inside the next arm"
    );
}

// REQ-PASK-007 — Decision: a failure means tracked text still states a gate or a claim the production path no longer has, so a reader acts on a false rule.
#[test]
fn no_stale_gate_statement_in_tracked_text() {
    let mut stale = Vec::new();
    if read_repo(GATE_FILE).contains(REMOVED_GATE_CLAIM) {
        stale.push(format!("{GATE_FILE}: `{REMOVED_GATE_CLAIM}`"));
    }
    let optional = STALE_TEXT_OPTIONAL
        .iter()
        .filter(|rel| repo_root().join(rel).is_file());
    for rel in STALE_TEXT_REQUIRED.iter().chain(optional) {
        if read_repo(rel).contains(REMOVED_GATE_NAME) {
            stale.push(format!("{rel}: `{REMOVED_GATE_NAME}`"));
        }
    }
    assert!(
        stale.is_empty(),
        "tracked text states what the code no longer has:\n{}",
        stale.join("\n")
    );
}
