//! INC-I-182 M1 — the payload decoders read only their prefix, so 64 bytes
//! appended to `extra_data` leave every decoded field and the tx encoding intact.
//!
// covers: data core transaction types
//
// OUTPUT CONTRACT: fn Transaction::withdrawal_request_data(&self) -> Option<WithdrawalRequestData>
// OUTPUT CONTRACT: fn Transaction::add_bond_data(&self) -> Option<AddBondData>
// OUTPUT CONTRACT: fn Transaction::registration_data(&self) -> Option<RegistrationData>
// OUTPUT CONTRACT: fn WithdrawalRequestData::from_bytes(&[u8]) -> Option<Self>; fn AddBondData::from_bytes(&[u8]) -> Option<Self>
// OUTPUT CONTRACT: bincode serialize / deserialize of Transaction
//   O1 return value of each; all pure, no receiver or store writes
// PATHS: P-WD 68-byte payload | P-AB 36-byte payload | P-REG bincode RegistrationData | P-TX Transaction bincode
// MATRIX
//   O1 x {P-WD, P-AB, P-REG} x {payload, payload + 64-byte tail} -> identical fields
//     (inc_i_182_m1_req_pauth_003_decoders_ignore_64_trailing_bytes)
//   O1 x P-TX x tailed tx of each type -> decodes equal, re-encodes to the same bytes (same test)
// INPUT PARTITIONS:
//   tail: 64 x 0xFF | 64 x 0x00 | ramp 0..63
//   registration payload: every field non-empty, BLS key 48 bytes, PoP 96 bytes

use crypto::{Hash, KeyPair};
use doli_core::transaction::{
    AddBondData, Input, RegistrationData, Transaction, WithdrawalRequestData,
};

fn tails() -> Vec<[u8; 64]> {
    let mut ramp = [0u8; 64];
    for (i, b) in ramp.iter_mut().enumerate() {
        *b = i as u8;
    }
    vec![[0xFF; 64], [0x00; 64], ramp]
}

fn inputs() -> Vec<Input> {
    vec![Input::new(Hash::from_bytes([0x18; 32]), 0)]
}

fn tailed(tx: &Transaction, tail: &[u8; 64]) -> Transaction {
    let mut t = tx.clone();
    t.extra_data.extend_from_slice(tail);
    t
}

fn assert_round_trip(tx: &Transaction, what: &str) {
    let bytes = bincode::serialize(tx).expect("encode tx");
    let back: Transaction = bincode::deserialize(&bytes).expect("decode tx");
    assert_eq!(
        &back, tx,
        "{what}: Transaction bincode round trip changed the tx"
    );
    assert_eq!(
        bincode::serialize(&back).expect("re-encode tx"),
        bytes,
        "{what}: re-encoding gives other bytes"
    );
}

// REQ-PAUTH-003 — Decision: a failure means a signature appended to extra_data changes what today's decoders read, so the fix would need a wire change instead of a forward-only gate.
#[test]
fn inc_i_182_m1_req_pauth_003_decoders_ignore_64_trailing_bytes() {
    let pk = *KeyPair::generate().public_key();
    let dest = Hash::from_bytes([0x5A; 32]);
    let wd = WithdrawalRequestData::new(pk, 3, dest);
    let wd_tx = Transaction::new_request_withdrawal(inputs(), pk, 3, dest, 1_000);
    let ab = AddBondData::new(pk, 2);
    let ab_tx = Transaction::new_add_bond(inputs(), pk, 2, 2_000, 0);
    let reg = RegistrationData {
        public_key: pk,
        epoch: 7,
        vdf_output: vec![0x11; 32],
        vdf_proof: vec![0x22; 40],
        prev_registration_hash: Hash::from_bytes([0x33; 32]),
        sequence_number: 9,
        bond_count: 2,
        bls_pubkey: vec![0x44; 48],
        bls_pop: vec![0x55; 96],
    };
    let mut reg_tx = Transaction::new_registration(inputs(), pk, 2_000, 0, 2);
    reg_tx.extra_data = bincode::serialize(&reg).expect("encode registration");
    assert_eq!(wd_tx.extra_data.len(), 68, "FIXTURE: withdrawal payload");
    assert_eq!(ab_tx.extra_data.len(), 36, "FIXTURE: add-bond payload");
    assert_eq!(
        wd_tx.withdrawal_request_data(),
        Some(wd.clone()),
        "FIXTURE: withdrawal decodes"
    );
    assert_eq!(
        ab_tx.add_bond_data(),
        Some(ab.clone()),
        "FIXTURE: add-bond decodes"
    );
    assert_eq!(
        reg_tx.registration_data(),
        Some(reg.clone()),
        "FIXTURE: registration decodes"
    );
    let reg_len = reg_tx.extra_data.len();
    for tail in tails() {
        let t = format!("tail starting {:#04x}", tail[1]);
        let (w, a, r) = (
            tailed(&wd_tx, &tail),
            tailed(&ab_tx, &tail),
            tailed(&reg_tx, &tail),
        );
        assert_eq!(
            w.withdrawal_request_data(),
            Some(wd.clone()),
            "withdrawal accessor, {t}"
        );
        assert_eq!(
            WithdrawalRequestData::from_bytes(&w.extra_data),
            Some(wd.clone()),
            "withdrawal from_bytes, {t}"
        );
        assert_eq!(
            a.add_bond_data(),
            Some(ab.clone()),
            "add-bond accessor, {t}"
        );
        assert_eq!(
            AddBondData::from_bytes(&a.extra_data),
            Some(ab.clone()),
            "add-bond from_bytes, {t}"
        );
        assert_eq!(
            r.registration_data(),
            Some(reg.clone()),
            "registration accessor, {t}"
        );
        for (tx, len, what) in [
            (&w, 68, "withdrawal"),
            (&a, 36, "add bond"),
            (&r, reg_len, "registration"),
        ] {
            assert_eq!(
                tx.extra_data.len(),
                len + 64,
                "FIXTURE: {what} tail appended"
            );
            assert_round_trip(tx, &format!("{what}, {t}"));
        }
    }
}
