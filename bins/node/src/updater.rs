//! Auto-update integration for doli-node
//!
//! This module integrates the doli-updater crate with the node,
//! handling the update check loop and veto voting process.

// Re-export items from the updater crate
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};
pub use updater::{
    apply_update, calculate_veto_result, current_version, fetch_latest_release, is_newer_version,
    restart_node, verify_release_signatures, veto_deadline, veto_period_ended, Release,
    UpdateConfig, UpdateError, Vote, VoteMessage, VoteTracker, VETO_PERIOD, VETO_THRESHOLD_PERCENT,
};

/// State of a pending update
#[derive(Debug)]
pub struct PendingUpdate {
    pub release: Release,
    pub vote_tracker: VoteTracker,
}

/// The update service that runs in the background
pub struct UpdateService {
    config: UpdateConfig,
    pending: Arc<RwLock<Option<PendingUpdate>>>,
    vote_tx: mpsc::Sender<VoteMessage>,
    vote_rx: mpsc::Receiver<VoteMessage>,
}

impl UpdateService {
    /// Create a new update service
    pub fn new(config: UpdateConfig) -> Self {
        let (vote_tx, vote_rx) = mpsc::channel(100);
        Self {
            config,
            pending: Arc::new(RwLock::new(None)),
            vote_tx,
            vote_rx,
        }
    }

    /// Get a sender for vote messages (to be connected to network gossip)
    pub fn vote_sender(&self) -> mpsc::Sender<VoteMessage> {
        self.vote_tx.clone()
    }

    /// Get the pending update if any
    pub async fn pending_update(&self) -> Option<Release> {
        self.pending
            .read()
            .await
            .as_ref()
            .map(|p| p.release.clone())
    }

    /// Run the update service
    pub async fn run(
        mut self,
        producer_count_fn: impl Fn() -> usize + Send + Sync + 'static,
        is_producer_fn: impl Fn(&str) -> bool + Send + Sync + 'static,
    ) {
        if !self.config.enabled {
            info!("Auto-updates disabled");
            return;
        }

        info!(
            "Update service started (check interval: {}h)",
            self.config.check_interval_secs / 3600
        );

        let check_interval = Duration::from_secs(self.config.check_interval_secs);
        let mut check_ticker = tokio::time::interval(check_interval);

        // Also check every minute for vote processing
        let mut vote_ticker = tokio::time::interval(Duration::from_secs(60));

        loop {
            tokio::select! {
                // Periodic update check
                _ = check_ticker.tick() => {
                    self.check_for_updates().await;
                }

                // Process incoming votes
                Some(vote_msg) = self.vote_rx.recv() => {
                    self.handle_vote(vote_msg, &is_producer_fn).await;
                }

                // Check veto period status
                _ = vote_ticker.tick() => {
                    self.check_veto_status(&producer_count_fn).await;
                }
            }
        }
    }

    /// Check for available updates
    async fn check_for_updates(&mut self) {
        debug!("Checking for updates...");

        let release = match fetch_latest_release(self.config.custom_url.as_deref()).await {
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

        // Verify signatures
        if let Err(e) = verify_release_signatures(&release) {
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

        let mut pending = self.pending.write().await;
        *pending = Some(PendingUpdate {
            release,
            vote_tracker,
        });

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
            if p.release.version == vote_msg.version {
                if p.vote_tracker
                    .record_vote(vote_msg.producer_id.clone(), vote_msg.vote)
                {
                    debug!(
                        "Recorded {} vote from {}",
                        if vote_msg.vote == Vote::Veto {
                            "VETO"
                        } else {
                            "APPROVE"
                        },
                        &vote_msg.producer_id[..16]
                    );
                }
            }
        }
    }

    /// Check if veto period has ended and apply or reject update
    async fn check_veto_status(&mut self, producer_count_fn: &impl Fn() -> usize) {
        let should_apply = {
            let pending = self.pending.read().await;
            if let Some(ref p) = *pending {
                if veto_period_ended(&p.release) {
                    let total = producer_count_fn();
                    let result = calculate_veto_result(p.vote_tracker.veto_count(), total);

                    if result.approved {
                        info!(
                            "Update {} APPROVED ({}% veto, threshold {}%)",
                            p.release.version, result.veto_percent, VETO_THRESHOLD_PERCENT
                        );
                        Some(true)
                    } else {
                        warn!(
                            "Update {} REJECTED ({}% veto >= {}% threshold)",
                            p.release.version, result.veto_percent, VETO_THRESHOLD_PERCENT
                        );
                        Some(false)
                    }
                } else {
                    // Still in veto period - log status
                    let total = producer_count_fn();
                    let veto_pct = p.vote_tracker.veto_percent(total);
                    let deadline = veto_deadline(&p.release);
                    let remaining = deadline.saturating_sub(updater::current_timestamp());
                    let hours = remaining / 3600;

                    debug!(
                        "Update {} pending: {}% veto, {}h remaining",
                        p.release.version, veto_pct, hours
                    );
                    None
                }
            } else {
                None
            }
        };

        match should_apply {
            Some(true) => {
                // Apply the update
                let release = {
                    let mut pending = self.pending.write().await;
                    pending.take().map(|p| p.release)
                };

                if let Some(release) = release {
                    if self.config.notify_only {
                        info!("Notify-only mode: skipping update application");
                        return;
                    }

                    match apply_update(&release).await {
                        Ok(()) => {
                            info!("Update applied successfully, restarting...");
                            restart_node();
                        }
                        Err(e) => {
                            error!("Failed to apply update: {}", e);
                            // Keep pending so we can retry
                        }
                    }
                }
            }
            Some(false) => {
                // Clear the rejected update
                let mut pending = self.pending.write().await;
                *pending = None;
            }
            None => {
                // Still in veto period, nothing to do
            }
        }
    }
}

/// Start the update service in a background task
pub fn spawn_update_service(
    config: UpdateConfig,
    producer_count_fn: impl Fn() -> usize + Send + Sync + 'static,
    is_producer_fn: impl Fn(&str) -> bool + Send + Sync + 'static,
) -> mpsc::Sender<VoteMessage> {
    let service = UpdateService::new(config);
    let vote_tx = service.vote_sender();

    tokio::spawn(async move {
        service.run(producer_count_fn, is_producer_fn).await;
    });

    vote_tx
}

/// CLI for update management
pub mod cli {
    use super::*;

    /// Show update status
    pub fn show_status(pending: Option<&PendingUpdate>, total_producers: usize) {
        match pending {
            Some(p) => {
                let veto_pct = p.vote_tracker.veto_percent(total_producers);
                let deadline = veto_deadline(&p.release);
                let now = updater::current_timestamp();
                let remaining = deadline.saturating_sub(now);
                let days = remaining / 86400;
                let hours = (remaining % 86400) / 3600;

                println!("Pending Update: v{}", p.release.version);
                println!("Changelog: {}", p.release.changelog);
                println!();
                println!("Veto Status:");
                println!("  Vetos: {} ({}%)", p.vote_tracker.veto_count(), veto_pct);
                println!("  Threshold: {}%", VETO_THRESHOLD_PERCENT);
                println!("  Time remaining: {}d {}h", days, hours);
                println!();

                if veto_pct as u8 >= VETO_THRESHOLD_PERCENT {
                    println!("Status: WILL BE REJECTED (threshold reached)");
                } else {
                    println!("Status: ON TRACK TO PASS");
                }
            }
            None => {
                println!("No pending updates");
                println!("Current version: {}", current_version());
            }
        }
    }
}
