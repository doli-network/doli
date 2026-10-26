//! INC-I-257 M2 — the one slash-target population function reports locked
//! collateral: the accused owns at least one Unbonding output still locked at
//! the judged height.
//!
// covers: slash_admission slash_state
//
// OUTPUT CONTRACT: fn slash_targets(&ProducerSet, &UtxoSet, u64 /*judged height*/, txs) -> Vec<(PublicKey, SlashTargetState)>
//   O1 return: one entry per SlashProducer whose accused is registered; fields active, slashed,
//      slash_queued, locked_collateral; pure (sets are read-only)
// PATHS: P-LOCKED | P-UNLOCKED | P-NONE | P-FOREIGN | P-UNREGISTERED | P-MIXED (two accused in one call)
// INPUT PARTITIONS:
//   judged height vs lock L: L - 1 (locked) | L (unlocked) | L + 1
//   accused's outputs: Unbonding | Bond only | Normal only | none; Unbonding owner: accused | another key
//   accused status: Exited | Active; registered | not registered
// MATRIX:
//   O1 x {P-LOCKED, P-UNLOCKED} x height -> inc_i_257_m2_req_unbond_011_slash_targets_locked_collateral_by_height
//   O1 x {P-NONE, P-FOREIGN, P-UNREGISTERED} -> inc_i_257_m2_req_unbond_012_slash_targets_no_collateral_cases
//   O1 x P-MIXED -> inc_i_257_m2_req_unbond_sec_004_slash_targets_per_accused_in_one_call
//   O1 x P-BUDGET (MAX_SLASH_TXS_PER_BLOCK + 1 accused, all locked) -> inc_i_257_m2_req_unbond_sec_004_slash_targets_lookup_budget_fails_closed

use crypto::{Hash, KeyPair, PublicKey, Signature};
use doli_core::block::{BlockBuilder, BlockHeader};
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

/// slash_targets reads only the accused key, so unsigned evidence suffices.
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
        extra_data: Vec::new(),
    }
}

fn producers(keys: &[(&PublicKey, ProducerStatus)]) -> ProducerSet {
    let mut ps = ProducerSet::new();
    for (pk, status) in keys {
        ps.register_genesis_producer(**pk, 1, unit())
            .expect("FIXTURE: register");
        ps.get_by_pubkey_mut(pk)
            .expect("FIXTURE: registered")
            .status = *status;
    }
    ps
}

fn one(ps: &ProducerSet, utxo: &UtxoSet, h: u64, pk: &PublicKey) -> Option<SlashTargetState> {
    let tx = slash(pk);
    let got = slash_targets(ps, utxo, h, [&tx]);
    assert!(got.len() <= 1, "one entry per SlashProducer; got {got:?}");
    got.into_iter().find(|(k, _)| k == pk).map(|(_, s)| s)
}

// REQ-UNBOND-011 / REQ-UNBOND-010 — Decision: a failure means mempool, builder and block validation (all fed by this one function) see collateral that is spendable, or miss collateral that is still locked.
#[test]
fn inc_i_257_m2_req_unbond_011_slash_targets_locked_collateral_by_height() {
    let kp = KeyPair::generate();
    let pk = *kp.public_key();
    let ps = producers(&[(&pk, ProducerStatus::Exited)]);
    let mut utxo = UtxoSet::new();
    seed(&mut utxo, 0x01, unbonding(addr(&pk), L));
    let locked = one(&ps, &utxo, L - 1, &pk).expect("registered accused has an entry");
    assert_eq!(
        locked,
        SlashTargetState {
            locked_collateral: true,
            ..SlashTargetState::default()
        },
        "Exited accused, Unbonding locked at L - 1"
    );
    for h in [L, L + 1] {
        let s = one(&ps, &utxo, h, &pk).expect("registered accused has an entry");
        assert_eq!(
            s,
            SlashTargetState::default(),
            "h={h}: the Unbonding output is spendable, so no locked collateral"
        );
    }
    let active_ps = producers(&[(&pk, ProducerStatus::Active)]);
    let s = one(&active_ps, &utxo, L - 1, &pk).expect("entry");
    assert!(
        s.active && s.locked_collateral,
        "Active accused with locked Unbonding: both flags; got {s:?}"
    );
}

// REQ-UNBOND-012 — Decision: a failure means collateral is credited for outputs a slash cannot burn (Bond-only owners keep the old path; foreign or Normal outputs are never the accused's collateral), or an unregistered key gets an entry.
#[test]
fn inc_i_257_m2_req_unbond_012_slash_targets_no_collateral_cases() {
    let kp = KeyPair::generate();
    let pk = *kp.public_key();
    let ps = producers(&[(&pk, ProducerStatus::Exited)]);
    let cases: [(&str, Vec<(u8, Output)>); 4] = [
        ("no outputs", Vec::new()),
        (
            "Bond only",
            vec![(0x02, Output::bond(unit(), addr(&pk), u64::MAX, 0))],
        ),
        (
            "Normal only",
            vec![(0x03, Output::normal(unit(), addr(&pk)))],
        ),
        (
            "Unbonding of another key",
            vec![(0x04, unbonding(Hash::from_bytes([0x77; 32]), L))],
        ),
    ];
    for (case, outputs) in cases {
        let mut utxo = UtxoSet::new();
        for (tag, o) in outputs {
            seed(&mut utxo, tag, o);
        }
        let s = one(&ps, &utxo, L - 1, &pk).expect("registered accused has an entry");
        assert!(!s.locked_collateral && !s.active, "{case}: got {s:?}");
    }
    let stranger = *KeyPair::generate().public_key();
    let mut utxo = UtxoSet::new();
    seed(&mut utxo, 0x05, unbonding(addr(&stranger), L));
    assert_eq!(
        one(&ps, &utxo, L - 1, &stranger),
        None,
        "an unregistered accused is left out even with a locked Unbonding output"
    );
}

// REQ-UNBOND-SEC-004 — Decision: a failure means one call mixes up the collateral of two accused (one lookup result reused for another key), so a block with two slashes gets a wrong verdict.
#[test]
fn inc_i_257_m2_req_unbond_sec_004_slash_targets_per_accused_in_one_call() {
    let (a, b) = (
        *KeyPair::generate().public_key(),
        *KeyPair::generate().public_key(),
    );
    let ps = producers(&[(&a, ProducerStatus::Exited), (&b, ProducerStatus::Exited)]);
    let mut utxo = UtxoSet::new();
    seed(&mut utxo, 0x06, unbonding(addr(&a), L));
    let (ta, tb) = (slash(&a), slash(&b));
    let got = slash_targets(&ps, &utxo, L - 1, [&ta, &tb]);
    let find = |k: &PublicKey| got.iter().find(|(p, _)| p == k).map(|(_, s)| *s);
    assert_eq!(got.len(), 2, "one entry per SlashProducer; got {got:?}");
    assert!(
        find(&a).is_some_and(|s| s.locked_collateral),
        "accused A owns the locked output; got {got:?}"
    );
    assert!(
        find(&b).is_some_and(|s| !s.locked_collateral),
        "accused B owns nothing; got {got:?}"
    );
}

// REQ-UNBOND-SEC-004 — Decision: a failure means one call does more than MAX_SLASH_TXS_PER_BLOCK owner lookups (unbounded validation cost per block), or an accused past the budget is credited collateral it was never checked for.
#[test]
fn inc_i_257_m2_req_unbond_sec_004_slash_targets_lookup_budget_fails_closed() {
    use doli_core::consensus::MAX_SLASH_TXS_PER_BLOCK;
    let keys: Vec<PublicKey> = (0..=MAX_SLASH_TXS_PER_BLOCK)
        .map(|_| *KeyPair::generate().public_key())
        .collect();
    let exited: Vec<(&PublicKey, ProducerStatus)> =
        keys.iter().map(|k| (k, ProducerStatus::Exited)).collect();
    let ps = producers(&exited);
    let mut utxo = UtxoSet::new();
    for (tag, k) in (0x10u8..).zip(&keys) {
        seed(&mut utxo, tag, unbonding(addr(k), L));
    }
    let txs: Vec<Transaction> = keys.iter().map(slash).collect();
    let got = slash_targets(&ps, &utxo, L - 1, &txs);
    let flags: Vec<bool> = keys
        .iter()
        .map(|k| {
            got.iter()
                .find(|(p, _)| p == k)
                .is_some_and(|(_, s)| s.locked_collateral)
        })
        .collect();
    let mut want = vec![true; MAX_SLASH_TXS_PER_BLOCK];
    want.push(false);
    assert_eq!(
        flags, want,
        "the first MAX_SLASH_TXS_PER_BLOCK accused are looked up; the next one fails closed"
    );
}
