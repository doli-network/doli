//! Background update service — check loop, vote processing, state transitions.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};
use updater::{
    auto_apply_from_github, calculate_veto_result, current_version, fetch_latest_release,
    is_newer_version, restart_node, verify_release_signatures_with_keys, UpdateConfig,
    VersionEnforcement, Vote, VoteMessage, VoteTracker, VETO_THRESHOLD_PERCENT,
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
        producer_count_fn: impl Fn() -> usize + Send + Sync + 'static,
        is_producer_fn: impl Fn(&str) -> bool + Send + Sync + 'static,
        maintainer_keys_fn: impl Fn() -> Vec<String> + Send + Sync + 'static,
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
                    self.check_veto_status(&producer_count_fn).await;
                    self.maybe_show_reminder(&producer_count_fn).await;
                }
            }
        }
    }

    /// Show periodic reminder notification if enough time has passed
    async fn maybe_show_reminder(&self, producer_count_fn: &impl Fn() -> usize) {
        let pending = self.pending.read().await;
        if let Some(ref p) = *pending {
            let now = updater::current_timestamp();
            let last = *self.last_notification.read().await;

            if now - last >= NOTIFICATION_INTERVAL_SECS {
                let total = producer_count_fn();
                display_update_notification(p, total);
                *self.last_notification.write().await = now;
            }
        }
    }

    /// Check for available updates
    async fn check_for_updates(
        &mut self,
        producer_count_fn: &impl Fn() -> usize,
        maintainer_keys_fn: &impl Fn() -> Vec<String>,
    ) {
        debug!("Checking for updates...");

        let release =
            match fetch_latest_release(self.config.custom_url.as_deref(), Some(self.network)).await
            {
                Ok(Some(r)) => r,
                Ok(None) => {
                    debug!("No release info available");
                    return;
                }
                Err(e) => {
                    warn!("Failed to check for updates: {}", e);
                    return;
                }
            };

        // Check if newer than current
        if !is_newer_version(&release.version, current_version()) {
            debug!("Already on latest version ({})", current_version());
            return;
        }

        // Check if we already have this pending
        {
            let pending = self.pending.read().await;
            if let Some(ref p) = *pending {
                if p.release.version == release.version {
                    debug!("Update {} already pending", release.version);
                    return;
                }
            }
        }

        // Verify signatures using on-chain maintainer keys (falls back to bootstrap keys)
        let on_chain_keys = maintainer_keys_fn();
        if let Err(e) = verify_release_signatures_with_keys(&release, &on_chain_keys, self.network)
        {
            error!("Release {} has invalid signatures: {}", release.version, e);
            return;
        }

        info!(
            "New update available: {} -> {} (veto period: 7 days)",
            current_version(),
            release.version
        );
        info!("Changelog: {}", release.changelog);

        // Start veto period
        let vote_tracker = VoteTracker::new(release.version.clone());
        let now = updater::current_timestamp();

        let new_pending = PendingUpdate {
            release,
            vote_tracker,
            first_notified_at: now,
            approved: false,
            enforcement: None,
        };

        // Save to disk for persistence across restarts
        if let Err(e) = new_pending.save(&self.data_dir) {
            error!("Failed to save pending update to disk: {}", e);
        }

        // Show mandatory notification
        let total = producer_count_fn();
        display_update_notification(&new_pending, total);
        *self.last_notification.write().await = now;

        let mut pending = self.pending.write().await;
        *pending = Some(new_pending);

        if self.config.notify_only {
            info!("Notify-only mode: update will not be applied automatically");
        }
    }

    /// Handle an incoming vote
    async fn handle_vote(&self, vote_msg: VoteMessage, is_producer_fn: &impl Fn(&str) -> bool) {
        // Verify this is from an active producer
        if !is_producer_fn(&vote_msg.producer_id) {
            debug!("Ignoring vote from non-producer: {}", vote_msg.producer_id);
            return;
        }

        let mut pending = self.pending.write().await;
        if let Some(ref mut p) = *pending {
            if p.release.version == vote_msg.version
                && p.vote_tracker
                    .record_vote(vote_msg.producer_id.clone(), vote_msg.vote)
            {
                info!(
                    "Recorded {} vote from {}",
                    if vote_msg.vote == Vote::Veto {
                        "VETO"
                    } else {
                        "APPROVE"
                    },
                    &vote_msg.producer_id[..16]
                );
                // Persist vote tracker to disk
                if let Err(e) = p.save(&self.data_dir) {
                    warn!("Failed to save vote state: {}", e);
                }
            }
        }
    }

    /// Check if veto period has ended and transition to grace period or reject
    async fn check_veto_status(&mut self, producer_count_fn: &impl Fn() -> usize) {
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
                    // Check veto period using config-aware timing
                    let veto_deadline = p.release.published_at + self.config.veto_period_secs;
                    let veto_ended = updater::current_timestamp() >= veto_deadline;

                    if veto_ended {
                        // Veto period ended - check result
                        let total = producer_count_fn();
                        let result = calculate_veto_result(p.vote_tracker.veto_count(), total);

                        if result.approved {
                            Some(UpdateTransition::Approved(result))
                        } else {
                            Some(UpdateTransition::Rejected(result))
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

                        // Mark as approved and set enforcement (config-aware timing)
                        p.approved = true;
                        let enforcement_time = p.release.published_at
                            + self.config.veto_period_secs
                            + self.config.grace_period_secs;
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
                        self.auto_apply(&version).await;
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
                        self.auto_apply(&version).await;
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
    async fn auto_apply(&self, version: &str) {
        info!(
            "Auto-applying approved update v{} (current: v{})...",
            version,
            current_version()
        );

        match auto_apply_from_github(version).await {
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
    producer_count_fn: impl Fn() -> usize + Send + Sync + 'static,
    is_producer_fn: impl Fn(&str) -> bool + Send + Sync + 'static,
    maintainer_keys_fn: impl Fn() -> Vec<String> + Send + Sync + 'static,
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
