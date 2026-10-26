//! INC-I-236 M2 node intake: gossip pre-filter (P2), pi-variant dedup (finding 76), snap anchor (P12).
//!
// covers: bins/node/src/node/network_events.rs bins/node/src/node/block_handling.rs bins/node/src/node/header_auth.rs
// covers: bins/node/src/node/fork_cache.rs bins/node/src/node/fork_recovery.rs bins/node/src/node/snapshot_install.rs
//
// OUTPUT CONTRACT: fn Node::on_new_block_event(&mut self, block, peer) -> Result<()>
//   O1 sync_manager.network_tip_slot()          O2 sync_manager last_block_seen (is_chain_stale)
//   O3 rejected_fork_tips membership           O4 chain_state (best_height, best_hash)
//   O5 return value — not asserted (every drop arm returns Ok)
// PATHS: P1 ExtendsTip  P2 parent is a rejected fork tip  P3 Orphan cached, then drained
// INPUT PARTITIONS (h >= AH unless stated): forged wrong-key pi | unsigned (empty pi) | valid | unsigned below AH
//   P1 forged/unsigned: O1+O2+O4 unchanged   P1 valid: O1 raised, O4 advances   P1 below AH: as valid
//   P2 forged: O3 lacks the forged hash      P3 genuine then pi-variant: O4 reaches the genuine block
//   P1 pi-variant then genuine: O4 reaches the genuine block
//
// OUTPUT CONTRACT: fn Node::apply_snap_snapshot(&mut self, VerifiedSnapshot) -> Result<()>
//   O6 block_store header at the anchor hash   O7 block_store canonical hash at the snapshot height
//   O8 chain_state (best_height, best_hash), memory and state_db   O9 snap_sync_height
//   O10 return value — not asserted (refusal may be Ok via fallback or Err)
// INPUT PARTITIONS: anchor forged | anchor unsigned | anchor hash != quorum hash | valid | unsigned below AH
//   refused partitions: O6+O7+O8+O9 unchanged   installed partitions: O6 = anchor, O8 = snapshot tip, O9 set

#[path = "support/block_sig.rs"]
mod block_sig;

use std::time::Duration;

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Network, Transaction};
use doli_node::node::Node;
use network::PeerId;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

const AH_ENV: &str = "DOLI_INC_I_236_BLOCK_SIGNATURE_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "INC_I_236_M2_INTAKE_NODE_CHILD";
const ABOVE_TIP: u64 = 1_000_000_000;
const STALE: Duration = Duration::from_millis(200);

#[derive(Clone, Copy, Debug)]
enum Sig {
    Valid,
    Forged,
    Unsigned,
}

/// Re-exec `name` in a child with the devnet AH set to `ah`; true in the child.
fn in_child(name: &str, ah: u64) -> bool {
    if std::env::var_os(CHILD_ENV).is_some() {
        return true;
    }
    let out = std::process::Command::new(std::env::current_exe().expect("current exe"))
        .args([name, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1")
        .env(AH_ENV, ah.to_string())
        .output()
        .expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "child {name} failed or did not run:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    false
}

fn devnet_ah() -> u64 {
    Network::Devnet
        .params()
        .inc_i_236_block_signature_activation_height
}

async fn make_node(producers: &[KeyPair]) -> (Node, TempDir) {
    let temp = TempDir::new().unwrap();
    let node = Node::new_for_test(temp.path().to_path_buf(), producers.to_vec())
        .await
        .expect("Node::new_for_test failed");
    (node, temp)
}

fn keys(n: usize) -> Vec<KeyPair> {
    (0..n).map(|_| KeyPair::generate()).collect()
}

fn sign(header: &mut BlockHeader, producer: &KeyPair, sig: Sig) {
    match sig {
        Sig::Valid => block_sig::sign_header(header, producer),
        Sig::Forged => block_sig::sign_header(header, &KeyPair::generate()),
        Sig::Unsigned => header.vdf_proof.pi.clear(),
    }
}

fn block(
    height: u64,
    slot: u32,
    prev: Hash,
    producer: &KeyPair,
    sig: Sig,
    params: &ConsensusParams,
) -> Block {
    let pool = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(params.block_reward(height), pool, height, 0);
    let mut header = BlockHeader {
        version: 2,
        prev_hash: prev,
        merkle_root: doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase)),
        presence_root: Hash::ZERO,
        genesis_hash: doli_core::chainspec::ChainSpec::devnet().genesis_hash(),
        timestamp: params.genesis_time + slot as u64 * params.slot_duration,
        slot,
        producer: *producer.public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    sign(&mut header, producer, sig);
    Block::new(header, vec![coinbase])
}

/// Same header and body, `pi` replaced: `header.hash()` is unchanged.
fn pi_variant(b: &Block, producer: &KeyPair, sig: Sig) -> Block {
    let mut v = b.clone();
    sign(&mut v.header, producer, sig);
    assert_eq!(v.hash(), b.hash(), "fixture: pi is outside header.hash()");
    v
}

/// Wall-clock slot as `on_new_block_event` computes it (passes SLOT_SANITY).
fn wall_slot(params: &ConsensusParams) -> u32 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    params.timestamp_to_slot(now)
}

async fn tip(node: &Node) -> (u64, Hash) {
    let cs = node.chain_state.read().await;
    (cs.best_height, cs.best_hash)
}

async fn tip_slot(node: &Node) -> u32 {
    node.sync_manager.read().await.network_tip_slot()
}

async fn chain_stale(node: &Node) -> bool {
    node.sync_manager.read().await.is_chain_stale(STALE)
}

/// Snapshot of `src` at its tip; `anchor` becomes `block_header_bytes`.
async fn snapshot_of(src: &Node, anchor: Option<&BlockHeader>) -> network::VerifiedSnapshot {
    let cs = src.chain_state.read().await;
    let ps = src.producer_set.read().await;
    let cs_bytes = bincode::serialize(&*cs).unwrap();
    let ps_bytes = bincode::serialize(&*ps).unwrap();
    let utxo_bytes = src.state_db.serialize_canonical_utxo();
    let state_root = storage::compute_state_root_from_bytes(&cs_bytes, &utxo_bytes, &ps_bytes)
        .expect("fixture: state root");
    network::VerifiedSnapshot {
        block_hash: cs.best_hash,
        block_height: cs.best_height,
        chain_state: cs_bytes,
        utxo_set: utxo_bytes,
        utxo_staged: None,
        producer_set: ps_bytes,
        state_root,
        block_header_bytes: anchor.map(|h| bincode::serialize(h).unwrap()),
        epoch_state_bytes: None,
        epoch_bond_snapshot_bytes: None,
        epoch_accumulators_bytes: None,
    }
}

/// Source node at h=2 built from valid blocks; returns (source, b1, b2).
async fn snap_source(producers: &[KeyPair]) -> (Node, TempDir, Block, Block) {
    let (mut src, t) = make_node(producers).await;
    let params = src.params.clone();
    let genesis = tip(&src).await.1;
    let b1 = block(1, 1, genesis, &producers[0], Sig::Valid, &params);
    let b2 = block(2, 2, b1.hash(), &producers[0], Sig::Valid, &params);
    for b in [&b1, &b2] {
        src.apply_block(b.clone(), ValidationMode::Light)
            .await
            .expect("fixture: source chain");
    }
    (src, t, b1, b2)
}

async fn assert_install_refused(
    node: &Node,
    before: (u64, Hash),
    anchor_hash: Hash,
    h: u64,
    case: &str,
) {
    assert!(
        node.block_store.get_header(&anchor_hash).unwrap().is_none(),
        "{case}: anchor header written to the block store"
    );
    assert!(
        node.block_store.get_hash_by_height(h).unwrap().is_none(),
        "{case}: canonical index seeded at the snapshot height"
    );
    assert_eq!(
        tip(node).await,
        before,
        "{case}: in-memory chain_state swapped"
    );
    let durable = node
        .state_db
        .get_chain_state()
        .map(|c| (c.best_height, c.best_hash));
    assert_ne!(
        durable.map(|d| d.0),
        Some(h),
        "{case}: state_db chain_state swapped"
    );
    assert!(
        node.snap_sync_height.is_none(),
        "{case}: snap_sync_height set"
    );
}

async fn assert_installed(node: &Node, anchor: &BlockHeader, case: &str) {
    assert_eq!(
        tip(node).await,
        (2, anchor.hash()),
        "{case}: snapshot not installed"
    );
    let stored = node.block_store.get_header(&anchor.hash()).unwrap();
    assert_eq!(
        stored.map(|h| h.vdf_proof.pi),
        Some(anchor.vdf_proof.pi.clone()),
        "{case}: anchor header not persisted"
    );
    assert_eq!(node.snap_sync_height, Some(2), "{case}: snap_sync_height");
}

// REQ-AUTH-021 — Decision: a failure means one forged gossip block moves the production "behind peers" gate and the solo watchdog.
#[tokio::test]
async fn req_auth_021_p2_on_new_block_event_forged_no_tip_slot() {
    assert_eq!(devnet_ah(), 0, "PRECONDITION: devnet AH = 0");
    for sig in [Sig::Forged, Sig::Unsigned] {
        let p = keys(2);
        let (mut node, _t) = make_node(&p).await;
        let params = node.params.clone();
        let before = tip(&node).await;
        let slot = wall_slot(&params);
        let b = block(1, slot, before.1, &p[0], sig, &params);
        let slot0 = tip_slot(&node).await;
        assert!(
            slot0 < slot,
            "PRECONDITION: the block slot would raise the tip slot"
        );
        assert!(
            !node.sync_manager.read().await.is_snap_syncing(),
            "PRECONDITION"
        );
        tokio::time::sleep(STALE + Duration::from_millis(100)).await;
        assert!(chain_stale(&node).await, "PRECONDITION: no block seen yet");

        let _ = node.on_new_block_event(b, PeerId::random()).await;

        assert_eq!(
            tip_slot(&node).await,
            slot0,
            "{sig:?}: update_network_tip_slot fed by an unauthenticated block"
        );
        assert!(
            chain_stale(&node).await,
            "{sig:?}: note_block_received_from_peer/via_gossip fed by an unauthenticated block"
        );
        assert_eq!(tip(&node).await, before, "{sig:?}: block applied");
    }
}

// REQ-AUTH-021 — Decision: a failure means forged children of a rejected tip can flood and clear the INC-I-014 dead-fork filter.
#[tokio::test]
async fn req_auth_021_p2_on_new_block_event_forged_child_of_rejected_tip_not_recorded() {
    let p = keys(2);
    let (mut node, _t) = make_node(&p).await;
    let params = node.params.clone();
    let genesis = tip(&node).await.1;
    let slot = wall_slot(&params);
    let dead = block(1, slot - 2, genesis, &p[1], Sig::Valid, &params);
    node.rejected_fork_tips.insert(dead.hash());

    let forged = block(2, slot - 1, dead.hash(), &p[0], Sig::Forged, &params);
    let _ = node
        .on_new_block_event(forged.clone(), PeerId::random())
        .await;
    assert!(
        !node.rejected_fork_tips.contains(&forged.hash()),
        "forged child recorded in rejected_fork_tips"
    );

    let valid = block(2, slot, dead.hash(), &p[0], Sig::Valid, &params);
    let _ = node
        .on_new_block_event(valid.clone(), PeerId::random())
        .await;
    assert!(
        node.rejected_fork_tips.contains(&valid.hash()),
        "NON-VACUITY: a signed child of a rejected tip is still recorded (INC-I-014)"
    );
}

// REQ-AUTH-021 — Decision: a failure means the pre-filter fix dropped honest gossip blocks (regression lock).
#[tokio::test]
async fn req_auth_021_p2_on_new_block_event_valid_applied() {
    let p = keys(2);
    let (mut node, _t) = make_node(&p).await;
    let params = node.params.clone();
    let genesis = tip(&node).await.1;
    let slot = wall_slot(&params);
    let b = block(1, slot, genesis, &p[0], Sig::Valid, &params);
    tokio::time::sleep(STALE + Duration::from_millis(100)).await;

    node.on_new_block_event(b.clone(), PeerId::random())
        .await
        .expect("on_new_block_event");

    assert_eq!(tip(&node).await, (1, b.hash()), "valid block not applied");
    assert_eq!(
        tip_slot(&node).await,
        slot,
        "valid block must feed the tip slot"
    );
    assert!(
        !chain_stale(&node).await,
        "valid block must refresh last_block_seen"
    );
}

// REQ-AUTH-021 — Decision: a failure means the pre-filter fix rejects today's unsigned blocks before the height (rolling deploy breaks).
#[tokio::test]
async fn req_auth_021_p2_on_new_block_event_unsigned_below_ah_as_today() {
    if !in_child(
        "req_auth_021_p2_on_new_block_event_unsigned_below_ah_as_today",
        ABOVE_TIP,
    ) {
        return;
    }
    assert_eq!(
        devnet_ah(),
        ABOVE_TIP,
        "PRECONDITION: child reads the AH override"
    );
    let p = keys(2);
    let (mut node, _t) = make_node(&p).await;
    let params = node.params.clone();
    let genesis = tip(&node).await.1;
    let slot = wall_slot(&params);
    let b = block(1, slot, genesis, &p[0], Sig::Unsigned, &params);

    node.on_new_block_event(b.clone(), PeerId::random())
        .await
        .expect("on_new_block_event");

    assert_eq!(
        tip(&node).await,
        (1, b.hash()),
        "unsigned block below the AH not applied"
    );
    assert_eq!(
        tip_slot(&node).await,
        slot,
        "tip slot below the AH as today"
    );
}

// REQ-AUTH-021 — Decision: a failure means a forged pi-variant delivered first blocks the genuine block by its pi-blind hash.
#[tokio::test]
async fn req_auth_021_f76_node_forged_pi_variant_first_genuine_applied() {
    let p = keys(2);
    let (mut node, _t) = make_node(&p).await;
    let params = node.params.clone();
    let genesis = tip(&node).await.1;
    let genuine = block(1, wall_slot(&params), genesis, &p[0], Sig::Valid, &params);
    let forged = pi_variant(&genuine, &p[0], Sig::Forged);
    let peer = PeerId::random();

    let _ = node.on_new_block_event(forged, peer).await;
    assert_eq!(tip(&node).await, (0, genesis), "forged pi-variant applied");
    node.on_new_block_event(genuine.clone(), peer)
        .await
        .expect("on_new_block_event");

    assert_eq!(
        tip(&node).await,
        (1, genuine.hash()),
        "genuine block refused after its pi-variant"
    );
    let stored = node.block_store.get_header(&genuine.hash()).unwrap();
    assert_eq!(
        stored.map(|h| h.vdf_proof.pi),
        Some(genuine.header.vdf_proof.pi.clone()),
        "stored header must carry the genuine signature"
    );
}

/// Feeds b2 (orphan) then optionally its forged pi-variant, then b1; returns the tip.
async fn orphan_then_parent(with_forged_variant: bool) -> ((u64, Hash), Block) {
    let p = keys(2);
    let (mut node, _t) = make_node(&p).await;
    let params = node.params.clone();
    let genesis = tip(&node).await.1;
    let slot = wall_slot(&params);
    let b1 = block(1, slot - 2, genesis, &p[0], Sig::Valid, &params);
    let b2 = block(2, slot - 1, b1.hash(), &p[0], Sig::Valid, &params);
    let peer = PeerId::random();

    let _ = node.on_new_block_event(b2.clone(), peer).await;
    assert_eq!(
        tip(&node).await,
        (0, genesis),
        "PRECONDITION: b2 is an orphan"
    );
    if with_forged_variant {
        let forged = pi_variant(&b2, &p[0], Sig::Forged);
        let _ = node.on_new_block_event(forged, peer).await;
    }
    node.on_new_block_event(b1, peer)
        .await
        .expect("on_new_block_event");
    (tip(&node).await, b2)
}

// REQ-AUTH-021 — Decision: a failure means a forged pi-variant overwrites a cached genuine orphan, so the genuine block is lost.
#[tokio::test]
async fn req_auth_021_f76_node_forged_pi_variant_after_genuine_orphan_keeps_genuine() {
    let (control, b2) = orphan_then_parent(false).await;
    assert_eq!(
        control,
        (2, b2.hash()),
        "NON-VACUITY: cached genuine orphan drains"
    );

    let (actual, b2) = orphan_then_parent(true).await;
    assert_eq!(
        actual,
        (2, b2.hash()),
        "forged pi-variant replaced the cached genuine orphan (pi-blind fork-cache key)"
    );
}

// REQ-AUTH-017 — Decision: a failure means a snapshot peer can plant a forged or unsigned anchor header as canonical history.
#[tokio::test]
async fn req_auth_017_p12_snap_anchor_forged_refused() {
    assert_eq!(devnet_ah(), 0, "PRECONDITION: devnet AH = 0");
    for sig in [Sig::Forged, Sig::Unsigned] {
        let p = keys(2);
        let (src, _ts, _b1, b2) = snap_source(&p).await;
        let anchor = pi_variant(&b2, &p[0], sig).header;
        let snap = snapshot_of(&src, Some(&anchor)).await;
        assert_eq!(
            snap.block_hash,
            anchor.hash(),
            "PRECONDITION: anchor hash = quorum hash"
        );
        let (mut dst, _td) = make_node(&p).await;
        let before = tip(&dst).await;

        let _ = dst.apply_snap_snapshot(snap).await;

        assert_install_refused(&dst, before, b2.hash(), 2, &format!("{sig:?} anchor")).await;
    }
}

// REQ-AUTH-017 — Decision: a failure means the anchor header is not bound to the quorum block hash, so any header can be planted.
#[tokio::test]
async fn req_auth_017_p12_snap_anchor_hash_mismatch_refused() {
    let p = keys(2);
    let (src, _ts, b1, b2) = snap_source(&p).await;
    let snap = snapshot_of(&src, Some(&b1.header)).await;
    assert_eq!(
        snap.block_hash,
        b2.hash(),
        "PRECONDITION: quorum hash is b2"
    );
    let (mut dst, _td) = make_node(&p).await;
    let before = tip(&dst).await;

    let _ = dst.apply_snap_snapshot(snap).await;

    assert_install_refused(&dst, before, b1.hash(), 2, "mismatched anchor").await;
}

// REQ-AUTH-017 — Decision: a failure means the anchor check refuses honest snapshots (regression lock).
#[tokio::test]
async fn req_auth_017_p12_snap_anchor_valid_installed() {
    let p = keys(2);
    let (src, _ts, _b1, b2) = snap_source(&p).await;
    let snap = snapshot_of(&src, Some(&b2.header)).await;
    let (mut dst, _td) = make_node(&p).await;

    dst.apply_snap_snapshot(snap)
        .await
        .expect("apply_snap_snapshot");

    assert_installed(&dst, &b2.header, "valid anchor").await;
}

// REQ-AUTH-017 — Decision: a failure means snap sync below the height refuses today's unsigned anchors (rolling deploy breaks).
#[tokio::test]
async fn req_auth_017_p12_snap_anchor_unsigned_below_ah_installed() {
    if !in_child(
        "req_auth_017_p12_snap_anchor_unsigned_below_ah_installed",
        ABOVE_TIP,
    ) {
        return;
    }
    assert_eq!(
        devnet_ah(),
        ABOVE_TIP,
        "PRECONDITION: child reads the AH override"
    );
    let p = keys(2);
    let (src, _ts, _b1, b2) = snap_source(&p).await;
    let anchor = pi_variant(&b2, &p[0], Sig::Unsigned).header;
    let snap = snapshot_of(&src, Some(&anchor)).await;
    let (mut dst, _td) = make_node(&p).await;

    dst.apply_snap_snapshot(snap)
        .await
        .expect("apply_snap_snapshot");

    assert_installed(&dst, &anchor, "unsigned anchor below the AH").await;
}
