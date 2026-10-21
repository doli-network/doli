// OUTPUT CONTRACT: fn Node::handle_new_block(&mut self, block, peer) -> Result<()>
//   O1 fork_block_cache entry for the block hash (cache_block_with_eviction)
//   O2 sync_manager.reorg_handler weight for the block hash (record_fork_block_weight)
//   O3 chain_state best_hash / best_height (execute_reorg)
//   O4 sync_manager fork recovery active (start_fork_recovery)
//   O5 sync_manager network_tip (note_orphan_gossip_block)
//   O6 wedge_retained_tips entry (retain_sibling_and_try_escape)
//   O7 return value — not asserted: every arm returns Ok(())
// PATHS: A1 ReorgCandidate arm; A2 Orphan arm; A3 HeightOccupied{is_better:false} arm.
// INPUT PARTITIONS (above the signature AH unless noted):
//   A1a forged wrong-key pi    -> O1 O2 O4 untouched, O3 unchanged
//   A1b unsigned (empty pi)    -> O1 O2 O4 untouched, O3 unchanged
//   A1c producer-signed (lock) -> O1 set, O4 set
//   A2a forged wrong-key orphan             -> O1 untouched, O5 unchanged
//   A2b unsigned orphan, AH above (lock)    -> O1 set, O5 moved
//   A3a forged wrong-key worse sibling      -> O1 O2 O6 untouched, O3 unchanged
// UNREACHABLE: execute_reorg from A1 with a consistent store (the reorg door needs
//   prev_hash on the tip's ancestry, which classifies HeightOccupied) — O3 is an invariant.
// covers: block_handling
// covers: header_auth
// covers: fork_cache
// covers: network_events
#![allow(clippy::unwrap_used)]

#[path = "support/block_sig.rs"]
mod block_sig;
#[allow(dead_code)]
#[path = "it/inc_i_236_m4_common.rs"]
mod inc_i_236_m4_common;

use crypto::{Hash, KeyPair};
use doli_core::Block;
use doli_node::node::Node;
use tempfile::TempDir;

use inc_i_236_m4_common::{canonical_two, feed, header_block, reporter_node};

const AH_ENV: &str = "DOLI_INC_I_236_BLOCK_SIGNATURE_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "INC_I_236_M2_P3_ARMS_CHILD";
const BELOW_AH: u64 = 1_000_000_000;
const ORPHAN_SLOT: u32 = 40;

#[derive(Clone, Copy)]
enum Sig {
    Producer,
    WrongKey,
    Unsigned,
}

fn pick<'a>(sig: Sig, producer: &'a KeyPair, attacker: &'a KeyPair) -> Option<&'a KeyPair> {
    match sig {
        Sig::Producer => Some(producer),
        Sig::WrongKey => Some(attacker),
        Sig::Unsigned => None,
    }
}

/// Tip at h2 plus a stored-but-unapplied h3; a child of h3 classifies ReorgCandidate.
async fn reorg_candidate(sig: Sig) -> (Node, Block, TempDir) {
    let (mut node, _reporter, victim, tmp) = reporter_node().await;
    let attacker = KeyPair::generate();
    let (_b1, b2) = canonical_two(&mut node, &victim, 10).await;
    let pk = *victim.public_key();
    let stored = header_block(&mut node, 3, 12, b2.hash(), &pk, Some(&victim), 0).await;
    node.block_store.put_block_canonical(&stored, 3).unwrap();
    let signer = pick(sig, &victim, &attacker);
    let block = header_block(&mut node, 4, 14, stored.hash(), &pk, signer, 0).await;
    assert_eq!(
        node.block_store.get_height_by_hash(&stored.hash()).unwrap(),
        Some(3),
        "fixture: parent indexed"
    );
    assert!(
        node.block_store.get_block_by_height(4).unwrap().is_none(),
        "fixture: fork height unoccupied"
    );
    assert_ne!(
        node.chain_state.read().await.best_hash,
        stored.hash(),
        "fixture: parent is not the tip"
    );
    assert!(
        !node.sync_manager.read().await.is_fork_recovery_active(),
        "fixture: no recovery before feed"
    );
    (node, block, tmp)
}

async fn tip(node: &Node) -> (Hash, u64) {
    let s = node.chain_state.read().await;
    (s.best_hash, s.best_height)
}

async fn cached(node: &Node, hash: &Hash) -> bool {
    node.fork_block_cache.read().await.contains_key(hash)
}

async fn fork_weight_recorded(node: &Node, hash: &Hash) -> bool {
    node.sync_manager
        .read()
        .await
        .reorg_handler()
        .get_block_weight(hash)
        .is_some()
}

async fn assert_reorg_arm_inert(node: &Node, hash: &Hash, before: (Hash, u64)) {
    assert!(
        !cached(node, hash).await,
        "unauthenticated ReorgCandidate must not enter the fork cache"
    );
    assert!(
        !fork_weight_recorded(node, hash).await,
        "unauthenticated ReorgCandidate must not record fork weight"
    );
    assert_eq!(tip(node).await, before, "tip must not move (no reorg)");
    assert!(
        !node.sync_manager.read().await.is_fork_recovery_active(),
        "unauthenticated ReorgCandidate must not start fork recovery"
    );
}

async fn orphan(node: &mut Node, victim: &KeyPair, signer: Option<&KeyPair>) -> Block {
    let unknown = crypto::hash::hash(b"inc-i-236 p3 unknown parent");
    let pk = *victim.public_key();
    header_block(node, 5, ORPHAN_SLOT, unknown, &pk, signer, 0).await
}

/// A worse-slot sibling of the h2 tip whose named producer passes the eligibility gate.
async fn eligible_worse_sibling(
    node: &mut Node,
    keys: [&KeyPair; 2],
    parent: Hash,
    signer: &KeyPair,
) -> Block {
    for slot in 11..60u32 {
        for kp in keys {
            let b = header_block(node, 2, slot, parent, kp.public_key(), Some(signer), 1).await;
            if node.check_producer_eligibility(&b).await.is_ok() {
                return b;
            }
        }
    }
    panic!("fixture: no eligible producer for a worse sibling in slots 11..60");
}

// REQ-AUTH-021 — Decision: a failure means a forged reorg candidate still drives fork caching and fork recovery.
// REQ-AUTH-023 — Decision: a failure means the P3 ReorgCandidate arm is an intake path the fix left open.
#[tokio::test]
async fn req_auth_021_p3_forged_reorg_candidate_no_side_effect() {
    let (mut node, forged, _tmp) = reorg_candidate(Sig::WrongKey).await;
    assert!(
        !forged.header.vdf_proof.pi.is_empty(),
        "fixture: forged carries a signature"
    );
    let before = tip(&node).await;
    let hash = forged.hash();

    feed(&mut node, forged).await;

    assert_reorg_arm_inert(&node, &hash, before).await;
}

// REQ-AUTH-021 — Decision: a failure means a block with no signature at all still reaches the ReorgCandidate arm.
#[tokio::test]
async fn req_auth_021_p3_unsigned_reorg_candidate_no_side_effect() {
    let (mut node, unsigned, _tmp) = reorg_candidate(Sig::Unsigned).await;
    assert!(unsigned.header.vdf_proof.pi.is_empty(), "fixture: unsigned");
    let before = tip(&node).await;
    let hash = unsigned.hash();

    feed(&mut node, unsigned).await;

    assert_reorg_arm_inert(&node, &hash, before).await;
}

// REQ-AUTH-021 — Decision: a failure means the fix dropped authentic fork blocks too (regression lock).
#[tokio::test]
async fn req_auth_021_p3_signed_reorg_candidate_still_cached() {
    let (mut node, signed, _tmp) = reorg_candidate(Sig::Producer).await;
    let hash = signed.hash();

    feed(&mut node, signed).await;

    assert!(
        cached(&node, &hash).await,
        "a producer-signed ReorgCandidate must still be cached"
    );
    assert!(
        node.sync_manager.read().await.is_fork_recovery_active(),
        "a producer-signed ReorgCandidate must still start fork recovery"
    );
}

// REQ-AUTH-021 — Decision: a failure means a forged orphan still fills the fork cache and moves the sync tip view.
// REQ-AUTH-023 — Decision: a failure means the P3/P18 Orphan arm is an intake path the fix left open.
#[tokio::test]
async fn req_auth_021_p3_forged_orphan_no_cache_no_chase() {
    let (mut node, _reporter, victim, _tmp) = reporter_node().await;
    let attacker = KeyPair::generate();
    let forged = orphan(&mut node, &victim, Some(&attacker)).await;
    let hash = forged.hash();
    let net_before = node.sync_manager.read().await.network_tip();
    let before = tip(&node).await;

    feed(&mut node, forged).await;

    assert!(
        !cached(&node, &hash).await,
        "a forged orphan must not enter the fork cache"
    );
    assert_eq!(
        node.sync_manager.read().await.network_tip(),
        net_before,
        "a forged orphan must not reach note_orphan_gossip_block"
    );
    assert_eq!(tip(&node).await, before, "tip must not move");
}

// REQ-AUTH-021 — Decision: a failure means a forged worse sibling is retained, cached and weighted for wedge escape.
// REQ-AUTH-023 — Decision: a failure means the P3 HeightOccupied worse arm is an intake path the fix left open.
#[tokio::test]
async fn req_auth_021_p3_forged_worse_sibling_no_wedge_retain() {
    let (mut node, reporter, victim, _tmp) = reporter_node().await;
    let attacker = KeyPair::generate();
    let (b1, b2) = canonical_two(&mut node, &victim, 10).await;
    let forged =
        eligible_worse_sibling(&mut node, [&victim, &reporter], b1.hash(), &attacker).await;
    let hash = forged.hash();

    feed(&mut node, forged).await;

    assert!(
        !node.wedge_retained_tips.contains(&hash),
        "a forged worse sibling must not be retained for wedge escape"
    );
    assert!(
        !cached(&node, &hash).await,
        "a forged worse sibling must not enter the fork cache"
    );
    assert!(
        !fork_weight_recorded(&node, &hash).await,
        "a forged worse sibling must not record fork weight"
    );
    assert_eq!(
        node.chain_state.read().await.best_hash,
        b2.hash(),
        "tip must not move"
    );
}

// REQ-AUTH-021 — Decision: a failure means the fix changed pre-activation orphan handling without an AH gate.
#[tokio::test]
async fn req_auth_021_p3_below_ah_unsigned_orphan_cached_as_before() {
    if std::env::var_os(CHILD_ENV).is_none() {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "req_auth_021_p3_below_ah_unsigned_orphan_cached_as_before",
                "--exact",
                "--test-threads=1",
                "--nocapture",
            ])
            .env(CHILD_ENV, "1")
            .env(AH_ENV, BELOW_AH.to_string())
            .output()
            .expect("spawn child test process");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "below-AH child failed or did not run:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        return;
    }

    assert_eq!(
        doli_core::NetworkParams::load(doli_core::Network::Devnet)
            .inc_i_236_block_signature_activation_height,
        BELOW_AH,
        "child must pick up the AH env override"
    );
    let (mut node, _reporter, victim, _tmp) = reporter_node().await;
    let unsigned = orphan(&mut node, &victim, None).await;
    let hash = unsigned.hash();
    let net_before = node.sync_manager.read().await.network_tip();

    feed(&mut node, unsigned).await;

    assert!(
        cached(&node, &hash).await,
        "below AH: unsigned orphan cached as before"
    );
    let net_after = node.sync_manager.read().await.network_tip();
    assert_ne!(
        net_after, net_before,
        "below AH: network tip noted as before"
    );
    assert_eq!(net_after.1, ORPHAN_SLOT, "below AH: orphan slot noted");
}
