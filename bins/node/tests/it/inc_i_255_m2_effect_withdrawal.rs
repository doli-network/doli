//! INC-I-255 M2 — a Slashed or slash-queued producer cannot withdraw bonds:
//! refused by block economics and by admission; a resident withdrawal whose
//! Bond inputs a slash burned is evicted after the slash block.
//!
// OUTPUT CONTRACT: fn Node::validate_block_economics(&self, &Block, u64, ValidationMode) -> Result<()>
//   O1 return
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64, u32) -> Result<.., MempoolError>
//   O1 return; O2 residency
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()> (prunes and revalidates the mempool)
//   O1 mempool residency of withdrawals after the slash block
// PATHS: P-ACTIVE control producer | P-SLASHED status Slashed, bonds still live | P-QUEUED Slash pending, bonds still live
//        | P-RESIDENT withdrawal admitted before a slash block burns its inputs
// MATRIX
//   validate_block_economics O1 x {P-ACTIVE Ok, P-SLASHED Err, P-QUEUED Err} (inc_i_255_m2_probe_withdrawal, inc_i_255_m2_withdrawal_block_*)
//   add_transaction O1,O2 x {P-ACTIVE Ok, P-SLASHED Err, P-QUEUED Err}          (inc_i_255_m2_withdrawal_admission_*)
//   apply_block O1 x P-RESIDENT -> accused withdrawal evicted, control withdrawal resident (inc_i_255_m2_withdrawal_resident_*)
// INPUT PARTITIONS:
//   producer state: Active | Slashed | Slash queued; height: above every devnet gate | genesis tip (resident case)

use crypto::{KeyPair, PublicKey};
use doli_core::transaction::{Input, Transaction};
use doli_core::validation::ValidationMode;
use doli_node::node::Node;
use storage::{PendingProducerUpdate, ProducerSet};

use super::inc_i_180_common::{
    block_with, bond_unit, build_ledger, make_node, seed_bond_utxos, seed_owned_bond_utxos,
    withdrawal_tx, POST_AH,
};
use super::inc_i_255_m2_common::*;

const HELD: u32 = 2;
const WD_TAG: u8 = 0x5D;

#[derive(Debug, Clone, Copy)]
enum State {
    Active,
    Slashed,
    Queued,
}

fn ledger(node: &Node, pk: &PublicKey, state: State) -> ProducerSet {
    let mut ps = build_ledger(node, pk, HELD, 0);
    match state {
        State::Active => {}
        State::Slashed => {
            ps.slash_producer(pk, POST_AH - 10).expect("slash");
        }
        State::Queued => ps.queue_update(PendingProducerUpdate::Slash {
            pubkey: *pk,
            height: POST_AH - 1,
        }),
    }
    ps
}

fn signed_withdrawal(unit: u64, kp: &KeyPair, ops: &[storage::Outpoint]) -> Transaction {
    let inputs: Vec<Input> = ops
        .iter()
        .map(|o| {
            let mut inp = Input::new(o.tx_hash, o.index);
            inp.public_key = Some(*kp.public_key());
            inp
        })
        .collect();
    let n = ops.len() as u32;
    let dest = crypto::hash::hash(b"inc-i-255-m2-withdrawal-destination");
    let net = unit * u64::from(n) - unit / 100;
    let mut tx = run_b_withdrawal(inputs, *kp.public_key(), n, dest, net);
    for i in 0..tx.inputs.len() {
        let msg = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&msg, kp.private_key());
    }
    tx
}

async fn block_verdicts() -> Vec<(State, Result<(), String>)> {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();
    let wd = withdrawal_tx(&pk, HELD, WD_TAG);
    seed_bond_utxos(&node, &wd, &pk).await;
    let block = block_with(&node, POST_AH, pk, vec![wd]);
    let mut out = Vec::new();
    for state in [State::Active, State::Slashed, State::Queued] {
        *node.producer_set.write().await = ledger(&node, &pk, state);
        let r = node
            .validate_block_economics(&block, POST_AH, ValidationMode::Light)
            .await
            .map_err(|e| format!("{e:#}"));
        out.push((state, r));
    }
    out
}

async fn refused_by_block_economics() {
    let v = block_verdicts().await;
    assert!(
        v[0].1.is_ok(),
        "NON-VACUITY: an Active producer's withdrawal passes; got {:?}",
        v[0].1
    );
    for (state, r) in &v[1..] {
        assert!(
            r.is_err(),
            "block economics must refuse a withdrawal by a {state:?} producer whose bonds are still live"
        );
    }
}

// REQ-SLASH-003 — Decision: a failure means a slashed or slash-queued producer can still withdraw its bond in a block, escaping the burn.
#[tokio::test]
async fn inc_i_255_m2_probe_withdrawal() {
    refused_by_block_economics().await;
}

// REQ-SLASH-003 — Decision: a failure means admission keeps a withdrawal that block economics refuses, so the builder retries a dead tx every slot.
#[tokio::test]
async fn inc_i_255_m2_withdrawal_admission_refuses_slashed_and_queued() {
    let mut verdicts = Vec::new();
    for state in [State::Active, State::Slashed, State::Queued] {
        let (node, kp, _t) = make_node().await;
        let pk = *kp.public_key();
        *node.producer_set.write().await = ledger(&node, &pk, state);
        node.refresh_mempool_producer_snapshot(POST_AH - 1).await;
        seed_owned_bond_utxos(&node, &pk, WD_TAG, HELD).await;
        let tx = signed_withdrawal(bond_unit(&node), &kp, &outpoints(WD_TAG, HELD));
        let r = {
            let utxo = node.utxo_set.read().await;
            node.mempool
                .write()
                .await
                .add_transaction(tx.clone(), &utxo, POST_AH, 1)
                .map(|_| ())
                .map_err(|e| e.to_string())
        };
        let resident = node.mempool.read().await.contains(&tx.hash());
        verdicts.push((state, r, resident));
    }
    assert!(
        verdicts[0].1.is_ok() && verdicts[0].2,
        "NON-VACUITY: an Active producer's withdrawal is admitted; got {:?}",
        verdicts[0].1
    );
    for (state, r, resident) in &verdicts[1..] {
        assert!(
            r.is_err() && !resident,
            "admission must refuse a withdrawal by a {state:?} producer; got {r:?}"
        );
    }
}

// REQ-SLASH-003 — Decision: a failure means a withdrawal spending burned Bond UTXOs stays resident and is offered to the builder forever.
#[tokio::test]
async fn inc_i_255_m2_withdrawal_resident_with_burned_inputs_is_evicted() {
    let mut r = rig(&Keys::new()).await;
    let unit = r.unit();
    let accused_wd = signed_withdrawal(
        unit,
        &r.keys.accused,
        &outpoints(ACCUSED_TAG, ACCUSED_BONDS),
    );
    let control_wd = signed_withdrawal(unit, &r.keys.honest, &outpoints(HONEST_TAG, HONEST_BONDS));
    let (h, s, _) = r.best().await;
    for tx in [&accused_wd, &control_wd] {
        let res = {
            let utxo = r.node.utxo_set.read().await;
            r.node
                .mempool
                .write()
                .await
                .add_transaction(tx.clone(), &utxo, h, s)
                .map(|_| ())
        };
        assert!(
            res.is_ok(),
            "PRECONDITION: withdrawal admitted; got {res:?}"
        );
    }
    r.step(vec![r.slash()]).await;
    let pool = r.node.mempool.read().await;
    assert!(
        pool.contains(&control_wd.hash()),
        "NON-VACUITY: an unrelated withdrawal stays resident"
    );
    assert!(
        !pool.contains(&accused_wd.hash()),
        "a resident withdrawal whose Bond inputs the slash burned must be evicted"
    );
}
