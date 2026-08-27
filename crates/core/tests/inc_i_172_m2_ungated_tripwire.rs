//! INC-I-172 M2 review **F8** — tripwires for the ONE deliberately UNGATED
//! change.
//!
//! What is ungated, and why that was allowed
//! -----------------------------------------
//! `MaintainerSet::calculate_threshold(0)` returns `MAINTAINER_THRESHOLD` (was
//! `0`), and every verifier short-circuits `false` on `!is_authorizable()` — at
//! **every height**, including below
//! `maintainer_derivation_activation_height`. That is a real pre-activation
//! behavior change: an empty set used to satisfy `valid_count (0) >= threshold
//! (0)` and therefore ACCEPTED a zero-signature `AddMaintainer` /
//! `ProtocolActivation` (FM-02 / AUDIT-P1-010). It now refuses.
//!
//! It ships ungated because the outcome is not consensus-visible **in this
//! tree**, on three legs (M2 dev notes §3.2, re-verified by the reviewer):
//!
//! 1. `ChainState::serialize_canonical` is a fixed-size buffer that contains
//!    neither `active_protocol_version` nor `pending_protocol_activation`, so an
//!    activation accept/reject divergence cannot move the state root.
//! 2. `is_protocol_active` has ZERO production callers, so
//!    `active_protocol_version` currently gates nothing.
//! 3. `process_transaction_governance` never rejects a block; every failure path
//!    only logs.
//!
//! Why this file exists
//! --------------------
//! Legs 1 and 2 are **facts about the tree, not invariants**. The day anything
//! wires `is_protocol_active` into a consensus rule, or adds an activation field
//! to the canonical chain-state encoding, the argument expires and the ungated
//! refusal becomes a consensus-visible change below the gate — retroactively,
//! over history the gate already governs. The dev notes said exactly that and
//! attached no mechanism to it. These tests are the mechanism: they FAIL when
//! either leg stops being true, so the argument cannot rot in silence.
//!
//! Recorded as `INV-AUTH-001` in `.omega/memory.db` with these two tests linked
//! as its regression tests.
//!
//! Scope note: a green run here proves the SAFETY ARGUMENT is still standing. It
//! does NOT prove the ungated change is harmless in some tree where the legs
//! have moved — at that point the change must be RE-DERIVED, not re-assumed.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT
//! ---------------------------------------------------------------------------
//! These are STRUCTURAL tests: the unit under test is the source tree itself,
//! and the "functions" are the two properties the M2 safety argument rests on.
//!
//! Properties under test:
//!   G1: the CALLER SET of `doli_core::consensus::is_protocol_active` restricted
//!       to production (non-test) code
//!   G2: the encoded FIELD SET of `storage::ChainState::serialize_canonical`
//!
//! OUTPUTS
//!   O1 (derived value)  `offenders` — production, non-allowlisted files that
//!      name `is_protocol_active`. The assertion subject for G1.
//!   O2 (instrument)     the number of `.rs` files the walker visited — an
//!      anti-vacuity instrument, NOT a property of the system under test
//!   O3 (instrument)     `definition_seen` — whether the scan could see the
//!      symbol's definition site at all (anti-vacuity)
//!   O4 (derived value)  the brace-matched body text of `serialize_canonical`
//!   O5 (derived value)  presence of each name in `ACTIVATION_FIELDS` within O4.
//!      The assertion subject for G2.
//!   O6 (mutable params)       — NONE; both tests are read-only over the tree
//!   O7 (receiver mutation)    — NONE; free functions
//!   O8 (persistent store)     — NONE; no file is written
//!
//! PATHS
//!   PT-clean   — the leg still holds; the tripwire stays green
//!   PT-tripped — the leg has moved; the tripwire fails with the re-derivation
//!                instruction
//!   PT-broken  — the scanner itself stopped working (moved symbol, failed
//!                extraction, empty walk); must FAIL, never pass vacuously
//!
//! INPUT PARTITIONS  (the input is the file tree; partitions are file classes)
//!   IP-T1  production source with no mention of the symbol   -> ignored
//!   IP-T2  allowlisted file: the definition
//!          (`crates/core/src/consensus/constants.rs`) and the crate re-export
//!          (`crates/core/src/lib.rs`)                        -> ignored [PT-clean]
//!   IP-T3  test-only mention: a file under `*/tests/`, named `tests.rs` /
//!          `tests_*` / `test_*`, or inside a trailing inline `#[cfg(test)]`
//!          module                                            -> ignored [PT-clean]
//!   IP-T4  mention inside a `//` comment or doc comment       -> ignored [PT-clean]
//!   IP-T5  production, non-allowlisted, non-comment mention   -> O1 non-empty
//!                                                                [PT-tripped]
//!   IP-T6  `serialize_canonical` body WITHOUT any activation field
//!                                                             -> O5 all absent
//!                                                                [PT-clean]
//!   IP-T7  `serialize_canonical` body WITH an activation field -> O5 present
//!                                                                [PT-tripped]
//!   IP-T8  scanner degeneracy: <=100 files walked, definition site absent, or
//!          an extracted body missing `best_height`/`genesis_hash`
//!                                                             -> [PT-broken]
//!
//! MATRIX
//!   O1 x {IP-T1, IP-T2, IP-T3, IP-T4, IP-T5} = 1 assertion covering all five
//!        classes (the offender list is the quotient of the whole tree by them)
//!   O2 x {IP-T8}                             = 1 assertion
//!   O3 x {IP-T8}                             = 1 assertion
//!   O4 x {IP-T8}                             = 2 assertions (extraction sanity)
//!   O5 x {IP-T6, IP-T7}                      = 2 assertions (one per field)
//!   O6/O7/O8 — structurally absent; both tests only read.
//!
//! ANTI-VACUITY
//!   O2/O3 exist solely so that a rename, a move, or a broken walk FAILS
//!   (PT-broken) instead of yielding an empty offender list that looks green.
//!   O4's `best_height`/`genesis_hash` check plays the same role for G2: without
//!   it, a mis-extracted (e.g. empty) body would trivially satisfy IP-T6.

use std::fs;
use std::path::{Path, PathBuf};

/// Files permitted to mention `is_protocol_active`: its definition and the
/// crate re-export. Anything else is a caller and trips the wire.
const IS_PROTOCOL_ACTIVE_ALLOWLIST: &[&str] = &[
    "crates/core/src/consensus/constants.rs",
    "crates/core/src/lib.rs",
];

/// Fields that must stay OUT of the canonical chain-state encoding.
const ACTIVATION_FIELDS: &[&str] = &["active_protocol_version", "pending_protocol_activation"];

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

/// Strip `//` line comments so a doc comment naming the symbol is not counted
/// as a caller. Block comments are not used for this in tree.
fn strip_line_comments(src: &str) -> String {
    src.lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// LEG 2 TRIPWIRE. O1, O2, O3 x {IP-T1..IP-T5, IP-T8}.
///
/// Fails when `is_protocol_active` gains a production caller. The moment it has
/// one, `active_protocol_version` gates something, and the ungated
/// `is_authorizable` / `calculate_threshold(0)` refusal becomes
/// consensus-visible BELOW `maintainer_derivation_activation_height`.
#[test]
fn tripwire_is_protocol_active_has_no_production_callers() {
    let root = repo_root();
    let mut files = Vec::new();
    rust_files(&root.join("crates"), &mut files);
    rust_files(&root.join("bins"), &mut files);

    // O2 / IP-T8 — ANTI-VACUITY: the walk actually found the tree.
    assert!(
        files.len() > 100,
        "scanner is broken: only {} .rs files found under {}. A vacuous pass \
         here would silently retire the tripwire.",
        files.len(),
        root.display()
    );

    let mut definition_seen = false;
    let mut offenders: Vec<String> = Vec::new();

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
        if !code.contains("is_protocol_active") {
            continue;
        }
        if rel == "crates/core/src/consensus/constants.rs" {
            definition_seen = true;
        }
        if !IS_PROTOCOL_ACTIVE_ALLOWLIST.contains(&rel.as_str()) && !is_test_file(&rel) {
            offenders.push(rel);
        }
    }

    // O3 / IP-T8 — ANTI-VACUITY: the scan can see the symbol at all. If the
    // definition moves or is renamed, this fails instead of passing on an empty
    // search.
    assert!(
        definition_seen,
        "scanner is broken: `is_protocol_active` was not found in \
         crates/core/src/consensus/constants.rs. If it moved, update \
         IS_PROTOCOL_ACTIVE_ALLOWLIST and re-derive the M2 ungated-change \
         safety argument — do not just silence this test."
    );

    // O1 — the property itself.
    assert!(
        offenders.is_empty(),
        "INC-I-172 M2 / INV-AUTH-001 TRIPWIRE: `is_protocol_active` now has \
         production caller(s): {:?}\n\n\
         The M2 safety argument for the UNGATED `calculate_threshold(0)` / \
         `is_authorizable()` refusal rests on `active_protocol_version` gating \
         NOTHING. With a caller it gates something, so the pre-activation \
         behavior change (an empty set used to ACCEPT a zero-signature \
         AddMaintainer / ProtocolActivation, and now refuses) becomes \
         consensus-visible BELOW maintainer_derivation_activation_height — \
         retroactively, over history that gate already governs.\n\n\
         Required action: re-derive the argument in \
         crates/core/src/network_params/mod.rs and \
         docs/.workflow/inc-i-172-M2-dev-notes.md §3.2, and decide whether the \
         refusal now needs its OWN activation height. Adding the new caller to \
         the allowlist is NOT the fix.",
        offenders
    );
}

/// LEG 1 TRIPWIRE. O4, O5 x {IP-T6, IP-T7, IP-T8}.
///
/// Fails when the canonical chain-state encoding starts carrying a
/// protocol-activation field. `test_serialize_canonical_fixed_size` in
/// `crates/storage` already fails if the buffer LENGTH changes; this one names
/// the specific fields, so a same-length substitution is caught too.
#[test]
fn tripwire_serialize_canonical_excludes_protocol_activation_fields() {
    let root = repo_root();
    let path = root.join("crates/storage/src/chain_state.rs");
    let src = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "scanner is broken: cannot read {} ({e}). Do not silence this — \
             re-derive the M2 ungated-change safety argument.",
            path.display()
        )
    });

    let start = src
        .find("fn serialize_canonical")
        .expect("scanner is broken: `fn serialize_canonical` not found in chain_state.rs");
    let body_open = src[start..]
        .find('{')
        .expect("scanner is broken: no body for serialize_canonical")
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
    let body = &src[body_open..=end];

    // O4 / IP-T8 — ANTI-VACUITY: the extracted body really is the encoder. If
    // this fails the extraction is wrong and the field check below would pass
    // vacuously.
    assert!(
        body.contains("best_height") && body.contains("genesis_hash"),
        "scanner is broken: the extracted serialize_canonical body does not \
         contain best_height/genesis_hash, so it is not the canonical encoder. \
         Fix the extraction before trusting the result."
    );

    // O5 — the property itself.
    for field in ACTIVATION_FIELDS {
        assert!(
            !body.contains(field),
            "INC-I-172 M2 / INV-AUTH-001 TRIPWIRE: \
             `ChainState::serialize_canonical` now encodes `{field}`.\n\n\
             The state root is H(H(chain_state) || H(utxo) || H(producer_set)), \
             so a ProtocolActivation accept/reject divergence is now \
             STATE-ROOT-VISIBLE. The UNGATED `calculate_threshold(0)` / \
             `is_authorizable()` refusal changes that acceptance below \
             maintainer_derivation_activation_height, which now forks the chain \
             over history the gate already governs.\n\n\
             Required action: this needs its own activation height. Re-derive \
             the argument in docs/.workflow/inc-i-172-M2-dev-notes.md §3.2 \
             before shipping."
        );
    }
}
