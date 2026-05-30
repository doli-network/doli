# SPEC-002 Manual Review — Heartbeat Preimage Collision

**Audit reference:** `docs/audits/security-audit-oracle-2026-05-29.md` → SPEC-002
**Original concern:** the `crypto::hash::hash()` calls in `heartbeat.rs` might produce a preimage byte-pattern interchangeable with a `PriceAttestation` signing message, enabling cross-type signature reuse.

## Heartbeat signing preimages (verified by reading code 2026-05-30)

### `crates/core/src/heartbeat.rs` (production "presence" heartbeat)
```rust
hash_concat(&[
    b"DOLI_HEARTBEAT_SIGN_V1",  //   21 bytes — explicit domain
    &[version],                  //    1 byte
    producer.as_bytes(),         //   32 bytes
    &slot.to_le_bytes(),         //    8 bytes
    prev_hash.as_bytes(),        //   32 bytes
    vdf_output,                  //   32 bytes
])
```
- **Preimage length:** 126 bytes
- **Has domain prefix:** YES (`b"DOLI_HEARTBEAT_SIGN_V1"`)

### `crates/core/src/tpop/heartbeat.rs` (`PresenceHeartbeat` struct)
```rust
hash_concat(&[
    &[version],                  //    1 byte
    &slot.to_le_bytes(),         //    8 bytes
    prev_hash.as_bytes(),        //   32 bytes
    vdf_output,                  //   32 bytes
])
```
- **Preimage length:** 73 bytes
- **Has domain prefix:** NO

## PriceAttestation signing preimage (post-AUDIT-P2-002)
```rust
[b"PRICE_ATTESTATION_V1"        //   20 bytes — domain
 || pair_id                      //   32 bytes
 || price_cents.to_le_bytes()    //    8 bytes
 || epoch_number.to_le_bytes()]  //    8 bytes
```
- **Preimage length:** 68 bytes (was 48 pre-fix)

## Collision analysis

| Preimage | Length | Collides with PriceAttestation (68B post-fix)? | Collides with pre-fix PA (48B)? |
|---|---|---|---|
| heartbeat.rs | 126 | NO (length differs) | NO |
| tpop/heartbeat.rs | 73 | NO (length differs) | NO |

**Verdict:** No structural collision exists between any current heartbeat signing message and the PriceAttestation signing message — neither pre- nor post-AUDIT-P2-002. The original SPEC-002 concern is unfounded.

## Independent finding: tpop/heartbeat.rs lacks a domain prefix

While not a PriceAttestation-collision risk, the `PresenceHeartbeat::signing_message` at `crates/core/src/tpop/heartbeat.rs:211` is missing a domain prefix — a defense-in-depth gap independent of the oracle audit. The presence heartbeat in `crates/core/src/heartbeat.rs` uses `b"DOLI_HEARTBEAT_SIGN_V1"`; `tpop/heartbeat.rs` should follow the same convention for any future preimage of length 73 that could collide.

**Severity:** P3 (no known live collision today; future-proofing only)
**Not in scope for the oracle audit** — filed here for visibility. Should be addressed under a separate cleanup pass (`/omega-audit --scope=crates/core/src/tpop`) if/when tpop work resumes.

## Resolution

SPEC-002: **RESOLVED — no collision found.** No code change required for the oracle audit. Independent tpop/heartbeat domain-prefix gap noted for future cleanup.
