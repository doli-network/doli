//! INC-I-262 M2 / R-4 — below the effective slashing height, block validation
//! and the block builder build no slash target facts; at/above the gate they
//! build the same facts as a direct `slash_targets` call. `NetworkParams` is
//! cached per process, so each below-gate body runs in a re-exec'd child.
//!
// covers: validation_checks assembly slash_admission slash_facts
//
// OUTPUT CONTRACT: fn Node::validate_block_for_apply(&self, &Block, u64, ValidationMode) -> Result<(), ValidationError>
//   O1 return (verdict) for a block holding a SlashProducer (and an AddBond)
//   O2 RocksDB per-thread perf VISITS (memtable gets + seeks + nexts) on the calling thread during the call
// OUTPUT CONTRACT: fn Node::build_block_content(&mut self, Hash, u32, u64, u32, PublicKey) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>
//   O1 selected transactions; O2 VISITS as above
// OUTPUT CONTRACT: fn Mempool::revalidate(&mut self, &UtxoSet, u64, u32) / fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O3 resident set after revalidation; apply return and chain tip
// PATHS: P-BELOW slash gate far above the tip | P-BAND slash gate 1, block-signature gate far above | P-AT devnet default
// INPUT PARTITIONS:
//   slash accused: registered producer owning OWNER_UTXOS UTXOs | unregistered stranger (no owner lookup on any path)
//   producer state: clean | slash-queued (accused and an AddBond producer)
// MATRIX:
//   O2,O1 x validate Full x P-BELOW -> inc_i_262_m2_below_gate_validation_builds_no_slash_facts
//   O2,O1 x validate Full x P-BAND -> inc_i_262_m2_in_band_validation_builds_no_slash_facts
//   O2,O1 x builder x P-BELOW -> inc_i_262_m2_below_gate_builder_builds_no_slash_facts
//   O2,O1 x {validate Full, builder} x P-AT -> inc_i_262_m2_at_gate_sites_build_direct_facts
//   O1,O3 x {validate Full, apply Full, revalidate} x {P-BELOW, P-BAND} + P-AT control -> inc_i_262_m2_below_gate_and_band_verdicts_unchanged

use std::future::Future;
use std::os::raw::{c_int, c_void};

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::transaction::{Input, Output, Transaction};
use doli_core::validation::{bond_owner, ValidationError, ValidationMode};
use doli_core::Network;
use doli_node::node::Node;
use mempool::slash_admission::slash_targets;
use storage::{Outpoint, PendingProducerUpdate, UtxoEntry};

use super::inc_i_178_m0_common::wait_for_fresh_second;
use super::inc_i_255_m1_common::{
    admit, canonical, chain_genesis, full_block_with, inject, pair, tip, valid_slash, world, World,
};
use super::inc_i_255_m2_common::producer_auth;
use super::inc_i_255_m4_common::{resident, revalidate_at};
use super::inc_i_255_m6_common::seed_coin;

const OWNER_UTXOS: u32 = 2_000;
const FAR: u64 = 1_000_000_000;
const FEE: u64 = 100_000;
const LOCK: u64 = 10_000_000;
const SLASH_GATE_ENV: &str = "DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT";
const SIG_GATE_ENV: &str = "DOLI_INC_I_236_BLOCK_SIGNATURE_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "INC_I_262_M2_FACTS_CHILD";
const MOD: &str = "inc_i_262_m2_gated_slash_facts::";
const BELOW: [(&str, u64); 1] = [(SLASH_GATE_ENV, FAR)];
const BAND: [(&str, u64); 2] = [(SLASH_GATE_ENV, 1), (SIG_GATE_ENV, FAR)];

unsafe extern "C" {
    fn rocksdb_set_perf_level(level: c_int);
    fn rocksdb_perfcontext_create() -> *mut c_void;
    fn rocksdb_perfcontext_reset(ctx: *mut c_void);
    fn rocksdb_perfcontext_metric(ctx: *mut c_void, metric: c_int) -> u64;
    fn rocksdb_perfcontext_destroy(ctx: *mut c_void);
}

const PERF_DISABLE: c_int = 1;
const PERF_ENABLE_COUNT: c_int = 2;
/// get_from_memtable_count, seek_on_memtable_count, next_on_memtable_count.
const VISIT_METRICS: [c_int; 3] = [16, 20, 21];

/// RocksDB per-thread perf context, counting from `start` to `stop`.
struct Perf(*mut c_void);

impl Perf {
    fn start() -> Self {
        // SAFETY: C API of the RocksDB library this binary links; the context is thread-local.
        unsafe {
            rocksdb_set_perf_level(PERF_ENABLE_COUNT);
            let ctx = rocksdb_perfcontext_create();
            rocksdb_perfcontext_reset(ctx);
            Self(ctx)
        }
    }

    fn stop(self) -> u64 {
        // SAFETY: `self.0` is a live context from `start`.
        let n = VISIT_METRICS
            .iter()
            .map(|m| unsafe { rocksdb_perfcontext_metric(self.0, *m) })
            .sum();
        unsafe { rocksdb_set_perf_level(PERF_DISABLE) };
        n
    }
}

impl Drop for Perf {
    fn drop(&mut self) {
        // SAFETY: created by `rocksdb_perfcontext_create`, destroyed once.
        unsafe { rocksdb_perfcontext_destroy(self.0) };
    }
}

async fn counted<T>(fut: impl Future<Output = T>) -> (u64, T) {
    let p = Perf::start();
    let out = fut.await;
    (p.stop(), out)
}

fn counted_sync<T>(f: impl FnOnce() -> T) -> (u64, T) {
    let p = Perf::start();
    let out = f();
    (p.stop(), out)
}

fn in_child() -> bool {
    std::env::var_os(CHILD_ENV).is_some()
}

/// Re-exec `test` alone with `env` set; returns the child's stdout.
fn run_child(test: &str, env: &[(&str, u64)]) -> String {
    let mut cmd = std::process::Command::new(std::env::current_exe().expect("current exe"));
    cmd.args([test, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1")
        .env(
            "DOLI_INC_I_257_UNBONDING_ACTIVATION_HEIGHT",
            u64::MAX.to_string(),
        );
    for (k, v) in env {
        cmd.env(k, v.to_string());
    }
    let out = cmd.output().expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "child {test} with {env:?} failed or did not run:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

fn assert_gate(want: u64) {
    assert_eq!(
        Network::Devnet
            .params()
            .effective_slashing_activation_height(),
        want,
        "PRECONDITION: the env moves the devnet effective slashing height"
    );
}

/// `OWNER_UTXOS` Normal UTXOs owned by `bond_owner(pk)` in the node's RocksDB set.
async fn seed_owner_index(node: &Node, pk: &PublicKey) {
    let owner = bond_owner(pk);
    let mut utxo = node.utxo_set.write().await;
    assert!(
        utxo.is_rocksdb(),
        "FIXTURE: production RocksDB UTXO backend"
    );
    let held = utxo.get_by_pubkey_hash(&owner).len();
    for i in 0..OWNER_UTXOS {
        let mut h = [0xE7u8; 32];
        h[..4].copy_from_slice(&i.to_le_bytes());
        let entry = UtxoEntry {
            output: Output::normal(1, owner),
            height: 0,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        utxo.insert(Outpoint::new(Hash::from_bytes(h), 0), entry)
            .expect("FIXTURE: seed owner UTXO");
    }
    assert_eq!(
        utxo.get_by_pubkey_hash(&owner).len(),
        held + OWNER_UTXOS as usize,
        "FIXTURE: owner index holds the seeded entries"
    );
}

fn stranger_slash() -> Transaction {
    let stranger = KeyPair::generate();
    let (a, b) = pair(&stranger, chain_genesis());
    canonical(*stranger.public_key(), a, b)
}

/// VISITS of a direct `slash_targets` call over `named` minus over `base`, and
/// the number of facts it builds for `named`.
async fn direct_cost(
    node: &Node,
    height: u64,
    named: &[Transaction],
    base: &[Transaction],
) -> (u64, usize) {
    let ps = node.producer_set.read().await;
    let utxo = node.utxo_set.read().await;
    let (vn, facts) = counted_sync(|| slash_targets(&ps, &utxo, height, named.iter()));
    let (vb, _) = counted_sync(|| slash_targets(&ps, &utxo, height, base.iter()));
    (vn.saturating_sub(vb), facts.len())
}

#[derive(Debug)]
struct Probe {
    named: u64,
    base: u64,
    direct: u64,
    facts: usize,
    verdicts: (String, String),
}

/// Full-mode validation of a height-1 block holding a slash on the accused
/// (`named`) and one holding a slash on a stranger (`base`).
async fn validation_probe(w: &mut World) -> Probe {
    seed_owner_index(&w.node, &w.accused_pk()).await;
    let slash = valid_slash(w);
    let (named, _) = full_block_with(&mut w.node, &[slash]).await;
    let (base, _) = full_block_with(&mut w.node, &[stranger_slash()]).await;
    let (direct, facts) = direct_cost(&w.node, 1, &named.transactions, &base.transactions).await;
    for b in [&named, &base] {
        let _ = w
            .node
            .validate_block_for_apply(b, 1, ValidationMode::Full)
            .await;
    }
    let (vn, rn) = counted(
        w.node
            .validate_block_for_apply(&named, 1, ValidationMode::Full),
    )
    .await;
    let (vb, rb) = counted(
        w.node
            .validate_block_for_apply(&base, 1, ValidationMode::Full),
    )
    .await;
    Probe {
        named: vn,
        base: vb,
        direct,
        facts,
        verdicts: (format!("{rn:?}"), format!("{rb:?}")),
    }
}

/// One counted `build_block_content` for the next height; selected hashes.
async fn build_counted(node: &mut Node) -> (u64, Vec<Hash>) {
    let pk = *node
        .producer_key
        .as_ref()
        .expect("FIXTURE: producer key")
        .public_key();
    let (prev_hash, prev_slot, height) = {
        let st = node.chain_state.read().await;
        (st.best_hash, st.best_slot, st.best_height + 1)
    };
    for _ in 0..8 {
        let slot = node.params.timestamp_to_slot(wait_for_fresh_second());
        let (n, built) =
            counted(node.build_block_content(prev_hash, prev_slot, height, slot, pk)).await;
        if let Some((_, txs, _)) = built.expect("FIXTURE: build_block_content returned Err") {
            return (n, txs.iter().map(|t| t.hash()).collect());
        }
    }
    panic!("FIXTURE: build_block_content returned None on every attempt");
}

/// Builder VISITS with a resident slash on a stranger, then on the accused.
async fn builder_probe(w: &mut World) -> Probe {
    seed_owner_index(&w.node, &w.accused_pk()).await;
    let (named, base) = (valid_slash(w), stranger_slash());
    assert!(
        inject(&w.node, &base).await,
        "PRECONDITION: the stranger's slash is resident"
    );
    let _ = build_counted(&mut w.node).await;
    let (vb, sel_b) = build_counted(&mut w.node).await;
    w.node
        .mempool
        .write()
        .await
        .remove_transaction(&base.hash());
    assert!(
        inject(&w.node, &named).await,
        "PRECONDITION: the accused's slash is resident"
    );
    let (vn, sel_n) = build_counted(&mut w.node).await;
    let h = tip(&w.node).await.0 + 1;
    let (direct, facts) = direct_cost(
        &w.node,
        h,
        std::slice::from_ref(&named),
        std::slice::from_ref(&base),
    )
    .await;
    Probe {
        named: vn,
        base: vb,
        direct,
        facts,
        verdicts: (
            format!("selected={}", sel_n.contains(&named.hash())),
            format!("selected={}", sel_b.contains(&base.hash())),
        ),
    }
}

async fn register(node: &Node, pk: &PublicKey) {
    let unit = node.config.network.bond_unit();
    node.producer_set
        .write()
        .await
        .register_genesis_producer(*pk, 1, unit)
        .expect("FIXTURE: register producer");
}

async fn mark_queued(node: &Node, pk: &PublicKey) {
    let h = tip(node).await.0;
    node.producer_set
        .write()
        .await
        .queue_update(PendingProducerUpdate::Slash {
            pubkey: *pk,
            height: h,
        });
}

/// Producer-signed AddBond by `kp`, paid from a fresh coin at `[tag; 32]:0`.
async fn add_bond(node: &Node, kp: &KeyPair, tag: u8) -> Transaction {
    let pk = *kp.public_key();
    let unit = node.config.network.bond_unit();
    let op = seed_coin(node, &pk, tag, unit + FEE).await;
    let mut input = Input::new(op.tx_hash, op.index);
    input.public_key = Some(pk);
    let mut tx = Transaction::new_add_bond(vec![input], pk, 1, unit, LOCK);
    let msg = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&msg, kp.private_key());
    producer_auth(tx, kp)
}

/// Resident slash on the accused and AddBond by a third producer, both made
/// slash-queued, then one revalidation; (slash resident, AddBond resident).
async fn eviction_probe() -> (bool, bool) {
    let w = world().await;
    let third = KeyPair::generate();
    register(&w.node, third.public_key()).await;
    let bond = add_bond(&w.node, &third, 0x47).await;
    let r = admit(&w.node, &bond).await;
    assert!(
        r.is_ok(),
        "PRECONDITION: the clean AddBond is admitted; got {r:?}"
    );
    let slash = valid_slash(&w);
    assert!(
        inject(&w.node, &slash).await,
        "PRECONDITION: the slash is resident"
    );
    mark_queued(&w.node, &w.accused_pk()).await;
    mark_queued(&w.node, third.public_key()).await;
    let h = tip(&w.node).await.0;
    revalidate_at(&w.node, h).await;
    (
        resident(&w.node, &slash).await,
        resident(&w.node, &bond).await,
    )
}

/// Below-gate verdicts: Full validation of [slash, AddBond by a slash-queued
/// producer] is Ok, Full apply of [AddBond] is Ok and moves the tip, and a
/// revalidation keeps both resident txs.
async fn verdict_body() {
    let mut w = world().await;
    let third = KeyPair::generate();
    register(&w.node, third.public_key()).await;
    mark_queued(&w.node, third.public_key()).await;
    let bond = add_bond(&w.node, &third, 0x48).await;
    let slash = valid_slash(&w);
    let (mixed, _) = full_block_with(&mut w.node, &[slash, bond.clone()]).await;
    let r = w
        .node
        .validate_block_for_apply(&mixed, 1, ValidationMode::Full)
        .await;
    assert!(
        r.is_ok(),
        "below the gate Full validation stays Ok; got {r:?}"
    );
    let (block, _) = full_block_with(&mut w.node, &[bond]).await;
    let r = w
        .node
        .apply_block(block.clone(), ValidationMode::Full)
        .await;
    assert!(
        r.is_ok(),
        "below the gate Full apply of the AddBond stays Ok; got {r:?}"
    );
    assert_eq!(
        tip(&w.node).await,
        (1, block.hash()),
        "Full apply moves the tip"
    );
    let kept = eviction_probe().await;
    assert_eq!(
        kept,
        (true, true),
        "below the gate revalidation evicts neither (slash, AddBond)"
    );
}

// REQ-DEF-004 — Decision: a failure means block validation below the slashing gate still walks the accused's owner index (and takes the producer-set and UTXO locks) for facts no verdict reads there.
#[tokio::test]
async fn inc_i_262_m2_below_gate_validation_builds_no_slash_facts() {
    if !in_child() {
        let test = format!("{MOD}inc_i_262_m2_below_gate_validation_builds_no_slash_facts");
        run_child(&test, &BELOW);
        return;
    }
    assert_gate(FAR);
    let p = validation_probe(&mut world().await).await;
    println!("MEASURED validate below-gate {p:?}");
    assert_validation_below(&p);
}

fn assert_validation_below(p: &Probe) {
    assert_eq!(
        p.facts, 1,
        "NON-VACUITY: a direct slash_targets call builds the accused's fact"
    );
    assert!(
        p.direct >= u64::from(OWNER_UTXOS),
        "NON-VACUITY: a direct slash_targets call reads the owner index: {p:?}"
    );
    assert_eq!(
        p.verdicts,
        ("Ok(())".to_string(), "Ok(())".to_string()),
        "below the gate Full validation stays Ok for both blocks"
    );
    assert_eq!(
        p.named, p.base,
        "below the gate validate_block_for_apply must build no slash facts: VISITS with the accused's slash {} vs a stranger's {} (a direct slash_targets call costs {}); {p:?}",
        p.named, p.base, p.direct
    );
}

// REQ-DEF-004 — Decision: a failure means the fact gate compares the slashing height alone, so the band below the block-signature gate still pays the owner lookup.
#[tokio::test]
async fn inc_i_262_m2_in_band_validation_builds_no_slash_facts() {
    if !in_child() {
        let test = format!("{MOD}inc_i_262_m2_in_band_validation_builds_no_slash_facts");
        run_child(&test, &BAND);
        return;
    }
    assert_gate(FAR);
    assert_eq!(
        Network::Devnet
            .params()
            .inc_i_255_slashing_activation_height,
        1,
        "PRECONDITION: the slashing height alone is below the probe height"
    );
    let p = validation_probe(&mut world().await).await;
    println!("MEASURED validate in-band {p:?}");
    assert_validation_below(&p);
}

// REQ-DEF-004 — Decision: a failure means every block build below the slashing gate still walks the owner index of each resident slash's accused under the producer-set and UTXO locks.
#[tokio::test]
async fn inc_i_262_m2_below_gate_builder_builds_no_slash_facts() {
    if !in_child() {
        let test = format!("{MOD}inc_i_262_m2_below_gate_builder_builds_no_slash_facts");
        run_child(&test, &BELOW);
        return;
    }
    assert_gate(FAR);
    let p = builder_probe(&mut world().await).await;
    println!("MEASURED builder below-gate {p:?}");
    assert_eq!(
        p.facts, 1,
        "NON-VACUITY: a direct slash_targets call builds the accused's fact"
    );
    assert!(
        p.direct >= u64::from(OWNER_UTXOS),
        "NON-VACUITY: a direct slash_targets call reads the owner index: {p:?}"
    );
    assert_eq!(
        p.verdicts,
        ("selected=false".to_string(), "selected=false".to_string()),
        "below the gate the builder keeps skipping both slashes"
    );
    assert_eq!(
        p.named, p.base,
        "below the gate build_block_content must build no slash facts: VISITS with the accused's slash resident {} vs a stranger's {} (a direct slash_targets call costs {}); {p:?}",
        p.named, p.base, p.direct
    );
}

// REQ-DEF-004 — Decision: a failure means the gate dropped or changed the facts at/above the slashing height, so a slash verdict that reads them changes on live blocks.
#[tokio::test]
async fn inc_i_262_m2_at_gate_sites_build_direct_facts() {
    let gate = Network::Devnet
        .params()
        .effective_slashing_activation_height();
    assert!(
        gate <= 1,
        "PRECONDITION: devnet default gate at or below height 1; got {gate}"
    );
    let v = validation_probe(&mut world().await).await;
    println!("MEASURED validate at-gate {v:?}");
    assert!(
        v.facts == 1 && v.direct >= u64::from(OWNER_UTXOS),
        "NON-VACUITY: the direct call reads the owner index: {v:?}"
    );
    assert_eq!(
        v.named - v.base,
        v.direct,
        "at the gate validation must build exactly the facts a direct slash_targets call builds: {v:?}"
    );
    let b = builder_probe(&mut world().await).await;
    println!("MEASURED builder at-gate {b:?}");
    assert!(
        b.facts == 1 && b.direct >= u64::from(OWNER_UTXOS),
        "NON-VACUITY: the direct call reads the owner index: {b:?}"
    );
    assert_eq!(
        b.named - b.base,
        b.direct,
        "at the gate the builder must build exactly the facts a direct slash_targets call builds: {b:?}"
    );

    let mut w = world().await;
    mark_queued(&w.node, &w.accused_pk()).await;
    let slash = valid_slash(&w);
    let (block, _) = full_block_with(&mut w.node, &[slash]).await;
    let r = w
        .node
        .validate_block_for_apply(&block, 1, ValidationMode::Full)
        .await;
    assert!(
        matches!(&r, Err(ValidationError::InvalidSlash(m)) if m == "slash target already has a pending slash"),
        "at the gate a slash on a slash-queued accused reads its fact; got {r:?}"
    );
}

// REQ-DEF-004 — Decision: a failure means gating the facts changed a below-gate verdict on validation, apply or mempool eviction, a consensus-history break.
#[tokio::test]
async fn inc_i_262_m2_below_gate_and_band_verdicts_unchanged() {
    if in_child() {
        verdict_body().await;
        return;
    }
    let test = format!("{MOD}inc_i_262_m2_below_gate_and_band_verdicts_unchanged");
    run_child(&test, &BELOW);
    run_child(&test, &BAND);
    let evicted = eviction_probe().await;
    assert_eq!(
        evicted,
        (false, false),
        "NON-VACUITY: at the devnet default gate revalidation evicts both (slash, AddBond)"
    );
}
