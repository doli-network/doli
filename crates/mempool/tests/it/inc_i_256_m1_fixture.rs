//! Shared rig for the `inc_i_256_m1_*` mempool tests.
//!
//! OUTPUT CONTRACT: N/A — fixture file (builders only, no assertions on the code under test).
//! INPUT PARTITIONS: N/A — fixture file.

use std::sync::Arc;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::block::{compute_merkle_root, BlockBuilder, BlockHeader};
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::tpop::heartbeat::hash_chain_vdf;
use doli_core::transaction::{Input, Output, SlashData, Transaction};
use mempool::{Mempool, MempoolPolicy};
use storage::{Outpoint, ProducerSet, UtxoEntry, UtxoSet};

pub(crate) const SLOT: u32 = 7;
pub(crate) const TIP: u64 = 5;
pub(crate) const COIN: u64 = 5_000_000;
pub(crate) const LOW_FEE: u64 = 1_000;
pub(crate) const HIGH_FEE: u64 = 100_000;

pub(crate) struct Rig {
    pub(crate) pool: Mempool,
    pub(crate) producers: Arc<tokio::sync::RwLock<ProducerSet>>,
    pub(crate) utxo: UtxoSet,
}

pub(crate) fn addr(pk: &PublicKey) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes())
}

pub(crate) fn policy(max_count: usize, max_age: u64) -> MempoolPolicy {
    MempoolPolicy {
        max_count,
        max_age,
        ..MempoolPolicy::testnet()
    }
}

/// Devnet pool sharing a live producer set where each `accused` is Active.
pub(crate) fn rig(policy: MempoolPolicy, accused: &[&KeyPair]) -> Rig {
    let mut ps = ProducerSet::new();
    for kp in accused {
        ps.register_genesis_producer(*kp.public_key(), 1, Network::Devnet.bond_unit())
            .expect("FIXTURE: register accused");
    }
    let producers = Arc::new(tokio::sync::RwLock::new(ps));
    let mut pool = Mempool::new(policy, ConsensusParams::devnet(), Network::Devnet);
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

/// Signed double-production evidence for `kp`; `salts` pick the header pair.
pub(crate) fn slash(kp: &KeyPair, salts: (u8, u8)) -> Transaction {
    let data = SlashData::double_production(
        *kp.public_key(),
        evidence(kp, salts.0),
        evidence(kp, salts.1),
    );
    Transaction::new_slash_producer(data)
}

pub(crate) fn seed(utxo: &mut UtxoSet, at: Outpoint, output: Output) {
    let entry = UtxoEntry {
        output,
        height: 0,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    utxo.insert(at, entry).expect("FIXTURE: seed UTXO");
}

/// A signed Transfer of a fresh `COIN` Normal output (payer from `tag`) paying `fee`.
pub(crate) fn funded_transfer(utxo: &mut UtxoSet, tag: u8, fee: u64) -> Transaction {
    let kp = KeyPair::from_seed([tag; 32]);
    let own = addr(kp.public_key());
    let from = Outpoint::new(Hash::from_bytes([tag; 32]), 0);
    seed(utxo, from, Output::normal(COIN, own));
    let mut input = Input::new(from.tx_hash, from.index);
    input.public_key = Some(*kp.public_key());
    let mut tx = Transaction::new_transfer(vec![input], vec![Output::normal(COIN - fee, own)]);
    let msg = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&msg, kp.private_key());
    tx
}

/// Parent: re-exec `module::test` alone with `env` set and return true. Child: false.
pub(crate) fn spawned_child(path: &str, child_env: &str, env: &[(&str, u64)]) -> bool {
    if std::env::var_os(child_env).is_some() {
        return false;
    }
    let mut cmd = std::process::Command::new(std::env::current_exe().expect("current exe"));
    cmd.arg(path)
        .args(["--exact", "--test-threads=1", "--nocapture"])
        .env(child_env, "1");
    for (k, v) in env {
        cmd.env(k, v.to_string());
    }
    let out = cmd.output().expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "{path}: child failed or did not run:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    true
}
