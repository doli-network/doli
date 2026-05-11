# INC-I-069: Conservation Check Regression Analysis

## Triage Verdict

```
━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.90, measured)
Reasoning: Root cause confirmed in prior 4-domain investigation.
           Decision is architectural tradeoff, not bug hunt.
━━━━━━━━━━━━━━━━━━━━━━
```

## Architecture Context

### The Three Fixes (commit 71918154, INC-I-064)

| Fix | What | Where | Status |
|-----|------|-------|--------|
| P0 | Propagate `spend_transaction` errors (was `let _`) | tx_processing.rs:138-154 | Safe, ship |
| P1 | Run ECON_EPOCH_INPUTS_MISMATCH in Light mode | validation_checks.rs:681-725 | Safe, ship |
| P2 | Supply conservation check: `total_value()` before/after | apply_block/mod.rs:143, 190-223 | **REMOVE** |

### Defense Layer Analysis

The question: does removing P2 leave an inflation vector open?

**Layer 1 — Transaction-level balance** (validation/utxo.rs:80, 202):
`validate_transaction_with_utxos` enforces `total_input >= total_output` for every non-coinbase TX. This is the foundational conservation law. It runs for ALL modes.

**Layer 2 — P0: spend_transaction error propagation** (tx_processing.rs:138-154):
Before INC-I-064, `let _ = utxo.spend_transaction(tx)` silently discarded spend failures. When pool UTXOs were already consumed (E362), `add_transaction` created outputs from nothing. P0 fixes this — spend failures now bail apply_block (or warn in Replay mode). This was the **actual root cause** of the 457 DOLI inflation.

**Layer 3 — P1: Pool input verification in Light mode** (validation_checks.rs:681-725):
EpochReward input verification now runs in Full + Light modes (not Replay). Syncing nodes can no longer accept EpochReward TXs with stale/wrong pool inputs. Defense-in-depth against the E362 vector during sync.

**Layer 4 — Existing ECON checks** (validation_checks.rs):
- `ECON_EPOCH_OVERFLOW`: reward total must not exceed pool balance
- `ECON_EPOCH_DISTRIBUTION` (Full mode): exact match of amounts/recipients
- `ECON_EPOCH_NUMBER`: epoch number structural check
- `ECON_EPOCH_NOT_BOUNDARY`: structural check

**Layer 5 — P2: Total supply conservation** (apply_block/mod.rs:143, 190-223):
Generic check: `total_after <= total_before + coinbase_amount`. Catches ANY inflation vector.

### What inflation vector would P2 catch that Layers 1-4 don't?

For supply to inflate, one of these must happen:
1. **TX creates outputs > inputs** — caught by Layer 1 (`total_input < total_output` → `InsufficientFunds`)
2. **spend_transaction silently fails, outputs created from nothing** — caught by Layer 2 (P0)
3. **EpochReward with wrong inputs accepted during sync** — caught by Layer 3 (P1)
4. **EpochReward distributes more than pool** — caught by Layer 4 (`ECON_EPOCH_OVERFLOW`)
5. **Coinbase mints wrong amount** — caught by `validate_block_economics` → `block_reward()` comparison
6. **Bug in add_transaction that creates extra value** — `add_transaction` writes outputs as-is from TX data; no value creation possible unless the TX itself is invalid (caught by Layer 1)

**No realistic inflation vector bypasses Layers 1-4.** P2 is defense-in-depth against an unknown vector.

### P2 Operational Cost

1. **CPU: 2x O(N) full RocksDB iteration per block** — `total_value()` at utxo_rocks.rs:263 iterates ALL UTXOs with bincode deserialization. Called at lines 143 and 190 of apply_block/mod.rs. Combined with existing `serialize_canonical()` for state_root = 3x full UTXO scans per block vs 1x on v6.21.8. On 1-vCPU servers running 3-4 nodes, this caused CPU exhaustion and unresponsiveness.

2. **Atomicity violation** — `bail!()` at line 212 fires AFTER `spend_transaction` and `add_transaction` have written to RocksDB (immediate writes at utxo_rocks.rs:252, utxo_rocks.rs:107). If the check fails, UTXO effects are applied but chain_state is not advanced and the block is not stored → **corrupted state**. This is why N6-N8 couldn't recover after rollback — each failed block deepened the corruption.

3. **No mode gate** — runs during sync catch-up, replay, everything. A node catching up 4,400 blocks does 8,800 full UTXO scans back-to-back.

### Alternative: O(1) Running Total

Could maintain an `AtomicU64` or in-memory counter that tracks total supply, updated on each spend/add. This would make the check O(1). However:
- Requires updating in `spend_transaction`, `add_transaction`, rollback, snap sync restore
- The atomicity problem persists unless UTXO writes become batched
- Adds state that must be kept in sync across 4+ code paths
- Complexity not justified when Layers 1-4 already cover all known vectors

## Recommendation: REMOVE P2

**Remove the conservation check entirely.** Ship P0 + P1 only.

Rationale:
1. P0 + P1 close the specific inflation vector that caused E362
2. Layers 1-4 cover all identifiable inflation vectors
3. P2's O(N) cost is incompatible with the deployment topology (1-vCPU, 3-4 nodes/server)
4. P2's atomicity bug creates worse damage (UTXO corruption) than the bug it's meant to catch
5. The 457 DOLI surplus is already spent/burned — no retroactive correction possible
6. An O(1) replacement adds complexity without addressing a real gap

### If future defense-in-depth is desired:
Add a periodic (not per-block) conservation audit that runs every N blocks or on epoch boundaries, comparing a cached total against actual. This avoids per-block O(N) cost while still catching drift. But this is a future enhancement, not a blocker for the next deploy.

## Milestones

### M1: Remove P2 conservation check
- Delete lines 143 and 179-223 from `apply_block/mod.rs`
- Remove `total_value()` calls from the hot path
- Update the INC-I-064 test file to remove P2-specific test expectations
- Keep P0 and P1 fixes intact

### Acceptance Criteria
- `cargo build --release && cargo clippy -- -D warnings && cargo fmt --check`
- `cargo test -p doli-node --lib` passes
- INC-I-064 test still validates P0 (spend error propagation) and P1 (Light mode validation)
- No `total_value()` calls remain in `apply_block`
