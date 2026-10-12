//! INC-I-237 M1.5 (R3) — node side of the RPC broadcast reservation.
//!
//! OUTPUT CONTRACT: fn reserve_tx_broadcast(&mpsc::Sender<NetworkCommand>, &Mutex<TokenBucket>)
//!   -> Option<rpc::TxBroadcastPermit> / broadcast_vote_now(&cmd_tx, local, vote_data) -> bool
//!   O1: none   O2: none   O3: Option<permit> / bool   O4: none   O5: none
//!   O6: command channel (BroadcastTransaction / BroadcastVote, count + payload);
//!       local vote channel (parsed VoteMessage); slot occupancy (next reserve)
//! PATHS: P1 command channel full -> None / false; P2 closed -> None / false;
//!   P3 free -> Some, call sends once; P4 permit dropped unused -> slot freed;
//!   P5 vote free -> BroadcastVote + local delivery; P6 local channel full or
//!   absent -> still broadcast (true)
//! INPUT PARTITIONS: command channel {free, full, closed}; permit {invoked,
//!   dropped}; local {None, free, full}
//! MATRIX: O3 x P1-P6, O6 x P1-P6 — all asserted below.

use std::sync::Mutex;

use doli_core::Transaction;
use network::rate_limit::TokenBucket;
use network::NetworkCommand;
use tokio::sync::mpsc;

use super::rpc_broadcast::{broadcast_vote_now, reserve_tx_broadcast};
use crate::updater::{Vote, VoteMessage};

/// An origin budget that never binds, so these rows judge the command slot alone.
fn open_origin() -> Mutex<TokenBucket> {
    Mutex::new(TokenBucket::new(1 << 30, 0.0))
}

fn tx() -> Transaction {
    let target = *crypto::KeyPair::from_seed([0x37; 32]).public_key();
    Transaction::new_add_maintainer(target, Vec::new())
}

fn vote_bytes() -> Vec<u8> {
    let v = VoteMessage::new("6.44.0".into(), Vote::Approve, "ab".repeat(32));
    serde_json::to_vec(&v).expect("vote json")
}

fn full_channel() -> (mpsc::Sender<NetworkCommand>, mpsc::Receiver<NetworkCommand>) {
    let (tx, rx) = mpsc::channel(1);
    tx.try_send(NetworkCommand::Bootstrap)
        .expect("fill the only slot");
    (tx, rx)
}

fn only_bootstrap(rx: &mut mpsc::Receiver<NetworkCommand>) {
    assert!(matches!(rx.try_recv(), Ok(NetworkCommand::Bootstrap)));
    assert!(
        rx.try_recv().is_err(),
        "nothing was queued behind the full slot"
    );
}

// REQ-GRV-007 / F-15 — Decision: a failure means a full or closed command channel makes the RPC broadcast wait (or spawn) instead of answering Busy, so an RPC flood parks RPC workers or grows tasks without bound.
#[test]
fn req_grv_007_reserve_on_full_or_closed_command_channel_is_none() {
    let (cmd, mut rx) = full_channel();
    assert!(
        reserve_tx_broadcast(&cmd, &open_origin()).is_none(),
        "full -> Busy"
    );
    only_bootstrap(&mut rx);
    let (cmd, rx) = mpsc::channel::<NetworkCommand>(1);
    drop(rx);
    assert!(
        reserve_tx_broadcast(&cmd, &open_origin()).is_none(),
        "closed -> Busy"
    );
}

// REQ-GRV-007 / F-15 — Decision: a failure means a reserved permit broadcasts nothing, the wrong tx, or a second copy.
#[test]
fn req_grv_007_reserved_permit_sends_broadcast_transaction_exactly_once() {
    let (cmd, mut rx) = mpsc::channel(1);
    let permit = reserve_tx_broadcast(&cmd, &open_origin()).expect("free slot");
    let t = tx();
    permit(t.clone());
    match rx.try_recv() {
        Ok(NetworkCommand::BroadcastTransaction(got)) => assert_eq!(got.hash(), t.hash()),
        other => panic!("expected BroadcastTransaction, got {other:?}"),
    }
    assert!(rx.try_recv().is_err(), "exactly one command");
}

// REQ-GRV-007 / F-15 — Decision: a failure means a permit dropped on an admission error keeps its slot, so rejected txs leak capacity until every later submit is Busy.
#[test]
fn req_grv_007_dropped_unused_permit_frees_the_slot() {
    let (cmd, mut rx) = mpsc::channel(1);
    let held = reserve_tx_broadcast(&cmd, &open_origin()).expect("free slot");
    assert!(
        reserve_tx_broadcast(&cmd, &open_origin()).is_none(),
        "the permit holds the only slot"
    );
    drop(held);
    assert!(
        reserve_tx_broadcast(&cmd, &open_origin()).is_some(),
        "drop must release the slot"
    );
    assert!(rx.try_recv().is_err(), "an unused permit sends nothing");
}

// REQ-GRV-007 / F-15 — Decision: a failure means a vote is reported as broadcast while the command channel was full or closed, so the RPC tells the producer its vote left when it did not.
#[test]
fn req_grv_007_broadcast_vote_now_is_false_on_full_or_closed_channel() {
    let (cmd, mut rx) = full_channel();
    let (local, _local_rx) = mpsc::channel::<VoteMessage>(4);
    assert!(!broadcast_vote_now(&cmd, Some(&local), vote_bytes()));
    only_bootstrap(&mut rx);
    let (cmd, rx) = mpsc::channel::<NetworkCommand>(1);
    drop(rx);
    assert!(!broadcast_vote_now(&cmd, None, vote_bytes()));
}

// REQ-GRV-007 / F-15 — Decision: a failure means an accepted vote is not gossiped byte-for-byte, or the local update service never sees it (gossip does not echo to the publisher).
#[test]
fn req_grv_007_broadcast_vote_now_sends_and_delivers_locally() {
    let (cmd, mut rx) = mpsc::channel(1);
    let (local, mut local_rx) = mpsc::channel::<VoteMessage>(1);
    let bytes = vote_bytes();
    assert!(broadcast_vote_now(&cmd, Some(&local), bytes.clone()));
    match rx.try_recv() {
        Ok(NetworkCommand::BroadcastVote(got)) => assert_eq!(got, bytes),
        other => panic!("expected BroadcastVote, got {other:?}"),
    }
    let delivered = local_rx.try_recv().expect("local delivery");
    assert_eq!(delivered.version, "6.44.0");
}

// REQ-GRV-007 / F-15 — Decision: a failure means a full (or absent) local vote channel blocks or cancels the network broadcast of an accepted vote.
#[test]
fn req_grv_007_broadcast_vote_now_full_or_absent_local_still_broadcasts() {
    let (cmd, mut rx) = mpsc::channel(2);
    let (local, _local_rx) = mpsc::channel::<VoteMessage>(1);
    let filler = VoteMessage::new("0.0.0".into(), Vote::Veto, "cd".repeat(32));
    local.try_send(filler).expect("fill local");
    assert!(broadcast_vote_now(&cmd, Some(&local), vote_bytes()));
    assert!(broadcast_vote_now(&cmd, None, vote_bytes()));
    for _ in 0..2 {
        assert!(matches!(
            rx.try_recv(),
            Ok(NetworkCommand::BroadcastVote(_))
        ));
    }
}
