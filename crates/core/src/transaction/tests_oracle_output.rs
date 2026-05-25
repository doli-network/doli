//! Tests for `OutputType::OraclePrice` (=15) — Phase 2.1 Oracle M5.
//!
//! Spec: `specs/oracle-structural-anchored-economics.md` §1.2.
//!
//! M5 scope (this test file):
//!   - `OutputType::OraclePrice` round-trips through `from_u8(15)` /
//!     discriminant `as u8`.
//!   - `Output::oracle_price(...)` constructor produces the right
//!     50-byte extra_data layout, amount=0, lock_until=0, and the
//!     `pubkey_hash` matches `oracle_price_address(pair_id)`.
//!   - `Output::oracle_price_address(pair_id)` is deterministic and
//!     domain-separated (different pair_id -> different address;
//!     same pair_id under a different domain prefix -> different
//!     address).
//!   - `parse_oracle_price(&output)` round-trips: construct -> parse
//!     yields the four input fields. Wrong output_type -> None.
//!     Wrong length -> None.
//!   - `is_native_amount(OraclePrice) == false` (price_cents is NOT
//!     DOLI; summing it as DOLI would corrupt supply accounting).
//!   - `is_conditioned(OraclePrice) == false` (system-spent only;
//!     no condition prefix in extra_data).
//!   - Canonical (state-root) serialization round-trip via
//!     `UtxoEntry::serialize_canonical_bytes` is left to a storage
//!     crate test — M5 spec verification is satisfied by confirming
//!     `output_type as u8 == 15` lands in the canonical encoding
//!     unchanged (the encoder writes `output.output_type as u8`
//!     verbatim — see `crates/storage/src/utxo/types.rs:60`).

use crate::transaction::{Output, OutputType};
use crypto::Hash;

// OUTPUT CONTRACT: fn OutputType::from_u8 — OraclePrice discriminant
//   O1: return — Some(OutputType::OraclePrice) for input 15
//   O2: return — discriminant `as u8` round-trips: 15 -> OraclePrice -> 15
// PATHS:
//   P1: input = 15 -> matches OraclePrice arm
//   P2: OutputType::OraclePrice as u8 -> 15
// INPUT PARTITIONS:
//   part-A (P1): the M5 discriminant 15 — exactly one value
//   part-B (P2): the variant itself — exactly one value
// MATRIX: 2 outputs × 2 paths × 2 partitions = sparse
//   P1×part-A: O1✓     P2×part-B: O2✓
#[test]
fn test_oracle_price_discriminant_round_trips() {
    // P1
    assert_eq!(
        OutputType::from_u8(15),
        Some(OutputType::OraclePrice),
        "OutputType::from_u8(15) must return Some(OraclePrice)"
    ); // O1
       // P2
    assert_eq!(
        OutputType::OraclePrice as u8,
        15,
        "OutputType::OraclePrice as u8 must equal 15"
    ); // O2
}

// OUTPUT CONTRACT: fn Output::oracle_price — constructor shape
//   O1: return.output_type — OutputType::OraclePrice
//   O2: return.amount — 0 (spec §1.2: no DOLI locked)
//   O3: return.lock_until — 0 (system-spent only)
//   O4: return.pubkey_hash — Output::oracle_price_address(&pair_id)
//   O5: return.extra_data.len() — 50 (= ORACLE_PRICE_EXTRA_DATA_SIZE)
//   O6: return.extra_data layout — [price_cents (8 LE) ||
//                                   last_update_height (8 LE) ||
//                                   contributor_count (2 LE) ||
//                                   pair_id (32)]
// PATHS:
//   P1: constructor with typical inputs
// INPUT PARTITIONS:
//   part-A (P1): pair_id=[0x11;32], price_cents=12345,
//                last_update_height=720, contributor_count=12
// MATRIX: 6 outputs × 1 path × 1 partition = 6 cells
//   P1×part-A: O1✓ O2✓ O3✓ O4✓ O5✓ O6✓
#[test]
fn test_oracle_price_constructor_shape() {
    let pair_id = Hash::from_bytes([0x11; 32]);
    let out = Output::oracle_price(pair_id, 12_345, 720, 12);

    assert_eq!(out.output_type, OutputType::OraclePrice); // O1
    assert_eq!(out.amount, 0); // O2
    assert_eq!(out.lock_until, 0); // O3
    assert_eq!(out.pubkey_hash, Output::oracle_price_address(&pair_id)); // O4
    assert_eq!(out.extra_data.len(), Output::ORACLE_PRICE_EXTRA_DATA_SIZE); // O5
    assert_eq!(Output::ORACLE_PRICE_EXTRA_DATA_SIZE, 50);

    // O6 — verify byte layout explicitly.
    assert_eq!(
        u64::from_le_bytes(out.extra_data[0..8].try_into().unwrap()),
        12_345
    );
    assert_eq!(
        u64::from_le_bytes(out.extra_data[8..16].try_into().unwrap()),
        720
    );
    assert_eq!(
        u16::from_le_bytes(out.extra_data[16..18].try_into().unwrap()),
        12
    );
    assert_eq!(&out.extra_data[18..50], pair_id.as_bytes());
}

// OUTPUT CONTRACT: fn Output::oracle_price_address — address derivation
//   O1: same pair_id -> same address (deterministic)
//   O2: different pair_id -> different address (collision-resistant)
//   O3: domain-separated from REWARD_POOL pool address
//        (`hash_with_domain(b"ORACLE_PRICE", pair_id)` !=
//         `hash_with_domain(b"REWARD_POOL", pair_id)`)
// PATHS:
//   P1: two calls with the same pair_id
//   P2: two calls with different pair_ids
//   P3: same 32-byte input under ORACLE_PRICE vs REWARD_POOL domain
// INPUT PARTITIONS:
//   part-A (P1): pair_id = [0x42;32]
//   part-B (P2): pair_id_a = [0x11;32], pair_id_b = [0x99;32]
//   part-C (P3): same 32-byte input, domain prefix differs
// MATRIX: 3 outputs × 3 paths × 3 partitions = sparse
//   P1×part-A: O1✓     P2×part-B: O2✓     P3×part-C: O3✓
#[test]
fn test_oracle_price_address_is_deterministic_and_collision_resistant() {
    let pair_id = Hash::from_bytes([0x42; 32]);

    // P1×part-A: determinism
    let addr_a = Output::oracle_price_address(&pair_id);
    let addr_b = Output::oracle_price_address(&pair_id);
    assert_eq!(addr_a, addr_b); // O1

    // P2×part-B: different pair_ids -> different addresses
    let pair_a = Hash::from_bytes([0x11; 32]);
    let pair_b = Hash::from_bytes([0x99; 32]);
    assert_ne!(
        Output::oracle_price_address(&pair_a),
        Output::oracle_price_address(&pair_b)
    ); // O2
}

#[test]
fn test_oracle_price_address_is_domain_separated_from_reward_pool() {
    // Same 32-byte input under different domain prefixes must produce
    // different hashes. We don't directly call `reward_pool_address`
    // (which uses b"doli" not a 32-byte input), but we can verify the
    // domain separation property by computing the ORACLE_PRICE address
    // ourselves and comparing against an alternate-domain hash of the
    // same bytes.
    let same_bytes = [0x55; 32];
    let oracle_addr = Output::oracle_price_address(&Hash::from_bytes(same_bytes));
    let alt_addr = crypto::hash::hash_with_domain(b"REWARD_POOL", &same_bytes);
    assert_ne!(
        oracle_addr, alt_addr,
        "ORACLE_PRICE and REWARD_POOL domains must collision-resist on same input"
    ); // O3
}

// OUTPUT CONTRACT: fn Output::parse_oracle_price — inverse of constructor
//   O1: return — Some((price_cents, last_update_height,
//                      contributor_count, pair_id)) when output_type
//                is OraclePrice and extra_data is exactly 50 bytes
//   O2: return — None when output_type is not OraclePrice
//   O3: return — None when extra_data length is not 50
// PATHS:
//   P1: round-trip constructor -> parse
//   P2: parse on a Normal output (wrong output_type)
//   P3: parse on an OraclePrice with truncated extra_data
// INPUT PARTITIONS:
//   part-A (P1): pair_id=[0xAA;32], price=9999, height=1080, count=8
//   part-B (P2): Normal output with otherwise-valid 50-byte data
//   part-C (P3): OraclePrice with 49-byte extra_data (truncated)
// MATRIX: sparse
//   P1×part-A: O1✓     P2×part-B: O2✓     P3×part-C: O3✓
#[test]
fn test_oracle_price_parse_round_trips() {
    let pair_id = Hash::from_bytes([0xAA; 32]);
    let out = Output::oracle_price(pair_id, 9_999, 1_080, 8);
    let parsed = out.parse_oracle_price();
    assert_eq!(parsed, Some((9_999u64, 1_080u64, 8u16, pair_id))); // O1
}

#[test]
fn test_oracle_price_parse_rejects_wrong_output_type() {
    let mut out = Output::oracle_price(Hash::from_bytes([0xAA; 32]), 1, 2, 3);
    // Manually flip the output_type while keeping the same extra_data.
    out.output_type = OutputType::Normal;
    assert_eq!(out.parse_oracle_price(), None); // O2
}

#[test]
fn test_oracle_price_parse_rejects_truncated_extra_data() {
    let mut out = Output::oracle_price(Hash::from_bytes([0xAA; 32]), 1, 2, 3);
    out.extra_data.truncate(49);
    assert_eq!(out.parse_oracle_price(), None); // O3
}

// OUTPUT CONTRACT: fn OutputType::{is_native_amount, is_conditioned}
//   O1: is_native_amount(OraclePrice) -> false (price_cents is NOT DOLI)
//   O2: is_conditioned(OraclePrice) -> false (system-spent, no condition)
// PATHS / PARTITIONS: trivial — single variant, two predicates.
// MATRIX:
//   OraclePrice × is_native_amount: O1✓
//   OraclePrice × is_conditioned:   O2✓
#[test]
fn test_oracle_price_is_not_native_amount() {
    assert!(!OutputType::OraclePrice.is_native_amount()); // O1
}

#[test]
fn test_oracle_price_is_not_conditioned() {
    assert!(!OutputType::OraclePrice.is_conditioned()); // O2
}
