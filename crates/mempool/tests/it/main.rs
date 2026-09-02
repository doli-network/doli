//! ONE integration-test binary for the `mempool` crate.
//!
//! OUTPUT CONTRACT: N/A — target root. It declares modules and hosts the
//! INC-I-203 M2 probe, which asserts nothing (it is a measurement).
//! INPUT PARTITIONS: N/A — fixture file.
//!
//! Cargo compiles every top-level `tests/*.rs` into its OWN executable and
//! macOS Gatekeeper scans every fresh binary on its first exec, so new
//! integration tests are modules of THIS binary.
//! `.claude/hooks/test-binary-gate.sh` enforces the layout.

mod inc_i_180_admission_parity;
mod inc_i_203_addbond_cap;
mod inc_i_203_admission_gap;

/// INC-I-203 M2 outcome metric. Root-level because libtest's `--exact` matches
/// the full `module::fn` name and the milestone command addresses it bare.
/// Asserts nothing: it must run on the buggy AND the fixed tree, reporting `1`
/// today and `0` after.
#[test]
fn inc_i_203_m2_probe_resident_over_cap_addbonds() {
    println!(
        "INC-I-203-M2-RESIDENT={}",
        inc_i_203_admission_gap::probe_resident_over_cap_addbonds()
    );
}
