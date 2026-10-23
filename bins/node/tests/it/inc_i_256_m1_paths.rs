//! INC-I-256 M1 — a block that fails inside the transaction loop, delivered by
//! each intake path, leaves the producer set exactly as on a control node that
//! never saw it. Production RocksDB UTXO backend.
//!
// covers: apply_block producer_undo helpers mod block_handling rollback periodic fork_cache set_core set_lifecycle set_persistence chain snapshot_install
//
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()> (and its callers)
//   O1 return: Err naming the ghost transfer (direct call); callers return Ok / break / try the next sibling
//   O2 producer set: serialize_canonical, pending_update_count, pending_updates_for(p),
//      withdrawal_pending_count(p), is_slashed_or_slash_queued(p), pending_addbond_count(p)
//   O3 state_root_component(true) and (false) of the live set
//   O4 chain tip (unchanged by F; moved by the valid sibling on the drain path)
// PATHS: P-GOSSIP handle_new_block(F) twice | P-SYNC pending sync blocks applied by run_periodic_tasks
//   | P-DRAIN drain_cached_children with F and a valid sibling B2 | P-DIRECT apply_block(F, Light)
// INPUT PARTITIONS: F = [coinbase, RequestWithdrawal(p, 2 of 2 bonds), Transfer of a missing outpoint]
//   at a mid-epoch height below the holdings gate; one partition covers all paths (the residue
//   comes from the same in-loop arm on every path)
// MATRIX: O2-O4 x {P-GOSSIP x 2 attempts, P-SYNC, P-DRAIN, P-DIRECT} + O1 x P-DIRECT = 5 tests

use doli_core::Block;
use network::protocols::{SyncRequest, SyncResponse};
use network::PeerId;
use tracing_subscriber::layer::SubscriberExt;

use super::inc_i_182_m1_common::Logs;
use super::inc_i_255_m2_common::Rig;
use super::inc_i_255_m4_common::err_text;
use super::inc_i_256_m1_common::*;

const MID_EPOCH_TIP: u64 = 5;

// R-1 (INC-I-256) — Decision: a failure means a peer can push the same failing block twice through gossip or a sync response and leave this node with queued withdrawals and a different pending root than every other node.
#[tokio::test]
async fn inc_i_256_m1_gossip_twice_leaves_no_residue() {
    let (mut s, c) = twins(MID_EPOCH_TIP).await;
    let p = p_of(&s);
    let want = obs(&c.node, &p).await;
    let before = tip(&s).await;
    let f = failing_block(&s).await;
    s.node.floor_fallback_window = true;
    let peer = PeerId::random();
    let mut seen = Vec::new();
    for attempt in 1..=2 {
        let r = s.node.handle_new_block(f.clone(), peer).await;
        let got = obs(&s.node, &p).await;
        report(&got, &want);
        seen.push((attempt, r.is_ok(), tip(&s).await, got));
    }
    for (attempt, ok, t, got) in &seen {
        let what = format!("handle_new_block(F) attempt {attempt}");
        assert!(ok, "{what}: gossip intake returns Ok on a rejected block");
        assert_eq!(*t, before, "{what}: F must not move the tip");
        assert_obs(got, &want, &what);
    }
}

/// Answers the node's header and body requests from `chain` until the
/// requests stop; returns the request kinds served.
async fn serve_sync(s: &Rig, peer: PeerId, chain: &[Block]) -> Vec<&'static str> {
    let mut served = Vec::new();
    for _ in 0..12 {
        let Some((to, req)) = s.node.sync_manager.write().await.next_request() else {
            break;
        };
        assert_eq!(to, peer, "FIXTURE: the only peer is asked");
        let resp = match req {
            SyncRequest::GetHeaders {
                start_hash,
                max_count,
            } => {
                served.push("headers");
                let from = chain
                    .iter()
                    .position(|b| b.header.prev_hash == start_hash)
                    .unwrap_or(chain.len());
                SyncResponse::Headers(
                    chain[from..]
                        .iter()
                        .take(max_count as usize)
                        .map(|b| b.header.clone())
                        .collect(),
                )
            }
            SyncRequest::GetBodies { hashes } => {
                served.push("bodies");
                SyncResponse::Bodies(
                    chain
                        .iter()
                        .filter(|b| hashes.contains(&b.hash()))
                        .cloned()
                        .collect(),
                )
            }
            _ => {
                served.push("other");
                continue;
            }
        };
        s.node.sync_manager.write().await.handle_response(to, resp);
    }
    served
}

// R-1 (INC-I-256) — Decision: a failure means a failing block that arrives through header-first sync and is applied by the periodic loop leaves producer-set residue on this node only.
#[tokio::test]
async fn inc_i_256_m1_periodic_sync_apply_leaves_no_residue() {
    let (s, c) = twins(MID_EPOCH_TIP).await;
    let mut s = s;
    let p = p_of(&s);
    let want = obs(&c.node, &p).await;
    let before = tip(&s).await;
    let (h, slot, prev) = s.best().await;
    let f = failing_block_at(&s, h + 1, slot + 1, prev);
    let g = block_at(&s, h + 2, slot + 2, f.hash(), Vec::new());
    let k = block_at(&s, h + 3, slot + 3, g.hash(), Vec::new());
    let chain = vec![f, g, k.clone()];
    let peer = PeerId::random();
    s.node
        .sync_manager
        .write()
        .await
        .add_peer(peer, h + 3, k.hash(), k.header.slot);
    let served = serve_sync(&s, peer, &chain).await;
    let phase = s
        .node
        .sync_manager
        .read()
        .await
        .sync_state_name()
        .to_string();
    let logs = Logs::default();
    let periodic = {
        let _g =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
        s.node.run_periodic_tasks().await
    };
    let got = obs(&s.node, &p).await;
    report(&got, &want);
    assert!(
        served.contains(&"headers") && served.contains(&"bodies"),
        "PRECONDITION: header-first sync fetched F's header and body; served {served:?}, state {phase}"
    );
    assert!(
        !logs
            .mentioning("Failed to apply pending sync block")
            .is_empty(),
        "PRECONDITION: the periodic pending-sync loop applied F and saw it fail; state {phase}"
    );
    assert!(periodic.is_ok(), "run_periodic_tasks: {:?}", periodic.err());
    assert_eq!(
        tip(&s).await,
        before,
        "F must not move the tip (served {served:?}, state before the tick {phase})"
    );
    assert_obs(&got, &want, "periodic pending-sync apply of F");
}

// R-1 (INC-I-256) — Decision: a failure means the fork-cache drain applies the next valid sibling on top of a failed sibling's residue, so the residue is committed to disk with the valid block.
#[tokio::test]
async fn inc_i_256_m1_drain_sibling_after_failed_child_matches_control() {
    let (mut s, mut c) = twins(MID_EPOCH_TIP - 1).await;
    let p = p_of(&s);
    let (h, slot, prev) = s.best().await;
    let t = block_at(&s, h + 1, slot + 1, prev, Vec::new());
    let f = failing_block_at(&s, h + 2, slot + 2, t.hash());
    let b2 = block_at(&s, h + 2, slot + 3, t.hash(), Vec::new());
    c.apply(t.clone()).await.expect("control applies T");
    c.apply(b2.clone()).await.expect("control applies B2");
    {
        let mut cache = s.node.fork_block_cache.write().await;
        cache.insert(f.hash(), f.clone());
        cache.insert(b2.hash(), b2.clone());
    }
    s.node.floor_fallback_window = true;
    let r = s.node.handle_new_block(t, PeerId::random()).await;
    let got = obs(&s.node, &p).await;
    let want = obs(&c.node, &p).await;
    report(&got, &want);
    assert!(r.is_ok(), "handle_new_block(T): {}", err_text(&r));
    assert_eq!(
        tip(&s).await,
        (h + 2, b2.hash()),
        "PRECONDITION: the drain tried F (lower slot) and then applied B2"
    );
    assert!(
        !s.node.fork_block_cache.read().await.contains_key(&f.hash()),
        "PRECONDITION: the drain consumed F"
    );
    assert_obs(&got, &want, "drain: F failed, then B2 applied");
}

// R-1 (INC-I-256) — Decision: a failure means apply_block itself returns Err with the producer set mutated, which every caller inherits.
#[tokio::test]
async fn inc_i_256_m1_direct_apply_err_leaves_no_residue() {
    let (mut s, c) = twins(MID_EPOCH_TIP).await;
    let p = p_of(&s);
    let want = obs(&c.node, &p).await;
    let before = tip(&s).await;
    let f = failing_block(&s).await;
    let ghost = f.transactions[2].hash().to_string();
    let r = s.apply(f).await;
    let got = obs(&s.node, &p).await;
    report(&got, &want);
    let text = err_text(&r);
    assert!(
        r.is_err() && text.contains(&ghost),
        "PRECONDITION: F must fail inside the tx loop at the ghost transfer {ghost}; got {text}"
    );
    assert_eq!(tip(&s).await, before, "F must not move the tip");
    assert_obs(&got, &want, "apply_block(F, Light) returned Err");
}
