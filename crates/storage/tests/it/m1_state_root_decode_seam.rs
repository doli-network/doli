//! UTXO-scalability M1 [F1] — the verify/decode seam of `compute_state_root_from_bytes`.
//!
//! REQ-SCALE-001 — Decision: a failure reveals that collapsing the verify+decode seam
//! changed the root VALUE, not just how many times the bytes are decoded.
//! REQ-SCALE-006 — Decision: a failure reveals the canonical decode is not the exact
//! inverse of the canonical encode, breaking INV-SYNC-007 on every snap install.
//!
//! OUTPUT CONTRACT: fn compute_state_root_from_bytes(cs_bytes, utxo_bytes, ps_bytes)
//!                     -> Result<Hash, StorageError>
//!   Outputs: O1 the returned Hash | O2 the Result variant
//!   Paths:   P1 all three blobs decode | P2 the UTXO blob is truncated
//!   MATRIX:  P1 x O1 -> m1_decode_once_then_root_equals_root_from_bytes
//!            P1 x O1 -> m1_decoded_set_round_trips_to_the_same_canonical_bytes
//!            P2 x O2 -> m1_truncated_utxo_blob_is_an_error_not_a_silent_short_set
//! INPUT PARTITIONS: IP1 an empty UTXO set; IP2 a multi-entry set with mixed
//!   coinbase/reward flags and non-empty extra_data; IP3 the IP2 blob truncated.

use crypto::Hash;
use doli_core::transaction::{Output, OutputType};
use storage::{
    compute_state_root, compute_state_root_from_bytes, ChainState, Outpoint, ProducerSet,
    UtxoEntry, UtxoSet,
};

fn sample_utxo_set(n: usize) -> UtxoSet {
    let mut set = UtxoSet::new();
    for i in 0..n {
        let v = i as u64;
        let mut tx_hash = [0u8; 32];
        tx_hash[0..8].copy_from_slice(&v.to_le_bytes());
        let mut pkh = [0u8; 32];
        pkh[0..8].copy_from_slice(&(v ^ 0x00FF_00FF_00FF_00FF).to_le_bytes());
        let mut output = Output::normal(1_000 + v, Hash::from_bytes(pkh));
        if i % 3 == 0 {
            output.output_type = OutputType::Bond;
            output.lock_until = u64::MAX;
            output.extra_data = vec![(i % 251) as u8; i % 7];
        }
        set.insert(
            Outpoint::new(Hash::from_bytes(tx_hash), (i % 5) as u32),
            UtxoEntry {
                output,
                height: v,
                is_coinbase: i % 4 == 0,
                is_epoch_reward: i % 5 == 0,
            },
        )
        .expect("insert must succeed");
    }
    set
}

fn blobs(n: usize) -> (Vec<u8>, Vec<u8>, Vec<u8>, ChainState, ProducerSet) {
    let mut cs = ChainState::new(Hash::from_bytes([0x11; 32]));
    cs.best_height = 4_242;
    cs.best_hash = Hash::from_bytes([0x22; 32]);
    let ps = ProducerSet::new();
    let set = sample_utxo_set(n);
    (
        bincode::serialize(&cs).unwrap(),
        set.serialize_canonical(),
        bincode::serialize(&ps).unwrap(),
        cs,
        ps,
    )
}

/// REQ-SCALE-001 — Decision: a failure reveals the one-decode install would produce a
/// different root than the two-decode install it replaces.
#[test]
fn m1_decode_once_then_root_equals_root_from_bytes() {
    for n in [0usize, 1, 37] {
        let (cs_bytes, utxo_bytes, ps_bytes, cs, ps) = blobs(n);

        let from_bytes = compute_state_root_from_bytes(&cs_bytes, &utxo_bytes, &ps_bytes)
            .expect("root from bytes must compute");

        let decoded =
            UtxoSet::deserialize_canonical(&utxo_bytes).expect("canonical decode must succeed");
        let decode_once = compute_state_root(&cs, &decoded, &ps).expect("root must compute");

        assert_eq!(
            from_bytes, decode_once,
            "n={}: decoding the blob ONCE and hashing the decoded set must equal \
             compute_state_root_from_bytes over the same blob",
            n
        );
    }
}

/// REQ-SCALE-006 — Decision: a failure reveals the canonical encode/decode pair lost
/// a field, so a re-serialized installed set would not match the wire bytes.
#[test]
fn m1_decoded_set_round_trips_to_the_same_canonical_bytes() {
    for n in [0usize, 1, 37] {
        let (_, utxo_bytes, _, _, _) = blobs(n);
        let decoded =
            UtxoSet::deserialize_canonical(&utxo_bytes).expect("canonical decode must succeed");
        assert_eq!(
            decoded.serialize_canonical(),
            utxo_bytes,
            "n={}: canonical decode must be the exact inverse of canonical encode",
            n
        );
        assert_eq!(
            decoded.utxo_count(),
            n as u64,
            "n={}: the decoded set must carry every entry",
            n
        );
    }
}

/// REQ-SCALE-001 — Decision: a failure reveals a truncated transfer would be accepted
/// as a shorter-but-valid set, which the streaming install must not start doing.
#[test]
fn m1_truncated_utxo_blob_is_an_error_not_a_silent_short_set() {
    let (cs_bytes, utxo_bytes, ps_bytes, _, _) = blobs(37);
    let cut = utxo_bytes.len() - 20;
    let truncated = utxo_bytes[..cut].to_vec();

    assert!(
        UtxoSet::deserialize_canonical(&truncated).is_err(),
        "a truncated canonical blob must be rejected, never decoded into a short set"
    );
    assert!(
        compute_state_root_from_bytes(&cs_bytes, &truncated, &ps_bytes).is_err(),
        "a truncated canonical blob must not yield a state root"
    );
}
