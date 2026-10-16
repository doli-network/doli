//! INC-I-248 M1 — a node that did not replay the chain never seeds its trust root from
//! the live ProducerSet.
//!
//! REQ-248-002 (Must) — Decision: a failure means a snap-joined or wiped-then-snapped node still trusts the producer-derived root (on mainnet the 5 retired, leaked genesis keys) instead of failing closed.
//!
//! OUTPUT CONTRACT: fn Node::maybe_bootstrap_maintainer_set(&self, height)
//!   O2 receiver: maintainer_state.set.members, maintainer_state.last_derived_height
//!   O3 return: unit (none)
//!   O4 persistent store: maintainer_state.bin (must equal memory)
//!   Derived: resolve_trust_root(state).is_usable(); verify_release_with_trust_root(3 rotated)
//!   O1/O5/O6: none
//! PATHS: P1 snap-synced via apply_snap_snapshot, no peer answers -> EMPTY@snap height;
//!   P2 snap-synced via startup recovery marker (doli wipe + snap), no file -> EMPTY@h0.
//! INPUT PARTITIONS: P1 the ProducerSet came from the snapshot (5 producers);
//!   P2 the node's own genesis ProducerSet (5 producers), no snapshot install.
//! MATRIX: 2 paths x (members, last_derived_height, disk, usable, verify) = 10 cells.
//!
//! OUTPUT CONTRACT: fn Node::apply_maintainer_quorum_answers(&self, answers) -> QuorumVerdict
//!   O2 receiver: maintainer_state.set (members, threshold, last_updated), last_derived_height
//!   O3 return: QuorumVerdict
//!   O4 persistent store: maintainer_state.bin (must equal memory)
//!   Derived: digest(B) == digest(A); verify_release_with_trust_root(3 rotated) == Ok(3)
//! OUTPUT CONTRACT: fn Node::serve_maintainer_state(&self) -> SyncResponse
//!   O3 return: MaintainerStateAt { block_hash, block_height, digest, set_bytes, last_derived_height }
//! OUTPUT CONTRACT: Node::handle_network_event(SyncResponse{MaintainerStateAt}) on the receiver
//!   O2 receiver: maintainer_state.set (adopted or unchanged); O4 maintainer_state.bin
//! PATHS: P3 snap-joined B + 3 agreeing answers -> Adopt, seed gate then a no-op;
//!   P4 served frame round-trips the wire and 3 peers make B adopt at the window end (REL-M1);
//!   P5 frame digest bound to a foreign genesis -> dropped; P6 frame digest != body -> dropped;
//!   P7 frame to a node that is not collecting -> dropped (control: the same EMPTY@H node,
//!   armed, adopts at the window end; a replay node never arms).
//! INPUT PARTITIONS: P3/P4 canonical rotated set S1..S5; P5 same set, other genesis;
//!   P6 same set, forged digest; P7 EMPTY@H node and replay node holding the producer seed.
//! MATRIX: P3 6 cells (verdict, digest, set, last_derived_height, disk, verify);
//!   P4 frame fields 5 + adopted set 2; P5/P6/P7 set unchanged + control.
//!
//! OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode::Light) -> Result<()>
//!   (update_chain_state_for_block stamp -> clear_snap_sync -> maybe_bootstrap_maintainer_set)
//!   O1 none; O2 receiver: chain_state.is_snap_synced, maintainer_state.set, last_derived_height
//!   O3 return: Result (Ok, tip advances by 1); O4 persistent store: maintainer_state.bin
//!   Derived: resolve_trust_root(state).is_usable(); verify_release_with_trust_root(3 rotated)
//! PATHS: P8 snap-joined at H, no answers, block H+1 -> EMPTY@H, marker cleared;
//!   P9 snap-joined, adopted A's root, block H+1 -> root unchanged;
//!   P10 snap marker below the gate (seed hook direct and via block) -> equals replay node;
//!   P11 no maintainer state attached (quorum answers, seed hook, block) -> inert.
//! INPUT PARTITIONS: P8/P9 snapshot ProducerSet of 5 producers; P10 own genesis producers,
//!   snap vs not snap; P11 snap-marked node, None maintainer state, 3 genuine answers.
//! MATRIX: P8 6 cells (marker, members, last_derived_height, disk, usable, verify);
//!   P9 6 cells (marker, digest, set, last_derived_height, disk, verify);
//!   P10 2 entry points x (set, last_derived_height, disk) + control;
//!   P11 verdict, no panic, state None, no file.

// covers: quorum mod lib sync sync_codec event_size response snap_sync maintainer_quorum maintainer_state_serve init periodic network_events fork_recovery validation_checks state_update

use crate::inc_i_248_harness as h;
use doli_core::maintainer::{KeepReason, QuorumVerdict};
use doli_core::MaintainerSet;
use doli_node::node::maintainer_quorum::ProbeReason;
use network::protocols::SyncResponse;
use updater::UpdateError;

fn describe(set: &doli_core::MaintainerSet) -> String {
    format!(
        "members={} producer_seed={} threshold={} last_updated={}",
        set.members.len(),
        h::is_producer_seed(set),
        set.threshold,
        set.last_updated
    )
}

#[tokio::test]
async fn inc_i_248_snap_join_without_answers_fails_closed() {
    let a = h::build_replay_node_a().await;
    let snapshot = h::snapshot_of(&a.node).await;
    let (node_b, _temp_b) = h::snap_joined_node_b(snapshot).await;

    node_b
        .maybe_bootstrap_maintainer_set(h::SNAP_HEIGHT + 1)
        .await;

    let state = h::state_of(&node_b).await;
    assert!(
        state.set.members.is_empty(),
        "snap-joined node with no peer answers must hold an EMPTY trust root, got {}",
        describe(&state.set)
    );
    assert_eq!(
        state.last_derived_height,
        h::SNAP_HEIGHT,
        "the EMPTY root must be stamped at the snap height"
    );
    let disk = h::disk_state_of(&node_b);
    assert_eq!(disk.set, state.set, "disk set != memory set");
    assert_eq!(
        disk.last_derived_height, state.last_derived_height,
        "disk last_derived_height != memory"
    );
    let root = h::resolved_root(&node_b).await;
    assert!(!root.is_usable(), "an EMPTY snap root must not be usable");
    let result = h::verify_rotated_release(&node_b).await;
    assert!(
        matches!(result, Err(UpdateError::TrustRootUnavailable { .. })),
        "release verification must fail closed, got {}",
        h::verdict_name(&result)
    );
}

#[tokio::test]
async fn inc_i_248_doli_wipe_then_fresh_node_never_seeds_from_producers() {
    const H0: u64 = h::SNAP_HEIGHT;
    let producers = h::producer_keys();
    let (node, _temp) = h::testnet_node(&producers).await;
    assert!(
        !node.config.data_dir.join("maintainer_state.bin").exists(),
        "fixture: a wiped node has no maintainer file"
    );
    assert!(
        node.producer_set.read().await.all_producers().len() >= h::SEED_MIN_PRODUCERS,
        "fixture: enough producers to seed"
    );
    {
        let mut cs = node.chain_state.write().await;
        cs.best_height = H0;
        cs.mark_snap_synced(H0);
    }

    node.maybe_bootstrap_maintainer_set(H0 + 1).await;

    let state = h::state_of(&node).await;
    let producer_keys: Vec<crypto::PublicKey> = producers.iter().map(|k| *k.public_key()).collect();
    assert!(
        !state.set.members.iter().any(|m| producer_keys.contains(m)),
        "a snap-synced node must never seed producer keys, got {}",
        describe(&state.set)
    );
    assert!(
        state.set.members.is_empty(),
        "a snap-synced node with no file must hold an EMPTY root, got {}",
        describe(&state.set)
    );
    assert_eq!(
        state.last_derived_height, H0,
        "the EMPTY root must be stamped at the snap height"
    );
    let disk = h::disk_state_of(&node);
    assert_eq!(disk.set, state.set, "disk set != memory set");
    assert_eq!(
        disk.last_derived_height, state.last_derived_height,
        "disk last_derived_height != memory"
    );
    assert!(
        !h::resolved_root(&node).await.is_usable(),
        "the EMPTY root must not be usable"
    );
}

fn frame_fields(resp: &SyncResponse) -> (crypto::Hash, u64, [u8; 32], MaintainerSet, u64) {
    match resp {
        SyncResponse::MaintainerStateAt {
            block_hash,
            block_height,
            digest,
            set_bytes,
            last_derived_height,
        } => (
            *block_hash,
            *block_height,
            *digest.as_bytes(),
            bincode::deserialize(set_bytes).expect("set_bytes must decode as MaintainerSet"),
            *last_derived_height,
        ),
        other => panic!(
            "serve_maintainer_state must return MaintainerStateAt, got {}",
            other.type_name()
        ),
    }
}

async fn assert_root_untouched(
    node: &doli_node::node::Node,
    before: &storage::MaintainerState,
    what: &str,
) {
    let state = h::state_of(node).await;
    assert_eq!(
        state.set,
        before.set,
        "{what}: set changed, got {}",
        describe(&state.set)
    );
    assert_eq!(
        state.last_derived_height, before.last_derived_height,
        "{what}: last_derived_height changed"
    );
    if node.config.data_dir.join("maintainer_state.bin").exists() {
        assert_eq!(
            h::disk_state_of(node).set,
            before.set,
            "{what}: disk set changed"
        );
    }
}

#[tokio::test]
async fn inc_i_248_snap_join_root_equals_canonical() {
    // covers: quorum mod lib sync sync_codec event_size response snap_sync maintainer_quorum maintainer_state_serve init periodic network_events fork_recovery validation_checks state_update
    let a = h::build_replay_node_a().await;
    let view = h::peer_view(&a.node).await;
    let snapshot = h::snapshot_of(&a.node).await;
    let (node_b, _temp_b) = h::snap_joined_node_b(snapshot).await;
    let digest = h::node_digest(&node_b, &view.set);
    assert_eq!(digest, view.digest, "fixture: A and B share one genesis");
    let answers = vec![h::answer_of(&view, digest); 3];

    let verdict = node_b.apply_maintainer_quorum_answers(answers).await;
    assert_eq!(
        verdict,
        QuorumVerdict::Adopt {
            set: view.set.clone(),
            agreeing: 3
        }
    );

    node_b
        .maybe_bootstrap_maintainer_set(h::SNAP_HEIGHT + 1)
        .await;

    let state = h::state_of(&node_b).await;
    assert_eq!(
        h::node_digest(&node_b, &state.set),
        view.digest,
        "B's trust root must equal A's, got {}",
        describe(&state.set)
    );
    assert_eq!(state.set, view.set, "B's set must equal A's set");
    assert_eq!(
        state.set.last_updated, view.set.last_updated,
        "B's last_updated must be A's history value"
    );
    let tip_b = node_b.chain_state.read().await.best_height;
    assert_eq!(tip_b, h::SNAP_HEIGHT, "fixture: B stays at the snap height");
    assert_eq!(
        state.last_derived_height, tip_b,
        "adoption stamps last_derived_height at B's tip"
    );
    let disk = h::disk_state_of(&node_b);
    assert_eq!(disk.set, state.set, "disk set != memory set");
    assert_eq!(
        disk.last_derived_height, state.last_derived_height,
        "disk last_derived_height != memory"
    );
    let result = h::verify_rotated_release(&node_b).await;
    assert!(
        matches!(result, Ok(3)),
        "a release signed by 3 rotated maintainers must verify, got {}",
        h::verdict_name(&result)
    );
}

#[tokio::test]
async fn inc_i_248_serve_maintainer_state_round_trip() {
    let a = h::build_replay_node_a().await;
    let view = h::peer_view(&a.node).await;
    let served = a.node.serve_maintainer_state().await;
    let (block_hash, block_height, digest, set, last_derived_height) = frame_fields(&served);
    assert_eq!(
        block_hash, view.block_hash,
        "served block_hash != A's tip hash"
    );
    assert_eq!(
        block_height,
        h::SNAP_HEIGHT,
        "served block_height != A's tip"
    );
    assert_eq!(digest, view.digest, "served digest != A's digest");
    assert_eq!(set, view.set, "served set != A's set");
    assert_eq!(
        last_derived_height, view.last_derived_height,
        "served last_derived_height != A's"
    );

    let wire = bincode::serialize(&served).expect("MaintainerStateAt must serialize");
    let decoded: SyncResponse = bincode::deserialize(&wire).expect("MaintainerStateAt must decode");
    assert_eq!(
        bincode::serialize(&decoded).unwrap(),
        wire,
        "wire round-trip changed the frame"
    );

    let snapshot = h::snapshot_of(&a.node).await;
    let (mut node_b, _temp_b) = h::snap_joined_node_b(snapshot).await;
    h::deliver(&mut node_b, &decoded, 3).await;
    window_end(&node_b).await;

    let state = h::state_of(&node_b).await;
    assert_eq!(
        h::node_digest(&node_b, &state.set),
        view.digest,
        "3 genuine answers must make the snap-joined node adopt A's root, got {}",
        describe(&state.set)
    );
    assert_eq!(state.set, view.set, "adopted set != A's set");
    assert_eq!(
        h::disk_state_of(&node_b).set,
        state.set,
        "disk set != memory set"
    );
}

#[tokio::test]
async fn inc_i_248_serve_maintainer_state_foreign_genesis_dropped() {
    let a = h::build_replay_node_a().await;
    let view = h::peer_view(&a.node).await;
    let (foreign, _temp_f) = h::foreign_genesis_node(h::state_of(&a.node).await).await;
    let served = foreign.serve_maintainer_state().await;
    let (_, _, digest, set, _) = frame_fields(&served);
    assert_eq!(set, view.set, "fixture: the foreign node holds A's set");
    assert_ne!(
        digest, view.digest,
        "fixture: the foreign digest is bound to another genesis"
    );

    let snapshot = h::snapshot_of(&a.node).await;
    let (mut node_b, _temp_b) = h::snap_joined_node_b(snapshot).await;
    assert_ne!(
        h::node_genesis(&foreign),
        h::node_genesis(&node_b),
        "fixture: genesis must differ"
    );
    let before = h::state_of(&node_b).await;
    h::deliver(&mut node_b, &served, 3).await;
    assert_root_untouched(&node_b, &before, "answers from another chain").await;

    h::deliver(&mut node_b, &a.node.serve_maintainer_state().await, 3).await;
    window_end(&node_b).await;
    assert_eq!(
        h::node_digest(&node_b, &h::state_of(&node_b).await.set),
        view.digest,
        "control: the same node must adopt 3 genuine answers"
    );
}

#[tokio::test]
async fn inc_i_248_serve_maintainer_state_digest_mismatch_dropped() {
    let a = h::build_replay_node_a().await;
    let view = h::peer_view(&a.node).await;
    let mut forged = a.node.serve_maintainer_state().await;
    match &mut forged {
        SyncResponse::MaintainerStateAt { digest, .. } => {
            *digest = crypto::hash::hash(b"INC-I-248 digest that is not the body's")
        }
        other => panic!(
            "serve_maintainer_state must return MaintainerStateAt, got {}",
            other.type_name()
        ),
    }

    let snapshot = h::snapshot_of(&a.node).await;
    let (mut node_b, _temp_b) = h::snap_joined_node_b(snapshot).await;
    let before = h::state_of(&node_b).await;
    h::deliver(&mut node_b, &forged, 3).await;
    assert_root_untouched(
        &node_b,
        &before,
        "answers whose digest does not match the body",
    )
    .await;

    h::deliver(&mut node_b, &a.node.serve_maintainer_state().await, 3).await;
    window_end(&node_b).await;
    assert_eq!(
        h::node_digest(&node_b, &h::state_of(&node_b).await.set),
        view.digest,
        "control: the same node must adopt 3 genuine answers"
    );
}

async fn window_end(node: &doli_node::node::Node) {
    node.maintainer_quorum_tick_at(std::time::Instant::now() + std::time::Duration::from_secs(30))
        .await;
}

// REQ-248-004 (Must) — Decision: a failure means a node replaces its root from unsolicited peer frames it never asked for, or a replay node is armed at all.
#[tokio::test]
async fn inc_i_248_serve_maintainer_state_not_collecting_dropped() {
    let a = h::build_replay_node_a().await;
    let view = h::peer_view(&a.node).await;
    let served = a.node.serve_maintainer_state().await;
    let (mut empty, _temp_e) = h::testnet_node(&h::producer_keys()).await;
    empty.set_maintainer_state(std::sync::Arc::new(tokio::sync::RwLock::new(
        storage::MaintainerState {
            last_derived_height: h::SNAP_HEIGHT,
            ..storage::MaintainerState::default()
        },
    )));
    {
        let mut cs = empty.chain_state.write().await;
        cs.best_height = h::SNAP_HEIGHT;
        cs.best_hash = h::tip_hash();
    }
    let before = h::state_of(&empty).await;

    h::deliver(&mut empty, &served, 3).await;
    window_end(&empty).await;
    assert_root_untouched(&empty, &before, "answers to a node that is not collecting").await;

    empty.arm_maintainer_quorum(ProbeReason::Startup).await;
    h::deliver(&mut empty, &served, 3).await;
    window_end(&empty).await;
    assert_eq!(
        h::node_digest(&empty, &h::state_of(&empty).await.set),
        view.digest,
        "control: once armed, 3 unanimous answers must be adopted at the window end"
    );

    let (mut replay, _temp_r) = h::replay_node_at_snap_height().await;
    let before = h::state_of(&replay).await;
    replay.arm_maintainer_quorum(ProbeReason::Startup).await;
    assert_eq!(
        replay.maintainer_probe_phase(),
        "idle",
        "a replay root never arms"
    );
    h::deliver(&mut replay, &served, 3).await;
    window_end(&replay).await;
    assert_root_untouched(&replay, &before, "replay node after arm + window end").await;
}

fn producer_pubkeys() -> Vec<crypto::PublicKey> {
    h::producer_keys().iter().map(|k| *k.public_key()).collect()
}

fn assert_disk_equals_memory(node: &doli_node::node::Node, state: &storage::MaintainerState) {
    let disk = h::disk_state_of(node);
    assert_eq!(disk.set, state.set, "disk set != memory set");
    assert_eq!(
        disk.last_derived_height, state.last_derived_height,
        "disk last_derived_height != memory"
    );
}

// REQ-248-002 (Must) — Decision: a failure means the first real block after a snap clears the snap marker before the seed hook runs, so a snap-joined node still seeds its trust root from the live producers.
#[tokio::test]
async fn inc_i_248_snap_join_first_applied_block_never_seeds_from_producers() {
    let a = h::build_replay_node_a().await;
    let snapshot = h::snapshot_of(&a.node).await;
    let (mut node_b, _temp_b) = h::snap_joined_node_b(snapshot).await;
    assert!(
        node_b.producer_set.read().await.all_producers().len() >= h::SEED_MIN_PRODUCERS,
        "fixture: the snapshot ProducerSet must be large enough to seed"
    );

    let applied = h::apply_next_block(&mut node_b, &a.producers).await;
    assert_eq!(applied, h::SNAP_HEIGHT + 1, "fixture: one block at H+1");

    assert!(
        !node_b.chain_state.read().await.is_snap_synced(),
        "the first applied block must still clear the snap marker"
    );
    let state = h::state_of(&node_b).await;
    let producers = producer_pubkeys();
    assert!(
        !state.set.members.iter().any(|m| producers.contains(m)),
        "the first applied block seeded producer keys: {}",
        describe(&state.set)
    );
    assert!(
        state.set.members.is_empty(),
        "after the first applied block the root must be EMPTY, got {}",
        describe(&state.set)
    );
    assert_eq!(
        state.last_derived_height,
        h::SNAP_HEIGHT,
        "the EMPTY root must be stamped at the snap height"
    );
    assert_disk_equals_memory(&node_b, &state);
    assert!(
        !h::resolved_root(&node_b).await.is_usable(),
        "the EMPTY root must not be usable"
    );
    let result = h::verify_rotated_release(&node_b).await;
    assert!(
        matches!(result, Err(UpdateError::TrustRootUnavailable { .. })),
        "release verification must fail closed, got {}",
        h::verdict_name(&result)
    );
}

// REQ-248-002 (Must) — Decision: a failure means the per-block snap stamp overwrites a root the node already adopted from the peer quorum.
#[tokio::test]
async fn inc_i_248_adopted_root_survives_first_applied_block() {
    let a = h::build_replay_node_a().await;
    let view = h::peer_view(&a.node).await;
    let snapshot = h::snapshot_of(&a.node).await;
    let (mut node_b, _temp_b) = h::snap_joined_node_b(snapshot).await;
    let digest = h::node_digest(&node_b, &view.set);
    let verdict = node_b
        .apply_maintainer_quorum_answers(vec![h::answer_of(&view, digest); 3])
        .await;
    assert_eq!(
        verdict,
        QuorumVerdict::Adopt {
            set: view.set.clone(),
            agreeing: 3
        },
        "fixture: B must adopt A's root before the block"
    );

    h::apply_next_block(&mut node_b, &a.producers).await;

    assert!(
        !node_b.chain_state.read().await.is_snap_synced(),
        "the first applied block must still clear the snap marker"
    );
    let state = h::state_of(&node_b).await;
    assert_eq!(
        h::node_digest(&node_b, &state.set),
        view.digest,
        "the adopted root changed on the first applied block, got {}",
        describe(&state.set)
    );
    assert_eq!(state.set, view.set, "B's set must still equal A's set");
    assert_eq!(
        state.last_derived_height,
        h::SNAP_HEIGHT,
        "the adoption stamp must not move on the first applied block"
    );
    assert_disk_equals_memory(&node_b, &state);
    let result = h::verify_rotated_release(&node_b).await;
    assert!(
        matches!(result, Ok(3)),
        "a release signed by 3 rotated maintainers must verify, got {}",
        h::verdict_name(&result)
    );
}

async fn below_gate_node(height: u64, snap: bool) -> (doli_node::node::Node, tempfile::TempDir) {
    let (node, temp) = h::testnet_node(&h::producer_keys()).await;
    {
        let mut cs = node.chain_state.write().await;
        cs.best_height = height;
        cs.best_hash = h::tip_hash();
        if snap {
            cs.mark_snap_synced(height);
        }
    }
    node.block_store
        .seed_canonical_index(h::tip_hash(), height)
        .expect("fixture: tip at H0 without block history");
    (node, temp)
}

// REQ-248-004 (Must) — Decision: a failure means the snap guard leaked below the derivation gate and changed the consensus-visible pre-activation maintainer set.
#[tokio::test]
async fn inc_i_248_snap_below_gate_keeps_pre_activation_derivation() {
    const H0: u64 = h::GATE - 100;
    let producers = h::producer_keys();
    for applied in [false, true] {
        let (mut snap, _ts) = below_gate_node(H0, true).await;
        let (mut replay, _tr) = below_gate_node(H0, false).await;
        if applied {
            h::apply_next_block(&mut snap, &producers).await;
            h::apply_next_block(&mut replay, &producers).await;
        } else {
            snap.maybe_bootstrap_maintainer_set(H0 + 1).await;
            replay.maybe_bootstrap_maintainer_set(H0 + 1).await;
            assert!(
                snap.chain_state.read().await.is_snap_synced(),
                "fixture: the snap marker must be set during the seed hook"
            );
        }
        let s = h::state_of(&snap).await;
        let r = h::state_of(&replay).await;
        assert!(
            h::is_producer_seed(&r.set),
            "control (applied={applied}): the replay node must seed its producers, got {}",
            describe(&r.set)
        );
        assert_eq!(
            (
                h::sorted(&s.set.members),
                s.set.threshold,
                s.set.last_updated
            ),
            (
                h::sorted(&r.set.members),
                r.set.threshold,
                r.set.last_updated
            ),
            "applied={applied}: snap node {} != replay node {}",
            describe(&s.set),
            describe(&r.set)
        );
        assert_eq!(
            s.last_derived_height, r.last_derived_height,
            "applied={applied}: last_derived_height differs from the replay node"
        );
        assert_eq!(
            s.last_derived_height,
            H0 + 1,
            "applied={applied}: the pre-activation seed stamps the block height"
        );
        assert_disk_equals_memory(&snap, &s);
    }
}

// REQ-248-002 (Must) — Decision: a failure means a node that runs without a maintainer state panics or writes a trust root on the quorum, seed or block path.
#[tokio::test]
async fn inc_i_248_no_maintainer_state_is_inert() {
    let a = h::build_replay_node_a().await;
    let view = h::peer_view(&a.node).await;
    let temp = tempfile::TempDir::new().unwrap();
    let mut node =
        doli_node::node::Node::new_for_test(temp.path().to_path_buf(), h::producer_keys())
            .await
            .expect("Node::new_for_test failed");
    node.config.network = doli_core::Network::Testnet;
    assert!(
        node.maintainer_state.is_none(),
        "fixture: no maintainer state attached"
    );
    {
        let mut cs = node.chain_state.write().await;
        cs.best_height = h::SNAP_HEIGHT;
        cs.best_hash = h::tip_hash();
        cs.mark_snap_synced(h::SNAP_HEIGHT);
    }
    node.block_store
        .seed_canonical_index(h::tip_hash(), h::SNAP_HEIGHT)
        .expect("fixture: tip at H without block history");
    let digest = h::node_digest(&node, &view.set);

    let verdict = node
        .apply_maintainer_quorum_answers(vec![h::answer_of(&view, digest); 3])
        .await;
    assert_eq!(verdict, QuorumVerdict::Keep(KeepReason::NoAnswers));

    node.maybe_bootstrap_maintainer_set(h::SNAP_HEIGHT + 1)
        .await;
    let applied = h::apply_next_block(&mut node, &a.producers).await;
    assert_eq!(applied, h::SNAP_HEIGHT + 1, "fixture: one block at H+1");

    assert!(
        node.maintainer_state.is_none(),
        "no path may attach a maintainer state"
    );
    assert!(
        !node.config.data_dir.join("maintainer_state.bin").exists(),
        "no path may write maintainer_state.bin"
    );
}
