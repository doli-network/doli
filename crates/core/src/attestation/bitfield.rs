//! On-chain attestation bitfield codecs and the minute helpers.
//!
//! Each block producer commits which producers attested the current minute.
//! Bit N = 1 means the producer at index N (sorted by pubkey, same order as
//! `DeterministicScheduler`) sent an attestation for that minute. The header
//! variants pack the bits into the 32-byte `presence_root` (256-producer cap);
//! the `_vec` variants store them in the block body with no cap.

use crypto::Hash;

/// Slots per attestation minute (6 slots × 10s = 60s).
pub const SLOTS_PER_ATTESTATION_MINUTE: u32 = 6;

/// Attestation minutes per epoch (360 slots / 6 = 60 minutes) — mainnet default.
pub const ATTESTATION_MINUTES_PER_EPOCH: u32 = 60;

/// Attestation qualification threshold: 90% of 60 = 54 minutes — mainnet default.
pub const ATTESTATION_QUALIFICATION_THRESHOLD: u32 = 54;

/// Compute attestation minutes per epoch from blocks_per_epoch.
/// For mainnet (360): 360/6 = 60. For testnet (36): 36/6 = 6.
#[inline]
pub fn attestation_minutes_per_epoch(blocks_per_epoch: u64) -> u32 {
    (blocks_per_epoch as u32) / SLOTS_PER_ATTESTATION_MINUTE
}

/// Compute attestation qualification threshold (90%) from blocks_per_epoch.
/// For mainnet (360): 90% of 60 = 54. For testnet (36): 90% of 6 = 5.
#[inline]
pub fn attestation_qualification_threshold(blocks_per_epoch: u64) -> u32 {
    let minutes = attestation_minutes_per_epoch(blocks_per_epoch);
    (minutes * 90) / 100
}

/// Compute the attestation minute from a slot number.
///
/// Each minute covers 6 slots (60 seconds at 10s/slot).
/// Deterministic: all nodes compute the same minute from the same slot.
#[inline]
pub fn attestation_minute(slot: u32) -> u32 {
    slot / SLOTS_PER_ATTESTATION_MINUTE
}

/// Encode an attestation bitfield into a Hash for `presence_root`.
///
/// `attested_indices` are indices into the sorted producer list.
/// Bit N = 1 means producer at index N attested the current minute.
/// Supports up to 256 producers (32 bytes × 8 bits).
pub fn encode_attestation_bitfield(attested_indices: &[usize]) -> Hash {
    let mut bytes = [0u8; 32];
    for &idx in attested_indices {
        if idx < 256 {
            bytes[idx / 8] |= 1 << (idx % 8);
        }
    }
    Hash::from_bytes(bytes)
}

/// Encode an attestation bitfield into a Vec<u8> (no 256-producer cap).
///
/// Used for body-stored bitfields, where `presence_root` carries the BLAKE3
/// commitment instead of the raw bits.
///
/// `attested_indices` are indices into the sorted producer list.
/// `producer_count` determines the length of the output (ceil(producer_count / 8) bytes).
pub fn encode_attestation_bitfield_vec(
    attested_indices: &[usize],
    producer_count: usize,
) -> Vec<u8> {
    let byte_count = producer_count.div_ceil(8);
    let mut bytes = vec![0u8; byte_count];
    for &idx in attested_indices {
        if idx < producer_count {
            bytes[idx / 8] |= 1 << (idx % 8);
        }
    }
    bytes
}

/// Decode attestation bitfield from a Vec<u8> (no 256-producer cap).
///
/// Used for body-stored bitfields. Returns indices of producers that attested.
/// `producer_count` limits the scan range.
pub fn decode_attestation_bitfield_vec(bitfield: &[u8], producer_count: usize) -> Vec<usize> {
    let mut indices = Vec::new();
    for idx in 0..producer_count {
        let byte_idx = idx / 8;
        if byte_idx >= bitfield.len() {
            break;
        }
        if bitfield[byte_idx] & (1 << (idx % 8)) != 0 {
            indices.push(idx);
        }
    }
    indices
}

/// Validate a body attestation bitfield: no bits set beyond `producer_count`.
pub fn validate_attestation_bitfield_vec(bitfield: &[u8], producer_count: usize) -> bool {
    let expected_bytes = producer_count.div_ceil(8);
    // Extra bytes beyond expected must be zero
    for b in bitfield.iter().skip(expected_bytes) {
        if *b != 0 {
            return false;
        }
    }
    // Stray bits in the last expected byte
    let remainder = producer_count % 8;
    if remainder > 0 && expected_bytes <= bitfield.len() {
        let mask = !((1u8 << remainder) - 1); // bits above remainder
        if bitfield[expected_bytes - 1] & mask != 0 {
            return false;
        }
    }
    true
}

/// Decode attestation bitfield from `presence_root`.
///
/// Returns indices of producers that attested this minute.
/// `producer_count` limits the scan range.
pub fn decode_attestation_bitfield(presence_root: &Hash, producer_count: usize) -> Vec<usize> {
    let bytes = presence_root.as_bytes();
    let max = producer_count.min(256);
    let mut indices = Vec::new();
    for idx in 0..max {
        if bytes[idx / 8] & (1 << (idx % 8)) != 0 {
            indices.push(idx);
        }
    }
    indices
}

/// Validate that a presence_root bitfield has no bits set beyond `producer_count`.
///
/// Returns false if any stray bits are set (potential manipulation).
pub fn validate_attestation_bitfield(presence_root: &Hash, producer_count: usize) -> bool {
    if producer_count >= 256 {
        return true; // All bits valid
    }
    let bytes = presence_root.as_bytes();
    for idx in producer_count..256 {
        if bytes[idx / 8] & (1 << (idx % 8)) != 0 {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod bitfield_tests {
    use super::*;

    #[test]
    fn test_attestation_minute() {
        assert_eq!(attestation_minute(0), 0);
        assert_eq!(attestation_minute(5), 0);
        assert_eq!(attestation_minute(6), 1);
        assert_eq!(attestation_minute(11), 1);
        assert_eq!(attestation_minute(12), 2);
        assert_eq!(attestation_minute(359), 59);
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let indices = vec![0, 3, 7, 11];
        let hash = encode_attestation_bitfield(&indices);
        let decoded = decode_attestation_bitfield(&hash, 12);
        assert_eq!(decoded, indices);
    }

    #[test]
    fn test_encode_empty() {
        let hash = encode_attestation_bitfield(&[]);
        assert!(hash.is_zero());
    }

    #[test]
    fn test_decode_zero_is_empty() {
        let decoded = decode_attestation_bitfield(&Hash::ZERO, 12);
        assert!(decoded.is_empty());
    }

    #[test]
    fn test_encode_all_producers() {
        let indices: Vec<usize> = (0..12).collect();
        let hash = encode_attestation_bitfield(&indices);
        let decoded = decode_attestation_bitfield(&hash, 12);
        assert_eq!(decoded, indices);
    }

    #[test]
    fn test_validate_bitfield_clean() {
        let indices = vec![0, 3, 7];
        let hash = encode_attestation_bitfield(&indices);
        assert!(validate_attestation_bitfield(&hash, 12));
    }

    #[test]
    fn test_validate_bitfield_stray_bit() {
        // Set bit 15 but claim only 12 producers
        let hash = encode_attestation_bitfield(&[0, 15]);
        assert!(!validate_attestation_bitfield(&hash, 12));
    }

    #[test]
    fn test_validate_bitfield_256_producers() {
        let hash = encode_attestation_bitfield(&[255]);
        assert!(validate_attestation_bitfield(&hash, 256));
    }

    #[test]
    fn test_constants() {
        assert_eq!(SLOTS_PER_ATTESTATION_MINUTE, 6);
        assert_eq!(ATTESTATION_MINUTES_PER_EPOCH, 60);
        assert_eq!(ATTESTATION_QUALIFICATION_THRESHOLD, 54);
    }
}
