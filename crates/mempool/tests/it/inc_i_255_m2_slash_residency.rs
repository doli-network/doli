//! INC-I-255 M2 — the mempool holds at most one slash per accused, and
//! revalidation evicts a resident slash whose accused is already queued or Slashed.
//!
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64, u32) -> Result<AddTransactionResult, MempoolError>
//   O1 return; O2 residency (receiver)
// OUTPUT CONTRACT: fn Mempool::revalidate(&mut self, &UtxoSet, u64, u32)
//   O1 residency of each slash (receiver)
// PATHS: admission with the live producer set shared | revalidate after the accused's state changed
//        | P-BELOW revalidate below an env-raised gate | P-ABSENT no live set | P-BUSY live set write-locked
//        | P-BAD undecodable slash data | P-WD RequestWithdrawal by a Slashed or queued producer
// MATRIX
//   add_transaction O1,O2 x {first evidence -> Ok, resident; second evidence same accused -> Err, not resident;
//     other accused -> Ok; second after the first is removed -> Ok} (inc_i_255_m2_mempool_one_resident_slash_per_accused)
//   revalidate O1 x {unchanged -> kept; Slash queued -> evicted} (inc_i_255_m2_mempool_revalidate_evicts_queued_accused)
//   revalidate O1 x Slashed -> evicted (inc_i_255_m2_mempool_revalidate_evicts_slashed_accused)
//   revalidate O1 x P-BELOW x queued -> kept; at the gate -> evicted (inc_i_255_m2_mempool_revalidate_keeps_slash_below_gate)
//   add_transaction, revalidate O1,O2 x P-ABSENT -> admitted, second refused, kept (inc_i_255_m2_mempool_no_live_set_fails_open)
//   add_transaction, revalidate O1,O2 x P-BUSY x queued -> kept, second refused; lock free -> evicted
//     (inc_i_255_m2_mempool_busy_live_set_fails_open)
//   add_transaction O1,O2 x P-BAD x {live set, none} -> InvalidSlash, not resident (inc_i_255_m2_mempool_undecodable_slash_*)
//   add_transaction O1,O2 x P-WD x {queued, Slashed} x env-raised gate: below -> Ok, resident; at -> refused, not resident
//     (inc_i_255_m2_mempool_withdrawal_by_slashed_producer_admitted_below_gate)
// INPUT PARTITIONS:
//   evidence per accused: pair A | pair B; accused: X | Y
//   accused state: Active | Slash queued | Slashed
//   live set: shared | absent | write-locked; gate: devnet default | env-raised; slash data: decodable | undecodable
//   tx: SlashProducer | RequestWithdrawal; judged height (best + 1): below the gate | at the gate

use std::sync::Arc;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::block::{compute_merkle_root, BlockBuilder, BlockHeader};
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::tpop::heartbeat::hash_chain_vdf;
use doli_core::transaction::{Input, Output, SlashData, Transaction};
use doli_core::validation::ValidationError;
use mempool::{AddTransactionResult, Mempool, MempoolError, MempoolPolicy};
use storage::{Outpoint, PendingProducerUpdate, ProducerSet, UtxoEntry, UtxoSet};

const SLOT: u32 = 7;

struct Rig {
    pool: Mempool,
    producers: Arc<tokio::sync::RwLock<ProducerSet>>,
    utxo: UtxoSet,
}

fn rig(accused: &[&KeyPair]) -> Rig {
    let mut ps = ProducerSet::new();
    for kp in accused {
        ps.register_genesis_producer(*kp.public_key(), 1, Network::Devnet.bond_unit())
            .expect("FIXTURE: register accused");
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
        producers,
        utxo: UtxoSet::new(),
    }
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

fn slash(kp: &KeyPair, salts: (u8, u8)) -> Transaction {
    let data = SlashData::double_production(
        *kp.public_key(),
        evidence(kp, salts.0),
        evidence(kp, salts.1),
    );
    Transaction::new_slash_producer(data)
}

impl Rig {
    fn add(&mut self, tx: &Transaction) -> bool {
        self.pool
            .add_transaction(tx.clone(), &self.utxo, 0, 0)
            .is_ok()
    }

    fn revalidate(&mut self) {
        self.pool.revalidate(&self.utxo, 0, 0);
    }
}

// REQ-SLASH-004 — Decision: a failure means one accused can hold two resident slashes, so both reach the builder and the wire.
#[test]
fn inc_i_255_m2_mempool_one_resident_slash_per_accused() {
    let (x, y) = (KeyPair::generate(), KeyPair::generate());
    let mut r = rig(&[&x, &y]);
    let (first, second) = (slash(&x, (0xA1, 0xB2)), slash(&x, (0xC3, 0xD4)));
    assert!(
        r.add(&first),
        "PRECONDITION: the first slash for X is admitted"
    );
    let refused = r.pool.add_transaction(second.clone(), &r.utxo, 0, 0);
    assert!(
        refused.is_err(),
        "a second slash for a resident accused must be refused; got {refused:?}"
    );
    assert!(
        !r.pool.contains(&second.hash()),
        "the second slash must not be resident"
    );
    assert!(
        r.add(&slash(&y, (0xA1, 0xB2))),
        "NON-VACUITY: another accused is admitted"
    );
    r.pool.remove_transaction(&first.hash());
    assert!(
        r.add(&second),
        "NON-VACUITY: with X's slash gone the second evidence is admissible"
    );
}

// REQ-SLASH-004 — Decision: a failure means a resident slash survives its accused's queued slash and is re-offered to the builder every slot.
#[test]
fn inc_i_255_m2_mempool_revalidate_evicts_queued_accused() {
    let x = KeyPair::generate();
    let mut r = rig(&[&x]);
    let tx = slash(&x, (0xA1, 0xB2));
    assert!(r.add(&tx), "PRECONDITION: resident");
    r.revalidate();
    assert!(
        r.pool.contains(&tx.hash()),
        "NON-VACUITY: kept while the accused is unchanged"
    );
    r.producers
        .try_write()
        .expect("FIXTURE: lock")
        .queue_update(PendingProducerUpdate::Slash {
            pubkey: *x.public_key(),
            height: 0,
        });
    r.revalidate();
    assert!(
        !r.pool.contains(&tx.hash()),
        "revalidate must evict a slash for a queued accused"
    );
}

// REQ-SLASH-004 — Decision: a failure means a resident slash survives its accused becoming Slashed and is re-offered to the builder every slot.
#[test]
fn inc_i_255_m2_mempool_revalidate_evicts_slashed_accused() {
    let x = KeyPair::generate();
    let mut r = rig(&[&x]);
    let tx = slash(&x, (0xA1, 0xB2));
    assert!(r.add(&tx), "PRECONDITION: resident");
    r.producers
        .try_write()
        .expect("FIXTURE: lock")
        .slash_producer(x.public_key(), 0)
        .expect("FIXTURE: slash X");
    r.revalidate();
    assert!(
        !r.pool.contains(&tx.hash()),
        "revalidate must evict a slash for a Slashed accused"
    );
}

const GATE_ENV: &str = "DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "INC_I_255_M2_MEMPOOL_CHILD";
const RAISED_GATE: u64 = 2;

fn in_child() -> bool {
    std::env::var_os(CHILD_ENV).is_some()
}

fn run_in_child(test: &str) {
    let out = std::process::Command::new(std::env::current_exe().expect("current exe"))
        .args([test, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1")
        .env(GATE_ENV, RAISED_GATE.to_string())
        .env(
            "DOLI_INC_I_257_UNBONDING_ACTIVATION_HEIGHT",
            u64::MAX.to_string(),
        )
        .output()
        .expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "child {test} failed or did not run:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn queue_slash(producers: &tokio::sync::RwLock<ProducerSet>, kp: &KeyPair) {
    producers
        .try_write()
        .expect("FIXTURE: lock")
        .queue_update(PendingProducerUpdate::Slash {
            pubkey: *kp.public_key(),
            height: 0,
        });
}

fn undecodable(kp: &KeyPair) -> Transaction {
    let mut tx = slash(kp, (0xA1, 0xB2));
    tx.extra_data = vec![0xFF; 4];
    assert!(tx.slash_data().is_none(), "FIXTURE: undecodable slash data");
    tx
}

fn decode_refusal(r: &Result<AddTransactionResult, MempoolError>) -> bool {
    matches!(r, Err(MempoolError::Validation(ValidationError::InvalidSlash(m))) if m.contains("invalid slash data"))
}

// REQ-SLASH-004 — Decision: a failure means revalidation below the slashing gate evicts slashes on producer-set state the gate has not armed.
#[test]
fn inc_i_255_m2_mempool_revalidate_keeps_slash_below_gate() {
    if !in_child() {
        run_in_child(
            "inc_i_255_m2_slash_residency::inc_i_255_m2_mempool_revalidate_keeps_slash_below_gate",
        );
        return;
    }
    let gate = Network::Devnet
        .params()
        .effective_slashing_activation_height();
    assert_eq!(gate, RAISED_GATE, "FIXTURE: env-raised slash gate");
    let x = KeyPair::generate();
    let mut r = rig(&[&x]);
    let tx = slash(&x, (0xA1, 0xB2));
    r.pool
        .add_transaction(tx.clone(), &r.utxo, gate - 1, 0)
        .expect("PRECONDITION: admitted when best + 1 reaches the gate");
    queue_slash(&r.producers, &x);

    r.pool.revalidate(&r.utxo, gate - 2, 0);
    assert!(
        r.pool.contains(&tx.hash()),
        "below the gate revalidate must keep a slash whose accused is queued"
    );
    r.pool.revalidate(&r.utxo, gate - 1, 0);
    assert!(
        !r.pool.contains(&tx.hash()),
        "NON-VACUITY: at the gate the same slash is evicted"
    );
}

fn bonded_withdrawal(utxo: &mut UtxoSet, kp: &KeyPair, tag: u8) -> Transaction {
    let pk = *kp.public_key();
    let owner = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes());
    let bond = Network::Devnet.bond_unit();
    let prev = Hash::from_bytes([tag; 32]);
    let entry = UtxoEntry {
        output: Output::bond(bond, owner, u64::MAX, 0),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    utxo.insert(Outpoint::new(prev, 0), entry)
        .expect("FIXTURE: seed Bond UTXO");
    let mut input = Input::new(prev, 0);
    input.public_key = Some(pk);
    let dest = crypto::hash::hash(b"inc-i-255-m2-withdrawal-destination");
    let mut tx = Transaction::new_request_withdrawal(vec![input], pk, 1, dest, bond - bond / 100);
    let signing_hash = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&signing_hash, kp.private_key());
    // INC-I-182: the devnet producer-auth gate is 0, so the producer signs the payload.
    let genesis = ConsensusParams::devnet().genesis_hash;
    doli_core::transaction::sign_producer_auth(&mut tx, &genesis, kp);
    tx
}

// REQ-SLASH-003 — Decision: a failure means admission refuses a withdrawal for slashed status before the slashing gate arms that rule, or stops refusing it once armed.
#[test]
fn inc_i_255_m2_mempool_withdrawal_by_slashed_producer_admitted_below_gate() {
    if !in_child() {
        run_in_child(
            "inc_i_255_m2_slash_residency::inc_i_255_m2_mempool_withdrawal_by_slashed_producer_admitted_below_gate",
        );
        return;
    }
    let gate = Network::Devnet
        .params()
        .effective_slashing_activation_height();
    assert_eq!(gate, RAISED_GATE, "FIXTURE: env-raised slash gate");
    let (queued, slashed) = (KeyPair::generate(), KeyPair::generate());
    let mut r = rig(&[&queued, &slashed]);
    queue_slash(&r.producers, &queued);
    r.producers
        .try_write()
        .expect("FIXTURE: lock")
        .slash_producer(slashed.public_key(), 0)
        .expect("FIXTURE: slash");

    for (kp, tag) in [(&queued, 0xE0), (&slashed, 0xE1)] {
        let tx = bonded_withdrawal(&mut r.utxo, kp, tag);
        let below = r.pool.add_transaction(tx.clone(), &r.utxo, gate - 2, 0);
        assert!(
            below.is_ok() && r.pool.contains(&tx.hash()),
            "below the gate the withdrawal must be admitted; got {below:?}"
        );
        r.pool.remove_transaction(&tx.hash());
        let at = r.pool.add_transaction(tx.clone(), &r.utxo, gate - 1, 0);
        assert!(
            matches!(&at, Err(MempoolError::InvalidTransaction(m)) if m.contains("slashed or slash-queued")),
            "NON-VACUITY: at the gate the same withdrawal is refused; got {at:?}"
        );
        assert!(!r.pool.contains(&tx.hash()));
    }
}

// REQ-SLASH-004 — Decision: a failure means a mempool with no live producer set panics or drops slashes, or loses the pool-local residency rule.
#[test]
fn inc_i_255_m2_mempool_no_live_set_fails_open() {
    let x = KeyPair::generate();
    let mut pool = Mempool::new(
        MempoolPolicy::testnet(),
        ConsensusParams::devnet(),
        Network::Devnet,
    );
    let utxo = UtxoSet::new();
    let (first, second) = (slash(&x, (0xA1, 0xB2)), slash(&x, (0xC3, 0xD4)));
    pool.add_transaction(first.clone(), &utxo, 0, 0)
        .expect("with no live producer set the state check fails open");
    let refused = pool.add_transaction(second.clone(), &utxo, 0, 0);
    assert!(
        refused.is_err(),
        "residency is pool-local and must still refuse; got {refused:?}"
    );
    pool.revalidate(&utxo, 0, 0);
    assert!(
        pool.contains(&first.hash()),
        "with no live producer set revalidate must keep the slash"
    );
    assert!(!pool.contains(&second.hash()));
}

// REQ-SLASH-004 — Decision: a failure means a busy producer-set lock evicts or blocks on resident slashes, or lets a second slash in.
#[test]
fn inc_i_255_m2_mempool_busy_live_set_fails_open() {
    let x = KeyPair::generate();
    let mut r = rig(&[&x]);
    let (first, second) = (slash(&x, (0xA1, 0xB2)), slash(&x, (0xC3, 0xD4)));
    assert!(r.add(&first), "PRECONDITION: resident");
    queue_slash(&r.producers, &x);
    let producers = r.producers.clone();
    let busy = producers.try_write().expect("FIXTURE: hold the write lock");

    r.revalidate();
    assert!(
        r.pool.contains(&first.hash()),
        "a busy producer-set lock must not evict (fail open)"
    );
    let refused = r.pool.add_transaction(second.clone(), &r.utxo, 0, 0);
    assert!(
        refused.is_err(),
        "residency must refuse a second slash while the lock is busy; got {refused:?}"
    );

    drop(busy);
    r.revalidate();
    assert!(
        !r.pool.contains(&first.hash()),
        "NON-VACUITY: with the lock free the queued accused's slash is evicted"
    );
}

// REQ-SLASH-004 — Decision: a failure means a slash whose data does not decode reaches residency instead of the decode refusal.
#[test]
fn inc_i_255_m2_mempool_undecodable_slash_refused_before_residency_check() {
    let x = KeyPair::generate();
    let mut r = rig(&[&x]);
    let bad = undecodable(&x);
    let with_set = r.pool.add_transaction(bad.clone(), &r.utxo, 0, 0);
    assert!(
        decode_refusal(&with_set),
        "live set shared: got {with_set:?}"
    );
    let mut bare = Mempool::new(
        MempoolPolicy::testnet(),
        ConsensusParams::devnet(),
        Network::Devnet,
    );
    let without_set = bare.add_transaction(bad.clone(), &r.utxo, 0, 0);
    assert!(
        decode_refusal(&without_set),
        "no live set: got {without_set:?}"
    );
    assert!(!r.pool.contains(&bad.hash()) && !bare.contains(&bad.hash()));
    assert!(
        r.add(&slash(&x, (0xA1, 0xB2))),
        "NON-VACUITY: the decodable slash is admitted"
    );
}
