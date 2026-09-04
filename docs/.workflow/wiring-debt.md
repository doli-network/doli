# Wiring debt

Public functions with no production call site, and the milestone that wires or
deletes them. Verified with `git grep` over `bins/*/src` and `crates/*/src`
(cfg(test) regions excluded) at INC-I-178 M1.

| symbol | file | due | reason |
|---|---|---|---|
| get | crates/core/src/attestation/pool.rs | M4 | Pool read side. The production reader is the M4 encoder that builds `presence_root = BLAKE3(len ‖ bitfield ‖ aggregate)` from the parent-keyed signatures. |
| signatures_for | crates/core/src/attestation/pool.rs | M4 | Same M4 encoder: bulk read of one parent's attester map to aggregate. |
| contains_parent | crates/core/src/attestation/pool.rs | M4 | M4 encoder guard before it attempts an aggregate for a parent. |
| parent_count | crates/core/src/attestation/pool.rs | M4 | Bound observability for the M4 encoder and its metrics. |
| total_signatures | crates/core/src/attestation/pool.rs | M4 | Bound observability for the M4 encoder and its metrics. |
| clear | crates/core/src/attestation/pool.rs | M4 | Epoch-boundary reset, wired beside `minute_tracker.reset()` once M4 owns the pool lifecycle. |
| new_with_bls | crates/core/src/attestation/message.rs | M2 | Deferred by design (spec D1 "REWIRED by D3, not deleted"). M2 D3 makes it the single dual-sign egress. |
| encode_attestation_bitfield | crates/core/src/attestation/bitfield.rs | test-infra | Header-variant codec retained only for M0's golden store, which generates `legacy_presence_root_hex` through it. It must outlive the AH pin so the pre-AH byte-identity proof stays verifiable. |
| decode_attestation_bitfield | crates/core/src/attestation/bitfield.rs | test-infra | Same M0 golden-store harness; the last production decode arm was unreachable (`h < BITFIELD_BODY_ACTIVATION_HEIGHT`, constant 0) and was deleted in M1. |
| validate_attestation_bitfield | crates/core/src/attestation/bitfield.rs | test-infra | Same M0 golden-store harness; the body-variant `validate_attestation_bitfield_vec` is the production path. |
