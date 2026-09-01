//! ONE integration-test binary for the `doli-node` crate.
//!
//! OUTPUT CONTRACT: N/A — fixture file (a target root, not a test). It
//! declares modules and asserts nothing.
//! INPUT PARTITIONS: N/A — fixture file.
//!
//! Cargo compiles every top-level `tests/*.rs` into its OWN executable and
//! macOS Gatekeeper scans every fresh binary on its first exec, so a crate
//! that grows one target per test file pays a rebuild tax that nothing in the
//! test output attributes. New integration tests are modules of THIS binary;
//! the ~60 legacy top-level files stay where they are until they are migrated.
//! `.claude/hooks/test-binary-gate.sh` enforces the layout.

mod inc_i_180_allowance_parity;
mod inc_i_180_apply_block_utxo_destruction;
mod inc_i_180_builder_parity;
mod inc_i_180_common;
mod inc_i_180_drain_everything;
mod inc_i_180_gate_bindings;
mod inc_i_180_holdings_fallback;
mod inc_i_180_in_block_parity;
mod inc_i_180_rebuild_parity;
mod inc_i_180_replay_mode;
mod inc_i_180_withdrawal_holdings_gate;
mod inc_i_204_m0_common;
mod inc_i_204_m0_export_paths;
mod inc_i_204_m0_fork_guard_sites;
mod inc_i_204_m0_wedge_alarm;
mod inc_i_204_m41_common;
mod inc_i_204_m41_metrics;
mod inc_i_204_m41_refusals;
mod inc_i_204_m41_rescue;
mod tied_fork_finality;
