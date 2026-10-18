use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use crypto::Hash;
use storage::archiver::{self, ArchiveBlock, BlockArchiver, CatchUpReport};
use storage::BlockStore;
use tokio::sync::mpsc::Receiver;
use tracing::{error, info, warn};

use super::Node;

pub struct ArchiveThread {
    pub thread: JoinHandle<ArchiveThreadExit>,
    pub stale_dir: Option<PathBuf>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ArchiveThreadExit {
    pub catch_up_runs: u32,
    pub catch_up: Option<CatchUpReport>,
    pub live_archived: u64,
}

/// Fences a foreign-genesis archive (O(1)), then hands the stale-dir delete,
/// the catch-up and the live archive loop to one detached OS thread, the only
/// `manifest.json` writer. Returns without waiting for any of them.
pub fn start_archive_thread(
    node: &mut Node,
    archive_dir: PathBuf,
    tip: u64,
) -> std::io::Result<ArchiveThread> {
    let genesis = node.params.genesis_hash;
    let stale_dir = archiver::fence_stale_archive(&archive_dir, &genesis).unwrap_or_else(|e| {
        warn!(
            "[ARCHIVER] Genesis fence of {:?} failed: {}",
            archive_dir, e
        );
        None
    });
    if let Some(stale) = &stale_dir {
        info!("[ARCHIVER] Foreign-genesis archive fenced to {:?}", stale);
    }

    let (tx, rx) = tokio::sync::mpsc::channel(256);
    let job = Job {
        dir: archive_dir.clone(),
        store: node.block_store().clone(),
        genesis,
        tip,
        stale: stale_dir.clone(),
        rx,
    };
    let thread = std::thread::Builder::new()
        .name("doli-archiver".into())
        .spawn(move || job.run())?;
    node.set_archive_tx(tx, archive_dir);
    info!("Block archiver enabled");
    Ok(ArchiveThread { thread, stale_dir })
}

struct Job {
    dir: PathBuf,
    store: Arc<BlockStore>,
    genesis: Hash,
    tip: u64,
    stale: Option<PathBuf>,
    rx: Receiver<ArchiveBlock>,
}

impl Job {
    fn run(mut self) -> ArchiveThreadExit {
        let mut exit = ArchiveThreadExit::default();
        if let Some(stale) = &self.stale {
            match std::fs::remove_dir_all(stale) {
                Ok(()) => info!("[ARCHIVER] Removed stale archive {:?}", stale),
                Err(e) => warn!(
                    "[ARCHIVER] Failed to remove stale archive {:?}: {}",
                    stale, e
                ),
            }
        }
        if self.tip > 0 {
            exit.catch_up_runs = 1;
            exit.catch_up = self.catch_up();
        }
        while let Some(block) = self.rx.blocking_recv() {
            match archiver::archive_finalized(&self.dir, &self.store, &block, &self.genesis) {
                Ok(_) => exit.live_archived += 1,
                Err(e) => error!("[ARCHIVER] Failed to archive block {}: {}", block.height, e),
            }
        }
        info!(
            "[ARCHIVER] Channel closed: catch_up_runs={} verified_height={:?} live_archived={}",
            exit.catch_up_runs,
            exit.catch_up.map(|r| r.verified_height),
            exit.live_archived
        );
        exit
    }

    fn catch_up(&self) -> Option<CatchUpReport> {
        info!(
            "[ARCHIVER] Catch-up started: tip {} dir {:?}",
            self.tip, self.dir
        );
        let started = Instant::now();
        match BlockArchiver::catch_up_bounded(&self.dir, &self.store, self.tip, &self.genesis) {
            Ok(r) => {
                info!(
                    "[ARCHIVER] Catch-up finished: files_read={} files_written={} verified_height={} in {:?}",
                    r.files_read,
                    r.files_written,
                    r.verified_height,
                    started.elapsed()
                );
                Some(r)
            }
            Err(e) => {
                warn!(
                    "[ARCHIVER] Catch-up error after {:?}: {}",
                    started.elapsed(),
                    e
                );
                None
            }
        }
    }
}
