use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

fn track(path: &Path) {
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

// A missing rerun-if-changed path is always dirty, so only existing paths are declared.
fn track_git_head() {
    let Some(manifest) = env::var_os("CARGO_MANIFEST_DIR") else {
        return;
    };
    let repo = PathBuf::from(manifest).join("../..");
    let dot_git = repo.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        let Some(dir) = fs::read_to_string(&dot_git).ok().and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("gitdir:"))
                .map(|p| repo.join(p.trim()))
        }) else {
            return;
        };
        dir
    };
    let common_dir = fs::read_to_string(git_dir.join("commondir"))
        .map(|c| git_dir.join(c.trim()))
        .unwrap_or_else(|_| git_dir.clone());
    let head = git_dir.join("HEAD");
    track(&head);
    if let Some(name) = fs::read_to_string(&head)
        .ok()
        .and_then(|h| h.strip_prefix("ref:").map(|r| r.trim().to_string()))
    {
        let mut ref_file = git_dir.join(&name);
        if !ref_file.exists() {
            ref_file = common_dir.join(&name);
        }
        track(&ref_file);
    }
    track(&common_dir.join("refs/heads"));
}

fn main() {
    track_git_head();
    let hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    let hash = hash.trim();
    let version = env!("CARGO_PKG_VERSION");
    if hash.is_empty() {
        println!("cargo:rustc-env=DOLI_VERSION_STRING={version}");
    } else {
        println!("cargo:rustc-env=DOLI_VERSION_STRING={version} ({hash})");
    }
}
