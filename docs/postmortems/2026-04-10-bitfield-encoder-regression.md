# INC-2026-04-10: Bitfield Encoder Regression

## Severity: High
## Date: 2026-04-10
## Affected versions: v6.10.0 — v6.11.1
## Status: Active (fix in progress)

---

## Summary

The bitfield encode fix (v6.10.0, h=2950) changed the attestation encoder to use `epoch_producer_list` instead of `active_producers_at_height`. This was intended to fix a bitfield decode mismatch. Instead, it **broke the fundamental attestation model** by making the bitfield encoder a gatekeeper that excludes any producer not in the scheduler list (max 50).

DOLI's design allows **unlimited producers** to attest and earn rewards — only production (who makes blocks) is limited to 50. The v6.10.0 change conflated these two concepts, making attestation rewards dependent on being in the scheduler.

---

## Timeline

| Time | Event |
|------|-------|
| h=2700 | Bitfield decode fix activated — decoder uses `epoch_producer_list` |
| h=2950 | Bitfield encode fix activated — **encoder also switched to `epoch_producer_list`** |
| h=3353-3384 | 6 new producers registered (zodiacus, isuale, alessandro, isudoajl, santiago, dolifather) |
| h=3600 (E10) | Epoch boundary — 6 new producers passed ACTIVATION_DELAY, added to ProducerSet |
| h=3960 (E11) | Epoch rewards distributed — **6 new producers received 0 rewards despite attesting** |

---

## Root Cause

### The design

DOLI has two separate producer roles:

1. **Scheduler** (`active_production_list`, max 50): determines who produces blocks. Round-robin via `slot % N`. Limited to 50 for network performance.
2. **Attestation** (all active producers): any registered producer with bonds can attest every block and earn bond-weighted epoch rewards. No cap.

These are intentionally decoupled. A network with 500 producers would have 50 producing blocks and 500 attesting. All 500 earn rewards proportional to their bonds and attestation attendance.

### The encoder's role

The block producer encodes a bitfield in each block: "which producers attested this minute?" The bitfield maps sorted producer indices to bits. At epoch boundary, the decoder scans all blocks, decodes the bitfields, and determines who attested how many minutes.

### What went wrong

The original bug (pre-v6.10.0): encoder used `active_producers_at_height(block_height)` which could change mid-epoch when new producers passed ACTIVATION_DELAY. Decoder used a different list → index mismatch → false positives/negatives in attestation → incorrect rewards.

The "fix" (v6.10.0): encoder changed to `epoch_producer_list` — the frozen scheduler list. This made encoder and decoder use the same list, eliminating the index mismatch. **But it also made the bitfield only encode attestations from the 12 producers in the scheduler list.** Any producer not in the scheduler (newly registered, or future >50th producer) is silently dropped from the bitfield. Their attestations exist in gossip but are never recorded on-chain.

### The fundamental error

The fix treated the **scheduler list** as the **attestation list**. These are different concepts:

| Concept | Source | Cap | Purpose |
|---------|--------|-----|---------|
| Scheduler | `active_production_list` | 50 | Who produces blocks |
| Attestation | `active_producers_at_height` | None | Who attests and earns rewards |

By using `epoch_producer_list` (derived from scheduler) for the bitfield encoder, the fix made attestation rewards impossible for any producer outside the scheduler — destroying the economic model for any network with >50 producers.

---

## Impact

### Immediate (v6.10.0 — v6.11.1)

- 6 newly registered producers attested every block but received 0 rewards at epoch boundary
- 60 DOLI in bonds earning no returns
- Producers' attestation visible in gossip but invisible on-chain
- Dashboard showed incorrect attestation data (RPC had same bug, fixed in v6.11.2)

### Architectural (if not fixed)

- Maximum effective producer count limited to 50 (scheduler cap becomes attestation cap)
- No economic incentive for producers beyond the top 50 to stay online
- The "attest to earn" model collapses to "produce to earn"
- Network cannot scale beyond 50 economically viable producers

---

## Cascading failures

The initial bitfield decode bug (correct diagnosis) led to a series of increasingly wrong fixes:

1. **v6.9.0 (h=2700)**: Decoder changed to `epoch_producer_list` — correct for matching encoder's list at the time.
2. **v6.10.0 (h=2950)**: Encoder changed to `epoch_producer_list` — **incorrect**. Conflated scheduler with attestation.
3. **v6.11.0 (h=4100)**: "Onboarding fix" — added newly registered producers to `epoch_producer_list` via 2-epoch lookback. A patch on top of the wrong abstraction.
4. **v6.11.1**: Fixed `init.rs` startup seeding of `epoch_producer_list`. Another patch.
5. **v6.11.2**: Fixed RPC `getAttestationStats` to use correct list. Correct but still treating scheduler list as attestation list.

Each fix addressed a symptom without questioning the root decision: **should the encoder use the scheduler list at all?**

---

## Correct solution

Two separate frozen lists per epoch:

1. `epoch_producer_list` / `active_production_list` (max 50): for scheduler only. Unchanged.
2. `epoch_attestation_list` (all active): for bitfield encoding/decoding and reward calculation. New.

The `epoch_attestation_list` is frozen at epoch boundary from `active_producers_at_height(epoch_boundary_height)`. Both encoder and decoder use this list. It includes ALL active producers, not just the top 50.

This preserves:
- Deterministic bitfield encoding (frozen list, no mid-epoch drift)
- Correct index mapping (encoder and decoder use same list)
- Unlimited attestation (any active producer can attest and earn)
- Scheduler independence (top 50 produce, everyone attests)

---

## Lessons

1. **Understand the model before fixing the code.** The fix author understood the bitfield index mismatch but not the attestation economic model. Asking "what is this list used for?" before changing it would have prevented the error.
2. **Scheduler != Attestation.** These are separate concepts that happen to use similar data structures. Merging them breaks the scaling model.
3. **Each patch increased complexity without solving the root cause.** Four versions (v6.10.0 → v6.11.2) of patches, each fixing a symptom of using the wrong list.
4. **Test with >50 producers.** The bug is invisible with 12 producers. A test with 51+ producers would have immediately shown that producer #51 cannot earn rewards.

---

*Report by: Ivan D. Lozada*
*Date: 2026-04-10*
