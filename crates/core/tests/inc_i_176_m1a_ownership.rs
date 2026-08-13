//! INC-I-176 **M1a** — SINGLE OWNERSHIP of the maintainer signing message.
//!
//! REQ-176-030 ("exactly ONE implementation of the maintainer-authorization
//! predicate"), REQ-176-031 (reuse an in-tree primitive), the `derivation.rs`
//! lockstep, and the leaf-module discipline that keeps `crates::maintainer` free
//! of a `network_params` / `chainspec` edge.
//!
//! The ENCODING contract is in `inc_i_176_m1a_authmsg.rs`, the BINDING contract
//! in `inc_i_176_m1a_binding.rs`, the WIRE FREEZE in
//! `inc_i_176_m1a_wire_freeze.rs`.
//!
//! ---------------------------------------------------------------------------
//! WHY THESE ARE SOURCE-TEXT TESTS, AND WHAT THAT COSTS
//! ---------------------------------------------------------------------------
//! "Exactly one function computes the signed message" is a property of the
//! REPOSITORY, not of any value a function returns. No behavioural test can
//! observe it: a second, byte-identical copy of the format is invisible to every
//! assertion about bytes — right up to the day the two copies drift, which is
//! precisely AUDIT-P1-004's failure mode.
//!
//! So these tests read source text. Source-text tests fail vacuously when the
//! scan itself breaks (wrong root, wrong glob, renamed file), and a vacuous PASS
//! on a security property is worse than no test. **Every scan below therefore
//! carries a POSITIVE CONTROL**: a second scan, over the same roots with the same
//! walker, for a pattern that is KNOWN to be present in quantity. If the control
//! finds nothing, the instrument is broken and the test fails loudly instead of
//! passing silently.
//!
//! Each root (`crates/`, `bins/`) is reported SEPARATELY. An aggregate count is
//! not a per-root fact.
//!
//! TDD RED. Against the tree at `3f8bf185` these fail: `authmsg.rs` does not
//! exist, so the owner-file assertions cannot hold.
//!
//! Contract: `docs/.workflow/inc-i-176-M1a-output-contract.md`.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT
//! ---------------------------------------------------------------------------
//! The "function under test" here is the REPOSITORY LAYOUT. Its observable
//! outputs are file contents.
//!   G-O1 the SET of non-test source files that construct the legacy maintainer
//!        message. Required cardinality: exactly 1, and it must be
//!        `crates/core/src/maintainer/authmsg.rs`.
//!   G-O2 the CONTENT of `derivation.rs`: both replay arms route through the
//!        shared constructor, neither re-derives the format.
//!   G-O3 the CONTENT of `authmsg.rs`: no `network_params` / `chainspec` edge
//!        (leaf-module discipline), and the house hasher idiom, not a new one.
//!   G-O4 the CONTENT of `crates/updater/src/verification.rs`: the release-signing
//!        family still has the shape the REQ-176-040 confusability test models.
//!        A PREMISE LOCK — if the modelled shape drifts, that test silently stops
//!        testing the real collision.
//!   G-O5 the CONTENT of the five `inc_i_174_*` suites: free of attempt-1
//!        payload-shape edits (REQ-176-003 requires them to pass UNMODIFIED).
//!   mutable params / receiver / persistent store / side channels: NONE.
//!
//! CODE PATHS
//!   P-SCAN  the walker finds files and reads them (the only path; a walker that
//!           finds nothing is caught by the positive control, not by this path).
//!
//! INPUT PARTITIONS
//!   IP-ROOT-C  root `crates/`  -> owner file lives here
//!   IP-ROOT-B  root `bins/`    -> must contribute ZERO non-test producers
//!   IP-NARROW  the discriminating pattern (`format!("{}:{}"` on a line that also
//!              names the action) -> G-O1
//!   IP-BROAD   the undiscriminating pattern (`format!("{}:{}"` anywhere)
//!              -> POSITIVE CONTROL; must match many lines in both roots
//!   IP-TESTDIR paths under a `tests/` directory -> EXCLUDED by design, see below
//!   MATRIX: G-O1 × {IP-ROOT-C, IP-ROOT-B} × {IP-NARROW, IP-BROAD};
//!           G-O2..G-O5 × the named files.
//!
//! ---------------------------------------------------------------------------
//! THE `tests/` EXCLUSION IS DELIBERATE AND REQUIRED
//! ---------------------------------------------------------------------------
//! Five test files re-derive the legacy message inline:
//! `bins/node/tests/inc_i_174_{maintainer_reorg, maintainer_rewind_guards,
//! maintainer_undo_capture, maintainer_undo, snapshot_binding}.rs`.
//! REQ-176-003 requires all five to keep passing **UNMODIFIED**, so M1a may not
//! touch them. Their duplication is an ACCEPTED, NAMED exception, not an
//! oversight: what keeps them honest is
//! `req_176_030_legacy_message_is_byte_identical_to_todays_format`
//! (`inc_i_176_m1a_authmsg.rs`), which pins the owned constructor to the exact
//! same format string. If the owner drifts, that test fails before these five do.

use std::fs;
use std::path::{Path, PathBuf};

/// Repository root, derived from this crate's manifest directory
/// (`<repo>/crates/core`).
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/core must sit two levels below the repository root")
        .to_path_buf()
}

/// Every `.rs` file under `root`. `include_tests = false` drops anything whose
/// path contains a `tests` directory component and anything named `tests.rs`.
fn rust_files(root: &Path, include_tests: bool) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                // Build output and VCS metadata are not source.
                if name == "target" || name == ".git" {
                    continue;
                }
                walk(&path, out);
            } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    if !include_tests {
        out.retain(|p| {
            let s = p.to_string_lossy();
            !s.contains("/tests/") && !s.ends_with("/tests.rs")
        });
    }
    out.sort();
    out
}

/// `(path, line number, line)` for every line matching `pred`.
fn scan<F>(files: &[PathBuf], pred: F) -> Vec<(String, usize, String)>
where
    F: Fn(&str) -> bool,
{
    let mut hits = Vec::new();
    for path in files {
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        for (i, line) in text.lines().enumerate() {
            if pred(line) {
                hits.push((path.to_string_lossy().to_string(), i + 1, line.to_string()));
            }
        }
    }
    hits
}

/// The BROAD pattern. Present in quantity across both roots (bridge swap ids,
/// RPC outpoints, CLI service users, the release-signing family, …). Used ONLY
/// as the positive control.
fn is_broad(line: &str) -> bool {
    line.contains("format!(\"{}:{}\"")
}

/// The NARROW, discriminating pattern: a `"{}:{}"` format whose arguments name
/// the maintainer ACTION. This is what distinguishes the governance message from
/// every other colon-joined string in the tree.
///
/// Doc-comment lines are excluded — prose that QUOTES the format is not a second
/// producer of it, and `authmsg.rs` quotes it deliberately so the defect can be
/// pointed at.
fn is_maintainer_message_producer(line: &str) -> bool {
    let code = line.trim_start();
    if code.starts_with("//") {
        return false;
    }
    is_broad(line)
        && (code.contains(", action,") || code.contains("\"add\"") || code.contains("\"remove\""))
}

// ===========================================================================
// REQ-176-030 — EXACTLY ONE OWNER
// ===========================================================================

/// REQ-176-030 / G-O1 — exactly ONE non-test source file constructs the legacy
/// maintainer message, and it is `crates/core/src/maintainer/authmsg.rs`.
///
/// Reported per root. The positive control runs first: if the broad pattern
/// finds nothing, the walker is broken and an empty narrow result would be
/// meaningless.
#[test]
fn req_176_030_exactly_one_source_file_owns_the_maintainer_signing_message() {
    let root = repo_root();

    for (label, dir) in [("crates/", "crates"), ("bins/", "bins")] {
        let files = rust_files(&root.join(dir), false);

        // ---- POSITIVE CONTROL (IP-BROAD) -------------------------------
        assert!(
            files.len() > 20,
            "POSITIVE CONTROL: the walker found only {} .rs files under {label}. The scan is \
             broken, so any narrow result below would be vacuous.",
            files.len()
        );
        let broad = scan(&files, is_broad);
        assert!(
            broad.len() >= 3,
            "POSITIVE CONTROL: the broad pattern `format!(\"{{}}:{{}}\"` matched only {} lines \
             under {label}. It is known to be common there; if the control cannot find it, the \
             narrow scan proves nothing.",
            broad.len()
        );

        // ---- THE ACTUAL PROPERTY (IP-NARROW) ---------------------------
        let owners = scan(&files, is_maintainer_message_producer);
        let owner_files: Vec<&String> = owners.iter().map(|(p, _, _)| p).collect();

        match dir {
            "bins" => assert!(
                owners.is_empty(),
                "REQ-176-030 / G-O1 (root {label}): no non-test source file under bins/ may \
                 construct the maintainer signing message. Found: {owner_files:?}. \
                 `bins/node/src/commands/maintainer.rs` and the RPC path must call \
                 `doli_core::maintainer::signing_message_legacy`, never re-type the format."
            ),
            _ => {
                assert_eq!(
                    owners.len(),
                    1,
                    "REQ-176-030 / G-O1 (root {label}): expected EXACTLY ONE producer of the \
                     maintainer signing message; found {}. Hits: {owner_files:?}. Two producers \
                     of the same signed bytes is AUDIT-P1-004's failure mode — they drift, and \
                     the drift is only observable as signatures that stop verifying on a live \
                     chain.",
                    owners.len()
                );
                assert!(
                    owners[0]
                        .0
                        .ends_with("crates/core/src/maintainer/authmsg.rs"),
                    "REQ-176-030 / G-O1: the single owner must be \
                     crates/core/src/maintainer/authmsg.rs, not {}",
                    owners[0].0
                );
            }
        }
    }
}

/// REQ-176-030 / G-O2 — `derivation.rs` replay arms move in LOCKSTEP.
///
/// Both `MaintainerChange::Add` and `MaintainerChange::Remove` must obtain their
/// message from the ONE shared constructor. Historically each arm called
/// `data.signing_message(is_add)` separately; that is where a future height gate
/// has to be threaded, and two independently-edited arms is exactly how one gets
/// gated and the other does not.
///
/// Positive control included: the file must actually contain both arms, or the
/// absence of a duplicate format string would be vacuous.
#[test]
fn req_176_030_both_derivation_arms_route_through_the_shared_constructor() {
    let path = repo_root().join("crates/core/src/maintainer/derivation.rs");
    let text = fs::read_to_string(&path).expect("derivation.rs must exist");

    // ---- POSITIVE CONTROL ---------------------------------------------
    assert!(
        text.contains("MaintainerChange::Add") && text.contains("MaintainerChange::Remove"),
        "POSITIVE CONTROL: derivation.rs must still contain BOTH replay arms. If the arms were \
         renamed or moved, every assertion below is vacuous."
    );

    let code_lines: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect();

    let inline_format = code_lines
        .iter()
        .filter(|l| is_maintainer_message_producer(l))
        .count();
    assert_eq!(
        inline_format, 0,
        "REQ-176-030 / G-O2: derivation.rs must not re-derive the message format inline; it must \
         call the owned constructor"
    );

    let via_owner = code_lines
        .iter()
        .filter(|l| l.contains("signing_message_legacy("))
        .count();
    assert_eq!(
        via_owner, 2,
        "REQ-176-030 / G-O2: BOTH replay arms (Add and Remove) must call \
         `signing_message_legacy` — found {via_owner} call(s). Lockstep is the property: when M2 \
         swaps these for `signing_message_at`, one gated arm and one ungated arm would make the \
         derived set depend on which arm a change took."
    );
}

// ===========================================================================
// REQ-176-031 / REQ-176-040 — REUSE AND SEPARATION, ANCHORED TO REALITY
// ===========================================================================

/// REQ-176-031 / G-O3 — `authmsg.rs` reuses the HOUSE digest primitive and stays
/// a LEAF module.
///
/// The house idiom is `digest.rs`: a plain `Hasher::new()` fed a domain tag, NOT
/// `new_with_domain` (which length-prefixes and would not match). The leaf
/// property is what lets `crates::maintainer` be used by `crates::validation`
/// without a cycle: the genesis hash arrives as `&[u8]` and the activation height
/// as a plain `u64`.
#[test]
fn req_176_031_authmsg_reuses_the_house_primitive_and_stays_a_leaf() {
    let root = repo_root();
    let authmsg = fs::read_to_string(root.join("crates/core/src/maintainer/authmsg.rs"))
        .expect("authmsg.rs must exist");
    let digest = fs::read_to_string(root.join("crates/core/src/maintainer/digest.rs"))
        .expect("digest.rs must exist");

    // ---- POSITIVE CONTROL: the token IS findable in this tree ----------
    assert!(
        digest.contains("Hasher::new()"),
        "POSITIVE CONTROL: digest.rs is the house idiom being reused; if it no longer contains \
         `Hasher::new()`, the comparison below is meaningless"
    );
    let params_dir = root.join("crates/core/src/network_params");
    assert!(
        params_dir.is_dir(),
        "POSITIVE CONTROL: crates/core/src/network_params/ must exist, or 'authmsg has no \
         network_params edge' is trivially true"
    );

    assert!(
        authmsg.contains("Hasher::new()"),
        "REQ-176-031 / G-O3: authmsg.rs must hash with the house `Hasher::new()` idiom, the same \
         primitive digest.rs already uses"
    );

    let code: String = authmsg
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in ["network_params", "NetworkParams", "chainspec", "ChainSpec"] {
        assert!(
            !code.contains(forbidden),
            "REQ-176-031 / G-O3: authmsg.rs must stay a LEAF module — it may not reference \
             `{forbidden}`. The genesis hash arrives as `&[u8]` and the activation height as a \
             plain `u64`, exactly as `MaintainerSet::verify_multisig_at` already takes them."
        );
    }
}

/// REQ-176-040 / G-O4 — PREMISE LOCK on the release-signing family.
///
/// `req_176_040_digest_is_not_confusable_with_the_release_or_legacy_families`
/// (`inc_i_176_m1a_authmsg.rs`) reproduces the release family's SHAPE locally,
/// because M1a does not touch `crates/updater/`. That reproduction is only
/// evidence while the real thing still has that shape. AUDIT-P0-011 exists
/// because `format!("{}:{}", version, binary_sha256)` with `version = "add"` and
/// `binary_sha256 = target.to_hex()` is byte-identical to the legacy governance
/// message.
///
/// If this test fails, the confusability test is no longer modelling reality —
/// fix the model, do not delete this lock.
#[test]
fn req_176_040_release_signing_family_still_has_the_modelled_shape() {
    let path = repo_root().join("crates/updater/src/verification.rs");
    let text = fs::read_to_string(&path).expect("crates/updater/src/verification.rs must exist");

    assert!(
        text.contains("format!(\"{}:{}\", version, binary_sha256)"),
        "REQ-176-040 / G-O4: the release-signing message is modelled in \
         inc_i_176_m1a_authmsg.rs as `format!(\"{{}}:{{}}\", version, binary_sha256)`. That shape \
         is no longer present in crates/updater/src/verification.rs, so the confusability test \
         has silently stopped testing the real collision."
    );
    assert!(
        text.contains("pub fn sign_release_hash"),
        "POSITIVE CONTROL: the release-signing entry point must still be here, or the assertion \
         above could pass on an unrelated line"
    );
}

// ===========================================================================
// REQ-176-003 — THE INC-I-174 SUITES ARE UNTOUCHED
// ===========================================================================

/// REQ-176-003 / G-O5 — all five `inc_i_174_*` suites exist and carry NO
/// attempt-1 payload-shape edit.
///
/// REQ-176-003 requires them to pass UNMODIFIED. Attempt 1 edited the maintainer
/// payload shape, which forced signing-shape edits into these files; M1a changes
/// no payload, so they must be clean of `valid_before` entirely. Their inline
/// re-derivation of the legacy message is the NAMED exception documented in this
/// file's header.
///
/// This is a cheap tripwire, not a substitute for running them.
#[test]
fn req_176_003_the_five_inc_i_174_suites_carry_no_payload_shape_edit() {
    let dir = repo_root().join("bins/node/tests");
    let names = [
        "inc_i_174_maintainer_reorg.rs",
        "inc_i_174_maintainer_rewind_guards.rs",
        "inc_i_174_maintainer_undo_capture.rs",
        "inc_i_174_maintainer_undo.rs",
        "inc_i_174_snapshot_binding.rs",
    ];

    for name in names {
        let path = dir.join(name);
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("REQ-176-003: {name} must exist and be readable"));

        // POSITIVE CONTROL: the file really is the maintainer suite we think it is.
        assert!(
            text.contains("MaintainerChangeData"),
            "POSITIVE CONTROL: {name} must reference MaintainerChangeData, or the absence of \
             `valid_before` below proves nothing"
        );
        assert!(
            !text.contains("valid_before"),
            "REQ-176-003 / G-O5: {name} references `valid_before`. That is an attempt-1 payload \
             edit; M1a changes no payload, and REQ-176-003 requires these five suites to pass \
             UNMODIFIED. Revert the file."
        );
        assert!(
            !text.contains("with_valid_before"),
            "REQ-176-003 / G-O5: {name} calls `with_valid_before`, a constructor that does not \
             exist in M1a"
        );
    }
}
