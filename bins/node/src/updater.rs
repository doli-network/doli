//! Auto-update integration for doli-node
//!
//! This module integrates the doli-updater crate with the node,
//! handling the update check loop and veto voting process.
//!
//! # Mandatory Notification System
//!
//! When an update is detected:
//! - ALL nodes display a prominent alert
//! - The alert shows version, changelog summary, and veto period
//! - Producers can vote to veto if they have objections
//! - If no veto threshold reached after 7 days, update auto-applies

// Re-export items from the updater crate
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};
pub use updater::{
    apply_update, calculate_veto_result, check_production_allowed, current_version,
    fetch_latest_release, is_newer_version, restart_node, rollback, verify_release_signatures,
    veto_deadline, veto_period_ended, ProductionBlocked, Release, UpdateConfig, VersionEnforcement,
    Vote, VoteMessage, VoteTracker, VETO_THRESHOLD_PERCENT,
};

/// ANSI color codes for terminal output
mod colors {
    pub const YELLOW: &str = "\x1b[1;33m";
    pub const CYAN: &str = "\x1b[0;36m";
    pub const GREEN: &str = "\x1b[0;32m";
    pub const RED: &str = "\x1b[0;31m";
    pub const BOLD: &str = "\x1b[1m";
    pub const RESET: &str = "\x1b[0m";
}

/// State of a pending update (persisted to disk)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PendingUpdate {
    pub release: Release,
    #[serde(default)]
    pub vote_tracker: VoteTracker,
    /// When the notification was first shown
    #[serde(default)]
    pub first_notified_at: u64,
    /// Whether the update was approved (veto period passed)
    #[serde(default)]
    pub approved: bool,
    /// Version enforcement state (active after grace period)
    #[serde(default)]
    pub enforcement: Option<VersionEnforcement>,
}

impl PendingUpdate {
    /// Load pending update from disk
    pub fn load(data_dir: &Path) -> Option<Self> {
        let path = data_dir.join("pending_update.json");
        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(content) => match serde_json::from_str(&content) {
                    Ok(pending) => Some(pending),
                    Err(e) => {
                        warn!("Failed to parse pending_update.json: {}", e);
                        None
                    }
                },
                Err(e) => {
                    warn!("Failed to read pending_update.json: {}", e);
                    None
                }
            }
        } else {
            None
        }
    }

    /// Save pending update to disk
    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        let path = data_dir.join("pending_update.json");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)
    }

    /// Remove pending update from disk
    pub fn remove(data_dir: &Path) -> std::io::Result<()> {
        let path = data_dir.join("pending_update.json");
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    /// Calculate days remaining in veto period
    pub fn days_remaining(&self) -> u64 {
        let deadline = veto_deadline(&self.release);
        let now = updater::current_timestamp();
        let remaining_secs = deadline.saturating_sub(now);
        remaining_secs / 86400
    }

    /// Calculate hours remaining (after days)
    pub fn hours_remaining(&self) -> u64 {
        let deadline = veto_deadline(&self.release);
        let now = updater::current_timestamp();
        let remaining_secs = deadline.saturating_sub(now);
        (remaining_secs % 86400) / 3600
    }
}

/// Display mandatory update notification to console
///
/// Shows different banners based on update state:
/// - Veto period: Shows countdown and how to veto
/// - Grace period: Shows update approved, countdown to enforcement
/// - Enforcement: Shows production paused, how to update
pub fn display_update_notification(pending: &PendingUpdate, total_producers: usize) {
    use colors::*;

    // Check which phase we're in
    if let Some(ref enforcement) = pending.enforcement {
        if enforcement.should_enforce() {
            // ENFORCEMENT ACTIVE - production blocked
            display_enforcement_notification(pending);
            return;
        }
    }

    if pending.approved {
        // GRACE PERIOD - update approved, countdown to enforcement
        display_grace_period_notification(pending);
        return;
    }

    // VETO PERIOD - voting still open
    let days = pending.days_remaining();
    let hours = pending.hours_remaining();
    let veto_count = pending.vote_tracker.veto_count();
    let veto_pct = pending.vote_tracker.veto_percent(total_producers);
    let veto_color = if veto_pct >= VETO_THRESHOLD_PERCENT {
        RED
    } else {
        GREEN
    };

    // Prominent banner
    eprintln!();
    eprintln!(
        "{}╔══════════════════════════════════════════════════════════════════╗{}",
        YELLOW, RESET
    );
    eprintln!(
        "{}║                    ⚠️  UPDATE PENDING                             ║{}",
        YELLOW, RESET
    );
    eprintln!(
        "{}╠══════════════════════════════════════════════════════════════════╣{}",
        YELLOW, RESET
    );
    eprintln!(
        "{}║{}  Version: {}{} -> {}{}                                         {}║{}",
        YELLOW,
        RESET,
        CYAN,
        current_version(),
        pending.release.version,
        RESET,
        YELLOW,
        RESET
    );
    eprintln!(
        "{}║{}  Veto period: {}{} days, {} hours remaining{}                  {}║{}",
        YELLOW, RESET, BOLD, days, hours, RESET, YELLOW, RESET
    );
    eprintln!(
        "{}║{}  Current vetos: {}{}/{} ({}%){}                                {}║{}",
        YELLOW, RESET, veto_color, veto_count, total_producers, veto_pct, RESET, YELLOW, RESET
    );
    eprintln!(
        "{}║{}  Threshold to reject: {}%                                    {}║{}",
        YELLOW, RESET, VETO_THRESHOLD_PERCENT, YELLOW, RESET
    );
    eprintln!(
        "{}╠══════════════════════════════════════════════════════════════════╣{}",
        YELLOW, RESET
    );
    eprintln!(
        "{}║{}  Review changelog and vote if you have objections:             {}║{}",
        YELLOW, RESET, YELLOW, RESET
    );
    eprintln!(
        "{}║{}                                                                {}║{}",
        YELLOW, RESET, YELLOW, RESET
    );
    eprintln!(
        "{}║{}    {}doli-node update status{}         # See full details        {}║{}",
        YELLOW, RESET, CYAN, RESET, YELLOW, RESET
    );
    eprintln!(
        "{}║{}    {}doli-node update vote --veto --key <producer.json>{}        {}║{}",
        YELLOW, RESET, CYAN, RESET, YELLOW, RESET
    );
    eprintln!(
        "{}║{}                                                                {}║{}",
        YELLOW, RESET, YELLOW, RESET
    );
    eprintln!(
        "{}╚══════════════════════════════════════════════════════════════════╝{}",
        YELLOW, RESET
    );
    eprintln!();

    // Log to tracing as well
    info!(
        "Update v{} pending - {} days {}h remaining - {}/{} vetos ({}%)",
        pending.release.version, days, hours, veto_count, total_producers, veto_pct
    );
}

/// Display grace period notification (approved, waiting to enforce)
fn display_grace_period_notification(pending: &PendingUpdate) {
    use colors::*;

    let hours = pending
        .enforcement
        .as_ref()
        .map(|e| e.hours_until_enforcement())
        .unwrap_or(48);

    eprintln!();
    eprintln!(
        "{}╔══════════════════════════════════════════════════════════════════╗{}",
        GREEN, RESET
    );
    eprintln!(
        "{}║                    ✅ UPDATE APPROVED                             ║{}",
        GREEN, RESET
    );
    eprintln!(
        "{}╠══════════════════════════════════════════════════════════════════╣{}",
        GREEN, RESET
    );
    eprintln!(
        "{}║{}  Version: {}{} -> {}{}                                         {}║{}",
        GREEN,
        RESET,
        CYAN,
        current_version(),
        pending.release.version,
        RESET,
        GREEN,
        RESET
    );
    eprintln!(
        "{}║{}                                                                {}║{}",
        GREEN, RESET, GREEN, RESET
    );
    eprintln!(
        "{}║{}  The community has approved this update.                       {}║{}",
        GREEN, RESET, GREEN, RESET
    );
    eprintln!(
        "{}║{}  Grace period: {}{} hours{} until enforcement.                   {}║{}",
        GREEN, RESET, BOLD, hours, RESET, GREEN, RESET
    );
    eprintln!(
        "{}║{}                                                                {}║{}",
        GREEN, RESET, GREEN, RESET
    );
    eprintln!(
        "{}║{}  After grace period, {}outdated nodes cannot produce blocks{}.   {}║{}",
        GREEN, RESET, YELLOW, RESET, GREEN, RESET
    );
    eprintln!(
        "{}╠══════════════════════════════════════════════════════════════════╣{}",
        GREEN, RESET
    );
    eprintln!(
        "{}║{}  To update now:                                                {}║{}",
        GREEN, RESET, GREEN, RESET
    );
    eprintln!(
        "{}║{}    {}doli-node update apply{}                                    {}║{}",
        GREEN, RESET, CYAN, RESET, GREEN, RESET
    );
    eprintln!(
        "{}║{}                                                                {}║{}",
        GREEN, RESET, GREEN, RESET
    );
    eprintln!(
        "{}╚══════════════════════════════════════════════════════════════════╝{}",
        GREEN, RESET
    );
    eprintln!();

    info!(
        "Update v{} APPROVED - {} hours until enforcement - run 'doli-node update apply'",
        pending.release.version, hours
    );
}

/// Display enforcement notification (production blocked)
fn display_enforcement_notification(pending: &PendingUpdate) {
    use colors::*;

    eprintln!();
    eprintln!(
        "{}╔══════════════════════════════════════════════════════════════════╗{}",
        RED, RESET
    );
    eprintln!(
        "{}║                    ⚠️  PRODUCTION PAUSED                          ║{}",
        RED, RESET
    );
    eprintln!(
        "{}╠══════════════════════════════════════════════════════════════════╣{}",
        RED, RESET
    );
    eprintln!(
        "{}║{}                                                                {}║{}",
        RED, RESET, RED, RESET
    );
    eprintln!(
        "{}║{}  Your node ({}{}{}) is outdated.                               {}║{}",
        RED,
        RESET,
        YELLOW,
        current_version(),
        RESET,
        RED,
        RESET
    );
    eprintln!(
        "{}║{}  Required version: {}{}{}                                        {}║{}",
        RED, RESET, GREEN, pending.release.version, RESET, RED, RESET
    );
    eprintln!(
        "{}║{}                                                                {}║{}",
        RED, RESET, RED, RESET
    );
    eprintln!(
        "{}║{}  Network security requires all producers to be updated.        {}║{}",
        RED, RESET, RED, RESET
    );
    eprintln!(
        "{}║{}  Block production is paused until update is complete.          {}║{}",
        RED, RESET, RED, RESET
    );
    eprintln!(
        "{}║{}                                                                {}║{}",
        RED, RESET, RED, RESET
    );
    eprintln!(
        "{}╠══════════════════════════════════════════════════════════════════╣{}",
        RED, RESET
    );
    eprintln!(
        "{}║{}  To update, run:                                               {}║{}",
        RED, RESET, RED, RESET
    );
    eprintln!(
        "{}║{}    {}doli-node update apply{}                                    {}║{}",
        RED, RESET, CYAN, RESET, RED, RESET
    );
    eprintln!(
        "{}║{}                                                                {}║{}",
        RED, RESET, RED, RESET
    );
    eprintln!(
        "{}║{}  After updating, production will resume automatically.         {}║{}",
        RED, RESET, RED, RESET
    );
    eprintln!(
        "{}║{}                                                                {}║{}",
        RED, RESET, RED, RESET
    );
    eprintln!(
        "{}╚══════════════════════════════════════════════════════════════════╝{}",
        RED, RESET
    );
    eprintln!();

    warn!(
        "PRODUCTION PAUSED: Node {} is outdated, required version {}",
        current_version(),
        pending.release.version
    );
}

/// The update service that runs in the background
pub struct UpdateService {
    config: UpdateConfig,
    pending: Arc<RwLock<Option<PendingUpdate>>>,
    vote_tx: mpsc::Sender<VoteMessage>,
    vote_rx: mpsc::Receiver<VoteMessage>,
    data_dir: PathBuf,
    /// Track when we last showed the notification (to avoid spam)
    last_notification: Arc<RwLock<u64>>,
}

/// How often to show the notification reminder (every 6 hours)
const NOTIFICATION_INTERVAL_SECS: u64 = 6 * 3600;

impl UpdateService {
    /// Create a new update service
    pub fn new(config: UpdateConfig, data_dir: PathBuf) -> Self {
        let (vote_tx, vote_rx) = mpsc::channel(100);

        // Try to load existing pending update from disk
        let pending = PendingUpdate::load(&data_dir);
        if let Some(ref p) = pending {
            info!(
                "Loaded pending update v{} from disk ({} days remaining)",
                p.release.version,
                p.days_remaining()
            );
        }

        Self {
            config,
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
                    self.check_for_updates(&producer_count_fn).await;
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
    async fn check_for_updates(&mut self, producer_count_fn: &impl Fn() -> usize) {
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
                } else if veto_period_ended(&p.release) {
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
            } else {
                None
            }
        };

        // Handle transitions
        match transition {
            Some(UpdateTransition::Approved(result)) => {
                // Transition to grace period
                let mut pending = self.pending.write().await;
                if let Some(ref mut p) = *pending {
                    info!(
                        "Update {} APPROVED ({}% veto, threshold {}%) - Grace period: 48 hours",
                        p.release.version, result.veto_percent, VETO_THRESHOLD_PERCENT
                    );

                    // Mark as approved and set enforcement
                    p.approved = true;
                    p.enforcement = Some(VersionEnforcement::from_approved_release(&p.release));

                    // Save updated state
                    if let Err(e) = p.save(&self.data_dir) {
                        error!("Failed to save approved update state: {}", e);
                    }

                    // Show grace period notification
                    display_grace_period_notification(p);
                    *self.last_notification.write().await = updater::current_timestamp();
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
                // Activate enforcement
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
                }
            }
            None => {
                // No transition needed
            }
        }
    }
}

/// State transitions for updates
enum UpdateTransition {
    /// Veto period ended, update approved
    Approved(updater::VoteResult),
    /// Veto period ended, update rejected
    Rejected(updater::VoteResult),
    /// Grace period ended, activate enforcement
    ActivateEnforcement,
}

/// Start the update service in a background task
///
/// Returns `(vote_tx, pending)`:
/// - `vote_tx`: channel to forward gossip votes into the UpdateService
/// - `pending`: shared state for RPC to read live update status
pub fn spawn_update_service(
    config: UpdateConfig,
    data_dir: PathBuf,
    producer_count_fn: impl Fn() -> usize + Send + Sync + 'static,
    is_producer_fn: impl Fn(&str) -> bool + Send + Sync + 'static,
) -> (
    mpsc::Sender<VoteMessage>,
    Arc<RwLock<Option<PendingUpdate>>>,
) {
    let service = UpdateService::new(config, data_dir);
    let vote_tx = service.vote_sender();
    let pending = service.pending_state();

    tokio::spawn(async move {
        service.run(producer_count_fn, is_producer_fn).await;
    });

    (vote_tx, pending)
}

/// Get the pending update version from disk (for CLI use)
pub fn get_pending_version(data_dir: &Path) -> Option<String> {
    PendingUpdate::load(data_dir).map(|p| p.release.version)
}

/// Get the current version enforcement state (for production check)
pub fn get_version_enforcement(data_dir: &Path) -> Option<VersionEnforcement> {
    PendingUpdate::load(data_dir).and_then(|p| p.enforcement)
}

/// Check if production is currently allowed based on version enforcement
pub fn is_production_allowed(data_dir: &Path) -> Result<(), ProductionBlocked> {
    let enforcement = get_version_enforcement(data_dir);
    check_production_allowed(enforcement.as_ref())
}

/// Get the pending update for display (for CLI use)
pub fn get_pending_update(data_dir: &Path) -> Option<PendingUpdate> {
    PendingUpdate::load(data_dir)
}

/// CLI for update management
pub mod cli {
    use super::*;

    /// Show update status from disk (for CLI commands)
    pub fn show_status_from_disk(data_dir: &Path) {
        match PendingUpdate::load(data_dir) {
            Some(p) => {
                use colors::*;

                println!();
                println!("{}═══ Pending Update ═══{}", BOLD, RESET);
                println!();
                println!(
                    "{}Version:{} {} -> {}",
                    BOLD,
                    RESET,
                    current_version(),
                    p.release.version
                );
                println!(
                    "{}Published:{} {}",
                    BOLD,
                    RESET,
                    chrono::DateTime::from_timestamp(p.release.published_at as i64, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
                        .unwrap_or_else(|| "Unknown".to_string())
                );
                println!();
                println!("{}═══ Changelog ═══{}", BOLD, RESET);
                println!();
                println!("{}", p.release.changelog);
                println!();

                // Show different status based on update phase
                if let Some(ref enforcement) = p.enforcement {
                    if enforcement.should_enforce() {
                        // ENFORCEMENT ACTIVE
                        println!("{}═══ Status: ENFORCEMENT ACTIVE ═══{}", RED, RESET);
                        println!();
                        if enforcement.version_meets_requirement(current_version()) {
                            println!(
                                "{}✅ Your node meets the version requirement.{}",
                                GREEN, RESET
                            );
                        } else {
                            println!("{}❌ Your node is OUTDATED.{}", RED, RESET);
                            println!("{}   Block production is PAUSED.{}", RED, RESET);
                        }
                        println!();
                        println!("Required version: {}", enforcement.min_version);
                        println!();
                        println!("{}═══ To Update ═══{}", BOLD, RESET);
                        println!();
                        println!("  {}doli-node update apply{}", CYAN, RESET);
                        println!();
                    } else {
                        // GRACE PERIOD
                        let hours = enforcement.hours_until_enforcement();
                        println!("{}═══ Status: APPROVED - Grace Period ═══{}", GREEN, RESET);
                        println!();
                        println!("The community has approved this update.");
                        println!("Time until enforcement: {}{} hours{}", BOLD, hours, RESET);
                        println!();
                        println!("After the grace period, outdated nodes cannot produce blocks.");
                        println!();
                        println!("{}═══ To Update Now ═══{}", BOLD, RESET);
                        println!();
                        println!("  {}doli-node update apply{}", CYAN, RESET);
                        println!();
                    }
                } else if p.approved {
                    // Approved but no enforcement yet (shouldn't happen normally)
                    println!("{}═══ Status: APPROVED ═══{}", GREEN, RESET);
                    println!();
                    println!("Update has been approved by the community.");
                    println!();
                } else {
                    // VETO PERIOD
                    let days = p.days_remaining();
                    let hours = p.hours_remaining();
                    let veto_count = p.vote_tracker.veto_count();

                    println!("{}═══ Status: Veto Period ═══{}", YELLOW, RESET);
                    println!();
                    println!("Time remaining:  {} days, {} hours", days, hours);
                    println!("Veto count:      {}", veto_count);
                    println!("Veto threshold:  {}%", VETO_THRESHOLD_PERCENT);
                    println!();

                    if days == 0 && hours < 24 {
                        println!("{}⏰ Veto period ending soon!{}", YELLOW, RESET);
                        println!();
                    }

                    println!("{}═══ How to Vote ═══{}", BOLD, RESET);
                    println!();
                    println!("If you have objections to this update, vote to veto:");
                    println!();
                    println!(
                        "  {}doli-node update vote --veto --key <producer.json>{}",
                        CYAN, RESET
                    );
                    println!();
                    println!(
                        "If the veto threshold ({}%) is reached, the update will be rejected.",
                        VETO_THRESHOLD_PERCENT
                    );
                    println!("If not vetoed, the update will be approved after the veto period.");
                    println!();
                }

                // Timeline summary
                println!("{}═══ Update Timeline ═══{}", BOLD, RESET);
                println!();
                println!("  Day 0-7:  Veto period (producers can vote to reject)");
                println!("  Day 7-9:  Grace period (48h to update)");
                println!("  Day 9+:   Enforcement (outdated nodes cannot produce)");
                println!();
            }
            None => {
                println!("No pending updates");
                println!("Current version: {}", current_version());
            }
        }
    }
}
