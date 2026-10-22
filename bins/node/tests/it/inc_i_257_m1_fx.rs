//! INC-I-257 M1 node fixtures: twin Run B rigs (production RocksDB UTXO
//! backend) and an honest one-bond withdrawal whose payout shape is chosen by
//! the caller. The node apply path writes UTXO changes only through the state
//! database batch, so an in-memory UTXO set is not reachable at node level.
//!
// OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.

#![allow(dead_code)]

use doli_core::transaction::Transaction;
use storage::Outpoint;

use super::inc_i_255_m2_common::{assert_same_root, outpoints, rig, snap, Keys, Rig, HONEST_TAG};
use super::inc_i_255_m6_common::transfer;
use super::inc_i_257_m1_common::{gated_type_exists, gated_withdrawal, instant_withdrawal, FEE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Gated,
    Instant,
}

pub fn require_gated_type() {
    assert!(
        gated_type_exists(),
        "PRECONDITION: type byte 16 (Unbonding) must decode, so the gated withdrawal shape exists"
    );
}

/// The unbonding period the node itself loaded (devnet 60 unless overridden).
pub fn period(r: &Rig) -> u64 {
    r.node.config.network.params().unbonding_period
}

/// Two identical rigs at `height`.
pub async fn twins(height: u64) -> (Rig, Rig) {
    let keys = Keys::new();
    let (mut a, mut b) = (rig(&keys).await, rig(&keys).await);
    a.advance_to(height).await;
    b.advance_to(height).await;
    assert_same_root(
        &snap(&a.node).await,
        &snap(&b.node).await,
        "PRECONDITION: twins agree",
    );
    (a, b)
}

pub struct Wd {
    pub tx: Transaction,
    pub payout: Outpoint,
    pub bond: Outpoint,
    pub lock: u64,
    pub net: u64,
}

/// One honest bond withdrawn at the next height `n`; lock = n + period (the consensus floor).
pub async fn withdrawal_for_next(r: &Rig, shape: Shape) -> Wd {
    let n = r.best().await.0 + 1;
    let lock = n + period(r);
    let net = r.unit() - FEE;
    let bond = outpoints(HONEST_TAG, 1)[0];
    let tx = match shape {
        Shape::Gated => gated_withdrawal(&r.keys.honest, &[bond], net, lock),
        Shape::Instant => instant_withdrawal(&r.keys.honest, &[bond], net),
    };
    Wd {
        payout: Outpoint::new(tx.hash(), 0),
        tx,
        bond,
        lock,
        net,
    }
}

pub fn spend_of(r: &Rig, wd: &Wd) -> Transaction {
    transfer(&r.keys.honest, wd.payout, wd.net, FEE)
}
