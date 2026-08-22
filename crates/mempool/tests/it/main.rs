//! ONE integration-test binary for the `mempool` crate.
//!
//! OUTPUT CONTRACT: N/A — fixture file (a target root, not a test). It declares
//! modules and asserts nothing.
//! INPUT PARTITIONS: N/A — fixture file.
//!
//! Cargo compiles every top-level `tests/*.rs` into its OWN executable and
//! macOS Gatekeeper scans every fresh binary on its first exec, so new
//! integration tests are modules of THIS binary.
//! `.claude/hooks/test-binary-gate.sh` enforces the layout.

mod inc_i_180_admission_parity;
