//! INC-I-257 M1 — RED: the value a bond withdrawal pays out is not spendable
//! in the next block on any path; a gated payout unlocks at its lock height.
//!
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64, u32) -> Result<AddTransactionResult, MempoolError>
//   O1 return for the withdrawal and for the spend of its output 0; O2 residency
// OUTPUT CONTRACT: fn Node::build_block_content(&mut self, ..) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>
//   O1 transactions selected at heights N and N+1
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O1 return for the withdrawal block N and the spend block; O2 chain tip
// PATHS: P-MEMPOOL admission | P-BUILDER selection | P-APPLY Full apply (devnet: slashing and unbonding gates at 0)
//        P-LOCK gated payout spent at lock_until - 1 and at lock_until (mempool judged at tip + 1, Light apply)
// MATRIX
//   {P-MEMPOOL, P-BUILDER, P-APPLY} x (withdrawal accepted AND spend at N+1 accepted) -> false on all three
//     (headline; metric UNBOND_EARLY_SPEND_ACCEPTED_PATHS must be 0)
//   P-LOCK x {lock_until - 1 -> refused, lock_until -> accepted} x {mempool O1, apply O1}
// INPUT PARTITIONS:
//   payout shape: production constructor (instant) | gated (type byte 16, own address, empty extra_data, lock N + 60)
//   spend height: N + 1 | lock_until - 1 | lock_until
// Paths covered by M1: transaction types, core, output; network_params mod, defaults, env_loader,
//   ordering, params; validation unbonding, utxo, transaction, tx_types, output_authority, error,
//   context, builder; mempool pool; node tx_processing, assembly, validation_checks, slash_withdrawal;
//   rpc balance, block; cli serialize, parsers, cmd_wallet, wallet, cmd_producer withdrawal; lib, main;
//   cli unbonding_tx, parsers_tests; rpc chain; specs protocol, security_model; docs rpc_reference, cli.

use doli_core::validation::ValidationMode;
use storage::{Outpoint, ProducerSet};

use super::inc_i_255_m1_common::{full_block_with, tip, world, World};
use super::inc_i_255_m2_common::{outpoints, rig, Keys, EPOCH, HONEST_TAG};
use super::inc_i_255_m4_common::{admit_at, err_text, seed_bond};
use super::inc_i_255_m6_common::{block_with, transfer};
use super::inc_i_257_m1_common::*;

const P_TAG: u8 = 0x57;
const ACCUSED_LEDGER_BONDS: u32 = 2;

struct Fx {
    w: World,
    rw: doli_core::transaction::Transaction,
    t: doli_core::transaction::Transaction,
}

async fn fixture() -> Fx {
    let w = world().await;
    let unit = w.node.config.network.bond_unit();
    let mut ps = ProducerSet::new();
    ps.register_genesis_producer(*w.reporter.public_key(), 1, unit)
        .expect("FIXTURE: reporter ledger");
    ps.register_genesis_producer(w.accused_pk(), ACCUSED_LEDGER_BONDS, unit)
        .expect("FIXTURE: accused ledger");
    w.node.state_db.write_producer_set(&ps).expect("persist ps");
    *w.node.producer_set.write().await = ps;
    let op = seed_bond(&w.node, &w.accused_pk(), P_TAG).await;
    let net = unit - FEE;
    let rw = instant_withdrawal(&w.accused, &[op], net);
    let t = transfer(&w.accused, Outpoint::new(rw.hash(), 0), net, FEE);
    Fx { w, rw, t }
}

async fn mempool_path() -> (bool, String) {
    let mut f = fixture().await;
    let h = tip(&f.w.node).await.0;
    let wd = admit_at(&f.w.node, &f.rw, h).await;
    f.w.node
        .mempool
        .write()
        .await
        .remove_transaction(&f.rw.hash());
    let block = block_with(&mut f.w.node, std::slice::from_ref(&f.rw)).await;
    let mined = f.w.node.apply_block(block, ValidationMode::Full).await;
    let n = tip(&f.w.node).await.0;
    let spend = admit_at(&f.w.node, &f.t, n).await;
    let ok = wd.is_ok() && mined.is_ok() && spend.is_ok();
    let detail = format!(
        "withdrawal admission {wd:?}; block N {}; spend admission with tip {n}: {spend:?}",
        err_text(&mined)
    );
    (ok, detail)
}

async fn builder_path() -> (bool, String) {
    let mut f = fixture().await;
    let h = tip(&f.w.node).await.0;
    let wd = admit_at(&f.w.node, &f.rw, h).await;
    let (b1, sel1) = full_block_with(&mut f.w.node, &[]).await;
    let a1 = f.w.node.apply_block(b1, ValidationMode::Full).await;
    let n = tip(&f.w.node).await.0;
    let spend = admit_at(&f.w.node, &f.t, n).await;
    let (b2, sel2) = full_block_with(&mut f.w.node, &[]).await;
    let a2 = f.w.node.apply_block(b2, ValidationMode::Full).await;
    let (wd_sel, t_sel) = (sel1.contains(&f.rw.hash()), sel2.contains(&f.t.hash()));
    let ok = wd_sel && a1.is_ok() && t_sel && a2.is_ok();
    let detail = format!(
        "withdrawal admission {wd:?}, selected {wd_sel}, block N {}; spend admission {spend:?}, selected {t_sel}, block N+1 {}",
        err_text(&a1),
        err_text(&a2)
    );
    (ok, detail)
}

async fn apply_path() -> (bool, String) {
    let mut f = fixture().await;
    let b1 = block_with(&mut f.w.node, std::slice::from_ref(&f.rw)).await;
    let a1 = f.w.node.apply_block(b1, ValidationMode::Full).await;
    let b2 = block_with(&mut f.w.node, std::slice::from_ref(&f.t)).await;
    let a2 = f.w.node.apply_block(b2, ValidationMode::Full).await;
    let ok = a1.is_ok() && a2.is_ok();
    let detail = format!("block N {}; block N+1 {}", err_text(&a1), err_text(&a2));
    (ok, detail)
}

// REQ-UNBOND-001 — Decision: a failure means a producer can withdraw its bond and move the coins in the next block, before any slash for earlier misconduct can reach them.
#[tokio::test]
async fn inc_i_257_m1_req_unbond_001_withdrawn_value_not_spendable_next_block() {
    let seen = [
        ("mempool", mempool_path().await),
        ("builder", builder_path().await),
        ("apply", apply_path().await),
    ];
    let n = seen.iter().filter(|(_, (ok, _))| *ok).count();
    println!("UNBOND_EARLY_SPEND_ACCEPTED_PATHS={n}");
    assert_eq!(
        n, 0,
        "the withdrawn value must not be spendable at N+1 on any path; per path: {seen:#?}"
    );
}

// REQ-UNBOND-001 — Decision: a failure means the gated payout unlocks one block early or stays locked past its lock height.
#[tokio::test]
async fn inc_i_257_m1_req_unbond_001_lock_boundary() {
    let mut r = rig(&Keys::new()).await;
    r.advance_to(EPOCH).await;
    let n = r.best().await.0 + 1;
    let lock = n + DEVNET_UNBONDING_PERIOD;
    let net = r.unit() - FEE;
    let rw = gated_withdrawal(&r.keys.honest, &outpoints(HONEST_TAG, 1), net, lock);
    let block = r.next_block(vec![rw.clone()], 0).await;
    let mined = r.apply(block).await;
    assert!(
        mined.is_ok(),
        "PRECONDITION: the gated withdrawal (type {:?}, lock {lock}) is mined at N={n}; got {}",
        gated_payout_type(),
        err_text(&mined)
    );
    let payout = Outpoint::new(rw.hash(), 0);
    let t = transfer(&r.keys.honest, payout, net, FEE);
    r.advance_to(lock - 2).await;
    let early_pool = admit_at(&r.node, &t, lock - 2).await;
    r.node.mempool.write().await.remove_transaction(&t.hash());
    let early_block = r.next_block(vec![t.clone()], 0).await;
    let early_apply = r.apply(early_block).await;
    r.advance_to(lock - 1).await;
    let on_pool = admit_at(&r.node, &t, lock - 1).await;
    r.node.mempool.write().await.remove_transaction(&t.hash());
    let on_block = r.next_block(vec![t.clone()], 0).await;
    let on_apply = r.apply(on_block).await;
    assert!(
        early_pool.is_err() && early_apply.is_err(),
        "at height lock_until - 1 = {} the payout spend must be refused; mempool {early_pool:?}, apply {}",
        lock - 1,
        err_text(&early_apply)
    );
    assert!(
        on_pool.is_ok() && on_apply.is_ok(),
        "at height lock_until = {lock} the payout spend must be accepted; mempool {on_pool:?}, apply {}",
        err_text(&on_apply)
    );
}
