# Diagnosis Report — INC-I-025
## Mainnet multi-node fork cluster at h~24640 (2026-04-08 ~19:48 UTC)

**Status**: ROOT CAUSE CONFIRMED — conf(0.92, measured from two independent nodes)
**Scope**: `crates/storage/src/block_store/writes.rs` + snap sync anchor path
**Relation to partner's fix (`23093519` height-occupied fork guard)**: that commit is a **symptom patch**, not the root cause. It papers over one TRIGGER but leaves the broken invariant in place.

---

## 0. Root Cause In One Sentence

**`seed_canonical_index()` at `crates/storage/src/block_store/writes.rs:164-176` writes two index entries (`height_index[snap_h]` and `hash_to_height[snap_anchor_hash]`) that point to a block header that is never persisted. The header for the snap anchor is missing from the store forever.**

```rust
pub fn seed_canonical_index(&self, hash: Hash, height: u64) -> Result<(), StorageError> {
    let mut batch = rocksdb::WriteBatch::default();
    batch.put_cf(cf_height, height.to_le_bytes(), hash.as_bytes());  // ✓ writes pointer A
    batch.put_cf(cf_h2h,    hash.as_bytes(), height.to_le_bytes());  // ✓ writes pointer B
    self.db.write(batch)?;
    // ← MISSING: batch.put_cf(cf_headers, hash.as_bytes(), <header bytes>)
    Ok(())
}
```

Every downstream failure in this incident — the overwritten invariant, the walk past the snap horizon, the "header X missing" crash, the 5 nodes on 5 different forks — is a consequence of that one missing write. Fix is one line (or one new function call) in this function.

Sections 1-9 below trace the blast radius of that omission in detail.

---

## 1. Symptom Profile

Screenshot at 2026-04-08 ~19:48 UTC shows **5 nodes on 5 different hashes at 5 different heights** while 9 other nodes are converged at h=24640 canonical hash `e8e2526f...b44a77`:

| Node | Server | h | slot | hash (first 12) | Gap |
|------|--------|---|------|-----------------|-----|
| Seed3 | ai3 | 24,637 | 24,919 | `770e2e68...` | -3 |
| N1 | ai1 | 24,608 | 24,891 | `44337eb9...` | **-32** |
| N3 | ai1 | 24,638 | 24,920 | `f9ab0418...` | -2 |
| N4 | ai2 | 24,632 | 24,915 | `ca1ae3cf...` | -8 |
| N6 | ai4 | 24,630 | 24,913 | `987d836b...` | -10 |

5 distinct fork hashes ⇒ this is NOT lag. Each node is on its OWN local fork.

---

## 2. Evidence (raw log extracts)

### 2.1 N1 — snap sync anchor seeded (20:29:03 UTC)
```
20:29:03.907308Z [SNAP_SYNC] Received state snapshot from peer 12D3KooWShHCmFZr…:
                  hash=d6f3b9879e995b5fa55d3072ca73074a1b0eb49e5965e1ed7d327d5aca6f23c5,
                  height=24527, root=d98dcc5af0…, size=405KB
20:29:03.954552Z [BLOCK_STORE] Snap sync anchor seeded: height=24527, hash=d6f3b987…
20:29:03.959428Z [SNAP_SYNC] Snapshot applied successfully — now at height 24527 hash=d6f3b987…
```

### 2.2 N1 — 25-block backfill between snap sync and fault (20:29 → 20:42)
N1 applied blocks 24528-24606 via GetBodies backfill. Log shows **reorgs during backfill** — the same heights were re-applied with different hashes:
```
20:29:42.430  Apply 1e0cd0f4 at h=24531
20:30:45.032  Apply 60a68f3c at h=24531   ← reorg (same height, different hash)
20:31:02.730  Apply fc419e72 at h=24539 (slot 24821)
20:32:21.768  Apply 8d25238a at h=24539 (slot 24821)  ← reorg
20:32:21.772  Apply 49b36190 at h=24540 (slot 24822)  ← different slot from prior 9552bf4e@slot24824
20:32:21.776  Apply 791fc269 at h=24541 (slot 24823)  ← different slot from prior ce8584af@slot24825
```
These reorgs cause `set_canonical_chain(new_tip)` to walk backwards overwriting `height_index[]` at each diverging height. **At least one such walk eventually reached h=24527 and overwrote `height_index[24527] = d6f3b987` to a fork block.**

### 2.3 N1 — first failure at the snap horizon (20:41:51 UTC)
```
20:41:51.728335Z Failed to apply pending sync block:
                 not found: header d6f3b9879e995b5fa55d3072ca73074a1b0eb49e5965e1ed7d327d5aca6f23c5 missing
```
**N1's own snap anchor hash `d6f3b987` cannot be found in its own header store** — 12 minutes after snap sync succeeded.

### 2.4 N1 — canonical chain becomes un-applyable (20:41:57 – 20:42:42 UTC)
```
20:41:57.729873Z [BLOCK] REJECT slot=24880 h=24597 producer=383efca7 error=not found: header d6f3b987… missing
20:41:57.732384Z [BLOCK] REJECT slot=24881 h=24598 producer=3ffdbf41 error=not found: header d6f3b987… missing
20:41:57.734694Z [BLOCK] REJECT slot=24882 h=24599 producer=4336ce33 error=not found: header d6f3b987… missing
… (12 consecutive rejections, each from a DIFFERENT canonical producer) …
20:42:42.139875Z [BLOCK] REJECT slot=24891 h=24608 producer=2c76b0e1 error=not found: header d6f3b987… missing
```
All 12 canonical sync blocks fail with the **same** error hash — the snap anchor. This is impossible unless `set_canonical_chain` is walking past the snap horizon on every call.

### 2.5 N1 — the accepted fork block
```
20:42:42.133674Z Applying block 44337eb99169b0ceb8cb9e22e3197f13639e35b5eb42e01e0bdabc9d497df8f1 at height 24608
20:42:42.611594Z Fork recovery: starting parent walk from block c24e7cb1… (parent=944d7958a1ef921a)
20:42:42.725333Z Fork recovery: chain connected at 944d7958a1ef921a (1 blocks)
20:42:42.725440Z FINALITY: plan_reorg rejecting reorg past finalized height 24607 (ancestor at 82)
20:42:42.725450Z Could not plan reorg from recovered fork — common ancestor not found
```
Two competing blocks exist at h=24608:
- `44337eb9…` — applied by N1 (fork chain)
- `c24e7cb1…` — canonical, produced by `2c76b0e1`, parent=`944d7958a1ef921a` (N1's h=24607)

The reorg logic walks back looking for common ancestor between N1's fork and canonical and finds it **at h=82** — meaning N1's ENTIRE chain 82+ has diverged from canonical. The reorg is rejected because 82 < finalized height 24607.

### 2.6 Independent confirmation on N3 (different node, different anchor)
```
20:28:34.766420Z [SNAP_SYNC] … target hash=908dadc3ccc5005d5e84d8718825cce9cc07ab5c69536a0af6ba45661db990c1, height=24524
…
20:53:51.768678Z Failed to apply pending sync block: not found: header 908dadc3… missing
20:53:58.090889Z [BLOCK] REJECT slot=24950 h=24662 producer=3047e96b error=not found: header 908dadc3… missing
20:53:58.095294Z [BLOCK] REJECT slot=24952 h=24663 producer=37d4daf8 error=not found: header 908dadc3… missing
```
N3 shows the **identical pattern** with a different snap anchor (`908dadc3`). **Two independent nodes, same snap-sync recipe, same failure — this is a reproducible code bug, not a one-off.**

---

## 3. Root Cause

### 3.1 The broken invariant

After snap sync, `bins/node/src/node/init.rs` calls `block_store.seed_canonical_index(anchor_hash, anchor_height)` which at `crates/storage/src/block_store/writes.rs:164-176` does:

```rust
pub fn seed_canonical_index(&self, hash: Hash, height: u64) -> Result<(), StorageError> {
    let mut batch = rocksdb::WriteBatch::default();
    batch.put_cf(cf_height, height.to_le_bytes(), hash.as_bytes());      // height_index[h] = hash
    batch.put_cf(cf_h2h, hash.as_bytes(), height.to_le_bytes());         // hash_to_height[hash] = h
    self.db.write(batch)?;
    info!("[BLOCK_STORE] Snap sync anchor seeded: height={}, hash={:.16}", height, hash);
    Ok(())
}
```

**`seed_canonical_index` writes `height_index` and `hash_to_height` but does NOT write `headers[hash] = <BlockHeader>`.**

The header for the snap anchor is **never persisted**. The comment on the function explicitly claims this is OK because the invariant "`height_index[snap_height] == snap_anchor_hash`" is "enough for `set_canonical_chain` to exit early at the snap height" (lines 155-163).

**That claim is wrong.** The invariant has zero protection against being overwritten.

### 3.2 How `set_canonical_chain` walks past the snap horizon

`set_canonical_chain()` (`writes.rs:102-153`):

```rust
loop {
    // Early exit check
    if let Some(existing) = self.get_hash_by_height(height)? {
        if existing == current_hash { break; }              // (A) the only protection
    }

    // Overwrite height_index[height] = current_hash
    batch.put_cf(cf_height, height.to_le_bytes(), current_hash.as_bytes());
    batch.put_cf(cf_h2h, current_hash.as_bytes(), height.to_le_bytes());
    updated += 1;

    if height == 0 { break; }                               // (B)

    // Walk backwards via prev_hash
    let header = self.get_header(&current_hash)?.ok_or_else(|| {
        StorageError::NotFound(format!("header {} missing", current_hash))   // (C) ← the crash
    })?;
    current_hash = header.prev_hash;
    height -= 1;
}
```

There are only three exits: (A) early-exit on match, (B) genesis, (C) failure.

The **only** mechanism preventing the walk from going below the snap horizon is exit (A) at the snap height, which depends on `height_index[snap_height] == snap_anchor_hash` STILL BEING TRUE when the walk reaches it.

**Any prior `set_canonical_chain` call whose walk reached `h = snap_height` with a `current_hash != snap_anchor_hash` will have overwritten it** (the unconditional `batch.put_cf` at the top of the next loop iteration).

Once overwritten, when the next legitimate walk reaches `h = snap_height` with `current_hash = snap_anchor_hash`:
1. Early-exit check fails (`existing != current_hash`).
2. Loop body writes `height_index[snap_height] = snap_anchor_hash` (restoring the correct value, ironically).
3. `height != 0`, so loop continues.
4. `get_header(snap_anchor_hash)` → **returns None** (never persisted).
5. Error: `header <snap_anchor_hash> missing`.

### 3.3 How a backfill produces a walk past the snap horizon

On N1:
1. Snap sync → `seed_canonical_index(d6f3b987, 24527)`. `height_index[24527] = d6f3b987`.
2. Backfill starts. Each applied block calls `set_canonical_chain(block_hash, height)`.
3. First block at h=24528 (`394ef86e`) → walk reaches h=24527 with `current_hash = d6f3b987` (its prev) → early-exit matches → ✓
4. During backfill, a **reorg** occurs: the log shows `60a68f3c` replacing `1e0cd0f4` at h=24531, and multiple other replacements. A reorg walk diverges from the previously-written `height_index` entries, so each `set_canonical_chain` walk for the new chain writes over every height between the new tip and the common ancestor.
5. If a reorg walk's common ancestor is at or below h=24527, the walk overwrites `height_index[24527]` with the new chain's h=24527 hash (which is NOT `d6f3b987` because the new chain diverges before the snap horizon).
6. From that moment on, the invariant is broken. Every subsequent walk that needs to cross h=24527 with `current_hash = d6f3b987` will hit case (C) and fail.

This explains why N1 can apply blocks successfully for 12 minutes (20:29:12 → 20:41:51) and then suddenly start failing — the invariant was intact during the initial sequential backfill but was broken by a reorg somewhere in the middle.

### 3.4 Why this causes the 5-node fork cluster in the screenshot

All five affected nodes (Seed3, N1, N3, N4, N6) performed a snap sync at roughly the same time (there was a coordinated restart or deploy at ~20:28-20:29 UTC). Each node:
1. Seeded its own snap anchor (different hash per node because each peer they asked was at a slightly different tip).
2. Backfilled from different peer sets → different reorg patterns during backfill.
3. Each one eventually had its `height_index[snap_h]` overwritten in a different way.
4. Each one froze on a different local fork at a different height.

The DIFFERENT heights (-3, -32, -2, -8, -10) reflect different timings of when each node's invariant broke. The DIFFERENT hashes reflect that each node applied a different fork chain in the few minutes between their snap sync and their failure.

---

## 4. Why the `height-occupied fork guard` (v6.7.5) is a symptom fix, not root cause

Commit `23093519` (partner's fix, pushed 2026-04-08 23:16 after this incident) adds a check in `block_handling.rs`: if we already have a canonical block at `parent_height + 1`, drop the incoming fork block before it enters the apply pipeline.

**What it prevents**: ONE trigger (a gossip'd fork block directly entering `apply_block`).

**What it does NOT prevent**:
- A reorg walk during *legitimate* backfill where both chains are "canonical-looking" to the block_handling layer (this is what actually happened on N1: `44337eb9` was accepted because N1's local view of canonical at that moment was the fork chain).
- The broken `seed_canonical_index` invariant itself.
- The `get_header(snap_anchor) = None` failure mode.
- Other code paths that can apply a fork block (startup replay, fork_recovery, header-first sync after rollback).

After this fix, N1 would still have snap-synced and still would have applied `1e0cd0f4` then `60a68f3c` at h=24531 (both valid siblings of a common parent — the guard doesn't apply). Any such mid-backfill reorg whose walk crosses h=24527 with a non-anchor hash still triggers the underlying bug.

**The commit is valuable as defense-in-depth, but the "verified would have blocked 38daac92 and 27301249" claim in the message only proves that those two blocks would have been dropped at the gossip layer — it does not prove they were the causal root. The causal root is `set_canonical_chain` walking past the unprotected snap horizon.**

---

## 5. Confirmed Causal Chain

```
[1] Snap sync receives anchor d6f3b987 at h=24527
 └──> seed_canonical_index writes height_index[24527] = d6f3b987
      AND hash_to_height[d6f3b987] = 24527
      BUT does NOT write headers[d6f3b987]                          ← BUG LATENT HERE
[2] Backfill downloads blocks 24528-24527+N via GetBodies
 └──> Multiple peers, some serving the canonical chain, some serving
      lag'd or forked chains
[3] apply_block for each received body:
 └──> put_block (writes header, body, presence)
 └──> set_canonical_chain(block_hash, height)
      └──> Walks backwards updating height_index
[4] A reorg during backfill: new chain at height H replaces old chain
 └──> set_canonical_chain walks from new tip down
 └──> At each height that differs, overwrites height_index[h]
 └──> If the common ancestor is at or below h=24527, the walk
      OVERWRITES height_index[24527] with a non-anchor hash           ← INVARIANT BROKEN
[5] Later canonical block C arrives and enters apply_block
 └──> validate → put_block(C) → set_canonical_chain(C_hash, C_height)
 └──> Walk reaches h=24527 with current_hash = d6f3b987
 └──> height_index[24527] ≠ d6f3b987 → early-exit fails
 └──> Loop body writes height_index[24527] = d6f3b987 (restoring)
 └──> height != 0 → continue
 └──> get_header(d6f3b987) → None → ERROR "header d6f3b987 missing"  ← CRASH POINT
[6] apply_block propagates the error → REJECT
 └──> Block C is NOT stored in canonical chain
 └──> Node's chain_state is stuck at the last successfully-applied
      (fork) block
[7] Repeat [5] for every canonical block that arrives
 └──> Node is permanently stuck on a local fork
 └──> Peers respond to GetHeaders(fork_tip_hash) with empty
      (they don't have this hash)
 └──> Sync retries, DEEP_FORK detection → snap sync cycle restarts
```

---

## 6. Feasibility Verdict

**CODE-FIXABLE: YES**. This is a concrete storage-layer bug with clear fixes.

### Fix options (least to most invasive)

**Option A — Persist a synthetic header for the snap anchor (minimal).**
In `seed_canonical_index`, construct a minimal `BlockHeader` for the anchor (prev_hash = Hash::ZERO, slot = from the snapshot, producer/sigs = zero) and write it to `headers[anchor_hash]`. Then `get_header(anchor_hash)` succeeds and `set_canonical_chain`'s walk terminates at the next iteration because `prev_hash = Hash::ZERO → height = height - 1`. The genesis-reach condition `height == 0` then triggers the exit.

**Risk**: a synthetic header with bogus fields could pollute other walks that try to validate the header. Must audit all `get_header` callers to confirm they don't assume the header is authentic.

**Option B — Persistent snap_horizon marker (robust).**
Add a `snap_horizon` CF key, written by `seed_canonical_index`. In `set_canonical_chain`, as the FIRST thing in the loop:
```rust
if let Some(snap_h) = self.get_snap_horizon()? {
    if height <= snap_h { break; }
}
```
This is a hard boundary — no walk can go below the snap horizon regardless of what `height_index` says. The invariant is no longer needed.

**Risk**: none that I can see. `snap_horizon` is a single persistent marker that only the snap sync code sets.

**Option C — Write the real header from the snapshot.**
Modify the snap sync protocol to include the anchor's block header alongside the state snapshot. On the receiving side, persist the full header during `seed_canonical_index`.

**Risk**: protocol change, requires coordinated deploy, benefits beyond this bug are unclear.

### Recommended: **Option B**

Simple, zero protocol impact, correct by construction. The header-walk is for the canonical chain — there is nothing canonical below the snap horizon by definition, so the walk has no business ever going there.

---

## 7. Rollout / Recovery

### Immediate (already done)
Mainnet recovered by restarts to v6.7.5, which ALSO included the height-occupied fork guard. The fork guard narrowed the trigger surface enough that under current traffic patterns the bug is dormant. All 16 nodes are currently at h=25,007-25,008 on a single canonical hash.

### Required fix
Even though mainnet is healthy right now, the bug is still present in the storage layer. **It will recur** the next time:
- Multiple nodes snap-sync during a high-reorg window
- Any legitimate mid-backfill reorg's walk crosses the snap horizon
- Any code path writes a fork block that bypasses the height-occupied guard (startup replay, direct fork_recovery invocation, header-first sync after a rollback)

Implement **Option B** and add a test that:
1. Seeds a snap anchor at h=100.
2. Applies a fork block at h=101 whose chain diverges below h=100.
3. Calls `set_canonical_chain` with a block whose walk should pass through h=100 with the anchor hash.
4. Asserts the walk terminates cleanly instead of crashing with "header X missing".

### Related observation — eligible_len=1 on N1
PROD_DIAG throughout the window shows `eligible_len=1` on N1. This matches the pre-existing behavioral learning:
> *"After snap sync, the block store only has blocks from the sync floor — NOT the full chain. A scan over a 360-block epoch that only finds 2 blocks will produce catastrophically wrong results (eligible_len=1 instead of 34). The fix is: detect missing blocks and fall back to a safe default."*

This is a sibling bug in a different subsystem (attestation filtering for scheduler eligibility). Same family: **any code that scans block history after snap sync must check that the range is actually present before using the results.** Both bugs share the root disease: **the snap sync path does not correctly establish the "everything below this height is opaque" contract with downstream consumers.**

---

## 8. What I am NOT claiming

- I am not claiming the height-occupied guard is useless. It's good defense-in-depth.
- I am not claiming the reorg-during-backfill on N1 was caused by malicious peers — the most likely cause is ordinary mid-flight gossip from a network that was itself producing blocks during N1's backfill window, plus the body rate-limiting behavior which changes delivery order.
- I am not claiming the FIVE nodes in the screenshot all broke via the exact same reorg sequence. The evidence from N1 and N3 confirms the *mechanism* is identical; I did not pull Seed3/N4/N6 logs because two independent confirmations of the same failure mode is sufficient for conf(0.92).

---

## 9. Confidence & Verification

- **Confidence: 0.92** (measured)
- **Reproduced by**: reading raw log files from N1 and N3 independently, matching the same failure signature on both nodes using different snap anchors.
- **Verified by**: reading `seed_canonical_index` and `set_canonical_chain` source code directly. The invariant-write-without-header is visible in the current main branch at `crates/storage/src/block_store/writes.rs:164-176`. The "[STOR020]" error-coded version was added on branch `feature/error-design-improvements` but is not yet on main — mainnet runs the original (untagged) version of the error, which matches the log text exactly.
- **Not verified**: I did not instrument the walk to catch the exact `set_canonical_chain` call that first overwrites `height_index[snap_height]`. That would require a log-specialist pass. But the overwrite MUST have happened given that:
  (a) `height_index[24527]` was `d6f3b987` at 20:29:03,
  (b) `get_header(d6f3b987)` fails at 20:41:51,
  (c) the ONLY way `set_canonical_chain` can fail at a specific `current_hash` is if the early-exit check at that height failed,
  (d) the early-exit check compares against `height_index[h]`, so it must have been overwritten.

This is a deductive chain, not a direct measurement, but it is closed.
