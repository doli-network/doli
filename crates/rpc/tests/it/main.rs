//! Consolidated integration-test binary for `rpc` (single test binary per crate —
//! see .claude/hooks/test-binary-gate.sh). New integration tests are modules here,
//! not top-level `tests/*.rs` files.
//!
//! OUTPUT CONTRACT: N/A — fixture file (module aggregator only, no test logic).
//! INPUT PARTITIONS: N/A — fixture file.

mod audit_rpc_access;
mod audit_rpc_access_fixture;
mod audit_rpc_effects;
mod inc_i_171_m7_bond_details_shape;
mod inc_i_178_m4_empty_detection;
mod inc_i_180_ledger_fields;
mod inc_i_204_m41_force_reorg_rpc;
mod inc_i_233_confirmed_fee;
mod inc_i_248_state_snapshot_maintainer;
mod inc_i_308_m12_reorg_race;
mod inc_i_308_r1_anchor;
mod inc_i_308_r2_populations;
