# INC-I-034 — M-RC11: FORK_GUARD backfill invariant in `execute_reorg`

**Date:** 2026-04-16
**Branch:** `synmgrefactor`
**Files touched:**
- `bins/node/src/node/block_handling.rs` (fix `execute_reorg` rollback anchor)
- `crates/storage/src/block_store/queries.rs` (new `ensure_blocks_present` helper)
- `crates/storage/src/block_store/tests.rs` (unit tests for the helper)

**Tests:** `bins/node/tests/m_rc11_fork_guard_backfill_regression.rs` — 3/3 PASS (A anchor + B primary + C no-op). Fork recovery canonical suite (`tests/fork_recovery.rs`) — 11/11 still PASS.
**Incident:** INC-I-034 (live mainnet cascade 2026-04-16 05:11 UTC, santiago / ivan / seed3 on ai3).
**Prior milestones:** M-RC9 (`inc-i-034-m-rc9-silent-vec-fix.md`), M-RC10 (`inc-i-034-m-rc10-apply-after-reject-fix.md`).
**Requirement:** REQ-REDESIGN-011 (chain_state↔block_store completeness invariant).

## Symptom (replayed from the 2026-04-16 cascade)

```
05:11:14.470Z  [BLOCK] Applied h=39599 hash=602990400e... (canonical winner)
05:11:14.874Z  [FORK_GUARD] Dropping fork block ed9bab0b at h=39599
               — canonical 602990400e exists                    (block_handling.rs:90)
05:11:31.916Z  Empty headers from peers (network now at h=39603,
               local stuck at 39599 — peers blacklisted as fork-evidence)
... santiago accepts forward gossip but never re-fetches 39600-39628 ...
Result: santiago tip catches up but block_store has a permanent
        gap at 39600-39628. Cascading peer blacklist from empty
        GetHeaders(best_hash) responses.
```

The test-writer's audit identified that the visible `[FORK_GUARD]` drop at
`block_handling.rs:90` (in `Node::handle_new_block`) is **correct** — it
rejects a competing gossip block at an already-occupied canonical height.
The actual defect lives in the reorg path that fires when the fork chain is
**heavier**: `Node::execute_reorg`, same file, at lines 399-413 (pre-fix).

## Root cause

```rust
// bins/node/src/node/block_handling.rs, pre-fix (399-413)
let common_ancestor_block = if target_height == 0 {
    None
} else {
    self.block_store.get_block_by_height(target_height)?   // (A)
};

let genesis_hash = self.chain_state.read().await.genesis_hash;
let common_ancestor_hash = common_ancestor_block
    .as_ref()
    .map(|b| b.hash())
    .unwrap_or(genesis_hash);                              // (B) SILENT BUG
let common_ancestor_slot = common_ancestor_block
    .as_ref()
    .map(|b| b.header.slot)
    .unwrap_or(0);
```

At (A), `get_block_by_height` can legitimately return `Ok(None)` if the
block at `target_height` is not present in `block_store`:

- a prior partial reorg truncated the index above `target_height`
- snap sync raced with archiver pruning
- an earlier chain_state advance wrote past a known gap (M-RC10 predecessor)

At (B), the missing block is **silently substituted with `genesis_hash`**.
The rollback then executes (lines 462-466 in the undo path, 487-488 in the
legacy fallback):

```rust
state.best_height = target_height;     // e.g. 5
state.best_hash   = common_ancestor_hash;   // = genesis_hash (BUG)
state.best_slot   = common_ancestor_slot;   // = 0
```

`chain_state` is now corrupt: `best_height > 0` but `best_hash` is the
genesis anchor. `block_store.get_block_by_height(best_height)` returns
`None`. Every downstream path that reads `best_hash` sees a value that has
no block in local storage:

- gossip eligibility checks fail against this phantom anchor
- `set_canonical_chain()` in `apply_block` walks backwards via `prev_hash`
  and crashes or silently short-circuits when the parent is not in the store
- sync broadcasts advertise a `best_hash` no peer has ever seen
- `GetHeaders(best_hash)` returns empty from every peer → scoring treats the
  silence as fork evidence → peer blacklist cascade

That is **exactly** the santiago / ivan / seed3 pattern from the 2026-04-16
investigation (see `docs/.workflow/blockchain-investigation-consensus.md`
and the "Empty headers from peers" signature at 05:11:31.916Z).

The test-writer's deeper finding: even when `apply_block` later runs on top
of the corrupt anchor, the bad state propagates. `apply_block` computes the
new height as `chain_state.best_height + 1` **without** re-verifying that
`block.prev_hash == chain_state.best_hash`. A reorg that writes a corrupt
anchor and then attempts to apply new blocks on top ratchets the corruption
downstream — the post-reorg state seen in test B is
`(best_height=6, best_hash=b6_hash)` with `block_store[6] = None`, because
the follow-up `set_canonical_chain` walk fails silently when it reaches the
missing parent.

Sibling violation at lines 467-517 (legacy undo-missing fallback): same
silent substitution at line 487, plus a `utxo.clear()` + rebuild loop
that walks `block_store.get_block_by_height(h).ok().flatten()` for
`1..=target_height` — any mid-chain gap is silently skipped and the UTXO
set is structurally corrupted to match the corrupt `chain_state`.

## REQ-REDESIGN-011 (invariant restated)

> After ANY mutation of `chain_state.best_hash`, the system MUST guarantee
> that:
>
> 1. `block_store.get_block_by_height(chain_state.best_height)` returns a
>    block whose hash equals `chain_state.best_hash`, AND
> 2. Every height in `1..=chain_state.best_height` is retrievable from
>    `block_store` (no mid-chain gap).
>
> If either condition cannot be satisfied, the switch MUST NOT occur —
> `chain_state` stays on the OLD canonical until backfill completes.

## Fix

### Layer 1 — `BlockStore::ensure_blocks_present`

New helper in `crates/storage/src/block_store/queries.rs`:

```rust
pub fn ensure_blocks_present(&self, low: u64, high: u64) -> Result<(), StorageError> {
    if low > high {
        return Ok(());
    }
    let start = low.max(1); // genesis (h=0) is not in height_index
    for h in start..=high {
        if self.get_hash_by_height(h)?.is_none() {
            return Err(StorageError::NotFound(format!(
                "[FORK_GUARD_BACKFILL] block_store missing canonical block at \
                 height {} (range checked: {}..={})",
                h, start, high
            )));
        }
    }
    Ok(())
}
```

Design notes:

- **Point lookups only** against the height index column family — no
  header/body deserialization. O(range) against a hot CF.
- **First-missing diagnostics**: the error names the first missing height
  so operators and sync recovery can target the backfill precisely.
- **Genesis tolerance**: `low == 0` is silently rounded up to 1, because
  genesis is a chain anchor, not a stored block, on non-snap-sync nodes.
- **Empty-range tolerance**: `low > high` is a vacuous success. This lets
  callers pass `(1, target_height)` without branching on `target_height == 0`.

Four unit tests added (in `tests.rs`): empty-range, low-zero-tolerance,
dense-range, and first-missing-height diagnostics.

### Layer 2 — `execute_reorg` anchor hardening

Pre-flight check inserted immediately before the silent substitution site.
If the rolled-back range is not dense in `block_store`, `execute_reorg`
bails with a clear `[FORK_GUARD_BACKFILL_REQUIRED]` error **before** any
`chain_state` mutation:

```rust
self.block_store
    .ensure_blocks_present(1, target_height)
    .map_err(|e| {
        error!(
            "[FORK_GUARD_BACKFILL_REQUIRED] Reorg refused: \
             block_store missing canonical blocks in 1..={} — {}. \
             chain_state.best_hash NOT advanced. Backfill required.",
            target_height, e
        );
        anyhow::anyhow!(
            "[FORK_GUARD_BACKFILL_REQUIRED] block_store incomplete in \
             range 1..={}: {}",
            target_height, e
        )
    })?;
```

The existing call site in `handle_new_block` (line 169-171) already wraps
`execute_reorg` in `if let Err(e) = ... { error!("Failed to execute reorg: {}", e); }`,
so returning `Err` here means:

1. `chain_state` is untouched (the guard fires before any `write().await`
   on `chain_state`).
2. The error is logged with a distinctive `[FORK_GUARD_BACKFILL_REQUIRED]`
   tag for operator triage.
3. Normal header-first sync will fill the gap on the next request and the
   reorg can succeed on the subsequent attempt.

The silent `unwrap_or(genesis_hash)` at the anchor resolution site is
replaced with an explicit `match` that returns `Err` defensively if
`get_block_by_height(target_height)` returns `None` despite
`ensure_blocks_present` having passed (belt-and-braces against a concurrent
prune between the two calls):

```rust
let common_ancestor_block = if target_height == 0 {
    None
} else {
    match self.block_store.get_block_by_height(target_height)? {
        Some(b) => Some(b),
        None => {
            error!(
                "[FORK_GUARD_BACKFILL_REQUIRED] Reorg refused: \
                 common ancestor at h={} missing from block_store \
                 after completeness check. chain_state.best_hash NOT advanced.",
                target_height
            );
            anyhow::bail!(
                "[FORK_GUARD_BACKFILL_REQUIRED] common ancestor at \
                 h={} missing from block_store",
                target_height
            );
        }
    }
};
```

The `unwrap_or(genesis_hash)` / `unwrap_or(0)` suffixes on the subsequent
`common_ancestor_hash` and `common_ancestor_slot` derivations are RETAINED,
because after the guards above they are reached only when
`target_height == 0` — the legitimate full-rollback-to-genesis case — which
is REQ-REDESIGN-011-compliant by construction (genesis is always present
and is anchor, not block).

### Layer 3 — write-order atomicity

Not required: `apply_block` already orders `put_block` + `set_canonical_chain`
**before** `chain_state.update(...)` (see `bins/node/src/node/apply_block/mod.rs:230-231`
and `.../state_update.rs:47-48`). Block store is populated first, chain state
second. No change needed at this layer.

## Test evidence

### Pre-fix baseline (test B)

```
test test_b_deeper_reorg_with_missing_ancestor_preserves_invariant ... FAILED

P2/REQ-REDESIGN-011 VIOLATION: execute_reorg mutated chain_state in the
face of a missing common ancestor.
Pre  state: best_height=10 best_hash=392de41ff5b6e2af...
Post state: best_height=6  best_hash=0be829e704d69ea0... (downstream
            propagation of the corrupt anchor via follow-up apply_block)
Invariant violation: O2 VIOLATION: chain_state.best_height=6 but
  block_store.get_block_by_height(6) = None.
```

### Post-fix

```
running 3 tests
test test_c_reorg_with_missing_new_block_does_not_advance_chain_state ... ok
test test_a_simple_tip_reorg_preserves_invariant ... ok
test test_b_deeper_reorg_with_missing_ancestor_preserves_invariant ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

- **Test A** (regression anchor, tip reorg with parent present) — PASS.
- **Test B** (deeper reorg, common ancestor deleted) — **FAIL → PASS**.
  `execute_reorg` now returns `Err([FORK_GUARD_BACKFILL_REQUIRED] ...)`
  without mutating `chain_state`. `post == pre` holds; `best_height=10`,
  `best_hash` unchanged.
- **Test C** (missing new block, pre-existing early-exit) — PASS (not
  regressed by the fix).

### Canonical regression suite

- `bins/node/tests/fork_recovery.rs` — **11/11 PASS**. Legitimate reorgs
  with a dense `block_store` continue to succeed; the pre-flight
  `ensure_blocks_present` check is O(rollback_depth) point lookups against
  a hot CF and adds no observable latency.
- `bins/node/tests/m_rc9_silent_vec_regression.rs` — **3/3 PASS**.
- `bins/node/tests/m_rc10_apply_after_reject_regression.rs` — **4/4 PASS**.
- `crates/storage --lib` — **170/170 PASS** (includes the 4 new
  `ensure_blocks_present` unit tests).
- `bins/node --lib` — **10/10 PASS**.
- `bins/node --test test_network` — **13/13 PASS** (12 ignored).
- `bins/node --test epoch_reward_explicit_inputs` — **7/7 PASS**.
- `cargo build --release -p doli-node` — **clean**.
- `cargo clippy -p doli-node -p storage -- -D warnings` — **clean**.

## Why this does not regress the common case

The pre-flight check `ensure_blocks_present(1, target_height)` is a
point-lookup scan against the `height_index` CF. In steady-state operation,
every height in `1..=target_height` has been written by a successful
`apply_block` (which always runs `put_block` + `set_canonical_chain` before
advancing `chain_state`). The check is therefore O(rollback_depth) hot
reads and returns `Ok(())` unconditionally on a healthy node. The only
time it fires an error is when `block_store` has already diverged from
`chain_state` — which is, by REQ-REDESIGN-011, exactly the state in which
a reorg must be refused.

The 11-test `fork_recovery` canonical suite exercises reorgs, rollbacks,
recovery-after-cap, and post-snap validation; all 11 still pass after the
fix, confirming that legitimate reorgs with a dense block_store continue
to flow through the rollback path unchanged.

## Operator-visible surface

New log line on the refused-switch path:

```
[FORK_GUARD_BACKFILL_REQUIRED] Reorg refused: block_store missing canonical
  blocks in 1..=<target_height> — [FORK_GUARD_BACKFILL] block_store missing
  canonical block at height <h> (range checked: 1..=<target_height>).
  chain_state.best_hash NOT advanced. Backfill required before this reorg
  can proceed.
```

When an operator sees this line, the correct response is:

1. Note the first missing height `<h>` reported in the error.
2. Trigger a header-first sync backfill from a known-good peer. The
   standard `verifyChainIntegrity` + `backfillFromPeer <rpc-url>` flow
   (see `MEMORY.md` #1 rule) is directly applicable.
3. The next reorg attempt will succeed automatically once the gap closes.

This is strictly safer than the pre-fix silent corruption. A stuck node
advertising its OLD canonical tip is network-benign; a node advertising a
phantom anchor is a partition vector (the santiago cascade is the proof).

## Deployment checklist

Per `CLAUDE.md`:

- [x] `cargo build --release && cargo clippy -- -D warnings && cargo fmt --check`
  — build and clippy clean on `doli-node` + `storage`; fmt drift is in
  pre-existing test files the Developer must not modify.
- [x] `cargo test -p doli-node --lib`, `-p storage --lib` — all pass.
- [ ] **No protocol version bump required.** This is a correctness fix
  that restricts when `chain_state.best_hash` advances; no wire format,
  message, or cross-node semantic change. Old and new binaries remain
  interoperable at the protocol level.
- [ ] **No hard fork schedule entry.** The behavior change is local to
  the node's reorg safety and does not alter block validity or consensus
  outcomes on a healthy chain.
- [ ] Spec/docs alignment: this file is the changelog entry. No separate
  `specs/protocol.md` or `docs/architecture.md` update needed — the fix
  tightens an internal invariant that was already implicit in the
  architecture; the external contract is unchanged.
- [ ] Testnet first, mainnet behind explicit confirmation.
