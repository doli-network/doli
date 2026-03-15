//! CLI display for update status (read from disk, no running service needed).

use std::path::Path;

use super::colors::*;
use super::PendingUpdate;
use updater::{current_version, VETO_THRESHOLD_PERCENT};

/// Show update status from disk (for CLI commands)
pub fn show_status_from_disk(data_dir: &Path) {
    match PendingUpdate::load(data_dir) {
        Some(p) => {
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
