━━━ FINDINGS — 2 total (Minor:2) ━━━

  [F1] MINOR conf(0.92, observed) — specs/engine-parts.md:2357,2442 — documented signature `SyncManager::add_attestation_weight(block_hash, weight)` is now stale; the code takes `(block_hash, attester, weight)`. Trivial doc line, non-blocking.
  [F2] MINOR conf(0.85, observed) — crates/core/src/finality.rs:58-60,151,175 — numerator() is now O(P) recompute on a warm path and the early-attestations buffer grows from ~100 scalars to O(100·P); bounded and acceptable, recorded as a resource-cost observation, no change required.

  Speculative: 1 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Code Review: INC-I-190/191 M3 [F1-dedup] — Finality attester de-duplication

## Scope Reviewed
`git diff 67732f5c` — 4 files inside the worktree
`.claude/worktrees/inc-i-190-191-finality-auth/`:
- `crates/core/src/finality.rs` — `PendingBlock.attesters: HashMap<PublicKey,u64>` replaces scalar `attestation_weight`; private `numerator()`; `early_attestations: HashMap<Hash,HashMap<PublicKey,u64>>`; `add_attestation_weight(block_hash, attester, weight)`; `track_block` folds the buffered map; `check_finality` uses `numerator()`.
- `crates/network/src/sync/manager/production_gate.rs` — `SyncManager` wrapper threads `attester`.
- `bins/node/src/node/network_events.rs` — `on_new_attestation` passes `attestation.attester`.
- `bins/node/src/node/startup.rs` — `create_and_broadcast_attestation` passes own `public_key`.

Cross-read for context (not modified): `crates/core/src/attestation.rs` (`Attestation::verify`, `new`), `docs/qa/inc-i-190-M3-qa-report.md`, `specs/engine-parts.md`, protection registry `v_protection_surface`.

## Summary
✅ **Approved** — this is a genuine ROOT-CAUSE fix, not a symptom clamp.

The incident (numerator 5/5, 6/5 against a network total of 5) was a scalar accumulator that `saturating_add`-ed every delivery with no de-duplication by attester, so a re-delivered authenticated attester (self + gossip echo, or re-broadcast) was summed twice. The fix removes the double-count *at its source*: weight is now stored in a map keyed by the authenticated attester pubkey, and re-delivery is an idempotent `insert` overwrite. Crucially the fix does **not** clamp the numerator at `total_weight` (which would have hidden the bug) — it makes the over-count structurally impossible. That is the correct root-cause shape.

The change is **monotone-decreasing** on the numerator: for any identical sequence of deliveries, the new numerator ≤ the old numerator. It can therefore only make finalization *less* eager, never more — which is strictly safer with respect to every finality-guarded consumer (see System Impact).

## Verification Against the 6 Goals

**1. Weight counts AT MOST ONCE per block — CONFIRMED.**
`add_attestation_weight` (finality.rs:111-116) does `pending.attesters.insert(attester, weight)` inside the matched-block branch and returns; a `HashMap` insert overwrites the same key. `numerator()` (finality.rs:58-60) sums distinct map values. Proven by `test_duplicate_attester_counts_once` (same pubkey ×3 → 1).

**2. Numerator can no longer exceed total_weight — CONFIRMED for all reachable inputs.**
Both accounting paths are keyed by attester: the pending map (finality.rs:114) and the pre-track `early_attestations` inner map (finality.rs:120-123). At most P distinct producers each contribute one slot; under a stable ProducerSet the sum of distinct per-producer weights equals the network total that `track_block` captured, so numerator ≤ total_weight. The pre-track buffer folds into `attesters` verbatim on `track_block` (finality.rs:96,102) and any later delivery for the same attester overwrites — dedup survives the buffer→pending transition. `test_numerator_never_exceeds_total` drives 5×weight-1 + one echo → numerator 5 (not 6) and asserts `cp.attestation_weight <= cp.total_weight`. I agree with QA E1 that a cross-epoch weight-distribution skew is a *theoretical, pre-existing, node-local* edge (weights are derived per-delivery from the freshest local ProducerSet while total_weight is snapshotted at track time); it is not introduced by M3 and does not gate this milestone.

**3. No unintended behavior change — CONFIRMED.**
`FINALITY_THRESHOLD_PCT = 67` (finality.rs:13) and `CONFIRMATION_DEPTH = 2` (finality.rs:19) are untouched. The `FinalityCheckpoint` public struct is byte-for-byte identical (finality.rs:26-37); `attestation_weight` is still a `u64`, now sourced from `numerator()` (finality.rs:175). The F2 depth-2 gate `applied_tip_height >= height + CONFIRMATION_DEPTH` is intact (finality.rs:154-155). Liveness: a genuine 67%-of-distinct-weight quorum at depth ≥2 still finalizes — `test_normal_finality_at_depth2_no_liveness_regression` and `test_no_depth0_self_finality` stay green. The removed value is exactly the fake double-counted weight; honest quorums are unaffected.

**4. All call sites thread the AUTHENTICATED identity — CONFIRMED, no caller missed.**
`git grep add_attestation_weight` yields exactly two production callers plus the wrapper; the rest are tests.
- `network_events.rs:591-599`: executes only inside `if attestation.verify().is_ok()`, and `Attestation::verify` (attestation.rs:105-108) checks the Ed25519 signature over `block_hash||slot` **against `self.attester`**. So `attestation.attester` is cryptographically bound — a forged attester pubkey cannot produce a valid signature. The value is further filtered through `derive_attester_weight` against the LOCAL ProducerSet (non-members dropped). The wire-declared `attester_weight` is never used for the numerator. Not wire-forgeable.
- `startup.rs:621`: passes the node's own `public_key` (`*kp.public_key()`), weight locally derived and guarded `w == 0 → None`. Correct self-identity.
- `production_gate.rs:500-508`: pure pass-through, then re-runs the depth-2 finality check.

**5. Tests real and pass; no new guard; no version bump — CONFIRMED.**
The two new tests assert on real numeric outcomes (`numerator() == 1`, `== 5` and `attestation_weight <= total_weight`) — not stubs. QA reports finality 12/12, doli-core 992/992, network 524/524, node 74/74. No new early-return guard in non-test Rust (the `else { return }` in `on_new_attestation` predates M3, from INC-I-191 [F1]) → no Path-Coverage block required. `git diff --name-only` shows no `Cargo.toml` / protocol-version change → no version bump, consistent with node-local finality (not in the state root, no block-content change, no activation height needed).

**6. Specs/docs — one trivial drift (F1), non-blocking.**
`specs/engine-parts.md:2357` and `:2442` document the wrapper as `SyncManager::add_attestation_weight(block_hash, weight)` — the signature is now `(block_hash, attester, weight)`. `finality.rs:1352`'s prose description remains accurate. No spec documents the finality *accounting* semantics that would need a dedup note; `specs/attestation-gossip-scaling-architecture.md` describes gossip-layer dedup, which is unrelated. Recommendation (do NOT edit here): add the `attester` parameter to the two engine-parts.md signature lines when M3 lands on `main`.

## System Impact (protection-mechanism interaction)
The registry (`v_protection_surface`) lists the **stuck-fork recovery escalation ladder**, which is *finality-guarded* (`plan_reorg` refuses to reorg below `last_finalized_height`, INV-SYNC-008). Interaction analysis:
- Before M3, an over-counted numerator could drive *premature* finalization; combined with the depth-0 self-finality that F2 already removed, that was the INC-I-190 wedge mechanism (a locally-"finalized" block the guard then refuses to reorg off).
- M3 is monotone-decreasing on the numerator, so it can only make the finality guard fire **later or equal**, never earlier. It therefore *reduces* the guard's chance of wedging legitimate stuck-fork recovery. No feedback loop, no starvation, no new trigger surface.
- No new numeric threshold is introduced. `MAX_EARLY_ATTESTATIONS = 100` is unchanged; its semantics shift from "100 buffered scalars" to "100 distinct buffered block-hashes, each holding a per-attester map" (scale note under Resource Cost).

Conclusion: the two active protections sharing the finality surface do not adversely interact; the change strengthens the invariant the ladder depends on.

## Minor Findings
- **[F1]** `specs/engine-parts.md:2357,2442` — stale signature; add `attester`. Doc-only, non-blocking.
- **[F2]** `crates/core/src/finality.rs:58-60,151,175` — `numerator()` is an O(P) recompute called per pending block on each `check_finality` (invoked on every attestation via `finalize_if_ready`), and the early buffer grows to O(100·P). Allocation-free and bounded; recorded, no change required (see Resource Cost).

## Agreement with QA Observations
I concur that both QA observations are **non-blocking**:
- **OBS-1 (latest-wins overwrite)** — correct and safe: the map stores the freshest per-delivery locally-derived weight; node-local and self-healing. A one-line code comment documenting "latest-wins" intent would help a future reader but is optional.
- **OBS-2 (buffer footprint 100 → ~100·P)** — bounded (~a few MB at P=1000), transient (removed on `track_block`), node-local. Acceptable.

## Speculative Findings (low-confidence, not actionable)
- `numerator()` uses `Iterator::sum()` (finality.rs:59) rather than the prior `saturating_add`. For realistic network weights (total ≪ 2⁶³) overflow is impossible, so this is not a defect; flagged only for completeness. conf(0.3, inferred).

## Final Verdict
**Approved for merge.** Root cause is eliminated by de-duplication on an authenticated, non-forgeable identity; the numerator invariant holds on every reachable path including the pre-track buffer; public shape, thresholds, and the F2 depth-2 gate are unchanged; all call sites thread the correct authenticated identity; tests are real and green; no version bump and no consensus/block-content change. Two minor, non-blocking items (one doc line, one recorded resource-cost note).

━━━ RESOURCE COST ━━━ (impact of the change, and of the one proposed fix in F1)
- CPU: `numerator()` is O(P) (sum over the attester map) and replaces an O(1) scalar read. It runs per pending block inside `check_finality`, which fires on every attestation via `finalize_if_ready`. Warm-path worst case ≈ pending·P per check, ≈ P attestations/slot → O(pending·P²)/slot; at P=1000, pending≈2-3, 10s slots that is ~3M u64-adds/slot (~300k/s) — negligible CPU.
- Memory: `early_attestations` upper bound rises from ~100 scalars (~0.8 KB) to ~100·P (PublicKey 32B + u64) ≈ a few MB at P=1000. Bounded by the unchanged `MAX_EARLY_ATTESTATIONS=100` block-hash cap; transient (drained on `track_block`).
- Allocation: `numerator()` is allocation-free (iterator sum). The map insert may reallocate the per-block `HashMap` on growth (≤ P entries, amortized), replacing a single scalar write — a small, bounded per-attestation allocation on the warm path.
- I/O: none. Finality is in-memory, node-local; not persisted, not in the state root.
- Network: none. No wire format change; `Attestation` bytes are unchanged.
- Scale sensitivity: no numeric threshold added or tuned. The only scale-dependent quantity is the early-buffer footprint (O(100·P)); safe at the current mainnet P and at the 1000-producer design target. The F1 doc fix has zero runtime cost.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

━━━ SECURITY AUDIT VERDICT ━━━
Verdict: AUDIT-REQUIRED
Signals: external data (network attestation ingestion in on_new_attestation), state integrity (finality weight accounting / numerator invariant), trust boundary (authenticated attester identity keyed into the dedup map). Confirmation only — the 5-auditor sweep for this milestone was already dispatched.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
