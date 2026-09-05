# Wiring debt

Public functions with no production call site, and the milestone that wires or
deletes them. Verified with `git grep` over `bins/*/src` and `crates/*/src`
(cfg(test) regions excluded) at INC-I-178 M1.

| symbol | file | due | reason |
|---|---|---|---|
| contains_parent | crates/core/src/attestation/pool.rs | test-infra | Redundant in production: the M4 encoder reads `signatures_for` and branches on the `Option`. Retained as the pool-membership accessor the M4 lifecycle tests assert against. |
| encode_attestation_bitfield | crates/core/src/attestation/bitfield.rs | test-infra | Header-variant codec retained only for M0's golden store, which generates `legacy_presence_root_hex` through it. It must outlive the AH pin so the pre-AH byte-identity proof stays verifiable. |
| decode_attestation_bitfield | crates/core/src/attestation/bitfield.rs | test-infra | Same M0 golden-store harness; the last production decode arm was unreachable (`h < BITFIELD_BODY_ACTIVATION_HEIGHT`, constant 0) and was deleted in M1. |
| validate_attestation_bitfield | crates/core/src/attestation/bitfield.rs | test-infra | Same M0 golden-store harness; the body-variant `validate_attestation_bitfield_vec` is the production path. |
| attestation_bls_active | bins/node/src/node/attestation/commit.rs | test-infra | `&NetworkParams` overload. Production reads the gate through the `Node` mirror field and the `_at` variants; this one is the params-level binding `inc_i_178_attestation_bls_activation_height -> gate` that the M4 gate tests (F1) assert, satisfying INV-GOV-001's both-sides requirement against SHIPPED params rather than a test-chosen u64. |
| encoder_universe | bins/node/src/node/attestation/commit.rs | test-infra | Same `&NetworkParams` overload role (F2); `encoder_universe_at` is the production path called from `assembly.rs`. |
| post_commit_universe | bins/node/src/node/attestation/commit.rs | test-infra | Same `&NetworkParams` overload role (F3); `post_commit_universe_at` is the production path called from `apply_block/post_commit.rs`. |
| stray_bit_universe_width | bins/node/src/node/attestation/commit.rs | test-infra | Same `&NetworkParams` overload role (F4); `stray_bit_universe_width_at` is the production path called from `validation_checks.rs`. |
| build_attestation_commitment | bins/node/src/node/attestation/commit.rs | test-infra | Same `&NetworkParams` overload role (F5); `build_attestation_commitment_at` is the production path called from `assembly.rs`. |
