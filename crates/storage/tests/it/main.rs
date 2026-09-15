//! ONE integration-test binary for the `doli-storage` crate.
//!
//! OUTPUT CONTRACT: N/A — fixture file (a target root, not a test). It
//! declares modules and asserts nothing.
//! INPUT PARTITIONS: N/A — fixture file.
//!
//! Cargo compiles every top-level `tests/*.rs` into its OWN executable and
//! macOS Gatekeeper scans every fresh binary on its first exec, so a crate
//! that grows one target per test file pays a rebuild tax that nothing in the
//! test output attributes. New integration tests are modules of THIS binary.
//! `.claude/hooks/test-binary-gate.sh` enforces the layout.

#[path = "../common/mod.rs"]
mod common;

mod inc_i_171_m1_dead_impl_tripwire;
mod inc_i_171_m3_three_site_tripwire;
mod inc_i_171_m3_withdrawal_inputs;
mod inc_i_180_withdrawal_holdings;
mod m1_state_root_decode_seam;
mod m2_caller_port_locks;
mod m3_chunk_byte_equality;
mod m4_chunked_promotion;
