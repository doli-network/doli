//! Console notification banners for update status display.
//!
//! Three notification types based on update lifecycle:
//! - Veto period: countdown + vote instructions
//! - Grace period: approved, countdown to enforcement
//! - Enforcement: production blocked, update instructions

use super::colors::*;
use super::PendingUpdate;
use tracing::{info, warn};
use updater::{current_version, VETO_THRESHOLD_PERCENT};

/// Display mandatory update notification to console
///
/// Shows different banners based on update state:
/// - Veto period: Shows countdown and how to veto
/// - Grace period: Shows update approved, countdown to enforcement
/// - Enforcement: Shows production paused, how to update
pub fn display_update_notification(pending: &PendingUpdate, total_producers: usize) {
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
pub(super) fn display_grace_period_notification(pending: &PendingUpdate) {
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
pub(super) fn display_enforcement_notification(pending: &PendingUpdate) {
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
