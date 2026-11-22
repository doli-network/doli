//! INC-I-308 R-1 — anchor rule: a peer block is stored at h only if its hash is the hash our own chain commits to at h.
//!
// covers: crates/rpc/src/methods/backfill.rs crates/rpc/src/methods/backfill_anchor.rs crates/rpc/src/methods/guardian_repair_check.rs
// covers: crates/rpc/src/methods/guardian.rs crates/rpc/src/methods/guardian_repair_check.rs
//
// OUTPUT CONTRACT:
//   O1 block_store height index + body per height; O2 backfillStatus {imported, error}; O3 archive *.block files
//   PATHS: backfill walk Store | Break (STOP) | SkipKeepChain (hash ok, body rejected: skip, keep going) |
//     peer has no block (STOP) | honest fill; repair admit | refuse
// INPUT PARTITIONS:
//   positions P1 h=1 snap gap, P2 gap bottom, P3 gap middle, P5 tip absent, P6 one-block hole, P7 bodiless seed;
//   peer: hostile (claims our tip, self-signed forged blocks, no skip_divergence_check) | honest | missing block |
//     body variant (same header hash, presence bit flipped) at a middle height of an empty store
// MATRIX
//   O1,O2 x SkipKeepChain x P3 (empty store, h=3 of 6), then honest peer -> m14_backfill_rejected_middle_block_keeps_fill_below
//   O1,O2 x Break x {P1,P2,P3,P5} -> r1_backfill_refuses_forged_{h1_and_snap_gap,gap_bottom,gap_middle,tip}
//   O1,O2 x honest x {P1,P7,P2..P6} -> r1_backfill_honest_peer_fills_{snap_gap,every_position}
//   O1 x stale stored f1,f2 -> r1_backfill_heals_forged_low_blocks | O1,O2 x no block -> r1_backfill_stops_where_peer_has_no_block
//   O3 x {gap bottom, forged body, forged h1..3, honest} -> r1_repair_archive_*

use crypto::{Hash, KeyPair};
use doli_core::transaction::legacy::deserialize_block_compat;
use doli_core::Block;
use rpc::RpcContext;
use serde_json::{json, Value};
use tempfile::TempDir;

use super::audit_rpc_effects::{block, call, local_node, serve_peer, wait_done, Peer};

fn g() -> Hash {
    crypto::hash::hash(b"inc-i-308-r1-anchor")
}

fn chain_on(gen: Hash, kp: &KeyPair, n: u64) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    for h in 1..=n {
        let prev = out.last().map(|b| b.hash()).unwrap_or(gen);
        out.push(block(gen, h, prev, kp));
    }
    out
}

fn chain(kp: &KeyPair, n: u64) -> Vec<Block> {
    chain_on(g(), kp, n)
}

// repairArchiveFromPeer checks genesis_hash against ChainState::new(Hash::ZERO).
fn honest_z(n: u64) -> Vec<Block> {
    chain_on(Hash::ZERO, &KeyPair::from_seed([0x5A; 32]), n)
}

fn honest(n: u64) -> Vec<Block> {
    chain(&KeyPair::from_seed([0x5A; 32]), n)
}

fn forged(n: u64) -> Vec<Block> {
    chain(&KeyPair::from_seed([0x66; 32]), n)
}

fn store(ctx: &RpcContext, chain: &[Block], heights: &[u64]) {
    for &h in heights {
        ctx.block_store
            .put_block_canonical(&chain[h as usize - 1], h)
            .unwrap();
    }
}

fn idx(ctx: &RpcContext, h: u64) -> Option<Hash> {
    ctx.block_store.get_hash_by_height(h).unwrap()
}

async fn run(ctx: &RpcContext, tip: u64, tip_hash: Hash, blocks: Vec<Block>) -> (Value, Value) {
    let url = serve_peer(Peer {
        tip,
        blocks,
        garbage: false,
        claim_tip: Some(tip_hash),
    })
    .await;
    let reply = call(ctx, "backfillFromPeer", json!({"rpc_url": url})).await;
    let st = if reply["started"] == json!(true) {
        wait_done(ctx).await
    } else {
        json!({"error": null, "imported": 0})
    };
    (reply, st)
}

fn assert_stopped(st: &Value, h: u64, imported: u64) {
    let err = st["error"].as_str().unwrap_or("");
    assert!(
        err.contains(&format!("height {h}")),
        "error must name height {h}: {st}"
    );
    assert_eq!(
        st["imported"],
        json!(imported),
        "imported counts stored blocks only: {st}"
    );
}

fn assert_clean(st: &Value, imported: u64) {
    assert!(st["error"].is_null(), "honest fill reported an error: {st}");
    assert_eq!(st["imported"], json!(imported), "{st}");
}

// INC-I-308-R1 — Decision: a failure means a hostile peer writes a forged h=1 and snap gap under our tip.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r1_backfill_refuses_forged_h1_and_snap_gap() {
    let (h, f) = (honest(6), forged(3));
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(tmp.path(), 6, h[5].hash());
    store(&ctx, &h, &[4, 5, 6]);
    let peer = vec![
        f[0].clone(),
        f[1].clone(),
        f[2].clone(),
        h[3].clone(),
        h[4].clone(),
        h[5].clone(),
    ];
    let (reply, st) = run(&ctx, 6, h[5].hash(), peer).await;
    for k in 1..=3 {
        assert_eq!(
            idx(&ctx, k),
            None,
            "forged h{k} indexed; reply {reply} status {st}"
        );
    }
    assert_stopped(&st, 3, 0);
}

// INC-I-308-R1 — Decision: a failure means a forged block at the bottom of a gap is stored because only h-1 was checked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r1_backfill_refuses_forged_gap_bottom() {
    let h = honest(6);
    let f2 = block(g(), 2, h[0].hash(), &KeyPair::from_seed([0x66; 32]));
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(tmp.path(), 6, h[5].hash());
    store(&ctx, &h, &[1, 5, 6]);
    let peer = vec![
        h[0].clone(),
        f2,
        h[2].clone(),
        h[3].clone(),
        h[4].clone(),
        h[5].clone(),
    ];
    let (reply, st) = run(&ctx, 6, h[5].hash(), peer).await;
    assert_eq!(
        idx(&ctx, 2),
        None,
        "forged h2 indexed; reply {reply} status {st}"
    );
    assert_eq!(idx(&ctx, 3), Some(h[2].hash()), "{st}");
    assert_eq!(idx(&ctx, 4), Some(h[3].hash()), "{st}");
    assert_stopped(&st, 2, 2);
}

// INC-I-308-R1 — Decision: a failure means a forged block in the middle of a gap is stored after its honest parent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r1_backfill_refuses_forged_gap_middle() {
    let h = honest(6);
    let f4 = block(g(), 4, h[2].hash(), &KeyPair::from_seed([0x66; 32]));
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(tmp.path(), 6, h[5].hash());
    store(&ctx, &h, &[1, 6]);
    let peer = vec![
        h[0].clone(),
        h[1].clone(),
        h[2].clone(),
        f4,
        h[4].clone(),
        h[5].clone(),
    ];
    let (reply, st) = run(&ctx, 6, h[5].hash(), peer).await;
    for k in 2..=4 {
        assert_eq!(
            idx(&ctx, k),
            None,
            "h{k} indexed below the break; reply {reply} status {st}"
        );
    }
    assert_eq!(idx(&ctx, 5), Some(h[4].hash()), "{st}");
    assert_stopped(&st, 4, 1);
}

// INC-I-308-R1 — Decision: a failure means a forged block is stored at our own tip height.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r1_backfill_refuses_forged_tip() {
    let h = honest(5);
    let f5 = block(g(), 5, h[3].hash(), &KeyPair::from_seed([0x66; 32]));
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(tmp.path(), 5, h[4].hash());
    store(&ctx, &h, &[1, 2, 3]);
    let peer = vec![h[0].clone(), h[1].clone(), h[2].clone(), h[3].clone(), f5];
    let (reply, st) = run(&ctx, 5, h[4].hash(), peer).await;
    for k in 4..=5 {
        assert_eq!(
            idx(&ctx, k),
            None,
            "h{k} indexed; reply {reply} status {st}"
        );
    }
    assert_stopped(&st, 5, 0);
}

// INC-I-308-R1 — Decision: a failure means the anchor rule refuses an honest snap-gap fill (stored or bodiless seed above).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r1_backfill_honest_peer_fills_snap_gap() {
    let h = honest(6);
    for seeded in [false, true] {
        let tmp = TempDir::new().unwrap();
        let ctx = local_node(tmp.path(), 6, h[5].hash());
        if seeded {
            ctx.block_store
                .seed_canonical_index(h[3].hash(), 4)
                .unwrap();
            store(&ctx, &h, &[5, 6]);
        } else {
            store(&ctx, &h, &[4, 5, 6]);
        }
        let (reply, st) = run(&ctx, 6, h[5].hash(), h.clone()).await;
        for k in 1..=4u64 {
            let got = ctx
                .block_store
                .get_block_by_height(k)
                .unwrap()
                .map(|b| b.hash());
            assert_eq!(
                got,
                Some(h[k as usize - 1].hash()),
                "seeded={seeded} h{k}; reply {reply} status {st}"
            );
        }
        assert_clean(&st, if seeded { 4 } else { 3 });
    }
}

// INC-I-308-R1 — Decision: a failure means an honest fill breaks at one of the positions P2..P6.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r1_backfill_honest_peer_fills_every_position() {
    let h = honest(9);
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(tmp.path(), 9, h[8].hash());
    store(&ctx, &h, &[1, 5, 7, 8]);
    let (reply, st) = run(&ctx, 9, h[8].hash(), h.clone()).await;
    for k in 1..=9u64 {
        let got = ctx
            .block_store
            .get_block_by_height(k)
            .unwrap()
            .map(|b| b.hash());
        assert_eq!(
            got,
            Some(h[k as usize - 1].hash()),
            "h{k}; reply {reply} status {st}"
        );
    }
    assert_clean(&st, 5);
}

// INC-I-308-R1 — Decision: a failure means forged low blocks stored earlier stay in the canonical index forever.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r1_backfill_heals_forged_low_blocks() {
    let (h, f) = (honest(6), forged(2));
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(tmp.path(), 6, h[5].hash());
    store(&ctx, &f, &[1, 2]);
    store(&ctx, &h, &[3, 4, 5, 6]);
    let (reply, st) = run(&ctx, 6, h[5].hash(), h.clone()).await;
    for k in 1..=2u64 {
        assert_eq!(
            idx(&ctx, k),
            Some(h[k as usize - 1].hash()),
            "h{k} not healed; reply {reply} status {st}"
        );
    }
}

// INC-I-308-R1 — Decision: a failure means a block is stored below a height the peer could not serve.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r1_backfill_stops_where_peer_has_no_block() {
    let h = honest(6);
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(tmp.path(), 6, h[5].hash());
    store(&ctx, &h, &[1, 6]);
    let peer = vec![h[1].clone(), h[3].clone(), h[4].clone()];
    let (reply, st) = run(&ctx, 6, h[5].hash(), peer).await;
    assert_eq!(
        idx(&ctx, 2),
        None,
        "h2 stored below the hole; reply {reply} status {st}"
    );
    assert_eq!(idx(&ctx, 4), Some(h[3].hash()), "{st}");
    assert_eq!(idx(&ctx, 5), Some(h[4].hash()), "{st}");
    assert_stopped(&st, 3, 2);
}

/// `b` with one attestation bit flipped: same header hash, body rejected by `body_matches_header`.
fn body_variant(b: &Block) -> Block {
    let mut v = b.clone();
    match v.attestation_bitfield.first_mut() {
        Some(byte) => *byte ^= 1,
        None => v.attestation_bitfield.push(1),
    }
    assert_eq!(v.hash(), b.hash(), "fixture: same header hash");
    assert_ne!(v, *b, "fixture: a different body");
    v
}

fn body_at(ctx: &RpcContext, k: u64) -> Option<Block> {
    ctx.block_store.get_block_by_height(k).unwrap()
}

// INC-I-308-M14 — Decision: a failure means one body-rejected block in the middle stops every honest block below it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m14_backfill_rejected_middle_block_keeps_fill_below() {
    let h = honest(6);
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(tmp.path(), 6, h[5].hash());
    let mut served = h.clone();
    served[2] = body_variant(&h[2]);
    let (reply, st) = run(&ctx, 6, h[5].hash(), served.clone()).await;
    for k in [1u64, 2, 4, 5, 6] {
        assert_eq!(
            body_at(&ctx, k).as_ref(),
            Some(&h[k as usize - 1]),
            "peer A: honest h{k} not stored; reply {reply} status {st}"
        );
    }
    assert_eq!(
        idx(&ctx, 3),
        None,
        "peer A: rejected h3 indexed; status {st}"
    );
    assert_clean(&st, 5);
    let (reply_b, st_b) = run(&ctx, 6, h[5].hash(), h.clone()).await;
    for k in 1..=6u64 {
        assert_eq!(
            body_at(&ctx, k).as_ref(),
            Some(&h[k as usize - 1]),
            "peer B: h{k} not the honest block; reply {reply_b} status {st_b}"
        );
    }
    assert_clean(&st_b, 1);
}

async fn repair(ctx: &RpcContext, tip: u64, blocks: Vec<Block>) -> Value {
    let url = serve_peer(Peer {
        tip,
        blocks,
        garbage: false,
        claim_tip: None,
    })
    .await;
    call(ctx, "repairArchiveFromPeer", json!({"rpc_url": url})).await
}

fn archive_ctx(tmp: &TempDir, tip: u64, tip_hash: Hash) -> (RpcContext, std::path::PathBuf) {
    let arch = tmp.path().join("arch");
    std::fs::create_dir_all(&arch).unwrap();
    let ctx = local_node(&tmp.path().join("blocks"), tip, tip_hash).with_archive_dir(arch.clone());
    (ctx, arch)
}

fn file(arch: &std::path::Path, h: u64) -> std::path::PathBuf {
    arch.join(format!("{h:010}.block"))
}

// INC-I-308-R1 — Decision: a failure means repairArchiveFromPeer archives a forged block linked only to h-1.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r1_repair_archive_refuses_forged_gap_bottom() {
    let h = honest_z(4);
    let f2 = block(Hash::ZERO, 2, h[0].hash(), &KeyPair::from_seed([0x66; 32]));
    let tmp = TempDir::new().unwrap();
    let (ctx, arch) = archive_ctx(&tmp, 4, h[3].hash());
    store(&ctx, &h, &[1, 4]);
    let v = repair(&ctx, 4, vec![h[0].clone(), f2, h[2].clone(), h[3].clone()]).await;
    assert!(!file(&arch, 2).exists(), "forged h2 archived: {v}");
}

// INC-I-308-R1 — Decision: a failure means the archive takes a canonical header with a forged body.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r1_repair_archive_refuses_canonical_header_forged_body() {
    let h = honest_z(3);
    let mut v2 = h[1].clone();
    match v2.attestation_bitfield.first_mut() {
        Some(byte) => *byte ^= 1,
        None => v2.attestation_bitfield.push(1),
    }
    assert_eq!(v2.hash(), h[1].hash(), "fixture: same header hash");
    assert_ne!(v2, h[1], "fixture: a different body");
    let tmp = TempDir::new().unwrap();
    let (ctx, arch) = archive_ctx(&tmp, 3, h[2].hash());
    store(&ctx, &h, &[1, 2, 3]);
    let v = repair(&ctx, 3, vec![h[0].clone(), v2, h[2].clone()]).await;
    if let Ok(data) = std::fs::read(file(&arch, 2)) {
        let got = deserialize_block_compat(&data).unwrap();
        assert_eq!(got, h[1], "forged body archived at h2: {v}");
    }
}

// INC-I-308-R1 — Decision: a failure means forged h1..h3 enter the archive under a stored 4,5.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r1_repair_archive_refuses_forged_h1_middle_top() {
    let (h, f) = (
        honest_z(5),
        chain_on(Hash::ZERO, &KeyPair::from_seed([0x66; 32]), 3),
    );
    let tmp = TempDir::new().unwrap();
    let (ctx, arch) = archive_ctx(&tmp, 5, h[4].hash());
    store(&ctx, &h, &[4, 5]);
    let peer = vec![
        f[0].clone(),
        f[1].clone(),
        f[2].clone(),
        h[3].clone(),
        h[4].clone(),
    ];
    let v = repair(&ctx, 5, peer).await;
    for k in 1..=3 {
        assert!(!file(&arch, k).exists(), "forged h{k} archived: {v}");
    }
}

// INC-I-308-R1 — Decision: a failure means the stricter admit refuses an honest peer on a full-history node.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r1_repair_archive_honest_peer_full_store() {
    let h = honest_z(5);
    let tmp = TempDir::new().unwrap();
    let (ctx, arch) = archive_ctx(&tmp, 5, h[4].hash());
    store(&ctx, &h, &[1, 2, 3, 4, 5]);
    let v = repair(&ctx, 5, h.clone()).await;
    for k in 1..=5u64 {
        let data = std::fs::read(file(&arch, k))
            .unwrap_or_else(|e| panic!("h{k} not archived ({e}): {v}"));
        assert_eq!(
            deserialize_block_compat(&data).unwrap().hash(),
            h[k as usize - 1].hash()
        );
    }
    assert_eq!(v["errors"], json!(0), "{v}");
}
