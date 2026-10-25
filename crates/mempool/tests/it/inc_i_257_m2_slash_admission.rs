//! INC-I-257 M2 — mempool admission: a slash against an Exited accused is
//! admitted while it owns a locked Unbonding output (judged at tip + 1) and
//! refused once that output unlocks; a Transfer with a Bond output is refused
//! at the devnet gate.
//!
// covers: slash_admission pool slash_state output_authority
//
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64 /*tip*/, u32) -> Result<AddTransactionResult, MempoolError>
//   O1 return (Ok | Err, InvalidSlash message); O2 residency of the tx (receiver: contains)
//   no store writes; the UtxoSet and the shared producer set are read-only
// PATHS: P-LOCKED Exited accused, Unbonding locked at tip + 1 | P-UNLOCKED Unbonding lock reached at tip + 1
//        | P-NONE Exited accused, no Unbonding | P-ACTIVE Active accused, genesis Bond, no Unbonding (control)
//        | P-BOND Transfer creating a Bond output | P-NORMAL the same Transfer paying Normal (control)
// INPUT PARTITIONS:
//   tip vs lock L: tip = L - 2 (judged L - 1, locked) | tip = L - 1 (judged L, unlocked)
//   Unbonding owner: accused | another key; accused status: Exited | Active
//   Transfer output type: Bond | Normal
// MATRIX:
//   O1,O2 x {P-LOCKED, P-UNLOCKED} -> inc_i_257_m2_req_unbond_011_mempool_admits_slash_while_collateral_locked
//   O1,O2 x {P-NONE, foreign Unbonding, P-ACTIVE} -> inc_i_257_m2_req_unbond_012_mempool_refuses_slash_without_collateral
//   O1,O2 x {P-BOND, P-NORMAL} -> inc_i_257_m2_req_unbond_013_mempool_refuses_transfer_with_bond_output

use std::sync::Arc;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::block::{compute_merkle_root, BlockBuilder, BlockHeader};
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::tpop::heartbeat::hash_chain_vdf;
use doli_core::transaction::{Input, Output, OutputType, SlashData, Transaction};
use doli_core::validation::ValidationError;
use mempool::{Mempool, MempoolError, MempoolPolicy};
use storage::{Outpoint, ProducerSet, ProducerStatus, UtxoEntry, UtxoSet};

const SLOT: u32 = 7;
const L: u64 = 100;
const COIN: u64 = 5_000_000;
const FEE: u64 = 500_000;
const REFUSAL: &str = "not an active or unbonding producer";

fn addr(pk: &PublicKey) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes())
}

fn unit() -> u64 {
    Network::Devnet.bond_unit()
}

fn vdf_input(prev: &Hash, merkle: &Hash, slot: u32, producer: &PublicKey) -> Hash {
    let mut h = crypto::Hasher::new();
    h.update(b"DOLI_VDF_BLOCK_V1");
    h.update(prev.as_bytes());
    h.update(merkle.as_bytes());
    h.update(&slot.to_le_bytes());
    h.update(producer.as_bytes());
    h.finalize()
}

fn evidence(kp: &KeyPair, salt: u8) -> BlockHeader {
    let p = ConsensusParams::devnet();
    let producer = *kp.public_key();
    let prev = Hash::from_bytes([salt; 32]);
    let (mut h, _) = BlockBuilder::new(prev, 0, producer)
        .build(p.slot_to_timestamp(SLOT))
        .expect("FIXTURE: header");
    h.prev_hash = prev;
    h.genesis_hash = p.genesis_hash;
    h.slot = SLOT;
    h.timestamp = p.slot_to_timestamp(SLOT);
    h.merkle_root = compute_merkle_root(&[]);
    let input = vdf_input(&prev, &h.merkle_root, SLOT, &producer);
    h.vdf_output.value =
        hash_chain_vdf(&input, Network::Devnet.heartbeat_vdf_iterations()).to_vec();
    let msg = h.signing_message();
    h.vdf_proof.pi = crypto::signature::sign(msg.as_bytes(), kp.private_key()).to_vec();
    h
}

fn slash(kp: &KeyPair) -> Transaction {
    let data =
        SlashData::double_production(*kp.public_key(), evidence(kp, 0xA1), evidence(kp, 0xB2));
    Transaction::new_slash_producer(data)
}

fn seed(utxo: &mut UtxoSet, tag: u8, output: Output) -> Outpoint {
    let op = Outpoint::new(Hash::from_bytes([tag; 32]), 0);
    let entry = UtxoEntry {
        output,
        height: 0,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    utxo.insert(op, entry).expect("FIXTURE: seed UTXO");
    op
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

/// Pool sharing a producer set where `accused` is registered with `status`.
fn pool_with(accused: &KeyPair, status: ProducerStatus) -> Mempool {
    let mut ps = ProducerSet::new();
    ps.register_genesis_producer(*accused.public_key(), 1, unit())
        .expect("FIXTURE: register accused");
    ps.get_by_pubkey_mut(accused.public_key())
        .expect("FIXTURE: accused registered")
        .status = status;
    let mut pool = Mempool::new(
        MempoolPolicy::testnet(),
        ConsensusParams::devnet(),
        Network::Devnet,
    );
    pool.share_producer_set(Arc::new(tokio::sync::RwLock::new(ps)));
    pool
}

fn slash_msg(r: &Result<(), MempoolError>) -> Option<String> {
    match r {
        Err(MempoolError::Validation(ValidationError::InvalidSlash(m))) => Some(m.clone()),
        _ => None,
    }
}

/// Fresh pool; offer the accused's slash at `tip`; (result, resident).
fn offer_slash(
    accused: &KeyPair,
    status: ProducerStatus,
    utxo: &UtxoSet,
    tip: u64,
) -> (Result<(), MempoolError>, bool) {
    let mut pool = pool_with(accused, status);
    let tx = slash(accused);
    let r = pool.add_transaction(tx.clone(), utxo, tip, 0).map(|_| ());
    (r, pool.contains(&tx.hash()))
}

// REQ-UNBOND-011 / REQ-UNBOND-010 — Decision: a failure means the mempool refuses the only slash that can still burn an exited accused's locked withdrawal (the funds escape), or keeps admitting it once the output is spendable (judged at the tip instead of tip + 1).
#[test]
fn inc_i_257_m2_req_unbond_011_mempool_admits_slash_while_collateral_locked() {
    let accused = KeyPair::generate();
    let mut utxo = UtxoSet::new();
    seed(&mut utxo, 0x01, unbonding(addr(accused.public_key()), L));
    let (locked, resident) = offer_slash(&accused, ProducerStatus::Exited, &utxo, L - 2);
    assert!(
        locked.is_ok() && resident,
        "judged at L - 1 the Exited accused still owns a locked Unbonding output: admit; got {locked:?}"
    );
    let (unlocked, resident) = offer_slash(&accused, ProducerStatus::Exited, &utxo, L - 1);
    assert!(
        slash_msg(&unlocked).is_some_and(|m| m.contains(REFUSAL)) && !resident,
        "judged at L the output is spendable, nothing is burnable: refuse; got {unlocked:?}"
    );
}

// REQ-UNBOND-012 — Decision: a failure means the mempool admits a slash against an exited accused with nothing burnable (a mineable no-op), or the rig refuses an Active accused for an unrelated reason.
#[test]
fn inc_i_257_m2_req_unbond_012_mempool_refuses_slash_without_collateral() {
    let accused = KeyPair::generate();
    let empty = UtxoSet::new();
    let mut bonded = UtxoSet::new();
    seed(
        &mut bonded,
        0x04,
        Output::bond(unit(), addr(accused.public_key()), u64::MAX, 0),
    );
    let (active, resident) = offer_slash(&accused, ProducerStatus::Active, &bonded, L - 2);
    assert!(
        active.is_ok() && resident,
        "NON-VACUITY: a slash of an Active accused is admitted; got {active:?}"
    );
    let (none, resident) = offer_slash(&accused, ProducerStatus::Exited, &empty, L - 2);
    assert!(
        slash_msg(&none).is_some_and(|m| m.contains(REFUSAL)) && !resident,
        "Exited accused with no Unbonding output: refuse; got {none:?}"
    );
    let mut foreign = UtxoSet::new();
    seed(
        &mut foreign,
        0x02,
        unbonding(Hash::from_bytes([0x77; 32]), L),
    );
    seed(
        &mut foreign,
        0x03,
        Output::normal(COIN, addr(accused.public_key())),
    );
    let (r, resident) = offer_slash(&accused, ProducerStatus::Exited, &foreign, L - 2);
    assert!(
        slash_msg(&r).is_some_and(|m| m.contains(REFUSAL)) && !resident,
        "a locked Unbonding output of another key, or a Normal coin, is not the accused's collateral; got {r:?}"
    );
}

fn signed_transfer(kp: &KeyPair, from: Outpoint, out: Output) -> Transaction {
    let mut input = Input::new(from.tx_hash, from.index);
    input.public_key = Some(*kp.public_key());
    let mut tx = Transaction::new_transfer(vec![input], vec![out]);
    let msg = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&msg, kp.private_key());
    tx
}

// REQ-UNBOND-013 — Decision: a failure means the mempool relays and holds a Transfer that mints a Bond output, which the builder would then put into blocks that apply refuses.
#[test]
fn inc_i_257_m2_req_unbond_013_mempool_refuses_transfer_with_bond_output() {
    assert_eq!(
        Network::Devnet
            .params()
            .inc_i_257_unbonding_activation_height,
        0,
        "FIXTURE: devnet gate"
    );
    let kp = KeyPair::generate();
    let own = addr(kp.public_key());
    let mut utxo = UtxoSet::new();
    let coin = seed(&mut utxo, 0x04, Output::normal(COIN, own));
    let mut pool = pool_with(&kp, ProducerStatus::Active);
    let normal = signed_transfer(&kp, coin, Output::normal(COIN - FEE, own));
    let ok = pool.add_transaction(normal.clone(), &utxo, 10, 0);
    assert!(
        ok.is_ok(),
        "NON-VACUITY: the Normal twin is admitted; got {ok:?}"
    );
    pool.remove_transaction(&normal.hash());
    let bond = signed_transfer(&kp, coin, Output::bond(COIN - FEE, own, 10_000, 0));
    let r = pool.add_transaction(bond.clone(), &utxo, 10, 0);
    assert!(
        r.is_err() && !pool.contains(&bond.hash()),
        "a Transfer with a Bond output must be refused at the gate; got {r:?}"
    );
}
