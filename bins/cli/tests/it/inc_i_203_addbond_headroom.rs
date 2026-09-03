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
// covers: bins/cli/src/rpc_client.rs
//
// ===========================================================================
// M6 / REV-203-002 + REV-203-010 + REV-203-011
// ===========================================================================
// OUTPUT CONTRACT: fn addbond_current_from_rpc(producer_set_bond_count: Option<u32>,
//                                              utxo_bond_count: u32) -> u32
// O6: Return value — the ProducerSet count when the node supplies it
// O7: Return value — the UTXO-derived count when it does not (older node)
// O8: No mutable params, no receiver, no store writes, no globals (pure fn)
// O9: Source-level wiring — rpc_client.rs declares producer_set_bond_count and
//     bonds.rs feeds addbond_current_from_rpc, not producer.bond_count, into the guard
// O10: Err message text at headroom == 0 — names the cap and delegation, never "--count 0"
// PATHS: (d) node supplies the field, (e) node omits it, (f) at-cap message,
//        (g) below-cap message
// MATRIX: O6xP_midepoch_addbond, O6xP_midepoch_withdrawal, O7xP_legacy_node,
//         O9xP_rpc_field, O9xP_bonds_call, O9xP_no_raw_bond_count, O9xP_shared_rule,
//         O10xP_at_cap, O3xP_below_cap
//
// INPUT PARTITIONS (M6):
// - P_midepoch_addbond:    ProducerSet 2990, RPC bondCount 2995, pending 5 (accept — 3000)
// - P_midepoch_withdrawal: ProducerSet 3000, RPC bondCount 2995, pending 0 (reject — 3005)
// - P_legacy_node:         producerSetBondCount absent, RPC bondCount 2995 (fall back)
// - P_at_cap:              bond_count=3000, pending=0, requested=5 (headroom saturates to 0)
// - P_below_cap:           bond_count=2999, pending=0, requested=4 (headroom 1)
//
// RED: `addbond_headroom_check` does not exist yet, so this module does not compile.
// That compile failure is the red for the pure cells; the source-level cells are red
// because bonds.rs today reads bond_count from get_producers and ignores it.
// The existing "not registered" bail (bonds.rs:34-40) is unchanged and is not retested here.
// handle_add_bond is async and needs Wallet+RpcClient, so it is not directly callable;
// its wiring is asserted at source level via include_str!, as in delegation_bond_cap.rs.

use doli_cli::producer_ledger::{addbond_current_from_rpc, addbond_headroom_check};
use doli_core::consensus::MAX_BONDS_PER_PRODUCER;

const BONDS_SRC: &str = include_str!("../../src/cmd_producer/bonds.rs");
const LEDGER_SRC: &str = include_str!("../../src/producer_ledger.rs");
const RPC_SRC: &str = include_str!("../../src/rpc_client.rs");

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

// ═══════════════════════════════════════════════════════════════════════════
// M6 / REV-203-010 — the at-cap message must not name an impossible command
// ═══════════════════════════════════════════════════════════════════════════

// REQ-BOND-007 — Decision: a failure means the CLI tells a producer at the cap to
// "Re-run with --count 0", which bonds.rs rejects up front (`1..=10000`), so the
// only advice the operator gets is a command that cannot work.
#[test]
fn at_cap_refusal_states_the_cap_is_reached_and_points_to_delegation() {
    let msg = addbond_headroom_check(MAX_BONDS_PER_PRODUCER, 0, 5)
        .expect_err("3000 held + 5 requested must exceed the cap");
    assert!(
        !msg.contains("--count 0"),
        "REV-203-010: headroom saturates to 0 here and the message advises a count \
         bonds.rs refuses before it ever reaches this function \
         (`if !(1..=10000).contains(&count)`). got: {msg}"
    );
    assert!(
        msg.to_lowercase().contains("delegation"),
        "at the cap the only remaining growth path is delegation, so the message \
         must name it. got: {msg}"
    );
}

// REQ-BOND-007 — Decision: a failure means the at-cap rewrite also erased the
// actionable count for producers who DO still have room, turning a useful refusal
// into a dead end for everyone.
#[test]
fn below_cap_refusal_still_states_the_count_the_operator_may_use() {
    let msg = addbond_headroom_check(2_999, 0, 4).expect_err("2999 + 4 must exceed the cap");
    assert!(
        msg.contains("You may still add 1"),
        "REV-203-010: with headroom 1 the existing actionable form must survive the \
         at-cap rewrite. got: {msg}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// M6 / REV-203-002 — the CLI must count the field the node counts
// ═══════════════════════════════════════════════════════════════════════════

// REQ-BOND-007 — Decision: a failure means the CLI refuses a top-up the node would
// accept. getProducers' bondCount is UTXO-derived and already includes AddBonds
// mined this epoch, while the gate reads the flush-only ProducerSet count plus
// pending_addbond_count — so the mined bonds are counted twice and the operator is
// locked out of headroom they own.
#[test]
fn midepoch_addbond_is_not_counted_twice_against_the_cap() {
    assert_eq!(
        addbond_current_from_rpc(Some(2_990), 2_995),
        2_990,
        "REV-203-002: the gate's `current` term is the FLUSHED ProducerSet count. \
         The 5 bonds mined this epoch are already in `pending`, so taking the \
         UTXO-derived 2995 counts them a second time."
    );
    let verdict = addbond_headroom_check(2_990, 5, 5);
    assert!(
        verdict.is_ok(),
        "2990 flushed + 5 pending + 5 requested = 3000, which the node ACCEPTS \
         (the comparison is `>`). Today the CLI computes 2995 + 5 + 5 = 3005 and \
         refuses a transaction that would have been valid. got {verdict:?}"
    );
}

// REQ-BOND-007 — Decision: a failure means the CLI passes, signs and submits an
// AddBond the node then rejects — a mid-epoch withdrawal removes Bond UTXOs, so
// the UTXO-derived count DROPS below the ProducerSet count the gate still reads.
#[test]
fn midepoch_withdrawal_does_not_grant_headroom_the_node_will_refuse() {
    assert_eq!(
        addbond_current_from_rpc(Some(3_000), 2_995),
        3_000,
        "REV-203-002: the withdrawal is not flushed yet, so the ProducerSet still \
         holds 3000. Trusting the UTXO-derived 2995 invents 5 bonds of headroom."
    );
    let verdict = addbond_headroom_check(3_000, 0, 5);
    assert!(
        verdict.is_err(),
        "3000 flushed + 0 pending + 5 requested = 3005 > 3000: the node REJECTS \
         this. Passing here costs the operator a signed, submitted, doomed \
         transaction. got {verdict:?}"
    );
}

// REQ-BOND-007 — Decision: a failure means a CLI talking to a node that predates
// the new RPC field either panics or reads 0 headroom, breaking add-bond for every
// operator until the whole fleet is upgraded.
#[test]
fn a_node_without_the_new_field_falls_back_to_the_utxo_derived_count() {
    assert_eq!(
        addbond_current_from_rpc(None, 2_995),
        2_995,
        "`producerSetBondCount` is `#[serde(default)]`, so an older node yields \
         `None`. The documented fallback is the UTXO-derived bondCount — the \
         behaviour the CLI has today."
    );
}

// REQ-BOND-007 — Decision: a failure means the field is asserted about in tests but
// never actually parsed off the wire, so the CLI silently stays on the old count.
#[test]
fn rpc_producer_info_carries_the_producer_set_bond_count() {
    assert!(
        RPC_SRC.contains("producer_set_bond_count"),
        "REV-203-002: `ProducerInfo` must declare `producer_set_bond_count` \
         (camelCase `producerSetBondCount` on the wire) with `#[serde(default)]` \
         so older nodes still deserialize"
    );
}

// REQ-BOND-007 — Decision: a failure means the new resolver exists but handle_add_bond
// still feeds the raw UTXO count into the guard, so REV-203-002 is unfixed in the
// only place a user can reach.
#[test]
fn bonds_handler_feeds_the_resolved_count_into_the_headroom_guard() {
    assert!(
        BONDS_SRC.contains("addbond_current_from_rpc"),
        "REV-203-002: handle_add_bond must resolve the current count through \
         `addbond_current_from_rpc(producer.producer_set_bond_count, producer.bond_count)`"
    );
    assert!(
        !BONDS_SRC.contains("addbond_headroom_check(producer.bond_count"),
        "REV-203-002: the UTXO-derived `producer.bond_count` must not reach the \
         guard directly — that is the double-count this finding is about"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// M6 / REV-203-011 — one copy of the cap rule
// ═══════════════════════════════════════════════════════════════════════════

// REQ-BOND-007 — Decision: a failure means the CLI keeps its own re-derivation of
// the cap expression, which is the INC-I-180 `allowance_with` drift: the two agree
// until consensus changes one of them, and nothing catches it.
#[test]
fn producer_ledger_calls_the_shared_consensus_rule() {
    assert!(
        LEDGER_SRC.contains("check_addbond_cap"),
        "REV-203-011: `addbond_headroom_check` must CALL \
         `doli_core::validation::check_addbond_cap(bond_count, pending, requested, 0, 0)` \
         instead of restating `current + pending + requested <= cap` inline"
    );
}
