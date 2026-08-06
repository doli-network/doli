//! INC-I-153 — reproduction + contract for the auto-updater install mode.
//!
//! Symptom (measured on a mainnet host, already recovered):
//!   installed `/usr/bin/doli-node` was `-rwxr-x--- root:root`
//!   `sudo -u doli test -x /usr/bin/doli-node` -> rc=1
//!   systemd: `Failed at step EXEC ... Permission denied`, `status=203/EXEC`
//!
//! Root cause under test: `crates/updater/src/apply.rs::install_binary_sudo()`
//! stages the new binary at `0o750` and then does `sudo rm -f <target>` followed
//! by `sudo cp <staged> <target>` with no `-p` and no `chmod`. Because the target
//! was just unlinked, `cp` takes its CREATE path, where the new inode's mode is
//! derived from the SOURCE mode. `0o750` has no other-execute bit under ANY
//! umask, and the systemd unit runs `User=doli` (uid 967) which is neither the
//! file's owner nor in its group -> `execve` returns EACCES.
//!
//! The sibling direct-write branch (`apply.rs:161-164`) sets `0o755` explicitly
//! and is correct. The two branches disagree and nothing asserts they agree.
//!
//! TESTABILITY GAP (flagged, not fixed here): `install_binary_sudo` is private,
//! hardcodes `STAGED_BINARY_PATH = "/var/lib/doli/update.bin"`, and shells out to
//! literal `sudo`. It is therefore STRUCTURALLY UNREACHABLE from an unprivileged
//! CI test. The fix should make the staging path and the privileged command
//! injectable so the structural test below can be replaced by a behavioural one.

#![cfg(unix)]

// OUTPUT CONTRACT: fn install_binary_sudo(binary: &[u8], target: &Path) -> Result<()>
//   O1: params — `binary: &[u8]`, `target: &Path` are SHARED refs; no mutable params. (n/a)
//   O2: receiver — free async fn, no `self`. (n/a)
//   O3: return — Result<(), UpdateError>: Ok(()) after a successful `sudo cp`;
//       Err(InstallFailed) on staging-dir create failure, O_NOFOLLOW open failure,
//       staged-write failure, or non-zero `sudo cp`.
//   O4: fs / target BYTES — target content must equal `binary`.
//   O4: fs / target MODE — target must be executable by the systemd `User=` account,
//       which is neither owner nor group => other-execute bit (mode & 0o001) REQUIRED.
//       *** This is the output the function never asserts. It is the bug. ***
//   O4: fs / target OWNER:GROUP — root:root (created by the privileged `cp`).
//   O4: fs / target PREVIOUS INODE — unlinked by `sudo rm -f` before the copy;
//       this is what forces `cp` onto its CREATE path (mode from source, not target).
//   O4: fs / staged file `/var/lib/doli/update.bin` — created 0o750 (O_NOFOLLOW),
//       removed on BOTH the success and the cp-failure exit.
//   O5: globals — none.
//   O6: caller-visible SUCCESS SIGNAL — `Ok(())` + `info!("Binary installed to
//       {:?} (via sudo)")`. Today this signal is emitted for a target the service
//       account cannot execute: success is reported for a bricked install.
// PATHS:
//   P1: install_binary direct-write branch — fs::write OK -> chmod 0o755 -> rename.
//   P2: install_binary sudo-fallback branch — fs::write EACCES -> install_binary_sudo.
//   P3: P2 + create_dir_all(staging parent) fails -> Err(InstallFailed).
//   P4: P2 + O_NOFOLLOW open / write of staged file fails -> Err(InstallFailed).
//   P5: P2 + `sudo cp` exits non-zero -> staged removed, Err(InstallFailed).
//   P6: P2 + `sudo cp` exits zero -> staged removed, Ok(()).
// INPUT PARTITIONS:
//   P1a: target pre-exists at 0o755 (normal upgrade) — rename replaces it, mode 0o755.
//   P1b: target absent (fresh install) — same, mode 0o755.
//   P6a: target ABSENT at cp time (ALWAYS true — `sudo rm -f` ran first) => cp CREATE
//        path => installed mode = staged_mode & ~umask. o+x absent iff absent in source.
//   P6b: target PRESENT at cp time (unreachable today) => cp OVERWRITE path => the
//        target KEEPS its own pre-existing mode, so the bug would not manifest. This
//        partition is exactly why `rm -f` turned a latent defect into a brick.
//   P6c: effective umask 0o022 vs 0o027 vs 0o077. `sudo`'s effective umask is
//        `caller_umask | sudoers Defaults umask`. From 0o750 staging EVERY umask
//        yields no o+x; from 0o755 staging a site with `Defaults umask=0027` STILL
//        yields 0o750 -> identical brick. Raising the staged mode is therefore
//        NECESSARY BUT NOT SUFFICIENT; only an explicit chmod + read-back on the
//        installed target holds unconditionally.
//   P6d: staged mode WITH o+x (0o755) vs WITHOUT (0o750) — the discriminating input.
// MATRIX: 6 outputs x 8 (paths x partitions) = 48 cells.
//   Reachable from an unprivileged test and asserted here:
//     P1a/P1b x {O3, O4-bytes, O4-mode}  -> `install_branches_must_agree_on_installed_mode`
//     P6a x P6d x O4-mode                -> `staged_mode_must_survive_privileged_copy_as_other_executable`
//     P6a x O4-mode x O3/O6              -> `privileged_install_must_set_and_verify_target_mode`
//   NOT reachable, and why (each stated, none silently skipped):
//     P2 entry           — requires a write-denied target dir; as root in a container
//                          the EACCES never fires, so the branch is nondeterministic.
//     P3, P4, P5         — require creating/denying `/var/lib/doli`, i.e. root.
//     P6 end-to-end      — requires real `sudo`; forbidden in CI.
//     O4-owner (root:root) — requires root; cannot be produced or observed.
//     O4-staged-cleanup  — lives at the hardcoded absolute `/var/lib/doli/update.bin`.
//     P6b                — unreachable BY CONSTRUCTION: `sudo rm -f` always precedes cp.
//     P6c (real umask)   — `std::fs::copy` chmods the destination from the source and
//                          ignores umask, so it is the MOST PERMISSIVE faithful stand-in
//                          for `cp`: an o+x bit absent here is absent under `cp` for
//                          every umask. The umask half is covered structurally by
//                          `privileged_install_must_set_and_verify_target_mode`.

use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

/// The production source under test. `install_binary_sudo` is private and its
/// staged mode is a bare literal in the function body, so the mode the fix must
/// change is not reachable as a symbol.
///
/// COUPLING RISK (stated, per the Output Contract rules): these tests read the
/// staged mode out of the source text. If the fix replaces the literal with a
/// named constant, `production_staged_modes()` falls back to scanning for a
/// `const *_MODE ... = 0o...`; if it finds neither it fails loudly rather than
/// passing vacuously.
const APPLY_RS: &str = include_str!("../src/apply.rs");

/// Body text of `install_binary_sudo`, with `//` line comments stripped so an
/// explanatory comment mentioning an old mode cannot satisfy or break a check.
/// (Naive stripping: a `//` inside a string literal would truncate the line.
/// The function body contains no such literal today.)
fn sudo_install_body() -> String {
    let start = APPLY_RS.find("async fn install_binary_sudo").expect(
        "INC-I-153: crates/updater/src/apply.rs must define `async fn install_binary_sudo`. \
         If it was renamed or split, update this test — do not delete it.",
    );
    let rest = &APPLY_RS[start..];
    let end = rest
        .find("\n}\n")
        .map(|i| i + 2)
        .expect("INC-I-153: could not find the end of the install_binary_sudo body");
    rest[..end]
        .lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn octal_after(src: &str, marker: &str) -> Vec<u32> {
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(p) = rest.find(marker) {
        let after = &rest[p + marker.len()..];
        let digits: String = after.chars().take_while(|c| c.is_digit(8)).collect();
        if let Ok(v) = u32::from_str_radix(&digits, 8) {
            out.push(v);
        }
        rest = &rest[p + marker.len()..];
    }
    out
}

/// Every mode the privileged branch applies to the STAGED file, in source order.
/// Today: `[0o750, 0o750]` (`apply.rs:228` `.mode(0o750)` and `apply.rs:239`
/// `perms.set_mode(0o750)`).
fn production_staged_modes() -> Vec<u32> {
    let body = sudo_install_body();
    let mut modes = octal_after(&body, "mode(0o");
    if modes.is_empty() {
        // Fallback: the mode was hoisted into a named constant.
        modes = APPLY_RS
            .lines()
            .filter(|l| l.contains("const") && l.contains("MODE") && l.contains("0o"))
            .flat_map(|l| octal_after(l, "0o"))
            .collect();
    }
    assert!(
        !modes.is_empty(),
        "INC-I-153: could not determine the mode `install_binary_sudo` gives the staged \
         binary. The staged mode must stay discoverable (a literal in the body or a \
         `const *_MODE = 0o...`) or this reproduction test cannot verify it."
    );
    modes
}

/// The mode the staged file actually ends up with (the LAST mode applied).
fn effective_staged_mode() -> u32 {
    *production_staged_modes()
        .last()
        .expect("non-empty by construction")
}

fn mode_of(p: &Path) -> u32 {
    std::fs::metadata(p).unwrap().permissions().mode() & 0o7777
}

/// Replays the EXACT production sequence of the privileged branch, minus `sudo`,
/// entirely inside a caller-owned temp dir.
///
/// 1. a working binary is already installed at the target (mode 0o755)
/// 2. stage the new binary  == `apply.rs:224-240` (O_NOFOLLOW create at
///    `staged_mode`, then an explicit `set_permissions(staged_mode)`, which is
///    `chmod(2)` and therefore exact regardless of the runner's umask)
/// 3. `sudo rm -f <target>`  == `remove_file(target)` — the target is UNLINKED
/// 4. `sudo cp <staged> <target>` (no `-p`, no chmod) == `std::fs::copy`
///
/// Returns the mode of the installed target.
///
/// Fidelity note: `std::fs::copy` sets the destination permissions from the
/// source explicitly, so it ignores umask; GNU `cp`'s create path yields
/// `src_mode & ~umask`, a SUBSET. The stand-in is therefore strictly more
/// permissive than the real thing: any bit missing here is missing under `cp`
/// for every umask. Deterministic, and independent of the CI runner's umask.
fn replay_privileged_install(dir: &Path, staged_mode: u32) -> u32 {
    let staged = dir.join("update.bin");
    let target = dir.join("doli-node");

    std::fs::write(&target, b"currently-running-binary").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();

    let _ = std::fs::remove_file(&staged);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true)
        .create(true)
        .truncate(true)
        .mode(staged_mode)
        .custom_flags(libc::O_NOFOLLOW);
    let mut f = opts.open(&staged).unwrap();
    f.write_all(b"new-binary").unwrap();
    f.sync_all().ok();
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(staged_mode)).unwrap();

    // `sudo rm -f <target>`: unlinking is what forces `cp` onto its CREATE path.
    let _ = std::fs::remove_file(&target);
    // `sudo cp <staged> <target>`
    std::fs::copy(&staged, &target).unwrap();

    // O4/bytes: the copy must be faithful, on every path.
    assert_eq!(
        std::fs::read(&target).unwrap(),
        b"new-binary",
        "INC-I-153: privileged install must write the new binary's bytes"
    );

    mode_of(&target)
}

// ---------------------------------------------------------------------------
// T1 — REPRODUCTION. P6a x P6d x O4-mode.
// ---------------------------------------------------------------------------

/// The mode `install_binary_sudo` stages the binary with is the mode the
/// installed copy inherits, because the target is unlinked first. It must
/// therefore be executable by a user who is neither the owner nor in the group
/// — i.e. `mode & 0o001 != 0`.
#[test]
fn staged_mode_must_survive_privileged_copy_as_other_executable() {
    let dir = tempfile::tempdir().unwrap();

    for (idx, staged_mode) in production_staged_modes().into_iter().enumerate() {
        let case = dir.path().join(format!("case{}", idx));
        std::fs::create_dir_all(&case).unwrap();

        let installed = replay_privileged_install(&case, staged_mode);

        assert_eq!(
            installed & 0o111,
            0o111,
            "INC-I-153: apply.rs::install_binary_sudo stages the binary at mode {:#o}; \
             after `sudo rm -f` + `sudo cp` the installed binary is mode {:#o} \
             (other-execute bit = {:#o}, required non-zero). systemd runs the node as \
             `User=doli`, which is neither the file's owner nor in its group, so \
             execve() returns EACCES and the unit dies with status=203/EXEC. \
             Every umask makes this worse, none makes it better: {:#o} & ~umask can \
             never gain a bit that {:#o} does not have.",
            staged_mode,
            installed,
            installed & 0o001,
            staged_mode,
            staged_mode
        );
    }
}

// ---------------------------------------------------------------------------
// T2 — BRANCH PARITY. P1a/P1b x {O3, O4-bytes, O4-mode} vs P6a x O4-mode.
// ---------------------------------------------------------------------------

/// `install_binary` has two branches that install the same artifact to the same
/// path for the same purpose. They MUST agree on the installed mode. The direct
/// branch half of this test runs the real production code; the privileged half
/// is the mechanism replay above (that branch is unreachable unprivileged — see
/// the header).
#[tokio::test]
async fn install_branches_must_agree_on_installed_mode() {
    let dir = tempfile::tempdir().unwrap();

    // --- P1a: direct-write branch, real production code, target pre-exists ---
    let direct_dir = dir.path().join("direct");
    std::fs::create_dir_all(&direct_dir).unwrap();
    let target = direct_dir.join("doli-node");
    std::fs::write(&target, b"currently-running-binary").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();

    // O3: return
    updater::install_binary(b"new-binary", &target)
        .await
        .expect("INC-I-153: direct-write install into a writable dir must succeed");
    // O4: bytes
    assert_eq!(
        std::fs::read(&target).unwrap(),
        b"new-binary",
        "INC-I-153: direct branch must write the new binary's bytes"
    );
    // O4: mode
    let direct_mode = mode_of(&target);
    assert_eq!(
        direct_mode & 0o111,
        0o111,
        "INC-I-153: direct-write branch installed mode {:#o} — not other-executable",
        direct_mode
    );

    // --- P1b: direct-write branch, target absent (fresh install) ---
    let fresh_dir = dir.path().join("fresh");
    std::fs::create_dir_all(&fresh_dir).unwrap();
    let fresh = fresh_dir.join("doli-node");
    updater::install_binary(b"new-binary", &fresh)
        .await
        .expect("INC-I-153: direct-write install onto an absent target must succeed");
    assert_eq!(std::fs::read(&fresh).unwrap(), b"new-binary");
    assert_eq!(
        mode_of(&fresh),
        direct_mode,
        "INC-I-153: direct branch must install the same mode whether or not the target existed"
    );

    // --- P6a: privileged branch, same artifact, same target path ---
    let sudo_dir = dir.path().join("sudo");
    std::fs::create_dir_all(&sudo_dir).unwrap();
    let sudo_mode = replay_privileged_install(&sudo_dir, effective_staged_mode());

    assert_eq!(
        direct_mode,
        sudo_mode,
        "INC-I-153: the two install branches of `install_binary` disagree on the \
         installed mode — direct-write yields {:#o} (apply.rs:161-164 sets 0o755 \
         explicitly) but the sudo fallback yields {:#o} (apply.rs:228/238-240 stages \
         at {:#o} and `sudo cp` onto an unlinked target inherits it). Same artifact, \
         same path, same service account, two different answers, and nothing in the \
         code asserts they agree. The sudo answer is the one that bricks the node.",
        direct_mode,
        sudo_mode,
        effective_staged_mode()
    );
}

// ---------------------------------------------------------------------------
// T3 — POSTCONDITION. P6a x O4-mode x O3/O6.
// ---------------------------------------------------------------------------

/// The unconditional half of the fix.
///
/// Raising the staged mode alone is NOT sufficient: `sudo`'s effective umask is
/// `caller_umask | sudoers Defaults umask`, so a site with `Defaults umask=0027`
/// reproduces the identical brick from 0o755 staging (P6c). The privileged
/// install must therefore
///   (b) set the mode EXPLICITLY on the installed target (chmod(2) ignores umask
///       — this is what the function's own doc comment at apply.rs:192-193
///       already CLAIMS it does: "then uses `sudo cp` + `sudo chmod` to install
///       it" — the body contains no chmod at all), and
///   (c) READ BACK the installed mode and return `Err` if it is not executable
///       by others, instead of returning `Ok(())` and logging "Binary installed".
///       (c) is not redundant with (b): every `sudo` invocation in this function
///       today either ignores its exit status (`let _ = ... rm`) or trusts it; a
///       sudoers rule that permits `cp` but not `chmod` would silently reproduce
///       the brick. The only trustworthy evidence is the mode of the file on disk.
///
/// Structural, not behavioural, because `install_binary_sudo` is private and
/// hardcodes `/var/lib/doli/update.bin` + literal `sudo` (see header). Replace
/// this with a behavioural test once the fix makes those injectable.
#[test]
fn privileged_install_must_set_and_verify_target_mode() {
    let body = sudo_install_body();
    let cp_at = body.find("\"cp\"").expect(
        "INC-I-153: expected a `sudo cp` invocation in install_binary_sudo. If the \
         privileged copy changed shape, update this test — do not delete it.",
    );
    let after_cp = &body[cp_at..];

    // (b) explicit mode set on the installed target after the copy
    let sets_mode = after_cp.contains("chmod")
        || after_cp.contains("set_permissions")
        || after_cp.contains("\"-m\""); // `install -m 0755` is also acceptable
    assert!(
        sets_mode,
        "INC-I-153: install_binary_sudo does not set the mode of the installed target \
         after the privileged copy. It leaves the mode to `cp`'s CREATE path, i.e. to \
         `staged_mode & ~umask`, where umask is `caller | sudoers Defaults umask` — a \
         value this process does not control. Staging at {:#o} is what bricked the \
         mainnet host; staging at 0o755 still bricks any site with \
         `Defaults umask=0027`. The function's own doc comment (apply.rs:192-193) \
         already promises `sudo cp` + `sudo chmod`; the body has no chmod.",
        effective_staged_mode()
    );

    // (c) read the installed mode back and reject a non-other-executable result
    let verifies_mode = after_cp.contains("metadata(")
        && (after_cp.contains("0o111") || after_cp.contains("0o001") || after_cp.contains("0o005"));
    assert!(
        verifies_mode,
        "INC-I-153: install_binary_sudo returns Ok(()) and logs \"Binary installed\" \
         without ever reading back the mode of the file it just installed. It must \
         stat the target and return Err(InstallFailed) when `mode & 0o111 != 0o111`, \
         so a non-executable install is reported as a FAILURE instead of a success. \
         Measured consequence of not doing so: the updater reported success, systemd \
         then failed at step EXEC with Permission denied (status=203/EXEC), and the \
         node was down until a human ran chmod."
    );
}
