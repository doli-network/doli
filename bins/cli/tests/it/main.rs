//! Consolidated integration-test binary for `doli-cli` (single test binary per
//! crate — see .claude/hooks/test-binary-gate.sh). New integration tests are
//! modules here, not top-level `tests/*.rs` files.
//!
//! OUTPUT CONTRACT: N/A — fixture file (module aggregator only, no test logic).
//! INPUT PARTITIONS: N/A — fixture file.

mod inc_i_171_m7_ladder_single_source;
mod inc_i_180_withdrawal_guard;
mod inc_i_188_upgrade_reset_failed_test;
mod inc_i_203_addbond_headroom;
mod inc_i_214_release_sign_verify;
mod inc_i_215_docs_tripwire;
mod inc_i_215_fixture;
mod inc_i_215_from_staged;
mod inc_i_215_helper_units_test;
mod inc_i_217_import_bls_golden;
mod inc_i_217_m10_rotate_bls_golden;
mod inc_i_217_m10_wallet_untouched;
