// SK-M1 — ownership and permissions of the installed skills tree (TDD RED).
// REQ-SKILLS-003 (Must) — Decision: whether a re-install under sudo silently hands the
//   operator's skills directory to root, so the very next agent read fails with EACCES.
//
// THE DEFECT: `install_skills_into` does `remove_dir_all` then recreates. Running as root
// that destroys the previous owner and re-creates the tree owned by root, and the file
// modes are whatever the process umask happens to be.
//
// STATE: RED. `updater::SkillsOwnerEnv`, `updater::skills_owner_for` and
// `updater::apply_skills_ownership` do not exist, so this module does not compile. That
// compile error IS the contract.
//
// REQUIRED SIGNATURES (binding on the developer):
//   pub struct SkillsOwnerEnv {          // both fields `pub`
//       pub sudo_uid: Option<String>,        // SUDO_UID
//       pub sudo_gid: Option<String>,        // SUDO_GID
//   }
//   /// Which (uid, gid) the skills tree must end up owned by. PURE — no syscalls.
//   /// `existing` is the owner read off the directory BEFORE it was removed, if it was
//   /// there. `None` return means: request no chown at all.
//   pub fn skills_owner_for(
//       existing: Option<(u32, u32)>,
//       env: &SkillsOwnerEnv,
//   ) -> Option<(u32, u32)>
//   /// Applies the decision. No-op when `owner` is None or off unix.
//   pub fn apply_skills_ownership(
//       skills_dir: &std::path::Path,
//       owner: Option<(u32, u32)>,
//   ) -> std::io::Result<()>
//   All three re-exported from crates/updater/src/lib.rs.
//
// ============================================================================
// OUTPUT CONTRACT (.claude/protocols/output-contract.md)
// ============================================================================
// Subject A: fn skills_owner_for(Option<(u32,u32)>, &SkillsOwnerEnv) -> Option<(u32,u32)>
//   O1 return — the chosen owner, or None for "do not chown".
//   mutable params: NONE. receiver: NONE. store: NONE (pure). env: NONE WRITTEN.
// Subject B: fn install_skills_into(&[u8], &Path) -> Result<usize>
//   O2 return count.
//   O3 PERSISTENT STORE — the permission bits of every created file and directory.
//      O3 is the observable that REQ-SKILLS-003 ("files 0644, dirs 0755") names.
// Subject C: fn apply_skills_ownership(&Path, Option<(u32,u32)>) -> io::Result<()>
//   O4 PERSISTENT STORE — st_uid / st_gid of the tree. Root-only; skipped otherwise.
// CODE PATHS of skills_owner_for:
//   A) existing Some          -> O1 = existing (preserve), SUDO_* ignored
//   B) existing None, both SUDO_UID and SUDO_GID parse as u32 -> O1 = those
//   C) existing None, anything missing or unparsable           -> O1 = None
// INPUT PARTITIONS:
//   P1 existing set + SUDO_UID/GID set and different -> A
//   P2 existing None + SUDO_UID/GID numeric          -> B
//   P3 existing None + neither set                   -> C
//   P4 existing None + SUDO_UID non-numeric          -> C
//   P5 existing None + SUDO_UID set, SUDO_GID absent -> C
//   P6 a real tarball installed into a temp dir under a hostile umask -> O3
//   P7 running as root                               -> O4
// MATRIX: O1 x P1 -> w1 ; O1 x P2 -> w2 ; O1 x P3 -> w3 ; O1,O2,O3 x P4 -> w4 ;
//         O1 x P5 -> w5 ; O3 x P6 -> w6 ; O4 x P7 -> w7.
// NOT COVERED: reading the pre-existing owner off disk before removal — that is a plain
//   `metadata().uid()` read with no branching worth a test, and w7 covers the write side.

use std::io::Write;
use std::path::Path;

use updater::SkillsOwnerEnv;

fn owner_env(sudo_uid: Option<&str>, sudo_gid: Option<&str>) -> SkillsOwnerEnv {
    SkillsOwnerEnv {
        sudo_uid: sudo_uid.map(str::to_string),
        sudo_gid: sudo_gid.map(str::to_string),
    }
}

fn tarball(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut builder = tar::Builder::new(enc);
    for (path, body) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o600);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        builder.append_data(&mut header, path, *body).unwrap();
    }
    let mut enc = builder.into_inner().unwrap();
    enc.flush().unwrap();
    enc.finish().unwrap()
}

fn sample_tarball() -> Vec<u8> {
    tarball(&[
        ("pkg/skills/sk-m1-a/SKILL.md", b"alpha" as &[u8]),
        ("pkg/skills/sk-m1-a/notes.md", b"beta" as &[u8]),
    ])
}

fn running_as_root() -> bool {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim() == "0")
        .unwrap_or(false)
}

// REQ-SKILLS-003 (Must) — Acceptance: an existing directory keeps its owner across re-install.
#[test]
fn w1_existing_owner_is_preserved_and_beats_sudo_uid_gid() {
    let e = owner_env(Some("501"), Some("20"));
    assert_eq!(
        updater::skills_owner_for(Some((1000, 1000)), &e),
        Some((1000, 1000)),
        "O1/P1 REQ-SKILLS-003: a pre-existing owner survives the remove/recreate cycle"
    );
}

// REQ-SKILLS-003 (Must) — Acceptance: a new directory under sudo is chowned to the invoker.
#[test]
fn w2_new_directory_uses_sudo_uid_and_gid() {
    let e = owner_env(Some("501"), Some("20"));
    assert_eq!(
        updater::skills_owner_for(None, &e),
        Some((501, 20)),
        "O1/P2 REQ-SKILLS-003: a fresh tree under sudo belongs to the invoking user"
    );
}

// REQ-SKILLS-003 (Must) — Acceptance: without sudo there is nothing to hand back, so no chown.
#[test]
fn w3_new_directory_without_sudo_env_requests_no_chown() {
    let e = owner_env(None, None);
    assert_eq!(
        updater::skills_owner_for(None, &e),
        None,
        "O1/P3 REQ-SKILLS-003: not under sudo, the creating process is already the right owner"
    );
}

// REQ-SKILLS-003 (Must) — Acceptance: a malformed SUDO_UID never becomes a chown target,
// and the install itself still succeeds.
#[test]
fn w4_malformed_sudo_uid_requests_no_chown_and_install_still_succeeds() {
    let e = owner_env(Some("not-a-number"), Some("20"));
    assert_eq!(
        updater::skills_owner_for(None, &e),
        None,
        "O1/P4 REQ-SKILLS-003: an unparsable SUDO_UID must not be coerced into a uid"
    );

    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("skills");
    let count = updater::install_skills_into(&sample_tarball(), &dest)
        .expect("O2/P4 REQ-SKILLS-003: a bad SUDO_UID must not fail the install");
    assert_eq!(count, 1, "O2/P4: one SKILL.md in the fixture tarball");
    assert!(
        dest.join("sk-m1-a/SKILL.md").exists(),
        "O3/P4 REQ-SKILLS-003: the skill file must be on disk even with no chown"
    );
}

// REQ-SKILLS-003 (Must) — Acceptance: a half-set sudo environment is not a chown target.
#[test]
fn w5_sudo_uid_without_sudo_gid_requests_no_chown() {
    let e = owner_env(Some("501"), None);
    assert_eq!(
        updater::skills_owner_for(None, &e),
        None,
        "O1/P5 REQ-SKILLS-003: both halves must parse or the chown is not requested"
    );
}

// ---------------------------------------------------------------------------
// w6: modes. The assertion runs in a CHILD process under `umask 077`, because a mode
// test under the ambient umask 022 would pass on the current, umask-dependent code and
// prove nothing. The child prints CHILD_MARKER; the parent requires that marker, so a
// filter that matched no test cannot be mistaken for a pass.
// ---------------------------------------------------------------------------

#[cfg(unix)]
const CHILD_ENV: &str = "SK_M1_MODE_CHILD";
#[cfg(unix)]
const CHILD_MARKER: &str = "SK-M1-MODE-CHILD-OK";
#[cfg(unix)]
const CHILD_TEST: &str = "sk_m1_skills_ownership::w6_installed_modes_are_0644_files_and_0755_dirs";

#[cfg(unix)]
fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .unwrap_or_else(|e| panic!("cannot stat {}: {e}", path.display()))
        .permissions()
        .mode()
        & 0o777
}

#[cfg(unix)]
fn assert_installed_modes() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("skills");
    updater::install_skills_into(&sample_tarball(), &dest).expect("install must succeed");

    assert_eq!(
        mode_of(&dest),
        0o755,
        "O3/P6 REQ-SKILLS-003: the skills root must be 0755 regardless of the umask"
    );
    assert_eq!(
        mode_of(&dest.join("sk-m1-a")),
        0o755,
        "O3/P6 REQ-SKILLS-003: nested skill directories must be 0755 regardless of the umask"
    );
    assert_eq!(
        mode_of(&dest.join("sk-m1-a/SKILL.md")),
        0o644,
        "O3/P6 REQ-SKILLS-003: skill files must be 0644 regardless of the umask"
    );
    println!("{CHILD_MARKER}");
}

// REQ-SKILLS-003 (Must) — Acceptance: files 0644 and dirs 0755, set explicitly rather
// than inherited from whatever umask the installing service happens to run with.
#[cfg(unix)]
#[test]
fn w6_installed_modes_are_0644_files_and_0755_dirs() {
    if std::env::var(CHILD_ENV).as_deref() == Ok("1") {
        assert_installed_modes();
        return;
    }

    let exe = std::env::current_exe().expect("test binary path");
    let script = format!(
        "umask 077; exec {:?} --exact {} --nocapture --test-threads=1",
        exe, CHILD_TEST
    );
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(&script)
        .env(CHILD_ENV, "1")
        .output()
        .expect("failed to re-run this test under a hostile umask");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains(CHILD_MARKER),
        "O3/P6 REQ-SKILLS-003: the umask-077 child never reached its assertions (marker \
         {CHILD_MARKER} absent). status {:?}\n--- child stdout ---\n{stdout}\n--- child stderr \
         ---\n{stderr}",
        out.status.code()
    );
    assert!(
        out.status.success(),
        "O3/P6 REQ-SKILLS-003: modes must not depend on the process umask. status {:?}\n\
         --- child stdout ---\n{stdout}\n--- child stderr ---\n{stderr}",
        out.status.code()
    );
}

// REQ-SKILLS-003 (Must) — Acceptance: the chosen owner is actually applied to the tree.
// Root-only: chown to a foreign uid is EPERM for anyone else. Skipped, never failed.
#[cfg(unix)]
#[test]
fn w7_apply_skills_ownership_sets_uid_and_gid_when_root() {
    use std::os::unix::fs::MetadataExt;

    if !running_as_root() {
        println!(
            "SKIP w7 (REQ-SKILLS-003 syscall path): not running as root, chown would be EPERM"
        );
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("skills");
    updater::install_skills_into(&sample_tarball(), &dest).expect("install must succeed");

    let target = (1u32, 1u32);
    updater::apply_skills_ownership(&dest, Some(target)).expect("chown must succeed as root");

    for p in [
        dest.clone(),
        dest.join("sk-m1-a"),
        dest.join("sk-m1-a/SKILL.md"),
    ] {
        let md = std::fs::metadata(&p).unwrap();
        assert_eq!(
            (md.uid(), md.gid()),
            target,
            "O4/P7 REQ-SKILLS-003: {} must be handed to the chosen owner",
            p.display()
        );
    }
}
