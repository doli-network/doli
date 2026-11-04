//! INC-I-294 M-3: an operator truncate leaves the node with the epoch state of a
//! node that never saw the dropped blocks, as the live rollback does.
// covers: bins/node/src/operations/chain.rs, bins/node/src/operations/truncate_restore.rs
//
// OUTPUT CONTRACT: `doli-node --network devnet --data-dir D truncate --blocks k --yes` (truncate_chain)
//   O1 META_EPOCH_STATE after the command (and node.epoch_state after Node::new reopen)
//   O2 epoch tables: dropped boundary starts gone, kept starts present
//   O3 chain commitment (META_CHAIN_COMMITMENT / _TIP)
//   O4 block-store height index above the new tip; chain_state tip
//   O5 command output (stdout+stderr) and exit code
// PATHS: P1 undo(new_tip+1) carries an epoch snapshot | P2 undo(new_tip+1) present, snapshot None
//   | P3 an undo record of a dropped height is missing (command fails) | P4 healthy restart, no truncate
// INPUT PARTITIONS: IP-X dropped range crosses an epoch boundary (h=8, bpe 4) and minutes 3..5
//   | IP-IN dropped range inside one epoch: not exercised (O2 vacuous there)
//   | IP-MIN attestation minutes crossed by the dropped blocks (slot = 3h, minute = slot/6)
// MATRIX:
//   O1 O5 x P1 x IP-X IP-MIN -> r2_truncate_restores_the_epoch_state_of_the_new_tip
//   O1 x P1 x IP-X IP-MIN, then a shared boundary -> truncated_node_derives_the_same_boundary_...
//   O1 x P4 -> restart_of_a_healthy_node_keeps_its_epoch_state
//   O1 O4 O5 x P2 x IP-X -> truncate_with_no_epoch_snapshot_..._warns
//   O1 O4 O5 x P3 -> truncate_with_a_missing_undo_record_fails_...
//   O2 O3 O4 O5 x P1 x IP-X -> truncate_drops_dropped_boundary_tables_...

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use crate::inc_i_156_m1_harness as h;
use crate::inc_i_289_common::{block, leader};
use crypto::KeyPair;
use doli_core::validation::ValidationMode;
use doli_core::{Block, EpochState};
use doli_node::config::NodeConfig;
use doli_node::node::Node;
use storage::{BlockStore, EpochTable, StateDb};
use tempfile::TempDir;

/// The developer prints each line below; a line of the output must END with it.
/// `epoch state restored for tip {new_tip} from undo at height {new_tip + 1}`
const RESTORED_PREFIX: &str = "epoch state restored for tip ";
const RESTORED_MID: &str = " from undo at height ";
/// `WARNING: no epoch state snapshot in undo at height {new_tip + 1}`
const NO_SNAPSHOT_WARNING: &str = "WARNING: no epoch state snapshot in undo at height ";
/// `epoch tables dropped: {n}`
const TABLES_DROPPED: &str = "epoch tables dropped: ";

const TIP: u64 = 10;
const K: u64 = 4;
const NEW_TIP: u64 = TIP - K;
const DROPPED_BOUNDARY: u64 = 8;
const KEPT_BOUNDARY: u64 = 4;
/// Last height of the replacement branch: through boundaries 8 and 12.
const NEW_END: u64 = 13;

fn restored_line(new_tip: u64) -> String {
    format!("{RESTORED_PREFIX}{new_tip}{RESTORED_MID}{}", new_tip + 1)
}

fn has_line(out: &str, want: &str) -> bool {
    out.lines().any(|l| l.trim_end().ends_with(want))
}

/// Dropped branch: slot 3h, so h=7..10 sit in minutes 3, 4, 5.
fn old_slot(h: u64) -> u32 {
    (3 * h) as u32
}

/// Replacement branch: later minutes than every dropped block.
fn new_slot(h: u64) -> u32 {
    (3 * h + 60) as u32
}

fn keys3() -> Vec<KeyPair> {
    (0..3).map(|_| KeyPair::generate()).collect()
}

async fn fresh(keys: &[KeyPair], tmp: &TempDir) -> Node {
    h::init_env();
    let node = Node::new_for_test(tmp.path().to_path_buf(), keys.to_vec())
        .await
        .expect("fixture: Node::new_for_test");
    h::install_production_utxo_backend(&node).await;
    node
}

/// Applies `heights` on the tip, each block by its slot leader.
async fn apply(
    node: &mut Node,
    keys: &[KeyPair],
    heights: std::ops::RangeInclusive<u64>,
    slot: fn(u64) -> u32,
) -> Vec<Block> {
    let p = node.params.clone();
    let mut prev = node.chain_state.read().await.best_hash;
    let mut out = Vec::new();
    for height in heights {
        let s = slot(height);
        let b = block(height, s, prev, leader(keys, s), &p);
        prev = b.hash();
        node.apply_block(b.clone(), ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("fixture: block {height} must apply: {e}"));
        out.push(b);
    }
    out
}

async fn restart(config: NodeConfig, keys: &[KeyPair]) -> Node {
    Node::new(config, Some(keys[0].clone()), None, None, None, None)
        .await
        .expect("Node::new must reopen the data dir")
}

/// A node on 1..=TIP (slot 3h) whose config is kept and whose stores are closed.
async fn stopped_node(keys: &[KeyPair], tmp: &TempDir) -> (NodeConfig, Vec<Block>, EpochState) {
    let mut node = fresh(keys, tmp).await;
    let chain = apply(&mut node, keys, 1..=TIP, old_slot).await;
    let (config, es) = (node.config.clone(), node.epoch_state.clone());
    drop(node);
    (config, chain, es)
}

fn state_db(data: &Path) -> StateDb {
    StateDb::open(&data.join("state_db")).expect("open state_db (node stopped)")
}

fn undo_snapshot(data: &Path, height: u64) -> EpochState {
    let undo = state_db(data)
        .get_undo(height)
        .expect("precondition: undo record");
    let bytes = undo
        .epoch_state_snapshot
        .expect("precondition: the undo record carries an epoch snapshot");
    EpochState::deserialize(&bytes).expect("precondition: undo epoch snapshot decodes")
}

fn minutes(es: &EpochState) -> BTreeSet<u32> {
    (es.attestation_accum.iter())
        .flat_map(|m| m.values().flatten().copied())
        .collect()
}

struct Run {
    ok: bool,
    out: String,
}

fn truncate(data: &Path, k: u64) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_doli-node"));
    for (key, _) in std::env::vars() {
        if key.starts_with("DOLI_") {
            cmd.env_remove(key);
        }
    }
    let k = k.to_string();
    let o = (cmd.current_dir(data))
        .args(["--network", "devnet", "--data-dir"])
        .arg(data)
        .args(["truncate", "--blocks", k.as_str(), "--yes"])
        .output()
        .expect("spawn doli-node truncate");
    Run {
        ok: o.status.success(),
        out: format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ),
    }
}

// INC-I-294 M-3 R-2 — Decision: a failure means truncate leaves the dropped blocks' attestation minutes in the stored epoch state
#[tokio::test]
async fn inc_i_294_m3_r2_truncate_restores_the_epoch_state_of_the_new_tip() {
    let (keys, tmp) = (keys3(), TempDir::new().unwrap());
    let (config, _, stale) = stopped_node(&keys, &tmp).await;
    let want = undo_snapshot(tmp.path(), NEW_TIP + 1);
    assert!(
        minutes(&stale).contains(&5) && !minutes(&want).contains(&5),
        "precondition: minute 5 comes only from the dropped blocks (stale {:?}, want {:?})",
        minutes(&stale),
        minutes(&want)
    );
    assert_ne!(stale.epoch, 1, "precondition: old tip is past boundary 8");
    assert_eq!(want.epoch, 1, "precondition: new tip epoch is 1");

    let run = truncate(tmp.path(), K);
    assert!(run.ok, "precondition: truncate succeeds:\n{}", run.out);
    let node = restart(config, &keys).await;
    assert_eq!(node.chain_state.read().await.best_height, NEW_TIP);
    assert_eq!(
        node.epoch_state.hash(),
        want.hash(),
        "R-2: after truncate to {NEW_TIP} + reopen, epoch state hash {} must equal undo({}) snapshot hash {} (old-tip hash {})",
        node.epoch_state.hash(),
        NEW_TIP + 1,
        want.hash(),
        stale.hash()
    );
    let line = restored_line(NEW_TIP);
    assert!(
        has_line(&run.out, &line),
        "R-2: output must end a line with `{line}`:\n{}",
        run.out
    );
}

// INC-I-294 M-3 twin — Decision: a failure means a truncated node derives a different epoch boundary than a node that never held the branch
// With 3 producers the minute filter (above 50 producers) leaves the lists equal; the hash is the red signal.
#[tokio::test]
async fn inc_i_294_m3_truncated_node_derives_the_same_boundary_as_a_node_that_never_held_the_blocks(
) {
    let keys = keys3();
    let (ta, tb) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let (ca, chain_a, _) = stopped_node(&keys, &ta).await;
    let run = truncate(ta.path(), K);
    assert!(run.ok, "precondition: truncate succeeds:\n{}", run.out);
    let mut a = restart(ca, &keys).await;

    let mut b = fresh(&keys, &tb).await;
    let base_b = apply(&mut b, &keys, 1..=NEW_TIP, old_slot).await;
    let same_base =
        (chain_a[..NEW_TIP as usize].iter().zip(&base_b)).all(|(x, y)| x.hash() == y.hash());
    assert!(same_base, "fixture: B holds A's chain up to {NEW_TIP}");
    let cb = b.config.clone();
    drop(b);
    let mut b = restart(cb, &keys).await;

    let na = apply(&mut a, &keys, NEW_TIP + 1..=NEW_END, new_slot).await;
    let nb = apply(&mut b, &keys, NEW_TIP + 1..=NEW_END, new_slot).await;
    assert_eq!(
        na.last().map(Block::hash),
        nb.last().map(Block::hash),
        "fixture: both apply the same replacement branch"
    );
    assert_ne!(
        na[0].hash(),
        chain_a[NEW_TIP as usize].hash(),
        "fixture: the replacement branch differs from the dropped one"
    );
    let (ea, eb) = (&a.epoch_state, &b.epoch_state);
    assert_eq!(
        ea.producer_list, eb.producer_list,
        "producer_list at h={NEW_END}"
    );
    assert_eq!(ea.active_list, eb.active_list, "active_list at h={NEW_END}");
    assert_eq!(
        ea.hash(),
        eb.hash(),
        "truncated node A epoch state {} != never-held node B {} at h={NEW_END} (minutes A {:?}, B {:?})",
        ea.hash(),
        eb.hash(),
        minutes(ea),
        minutes(eb)
    );
}

// INC-I-294 M-3 control — Decision: a failure means the fix changed the epoch state a healthy restart loads
#[tokio::test]
async fn inc_i_294_m3_restart_of_a_healthy_node_keeps_its_epoch_state() {
    let (keys, tmp) = (keys3(), TempDir::new().unwrap());
    let (config, chain, before) = stopped_node(&keys, &tmp).await;
    let node = restart(config, &keys).await;
    let tip = node.chain_state.read().await.best_hash;
    assert_eq!(tip, chain[TIP as usize - 1].hash(), "restart keeps the tip");
    assert_eq!(
        node.epoch_state.hash(),
        before.hash(),
        "a healthy restart must load the epoch state it stopped with"
    );
}

// INC-I-294 M-3 no-snapshot — Decision: a failure means truncate silently keeps a stale epoch state, or clobbers it, when the undo record has no snapshot
#[tokio::test]
async fn inc_i_294_m3_truncate_with_no_epoch_snapshot_in_the_undo_record_keeps_the_stored_epoch_state_and_warns(
) {
    let (keys, tmp) = (keys3(), TempDir::new().unwrap());
    let _ = stopped_node(&keys, &tmp).await;
    let before = {
        let db = state_db(tmp.path());
        let mut undo = db.get_undo(NEW_TIP + 1).expect("precondition: undo record");
        assert!(
            undo.epoch_state_snapshot.is_some(),
            "precondition: snapshot"
        );
        undo.epoch_state_snapshot = None;
        db.put_undo(NEW_TIP + 1, &undo).expect("rewrite undo");
        assert!(db
            .get_undo(NEW_TIP + 1)
            .unwrap()
            .epoch_state_snapshot
            .is_none());
        db.get_epoch_state()
            .expect("precondition: stored epoch state")
    };

    let run = truncate(tmp.path(), K);
    assert!(run.ok, "truncate must still succeed:\n{}", run.out);
    let db = state_db(tmp.path());
    assert_eq!(db.get_chain_state().map(|c| c.best_height), Some(NEW_TIP));
    assert!(
        db.get_epoch_state() == Some(before),
        "the stored epoch state must stay untouched when undo({}) has no snapshot",
        NEW_TIP + 1
    );
    let line = format!("{NO_SNAPSHOT_WARNING}{}", NEW_TIP + 1);
    assert!(
        has_line(&run.out, &line),
        "output must end a line with `{line}`:\n{}",
        run.out
    );
}

// INC-I-294 M-3 missing-undo — Decision: a failure means a failed truncate wrote a partial epoch state or moved the tip
#[tokio::test]
async fn inc_i_294_m3_truncate_with_a_missing_undo_record_fails_and_keeps_the_epoch_state() {
    let (keys, tmp) = (keys3(), TempDir::new().unwrap());
    let (_, chain, _) = stopped_node(&keys, &tmp).await;
    let before = {
        let db = state_db(tmp.path());
        db.prune_undo_above(TIP - 1);
        assert!(db.get_undo(TIP).is_none(), "precondition: undo({TIP}) gone");
        assert!(
            db.get_undo(NEW_TIP + 1).is_some(),
            "precondition: undo(7) kept"
        );
        db.get_epoch_state()
            .expect("precondition: stored epoch state")
    };

    let run = truncate(tmp.path(), K);
    assert!(
        !run.ok,
        "truncate must fail without undo({TIP}):\n{}",
        run.out
    );
    let why = format!("Missing undo data at height {TIP}");
    assert!(
        run.out.contains(&why),
        "fails for the missing undo:\n{}",
        run.out
    );
    {
        let db = state_db(tmp.path());
        assert!(
            db.get_epoch_state() == Some(before),
            "a failed truncate must keep the stored epoch state"
        );
        assert_eq!(db.get_chain_state().map(|c| c.best_height), Some(TIP));
    }
    let bs = BlockStore::open(&tmp.path().join("blocks")).expect("open blocks");
    assert_eq!(
        bs.get_hash_by_height(TIP).unwrap(),
        Some(chain[TIP as usize - 1].hash()),
        "a failed truncate keeps the tip block"
    );
}

// INC-I-294 M-3 tables — Decision: a failure means a dropped boundary's epoch table or the chain commitment survives the truncate
#[tokio::test]
async fn inc_i_294_m3_truncate_drops_dropped_boundary_tables_the_chain_commitment_and_height_index()
{
    let (keys, tmp) = (keys3(), TempDir::new().unwrap());
    let (_, chain, _) = stopped_node(&keys, &tmp).await;
    let boundary_hash = chain[DROPPED_BOUNDARY as usize - 1].hash();
    {
        let db = state_db(tmp.path());
        if db.get_epoch_table(DROPPED_BOUNDARY).is_none() {
            let mut t = EpochTable::default();
            (t.epoch_start, t.boundary_hash) = (DROPPED_BOUNDARY, Some(boundary_hash));
            db.put_epoch_table(&t);
        }
        let t = db.get_epoch_table(DROPPED_BOUNDARY);
        assert_eq!(t.and_then(|t| t.boundary_hash), Some(boundary_hash));
        assert!(
            db.get_epoch_table(KEPT_BOUNDARY).is_some(),
            "precondition: table 4"
        );
        if db.get_chain_commitment().is_none() {
            db.put_chain_commitment_with_tip(&chain[TIP as usize - 1].hash(), TIP);
        }
        assert!(
            db.get_chain_commitment().is_some(),
            "precondition: commitment"
        );
    }

    let run = truncate(tmp.path(), K);
    assert!(run.ok, "precondition: truncate succeeds:\n{}", run.out);
    {
        let bs = BlockStore::open(&tmp.path().join("blocks")).expect("open blocks");
        for height in NEW_TIP + 1..=TIP {
            let at = bs.get_hash_by_height(height).unwrap();
            assert_eq!(at, None, "height index above the new tip at h={height}");
        }
        let kept = bs.get_hash_by_height(NEW_TIP).unwrap();
        assert_eq!(
            kept,
            Some(chain[NEW_TIP as usize - 1].hash()),
            "new tip indexed"
        );
    }
    let db = state_db(tmp.path());
    assert!(
        db.get_epoch_table(DROPPED_BOUNDARY).is_none(),
        "the epoch table of dropped boundary {DROPPED_BOUNDARY} must be deleted (starts now {:?})",
        db.epoch_table_starts()
    );
    assert!(db.get_epoch_table(KEPT_BOUNDARY).is_some(), "kept table 4");
    assert_eq!(
        db.get_chain_commitment(),
        None,
        "chain commitment must be deleted"
    );
    let line = format!("{TABLES_DROPPED}1");
    assert!(
        has_line(&run.out, &line),
        "output must end a line with `{line}`:\n{}",
        run.out
    );
}
