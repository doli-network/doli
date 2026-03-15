use std::path::Path;

use anyhow::Result;
use doli_core::Network;
use tracing::{error, info};

use crate::cli::UpdateCommands;
use crate::keys::load_producer_key;
use crate::updater;

pub(crate) async fn handle_update_command(
    action: UpdateCommands,
    data_dir: &Path,
    network: Network,
) -> Result<()> {
    match action {
        UpdateCommands::Check => {
            info!("Checking for updates...");
            match updater::fetch_latest_release(None, None).await {
                Ok(Some(release)) => {
                    let current = updater::current_version();
                    if updater::is_newer_version(&release.version, current) {
                        println!("Update available: {} -> {}", current, release.version);
                        println!("Changelog: {}", release.changelog);
                    } else {
                        println!("Already on latest version ({})", current);
                    }
                }
                Ok(None) => {
                    println!("Could not fetch release information");
                }
                Err(e) => {
                    error!("Failed to check for updates: {}", e);
                }
            }
        }
        UpdateCommands::Status => {
            // Show status from disk-persisted pending update
            updater::cli::show_status_from_disk(data_dir);
        }
        UpdateCommands::Vote { veto, approve, key } => {
            if veto == approve {
                error!("Specify either --veto or --approve, not both or neither");
                return Ok(());
            }

            let vote_type = if veto {
                updater::Vote::Veto
            } else {
                updater::Vote::Approve
            };

            // Load producer key
            let keypair = load_producer_key(&key)?;
            let pubkey_hex = keypair.public_key().to_hex();
            info!(
                "Voting {} with producer key: {}",
                if veto { "VETO" } else { "APPROVE" },
                &pubkey_hex[..16]
            );

            // Auto-detect version from pending update on disk
            let version = std::env::var("DOLI_VOTE_VERSION")
                .ok()
                .or_else(|| updater::get_pending_version(data_dir));

            let version = match version {
                Some(v) => v,
                None => {
                    println!("No pending update found.");
                    println!("Run 'doli-node update status' to check for pending updates.");
                    println!();
                    println!("If you want to vote on a specific version, set DOLI_VOTE_VERSION:");
                    println!("  DOLI_VOTE_VERSION=1.2.0 doli-node update vote --veto --key producer.json");
                    return Ok(());
                }
            };

            println!("Voting on update: v{}", version);

            let mut vote_msg = updater::VoteMessage::new(version.clone(), vote_type, pubkey_hex);

            // Sign the vote
            let message_bytes = vote_msg.message_bytes();
            let signature = crypto::signature::sign(&message_bytes, keypair.private_key());
            vote_msg.signature = signature.to_hex();

            // Serialize vote message
            let vote_json = serde_json::to_string_pretty(&vote_msg)?;
            println!();
            println!("Signed vote message:");
            println!("{}", vote_json);
            println!();
            println!("To submit this vote, send it to a running node via RPC:");
            println!("curl -X POST http://localhost:28500 \\");
            println!("  -H 'Content-Type: application/json' \\");
            println!("  -d '{{\"jsonrpc\":\"2.0\",\"method\":\"submitVote\",\"params\":{{\"vote\":{}}},\"id\":1}}'",
                     serde_json::to_string(&vote_msg)?);
        }
        UpdateCommands::Apply { force } => {
            // Get pending update
            let pending = match updater::get_pending_update(data_dir) {
                Some(p) => p,
                None => {
                    println!("No pending update to apply.");
                    println!("Run 'doli-node update check' to check for available updates.");
                    return Ok(());
                }
            };

            // Check if update is approved
            if !pending.approved && !force {
                println!("Update v{} is not yet approved.", pending.release.version);
                println!("Veto period: {} days remaining.", pending.days_remaining());
                println!();
                println!("Use --force to apply anyway (not recommended).");
                return Ok(());
            }

            println!();
            println!("╔══════════════════════════════════════════════════════════════════╗");
            println!("║                    Applying Update                               ║");
            println!("╠══════════════════════════════════════════════════════════════════╣");
            println!(
                "║  Current version: {}                                            ║",
                updater::current_version()
            );
            println!(
                "║  Target version:  {}                                            ║",
                pending.release.version
            );
            println!("╚══════════════════════════════════════════════════════════════════╝");
            println!();

            println!("Downloading {}...", pending.release.version);

            // --force bypasses approval check but NEVER bypasses veto period check
            // The veto period check is a critical security measure to prevent
            // producers from applying potentially malicious updates before
            // the community has had time to review.
            let approved_or_forced = pending.approved || force;

            match updater::apply_update(&pending.release, approved_or_forced, None).await {
                Ok(()) => {
                    println!();
                    println!(
                        "╔══════════════════════════════════════════════════════════════════╗"
                    );
                    println!(
                        "║                    ✅ UPDATE SUCCESSFUL                          ║"
                    );
                    println!(
                        "╠══════════════════════════════════════════════════════════════════╣"
                    );
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  Version: {} → {}                                            ║",
                        updater::current_version(),
                        pending.release.version
                    );
                    println!(
                        "║  Status: Production enabled                                      ║"
                    );
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  Your node is now up to date.                                    ║"
                    );
                    println!(
                        "║  Restarting node to apply changes...                             ║"
                    );
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "╚══════════════════════════════════════════════════════════════════╝"
                    );
                    println!();

                    // Remove pending update file
                    if let Err(e) = updater::PendingUpdate::remove(data_dir) {
                        error!("Failed to remove pending_update.json: {}", e);
                    }

                    // Restart node
                    updater::restart_node();
                }
                Err(e) => {
                    error!("Failed to apply update: {}", e);
                    println!();
                    println!("Update failed. Please try again or update manually.");
                }
            }
        }
        UpdateCommands::Votes { version } => {
            // Get version from argument or pending update
            let target_version = version.or_else(|| updater::get_pending_version(data_dir));

            let target_version = match target_version {
                Some(v) => v,
                None => {
                    println!("No version specified and no pending update found.");
                    println!("Use --version <ver> to check votes for a specific version.");
                    return Ok(());
                }
            };

            println!();
            println!("╔══════════════════════════════════════════════════════════════════╗");
            println!(
                "║                    VOTE STATUS: v{}                              ║",
                target_version
            );
            println!("╠══════════════════════════════════════════════════════════════════╣");

            // Check for votes in pending update
            if let Some(pending) = updater::get_pending_update(data_dir) {
                if pending.release.version == target_version {
                    let tracker = &pending.vote_tracker;
                    let veto_count = tracker.veto_count();
                    let approve_count = tracker.approval_count();
                    let total = tracker.total_votes();
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  Veto votes:    {}                                               ║",
                        veto_count
                    );
                    println!(
                        "║  Approve votes: {}                                               ║",
                        approve_count
                    );
                    println!(
                        "║  Total votes:   {}                                               ║",
                        total
                    );
                    println!(
                        "║  Threshold:     40% of weighted votes                            ║"
                    );
                    println!(
                        "║                                                                  ║"
                    );
                    if pending.approved {
                        println!(
                            "║  Status: APPROVED                                                ║"
                        );
                    } else {
                        println!(
                            "║  Status: Voting in progress                                      ║"
                        );
                    }
                } else {
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  No votes found for this version.                                ║"
                    );
                }
            } else {
                println!("║                                                                  ║");
                println!("║  No pending update found.                                         ║");
            }
            println!("║                                                                  ║");
            println!("╚══════════════════════════════════════════════════════════════════╝");
        }
        UpdateCommands::Rollback => {
            println!();
            println!("╔══════════════════════════════════════════════════════════════════╗");
            println!("║                    MANUAL ROLLBACK                               ║");
            println!("╠══════════════════════════════════════════════════════════════════╣");

            match updater::rollback().await {
                Ok(()) => {
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  ✅ Rollback successful                                          ║"
                    );
                    println!(
                        "║                                                                  ║"
                    );
                    println!("║  Backup restored. Restart the node to use the previous version. ║");
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "╚══════════════════════════════════════════════════════════════════╝"
                    );
                }
                Err(e) => {
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  ❌ Rollback failed: {}                                          ║",
                        e
                    );
                    println!(
                        "║                                                                  ║"
                    );
                    println!("║  No backup found or backup is corrupted.                        ║");
                    println!("║  You may need to reinstall manually.                            ║");
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "╚══════════════════════════════════════════════════════════════════╝"
                    );
                }
            }
        }
        UpdateCommands::Verify { version } => {
            println!();
            println!("╔══════════════════════════════════════════════════════════════════╗");
            println!("║                    VERIFY RELEASE                                ║");
            println!("╠══════════════════════════════════════════════════════════════════╣");

            // First check if we have a pending update matching the version
            let release = if let Some(pending) = updater::get_pending_update(data_dir) {
                if pending.release.version == version {
                    Some(pending.release)
                } else {
                    None
                }
            } else {
                None
            };

            // If no pending update, try to fetch from network
            let release = match release {
                Some(r) => Some(r),
                None => {
                    info!("Fetching latest release to verify...");
                    match updater::fetch_latest_release(None, None).await {
                        Ok(Some(r)) if r.version == version => Some(r),
                        Ok(Some(r)) => {
                            println!("║                                                                  ║");
                            println!("║  ❌ Requested version {} not found                               ║", version);
                            println!("║     Latest available: {}                                         ║", r.version);
                            println!("║                                                                  ║");
                            println!("╚══════════════════════════════════════════════════════════════════╝");
                            return Ok(());
                        }
                        Ok(None) => None,
                        Err(e) => {
                            println!("║                                                                  ║");
                            println!("║  ❌ Failed to fetch release: {}                                  ║", e);
                            println!("║                                                                  ║");
                            println!("╚══════════════════════════════════════════════════════════════════╝");
                            return Ok(());
                        }
                    }
                }
            };

            match release {
                Some(release) => {
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  Version: {}                                                     ║",
                        release.version
                    );
                    if release.binary_sha256.len() >= 16 {
                        println!(
                            "║  SHA256:  {}...                                                  ║",
                            &release.binary_sha256[..16]
                        );
                    }
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  MAINTAINER SIGNATURES                                           ║"
                    );

                    match updater::verify_release_signatures(&release, network) {
                        Ok(()) => {
                            for sig in &release.signatures {
                                if sig.public_key.len() >= 16 {
                                    println!(
                                        "║  ✓ {}...                                              ║",
                                        &sig.public_key[..16]
                                    );
                                }
                            }
                            println!("║                                                                  ║");
                            println!("║  ✅ Release signatures verified (3/5 threshold met)             ║");
                        }
                        Err(e) => {
                            println!("║  ❌ Verification failed: {}                                      ║", e);
                        }
                    }
                    println!(
                        "║                                                                  ║"
                    );
                }
                None => {
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  ❌ Release v{} not found                                        ║",
                        version
                    );
                    println!(
                        "║                                                                  ║"
                    );
                }
            }
            println!("╚══════════════════════════════════════════════════════════════════╝");
        }
    }
    Ok(())
}
