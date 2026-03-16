# Incident Report: Phantom Delegation Reward Split in Production Code

**Date:** 2026-03-15
**Severity:** Medium (correctness risk) / High (design risk)
**Networks affected:** All (constants compiled into binary)
**Author:** I. Lozada

---

## Summary

Production code contains a hardcoded 10%/90% delegation reward split (`DELEGATE_REWARD_PCT=10`, `STAKER_REWARD_PCT=90`) that serves no purpose. There are no delegation transactions active, no Tier system enabled, and no staker category of nodes. The constants and associated reward logic are dead code that was added speculatively as part of the 3-tier scalability plan but never gated behind a feature flag or era activation.

## Impact

1. **Dead code in critical path**: `rewards.rs:225` executes the delegation split formula on every epoch boundary. Currently `delegated_share` is always 0, so the result is harmless — but the code path exists and runs.
2. **False documentation**: `specs/protocol.md:672`, `docs/rewards.md`, and `transaction/types.rs` all describe delegation as if it's a live feature, creating confusion about what the network actually does.
3. **Hardcoded economics**: The 10% fee is a constant, not a producer-configurable parameter. When delegation is actually implemented, this must be rethought — most PoS networks let validators set their own commission rate.

## Affected Files

| File | What | Line(s) |
|------|------|---------|
| `crates/core/src/consensus/constants.rs` | Constants defined | 395-398 |
| `bins/node/src/node/rewards.rs` | Split formula executed | 225 |
| `bins/node/src/node/mod.rs` | Import | 35 |
| `crates/core/src/lib.rs` | Re-export | 180, 212 |
| `crates/core/src/transaction/types.rs` | Delegation tx type docs | 49-50 |
| `specs/protocol.md` | Describes delegation as live | 672 |
| `docs/rewards.md` | Documents split as active | 75, 183, 192, 296 |
| `docs/legacy/SCALABILITY_PLAN.md` | Origin of the constants | 473-474, 766-767 |

## Root Cause

The 3-tier scalability plan (`SCALABILITY_PLAN.md`) defined these constants as part of a future delegation system. They were merged into production code and documentation without being gated behind an era check or feature flag. No delegation transaction type is currently processable by `apply_block()`, so the split never triggers on real data, but the infrastructure pretends it exists.

## Required Changes

### Phase 1 — Clean up (pre-delegation, safe to do now)
1. Remove `DELEGATE_REWARD_PCT` and `STAKER_REWARD_PCT` from `constants.rs`
2. Remove the delegation split branch from `rewards.rs` (the `delegated_share` path)
3. Remove imports/re-exports from `mod.rs` and `lib.rs`
4. Update `specs/protocol.md` and `docs/rewards.md` to mark delegation as **planned, not active**
5. Update `transaction/types.rs` docstrings to mark Delegation tx as reserved/future

### Phase 2 — Redesign (when delegation is actually implemented)
1. Commission rate should be a per-producer configurable parameter (stored on-chain), not a global constant
2. Delegation must be gated behind an era activation (e.g., Era 2+)
3. Reward split must be validated in `validation.rs` (currently disconnected — see MEMORY.md)

## Does This Affect the 3-Tier Architecture?

**No.** The 3-tier architecture (Tier 1 Validators, Tier 2 Attestors, Tier 3 Stakers) depends on:
- Tier assignment by effective weight (implemented in `tiers.rs`)
- Gossip topic routing per tier/region (implemented in `gossip/config.rs`)
- BLS aggregate attestations (primitives exist in `crypto/bls.rs`, not wired)
- Epoch boundary tier recomputation (not implemented)

The reward split is purely a distribution formula. Removing it does not touch network topology, gossip structure, or consensus. When delegation goes live, the split can be reintroduced as a proper per-producer parameter.

## Action Items

- [ ] Phase 1 cleanup — remove dead delegation split code and update docs
- [ ] Audit for other speculative constants from SCALABILITY_PLAN.md that leaked into production
- [ ] Design per-producer commission parameter for future delegation feature
