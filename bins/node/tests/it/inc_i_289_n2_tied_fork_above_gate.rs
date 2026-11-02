//! INC-I-289 N-2: the above-gate twin of the INC-I-190 F2 tied-fork test
//! (`tied_fork_finality::test_inc_i190_f2_*`, now pinned in a below-gate child).
//! Same shape: base h=1..=F and sibling A (h=F+1) by the node's own key, every
//! producer attests A, then B (h=F+1), D1, D2 by three distinct slot leaders
//! arrive by gossip. Under the K-1 bond rule both sides weigh 3: A = its producer
//! plus the two OTHER producers' gossip attestations (the node's own attestation
//! does not count); B..D2 = three distinct leaders.
//!
//! Which tie rule applies: B is a HeightOccupied sibling and D1/D2 are orphans of
//! a retained sibling, so every arrival reaches `retain_sibling_and_try_escape`
//! (`block_handling.rs:166-234`). That door takes a branch only on a strict gain
//! (`wedge_escape.rs:260-291`, AUDIT-P2-003): a tie keeps our tip whatever the
//! hashes. No peer is registered, so the fork-recovery walk (lower-hash tie rule)
//! is not reachable. Expected tip: A, for both hash orders of D2 vs A; then the
//! node keeps producing on A.
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(block, peer) (HeightOccupied / Orphan of a retained
//   sibling -> wedge door), fn Node::run_periodic_tasks(), fn SyncManager::can_produce(slot),
//   the node's own block (build_block_content + compute_block_vdf, then fn Node::apply_block)
//   O1: chain_state best_hash / best_height, canonical entry, persisted tip, sync tip (assert_tip)
//   O2: fork_block_cache + wedge_retained_tips hold D2 (the branch was admitted and weighed)
//   O3: the wedge door's verdict line for D2 ("not strictly heavier"; never "branch heavier")
//   O4: last_finalized_height() == None (no depth-0 lock decides the tie)
//   O5: per tick (N_TICKS): tip == A (no flap) and can_produce(slot) == Authorized
//   O6: the node's own next block: prev_hash == A; applied -> tip at h=F+2
//   O7: return values Ok (handle_new_block, run_periodic_tasks, apply_block)
//   O8: K-1 support of each side (distinct supporters x bond), computed from public state
//   PATHS: P1 equal support 3 vs 3 -> tie -> keep A -> production on A;
//     P2 control: A supported 2 vs 3 -> the same branch reorgs the node to D2
//   INPUT PARTITIONS: N2A-TIE-HI D2 hashes above A; N2A-TIE-LO D2 hashes below A
//     (a lower-hash rule would flip only this cell); N2A-CTRL A attested by one other
//     producer. N2B (lower-hash mirror) N/A: the rule for this shape is keep-ours.
//   MATRIX: N2A-TIE-HI O1-O8 · N2A-TIE-LO O1-O8 · N2A-CTRL O1,O7,O8
//   ISOLATION: every cell runs in a re-exec child alone (O3 is a thread-local capture)
//   covers: block_handling wedge_escape branch_support doors fork_choice production_gate assembly

use std::collections::BTreeMap;

use crypto::{Hash, KeyPair};
use doli_core::validation::ValidationMode;
use doli_core::{Attestation, Block, Network, NetworkParams};
use doli_node::node::Node;
use network::{PeerId, ProductionAuthorization};
use tempfile::TempDir;
use tracing_subscriber::layer::SubscriberExt;

use super::inc_i_182_m1_common::Logs;
use super::inc_i_236_m2_common::{build_devnet_block, sign_into_pi};
use super::inc_i_289_common::{block, leader, AH_ENV, F};
use super::inc_i_289_k1_bond_weight::{assert_tip, tip};

/// Periodic ticks driven after the tie, each followed by a production check.
const N_TICKS: usize = 5;

const N2A: &str =
    "inc_i_289_n2_tied_fork_above_gate::n2a_tied_fork_above_gate_resolves_and_keeps_producing";
const ISOLATED_ENV: &str = "INC_I_289_N2_ISOLATED_CHILD";

// INC-I-289: tracing-core caches callsite interest per process, and a parallel test that reaches the wedge door's log callsite first, while this test's thread-local capture is the only subscriber, caches it as `never` (O3 then reads no line), so the test re-runs alone in a child (no AH override) and every assertion stays as written.
/// Parent: re-exec N2A alone; returns true in the child.
fn in_isolated_child() -> bool {
    if std::env::var_os(ISOLATED_ENV).is_some() {
        return true;
    }
    let out = std::process::Command::new(std::env::current_exe().expect("current exe"))
        .args([N2A, "--exact", "--test-threads=1", "--nocapture"])
        .env(ISOLATED_ENV, "1")
        .env_remove(AH_ENV)
        .output()
        .expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    for line in stderr.lines().filter(|l| l.starts_with("[N2A")) {
        eprintln!("{line}");
    }
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "isolated N2A child failed or did not run:\n{stdout}\n{stderr}"
    );
    false
}

struct Tie {
    node: Node,
    keys: Vec<KeyPair>,
    a: Block,
    /// B (h=F+1), D1 (h=F+2), D2 (h=F+3) on F.
    branch: Vec<Block>,
    peer: PeerId,
    _tmp: TempDir,
}

/// F2's shape: base h=1..=F and A (h=F+1, slot 100) by the node's key, applied;
/// `keys[..attesters]` attest A by gossip; B, D1, D2 on F at slots s, s+1, s+2
/// (three distinct leaders), D2 hashing below A iff `d2_lower`.
async fn tie_shape(attesters: usize, d2_lower: bool) -> Tie {
    let gate = NetworkParams::load(Network::Devnet).inc_i_288_fork_choice_activation_height;
    assert!(
        gate <= F + 1,
        "fixture: the node must run above the gate ({gate})"
    );
    let tmp = TempDir::new().unwrap();
    let keys: Vec<KeyPair> = (0..3).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(tmp.path().to_path_buf(), keys.clone())
        .await
        .expect("Node::new_for_test");
    let own = *node.producer_key.as_ref().expect("own key").public_key();
    assert_eq!(
        &own,
        keys[0].public_key(),
        "fixture: keys[0] is the node's key"
    );
    let p = node.params.clone();
    let mut prev = Hash::ZERO;
    for h in 1..=F {
        let b = block(h, h as u32, prev, &keys[0], &p);
        prev = b.hash();
        node.apply_block(b, ValidationMode::Light)
            .await
            .expect("fixture: base block");
    }
    let f_hash = prev;
    let a = block(F + 1, 100, f_hash, &keys[0], &p);
    node.apply_block(a.clone(), ValidationMode::Light)
        .await
        .expect("fixture: A applied");
    assert_eq!(tip(&node).await, a.hash(), "fixture: tip is A");

    let peer = PeerId::random();
    for k in &keys[..attesters] {
        let att = Attestation::new(
            a.hash(),
            a.header.slot,
            F + 1,
            1,
            k.private_key(),
            *k.public_key(),
        );
        assert!(att.verify().is_ok(), "fixture: attestation self-signed");
        node.on_new_attestation(att.to_bytes(), peer).await;
    }

    let branch = (200..800u32)
        .map(|s| {
            let b = block(F + 1, s, f_hash, leader(&keys, s), &p);
            let d1 = block(F + 2, s + 1, b.hash(), leader(&keys, s + 1), &p);
            let d2 = block(F + 3, s + 2, d1.hash(), leader(&keys, s + 2), &p);
            vec![b, d1, d2]
        })
        .find(|br| (br[2].hash().as_bytes() < a.hash().as_bytes()) == d2_lower)
        .expect("fixture: a branch whose tip hashes on the wanted side of A");
    Tie {
        node,
        keys,
        a,
        branch,
        peer,
        _tmp: tmp,
    }
}

/// K-1 support in bond as `support_view` + `union_weight` count it for blocks
/// without bits: each block's producer plus the gossip attesters held for it,
/// the node's own key excluded from gossip, each distinct supporter once.
/// (No attestation names a B-branch block here, so fork credit adds nothing.)
async fn support(node: &Node, blocks: &[&Block]) -> BTreeMap<[u8; 32], u64> {
    let own = *node.producer_key.as_ref().unwrap().public_key();
    let height = node.chain_state.read().await.best_height;
    let audit = NetworkParams::load(Network::Devnet).security_audit_activation_height;
    let pending = node.sync_manager.read().await.pending_attesters();
    let ps = node.producer_set.read().await;
    let mut out = BTreeMap::new();
    for b in blocks {
        let gossip = pending
            .iter()
            .filter(|(h, _)| *h == b.hash())
            .flat_map(|(_, v)| v.iter().copied())
            .filter(|pk| *pk != own);
        for pk in std::iter::once(b.header.producer).chain(gossip) {
            let bond = ps
                .get_by_pubkey(&pk)
                .map(|pi| pi.selection_weight_at(height, audit))
                .expect("fixture: supporter is a producer");
            out.insert(*pk.as_bytes(), bond);
        }
    }
    out
}

async fn deliver_branch(t: &mut Tie) {
    for b in t.branch.clone() {
        t.node
            .handle_new_block(b, t.peer)
            .await
            .expect("handle_new_block returns Ok");
    }
}

/// N2A-TIE: equal support; the tie keeps A, then the node keeps producing on A.
async fn tie_cell(d2_lower: bool) {
    let cell = if d2_lower { "N2A-TIE-LO" } else { "N2A-TIE-HI" };
    let logs = Logs::default();
    let _g = tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
    let mut t = tie_shape(3, d2_lower).await;
    let (a, d2) = (t.a.clone(), t.branch[2].clone());

    let ours = support(&t.node, &[&a]).await;
    let theirs = support(&t.node, &[&t.branch[0], &t.branch[1], &d2]).await;
    let (w_ours, w_theirs): (u64, u64) = (ours.values().sum(), theirs.values().sum());
    eprintln!("[{cell}] support ours={w_ours} theirs={w_theirs}");
    assert_eq!(ours.len(), 3, "{cell}: A's supporters are all 3 producers");
    assert_eq!(theirs.len(), 3, "{cell}: the branch has 3 distinct leaders");
    assert_eq!(w_ours, 3, "{cell}: A's support in bond");
    assert_eq!(w_ours, w_theirs, "{cell}: the two sides must tie");
    assert_eq!(
        t.node.sync_manager.read().await.last_finalized_height(),
        None,
        "{cell}: A at depth 0 must not finalize (INC-I-190 F2)"
    );

    deliver_branch(&mut t).await;

    assert!(
        t.node
            .fork_block_cache
            .read()
            .await
            .contains_key(&d2.hash())
            && t.node.wedge_retained_tips.contains(&d2.hash()),
        "{cell}: D2 never reached the wedge door (dropped before weighing)"
    );
    let d2_hex = &d2.hash().to_hex()[..8];
    let verdict = logs.mentioning("not strictly heavier");
    eprintln!("[{cell}] door lines: {verdict:#?}");
    assert!(
        verdict.iter().any(|l| l.contains(d2_hex)),
        "{cell}: the wedge door did not weigh D2 as a non-gain"
    );
    assert!(
        logs.mentioning("Sibling branch heavier").is_empty(),
        "{cell}: the wedge door saw a strict gain on a tie"
    );
    assert_tip(&t.node, &a, F + 1, &format!("{cell}: the tie keeps A")).await;
    assert_eq!(
        t.node.sync_manager.read().await.last_finalized_height(),
        None,
        "{cell}: no finality lock decided the tie"
    );

    let slot0 = d2.header.slot + 1;
    for i in 0..N_TICKS {
        t.node
            .run_periodic_tasks()
            .await
            .expect("run_periodic_tasks Ok");
        let now = tip(&t.node).await;
        assert_eq!(now, a.hash(), "{cell}: tick {i}: the tip left A ({now})");
        // The genesis-phase active-status refresh (`startup.rs`) raises min peers
        // to 1 on the first tick; F2's shape has no peer (a peer would open the
        // walk door), so the rig keeps `new_for_test`'s 0. Peer count is not a
        // fork-choice input; every other production check still runs.
        let mut sm = t.node.sync_manager.write().await;
        sm.set_min_peers_for_production(0);
        let auth = sm.can_produce(slot0 + i as u32);
        drop(sm);
        assert_eq!(
            auth,
            ProductionAuthorization::Authorized,
            "{cell}: tick {i}: production blocked on the chosen tip"
        );
    }

    let mut own = build_devnet_block(&mut t.node).await;
    assert_eq!(
        own.header.prev_hash,
        a.hash(),
        "{cell}: the node built its next block off the chosen tip"
    );
    sign_into_pi(&mut own, &t.keys[0]);
    t.node
        .apply_block(own.clone(), ValidationMode::Light)
        .await
        .expect("own block applies on A");
    t.node
        .run_periodic_tasks()
        .await
        .expect("run_periodic_tasks Ok");
    assert_tip(
        &t.node,
        &own,
        F + 2,
        &format!("{cell}: the tip advances on A"),
    )
    .await;
}

// N2A INC-I-289 — Decision: a failure means above the INC-I-288 gate a tied fork of the INC-I-190 F2 shape (equal bond support) moves the node off the tip the tie rule keeps, flaps, or stops its production, so the bond rule re-opens the tied-fork wedge.
#[tokio::test]
async fn n2a_tied_fork_above_gate_resolves_and_keeps_producing() {
    if !in_isolated_child() {
        return;
    }
    tie_cell(false).await;
    tie_cell(true).await;

    // N2A-CTRL: the same branch against A with one other attester (2 vs 3) wins,
    // so the tie cells above are decided by support, not by a dropped branch.
    let mut t = tie_shape(2, false).await;
    let ours = support(&t.node, &[&t.a]).await;
    let theirs = support(&t.node, &[&t.branch[0], &t.branch[1], &t.branch[2]]).await;
    let (w_ours, w_theirs): (u64, u64) = (ours.values().sum(), theirs.values().sum());
    eprintln!("[N2A-CTRL] support ours={w_ours} theirs={w_theirs}");
    assert_eq!((w_ours, w_theirs), (2, 3), "N2A-CTRL: fixture support");
    deliver_branch(&mut t).await;
    let d2 = t.branch[2].clone();
    assert_tip(&t.node, &d2, F + 3, "N2A-CTRL: 3 vs 2 reorgs onto D2").await;
}
