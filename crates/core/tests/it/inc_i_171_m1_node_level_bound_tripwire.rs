//! OUTPUT CONTRACT: N/A — source-content tripwire (no fn under test); asserts include_str!() text.
//! INPUT PARTITIONS: false-claim string present/absent; accurate-reference present/absent.
//! REQ-VEST-002 — Decision: reveals whether the false claim of a node-level payout-amount bound was corrected and the real consensus gate is now named where a reader looks for it.
// M4: extend this tripwire to also assert TX_TYPES_RS names `check_withdrawal_payout_bound`.

const TX_TYPES_RS: &str = include_str!("../../src/validation/tx_types.rs");

#[test]
fn req_vest_002_false_node_level_payout_bound_claim_is_gone() {
    assert!(
        !TX_TYPES_RS.contains("Output amount <= FIFO net calculation"),
        "REQ-VEST-002: false claim of a node-level payout-amount bound must be removed \
         from crates/core/src/validation/tx_types.rs:616"
    );
}

#[test]
fn req_vest_002_withdrawal_request_validator_names_the_real_gate() {
    assert!(
        TX_TYPES_RS.contains("inc_i_171_vesting_penalty_activation_height"),
        "REQ-VEST-002: validate_withdrawal_request_data comment must name \
         `inc_i_171_vesting_penalty_activation_height` in \
         crates/core/src/validation/tx_types.rs:613-616"
    );
}
