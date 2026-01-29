//! Lock file guard for preventing multiple producer instances

use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::path::Path;
use tracing::{debug, info};

use super::constants::PRODUCER_LOCK_FILE;
use super::errors::ProducerStartupError;

/// Guard that holds an exclusive lock on the producer lock file
///
/// This prevents two producer instances from running on the same machine.
/// The lock is automatically released when the guard is dropped.
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
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;

        // Try to acquire exclusive lock (non-blocking)
        match file.try_lock_exclusive() {
            Ok(()) => {
                info!("Producer lock acquired: {:?}", lock_path);

                // Write PID to lock file for debugging
                use std::io::Write;
                let mut f = &file;
                let _ = writeln!(f, "{}", std::process::id());

                Ok(Self {
                    lock_file: file,
                    lock_path,
                })
            }
            Err(_) => {
                // Lock failed - another instance is running
                Err(ProducerStartupError::AnotherLocalInstance)
            }
        }
    }
}

impl Drop for ProducerGuard {
    fn drop(&mut self) {
        // Unlock is automatic when file is closed, but be explicit
        if let Err(e) = self.lock_file.unlock() {
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
