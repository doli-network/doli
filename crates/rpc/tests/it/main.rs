//! Consolidated integration-test binary for `rpc` (single test binary per crate —
//! see .claude/hooks/test-binary-gate.sh). New integration tests are modules here,
//! not top-level `tests/*.rs` files.
//!
//! OUTPUT CONTRACT: N/A — fixture file (module aggregator only, no test logic).
//! INPUT PARTITIONS: N/A — fixture file.

mod inc_i_180_ledger_fields;
mod inc_i_204_m41_force_reorg_rpc;
