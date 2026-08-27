//! Agent-skill extraction from a release tarball.
//!
//! Split out of `apply.rs` (INC-I-172 M1, AUDIT-P2-010) because this is the one place in
//! the updater that writes ATTACKER-NAMED paths as root: `sudo doli upgrade` and the
//! unattended auto-apply both call it with a tarball fetched from the release origin,
//! and until this change it joined a raw tar entry path onto the skills directory and
//! `fs::write`-d the result with no containment check at all.

use std::io::Read;
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use tar::Archive;
use tracing::{info, warn};

use crate::types::{Result, UpdateError};

/// Largest skill file this will write. Skills are markdown; anything larger is not one,
/// and `read_to_end` on an attacker-sized tar entry is an unbounded allocation as root.
const MAX_SKILL_BYTES: u64 = 4 * 1024 * 1024;

/// Is this tar entry path safe to join onto the skills directory? (AUDIT-P2-010)
///
/// `Path::join` with an ABSOLUTE argument REPLACES the base entirely: an entry named
/// `pkg/skills//etc/cron.d/pwn` yields the relative segment `/etc/cron.d/pwn`, and
/// `skills_dir.join("/etc/cron.d/pwn")` IS `/etc/cron.d/pwn`. That is the sharper of the
/// two cases — it does not escape the base, it discards it. `..` walks out the ordinary
/// way. Both were then written with `fs::write` as root.
///
/// Checked BEFORE any directory is created, so a rejected entry leaves nothing on disk.
/// The canonicalized containment check in [`install_skills_into`] is the belt to these
/// suspenders: it catches escapes these component rules cannot see (a symlinked ancestor,
/// say), but it can only run once the parent exists — so both are needed.
pub fn skill_entry_path_is_safe(relative: &str) -> bool {
    use std::path::Component;

    let path = Path::new(relative);
    if path.is_absolute() {
        return false;
    }
    path.components().all(|c| match c {
        Component::Normal(_) | Component::CurDir => true,
        // ParentDir escapes; RootDir / Prefix are the absolute forms (including Windows
        // `C:\` and UNC paths), which `is_absolute` alone does not catch cross-platform.
        Component::ParentDir | Component::RootDir | Component::Prefix(_) => false,
    })
}

/// Extract and install agent skills from a release tarball to `~/.doli/skills/`.
///
/// Skills are markdown files that enable AI agents to operate DOLI nodes. They live in
/// the tarball under `*/skills/**`, and any previously installed skills are replaced.
///
/// Best-effort: returns `Ok(count)` on success, `Err` on failure. Callers should treat
/// failure as non-fatal — skills are not required for node operation.
pub fn install_skills_from_tarball(tarball: &[u8]) -> Result<usize> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| UpdateError::InstallFailed("Cannot determine home directory".into()))?;
    let skills_dir = PathBuf::from(&home).join(".doli").join("skills");
    install_skills_into(tarball, &skills_dir)
}

/// The body of [`install_skills_from_tarball`], with the destination injected.
///
/// Separated so the containment rules can be tested against a temp directory instead of
/// the caller's real `$HOME` — a guard that runs as root deserves a test that does not
/// depend on process-wide environment mutation.
///
/// Every entry is checked before anything is written: no absolute paths, no `..`, no
/// symlink or hardlink entries, a per-file size cap, and a final containment check
/// against the canonicalized destination. A rejected entry is SKIPPED with a warning
/// rather than aborting: skills are best-effort, the binary is already installed by this
/// point, and turning a hostile entry into a hard failure would hand the origin a way to
/// fail every upgrade.
pub fn install_skills_into(tarball: &[u8], skills_dir: &Path) -> Result<usize> {
    let decoder = GzDecoder::new(tarball);
    let mut archive = Archive::new(decoder);
    let mut skill_count = 0;

    // Clear previous skills
    if skills_dir.exists() {
        std::fs::remove_dir_all(skills_dir).map_err(|e| {
            UpdateError::InstallFailed(format!("Failed to clear old skills: {}", e))
        })?;
    }

    // Re-create the destination immediately and resolve it ONCE, before any entry is
    // considered. This is load-bearing, not tidiness: `canonicalize` fails on a path that
    // does not exist, so a base resolved lazily inside the loop is UNRESOLVABLE for the
    // first entry — and a containment check that cannot resolve its base is a containment
    // check that does not run. Establishing it up front also makes a failure here a hard
    // error, which is correct: if the destination cannot be created there is nothing to
    // install anyway.
    std::fs::create_dir_all(skills_dir).map_err(|e| {
        UpdateError::InstallFailed(format!("Failed to create {}: {}", skills_dir.display(), e))
    })?;
    let base = skills_dir.canonicalize().map_err(|e| {
        UpdateError::InstallFailed(format!("Failed to resolve {}: {}", skills_dir.display(), e))
    })?;

    for entry in archive
        .entries()
        .map_err(|e| UpdateError::InstallFailed(e.to_string()))?
    {
        let entry = entry.map_err(|e| UpdateError::InstallFailed(e.to_string()))?;
        let path = entry
            .path()
            .map_err(|e| UpdateError::InstallFailed(e.to_string()))?
            .to_path_buf();

        // Match entries like "doli-v1.0.0-target/skills/core/SKILL.md"
        let path_str = path.to_string_lossy();
        let Some(skills_idx) = path_str.find("/skills/") else {
            continue;
        };
        let relative = path_str[skills_idx + "/skills/".len()..].to_string();
        if relative.is_empty() {
            continue;
        }

        // AUDIT-P2-010: reject before creating any directory, so a hostile entry leaves
        // nothing behind.
        if !skill_entry_path_is_safe(&relative) {
            warn!(
                "Refusing skill entry with an unsafe path: {:?}. Absolute paths and `..` \
                 segments escape {} and are written as root.",
                relative,
                skills_dir.display()
            );
            continue;
        }

        // Only extract regular files. A symlink or hardlink entry is a write primitive
        // aimed anywhere on the filesystem, and no legitimate skill needs one.
        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            warn!(
                "Refusing {:?} skill entry {:?}: link entries are never installed",
                entry_type, relative
            );
            continue;
        }
        if !entry_type.is_file() {
            continue;
        }

        let dest = skills_dir.join(&relative);
        let Some(parent) = dest.parent().map(Path::to_path_buf) else {
            continue;
        };
        std::fs::create_dir_all(&parent).map_err(|e| UpdateError::InstallFailed(e.to_string()))?;

        // Belt to the component check: resolve the parent that now exists and require it
        // to live under the canonicalized destination. This catches escapes the component
        // rules cannot see — a symlinked ancestor, for instance.
        let Ok(resolved_parent) = parent.canonicalize() else {
            warn!(
                "Refusing skill entry {:?}: could not resolve its destination under {}",
                relative,
                skills_dir.display()
            );
            continue;
        };
        if !resolved_parent.starts_with(&base) {
            warn!(
                "Refusing skill entry {:?}: it resolves to {}, outside {}",
                relative,
                resolved_parent.display(),
                base.display()
            );
            continue;
        }

        // Bounded read: an unbounded `read_to_end` on an attacker-sized entry is a
        // root-privileged allocation driven by the untrusted origin.
        let mut contents = Vec::new();
        let read = entry
            .take(MAX_SKILL_BYTES + 1)
            .read_to_end(&mut contents)
            .map_err(|e| UpdateError::InstallFailed(e.to_string()))?;
        if read as u64 > MAX_SKILL_BYTES {
            warn!(
                "Refusing skill entry {:?}: larger than the {}-byte cap",
                relative, MAX_SKILL_BYTES
            );
            continue;
        }
        std::fs::write(&dest, &contents).map_err(|e| UpdateError::InstallFailed(e.to_string()))?;

        if relative.ends_with("SKILL.md") {
            skill_count += 1;
        }
    }

    if skill_count > 0 {
        info!(
            "Installed {} agent skills to {}",
            skill_count,
            skills_dir.display()
        );
    }

    Ok(skill_count)
}
