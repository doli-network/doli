//! INC-I-176 **M2** QA finding **F2 / GAP-176-M2-02** — tripwires for the ONE
//! maintainer-authorization verifier M2 deliberately LEFT on the legacy arm.
//!
//! What is ungated, and why that was allowed
//! -----------------------------------------
//! M2 wired gate #22 (`inc_i_176_auth_binding_activation_height`) into the
//! production verifier in `bins/node/src/node/apply_block/governance.rs`. A
//! SECOND verifier exists in the same crate as the message constructor:
//! `doli_core::maintainer::derive_maintainer_set` verifies `MaintainerChange::Add`
//! and `::Remove` signatures with `signing_message_legacy` **unconditionally**,
//! even though it already threads `height` and gate #20.
//!
//! Converting it is **REQ-176-041**, recorded as **WON'T, this run** in
//! `docs/.workflow/milestone-progress.md:47`. It ships unconverted because the
//! divergence it could cause is UNREACHABLE, on one leg:
//!
//! * `derive_maintainer_set` has **zero production callers**. The production
//!   trust-root paths (`crates/rpc/src/methods/governance.rs`,
//!   `bins/node/src/node/periodic.rs`, `crates/updater/src/trust_root.rs`) all
//!   call the DIFFERENT function `derive_canonical_maintainer_set`, which seats
//!   members by registration order and verifies no signatures at all.
//!
//! Why this file exists
//! --------------------
//! That leg is a **fact about the tree, not an invariant**. The day anything wires
//! `derive_maintainer_set` into a production path — INC-I-172 M3 / R1 proposes
//! exactly that for the seed path — a replay-derived root would ACCEPT the legacy
//! message and REJECT the bound one at and above #22, disagreeing with the apply
//! path. That is the trust-root fragmentation #22 exists to prevent, and it would
//! appear silently, because this site is non-fatal and only logs.
//!
//! CLAUDE.md's own INC-I-075 lesson is that **"currently unused" is NEVER a valid
//! reason to skip a gate**. These tests are the mechanism that keeps the premise
//! from rotting in silence: they FAIL when either leg stops being true.
//!
//! Mirrors `crates/core/tests/inc_i_172_m2_ungated_tripwire.rs` (recorded as
//! `INV-AUTH-001`), including its anti-vacuity instruments, and adds a POSITIVE
//! CONTROL the reference file did not need — see ANTI-VACUITY below.
//!
//! Scope note: a green run here proves the F2 SAFETY ARGUMENT is still standing.
//! It does NOT prove leaving the site on the legacy arm is harmless in some tree
//! where the leg has moved — at that point REQ-176-041 must be DONE, not deferred
//! again.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT
//! ---------------------------------------------------------------------------
//! These are STRUCTURAL tests: the unit under test is the source tree itself, and
//! the "functions" are the two properties the F2 deferral rests on.
//!
//! Properties under test:
//!   G1: the CALLER SET of `doli_core::maintainer::derive_maintainer_set`
//!       restricted to production (non-test) code
//!   G2: WHICH message constructor the two signature arms inside
//!       `derive_maintainer_set` call
//!
//! OUTPUTS
//!   O1 (derived value)  `offenders` — production, non-allowlisted files that name
//!      `derive_maintainer_set`. The assertion subject for G1.
//!   O2 (instrument)     the number of `.rs` files the walker visited — an
//!      anti-vacuity instrument, NOT a property of the system under test
//!   O3 (instrument)     `definition_seen` — whether the scan could see the
//!      symbol's definition site at all (anti-vacuity)
//!   O4 (instrument)     `control_callers` — production files naming the SIBLING
//!      function `derive_canonical_maintainer_set` from OUTSIDE `doli-core`'s
//!      maintainer module. A POSITIVE CONTROL: the same scanner, over the same
//!      file set, must be able to FIND a production caller when one exists.
//!   O5 (derived value)  the brace-matched body text of `derive_maintainer_set`
//!   O6 (derived value)  presence of `signing_message_legacy` / absence of
//!      `signing_message_at` within O5. The assertion subject for G2.
//!   O7 (mutable params)       — NONE; both tests are read-only over the tree
//!   O8 (receiver mutation)    — NONE; free functions
//!   O9 (persistent store)     — NONE; no file is written
//!
//! PATHS
//!   PT-clean   — the leg still holds; the tripwire stays green
//!   PT-tripped — the leg has moved; the tripwire fails with the re-derivation
//!                instruction
//!   PT-broken  — the scanner itself stopped working (moved symbol, failed
//!                extraction, empty walk, dead control); must FAIL, never pass
//!                vacuously
//!
//! INPUT PARTITIONS  (the input is the file tree; partitions are file classes)
//!   IP-T1  production source with no mention of the symbol   -> ignored
//!   IP-T2  allowlisted file: the definition
//!          (`crates/core/src/maintainer/derivation.rs`) and the two re-exports
//!          (`crates/core/src/maintainer/mod.rs`, `crates/core/src/lib.rs`)
//!                                                            -> ignored [PT-clean]
//!   IP-T3  test-only mention: a file under `*/tests/`, named `tests.rs` /
//!          `tests_*` / `test_*`, or inside a trailing inline `#[cfg(test)]`
//!          module                                            -> ignored [PT-clean]
//!   IP-T4  mention inside a `//` comment or doc comment       -> ignored [PT-clean]
//!   IP-T5  production, non-allowlisted, non-comment mention   -> O1 non-empty
//!                                                                [PT-tripped]
//!   IP-T6  `derive_maintainer_set` body calling only
//!          `signing_message_legacy`                           -> [PT-clean]
//!   IP-T7  `derive_maintainer_set` body calling
//!          `signing_message_at`                               -> [PT-tripped:
//!                                                                REQ-176-041 landed]
//!   IP-T8  scanner degeneracy: <=100 files walked, definition site absent, an
//!          empty positive control, or an extracted body missing
//!          `verify_multisig_at`                               -> [PT-broken]
//!
//! MATRIX
//!   O1 x {IP-T1, IP-T2, IP-T3, IP-T4, IP-T5} = 1 assertion covering all five
//!        classes (the offender list is the quotient of the whole tree by them)
//!   O2 x {IP-T8}                             = 1 assertion
//!   O3 x {IP-T8}                             = 1 assertion
//!   O4 x {IP-T8}                             = 1 assertion (positive control)
//!   O5 x {IP-T8}                             = 1 assertion (extraction sanity)
//!   O6 x {IP-T6, IP-T7}                      = 2 assertions
//!   O7/O8/O9 — structurally absent; both tests only read.
//!
//! ANTI-VACUITY
//!   O2/O3 exist so that a rename, a move, or a broken walk FAILS (PT-broken)
//!   instead of yielding an empty offender list that looks green. O4 is stronger
//!   than either: the substring `derive_maintainer_set` is NOT contained in
//!   `derive_canonical_maintainer_set`, so the two symbols are independent — and
//!   the sibling DOES have production callers in other crates and bins. If the
//!   scanner can still see those, an empty offender list for O1 is a fact about
//!   the tree rather than a fact about the scanner. O5's `verify_multisig_at`
//!   check plays the same role for G2: a mis-extracted (e.g. empty) body would
//!   trivially satisfy IP-T6.

use std::fs;
use std::path::{Path, PathBuf};

/// Files permitted to mention `derive_maintainer_set`: its definition and the two
/// crate re-exports. Anything else is a caller and trips the wire.
const DERIVE_ALLOWLIST: &[&str] = &[
    "crates/core/src/maintainer/derivation.rs",
    "crates/core/src/maintainer/mod.rs",
    "crates/core/src/lib.rs",
];

/// Path prefixes that are NOT eligible to serve as the positive control: the
/// sibling's own definition and re-export sites. The control must be a caller in
/// a DIFFERENT crate or bin, otherwise it proves nothing the allowlist did not
/// already assume.
const CONTROL_EXCLUDED: &[&str] = &["crates/core/src/maintainer/", "crates/core/src/lib.rs"];

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is <repo>/crates/core.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root must resolve")
}

/// Every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // `target/` holds generated sources; `benches/` is not production.
            if name == "target" || name == "benches" {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// True if `rel` is test-only: an integration-test directory, or a file whose
/// entire purpose is tests by this repo's naming convention.
fn is_test_file(rel: &str) -> bool {
    let file = rel.rsplit('/').next().unwrap_or(rel);
    rel.contains("/tests/")
        || file == "tests.rs"
        || file.starts_with("tests_")
        || file.starts_with("test_")
}

/// Drop the trailing inline `#[cfg(test)]` module, which by this repo's
/// convention is the last item in a source file. Anything before it is
/// production code.
fn strip_inline_test_module(src: &str) -> &str {
    match src.find("#[cfg(test)]") {
        Some(i) => &src[..i],
        None => src,
    }
}

/// Strip `//` line comments so a doc comment naming the symbol is not counted as
/// a caller. Block comments are not used for this in tree.
fn strip_line_comments(src: &str) -> String {
    src.lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// THE LEG. O1, O2, O3, O4 x {IP-T1..IP-T5, IP-T8}.
///
/// Fails when `derive_maintainer_set` gains a production caller. The moment it has
/// one, the unconverted `signing_message_legacy` arms inside it become reachable
/// from production, and above #22 a replay-derived trust root disagrees with the
/// gated apply path in `bins/node/src/node/apply_block/governance.rs`.
#[test]
fn tripwire_derive_maintainer_set_has_no_production_callers() {
    let root = repo_root();
    let mut files = Vec::new();
    rust_files(&root.join("crates"), &mut files);
    rust_files(&root.join("bins"), &mut files);

    // O2 / IP-T8 — ANTI-VACUITY: the walk actually found the tree.
    assert!(
        files.len() > 100,
        "scanner is broken: only {} .rs files found under {}. A vacuous pass here \
         would silently retire the tripwire.",
        files.len(),
        root.display()
    );

    let mut definition_seen = false;
    let mut offenders: Vec<String> = Vec::new();
    let mut control_callers: Vec<String> = Vec::new();

    for path in &files {
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let src = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let code = strip_line_comments(strip_inline_test_module(&src));

        // O4 — POSITIVE CONTROL, computed from the SAME `code` string the
        // property below is computed from.
        if code.contains("derive_canonical_maintainer_set")
            && !is_test_file(&rel)
            && !CONTROL_EXCLUDED.iter().any(|p| rel.starts_with(p))
        {
            control_callers.push(rel.clone());
        }

        if !code.contains("derive_maintainer_set") {
            continue;
        }
        if rel == "crates/core/src/maintainer/derivation.rs" {
            definition_seen = true;
        }
        if !DERIVE_ALLOWLIST.contains(&rel.as_str()) && !is_test_file(&rel) {
            offenders.push(rel);
        }
    }

    // O3 / IP-T8 — ANTI-VACUITY: the scan can see the symbol at all. If the
    // definition moves or is renamed, this fails instead of passing on an empty
    // search.
    assert!(
        definition_seen,
        "scanner is broken: `derive_maintainer_set` was not found in \
         crates/core/src/maintainer/derivation.rs. If it moved, update \
         DERIVE_ALLOWLIST and re-derive the F2 deferral argument — do not just \
         silence this test."
    );

    // O4 / IP-T8 — POSITIVE CONTROL. The sibling `derive_canonical_maintainer_set`
    // IS called from production outside doli-core's maintainer module. If this
    // scanner cannot find those calls, its empty offender list below proves
    // nothing about the tree.
    assert!(
        control_callers.len() >= 2,
        "POSITIVE CONTROL FAILED: the scanner found {} production caller(s) of the \
         sibling `derive_canonical_maintainer_set` outside {:?}, expected at least \
         2 (crates/rpc/src/methods/governance.rs and \
         bins/node/src/node/periodic.rs). Found: {:?}.\n\n\
         With a dead control, an empty offender list for `derive_maintainer_set` is \
         a fact about the SCANNER, not about the tree. Fix the scanner — or, if the \
         production callers genuinely moved, re-derive the control before trusting \
         the result.",
        control_callers.len(),
        CONTROL_EXCLUDED,
        control_callers
    );

    // O1 — the property itself.
    assert!(
        offenders.is_empty(),
        "INC-I-176 M2 / F2 GAP-176-M2-02 TRIPWIRE: `derive_maintainer_set` now has \
         production caller(s): {:?}\n\n\
         Both of its signature arms still build the LEGACY authorization message \
         (`signing_message_legacy`) UNCONDITIONALLY — INC-I-176 M2 wired gate #22 \
         into `bins/node/src/node/apply_block/governance.rs` and deliberately did \
         NOT convert this site (REQ-176-041, WON'T-this-run, \
         docs/.workflow/milestone-progress.md:47). The deferral is sound ONLY \
         while this function is unreachable from production.\n\n\
         With a production caller, at and above \
         `inc_i_176_auth_binding_activation_height` a replay-derived maintainer set \
         ACCEPTS the legacy message and REJECTS the bound one, so it disagrees with \
         the apply path — trust-root fragmentation, arriving silently because this \
         site only logs.\n\n\
         Required action: complete REQ-176-041 (thread the genesis hash and the #22 \
         height into `derive_maintainer_set` and call `signing_message_at` in BOTH \
         arms), then retire this tripwire deliberately. Adding the new caller to \
         DERIVE_ALLOWLIST is NOT the fix.",
        offenders
    );
}

/// THE ARM. O5, O6 x {IP-T6, IP-T7, IP-T8}.
///
/// Fails when `derive_maintainer_set` stops being on the legacy arm — i.e. when
/// REQ-176-041 lands. At that point the F2 hazard is closed by WIRING rather than
/// by unreachability, and the caller tripwire above must be retired on purpose
/// instead of left standing as a rule nobody derived.
#[test]
fn tripwire_derive_maintainer_set_is_still_on_the_legacy_arm() {
    let root = repo_root();
    let path = root.join("crates/core/src/maintainer/derivation.rs");
    let src = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "scanner is broken: cannot read {} ({e}). Do not silence this — \
             re-derive the F2 deferral argument.",
            path.display()
        )
    });

    let start = src
        .find("pub fn derive_maintainer_set")
        .expect("scanner is broken: `pub fn derive_maintainer_set` not found in derivation.rs");
    let body_open = src[start..]
        .find('{')
        .expect("scanner is broken: no body for derive_maintainer_set")
        + start;

    // Brace-match to the end of the function body.
    let mut depth = 0usize;
    let mut end = body_open;
    for (i, c) in src[body_open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = body_open + i;
                    break;
                }
            }
            _ => {}
        }
    }
    let body = strip_line_comments(&src[body_open..=end]);

    // O5 / IP-T8 — ANTI-VACUITY: the extracted body really is the replay loop. If
    // this fails the extraction is wrong and the arm check below would pass
    // vacuously.
    assert!(
        body.contains("verify_multisig_at") && body.contains("verify_multisig_excluding_at"),
        "scanner is broken: the extracted derive_maintainer_set body does not \
         contain verify_multisig_at/verify_multisig_excluding_at, so it is not the \
         replay loop. Fix the extraction before trusting the result."
    );

    // O6 / IP-T6 — the legacy constructor is still the one in use, on both arms.
    assert_eq!(
        body.matches("signing_message_legacy").count(),
        2,
        "INC-I-176 M2 / F2 TRIPWIRE: `derive_maintainer_set` no longer calls \
         `signing_message_legacy` exactly twice (Add arm + Remove arm). Either an \
         arm was converted on its own — which would leave the other unbound, the \
         exact copy-paste hazard REQ-176-012 names — or the replay loop was \
         restructured. Re-derive the F2 argument; do not adjust the count to match."
    );

    // O6 / IP-T7 — the bound constructor has NOT appeared here.
    assert!(
        !body.contains("signing_message_at"),
        "INC-I-176 REQ-176-041 HAS LANDED: `derive_maintainer_set` now calls \
         `signing_message_at`.\n\n\
         That is the correct end state, not a defect — but it RETIRES the argument \
         this file exists to protect. The F2 deferral said the unconverted legacy \
         arms were safe because the function has zero production callers; with the \
         site converted, the reachability premise is no longer load-bearing.\n\n\
         Required action: verify the conversion binds BOTH arms to the chain-derived \
         `height` and the node's OWN genesis hash (never a network constant), then \
         DELETE this file and the deferral comment in derivation.rs. Do not leave \
         `tripwire_derive_maintainer_set_has_no_production_callers` standing as a \
         rule whose reason has expired."
    );
}
