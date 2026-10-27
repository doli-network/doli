//! INC-I-281 M2.2 fixtures: a node ran a dead branch F -> D1 -> D2 (D3 exists on
//! the dead peer only) and a fork-remedy rollback took it back to F; the canonical
//! branch is C1.. on F.
//!
//! OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::Block;
use doli_node::node::{Node, RollbackAuthority, RollbackOutcome};
use network::protocols::{SyncRequest, SyncResponse};
use network::PeerId;
use tempfile::TempDir;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;

use crate::inc_i_236_m2_reorg_support::{block, leader, Sig};

/// Fork point: the last block both branches share (mid-epoch on devnet).
pub const F: u64 = 5;
/// D3 (slot 207) is the dead peer's next block; this node never applied it.
const DEAD_SLOTS: [u32; 3] = [200, 201, 207];
/// C1 has a later slot than D1, as in the incident.
const CANON_SLOTS: [u32; 5] = [202, 203, 204, 205, 206];

pub struct Rig {
    pub node: Node,
    pub params: ConsensusParams,
    pub f_hash: Hash,
    pub dead: Vec<Block>,
    pub canon: Vec<Block>,
    keys: Vec<KeyPair>,
    _tmp: TempDir,
}

/// A scheduled child of `parent` at `height`, `slot`; `slot` must be unused by the rig.
pub fn child_of(r: &Rig, parent: &Block, height: u64, slot: u32) -> Block {
    let by = leader(&r.keys, slot);
    block(height, slot, parent.hash(), by, Sig::Valid, &r.params)
}

/// Every chain-tip output a re-adoption could move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Obs {
    pub best_hash: Hash,
    pub best_height: u64,
    pub at_f1: Option<Hash>,
    pub at_f2: Option<Hash>,
    pub persisted: Option<Hash>,
    pub sync_height: u64,
}

fn chain(params: &ConsensusParams, keys: &[KeyPair], prev: Hash, slots: &[u32]) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::with_capacity(slots.len());
    let mut prev = prev;
    for (i, slot) in slots.iter().enumerate() {
        let b = block(
            F + 1 + i as u64,
            *slot,
            prev,
            leader(keys, *slot),
            Sig::Valid,
            params,
        );
        prev = b.hash();
        out.push(b);
    }
    out
}

/// Base h=1..=F, then the node's own dead D1 (h=F+1) and D2 (h=F+2): tip = D2.
pub async fn rig() -> Rig {
    let tmp = TempDir::new().unwrap();
    let keys: Vec<KeyPair> = (0..3).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(tmp.path().to_path_buf(), keys.clone())
        .await
        .expect("Node::new_for_test");
    let params = node.params.clone();
    let mut prev = Hash::ZERO;
    for h in 1..=F {
        let b = block(h, h as u32, prev, &keys[0], Sig::Valid, &params);
        prev = b.hash();
        node.apply_block(b, ValidationMode::Light)
            .await
            .expect("fixture: base block");
    }
    let dead = chain(&params, &keys, prev, &DEAD_SLOTS);
    let canon = chain(&params, &keys, prev, &CANON_SLOTS);
    for b in &dead[..2] {
        node.apply_block(b.clone(), ValidationMode::Light)
            .await
            .expect("fixture: dead block");
    }
    assert_eq!(
        node.chain_state.read().await.best_hash,
        dead[1].hash(),
        "fixture: tip = dead D2"
    );
    Rig {
        node,
        params,
        f_hash: prev,
        dead,
        canon,
        keys,
        _tmp: tmp,
    }
}

/// The coordinator's ShallowRollback { depth: 1 }, executed twice (D2, then D1).
/// Returns the instant the rollback completed.
pub async fn roll_back_to_f(r: &mut Rig) -> Instant {
    for _ in 0..2 {
        let out = r
            .node
            .rollback_one_block(RollbackAuthority::CoordinatorApproved { depth: 1 })
            .await
            .expect("rollback_one_block");
        assert_eq!(
            out,
            RollbackOutcome::RolledBack,
            "precondition: rolled back"
        );
    }
    let done = Instant::now();
    let o = obs(r).await;
    assert_eq!(o.best_hash, r.f_hash, "precondition: tip = F");
    assert_eq!(
        (o.at_f1, o.at_f2),
        (None, None),
        "precondition: index rows gone"
    );
    for d in &r.dead[..2] {
        assert!(
            r.node.block_store.get_block(&d.hash()).unwrap().is_some(),
            "precondition: the dead body survives the rollback"
        );
    }
    done
}

/// Real time is used (tokio test-util is not enabled for this crate), so a cell
/// that relies on the hold must check it never left the window.
pub fn assert_within_window(r: &Rig, since_rollback: Instant) {
    assert!(
        since_rollback.elapsed() < Duration::from_secs(slots(r).0),
        "precondition: every offer landed inside one slot after the rollback ({:?})",
        since_rollback.elapsed()
    );
}

pub async fn obs(r: &Rig) -> Obs {
    let (best_hash, best_height) = {
        let cs = r.node.chain_state.read().await;
        (cs.best_hash, cs.best_height)
    };
    Obs {
        best_hash,
        best_height,
        at_f1: r.node.block_store.get_hash_by_height(F + 1).unwrap(),
        at_f2: r.node.block_store.get_hash_by_height(F + 2).unwrap(),
        persisted: r.node.state_db.get_chain_state().map(|c| c.best_hash),
        sync_height: r.node.sync_manager.read().await.local_tip().0,
    }
}

/// Tip at F with nothing canonical above it, on every output.
pub fn at_fork_point(r: &Rig, o: &Obs) -> bool {
    o.best_hash == r.f_hash
        && o.best_height == F
        && o.at_f1.is_none()
        && o.at_f2.is_none()
        && o.persisted == Some(r.f_hash)
        && o.sync_height == F
}

pub async fn assert_on(r: &Rig, b: &Block, height: u64, label: &str) {
    let o = obs(r).await;
    assert_eq!(o.best_hash, b.hash(), "{label}: tip hash ({o:?})");
    assert_eq!(o.best_height, height, "{label}: tip height");
    assert_eq!(
        r.node.block_store.get_hash_by_height(height).unwrap(),
        Some(b.hash()),
        "{label}: canonical entry"
    );
    assert_eq!(o.persisted, Some(b.hash()), "{label}: persisted tip");
    assert_eq!(o.sync_height, height, "{label}: sync manager tip");
}

pub async fn gossip(r: &mut Rig, b: &Block) {
    r.node
        .handle_new_block(b.clone(), PeerId::random())
        .await
        .expect("handle_new_block returns Ok");
}

/// Past the hold window whichever slot source it uses, plus a margin.
pub fn past_window(r: &Rig) -> Duration {
    Duration::from_secs(slots(r).1 + 1)
}

/// (shorter, longer) slot duration: the test node runs 1 s consensus slots while
/// its network params say 10 s; the hold may be keyed to either.
fn slots(r: &Rig) -> (u64, u64) {
    let a = r.params.slot_duration;
    let b = r.node.config.network.params().slot_duration;
    (a.min(b), a.max(b))
}

/// Answers the node's header and body requests from `chain` through the node's
/// sync-response intake until the requests stop; returns the kinds served.
pub async fn serve_sync(node: &mut Node, peer: PeerId, chain: &[Block]) -> Vec<&'static str> {
    let mut served = Vec::new();
    for _ in 0..12 {
        let Some((to, req)) = node.sync_manager.write().await.next_request() else {
            break;
        };
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
        assert_eq!(to, peer, "fixture: the only peer is asked");
        node.on_sync_response(to, resp)
            .await
            .expect("on_sync_response returns Ok");
    }
    served
}

pub fn probe_reapply(path: &str, readopted: bool) {
    println!("INC281_PROBE reapply={path} readopted={}", readopted as u8);
}

pub fn probe_suppressor(name: &str, peer_progress: bool) {
    println!(
        "INC281_PROBE suppressor={name} peer_progress={}",
        peer_progress as u8
    );
}

#[derive(Clone, Default)]
pub struct Logs(Arc<Mutex<Vec<String>>>);

impl Logs {
    pub fn mentioning(&self, needle: &str) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.contains(needle))
            .count()
    }

    pub fn subscriber(&self) -> impl tracing::Subscriber + Send + Sync {
        tracing_subscriber::registry().with(self.clone())
    }
}

struct Fields(String);

impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{}={v:?} ", f.name()));
    }
}

impl<S: tracing::Subscriber> Layer<S> for Logs {
    fn on_event(&self, e: &tracing::Event<'_>, _: Context<'_, S>) {
        let mut f = Fields(String::new());
        e.record(&mut f);
        self.0.lock().unwrap().push(f.0);
    }
}
