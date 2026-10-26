//! B3 M2 — mempool admission and revalidation refuse a Registration or AddBond
//! whose key is Slashed or slash-queued in the shared live producer set, from
//! the slashing gate on; below the gate the verdict is unchanged.
//!
// covers: pool slash_admission registration transaction slash_state slash_registration types
//
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64, u32) -> Result<AddTransactionResult, MempoolError>
//   O3 return: Err whose text carries "[ERRTX071]" for a slashed / slash-queued key at/above the gate; Ok otherwise
//   O2 self.entries: the refused tx is not resident
// OUTPUT CONTRACT: fn Mempool::revalidate(&mut self, &UtxoSet, u64, u32)
//   O2 self.entries: a resident Registration / AddBond whose key became slashed or slash-queued is evicted
//      at/above the gate and kept below it; UtxoSet and live set read-only
// PATHS: P-ADMIT-REFUSE | P-ADMIT-CLEAN (control) | P-ADMIT-BELOW (child, gate env far above the tip)
//        | P-REVAL-EVICT | P-REVAL-BELOW (child)
// INPUT PARTITIONS:
//   key state in the live set: Slashed | slash-queued (Active + pending Slash) | Active | absent
//   tx: Registration | AddBond; gate: devnet 0 (above) | DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT = 1_000_000 (below)
// MATRIX:
//   O3,O2 x P-ADMIT-REFUSE x {Slashed, queued} x {Registration, AddBond} + P-ADMIT-CLEAN -> b3_m2_req_b3_003_admission_refuses_slashed_or_queued_key
//   O3,O2 x P-ADMIT-BELOW x {Slashed, queued} x both types -> b3_m2_req_b3_003_admission_below_gate_unchanged
//   O2,O3 x P-REVAL-EVICT x {Slashed, queued} x both types (+ re-admission code) -> b3_m2_req_b3_003_revalidate_evicts_once_key_slashed
//   O2 x P-REVAL-BELOW x {Slashed, queued} x both types -> b3_m2_req_b3_003_revalidate_below_gate_keeps

use std::sync::Arc;

use crypto::{BlsKeyPair, Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::transaction::{sign_producer_auth, Input, Output, Transaction, TxType};
use mempool::{Mempool, MempoolError, MempoolPolicy};
use storage::{Outpoint, PendingProducerUpdate, ProducerSet, UtxoEntry, UtxoSet};

use super::inc_i_256_m1_fixture::spawned_child;

const CODE: &str = "ERRTX071";
const TIP: u64 = 5;
const FEE: u64 = 100_000;
const BELOW_GATE: u64 = 1_000_000;
const GATE_ENV: &str = "DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "B3_M2_SLASHED_REG_CHILD";
const KINDS: [TxType; 2] = [TxType::Registration, TxType::AddBond];

#[derive(Clone, Copy, Debug)]
enum State {
    Slashed,
    Queued,
    Active,
    Absent,
}

struct Rig {
    pool: Mempool,
    producers: Arc<tokio::sync::RwLock<ProducerSet>>,
    utxo: UtxoSet,
}

fn addr(pk: &PublicKey) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes())
}

fn rig() -> Rig {
    let producers = Arc::new(tokio::sync::RwLock::new(ProducerSet::new()));
    let mut pool = Mempool::new(
        MempoolPolicy::testnet(),
        ConsensusParams::devnet(),
        Network::Devnet,
    );
    pool.share_producer_set(producers.clone());
    Rig {
        pool,
        producers,
        utxo: UtxoSet::new(),
    }
}

fn set_state(r: &Rig, pk: &PublicKey, state: State) {
    let mut ps = r.producers.try_write().expect("FIXTURE: live set unlocked");
    let unit = Network::Devnet.bond_unit();
    if !matches!(state, State::Absent) && ps.get_by_pubkey(pk).is_none() {
        ps.register_genesis_producer(*pk, 1, unit)
            .expect("FIXTURE: register producer");
    }
    match state {
        State::Slashed => {
            ps.slash_producer(pk, 1).expect("FIXTURE: slash producer");
        }
        State::Queued => ps.queue_update(PendingProducerUpdate::Slash {
            pubkey: *pk,
            height: 1,
        }),
        State::Active | State::Absent => {}
    }
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
    let lock = BELOW_GATE + ConsensusParams::devnet().blocks_per_era;
    let mut tx = if kind == TxType::Registration {
        let mut t = Transaction::new_registration(vec![input], pk, unit, lock, 1);
        with_bls(&mut t, tag);
        t
    } else {
        Transaction::new_add_bond(vec![input], pk, 1, unit, lock)
    };
    let msg = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&msg, kp.private_key());
    sign_producer_auth(&mut tx, &ConsensusParams::devnet().genesis_hash, kp);
    tx
}

fn coded(r: &Result<(), MempoolError>) -> bool {
    matches!(r, Err(e) if format!("{e} {e:?}").contains(CODE))
}

fn offer(r: &mut Rig, tx: &Transaction) -> Result<(), MempoolError> {
    r.pool
        .add_transaction(tx.clone(), &r.utxo, TIP, 1)
        .map(|_| ())
}

/// Fresh rig, key in `state`, then one admission of `kind`.
fn admit_with(state: State, kind: TxType, tag: u8) -> (Result<(), MempoolError>, bool) {
    let mut r = rig();
    let kp = KeyPair::from_seed([tag; 32]);
    set_state(&r, kp.public_key(), state);
    let tx = producer_tx(kind, &mut r.utxo, &kp, tag);
    let res = offer(&mut r, &tx);
    (res, r.pool.contains(&tx.hash()))
}

fn below_gate_precondition() {
    assert_eq!(
        Network::Devnet
            .params()
            .effective_slashing_activation_height(),
        BELOW_GATE,
        "PRECONDITION: {GATE_ENV} moves the devnet slashing gate"
    );
}

// REQ-B3-003 — Decision: a failure means the mempool admits (and relays) a Registration or AddBond for a Slashed or slash-queued key at/above the gate, or the fixture is not admissible at all.
#[test]
fn b3_m2_req_b3_003_admission_refuses_slashed_or_queued_key() {
    for (kind, clean) in [
        (TxType::Registration, State::Absent),
        (TxType::AddBond, State::Active),
    ] {
        let (res, resident) = admit_with(clean, kind, 0x31);
        assert!(
            res.is_ok() && resident,
            "NON-VACUITY: {kind:?} for a {clean:?} key must be admitted: {res:?}"
        );
    }
    for (i, state) in [State::Slashed, State::Queued].into_iter().enumerate() {
        for (j, kind) in KINDS.into_iter().enumerate() {
            let (res, resident) = admit_with(state, kind, 0x40 + (i * 2 + j) as u8);
            assert!(
                coded(&res) && !resident,
                "{kind:?} for a {state:?} key must be refused with {CODE} at admission and not be resident: {res:?}"
            );
        }
    }
}

// REQ-B3-003 — Decision: a failure means the new admission refusal leaks below the slashing gate, so the mempool and the block rule disagree before activation.
#[test]
fn b3_m2_req_b3_003_admission_below_gate_unchanged() {
    if spawned_child(
        "b3_m2_slashed_registration_admission::b3_m2_req_b3_003_admission_below_gate_unchanged",
        CHILD_ENV,
        &[(GATE_ENV, BELOW_GATE)],
    ) {
        return;
    }
    below_gate_precondition();
    for (i, state) in [State::Slashed, State::Queued].into_iter().enumerate() {
        for (j, kind) in KINDS.into_iter().enumerate() {
            let (res, resident) = admit_with(state, kind, 0x50 + (i * 2 + j) as u8);
            assert!(
                res.is_ok() && resident,
                "below the gate {kind:?} for a {state:?} key keeps the old verdict (admitted): {res:?}"
            );
        }
    }
}

/// Both kinds resident for a key admitted while Active, then the key moves to `state` and the pool revalidates.
fn revalidate_after(state: State, tag: u8) -> (Rig, Vec<Transaction>) {
    let mut r = rig();
    let kp = KeyPair::from_seed([tag; 32]);
    set_state(&r, kp.public_key(), State::Active);
    let txs: Vec<Transaction> = KINDS
        .into_iter()
        .zip([tag, tag.wrapping_add(1)])
        .map(|(kind, coin)| producer_tx(kind, &mut r.utxo, &kp, coin))
        .collect();
    for tx in &txs {
        let res = offer(&mut r, tx);
        assert!(
            res.is_ok(),
            "PRECONDITION: {:?} admitted while the key is Active: {res:?}",
            tx.tx_type
        );
    }
    set_state(&r, kp.public_key(), state);
    r.pool.revalidate(&r.utxo, TIP, 1);
    (r, txs)
}

// REQ-B3-003 — Decision: a failure means a Registration or AddBond admitted before its key was slashed stays resident, so the builder keeps retrying it and peers keep relaying it.
#[test]
fn b3_m2_req_b3_003_revalidate_evicts_once_key_slashed() {
    for (state, tag) in [(State::Queued, 0x60), (State::Slashed, 0x70)] {
        let (mut r, txs) = revalidate_after(state, tag);
        for tx in &txs {
            assert!(
                !r.pool.contains(&tx.hash()),
                "revalidate must evict the {:?} once its key is {state:?}",
                tx.tx_type
            );
            let again = offer(&mut r, tx);
            assert!(
                coded(&again),
                "re-admission of the evicted {:?} ({state:?} key) must be refused with {CODE}: {again:?}",
                tx.tx_type
            );
        }
    }
}

// REQ-B3-003 — Decision: a failure means revalidation evicts a valid producer tx below the slashing gate, before the rule is consensus.
#[test]
fn b3_m2_req_b3_003_revalidate_below_gate_keeps() {
    if spawned_child(
        "b3_m2_slashed_registration_admission::b3_m2_req_b3_003_revalidate_below_gate_keeps",
        CHILD_ENV,
        &[(GATE_ENV, BELOW_GATE)],
    ) {
        return;
    }
    below_gate_precondition();
    for (state, tag) in [(State::Queued, 0x80), (State::Slashed, 0x90)] {
        let (r, txs) = revalidate_after(state, tag);
        for tx in &txs {
            assert!(
                r.pool.contains(&tx.hash()),
                "below the gate revalidate keeps the {:?} of a {state:?} key",
                tx.tx_type
            );
        }
    }
}
