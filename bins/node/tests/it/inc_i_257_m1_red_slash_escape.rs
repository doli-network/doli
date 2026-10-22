//! INC-I-257 M1 — RED until M2: a producer that double-produced, then
//! withdrew its full bond, loses the withdrawn value to a later valid slash.
//!
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()> over the chain
//   [withdrawal at N] .. [SlashProducer at N + k]
//   O1 return of the withdrawal block and of the slash block
//   O2 UTXO set total value (supply) net of coinbase minting
//   O3 native value owned by the accused (spendable + locked, minted entries excluded)
// PATHS: P-IN slash mined in the withdrawal's epoch (accused still Active)
//        P-AFTER slash mined after the epoch boundary (full withdrawal applied, accused Exited)
// MATRIX
//   O1 withdrawal mined; O2 supply drop == full bond amount; O3 accused owned value drop == full bond amount
//     x {P-IN, P-AFTER}
// INPUT PARTITIONS:
//   slash distance k: inside the epoch | past the boundary
//   payout shape: gated (type byte 16, lock N + 60) once it decodes, else the production constructor

use storage::ProducerStatus;

use super::inc_i_255_m2_common::*;
use super::inc_i_255_m4_common::err_text;
use super::inc_i_257_m1_common::*;

#[allow(dead_code)]
#[derive(Debug)]
struct Seen {
    slash_height: u64,
    status_at_slash: Option<ProducerStatus>,
    slash_block: String,
    supply_drop: i128,
    owned_drop: i128,
    bond_amount: u64,
}

async fn escape_case(k: u64) -> Seen {
    let mut r = rig(&Keys::new()).await;
    r.advance_to(EPOCH).await;
    let pk = r.accused_pk();
    let bond_amount = u64::from(ACCUSED_BONDS) * r.unit();
    let before = snap(&r.node).await;
    let owned_before = owned_native_value(&r.node, &pk).await;
    let n = before.height + 1;
    let ops = outpoints(ACCUSED_TAG, ACCUSED_BONDS);
    let rw = current_withdrawal(
        &r.keys.accused,
        &ops,
        bond_amount - FEE,
        n + DEVNET_UNBONDING_PERIOD,
    );
    let block = r.next_block(vec![rw], 0).await;
    let mut minted = coinbase_outputs(&block).0;
    let mined = r.apply(block).await;
    assert!(
        mined.is_ok(),
        "PRECONDITION: the accused's full withdrawal is mined at N={n}; got {}",
        err_text(&mined)
    );
    while r.best().await.0 + 1 < n + k {
        minted += coinbase_outputs(&r.step(Vec::new()).await).0;
    }
    let status_at_slash = status(&r.node, &pk).await;
    let block = r.next_block(vec![r.slash()], 0).await;
    let slash_minted = coinbase_outputs(&block).0;
    let slashed = r.apply(block).await;
    if slashed.is_ok() {
        minted += slash_minted;
    }
    let after = snap(&r.node).await;
    let owned_after = owned_native_value(&r.node, &pk).await;
    Seen {
        slash_height: n + k,
        status_at_slash,
        slash_block: err_text(&slashed),
        supply_drop: i128::from(before.supply) + i128::from(minted) - i128::from(after.supply),
        owned_drop: i128::from(owned_before) - i128::from(owned_after),
        bond_amount,
    }
}

fn assert_burned(s: &Seen, case: &str) {
    let bond = i128::from(s.bond_amount);
    assert_eq!(
        s.supply_drop, bond,
        "{case}: supply (net of coinbase minting) must drop by the full bond amount; observed {s:#?}"
    );
    assert_eq!(
        s.owned_drop, bond,
        "{case}: the accused must keep none of the withdrawn value, spendable or locked; observed {s:#?}"
    );
}

// REQ-UNBOND-002 — Decision: a failure means a slash mined in the same epoch as the accused's full withdrawal burns nothing, so the offender keeps its bond.
#[tokio::test]
#[ignore = "needs the slash burn of locked unbonding outputs"]
async fn inc_i_257_m1_req_unbond_002_slash_in_epoch_burns_withdrawn_bond() {
    let s = escape_case(2).await;
    assert_eq!(
        s.status_at_slash,
        Some(ProducerStatus::Active),
        "FIXTURE: the accused is still Active inside the epoch; observed {s:#?}"
    );
    assert_burned(&s, "slash inside the epoch");
}

// REQ-UNBOND-002 — Decision: a failure means a slash mined after the boundary that exits the accused is refused or burns nothing, so the offender keeps its bond.
#[tokio::test]
#[ignore = "needs the slash burn of locked unbonding outputs"]
async fn inc_i_257_m1_req_unbond_002_slash_after_boundary_burns_withdrawn_bond() {
    let s = escape_case(EPOCH).await;
    assert_ne!(
        s.status_at_slash,
        Some(ProducerStatus::Active),
        "FIXTURE: the boundary applied the full withdrawal before the slash; observed {s:#?}"
    );
    assert_burned(&s, "slash after the boundary");
}
