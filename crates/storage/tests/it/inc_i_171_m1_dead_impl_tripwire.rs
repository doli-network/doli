//! OUTPUT CONTRACT: N/A — source-content tripwire (no fn under test); asserts include_str!() text.
//! INPUT PARTITIONS: symbol-present (red, today) vs symbol-absent (green, after M1 deletion).
//! REQ-VEST-010 — Decision: reveals whether the dead FIFO withdrawal calculators at info.rs:21,443,450 and their mod.rs:54 re-export were actually deleted or silently left in place.

const INFO_RS: &str = include_str!("../../src/producer/info.rs");
const MOD_RS: &str = include_str!("../../src/producer/mod.rs");

#[test]
fn req_vest_010_calculate_withdrawal_from_bonds_is_deleted() {
    assert!(
        !INFO_RS.contains("fn calculate_withdrawal_from_bonds("),
        "REQ-VEST-010: dead free fn `calculate_withdrawal_from_bonds` must be deleted \
         from crates/storage/src/producer/info.rs:21"
    );
    assert!(
        !MOD_RS.contains("calculate_withdrawal_from_bonds"),
        "REQ-VEST-010: re-export of `calculate_withdrawal_from_bonds` must be deleted \
         from crates/storage/src/producer/mod.rs:54"
    );
}

#[test]
fn req_vest_010_producer_info_calculate_withdrawal_methods_are_deleted() {
    assert!(
        !INFO_RS.contains("fn calculate_withdrawal("),
        "REQ-VEST-010: dead method `ProducerInfo::calculate_withdrawal` must be deleted \
         from crates/storage/src/producer/info.rs:443"
    );
    assert!(
        !INFO_RS.contains("fn calculate_withdrawal_with_quarter("),
        "REQ-VEST-010: dead method `ProducerInfo::calculate_withdrawal_with_quarter` must be \
         deleted from crates/storage/src/producer/info.rs:450"
    );
}
