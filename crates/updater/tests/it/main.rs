//! Consolidated integration-test binary for `updater` (one test binary per crate — see
//! .claude/hooks/test-binary-gate.sh). New integration tests are modules here, not
//! top-level `tests/*.rs` files.
//!
//! OUTPUT CONTRACT: N/A — fixture file (module aggregator only, no test logic).
//! INPUT PARTITIONS: N/A — fixture file.

mod sk_m1_outcome_probe;
mod sk_m1_skills_dir_resolution;
mod sk_m1_skills_ownership;
