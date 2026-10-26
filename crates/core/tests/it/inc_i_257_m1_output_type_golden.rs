//! INC-I-257 M1 — the new output type is appended last: every existing output
//! keeps its bincode bytes, the new variant takes the next position, and any
//! unknown type fails closed.
//!
// covers: transaction types output
//
// OUTPUT CONTRACT: fn bincode::serialize(&Output) -> Vec<u8> (serde-derived, variant POSITION)
//   O1 return bytes for one fixed output of each existing type (golden, pinned on the base commit)
// OUTPUT CONTRACT: fn bincode::deserialize::<Output>(&[u8]) -> Result<Output>
//   O2 return: Ok for positions 0..=14, Err for 15 and above
// OUTPUT CONTRACT: fn OutputType::from_u8(u8) -> Option<OutputType>
//   O3 return: Some for 0..=10, 13..=16; None for 11, 12, 17..=255
// OUTPUT CONTRACT: fn OutputType::is_native_amount / is_conditioned
//   O4 return for Unbonding: native true, conditioned false
// PATHS: P-OLD existing types | P-NEW Unbonding | P-BAD unknown position / byte
// INPUT PARTITIONS:
//   P-OLD: each of the 14 existing variants (positions 0..=13), non-empty extra_data, non-zero lock
//   P-NEW: Unbonding (byte 16, position 14) with empty extra_data and a lock
//   P-BAD: position 15 and u32::MAX; bytes 11, 12 (tombstones), 17, 255
// MATRIX:
//   O1,O2 x P-OLD -> inc_i_257_m1_req_unbond_003_bincode_golden_existing_types
//   O1,O2,O4 x P-NEW -> inc_i_257_m1_req_unbond_003_unbonding_takes_position_14
//   O3 x all -> inc_i_257_m1_req_unbond_003_from_u8_table
//   O2 x P-BAD -> inc_i_257_m1_req_unbond_sec_002_bincode_unknown_position_errors

use crypto::Hash;
use doli_core::transaction::{Output, OutputType};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn fixed_output(output_type: OutputType, i: u8) -> Output {
    Output {
        output_type,
        amount: 1_000 + u64::from(i),
        pubkey_hash: Hash::from_bytes([0x10 + i; 32]),
        lock_until: 7 * u64::from(i),
        extra_data: vec![0xE0, i],
    }
}

// Pinned on the base commit, before the new variant existed.
const GOLDEN: [(OutputType, &str); 14] = [
    (OutputType::Normal, "00000000e8030000000000002000000000000000101010101010101010101010101010101010101010101010101010101010101000000000000000000200000000000000e000"),
    (OutputType::Bond, "01000000e9030000000000002000000000000000111111111111111111111111111111111111111111111111111111111111111107000000000000000200000000000000e001"),
    (OutputType::Multisig, "02000000ea03000000000000200000000000000012121212121212121212121212121212121212121212121212121212121212120e000000000000000200000000000000e002"),
    (OutputType::Hashlock, "03000000eb030000000000002000000000000000131313131313131313131313131313131313131313131313131313131313131315000000000000000200000000000000e003"),
    (OutputType::HTLC, "04000000ec03000000000000200000000000000014141414141414141414141414141414141414141414141414141414141414141c000000000000000200000000000000e004"),
    (OutputType::Vesting, "05000000ed030000000000002000000000000000151515151515151515151515151515151515151515151515151515151515151523000000000000000200000000000000e005"),
    (OutputType::NFT, "06000000ee03000000000000200000000000000016161616161616161616161616161616161616161616161616161616161616162a000000000000000200000000000000e006"),
    (OutputType::FungibleAsset, "07000000ef030000000000002000000000000000171717171717171717171717171717171717171717171717171717171717171731000000000000000200000000000000e007"),
    (OutputType::BridgeHTLC, "08000000f0030000000000002000000000000000181818181818181818181818181818181818181818181818181818181818181838000000000000000200000000000000e008"),
    (OutputType::Pool, "09000000f103000000000000200000000000000019191919191919191919191919191919191919191919191919191919191919193f000000000000000200000000000000e009"),
    (OutputType::LPShare, "0a000000f20300000000000020000000000000001a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a46000000000000000200000000000000e00a"),
    (OutputType::ZKRollup, "0b000000f30300000000000020000000000000001b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b4d000000000000000200000000000000e00b"),
    (OutputType::EncryptedContent, "0c000000f40300000000000020000000000000001c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c54000000000000000200000000000000e00c"),
    (OutputType::OraclePrice, "0d000000f50300000000000020000000000000001d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d5b000000000000000200000000000000e00d"),
];

// REQ-UNBOND-003 — Decision: a failure means the new variant shifted an existing serde position, so every stored row and wire output of that type changes bytes (a chain-wide split).
#[test]
fn inc_i_257_m1_req_unbond_003_bincode_golden_existing_types() {
    for (i, (ty, want)) in GOLDEN.iter().enumerate() {
        let out = fixed_output(*ty, i as u8);
        let bytes = bincode::serialize(&out).expect("serialize");
        assert_eq!(hex(&bytes), *want, "{ty:?}: bincode bytes changed");
        let back: Output = bincode::deserialize(&unhex(want)).expect("golden decodes");
        assert_eq!(
            back, out,
            "{ty:?}: golden bytes decode to a different output"
        );
    }
}

// REQ-UNBOND-003 — Decision: a failure means the new variant was not appended last (wrong serde position) or is not a plain native signature output.
#[test]
fn inc_i_257_m1_req_unbond_003_unbonding_takes_position_14() {
    let out = Output {
        output_type: OutputType::Unbonding,
        amount: 2_000,
        pubkey_hash: Hash::from_bytes([0x2A; 32]),
        lock_until: 60,
        extra_data: Vec::new(),
    };
    let want = format!(
        "0e000000{}2000000000000000{}{}0000000000000000",
        hex(&2_000u64.to_le_bytes()),
        "2a".repeat(32),
        hex(&60u64.to_le_bytes()),
    );
    let bytes = bincode::serialize(&out).expect("serialize");
    assert_eq!(
        hex(&bytes),
        want,
        "Unbonding must encode at serde position 14"
    );
    let back: Output = bincode::deserialize(&bytes).expect("position 14 decodes");
    assert_eq!(back, out);
    assert_eq!(OutputType::Unbonding as u8, 16, "discriminant");
    assert!(OutputType::Unbonding.is_native_amount(), "native amount");
    assert!(!OutputType::Unbonding.is_conditioned(), "signature-spent");
}

// REQ-UNBOND-003 — Decision: a failure means a tombstoned or unassigned type byte decodes, or the new byte does not.
#[test]
fn inc_i_257_m1_req_unbond_003_from_u8_table() {
    for (i, (ty, _)) in GOLDEN.iter().enumerate() {
        assert_eq!(OutputType::from_u8(*ty as u8), Some(*ty), "existing #{i}");
    }
    assert_eq!(OutputType::from_u8(16), Some(OutputType::Unbonding));
    for b in [11u8, 12, 17, 18, 200, 255] {
        assert_eq!(OutputType::from_u8(b), None, "byte {b} must not decode");
    }
    let some: Vec<u8> = (0..=255u8)
        .filter(|b| OutputType::from_u8(*b).is_some())
        .collect();
    let want: Vec<u8> = (0..=10).chain(13..=16).collect();
    assert_eq!(some, want, "exactly these type bytes decode");
}

// REQ-UNBOND-SEC-002 — Decision: a failure means a row or wire output with an unknown variant position decodes instead of failing closed.
#[test]
fn inc_i_257_m1_req_unbond_sec_002_bincode_unknown_position_errors() {
    let base = unhex(GOLDEN[0].1);
    for pos in [15u32, 16, u32::MAX] {
        let mut bytes = base.clone();
        bytes[..4].copy_from_slice(&pos.to_le_bytes());
        assert!(
            bincode::deserialize::<Output>(&bytes).is_err(),
            "variant position {pos} must not decode"
        );
    }
    let mut bytes = base;
    bytes[..4].copy_from_slice(&14u32.to_le_bytes());
    let got: Output = bincode::deserialize(&bytes).expect("position 14 decodes");
    assert_eq!(got.output_type, OutputType::Unbonding);
}
