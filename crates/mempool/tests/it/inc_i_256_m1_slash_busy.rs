//! INC-I-256 M1 / R-2 — slash admission fails CLOSED with a transient refusal
//! while the live producer set is write-locked; the holdings arms stay open.
//!
// covers: pool slash_admission entry lib slash_state error
//
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64, u32) -> Result<AddTransactionResult, MempoolError>
//   O1 return (Ok tx_hash | Err transient Busy | Err InvalidSlash | Err InvalidTransaction holdings code)
//   O2 receiver residency (contains, len); the shared producer set and the UtxoSet are read-only
// PATHS: P-BUSY SlashProducer, producer-set write lock held | P-RETRY same tx after release
//        | P-UNKNOWN accused not registered | P-WD RequestWithdrawal, lock held vs free | P-AB AddBond, lock held vs free
// INPUT PARTITIONS:
//   accused: registered Active | unregistered; attempt while locked: first | repeated
//   withdrawal producer: Slashed | allowance 0 (withdrawal pending); AddBond producer: at the bond cap
// MATRIX:
//   O1,O2 x {P-BUSY first, repeated, P-RETRY} x {registered, unregistered}
//     -> inc_i_256_m1_slash_admission_fails_closed_busy_while_producer_set_locked
//   O1,O2 x P-WD x {Slashed, allowance 0} x {free -> refused, held -> admitted}
//     -> inc_i_256_m1_withdrawal_arms_stay_fail_open_while_locked
//   O1,O2 x P-AB x {free -> refused, held -> admitted} -> inc_i_256_m1_addbond_cap_arm_stays_fail_open_while_locked

use std::sync::Arc;

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::transaction::{sign_producer_auth, Input, Output, OutputType, Transaction};
use doli_core::validation::ValidationError;
use mempool::{Mempool, MempoolError, MempoolPolicy};
use storage::{Outpoint, ProducerSet, UtxoSet};

use crate::inc_i_256_m1_fixture::{addr, policy, rig, seed, slash, Rig, TIP};

const FEE: u64 = 1_000;
const WD_HEIGHT: u64 = 30;
const TESTNET_HEIGHT: u64 = 100_007;

fn is_busy(r: &Result<mempool::AddTransactionResult, MempoolError>) -> bool {
    matches!(r, Err(e) if format!("{e:?}").contains("Busy"))
}

// R-2 — Decision: a failure means a slash offered while the producer set is write-locked skips the target check and is admitted (fail-open), or is refused for good instead of retryably.
#[test]
fn inc_i_256_m1_slash_admission_fails_closed_busy_while_producer_set_locked() {
    let (x, unknown) = (KeyPair::generate(), KeyPair::generate());
    let mut r = rig(policy(10, MempoolPolicy::testnet().max_age), &[&x]);
    let valid = slash(&x, (0xA1, 0xB2));
    let forged = slash(&unknown, (0xA1, 0xB2));
    let producers = r.producers.clone();
    let held = producers.try_write().expect("FIXTURE: hold the write lock");

    for attempt in 1..=2 {
        for tx in [&valid, &forged] {
            let res = r.pool.add_transaction(tx.clone(), &r.utxo, TIP, 0);
            assert!(
                is_busy(&res),
                "attempt {attempt}: with the producer set write-locked a slash must be refused with the transient Busy; got {res:?}"
            );
            assert!(!r.pool.contains(&tx.hash()));
        }
        assert_eq!(r.pool.len(), 0, "a Busy refusal leaves the pool unchanged");
    }

    drop(held);
    let res = r.pool.add_transaction(valid.clone(), &r.utxo, TIP, 0);
    assert!(
        matches!(&res, Ok(a) if a.tx_hash == valid.hash()),
        "after release the same slash is admitted under the same id; got {res:?}"
    );
    assert!(r.pool.contains(&valid.hash()));
    let res = r.pool.add_transaction(forged.clone(), &r.utxo, TIP, 0);
    assert!(
        matches!(&res, Err(MempoolError::Validation(ValidationError::InvalidSlash(m)))
            if m.contains("not a registered producer")),
        "after release a slash of an unregistered key gets its permanent refusal; got {res:?}"
    );
    assert_eq!(r.pool.len(), 1);
}

fn withdrawal(utxo: &mut UtxoSet, kp: &KeyPair, tag: u8) -> Transaction {
    let pk = *kp.public_key();
    let unit = Network::Devnet.bond_unit();
    let at = Outpoint::new(Hash::from_bytes([tag; 32]), 0);
    seed(utxo, at, Output::bond(unit, addr(&pk), u64::MAX, 0));
    let mut input = Input::new(at.tx_hash, at.index);
    input.public_key = Some(pk);
    let net = unit - FEE;
    let mut tx = Transaction::new_request_withdrawal(vec![input], pk, 1, addr(&pk), net);
    tx.outputs = vec![Output {
        output_type: OutputType::Unbonding,
        amount: net,
        pubkey_hash: addr(&pk),
        lock_until: WD_HEIGHT + 1 + Network::Devnet.params().unbonding_period,
        extra_data: Vec::new(),
    }];
    let msg = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&msg, kp.private_key());
    sign_producer_auth(&mut tx, &ConsensusParams::devnet().genesis_hash, kp);
    tx
}

/// Rig with `slashed` Slashed and `drained` holding allowance 0, plus one withdrawal each.
fn withdrawal_rig(slashed: &KeyPair, drained: &KeyPair) -> (Rig, [Transaction; 2]) {
    let mut r = rig(
        policy(10, MempoolPolicy::testnet().max_age),
        &[slashed, drained],
    );
    {
        let mut ps = r.producers.try_write().expect("FIXTURE: lock");
        ps.slash_producer(slashed.public_key(), 0)
            .expect("FIXTURE: slash");
        ps.get_by_pubkey_mut(drained.public_key())
            .expect("FIXTURE: registered")
            .withdrawal_pending_count = 1;
    }
    let txs = [
        withdrawal(&mut r.utxo, slashed, 0xE0),
        withdrawal(&mut r.utxo, drained, 0xE1),
    ];
    (r, txs)
}

// R-2 — Decision: a failure means the Busy refusal leaked into the RequestWithdrawal arms, so a contended lock refuses withdrawals the block rules would accept.
#[test]
fn inc_i_256_m1_withdrawal_arms_stay_fail_open_while_locked() {
    assert!(
        WD_HEIGHT
            >= Network::Devnet
                .params()
                .withdrawal_holdings_gate_activation_height,
        "FIXTURE: the holdings arm is armed at WD_HEIGHT"
    );
    let (slashed, drained) = (KeyPair::generate(), KeyPair::generate());
    let codes = ["slashed or slash-queued", "ECON_WITHDRAWAL_OVER_HOLDINGS"];

    let (mut free, txs) = withdrawal_rig(&slashed, &drained);
    for (tx, code) in txs.iter().zip(codes) {
        let res = free
            .pool
            .add_transaction(tx.clone(), &free.utxo, WD_HEIGHT, 0);
        assert!(
            matches!(&res, Err(MempoolError::InvalidTransaction(m)) if m.contains(code)),
            "NON-VACUITY: with the lock free the arm refuses with {code}; got {res:?}"
        );
    }

    let (mut locked, txs) = withdrawal_rig(&slashed, &drained);
    let producers = locked.producers.clone();
    let _held = producers.try_write().expect("FIXTURE: hold the write lock");
    for tx in &txs {
        let res = locked
            .pool
            .add_transaction(tx.clone(), &locked.utxo, WD_HEIGHT, 0);
        assert!(
            matches!(&res, Ok(a) if a.tx_hash == tx.hash()),
            "with the lock held the withdrawal arms stay fail-open; got {res:?}"
        );
    }
    assert_eq!(locked.pool.len(), 2);
}

// R-2 — Decision: a failure means the Busy refusal leaked into the AddBond cap arm, so a contended lock refuses AddBonds.
#[test]
fn inc_i_256_m1_addbond_cap_arm_stays_fail_open_while_locked() {
    let kp = KeyPair::from_seed([0x41; 32]);
    let pk = *kp.public_key();
    let unit = Network::Testnet.bond_unit();
    let mut utxo = UtxoSet::new();
    let at = Outpoint::new(Hash::from_bytes([0x1B; 32]), 0);
    seed(&mut utxo, at, Output::normal(unit + FEE, addr(&pk)));
    let mut input = Input::new(at.tx_hash, at.index);
    input.public_key = Some(pk);
    let mut tx = Transaction::new_add_bond(vec![input], pk, 1, unit, u64::MAX);
    let msg = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&msg, kp.private_key());
    sign_producer_auth(&mut tx, &ConsensusParams::testnet().genesis_hash, &kp);

    let mut ps = ProducerSet::new();
    ps.register_genesis_producer(pk, doli_core::MAX_BONDS_PER_PRODUCER, unit)
        .expect("FIXTURE: producer at the bond cap");
    let live = Arc::new(tokio::sync::RwLock::new(ps));

    let mut free = Mempool::testnet();
    free.share_producer_set(live.clone());
    let res = free.add_transaction(tx.clone(), &utxo, TESTNET_HEIGHT, 1);
    assert!(
        matches!(&res, Err(MempoolError::InvalidTransaction(m)) if m.contains("ADDBOND_CAP")),
        "NON-VACUITY: with the lock free the cap arm refuses; got {res:?}"
    );

    let mut locked = Mempool::testnet();
    locked.share_producer_set(live.clone());
    let _held = live.try_write().expect("FIXTURE: hold the write lock");
    let res = locked.add_transaction(tx.clone(), &utxo, TESTNET_HEIGHT, 1);
    assert!(
        matches!(&res, Ok(a) if a.tx_hash == tx.hash()),
        "with the lock held the AddBond cap arm stays fail-open; got {res:?}"
    );
    assert!(locked.contains(&tx.hash()));
}
