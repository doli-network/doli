//! B3 M2 / R-5 (INC-I-261) — `doli-node check-producer-set` replays a STOPPED
//! node's block store through the headless Replay path into a scratch directory
//! and compares the rebuilt ProducerSet with the persisted one. Read-only.
//!
// covers: cli.rs main.rs chain.rs producer_set_check.rs operations/mod.rs commands/mod.rs lib.rs troubleshooting.md cli.md
//
// OUTPUT CONTRACT: fn `doli-node --network devnet --data-dir <D> --log-level error check-producer-set`
//   O3 exit code: 0 EQUAL | 2 DIFFERS | 3 NOT-CHECKABLE | 1 error
//   O6 stdout: exactly ONE line equal to EQUAL | DIFFERS | NOT-CHECKABLE; the NEXT line is key=value:
//      EQUAL         height=<h> producers=<n> pending=<k>
//      DIFFERS       height=<h> producers=<p>/<r> status_mismatches=<n> pending=<p>/<r>  (p persisted, r rebuilt)
//      NOT-CHECKABLE reason=snap_sync_height | holed_block_store | lock_held
//   O4 data dir: every file byte-identical, none added or removed
//   O4 scratch: nothing left under the child's TMPDIR (std::env::temp_dir())
// PATHS: P-EQ clean store | P-DIFF residue | P-NC snap-synced | P-NC lock held
// INPUT PARTITIONS:
//   clean: devnet tip 42 (genesis window 40 + 2), 1 genesis producer, 1 queued Register (mid-epoch)
//   residue: one extra queued Exit (pending k+1/k) | one producer Active -> Exited (status 1, pending k/k)
//   not checkable: persisted snap_sync_height = Some(20) | state_db and blocks held open by this process
// MATRIX: O3+O6 x {clean, pending residue, status residue, snap, lock} + O4 x {clean, pending residue, snap}
//   = 6 tests; every fixture is certified by an in-process Replay of the same blocks (PRECONDITION)

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crypto::{BlsKeyPair, KeyPair, PublicKey};
use doli_core::transaction::{Input, Transaction};
use doli_core::validation::ValidationMode;
use doli_core::{Block, Network};
use doli_node::node::Node;
use storage::{ChainState, PendingProducerUpdate, ProducerSet, ProducerStatus, StateDb};
use tempfile::TempDir;

use super::inc_i_255_m2_common::producer_auth;
use crate::inc_i_156_m1_harness::{build_block_with_txs, coinbase_for};

const GENESIS: u64 = 40;
const EPOCH: u64 = 4;
pub(super) const TIP: u64 = GENESIS + 2;
const SNAP_H: u64 = 20;
const VERDICTS: [&str; 3] = ["EQUAL", "DIFFERS", "NOT-CHECKABLE"];

pub(super) struct Fx {
    _tmp: TempDir,
    pub(super) data: PathBuf,
    pub(super) scratch_root: PathBuf,
    producer: PublicKey,
    pub(super) producers: usize,
    pub(super) pending: usize,
}

pub(super) struct Run {
    pub(super) code: Option<i32>,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

/// Producer-signed Registration with a BLS key + PoP. `bonds == 0` is the
/// genesis-window VDF-proof container the completion derives producers from.
fn registration(kp: &KeyPair, bls_seed: u8, unit: u64, bonds: u32) -> Transaction {
    let pk = *kp.public_key();
    let inputs = if bonds == 0 {
        Vec::new()
    } else {
        let mut i = Input::new(crypto::Hash::from_bytes([bls_seed; 32]), 0);
        i.public_key = Some(pk);
        vec![i]
    };
    let mut tx =
        Transaction::new_registration(inputs, pk, unit * u64::from(bonds), u64::MAX, bonds);
    let mut d = tx.registration_data().expect("FIXTURE: registration data");
    let bls = BlsKeyPair::from_seed(&[bls_seed; 32]).expect("FIXTURE: BLS seed");
    d.bls_pubkey = bls.public_key().as_bytes().to_vec();
    d.bls_pop = bls
        .proof_of_possession()
        .expect("FIXTURE: PoP")
        .as_bytes()
        .to_vec();
    if bonds == 0 {
        d.vdf_output = vec![0x11; 32];
    }
    tx.extra_data = bincode::serialize(&d).expect("FIXTURE: encode registration data");
    if bonds > 0 {
        let msg = tx.signing_message_for_input(0);
        tx.inputs[0].signature = crypto::signature::sign_hash(&msg, kp.private_key());
    }
    producer_auth(tx, kp)
}

fn queue_bytes(ps: &ProducerSet) -> Vec<u8> {
    bincode::serialize(ps.as_parts().2).expect("pending queue must serialize")
}

fn persisted(data: &Path) -> (ProducerSet, ChainState) {
    let db =
        StateDb::open(&data.join("state_db")).expect("FIXTURE: reopen state_db (lock released?)");
    let cs = db
        .get_chain_state()
        .expect("FIXTURE: persisted chain state");
    (db.load_producer_set(), cs)
}

async fn replay_node(dir: PathBuf) -> Node {
    let node = Node::new_for_replay(dir, Network::Devnet)
        .await
        .expect("FIXTURE: Node::new_for_replay");
    assert_eq!(
        node.config.network.genesis_blocks(),
        GENESIS,
        "PRECONDITION: devnet genesis window"
    );
    assert_eq!(
        node.config.network.blocks_per_reward_epoch(),
        EPOCH,
        "PRECONDITION: devnet epoch"
    );
    node
}

/// The same headless Replay path the check must use, over `blocks`, in its own dir.
async fn rebuilt_in_process(blocks: &[Block]) -> ProducerSet {
    let tmp = TempDir::new().unwrap();
    let mut node = replay_node(tmp.path().join("d")).await;
    for b in blocks {
        node.apply_block(b.clone(), ValidationMode::Replay)
            .await
            .expect("PRECONDITION: in-process replay applies");
    }
    let ps = node.producer_set.read().await.clone();
    ps
}

/// A stopped devnet node: blocks 1..=TIP applied through apply_block(Replay), node dropped.
pub(super) async fn build() -> Fx {
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    let scratch_root = tmp.path().join("scratch-root");
    std::fs::create_dir_all(&scratch_root).unwrap();
    let producer = KeyPair::generate();
    let newcomer = KeyPair::generate();
    let mut blocks = Vec::new();
    {
        let mut node = replay_node(data.clone()).await;
        let unit = node.config.network.bond_unit();
        let mut prev = node.chain_state.read().await.best_hash;
        for h in 1..=TIP {
            let mut txs = vec![coinbase_for(h, h as u32, &node.params)];
            if h == 1 {
                txs.push(registration(&producer, 0x61, unit, 0));
            }
            if h == TIP {
                txs.push(registration(&newcomer, 0x62, unit, 1));
            }
            let block = build_block_with_txs(h as u32, prev, &producer, &node.params, txs);
            prev = block.hash();
            node.apply_block(block.clone(), ValidationMode::Replay)
                .await
                .unwrap_or_else(|e| panic!("FIXTURE: apply h={h}: {e:#}"));
            blocks.push(block);
        }
    }
    let (ps, cs) = persisted(&data);
    assert_eq!(cs.best_height, TIP, "PRECONDITION: persisted tip");
    assert_eq!(cs.snap_sync_height, None, "PRECONDITION: full-replay node");
    let producers = ps.all_producers().len();
    let pending = ps.as_parts().2.len();
    assert!(
        producers >= 1,
        "PRECONDITION: the genesis producer exists at the tip"
    );
    assert!(
        pending >= 1,
        "PRECONDITION: the mid-epoch Register is still queued"
    );
    let again = rebuilt_in_process(&blocks).await;
    assert_eq!(
        (again.serialize_canonical(), queue_bytes(&again)),
        (ps.serialize_canonical(), queue_bytes(&ps)),
        "PRECONDITION: a Replay of the same blocks rebuilds the persisted set byte for byte"
    );
    Fx {
        _tmp: tmp,
        data,
        scratch_root,
        producer: *producer.public_key(),
        producers,
        pending,
    }
}

fn mutate_persisted(data: &Path, f: impl FnOnce(&mut ProducerSet)) {
    let db = StateDb::open(&data.join("state_db")).expect("FIXTURE: open state_db");
    let mut ps = db.load_producer_set();
    f(&mut ps);
    db.write_producer_set(&ps).expect("FIXTURE: write residue");
}

fn inject_pending(fx: &Fx) {
    let pubkey = fx.producer;
    mutate_persisted(&fx.data, |ps| {
        ps.queue_update(PendingProducerUpdate::Exit {
            pubkey,
            height: TIP,
        })
    });
}

fn inject_status(fx: &Fx) {
    let pubkey = fx.producer;
    mutate_persisted(&fx.data, |ps| {
        let p = ps
            .get_by_pubkey_mut(&pubkey)
            .expect("FIXTURE: genesis producer");
        assert_eq!(
            p.status,
            ProducerStatus::Active,
            "PRECONDITION: producer is Active"
        );
        p.status = ProducerStatus::Exited;
    });
}

pub(super) fn run_check(fx: &Fx) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_doli-node"));
    for (k, _) in std::env::vars() {
        if k.starts_with("DOLI_") {
            cmd.env_remove(k);
        }
    }
    let out = cmd
        .env("TMPDIR", &fx.scratch_root)
        .current_dir(&fx.scratch_root)
        .args(["--network", "devnet", "--data-dir"])
        .arg(&fx.data)
        .args(["--log-level", "error", "check-producer-set"])
        .output()
        .expect("spawn doli-node");
    Run {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// Exactly one verdict line, and the key=value summary on the line after it.
fn verdict(r: &Run) -> (String, BTreeMap<String, String>) {
    let lines: Vec<&str> = r.stdout.lines().map(str::trim).collect();
    let at: Vec<usize> = (0..lines.len())
        .filter(|&i| VERDICTS.contains(&lines[i]))
        .collect();
    assert_eq!(
        at.len(),
        1,
        "expected exactly one verdict line; exit={:?}\nstdout:\n{}\nstderr:\n{}",
        r.code,
        r.stdout,
        r.stderr
    );
    let summary = lines.get(at[0] + 1).copied().unwrap_or("");
    let kv = summary
        .split_whitespace()
        .filter_map(|t| t.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    (lines[at[0]].to_string(), kv)
}

pub(super) fn assert_outcome(r: &Run, want: &str, code: i32, kv: &[(&str, String)]) {
    let (got, map) = verdict(r);
    assert_eq!(got, want, "verdict; stdout:\n{}", r.stdout);
    assert_eq!(
        r.code,
        Some(code),
        "exit code for {want}; stderr:\n{}",
        r.stderr
    );
    for (k, v) in kv {
        assert_eq!(
            map.get(*k),
            Some(v),
            "summary key {k}; stdout:\n{}",
            r.stdout
        );
    }
}

pub(super) fn tree(root: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).expect("read_dir") {
            let p = e.expect("dir entry").path();
            let rel = p.strip_prefix(root).unwrap().display().to_string();
            if p.is_dir() {
                out.insert(rel, "dir".to_string());
                stack.push(p);
            } else {
                let bytes = std::fs::read(&p).expect("read file");
                out.insert(rel, crypto::hash::hash(&bytes).to_hex());
            }
        }
    }
    out
}

pub(super) fn assert_untouched(fx: &Fx, before: &BTreeMap<String, String>, what: &str) {
    assert_eq!(
        &tree(&fx.data),
        before,
        "{what}: the check changed the data directory"
    );
    let left: Vec<String> = std::fs::read_dir(&fx.scratch_root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        left.is_empty(),
        "{what}: scratch left under TMPDIR: {left:?}"
    );
}

// REQ-B3-005 — Decision: a failure means a clean stopped node reads as carrying residue, or the check did not replay to the persisted tip, so an operator acts on a false alarm.
#[tokio::test]
async fn b3_m2_clean_node_reports_none() {
    let fx = build().await;
    let r = run_check(&fx);
    let want = [
        ("height", TIP.to_string()),
        ("producers", fx.producers.to_string()),
        ("pending", fx.pending.to_string()),
    ];
    assert_outcome(&r, "EQUAL", 0, &want);
}

// REQ-B3-005 — Decision: a failure means residue an old binary left in the persisted pending queue stays invisible to the operator.
#[tokio::test]
async fn b3_m2_injected_residue_reported() {
    let fx = build().await;
    inject_pending(&fx);
    let r = run_check(&fx);
    let n = fx.producers;
    let want = [
        ("height", TIP.to_string()),
        ("producers", format!("{n}/{n}")),
        ("status_mismatches", "0".to_string()),
        ("pending", format!("{}/{}", fx.pending + 1, fx.pending)),
    ];
    assert_outcome(&r, "DIFFERS", 2, &want);
}

// REQ-B3-005 — Decision: a failure means the check compares only queue lengths, so a persisted producer status the chain never produced reads as EQUAL.
#[tokio::test]
async fn b3_m2_injected_status_residue_reported() {
    let fx = build().await;
    inject_status(&fx);
    let r = run_check(&fx);
    let (n, k) = (fx.producers, fx.pending);
    let want = [
        ("height", TIP.to_string()),
        ("producers", format!("{n}/{n}")),
        ("status_mismatches", "1".to_string()),
        ("pending", format!("{k}/{k}")),
    ];
    assert_outcome(&r, "DIFFERS", 2, &want);
}

// REQ-B3-005 — Decision: a failure means a snap-synced node (no blocks below its floor) gets a verdict, or the check writes into it.
#[tokio::test]
async fn b3_m2_snap_synced_node_not_checkable() {
    let fx = build().await;
    {
        let db = StateDb::open(&fx.data.join("state_db")).expect("FIXTURE: open state_db");
        let mut cs = db.get_chain_state().expect("FIXTURE: chain state");
        cs.snap_sync_height = Some(SNAP_H);
        db.put_chain_state(&cs)
            .expect("FIXTURE: persist snap_sync_height");
    }
    let before = tree(&fx.data);
    let r = run_check(&fx);
    assert_outcome(
        &r,
        "NOT-CHECKABLE",
        3,
        &[("reason", "snap_sync_height".to_string())],
    );
    assert_untouched(&fx, &before, "snap-synced");
}

// REQ-B3-005 — Decision: a failure means the check runs against a live node's open databases and reports a verdict from a moving state.
#[tokio::test]
async fn b3_m2_running_node_not_checkable() {
    let fx = build().await;
    let db = StateDb::open(&fx.data.join("state_db")).expect("FIXTURE: hold state_db");
    let bs = storage::BlockStore::open(&fx.data.join("blocks")).expect("FIXTURE: hold blocks");
    let r = run_check(&fx);
    drop((db, bs));
    assert_outcome(
        &r,
        "NOT-CHECKABLE",
        3,
        &[("reason", "lock_held".to_string())],
    );
}

// REQ-B3-005 — Decision: a failure means the read-only check modifies the node's data directory or leaks its scratch replay.
#[tokio::test]
async fn b3_m2_check_writes_nothing() {
    let fx = build().await;
    let before = tree(&fx.data);
    let r = run_check(&fx);
    assert_outcome(&r, "EQUAL", 0, &[]);
    assert_untouched(&fx, &before, "clean");

    inject_pending(&fx);
    let before = tree(&fx.data);
    let r = run_check(&fx);
    assert_outcome(&r, "DIFFERS", 2, &[]);
    assert_untouched(&fx, &before, "residue");
}
