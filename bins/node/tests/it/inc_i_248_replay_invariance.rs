//! INC-I-248 M1 — replay-node maintainer-set invariance guard (passes before and after).
//!
//! REQ-248-004 (Must) — Decision: a failure means the M1 change altered the maintainer set or an AddMaintainer/RemoveMaintainer/ProtocolActivation verdict of a node that replayed the chain, i.e. a replay node forks from the fleet.
//!
//! OUTPUT CONTRACT:
//!   F1 `Node::maybe_bootstrap_maintainer_set(&self, height)` (gate seed + per-block hook)
//!   F2 `Node::process_transaction_governance(&self, tx, height, producers) -> Option<(u32, u64)>`
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
//! MATRIX: 21 states x (O2 set, O2 ldh, O3, O4, digest vs reference, digest vs frozen hex).

use crate::inc_i_248_harness as h;
use doli_core::maintainer::{derive_canonical_maintainer_set, maintainer_set_digest};
use doli_core::MaintainerSet;

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
    let (node, _temp) = h::testnet_node(&producers).await;
    let genesis = h::node_genesis(&node);
    assert!(
        !node.chain_state.read().await.is_snap_synced(),
        "fixture: the replay node must not be snap-synced"
    );

    let mut reference = reference_seed();
    node.maybe_bootstrap_maintainer_set(h::GATE).await;

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
