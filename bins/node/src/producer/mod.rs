//! Producer safety protections
//!
//! This module provides protections against accidental slashing:
//! 1. Lock file - prevents two instances on the same machine
//! 2. Signed slots database - prevents double-signing after restart
//! 3. Duplicate key detection - detects if another node is using our key
//! 4. --force-start flag - escape hatch with safeguards

mod constants;
mod errors;
mod guard;
mod signed_slots;

pub use constants::*;
pub use errors::ProducerStartupError;
pub use guard::ProducerGuard;
pub use signed_slots::SignedSlotsDb;

use crypto::PublicKey;
use std::path::Path;
use tracing::{info, warn};

/// Complete producer startup check
///
/// This performs all safety checks before allowing block production:
/// 1. Acquire lock file (prevents two local instances)
/// 2. Open signed slots database (prevents double-signing)
/// 3. Check for duplicate key in recent blocks (optional, can be skipped with force_start)
///
/// Returns the guard and signed slots DB if all checks pass.
pub async fn startup_checks(
    data_dir: &Path,
    our_pubkey: &PublicKey,
    force_start: bool,
    recent_blocks: Option<Vec<(u64, PublicKey, u64)>>, // (slot, producer, timestamp)
) -> Result<(ProducerGuard, SignedSlotsDb), ProducerStartupError> {
    // 1. Acquire lock file
    info!("Acquiring producer lock file...");
    let guard = ProducerGuard::acquire(data_dir)?;
    info!("Lock file acquired");

    // 2. Open signed slots database
    info!("Opening signed slots database...");
    let signed_slots = SignedSlotsDb::open(data_dir)?;
    info!("Signed slots database opened");

    // 3. Check for duplicate key in recent blocks (unless force_start)
    if !force_start {
        if let Some(blocks) = recent_blocks {
            check_for_active_key(our_pubkey, &blocks)?;
        }
    } else {
        warn!("--force-start specified: skipping duplicate key detection");
        warn!("If another node is running with this key, YOU WILL BE SLASHED!");
    }

    Ok((guard, signed_slots))
}

/// Check if our key was active recently in the network
fn check_for_active_key(
    our_pubkey: &PublicKey,
    recent_blocks: &[(u64, PublicKey, u64)], // (slot, producer, timestamp)
) -> Result<(), ProducerStartupError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    for (slot, producer, timestamp) in recent_blocks {
        if producer == our_pubkey {
            let age_seconds = now.saturating_sub(*timestamp);

            if age_seconds < DUPLICATE_KEY_DETECTION_SECONDS {
                let wait_seconds = DUPLICATE_KEY_DETECTION_SECONDS - age_seconds;
                return Err(ProducerStartupError::DuplicateKeyActive {
                    last_block_slot: *slot,
                    seconds_ago: age_seconds,
                    wait_seconds,
                });
            }
        }
    }

    Ok(())
}

/// Interactive confirmation for --force-start
pub fn confirm_force_start() -> bool {
    use std::io::{self, Write};

    println!();
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║                    ⚠️  WARNING: FORCE START                        ║");
    println!("╠══════════════════════════════════════════════════════════════════╣");
    println!("║                                                                  ║");
    println!("║  You are about to skip duplicate key detection.                  ║");
    println!("║                                                                  ║");
    println!("║  If another node is running with this key, your bond will be     ║");
    println!("║  PERMANENTLY BURNED (100% slashing). There is NO recovery.       ║");
    println!("║                                                                  ║");
    println!("║  Only proceed if you are ABSOLUTELY CERTAIN the other node       ║");
    println!("║  is completely stopped.                                          ║");
    println!("║                                                                  ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!();
    print!("Type 'I UNDERSTAND' to proceed: ");
    io::stdout().flush().ok();

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_ok() {
        return input.trim() == "I UNDERSTAND";
    }

    false
}
