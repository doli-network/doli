//! INC-I-215 M1 — RED tests for the staged-upgrade handoff (crates/updater).
//!
//! The node's systemd sandbox forbids it writing its own executable, so auto-update dies at
//! the final `write(2)`. M1 adds a probe ("can I install here at all?") and a stager that lays
//! a VERIFIED release into `{data_dir}/updates/` for a root helper. What is staged must be the
//! SIGNED TARBALL: the chain is SIGNATURES.json -> sha256(CHECKSUMS.txt) -> per-platform hash
//! -> .tar.gz (`install_gate.rs:16-21`, L4 at `:135-138`) and NO signed value covers the
//! extracted ELF, so a staged ELF leaves the root installer nothing to re-verify against.
//!
//! covers: crates/updater/src/staging.rs, crates/updater/src/fetch_verified.rs,
//! covers: crates/updater/src/install_gate.rs, crates/updater/src/lib.rs, crates/updater/src/apply.rs
//!
//! ===========================================================================
//! OUTPUT CONTRACT: fn target_dir_is_writable(target: &Path) -> bool
//!   O1: params — `target: &Path`, shared ref, no interior mutability. (n/a)
//!   O2: receiver — free fn, no `self`. (n/a)
//!   O3: return — bool; true IFF a file can be created AND removed in `target.parent()`.
//!   O4: fs / `target.parent()` — a uniquely named probe file is created and MUST be removed on
//!       every path; sorted listing before == after; an existing `target` keeps its bytes.
//!   O5: globals — none. O6: channels — none. Panics/`Err` — NEITHER is permitted.
//! PATHS: P1 parent writable -> true; P2 parent exists but denies create (EACCES) -> false;
//!        P3 parent missing -> false; P4 any other io error (EROFS on a read-only mount) -> false.
//! INPUT PARTITIONS:
//!   P1a: empty parent dir — listing empty before and after.
//!   P1b: parent holding `doli-node` + `doli` — listing AND target bytes unchanged.
//!   P1c: two probes back to back — unique names, both true, no residue (a fixed name would
//!        collide between the node and a concurrent helper).
//!   P2a: dir mode 0o555. EROFS arrives through the SAME `io::Error` channel, so special-casing
//!        `ErrorKind::PermissionDenied` (the mistake at `apply.rs:200-205`) fails this partition.
//!   P3a: target under a directory that does not exist — and the probe must not create it.
//! MATRIX: 2 outputs (O3, O4) x 5 partitions = 10 cells.
//!
//! OUTPUT CONTRACT: fn stage_release(staging_dir, version, tarball, checksums_body, signatures) -> Result<PathBuf>
//!   O1/O2: none — all params are shared refs, free fn.
//!   O3: return — Ok(`<staging_dir>/ready`) on success; Err on any io failure.
//!   O4: fs / `<staging_dir>` — after Ok it holds EXACTLY `release.tar.gz` (== `tarball`),
//!       `CHECKSUMS.txt` (== `checksums_body`), `SIGNATURES.json` (round-trips to `signatures`),
//!       `ready` (content == `version`), NO `*.tmp` residue; `ready` is published by temp+rename;
//!       nothing is written outside `staging_dir` (sandbox `ReadWritePaths`).
//! PATHS: P1 fresh dir (created); P2 dir staged with a DIFFERENT version (replaced); P3 same
//!        version (re-stage is idempotent).
//! INPUT PARTITIONS: P1a real gzip tarball with an ELF entry — the staged operand is still the
//!   .tar.gz; P2a stale X then Y>X — a node must never be stuck on X.
//! MATRIX: 2 outputs (O3, O4) x 3 partitions = 6 cells.
//!
//! OUTPUT CONTRACT: fn staged_ready_version(staging_dir) -> Option<String> / fn read_staged(dir) -> Result<StagedRelease>
//!   O3: `staged_ready_version` — Some(version) only when `ready` exists; None for a missing
//!       dir, an empty dir, or a dir whose marker was removed.
//!   O3: `read_staged` — Ok(StagedRelease{version, tarball, checksums_body, signatures}) equal
//!       byte-for-byte to what was staged; Err NAMING the missing file; Err on version mismatch.
//!   O4: fs — both are READ-ONLY; the staging dir listing is unchanged by either call.
//! PATHS: P1 complete staging; P2 missing ready / SIGNATURES.json / CHECKSUMS.txt / tarball;
//!        P3 dir does not exist; P4 ready/manifest version disagreement.
//! INPUT PARTITIONS: P2a-P2d one missing file each (four distinct error strings).
//! MATRIX: 2 outputs (O3, O4) x 7 partitions = 14 cells.
//!
//! OUTPUT CONTRACT: fn verify_release_artifact_bytes(version, checksums_body, tarball, signatures, root) -> Result<usize>
//!   O1/O2/O4/O5/O6: none — pure over shared refs (same shape as `verify_release_artifact`).
//!   O3: return — Ok(distinct signer count) or Err(one of the L1-L4 refusals).
//! PATHS: P1 self-consistent release -> Ok(n); P2 tarball altered -> Err(HashMismatch);
//!        P3 fewer distinct signers than the ROOT threshold -> Err(InsufficientSignatures).
//! INPUT PARTITIONS:
//!   P1a: 3 signatures under a 3-of-3 root — must equal `verify_release_artifact`'s verdict.
//!   P3a: 2 signatures under a threshold-3 root — refused.
//!   P3b: the SAME 2-signature manifest under a threshold-2 root — AUTHORISED. This is the
//!        REQ-215-017 falsifier: an implementation reading `REQUIRED_SIGNATURES` (=3) instead
//!        of `TrustRoot::threshold()` passes P1a and P3a and fails here.
//! MATRIX: 1 output (O3) x 4 partitions = 4 cells (x2 for the equivalence pair).
//!
//! OUTPUT CONTRACT: module budget / one-verifier structure (source text of 4 files)
//!   O3: per-file line counts + delegation markers; no runtime outputs. PATHS: P1 all four
//!   within budget AND apply.rs delegating. PARTITIONS: one — the file set is fixed.
//! MATRIX: 4 files x (budget + delegation) = 8 cells.
//! ===========================================================================

#![cfg(unix)]

mod common;

use common::*;

const APPLY_RS: &str = include_str!("../src/apply.rs");
const STAGING_RS: &str = include_str!("../src/staging.rs");
const FETCH_VERIFIED_RS: &str = include_str!("../src/fetch_verified.rs");
const INSTALL_GATE_RS: &str = include_str!("../src/install_gate.rs");

// ---------------------------------------------------------------------------
// REQ-215-001 — writability probe
// ---------------------------------------------------------------------------

// REQ-215-001 (Must) — Decision: a probe that answers "writable" on a sandboxed or read-only
// install target sends the node back into the in-process install that returns EROFS, which is
// the dead auto-update loop this incident is about; a false answer here silently disables staging.
#[test]
fn probe_reports_not_writable_for_a_read_only_directory() {
    let tmp = tempfile::tempdir().expect("tempdir");

    // P2a — EACCES: the install directory denies creation.
    let ro = tmp.path().join("ro");
    fs::create_dir(&ro).expect("create ro dir");
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o555)).expect("chmod 0555");
    let target = ro.join("doli-node");

    if running_as_root(&ro) {
        eprintln!("running as root: the EACCES partition is unobservable, asserting P3a only");
    } else {
        assert!(
            !target_dir_is_writable(&target),
            "a 0o555 install dir must report NOT writable; EROFS on a read-only mount arrives \
             through the same io::Error channel, so special-casing PermissionDenied \
             (apply.rs:200-205) is the failing implementation"
        );
        assert!(
            listing(&ro).is_empty(),
            "a refused probe must leave the dir untouched, found {:?}",
            listing(&ro)
        );
    }

    // P3a — the parent directory does not exist at all.
    let missing_parent = tmp.path().join("no-such-dir");
    assert!(
        !target_dir_is_writable(&missing_parent.join("doli-node")),
        "a target whose parent does not exist must report NOT writable, not panic"
    );
    assert!(
        !missing_parent.exists(),
        "the probe must never create the missing install directory"
    );

    fs::set_permissions(&ro, fs::Permissions::from_mode(0o755)).expect("restore mode");
}

// REQ-215-001 (Must) — Decision: a probe file left in /usr/bin (or a fixed name that two probes
// collide on) turns a read-only diagnostic into a writer that pollutes the install directory and
// can race a concurrent helper, so residue here is a real operational defect.
#[test]
fn probe_leaves_no_artifact_in_a_writable_directory() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();

    // P1a — empty directory.
    assert!(
        target_dir_is_writable(&dir.join("doli-node")),
        "a writable install dir must report writable"
    );
    assert!(
        listing(dir).is_empty(),
        "probe residue after P1a: {:?}",
        listing(dir)
    );

    // P1b — directory that already holds the binaries.
    fs::write(dir.join("doli-node"), b"old node binary").expect("write node");
    fs::write(dir.join("doli"), b"old cli binary").expect("write cli");
    let before = listing(dir);
    assert!(target_dir_is_writable(&dir.join("doli-node")));
    assert_eq!(
        before,
        listing(dir),
        "the directory listing must be identical before and after the probe"
    );
    assert_eq!(
        fs::read(dir.join("doli-node")).expect("read node"),
        b"old node binary".to_vec(),
        "the probe must never write through the target path itself"
    );

    // P1c — two consecutive probes must both succeed and collide on nothing.
    assert!(target_dir_is_writable(&dir.join("doli-node")));
    assert!(target_dir_is_writable(&dir.join("doli-node")));
    assert_eq!(before, listing(dir), "residue after repeated probes");
    assert!(
        !listing(dir).iter().any(|n| n.contains("probe")),
        "a probe artifact survived: {:?}",
        listing(dir)
    );
}

// ---------------------------------------------------------------------------
// REQ-215-002 — staging the verified tarball
// ---------------------------------------------------------------------------

// REQ-215-002 (Must) — Decision: a missing or misnamed staged file makes the root .path unit fire
// on an incomplete handoff, and the constants are the contract M2's unit file and M3's
// --from-staged reader are written against, so a rename here breaks the installer silently.
#[test]
fn stage_writes_tarball_manifest_checksums_and_ready() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let data_dir = tmp.path();

    assert_eq!(STAGING_SUBDIR, "updates");
    assert_eq!(READY_MARKER, "ready");
    assert_eq!(STAGED_SIGNATURES, "SIGNATURES.json");
    assert_eq!(STAGED_CHECKSUMS, "CHECKSUMS.txt");
    assert_eq!(STAGED_TARBALL, "release.tar.gz");

    let dir = staging_dir(data_dir);
    assert_eq!(
        dir,
        data_dir.join(STAGING_SUBDIR),
        "staging must live inside data_dir, the only sandbox-writable path"
    );
    assert!(!dir.exists(), "precondition: nothing staged yet");

    let f = fixture(
        "6.30.0",
        &tarball_bytes("doli-node", &elf_payload()),
        &[&kp(1), &kp(2), &kp(3)],
    );
    let ready = stage(&dir, &f);

    assert_eq!(ready, dir.join(READY_MARKER), "returned marker path");
    assert!(ready.exists(), "the ready marker must exist after staging");
    assert_eq!(
        fs::read(dir.join(STAGED_TARBALL)).expect("read tarball"),
        f.tarball,
        "the staged tarball must be byte-identical to the verified bytes"
    );
    assert_eq!(
        fs::read(dir.join(STAGED_CHECKSUMS)).expect("read checksums"),
        f.checksums_body,
        "the staged CHECKSUMS.txt must be the bytes the signatures cover"
    );
    let parsed: SignaturesFile =
        serde_json::from_slice(&fs::read(dir.join(STAGED_SIGNATURES)).expect("read manifest"))
            .expect("SIGNATURES.json must be valid serde JSON for the CLI to read back");
    assert_eq!(parsed.version, f.signatures.version);
    assert_eq!(parsed.checksums_sha256, f.signatures.checksums_sha256);
    assert_eq!(
        parsed
            .signatures
            .iter()
            .map(|s| (s.public_key.clone(), s.signature.clone()))
            .collect::<Vec<_>>(),
        f.signatures
            .signatures
            .iter()
            .map(|s| (s.public_key.clone(), s.signature.clone()))
            .collect::<Vec<_>>(),
        "every maintainer signature must survive the round trip"
    );
    assert_eq!(
        fs::read_to_string(&ready).expect("read ready").trim(),
        "6.30.0",
        "the ready marker names the staged version"
    );

    assert_eq!(
        listing(&dir),
        staged_names(),
        "exactly the four staged files, no .tmp residue"
    );
    assert_eq!(
        listing(data_dir),
        vec![STAGING_SUBDIR.to_string()],
        "nothing may be written outside {{data_dir}}/updates"
    );
    assert_eq!(
        sha256_hex(&fs::read(dir.join(STAGED_CHECKSUMS)).expect("read checksums")),
        parsed.checksums_sha256,
        "the staged CHECKSUMS.txt must hash to the value the manifest was signed over"
    );
}

// REQ-215-002 (Must) — Decision: a `ready` opened for write at its final name is observable by
// the root .path unit while still empty or half-written, which installs a truncated handoff; only
// temp+rename makes the marker atomic for the watcher.
#[test]
fn ready_marker_is_created_by_rename_not_in_place_write() {
    let code: Vec<&str> = STAGING_RS
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("//"))
        .collect();

    let mentions_ready =
        |l: &str| l.contains("READY_MARKER") || l.contains("ready_path") || l.contains("\"ready\"");
    let writes = ["File::create", "fs::write", "write_all", "OpenOptions"];

    let in_place: Vec<&&str> = code
        .iter()
        .filter(|l| mentions_ready(l) && writes.iter().any(|w| l.contains(w)) && !l.contains("tmp"))
        .collect();
    assert!(
        in_place.is_empty(),
        "the ready marker must only ever be the DESTINATION of a rename; these lines open it \
         for writing at its final name: {in_place:?}"
    );
    assert!(
        code.iter().any(|l| l.contains("rename")),
        "staging must publish its files with fs::rename"
    );
    assert!(
        code.iter()
            .any(|l| l.contains("sync_all") || l.contains("sync_data")),
        "the tarball and both manifests must be fsynced before the marker is renamed in"
    );

    // The behavioural half: a completed staging leaves no half-written temp file behind.
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = staging_dir(tmp.path());
    let f = fixture("6.30.0", &tarball_bytes("doli-node", b"payload"), &[&kp(1)]);
    let ready = stage(&dir, &f);
    assert!(
        !listing(&dir).iter().any(|n| n.ends_with(".tmp")),
        "temp files must be renamed away, found {:?}",
        listing(&dir)
    );
    assert_eq!(
        fs::read_to_string(&ready).expect("read ready").trim(),
        "6.30.0",
        "the published marker carries the version, not an empty placeholder"
    );
}

// REQ-215-002 (Must) — Decision: staging the extracted ELF leaves the root installer with no
// signed operand to re-verify (no signature covers the extracted binary), so INV-REL-001 would
// hold up to the node and stop there — the whole point of the handoff.
#[test]
fn staged_artifact_is_the_tarball_not_the_extracted_binary() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = staging_dir(tmp.path());
    let elf = elf_payload();
    let f = fixture(
        "6.30.0",
        &tarball_bytes("doli-node", &elf),
        &[&kp(1), &kp(2), &kp(3)],
    );
    stage(&dir, &f);

    let staged = fs::read(dir.join(STAGED_TARBALL)).expect("read staged artifact");
    assert_eq!(
        staged, f.tarball,
        "staged bytes must be the .tar.gz operand"
    );
    assert_eq!(
        &staged[..2],
        &[0x1f, 0x8b],
        "staged artifact must be a gzip stream, not a bare binary"
    );
    assert_eq!(
        sha256_hex(&staged),
        platform_hash_in(&f.checksums_body),
        "the staged bytes must hash to the per-platform value CHECKSUMS.txt names (L4)"
    );

    let root = root_of(&[&kp(1), &kp(2), &kp(3)], 3);
    assert_eq!(
        verify_release_artifact_bytes(&f.version, &f.checksums_body, &staged, &f.signatures, &root)
            .expect("the staged operand must still satisfy the full L1-L4 chain offline"),
        3
    );

    for name in listing(&dir) {
        let bytes = fs::read(dir.join(&name)).expect("read staged file");
        assert_ne!(
            bytes, elf,
            "{name} is the extracted binary, not a signed artifact"
        );
        assert!(
            !bytes.starts_with(b"\x7fELF"),
            "{name} starts with the ELF magic: no signed value covers an extracted binary"
        );
    }
}

// ---------------------------------------------------------------------------
// REQ-215-005 — idempotency
// ---------------------------------------------------------------------------

// REQ-215-005 (Must) — Decision: without a marker read that answers "already staged", the
// ActivateEnforcement transition re-downloads and re-stages the same release on every restart,
// and a stale marker for an older version would pin the node to a superseded build.
#[test]
fn staged_ready_for_same_version_reports_already_staged() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = staging_dir(tmp.path());

    assert_eq!(
        staged_ready_version(&dir),
        None,
        "a staging dir that does not exist is not staged"
    );
    fs::create_dir_all(&dir).expect("create staging dir");
    assert_eq!(
        staged_ready_version(&dir),
        None,
        "an empty staging dir is not staged"
    );

    let x = fixture(
        "6.30.0",
        &tarball_bytes("doli-node", b"release X"),
        &[&kp(1), &kp(2), &kp(3)],
    );
    stage(&dir, &x);
    assert_eq!(
        staged_ready_version(&dir).as_deref(),
        Some("6.30.0"),
        "a ready marker for this version must short-circuit before any download"
    );

    let y = fixture(
        "6.31.0",
        &tarball_bytes("doli-node", b"release Y"),
        &[&kp(1), &kp(2), &kp(3)],
    );
    stage(&dir, &y);
    assert_eq!(
        staged_ready_version(&dir).as_deref(),
        Some("6.31.0"),
        "a newer approved release must replace the stale staging"
    );
    assert_eq!(
        fs::read(dir.join(STAGED_TARBALL)).expect("read tarball"),
        y.tarball,
        "the replaced staging must carry Y's artifact, not X's"
    );
    assert_eq!(
        listing(&dir),
        staged_names(),
        "no residue from the X staging"
    );

    let staged: updater::StagedRelease =
        read_staged(&dir).expect("the replaced staging reads back");
    assert_eq!(staged.version, "6.31.0");
    assert_eq!(staged.signatures.version, "6.31.0");
    assert_eq!(staged.checksums_body, y.checksums_body);

    fs::remove_file(dir.join(READY_MARKER)).expect("remove marker");
    assert_eq!(
        staged_ready_version(&dir),
        None,
        "without the marker the dir is a torn staging, not a staged release"
    );
}

// ---------------------------------------------------------------------------
// REQ-215-009 groundwork — the reader refuses an incomplete staging
// ---------------------------------------------------------------------------

// REQ-215-009 (Must) — Decision: a reader that returns a generic io error leaves the operator
// with a root helper that failed for an unnameable reason, and one that accepts a
// ready/manifest version disagreement installs a build nobody approved for that marker.
#[test]
fn read_staged_names_the_missing_file_and_rejects_a_ready_version_mismatch() {
    let tmp = tempfile::tempdir().expect("tempdir");

    // P1 — the complete staging reads back exactly what was written.
    let base = tmp.path().join("complete");
    let f = fixture(
        "6.30.0",
        &tarball_bytes("doli-node", &elf_payload()),
        &[&kp(1), &kp(2), &kp(3)],
    );
    stage(&base, &f);
    let staged = read_staged(&base).expect("a complete staging must read back");
    assert_eq!(staged.version, "6.30.0");
    assert_eq!(staged.tarball, f.tarball);
    assert_eq!(staged.checksums_body, f.checksums_body);
    assert_eq!(
        staged.signatures.checksums_sha256,
        f.signatures.checksums_sha256
    );
    assert_eq!(staged.signatures.signatures.len(), 3);
    assert_eq!(
        listing(&base),
        staged_names(),
        "read_staged must not mutate the dir"
    );

    // P2a-P2d — each missing file is named in the refusal.
    for (removed, needle) in [
        (STAGED_CHECKSUMS, STAGED_CHECKSUMS),
        (STAGED_SIGNATURES, STAGED_SIGNATURES),
        (STAGED_TARBALL, STAGED_TARBALL),
        (READY_MARKER, READY_MARKER),
    ] {
        let dir = tmp.path().join(format!("missing-{removed}"));
        stage(&dir, &f);
        fs::remove_file(dir.join(removed)).expect("remove staged file");
        let err = read_staged(&dir)
            .err()
            .unwrap_or_else(|| panic!("a staging without {removed} must be refused"));
        assert!(
            err.to_string().contains(needle),
            "the refusal must name the missing file {needle}, got: {err}"
        );
    }

    // P3 — a staging directory that does not exist.
    let absent = tmp.path().join("never-staged");
    let err = read_staged(&absent).expect_err("a non-existent staging dir must be refused");
    assert!(
        err.to_string().contains(READY_MARKER) || err.to_string().contains("never-staged"),
        "the refusal must name what is missing, got: {err}"
    );

    // P4 — the marker and the manifest disagree about the version.
    let mismatched = tmp.path().join("mismatch");
    stage(&mismatched, &f);
    fs::write(mismatched.join(READY_MARKER), b"6.31.0\n").expect("rewrite marker");
    let err = read_staged(&mismatched)
        .expect_err("a ready marker that disagrees with the manifest must be refused");
    let text = err.to_string();
    assert!(
        text.contains("6.31.0") && text.contains("6.30.0"),
        "the refusal must show both versions, got: {text}"
    );
}

// ---------------------------------------------------------------------------
// REQ-215-017 — ONE verifier
// ---------------------------------------------------------------------------

// REQ-215-017 (Must) — Decision: a second copy of the L1-L4 chain for the staged path is how
// INC-I-172 F1/F2 drifted apart, and a threshold read from REQUIRED_SIGNATURES instead of the
// resolved trust root ignores the on-chain maintainer set the operator actually approved with.
#[test]
fn verify_release_artifact_bytes_matches_verify_release_artifact_on_the_same_inputs() {
    let f = fixture(
        "6.30.0",
        &tarball_bytes("doli-node", &elf_payload()),
        &[&kp(1), &kp(2), &kp(3)],
    );
    let root = root_of(&[&kp(1), &kp(2), &kp(3)], 3);

    // P1a — identical verdict from both entry points.
    let via_info = verify_release_artifact(&f.info, &f.tarball, &f.signatures, &root)
        .expect("the existing gate authorises a self-consistent release");
    let via_bytes = verify_release_artifact_bytes(
        &f.version,
        &f.checksums_body,
        &f.tarball,
        &f.signatures,
        &root,
    )
    .expect("the bytes entry point must reach the same verdict");
    assert_eq!(
        via_bytes, via_info,
        "both entry points must report the same distinct-signer count"
    );
    assert_eq!(via_bytes, 3);

    // P2 — one flipped tarball byte is refused by both, with the same error kind.
    let mut tampered = f.tarball.clone();
    let mid = tampered.len() / 2;
    tampered[mid] ^= 0x01;
    let err_bytes = verify_release_artifact_bytes(
        &f.version,
        &f.checksums_body,
        &tampered,
        &f.signatures,
        &root,
    )
    .expect_err("a tampered tarball must never authorise an install");
    assert!(
        matches!(err_bytes, UpdateError::HashMismatch { .. }),
        "expected HashMismatch, got {err_bytes}"
    );
    let err_info = verify_release_artifact(&f.info, &tampered, &f.signatures, &root)
        .expect_err("the existing gate refuses it too");
    assert_eq!(
        std::mem::discriminant(&err_bytes),
        std::mem::discriminant(&err_info),
        "the two entry points must refuse for the same reason"
    );

    // P3a — two signatures under a threshold-3 root are sub-threshold.
    let two = fixture(
        "6.30.0",
        &tarball_bytes("doli-node", &elf_payload()),
        &[&kp(1), &kp(2)],
    );
    let root_of_3 = root_of(&[&kp(1), &kp(2), &kp(3)], 3);
    let err = verify_release_artifact_bytes(
        &two.version,
        &two.checksums_body,
        &two.tarball,
        &two.signatures,
        &root_of_3,
    )
    .expect_err("2 signatures must not satisfy a 3-of-5 style root");
    assert!(
        matches!(
            err,
            UpdateError::InsufficientSignatures {
                found: 2,
                required: 3
            }
        ),
        "expected InsufficientSignatures 2/3, got {err}"
    );

    // P3b — the SAME manifest under a threshold-2 root IS authorised: the threshold comes from
    // the resolved root, never from the REQUIRED_SIGNATURES constant.
    let root_threshold_2 = root_of(&[&kp(1), &kp(2), &kp(3)], 2);
    assert_eq!(
        verify_release_artifact_bytes(
            &two.version,
            &two.checksums_body,
            &two.tarball,
            &two.signatures,
            &root_threshold_2,
        )
        .expect("a threshold-2 root must authorise 2 distinct signers"),
        2,
        "the threshold must be read from TrustRoot::threshold()"
    );
}

// ---------------------------------------------------------------------------
// REQ-215-016 — module budget and the lifted fetch/verify prefix
// ---------------------------------------------------------------------------

// REQ-215-016 (Must) — Decision: apply.rs shrinking by DELETION rather than by lifting the
// fetch/verify prefix would drop the TOCTOU CHECKSUMS check from the auto path, so the budget
// and the delegation marker have to be asserted together to tell the two apart.
#[test]
fn apply_rs_is_within_the_module_budget() {
    for (name, src) in [
        ("apply.rs", APPLY_RS),
        ("staging.rs", STAGING_RS),
        ("fetch_verified.rs", FETCH_VERIFIED_RS),
        ("install_gate.rs", INSTALL_GATE_RS),
    ] {
        let lines = src.lines().count();
        assert!(
            lines <= 500,
            "{name} is {lines} lines, over the 500-line module budget (Rule 19)"
        );
    }

    assert!(
        FETCH_VERIFIED_RS.contains("pub async fn fetch_verified_release("),
        "the lifted fetch/verify prefix must be the exported entry point M2 calls"
    );
    assert!(
        APPLY_RS.contains("fetch_verified_release"),
        "auto_apply_from_github must DELEGATE to the lifted prefix; apply.rs shrinking without \
         this marker means the verify chain was deleted, not moved"
    );
    for (name, src) in [
        ("staging.rs", STAGING_RS),
        ("fetch_verified.rs", FETCH_VERIFIED_RS),
        ("install_gate.rs", INSTALL_GATE_RS),
    ] {
        assert!(
            !src.contains("REQUIRED_SIGNATURES"),
            "{name} must take its threshold from TrustRoot::threshold(), not the constant"
        );
    }
}
