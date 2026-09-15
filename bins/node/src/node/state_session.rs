//! M3 [F3] serving-side state sessions: one pinned UTXO view per transfer.
//!
//! `canonical_range` re-pins per call, so a multi-message transfer served that
//! way would tear whenever the chain advanced. Each session therefore owns a
//! worker thread that creates the pin INSIDE its own scope and answers chunk
//! jobs from it until the walk is exhausted, the TTL expires, or it goes idle.
//! The pin never escapes the closure, and the worker holds no lock on
//! `utxo_set`, so a live session cannot block `apply_block`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crypto::Hash;
use network::protocols::sync::{STATE_SESSION_IDLE_TIMEOUT_SECS, STATE_SESSION_TTL_SECS};
use storage::{StorageError, UtxoSet};
use tracing::warn;

type RangeResult = Result<(Vec<u8>, Option<Vec<u8>>), String>;

struct ChunkJob {
    start_key: Option<Vec<u8>>,
    max_bytes: usize,
    reply: tokio::sync::oneshot::Sender<RangeResult>,
}

struct SessionHandle {
    jobs: std::sync::mpsc::Sender<ChunkJob>,
    created: Instant,
    last_used: Instant,
}

/// What a freshly opened session announces about its pinned view.
pub struct SessionOpening {
    pub session_id: u64,
    pub utxo_hash: Hash,
    pub utxo_count: u64,
}

/// The bounded set of sessions one node will serve at a time.
#[derive(Default)]
pub struct StateSessionRegistry {
    live: Mutex<HashMap<u64, SessionHandle>>,
    next_id: AtomicU64,
}

impl StateSessionRegistry {
    fn ttl() -> Duration {
        Duration::from_secs(STATE_SESSION_TTL_SECS)
    }

    fn idle() -> Duration {
        Duration::from_secs(STATE_SESSION_IDLE_TIMEOUT_SECS)
    }

    /// Drop sessions past their TTL or idle window. Dropping the job sender ends
    /// the worker, which releases the pin.
    pub fn evict_expired(&self) {
        let mut live = match self.live.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        live.retain(|id, h| {
            let alive = h.created.elapsed() < Self::ttl() && h.last_used.elapsed() < Self::idle();
            if !alive {
                warn!(
                    "[SNAP_SYNC] State session {} expired — releasing pinned view",
                    id
                );
            }
            alive
        });
    }

    pub fn live_count(&self) -> usize {
        match self.live.lock() {
            Ok(g) => g.len(),
            Err(poisoned) => poisoned.into_inner().len(),
        }
    }

    pub fn release(&self, session_id: u64) {
        let mut live = match self.live.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        live.remove(&session_id);
    }

    /// Start a worker that pins `handle` and answers chunk jobs from it.
    ///
    /// `handle` must already be an owned, lock-free view of the set
    /// (`UtxoSet::pinnable_handle`).
    pub async fn open(&self, handle: UtxoSet) -> Result<SessionOpening, StorageError> {
        let session_id = self.next_id.fetch_add(1, Ordering::Relaxed) ^ 0xD0_11_5E_55_00_00_00_00;
        let (jobs_tx, jobs_rx) = std::sync::mpsc::channel::<ChunkJob>();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();

        std::thread::Builder::new()
            .name(format!("doli-state-session-{}", session_id))
            .spawn(move || {
                handle.with_pinned_view(|view| {
                    let opened = view
                        .canonical_row_count()
                        .and_then(|count| view.canonical_digest_and_len().map(|(h, _)| (h, count)));
                    if ready_tx.send(opened).is_err() {
                        return;
                    }
                    let deadline = Instant::now() + Self::ttl();
                    while let Ok(job) = jobs_rx.recv_timeout(Self::idle()) {
                        if Instant::now() > deadline {
                            break;
                        }
                        let answer = view
                            .canonical_range(job.start_key.as_deref(), job.max_bytes)
                            .map_err(|e| e.to_string());
                        let _ = job.reply.send(answer);
                    }
                });
            })
            .map_err(|e| StorageError::Database(format!("state session thread: {}", e)))?;

        let (utxo_hash, utxo_count) = ready_rx
            .await
            .map_err(|_| StorageError::Database("state session worker died".to_string()))??;

        let now = Instant::now();
        let mut live = match self.live.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        live.insert(
            session_id,
            SessionHandle {
                jobs: jobs_tx,
                created: now,
                last_used: now,
            },
        );

        Ok(SessionOpening {
            session_id,
            utxo_hash,
            utxo_count,
        })
    }

    /// Ask a live session for one range. `None` means the session is gone.
    pub async fn request_range(
        &self,
        session_id: u64,
        start_key: Option<Vec<u8>>,
        max_bytes: usize,
    ) -> Option<RangeResult> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        {
            let mut live = match self.live.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            let handle = live.get_mut(&session_id)?;
            handle.last_used = Instant::now();
            handle
                .jobs
                .send(ChunkJob {
                    start_key,
                    max_bytes,
                    reply: reply_tx,
                })
                .ok()?;
        }
        reply_rx.await.ok()
    }
}
