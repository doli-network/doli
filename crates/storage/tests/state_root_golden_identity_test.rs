//! State-Root Lazy Tier-0 — M1 golden-identity regression lock (RUN 459).
//!
//! These tests LOCK the current `storage::compute_state_root` behavior so that
//! the M1 lazy+memoize change (add write-back on the ONE live `GetStateRoot`
//! handler) cannot silently alter the ROOT VALUE at any height. Root value is
//! required to be BYTE-IDENTICAL at every height (spec: "Proposed Architecture
//! (Definite + Recommended — Tier 0)" — "Formula, root value, wire format:
//! BYTE-IDENTICAL at every height").
//!
//! Requirements:
//!   REQ-SROOT-001/002 (Must) — the memoized handler's returned root VALUE
//!     equals legacy `storage::compute_state_root` for the same state at a given
//!     height, byte-identical. This file locks the legacy formula itself; the
//!     node-crate test (`bins/node/tests/state_root_memoize_m1.rs`) locks that
//!     the handler returns exactly this value.
//!
//! Placement note: these are regression-lock tests that MUST PASS NOW (Tier 0 is
//! behavior-neutral for the root value). They exercise ONLY `compute_state_root`
//! and the public canonical encoders — no implementation edits required.
//
// OUTPUT CONTRACT: fn compute_state_root(cs, utxo, ps) -> Result<Hash, StorageError>
// O1: return — Hash (the state root) on Ok
// PATHS (for the golden-identity lock):
//   P1: any (cs, utxo, ps) → root == H(H(cs_canon) || H(utxo_canon) || H(ps_canon))
//   P2: same (cs, utxo, ps) called repeatedly → identical root (determinism)
//   P3: producer insertion-order permuted, same members → identical root
//   P4: any single component mutated → different root (sensitivity)
// MATRIX: O1×P1 = formula-lock, O1×P2 = determinism, O1×P3 = order-independence,
//         O1×P4 = per-component sensitivity.
//
// INPUT PARTITIONS:
//   P1 (formula): populated multi-producer state, advanced height → one class
//        (formula must hold for a representative non-trivial state).
//   P2 (determinism): identical inputs repeated N times → one class (repeat).
//   P3 (order-independence): {ascending-insert} vs {descending-insert} of the
//        SAME producer members → two input classes that MUST collapse to one root.
//   P4 (sensitivity): partitioned per mutated component —
//        (a) chain_state height h vs h+1, ps/utxo fixed → one class,
//        (b) producer set {empty} vs {one producer}, cs/utxo fixed → one class.
//        Each mutated component is a distinct partition (chain_state, producer_set);
//        the utxo component's order/byte parity is covered by dedicated encoder
//        tests in the storage crate and is intentionally out of scope here.

use crypto::{Hash, PublicKey};
use storage::chain_state::ChainState;
use storage::producer::ProducerSet;
use storage::utxo::UtxoSet;

const BOND_UNIT: u64 = 1_000_000_000;

/// Build a representative non-trivial state: a chain_state advanced past genesis
/// plus a populated producer set. Empty UTXO set is sufficient to lock the
/// three-component formula (the UTXO canonical encoder has its own dedicated
/// order-parity tests elsewhere).
fn sample_state() -> (ChainState, UtxoSet, ProducerSet) {
    let mut cs = ChainState::new(Hash::from_bytes([7u8; 32]));
    cs.best_hash = Hash::from_bytes([9u8; 32]);
    cs.best_height = 4242;

    let utxo = UtxoSet::new();

    let mut ps = ProducerSet::new();
    for i in 1u8..=4 {
        let pk = PublicKey::from_bytes([i; 32]);
        ps.register_genesis_producer(pk, 1, BOND_UNIT)
            .expect("register_genesis_producer");
    }

    (cs, utxo, ps)
}

/// Re-derive the state root using the documented formula directly from the
/// public canonical encoders. Locks that `compute_state_root` computes exactly
/// `H(H(cs_canon) || H(utxo_canon) || H(ps_canon))` — the byte-identity contract
/// M1 must preserve.
fn recompute_formula(cs: &ChainState, utxo: &UtxoSet, ps: &ProducerSet) -> Hash {
    let cs_hash = crypto::hash::hash(&cs.serialize_canonical());
    let utxo_hash = crypto::hash::hash(&utxo.serialize_canonical());
    let ps_hash = crypto::hash::hash(&ps.serialize_canonical());

    let mut combined = Vec::with_capacity(96);
    combined.extend_from_slice(cs_hash.as_bytes());
    combined.extend_from_slice(utxo_hash.as_bytes());
    combined.extend_from_slice(ps_hash.as_bytes());
    crypto::hash::hash(&combined)
}

/// REQ-SROOT-001/002 (Must) — P1 formula lock.
/// The canonical root equals the documented three-component BLAKE3 formula,
/// byte-for-byte. If M1 (or any refactor) changes the formula, this fails.
#[test]
fn test_compute_state_root_equals_documented_formula() {
    let (cs, utxo, ps) = sample_state();

    let root = storage::compute_state_root(&cs, &utxo, &ps).expect("compute_state_root");
    let expected = recompute_formula(&cs, &utxo, &ps);

    assert_eq!(
        root, expected,
        "state root must equal H(H(cs)||H(utxo)||H(ps)) byte-for-byte"
    );
}

/// REQ-SROOT-001/002 (Must) — P2 determinism / byte-stability.
/// Repeated calls on the SAME state return the identical root. This is the
/// property the memo relies on: computing lazily on-demand yields the same value
/// the eager path would have cached.
#[test]
fn test_compute_state_root_byte_stable_across_calls() {
    let (cs, utxo, ps) = sample_state();

    let r1 = storage::compute_state_root(&cs, &utxo, &ps).expect("r1");
    let r2 = storage::compute_state_root(&cs, &utxo, &ps).expect("r2");
    let r3 = storage::compute_state_root(&cs, &utxo, &ps).expect("r3");

    assert_eq!(r1, r2, "root must be deterministic across calls");
    assert_eq!(r2, r3, "root must be deterministic across calls");
    assert_eq!(
        r1.as_bytes(),
        r3.as_bytes(),
        "root bytes must be identical across calls"
    );
}

/// REQ-SROOT-001/002 (Must) — P3 producer insertion-order independence.
/// Two producer sets with the same members inserted in different orders yield
/// the same root (canonical sort). Locks that a lazy recompute after any
/// in-memory reordering still matches.
#[test]
fn test_compute_state_root_producer_order_independent() {
    let cs = ChainState::new(Hash::from_bytes([1u8; 32]));
    let utxo = UtxoSet::new();

    let pks: Vec<PublicKey> = (1u8..=4).map(|i| PublicKey::from_bytes([i; 32])).collect();

    let mut ps_a = ProducerSet::new();
    for pk in pks.iter() {
        ps_a.register_genesis_producer(*pk, 1, BOND_UNIT).unwrap();
    }

    let mut ps_b = ProducerSet::new();
    for pk in pks.iter().rev() {
        ps_b.register_genesis_producer(*pk, 1, BOND_UNIT).unwrap();
    }

    let root_a = storage::compute_state_root(&cs, &utxo, &ps_a).expect("root_a");
    let root_b = storage::compute_state_root(&cs, &utxo, &ps_b).expect("root_b");

    assert_eq!(
        root_a, root_b,
        "producer insertion order must not affect the state root"
    );
}

/// REQ-SROOT-001/002 (Must) — P4(a) per-component sensitivity (chain_state height).
/// A change in any single component must change the root — otherwise the golden
/// identity is meaningless (the memo could serve a wrong-height root undetected).
#[test]
fn test_compute_state_root_sensitive_to_chain_state_height() {
    let utxo = UtxoSet::new();
    let ps = ProducerSet::new();

    let cs_low = {
        let mut c = ChainState::new(Hash::ZERO);
        c.best_height = 100;
        c
    };
    let cs_high = {
        let mut c = ChainState::new(Hash::ZERO);
        c.best_height = 101;
        c
    };

    let root_low = storage::compute_state_root(&cs_low, &utxo, &ps).expect("low");
    let root_high = storage::compute_state_root(&cs_high, &utxo, &ps).expect("high");

    assert_ne!(
        root_low, root_high,
        "different chain_state height must produce a different root"
    );
}

/// REQ-SROOT-001/002 (Must) — P4(b) per-component sensitivity (producer set).
#[test]
fn test_compute_state_root_sensitive_to_producer_set() {
    let cs = ChainState::new(Hash::ZERO);
    let utxo = UtxoSet::new();

    let ps_empty = ProducerSet::new();
    let mut ps_one = ProducerSet::new();
    ps_one
        .register_genesis_producer(PublicKey::from_bytes([1u8; 32]), 1, BOND_UNIT)
        .unwrap();

    let root_empty = storage::compute_state_root(&cs, &utxo, &ps_empty).expect("empty");
    let root_one = storage::compute_state_root(&cs, &utxo, &ps_one).expect("one");

    assert_ne!(
        root_empty, root_one,
        "adding a producer must produce a different root"
    );
}
