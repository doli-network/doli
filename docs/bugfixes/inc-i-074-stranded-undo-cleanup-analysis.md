# Analyst Analysis — INC-I-074 (INC-I-071 followup)

**Incident:** INC-I-074
**Scope:** `crates/storage/src/state_db/undo.rs`, `bins/node/src/node/init.rs`
**Branch:** `main`
**Run ID:** 314

## Architecture Context

`cf_undo` is a single RocksDB column family inside `StateDb` (`crates/storage/src/state_db/`). It holds one `UndoData` entry per applied block, keyed by little-endian 8-byte block height. Three lifecycle operations exist today:

- **`put_undo`** — appends an entry inside the block-apply WriteBatch. Producer for the data.
- **`prune_undo_before(keep)`** — called every block after commit (`apply_block/mod.rs:347`). Forward-only single-entry delete + periodic compact. Designed for monotonic retention-window advance.
- **`prune_undo_above(keep)`** — bulk-delete used by truncation/rollback. Iterates the whole CF and batches deletes for `height > keep`.

INC-I-071 reduced `UNDO_KEEP_DEPTH` from 2000 to 360. The reduction is one-shot and discrete — it permanently shrinks the retention window. `prune_undo_before` only walks forward in 1-step increments, so the historic tail `[H_deploy - 1999 .. H_deploy - 361]` is stranded forever in RocksDB SSTs. Mainnet measurements confirm: state_db at 562 MB vs ~10 MB expected on N4/N5.

**Blast radius:** local-only. cf_undo is never on the wire, never in a state root, never in any block content. The fix is a one-shot startup operation in `init.rs` that calls a new bulk-delete method symmetric with `prune_undo_above`. No consensus rules, no block content, no protocol version, no activation height.

## Requirements

| ID | Priority | Requirement | Acceptance Criteria |
|----|----------|-------------|---------------------|
| REQ-I-074-001 | Must | New method `StateDb::prune_undo_below(keep_height)` bulk-deletes all cf_undo entries with `height < keep_height`. | Unit test P1: insert keys [0..=10], call `prune_undo_below(5)`, verify entries 0..=4 gone and 5..=10 retained. Returns count deleted. |
| REQ-I-074-002 | Must | `prune_undo_below` is idempotent: a second call on a clean DB returns 0 and does nothing harmful. | Unit test P2: same as P1, call again, verify return = 0, entries 5..=10 still retained. |
| REQ-I-074-003 | Must | `prune_undo_below(0)` is a no-op (matches `prune_undo_before` semantics). | Unit test P3: insert entries, call `prune_undo_below(0)`, verify all entries retained, return = 0. |
| REQ-I-074-004 | Must | Startup wire-up calls `prune_undo_below(tip - 360)` exactly once during `Node::new`, before network/event-loop/production. | Manual verification in init.rs read-back; deploy log shows the `[STARTUP] Pruned N stranded cf_undo entries...` line on first restart. |
| REQ-I-074-005 | Must | No `CURRENT_PROTOCOL_VERSION` bump (no EpochState change). | Diff inspection — `crates/core/src/consensus.rs` and any EpochState struct unchanged. |
| REQ-I-074-006 | Must | Backward-compatible: a node restarting on this binary with cf_undo already pruned must produce zero deletions and no error. | P2 partition covers this — `prune_undo_below(N)` on a DB with no entries < N returns 0. |
| REQ-I-074-007 | Should | Compaction hint after bulk delete reclaims SST space. | `compact_range_cf(cf, Some(start), Some(end))` over `[0, keep_height)` byte range, mirroring `prune_undo_above`. |
| REQ-I-074-008 | Could | Existing INC-I-071 regression test (`inc_i_071_undo_snapshot_sentinel`) continues to pass. | `cargo test -p doli-node --test inc_i_071_undo_snapshot_sentinel` — PASS. |

## Insertion Point — Startup Wire-Up

Location: `bins/node/src/node/init.rs` around line 546, immediately **before** the producer liveness rebuild block (line 547+). At that point:
- `state_db` is in scope as `Arc<StateDb>` (created at line 220).
- `chain_state.best_height` is available (chain_state is not yet wrapped in Arc<RwLock> — that happens at line 571).
- Network start has not begun (line 685 onward).
- No event loop, no block production yet.

This satisfies the "BEFORE network start / event loop / production, so it runs once per process without interference" constraint from the user spec.

The constant `UNDO_KEEP_DEPTH = 360` is currently a local const inside `apply_block/mod.rs:345`. SSF: duplicate the literal 360 at the call site in `init.rs` with a comment referencing apply_block. No premature abstraction.

## Triage Verdict

```
━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.95, evidence-backed)
Reasoning: Diagnosis is locked in by user with measured evidence (562MB observed,
           1640 entries × ~280KB = 459MB stranded), explicit code references
           (undo.rs:39-61 forward-only, undo.rs:64-88 pattern to mirror), and
           a complete fix plan with safety guarantees pre-verified. Localized
           to 2 files (undo.rs + init.rs). No cross-component interaction.
           Deterministic. No prior failed fix attempts (this is a NEW incident
           splitting off from the resolved INC-I-071).
━━━━━━━━━━━━━━━━━━━━━━
```

**No DEEP triggers fire:**
1. Probable cause identified — YES (locked-in by user).
2. Cross-module — NO (storage CF + one startup call site).
3. Intermittent — NO (deterministic disk state).
4. Resumed with failed attempts — NO (new incident).
5. Architectural issues — NO (cf_undo is well-isolated; design is sound; gap is one missing function).
6. Fundamentals issues — NO (build/test green; all checks PASS).

Proceeding directly to Milestone Loop with single milestone M1.

## Single Milestone — M1: prune_undo_below + startup wire-up

1. **Test Writer FIRST**: Add `prune_undo_below_bulk_deletes_stranded_entries` to `crates/storage/src/state_db/tests.rs`. Output Contract block with P1/P2/P3 partitions. Test MUST FAIL on current code (method does not exist → compile error).
2. **Developer SECOND**: Implement `prune_undo_below` in `crates/storage/src/state_db/undo.rs` mirroring `prune_undo_above` pattern; wire one-shot call in `bins/node/src/node/init.rs`.
3. **QA**: Verify acceptance — `cargo test -p storage --lib` PASS; `cargo test -p doli-node --test inc_i_071_undo_snapshot_sentinel` PASS; full `cargo build --release` PASS; clippy + fmt clean.
4. **Docs**: Add "Follow-up: stranded entries cleanup" section to `docs/bugfixes/inc-i-071-undo-storage-bloat-analysis.md`.
