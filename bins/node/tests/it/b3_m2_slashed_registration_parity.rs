//! B3 M2 — one verdict on builder, apply (Full, Light) and replay for a
//! Registration or AddBond whose key is Slashed or slash-queued: refused with
//! the shared code from the slashing gate on, unchanged below it.
//!
// covers: assembly mod chain slash_state slash_registration registration transaction slash_admission pool validation_checks
//
// OUTPUT CONTRACT: fn Node::build_block_content(&mut self, ..) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>
//   O3 selected txs: a resident Registration / AddBond whose key became slashed or slash-queued is not selected
//      at/above the gate; a clean AddBond is selected (control); below the gate all are selected
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O3 return: Err whose chain carries "[ERRTX071]" at/above the gate; Ok below it
//   O2 self chain tip: unchanged on Err, advanced on Ok
// PATHS: P-BUILD | P-APPLY x {Full, Light, Replay (the mode operations/chain.rs replays with)} | P-BELOW (child, gate far above the tip)
// INPUT PARTITIONS:
//   key state: Slashed (Registration and AddBond) | Active + queued Slash (AddBond) | clean Active (control)
//   gate: devnet 0 (above) | DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT = 1_000_000 (below)
// MATRIX:
//   O3 x P-BUILD x {Slashed Reg, Slashed AddBond, queued AddBond, clean AddBond} -> b3_m2_req_b3_003_builder_skips_slashed_registrant
//   O3 x P-BUILD x P-BELOW -> b3_m2_req_b3_003_builder_below_gate_unchanged
//   O3,O2 x P-APPLY x 3 modes x 3 refused cases + clean control -> b3_m2_req_b3_003_apply_and_replay_refuse_slashed_registrant
//   O3,O2 x P-APPLY x 3 modes x P-BELOW -> b3_m2_req_b3_003_apply_below_gate_unchanged

use crypto::{BlsKeyPair, KeyPair, PublicKey};
use doli_core::transaction::{Input, Transaction, TxType};
use doli_core::validation::ValidationMode;
use doli_node::node::Node;
use storage::PendingProducerUpdate;

use super::inc_i_255_m1_common::{
    full_block_with, in_child, run_in_child, tip, world_with, World, MODES,
};
use super::inc_i_255_m2_common::producer_auth;
use super::inc_i_255_m4_common::admit_at;
use super::inc_i_255_m6_common::seed_coin;

const CODE: &str = "ERRTX071";
const FEE: u64 = 100_000;
const LOCK: u64 = 10_000_000;
const BELOW_GATE: u64 = 1_000_000;

/// Replaces the two empty trailing `Vec<u8>` prefixes (BLS key, PoP) of the encoded payload.
fn with_bls(tx: &mut Transaction, seed: u8) {
    let bls = BlsKeyPair::from_seed(&[seed; 32]).expect("FIXTURE: BLS seed");
    let pop = bls.proof_of_possession().expect("FIXTURE: PoP");
    let n = tx.extra_data.len();
    assert_eq!(
        &tx.extra_data[n - 16..],
        &[0u8; 16],
        "FIXTURE: RegistrationData layout"
    );
    tx.extra_data.truncate(n - 16);
    for part in [
        bls.public_key().as_bytes().to_vec(),
        pop.as_bytes().to_vec(),
    ] {
        tx.extra_data
            .extend_from_slice(&(part.len() as u64).to_le_bytes());
        tx.extra_data.extend_from_slice(&part);
    }
}

/// Producer-signed `kind` for `kp`, paid from a fresh coin of its own at `[tag; 32]:0`.
async fn producer_tx(node: &Node, kind: TxType, kp: &KeyPair, tag: u8) -> Transaction {
    let pk = *kp.public_key();
    let unit = node.config.network.bond_unit();
    let op = seed_coin(node, &pk, tag, unit + FEE).await;
    let mut input = Input::new(op.tx_hash, op.index);
    input.public_key = Some(pk);
    let mut tx = if kind == TxType::Registration {
        let mut t = Transaction::new_registration(vec![input], pk, unit, LOCK, 1);
        with_bls(&mut t, tag);
        t
    } else {
        Transaction::new_add_bond(vec![input], pk, 1, unit, LOCK)
    };
    let msg = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&msg, kp.private_key());
    producer_auth(tx, kp)
}

async fn register(node: &Node, pk: &PublicKey) {
    let unit = node.config.network.bond_unit();
    node.producer_set
        .write()
        .await
        .register_genesis_producer(*pk, 1, unit)
        .expect("FIXTURE: register producer");
}

async fn mark_slashed(node: &Node, pk: &PublicKey) {
    let h = tip(node).await.0;
    node.producer_set
        .write()
        .await
        .slash_producer(pk, h)
        .expect("FIXTURE: slash producer");
}

async fn mark_queued(node: &Node, pk: &PublicKey) {
    let h = tip(node).await.0;
    node.producer_set
        .write()
        .await
        .queue_update(PendingProducerUpdate::Slash {
            pubkey: *pk,
            height: h,
        });
}

fn coded(r: &anyhow::Result<()>) -> bool {
    matches!(r, Err(e) if format!("{e:#} {e:?}").contains(CODE))
}

/// World with the accused Slashed and a third producer Active with a queued Slash;
/// the refused cases are [Registration(accused), AddBond(accused), AddBond(third)].
async fn apply_world() -> (World, Vec<(&'static str, Transaction)>, Transaction) {
    let w = world_with(&KeyPair::generate(), &KeyPair::generate()).await;
    let third = KeyPair::generate();
    register(&w.node, third.public_key()).await;
    let cases = vec![
        (
            "Registration by a Slashed key",
            producer_tx(&w.node, TxType::Registration, &w.accused, 0x31).await,
        ),
        (
            "AddBond by a Slashed producer",
            producer_tx(&w.node, TxType::AddBond, &w.accused, 0x32).await,
        ),
        (
            "AddBond by a slash-queued producer",
            producer_tx(&w.node, TxType::AddBond, &third, 0x33).await,
        ),
    ];
    let clean = producer_tx(&w.node, TxType::AddBond, &w.reporter, 0x34).await;
    mark_slashed(&w.node, &w.accused_pk()).await;
    mark_queued(&w.node, third.public_key()).await;
    (w, cases, clean)
}

/// One fresh world per (mode, case): the next block carries only case `i`
/// (index 3 is the clean AddBond); returns the verdict and whether the tip moved.
async fn apply_case(mode: ValidationMode, i: usize) -> (&'static str, anyhow::Result<()>, bool) {
    let (mut w, mut cases, clean) = apply_world().await;
    cases.push(("clean AddBond (control)", clean));
    let (name, tx) = cases.swap_remove(i);
    let before = tip(&w.node).await;
    let block = full_block_with(&mut w.node, std::slice::from_ref(&tx))
        .await
        .0;
    let res = w.node.apply_block(block, mode).await;
    (name, res, before != tip(&w.node).await)
}

async fn apply_in_every_mode(want_refused: bool) {
    for mode in MODES {
        for i in 0..3 {
            let (name, res, moved) = apply_case(mode, i).await;
            if want_refused {
                assert!(
                    coded(&res),
                    "{mode:?}: a block carrying {name} must be rejected with {CODE}: {res:?}"
                );
                assert!(
                    !moved,
                    "{mode:?}: the rejected block must not move the tip ({name})"
                );
            } else {
                assert!(res.is_ok(), "{mode:?} below the gate: a block carrying {name} keeps the old verdict Ok: {res:?}");
                assert!(
                    moved,
                    "{mode:?} below the gate: the accepted block moves the tip ({name})"
                );
            }
        }
        let (name, res, moved) = apply_case(mode, 3).await;
        assert!(
            res.is_ok() && moved,
            "NON-VACUITY {mode:?}: a block carrying a {name} applies: {res:?}"
        );
    }
}

// REQ-B3-003 — Decision: a failure means apply or replay accepts a block that registers or bonds a Slashed or slash-queued key at/above the gate, so the chain mints Bond UTXOs that storage orphans at the boundary, or the modes disagree.
#[tokio::test]
async fn b3_m2_req_b3_003_apply_and_replay_refuse_slashed_registrant() {
    apply_in_every_mode(true).await;
}

// REQ-B3-003 — Decision: a failure means apply or replay changes its verdict below the slashing gate, a consensus change without an activation height.
#[tokio::test]
async fn b3_m2_req_b3_003_apply_below_gate_unchanged() {
    if !in_child() {
        return run_in_child(
            "b3_m2_slashed_registration_parity::b3_m2_req_b3_003_apply_below_gate_unchanged",
            BELOW_GATE,
        );
    }
    apply_in_every_mode(false).await;
}

/// Builder selection after: AddBond(accused) and AddBond(third) admitted while Active, Registration(fresh)
/// admitted while unknown, a clean AddBond(reporter); then accused and fresh Slashed, third slash-queued.
async fn builder_selection() -> (Vec<(&'static str, bool)>, bool) {
    let mut w = world_with(&KeyPair::generate(), &KeyPair::generate()).await;
    let (third, fresh) = (KeyPair::generate(), KeyPair::generate());
    register(&w.node, third.public_key()).await;
    let h = tip(&w.node).await.0;
    let cases = [
        (
            "Registration by a key slashed after admission",
            producer_tx(&w.node, TxType::Registration, &fresh, 0x41).await,
        ),
        (
            "AddBond by a producer slashed after admission",
            producer_tx(&w.node, TxType::AddBond, &w.accused, 0x42).await,
        ),
        (
            "AddBond by a producer slash-queued after admission",
            producer_tx(&w.node, TxType::AddBond, &third, 0x43).await,
        ),
    ];
    let clean = producer_tx(&w.node, TxType::AddBond, &w.reporter, 0x44).await;
    for tx in cases.iter().map(|(_, t)| t).chain([&clean]) {
        let r = admit_at(&w.node, tx, h).await;
        assert!(
            r.is_ok(),
            "PRECONDITION: {:?} admitted before the slash: {r:?}",
            tx.tx_type
        );
    }
    register(&w.node, fresh.public_key()).await;
    mark_slashed(&w.node, fresh.public_key()).await;
    mark_slashed(&w.node, &w.accused_pk()).await;
    mark_queued(&w.node, third.public_key()).await;
    let selected = full_block_with(&mut w.node, &[]).await.1;
    let seen = cases
        .iter()
        .map(|(name, tx)| (*name, selected.contains(&tx.hash())))
        .collect();
    (seen, selected.contains(&clean.hash()))
}

// REQ-B3-003 — Decision: a failure means the builder packs a resident Registration or AddBond whose key was slashed after admission, so this node produces a block every peer rejects (or that orphans the bonds).
#[tokio::test]
async fn b3_m2_req_b3_003_builder_skips_slashed_registrant() {
    let (seen, clean) = builder_selection().await;
    assert!(
        clean,
        "NON-VACUITY: the builder selects the clean resident AddBond"
    );
    for (name, picked) in seen {
        assert!(
            !picked,
            "the builder must not select the {name} at/above the gate"
        );
    }
}

// REQ-B3-003 — Decision: a failure means the builder drops a valid producer tx below the slashing gate.
#[tokio::test]
async fn b3_m2_req_b3_003_builder_below_gate_unchanged() {
    if !in_child() {
        return run_in_child(
            "b3_m2_slashed_registration_parity::b3_m2_req_b3_003_builder_below_gate_unchanged",
            BELOW_GATE,
        );
    }
    let (seen, clean) = builder_selection().await;
    assert!(
        clean,
        "NON-VACUITY: the builder selects the clean resident AddBond"
    );
    for (name, picked) in seen {
        assert!(
            picked,
            "below the gate the builder keeps selecting the {name}"
        );
    }
}
