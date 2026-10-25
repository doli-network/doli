//! INC-I-262 M2 / REQ-DEF-003 — the slash-target fact builder reports the
//! registration floor slot: the smallest stamped slot over the Bond UTXOs of
//! the accused's bond owner, read in the same bounded owner lookup.
//!
// covers: slash_admission slash_state
//
// OUTPUT CONTRACT: fn slash_targets(&ProducerSet, &UtxoSet, u64 /*judged height*/, txs) -> Vec<(PublicKey, SlashTargetState)>
//   O1 return .oldest_bond_slot per accused
//   O2 return .locked_collateral (unchanged), .active, .slashed, .slash_queued, .registered_at
//   O3 mutable params / receiver / store writes: none (sets read-only)
// PATHS: P-MIN owner holds Bond UTXOs | P-NONE owner holds no Bond UTXO | P-CAP accused past
//        MAX_SLASH_TXS_PER_BLOCK lookups | P-REG Registration / AddBond key (no UTXO read)
// INPUT PARTITIONS:
//   owner outputs: Bonds at slots 300, 120, 450 (min not first) + Normal + Unbonding with 4-byte
//     extra_data + a foreign Bond at slot 1 | Normal + Unbonding only | nothing
//   judged height vs Unbonding lock L: L - 1 (locked) | L (unlocked)
//   accused count: MAX_SLASH_TXS_PER_BLOCK + 1
// MATRIX:
//   O1,O2 x P-MIN x height -> inc_i_262_m2_bond_slot_facts_min_over_owned_bonds
//   O1,O2 x P-NONE         -> inc_i_262_m2_bond_slot_facts_none_without_bond_utxo
//   O1 x P-CAP             -> inc_i_262_m2_bond_slot_facts_none_past_the_lookup_cap
//   O1,O2 x P-REG          -> inc_i_262_m2_bond_slot_facts_registrant_key_reads_no_bond_slot

use crypto::{Hash, KeyPair, PublicKey, Signature};
use doli_core::block::{BlockBuilder, BlockHeader};
use doli_core::consensus::MAX_SLASH_TXS_PER_BLOCK;
use doli_core::network::Network;
use doli_core::transaction::{Output, OutputType, SlashData, SlashingEvidence, Transaction};
use doli_core::validation::SlashTargetState;
use mempool::slash_admission::slash_targets;
use storage::{Outpoint, ProducerSet, ProducerStatus, UtxoEntry, UtxoSet};

const L: u64 = 100;

fn addr(pk: &PublicKey) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes())
}

fn unit() -> u64 {
    Network::Devnet.bond_unit()
}

fn slash(pk: &PublicKey) -> Transaction {
    let header = |salt: u8| -> BlockHeader {
        BlockBuilder::new(Hash::from_bytes([salt; 32]), 0, *pk)
            .build(1)
            .expect("FIXTURE: header")
            .0
    };
    Transaction::new_slash_producer(SlashData {
        producer_pubkey: *pk,
        evidence: SlashingEvidence::DoubleProduction {
            block_header_1: header(0xA1),
            block_header_2: header(0xB2),
        },
        reporter_signature: Signature::default(),
    })
}

fn seed(utxo: &mut UtxoSet, tag: u8, output: Output) {
    let entry = UtxoEntry {
        output,
        height: 0,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    utxo.insert(Outpoint::new(Hash::from_bytes([tag; 32]), 0), entry)
        .expect("FIXTURE: seed UTXO");
}

fn unbonding(owner: Hash, lock_until: u64) -> Output {
    Output {
        output_type: OutputType::Unbonding,
        amount: unit(),
        pubkey_hash: owner,
        lock_until,
        extra_data: 5u32.to_le_bytes().to_vec(),
    }
}

fn bond(owner: Hash, slot: u32) -> Output {
    Output::bond(unit(), owner, u64::MAX, slot)
}

fn producers(keys: &[&PublicKey]) -> ProducerSet {
    let mut ps = ProducerSet::new();
    for pk in keys {
        ps.register_genesis_producer(**pk, 1, unit())
            .expect("FIXTURE: register");
        ps.get_by_pubkey_mut(pk)
            .expect("FIXTURE: registered")
            .status = ProducerStatus::Active;
    }
    ps
}

fn one(
    ps: &ProducerSet,
    utxo: &UtxoSet,
    h: u64,
    tx: &Transaction,
    pk: &PublicKey,
) -> SlashTargetState {
    let got = slash_targets(ps, utxo, h, [tx]);
    assert_eq!(
        got.len(),
        1,
        "one entry for one registered key; got {got:?}"
    );
    got.into_iter()
        .find(|(k, _)| k == pk)
        .map(|(_, s)| s)
        .expect("entry for the key")
}

// REQ-DEF-003 — Decision: a failure means the floor is not the smallest stamped Bond slot of the accused's owner (first entry, a foreign Bond, or an Unbonding / Normal output counted), so an offence before the registration passes or one after it is refused.
#[test]
fn inc_i_262_m2_bond_slot_facts_min_over_owned_bonds() {
    let pk = *KeyPair::generate().public_key();
    let ps = producers(&[&pk]);
    let owner = addr(&pk);
    let mut utxo = UtxoSet::new();
    seed(&mut utxo, 0x01, bond(owner, 300));
    seed(&mut utxo, 0x02, bond(owner, 120));
    seed(&mut utxo, 0x03, bond(owner, 450));
    seed(&mut utxo, 0x04, Output::normal(unit(), owner));
    seed(&mut utxo, 0x05, unbonding(owner, L));
    seed(&mut utxo, 0x06, bond(Hash::from_bytes([0x77; 32]), 1));
    let tx = slash(&pk);
    let locked = one(&ps, &utxo, L - 1, &tx, &pk);
    assert_eq!(
        locked,
        SlashTargetState {
            active: true,
            locked_collateral: true,
            oldest_bond_slot: Some(120),
            ..SlashTargetState::default()
        },
        "Bonds at 300, 120, 450; Unbonding locked at L - 1"
    );
    let unlocked = one(&ps, &utxo, L, &tx, &pk);
    assert_eq!(
        (unlocked.locked_collateral, unlocked.oldest_bond_slot),
        (false, Some(120)),
        "at L the Unbonding output is spendable; the floor slot is unchanged"
    );
}

// REQ-DEF-003 — Decision: a failure means an owner with no Bond UTXO is given a floor slot (a default 0 admits any offence), or locked collateral is no longer reported for it.
#[test]
fn inc_i_262_m2_bond_slot_facts_none_without_bond_utxo() {
    let pk = *KeyPair::generate().public_key();
    let ps = producers(&[&pk]);
    let owner = addr(&pk);
    let tx = slash(&pk);
    let empty = one(&ps, &UtxoSet::new(), L - 1, &tx, &pk);
    assert_eq!(
        (empty.locked_collateral, empty.oldest_bond_slot),
        (false, None),
        "no outputs"
    );
    let mut utxo = UtxoSet::new();
    seed(&mut utxo, 0x11, Output::normal(unit(), owner));
    seed(&mut utxo, 0x12, unbonding(owner, L));
    let collateral = one(&ps, &utxo, L - 1, &tx, &pk);
    assert_eq!(
        (collateral.locked_collateral, collateral.oldest_bond_slot),
        (true, None),
        "Normal + locked Unbonding, no Bond UTXO"
    );
}

// REQ-DEF-003 — Decision: a failure means an accused past the bounded lookup budget gets a floor slot it was never checked for, or the budget grew an extra lookup.
#[test]
fn inc_i_262_m2_bond_slot_facts_none_past_the_lookup_cap() {
    let keys: Vec<PublicKey> = (0..=MAX_SLASH_TXS_PER_BLOCK)
        .map(|_| *KeyPair::generate().public_key())
        .collect();
    let refs: Vec<&PublicKey> = keys.iter().collect();
    let ps = producers(&refs);
    let mut utxo = UtxoSet::new();
    for (tag, k) in (0x20u8..).zip(&keys) {
        seed(&mut utxo, tag, bond(addr(k), 7));
    }
    let txs: Vec<Transaction> = keys.iter().map(slash).collect();
    let got = slash_targets(&ps, &utxo, L - 1, &txs);
    let slots: Vec<Option<u32>> = keys
        .iter()
        .map(|k| {
            got.iter()
                .find(|(p, _)| p == k)
                .and_then(|(_, s)| s.oldest_bond_slot)
        })
        .collect();
    let mut want = vec![Some(7); MAX_SLASH_TXS_PER_BLOCK];
    want.push(None);
    assert_eq!(
        slots, want,
        "the first MAX_SLASH_TXS_PER_BLOCK accused are looked up; the next one has no floor slot"
    );
}

// REQ-DEF-003 — Decision: a failure means a Registration / AddBond key now triggers an owner lookup (unbounded reads per block) or carries a floor slot into the slashed-registrant rule.
#[test]
fn inc_i_262_m2_bond_slot_facts_registrant_key_reads_no_bond_slot() {
    let pk = *KeyPair::generate().public_key();
    let ps = producers(&[&pk]);
    let owner = addr(&pk);
    let mut utxo = UtxoSet::new();
    seed(&mut utxo, 0x31, bond(owner, 40));
    seed(&mut utxo, 0x32, unbonding(owner, L));
    let add_bond = Transaction::new_add_bond(Vec::new(), pk, 1, unit(), u64::MAX);
    let s = one(&ps, &utxo, L - 1, &add_bond, &pk);
    assert_eq!(
        (s.locked_collateral, s.oldest_bond_slot),
        (false, None),
        "registrant-only key: no UTXO read"
    );
}
