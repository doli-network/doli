//! `check-producer-set`: offline, read-only residue check of a stopped node.
//!
//! Opens the node's `state_db` and `blocks` with RocksDB read-only opens (no
//! copy; RocksDB info logs go to the scratch dir), replays heights
//! `1..=best_height` through `apply_block(Replay)` on a headless node in a
//! scratch dir under the temp dir, and compares the rebuilt ProducerSet with
//! the persisted one. Nothing is written to the data dir. The scratch dir is
//! removed on every normal and error exit and on SIGINT / SIGTERM (exit 1).
//! On start, scratch dirs of check processes that are no longer alive are
//! removed; an abort that runs no cleanup leaves one for the next run.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use crypto::Hash;
use doli_core::validation::ValidationMode;
use doli_core::Network;
use storage::{BlockStore, ProducerSet, StateDb};

use crate::node::Node;

const EXIT_DIFFERS: i32 = 2;
const EXIT_NOT_CHECKABLE: i32 = 3;
const SCRATCH_PREFIX: &str = "doli-check-producer-set-";

/// Scratch directory `<temp>/doli-check-producer-set-<pid>-<nanos>`, removed
/// on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn create() -> Result<Self> {
        let root = std::env::temp_dir();
        #[cfg(unix)]
        sweep_stale(&root);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = root.join(format!("{SCRATCH_PREFIX}{}-{nanos}", std::process::id()));
        std::fs::create_dir(&dir)?;
        Ok(Self(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Removes `dir` and exits 1 on SIGINT / SIGTERM until dropped.
#[cfg(unix)]
struct SignalCleanup(tokio::task::JoinHandle<()>);

#[cfg(unix)]
impl SignalCleanup {
    fn arm(dir: &Path) -> Result<Self> {
        use tokio::signal::unix::{signal, SignalKind};
        let mut int = signal(SignalKind::interrupt())?;
        let mut term = signal(SignalKind::terminate())?;
        let dir = dir.to_path_buf();
        Ok(Self(tokio::spawn(async move {
            tokio::select! {
                _ = int.recv() => {},
                _ = term.recv() => {},
            }
            let _ = std::fs::remove_dir_all(&dir);
            std::process::exit(1);
        })))
    }
}

#[cfg(unix)]
impl Drop for SignalCleanup {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Best effort: removes scratch dirs under `root` whose pid is not a live
/// process. Names that do not parse are kept.
#[cfg(unix)]
fn sweep_stale(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        let pid = entry.file_name().to_str().and_then(scratch_pid);
        if is_dir && pid.is_some_and(pid_dead) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// `<pid>` of a `doli-check-producer-set-<pid>-<nanos>` name.
#[cfg(unix)]
fn scratch_pid(name: &str) -> Option<libc::pid_t> {
    let (pid, nanos) = name.strip_prefix(SCRATCH_PREFIX)?.split_once('-')?;
    nanos.parse::<u128>().ok()?;
    pid.parse::<libc::pid_t>().ok().filter(|p| *p > 0)
}

#[cfg(unix)]
fn pid_dead(pid: libc::pid_t) -> bool {
    // SAFETY: signal 0 only probes for existence.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

/// Runs the check and prints the verdict line plus its `key=value` summary.
///
/// Returns the exit code: 0 EQUAL, 2 DIFFERS, 3 NOT-CHECKABLE. Errors map to 1.
pub(crate) async fn check_producer_set(network: Network, data_dir: &Path) -> Result<i32> {
    let state_path = data_dir.join("state_db");
    let blocks_path = data_dir.join("blocks");
    for p in [&state_path, &blocks_path] {
        if !p.is_dir() {
            bail!("{} not found", p.display());
        }
    }
    if lock_held(&state_path)? || lock_held(&blocks_path)? {
        return not_checkable("lock_held");
    }

    let scratch = Scratch::create()?;
    #[cfg(unix)]
    let _on_signal = SignalCleanup::arm(&scratch.0)?;
    let state_db = StateDb::open_read_only(&state_path, &scratch.0)?;
    let cs = state_db
        .get_chain_state()
        .ok_or_else(|| anyhow!("state_db holds no chain state"))?;
    if cs.snap_sync_height.is_some() {
        return not_checkable("snap_sync_height");
    }
    let blocks = BlockStore::open_read_only(&blocks_path, &scratch.0)?;
    let tip = cs.best_height;
    if !blocks.has_contiguous_bodies(1, tip) {
        return not_checkable("holed_block_store");
    }
    let persisted = state_db.load_producer_set();
    drop(state_db);

    let Some((rebuilt, hash)) = rebuild(network, &blocks, tip, scratch.0.join("replay")).await?
    else {
        return not_checkable("holed_block_store");
    };
    if hash != cs.best_hash {
        bail!(
            "replayed block at height {tip} is {hash}, persisted best hash is {}",
            cs.best_hash
        );
    }

    let (pq, rq) = (queue_bytes(&persisted)?, queue_bytes(&rebuilt)?);
    let (pn, rn) = (
        persisted.all_producers().len(),
        rebuilt.all_producers().len(),
    );
    let (pk, rk) = (persisted.as_parts().2.len(), rebuilt.as_parts().2.len());
    if persisted.serialize_canonical() == rebuilt.serialize_canonical() && pq == rq {
        return report(
            "EQUAL",
            &format!("height={tip} producers={pn} pending={pk}"),
            0,
        );
    }
    let mismatches = status_mismatches(&persisted, &rebuilt);
    report(
        "DIFFERS",
        &format!(
            "height={tip} producers={pn}/{rn} status_mismatches={mismatches} pending={pk}/{rk}"
        ),
        EXIT_DIFFERS,
    )
}

/// Replays `1..=tip` from `blocks` into a headless node at `dir`.
///
/// `None` when a height has no block.
async fn rebuild(
    network: Network,
    blocks: &BlockStore,
    tip: u64,
    dir: PathBuf,
) -> Result<Option<(ProducerSet, Hash)>> {
    let mut node = Node::new_for_replay(dir, network)
        .await
        .map_err(|e| anyhow!("Failed to construct replay node: {}", e))?;
    for height in 1..=tip {
        let Some(block) = blocks.get_block_by_height(height)? else {
            return Ok(None);
        };
        node.apply_block(block, ValidationMode::Replay)
            .await
            .map_err(|e| anyhow!("apply_block failed at height {}: {}", height, e))?;
    }
    let ps = node.producer_set.read().await.clone();
    let hash = node.chain_state.read().await.best_hash;
    Ok(Some((ps, hash)))
}

fn queue_bytes(ps: &ProducerSet) -> Result<Vec<u8>> {
    Ok(bincode::serialize(ps.as_parts().2)?)
}

/// Producers whose status differs; a producer on one side only counts.
fn status_mismatches(a: &ProducerSet, b: &ProducerSet) -> usize {
    let (pa, pb) = (a.as_parts().0, b.as_parts().0);
    let keys: HashSet<&Hash> = pa.keys().chain(pb.keys()).collect();
    keys.into_iter()
        .filter(|k| pa.get(*k).map(|p| &p.status) != pb.get(*k).map(|p| &p.status))
        .count()
}

fn not_checkable(reason: &str) -> Result<i32> {
    report(
        "NOT-CHECKABLE",
        &format!("reason={reason}"),
        EXIT_NOT_CHECKABLE,
    )
}

fn report(verdict: &str, summary: &str, code: i32) -> Result<i32> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{verdict}\n{summary}")?;
    out.flush()?;
    Ok(code)
}

/// True when another process holds the RocksDB `LOCK` file of `db_dir`.
///
/// Probes with a non-blocking fcntl write lock (the lock kind RocksDB takes)
/// and releases it at once. Never creates or writes the file.
#[cfg(unix)]
fn lock_held(db_dir: &Path) -> Result<bool> {
    use std::os::unix::io::AsRawFd;
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(db_dir.join("LOCK"))
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    // SAFETY: `flock` is plain data; all-zero is the whole-file range.
    let mut fl: libc::flock = unsafe { std::mem::zeroed() };
    fl.l_type = libc::F_WRLCK as _;
    fl.l_whence = libc::SEEK_SET as _;
    // SAFETY: `file` is an open descriptor and `fl` outlives the call.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &fl) } == -1 {
        let err = std::io::Error::last_os_error();
        return match err.raw_os_error() {
            Some(libc::EAGAIN) | Some(libc::EACCES) => Ok(true),
            _ => Err(err.into()),
        };
    }
    fl.l_type = libc::F_UNLCK as _;
    // SAFETY: as above.
    unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &fl) };
    Ok(false)
}

#[cfg(not(unix))]
fn lock_held(_db_dir: &Path) -> Result<bool> {
    bail!("check-producer-set needs a unix host for its LOCK probe")
}
