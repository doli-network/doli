# Production Gate Deadlock — Root Cause Analysis & Requirements

> **Severity**: Critical — both testnet (h=37406) and mainnet (h=31291) halted permanently
> **Date**: 2026-03-15
> **Status**: Implemented — awaiting deploy (simultaneous, Law #7)
> **Domain**: `crates/network/src/sync/manager/production_gate.rs`

---

## Root Cause Classification

### RC-1: `reset_resync_counter()` is Dead Code (Layer 5)

- **Location**: `production_gate.rs:580`
- **Evidence**: Function exists but is called from **zero locations** in the entire codebase. Confirmed via codebase-wide grep.
- **Mechanism**: `consecutive_resync_count` is incremented by `start_resync()` (line 538, called from `reset_local_state()` in `block_lifecycle.rs:192`) but NEVER decremented or reset.
- **Effect**: Exponential backoff formula at lines 161-162 (`resync_grace_period_secs * (1 << (count - 1).min(4))`) ratchets irreversibly: 30s → 60s → 120s → 240s → 480s.
- **Misleading comment**: `mod.rs` line 640 says "We'll reset the counter after the grace period in can_produce()" — this was never implemented.

### RC-2: Circuit Breaker (Layer 10.5) Has No Recovery Path

- **Location**: `production_gate.rs:460-486`
- **Evidence**: When `local_height >= best_peer_height` and `silence_secs > 50`, the circuit breaker fires permanently. `last_block_received_via_gossip` is only updated by gossip block receipt or full state reset — neither occurs when production is blocked.
- **Mechanism**: `silence_secs` only grows. No timeout, no periodic retry, no peer-agreement check.
- **Irony**: Lines 438-440 explicitly warn about this exact deadlock for Layer 10, then Layer 10.5 reintroduces it.

### RC-3: Gossip Timer Deliberately Excluded on Self-Production

- **Location**: `bins/node/src/node/production/mod.rs:419-422`
- **Evidence**: Explicit comment: "Do NOT call note_block_received_via_gossip() here."
- **Design conflict**: Valid for the isolated-fork case, fatal for the sole-remaining-producer case.

### The Cancer Root (Cross-Layer Interaction)

```
RC-1 (dead reset) → grace periods ratchet → mass producer starvation
         ↓
   Only ~5/13 producers remain active
         ↓
   Unlucky slot scheduling → 50s of no blocks
         ↓
RC-2 (no recovery) → circuit breaker locks ALL remaining producers
         ↓
RC-3 (no self-gossip) → even if one tries to produce, timer doesn't reset
         ↓
   PERMANENT IRRECOVERABLE CHAIN HALT
```

Meta-safety violation: **The production gate can disable ALL producers simultaneously.** Each layer was designed in isolation without considering cross-layer interaction effects.

---

## Requirements

| ID | Requirement | Priority | Rationale |
|----|------------|----------|-----------|
| REQ-PGD-001 | Wire up `reset_resync_counter()` — call it after N consecutive stable block applications | **Must** | Dead code is the root cause of grace period escalation |
| REQ-PGD-002 | Cap `effective_grace` at 60s regardless of `consecutive_resync_count` | **Must** | Prevents producer starvation even if reset fails |
| REQ-PGD-003 | Circuit breaker recovery: when `local_h == peer_h` and peers agree, allow periodic production retry | **Must** | Only fix that prevents permanent deadlock |
| REQ-PGD-004 | Network stall detection: when all peers at same height for >60s, bypass Layer 10.5 | **Should** | Structural fix distinguishing "isolated" from "stalled" |
| REQ-PGD-005 | Self-produced blocks update gossip timer during network stall only | **Should** | Preserves anti-fork protection in normal operation |
| REQ-PGD-006 | Fix misleading comment at `mod.rs:640` | **Must** | Code hygiene — the comment describes dead intent |
| REQ-PGD-007 | Monitoring/metrics for production gate layers | **Could** | Aids future debugging |
| REQ-PGD-008 | Unit tests for cross-layer production gate interactions | **Must** | Prevents regression |

---

## Acceptance Criteria

### REQ-PGD-001: Wire up `reset_resync_counter()`
- Given a node with `consecutive_resync_count=3`, when 5 canonical gossip blocks are applied in sequence, then counter resets to 0
- Given `resync_in_progress=true`, when blocks are applied during sync, then counter is NOT reset
- Call site: `block_applied_with_weight()` after a stability threshold (5 blocks since last resync completion)

### REQ-PGD-002: Cap effective grace period
- Given `consecutive_resync_count=5` and base grace=30s, then `effective_grace = min(480, 60) = 60s`
- Given `consecutive_resync_count=1`, then `effective_grace = 30s` (unchanged, below cap)
- New field: `max_grace_cap_secs: u64` with default 60

### REQ-PGD-003: Circuit breaker recovery
- Given circuit breaker active AND `local_h == peer_h`, when 30s elapsed since activation, then one production attempt is allowed
- If produced block confirmed via gossip within 10s → circuit breaker clears
- If no confirmation → re-engage, next retry in 30s
- If `local_h != peer_h` → no retry (node is behind, not stalled)

### REQ-PGD-004: Network stall detection
- Given `local_h == peer_h` AND no peer has reported higher height for 60s → stall detected
- During stall → Layer 10.5 returns Authorized
- When any peer reports higher height → stall clears immediately

### REQ-PGD-005: Conditional gossip timer on self-production
- During stall → self-produced blocks update `last_block_received_via_gossip`
- Normal operation → self-produced blocks do NOT update (preserve anti-fork behavior)

### REQ-PGD-008: Unit tests
- Test resync counter reset after stable blocks
- Test grace period cap enforcement
- Test circuit breaker fire AND recovery
- Test solo producer in stall vs. isolated fork
- At least 6 test cases in `tests.rs`

---

## Impact Analysis

| File | Change | Risk |
|------|--------|------|
| `production_gate.rs` | Layer 5 grace cap, Layer 10.5 recovery, stall detection | **High** — consensus-critical |
| `mod.rs` | New fields: `max_grace_cap_secs`, `circuit_breaker_retry_at`, `stall_detected_since` | Medium — additive |
| `block_lifecycle.rs` | `block_applied_with_weight()` calls `reset_resync_counter()` after threshold | Medium |
| `production/mod.rs` | Conditional gossip timer update | Medium |
| `event_loop.rs` | No changes | None |

**Deployment**: Simultaneous across all nodes (Law #7). No rolling deploy.

---

## Specs Drift Detected

| Location | Issue |
|----------|-------|
| `mod.rs:640` | Comment says "We'll reset the counter in can_produce()" — never implemented |
| `production_gate.rs:3` | Doc says "11 layers" — actual count is 15 checkpoints across 11+ active layers |

---

## Files Referenced

| File | Relevance |
|------|-----------|
| `crates/network/src/sync/manager/production_gate.rs` | Primary buggy code (Layers 5, 10.5, dead reset) |
| `crates/network/src/sync/manager/mod.rs` | Defaults, fields, misleading comment |
| `crates/network/src/sync/manager/block_lifecycle.rs` | `start_resync()` call, `block_applied_with_weight()` reset point |
| `bins/node/src/node/production/mod.rs` | Gossip timer exclusion |
| `bins/node/src/node/event_loop.rs` | Gossip timer update |
