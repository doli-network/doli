//! Lock file guard for preventing multiple producer instances

use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::Path;
use tracing::{debug, info, warn};

use super::constants::PRODUCER_LOCK_FILE;
use super::errors::ProducerStartupError;

/// Guard that holds an exclusive lock on the producer lock file
///
/// This prevents two producer instances from running on the same machine.
/// The lock is automatically released when the guard is dropped.
/// If the lock holder is a dead process (crash/SIGKILL), the stale lock
/// is reclaimed automatically via PID check.
pub struct ProducerGuard {
    /// The locked file handle
    #[allow(dead_code)] // Kept alive for the lock
    lock_file: File,
    /// Path to the lock file (for logging)
    lock_path: std::path::PathBuf,
}

impl ProducerGuard {
    /// Attempt to acquire the producer lock
    ///
    /// Returns an error if another instance is already running.
    pub fn acquire(data_dir: &Path) -> Result<Self, ProducerStartupError> {
        let lock_path = data_dir.join(PRODUCER_LOCK_FILE);

        debug!("Attempting to acquire lock: {:?}", lock_path);

        // Create/open the lock file
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;

        // Try to acquire exclusive lock (non-blocking)
        match file.try_lock_exclusive() {
            Ok(()) => {
                Self::write_pid(&file);
                info!("Producer lock acquired: {:?}", lock_path);
                Ok(Self {
                    lock_file: file,
                    lock_path,
                })
            }
            Err(_) => {
                // Lock held — check if the holder is still alive
                if Self::holder_is_dead(&file) {
                    warn!("Stale producer lock detected (holder process is dead). Reclaiming.");
                    // Force unlock the stale lock, then re-acquire
                    let _ = FileExt::unlock(&file);
                    drop(file);

                    // Re-open and try again
                    let file = OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .open(&lock_path)?;

                    match file.try_lock_exclusive() {
                        Ok(()) => {
                            Self::write_pid(&file);
                            info!("Producer lock reclaimed: {:?}", lock_path);
                            Ok(Self {
                                lock_file: file,
                                lock_path,
                            })
                        }
                        Err(_) => Err(ProducerStartupError::AnotherLocalInstance),
                    }
                } else {
                    Err(ProducerStartupError::AnotherLocalInstance)
                }
            }
        }
    }

    /// Write our PID to the lock file
    fn write_pid(file: &File) {
        use std::io::Write;
        let mut f = file;
        let _ = f.set_len(0);
        let _ = writeln!(f, "{}", std::process::id());
    }

    /// Check if the PID in the lock file is still a running process
    fn holder_is_dead(file: &File) -> bool {
        let mut contents = String::new();
        let mut f = file;
        if f.read_to_string(&mut contents).is_err() {
            return false; // Can't read — assume alive to be safe
        }
        let pid_str = contents.trim();
        if pid_str.is_empty() {
            return true; // Empty lock file — stale
        }
        match pid_str.parse::<u32>() {
            Ok(pid) => {
                // Check if process exists via kill -0 (no signal sent)
                !std::process::Command::new("kill")
                    .args(["-0", &pid.to_string()])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
            }
            Err(_) => true, // Garbage in lock file — stale
        }
    }
}

impl Drop for ProducerGuard {
    fn drop(&mut self) {
        // Unlock is automatic when file is closed, but be explicit
        if let Err(e) = FileExt::unlock(&self.lock_file) {
            debug!("Failed to unlock producer lock file: {}", e);
        }
        info!("Producer lock released: {:?}", self.lock_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_lock_file_prevents_second_instance() {
        let temp_dir = TempDir::new().unwrap();

        // First instance should succeed
        let guard1 = ProducerGuard::acquire(temp_dir.path());
        assert!(guard1.is_ok());

        // Second instance should fail
        let guard2 = ProducerGuard::acquire(temp_dir.path());
        assert!(matches!(
            guard2,
            Err(ProducerStartupError::AnotherLocalInstance)
        ));

        // After dropping first, second should succeed
        drop(guard1);
        let guard3 = ProducerGuard::acquire(temp_dir.path());
        assert!(guard3.is_ok());
    }

    #[test]
    fn test_lock_file_released_on_shutdown() {
        let temp_dir = TempDir::new().unwrap();

        // Create and drop guard
        {
            let _guard = ProducerGuard::acquire(temp_dir.path()).unwrap();
            // Guard dropped here
        }

        // Should be able to acquire again
        let guard2 = ProducerGuard::acquire(temp_dir.path());
        assert!(guard2.is_ok());
    }
}
