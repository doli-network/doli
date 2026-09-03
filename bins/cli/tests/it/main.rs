//! Consolidated integration-test binary for `doli-cli` (single test binary per
//! crate — see .claude/hooks/test-binary-gate.sh). New integration tests are
//! modules here, not top-level `tests/*.rs` files.
//!
//! OUTPUT CONTRACT: N/A — fixture file (module aggregator only, no test logic).
//! INPUT PARTITIONS: N/A — fixture file.

mod inc_i_180_withdrawal_guard;
mod inc_i_188_upgrade_reset_failed_test;
mod inc_i_203_addbond_headroom;
