//! INC-I-257 M1 — rebuild-from-blocks parity: a rollback with no undo data
//! rebuilds the UTXO set from stored blocks and must equal forward apply, with
//! the Unbonding output live (same type and lock) before the unlock and spent
//! after it. Every UTXO comes from a block. Runs in a child process with a short
//! devnet unbonding period so the whole chain stays inside the genesis phase
//! (heights <= 40), where rebuild and forward apply of test blocks agree.
//!
// OUTPUT CONTRACT: fn Node::rollback_one_block(&mut self, RollbackAuthority) -> Result<RollbackOutcome> (no undo data)
//   O1 return (RolledBack); O2 UTXO outpoints, canonical digest, supply; O3 chain tip
// PATHS: P-LIVE rebuild target between the request and the unlock | P-SPENT rebuild target after the spend
// INPUT PARTITIONS:
//   payout at the target height: live (locked) | spent at lock_until
//   backend: production RocksDB (the node apply path writes only through the state database batch)
// MATRIX:
//   O1-O3 x P-LIVE -> inc_i_257_m1_req_unbond_005_rebuild_matches_apply_payout_live
//   O1-O3 x P-SPENT -> inc_i_257_m1_req_unbond_005_rebuild_matches_apply_payout_spent

use crypto::Hash;
use doli_core::transaction::{Input, Output, Transaction};
use storage::{Outpoint, UtxoEntry};

use super::inc_i_255_m2_common::{addr, live, rig_with, snap, Keys, Rig, Snap};
use super::inc_i_255_m4_common::err_text;
use super::inc_i_255_m6_common::transfer;
use super::inc_i_257_m1_common::{gated_withdrawal, FEE};
use super::inc_i_257_m1_fx::{period, require_gated_type};

const FUND_TAG: u8 = 0xF7;
const SHORT_PERIOD: u64 = 8;
const PERIOD_ENV: &str = "DOLI_UNBONDING_PERIOD";
const CHILD_ENV: &str = "INC_I_257_M1_REBUILD_CHILD";

/// Parent: re-exec `test` alone with the short period, return true. Child: return false.
fn spawned_child(test: &str) -> bool {
    if std::env::var_os(CHILD_ENV).is_some() {
        return false;
    }
    let out = std::process::Command::new(std::env::current_exe().expect("current exe"))
        .arg(format!("inc_i_257_m1_rebuild::{test}"))
        .args(["--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1")
        .env(PERIOD_ENV, SHORT_PERIOD.to_string())
        .output()
        .expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "{test}: child failed or did not run:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    true
}

fn funding() -> Outpoint {
    Outpoint::new(Hash::from_bytes([FUND_TAG; 32]), 0)
}

async fn mine(r: &mut Rig, txs: Vec<Transaction>, what: &str) {
    let block = r.next_block(txs, 0).await;
    let h = r.best().await.0 + 1;
    let res = r.apply(block).await;
    assert!(
        res.is_ok(),
        "PRECONDITION: {what} is mined at {h}; got {}",
        err_text(&res)
    );
}

struct Chain {
    r: Rig,
    payout: Outpoint,
    lock: u64,
    net: u64,
}

/// Block 1: AddBond funded by the only seeded coin. Block 2: gated withdrawal of that Bond.
async fn in_block_withdrawal() -> Chain {
    let mut r = rig_with(&Keys::new(), 0).await;
    assert_eq!(
        period(&r),
        SHORT_PERIOD,
        "PRECONDITION: the child loaded the short devnet period"
    );
    let unit = r.unit();
    let pk = *r.keys.honest.public_key();
    {
        let mut utxo = r.node.utxo_set.write().await;
        utxo.clear().expect("FIXTURE: empty UTXO set");
        let entry = UtxoEntry {
            output: Output::normal(unit + unit / 100, addr(&pk)),
            height: 0,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        utxo.insert(funding(), entry).expect("FIXTURE: fund");
    }
    let mut inp = Input::new(funding().tx_hash, funding().index);
    inp.public_key = Some(pk);
    let mut ab = Transaction::new_add_bond(vec![inp], pk, 1, unit, u64::MAX);
    let msg = ab.signing_message_for_input(0);
    ab.inputs[0].signature = crypto::signature::sign_hash(&msg, r.keys.honest.private_key());
    let ab = super::inc_i_255_m2_common::producer_auth(ab, &r.keys.honest);
    mine(&mut r, vec![ab.clone()], "the AddBond").await;
    let bond = Outpoint::new(ab.hash(), 0);
    let lock = r.best().await.0 + 1 + SHORT_PERIOD;
    let net = unit - FEE;
    let wd = gated_withdrawal(&r.keys.honest, &[bond], net, lock);
    mine(&mut r, vec![wd.clone()], "the gated withdrawal").await;
    Chain {
        r,
        payout: Outpoint::new(wd.hash(), 0),
        lock,
        net,
    }
}

/// Step once past `want`, drop all undo data, roll back: the UTXO set is rebuilt from blocks.
async fn rebuild_to(c: &mut Chain, want: &Snap) {
    c.r.step(Vec::new()).await;
    c.r.node.state_db.prune_undo_above(0);
    assert!(
        c.r.node.state_db.get_undo(want.height + 1).is_none(),
        "PRECONDITION: no undo data, so the UTXO set is rebuilt from blocks"
    );
    c.r.rollback().await;
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
        "{what}: UTXO entries differ (type or lock)"
    );
}

// REQ-UNBOND-005 / REQ-UNBOND-007 — Decision: a failure means a node that rebuilds from blocks drops the locked payout or rewrites its type or lock, a state-root split with nodes that applied live.
#[tokio::test]
async fn inc_i_257_m1_req_unbond_005_rebuild_matches_apply_payout_live() {
    if spawned_child("inc_i_257_m1_req_unbond_005_rebuild_matches_apply_payout_live") {
        return;
    }
    require_gated_type();
    let mut c = in_block_withdrawal().await;
    c.r.advance_to(5).await;
    assert!(
        c.lock > 6,
        "FIXTURE: the target height is before the unlock"
    );
    let want = snap(&c.r.node).await;
    assert!(
        want.ops.contains(&c.payout),
        "NON-VACUITY: the payout is live at the target"
    );
    rebuild_to(&mut c, &want).await;
    assert_same_utxo(
        &snap(&c.r.node).await,
        &want,
        "rebuild with the payout locked",
    );
}

// REQ-UNBOND-005 / REQ-UNBOND-007 — Decision: a failure means a rebuild resurrects a spent Unbonding output or drops the spend that consumed it.
#[tokio::test]
async fn inc_i_257_m1_req_unbond_005_rebuild_matches_apply_payout_spent() {
    if spawned_child("inc_i_257_m1_req_unbond_005_rebuild_matches_apply_payout_spent") {
        return;
    }
    require_gated_type();
    let mut c = in_block_withdrawal().await;
    c.r.advance_to(c.lock - 1).await;
    let spend = transfer(&c.r.keys.honest, c.payout, c.net, FEE);
    mine(&mut c.r, vec![spend.clone()], "the spend at lock_until").await;
    c.r.advance_to(c.lock + 1).await;
    let want = snap(&c.r.node).await;
    let change = Outpoint::new(spend.hash(), 0);
    assert!(
        !want.ops.contains(&c.payout) && want.ops.contains(&change),
        "NON-VACUITY: the payout is spent and the spend output is live at the target"
    );
    rebuild_to(&mut c, &want).await;
    assert_same_utxo(&snap(&c.r.node).await, &want, "rebuild after the spend");
    assert!(
        live(&c.r.node, &[c.payout]).await.is_empty(),
        "rebuild after the spend: the spent payout came back"
    );
}
