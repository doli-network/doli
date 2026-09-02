// OUTPUT CONTRACT: fn addbond_headroom_check(bond_count: u32, pending: u32, requested: u32) -> Result<(), String>
// O1: Return value — Ok(()) when bond_count + pending + requested <= MAX_BONDS_PER_PRODUCER
// O2: Return value — Err(String) when that sum exceeds MAX_BONDS_PER_PRODUCER
// O3: Err message text — states current count, the cap, and the remaining headroom
// O4: No mutable params, no receiver mutation, no store writes, no globals, no channels (pure fn)
// O5: Call-site ordering in handle_add_bond — guard precedes get_utxos / sign / submit
// PATHS: (a) within-headroom accept, (b) over-headroom reject, (c) saturating reject on overflow
// MATRIX: O1xP_zero_base, O1xP_exact_cap, O1xP_pending_exact, O2xP_headline, O2xP_pending_over,
//         O2xP_above_cap_from_zero, O2xP_u32_max, O3xP_headline, O3xP_wide_headroom, O5xP_source_order
//
// INPUT PARTITIONS:
// - P_zero_base:           bond_count=0,    pending=0, requested=3000  (accept — full cap)
// - P_above_cap_from_zero: bond_count=0,    pending=0, requested=3001  (reject — boundary +1)
// - P_exact_cap:           bond_count=2999, pending=0, requested=1     (accept — exactly fills cap)
// - P_headline:            bond_count=2999, pending=0, requested=4     (reject — live testnet repro)
// - P_wide_headroom:       bond_count=2000, pending=0, requested=1500  (reject — distinctive headroom 1000)
// - P_pending_exact:       bond_count=2990, pending=9, requested=1     (accept — sums to 3000)
// - P_pending_over:        bond_count=2990, pending=9, requested=2     (reject — sums to 3001)
// - P_u32_max:             all three u32::MAX                          (reject, must not panic)
//
// covers: bins/cli/src/cmd_producer/bonds.rs
// covers: bins/cli/src/producer_ledger.rs
//
// RED: `addbond_headroom_check` does not exist yet, so this module does not compile.
// That compile failure is the red for the pure cells; the source-level cells are red
// because bonds.rs today reads bond_count from get_producers and ignores it.
// The existing "not registered" bail (bonds.rs:34-40) is unchanged and is not retested here.
// handle_add_bond is async and needs Wallet+RpcClient, so it is not directly callable;
// its wiring is asserted at source level via include_str!, as in delegation_bond_cap.rs.

use doli_cli::producer_ledger::addbond_headroom_check;
use doli_core::consensus::MAX_BONDS_PER_PRODUCER;

const BONDS_SRC: &str = include_str!("../../src/cmd_producer/bonds.rs");
const LEDGER_SRC: &str = include_str!("../../src/producer_ledger.rs");

// REQ-BOND-007 — Decision: a changed cap would silently invalidate every headroom bound below and force re-deriving the CLI guard.
#[test]
fn consensus_max_bonds_constant_is_3000() {
    assert_eq!(MAX_BONDS_PER_PRODUCER, 3_000);
}

// REQ-BOND-007 — Decision: failure means the CLI still builds and submits the over-cap AddBond that testnet accepted then poisoned.
#[test]
fn cli_add_bond_refuses_over_remaining_bonds_headroom() {
    let result = addbond_headroom_check(2_999, 0, 4);
    assert!(
        result.is_err(),
        "--count 4 at bond_count=2999 must be refused before signing; got {result:?}"
    );
}

// REQ-BOND-007 — Decision: failure means the operator is refused without being told how many bonds they may still add.
#[test]
fn add_bond_refusal_states_current_count_cap_and_remaining_headroom() {
    let cap = MAX_BONDS_PER_PRODUCER.to_string();

    let msg = addbond_headroom_check(2_999, 0, 4).expect_err("2999 + 4 must exceed the cap");
    assert!(
        msg.contains("2999"),
        "message must state current count 2999: {msg}"
    );
    assert!(
        msg.contains(&cap),
        "message must state the cap {cap}: {msg}"
    );
    assert!(
        msg.contains('1'),
        "message must state remaining headroom 1: {msg}"
    );

    let wide =
        addbond_headroom_check(2_000, 0, 1_500).expect_err("2000 + 1500 must exceed the cap");
    assert!(
        wide.contains("2000") && wide.contains(&cap) && wide.contains("1000"),
        "message must state count 2000, cap {cap} and headroom 1000: {wide}"
    );
}

// REQ-BOND-007 — Decision: failure means the guard is off by one and blocks the legitimate last bond a producer may hold.
#[test]
fn add_bond_accepts_the_request_that_exactly_fills_the_cap() {
    let result = addbond_headroom_check(2_999, 0, 1);
    assert!(
        result.is_ok(),
        "--count 1 at bond_count=2999 fills the cap exactly and must succeed; got {result:?}"
    );
}

// REQ-BOND-007 — Decision: failure means pending AddBond updates are ignored, so two CLI calls in one epoch still cross the cap.
#[test]
fn add_bond_headroom_subtracts_pending_addbond_updates() {
    let at_cap = addbond_headroom_check(2_990, 9, 1);
    assert!(
        at_cap.is_ok(),
        "2990 held + 9 pending + 1 requested = 3000 must be accepted; got {at_cap:?}"
    );

    let over = addbond_headroom_check(2_990, 9, 2);
    assert!(
        over.is_err(),
        "2990 held + 9 pending + 2 requested = 3001 must be refused; got {over:?}"
    );
}

// REQ-BOND-007 — Decision: failure means a fresh producer is either capped below the protocol limit or allowed past it in one call.
#[test]
fn add_bond_from_zero_accepts_the_full_cap_and_refuses_one_more() {
    let full = addbond_headroom_check(0, 0, 3_000);
    assert!(
        full.is_ok(),
        "a fresh producer may request the full cap; got {full:?}"
    );

    let over = addbond_headroom_check(0, 0, 3_001);
    assert!(
        over.is_err(),
        "3001 from zero must be refused; got {over:?}"
    );
}

// REQ-BOND-007 — Decision: failure means a malformed or hostile RPC bond_count panics the CLI instead of refusing the request.
#[test]
fn add_bond_headroom_check_saturates_instead_of_overflowing() {
    let result = addbond_headroom_check(u32::MAX, u32::MAX, u32::MAX);
    assert!(
        result.is_err(),
        "u32::MAX triple must be refused by saturating arithmetic, not wrapped; got {result:?}"
    );
}

// REQ-BOND-007 — Decision: failure means the guard exists but runs after UTXO selection, so a refused request still spends or submits.
#[test]
fn add_bond_guard_is_wired_into_bonds_handler_before_any_utxo_is_consumed() {
    let guard_at = BONDS_SRC.find("addbond_headroom_check").expect(
        "handle_add_bond must call addbond_headroom_check after the registration check \
         (bonds.rs:34-40) and before building the transaction",
    );
    let utxo_at = BONDS_SRC
        .find("get_utxos")
        .expect("bonds.rs must still fetch spendable UTXOs via get_utxos");
    assert!(
        guard_at < utxo_at,
        "headroom guard at byte {guard_at} must precede get_utxos at byte {utxo_at}: \
         the refusal has to happen before any UTXO is selected, signed or submitted"
    );
}

// REQ-BOND-007 — Decision: failure means the CLI carries its own copy of the cap and will drift from consensus on the next change.
#[test]
fn producer_ledger_must_not_hardcode_the_bond_cap() {
    assert!(
        !LEDGER_SRC.contains("3000") && !LEDGER_SRC.contains("3_000"),
        "producer_ledger.rs must reference MAX_BONDS_PER_PRODUCER, not a hardcoded cap literal"
    );
}
