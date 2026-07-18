# Code Review: State-Root Lazy Tier-0 — M2 (Eager-compute removal)

Milestone: M2 — "Eager-compute removal (the subtraction)"
Workflow: redesign | Branch: feature/state-root-lazy-tier0 | Run: 459

## Summary
Approved. The subtraction is complete, correct, and byte-safe. Every claim in the QA report holds against the code.

## Findings against the 7 review goals

1. **Subtraction complete — PASS.** `state_update.rs:132-138` replaces the entire eager Phase-2/3 compute+publish with a comment; the function no longer touches `cached_state_root`. Grep of all `cached_state_root` sites confirms no hot-path writer remains. The four sites: `state_root_serve.rs:36/63` (live serve, memoize-on-miss), `event_loop.rs:509` (dead `handle_sync_request_bg`, fresh-compute fallback), `apply_block/mod.rs:456` (honest STATE_FP reader), `fork_recovery.rs:342` (write after snap install). None assumes eager population.

2. **`state_fp_sr_field` honest + wired — PASS.** `apply_block/mod.rs:27-35`: `None`→"none"; `best_hash == current_block_hash`→16-hex prefix; any mismatch→"none". Keys on `best_hash`, never height. Wired live at `mod.rs:456-457`. Deletion and honest-fix are in the same working set — regression `test_sr_field_stale_hash_prints_none` fails against a naive delete, passes only with the fix.

3. **Byte-identity — PASS.** No edit to `storage::compute_state_root`, `StateSnapshot::create`, or `compute_state_root_from_bytes`. `state_root_byte_identity_m2.rs` locks served == legacy == snap-build == snap-install. Canonical formula separately locked by the unchanged golden-identity suite.

4. **Hard constraints — PASS.** `CURRENT_PROTOCOL_VERSION` = 8 (untouched). No new activation height. EpochState format untouched. `EPOCH_SNAPSHOT_HF` retained with explicit PARKED note forbidding its planned protocol-version bump (INC-I-054) — entry NOT deleted, version NOT changed.

5. **mmr tombstone — PASS.** `mmr.rs:104-136` tombstone doc-comment on `IncrementalStateRoot`; struct + impl + tests retained and compiling. Zero non-test callers.

6. **specs/engine-parts.md correction — PASS.** Now marks `handle_sync_request_bg` as `#[allow(dead_code)]` DEAD and names the live path `validation_checks.rs::handle_sync_request` → `serve_state_root()`. Matches code.

7. **Over/under-deletion — none found.**

## Non-blocking observations
- **OBS-1 (Minor):** Per-block `[STATE_FP] sr=` now honestly prints `none` for most heights (only populated by an actual GetStateRoot serve or snap install at that exact tip). Intended honesty tradeoff; divergence detection leans on `scheduler_root` (still logged every block), the GetStateRoot RPC, and snap-sync quorum. Acceptable.
- **OBS-2 (Nit):** `hardfork.rs` "6 call-sites" comment matches the ~6 production callers of the 3-arg `compute_state_root`. Cosmetic doc count, no runtime effect.
- **OBS-3 (carried from QA):** dead `handle_sync_request_bg` GetStateRoot arm remains in-tree; harmless fresh-compute fallback; out of scope for M2.

## Specs/Docs drift
None outstanding.

## Final Verdict
Approved for merge. No P0/P1. The change alters only when the root is computed, never its bytes.

## Security Audit Verdict
**AUDIT-SKIP** — deletes an internal compute path and adds a pure diagnostic-log helper over local memo; no external input, no auth/crypto/parsing surface, no network wire change.
