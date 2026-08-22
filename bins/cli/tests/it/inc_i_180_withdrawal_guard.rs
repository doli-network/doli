//! INC-I-180 M3 — RED reproduction for the CLI withdrawal/exit two-ledger defect.
//! covers: withdrawal, exit  (source stems this file drives)
//!
//! KIND: COMPILE-RED. `doli-cli` has no library target at HEAD 448dca75, so
//! `use doli_cli::producer_ledger::*` cannot resolve. That compile failure IS the
//! RED proof that the pure helpers REQ-I180-005 / AUDIT-P2-003 require do not exist.
//! The developer makes it GREEN by adding `bins/cli/src/lib.rs`
//! (`pub mod producer_ledger;`) + `bins/cli/src/producer_ledger.rs` with the four
//! functions asserted below, THEN rewiring withdrawal.rs / exit.rs. The include_str!
//! structural tests stay assertion-RED until that rewire lands.
//!
//! ── OUTPUT CONTRACT ─────────────────────────────────────────────────────────
//! Function under test: the CLI request-withdrawal / exit allowance+selection path,
//! extracted into pure helpers in `doli_cli::producer_ledger`.
//! OBSERVABLE OUTPUTS:
//!   O1 allowance number P            = producer_set_allowance(sw, recv, deleg)
//!   O2 max withdrawable              = max_withdrawable(P, withdrawal_pending)
//!   O3 mismatch bool                 = withdrawal_ledger_mismatch(utxo_bond_count, P)
//!   O4 selected bond-input set       = select_bond_inputs_by_count(amounts, count)
//!   O5 (structural) withdrawal.rs / exit.rs WIRING: buggy formula removed, helpers
//!      called, both numbers (UTXO count AND ProducerSet allowance) printed pre-sign.
//!   no mutable params; no persistent store; no side channel (pure fns).
//! CODE PATHS: aligned ledgers; n11 zero-bond shape (P>0, UTXO=0); pending-withdrawal
//!   present (W>0); over-request; under-owned selection.
//! INPUT PARTITIONS: normal producer; n11 zero-bond shape; pending-withdrawal present.
//! MATRIX (O1..O4) × {aligned, n11, pending, over} — every cell asserted below.
//! ────────────────────────────────────────────────────────────────────────────

use doli_cli::producer_ledger::{
    max_withdrawable, producer_set_allowance, select_bond_inputs_by_count,
    withdrawal_ledger_mismatch,
};

const BOND_UNIT: u64 = 1_000_000_000;

// ── REQ-I180-005 · allowance is a PURE ProducerSet inversion ────────────────
// covers: withdrawal, exit
// Decision: a failure here reveals the CLI is not inverting the ProducerSet
// (P = selectionWeight − Σ receivedDelegations + delegatedBonds) and has fallen
// back to the mixed UTXO-minus-pending formula that oversized n11's withdrawal.

/// REQ-I180-005 (Must). n11 shape: weight 434, no delegations → own-bond P == 434.
#[test]
fn req_i180_005_allowance_recovers_own_bonds_for_the_n11_shape() {
    // selection_weight=434, received=0, delegated=0
    assert_eq!(producer_set_allowance(434, 0, 0), 434);
}

/// REQ-I180-005 (Must). Delegation case: own=10 bonds, delegated 3 away, received 5.
/// selection_weight = own − delegated + received = 10 − 3 + 5 = 12. The inversion
/// must recover own = 10, NOT the 12 that selection_weight shows.
#[test]
fn req_i180_005_allowance_inverts_delegations_back_to_own_bonds() {
    let selection_weight = 12; // 10 - 3 + 5
    assert_eq!(producer_set_allowance(selection_weight, 5, 3), 10);
}

/// REQ-I180-005 (Must). max withdrawable = P − W (pending withdrawals subtracted
/// from the ProducerSet allowance, never from the UTXO count).
/// Decision: failure means W was subtracted from the wrong (UTXO) ledger again.
#[test]
fn req_i180_005_max_withdrawable_subtracts_pending_from_allowance() {
    // covers: withdrawal, exit
    assert_eq!(max_withdrawable(434, 0), 434); // n11: nothing pending
    assert_eq!(max_withdrawable(433, 2), 431); // 2 already pending this epoch
    assert_eq!(max_withdrawable(1, 5), 0); // saturating: never underflows
}

// ── REQ-I180-005 · two-ledger mismatch guard (NEW early-return) ─────────────
// covers: withdrawal, exit
// Decision: a failure here means the CLI would emit a withdrawal whose declared
// count came from a ledger the node's gate disagrees with — the exact n11 event
// (UTXO 434 vs ProducerSet 433). The CLI must ABORT, not emit.

/// REQ-I180-005 (Must). The live n11 window: UTXO count leads the ProducerSet by
/// one after an unflushed AddBond. 434 != 433 → mismatch → CLI must abort.
#[test]
fn req_i180_005_mismatch_detects_the_unflushed_addbond_window() {
    assert!(withdrawal_ledger_mismatch(434, 433));
}

/// REQ-I180-005 (Must). n11 zero-bond shape: all Bond UTXOs spent (0) but the
/// ProducerSet still says 434. Machine-detectable disagreement → abort.
#[test]
fn req_i180_005_mismatch_detects_the_zero_bond_shape() {
    assert!(withdrawal_ledger_mismatch(0, 434));
}

/// REQ-I180-005 (Must). Aligned ledgers (post-boundary, n12 case) → no mismatch,
/// withdrawal is allowed to proceed. GREEN-lock so the guard is not a blanket abort.
#[test]
fn req_i180_005_aligned_ledgers_do_not_trip_the_guard() {
    assert!(!withdrawal_ledger_mismatch(431, 431));
}

// ── AUDIT-P2-003 · bond-INPUT selection binds COUNT, not VALUE ──────────────
// covers: withdrawal, exit
// Decision: a failure here means the emitted tx's Bond-input COUNT can differ from
// the declared count because inputs were accumulated by VALUE until
// bond_unit*count was met — the current withdrawal.rs/exit.rs behavior.

/// AUDIT-P2-003 (Must). Value-based selection would stop early when one fat bond
/// UTXO already covers bond_unit*count; count-based selection MUST return exactly
/// `count` distinct inputs.
#[test]
fn audit_p2_003_selection_returns_exactly_count_not_value_cover() {
    // Four owned bond UTXOs; the first alone is worth 5 units.
    let amounts = [5 * BOND_UNIT, BOND_UNIT, BOND_UNIT, BOND_UNIT];
    let selected = select_bond_inputs_by_count(&amounts, 2).expect("owns >= 2 bonds");
    assert_eq!(
        selected.len(),
        2,
        "must select exactly `count` bond inputs, not stop once value is covered"
    );
}

/// AUDIT-P2-003 (Must). Exit withdraws ALL owned bonds: count == number owned →
/// every input selected.
#[test]
fn audit_p2_003_selection_of_all_owned_bonds() {
    let amounts = [BOND_UNIT, BOND_UNIT, BOND_UNIT];
    let selected = select_bond_inputs_by_count(&amounts, 3).expect("owns 3");
    assert_eq!(selected.len(), 3);
}

/// AUDIT-P2-003 (Must). Requesting more bonds than owned is an error, not a
/// short/oversized tx. Decision: failure means the CLI could emit a withdrawal it
/// cannot back with inputs.
#[test]
fn audit_p2_003_selection_over_owned_is_rejected() {
    let amounts = [BOND_UNIT, BOND_UNIT];
    assert!(select_bond_inputs_by_count(&amounts, 3).is_err());
}

// ── REQ-I180-005 · STRUCTURAL wiring of withdrawal.rs and exit.rs ───────────
// KIND: assertion-RED (compiles once the lib target exists; fails until rewire).
// `doli-cli` is a bin crate, so these fns are unreachable from an integration
// test; the include_str! convention is the one used by
// bins/cli/tests/inc_i_172_cli_trust_root_resolution_test.rs.

const WITHDRAWAL_SRC: &str = include_str!("../../src/cmd_producer/withdrawal.rs");
const EXIT_SRC: &str = include_str!("../../src/cmd_producer/exit.rs");

/// REQ-I180-005 (Must). covers: withdrawal.
/// The mixed-ledger formula must be gone from request-withdrawal.
#[test]
fn req_i180_005_withdrawal_drops_the_mixed_ledger_formula() {
    assert!(
        !WITHDRAWAL_SRC.contains("details.bond_count - details.withdrawal_pending_count"),
        "withdrawal.rs still computes `available` as UTXO bond_count − pending; that mixes \
         two ledgers and oversized n11. Derive P from the ProducerSet via \
         producer_ledger::producer_set_allowance instead."
    );
}

/// REQ-I180-005 (Must). covers: exit.
#[test]
fn req_i180_005_exit_drops_the_mixed_ledger_formula() {
    assert!(
        !EXIT_SRC.contains("details.bond_count - details.withdrawal_pending_count"),
        "exit.rs still computes `available` as UTXO bond_count − pending — same defect."
    );
}

/// REQ-I180-005 (Must). covers: withdrawal, exit.
/// Both paths must consult the new mismatch guard before signing.
#[test]
fn req_i180_005_both_paths_consult_the_ledger_mismatch_guard() {
    assert!(
        WITHDRAWAL_SRC.contains("withdrawal_ledger_mismatch"),
        "withdrawal.rs must abort on a UTXO/ProducerSet ledger mismatch before emitting"
    );
    assert!(
        EXIT_SRC.contains("withdrawal_ledger_mismatch"),
        "exit.rs must abort on a UTXO/ProducerSet ledger mismatch before emitting"
    );
}

/// REQ-I180-005 (Must). covers: withdrawal.
/// The pre-submit confirmation must print BOTH numbers: the UTXO bond count AND the
/// ProducerSet allowance. A source-text proxy: the request-withdrawal path must call
/// the ProducerSet allowance helper (so it HAS the second number to print).
#[test]
fn req_i180_005_withdrawal_prints_both_ledger_numbers_pre_submit() {
    assert!(
        WITHDRAWAL_SRC.contains("producer_set_allowance"),
        "request-withdrawal must compute and display the ProducerSet allowance next to \
         the UTXO bond count in its pre-submit confirmation"
    );
}
