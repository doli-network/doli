//! B3 M2.5 — REQ-B3-003 / REV-B3-001: the mempool keeps a slash for K and a Registration or AddBond for K apart.
//!
// covers: slash_admission, pool
//
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64, u32) -> Result<AddTransactionResult, MempoolError>
//   O1 return: Err carrying "[ERRTX071]" for a Registration / AddBond naming K while a gated SlashProducer(K) is resident;
//      Ok for a gated SlashProducer(K) beside a resident Registration / AddBond for K; the old verdict otherwise
//   O2 self.entries: the refused tx is not resident; admitting the slash evicts the resident Registration / AddBond for K
//      and keeps one for another key; the resident slash survives a refused registrant
//   O3 UtxoSet and live producer set: read-only
// PATHS: P-REFUSE registrant offered beside a resident slash | P-EVICT slash offered beside a resident registrant
//        | P-BELOW (child, gate 2) registrant judged below the gate beside a gated resident slash | P-OTHER registrant for K'
// INPUT PARTITIONS:
//   registrant: AddBond | Registration; key: accused K | other K' (Active) | fresh key (absent)
//   order: slash first | registrant first; judged height: at/above the gate | below it
// MATRIX:
//   O1,O2 x P-REFUSE x AddBond(K) -> b3_m25_add_bond_refused_beside_resident_slash
//   O1,O2 x P-REFUSE x Registration(K) -> b3_m25_registration_refused_beside_resident_slash
//   O1,O2 x P-EVICT x AddBond(K) (+ AddBond(K') kept) -> b3_m25_slash_evicts_resident_add_bond_same_key
//   O1,O2 x P-EVICT x Registration(K) (+ Registration(fresh) kept) -> b3_m25_slash_evicts_resident_registration_same_key
//   O1,O2 x P-BELOW x both registrants -> admitted, resident (b3_m25_admission_below_gate_unchanged)
//   O1,O2 x P-OTHER x both orders -> admitted, resident (b3_m25_other_key_add_bond_admitted_beside_slash)

use std::sync::Arc;

use crypto::{BlsKeyPair, Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::transaction::{sign_producer_auth, Input, Output, Transaction, TxType};
use mempool::{Mempool, MempoolError, MempoolPolicy};
use storage::{Outpoint, ProducerSet, UtxoEntry, UtxoSet};

use super::inc_i_256_m1_fixture::{bond_at, slash as fixture_slash, spawned_child};

const CODE: &str = "ERRTX071";
const FEE: u64 = 100_000;
const LOCK: u64 = 10_000_000;
const GATE_ENV: &str = "DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "B3_M25_SAME_BLOCK_ADMISSION_CHILD";
const RAISED_GATE: u64 = 2;
const KINDS: [TxType; 2] = [TxType::AddBond, TxType::Registration];

struct Rig {
    pool: Mempool,
    utxo: UtxoSet,
    _producers: Arc<tokio::sync::RwLock<ProducerSet>>,
}

fn addr(pk: &PublicKey) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes())
}

fn rig(registered: &[&KeyPair]) -> Rig {
    let mut ps = ProducerSet::new();
    let mut utxo = UtxoSet::new();
    for kp in registered {
        ps.register_genesis_producer(*kp.public_key(), 1, Network::Devnet.bond_unit())
            .expect("FIXTURE: register producer");
        bond_at(&mut utxo, kp.public_key(), 0);
    }
    let producers = Arc::new(tokio::sync::RwLock::new(ps));
    let mut pool = Mempool::new(
        MempoolPolicy::testnet(),
        ConsensusParams::devnet(),
        Network::Devnet,
    );
    pool.share_producer_set(producers.clone());
    Rig {
        pool,
        utxo,
        _producers: producers,
    }
}

fn slash(kp: &KeyPair) -> Transaction {
    fixture_slash(kp, (0xA1, 0xB2))
}

/// Replaces the two empty trailing `Vec<u8>` prefixes (BLS key, PoP) of the encoded payload.
fn with_bls(tx: &mut Transaction, seed: u8) {
    let bls = BlsKeyPair::from_seed(&[seed; 32]).expect("FIXTURE: BLS seed");
    let pop = bls.proof_of_possession().expect("FIXTURE: PoP");
    let n = tx.extra_data.len();
    assert_eq!(
        &tx.extra_data[n - 16..],
        &[0u8; 16],
        "FIXTURE: RegistrationData layout"
    );
    tx.extra_data.truncate(n - 16);
    for part in [
        bls.public_key().as_bytes().to_vec(),
        pop.as_bytes().to_vec(),
    ] {
        tx.extra_data
            .extend_from_slice(&(part.len() as u64).to_le_bytes());
        tx.extra_data.extend_from_slice(&part);
    }
}

/// Producer-signed `kind` for `kp`, paid from a fresh coin of its own at `[tag; 32]:0`.
fn producer_tx(kind: TxType, utxo: &mut UtxoSet, kp: &KeyPair, tag: u8) -> Transaction {
    let pk = *kp.public_key();
    let unit = Network::Devnet.bond_unit();
    let op = Outpoint::new(Hash::from_bytes([tag; 32]), 0);
    let entry = UtxoEntry {
        output: Output::normal(unit + FEE, addr(&pk)),
        height: 0,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    utxo.insert(op, entry).expect("FIXTURE: seed coin");
    let mut input = Input::new(op.tx_hash, op.index);
    input.public_key = Some(pk);
    let mut tx = if kind == TxType::Registration {
        let mut t = Transaction::new_registration(vec![input], pk, unit, LOCK, 1);
        with_bls(&mut t, tag);
        t
    } else {
        Transaction::new_add_bond(vec![input], pk, 1, unit, LOCK)
    };
    let msg = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&msg, kp.private_key());
    sign_producer_auth(&mut tx, &ConsensusParams::devnet().genesis_hash, kp);
    tx
}

fn coded(r: &Result<(), MempoolError>) -> bool {
    matches!(r, Err(e) if format!("{e} {e:?}").contains(CODE))
}

impl Rig {
    fn offer(&mut self, tx: &Transaction, tip: u64) -> Result<(), MempoolError> {
        self.pool
            .add_transaction(tx.clone(), &self.utxo, tip, 0)
            .map(|_| ())
    }

    fn admit(&mut self, tx: &Transaction, tip: u64, what: &str) {
        let r = self.offer(tx, tip);
        assert!(
            r.is_ok() && self.pool.contains(&tx.hash()),
            "PRECONDITION: {what} admitted and resident; got {r:?}"
        );
    }

    fn resident(&self, tx: &Transaction) -> bool {
        self.pool.contains(&tx.hash())
    }
}

fn alone_admissible(kind: TxType, tag: u8) {
    let k = KeyPair::from_seed([tag; 32]);
    let mut r = rig(&[&k]);
    let tx = producer_tx(kind, &mut r.utxo, &k, tag);
    r.admit(
        &tx,
        0,
        &format!("NON-VACUITY: {kind:?}(K) with no resident slash"),
    );
}

fn refused_beside_resident_slash(kind: TxType, tag: u8) {
    alone_admissible(kind, tag);
    let k = KeyPair::from_seed([tag; 32]);
    let mut r = rig(&[&k]);
    let s = slash(&k);
    r.admit(&s, 0, "Slash(K)");
    let tx = producer_tx(kind, &mut r.utxo, &k, tag);
    let res = r.offer(&tx, 0);
    assert!(
        coded(&res) && !r.resident(&tx),
        "{kind:?}(K) beside a resident gated Slash(K) must be refused with {CODE} and not be resident; got {res:?}"
    );
    assert!(r.resident(&s), "the resident Slash(K) must stay resident");
}

fn slash_evicts(kind: TxType, tag: u8) {
    let (k, k2) = (
        KeyPair::from_seed([tag; 32]),
        KeyPair::from_seed([tag + 1; 32]),
    );
    let fresh = KeyPair::from_seed([tag + 2; 32]);
    let mut r = rig(&[&k, &k2]);
    let same = producer_tx(kind, &mut r.utxo, &k, tag);
    let keep_key = if kind == TxType::AddBond { &k2 } else { &fresh };
    let kept = producer_tx(kind, &mut r.utxo, keep_key, tag + 1);
    r.admit(&same, 0, &format!("{kind:?}(K)"));
    r.admit(&kept, 0, &format!("{kind:?} for another key"));
    let s = slash(&k);
    let res = r.offer(&s, 0);
    assert!(
        res.is_ok() && r.resident(&s),
        "Slash(K) beside a resident {kind:?}(K) must be admitted; got {res:?}"
    );
    assert!(
        !r.resident(&same),
        "admitting a gated Slash(K) must evict the resident {kind:?}(K)"
    );
    assert!(
        r.resident(&kept),
        "admitting Slash(K) must keep the {kind:?} for another key"
    );
}

// REQ-B3-003 — Decision: a failure means the mempool admits AddBond(K) beside a resident Slash(K), so an honest builder packs both and the new bonds are orphaned.
#[test]
fn b3_m25_add_bond_refused_beside_resident_slash() {
    refused_beside_resident_slash(TxType::AddBond, 0x21);
}

// REQ-B3-003 — Decision: a failure means the mempool admits Registration(K) beside a resident Slash(K), so both reach the builder and the wire.
#[test]
fn b3_m25_registration_refused_beside_resident_slash() {
    refused_beside_resident_slash(TxType::Registration, 0x25);
}

// REQ-B3-003 — Decision: a failure means a resident AddBond(K) survives the admission of Slash(K), so the builder is offered both for one block.
#[test]
fn b3_m25_slash_evicts_resident_add_bond_same_key() {
    slash_evicts(TxType::AddBond, 0x31);
}

// REQ-B3-003 — Decision: a failure means a resident Registration(K) survives the admission of Slash(K), so the builder is offered both for one block.
#[test]
fn b3_m25_slash_evicts_resident_registration_same_key() {
    slash_evicts(TxType::Registration, 0x35);
}

// REQ-B3-003 — Decision: a failure means the new mempool refusal leaks below the slashing gate, so the mempool and the block rule disagree before activation.
#[test]
fn b3_m25_admission_below_gate_unchanged() {
    if spawned_child(
        "b3_m25_slash_admission_same_block::b3_m25_admission_below_gate_unchanged",
        CHILD_ENV,
        &[(GATE_ENV, RAISED_GATE)],
    ) {
        return;
    }
    let gate = Network::Devnet
        .params()
        .effective_slashing_activation_height();
    assert_eq!(gate, RAISED_GATE, "PRECONDITION: {GATE_ENV} moves the gate");
    for (i, kind) in KINDS.into_iter().enumerate() {
        let tag = 0x41 + i as u8;
        let k = KeyPair::from_seed([tag; 32]);
        let mut r = rig(&[&k]);
        r.admit(&slash(&k), gate, "Slash(K) judged at the gate");
        let tx = producer_tx(kind, &mut r.utxo, &k, tag);
        let res = r.offer(&tx, gate - 2);
        assert!(
            res.is_ok() && r.resident(&tx),
            "{kind:?}(K) judged below the gate keeps the old verdict (admitted); got {res:?}"
        );
    }
}

// REQ-B3-003 — Decision: a failure means the mempool over-refuses or over-evicts an AddBond or Registration for a key other than the accused.
#[test]
fn b3_m25_other_key_add_bond_admitted_beside_slash() {
    let (k, k2, fresh) = (
        KeyPair::from_seed([0x51; 32]),
        KeyPair::from_seed([0x52; 32]),
        KeyPair::from_seed([0x53; 32]),
    );
    for slash_first in [true, false] {
        let mut r = rig(&[&k, &k2]);
        let s = slash(&k);
        let others = [
            producer_tx(TxType::AddBond, &mut r.utxo, &k2, 0x52),
            producer_tx(TxType::Registration, &mut r.utxo, &fresh, 0x53),
        ];
        if slash_first {
            r.admit(&s, 0, "Slash(K)");
        }
        for tx in &others {
            let res = r.offer(tx, 0);
            assert!(
                res.is_ok(),
                "slash_first={slash_first}: {:?} for another key must be admitted; got {res:?}",
                tx.tx_type
            );
        }
        if !slash_first {
            r.admit(&s, 0, "Slash(K) after the other keys' txs");
        }
        for tx in &others {
            assert!(
                r.resident(tx),
                "slash_first={slash_first}: {:?} for another key must stay resident beside Slash(K)",
                tx.tx_type
            );
        }
    }
}
