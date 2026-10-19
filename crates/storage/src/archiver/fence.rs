use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::warn;

use super::manifest::read_manifest_genesis_hash;

/// O(1) chain-reset fence: reads only `manifest.json`. When its `genesis_hash`
/// is present and differs from `expected_genesis`, renames `dir` to
/// `<dir>.stale-<unix-ts>`, recreates an empty `dir` and returns the stale path.
/// When the rename fails, empties `dir` in place (`Ok(None)`); `Err` when that
/// fails too. Anything not provably foreign is left alone (`Ok(None)`).
pub fn fence_stale_archive(
    dir: &Path,
    expected_genesis: &crypto::Hash,
) -> io::Result<Option<PathBuf>> {
    let foreign = read_manifest_genesis_hash(dir).filter(|g| *g != expected_genesis.to_string());
    let Some(archive_genesis) = foreign else {
        return Ok(None);
    };
    let stale = match free_stale_path(dir).and_then(|s| std::fs::rename(dir, &s).map(|()| s)) {
        Ok(stale) => stale,
        Err(e) => {
            warn!(
                "[ARCHIVER] Archive genesis {:.16} != expected {:.16}: cannot move {:?} ({}), removing its contents",
                archive_genesis,
                expected_genesis.to_string(),
                dir,
                e
            );
            empty_dir(dir)?;
            return Ok(None);
        }
    };
    std::fs::create_dir_all(dir)?;
    warn!(
        "[ARCHIVER] Archive genesis {:.16} != expected {:.16}: moved {:?} to {:?}",
        archive_genesis,
        expected_genesis.to_string(),
        dir,
        stale
    );
    Ok(Some(stale))
}

/// Removes every entry inside `dir`, `manifest.json` last; symlinks are not
/// followed. Repeats until a pass finds nothing: readdir may skip entries
/// removed during the same pass.
fn empty_dir(dir: &Path) -> io::Result<()> {
    loop {
        let mut removed = false;
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_name() == "manifest.json" {
                continue;
            }
            if entry.file_type()?.is_dir() {
                std::fs::remove_dir_all(entry.path())?;
            } else {
                std::fs::remove_file(entry.path())?;
            }
            removed = true;
        }
        if !removed {
            return std::fs::remove_file(dir.join("manifest.json"));
        }
    }
}

fn free_stale_path(dir: &Path) -> io::Result<PathBuf> {
    let name = dir
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "archive dir has no name"))?;
    let mut ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    loop {
        let mut stale = name.to_os_string();
        stale.push(format!(".stale-{ts}"));
        let path = dir.with_file_name(stale);
        if !path.exists() {
            return Ok(path);
        }
        ts += 1;
    }
}
