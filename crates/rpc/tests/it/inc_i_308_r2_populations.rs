//! INC-I-308 R-2 — can each node population still fill its block-store gaps from an HONEST peer
//! under the anchor rule (dbdae50e)? One test per population; matrix in
//! docs/bugfixes/inc-i-308-r2-population-matrix.md.
//!
// covers: crates/rpc/src/methods/backfill.rs crates/rpc/src/methods/backfill_anchor.rs
// covers: crates/storage/src/block_store/maintenance.rs crates/storage/src/block_store/writes.rs
//
// OUTPUT CONTRACT:
//   O1 block_store height index + body per height; O2 backfillFromPeer reply (started | error);
//   O3 backfillStatus {imported, error}
//   PATHS: no gaps (started=false) | walk Store | walk Break | peer has no block | preflight refuse
// INPUT PARTITIONS (population = local store shape):
//   P1 full | P2 snap (gap [1,snap_h-1], header-only block at snap_h) | P3 middle holes | P4 pruned [1,cut-1]
//   P5 behind the peer (--no-snap-sync, peer tip > ours) | P6 losing branch (gap below / hole above fork)
//   P7 peer missing a block mid-gap, then a second peer | P8 store reopened from disk
// MATRIX
//   O1,O2 x P1 -> r2_p1_full_node_* | O1,O3 x P2 -> r2_p2_* | P3 -> r2_p3_* | P4 -> r2_p4_* | P5 -> r2_p5_*
//   O1,O2,O3 x P6 x {no skip, skip, hole above fork} -> r2_p6_* | P7 -> r2_p7_* | P8 -> r2_p8_*

use crypto::{Hash, KeyPair};
use doli_core::Block;
use rpc::RpcContext;
use serde_json::{json, Value};
use tempfile::TempDir;

use super::audit_rpc_effects::{block, call, local_node, raw, serve_peer, wait_done, Peer};

fn g() -> Hash {
    crypto::hash::hash(b"inc-i-308-r2-populations")
}

fn chain_from(prev0: Hash, from: u64, kp: &KeyPair, n: u64) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    for h in from..from + n {
        let prev = out.last().map(|b| b.hash()).unwrap_or(prev0);
        out.push(block(g(), h, prev, kp));
    }
    out
}

fn honest(n: u64) -> Vec<Block> {
    chain_from(g(), 1, &KeyPair::from_seed([0x5A; 32]), n)
}

fn store(ctx: &RpcContext, chain: &[Block], heights: &[u64]) {
    for &h in heights {
        ctx.block_store
            .put_block_canonical(&chain[h as usize - 1], h)
            .unwrap();
    }
}

fn body_hash(ctx: &RpcContext, h: u64) -> Option<Hash> {
    ctx.block_store
        .get_block_by_height(h)
        .unwrap()
        .map(|b| b.hash())
}

fn idx(ctx: &RpcContext, h: u64) -> Option<Hash> {
    ctx.block_store.get_hash_by_height(h).unwrap()
}

fn peer(tip: u64, claim: Hash, blocks: Vec<Block>) -> Peer {
    Peer {
        tip,
        blocks,
        garbage: false,
        claim_tip: Some(claim),
    }
}

async fn run(ctx: &RpcContext, p: Peer, skip: bool) -> (Value, Value) {
    let url = serve_peer(p).await;
    let reply = raw(
        ctx,
        "backfillFromPeer",
        json!({"rpc_url": url, "skip_divergence_check": skip}),
    )
    .await;
    let st = if reply["result"]["started"] == json!(true) {
        wait_done(ctx).await
    } else {
        json!({"error": null, "imported": 0})
    };
    (reply, st)
}

fn assert_has(
    ctx: &RpcContext,
    chain: &[Block],
    heights: impl IntoIterator<Item = u64>,
    ctxt: &str,
) {
    for k in heights {
        assert_eq!(
            body_hash(ctx, k),
            Some(chain[k as usize - 1].hash()),
            "h{k} not filled with the honest block; {ctxt}"
        );
    }
}

// INC-I-308-R2 — Decision: a failure means a complete full/archive node is told to fetch or gets rewritten.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r2_p1_full_node_has_nothing_to_fill_and_keeps_store() {
    let h = honest(5);
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(tmp.path(), 5, h[4].hash());
    store(&ctx, &h, &[1, 2, 3, 4, 5]);
    let (reply, _) = run(&ctx, peer(5, h[4].hash(), h.clone()), false).await;
    assert_eq!(reply["result"]["started"], json!(false), "{reply}");
    assert!(
        reply["result"]["message"]
            .as_str()
            .unwrap_or("")
            .starts_with("No gaps detected"),
        "{reply}"
    );
    assert_has(&ctx, &h, 1..=5, &reply.to_string());
}

// INC-I-308-R2 — Decision: a failure means a snap-synced producer can no longer fill [1, snap_h-1] from an honest peer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r2_p2_snap_synced_node_fills_gap_under_header_only_anchor() {
    let h = honest(8);
    let snap_h = 5u64;
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(tmp.path(), 8, h[7].hash());
    // Mirrors bins/node/src/node/install_contract.rs:62-73: index seed + header-only block at snap_h.
    ctx.block_store
        .seed_canonical_index(h[4].hash(), snap_h)
        .unwrap();
    let header_only = Block {
        header: h[4].header.clone(),
        transactions: vec![],
        aggregate_bls_signature: vec![],
        attestation_bitfield: vec![],
    };
    ctx.block_store.put_block(&header_only, snap_h).unwrap();
    store(&ctx, &h, &[6, 7, 8]);
    let (reply, st) = run(&ctx, peer(8, h[7].hash(), h.clone()), false).await;
    assert_has(&ctx, &h, 1..=8, &format!("reply {reply} status {st}"));
    assert!(st["error"].is_null(), "{st}");
    assert_eq!(st["imported"], json!(4), "{st}");
}

// INC-I-308-R2 — Decision: a failure means a node with several separate holes keeps some of them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r2_p3_middle_holes_all_filled() {
    let h = honest(12);
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(tmp.path(), 12, h[11].hash());
    store(&ctx, &h, &[1, 2, 5, 6, 7, 10, 12]);
    let (reply, st) = run(&ctx, peer(12, h[11].hash(), h.clone()), false).await;
    assert_has(&ctx, &h, 1..=12, &format!("reply {reply} status {st}"));
    assert!(st["error"].is_null(), "{st}");
    assert_eq!(st["imported"], json!(5), "{st}");
}

// INC-I-308-R2 — Decision: a failure means a pruned node can no longer restore history below its cutoff.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r2_p4_pruned_node_refills_below_cutoff() {
    // prune_blocks_below keeps MIN_RETENTION = 2000 blocks under the tip: tip 2_006, cutoff 6.
    let n = 2_006u64;
    let h = honest(n);
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(tmp.path(), n, h[n as usize - 1].hash());
    let all: Vec<u64> = (1..=n).collect();
    store(&ctx, &h, &all);
    let (deleted, _) = ctx.block_store.prune_blocks_below(6, n).unwrap();
    assert_eq!(deleted, 5, "fixture: heights 1..5 pruned");
    for k in 1..=5 {
        assert_eq!(idx(&ctx, k), None, "fixture: h{k} still indexed");
    }
    let served: Vec<Block> = h[..8].to_vec();
    let (reply, st) = run(&ctx, peer(n, h[n as usize - 1].hash(), served), false).await;
    assert_has(&ctx, &h, 1..=6, &format!("reply {reply} status {st}"));
    assert!(st["error"].is_null(), "{st}");
    assert_eq!(st["imported"], json!(5), "{st}");
}

// INC-I-308-R2 — Decision: a failure means a node behind the peer (--no-snap-sync) cannot fill or stores above its tip.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r2_p5_node_behind_peer_fills_own_gaps_only() {
    let h = honest(10);
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(tmp.path(), 6, h[5].hash());
    store(&ctx, &h, &[1, 2, 5, 6]);
    let (reply, st) = run(&ctx, peer(10, h[5].hash(), h.clone()), false).await;
    assert_has(&ctx, &h, 1..=6, &format!("reply {reply} status {st}"));
    for k in 7..=10 {
        assert_eq!(idx(&ctx, k), None, "h{k} above our tip stored: {st}");
    }
    assert_eq!(st["imported"], json!(2), "{st}");
}

struct Fork {
    h: Vec<Block>,
    f: Vec<Block>,
}

// Common h1..h3; our branch f4..f6 (tip f6); the peer's branch h4..h8.
fn fork() -> Fork {
    let h = honest(8);
    let f = chain_from(h[2].hash(), 4, &KeyPair::from_seed([0x77; 32]), 3);
    Fork { h, f }
}

fn losing_node(tmp: &TempDir, fk: &Fork, hole_above: bool) -> RpcContext {
    let ctx = local_node(tmp.path(), 6, fk.f[2].hash());
    store(&ctx, &fk.h, &[1, 3]);
    let ours: &[usize] = if hole_above { &[0, 2] } else { &[0, 1, 2] };
    for &i in ours {
        ctx.block_store
            .put_block_canonical(&fk.f[i], 4 + i as u64)
            .unwrap();
    }
    ctx
}

fn assert_own_branch(ctx: &RpcContext, fk: &Fork, heights: &[u64], st: &Value) {
    for &k in heights {
        assert_eq!(
            idx(ctx, k),
            Some(fk.f[k as usize - 4].hash()),
            "h{k}: own branch replaced by the peer's; {st}"
        );
    }
}

// INC-I-308-R2 — Decision: a failure means the preflight no longer stops a losing-branch node by default.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r2_p6_losing_branch_default_refused_by_preflight() {
    let fk = fork();
    let tmp = TempDir::new().unwrap();
    let ctx = losing_node(&tmp, &fk, false);
    let (reply, st) = run(&ctx, peer(8, fk.h[5].hash(), fk.h.clone()), false).await;
    let msg = reply["error"]["message"].as_str().unwrap_or("");
    assert!(msg.contains("Tip divergence"), "expected refusal: {reply}");
    assert_eq!(idx(&ctx, 2), None, "h2 written despite refusal: {st}");
    assert_own_branch(&ctx, &fk, &[4, 5, 6], &st);
}

// INC-I-308-R2 — Decision: a failure means a losing-branch node cannot fill below the fork, or its branch is overwritten.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r2_p6_losing_branch_skip_fills_below_fork_keeps_branch() {
    let fk = fork();
    let tmp = TempDir::new().unwrap();
    let ctx = losing_node(&tmp, &fk, false);
    let (reply, st) = run(&ctx, peer(8, fk.h[5].hash(), fk.h.clone()), true).await;
    assert_has(&ctx, &fk.h, 1..=3, &format!("reply {reply} status {st}"));
    assert!(st["error"].is_null(), "{st}");
    assert_own_branch(&ctx, &fk, &[4, 5, 6], &st);
}

// INC-I-308-R2 — Decision: a failure means a hole ABOVE the fork changes from "stop, keep branch" to a cross-branch write.
// Records END behaviour: the top-down walk breaks at the upper hole, so the gap below the fork stays open in that pass.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r2_p6_losing_branch_hole_above_fork_stops_walk() {
    let fk = fork();
    let tmp = TempDir::new().unwrap();
    let ctx = losing_node(&tmp, &fk, true);
    let (reply, st) = run(&ctx, peer(8, fk.h[5].hash(), fk.h.clone()), true).await;
    assert_eq!(idx(&ctx, 5), None, "peer's h5 stored on our branch: {st}");
    let err = st["error"].as_str().unwrap_or("");
    assert!(
        err.contains("height 5"),
        "error must name height 5: {reply} {st}"
    );
    assert_eq!(idx(&ctx, 2), None, "{st}");
    assert_own_branch(&ctx, &fk, &[4, 6], &st);
}

// INC-I-308-R2 — Decision: a failure means a peer missing one block mid-gap blocks recovery from a second honest peer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r2_p7_peer_missing_block_then_second_peer_completes() {
    let h = honest(7);
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(tmp.path(), 7, h[6].hash());
    store(&ctx, &h, &[1, 7]);
    let partial: Vec<Block> = h.iter().filter(|b| b.header.slot != 4).cloned().collect();
    let (_, st1) = run(&ctx, peer(9, h[6].hash(), partial), false).await;
    assert!(
        st1["error"].as_str().unwrap_or("").contains("height 4"),
        "{st1}"
    );
    assert_eq!(idx(&ctx, 3), None, "h3 stored below the peer's hole: {st1}");
    assert_has(&ctx, &h, 5..=6, &st1.to_string());
    let (reply, st2) = run(&ctx, peer(9, h[6].hash(), h.clone()), false).await;
    assert_has(&ctx, &h, 1..=7, &format!("reply {reply} status {st2}"));
    assert!(st2["error"].is_null(), "{st2}");
    assert_eq!(st2["imported"], json!(3), "{st2}");
}

// INC-I-308-R2 — Decision: a failure means a node reopened from disk cannot fill gaps (anchor read from a fresh store).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r2_p8_fresh_restart_reopened_store_fills() {
    let h = honest(6);
    let tmp = TempDir::new().unwrap();
    {
        let ctx = local_node(tmp.path(), 6, h[5].hash());
        store(&ctx, &h, &[1, 4, 6]);
    }
    let ctx = local_node(tmp.path(), 6, h[5].hash());
    assert_eq!(
        idx(&ctx, 4),
        Some(h[3].hash()),
        "fixture: store not reopened"
    );
    let (reply, st) = run(&ctx, peer(6, h[5].hash(), h.clone()), false).await;
    assert_has(&ctx, &h, 1..=6, &format!("reply {reply} status {st}"));
    assert!(st["error"].is_null(), "{st}");
    assert_eq!(st["imported"], json!(3), "{st}");
    let _ = call(&ctx, "backfillStatus", json!({})).await;
}
