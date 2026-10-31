//! INC-I-289 J-3 (AUDIT-P2-639-007, SEC-LG-639-001): above the INC-I-288 gate the
//! plan door weighs a candidate branch by the bond of its supporters. A block whose
//! slot does not advance past its parent's slot fails apply (`SlotNotAdvancing`), so
//! its support must not let the branch win the plan.
//!
// OUTPUT CONTRACT: Node::handle_completed_fork_recovery (plan door,
//   plan_reorg_with_support above the gate).
//   O1 "Executing reorg" log lines during the evaluation (the plan won)
//   O2 chain tip (hash, height, canonical entry, persisted tip, sync tip)
//   O3 return value
// PATHS: recovered fork -> plan door (the simple door has no common ancestor here)
// INPUT PARTITIONS (above the gate, ours = D by one producer):
//   head equal-slot child of its in-branch parent, backed by every other producer
//     -> no reorg runs, tip stays D (j3a)
//   head advances its parent's slot, same backing -> reorg, tip = head (j3b)
// MATRIX: j3a O1-O3 · j3b O1-O3
//! covers: crates/network/src/sync/reorg/support.rs doors.rs bins/node/src/node/branch_support.rs fork_recovery.rs

#[path = "support/block_sig.rs"]
mod block_sig;
#[path = "it/inc_i_289_common.rs"]
mod inc_i_289_common;

use std::sync::{Arc, Mutex};

use doli_core::{Attestation, Block};
use inc_i_289_common::*;
use network::sync::CompletedRecovery;
use network::PeerId;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;

const S: u32 = 210;

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<String>>>);

struct Fields(String);

impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{}={v:?} ", f.name()));
    }
}

impl<T: tracing::Subscriber> Layer<T> for Logs {
    fn on_event(&self, e: &tracing::Event<'_>, _: Context<'_, T>) {
        let mut f = Fields(String::new());
        e.record(&mut f);
        self.0.lock().unwrap().push(f.0);
    }
}

impl Logs {
    fn mentioning(&self, needle: &str) -> usize {
        let lines = self.0.lock().unwrap();
        lines.iter().filter(|l| l.contains(needle)).count()
    }
}

/// Rig above the gate, tip D at F+1 (one producer on our side).
async fn above_gate_rig() -> Rig {
    let r = rig(&[S]).await;
    let gate = (r.node.config.network.params()).inc_i_288_fork_choice_activation_height;
    assert!(gate <= F, "fixture: the node runs above the gate ({gate})");
    r
}

/// `head` cached, then attested over gossip by every producer but the node itself.
async fn back(r: &mut Rig, head: &Block, height: u64) {
    (r.node.fork_block_cache.write().await).insert(head.hash(), head.clone());
    let own = *r.node.producer_key.as_ref().unwrap().public_key();
    for k in r.keys.clone().iter().filter(|k| *k.public_key() != own) {
        let a = Attestation::new(
            head.hash(),
            head.header.slot,
            height,
            1,
            k.private_key(),
            *k.public_key(),
        );
        r.node
            .on_new_attestation(a.to_bytes(), PeerId::random())
            .await;
    }
}

/// Walks `branch` (connected at F) into the plan door; (result, "Executing reorg" lines).
async fn walk(r: &mut Rig, branch: &[Block]) -> (anyhow::Result<()>, usize) {
    let logs = Logs::default();
    let _g = tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
    let rec = CompletedRecovery {
        blocks: branch.to_vec(),
        connection_point: r.f_hash,
        suppliers: Vec::new(),
    };
    let out = r.node.handle_completed_fork_recovery(rec).await;
    (out, logs.mentioning("Executing reorg"))
}

// J-3a INC-I-289 — Decision: a failure means an equal-slot child (rejected at apply) still adds its bond, wins the plan and runs a doomed reorg the door repeats.
#[tokio::test]
async fn j3a_equal_slot_child_does_not_win_the_plan() {
    let mut r = above_gate_rig().await;
    let d = r.ours[0].clone();
    let p = r.node.params.clone();
    let s1 = chain_on(&r, r.f_hash, F + 1, &[S + 2]).remove(0);
    let b = block(F + 2, S + 2, s1.hash(), leader(&r.keys, S + 2), &p);
    assert_eq!(
        b.header.slot, s1.header.slot,
        "fixture: b does not advance on s1"
    );
    back(&mut r, &b, F + 2).await;
    let (out, executed) = walk(&mut r, &[s1, b]).await;
    assert_eq!(
        executed, 0,
        "J-3a: a branch whose head does not advance the slot won the plan door (reorg executed)"
    );
    assert!(out.is_ok(), "J-3a: the evaluation returns Ok: {out:?}");
    assert_on(&r, &d, F + 1, "J-3a: tip stays D").await;
}

// J-3b INC-I-289 — Decision: a failure means the slot screen also removed an advancing heavier branch.
#[tokio::test]
async fn j3b_advancing_slot_heavier_branch_wins() {
    let mut r = above_gate_rig().await;
    let s1 = chain_on(&r, r.f_hash, F + 1, &[S + 2]).remove(0);
    let c = chain_on(&r, s1.hash(), F + 2, &[S + 3]).remove(0);
    back(&mut r, &c, F + 2).await;
    let (out, executed) = walk(&mut r, &[s1, c.clone()]).await;
    assert!(out.is_ok(), "J-3b: the evaluation returns Ok: {out:?}");
    assert!(
        executed >= 1,
        "J-3b: the heavier advancing branch must win the plan"
    );
    assert_on(&r, &c, F + 2, "J-3b: tip = the advancing head").await;
}
