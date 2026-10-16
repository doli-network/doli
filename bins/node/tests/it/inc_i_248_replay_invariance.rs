//! INC-I-248 M1 — replay-node maintainer-set invariance guard (passes before and after).
//!
//! REQ-248-004 (Must) — Decision: a failure means the M1 change altered the maintainer set or an AddMaintainer/RemoveMaintainer/ProtocolActivation verdict of a node that replayed the chain, i.e. a replay node forks from the fleet.
//!
//! OUTPUT CONTRACT:
//!   F1 `Node::maybe_bootstrap_maintainer_set(&self, height)` (gate seed + per-block hook)
//!   F2 `Node::process_transaction_governance(&self, tx, height, producers) -> Option<(u32, u64)>`
//!   F3 the M2 probe path (REL-M1): startup tick + arm + 3 forged `MaintainerStateAt` answers +
//!      window-end tick, run at EMPTY@0 (before the gate), after the gate seed and after the plan
//!   O2 receiver: maintainer_state.set (members, order, threshold, last_updated) and
//!      maintainer_state.last_derived_height
//!   O3 return: F2 -> Some((version, epoch)) for an accepted activation, else None
//!   O4 persistent store: maintainer_state.bin (must equal memory after every step)
//!   O1/O5/O6: none (tracing only, not asserted)
//! PATHS: seed at the gate; add accepted; remove accepted; activation accepted; add rejected
//!   (non-member signers, existing member, full set, former members); remove rejected (below
//!   minimum); activation rejected (2-of-4, non-members, removed producers).
//! INPUT PARTITIONS: set composition P = all producers, M = producers + rotated keys,
//!   R = all rotated keys; set size 3, 4, 5 (threshold 2, 3, 3).
//! MATRIX: 21 states x (O2 set, O2 ldh, O3, O4, digest vs reference, digest vs frozen hex)
//!   + F3 at 3 points x (O2 set, O2 ldh, O4 bytes, probe phase).
//!
//! F4 (REL-M2): `Node::sanitize_retired_maintainer_set(&self) -> bool` on a full block store
//! (every block 0..=tip): O3 false, O2 set/ldh, O4 bytes, eligibility, probe phase. Partitions:
//! D = 1 with tip = D + 1 or tip = 60; D = last rotation with tip = SNAP_HEIGHT. 3 x 6 cells.
//!
// covers: crates/core/src/maintainer/quorum.rs, bins/node/src/node/maintainer_quorum.rs, bins/node/src/node/periodic.rs
// covers: bins/node/src/node/maintainer_sanitize.rs, bins/node/src/run.rs (start-time sanitize)

use std::time::{Duration, Instant};

use crate::inc_i_248_harness as h;
use crate::inc_i_248_rel_m2_fixture as m2;
use doli_core::maintainer::{derive_canonical_maintainer_set, maintainer_set_digest};
use doli_core::MaintainerSet;
use doli_node::node::maintainer_quorum::ProbeReason;
use network::protocols::SyncResponse;

const DIGEST_GENESIS: [u8; 32] = [0x48; 32];

/// Digest after the gate seed (index 0) and after each plan step (index 1..=20).
const FROZEN_DIGESTS: [&str; 21] = [
    "93c01bab5b47f245930f95e94abd555cd4c3bec8d658dd75ba01428dfe036836",
    "6b3a48f9f752a101f5ee24c44b0f5c4d6f34a8a9756d536420cecaeab33d809b",
    "6b3a48f9f752a101f5ee24c44b0f5c4d6f34a8a9756d536420cecaeab33d809b",
    "6b3a48f9f752a101f5ee24c44b0f5c4d6f34a8a9756d536420cecaeab33d809b",
    "6b3a48f9f752a101f5ee24c44b0f5c4d6f34a8a9756d536420cecaeab33d809b",
    "6b3a48f9f752a101f5ee24c44b0f5c4d6f34a8a9756d536420cecaeab33d809b",
    "6b3a48f9f752a101f5ee24c44b0f5c4d6f34a8a9756d536420cecaeab33d809b",
    "3a9dd5055eb08dd8f1e40b32c2d5b82e3700496f138edc4829f7c39365cb9d56",
    "52169898fc1e08aac4e49234b74180b75cd4f3d2367ca989aa3f6ea267e9b003",
    "14cee3e81bb6455faaaa6555580bd400680f7a6e96781d552352c859cbd9e435",
    "14cee3e81bb6455faaaa6555580bd400680f7a6e96781d552352c859cbd9e435",
    "99f0d1d2dfbb50cbdb3003aff70f44b5f0a74becd0581dcf31ea42a43cb380fb",
    "3b9fdb869011d1d19f459b01018ed3e09b04353a7eb8148c49db8a26584f681d",
    "e13cc6ecbb5c5e6890a4bda8d6d2730552912261b36428add44e67f841aba74b",
    "409b72a90983c679fb3cc7c2547098d1eb9f0cb49cf3f0d6c5aff35d9f907a7c",
    "5c4f59508afafd9430bf49bc34470632d7998e18509b8de184fa4ca47366e96b",
    "0dad8ffdd57a4092465467958e983e6c27b510b3b967f162ee6024a3eddbee1b",
    "0dad8ffdd57a4092465467958e983e6c27b510b3b967f162ee6024a3eddbee1b",
    "0dad8ffdd57a4092465467958e983e6c27b510b3b967f162ee6024a3eddbee1b",
    "0dad8ffdd57a4092465467958e983e6c27b510b3b967f162ee6024a3eddbee1b",
    "0dad8ffdd57a4092465467958e983e6c27b510b3b967f162ee6024a3eddbee1b",
];

struct Reference {
    set: MaintainerSet,
    last_derived_height: u64,
}

fn reference_seed() -> Reference {
    let registrations: Vec<(crypto::PublicKey, u64)> =
        h::PRODUCER_SEEDS.iter().map(|s| (h::pk(*s), 0)).collect();
    Reference {
        set: derive_canonical_maintainer_set(&registrations, h::GATE),
        last_derived_height: h::GATE,
    }
}

/// Returns (accepted, activation result) the way the frozen v6.44.0 rules decide them.
fn reference_apply(
    r: &mut Reference,
    genesis: &[u8; 32],
    s: &h::Step,
) -> (bool, Option<(u32, u64)>) {
    let sigs = h::step_signatures(genesis, s);
    let message = h::step_message(genesis, s);
    match s.op {
        h::Op::Add(t) => {
            let ok = r.set.verify_multisig_at(&sigs, &message, s.height, h::GATE)
                && r.set.add_maintainer(h::pk(t), s.height).is_ok();
            if ok {
                r.last_derived_height = s.height;
            }
            (ok, None)
        }
        h::Op::Remove(t) => {
            let target = h::pk(t);
            let ok =
                r.set
                    .verify_multisig_excluding_at(&sigs, &message, &target, s.height, h::GATE)
                    && r.set.remove_maintainer(&target, s.height).is_ok();
            if ok {
                r.last_derived_height = s.height;
            }
            (ok, None)
        }
        h::Op::Activate(v, e) => {
            let ok = r.set.is_authorizable()
                && r.set.verify_multisig_at(&sigs, &message, s.height, h::GATE);
            (ok, ok.then_some((v, e)))
        }
    }
}

fn hex_digest(set: &MaintainerSet) -> String {
    hex::encode(maintainer_set_digest(set, &DIGEST_GENESIS))
}

/// F3: no peer input may change a replaying node's set, at any point of its history.
async fn assert_probe_never_changes(node: &mut doli_node::node::Node, ctx: &str) {
    let before = h::state_of(node).await;
    let file = node.config.data_dir.join("maintainer_state.bin");
    let before_bytes = std::fs::read(&file).ok();
    let forged =
        MaintainerSet::with_members(h::OUTSIDER_SEEDS.iter().map(|s| h::pk(*s)).collect(), 0);
    let frame = SyncResponse::MaintainerStateAt {
        block_hash: h::tip_hash(),
        block_height: 50_000,
        digest: crypto::Hash::from_bytes(h::node_digest(node, &forged)),
        set_bytes: bincode::serialize(&forged).expect("forged set must serialize"),
        last_derived_height: 50_000,
    };
    node.maintainer_quorum_tick_at(Instant::now() + Duration::from_secs(11))
        .await;
    node.arm_maintainer_quorum(ProbeReason::Startup).await;
    assert_eq!(
        node.maintainer_probe_phase(),
        "idle",
        "[{ctx}] a replaying node is never eligible"
    );
    h::deliver(node, &frame, 3).await;
    node.maintainer_quorum_tick_at(Instant::now() + Duration::from_secs(30))
        .await;
    let after = h::state_of(node).await;
    assert_eq!(after.set, before.set, "[{ctx}] set changed by peer answers");
    assert_eq!(
        after.last_derived_height, before.last_derived_height,
        "[{ctx}] last_derived_height changed by peer answers"
    );
    assert_eq!(
        hex_digest(&after.set),
        hex_digest(&before.set),
        "[{ctx}] digest changed"
    );
    assert_eq!(
        std::fs::read(&file).ok(),
        before_bytes,
        "[{ctx}] maintainer_state.bin changed by peer answers"
    );
}

struct Row {
    label: &'static str,
    height: u64,
    node_digest: String,
    reference_digest: String,
}

#[tokio::test]
async fn inc_i_248_replay_node_maintainer_set_invariant() {
    let plan = h::rotation_plan();
    for kind in ["add", "remove", "activate"] {
        let of_kind = |s: &&h::Step| {
            matches!(
                (kind, s.op),
                ("add", h::Op::Add(_))
                    | ("remove", h::Op::Remove(_))
                    | ("activate", h::Op::Activate(..))
            )
        };
        assert!(
            plan.iter().filter(of_kind).any(|s| s.accept)
                && plan.iter().filter(of_kind).any(|s| !s.accept),
            "non-vacuity: the plan must accept AND reject at least one {kind}"
        );
    }

    let producers = h::producer_keys();
    let (mut node, _temp) = h::testnet_node(&producers).await;
    let genesis = h::node_genesis(&node);
    assert!(
        !node.chain_state.read().await.is_snap_synced(),
        "fixture: the replay node must not be snap-synced"
    );

    let mut reference = reference_seed();
    node.sync_manager.write().await.add_peer(
        network::PeerId::random(),
        50_000,
        h::tip_hash(),
        50_000,
    );
    assert_probe_never_changes(&mut node, "EMPTY@0 before the gate").await;
    node.maybe_bootstrap_maintainer_set(h::GATE).await;
    assert_probe_never_changes(&mut node, "producer seed at the gate").await;

    let mut rows: Vec<Row> = Vec::with_capacity(plan.len() + 1);
    let mut check_state = |label: &'static str,
                           height: u64,
                           node_state: storage::MaintainerState,
                           disk: storage::MaintainerState,
                           reference: &Reference| {
        assert_eq!(
            node_state.set, reference.set,
            "[{label} @ {height}] node set (members, order, threshold, last_updated) != reference"
        );
        assert_eq!(
            node_state.last_derived_height, reference.last_derived_height,
            "[{label} @ {height}] node last_derived_height != reference"
        );
        assert_eq!(
            disk.set, node_state.set,
            "[{label} @ {height}] disk set != memory"
        );
        assert_eq!(
            disk.last_derived_height, node_state.last_derived_height,
            "[{label} @ {height}] disk last_derived_height != memory"
        );
        rows.push(Row {
            label,
            height,
            node_digest: hex_digest(&node_state.set),
            reference_digest: hex_digest(&reference.set),
        });
    };

    check_state(
        "gate seed",
        h::GATE,
        h::state_of(&node).await,
        h::disk_state_of(&node),
        &reference,
    );

    for s in &plan {
        let before = h::state_of(&node).await;
        let node_ret = h::apply_step(&node, s).await;
        let after = h::state_of(&node).await;
        let (ref_accept, ref_ret) = reference_apply(&mut reference, &genesis, s);

        let node_accept = match s.op {
            h::Op::Activate(..) => node_ret.is_some(),
            _ => after.set != before.set,
        };
        assert_eq!(
            ref_accept, s.accept,
            "[{} @ {}] reference verdict != the plan's declared verdict",
            s.label, s.height
        );
        assert_eq!(
            node_accept, ref_accept,
            "[{} @ {}] node verdict != reference verdict",
            s.label, s.height
        );
        assert_eq!(
            node_ret, ref_ret,
            "[{} @ {}] process_transaction_governance return != reference",
            s.label, s.height
        );
        check_state(
            s.label,
            s.height,
            after,
            h::disk_state_of(&node),
            &reference,
        );
    }

    assert_probe_never_changes(&mut node, "after the plan").await;

    for (i, row) in rows.iter().enumerate() {
        println!(
            "INC-I-248-REPLAY {i:2} {:>6} {:<24} {}",
            row.height, row.label, row.node_digest
        );
    }
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(
            row.node_digest, row.reference_digest,
            "[{} @ {}] node digest != reference digest",
            row.label, row.height
        );
        assert_eq!(
            row.node_digest, FROZEN_DIGESTS[i],
            "[{} @ {}] digest != frozen v6.44.0 digest #{i}",
            row.label, row.height
        );
    }
}

// REQ-248-004 (Must) — Decision: a failure means the REL-M2 start-time check resets the root of a node that replayed every block, so the peer quorum could replace replayed history.
#[tokio::test]
async fn inc_i_248_replay_node_full_store_keeps_root_on_start() {
    let (mut seeded, _ts) = h::testnet_node(&h::producer_keys()).await;
    m2::assert_retired_check_off(&seeded);
    seeded.maybe_bootstrap_maintainer_set(1).await;
    let disk = h::disk_state_of(&seeded);
    assert!(
        h::is_producer_seed(&disk.set) && disk.last_derived_height == 1,
        "fixture: producer seed at h=1 on disk"
    );
    m2::install_state(&mut seeded, &disk);
    m2::put_heights(&seeded, &[0..=2]);
    m2::set_tip_to_store(&seeded, 2).await;
    m2::assert_start_keeps(&seeded, "seed D = 1, tip = D + 1").await;
    m2::put_heights(&seeded, &[3..=60]);
    m2::set_tip_to_store(&seeded, 60).await;
    m2::assert_start_keeps(&seeded, "seed D = 1, tip = 60").await;

    let a = h::build_replay_node_a().await;
    assert_eq!(
        h::state_of(&a.node).await.last_derived_height,
        h::last_rotation_height(),
        "fixture: D = last rotation height"
    );
    m2::put_heights(&a.node, &[0..=h::SNAP_HEIGHT]);
    m2::set_tip_to_store(&a.node, h::SNAP_HEIGHT).await;
    m2::assert_start_keeps(&a.node, "rotated D, tip = SNAP_HEIGHT").await;
    assert_eq!(
        hex_digest(&h::state_of(&a.node).await.set),
        FROZEN_DIGESTS[20],
        "digest != frozen v6.44.0 digest after the plan"
    );
}
