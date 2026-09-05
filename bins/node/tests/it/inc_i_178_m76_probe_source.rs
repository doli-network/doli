//! INC-I-178 M7.6 — REQ-BLS-006 AC-2 / REQ-BLS-007: the GS-018 probe must reach
//! its AC-2 verdict from /metrics alone, over a fleet-sized union.
//!
//! OUTPUT CONTRACT
//!
//! F1: `scripts/gauntlet-gs018.sh` as SOURCE TEXT. This repo has no bash test
//!   harness, so the oracle is the executable text of the two check functions,
//!   with whole-line `#` comments stripped — prose in the header block cannot
//!   satisfy an assertion about behaviour.
//!   Observable outputs of `_gs018_dual_check` (a shell verdict function):
//!     O1 return code — 0 PASS / 1 FAIL / 2 SKIP
//!     O2 `SKIP_REASONS` — appended on every non-verdict path
//!     O3 `FAIL_REASONS` — appended only on the AC-2 shortfall
//!     O4 `INFO_REASONS` — appended on PASS
//!     O5 no persistent store, no other mutable state
//!   PATHS:
//!     Q1 denominator unreadable                 -> SKIP  (shipped in M7.5)
//!     Q2 `capable == 0`                         -> SKIP  (absence of the marker)
//!     Q3 `capable < GS018_MIN_NODES`            -> SKIP  (M7.6, the union floor)
//!     Q4 label union empty                      -> SKIP  (zero-length window)
//!     Q5 matched < active                       -> FAIL
//!     Q6 matched == active                      -> PASS
//!   MATRIX: Q3 is the only path M7.6 adds, and it is the only RED assertion
//!     here. Q2 and Q4 are asserted as SURVIVAL tripwires because the developer
//!     is about to edit the exact `capable` block those SKIPs live in. Q1/Q5/Q6
//!     are M7.5 contracts already covered by `inc_i_178_m75_ingress_signal`.
//!   INPUT PARTITIONS: N/A — one text per file.
//!
//! F2: `_gs018_postah_check`, asserted only as an untouched-neighbour tripwire:
//!   it shares the file M7.6 edits and its pre-activation SKIP is the guard that
//!   stops "0 rejections" from reading as a PASS the chain never earned.

use std::fs;
use std::path::{Path, PathBuf};

const GS018_SH: &str = "scripts/gauntlet-gs018.sh";
const DUAL_FN: &str = "_gs018_dual_check";
const POSTAH_FN: &str = "_gs018_postah_check";
const MIN_NODES: &str = "GS018_MIN_NODES";
const VALID_SERIES: &str = "GS018_BLS_VALID_SERIES";
const ATTESTER_SERIES: &str = "GS018_BLS_ATTESTER_SERIES";
const VERIFY_SERIES: &str = "GS018_VERIFY_SERIES";
const REJECTED_SERIES: &str = "doli_attestation_verify_rejected_total";
/// The M7.6 log literal, at `debug!` from this milestone on. No GS-018 verdict
/// may stand on it.
const VALID_LOG_FRAGMENT: &str = "valid bls";

// ---------------------------------------------------------------------------
// Harness — same source-text idiom as `inc_i_178_m75_ingress_signal`
// ---------------------------------------------------------------------------

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root must resolve")
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// Shell source with every whole-line `#` comment removed.
fn sh_code_only(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Executable body of one POSIX-shell function, from its `name() {` header to
/// the first closing brace at column 0.
fn sh_fn_body(name: &str) -> String {
    let script = sh_code_only(&read(GS018_SH));
    let head = format!("{name}() {{");
    let at = script
        .find(&head)
        .unwrap_or_else(|| panic!("{GS018_SH} no longer defines `{name}`"));
    let rest = &script[at + head.len()..];
    let end = rest
        .find("\n}")
        .unwrap_or_else(|| panic!("`{name}` in {GS018_SH} has no closing brace at column 0"));
    let body = rest[..end].to_string();
    assert!(
        !body.contains("() {"),
        "body extraction for `{name}` ran past its closing brace into another function; the \
         assertions below would be reading the wrong code"
    );
    body
}

// ===========================================================================
// The union floor — the one path M7.6 adds
// ===========================================================================

/// REQ-BLS-006 — Decision: without the floor, a union read over one scraped node
/// during a ROLLING deploy decides AC-2 for ~30 mainnet producers and reports every
/// unscraped dual-signer as "not dual-signing" — a false FAIL that would block
/// pinning the activation height on manufactured evidence.
#[test]
fn req_bls_006_the_dual_check_gates_its_union_on_a_minimum_node_count() {
    let body = sh_fn_body(DUAL_FN);
    assert!(
        body.contains("capable"),
        "`{DUAL_FN}` no longer counts capable nodes, so there is nothing for a union floor \
         to gate"
    );
    assert!(
        body.contains(MIN_NODES),
        "`{DUAL_FN}` never compares `capable` against `{MIN_NODES}`. The sibling assertion \
         `gs018-presence-root-consistent` already SKIPs below that floor for answering nodes; \
         a union over attester labels needs the same floor, or a partially scraped fleet \
         yields a verdict it did not observe"
    );
}

// ===========================================================================
// Survival tripwires — the developer edits this exact block next
// ===========================================================================

/// REQ-BLS-007 — Decision: M7.6 moves the valid-bls line to `debug!` and the fleet runs
/// at info, so any verdict standing on that log line would read an empty grep and report
/// a silent fleet; the AC-2 evidence must come from /metrics alone.
#[test]
fn req_bls_007_no_dual_check_verdict_reads_the_valid_bls_log_line() {
    let raw = read(GS018_SH);
    assert!(
        !raw.contains(VALID_LOG_FRAGMENT),
        "{GS018_SH} mentions the `{VALID_LOG_FRAGMENT}` log literal. From M7.6 that line is \
         emitted at `debug!` and production runs at `--log-level info`, so a verdict built on \
         it greps nothing and reports a dual-signing fleet as silent"
    );

    let body = sh_fn_body(DUAL_FN);
    for token in ["_gs018_metrics", ATTESTER_SERIES] {
        assert!(
            body.contains(token),
            "`{DUAL_FN}` no longer reads `{token}`. Per-producer dual-sign evidence comes from \
             the /metrics union over the attester series, never from a log file"
        );
    }
    for line in body.lines().filter(|l| l.contains("_gs018_new_warn_count")) {
        assert!(
            line.contains("FAIL_REASONS"),
            "`{DUAL_FN}` calls `_gs018_new_warn_count` outside a FAIL_REASONS string. The warn \
             count is informational — it fires only on a relayed INVALID half, so it may \
             accompany a FAIL and must never decide one"
        );
    }
}

/// REQ-BLS-006 — Decision: if either pre-existing SKIP is folded into the new floor, a
/// fleet that predates M7.5 or a node restarted seconds ago is reported as producers not
/// dual-signing — the exact false FAIL those SKIPs were shipped to prevent.
#[test]
fn req_bls_006_the_dual_check_keeps_both_pre_existing_skips() {
    let body = sh_fn_body(DUAL_FN);
    assert!(
        body.lines()
            .any(|l| l.contains("capable") && l.contains("-eq 0")),
        "`{DUAL_FN}` dropped the `capable == 0` case. Total ABSENCE of the marker means the \
         fleet is below M7.5 and must SKIP; collapsing it into the node-count floor loses the \
         distinction between 'not scraped' and 'not built'"
    );
    assert!(
        body.contains(VALID_SERIES),
        "`{DUAL_FN}` no longer names `{VALID_SERIES}`, the capability marker the absence SKIP \
         keys on"
    );
    assert!(
        body.lines()
            .any(|l| l.contains("-z") && l.contains("labels")),
        "`{DUAL_FN}` dropped the empty-label-union SKIP. A zero-length observation window is \
         not evidence that nobody dual-signs"
    );
}

/// REQ-BLS-007 — Decision: M7.6 edits this file; if the post-AH assertion loses its
/// activation gate it reads `0 rejections` on a pre-AH fleet and publishes a PASS for a
/// verification path that has never run.
#[test]
fn req_bls_007_the_postah_check_still_gates_on_the_verify_series() {
    let body = sh_fn_body(POSTAH_FN);
    let gate = body.find(VERIFY_SERIES).unwrap_or_else(|| {
        panic!(
            "`{POSTAH_FN}` no longer reads `{VERIFY_SERIES}`, the marker that tells a pre-AH \
             fleet from a build that cannot report verifications at all"
        )
    });
    let rejected = body.find(REJECTED_SERIES).unwrap_or_else(|| {
        panic!("`{POSTAH_FN}` no longer reads `{REJECTED_SERIES}`, so no rejection can FAIL it")
    });
    assert!(
        gate < rejected,
        "`{POSTAH_FN}` reads `{REJECTED_SERIES}` before establishing `{VERIFY_SERIES}`; the \
         rejection count must only be believed once activation is shown to be crossed"
    );
    assert!(
        body.lines()
            .any(|l| l.contains("SKIP_REASONS") && l.contains("pre-AH")),
        "`{POSTAH_FN}` dropped its pre-activation SKIP; before the AH there is no aggregate to \
         verify and 0 rejections is not a PASS"
    );
}
