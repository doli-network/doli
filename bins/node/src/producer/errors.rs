//! Producer startup error types

use thiserror::Error;

/// Errors that can occur during producer startup
#[derive(Debug, Error)]
pub enum ProducerStartupError {
    /// Another instance is running on this machine
    #[error(
        "Another doli-node instance is running on this machine.\n\
         Stop it with: systemctl stop doli-node\n\
         Or find the process: ps aux | grep doli-node"
    )]
    AnotherLocalInstance,

    /// Our key produced a block recently, suggesting another node is running
    #[error(
        "Your key produced a block {seconds_ago} seconds ago (slot {last_block_slot}).\n\
         Wait {wait_seconds} more seconds, or use --force-start if you are CERTAIN\n\
         the other node is stopped.\n\n\
         Using --force-start incorrectly WILL cause slashing (100% bond loss)."
    )]
    DuplicateKeyActive {
        last_block_slot: u64,
        seconds_ago: u64,
        wait_seconds: u64,
    },

    /// Attempted to sign a slot that was already signed
    #[error(
        "BLOCKED: Already signed slot {slot}. Signing again would cause slashing.\n\
         This should not happen. Check if you have multiple instances running.\n\
         If you just restarted, this is a safety feature protecting your bond."
    )]
    SlotAlreadySigned { slot: u64 },

    /// Failed to create or acquire lock file
    #[error("Failed to create producer lock file: {0}")]
    LockFileFailed(#[from] std::io::Error),

    /// Failed to open signed slots database
    #[error("Failed to open signed slots database: {0}")]
    SignedSlotsDbFailed(String),
}
