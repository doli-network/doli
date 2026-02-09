//! Producer safety constants

/// Time in seconds since last block from our key before we consider it safe to start
/// This prevents starting a second producer if another instance produced recently.
/// 5 minutes = 300 seconds (enough time to notice if another node is running)
pub const DUPLICATE_KEY_DETECTION_SECONDS: u64 = 300;

/// Number of recent blocks to check for duplicate key detection
/// 50 blocks is ~500 seconds on mainnet (10s slots), ~250 seconds on devnet (5s slots)
#[allow(dead_code)]
pub const DUPLICATE_KEY_DETECTION_BLOCKS: u64 = 50;

/// Lock file name within data directory
pub const PRODUCER_LOCK_FILE: &str = "producer.lock";

/// Signed slots database directory name within data directory
pub const SIGNED_SLOTS_DB_DIR: &str = "signed_slots.db";
