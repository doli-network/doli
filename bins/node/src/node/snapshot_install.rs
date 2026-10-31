//! M4 [F3]: the staged half of `apply_snap_snapshot`, plus the startup
//! reconciliation of `cf_utxo_staging`.
//!
//! The rows are already durable when this runs — the session client streamed
//! them in. Installing is one `atomic_replace` out of the staging family, and
//! the whole canonical image is never resident.

use anyhow::Result;
use storage::{ChainState, ProducerSet};
use tracing::{error, info, warn};

use super::Node;

/// Bound on the local header walk that proves a snapshot anchor descends from
/// the finalized block. Only stored headers are walked, so a lagging node (no
/// headers for the gap) fails at the second step; it is admitted by the
/// peer-served chain instead (`SyncManager::snap_descent_proven`).
const MAX_FINALITY_PROOF_WALK: u64 = 1024;

/// The durable 3-state produced by a successful `SnapshotPrelude::Installed`.
pub(crate) struct InstalledSnapshot {
    pub(crate) chain_state: ChainState,
    pub(crate) producer_set: ProducerSet,
}

/// Outcome of the durable half of `apply_snap_snapshot`.
pub(crate) enum SnapshotPrelude {
    /// The durable 3-state is installed. On the staged arm `rebuild_in_progress`
    /// is ARMED and stays armed until the post-install root check passes.
    /// Boxed: this variant is ~360 bytes against `Refused`'s 0.
    Installed(Box<InstalledSnapshot>),
    /// Nothing was installed and nothing was armed; the caller falls back.
    Refused,
}

impl Node {
    /// Persist the snapshot's 3-state, by promoting staged rows when the session
    /// streamed them and by decoding the carried image otherwise.
    pub(crate) async fn install_snapshot_prelude(
        &self,
        snapshot: &network::VerifiedSnapshot,
    ) -> Result<SnapshotPrelude> {
        let outcome = if !self.snap_anchor_authenticated(snapshot)
            || !self.snap_anchor_keeps_finality(snapshot).await
        {
            Ok(SnapshotPrelude::Refused)
        } else if let Some(marker) = snapshot.utxo_staged.as_ref() {
            self.install_staged_utxos(snapshot, marker).await
        } else {
            return self.install_decoded_utxos(snapshot).await;
        };
        // Quarantine: rows a refused install left behind would be concatenated
        // onto the next peer's stream and match neither manifest.
        if snapshot.utxo_staged.is_some() && matches!(outcome, Ok(SnapshotPrelude::Refused)) {
            if let Err(e) = self.state_db.clear_staged_utxos() {
                warn!(
                    "[SNAP_SYNC] Failed to clear staging after a refused install: {}",
                    e
                );
            }
        }
        outcome
    }

    /// INC-I-289 K-2: at or above the fork-choice gate, with a finalized
    /// checkpoint, a snapshot is admitted only when its anchor provably descends
    /// from it. Unproven is refused (fail-closed); the caller falls back to
    /// normal sync.
    async fn snap_anchor_keeps_finality(&self, snapshot: &network::VerifiedSnapshot) -> bool {
        let sm = self.sync_manager.read().await;
        let fin = sm.fork_choice_finality();
        let gate = self
            .config
            .network
            .params()
            .inc_i_288_fork_choice_activation_height;
        let (Some(fh), Some(f)) = (fin.finalized_height, fin.finalized_hash) else {
            return true;
        };
        if fin.local_tip_height < gate
            || self.snap_anchor_descends_from(snapshot, fh, f)
            || sm.snap_descent_proven(snapshot.block_height, snapshot.block_hash, f)
        {
            return true;
        }
        warn!(
            "[SNAP_SYNC] Refusing snapshot h={} hash={}: anchor not proven to descend from finalized h={} {}",
            snapshot.block_height, snapshot.block_hash, fh, f
        );
        false
    }

    /// Walk parents from the anchor (its carried header first, then stored
    /// headers) until a hash our canonical index holds at that height. The
    /// anchor descends from `f` iff that meeting height is `>= fh` and the
    /// canonical index still holds `f` at `fh`.
    fn snap_anchor_descends_from(
        &self,
        snapshot: &network::VerifiedSnapshot,
        fh: u64,
        f: crypto::Hash,
    ) -> bool {
        let canonical = |h: u64| self.block_store.get_hash_by_height(h).ok().flatten();
        if canonical(fh) != Some(f) {
            return false;
        }
        let mut carried = snapshot
            .block_header_bytes
            .as_ref()
            .and_then(|b| bincode::deserialize::<doli_core::BlockHeader>(b).ok())
            .filter(|h| h.hash() == snapshot.block_hash);
        let (mut hash, mut height) = (snapshot.block_hash, snapshot.block_height);
        if height < fh {
            return false;
        }
        for _ in 0..=MAX_FINALITY_PROOF_WALK {
            if canonical(height) == Some(hash) {
                return true;
            }
            if height == fh {
                return false;
            }
            let header = match carried.take() {
                Some(h) => h,
                None => match self.block_store.get_header(&hash) {
                    Ok(Some(h)) => h,
                    _ => return false,
                },
            };
            hash = header.prev_hash;
            height -= 1;
        }
        false
    }

    /// M1/M3 path: the whole canonical image arrived on the wire.
    async fn install_decoded_utxos(
        &self,
        snapshot: &network::VerifiedSnapshot,
    ) -> Result<SnapshotPrelude> {
        let (computed_root, mut new_chain_state, new_utxo_set, new_producer_set) =
            match storage::verify_state_root_from_bytes_gated(
                &snapshot.chain_state,
                &snapshot.utxo_set,
                &snapshot.producer_set,
                self.pending_root_ah(),
            ) {
                Ok(decoded) => decoded,
                Err(e) => {
                    error!(
                        "[SNAP_SYNC] Snapshot deserialization failed at height={}: {} — rejecting",
                        snapshot.block_height, e
                    );
                    return Ok(SnapshotPrelude::Refused);
                }
            };
        if computed_root != snapshot.state_root {
            error!(
                "[SNAP_SYNC] State root mismatch! computed={}, expected={} — rejecting",
                computed_root, snapshot.state_root
            );
            return Ok(SnapshotPrelude::Refused);
        }

        // C3 defense: envelope must match deserialized state
        if new_chain_state.best_hash != snapshot.block_hash
            || new_chain_state.best_height != snapshot.block_height
        {
            error!("[SNAP_SYNC] Envelope/state mismatch — rejecting",);
            return Ok(SnapshotPrelude::Refused);
        }

        new_chain_state.genesis_hash = self.chain_state.read().await.genesis_hash;
        new_chain_state.mark_snap_synced(snapshot.block_height);

        // The decoded set is MOVED into the batch, so it is gone before the
        // post-install root derivation reads the backend.
        if let Err(e) = self.state_db.atomic_replace(
            &new_chain_state,
            &new_producer_set,
            new_utxo_set.into_pairs(),
        ) {
            // Nothing was installed. Publishing the snapshot into the in-memory
            // 3-state here would leave memory on the new chain and disk on the old
            // one — the split INC-I-156 paid for.
            error!(
                "[SNAP_SYNC] StateDb atomic_replace failed: {} — nothing installed",
                e
            );
            return Ok(SnapshotPrelude::Refused);
        }

        // INC-I-156 / AUDIT-P2-101: disarm the rebuild halt. Ok arm ONLY — the
        // durable set has just been replaced wholesale by a root-verified
        // snapshot, so this REPAIRS a truncation rather than laundering one.
        // `atomic_replace` deliberately excludes CF_META from deletable_cfs, so
        // without this the marker outlives the operation that repaired the
        // ledger. A failed disarm is logged, never propagated: staying halted is
        // the fail-safe direction.
        if let Err(e) = self.state_db.clear_rebuild_in_progress() {
            warn!(
                "[SNAP_SYNC] Installed a verified snapshot but failed to clear the \
                 rebuild-in-progress marker: {} — the node stays halted until this \
                 is resolved",
                e
            );
        }

        Ok(SnapshotPrelude::Installed(Box::new(InstalledSnapshot {
            chain_state: new_chain_state,
            producer_set: new_producer_set,
        })))
    }

    /// Promote the staged rows into the live UTXO column family.
    ///
    /// No pre-install root check: the session client already matched the stream
    /// against the manifest digest, and F-10 demands the root be re-derived from
    /// the INSTALLED backend, which only exists after the promotion.
    ///
    /// `Err` means the promotion window may have replaced the live set and could
    /// not be shown to have finished — the marker stays ARMED and the node halts.
    async fn install_staged_utxos(
        &self,
        snapshot: &network::VerifiedSnapshot,
        marker: &network::StagedUtxoMarker,
    ) -> Result<SnapshotPrelude> {
        let mut chain_state: ChainState = match bincode::deserialize(&snapshot.chain_state) {
            Ok(cs) => cs,
            Err(e) => {
                error!(
                    "[SNAP_SYNC] Staged snapshot ChainState decode failed at height={}: {} — rejecting",
                    snapshot.block_height, e
                );
                return Ok(SnapshotPrelude::Refused);
            }
        };
        let producer_set = match ProducerSet::from_snapshot_bytes(&snapshot.producer_set) {
            Ok(ps) => ps,
            Err(e) => {
                error!(
                    "[SNAP_SYNC] Staged snapshot ProducerSet decode failed at height={}: {} — rejecting",
                    snapshot.block_height, e
                );
                return Ok(SnapshotPrelude::Refused);
            }
        };

        // C3 defense: envelope must match the decoded state.
        if chain_state.best_hash != snapshot.block_hash
            || chain_state.best_height != snapshot.block_height
        {
            error!("[SNAP_SYNC] Staged snapshot envelope/state mismatch — rejecting");
            return Ok(SnapshotPrelude::Refused);
        }

        // A staging family holding a different number of rows than the manifest
        // announced belongs to some other transfer; promoting it installs a set
        // nothing downstream rejects (`BlockHeader` carries no state root).
        let staged_rows = self.state_db.staged_utxo_len() as u64;
        if staged_rows != marker.utxo_count {
            error!(
                "[SNAP_SYNC] Staging holds {} rows, manifest announced {} — rejecting",
                staged_rows, marker.utxo_count
            );
            return Ok(SnapshotPrelude::Refused);
        }

        chain_state.genesis_hash = self.chain_state.read().await.genesis_hash;
        chain_state.mark_snap_synced(snapshot.block_height);

        // F-11: arm BEFORE the live family is touched. A crash from here to the
        // post-install root check leaves the marker as the only evidence that
        // the live set may be half-replaced.
        if let Err(e) = self.state_db.set_rebuild_in_progress(snapshot.block_height) {
            error!(
                "[SNAP_SYNC] Could not arm the rebuild marker before promoting: {} — nothing \
                 installed",
                e
            );
            return Ok(SnapshotPrelude::Refused);
        }

        if let Err(e) = self
            .state_db
            .promote_staged_utxos(&chain_state, &producer_set)
        {
            error!(
                "[SNAP_SYNC] Promotion of {} staged rows failed: {} — the node stays halted",
                staged_rows, e
            );
            anyhow::bail!(
                "[SNAP_SYNC] staged UTXO promotion failed at height {}: {}",
                snapshot.block_height,
                e
            );
        }

        info!(
            "[SNAP_SYNC] Promoted {} staged UTXO rows at height {}",
            staged_rows, snapshot.block_height
        );
        Ok(SnapshotPrelude::Installed(Box::new(InstalledSnapshot {
            chain_state,
            producer_set,
        })))
    }

    /// Startup reconciliation of `cf_utxo_staging`. Returns the rows cleared.
    ///
    /// Decision 136 — NO-RESUME. Staging is cleared on BOTH arms: no path can
    /// ever complete these rows, and anything left is prepended to the next
    /// session's first body. The rebuild marker is untouched, so a crash inside
    /// a promotion window keeps the node out of service until a fresh snap-sync.
    pub fn reconcile_staged_utxos_on_startup(&self) -> Result<u64> {
        let halted = self.state_db.get_rebuild_in_progress().is_some();
        let rows = self.state_db.staged_utxo_len() as u64;
        self.state_db.clear_staged_utxos()?;

        if halted {
            warn!(
                "[SNAP_SYNC] Cleared {} staged UTXO rows inside a promotion window — the rebuild \
                 marker stays armed and recovery is a fresh snap-sync",
                rows
            );
        } else if rows > 0 {
            info!(
                "[SNAP_SYNC] Cleared {} staged UTXO rows left by an abandoned transfer",
                rows
            );
        }
        Ok(rows)
    }
}
