//! Golden vectors freezing the TWO independent `TxType` numbering surfaces.
//!
//! covers: REQ-ROT-001
//!
//! Surface 1 — bincode variant index ("ordinal"): the DECLARATION POSITION.
//! Surface 2 — `#[repr(u32)]` discriminant: the `= N` literal, hashed into
//! `signing_message` and decoded by `TxType::from_u32`.
//! They alias only for 0..=22; `ZKSettle` is ordinal 23 / discriminant 31.

use crypto::{Hash, KeyPair, Signature};

use super::*;

/// `(variant, bincode ordinal, repr(u32) discriminant)` for all 24 live variants.
const WIRE_GOLDEN: [(TxType, u32, u32); 24] = [
    (TxType::Transfer, 0, 0),
    (TxType::Registration, 1, 1),
    (TxType::Exit, 2, 2),
    (TxType::ClaimReward, 3, 3),
    (TxType::ClaimBond, 4, 4),
    (TxType::SlashProducer, 5, 5),
    (TxType::Coinbase, 6, 6),
    (TxType::AddBond, 7, 7),
    (TxType::RequestWithdrawal, 8, 8),
    (TxType::ClaimWithdrawal, 9, 9),
    (TxType::EpochReward, 10, 10),
    (TxType::RemoveMaintainer, 11, 11),
    (TxType::AddMaintainer, 12, 12),
    (TxType::DelegateBond, 13, 13),
    (TxType::RevokeDelegation, 14, 14),
    (TxType::ProtocolActivation, 15, 15),
    (TxType::PriceAttestation, 16, 16),
    (TxType::MintAsset, 17, 17),
    (TxType::BurnAsset, 18, 18),
    (TxType::CreatePool, 19, 19),
    (TxType::AddLiquidity, 20, 20),
    (TxType::RemoveLiquidity, 21, 21),
    (TxType::Swap, 22, 22),
    (TxType::ZKSettle, 23, 31),
];

/// Deterministic transaction: fixed seed, fixed hashes, fixed signature bytes.
fn fixture_tx(tx_type: TxType) -> Transaction {
    let kp = KeyPair::from_seed([7u8; 32]);
    Transaction {
        version: 1,
        tx_type,
        inputs: vec![Input {
            prev_tx_hash: Hash::from_bytes([0xA1u8; 32]),
            output_index: 3,
            signature: Signature::from_bytes([0x5Cu8; 64]),
            sighash_type: SighashType::All,
            committed_output_count: 0,
            public_key: Some(*kp.public_key()),
        }],
        outputs: vec![Output {
            output_type: OutputType::Normal,
            amount: 1_234_567,
            pubkey_hash: Hash::from_bytes([0xB2u8; 32]),
            lock_until: 0,
            extra_data: Vec::new(),
        }],
        extra_data: vec![0xEE, 0xEE, 0xEE, 0xEE],
    }
}

fn bincode_ordinal(t: TxType) -> u32 {
    let bytes = bincode::serialize(&t).expect("TxType serializes");
    assert_eq!(bytes.len(), 4, "TxType must encode as exactly 4 bytes");
    u32::from_le_bytes(bytes[..4].try_into().unwrap())
}

// REQ-ROT-001 — Decision: whether the ordinals in WIRE_GOLDEN may be read as real wire bytes or are merely an assumption restated.
#[test]
fn bare_txtype_bincode_encoding_is_a_u32_little_endian_variant_index() {
    assert_eq!(
        bincode::serialize(&TxType::Transfer).unwrap(),
        vec![0, 0, 0, 0]
    );
    assert_eq!(
        bincode::serialize(&TxType::Registration).unwrap(),
        vec![1, 0, 0, 0]
    );
    assert_eq!(bincode::serialize(&TxType::Exit).unwrap(), vec![2, 0, 0, 0]);

    let last = bincode::serialize(&TxType::ZKSettle).unwrap();
    assert_eq!(last.len(), 4);
    assert_eq!(
        u32::from_le_bytes(last[..4].try_into().unwrap()),
        23,
        "ZKSettle is declared 24th, so its bincode index is 23 — NOT its discriminant 31"
    );
}

// REQ-ROT-001 — Decision: whether a reordered or renumbered variant changed what peers decode or what signatures commit to.
#[test]
fn txtype_golden_table_pins_both_numbering_surfaces() {
    for (variant, ordinal, discriminant) in WIRE_GOLDEN {
        assert_eq!(
            variant as u32, discriminant,
            "repr(u32) discriminant drifted for {variant:?}"
        );
        assert_eq!(
            TxType::from_u32(discriminant),
            Some(variant),
            "from_u32({discriminant}) no longer returns {variant:?}"
        );
        assert_eq!(
            bincode_ordinal(variant),
            ordinal,
            "bincode variant index drifted for {variant:?}"
        );
    }
}

// REQ-ROT-001 — Decision: whether a variant was inserted or removed in the middle of the declaration list, shifting every later ordinal.
#[test]
fn txtype_ordinals_are_the_contiguous_declaration_positions_zero_to_23() {
    let ordinals: Vec<u32> = WIRE_GOLDEN
        .iter()
        .map(|(v, _, _)| bincode_ordinal(*v))
        .collect();
    let expected: Vec<u32> = (0..24).collect();
    assert_eq!(
        ordinals, expected,
        "declaration order changed: ordinals must be 0..=23 in table order"
    );
}

// REQ-ROT-001 — Decision: whether the tx_type field still sits where the wallet and every peer decoder expect it.
#[test]
fn transaction_wire_layout_places_tx_type_at_byte_offset_four() {
    let mut tx = fixture_tx(TxType::Transfer);
    tx.version = 0x1122_3344;

    let bytes = bincode::serialize(&tx).unwrap();
    assert_eq!(&bytes[0..4], &0x1122_3344u32.to_le_bytes());
    assert_eq!(&bytes[4..8], &0u32.to_le_bytes());

    tx.tx_type = TxType::ZKSettle;
    let bytes = bincode::serialize(&tx).unwrap();
    assert_eq!(&bytes[0..4], &0x1122_3344u32.to_le_bytes());
    assert_eq!(
        &bytes[4..8],
        &23u32.to_le_bytes(),
        "the wire slot carries the ORDINAL, not the discriminant"
    );
}

// REQ-ROT-001 — Decision: whether a peer decoding a serialized Transaction would now resolve a different variant than the sender meant.
#[test]
fn every_txtype_round_trips_through_real_transaction_bytes() {
    for (variant, ordinal, _discriminant) in WIRE_GOLDEN {
        let tx = fixture_tx(variant);
        let bytes = bincode::serialize(&tx).expect("Transaction serializes");

        assert_eq!(&bytes[0..4], &1u32.to_le_bytes(), "version slot moved");
        assert_eq!(
            u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            ordinal,
            "on-wire tx_type ordinal drifted for {variant:?}"
        );

        let decoded: Transaction = bincode::deserialize(&bytes).expect("Transaction round-trips");
        assert_eq!(decoded.tx_type, variant, "decoded variant drifted");
        assert_eq!(decoded.version, tx.version);
        assert_eq!(decoded.inputs.len(), 1);
        assert_eq!(decoded.outputs.len(), 1);
        assert_eq!(decoded.extra_data, tx.extra_data);
    }
}

// REQ-ROT-001 — Decision: whether discriminant 23 (the permanent tombstone) or 32 (the number M4 claims) was quietly handed to a live variant.
#[test]
fn tombstoned_and_unassigned_discriminants_stay_undecodable() {
    assert_eq!(
        TxType::from_u32(23),
        None,
        "23 is a PERMANENT tombstone: it is ZKSettle's ordinal, never a discriminant"
    );
    for v in 24..=30u32 {
        assert_eq!(TxType::from_u32(v), None, "discriminant {v} is tombstoned");
    }
    assert_eq!(TxType::from_u32(31), Some(TxType::ZKSettle));

    // M4 TRIPWIRE: M4 gives RotateBlsKey discriminant 32 and MUST invert this line.
    assert_eq!(TxType::from_u32(32), None);

    for v in 33..=255u32 {
        assert_eq!(
            TxType::from_u32(v),
            None,
            "discriminant {v} must be unassigned"
        );
    }
    assert_eq!(TxType::from_u32(u32::MAX), None);
}

// REQ-ROT-001 — Decision: whether a 25th variant became decodable on the wire before M4 deliberately added one.
#[test]
fn wire_ordinal_24_is_not_decodable() {
    let tx = fixture_tx(TxType::Transfer);
    let mut bytes = bincode::serialize(&tx).unwrap();

    // Non-vacuity: patching the same slot to a LIVE ordinal must still decode.
    bytes[4..8].copy_from_slice(&23u32.to_le_bytes());
    let decoded: Transaction = bincode::deserialize(&bytes).expect("ordinal 23 decodes");
    assert_eq!(decoded.tx_type, TxType::ZKSettle);

    // M4 TRIPWIRE: M4 declares RotateBlsKey last, making ordinal 24 decodable.
    bytes[4..8].copy_from_slice(&24u32.to_le_bytes());
    assert!(
        bincode::deserialize::<Transaction>(&bytes).is_err(),
        "wire ordinal 24 must be undecodable until M4 declares a 25th variant"
    );
}

// REQ-ROT-001 — Decision: whether a variant was added or removed without the numbering freeze being revisited.
#[test]
fn exactly_24_discriminants_are_live() {
    let live = (0..=255u32)
        .filter(|v| TxType::from_u32(*v).is_some())
        .count();
    assert_eq!(live, WIRE_GOLDEN.len(), "TxType variant count changed");
    assert_eq!(live, 24);
}
