//! INC-I-237 M0 / REQ-GRV-SEC-005 (C7 a) — decoded tx batch memory vs wire bytes.
//! The pre-allocation bound (C7 b) needs a counting allocator: see the dedicated
//! binary `tests/inc_i_237_m0_sec005_alloc.rs`.
//!
//! OUTPUT CONTRACT: fn decode_tx_gossip(data) -> Option<TxGossipMessage>
//!   O1: none   O2: none
//!   O3: FullBatch(Vec<Transaction>) — decoded in-memory bytes =
//!       capacity x size_of::<Transaction>() + per-tx heap
//!   O4: none   O5: size_of::<Transaction>() (pinned layout)   O6: none
//! PATHS: P1 count at the 10_000 cap, P2 count over the cap (None)
//! INPUT PARTITIONS: P1a 10_000 minimum-size txs (worst bytes-per-wire-byte,
//!   cannot fill GOSSIP_MAX_TRANSMIT_SIZE under the count cap), P1b 10_000 padded
//!   txs that fill GOSSIP_MAX_TRANSMIT_SIZE, P2a count 10_001
//! MATRIX: O3 x {P1a, P1b, P2a} + O5 — all asserted.

use std::mem::size_of;

use doli_core::transaction::{Input, Output};
use doli_core::Transaction;
use network::gossip::{decode_tx_gossip, TxGossipMessage, GOSSIP_MAX_TRANSMIT_SIZE};

/// Measured on v6.43.3, aarch64-apple-darwin.
const SIZE_OF_TRANSACTION: usize = 80;
/// C7 (a) RECORDED value (M2 input): 10_000 x 80 / 360_005 = 2.2222.
/// It cannot be <= 2 without changing which inputs decode (a 32-byte tx is valid
/// bincode), so it is pinned, not bounded.
const MIN_TX_BATCH_MEM: usize = 800_000;
const MIN_TX_BATCH_WIRE: usize = 360_005;

fn batch(count: u32, body: &[u8]) -> Vec<u8> {
    let mut b = vec![0xBA];
    b.extend_from_slice(&count.to_le_bytes());
    for _ in 0..count {
        b.extend_from_slice(&(body.len() as u32).to_le_bytes());
        b.extend_from_slice(body);
    }
    b
}

fn min_tx_body() -> Vec<u8> {
    let t = Transaction {
        version: 1,
        tx_type: doli_core::TxType::Transfer,
        inputs: vec![],
        outputs: vec![],
        extra_data: vec![],
    };
    t.serialize()
}

fn heap(t: &Transaction) -> usize {
    t.inputs.capacity() * size_of::<Input>()
        + t.outputs.capacity() * size_of::<Output>()
        + t.outputs
            .iter()
            .map(|o| o.extra_data.capacity())
            .sum::<usize>()
        + t.extra_data.capacity()
}

fn decoded_mem(wire: &[u8]) -> (usize, usize) {
    match decode_tx_gossip(wire) {
        Some(TxGossipMessage::FullBatch(txs)) => {
            let mem =
                txs.capacity() * size_of::<Transaction>() + txs.iter().map(heap).sum::<usize>();
            (txs.len(), mem)
        }
        _ => panic!("batch must decode"),
    }
}

// REQ-GRV-SEC-005 — Decision: a failure means Transaction's in-memory layout changed, so every recorded decode-amplification ratio (and the M2 memory budget built on it) is stale.
#[test]
fn req_grv_sec_005_size_of_transaction_is_pinned() {
    assert_eq!(
        size_of::<Transaction>(),
        SIZE_OF_TRANSACTION,
        "re-measure C7 and update the M2 budget"
    );
    assert_eq!(
        min_tx_body().len(),
        32,
        "minimum legacy tx is 32 wire bytes"
    );
}

// REQ-GRV-SEC-005 — Decision: a failure means the worst-case decode amplification moved (layout, count cap or min tx size changed), so the recorded M2 input is wrong.
#[test]
fn req_grv_sec_005_min_size_batch_amplification_is_recorded() {
    let wire = batch(10_000, &min_tx_body());
    assert!(wire.len() <= GOSSIP_MAX_TRANSMIT_SIZE);
    let (n, mem) = decoded_mem(&wire);
    eprintln!(
        "C7(a) min-tx: n={n} mem={mem} wire={} ratio={:.4}",
        wire.len(),
        mem as f64 / wire.len() as f64
    );
    assert_eq!(n, 10_000);
    assert_eq!(
        (mem, wire.len()),
        (MIN_TX_BATCH_MEM, MIN_TX_BATCH_WIRE),
        "C7(a) recorded ratio moved"
    );
}

// REQ-GRV-SEC-005 — Decision: a failure means a batch that fills max_transmit_size decodes to more than 2x its wire size, so one gossip message can amplify memory past the SEC-005 bound.
#[test]
fn req_grv_sec_005_full_size_batch_decodes_within_two_x_wire() {
    let per_tx = (GOSSIP_MAX_TRANSMIT_SIZE - 5) / 10_000 - 4;
    let pad = per_tx - 32;
    let t = Transaction {
        version: 1,
        tx_type: doli_core::TxType::Transfer,
        inputs: vec![],
        outputs: vec![],
        extra_data: vec![0x42; pad],
    };
    let body = t.serialize();
    assert_eq!(body.len(), per_tx);
    let wire = batch(10_000, &body);
    assert!(
        wire.len() <= GOSSIP_MAX_TRANSMIT_SIZE,
        "must fit one gossip message"
    );
    assert!(
        GOSSIP_MAX_TRANSMIT_SIZE - wire.len() < 10_000,
        "must fill the cap to one byte per tx"
    );
    let (n, mem) = decoded_mem(&wire);
    eprintln!(
        "C7(a) full: n={n} mem={mem} wire={} ratio={:.4}",
        wire.len(),
        mem as f64 / wire.len() as f64
    );
    assert_eq!(n, 10_000);
    assert!(
        mem <= 2 * wire.len(),
        "SEC-005: {mem} decoded bytes > 2 x {} wire bytes",
        wire.len()
    );
}

// REQ-GRV-SEC-005 — Decision: a failure means the 10_000 count cap is gone, so a single message can decode more txs than any batch the swarm ever sends.
#[test]
fn req_grv_sec_005_count_over_cap_is_rejected_before_decode() {
    assert!(decode_tx_gossip(&batch(10_001, &min_tx_body())).is_none());
    let mut zero = vec![0xBA];
    zero.extend_from_slice(&0u32.to_le_bytes());
    assert!(decode_tx_gossip(&zero).is_none(), "count 0 must not decode");
}
