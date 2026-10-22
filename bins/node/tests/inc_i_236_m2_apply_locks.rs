//! INC-I-236 M2 apply-entry locks per caller mode, post-snap Light arm, detector input lock.
//!
// covers: bins/node/src/node/block_handling.rs bins/node/src/node/header_auth.rs bins/node/src/node/fork_recovery.rs
// covers: bins/node/src/node/apply_block/mod.rs bins/node/src/node/periodic.rs bins/node/src/node/snapshot_install.rs
//
// OUTPUT CONTRACT: apply entries — Node::handle_new_block (Full; Light via snap_sync_height or
//   floor_fallback_window), Node::apply_block(Light) (periodic in-order sync call), Node::new_for_replay +
//   apply_block(Replay), Node::try_apply_direct_successor (INC-I-081, Light)
//   O1 chain_state (best_height, best_hash)   O2 block_store canonical hash at the height
//   O3 return value where the entry surfaces it (apply_block: Err InvalidBlockSignature; direct successor: not Ok(true))
// INPUT PARTITIONS (h >= AH, devnet AH = 0): forged wrong-key pi | unsigned (empty pi) | valid
//   forged/unsigned: O1+O2 unchanged, O3 rejection   valid: O1 advances, O2 = block hash
//
// OUTPUT CONTRACT: EquivocationDetector reached from Node::handle_new_block
//   O4 tracked_count (seen_blocks)   O5 pending proofs naming the producer   O6 WARN+ events about equivocation
// INPUT PARTITIONS: forged first block | forged far-future slot after a real block | forged conflicting
//   header after a real block | valid conflicting header (non-vacuity)

#[path = "support/block_sig.rs"]
mod block_sig;

use std::sync::{Arc, Mutex};

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::{ValidationError, ValidationMode};
use doli_core::{Block, BlockHeader, Network, Transaction};
use doli_node::node::Node;
use network::PeerId;
use tempfile::TempDir;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;
use vdf::{VdfOutput, VdfProof};

#[derive(Clone, Copy, Debug)]
enum Sig {
    Valid,
    Forged,
    Unsigned,
}

const BAD: [Sig; 2] = [Sig::Forged, Sig::Unsigned];

fn keys(n: usize) -> Vec<KeyPair> {
    (0..n).map(|_| KeyPair::generate()).collect()
}

// Keep the equivocation-WARN callsite "interested" for the whole binary. Tests run
// in parallel: a sibling node that emits at that callsite with no subscriber active
// would cache its interest as `never`, and the thread-local capture subscriber in
// req_auth_022_detector_forged_conflict_emits_no_equivocation_warn would then never
// see the WARN (its own NON-VACUITY guard would fail). Installing one always-
// interested global default before any node is built keeps every callsite hot; the
// capture test still overrides it per-thread with set_default to read its own buffer.
// The global sink discards into a throwaway buffer, so it causes no cross-test talk.
fn ensure_callsites_hot() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let sink = DetectorWarns::default();
        let _ = tracing::subscriber::set_global_default(tracing_subscriber::registry().with(sink));
    });
}

async fn make_node(producers: &[KeyPair]) -> (Node, TempDir) {
    ensure_callsites_hot();
    let temp = TempDir::new().unwrap();
    let node = Node::new_for_test(temp.path().to_path_buf(), producers.to_vec())
        .await
        .expect("Node::new_for_test failed");
    (node, temp)
}

fn sign(header: &mut BlockHeader, producer: &KeyPair, sig: Sig) {
    match sig {
        Sig::Valid => block_sig::sign_header(header, producer),
        Sig::Forged => block_sig::sign_header(header, &KeyPair::generate()),
        Sig::Unsigned => header.vdf_proof.pi.clear(),
    }
}

/// `salt` varies the coinbase so two blocks for one (producer, slot) conflict.
#[allow(clippy::too_many_arguments)]
fn salted(
    height: u64,
    slot: u32,
    prev: Hash,
    producer: &KeyPair,
    sig: Sig,
    params: &ConsensusParams,
    genesis_hash: Hash,
    salt: u64,
) -> Block {
    let pool = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(params.block_reward(height) + salt, pool, height, 0);
    let mut header = BlockHeader {
        version: 2,
        prev_hash: prev,
        merkle_root: doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase)),
        presence_root: Hash::ZERO,
        genesis_hash,
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

fn block(
    height: u64,
    slot: u32,
    prev: Hash,
    producer: &KeyPair,
    sig: Sig,
    params: &ConsensusParams,
) -> Block {
    let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();
    salted(height, slot, prev, producer, sig, params, genesis_hash, 0)
}

fn variant(b: &Block, producer: &KeyPair, sig: Sig) -> Block {
    let mut v = b.clone();
    sign(&mut v.header, producer, sig);
    v
}

async fn tip(node: &Node) -> (u64, Hash) {
    let cs = node.chain_state.read().await;
    (cs.best_height, cs.best_hash)
}

fn canonical_at(node: &Node, h: u64) -> Option<Hash> {
    node.block_store.get_hash_by_height(h).unwrap()
}

async fn feed(node: &mut Node, b: Block) {
    let _ = node.handle_new_block(b, PeerId::random()).await;
}

fn is_sig_rejection(e: &anyhow::Error) -> bool {
    matches!(
        e.downcast_ref::<ValidationError>(),
        Some(ValidationError::InvalidBlockSignature { .. })
    )
}

/// Feeds forged and unsigned variants of `b` through `handle_new_block`, then the valid one.
async fn assert_handle_new_block_gate(node: &mut Node, b: &Block, producer: &KeyPair, case: &str) {
    let before = tip(node).await;
    let h = before.0 + 1;
    for sig in BAD {
        feed(node, variant(b, producer, sig)).await;
        assert_eq!(tip(node).await, before, "{case} {sig:?}: block applied");
        assert_eq!(
            canonical_at(node, h),
            None,
            "{case} {sig:?}: block stored as canonical"
        );
    }
    feed(node, b.clone()).await;
    assert_eq!(
        tip(node).await,
        (h, b.hash()),
        "{case}: valid block refused"
    );
    assert_eq!(
        canonical_at(node, h),
        Some(b.hash()),
        "{case}: valid block not stored"
    );
}

/// Same, through `apply_block(mode)` with the signature rejection asserted.
async fn assert_apply_block_gate(
    node: &mut Node,
    b: &Block,
    producer: &KeyPair,
    mode: ValidationMode,
) {
    let before = tip(node).await;
    let h = before.0 + 1;
    for sig in BAD {
        let err = node
            .apply_block(variant(b, producer, sig), mode)
            .await
            .expect_err("forged/unsigned block must be rejected");
        assert!(
            is_sig_rejection(&err),
            "{mode:?} {sig:?}: not the signature rule: {err:#}"
        );
        assert_eq!(tip(node).await, before, "{mode:?} {sig:?}: block applied");
        assert_eq!(
            canonical_at(node, h),
            None,
            "{mode:?} {sig:?}: block stored"
        );
    }
    node.apply_block(b.clone(), mode)
        .await
        .unwrap_or_else(|e| panic!("{mode:?}: valid block refused: {e:#}"));
    assert_eq!(
        tip(node).await,
        (h, b.hash()),
        "{mode:?}: valid block not applied"
    );
}

async fn snapshot_of(src: &Node, anchor: &BlockHeader) -> network::VerifiedSnapshot {
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
        block_header_bytes: Some(bincode::serialize(anchor).unwrap()),
        epoch_state_bytes: None,
        epoch_bond_snapshot_bytes: None,
        epoch_accumulators_bytes: None,
    }
}

async fn proofs_against(node: &Node, producer: &PublicKey) -> usize {
    let proofs = node
        .equivocation_detector
        .write()
        .await
        .take_pending_proofs();
    proofs.iter().filter(|p| p.producer == *producer).count()
}

async fn tracked(node: &Node) -> usize {
    node.equivocation_detector.read().await.tracked_count()
}

/// Records WARN+ events whose target or message names equivocation.
#[derive(Clone, Default)]
struct DetectorWarns(Arc<Mutex<Vec<String>>>);

struct Msg(String);

impl tracing::field::Visit for Msg {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<S: tracing::Subscriber> Layer<S> for DetectorWarns {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        if *meta.level() > tracing::Level::WARN {
            return;
        }
        let mut msg = Msg(String::new());
        event.record(&mut msg);
        let target = meta.target();
        if target.contains("equivocation") || msg.0.to_lowercase().contains("equivocation") {
            self.0.lock().unwrap().push(format!("{target}: {}", msg.0));
        }
    }
}

impl DetectorWarns {
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

// REQ-AUTH-017 — Decision: a failure means a forged or unsigned block extends a snap-synced node through the post-snap Light arm.
#[tokio::test]
async fn req_auth_017_post_snap_light_arm_rejects_forged_and_unsigned_accepts_valid() {
    let p = keys(2);
    let (mut src, _ts) = make_node(&p).await;
    let params = src.params.clone();
    let genesis = tip(&src).await.1;
    let b1 = block(1, 1, genesis, &p[0], Sig::Valid, &params);
    let b2 = block(2, 2, b1.hash(), &p[0], Sig::Valid, &params);
    for b in [&b1, &b2] {
        src.apply_block(b.clone(), ValidationMode::Light)
            .await
            .expect("fixture: source chain");
    }
    let snap = snapshot_of(&src, &b2.header).await;
    let (mut dst, _td) = make_node(&p).await;
    dst.apply_snap_snapshot(snap)
        .await
        .expect("fixture: snap install");
    assert_eq!(
        tip(&dst).await,
        (2, b2.hash()),
        "PRECONDITION: snapshot installed"
    );
    assert_eq!(
        dst.snap_sync_height,
        Some(2),
        "PRECONDITION: post-snap Light arm armed"
    );

    let b3 = block(3, 3, b2.hash(), &p[0], Sig::Valid, &params);
    assert_handle_new_block_gate(&mut dst, &b3, &p[0], "post-snap Light").await;
}

// REQ-AUTH-018 — Decision: a failure means a forged or unsigned gossip block is applied through the Full arm of handle_new_block.
#[tokio::test]
async fn req_auth_018_full_handle_new_block_rejects_forged_and_unsigned_accepts_valid() {
    let p = keys(2);
    let (mut node, _t) = make_node(&p).await;
    assert!(
        node.snap_sync_height.is_none() && !node.floor_fallback_window,
        "PRECONDITION: Full arm"
    );
    let genesis = tip(&node).await.1;
    let b1 = block(1, 1, genesis, &p[0], Sig::Valid, &node.params.clone());
    assert_handle_new_block_gate(&mut node, &b1, &p[0], "Full").await;
}

// REQ-AUTH-018 — Decision: a failure means the floor-fallback Light arm of handle_new_block applies forged or unsigned blocks.
#[tokio::test]
async fn req_auth_018_light_floor_fallback_handle_new_block_rejects_forged_and_unsigned_accepts_valid(
) {
    let p = keys(2);
    let (mut node, _t) = make_node(&p).await;
    node.floor_fallback_window = true;
    let genesis = tip(&node).await.1;
    let b1 = block(1, 1, genesis, &p[0], Sig::Valid, &node.params.clone());
    assert_handle_new_block_gate(&mut node, &b1, &p[0], "floor-fallback Light").await;
}

// REQ-AUTH-018 — Decision: a failure means the periodic in-order sync apply (apply_block Light) applies forged or unsigned bodies.
#[tokio::test]
async fn req_auth_018_light_in_order_apply_rejects_forged_and_unsigned_accepts_valid() {
    let p = keys(2);
    let (mut node, _t) = make_node(&p).await;
    let genesis = tip(&node).await.1;
    let b1 = block(1, 1, genesis, &p[0], Sig::Valid, &node.params.clone());
    assert_apply_block_gate(&mut node, &b1, &p[0], ValidationMode::Light).await;
}

// REQ-AUTH-018 — Decision: a failure means the operator recover (Replay) path imports forged or unsigned history at h >= AH.
#[tokio::test]
async fn req_auth_018_replay_new_for_replay_rejects_forged_and_unsigned_accepts_valid() {
    let p = keys(1);
    let temp = TempDir::new().unwrap();
    let mut node = Node::new_for_replay(temp.path().to_path_buf(), Network::Devnet)
        .await
        .expect("Node::new_for_replay failed");
    let params = node.params.clone();
    let genesis = tip(&node).await.1;
    let spec_genesis = doli_core::chainspec::ChainSpec::devnet().genesis_hash();
    let b1 = salted(1, 1, genesis, &p[0], Sig::Valid, &params, spec_genesis, 0);
    assert_apply_block_gate(&mut node, &b1, &p[0], ValidationMode::Replay).await;
}

// REQ-AUTH-018 — Decision: a failure means the INC-I-081 direct-successor fallback applies forged or unsigned fork-walk blocks.
#[tokio::test]
async fn req_auth_018_direct_successor_rejects_forged_and_unsigned_accepts_valid() {
    let p = keys(2);
    let (mut node, _t) = make_node(&p).await;
    let params = node.params.clone();
    let genesis = tip(&node).await.1;
    let b1 = block(1, 1, genesis, &p[0], Sig::Valid, &params);
    node.apply_block(b1.clone(), ValidationMode::Light)
        .await
        .expect("fixture: h1");
    let b2 = block(2, 2, b1.hash(), &p[1], Sig::Valid, &params);
    let before = tip(&node).await;

    for sig in BAD {
        let r = node
            .try_apply_direct_successor(&variant(&b2, &p[1], sig))
            .await;
        match &r {
            Err(e) => assert!(
                is_sig_rejection(e),
                "{sig:?}: not the signature rule: {e:#}"
            ),
            Ok(applied) => assert!(!applied, "{sig:?}: direct successor reported applied"),
        }
        assert_eq!(tip(&node).await, before, "{sig:?}: block applied");
        assert_eq!(canonical_at(&node, 2), None, "{sig:?}: block stored");
    }
    let applied = node
        .try_apply_direct_successor(&b2)
        .await
        .expect("valid direct successor");
    assert!(applied, "valid direct successor not applied");
    assert_eq!(tip(&node).await, (2, b2.hash()));
}

// REQ-AUTH-022 — Decision: a failure means unauthenticated headers fill the detector's seen_blocks table.
#[tokio::test]
async fn req_auth_022_detector_forged_leaves_no_seen_record() {
    let p = keys(2);
    let (mut node, _t) = make_node(&p).await;
    let genesis = tip(&node).await.1;
    let b1 = block(1, 1, genesis, &p[0], Sig::Valid, &node.params.clone());
    for sig in BAD {
        feed(&mut node, variant(&b1, &p[0], sig)).await;
        assert_eq!(
            tracked(&node).await,
            0,
            "{sig:?}: detector recorded an unauthenticated header"
        );
    }
    feed(&mut node, b1).await;
    assert_eq!(
        tracked(&node).await,
        1,
        "NON-VACUITY: a signed header is recorded"
    );
}

// REQ-AUTH-022 — Decision: a failure means one forged far-future header evicts real detector records and hides a real double production.
#[tokio::test]
async fn req_auth_022_detector_forged_far_slot_does_not_evict_real_record() {
    let p = keys(2);
    let (mut node, _t) = make_node(&p).await;
    let params = node.params.clone();
    let gh = doli_core::chainspec::ChainSpec::devnet().genesis_hash();
    let genesis = tip(&node).await.1;
    let a = salted(1, 5, genesis, &p[0], Sig::Valid, &params, gh, 0);
    feed(&mut node, a.clone()).await;
    assert_eq!(
        tracked(&node).await,
        1,
        "PRECONDITION: real header recorded"
    );

    let far = salted(2, 100_005, a.hash(), &p[0], Sig::Forged, &params, gh, 0);
    feed(&mut node, far).await;
    let b = salted(1, 5, genesis, &p[0], Sig::Valid, &params, gh, 1);
    assert_ne!(a.hash(), b.hash(), "fixture: headers must conflict");
    feed(&mut node, b).await;

    assert_eq!(
        proofs_against(&node, p[0].public_key()).await,
        1,
        "real double production undetected after a forged far-slot header"
    );
}

// REQ-AUTH-022 — Decision: a failure means a forged conflicting header makes the node log a false equivocation alarm.
#[tokio::test]
async fn req_auth_022_detector_forged_conflict_emits_no_equivocation_warn() {
    let warns = DetectorWarns::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(warns.clone()));
    let p = keys(2);
    let (mut node, _t) = make_node(&p).await;
    let params = node.params.clone();
    let gh = doli_core::chainspec::ChainSpec::devnet().genesis_hash();
    let genesis = tip(&node).await.1;
    let a = salted(1, 5, genesis, &p[0], Sig::Valid, &params, gh, 0);
    feed(&mut node, a).await;
    let _ = warns.take();

    for sig in BAD {
        feed(&mut node, salted(1, 5, genesis, &p[0], sig, &params, gh, 1)).await;
        assert_eq!(
            warns.take(),
            Vec::<String>::new(),
            "{sig:?}: equivocation WARN on a forged header"
        );
    }
    feed(
        &mut node,
        salted(1, 5, genesis, &p[0], Sig::Valid, &params, gh, 1),
    )
    .await;
    assert!(
        !warns.take().is_empty(),
        "NON-VACUITY: a signed conflict logs the equivocation WARN"
    );
}
