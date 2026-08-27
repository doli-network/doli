// INC-I-172 M1 security audit, AUDIT-P2-010 — archive path traversal in the skill
// installer: arbitrary file write as ROOT.
// REQ-172-006 (Must).
//
// THE DEFECT this locks shut: `install_skills_from_tarball` took the entry path straight
// out of the tarball, sliced everything after "/skills/", joined it onto
// `~/.doli/skills/` and `fs::write`-d it. It runs on `sudo doli upgrade` and on every
// unattended auto-apply, so the writer is root and the path is named by the release
// origin. Two distinct escapes:
//
//   - `..` walks out of the base the ordinary way;
//   - an ABSOLUTE entry is worse. `Path::join` with an absolute argument REPLACES the
//     base, so `pkg/skills//etc/cron.d/pwn` yields the segment `/etc/cron.d/pwn` and
//     `skills_dir.join(that)` IS `/etc/cron.d/pwn`. Nothing "escapes"; the destination is
//     simply discarded.
//
// ============================================================================
// OUTPUT CONTRACT
// ============================================================================
// Functions under test:
//   `updater::skill_entry_path_is_safe(&str) -> bool`
//   `updater::install_skills_into(&[u8], &Path) -> Result<usize>`
//
// ENUMERATION OF OBSERVABLE OUTPUTS.
//   - return value     : O1 the predicate's bool; O2 the installed SKILL.md count.
//   - mutable params   : NONE (both take shared refs).
//   - persistent store : O3 — THE FILESYSTEM. This is the finding: which paths exist
//                        after the call, inside AND outside the destination. O3 is the
//                        primary observable; O2 alone would pass on a guard that writes
//                        the file and then declines to count it.
//   - side channel     : one `warn!` per refused entry. DECLARED UNASSERTED — a refusal
//                        is fully visible in O3 (the file is absent).
//
// CODE PATHS of `install_skills_into`:
//   P1: entry outside "/skills/"            -> skipped (not exercised; pre-existing)
//   P2: entry with `..`                     -> refused [component guard]
//   P3: entry with an ABSOLUTE path         -> refused [component guard]
//   P4: symlink / hardlink entry            -> refused [entry-type guard]
//   P5: entry over MAX_SKILL_BYTES          -> refused [size guard]
//   P6: an ordinary nested skill file       -> INSTALLED (control)
//
// INPUT PARTITIONS: one tarball per hostile shape, plus a benign one. The benign case is
// load-bearing: without it every assertion below would pass on an installer that writes
// nothing at all.
//
// NOT asserted, and why: the real end-to-end observable is a root-owned file appearing at
// an absolute path such as `/etc/cron.d/`. A test cannot write there, and should not try.
// The temp-directory sibling used here (`<tmp>/outside/`) is the same defect with a
// writable target, and the ABSOLUTE-path case additionally asserts against a path the
// test constructs itself, so the escape is demonstrated rather than assumed.
// ============================================================================

use std::path::Path;

use flate2::write::GzEncoder;
use flate2::Compression;

/// Build a .tar.gz whose entries carry EXACTLY these names.
///
/// The raw 100-byte name field is written directly instead of calling
/// `Header::set_path`, because `set_path` is itself a sanitiser: it REJECTS any path
/// containing `..` and it COLLAPSES `skills//tmp/x` to `skills/tmp/x`. A fixture built
/// through it produces a harmless archive, and every assertion in this file would then
/// pass against the unfixed installer. Verified empirically before this file was
/// committed: with `set_path`, the hostile entries never reach the guards at all.
fn tarball(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
    for (path, contents) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        write_raw_name(&mut header, path);
        header.set_cksum();
        builder.append(&header, *contents).unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}

/// Write `path` verbatim into the header's name field, bypassing every sanitiser.
fn write_raw_name(header: &mut tar::Header, path: &str) {
    let bytes = path.as_bytes();
    let name = &mut header.as_old_mut().name;
    assert!(
        bytes.len() <= name.len(),
        "fixture path {path:?} does not fit the 100-byte tar name field"
    );
    name[..bytes.len()].copy_from_slice(bytes);
}

/// A tarball with one link entry of the given type pointing at `target`.
fn link_tarball(path: &str, target: &str, kind: tar::EntryType) -> Vec<u8> {
    let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
    let mut header = tar::Header::new_gnu();
    header.set_size(0);
    header.set_mode(0o777);
    header.set_entry_type(kind);
    write_raw_name(&mut header, path);
    header.set_link_name(target).unwrap();
    header.set_cksum();
    builder.append(&header, std::io::empty()).unwrap();
    builder.into_inner().unwrap().finish().unwrap()
}

fn skills_dir(root: &Path) -> std::path::PathBuf {
    let d = root.join("home").join(".doli").join("skills");
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// REQ-172-006 (Must). RED before the fix.
/// Acceptance: the predicate rejects both escape shapes and accepts ordinary names.
/// [P2, P3, P6 -> O1]
#[test]
fn the_entry_path_predicate_rejects_escapes_and_accepts_ordinary_names() {
    for hostile in [
        "../../../../etc/cron.d/pwn",
        "..",
        "a/../../b",
        "/etc/cron.d/pwn",
        "/etc/passwd",
    ] {
        assert!(
            !updater::skill_entry_path_is_safe(hostile),
            "{hostile:?} must be refused: it is joined onto the skills dir and written as \
             root. An ABSOLUTE name does not escape the base — `Path::join` DISCARDS the \
             base entirely (AUDIT-P2-010)."
        );
    }
    for benign in ["core/SKILL.md", "SKILL.md", "./a/b/SKILL.md", "a/b/c.md"] {
        assert!(
            updater::skill_entry_path_is_safe(benign),
            "{benign:?} is an ordinary skill path and must still install"
        );
    }
}

/// REQ-172-006 (Must). RED before the fix.
/// Acceptance: a `..` entry writes NOTHING outside the destination.
/// [P2 -> O2, O3]
#[test]
fn a_parent_dir_entry_writes_nothing_outside_the_destination() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = skills_dir(tmp.path());
    // `<tmp>/home/.doli/skills` + `../../../PWNED` resolves to `<tmp>/PWNED`: three
    // levels up from the skills dir. Counted, not guessed — an assertion against the
    // wrong path passes on the UNFIXED installer and proves nothing.
    let escape_target = tmp.path().join("PWNED");

    let tar = tarball(&[("pkg/skills/../../../PWNED", b"owned" as &[u8])]);
    let installed = updater::install_skills_into(&tar, &dest).expect("install must not abort");

    assert_eq!(installed, 0, "a refused entry must not be counted");
    assert!(
        !escape_target.exists(),
        "a `..` tar entry wrote {} — outside {}. This function runs as ROOT on \
         `sudo doli upgrade` and on every unattended auto-apply.",
        escape_target.display(),
        dest.display()
    );
}

/// REQ-172-006 (Must). RED before the fix.
/// Acceptance: an ABSOLUTE entry path writes nothing at the absolute location. This is
/// the shape the audit calls out specifically, because it does not look like traversal:
/// there is no `..` anywhere in it.
/// [P3 -> O2, O3]
#[test]
fn an_absolute_entry_path_does_not_replace_the_destination() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = skills_dir(tmp.path());
    // An absolute path inside the temp dir: writable, so the test observes the real
    // defect rather than a permission error standing in for a fix.
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let absolute_victim = outside.join("pwn");

    let entry = format!("pkg/skills/{}", absolute_victim.display());
    let tar = tarball(&[(entry.as_str(), b"owned" as &[u8])]);
    let installed = updater::install_skills_into(&tar, &dest).expect("install must not abort");

    assert_eq!(installed, 0);
    assert!(
        !absolute_victim.exists(),
        "an ABSOLUTE tar entry wrote {}. `skills_dir.join(\"/abs/path\")` IS `/abs/path` \
         — the destination is discarded, not escaped, so a `..`-only check would miss \
         this entirely (AUDIT-P2-010).",
        absolute_victim.display()
    );
}

/// REQ-172-006 (Must). GREEN-lock, NOT red-before-fix — stated rather than implied: the
/// pre-fix code already skipped links, because it wrote only entries whose type
/// `is_file()`. This test exists so a later refactor cannot restore them by widening that
/// condition.
/// Acceptance: symlink and hardlink entries are never materialised. A link entry is a
/// write primitive aimed anywhere on the filesystem, and a later entry writing "through"
/// it defeats the path checks entirely.
/// [P4 -> O2, O3]
#[test]
fn link_entries_are_never_installed() {
    for kind in [tar::EntryType::Symlink, tar::EntryType::Link] {
        let tmp = tempfile::tempdir().unwrap();
        let dest = skills_dir(tmp.path());
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();

        let tar = link_tarball("pkg/skills/escape", outside.to_str().unwrap(), kind);
        updater::install_skills_into(&tar, &dest).expect("install must not abort");

        let link = dest.join("escape");
        assert!(
            !link.exists() && link.symlink_metadata().is_err(),
            "a {kind:?} entry was materialised at {}. No skill needs a link, and one \
             pointed at a directory turns every later entry into an out-of-tree write.",
            link.display()
        );
    }
}

/// REQ-172-006 (Must). GREEN-lock.
/// Acceptance: an ordinary nested skill still installs. Without this control, every
/// assertion above would pass on an installer that writes nothing at all.
/// [P6 -> O2, O3]
#[test]
fn an_ordinary_nested_skill_still_installs() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = skills_dir(tmp.path());

    let tar = tarball(&[
        ("pkg/skills/core/SKILL.md", b"# core" as &[u8]),
        ("pkg/skills/core/notes.md", b"notes" as &[u8]),
    ]);
    let installed = updater::install_skills_into(&tar, &dest).expect("install must succeed");

    assert_eq!(
        installed, 1,
        "exactly one SKILL.md is present in the fixture, so the count must be 1 — if this \
         is 0 the guards are refusing legitimate skills and every other test here is vacuous"
    );
    assert_eq!(
        std::fs::read_to_string(dest.join("core/SKILL.md")).unwrap(),
        "# core"
    );
    assert!(dest.join("core/notes.md").exists());
}
