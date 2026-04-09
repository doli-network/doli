# INC-I-025 — Verified Root Cause (0.99 confidence)

**Incident**: Mainnet multi-node fork cluster at h~24,640 (2026-04-08 ~19:48 UTC)
**Verifier**: /omega-doctor deep re-audit, 2026-04-08 post /clear
**Branch**: feature/error-design-improvements
**Production tag**: v6.7.3 (verified against this exact tag)

---

## 0. Root cause in one line

**`seed_canonical_index()` at `crates/storage/src/block_store/writes.rs:164-176` writes `height_index[snap_h]` and `hash_to_height[snap_anchor_hash]` but never writes the header for `snap_anchor_hash` — creating an unprotected invariant that `set_canonical_chain()` must trip over, after which it crashes every subsequent canonical block with `"header {snap_anchor_hash} missing"`.**

The prior diagnosis was correct. This audit re-verified every claim against the v6.7.3 production source and five independent mainnet node logs, and upgrades confidence from 0.92 → **0.99**.

---

## 1. Claims that were independently re-verified

### 1.1 Source code claim — VERIFIED at tag v6.7.3

`git show v6.7.3:crates/storage/src/block_store/writes.rs` (lines 164-176):

```rust
pub fn seed_canonical_index(&self, hash: Hash, height: u64) -> Result<(), StorageError> {
    let cf_height = self.db.cf_handle(CF_HEIGHT_INDEX).unwrap();
    let cf_h2h = self.db.cf_handle(CF_HASH_TO_HEIGHT).unwrap();
    let mut batch = rocksdb::WriteBatch::default();
    batch.put_cf(cf_height, height.to_le_bytes(), hash.as_bytes());     // (1) index
    batch.put_cf(cf_h2h, hash.as_bytes(), height.to_le_bytes());        // (2) reverse index
    self.db.write(batch)?;                                              // (3) NO header write
    info!(
        "[BLOCK_STORE] Snap sync anchor seeded: height={}, hash={:.16}",
        height, hash
    );
    Ok(())
}
```

There is no `put_cf(cf_headers, …)` anywhere in this function. The comment above the function (lines 155-163) even ADMITS that only two writes happen:

> "Two writes: `height_index[height] = hash` and `hash_to_height[hash] = height`. This is enough for `set_canonical_chain` to exit early at the snap height."

It is **not** enough. See §1.2.

### 1.2 Error text claim — VERIFIED byte-for-byte

`git show v6.7.3:crates/storage/src/block_store/writes.rs` (lines 130-135) — the crash site inside `set_canonical_chain`:

```rust
let header = self.get_header(&current_hash)?.ok_or_else(|| {
    StorageError::NotFound(format!("header {} missing", current_hash))
})?;
```

Production log (Seed3, 2026-04-08 20:53:51):
```
WARN doli_node::node::periodic: Failed to apply pending sync block:
     not found: header d6f3b9879e995b5fa55d3072ca73074a1b0eb49e5965e1ed7d327d5aca6f23c5 missing
```

The literal format `"header {} missing"` in v6.7.3 source expands to exactly what the log shows. No other code path in the entire workspace emits this exact text with a snap-anchor hash. The crash origin is therefore unambiguously `set_canonical_chain()`.

(Note: the `feature/error-design-improvements` branch prefixes this with `[STOR020]` — that prefix is NOT in v6.7.3 and is NOT in the log, which is consistent with mainnet running v6.7.3.)

### 1.3 Snap sync protocol claim — VERIFIED

`crates/network/src/sync/manager/types.rs:153-166`:

```rust
pub struct VerifiedSnapshot {
    pub block_hash: Hash,
    pub block_height: u64,
    pub chain_state: Vec<u8>,       // serialized ChainState
    pub utxo_set: Vec<u8>,          // serialized UtxoSet
    pub producer_set: Vec<u8>,      // serialized ProducerSet
    pub state_root: Hash,
}
```

There is **no** `block_header: BlockHeader` field. The snap sync protocol structurally cannot transport the anchor header. Every caller of `seed_canonical_index` is therefore calling it without a header to persist.

### 1.4 Caller chain claim — VERIFIED

`grep -rn "seed_canonical_index"` finds exactly three callsites:

1. `bins/node/src/node/init.rs:333` — startup path, chain_state from snap sync
2. `bins/node/src/node/init.rs:370` — startup recovery, block store empty but chain_state present
3. `bins/node/src/node/fork_recovery.rs:666` — runtime `apply_snap_snapshot()`, immediately after replacing UTXO/ProducerSet

**None of the three is followed by a header write** for the anchor. Verified by reading each callsite.

### 1.5 Header-write-only-via-put_block claim — VERIFIED

`grep -rn "put_cf.*cf_headers\|put_cf\(cf_headers"` finds exactly **one** production write to CF_HEADERS: `writes.rs:31` inside `put_block()`. That function takes a full `Block` — something snap sync does not have for the anchor.

There is no other code path in the workspace that persists a header. Therefore, the anchor's header cannot be backfilled after snap sync unless a peer later sends the full block via GetBodies and it is routed through `put_block` — which only happens if the node explicitly fetches the anchor body, which snap sync does not do.

---

## 2. Five-node independent log confirmation

All five affected nodes in the screenshot were pulled and inspected independently. **All five show the identical signature with different snap-anchor hashes.**

| Node  | Server | Snap anchor hash (prefix) | Anchor height | Time seeded (UTC)          | Time first failure (UTC)   | Delay     | Rejected producers seen |
|-------|--------|---------------------------|---------------|----------------------------|----------------------------|-----------|-------------------------|
| N1    | ai1    | `d6f3b987…f23c5`          | 24527         | 2026-04-08 20:29:03.959428 | 2026-04-08 20:41:51.728335 | 12m 48s   | ≥12 distinct            |
| N3    | ai1    | `908dadc3…90c1`           | 24524         | 2026-04-08 20:28:34        | 2026-04-08 20:53:51        | 25m 17s   | ≥2 distinct             |
| N4    | ai2    | `908dadc3…90c1`           | 24524         | 2026-04-08 20:28:34.285987 | 2026-04-08 20:32:23.731790 | 3m 49s    | ≥10 distinct            |
| N6    | ai4    | `b25ec0ad…1a98`           | 24585¹        | 2026-04-08 20:38:46.128482 | 2026-04-08 20:43:46.034387 | 5m 00s    | ≥6 distinct             |
| Seed3 | ai3    | `d6f3b987…f23c5`          | 24527         | 2026-04-08 20:29:04.568230 | 2026-04-08 20:53:51.041621 | 24m 47s   | ≥9 distinct             |

¹ N6's first snap sync was to `95261285…0c06` at h=24530 at 20:29:34. That session stuck and triggered a second snap sync at 20:38:46 to `b25ec0ad…1a98` at h=24585. The header-missing failure at 20:43:46 is against the SECOND anchor.

**Observations:**
- N1 and Seed3 share the same anchor hash (`d6f3b987…`) because they happened to request the snapshot from peers that were both at h=24527 at the same instant. This is expected — it does NOT mean the nodes were synced with each other. Each one independently called `seed_canonical_index` on its own local store.
- N3 and N4 share `908dadc3…` for the same reason.
- Every node's error text contains the HASH THAT WAS SEEDED on that node, not any other hash. This rules out any "poisoning from a peer" explanation — the error is generated by each node's own `set_canonical_chain` calling `get_header` on its own previously-seeded value.
- Times between seeding and failure vary from 3m 49s (N4) to 25m 17s (N3). This variance reflects the timing of reorg walks and peer block delivery, not a different root cause.
- Every failure is followed by **systematic rejection of every subsequent canonical block**, with the error always naming the same anchor hash. This is the pattern of a walk that REACHES the anchor hash on every call, which can only happen if the walk's prev_hash chain consistently leads to the anchor.

---

## 3. The closed causal chain

```
[SEED]  seed_canonical_index(anchor_hash, anchor_h)
        ├── writes height_index[anchor_h] = anchor_hash       ✓
        ├── writes hash_to_height[anchor_hash] = anchor_h     ✓
        └── does NOT write headers[anchor_hash]                ← THE BUG

[GOOD]  Block at anchor_h+1 arrives, prev_hash = anchor_hash
        put_block(block_h+1, anchor_h+1)
        set_canonical_chain(block_h+1_hash, anchor_h+1):
          walk to (anchor_hash, anchor_h)
          early-exit: height_index[anchor_h] == anchor_hash → break  ✓
        [For as long as the invariant holds, every walk exits cleanly
         at the anchor height without touching the missing header.]

[BROKEN] Any sequence of operations that either
         (a) writes height_index[anchor_h] with a hash ≠ anchor_hash, OR
         (b) deletes height_index[anchor_h],
         puts the invariant into a state where the next walk that
         reaches (anchor_hash, anchor_h) will NOT early-exit.

[CRASH] Legitimate canonical block C at anchor_h + N arrives.
        apply_block(C, anchor_h + N)
          put_block(C, anchor_h + N)                — writes C's header
          set_canonical_chain(C_hash, anchor_h + N)
            walk:
              (C_hash, anchor_h + N)    → write, walk
              (C_prev, anchor_h + N-1)  → write, walk
              … all intermediate blocks have headers (they were put_block'd)
              → eventually reaches (anchor_hash, anchor_h)
            at (anchor_hash, anchor_h):
              early-exit: height_index[anchor_h] ≠ anchor_hash → continue
              write height_index[anchor_h] = anchor_hash      (restoring the index)
              height != 0 → continue loop
              get_header(anchor_hash) → None
              → Err("header {anchor_hash} missing")            ← exact log line
        apply_block returns Err → block REJECT
        chain_state.best_height does NOT advance

[STUCK] Every subsequent canonical block triggers the same walk path
        and the same crash. The node is frozen on its last
        successfully-applied block, which is some fork block applied
        before the invariant broke — explaining why each node is on a
        DIFFERENT hash at a DIFFERENT height.
```

The chain is closed. There is no step in it that depends on speculation or unverified code.

---

## 4. What breaks the invariant (probable mechanism)

This section is at **~0.85 confidence** — the root cause does not depend on it, but it is useful for reproducing the bug and for understanding why different nodes failed at different times.

The most likely trigger (with strong evidence from N1's log) is a **reorg walk whose prev_hash chain diverges below the snap horizon**, combined with Seed3's specific pre-snap rollback cascade.

### Evidence from Seed3's log (archive seed with pre-snap history)

Before the snap sync at 20:29:04, Seed3 was already running and had been through a rollback cascade:

```
20:20:02  ROLLBACK_1  h=24477 → h=24476
20:20:15  ROLLBACK_2  h=24476 → h=24475
20:27:35  ROLLBACK_3  h=24478 → h=24477
20:27:58  ROLLBACK_4  h=24477 → h=24476
20:28:03  ROLLBACK_5  h=24476 → h=24475
20:28:38  ROLLBACK_6  h=24475 → h=24474
20:29:03  Apply block f6a036c0 at h=24475          ← last pre-snap block
20:29:03  SNAP_SYNC Starting (gap=52, target h=24526)
20:29:04  Snap anchor seeded: h=24527, d6f3b987    ← invariant established
20:29:12  Apply block 394ef86e at h=24528         ← first post-snap block
…
20:30:37  ROLLBACK_7  h=24531 → h=24530  (gap=4)
…
20:41:36  ROLLBACK_8  h=24598 → h=24597
20:41:41  ROLLBACK_9  h=24597 → h=24596
20:43:37  ROLLBACK_10 h=24609 → h=24608
…
20:53:51  FIRST "header d6f3b987 missing" error   ← invariant trips
```

Important property of rollbacks: `rollback_one_block()` at `bins/node/src/node/rollback.rs:12-222` updates `chain_state`, `producer_set`, `utxo_set`, and `sync_manager`, but **does NOT touch `block_store.height_index` or `block_store.hash_to_height`**. This means every rollback leaves stale entries in the block store for the rolled-back heights.

On Seed3 (an archive seed), the block store had entries from the entire pre-snap history, down through heights 0…~24478 from the prior session. The snap sync seeded ONLY `height_index[24527] = d6f3b987` without clearing anything below. That creates a mixed state:

```
height_index[0..=24478]      — old pre-snap chain entries (stale)
height_index[24479..=24526]  — empty
height_index[24527]          — d6f3b987 (just seeded)
height_index[24528..]        — built up by backfill
```

### Likely trigger: a reorg walk from apply_block() whose chain eventually leads through h=24527

After the snap sync, every post-snap block application calls `set_canonical_chain(new_tip, new_height)` which walks backwards via `prev_hash`. The walk stops at the first match with `height_index`. In a steady-state run on a node whose state is entirely POST-snap, the walk always bottoms out at the most recent common ancestor within a few steps.

On Seed3, `excluded_producers` and scheduler state go through `rebuild_excluded_from_headers()` after rollback (`bins/node/src/node/rollback.rs:180`). That function scans `block_store.get_block_by_height(h)` for heights `[epoch_start..current_h]` (`bins/node/src/node/rewards.rs:414`) — in Seed3's case, `[24480..24530]`. Those lookups pass through `get_hash_by_height()` → `get_block()` → `get_cf(headers, hash)`. For heights 24479..24526, the only "hashes" in `height_index` are either stale old-chain hashes from the pre-snap session or empty. The lookups don't write anything to height_index, but they *do* resolve to stale block headers from the prior chain.

Those **stale pre-snap headers are then reachable by prev_hash walks** if any of them gets referenced as a parent. Specifically: if a `set_canonical_chain` call walks backwards and crosses into the pre-snap range, it will read a stale header (which still exists in the block store), follow its `prev_hash` into ANOTHER stale header, and so on — and will write over `height_index[24527]` in the process, because that stale chain is at the "wrong" hash at h=24527 from the snap perspective.

This is consistent with:
- N4 failing fastest (3m 49s) — it is a PRODUCER, so reorg walks are more common and deeper
- Seed3 failing slowest (24m 47s) — it is an archive seed, so walks are fewer but eventually one reaches the anchor
- N6 failing on its SECOND snap sync — the first session's state pollution was carried into the second snap sync
- The rejected-producer list being enormous on every node — once the invariant is broken, EVERY canonical block's walk hits the same crash

The important point is: **this is an auxiliary explanation of the trigger, not the root cause**. Even if no pre-snap state existed, even on a perfectly clean fresh node, the invariant would eventually break on any node that experiences a reorg whose common ancestor is at or below the snap horizon. That case is impossible on mainnet strictly within finality rules, but a peer can gossip a block whose prev_hash chain extends past the snap horizon — and the walk will follow it through the prev_hash chain until it finds a stored header, then fail on `get_header(d6f3b987)`.

---

## 5. Feasibility — CODE-FIXABLE: YES

The fix is trivial and fits in 15 lines. Three options (I agree with the prior diagnosis's recommendation of Option B):

### Option A — Persist a synthetic header for the anchor
Cheap, but pollutes the headers CF with a sentinel record that downstream code may not expect.

### Option B — Persistent `snap_horizon` floor marker (RECOMMENDED)
Add a new singleton CF entry `snap_horizon = u64`. `seed_canonical_index` writes it. `set_canonical_chain` reads it as the FIRST step in the loop and breaks if `height <= snap_horizon`. This is a structural hard floor — no invariant can be broken because the check does not depend on `height_index`.

```rust
// In seed_canonical_index:
batch.put_cf(cf_meta, b"snap_horizon", height.to_le_bytes());

// In set_canonical_chain (first line of the loop):
if let Some(floor) = self.get_snap_horizon()? {
    if height <= floor { break; }
}
```

Correct by construction. Zero protocol impact.

### Option C — Extend VerifiedSnapshot to carry the anchor header
Protocol change. Most robust long-term but highest coordination cost. Not necessary for the current incident.

### Additional mandatory fix: clear stale height_index entries during apply_snap_snapshot

`apply_snap_snapshot()` should explicitly clear any `height_index` and `hash_to_height` entries that point to heights `> snap_horizon`. This removes the second vulnerability documented in §4 — that stale pre-snap entries can be reached by prev_hash walks. One line using `delete_range_cf` on the height index CF is sufficient.

### Regression test
```rust
#[test]
fn set_canonical_chain_walk_stops_at_snap_floor_even_with_stale_index() {
    let store = BlockStore::open(tmp_dir())?;
    // Pre-populate with an "old chain" at h=50..100 (simulating pre-snap)
    for h in 50..100 { store.put_block_canonical(&fake_block(h), h)?; }
    // Clear chain_state equivalent — simulate snap sync
    store.seed_canonical_index(Hash::from([0xaa; 32]), 150)?;
    // Apply a block at h=151 whose prev_hash is the anchor
    let b = fake_block_with_prev(151, Hash::from([0xaa; 32]));
    store.put_block(&b, 151)?;
    // This call triggers the walk — with the fix, must not crash
    store.set_canonical_chain(b.hash(), 151)?;
    // Without the fix, the walk follows prev_hash chain into h=100..50
    // and eventually hits a gap or the anchor with get_header failing.
}
```

---

## 6. What I am NOT claiming

- I am not claiming the partner's v6.7.5 fix (commit `23093519` "height-occupied fork guard") is wrong. It is useful defense-in-depth and narrows the attack surface. But it is a SYMPTOM FIX: it blocks one class of fork block from entering `apply_block`. It does not touch `seed_canonical_index`, does not add a floor marker, and does not prevent any `set_canonical_chain` walk from reaching the anchor via a legitimate prev_hash chain. The bug is dormant under current traffic patterns, not eliminated.
- I am not claiming to have reproduced the bug in a local test. The reproducer in §5 is unverified — writing and running it is the next step before committing the fix.
- I am not claiming Seed3 and N1 hit the invariant via exactly the same code path. The symptom is identical on all five nodes; the precise trigger path may differ between producers and seeds. The fix (Option B + stale entry cleanup) covers both cases.

---

## 7. Confidence & verification summary

| Claim                                                                 | Confidence | Basis                                                |
|-----------------------------------------------------------------------|-----------|------------------------------------------------------|
| `seed_canonical_index` does not write the anchor header               | **0.999** | Read v6.7.3 source directly                           |
| `set_canonical_chain` fails with the exact production error string   | **0.999** | Byte-for-byte match between v6.7.3 source and log    |
| No other code path emits `"header X missing"` with the snap anchor    | **0.99**  | Full-workspace grep returned only this file         |
| The snap sync protocol does not carry the anchor header               | **0.999** | `VerifiedSnapshot` struct has no header field         |
| Only 3 callsites of `seed_canonical_index`, none persist the header  | **0.999** | Full-workspace grep + read of each callsite          |
| Five independent mainnet nodes exhibit the same failure signature     | **0.999** | Log pull from all 5 servers independently           |
| The probability of 5 nodes showing this pattern by any other cause   | **~0.001** | Would require 5 simultaneous unrelated crashes with the same error text — not credible |
| **Overall root cause: `seed_canonical_index` leaves an unprotected invariant that `set_canonical_chain` will eventually trip** | **0.99** | All five sub-claims are near-certain; the conjunction is bounded by the weakest |

The only thing I cannot prove at 0.99 is the EXACT trigger sequence (§4), which is at ~0.85. That does not affect the root cause identification — the fix (snap_horizon floor + stale entry cleanup) eliminates the bug regardless of which trigger sequence activates it.

---

## 8. Action items (gated on user approval)

1. **DO NOT auto-apply the fix.** This is consensus-critical storage code. User must approve the change and the rollout plan.
2. Implement Option B + stale entry cleanup on `feature/error-design-improvements`.
3. Write the regression test in §5.
4. Build + cargo clippy -D warnings + cargo fmt --check + cargo test -p storage + cargo test -p doli-node.
5. Testnet deploy first (`~/testnet/bin/doli-node` + codesign + delete pending_update.json + restart).
6. Verify testnet survives a synthetic snap-sync + reorg scenario.
7. Mainnet rollout: one node first, verify, then roll to the rest.
8. Close INC-I-025 only after the user confirms the fix works in production.
