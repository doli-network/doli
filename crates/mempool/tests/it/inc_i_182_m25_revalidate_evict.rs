//! INC-I-182 M2.5 — revalidate evicts a resident Registration or AddBond the
//! producer did not sign once the next height reaches the gate, and keeps the
//! producer-signed one.
//!
// covers: producer_auth pool unbonding transaction types register bonds
//
// OUTPUT CONTRACT: fn Mempool::revalidate(&mut self, &UtxoSet, u64 /*tip*/, u32)
//   O1 residency (contains) of the unsigned and the signed tx; UtxoSet read-only
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64, u32) -> Result<AddTransactionResult, MempoolError>
//   O2 re-admission Err text of the evicted tx at tip G-1
// PATHS: child process, producer-auth gate env G = 100 | admitted at tip G-2 | revalidate tip G-2 (control) | tip G-1
// MATRIX
//   O1, O2 x Registration x {unsigned, signed} -> inc_i_182_m25_revalidate_evicts_unsigned_registration
//   O1, O2 x AddBond x {unsigned, signed} -> inc_i_182_m25_revalidate_evicts_unsigned_add_bond
// INPUT PARTITIONS:
//   form: unsigned | producer-signed with the mempool's genesis; each tx spends its own producer's Normal coin

use crypto::{BlsKeyPair, Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::transaction::{sign_producer_auth, Input, Output, Transaction, TxType};
use mempool::{Mempool, MempoolError, MempoolPolicy};
use storage::{Outpoint, UtxoEntry, UtxoSet};

const CODE: &str = "[ERRTX-PAUTH001]";
const G: u64 = 100;
const FEE: u64 = 100_000;
const GATE_ENV: &str = "DOLI_INC_I_182_PRODUCER_AUTH_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "INC_I_182_M25_MEMPOOL_CHILD";
const UNSIGNED_TAG: u8 = 0x61;
const SIGNED_TAG: u8 = 0x62;

fn addr(pk: &PublicKey) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes())
}

fn funded_input(utxo: &mut UtxoSet, kp: &KeyPair, tag: u8) -> Input {
    let op = Outpoint::new(Hash::from_bytes([tag; 32]), 0);
    let entry = UtxoEntry {
        output: Output::normal(Network::Devnet.bond_unit() + FEE, addr(kp.public_key())),
        height: 0,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    utxo.insert(op, entry).expect("FIXTURE: seed coin");
    let mut input = Input::new(op.tx_hash, op.index);
    input.public_key = Some(*kp.public_key());
    input
}

/// Replaces the two empty trailing `Vec<u8>` prefixes (BLS key, PoP) of the encoded payload.
fn with_bls(tx: &mut Transaction, seed: u8) {
    let bls = BlsKeyPair::from_seed(&[seed; 32]).expect("FIXTURE: BLS seed");
    let key = bls.public_key().as_bytes().to_vec();
    let pop = bls.proof_of_possession().expect("FIXTURE: PoP");
    let n = tx.extra_data.len();
    assert_eq!(
        &tx.extra_data[n - 16..],
        &[0u8; 16],
        "FIXTURE: RegistrationData layout"
    );
    tx.extra_data.truncate(n - 16);
    for part in [key, pop.as_bytes().to_vec()] {
        tx.extra_data
            .extend_from_slice(&(part.len() as u64).to_le_bytes());
        tx.extra_data.extend_from_slice(&part);
    }
}

/// `kind` for the producer `[tag; 32]`, paid from its own coin; producer-signed when `signed`.
fn producer_tx(kind: TxType, utxo: &mut UtxoSet, tag: u8, signed: bool) -> Transaction {
    let kp = KeyPair::from_seed([tag; 32]);
    let pk = *kp.public_key();
    let input = funded_input(utxo, &kp, tag);
    let unit = Network::Devnet.bond_unit();
    let lock = G + ConsensusParams::devnet().blocks_per_era + 1_000;
    let mut tx = match kind {
        TxType::Registration => {
            let mut t = Transaction::new_registration(vec![input], pk, unit, lock, 1);
            with_bls(&mut t, tag);
            t
        }
        _ => Transaction::new_add_bond(vec![input], pk, 1, unit, lock),
    };
    let msg = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&msg, kp.private_key());
    if signed {
        sign_producer_auth(&mut tx, &ConsensusParams::devnet().genesis_hash, &kp);
    }
    tx
}

fn offer(p: &mut Mempool, tx: &Transaction, utxo: &UtxoSet, tip: u64) -> Result<(), MempoolError> {
    p.add_transaction(tx.clone(), utxo, tip, 1).map(|_| ())
}

fn coded(r: &Result<(), MempoolError>) -> bool {
    matches!(r, Err(e) if format!("{e} {e:?}").contains(CODE))
}

/// Parent: re-exec `test` alone with the gate at G, return true. Child: return false.
fn spawned_child(test: &str) -> bool {
    if std::env::var_os(CHILD_ENV).is_some() {
        assert_eq!(
            Network::Devnet
                .params()
                .inc_i_182_producer_auth_activation_height,
            G,
            "PRECONDITION: {GATE_ENV} moves the devnet gate"
        );
        return false;
    }
    let out = std::process::Command::new(std::env::current_exe().expect("current exe"))
        .arg(format!("inc_i_182_m25_revalidate_evict::{test}"))
        .args(["--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1")
        .env(GATE_ENV, G.to_string())
        .output()
        .expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "{test}: child failed or did not run:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    true
}

fn evicts_unsigned(kind: TxType) {
    let mut utxo = UtxoSet::new();
    let unsigned = producer_tx(kind, &mut utxo, UNSIGNED_TAG, false);
    let signed = producer_tx(kind, &mut utxo, SIGNED_TAG, true);
    let mut p = Mempool::new(
        MempoolPolicy::testnet(),
        ConsensusParams::devnet(),
        Network::Devnet,
    );
    for tx in [&unsigned, &signed] {
        let r = offer(&mut p, tx, &utxo, G - 2);
        assert!(
            r.is_ok(),
            "PRECONDITION: {kind:?} admitted at tip {}: {r:?}",
            G - 2
        );
    }
    p.revalidate(&utxo, G - 2, 1);
    assert!(
        p.contains(&unsigned.hash()) && p.contains(&signed.hash()),
        "CONTROL: revalidate at tip {} (next height below the gate) must keep both {kind:?} txs",
        G - 2
    );
    p.revalidate(&utxo, G - 1, 1);
    assert!(
        !p.contains(&unsigned.hash()),
        "revalidate at tip {} must evict the unsigned {kind:?} (next block is at the gate)",
        G - 1
    );
    assert!(
        p.contains(&signed.hash()),
        "revalidate must keep the signed {kind:?}"
    );
    let again = offer(&mut p, &unsigned, &utxo, G - 1);
    assert!(
        coded(&again),
        "the evicted unsigned {kind:?} must be refused with {CODE} at tip {}: {again:?}",
        G - 1
    );
}

// REQ-PAUTH-002 — Decision: a failure means an unsigned Registration admitted below the gate stays resident once the next block is at the gate, so the builder retries it and peers relay it (or a signed one is evicted).
#[test]
fn inc_i_182_m25_revalidate_evicts_unsigned_registration() {
    if spawned_child("inc_i_182_m25_revalidate_evicts_unsigned_registration") {
        return;
    }
    evicts_unsigned(TxType::Registration);
}

// REQ-PAUTH-002 — Decision: a failure means an unsigned AddBond admitted below the gate stays resident once the next block is at the gate, so the builder retries it and peers relay it (or a signed one is evicted).
#[test]
fn inc_i_182_m25_revalidate_evicts_unsigned_add_bond() {
    if spawned_child("inc_i_182_m25_revalidate_evicts_unsigned_add_bond") {
        return;
    }
    evicts_unsigned(TxType::AddBond);
}
