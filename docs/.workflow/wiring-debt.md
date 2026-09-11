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
| stray_bit_universe_width | bins/node/src/node/attestation/commit.rs | test-infra | Same `&NetworkParams` overload role (F4); `stray_bit_universe_width_at` is the production path called from `validation_checks/mod.rs`. |
| build_attestation_commitment | bins/node/src/node/attestation/commit.rs | test-infra | Same `&NetworkParams` overload role (F5); `build_attestation_commitment_at` is the production path called from `assembly.rs`. |
| RotateBlsData::decode | crates/core/src/transaction/rotate_bls.rs | M5 | INC-I-217. The 240-byte payload parser. Wired when the M5 stateless verdict reads `extra_data` to validate a `RotateBlsKey` transaction; M4 ships the codec alone, with no validator arm that accepts the type. |
| RotateBlsData::encode | crates/core/src/transaction/rotate_bls.rs | M10 | INC-I-217. The payload writer. Wired by the M10 `doli rotate-bls` CLI, which is the only production caller that BUILDS a rotation; M5-M8 only ever decode. |
| rotation_auth_preimage | crates/core/src/transaction/rotate_bls.rs | M10 | INC-I-217. The ONE published encoder of the Ed25519 authorisation message. Wired by the M10 CLI, which signs it; M5 validation consumes it through `rotation_auth_digest`. Published in M4 so the golden vector that stops the CLI drifting exists before any signer does. |
| rotation_auth_digest | crates/core/src/transaction/rotate_bls.rs | M5 | INC-I-217. BLAKE3-256 over the preimage above. Wired when M5 verifies the inner Ed25519 signature against `ProducerInfo.public_key`. |
| sign_rotation_pop | crates/crypto/src/bls_rotation.rs | M10 | INC-I-217 REQ-ROT-SEC-002. Produces the rotation proof of possession under `ROTATE_POP_DST`. Wired by the M10 CLI, the only production caller that holds the new BLS secret key. |
| verify_rotation_pop | crates/crypto/src/bls_rotation.rs | M5 | INC-I-217 REQ-ROT-SEC-002. Verifies the rotation PoP. Wired when M5 validates the payload; until then the registration PoP path (`bls_verify_pop`) remains the only PoP check in production. |
