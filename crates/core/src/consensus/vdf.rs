use crate::types::{BlockHeight, Slot};

// ==================== VDF Parameters ====================
//
// VDF is used for both block production (anti-grinding) and registration (anti-Sybil).
// Time is the scarce resource in DOLI - VDF ensures sequential computation.

/// VDF discriminant bits for proofs.
/// Must be large enough for cryptographic security.
pub const VDF_DISCRIMINANT_BITS: u32 = 1024;

/// Block VDF iterations (800,000 iterations ~= 55ms on reference hardware)
/// This is the fixed T parameter for block production VDF.
/// Reduced from 10M (~700ms) to enable 2s sequential fallback windows.
pub const T_BLOCK: u64 = 800_000;

/// Legacy alias for T_BLOCK
pub const T_BLOCK_BASE: u64 = T_BLOCK;

/// Maximum T value for blocks - same as T_BLOCK (fixed)
pub const T_BLOCK_CAP: u64 = T_BLOCK;

/// VDF target duration in milliseconds (~55ms heartbeat)
pub const VDF_TARGET_MS: u64 = 55;

/// VDF deadline in milliseconds (must complete within fallback window)
pub const VDF_DEADLINE_MS: u64 = 2_000;

/// Get T parameter for block VDF (fixed at T_BLOCK).
///
/// VDF is required for block production as anti-grinding protection.
/// The input is constructed from: prev_hash, tx_root, slot, producer_key
#[must_use]
pub fn t_block(_height: BlockHeight) -> u64 {
    T_BLOCK
}

/// Construct the VDF input for block production.
///
/// The VDF input is: HASH(prefix || prev_hash || tx_root || slot || producer_key)
/// This ensures the VDF computation is bound to the specific block context.
///
/// # Arguments
/// * `prev_hash` - Hash of the previous block
/// * `tx_root` - Merkle root of transactions in this block
/// * `slot` - The slot number for this block
/// * `producer_key` - The producer's public key
///
/// # Returns
/// A 32-byte hash to use as VDF input
#[must_use]
pub fn construct_vdf_input(
    prev_hash: &crypto::Hash,
    tx_root: &crypto::Hash,
    slot: Slot,
    producer_key: &crypto::PublicKey,
) -> crypto::Hash {
    use crypto::hash::hash_concat;
    hash_concat(&[
        b"DOLI_VDF_BLOCK_V1",
        prev_hash.as_bytes(),
        tx_root.as_bytes(),
        &slot.to_le_bytes(),
        producer_key.as_bytes(),
    ])
}

/// Registration VDF iterations (~30 seconds on reference hardware).
/// Fixed value — does NOT scale with producer count.
/// At scale, the bond (10 DOLI) provides Sybil protection via economic dilution.
/// The VDF is a lightweight anti-flash-attack barrier, not the primary defense.
pub const T_REGISTER_BASE: u64 = 5_000_000;

/// Target registrations per epoch (used for fee calculation)
pub const R_TARGET: u32 = 10;

/// Maximum registrations per epoch (used for fee calculation)
pub const R_CAP: u32 = 100;

/// Maximum registration VDF time — same as base (no escalation)
pub const T_REGISTER_CAP: u64 = 5_000_000;
