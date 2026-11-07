//! INC-I-295: above the gate and from next height 10, peer height stops a build
//! only through the bounded first-reply latch. Below the gate or below height
//! 10, the old rule stays. Every test drives the real `try_produce_block`.
//!
//! covers: mod gates peers response first_reply production_gate production_asks
//
// OUTPUT CONTRACT: fn Node::try_produce_block(&mut self) -> Result<()>
//   O2: self.last_produced_slot — Some(block slot) iff minted, else unchanged (None)
//   O2: self.chain_state best_height / best_hash — tip+1 and the block hash iff minted
//   O3: return — Ok(()) on every path; it cannot tell a mint from a defer, so it is
//       asserted inside `drive` on every poll
//   O4: block_store[tip+1] — Some(block with parent = old tip, by this key) iff minted
//   O1, O5: none. O6: network is None in this rig; the send is Milestone M1.2
// PATHS: P1 proceed (mint) | P2 deferred by the old peer rule | P3 held by the latch
// INPUT PARTITIONS:
//   A: next >= 10, gate 0, gap > 5, latch spent   -> P1 (v6.46.0: P2, the bug)
//   B: next >= 10, gate 0, gap = 5, no reply      -> P1 on both (non-vacuity control)
//   C: next >= 10, gate 0, gap > 5, no reply      -> P3, no mint (v6.46.0: P2)
//   D: C, then the reply                          -> P1 (v6.46.0: P2)
//   E: next = 9, gate 0, gap = 4, latch spent     -> P2 on both; the latch is not read
//   F: next = 9, gate 0, gap = 3                  -> P1 on both
//   G: next >= 10, below gate, gap = 6, latch spent -> P2 on both
//   H: next >= 10, below gate, gap = 5            -> P1 on both
// MATRIX: {last_produced_slot, tip, block_store} x {A..H}: A B D F H by `assert_minted`,
//   C E G by `assert_not_minted`; O3 in `drive` for all. Each no-mint arm first shows
//   can_produce == Authorized, and pairs with a mint arm that changes one input:
//   C<->D reply, E<->F gap, G<->H gap, A<->B gap.
// RIG: one producer, so this key is scheduled for every slot. Base blocks go through
//   apply_block on devnet params, then the node takes the testnet shape, because no
//   mint happens at devnet's 1 s slots (`producer_mints_with_small_peer_gap` showed it).
//   Every test re-execs itself: the gate comes from the env and is read once per
//   process, and the testnet default is above these heights. Heights stay below
//   genesis_blocks, so the build takes the bootstrap path; the peer rule runs before
//   that branch. Below-gate partitions keep the taller height out of `peers` with
//   update_network_tip_height: below the gate a Syncing state would block in
//   can_produce and hide the rule under test.
// REQ-PASK-SEC-003 — Decision: a grep hit in the names and messages of this file would reveal a public-detail leak; checked by grep, no runtime test.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::Network;
use doli_node::node::Node;
use network::protocols::SyncResponse;
use network::{PeerId, ProductionAuthorization};
use tempfile::TempDir;

use super::inc_i_289_common::{block, AH_ENV, BELOW_AH};

const POLL: Duration = Duration::from_millis(100);
/// Longer than one 10 s slot, so the window holds a slot boundary and a mint window.
const NO_MINT_BUDGET: Duration = Duration::from_secs(12);
const MINT_BUDGET: Duration = Duration::from_secs(25);
const TIP_ABOVE_TEN: u64 = 12;
const TIP_NEXT_NINE: u64 = 8;
const ABOVE_GATE: u64 = 0;
const CHILD_ENV: &str = "PRODUCTION_ASKS_CHILD";

/// Parent: re-exec `test` alone with the gate at `gate`, relay its probe line and
/// fail with its assertion text; returns false. Child: returns true.
fn in_child_with_gate(test: &str, gate: u64) -> bool {
    if std::env::var_os(CHILD_ENV).is_some() {
        for net in [Network::Devnet, Network::Testnet] {
            let got = net.params().inc_i_288_fork_choice_activation_height;
            assert_eq!(got, gate, "fixture: the child must pick up the gate");
        }
        return true;
    }
    let name = format!("production_asks_chosen_branch::{test}");
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name.as_str(), "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1")
        .env(AH_ENV, gate.to_string())
        .output()
        .expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    // libtest prints `test <name> ... ` before the child's own line on one thread.
    for probe in stdout
        .lines()
        .filter_map(|l| l.find("PROBE ").map(|i| &l[i..]))
    {
        println!("{probe}");
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let failure: Vec<&str> = stderr
        .lines()
        .filter(|l| !l.contains("RUST_BACKTRACE"))
        .collect();
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "{test}: {}",
        failure.join(" ")
    );
    false
}

struct Rig {
    node: Node,
    key: KeyPair,
    tip: u64,
    tip_hash: Hash,
    _tmp: TempDir,
}

/// One producer; base chain h=1..=tip applied through `apply_block`.
async fn rig_at(tip: u64) -> Rig {
    let tmp = TempDir::new().unwrap();
    let key = KeyPair::generate();
    let mut node = Node::new_for_test(tmp.path().to_path_buf(), vec![key.clone()])
        .await
        .expect("Node::new_for_test");
    let params = node.params.clone();
    let mut prev = Hash::ZERO;
    for h in 1..=tip {
        let b = block(h, h as u32, prev, &key, &params);
        prev = b.hash();
        node.apply_block(b, ValidationMode::Light)
            .await
            .expect("fixture: base block");
    }
    assert_eq!(node.chain_state.read().await.best_height, tip, "fixture");
    node.config.network = Network::Testnet;
    node.params = ConsensusParams::testnet();
    Rig {
        node,
        key,
        tip,
        tip_hash: prev,
        _tmp: tmp,
    }
}

fn current_slot(node: &Node) -> u32 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    node.params.timestamp_to_slot(now)
}

/// Two peers report `height` on a hash this node does not hold.
async fn taller_peers(r: &Rig, height: u64) -> PeerId {
    let slot = current_slot(&r.node);
    let other = crypto::hash::hash(b"other branch tip");
    let mut sm = r.node.sync_manager.write().await;
    let (first, second) = (PeerId::random(), PeerId::random());
    sm.add_peer(first, height, other, slot);
    sm.add_peer(second, height, other, slot);
    first
}

/// One peer level with this node; the network tip alone carries the gap.
async fn level_peer_with_tip_at(r: &Rig, height: u64) -> PeerId {
    let slot = current_slot(&r.node);
    let mut sm = r.node.sync_manager.write().await;
    let peer = PeerId::random();
    sm.add_peer(peer, r.tip, r.tip_hash, slot);
    sm.update_network_tip_height(height);
    peer
}

async fn reply(r: &Rig, peer: PeerId) {
    let mut sm = r.node.sync_manager.write().await;
    sm.handle_response(peer, SyncResponse::Headers(vec![]));
}

async fn assert_gate_authorizes(r: &Rig, partition: &str) {
    let slot = current_slot(&r.node);
    let auth = r.node.sync_manager.write().await.can_produce(slot);
    assert_eq!(
        auth,
        ProductionAuthorization::Authorized,
        "{partition}: precondition: can_produce must not be the stop under test"
    );
}

async fn assert_peer_gap(r: &Rig, gap: u64, partition: &str) {
    let best = r.node.sync_manager.read().await.best_peer_height();
    assert_eq!(best, r.tip + gap, "{partition}: precondition: peer gap");
}

/// Polls the production entry point; returns whether the tip advanced.
async fn drive(r: &mut Rig, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        r.node
            .try_produce_block()
            .await
            .expect("O3: try_produce_block returns Ok on every path");
        if r.node.chain_state.read().await.best_height > r.tip {
            return true;
        }
        tokio::time::sleep(POLL).await;
    }
    false
}

async fn assert_not_minted(r: &Rig, partition: &str) {
    assert_eq!(
        r.node.last_produced_slot, None,
        "{partition}: O2 last_produced_slot"
    );
    let cs = r.node.chain_state.read().await;
    assert_eq!(cs.best_height, r.tip, "{partition}: O2 best_height");
    assert_eq!(cs.best_hash, r.tip_hash, "{partition}: O2 best_hash");
    let at_next = r.node.block_store.get_block_by_height(r.tip + 1);
    assert!(
        at_next.expect("block_store read").is_none(),
        "{partition}: O4 no block at the next height"
    );
}

async fn assert_minted(r: &Rig, partition: &str) {
    let next = r.tip + 1;
    let built = r
        .node
        .block_store
        .get_block_by_height(next)
        .expect("block_store read")
        .unwrap_or_else(|| panic!("{partition}: O4 block at height {next}"));
    assert_eq!(
        built.header.prev_hash, r.tip_hash,
        "{partition}: O4 the parent is this node's own tip"
    );
    assert_eq!(
        &built.header.producer,
        r.key.public_key(),
        "{partition}: O4 built by this key"
    );
    assert_eq!(
        r.node.last_produced_slot,
        Some(built.header.slot as u64),
        "{partition}: O2 last_produced_slot"
    );
    let cs = r.node.chain_state.read().await;
    assert_eq!(cs.best_height, next, "{partition}: O2 best_height");
    assert_eq!(cs.best_hash, built.hash(), "{partition}: O2 best_hash");
}

// REQ-PASK-001 — Decision: a failure means peers on a refused branch can still stop a build on the branch this node selected, so that branch cannot grow.
#[tokio::test]
async fn producer_keeps_building_on_chosen_branch() {
    if !in_child_with_gate("producer_keeps_building_on_chosen_branch", ABOVE_GATE) {
        return;
    }
    let mut r = rig_at(TIP_ABOVE_TEN).await;
    let peer = taller_peers(&r, TIP_ABOVE_TEN + 20).await;
    reply(&r, peer).await;
    assert_peer_gap(&r, 20, "A").await;
    assert_gate_authorizes(&r, "A").await;

    let minted = drive(&mut r, MINT_BUDGET).await;

    println!("PROBE chosen_branch_mints={}", u8::from(minted));
    assert!(
        minted,
        "A: no mint: peers 20 taller on another hash stopped a build at this node's own slot"
    );
    assert_minted(&r, "A").await;
}

// REQ-PASK-001 — Decision: a failure means the rig cannot mint at this height, so a no-mint result in the first test proves nothing about the peer rule.
#[tokio::test]
async fn producer_mints_with_small_peer_gap() {
    if !in_child_with_gate("producer_mints_with_small_peer_gap", ABOVE_GATE) {
        return;
    }
    let mut r = rig_at(TIP_ABOVE_TEN).await;
    taller_peers(&r, TIP_ABOVE_TEN + 5).await;
    assert_peer_gap(&r, 5, "B").await;
    assert_gate_authorizes(&r, "B").await;

    let minted = drive(&mut r, MINT_BUDGET).await;

    assert!(minted, "B: no mint at a gap of 5");
    assert_minted(&r, "B").await;
}

// REQ-PASK-003 — Decision: a failure means the latch is not on the production path, or the reply does not release it, so a stale restart builds at once or a build never resumes.
#[tokio::test]
async fn taller_peer_holds_build_until_first_reply() {
    if !in_child_with_gate("taller_peer_holds_build_until_first_reply", ABOVE_GATE) {
        return;
    }
    let mut r = rig_at(TIP_ABOVE_TEN).await;
    let peer = taller_peers(&r, TIP_ABOVE_TEN + 20).await;
    assert_gate_authorizes(&r, "C").await;

    let held = !drive(&mut r, NO_MINT_BUDGET).await;

    assert!(held, "C: minted before any peer replied");
    assert_not_minted(&r, "C").await;

    reply(&r, peer).await;
    assert_gate_authorizes(&r, "D").await;
    let minted = drive(&mut r, MINT_BUDGET).await;

    assert!(minted, "D: no mint after the first reply");
    assert_minted(&r, "D").await;
}

// REQ-PASK-002 — Decision: a failure means the height-below-10 rule changed, which reopens orphan forks on a young chain and breaks the wiped-disk no-mint contract.
#[tokio::test]
async fn next_height_below_ten_keeps_old_peer_rule() {
    if !in_child_with_gate("next_height_below_ten_keeps_old_peer_rule", ABOVE_GATE) {
        return;
    }
    let mut far = rig_at(TIP_NEXT_NINE).await;
    let peer = taller_peers(&far, TIP_NEXT_NINE + 4).await;
    reply(&far, peer).await;
    assert_peer_gap(&far, 4, "E").await;
    assert_gate_authorizes(&far, "E").await;
    let deferred = !drive(&mut far, NO_MINT_BUDGET).await;
    assert!(deferred, "E: next height 9 at a gap of 4 must defer");
    assert_not_minted(&far, "E").await;

    let mut near = rig_at(TIP_NEXT_NINE).await;
    taller_peers(&near, TIP_NEXT_NINE + 3).await;
    assert_peer_gap(&near, 3, "F").await;
    assert_gate_authorizes(&near, "F").await;
    let minted = drive(&mut near, MINT_BUDGET).await;
    assert!(minted, "F: next height 9 at a gap of 3 must mint");
    assert_minted(&near, "F").await;
}

// REQ-PASK-002 — Decision: a failure means the change leaked below its activation height, so nodes on history before the gate build differently without a gate.
#[tokio::test]
async fn below_gate_keeps_old_peer_rule() {
    if !in_child_with_gate("below_gate_keeps_old_peer_rule", BELOW_AH) {
        return;
    }
    let mut far = rig_at(TIP_ABOVE_TEN).await;
    let peer = level_peer_with_tip_at(&far, TIP_ABOVE_TEN + 6).await;
    reply(&far, peer).await;
    assert_peer_gap(&far, 6, "G").await;
    assert_gate_authorizes(&far, "G").await;
    let deferred = !drive(&mut far, NO_MINT_BUDGET).await;
    assert!(deferred, "G: below the gate a gap of 6 must defer");
    assert_not_minted(&far, "G").await;

    let mut near = rig_at(TIP_ABOVE_TEN).await;
    level_peer_with_tip_at(&near, TIP_ABOVE_TEN + 5).await;
    assert_peer_gap(&near, 5, "H").await;
    assert_gate_authorizes(&near, "H").await;
    let minted = drive(&mut near, MINT_BUDGET).await;
    assert!(minted, "H: below the gate a gap of 5 must mint");
    assert_minted(&near, "H").await;
}
