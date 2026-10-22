//! INC-I-255 M2 — a mined slash spends every Bond UTXO the accused owns and
//! nothing else; the value leaves circulation.
//!
// covers: tx_processing helpers state_update rollback set_lifecycle set_core info block equivocation rewards ingress pool slash_admission withdrawal_economics assembly slash_submit slash_state slash_evidence slash_canonical slash_burn slash_reported constants in_memory set batch utxo types scheduler selection producer_state event_loop block_handling periodic revalidate gossip relay lib mod main validation_checks slash_withdrawal init
//
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O1 return
//   O2 UTXO set: spent outpoints, created outpoints, total value
//   O3 producer set: slash queued or flushed for the accused
//   O4 state root over each UTXO backend (state_db, InMemory image)
// PATHS: P-MID slash block mid-epoch | P-E0 slash block in epoch 0
//        | P-ZERO accused holds no Bond UTXO | P-OFF slash gate env-raised (Replay apply)
// MATRIX
//   O1,O2 x P-MID  -> Ok; spent = accused bonds; created = coinbase; supply += coinbase - burned (inc_i_255_m2_probe_burn)
//   O1,O2 x P-E0   -> same                                         (inc_i_255_m2_b10_epoch0_*)
//   O1-O3 x P-ZERO -> Ok; spent = []; slash recorded               (inc_i_255_m2_b10_zero_bond_*)
//   O1,O2 x P-OFF  -> Ok (Replay); spent = []                       (inc_i_255_m2_b10_below_gate_*)
//   O2,O4 x P-MID  -> burned set and root equal on both backends    (inc_i_255_m2_b10_both_backends_*)
// INPUT PARTITIONS:
//   accused UTXOs: own bonds incl. one delegated away | zero bonds | a normal coin (never burned)
//   other owners: delegator bonds delegated to the accused | honest producer bonds
//   height: epoch 0 | mid-epoch after a boundary; gate: devnet default | env-raised

use std::collections::HashSet;

use doli_core::validation::ValidationMode;
use storage::{Outpoint, UtxoSet};

use super::inc_i_255_m1_common::{in_child, run_in_child, slash_recorded};
use super::inc_i_255_m2_common::*;

const BELOW: u64 = 1_000_000_000;
const MOD: &str = "inc_i_255_m2_effect_burn::";

fn diff(before: &[Outpoint], after: &[Outpoint]) -> (HashSet<Outpoint>, HashSet<Outpoint>) {
    let (b, a): (HashSet<_>, HashSet<_>) = (
        before.iter().copied().collect(),
        after.iter().copied().collect(),
    );
    (
        b.difference(&a).copied().collect(),
        a.difference(&b).copied().collect(),
    )
}

async fn burn_at(slash_height: u64) {
    let mut r = rig(&Keys::new()).await;
    r.advance_to(slash_height - 1).await;
    let pk = r.accused_pk();
    let own = outpoints(ACCUSED_TAG, ACCUSED_BONDS);
    assert_eq!(
        live(&r.node, &own).await,
        own,
        "PRECONDITION: accused bonds live"
    );
    let before = snap(&r.node).await;
    let block = r.next_block(vec![r.slash()], 0).await;
    let (minted, cb_ops) = coinbase_outputs(&block);
    let res = r.apply(block).await;
    assert!(
        res.is_ok(),
        "PRECONDITION: a valid slash block applies; got {:?}",
        res.err().map(|e| format!("{e:#}"))
    );
    let after = snap(&r.node).await;

    let still = live(&r.node, &own).await;
    assert!(
        still.is_empty(),
        "B10: every Bond UTXO owned by the accused (incl. the one delegated away) must be spent by the slash block; still live {still:?}"
    );
    assert!(
        bond_outpoints_of(&r.node, &pk).await.is_empty(),
        "B10: the accused must own no Bond UTXO after the slash block"
    );
    let others = [
        outpoints(DELEGATOR_TAG, DELEGATOR_BONDS),
        outpoints(HONEST_TAG, HONEST_BONDS),
        outpoints(COIN_TAG, 1),
    ]
    .concat();
    assert_eq!(
        live(&r.node, &others).await,
        others,
        "a delegator's bond to the accused, other producers' bonds and the accused's normal coin must stay unspent"
    );
    let (spent, created) = diff(&before.ops, &after.ops);
    assert_eq!(
        spent,
        own.iter().copied().collect::<HashSet<_>>(),
        "the slash block must spend exactly the accused's Bond UTXOs"
    );
    assert_eq!(
        created,
        cb_ops.iter().copied().collect::<HashSet<_>>(),
        "the burn must create no output (no reporter reward, no pool credit)"
    );
    let burned = u64::from(ACCUSED_BONDS) * r.unit();
    assert_eq!(
        after.supply,
        before.supply + minted - burned,
        "supply must drop by exactly the burned bond amount (net of the coinbase)"
    );
}

// REQ-SLASH-003 — Decision: a failure means a mined slash leaves the offender's bond spendable, so slashing has no economic effect (B10).
#[tokio::test]
async fn inc_i_255_m2_probe_burn() {
    burn_at(EPOCH + 1).await;
}

// REQ-SLASH-003 — Decision: a failure means the burn depends on the epoch-0 every-block flush, so a slash mined in the first epoch keeps the bond.
#[tokio::test]
async fn inc_i_255_m2_b10_epoch0_slash_block_burns_every_owned_bond() {
    burn_at(1).await;
}

// REQ-SLASH-003 — Decision: a failure means an accused with no live Bond UTXO makes the slash block invalid, so an offender escapes by withdrawing first.
#[tokio::test]
async fn inc_i_255_m2_b10_zero_bond_accused_is_still_a_valid_slash() {
    let mut r = rig_with(&Keys::new(), 0).await;
    let pk = r.accused_pk();
    let before = snap(&r.node).await;
    let block = r.next_block(vec![r.slash()], 0).await;
    let (minted, cb_ops) = coinbase_outputs(&block);
    let res = r.apply(block).await;
    assert!(
        res.is_ok(),
        "a slash against an accused with zero Bond UTXOs must apply; got {:?}",
        res.err().map(|e| format!("{e:#}"))
    );
    assert!(
        slash_recorded(&r.node, &pk).await,
        "the slash must be recorded"
    );
    let after = snap(&r.node).await;
    let (spent, created) = diff(&before.ops, &after.ops);
    assert!(spent.is_empty(), "nothing to burn; spent {spent:?}");
    assert_eq!(created, cb_ops.into_iter().collect::<HashSet<_>>());
    assert_eq!(after.supply, before.supply + minted);
}

async fn below_gate_body() {
    let mut r = rig(&Keys::new()).await;
    let own = outpoints(ACCUSED_TAG, ACCUSED_BONDS);
    let before = snap(&r.node).await;
    let block = r.next_block(vec![r.slash()], 0).await;
    let (minted, _) = coinbase_outputs(&block);
    r.node
        .apply_block(block, ValidationMode::Replay)
        .await
        .expect("below the gate Replay keeps tolerating the slash");
    assert_eq!(
        live(&r.node, &own).await,
        own,
        "below the slash gate no Bond UTXO may be burned"
    );
    assert_eq!(snap(&r.node).await.supply, before.supply + minted);
}

// REQ-SLASH-003 — Decision: a failure means the burn runs below the activation height, rewriting the UTXO history of every replayed pre-gate block.
#[tokio::test]
async fn inc_i_255_m2_b10_below_gate_slash_burns_nothing() {
    if in_child() {
        below_gate_body().await;
        return;
    }
    run_in_child(
        &format!("{MOD}inc_i_255_m2_b10_below_gate_slash_burns_nothing"),
        BELOW,
    );
}

// REQ-SLASH-003 — Decision: a failure means the burn reaches the state_db backend and an InMemory image differently, so a snap-synced node gets another state root.
#[tokio::test]
async fn inc_i_255_m2_b10_both_backends_agree_on_burned_set_and_root() {
    let mut r = rig(&Keys::new()).await;
    r.advance_to(EPOCH).await;
    r.step(vec![r.slash()]).await;
    let own = outpoints(ACCUSED_TAG, ACCUSED_BONDS);
    let cs = r.node.chain_state.read().await.clone();
    let ps = r.node.producer_set.read().await.clone();
    let utxo = r.node.utxo_set.read().await;
    assert!(utxo.is_rocksdb(), "PRECONDITION: production backend");
    let mem = UtxoSet::deserialize_canonical(&utxo.serialize_canonical()).expect("decode image");
    assert!(!mem.is_rocksdb(), "PRECONDITION: InMemory image");
    for (name, set) in [("state_db", &*utxo), ("in-memory", &mem)] {
        let still: Vec<_> = own.iter().filter(|o| set.get(o).is_some()).collect();
        assert!(
            still.is_empty(),
            "{name} backend: accused bonds must be burned; still live {still:?}"
        );
    }
    assert_eq!(
        sorted_ops(&mem),
        sorted_ops(&utxo),
        "backends hold different outpoints"
    );
    assert_eq!(
        root_of(&r.node, &cs, &mem, &ps),
        root_of(&r.node, &cs, &utxo, &ps),
        "backends give different state roots after the burn"
    );
}
