//! ONE integration-test binary for the `network` crate.
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

mod inc_i_178_m2_scoring;
mod inc_i_204_m0_decision_pins;
mod inc_i_237_flood;
mod inc_i_237_loop_deadlock;
mod inc_i_237_m0_api;
mod inc_i_237_m0_corpus;
mod inc_i_237_m0_corpus_vectors;
mod inc_i_237_m0_sec005;
mod inc_i_237_m0_swarm;
mod inc_i_237_m15_contract;
mod inc_i_237_m1_block_gate;
mod inc_i_237_m1_contract;
mod inc_i_237_m1_relay_table;
mod inc_i_237_m2_common;
mod inc_i_237_m2_fake_node;
mod inc_i_237_m2_line;
mod inc_i_237_m2_origin;
mod inc_i_237_m2_verdict_path;
mod inc_i_237_m3_fake_node;
mod inc_i_237_m3_flood;
mod inc_i_237_m3_line;
mod inc_i_237_m3_relay_table;
mod inc_i_237_m4_doc_drift;
mod inc_i_237_m4_lane_bounds;
mod inc_i_237_m4_source_pins;
mod inc_i_237_m4_vendor_dedup;
mod inc_i_237_m5_attest_budget_line;
mod inc_i_237_m5_channel_ceiling_doc;
mod inc_i_237_m5_disclosure;
mod inc_i_237_m5_tx_budget_line;
mod inc_i_237_tx_relay;
mod m3_protocol_session;
mod m4_sink_seam;
mod m4_wire_unchanged;
