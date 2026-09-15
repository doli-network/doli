//! Where the skills tree goes, and who owns it (REQ-SKILLS-001/002/003).
//!
//! Split out of `skills.rs` for the module budget. The resolution and the ownership
//! DECISION are pure functions with their environment injected, so both are testable
//! without root and without process-wide env mutation.

use std::path::{Path, PathBuf};

use crate::types::{Result, UpdateError};

/// Environment inputs of the skills-directory resolution.
#[derive(Debug, Clone, Default)]
pub struct SkillsEnv {
    pub skills_dir: Option<String>,
    pub sudo_user: Option<String>,
    pub home: Option<String>,
    pub user_profile: Option<String>,
}

impl SkillsEnv {
    /// Reads the four variables off the process environment.
    pub fn from_process() -> Self {
        Self {
            skills_dir: std::env::var("DOLI_SKILLS_DIR").ok(),
            sudo_user: std::env::var("SUDO_USER").ok(),
            home: std::env::var("HOME").ok(),
            user_profile: std::env::var("USERPROFILE").ok(),
        }
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    let trimmed = value?.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// Resolve the skills directory from injected environment values.
///
/// Precedence: explicit `DOLI_SKILLS_DIR` verbatim, then the `SUDO_USER` real home, then
/// `HOME`/`USERPROFILE`. An undeterminable home is an error, never a guessed path.
pub fn skills_dir_for(env: &SkillsEnv) -> Result<PathBuf> {
    if let Some(dir) = non_empty(env.skills_dir.as_deref()) {
        return Ok(PathBuf::from(dir));
    }
    if let Some(user) = non_empty(env.sudo_user.as_deref()).filter(|u| *u != "root") {
        return Ok(real_home_dir_of(user).join(".doli").join("skills"));
    }
    let home = non_empty(env.home.as_deref())
        .or_else(|| non_empty(env.user_profile.as_deref()))
        .ok_or_else(|| UpdateError::InstallFailed("Cannot determine home directory".into()))?;
    Ok(PathBuf::from(home).join(".doli").join("skills"))
}

/// [`skills_dir_for`] against the process environment.
pub fn skills_dir() -> Result<PathBuf> {
    skills_dir_for(&SkillsEnv::from_process())
}

/// The home directory of `user`, through the platform's account database.
pub fn real_home_dir_of(user: &str) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("dscl")
            .args([
                ".",
                "-read",
                &format!("/Users/{}", user),
                "NFSHomeDirectory",
            ])
            .output()
        {
            if let Ok(s) = String::from_utf8(output.stdout) {
                if let Some(home) = s.split_whitespace().last() {
                    return PathBuf::from(home);
                }
            }
        }
        PathBuf::from(format!("/Users/{}", user))
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Ok(output) = std::process::Command::new("getent")
            .args(["passwd", user])
            .output()
        {
            if let Ok(s) = String::from_utf8(output.stdout) {
                if let Some(home) = s.split(':').nth(5) {
                    let home = home.trim();
                    if !home.is_empty() {
                        return PathBuf::from(home);
                    }
                }
            }
        }
        PathBuf::from(format!("/home/{}", user))
    }
}

/// Environment inputs of the ownership decision.
#[derive(Debug, Clone, Default)]
pub struct SkillsOwnerEnv {
    pub sudo_uid: Option<String>,
    pub sudo_gid: Option<String>,
}

impl SkillsOwnerEnv {
    /// Reads `SUDO_UID` / `SUDO_GID` off the process environment.
    pub fn from_process() -> Self {
        Self {
            sudo_uid: std::env::var("SUDO_UID").ok(),
            sudo_gid: std::env::var("SUDO_GID").ok(),
        }
    }
}

/// Which `(uid, gid)` the skills tree must end up owned by. Pure — no syscalls.
///
/// `existing` is the owner read off the directory before it was removed. `None` out means
/// "request no chown": the creating process is already the right owner.
pub fn skills_owner_for(existing: Option<(u32, u32)>, env: &SkillsOwnerEnv) -> Option<(u32, u32)> {
    if let Some(owner) = existing {
        return Some(owner);
    }
    let uid = non_empty(env.sudo_uid.as_deref())?.parse::<u32>().ok()?;
    let gid = non_empty(env.sudo_gid.as_deref())?.parse::<u32>().ok()?;
    Some((uid, gid))
}

/// The `(uid, gid)` of `path`, when it is there. `None` off unix.
pub fn existing_owner(path: &Path) -> Option<(u32, u32)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let md = std::fs::metadata(path).ok()?;
        Some((md.uid(), md.gid()))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// Applies the ownership decision to the whole tree. No-op with no owner, or off unix.
pub fn apply_skills_ownership(skills_dir: &Path, owner: Option<(u32, u32)>) -> std::io::Result<()> {
    let Some((uid, gid)) = owner else {
        return Ok(());
    };
    #[cfg(unix)]
    {
        chown_tree(skills_dir, uid, gid)
    }
    #[cfg(not(unix))]
    {
        let _ = (skills_dir, uid, gid);
        Ok(())
    }
}

/// `lchown`, not `chown`: a link entry must never redirect a root-privileged ownership
/// change onto a file outside the tree.
#[cfg(unix)]
fn chown_tree(path: &Path, uid: u32, gid: u32) -> std::io::Result<()> {
    std::os::unix::fs::lchown(path, Some(uid), Some(gid))?;
    if !std::fs::symlink_metadata(path)?.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(path)? {
        chown_tree(&entry?.path(), uid, gid)?;
    }
    Ok(())
}

/// Sets dirs to 0755 and files to 0644 across the tree (REQ-SKILLS-003).
///
/// Explicit, because the modes the installer would otherwise get are whatever umask the
/// calling service happens to run with. Best-effort: a mode failure never fails an install.
pub fn normalize_skill_modes(path: &Path) {
    #[cfg(unix)]
    {
        let Ok(md) = std::fs::symlink_metadata(path) else {
            return;
        };
        if md.file_type().is_symlink() {
            return;
        }
        set_mode(path, if md.is_dir() { 0o755 } else { 0o644 });
        if !md.is_dir() {
            return;
        }
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            normalize_skill_modes(&entry.path());
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}
