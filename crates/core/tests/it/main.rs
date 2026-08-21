//! ONE integration-test binary for the `doli-core` crate.
//!
//! OUTPUT CONTRACT: N/A — fixture file (a target root, not a test). It
//! declares modules and asserts nothing.
//! INPUT PARTITIONS: N/A — fixture file.
//!
//! Cargo compiles every top-level `tests/*.rs` into its OWN executable and
//! macOS Gatekeeper scans every fresh binary on its first exec, so a crate
//! that grows one target per test file pays a rebuild tax that nothing in the
//! test output attributes. New integration tests are modules of THIS binary;
//! the legacy top-level files stay where they are until they are migrated.
//! `.claude/hooks/test-binary-gate.sh` enforces the layout.

mod inc_i_180_activation_height;
