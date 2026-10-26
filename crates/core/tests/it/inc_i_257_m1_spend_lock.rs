//! INC-I-257 M1 — the one consensus spend-lock predicate shared by mempool,
//! builder and apply: below the gate it is today's verdict; at/above the gate a
//! Bond input is spendable only by RequestWithdrawal and an Unbonding input never
//! bypasses its lock.
//!
// covers: validation unbonding utxo
//
// OUTPUT CONTRACT: fn spend_lock_verdict(spending: TxType, spent: &Output, height: u64, unbonding_activation_height: u64) -> Result<(), ValidationError>
//   O1 return: Ok | Err(OutputLocked { lock_height: spent.lock_until, current_height: height })
//      | Err(message containing "ERRTX-UNBOND") for a Bond input of a non-RequestWithdrawal tx at/above the gate
//   pure: no mutable params, no receiver, no store, no globals
// PATHS: P-BELOW height < gate | P-UNB Unbonding input at/above | P-BOND Bond input at/above | P-OTHER other types at/above
// INPUT PARTITIONS:
//   P-BELOW: every (output type x tx type) x lock {0, H-1, H, H+1, MAX} x gate {H+1, MAX}
//   P-UNB:   every tx type x height {L-1, L, L+1}; gate {0, below the height}
//   P-BOND:  RequestWithdrawal x lock {0, passed, H, future, MAX} -> Ok; every other tx type -> refused
//   P-OTHER: every non-Bond, non-Unbonding output type x tx type x lock -> today's verdict
//   SEC-001: heights u64::MAX - 1 and u64::MAX, lock u64::MAX; gate boundary gate - 1 / gate
// MATRIX:
//   O1 x P-BELOW -> inc_i_257_m1_req_unbond_005_below_gate_verdict_equals_today_for_every_pair
//   O1 x P-UNB -> inc_i_257_m1_req_unbond_006_unbonding_input_never_bypasses_lock
//   O1 x P-BOND -> inc_i_257_m1_req_unbond_014_bond_input_only_by_request_withdrawal
//   O1 x gate boundary -> inc_i_257_m1_req_unbond_014_bond_transfer_past_lock_flips_at_gate
//   O1 x P-OTHER -> inc_i_257_m1_req_unbond_005_other_types_unchanged_at_or_above_gate
//   O1 x SEC-001 -> inc_i_257_m1_req_unbond_sec_001_extreme_heights_no_panic

use crypto::Hash;
use doli_core::transaction::{Output, OutputType, TxType};
use doli_core::validation::{spend_lock_verdict, ValidationError};

const H: u64 = 100;

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Ok,
    Locked(u64, u64),
    Unbond,
    Other(String),
}

fn verdict(r: Result<(), ValidationError>) -> Verdict {
    match r {
        Ok(()) => Verdict::Ok,
        Err(ValidationError::OutputLocked {
            lock_height,
            current_height,
        }) => Verdict::Locked(lock_height, current_height),
        Err(e) if e.to_string().contains("ERRTX-UNBOND") => Verdict::Unbond,
        Err(e) => Verdict::Other(e.to_string()),
    }
}

/// The verdict of `validate_transaction_with_utxos` on the base commit.
fn today(spending: TxType, spent: &Output, height: u64) -> Verdict {
    let bypass = matches!(spending, TxType::RequestWithdrawal | TxType::Exit)
        && spent.output_type == OutputType::Bond;
    if !bypass && height < spent.lock_until {
        Verdict::Locked(spent.lock_until, height)
    } else {
        Verdict::Ok
    }
}

fn out(output_type: OutputType, lock_until: u64) -> Output {
    Output {
        output_type,
        amount: 5_000,
        pubkey_hash: Hash::from_bytes([0x33; 32]),
        lock_until,
        extra_data: Vec::new(),
    }
}

fn all_tx_types() -> Vec<TxType> {
    (0..=64u32).filter_map(TxType::from_u32).collect()
}

fn all_output_types() -> Vec<OutputType> {
    (0..=255u8).filter_map(OutputType::from_u8).collect()
}

fn call(tx: TxType, o: &Output, height: u64, gate: u64) -> Verdict {
    verdict(spend_lock_verdict(tx, o, height, gate))
}

// REQ-UNBOND-005 / REQ-UNBOND-017 — Decision: a failure means the shared predicate changes a pre-gate verdict, so an old chain no longer replays bit-identically.
#[test]
fn inc_i_257_m1_req_unbond_005_below_gate_verdict_equals_today_for_every_pair() {
    let (txs, types) = (all_tx_types(), all_output_types());
    assert!(
        txs.len() >= 20 && types.len() >= 15,
        "NON-VACUITY: enumerations"
    );
    let mut mismatches = Vec::new();
    for gate in [H + 1, u64::MAX] {
        for ty in &types {
            for tx in &txs {
                for lock in [0, H - 1, H, H + 1, u64::MAX] {
                    let o = out(*ty, lock);
                    let (got, want) = (call(*tx, &o, H, gate), today(*tx, &o, H));
                    if got != want {
                        mismatches.push(format!(
                            "gate={gate} {ty:?} by {tx:?} lock={lock}: got {got:?}, want {want:?}"
                        ));
                    }
                }
            }
        }
    }
    assert!(
        mismatches.is_empty(),
        "below-gate verdicts differ from today:\n{}",
        mismatches.join("\n")
    );
}

// REQ-UNBOND-006 / REQ-UNBOND-005 — Decision: a failure means some tx type (RequestWithdrawal, Exit or any other) releases an Unbonding output before its lock height, or the output stays locked after it.
#[test]
fn inc_i_257_m1_req_unbond_006_unbonding_input_never_bypasses_lock() {
    let lock = 160;
    for gate in [0, 150] {
        for tx in all_tx_types() {
            let o = out(OutputType::Unbonding, lock);
            assert_eq!(
                call(tx, &o, lock - 1, gate),
                Verdict::Locked(lock, lock - 1),
                "gate={gate} {tx:?} at lock_until - 1"
            );
            assert_eq!(
                call(tx, &o, lock, gate),
                Verdict::Ok,
                "gate={gate} {tx:?} at lock_until"
            );
            assert_eq!(
                call(tx, &o, lock + 1, gate),
                Verdict::Ok,
                "gate={gate} {tx:?} after lock_until"
            );
        }
    }
}

// REQ-UNBOND-014 — Decision: a failure means a Bond is spendable by a tx other than RequestWithdrawal at/above the gate (a burn escape for CLI bonds whose finite lock has passed), or the withdrawal itself loses its bypass.
#[test]
fn inc_i_257_m1_req_unbond_014_bond_input_only_by_request_withdrawal() {
    for lock in [0, H - 50, H, H + 100, u64::MAX] {
        let o = out(OutputType::Bond, lock);
        assert_eq!(
            call(TxType::RequestWithdrawal, &o, H, 0),
            Verdict::Ok,
            "RequestWithdrawal keeps the Bond bypass (lock {lock})"
        );
        for tx in all_tx_types()
            .into_iter()
            .filter(|t| *t != TxType::RequestWithdrawal)
        {
            assert_eq!(
                call(tx, &o, H, 0),
                Verdict::Unbond,
                "{tx:?} spending a Bond (lock {lock}) must be refused at/above the gate"
            );
        }
    }
}

// REQ-UNBOND-014 — Decision: a failure means the Bond-input rule is off by one at the activation height, or leaks below it (breaking pre-gate replay).
#[test]
fn inc_i_257_m1_req_unbond_014_bond_transfer_past_lock_flips_at_gate() {
    let gate = 500;
    let o = out(OutputType::Bond, 10);
    assert_eq!(
        call(TxType::Transfer, &o, gate - 1, gate),
        Verdict::Ok,
        "below the gate: today's verdict"
    );
    assert_eq!(
        call(TxType::Transfer, &o, gate, gate),
        Verdict::Unbond,
        "at the gate: refused"
    );
    assert_eq!(
        call(TxType::Exit, &o, gate - 1, gate),
        Verdict::Ok,
        "below: Exit bypass kept"
    );
    assert_eq!(
        call(TxType::Exit, &o, gate, gate),
        Verdict::Unbond,
        "at: Exit refused"
    );
}

// REQ-UNBOND-005 — Decision: a failure means the shared predicate changes the lock verdict of an unrelated output type (Normal, Vesting, HTLC, ...) once the gate opens.
#[test]
fn inc_i_257_m1_req_unbond_005_other_types_unchanged_at_or_above_gate() {
    let mut mismatches = Vec::new();
    for ty in all_output_types()
        .into_iter()
        .filter(|t| !matches!(t, OutputType::Bond | OutputType::Unbonding))
    {
        for tx in all_tx_types() {
            for lock in [0, H - 1, H, H + 1, u64::MAX] {
                let o = out(ty, lock);
                let (got, want) = (call(tx, &o, H, 0), today(tx, &o, H));
                if got != want {
                    mismatches.push(format!(
                        "{ty:?} by {tx:?} lock={lock}: got {got:?}, want {want:?}"
                    ));
                }
            }
        }
    }
    assert!(
        mismatches.is_empty(),
        "at/above-gate verdicts changed:\n{}",
        mismatches.join("\n")
    );
}

// REQ-UNBOND-SEC-001 — Decision: a failure means the predicate panics or wraps at the top of the height range, so a crafted lock value crashes or unlocks a node.
#[test]
fn inc_i_257_m1_req_unbond_sec_001_extreme_heights_no_panic() {
    let m = u64::MAX;
    let unb = out(OutputType::Unbonding, m);
    assert_eq!(
        call(TxType::Transfer, &unb, m - 1, 0),
        Verdict::Locked(m, m - 1)
    );
    assert_eq!(call(TxType::Transfer, &unb, m, 0), Verdict::Ok);
    assert_eq!(
        call(TxType::RequestWithdrawal, &unb, m - 1, m - 1),
        Verdict::Locked(m, m - 1)
    );
    let bond = out(OutputType::Bond, m);
    assert_eq!(
        call(TxType::RequestWithdrawal, &bond, m - 1, 0),
        Verdict::Ok
    );
    assert_eq!(call(TxType::Transfer, &bond, m - 1, m - 1), Verdict::Unbond);
    assert_eq!(
        call(TxType::Transfer, &out(OutputType::Unbonding, 0), 0, 0),
        Verdict::Ok
    );
}
