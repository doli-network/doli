//! INC-I-171 M3 — the ONE withdrawal-input resolver that replaces three copies of the
//! "scan tx.inputs, count Bond UTXOs" fold.
//!
//! ============================ OUTPUT CONTRACT ============================
//!
//! FN UNDER TEST: `resolve_withdrawal_inputs(&Transaction, &UtxoSet, &Hash) -> WithdrawalInputs`
//! Math-free, gate-free, INFALLIBLE: no `Result`, no panic, no clock, no randomness.
//!
//! OUTPUTS (full enumeration):
//!   O1 mutable params      — NONE. All three params are shared refs.
//!   O2 receiver mutation   — N/A, free function.
//!   O3 return value        — `WithdrawalInputs`, five fields, each asserted independently:
//!        R1 `owned_bonds: u32`                       R2 `all_bonds: u32`
//!        R3 `spent_bonds: Vec<(u64, Option<u32>)>`   R4 `non_bond_total: u64`
//!        R5 `malformed_bond_inputs: Vec<(usize, usize)>`
//!   O4 persistent store    — MUST be none. The RocksDb-backed `UtxoSet` is read-only here;
//!                            `rocksdb_store_is_untouched` snapshots content before and after.
//!   O5 global/static state — NONE.  O6 channels/events — NONE.
//!   O7 termination         — must return for every input; the malformed-`extra_data` and
//!                            u64-saturation partitions are the ones that could panic.
//!
//! PATHS (per input, folded in tx-input order):
//!   P1 outpoint ABSENT from the set            -> contributes to NOTHING
//!   P2 present, `output_type != Bond`          -> R4 += amount (saturating)
//!   P3 present, Bond, owner match, len == 4    -> R1+1, R2+1, R3 push (amount, Some(slot))
//!   P4 present, Bond, owner differs, len == 4  -> R2+1, R3 push (amount, Some(slot))
//!   P5 present, Bond, len != 4                 -> counts as P3/P4 but R3 pushes
//!                                                 (amount, None) and R5 pushes (index, len)
//!   P6 tx with zero inputs                     -> the all-zero result
//!
//! INPUT PARTITIONS: all-owned-Bond; owned + foreign Bond; Bond/non-Bond interleaved; absent
//! outpoints; zero inputs; all non-Bond; equal creation slot with unequal amounts; three
//! permutations of one input triple; `extra_data` lengths {0, 3, 4, 5, 8}; amounts summing
//! past `u64::MAX`; both `UtxoSet` backends.
//!
//! CONTRACT NOTE: `spent_bonds` and `malformed_bond_inputs` cover EVERY Bond input, owned or
//! foreign. Ownership is carried by `owned_bonds` alone.
//!
//! REQ-VEST-007 — Decision: reveals that the collapse into one resolver changed `owned_bonds` or
//! `all_bonds` for some UTXO shape, which below the activation height would fork the chain
//! against every node still running the three-copy binary.
//! REQ-VEST-004 — Decision: reveals that the resolver's verdict depends on the storage backend,
//! on RocksDB/HashMap enumeration order, or on how `extra_data` happens to be sized — any of
//! which makes two honest nodes disagree about the same transaction.

use std::sync::Arc;

use crypto::hash::hash as crypto_hash;
use crypto::Hash;
use doli_core::transaction::{Input, Output, OutputType, Transaction, TxType};
use storage::producer::{resolve_withdrawal_inputs, WithdrawalInputs};
use storage::{Outpoint, StateDb, UtxoEntry, UtxoSet};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Reference oracle
// ---------------------------------------------------------------------------

/// Verbatim copy of the INLINE bond scan at
/// `bins/node/src/node/validation_checks.rs:669-684` (the third, unnamed copy).
/// Its `owner` is an `Option<Hash>` there, compared with `owner == Some(..)`.
fn validation_checks_oracle(tx: &Transaction, utxo: &UtxoSet, owner: Option<Hash>) -> (u32, u32) {
    let (mut owned, mut all_bonds) = (0u32, 0u32);
    for inp in &tx.inputs {
        let Some(entry) = utxo.get(&storage::Outpoint::new(inp.prev_tx_hash, inp.output_index))
        else {
            continue;
        };
        if entry.output.output_type != doli_core::transaction::OutputType::Bond {
            continue;
        }
        all_bonds = all_bonds.saturating_add(1);
        if owner == Some(entry.output.pubkey_hash) {
            owned = owned.saturating_add(1);
        }
    }
    (owned, all_bonds)
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn owner_hash() -> Hash {
    crypto_hash(b"inc-i-171-m3-owner")
}

fn foreign_hash() -> Hash {
    crypto_hash(b"inc-i-171-m3-foreign-producer")
}

/// What the UTXO set holds for one tx input.
#[derive(Clone)]
enum Held {
    Absent,
    Bond {
        amount: u64,
        who: Hash,
        extra: Vec<u8>,
    },
    NonBond {
        amount: u64,
        ty: OutputType,
    },
}

fn bond(amount: u64, who: Hash, slot: u32) -> Held {
    Held::Bond {
        amount,
        who,
        extra: slot.to_le_bytes().to_vec(),
    }
}

fn bond_raw(amount: u64, who: Hash, extra: Vec<u8>) -> Held {
    Held::Bond { amount, who, extra }
}

fn normal(amount: u64) -> Held {
    Held::NonBond {
        amount,
        ty: OutputType::Normal,
    }
}

fn outpoint_of(slot_in_fixture: usize) -> Outpoint {
    Outpoint::new(
        crypto_hash(format!("inc-i-171-m3-prev-tx-{slot_in_fixture}").as_bytes()),
        (slot_in_fixture % 4) as u32,
    )
}

fn entry_of(held: &Held) -> Option<UtxoEntry> {
    let output = match held {
        Held::Absent => return None,
        Held::Bond { amount, who, extra } => Output {
            output_type: OutputType::Bond,
            amount: *amount,
            pubkey_hash: *who,
            lock_until: u64::MAX,
            extra_data: extra.clone(),
        },
        Held::NonBond { amount, ty } => Output {
            output_type: *ty,
            amount: *amount,
            pubkey_hash: owner_hash(),
            lock_until: 0,
            extra_data: Vec::new(),
        },
    };
    Some(UtxoEntry {
        output,
        height: 42,
        is_coinbase: false,
        is_epoch_reward: false,
    })
}

/// A transaction whose inputs reference `shape[i]`, in that order, plus the
/// (outpoint, entry) pairs the UTXO set must hold.
fn tx_for(shape: &[usize]) -> Transaction {
    Transaction {
        version: 1,
        tx_type: TxType::RequestWithdrawal,
        inputs: shape
            .iter()
            .map(|i| {
                let op = outpoint_of(*i);
                Input::new(op.tx_hash, op.index)
            })
            .collect(),
        outputs: Vec::new(),
        extra_data: Vec::new(),
    }
}

fn pairs_for(held: &[Held]) -> Vec<(Outpoint, UtxoEntry)> {
    held.iter()
        .enumerate()
        .filter_map(|(i, h)| entry_of(h).map(|e| (outpoint_of(i), e)))
        .collect()
}

fn in_memory_set(pairs: &[(Outpoint, UtxoEntry)]) -> UtxoSet {
    let mut set = UtxoSet::new();
    for (op, e) in pairs {
        set.insert(*op, e.clone())
            .expect("fixture: in-memory insert must succeed");
    }
    set
}

fn rocksdb_set(pairs: &[(Outpoint, UtxoEntry)]) -> (UtxoSet, TempDir) {
    let dir = TempDir::new().expect("fixture: TempDir");
    let db = StateDb::open(dir.path()).expect("fixture: StateDb::open");
    let mut set = UtxoSet::from_state_db(Arc::new(db));
    for (op, e) in pairs {
        set.insert(*op, e.clone())
            .expect("fixture: RocksDb insert must succeed");
    }
    (set, dir)
}

/// Backend-independent CONTENT snapshot of a UTXO set, sorted for determinism.
fn content(set: &UtxoSet) -> Vec<String> {
    let mut v: Vec<String> = set
        .iter_all()
        .into_iter()
        .map(|(op, e)| {
            format!(
                "{op:?}|{}|{:?}|{:?}",
                e.output.amount, e.output.output_type, e.output.extra_data
            )
        })
        .collect();
    v.sort();
    v
}

/// One permutation case: the tx input order, and the `spent_bonds` it must produce.
type Perm = (Vec<usize>, Vec<(u64, Option<u32>)>);

fn assert_same_result(a: &WithdrawalInputs, b: &WithdrawalInputs, what: &str) {
    assert_eq!(a.owned_bonds, b.owned_bonds, "{what}: owned_bonds diverged");
    assert_eq!(a.all_bonds, b.all_bonds, "{what}: all_bonds diverged");
    assert_eq!(a.spent_bonds, b.spent_bonds, "{what}: spent_bonds diverged");
    assert_eq!(
        a.non_bond_total, b.non_bond_total,
        "{what}: non_bond_total diverged"
    );
    assert_eq!(
        a.malformed_bond_inputs, b.malformed_bond_inputs,
        "{what}: malformed_bond_inputs diverged"
    );
}

// ---------------------------------------------------------------------------
// REQ-VEST-007 — byte-identical counts (the below-activation-height obligation)
// ---------------------------------------------------------------------------

#[test]
fn req_vest_007_counts_match_the_validation_checks_oracle_on_six_shapes() {
    let me = owner_hash();
    let them = foreign_hash();

    let shapes: Vec<(&str, Vec<Held>)> = vec![
        (
            "a: every input is a Bond owned by the withdrawer",
            vec![bond(100, me, 1), bond(200, me, 2), bond(300, me, 3)],
        ),
        (
            "b: owned Bonds mixed with Bonds of a DIFFERENT pubkey_hash",
            vec![
                bond(100, me, 1),
                bond(700, them, 1),
                bond(200, me, 2),
                bond(800, them, 2),
            ],
        ),
        (
            "c: Bond and Normal outputs interleaved",
            vec![
                normal(10),
                bond(100, me, 5),
                normal(20),
                bond(700, them, 5),
                normal(30),
            ],
        ),
        (
            "d: inputs whose Outpoint is ABSENT from the UTXO set",
            vec![
                bond(100, me, 4),
                Held::Absent,
                bond(700, them, 4),
                Held::Absent,
            ],
        ),
        ("e: a transaction with zero inputs", vec![]),
        (
            "f: every input resolves to a non-Bond output",
            vec![
                normal(10),
                Held::NonBond {
                    amount: 20,
                    ty: OutputType::Vesting,
                },
                normal(30),
            ],
        ),
    ];

    for (label, held) in &shapes {
        let indices: Vec<usize> = (0..held.len()).collect();
        let tx = tx_for(&indices);
        let pairs = pairs_for(held);
        let set = in_memory_set(&pairs);

        let (oracle_owned, oracle_all) = validation_checks_oracle(&tx, &set, Some(me));
        let got = resolve_withdrawal_inputs(&tx, &set, &me);

        assert_eq!(
            got.owned_bonds, oracle_owned,
            "REQ-VEST-007 [{label}]: owned_bonds must be byte-identical to the inline scan at \
             bins/node/src/node/validation_checks.rs:669-684; a difference below the activation \
             height forks against every node still running the three-copy binary"
        );
        assert_eq!(
            got.all_bonds, oracle_all,
            "REQ-VEST-007 [{label}]: all_bonds must be byte-identical to the inline scan at \
             bins/node/src/node/validation_checks.rs:669-684"
        );
    }
}

// ---------------------------------------------------------------------------
// REQ-VEST-004 / INV-VEST-003 — backend and order invariance
// ---------------------------------------------------------------------------

/// `i0`/`i1` share a creation slot and differ in amount, so any implementation that
/// enumerates storage instead of `tx.inputs` reorders `spent_bonds` visibly.
fn equal_slot_fixture() -> Vec<Held> {
    let me = owner_hash();
    vec![
        bond(500, me, 7),
        bond(900, me, 7),
        bond(100, foreign_hash(), 9),
        normal(250),
        Held::Absent,
    ]
}

#[test]
fn req_vest_004_full_result_is_identical_on_both_backends() {
    let me = owner_hash();
    let held = equal_slot_fixture();
    let tx = tx_for(&(0..held.len()).collect::<Vec<_>>());
    let pairs = pairs_for(&held);

    let mem = in_memory_set(&pairs);
    let (rocks, _dir) = rocksdb_set(&pairs);

    let before = content(&rocks);
    let from_mem = resolve_withdrawal_inputs(&tx, &mem, &me);
    let from_rocks = resolve_withdrawal_inputs(&tx, &rocks, &me);

    assert_same_result(
        &from_mem,
        &from_rocks,
        "REQ-VEST-004/INV-VEST-003: UtxoSet::InMemory and UtxoSet::RocksDb must produce the same \
         WithdrawalInputs for the same tx (equal creation slot, unequal amounts)",
    );

    assert_eq!(
        from_mem.owned_bonds, 2,
        "REQ-VEST-004: only the two Bonds whose pubkey_hash equals the owner are owned"
    );
    assert_eq!(
        from_mem.all_bonds, 3,
        "REQ-VEST-004: all_bonds counts the foreign Bond too"
    );
    assert_eq!(
        from_mem.spent_bonds,
        vec![(500u64, Some(7u32)), (900, Some(7)), (100, Some(9))],
        "REQ-VEST-004: spent_bonds carries EVERY Bond input (owned and foreign) in tx input \
         order, with the creation slot decoded from extra_data"
    );
    assert_eq!(
        from_mem.non_bond_total, 250,
        "REQ-VEST-004: non_bond_total sums non-Bond inputs only; Bond amounts and the absent \
         outpoint must not appear in it"
    );
    assert!(
        from_mem.malformed_bond_inputs.is_empty(),
        "REQ-VEST-004: every Bond in this fixture carries a 4-byte extra_data"
    );

    assert_eq!(
        content(&rocks),
        before,
        "REQ-VEST-004: the resolver is READ-ONLY; a write to the RocksDb-backed UtxoSet would \
         change the UTXO state root of a validating node"
    );
}

#[test]
fn req_vest_004_spent_bonds_follows_tx_input_order_on_both_backends() {
    let me = owner_hash();
    let held = vec![
        bond(500, me, 7),
        bond(900, me, 7),
        bond(100, foreign_hash(), 9),
    ];
    let pairs = pairs_for(&held);

    let mem = in_memory_set(&pairs);
    let (rocks, _dir) = rocksdb_set(&pairs);

    let permutations: [Perm; 3] = [
        (
            vec![0, 1, 2],
            vec![(500, Some(7)), (900, Some(7)), (100, Some(9))],
        ),
        (
            vec![2, 0, 1],
            vec![(100, Some(9)), (500, Some(7)), (900, Some(7))],
        ),
        (
            vec![1, 2, 0],
            vec![(900, Some(7)), (100, Some(9)), (500, Some(7))],
        ),
    ];

    for (order, expected) in &permutations {
        let tx = tx_for(order);
        for (backend, set) in [("InMemory", &mem), ("RocksDb", &rocks)] {
            let got = resolve_withdrawal_inputs(&tx, set, &me);
            assert_eq!(
                &got.spent_bonds, expected,
                "REQ-VEST-004/INV-VEST-003 [{backend}, inputs {order:?}]: spent_bonds must follow \
                 TX INPUT ORDER, never a storage enumeration order; the two 500/900 Bonds share a \
                 creation slot, so only the amounts expose the reordering"
            );
            assert_eq!(
                got.owned_bonds, 2,
                "REQ-VEST-004 [{backend}, inputs {order:?}]: permuting the inputs must not change \
                 owned_bonds"
            );
            assert_eq!(
                got.all_bonds, 3,
                "REQ-VEST-004 [{backend}, inputs {order:?}]: permuting the inputs must not change \
                 all_bonds"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// REQ-VEST-004 — creation-slot decode
// ---------------------------------------------------------------------------

#[test]
fn req_vest_004_creation_slot_decodes_little_endian() {
    let me = owner_hash();
    let held = vec![bond_raw(100, me, vec![0x01, 0x02, 0x03, 0x04])];
    let tx = tx_for(&[0]);
    let set = in_memory_set(&pairs_for(&held));

    let got = resolve_withdrawal_inputs(&tx, &set, &me);

    assert_eq!(
        got.spent_bonds,
        vec![(100u64, Some(0x0403_0201u32))],
        "REQ-VEST-004: extra_data is LITTLE-endian (Output::bond writes \
         creation_slot.to_le_bytes(), crates/core/src/transaction/output.rs:144); a big-endian \
         read would yield 0x01020304 and age every bond wrongly"
    );
    assert!(
        got.malformed_bond_inputs.is_empty(),
        "REQ-VEST-004: a 4-byte extra_data is well-formed"
    );
}

#[test]
fn req_vest_004_malformed_extra_data_is_recorded_without_changing_the_counts() {
    let me = owner_hash();
    let them = foreign_hash();
    let held = vec![
        bond(111, me, 11),
        bond_raw(222, me, Vec::new()),
        bond_raw(333, them, vec![0xAA, 0xBB, 0xCC]),
        bond_raw(444, me, vec![1, 2, 3, 4, 5]),
        bond_raw(555, me, vec![9; 8]),
    ];
    let tx = tx_for(&[0, 1, 2, 3, 4]);
    let set = in_memory_set(&pairs_for(&held));

    let got = resolve_withdrawal_inputs(&tx, &set, &me);

    assert_eq!(
        got.spent_bonds,
        vec![
            (111u64, Some(11u32)),
            (222, None),
            (333, None),
            (444, None),
            (555, None)
        ],
        "REQ-VEST-004: extra_data lengths 0, 3, 5 and 8 decode to None, and the resolver must \
         return rather than panic on any of them (the gated M4 check raises \
         WithdrawalBondExtraDataMalformed; the resolver itself is infallible)"
    );
    assert_eq!(
        got.malformed_bond_inputs,
        vec![(1usize, 0usize), (2, 3), (3, 5), (4, 8)],
        "REQ-VEST-004: each malformed Bond input records its own (input_index, extra_data.len()); \
         index 0 is well-formed here so a hard-coded or off-by-one index is visible"
    );

    let (oracle_owned, oracle_all) = validation_checks_oracle(&tx, &set, Some(me));
    assert_eq!(
        (got.owned_bonds, got.all_bonds),
        (oracle_owned, oracle_all),
        "REQ-VEST-007: malformed extra_data must NOT change owned_bonds/all_bonds — the inline \
         scan at validation_checks.rs:669-684 never reads extra_data, so rejecting or skipping a \
         malformed Bond here would change consensus below the activation height"
    );
    assert_eq!(
        (got.owned_bonds, got.all_bonds),
        (4u32, 5u32),
        "REQ-VEST-004: four of the five Bonds are owned; all five are counted"
    );
    assert_eq!(
        got.non_bond_total, 0,
        "REQ-VEST-004: a malformed Bond is still a Bond, never non-Bond value"
    );
}

// ---------------------------------------------------------------------------
// REQ-VEST-004 — saturation and the empty results
// ---------------------------------------------------------------------------

#[test]
fn req_vest_004_non_bond_total_saturates_at_u64_max() {
    let me = owner_hash();
    let held = vec![normal(u64::MAX), normal(u64::MAX), normal(1)];
    let tx = tx_for(&[0, 1, 2]);
    let set = in_memory_set(&pairs_for(&held));

    let got = resolve_withdrawal_inputs(&tx, &set, &me);

    assert_eq!(
        got.non_bond_total,
        u64::MAX,
        "REQ-VEST-004: non_bond_total must SATURATE at u64::MAX — a wrapping sum would report a \
         tiny non-Bond total to the M4 bound, and a checked add would panic a validating node on \
         an attacker-shaped transaction"
    );
    assert_eq!(
        (got.all_bonds, got.owned_bonds),
        (0u32, 0u32),
        "REQ-VEST-004: no input resolves to a Bond here"
    );
}

#[test]
fn req_vest_004_zero_inputs_and_absent_outpoints_yield_the_empty_result() {
    let me = owner_hash();

    let empty_tx = tx_for(&[]);
    let empty_set = in_memory_set(&[]);
    let from_empty = resolve_withdrawal_inputs(&empty_tx, &empty_set, &me);

    let absent_held = vec![Held::Absent, Held::Absent];
    let absent_tx = tx_for(&[0, 1]);
    let absent_set = in_memory_set(&pairs_for(&absent_held));
    let from_absent = resolve_withdrawal_inputs(&absent_tx, &absent_set, &me);

    for (label, got) in [
        ("zero inputs", &from_empty),
        ("absent outpoints", &from_absent),
    ] {
        assert_eq!(
            (got.owned_bonds, got.all_bonds),
            (0u32, 0u32),
            "REQ-VEST-004 [{label}]: nothing resolves, so nothing is counted"
        );
        assert!(
            got.spent_bonds.is_empty(),
            "REQ-VEST-004 [{label}]: an input the UTXO set cannot resolve contributes no \
             spent bond"
        );
        assert_eq!(
            got.non_bond_total, 0,
            "REQ-VEST-004 [{label}]: an unresolvable input contributes no non-Bond value"
        );
        assert!(
            got.malformed_bond_inputs.is_empty(),
            "REQ-VEST-004 [{label}]: an unresolvable input is not a malformed Bond"
        );
    }
}
