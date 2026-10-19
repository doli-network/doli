use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Suffix of every tmp file this archiver writes; other writers use `.tmp`.
const TMP_SUFFIX: &str = ".tmp-archiver";

pub(super) fn blake3_hex(data: &[u8]) -> String {
    crypto::hash::hash(data).to_string()
}

pub(super) fn block_path(dir: &Path, height: u64) -> PathBuf {
    dir.join(format!("{:010}.block", height))
}

pub(super) fn sum_path(dir: &Path, height: u64) -> PathBuf {
    dir.join(format!("{:010}.blake3", height))
}

pub(super) fn pair_exists(dir: &Path, height: u64) -> bool {
    block_path(dir, height).exists() && sum_path(dir, height).exists()
}

/// Writes the `.block` + `.blake3` pair; both are durable when this returns Ok.
pub(super) fn write_pair(dir: &Path, height: u64, data: &[u8]) -> io::Result<()> {
    write_durable(&block_path(dir, height), data)?;
    write_durable(&sum_path(dir, height), blake3_hex(data).as_bytes())
}

/// tmp -> write_all -> sync_all -> rename.
pub(super) fn write_durable(path: &Path, data: &[u8]) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(TMP_SUFFIX);
    let tmp = PathBuf::from(tmp);
    let res = write_then_rename(&tmp, path, data);
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

fn write_then_rename(tmp: &Path, path: &Path, data: &[u8]) -> io::Result<()> {
    let mut f = File::create(tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    std::fs::rename(tmp, path)
}

/// Best effort: makes earlier renames in `dir` durable.
pub(super) fn sync_dir(dir: &Path) {
    let _ = File::open(dir).and_then(|d| d.sync_all());
}
