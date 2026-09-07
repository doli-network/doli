//! Binary rollback and process restart.

use tokio::fs;
use tracing::{error, info, warn};

use crate::apply::{backup_path, current_binary_path};
use crate::types::{Result, UpdateError};

/// Rollback to the backup binary
pub async fn rollback() -> Result<()> {
    let current = current_binary_path()?;
    let backup = backup_path()?;

    if !backup.exists() {
        error!("No backup found at {:?}", backup);
        return Err(UpdateError::InstallFailed("No backup available".into()));
    }

    warn!("Rolling back to previous version");

    // Restore from backup
    fs::copy(&backup, &current).await?;

    info!("Rollback completed");
    Ok(())
}

/// Restart the node process
///
/// This function does not return - it replaces the current process
pub fn restart_node() -> ! {
    info!("Restarting node...");

    let current = match current_binary_path() {
        Ok(p) => p,
        Err(e) => {
            error!("Failed to get binary path for restart: {}", e);
            std::process::exit(1);
        }
    };

    // Get current args (skip the program name)
    let args: Vec<String> = std::env::args().skip(1).collect();

    // On Unix, use exec to replace the process
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new(&current).args(&args).exec();
        // exec only returns on error
        error!("Failed to restart: {}", err);
        std::process::exit(1);
    }

    // On Windows, spawn new process and exit
    #[cfg(windows)]
    {
        match std::process::Command::new(&current).args(&args).spawn() {
            Ok(_) => std::process::exit(0),
            Err(e) => {
                error!("Failed to restart: {}", e);
                std::process::exit(1);
            }
        }
    }
}
