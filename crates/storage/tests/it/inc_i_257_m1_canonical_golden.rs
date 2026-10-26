//! INC-I-257 M1 — the canonical UTXO bytes (state root) and the bincode row
//! bytes (RocksDB) of every existing output type are unchanged by the new type;
//! the new type byte 16 round-trips; an unknown type byte fails closed.
//!
// covers: utxo types canonical
//
// OUTPUT CONTRACT: fn UtxoEntry::serialize_canonical_bytes(&self) -> Vec<u8>
//   O1 return bytes (golden per existing type; byte 0 = 16 for the new type)
// OUTPUT CONTRACT: fn bincode::serialize(&UtxoEntry) -> Vec<u8> (RocksDB row)
//   O2 return bytes (golden per existing type; serde position 14 for the new type)
// OUTPUT CONTRACT: fn UtxoEntry::deserialize_canonical_bytes(&[u8]) -> Option<UtxoEntry>
//   O3 return: Some(entry) for known bytes, None for 11, 12, 17, 255
// PATHS: P-OLD existing types | P-NEW type byte 16 | P-BAD unknown type byte
// INPUT PARTITIONS:
//   P-OLD: 14 existing types; coinbase and epoch-reward flags vary; non-empty extra_data
//   P-NEW: Unbonding, empty extra_data, lock set
//   P-BAD: tombstones 11, 12; unassigned 17, 255
// MATRIX:
//   O1,O2,O3 x P-OLD -> inc_i_257_m1_req_unbond_003_canonical_and_row_golden_existing_types
//   O1,O2,O3 x P-NEW -> inc_i_257_m1_req_unbond_003_type_byte_16_round_trips
//   O3 x P-BAD -> inc_i_257_m1_req_unbond_sec_002_canonical_unknown_type_fails_closed

use crypto::Hash;
use doli_core::transaction::{Output, OutputType};
use storage::UtxoEntry;

const UNBONDING_TYPE_BYTE: u8 = 16;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn entry(ty: OutputType, i: u8) -> UtxoEntry {
    UtxoEntry {
        output: Output {
            output_type: ty,
            amount: 1_000 + u64::from(i),
            pubkey_hash: Hash::from_bytes([0x10 + i; 32]),
            lock_until: 7 * u64::from(i),
            extra_data: vec![0xE0, i],
        },
        height: 100 + u64::from(i),
        is_coinbase: i % 2 == 1,
        is_epoch_reward: i % 3 == 2,
    }
}

// Pinned on the base commit: (type byte, canonical bytes, bincode row bytes).
const GOLDEN: [(u8, &str, &str); 14] = [
    (
        0,
        "00e80300000000000010101010101010101010101010101010101010101010101010101010101010100000000000000000640000000000000000000200e000",
        "00000000e8030000000000002000000000000000101010101010101010101010101010101010101010101010101010101010101000000000000000000200000000000000e00064000000000000000000",
    ),
    (
        1,
        "01e90300000000000011111111111111111111111111111111111111111111111111111111111111110700000000000000650000000000000001000200e001",
        "01000000e9030000000000002000000000000000111111111111111111111111111111111111111111111111111111111111111107000000000000000200000000000000e00165000000000000000100",
    ),
    (
        2,
        "02ea0300000000000012121212121212121212121212121212121212121212121212121212121212120e00000000000000660000000000000000010200e002",
        "02000000ea03000000000000200000000000000012121212121212121212121212121212121212121212121212121212121212120e000000000000000200000000000000e00266000000000000000001",
    ),
    (
        3,
        "03eb0300000000000013131313131313131313131313131313131313131313131313131313131313131500000000000000670000000000000001000200e003",
        "03000000eb030000000000002000000000000000131313131313131313131313131313131313131313131313131313131313131315000000000000000200000000000000e00367000000000000000100",
    ),
    (
        4,
        "04ec0300000000000014141414141414141414141414141414141414141414141414141414141414141c00000000000000680000000000000000000200e004",
        "04000000ec03000000000000200000000000000014141414141414141414141414141414141414141414141414141414141414141c000000000000000200000000000000e00468000000000000000000",
    ),
    (
        5,
        "05ed0300000000000015151515151515151515151515151515151515151515151515151515151515152300000000000000690000000000000001010200e005",
        "05000000ed030000000000002000000000000000151515151515151515151515151515151515151515151515151515151515151523000000000000000200000000000000e00569000000000000000101",
    ),
    (
        6,
        "06ee0300000000000016161616161616161616161616161616161616161616161616161616161616162a000000000000006a0000000000000000000200e006",
        "06000000ee03000000000000200000000000000016161616161616161616161616161616161616161616161616161616161616162a000000000000000200000000000000e0066a000000000000000000",
    ),
    (
        7,
        "07ef03000000000000171717171717171717171717171717171717171717171717171717171717171731000000000000006b0000000000000001000200e007",
        "07000000ef030000000000002000000000000000171717171717171717171717171717171717171717171717171717171717171731000000000000000200000000000000e0076b000000000000000100",
    ),
    (
        8,
        "08f003000000000000181818181818181818181818181818181818181818181818181818181818181838000000000000006c0000000000000000010200e008",
        "08000000f0030000000000002000000000000000181818181818181818181818181818181818181818181818181818181818181838000000000000000200000000000000e0086c000000000000000001",
    ),
    (
        9,
        "09f10300000000000019191919191919191919191919191919191919191919191919191919191919193f000000000000006d0000000000000001000200e009",
        "09000000f103000000000000200000000000000019191919191919191919191919191919191919191919191919191919191919193f000000000000000200000000000000e0096d000000000000000100",
    ),
    (
        10,
        "0af2030000000000001a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a46000000000000006e0000000000000000000200e00a",
        "0a000000f20300000000000020000000000000001a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a46000000000000000200000000000000e00a6e000000000000000000",
    ),
    (
        13,
        "0df5030000000000001d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d5b00000000000000710000000000000001000200e00d",
        "0b000000f50300000000000020000000000000001d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d5b000000000000000200000000000000e00d71000000000000000100",
    ),
    (
        14,
        "0ef6030000000000001e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e6200000000000000720000000000000000010200e00e",
        "0c000000f60300000000000020000000000000001e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e62000000000000000200000000000000e00e72000000000000000001",
    ),
    (
        15,
        "0ff7030000000000001f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f6900000000000000730000000000000001000200e00f",
        "0d000000f70300000000000020000000000000001f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f69000000000000000200000000000000e00f73000000000000000100",
    ),
];

// REQ-UNBOND-003 — Decision: a failure means the new type changed the canonical or row bytes of an existing UTXO, so utxo_hash and the state root of every node change (a chain reset).
#[test]
fn inc_i_257_m1_req_unbond_003_canonical_and_row_golden_existing_types() {
    for (b, canon, row) in GOLDEN {
        let ty = OutputType::from_u8(b).expect("existing type byte decodes");
        let e = entry(ty, b);
        assert_eq!(
            hex(&e.serialize_canonical_bytes()),
            canon,
            "{ty:?}: canonical bytes changed"
        );
        assert_eq!(
            hex(&bincode::serialize(&e).expect("row")),
            row,
            "{ty:?}: row bytes changed"
        );
        assert_eq!(
            UtxoEntry::deserialize_canonical_bytes(&unhex(canon)),
            Some(e.clone()),
            "{ty:?}: canonical golden does not decode to the same entry"
        );
        let back: UtxoEntry = bincode::deserialize(&unhex(row)).expect("row golden decodes");
        assert_eq!(back, e, "{ty:?}: row golden decodes to a different entry");
    }
}

// REQ-UNBOND-003 — Decision: a failure means an Unbonding UTXO cannot travel in snap sync or be stored, or is stored at a shifted serde position.
#[test]
fn inc_i_257_m1_req_unbond_003_type_byte_16_round_trips() {
    let ty = OutputType::from_u8(UNBONDING_TYPE_BYTE)
        .expect("PRECONDITION: type byte 16 (Unbonding) must decode");
    let e = UtxoEntry {
        output: Output {
            output_type: ty,
            amount: 2_000,
            pubkey_hash: Hash::from_bytes([0x2A; 32]),
            lock_until: 60,
            extra_data: Vec::new(),
        },
        height: 5,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    let canon = e.serialize_canonical_bytes();
    assert_eq!(canon[0], UNBONDING_TYPE_BYTE, "canonical type byte");
    assert_eq!(
        canon.len(),
        61,
        "fixed canonical size with empty extra_data"
    );
    assert_eq!(
        UtxoEntry::deserialize_canonical_bytes(&canon),
        Some(e.clone())
    );
    let row = bincode::serialize(&e).expect("row");
    assert_eq!(&row[..4], &14u32.to_le_bytes(), "row serde position 14");
    let back: UtxoEntry = bincode::deserialize(&row).expect("row decodes");
    assert_eq!(back, e);
}

// REQ-UNBOND-SEC-002 — Decision: a failure means a canonical entry with an unknown type byte decodes (AP-7: a skipped or guessed entry yields a different but valid-looking utxo_hash).
#[test]
fn inc_i_257_m1_req_unbond_sec_002_canonical_unknown_type_fails_closed() {
    let base = unhex(GOLDEN[0].1);
    assert!(
        UtxoEntry::deserialize_canonical_bytes(&base).is_some(),
        "NON-VACUITY"
    );
    for b in [11u8, 12, 17, 255] {
        let mut bytes = base.clone();
        bytes[0] = b;
        assert_eq!(
            UtxoEntry::deserialize_canonical_bytes(&bytes),
            None,
            "canonical type byte {b} must fail closed"
        );
    }
}
