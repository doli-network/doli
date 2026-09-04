# Wiring debt

Public functions with no production call site, and the milestone that wires or
deletes them. Verified with `git grep` over `bins/*/src` and `crates/*/src`
(cfg(test) regions excluded) at INC-I-178 M1.

| symbol | file | due | reason |
|---|---|---|---|
| signatures_for | crates/core/src/attestation/pool.rs | M4 | Same M4 encoder: bulk read of one parent's attester map to aggregate. |
| contains_parent | crates/core/src/attestation/pool.rs | M4 | M4 encoder guard before it attempts an aggregate for a parent. |
| parent_count | crates/core/src/attestation/pool.rs | M4 | Bound observability for the M4 encoder and its metrics. |
| total_signatures | crates/core/src/attestation/pool.rs | M4 | Bound observability for the M4 encoder and its metrics. |
| clear | crates/core/src/attestation/pool.rs | M4 | Epoch-boundary reset, wired beside `minute_tracker.reset()` once M4 owns the pool lifecycle. |
| encode_attestation_bitfield | crates/core/src/attestation/bitfield.rs | test-infra | Header-variant codec retained only for M0's golden store, which generates `legacy_presence_root_hex` through it. It must outlive the AH pin so the pre-AH byte-identity proof stays verifiable. |
| decode_attestation_bitfield | crates/core/src/attestation/bitfield.rs | test-infra | Same M0 golden-store harness; the last production decode arm was unreachable (`h < BITFIELD_BODY_ACTIVATION_HEIGHT`, constant 0) and was deleted in M1. |
| validate_attestation_bitfield | crates/core/src/attestation/bitfield.rs | test-infra | Same M0 golden-store harness; the body-variant `validate_attestation_bitfield_vec` is the production path. |
| attestation_universe | crates/core/src/attestation/universe.rs | M4 | D5 canonical bitfield universe. M3 ships the shared pure fn and its proof only; the four hand-rolled call sites (assembly.rs:423, post_commit.rs:52, schedule.rs:254/375) switch to it in M4 behind `inc_i_178_attestation_bls_activation_height`, because unifying the widths is consensus-visible (INV-DEPLOY-001). |
