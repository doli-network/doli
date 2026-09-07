// INC-I-215 M3 — `doli upgrade --from-staged <DIR>`: the privileged half of the staged
// handoff. M2 made an unwritable-target node stage a VERIFIED release into
// `{data_dir}/updates/`; nothing installs it yet. This file is the contract for the root
// process that does, and its whole value is that it re-verifies rather than trusting the
// staging dir (TB-1: written by `doli`, read by root).
//
// STATE: RED. `--from-staged` does not exist, so clap refuses the flag; the structural
// tests read `bins/cli/src/cmd_upgrade_staged.rs`, which does not exist either.
//
// ####################################################################################
// # SAFETY HAZARD — A CARELESS TEST HERE BOUNCES THE USER'S 18-NODE LOCAL TESTNET     #
// ####################################################################################
// `upgrade_restart.rs::restart_doli_service` on macOS enumerates `launchctl list`,
// filters EVERY label containing "doli", and `launchctl kickstart -k`s all of them. This
// machine runs an 18-node testnet under launchd. Every invocation below therefore goes
// through `inc_i_215_fixture::doli_from_staged`, which (1) passes `--service <fake>` so
// the CLI takes `restart_specific_service` (a `sudo systemctl …` shell-out) and never the
// launchd/pgrep tiers, and (2) prefixes PATH with a shim dir holding an inert `sudo` and
// `systemctl`. Refusal paths never reach a restart, but they are shimmed too. NEVER spawn
// the binary for this feature any other way.
//
// ============================================================================
// OUTPUT CONTRACT: fn cmd_upgrade_from_staged(dir, yes, doli_node_path, service,
//                                             data_dir, network)
//   Reached only across the process boundary: `doli-cli` is bin-only for this path, so
//   every observable is read from the real binary (env!("CARGO_BIN_EXE_doli")).
// OUTPUTS (no mutable params, no receiver mutation, no channels)
//   O1 = process exit status. Zero ONLY when the staged release verified, installed and
//        restarted; non-zero on every refusal.
//   O2 = operator text on stdout/stderr: the trust-root banner (provenance, key count,
//        threshold, data dir, on-chain last_derived_height), the distinct-signer count vs
//        threshold, and the reason for any refusal (which file, which layer, which
//        version).
//   O3 = filesystem effects, three independent cells:
//        O3a <node target> bytes — the staged payload on success, byte-identical to the
//            pre-existing binary on EVERY refusal (INC-I-153: never unlink on failure).
//        O3b <node target>.backup — exists holding the OLD bytes after a real install;
//            must NOT exist when nothing was installed.
//        O3c staging dir / marker — REMOVED on success; kept with `ready` renamed to
//            `ready.rejected` on refusal (evidence, REQ-215-007); `ready` renamed to
//            `ready.installed` when the install succeeded but the restart did not. No
//            path may leave `ready` in place: M4 adds a systemd `.path` unit with
//            `PathExists=<DIR>/ready`, and a surviving marker retriggers it in a tight
//            loop until TriggerLimit fails the unit.
//   O4 = the running services. Constrained to the fake unit by --service + PATH shims.
// PATHS
//   P1 success: complete staging, 3-of-5 signatures, newer version, writable target
//   P2 incomplete staging: ready / SIGNATURES.json / CHECKSUMS.txt / tarball missing, or
//      <DIR> absent
//   P3 marker-vs-manifest version disagreement (either direction)
//   P4 L1/L3 replay: signatures genuine, but made over a different version
//   P5 L2: CHECKSUMS.txt is not the file the signatures cover
//   P6 L3: fewer distinct signers than the resolved threshold
//   P7 L4: the tarball is not what the verified CHECKSUMS.txt names
//   P8 downgrade: staged version not newer than updater::current_version()
// INPUT PARTITIONS
//   P1a 3 of 5 signers, version 99.0.0 — signers == threshold exactly, so the printed
//       count is the threshold and an off-by-one in either direction is visible
//   P2a-P2e one absent file per sub-case; the message must name the file, not just fail
//   P3a ready newer than the manifest / P3b manifest newer than the ready marker — the
//       two directions catch a "fix" that trusts one side and ignores the other
//   P4a manifest.version rewritten forward over signatures made for 98.0.0
//   P6a 2 of 5 signers under threshold 3 — one short, the closest failing input
//   P7a exactly one flipped byte in the tarball — the smallest tamper that must still fail
//   P8a staged version == current_version() (equal, not lower: the boundary case)
// MATRIX: O1,O2,O3a,O3b,O3c x {P1a,P2a..e,P3a,P3b,P4a,P5,P6a,P7a,P8a} — the refusal
//   partitions share one assertion helper (assert_nothing_installed) so no cell is
//   silently skipped; O3c is additionally swept by from_staged_never_leaves_the_ready_marker.
//
// AUTHORISES EDITS TO: bins/cli/src/cmd_upgrade_staged.rs (new), bins/cli/src/cmd_upgrade.rs,
//                      bins/cli/src/commands.rs, bins/cli/src/main.rs, bins/cli/src/lib.rs

use std::path::{Path, PathBuf};
use std::process::Output;

use crate::inc_i_215_fixture::*;

/// `<tmp>/updates`, `<tmp>/data`, `<tmp>/bin/doli-node` (holding OLD_NODE_BYTES).
fn layout(tmp: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let staging = tmp.join("updates");
    let data_dir = tmp.join("data");
    let target = tmp.join("bin").join("doli-node");
    write_on_chain_root(&data_dir, &members(), ON_CHAIN_HEIGHT);
    write_install_target(&target);
    (staging, data_dir, target)
}

fn backup_of(target: &Path) -> PathBuf {
    PathBuf::from(format!("{}.backup", target.display()))
}

/// O1 + O3a + O3b for every refusal partition: non-zero exit, the pre-existing binary
/// byte-identical, and no backup — a `.backup` without an install means the target was
/// moved aside for a release that was never authorised.
fn assert_nothing_installed(out: &Output, target: &Path, case: &str) {
    let text = combined(out);
    assert!(
        !out.status.success(),
        "{case}/O1: a refused staging must exit non-zero. output: {text}"
    );
    assert_eq!(
        std::fs::read(target).expect("the install target must still exist"),
        OLD_NODE_BYTES,
        "{case}/O3a: the pre-existing binary must be byte-identical after a refusal \
         (INC-I-153: a refused install may never unlink or replace the target). output: {text}"
    );
    assert!(
        !backup_of(target).exists(),
        "{case}/O3b: no `.backup` may be created when nothing was installed. output: {text}"
    );
}

// REQ-215-006 — Decision: whether the root installer actually swaps the binary and keeps a
// recoverable copy; a green here that skipped the backup leaves a bricked host with no way back.
#[test]
fn from_staged_installs_and_reports_the_distinct_signer_count() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (staging, data_dir, target) = layout(tmp.path());
    write_good_staging(&staging);

    let out = doli_from_staged(tmp.path(), &staging, &data_dir, &target, &[]);
    let text = combined(&out);

    assert!(
        out.status.success(),
        "P1a/O1: a complete 3-of-5 staging for v{STAGED_VERSION} under a 5-member root at \
         threshold 3 must install and exit 0. exit={:?} output: {text}",
        out.status.code()
    );
    assert_eq!(
        std::fs::read(&target).expect("read install target"),
        NEW_NODE_BYTES,
        "P1a/O3a: the target must hold the payload extracted from the STAGED tarball. \
         output: {text}"
    );
    assert_eq!(
        std::fs::read(backup_of(&target)).expect("P1a/O3b: <target>.backup must be created"),
        OLD_NODE_BYTES,
        "P1a/O3b: the backup must hold the bytes that were replaced. `backup_current()` \
         backs up the RUNNING binary, which on this path is the CLI, not the node — using \
         it would leave the node with no rollback copy. output: {text}"
    );
    assert!(
        !staging.exists(),
        "P1a/O3c: the staging dir must be removed after a successful install + restart, or \
         M4's `.path` unit retriggers on the surviving marker. output: {text}"
    );
    let lower = text.to_lowercase();
    assert!(
        lower.contains("3 distinct"),
        "P1a/O2: the run must report the DISTINCT signer count it verified (3), the way \
         `doli release verify` does. output: {text}"
    );
    assert!(
        lower.contains("threshold 3"),
        "P1a/O2: the count is meaningless without the threshold it was judged against. \
         output: {text}"
    );
}

// REQ-215-006 — Decision: whether a stale local maintainer snapshot is visible to the operator;
// INC-I-206 was a fleet-wide refusal nobody could explain because the root's age was never printed.
#[test]
fn from_staged_prints_trust_root_provenance_and_last_derived_height() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (staging, data_dir, target) = layout(tmp.path());
    write_good_staging(&staging);

    let out = doli_from_staged(tmp.path(), &staging, &data_dir, &target, &[]);
    let text = combined(&out);

    assert!(
        text.contains("OnChain"),
        "P1a/O2: the root came from this host's maintainer_state.bin, so the provenance must \
         read OnChain. A Bootstrap banner here means resolve_upgrade_trust_root was bypassed \
         and the compiled keys are judging the install. output: {text}"
    );
    assert!(
        text.contains("5 key(s), threshold 3"),
        "P1a/O2: the resolved key count and threshold must be printed; they are what \
         distinguishes the on-chain root from the compiled bootstrap array. output: {text}"
    );
    assert!(
        text.contains(&ON_CHAIN_HEIGHT.to_string()),
        "P1a/O2: the on-chain last_derived_height ({ON_CHAIN_HEIGHT}) must be printed so a \
         stale snapshot is visible before the install, not after (INC-I-206). output: {text}"
    );
    assert!(
        text.contains(&data_dir.display().to_string()),
        "P1a/O2: the banner must name the data dir the root was read from. output: {text}"
    );
}

// REQ-215-007 — Decision: whether root re-verifies the artifact instead of trusting the
// staging dir; the entire design's value is that a compromised node cannot hand root an ELF.
#[test]
fn from_staged_refuses_a_tampered_tarball_and_leaves_the_binary_untouched() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (staging, data_dir, target) = layout(tmp.path());
    write_good_staging(&staging);

    let tarball_path = staging.join(updater::STAGED_TARBALL);
    let mut bytes = std::fs::read(&tarball_path).expect("read staged tarball");
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&tarball_path, &bytes).expect("write tampered tarball");

    let out = doli_from_staged(tmp.path(), &staging, &data_dir, &target, &[]);
    let text = combined(&out);
    assert_nothing_installed(&out, &target, "P7a");

    assert!(
        text.to_lowercase().contains("hash mismatch"),
        "P7a/O2: the refusal must name the artifact hash mismatch (L4). A generic failure \
         leaves the operator unable to tell tampering from a truncated download. output: {text}"
    );
    assert!(
        staging.exists(),
        "P7a/O3c: a refused staging must be PRESERVED as evidence, never deleted. output: {text}"
    );
    assert!(
        !staging.join(updater::READY_MARKER).exists(),
        "P7a/O3c: `ready` must not survive a refusal — M4's `.path` unit would retrigger on \
         it forever. output: {text}"
    );
    assert!(
        staging.join("ready.rejected").exists(),
        "P7a/O3c: the marker must be renamed to `ready.rejected`, which both disarms the \
         `.path` unit and records that root looked and said no. output: {text}"
    );
}

// REQ-215-008 — Decision: whether the resolved on-chain threshold is what gates the install;
// a manifest one signature short is the closest a real attacker gets, and it must still fail.
#[test]
fn from_staged_refuses_a_two_signature_manifest_under_a_five_member_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (staging, data_dir, target) = layout(tmp.path());

    let all = members();
    let tarball = tarball_with_node(NEW_NODE_BYTES);
    let checksums = checksums_body(STAGED_VERSION, &tarball);
    let sf = manifest(STAGED_VERSION, &checksums, &all[..2]);
    write_staging(
        &staging,
        Some(STAGED_VERSION),
        Some(&tarball),
        Some(&checksums),
        Some(&sf),
    );

    let out = doli_from_staged(tmp.path(), &staging, &data_dir, &target, &[]);
    let text = combined(&out);
    assert_nothing_installed(&out, &target, "P6a");

    let lower = text.to_lowercase();
    assert!(
        lower.contains("2/3") || (lower.contains("insufficient") && lower.contains("signature")),
        "P6a/O2: two genuine signatures under a threshold of three must be refused as \
         INSUFFICIENT, and the message must say how many of how many were found. output: {text}"
    );
    assert!(
        staging.join("ready.rejected").exists() && !staging.join(updater::READY_MARKER).exists(),
        "P6a/O3c: the marker must be renamed to `ready.rejected` on a verification refusal. \
         output: {text}"
    );
}

// REQ-215-008 — Decision: whether a genuine signature made for another release can authorise
// this one; that replay is the whole reason L1 binds the version into the signed message.
#[test]
fn from_staged_refuses_a_manifest_for_a_different_version() {
    let other_version = "98.0.0";
    let all = members();
    let tarball = tarball_with_node(NEW_NODE_BYTES);
    let checksums = checksums_body(STAGED_VERSION, &tarball);

    // P4a-i: an untouched, fully valid manifest for 98.0.0 laid beside a v99.0.0 staging.
    let tmp = tempfile::tempdir().expect("tempdir");
    let (staging, data_dir, target) = layout(tmp.path());
    let foreign = manifest(other_version, &checksums, &all[..3]);
    write_staging(
        &staging,
        Some(STAGED_VERSION),
        Some(&tarball),
        Some(&checksums),
        Some(&foreign),
    );
    let out = doli_from_staged(tmp.path(), &staging, &data_dir, &target, &[]);
    let text = combined(&out);
    assert_nothing_installed(&out, &target, "P4a-i");
    assert!(
        text.contains(STAGED_VERSION) && text.contains(other_version),
        "P4a-i/O2: the refusal must name BOTH versions — the release being installed and \
         the one the signatures actually authorise. output: {text}"
    );

    // P4a-ii: the same signatures with the `version` field rewritten forward. L1 now
    // passes; the signed message is still "98.0.0:<sha>", so L3 must find zero signers.
    let tmp2 = tempfile::tempdir().expect("tempdir");
    let (staging2, data_dir2, target2) = layout(tmp2.path());
    let mut replayed = manifest(other_version, &checksums, &all[..3]);
    replayed.version = STAGED_VERSION.to_string();
    write_staging(
        &staging2,
        Some(STAGED_VERSION),
        Some(&tarball),
        Some(&checksums),
        Some(&replayed),
    );
    let out2 = doli_from_staged(tmp2.path(), &staging2, &data_dir2, &target2, &[]);
    assert_nothing_installed(&out2, &target2, "P4a-ii");
    assert!(
        combined(&out2).to_lowercase().contains("0/3")
            || combined(&out2).to_lowercase().contains("insufficient"),
        "P4a-ii/O2: rewriting the version field must not launder the signatures — the signed \
         bytes are \"{{version}}:{{sha256(CHECKSUMS.txt)}}\", so none of the three may count. \
         output: {}",
        combined(&out2)
    );
}

// REQ-215-008 — Decision: whether the signatures are checked against the CHECKSUMS.txt actually
// on disk; if not, the per-platform hash that gates the tarball comes from an unsigned file.
#[test]
fn from_staged_refuses_a_checksums_file_that_does_not_match_the_manifest() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (staging, data_dir, target) = layout(tmp.path());

    let all = members();
    let tarball = tarball_with_node(NEW_NODE_BYTES);
    let real = checksums_body(STAGED_VERSION, &tarball);
    // The manifest is genuine, but it covers a DIFFERENT CHECKSUMS.txt: signatures over
    // `decoy`, staged bytes `real`. L2 must catch the substitution.
    let decoy = checksums_body(STAGED_VERSION, b"a completely different artifact");
    let sf = manifest(STAGED_VERSION, &decoy, &all[..3]);
    write_staging(
        &staging,
        Some(STAGED_VERSION),
        Some(&tarball),
        Some(&real),
        Some(&sf),
    );

    let out = doli_from_staged(tmp.path(), &staging, &data_dir, &target, &[]);
    let text = combined(&out);
    assert_nothing_installed(&out, &target, "P5");

    let lower = text.to_lowercase();
    assert!(
        lower.contains("checksums_sha256") || lower.contains("checksums.txt"),
        "P5/O2: the refusal must name the CHECKSUMS.txt binding (L2) — the signed file is \
         not the file the artifact hash was read from. output: {text}"
    );
}

// REQ-215-009 — Decision: whether a half-written handoff is refused by NAME; a helper unit that
// fires mid-copy is the expected failure, and "something went wrong" is unactionable at 3am.
#[test]
fn from_staged_refuses_when_ready_or_any_manifest_file_is_missing() {
    let all = members();
    let tarball = tarball_with_node(NEW_NODE_BYTES);
    let checksums = checksums_body(STAGED_VERSION, &tarball);
    let sf = manifest(STAGED_VERSION, &checksums, &all[..3]);

    // (file to omit, the token the message must carry)
    let cases: [(&str, &str); 4] = [
        (updater::READY_MARKER, updater::READY_MARKER),
        (updater::STAGED_SIGNATURES, updater::STAGED_SIGNATURES),
        (updater::STAGED_CHECKSUMS, updater::STAGED_CHECKSUMS),
        (updater::STAGED_TARBALL, updater::STAGED_TARBALL),
    ];

    for (omitted, token) in cases {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (staging, data_dir, target) = layout(tmp.path());
        write_staging(
            &staging,
            (omitted != updater::READY_MARKER).then_some(STAGED_VERSION),
            (omitted != updater::STAGED_TARBALL).then_some(tarball.as_slice()),
            (omitted != updater::STAGED_CHECKSUMS).then_some(checksums.as_slice()),
            (omitted != updater::STAGED_SIGNATURES).then_some(&sf),
        );

        let out = doli_from_staged(tmp.path(), &staging, &data_dir, &target, &[]);
        let text = combined(&out);
        assert_nothing_installed(&out, &target, omitted);
        assert!(
            names_file(&text, token),
            "P2/O2: with {omitted} absent the refusal must NAME it, so the operator knows \
             which half of the handoff is missing. output: {text}"
        );
    }

    // P2e: the directory itself does not exist.
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_unused, data_dir, target) = layout(tmp.path());
    let absent = tmp.path().join("no-such-staging-dir");
    let out = doli_from_staged(tmp.path(), &absent, &data_dir, &target, &[]);
    let text = combined(&out);
    assert_nothing_installed(&out, &target, "P2e");
    assert!(
        names_file(&text, updater::READY_MARKER) || text.contains("no-such-staging-dir"),
        "P2e/O2: a non-existent <DIR> must be refused by name, not by panic or by a bare \
         io error. output: {text}"
    );
    assert!(
        !absent.exists(),
        "P2e/O3c: refusing must never CREATE the staging dir it was pointed at. output: {text}"
    );
}

// REQ-215-009 — Decision: whether root installs only what the marker named; the marker is the
// only thing M4's `.path` unit reads, so a marker/manifest disagreement is an unapproved handoff.
#[test]
fn from_staged_refuses_a_ready_marker_whose_version_differs_from_the_manifest() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (staging, data_dir, target) = layout(tmp.path());

    let all = members();
    let tarball = tarball_with_node(NEW_NODE_BYTES);
    let checksums = checksums_body(STAGED_VERSION, &tarball);
    let sf = manifest(STAGED_VERSION, &checksums, &all[..3]);
    // Everything is a genuine, threshold-satisfying v99.0.0 release; only the marker
    // disagrees. This is P4a's mirror image: a fix that trusts the manifest and ignores
    // the marker passes that test and fails this one.
    write_staging(
        &staging,
        Some("98.0.0"),
        Some(&tarball),
        Some(&checksums),
        Some(&sf),
    );

    let out = doli_from_staged(tmp.path(), &staging, &data_dir, &target, &[]);
    let text = combined(&out);
    assert_nothing_installed(&out, &target, "P3b");

    assert!(
        text.contains("98.0.0") && text.contains(STAGED_VERSION),
        "P3b/O2: the refusal must name the marker's version and the manifest's, so the \
         operator can see which side is stale. output: {text}"
    );
}

// REQ-215-010 — Decision: whether a stale staging dir left by a crashed helper can silently
// reinstall an older build; the .path unit retriggers on every boot, so this would be a loop.
#[test]
fn from_staged_refuses_a_version_not_newer_than_the_installed_one() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (staging, data_dir, target) = layout(tmp.path());

    // Equal, not lower: the boundary. `is_newer_version` must be strict.
    let same = updater::current_version();
    let all = members();
    let tarball = tarball_with_node(NEW_NODE_BYTES);
    let checksums = checksums_body(same, &tarball);
    let sf = manifest(same, &checksums, &all[..3]);
    write_staging(
        &staging,
        Some(same),
        Some(&tarball),
        Some(&checksums),
        Some(&sf),
    );

    let out = doli_from_staged(tmp.path(), &staging, &data_dir, &target, &[]);
    let text = combined(&out);
    assert_nothing_installed(&out, &target, "P8a");

    assert!(
        text.contains(same),
        "P8a/O2: the refusal must name the version it declined to install (v{same}). \
         output: {text}"
    );
    assert!(
        staging.join("ready.rejected").exists() && !staging.join(updater::READY_MARKER).exists(),
        "P8a/O3c: a downgrade refusal must disarm the marker too, otherwise the helper unit \
         refires on every boot. output: {text}"
    );
}

// REQ-215-006 — Decision: whether any exit leaves `ready` in place; M4 arms a systemd .path unit
// on PathExists=<DIR>/ready, so a surviving marker is an install loop until TriggerLimit trips.
#[test]
fn from_staged_never_leaves_the_ready_marker_in_place() {
    let src = read_cli_src("cmd_upgrade_staged.rs").unwrap_or_else(|| {
        panic!(
            "bins/cli/src/cmd_upgrade_staged.rs does not exist. `--from-staged` must live in \
             its own module: cmd_upgrade.rs is already over the 500-line budget (REQ-215-016)."
        )
    });

    assert!(
        src.contains("ready.rejected"),
        "O3c: the module must rename the marker to `ready.rejected` on refusal — preserving \
         the evidence while disarming M4's `.path` unit."
    );
    assert!(
        src.contains("ready.installed"),
        "O3c: install-succeeded-but-restart-failed must rename the marker to \
         `ready.installed`. Leaving `ready` there re-runs a completed install; deleting the \
         staging dir loses the evidence of the failed restart."
    );
    assert!(
        src.contains("remove_dir_all"),
        "O3c: the success path must remove the WHOLE staging dir, not just the marker — a \
         leftover tarball is a signed artifact sitting in a doli-writable path."
    );

    // The filesystem half, on a partition none of the other tests own: a manifest that is
    // absent entirely still has to disarm the marker.
    let tmp = tempfile::tempdir().expect("tempdir");
    let (staging, data_dir, target) = layout(tmp.path());
    let tarball = tarball_with_node(NEW_NODE_BYTES);
    let checksums = checksums_body(STAGED_VERSION, &tarball);
    write_staging(
        &staging,
        Some(STAGED_VERSION),
        Some(&tarball),
        Some(&checksums),
        None,
    );

    let out = doli_from_staged(tmp.path(), &staging, &data_dir, &target, &[]);
    let text = combined(&out);
    assert_nothing_installed(&out, &target, "P2b-marker");
    assert!(
        !staging.join(updater::READY_MARKER).exists(),
        "P2b/O3c: `ready` must not survive an incomplete-staging refusal either. output: {text}"
    );
    assert!(
        staging.join("ready.rejected").exists(),
        "P2b/O3c: every refusal that finds a `ready` marker must rename it to \
         `ready.rejected`. output: {text}"
    );
}

// REQ-215-017 — Decision: whether a second, drifting copy of the L1-L4 chain was written for the
// staged path; INC-I-172 F1/F2 was exactly a verification that ran beside the install, not before.
#[test]
fn from_staged_verifies_before_it_installs() {
    let src = read_cli_src("cmd_upgrade_staged.rs").unwrap_or_else(|| {
        panic!(
            "bins/cli/src/cmd_upgrade_staged.rs does not exist; REQ-215-017 requires the \
             staged install path to call the ONE shared L1-L4 gate."
        )
    });

    let verify = src
        .find("verify_release_artifact_bytes")
        .unwrap_or_else(|| {
            panic!(
                "cmd_upgrade_staged.rs must call updater::verify_release_artifact_bytes — the \
             single L1-L4 implementation shared with the node and the publish gate."
            )
        });
    let install = src.find("install_binary").unwrap_or_else(|| {
        panic!("cmd_upgrade_staged.rs must install through updater::install_binary.")
    });
    assert!(
        verify < install,
        "O1/O3a: verification must appear BEFORE the first install call. A verify that runs \
         after (or beside) the install is the INC-I-172 F1 shape: the binary is already on \
         disk when the verdict arrives."
    );

    for reimplemented in ["verify_release_manifest", "platform_tarball_hash"] {
        assert!(
            !src.contains(reimplemented),
            "REQ-215-017: cmd_upgrade_staged.rs must not call `{reimplemented}` — reaching \
             past verify_release_artifact_bytes into its parts is how a second chain that \
             skips L4 gets written."
        );
    }
}

// REQ-215-016 — Decision: whether the new flag was bolted onto an already-oversized module; the
// 500-line budget is what kept cmd_upgrade.rs reviewable through three security incidents.
#[test]
fn cli_upgrade_files_are_within_the_module_budget() {
    for name in ["cmd_upgrade_staged.rs", "cmd_upgrade.rs"] {
        let src = read_cli_src(name)
            .unwrap_or_else(|| panic!("bins/cli/src/{name} does not exist (REQ-215-016)."));
        let lines = src.lines().count();
        assert!(
            lines <= 500,
            "REQ-215-016: bins/cli/src/{name} is {lines} lines, over the 500-line module \
             budget. cmd_upgrade.rs is 507 today: --from-staged must arrive with a split, \
             not on top of the overflow."
        );
    }
}
