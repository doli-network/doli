//! INC-I-171 M3 — three-site convergence tripwire (INV-VEST-008: one verdict from three sites).
//!
//! OUTPUT CONTRACT: N/A — source-content tripwire (no fn under test); asserts include_str!() text
//! of four files. The only output is the assertion verdict.
//! INPUT PARTITIONS: duplicated-copies-present (red, today: `bond_input_split` is defined at
//! crates/mempool/src/withdrawal_holdings.rs:120 and bins/node/src/node/production/withdrawal_holdings.rs:245,
//! and the third copy is INLINE at the node withdrawal pass, since M4 in
//! bins/node/src/node/validation_checks/withdrawal_economics.rs) vs
//! single-resolver (green, after M3).
//!
//! REQ-VEST-008 — Decision: reveals whether a later edit re-forked the withdrawal bond scan into
//! per-site copies again, which is exactly how mempool, builder and validation drifted to three
//! different verdicts for one transaction.

const VALIDATION_CHECKS: &str =
    include_str!("../../../../bins/node/src/node/validation_checks/withdrawal_economics.rs");
const MEMPOOL_HOLDINGS: &str =
    include_str!("../../../../crates/mempool/src/withdrawal_holdings.rs");
const PRODUCTION_HOLDINGS: &str =
    include_str!("../../../../bins/node/src/node/production/withdrawal_holdings.rs");
const PRODUCER_MOD: &str = include_str!("../../src/producer/mod.rs");

#[test]
fn req_vest_008_bond_input_split_is_deleted_from_both_copies() {
    assert!(
        !MEMPOOL_HOLDINGS.contains("bond_input_split"),
        "REQ-VEST-008: crates/mempool/src/withdrawal_holdings.rs must not keep its own \
         `bond_input_split` copy (:120) — mempool admission and block validation must reach \
         ONE verdict from ONE resolver, so the mempool site calls resolve_withdrawal_inputs"
    );
    assert!(
        !PRODUCTION_HOLDINGS.contains("bond_input_split"),
        "REQ-VEST-008: bins/node/src/node/production/withdrawal_holdings.rs must not keep its own \
         `bond_input_split` copy (:245) — the block builder must reach the SAME verdict as \
         validation, so the builder site calls resolve_withdrawal_inputs"
    );
}

#[test]
fn req_vest_008_all_three_sites_call_the_one_resolver() {
    assert!(
        VALIDATION_CHECKS.contains("resolve_withdrawal_inputs"),
        "REQ-VEST-008: the node withdrawal pass \
         (bins/node/src/node/validation_checks/withdrawal_economics.rs) must call \
         resolve_withdrawal_inputs — its bond scan was INLINE before M3, the copy that \
         has no name and therefore no way to drift visibly"
    );
    assert!(
        MEMPOOL_HOLDINGS.contains("resolve_withdrawal_inputs"),
        "REQ-VEST-008: crates/mempool/src/withdrawal_holdings.rs must call \
         resolve_withdrawal_inputs — deleting bond_input_split without adopting the resolver \
         would silently drop the mempool's holdings check"
    );
    assert!(
        PRODUCTION_HOLDINGS.contains("resolve_withdrawal_inputs"),
        "REQ-VEST-008: bins/node/src/node/production/withdrawal_holdings.rs must call \
         resolve_withdrawal_inputs — deleting bond_input_split without adopting the resolver \
         would silently drop the builder's holdings check"
    );
}

#[test]
fn req_vest_008_producer_mod_re_exports_the_one_resolver() {
    assert!(
        PRODUCER_MOD.contains("resolve_withdrawal_inputs"),
        "REQ-VEST-008: crates/storage/src/producer/mod.rs must re-export \
         resolve_withdrawal_inputs from withdrawal_inputs.rs — three crates (node, mempool, \
         storage) can only share ONE resolver if it has ONE public path"
    );
}
