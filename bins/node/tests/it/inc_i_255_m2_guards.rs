//! INC-I-255 M2 — slash guards: the rebuild-from-blocks burn mirror and the
//! undecodable-slash-data guards.
//!
// OUTPUT CONTRACT: fn Node::rollback_one_block(&mut self, RollbackAuthority) -> Result<RollbackOutcome>
//   O1 return; O2 UTXO set (outpoints, canonical digest, supply); O3 chain tip
// OUTPUT CONTRACT: fn Node::execute_reorg(&mut self, ReorgResult, Block) -> Result<()>
//   O1 return; O2 UTXO set (outpoints, canonical digest, supply); O3 chain tip
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O1 return (InvalidSlash message); O2 whole state incl. UTXO set and root (unchanged on refusal)
// OUTPUT CONTRACT: fn SlashBlockBudget::fits(&self, &Transaction, &ValidationContext) -> bool / accept(&mut self, ..)
//   O1 return; O2 receiver (accused taken)
// PATHS: P-RB rollback with no undo data | P-RG reorg with no undo data | P-BAD undecodable slash data
// MATRIX
//   rollback O1-O3 x P-RB -> Ok; UTXO equals forward apply; in-block bonds stay burned (inc_i_255_m2_guard_rollback_rebuild_*)
//   execute_reorg O1-O3 x P-RG -> Ok; same (inc_i_255_m2_guard_reorg_rebuild_*)
//   apply_block O1,O2 x P-BAD x Full/Light/Replay -> InvalidSlash, nothing burned (inc_i_255_m2_guard_undecodable_slash_block_*)
//   fits/accept O1,O2 x P-BAD -> never fits, takes nothing (inc_i_255_m2_guard_budget_*)
// INPUT PARTITIONS:
//   accused bonds: created in a block (AddBond) | seeded; slash data: decodable | undecodable
//   fallback: rollback depth 1 | reorg depth 1

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{Input, Output, Transaction};
use doli_core::validation::{SlashBlockBudget, ValidationContext};
use doli_core::Network;
use network::sync::ReorgResult;
use storage::{Outpoint, UtxoEntry};

use super::inc_i_255_m1_common::{apply_slash_msg, canonical, chain_genesis, pair, MODES, SLOT};
use super::inc_i_255_m2_common::*;

const BONDS: u32 = 2;
const FUND_TAG: u8 = 0xF0;

fn funding() -> Outpoint {
    Outpoint::new(Hash::from_bytes([FUND_TAG; 32]), 0)
}

fn add_bond(r: &Rig) -> Transaction {
    let pk = r.accused_pk();
    let mut inp = Input::new(funding().tx_hash, funding().index);
    inp.public_key = Some(pk);
    let unit = r.unit();
    let mut tx = Transaction::new_add_bond(vec![inp], pk, BONDS, unit * BONDS as u64, u64::MAX);
    let msg = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&msg, r.keys.accused.private_key());
    producer_auth(tx, &r.keys.accused)
}

/// Every UTXO comes from a block except the AddBond funding coin, which block 1
/// spends. Block 1 creates the accused's only Bond UTXOs; block 2 slashes it.
async fn in_block_bonds() -> (Rig, Vec<Outpoint>) {
    let mut r = rig_with(&Keys::new(), 0).await;
    let unit = r.unit();
    {
        let mut utxo = r.node.utxo_set.write().await;
        utxo.clear().expect("FIXTURE: empty UTXO set");
        let coin = Output::normal(unit * BONDS as u64 + unit / 100, addr(&r.accused_pk()));
        let entry = UtxoEntry {
            output: coin,
            height: 0,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        utxo.insert(funding(), entry).expect("FIXTURE: fund");
    }
    let tx = add_bond(&r);
    r.step(vec![tx.clone()]).await;
    let mut bonds: Vec<Outpoint> = (0..BONDS).map(|i| Outpoint::new(tx.hash(), i)).collect();
    bonds.sort_by_key(|o| o.to_bytes());
    let mut held = bond_outpoints_of(&r.node, &r.accused_pk()).await;
    held.sort_by_key(|o| o.to_bytes());
    assert_eq!(
        held, bonds,
        "PRECONDITION: block 1 creates the accused's only bonds"
    );
    r.step(vec![r.slash()]).await;
    assert!(
        live(&r.node, &bonds).await.is_empty(),
        "PRECONDITION: the slash block burns them"
    );
    (r, bonds)
}

fn no_undo_above(r: &Rig, height: u64) {
    r.node.state_db.prune_undo_above(0);
    assert!(
        r.node.state_db.get_undo(height + 1).is_none(),
        "PRECONDITION: no undo data, so the UTXO set is rebuilt from blocks"
    );
}

fn assert_same_utxo(got: &Snap, want: &Snap, what: &str) {
    assert_eq!(
        (got.height, got.tip),
        (want.height, want.tip),
        "{what}: chain tip differs"
    );
    assert_eq!(got.ops, want.ops, "{what}: UTXO outpoints differ");
    assert_eq!(got.supply, want.supply, "{what}: supply differs");
    assert_eq!(
        got.utxo_digest, want.utxo_digest,
        "{what}: UTXO entries differ"
    );
}

// REQ-SLASH-003 — Decision: a failure means a node that rolls back without undo data resurrects bonds a mined slash burned, a state-root split.
#[tokio::test]
async fn inc_i_255_m2_guard_rollback_rebuild_burns_in_block_bonds() {
    let (mut r, bonds) = in_block_bonds().await;
    let want = snap(&r.node).await;
    r.step(Vec::new()).await;
    no_undo_above(&r, want.height);

    r.rollback().await;

    assert!(
        live(&r.node, &bonds).await.is_empty(),
        "the rebuild resurrected bonds the slash block burned"
    );
    assert_same_utxo(
        &snap(&r.node).await,
        &want,
        "rollback rebuild-from-blocks vs forward apply",
    );
    r.rollback().await;
    assert_eq!(
        live(&r.node, &bonds).await,
        bonds,
        "NON-VACUITY: a rebuild that stops before the slash block re-creates the bonds"
    );
}

// REQ-SLASH-003 — Decision: a failure means a reorg without undo data resurrects bonds a mined slash burned, a state-root split.
#[tokio::test]
async fn inc_i_255_m2_guard_reorg_rebuild_burns_in_block_bonds() {
    let (mut r, bonds) = in_block_bonds().await;
    let want = snap(&r.node).await;
    let orphan = r.step(Vec::new()).await;
    no_undo_above(&r, want.height);
    let reorg = ReorgResult {
        rollback: vec![orphan.hash()],
        common_ancestor: want.tip,
        new_blocks: Vec::new(),
        weight_delta: 1,
    };

    r.node
        .execute_reorg(reorg, orphan)
        .await
        .unwrap_or_else(|e| panic!("reorg rebuild-from-blocks failed: {e:#}"));

    assert!(
        live(&r.node, &bonds).await.is_empty(),
        "the reorg rebuild resurrected bonds the slash block burned"
    );
    assert_same_utxo(
        &snap(&r.node).await,
        &want,
        "reorg rebuild-from-blocks vs forward apply",
    );
    let slash_block = r.node.block_store.get_block_by_height(want.height);
    let slash_block = slash_block
        .expect("store read")
        .expect("slash block stored");
    let reorg = ReorgResult {
        rollback: vec![slash_block.hash()],
        common_ancestor: slash_block.header.prev_hash,
        new_blocks: Vec::new(),
        weight_delta: 1,
    };
    r.node
        .execute_reorg(reorg, slash_block)
        .await
        .unwrap_or_else(|e| panic!("second reorg failed: {e:#}"));
    assert_eq!(
        live(&r.node, &bonds).await,
        bonds,
        "NON-VACUITY: a rebuild that stops before the slash block re-creates the bonds"
    );
}

// REQ-SLASH-002 — Decision: a failure means a block whose slash data does not decode is applied or burns bonds instead of being refused at validation.
#[tokio::test]
async fn inc_i_255_m2_guard_undecodable_slash_block_refused_before_burn() {
    let mut r = rig(&Keys::new()).await;
    let pk = r.accused_pk();
    let mut bad = r.slash();
    bad.extra_data = vec![0xFF; 4];
    assert!(
        bad.slash_data().is_none(),
        "FIXTURE: undecodable slash data"
    );
    let before = snap(&r.node).await;

    for mode in MODES {
        let block = r.next_block(vec![bad.clone()], 0).await;
        let res = r.node.apply_block(block, mode).await;
        assert!(
            apply_slash_msg(&res).is_some_and(|m| m.contains("invalid slash data")),
            "{mode:?}: an undecodable slash must be refused at validation; got {res:?}"
        );
        assert_same_root(&snap(&r.node).await, &before, "refused block");
    }

    assert_eq!(
        bond_outpoints_of(&r.node, &pk).await.len(),
        ACCUSED_BONDS as usize
    );
    r.step(vec![r.slash()]).await;
    assert!(
        bond_outpoints_of(&r.node, &pk).await.is_empty(),
        "NON-VACUITY: the decodable slash in the same position burns the bonds"
    );
}

// REQ-SLASH-004 — Decision: a failure means the builder picks an undecodable slash or lets it consume a cap slot.
#[test]
fn inc_i_255_m2_guard_budget_never_fits_undecodable_slash() {
    let p = ConsensusParams::devnet();
    let ts = p.slot_to_timestamp(SLOT);
    let ctx = ValidationContext::new(p, Network::Devnet, ts, 10)
        .with_inc_i_236_block_signature_activation_height(0)
        .with_inc_i_255_slashing_activation_height(0);
    let kp = KeyPair::generate();
    let (a, b) = pair(&kp, chain_genesis());
    let good = canonical(*kp.public_key(), a, b);
    let mut bad = good.clone();
    bad.extra_data = vec![0xFF; 4];
    let mut budget = SlashBlockBudget::default();

    assert!(
        !budget.fits(&bad, &ctx),
        "an undecodable slash must never fit"
    );
    budget.accept(&bad, &ctx);
    assert!(
        budget.fits(&good, &ctx),
        "accepting an undecodable slash must take no accused"
    );
    budget.accept(&good, &ctx);
    assert!(
        !budget.fits(&good, &ctx),
        "NON-VACUITY: an accepted decodable slash takes its accused"
    );
}
