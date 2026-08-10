//! Background update service — state transitions and the install gate.
//!
//! The update-poll and vote-intake methods live in the child module `checks`
//! (`service_checks.rs`) to keep this file inside the 500-line module budget.
//! `check_veto_status` and `auto_apply` stay HERE: they are the install-gating
//! decisions INC-I-172 M1 changed, and the regression tests anchor on this file.

#[path = "service_checks.rs"]
mod checks;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, warn};
use updater::{
    auto_apply_from_github, calculate_veto_result, current_version, is_newer_version, restart_node,
    verify_release_with_trust_root, TrustRoot, UpdateConfig, VersionEnforcement, VoteMessage,
    VETO_THRESHOLD_PERCENT,
};

use super::notifications::{
    display_enforcement_notification, display_grace_period_notification,
    display_update_notification,
};
use super::PendingUpdate;

/// How often to show the notification reminder (every 6 hours)
const NOTIFICATION_INTERVAL_SECS: u64 = 6 * 3600;

/// State transitions for updates
enum UpdateTransition {
    /// Veto period ended, update approved
    Approved(updater::VoteResult),
    /// Veto period ended, update rejected
    Rejected(updater::VoteResult),
    /// Grace period ended, activate enforcement
    ActivateEnforcement,
}

/// The update service that runs in the background
pub struct UpdateService {
    config: UpdateConfig,
    network: doli_core::network::Network,
    pending: Arc<RwLock<Option<PendingUpdate>>>,
    vote_tx: mpsc::Sender<VoteMessage>,
    vote_rx: mpsc::Receiver<VoteMessage>,
    data_dir: PathBuf,
    /// Track when we last showed the notification (to avoid spam)
    last_notification: Arc<RwLock<u64>>,
}

impl UpdateService {
    /// Create a new update service
    pub fn new(
        config: UpdateConfig,
        data_dir: PathBuf,
        network: doli_core::network::Network,
    ) -> Self {
        let (vote_tx, vote_rx) = mpsc::channel(100);

        // Try to load existing pending update from disk
        let pending = PendingUpdate::load(&data_dir);
        let pending = if let Some(p) = pending {
            // If we're already running the required version, clean up the stale
            // pending_update.json. Without this, every restart re-displays
            // "PRODUCTION PAUSED" and attempts a failed auto-apply download.
            if let Some(ref enforcement) = p.enforcement {
                if enforcement.version_meets_requirement(current_version()) {
                    info!(
                        "Pending update v{} already satisfied (running v{}) — cleaning up",
                        p.release.version,
                        current_version()
                    );
                    if let Err(e) = PendingUpdate::remove(&data_dir) {
                        warn!("Failed to remove stale pending_update.json: {}", e);
                    }
                    None
                } else {
                    info!(
                        "Loaded pending update v{} from disk ({} days remaining)",
                        p.release.version,
                        p.days_remaining()
                    );
                    Some(p)
                }
            } else {
                info!(
                    "Loaded pending update v{} from disk ({} days remaining)",
                    p.release.version,
                    p.days_remaining()
                );
                Some(p)
            }
        } else {
            None
        };

        Self {
            config,
            network,
            pending: Arc::new(RwLock::new(pending)),
            vote_tx,
            vote_rx,
            data_dir,
            last_notification: Arc::new(RwLock::new(0)),
        }
    }

    /// Get a sender for vote messages (to be connected to network gossip)
    pub fn vote_sender(&self) -> mpsc::Sender<VoteMessage> {
        self.vote_tx.clone()
    }

    /// Get the shared pending update state (for RPC to read live state)
    pub fn pending_state(&self) -> Arc<RwLock<Option<PendingUpdate>>> {
        self.pending.clone()
    }

    /// Run the update service
    pub async fn run(
        mut self,
        producer_count_fn: impl Fn() -> Option<usize> + Send + Sync + 'static,
        is_producer_fn: impl Fn(&str) -> Option<bool> + Send + Sync + 'static,
        maintainer_keys_fn: impl Fn() -> TrustRoot + Send + Sync + 'static,
    ) {
        if !self.config.enabled {
            info!("Auto-updates disabled");
            return;
        }

        info!(
            "Update service started (check interval: {}h)",
            self.config.check_interval_secs / 3600
        );

        // Show notification on startup if there's a pending update
        {
            let pending = self.pending.read().await;
            if let Some(ref p) = *pending {
                let total = producer_count_fn();
                display_update_notification(p, total);
                *self.last_notification.write().await = updater::current_timestamp();
            }
        }

        let check_interval = Duration::from_secs(self.config.check_interval_secs);
        let mut check_ticker = tokio::time::interval(check_interval);

        // Check every minute for vote processing and periodic notifications
        let mut vote_ticker = tokio::time::interval(Duration::from_secs(60));

        loop {
            tokio::select! {
                // Periodic update check
                _ = check_ticker.tick() => {
                    self.check_for_updates(&producer_count_fn, &maintainer_keys_fn).await;
                }

                // Process incoming votes
                Some(vote_msg) = self.vote_rx.recv() => {
                    self.handle_vote(vote_msg, &is_producer_fn).await;
                }

                // Check veto period status and show periodic reminders
                _ = vote_ticker.tick() => {
                    self.check_veto_status(&producer_count_fn, &maintainer_keys_fn).await;
                    self.maybe_show_reminder(&producer_count_fn).await;
                }
            }
        }
    }

    /// Check if veto period has ended and transition to grace period or reject
    ///
    /// Every deadline here is measured from `PendingUpdate::first_notified_at`, the
    /// node-local moment this node first saw the release (INC-I-172 F7(b)).
    async fn check_veto_status(
        &mut self,
        producer_count_fn: &impl Fn() -> Option<usize>,
        maintainer_keys_fn: &impl Fn() -> TrustRoot,
    ) {
        // First, check for state transitions
        let transition = {
            let pending = self.pending.read().await;
            if let Some(ref p) = *pending {
                // Already approved - check for enforcement transition
                if p.approved {
                    if let Some(ref enforcement) = p.enforcement {
                        if enforcement.should_enforce() && !enforcement.active {
                            Some(UpdateTransition::ActivateEnforcement)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    // Check veto period using config-aware timing, measured from the
                    // node-local first_notified_at.
                    // saturating_add (AUDIT-P2-013): a wrapping add on a value read from
                    // unauthenticated node-local JSON would put the deadline in the past.
                    let veto_deadline = p
                        .first_notified_at
                        .saturating_add(self.config.veto_period_secs);
                    let veto_ended = updater::current_timestamp() >= veto_deadline;

                    if veto_ended {
                        // Veto period ended - check result.
                        //
                        // AUDIT-P1-015: an UNKNOWN producer count takes NO transition at
                        // all. `producer_count_fn` is a non-blocking `try_read` on a lock
                        // the block-application path writes constantly, so `None` is
                        // ordinary contention — and the electorate size is the
                        // denominator of the veto percentage. Substituting 0 for it made
                        // `0 < 40` APPROVE, converting any number of veto votes into
                        // "approved" with no attacker action. Neither approving NOR
                        // rejecting is correct on an unknown count: this tick is skipped
                        // and the decision is retaken on the next one, 60 s later.
                        match producer_count_fn() {
                            Some(total) => {
                                let result =
                                    calculate_veto_result(p.vote_tracker.veto_count(), total);
                                if result.approved {
                                    Some(UpdateTransition::Approved(result))
                                } else {
                                    Some(UpdateTransition::Rejected(result))
                                }
                            }
                            None => {
                                warn!(
                                    "Veto period for {} has ended but the active producer count \
                                     could not be read (lock held). Taking NO decision this tick \
                                     — an unknown electorate size is not a 0% veto. Retrying in \
                                     60s.",
                                    p.release.version
                                );
                                None
                            }
                        }
                    } else {
                        // Still in veto period
                        None
                    }
                }
            } else {
                None
            }
        };

        // Handle transitions
        match transition {
            Some(UpdateTransition::Approved(result)) => {
                // Transition to grace period
                let version_to_apply;
                {
                    let mut pending = self.pending.write().await;
                    if let Some(ref mut p) = *pending {
                        info!(
                            "Update {} APPROVED ({}% veto, threshold {}%) - Grace period: {}s",
                            p.release.version,
                            result.veto_percent,
                            VETO_THRESHOLD_PERCENT,
                            self.config.grace_period_secs
                        );

                        // Mark as approved and set enforcement (config-aware timing,
                        // measured from the node-local first_notified_at).
                        p.approved = true;
                        let enforcement_time = p
                            .first_notified_at
                            .saturating_add(self.config.veto_period_secs)
                            .saturating_add(self.config.grace_period_secs);
                        p.enforcement = Some(VersionEnforcement {
                            min_version: p.release.version.clone(),
                            enforcement_time,
                            active: false,
                            binary_ready: false,
                        });

                        // Save updated state
                        if let Err(e) = p.save(&self.data_dir) {
                            error!("Failed to save approved update state: {}", e);
                        }

                        version_to_apply = Some(p.release.version.clone());

                        // Show grace period notification
                        display_grace_period_notification(p);
                        *self.last_notification.write().await = updater::current_timestamp();
                    } else {
                        version_to_apply = None;
                    }
                }

                // Auto-apply if enabled (not notify_only)
                if !self.config.notify_only {
                    if let Some(version) = version_to_apply {
                        self.auto_apply(&version, maintainer_keys_fn).await;
                    }
                }
            }
            Some(UpdateTransition::Rejected(result)) => {
                warn!(
                    "Update REJECTED ({}% veto >= {}% threshold)",
                    result.veto_percent, VETO_THRESHOLD_PERCENT
                );

                // Clear the rejected update
                let mut pending = self.pending.write().await;
                *pending = None;

                // Remove from disk
                if let Err(e) = PendingUpdate::remove(&self.data_dir) {
                    warn!("Failed to remove pending_update.json: {}", e);
                }
            }
            Some(UpdateTransition::ActivateEnforcement) => {
                // Activate enforcement — also try auto-apply as last resort
                let version_to_apply;
                {
                    let mut pending = self.pending.write().await;
                    if let Some(ref mut p) = *pending {
                        if let Some(ref mut enforcement) = p.enforcement {
                            enforcement.active = true;
                            info!(
                                "Version enforcement ACTIVE - nodes running < {} cannot produce",
                                enforcement.min_version
                            );

                            // Save updated state
                            if let Err(e) = p.save(&self.data_dir) {
                                error!("Failed to save enforcement state: {}", e);
                            }

                            // Show enforcement notification
                            display_enforcement_notification(p);
                            *self.last_notification.write().await = updater::current_timestamp();
                        }

                        // If we still haven't updated (auto-apply failed earlier?), try again
                        if is_newer_version(&p.release.version, current_version()) {
                            version_to_apply = Some(p.release.version.clone());
                        } else {
                            version_to_apply = None;
                        }
                    } else {
                        version_to_apply = None;
                    }
                }

                if !self.config.notify_only {
                    if let Some(version) = version_to_apply {
                        self.auto_apply(&version, maintainer_keys_fn).await;
                    }
                }
            }
            None => {
                // No transition needed
            }
        }
    }

    /// Download, install, and restart with the approved version
    ///
    /// On success, this function does NOT return — `restart_node()` calls `exec()`
    /// to replace the current process with the new binary.
    ///
    /// On failure, logs the error and continues running the old version.
    /// The enforcement mechanism will block production until the operator
    /// manually applies the update.
    async fn auto_apply(&self, version: &str, maintainer_keys_fn: &impl Fn() -> TrustRoot) {
        info!(
            "Auto-applying approved update v{} (current: v{})...",
            version,
            current_version()
        );

        // Extract the signed checksums hash from the pending release.
        // This is SHA256(CHECKSUMS.txt) that was verified against maintainer signatures.
        // Passing it to auto_apply_from_github closes the TOCTOU window (AUDIT-UPDATE-002).
        let staged_release = {
            let pending = self.pending.read().await;
            pending.as_ref().map(|p| p.release.clone())
        };
        let Some(staged_release) = staged_release else {
            warn!("Auto-apply for v{version} aborted: no pending release is staged");
            return;
        };

        // INC-I-172 F7(a): RE-VERIFY against the CURRENT trust root, immediately
        // before install. The pending update may have been staged before a veto
        // period and restored from disk across any number of restarts (see
        // `UpdateService::new`), so the root that authorised it may since have been
        // rotated. Revocation that cannot reach an in-flight update is not revocation.
        let trust_root = maintainer_keys_fn();
        if let Err(e) = verify_release_with_trust_root(&staged_release, &trust_root) {
            error!(
                "Auto-apply for v{} ABORTED: the staged release no longer verifies against the \
                 current {} trust root ({} keys, threshold {}): {}. Dropping the pending update.",
                version,
                trust_root.provenance(),
                trust_root.keys().len(),
                trust_root.threshold(),
                e
            );
            let mut pending = self.pending.write().await;
            *pending = None;
            if let Err(e) = PendingUpdate::remove(&self.data_dir) {
                warn!("Failed to remove pending_update.json: {}", e);
            }
            return;
        }
        let signed_checksums_sha256 = staged_release.binary_sha256.clone();

        match auto_apply_from_github(version, &signed_checksums_sha256).await {
            Ok(()) => {
                info!("Update v{} installed successfully, restarting...", version);

                // Clean up pending update before exec (won't return)
                if let Err(e) = PendingUpdate::remove(&self.data_dir) {
                    warn!("Failed to clean up pending_update.json: {}", e);
                }

                // exec() replaces the process — does NOT return
                // systemd/launchd see the same PID, no restart triggered
                restart_node();
            }
            Err(e) => {
                error!(
                    "Auto-apply failed for v{}: {}. Node continues with v{}. \
                     Will retry on next check cycle.",
                    version,
                    e,
                    current_version()
                );
                // Clear the pending update so the next check cycle re-detects the
                // release and retries from scratch. Without this, the node stays in
                // a "binary not downloaded" loop forever because binary_ready=false
                // and the pending state prevents re-processing the same version.
                let mut pending = self.pending.write().await;
                *pending = None;
                if let Err(e) = PendingUpdate::remove(&self.data_dir) {
                    warn!("Failed to remove pending_update.json: {}", e);
                }
            }
        }
    }
}

/// Start the update service in a background task
///
/// Returns `(vote_tx, pending)`:
/// - `vote_tx`: channel to forward gossip votes into the UpdateService
/// - `pending`: shared state for RPC to read live update status
pub fn spawn_update_service(
    config: UpdateConfig,
    data_dir: PathBuf,
    network: doli_core::network::Network,
    producer_count_fn: impl Fn() -> Option<usize> + Send + Sync + 'static,
    is_producer_fn: impl Fn(&str) -> Option<bool> + Send + Sync + 'static,
    maintainer_keys_fn: impl Fn() -> TrustRoot + Send + Sync + 'static,
) -> (
    mpsc::Sender<VoteMessage>,
    Arc<RwLock<Option<PendingUpdate>>>,
) {
    let service = UpdateService::new(config, data_dir, network);
    let vote_tx = service.vote_sender();
    let pending = service.pending_state();

    tokio::spawn(async move {
        service
            .run(producer_count_fn, is_producer_fn, maintainer_keys_fn)
            .await;
    });

    (vote_tx, pending)
}
