//! Update-poll and vote-intake half of `UpdateService`.
//!
//! Split out of `service.rs` to keep that file inside the 500-line module budget.
//! Declared as a CHILD module of `service` (`#[path]` + `mod checks;`), which is what
//! lets it read `UpdateService`'s private fields.
//!
//! The state-transition half — `check_veto_status` and `auto_apply` — deliberately
//! stays in `service.rs`: those are the install-gating decisions INC-I-172 M1 changed,
//! and the regression tests anchor on that file.

use std::time::Duration;

use tracing::{debug, error, info, warn};
use updater::{
    current_version, fetch_latest_release, is_newer_version, verify_release_with_trust_root,
    TrustRoot, Vote, VoteMessage, VoteTracker,
};

use super::super::notifications::display_update_notification;
use super::super::PendingUpdate;
use super::{UpdateService, NOTIFICATION_INTERVAL_SECS};

/// How many times a voter-eligibility lookup is retried before the vote is dropped
/// (AUDIT-P1-015). The producer set is written by block application, so a held lock is
/// short-lived; three attempts across ~200 ms cover it without stalling the service.
const VOTER_LOOKUP_ATTEMPTS: usize = 3;

/// Delay between voter-eligibility lookup attempts.
const VOTER_LOOKUP_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Truncate to at most `max` CHARACTERS.
///
/// `&s[..16]` on an origin- or peer-supplied string panics when byte 16 lands inside a
/// multi-byte UTF-8 sequence (AUDIT-P3-010).
pub(super) fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

impl UpdateService {
    /// Resolve whether `producer_id` is an active producer, retrying on lock contention.
    ///
    /// `None` means the answer is still unknown after [`VOTER_LOOKUP_ATTEMPTS`] — it is
    /// never conflated with `Some(false)`.
    async fn resolve_voter_eligibility(
        &self,
        is_producer_fn: &impl Fn(&str) -> Option<bool>,
        producer_id: &str,
    ) -> Option<bool> {
        for attempt in 0..VOTER_LOOKUP_ATTEMPTS {
            if let Some(answer) = is_producer_fn(producer_id) {
                return Some(answer);
            }
            if attempt + 1 < VOTER_LOOKUP_ATTEMPTS {
                tokio::time::sleep(VOTER_LOOKUP_RETRY_DELAY).await;
            }
        }
        None
    }

    /// Show periodic reminder notification if enough time has passed
    pub(super) async fn maybe_show_reminder(&self, producer_count_fn: &impl Fn() -> Option<usize>) {
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
    pub(super) async fn check_for_updates(
        &mut self,
        producer_count_fn: &impl Fn() -> Option<usize>,
        maintainer_keys_fn: &impl Fn() -> TrustRoot,
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

        // Verify signatures against the resolved trust root. An absent, empty or
        // sub-threshold root refuses here — it does NOT fall back to the compiled
        // bootstrap keys (INC-I-172 F1).
        let trust_root = maintainer_keys_fn();
        if let Err(e) = verify_release_with_trust_root(&release, &trust_root) {
            error!(
                "Release {} rejected by the {} trust root ({} keys, threshold {}): {}",
                release.version,
                trust_root.provenance(),
                trust_root.keys().len(),
                trust_root.threshold(),
                e
            );
            return;
        }

        info!(
            "New update available: {} -> {} (veto period: {}s)",
            current_version(),
            release.version,
            self.config.veto_period_secs
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
    pub(super) async fn handle_vote(
        &self,
        vote_msg: VoteMessage,
        is_producer_fn: &impl Fn(&str) -> Option<bool>,
    ) {
        // Verify this is from an active producer.
        //
        // AUDIT-P1-015: `None` is "I could not check" (the producer-set lock was held),
        // NOT "not a producer". Collapsing the two dropped genuine VETO votes on
        // ordinary lock contention, which moves the tally in the approve direction. The
        // lookup is retried a bounded number of times rather than answered by a guess;
        // the vote can only be recorded on a definite `Some(true)`, because recording an
        // unverified one would let any peer veto every release.
        match self
            .resolve_voter_eligibility(is_producer_fn, &vote_msg.producer_id)
            .await
        {
            Some(true) => {}
            Some(false) => {
                debug!("Ignoring vote from non-producer: {}", vote_msg.producer_id);
                return;
            }
            None => {
                error!(
                    "VOTE_DROPPED: could not check whether {} is an active producer after {} \
                     attempts (producer-set lock held throughout); the vote on {} is NOT counted. \
                     A dropped vote moves the tally toward APPROVE.",
                    truncate_chars(&vote_msg.producer_id, 16),
                    VOTER_LOOKUP_ATTEMPTS,
                    vote_msg.version
                );
                return;
            }
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
                    truncate_chars(&vote_msg.producer_id, 16)
                );
                // Persist vote tracker to disk
                if let Err(e) = p.save(&self.data_dir) {
                    warn!("Failed to save vote state: {}", e);
                }
            }
        }
    }
}
