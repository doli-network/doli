# INC-I-308 R-1 — anchor rule for peer store writes (design)

- Run: 734. Worktree `fix/rpc-admin-access`, HEAD `7b7cdd8e`, base `4cf8be85`.
- Input: verifier report `docs/verifications/verify-4cf8be8531e6-7b7cdd8e5bcb.md`, P1 (lines 23-30).
- Status: DESIGN ONLY. No source file is changed by this document.
- Every claim has a tag: **[M]** = measured (I read the line), **[I]** = inferred.
- Line numbers are those of `7b7cdd8e`. The verifier's line numbers (`backfill_neighbor.rs:123-151`,
  `guardian_repair_check.rs:316-336`) do not exist: the files have 39 and 54 lines [M]. The real places are
  `backfill_neighbor.rs:10-39` and `guardian_repair_check.rs:9-54`.
- `specs/populations.md` is missing. The populations in section 9 are named by this design.

## 1. The rule (Stupid Simple First)

**A block from a peer may be stored at height h only if its header hash equals the hash that our own chain
already commits to at h — `chain_state.best_hash` at the tip, otherwise the `prev_hash` of the trusted block at
h+1 — so each fill is one walk down from our own tip, and a block that breaks the hash chain stops the walk.**

Correction to the candidate in the brief: the bottom check ("block a's `prev_hash` must equal the stored hash
at a-1, a mismatch is a refusal") is not a security gate when the walk comes from above. Block a is already
proved by a+1. A mismatch proves that the **stored** a-1 is wrong, not block a (section 5). No genesis lookup
is necessary at h=1: block 1 is proved by block 2 (section 3, position P1).

## 2. Code-path findings

| # | Finding | Evidence |
|---|---------|----------|
| F-1 | `backfillFromPeer` scans `1..=tip_height` and lists each height where `get_block_by_height` is not `Some`. No height above the tip is ever written. | [M] `backfill.rs:97-100`, `:171-191` |
| F-2 | The write loop goes **up** (`for &h in &missing_heights`), so at the bottom of a gap no block above is known. | [M] `backfill.rs:327`, `:260-266` |
| F-3 | `neighbor::refusal` refuses only on a contradiction: index at h, stored h+1, stored h-1 (`if h > 1`). With none stored it returns `None` = accept. | [M] `backfill_neighbor.rs:17-38` |
| F-4 | The signature check uses `header.producer` from the same header. The header hash does **not** cover `vdf_proof` (the signature is in `vdf_proof.pi`). | [M] `backfill.rs:423`; `core/src/block.rs:81-102`; fixture `audit_rpc_effects.rs:62-63` |
| F-5 | Refused blocks only add to `skipped` and a log line. `state.error` stays `None`. | [M] `backfill.rs:436-440`, `:462-473` |
| F-6 | Phase 2 runs only when nothing is missing. Its reverse scan stops at the first height where peer and local hash agree, so it never finds forged low blocks. | [M] `backfill.rs:196`, `:216-244` |
| F-7 | Phase 2 `repair` heights are exempt from the h and h+1 checks. With `skip_divergence_check=true` a node on another branch gets its stored blocks `fork_point..=tip` replaced by the peer's branch. A test pins this. | [M] `backfill_neighbor.rs:17,24`; `backfill.rs:250,315`; `audit_rpc_effects.rs:406-424` |
| F-8 | Snap sync seeds `height_index[snap_h]` with the trusted hash and **no body** (`seed_canonical_index`). The install path can also store a header-only shell (empty body) at `snap_h`. | [M] `block_store/writes.rs:230-244`; `node/init.rs:481`; `node/install_contract.rs:62-73` |
| F-9 | `get_block_by_height` returns `None` when header or body is absent. So a bodiless seed counts as missing; a header-only shell counts as stored. | [M] `block_store/queries.rs:26-39`, `:171-177` |
| F-10 | `repairArchiveFromPeer`: `admissible` accepts a block whose hash equals the local index hash **without** signature or body check. A peer can send the canonical header with a forged body; the raw peer bytes are written. | [M] `guardian_repair_check.rs:18-19`; `guardian.rs:584-597` |
| F-11 | `linked` accepts a block when only h-1 is stored (`prev` alone). | [M] `guardian_repair_check.rs:33-54` |
| F-12 | Archive files feed the canonical store later: `backfill_heights_from_archive` → `check_links` (same weak "refuse on contradiction" rule) → `put_block_canonical`. | [M] `storage/src/archiver/import.rs:101-131`, `:241-260` |
| F-13 | The genesis hash is in the RPC context (`chain_state.genesis_hash`), and each header commits to `genesis_hash`. | [M] `guardian.rs:474-477`; `core/src/block.rs:87` |
| F-14 | The peer side needs only `getBlockRaw {height}` and `getBlockByHeight`. | [M] `methods/block.rs:61-85`; `backfill.rs:329-338` |
| F-15 | Callers: `dispatch.rs:40` and `dispatch.rs:76` only. | [M] grep |
| F-16 | Gauntlet fixture GS-052 (`gsn14holes2`) makes a two-hole store by a backfill from a donor whose tip is **below** the node's snap anchor. | [M] `scripts/gauntlet-stranded/fastpath.py:13-15`, `:244-250` |

## 3. Trusted sources

Only data that our own node made is an anchor. Peer data is never an anchor.

| Source | Why it is trusted | Use |
|--------|-------------------|-----|
| T1 `chain_state.best_hash` at `best_height` | It is our applied tip. It defines "our canonical chain". | Root of every walk. [M] `backfill.rs:97-100` |
| T2 `prev_hash` of a stored block at h+1 that the walk or the scan linked to T1 | The header hash covers `prev_hash` [M] `block.rs:84`. A block whose hash is proved commits to its parent hash. | Anchor at the top of a gap |
| T3 `prev_hash` of a header that this pass fetched and whose hash equals the expected hash | Same hash argument. It holds even when the block is not stored (bad signature or body). | Anchor inside a gap |
| T4 bodiless `height_index[h]` entry | Only `seed_canonical_index` writes an index entry with no body [M] F-8. It is our own snap anchor. | Cross-check only: if it differs from T2/T3, stop (section 5) |

Not trusted: the peer's tip answer, the peer's chain commitment, `prev_hash` of a fetched block alone, the
self-attested signature, and a stored block that the scan finds unlinked.

Is there always an upper anchor? Yes, for each listed height [M] F-1:
- h = tip → T1.
- h < tip and h+1 stored → T2.
- h < tip and h+1 is in the same gap → T3 from the block above, or the walk already stopped.

## 4. Design — `backfillFromPeer`

### 4.1 Changes

| # | File : function | Change |
|---|-----------------|--------|
| C-1 | `crates/rpc/src/methods/backfill_anchor.rs` (**replaces** `backfill_neighbor.rs`, net 0 files) | Pure functions: `anchor_above`, `holds`, `admit`. Target ≤ 120 lines. |
| C-2 | `backfill.rs:171-191` scan closure in `backfill_from_peer` | Keep the upward scan. Add a link check: remember the last stored `(height, hash)`; when h and h-1 are both stored and `blk.header.prev_hash != hash(h-1)`, push `h-1`. After the loop, when the stored tip hash is not `tip_hash`, push `tip_height`. |
| C-3 | `backfill.rs:260-266` | Do **not** merge `repair_heights` into the fetch list. Phase 2 stays as a report: `divergence_check = "diverged: peer differs from h=<fork_point>; not repaired"`. Delete `repair` (`:315`). |
| C-4 | `backfill.rs:327-460` write loop | Replace by the walk in 4.2. Move the loop body to `backfill_anchor.rs` as `async fn walk(...)` so `backfill.rs` (740 lines, over the 500 limit [M]) becomes shorter, not longer. |
| C-5 | `backfill.rs:462-473` | On a stop, set `state.error`. `imported` counts stored blocks only. |

```rust
// backfill_anchor.rs — signatures for the developer
/// Hash our chain commits to at `h`, from our own data: T1 at the tip, else T2.
pub(super) fn anchor_above(store: &BlockStore, h: u64, tip_height: u64, tip_hash: Hash) -> Option<Hash>;
/// True when the stored block at `h` (header and body present) has exactly this hash.
pub(super) fn holds(store: &BlockStore, h: u64, want: &Hash) -> bool;
pub(super) enum Verdict { Store, SkipKeepChain(String), Break(String) }
/// Break: hash != want. SkipKeepChain: hash == want, signature or body fails. Store: all pass.
pub(super) fn admit(block: &Block, h: u64, want: &Hash, sig_ah: u64, bls_ah: u64) -> Verdict;
```

### 4.2 The walk

`todo` = sorted list from the scan (missing heights and unlinked stored heights). The walk reads it from the end.

```
i = todo.len()
while i > 0:
    b = todo[i-1]
    want = anchor_above(store, b, tip_height, tip_hash)      # None -> STOP("no stored block at b+1")
    h = b
    loop:
        if h == 0: break                                     # reached genesis
        if todo[i-1] == h: i -= 1                            # listed height
        else if holds(store, h, want): break                 # rejoined our stored chain
        block = fetch(h)                                     # no block / HTTP / decode / BLAKE3 -> STOP
        match admit(block, h, want):
            Break(why)         -> STOP(why)                  # nothing written at h or below
            SkipKeepChain(why) -> skipped += 1               # not stored; chain continues
            Store ->
                index = store.get_hash_by_height(h)
                if index is Some(x) and x != want:
                    if store.get_block(x) is None: STOP("our index at h holds x, our chain commits to want")
                    store.remove_canonical_entry(h, x)       # stale or forged stored block
                store.put_block_canonical(block, h); imported += 1
        want = block.header.prev_hash; h -= 1
```

- `STOP` ends the whole pass, sets `state.error = "stopped at height h: <why>; <imported> of <total> stored"`
  and clears `running`.
- `SkipKeepChain` is sound: the header hash equals `want`, so its `prev_hash` is ours (T3). This keeps the two
  older tests' result "block with bad signature or bad body is not stored, the other blocks are"
  [M] `inc_i_236_m2_backfill.rs:193-222`.
- `body_matches_header` (INV-FORK-007) stays before the write, inside `admit`. Same order as today
  [M] `backfill.rs:428-435`.
- The preflight (`backfill.rs:116-163`) does not change. It is no longer a safety gate; it only saves time.
- When the walk fetches a height that is not in `todo` (a stored block that does not hold), add 1 to
  `state.total`, so `pct` stays at or below 100.

### 4.3 Each position

| Pos | Shape | Anchor | Forged block | Honest block |
|-----|-------|--------|--------------|--------------|
| P1 | h = 1, bottom of the snap gap | T3 from block 2 (T2 if 2 is stored, T1 if tip = 1) | hash != want → STOP, not written | written |
| P2 | bottom of a gap, a > 1 | T3 from a+1 | STOP | written; then `holds(a-1)` ends the gap |
| P3 | middle of a gap | T3 | STOP; lower heights are not fetched | written |
| P4 | top of a gap, h+1 stored | T2 | STOP | written |
| P5 | top of a gap = tip (tip body absent) | T1 | STOP | written |
| P6 | hole of one block | T2 | STOP | written |
| P7 | bodiless snap seed at h | T2/T1, and T4 must agree | STOP | written; index hash is the same |

### 4.4 Large gap, restart, blocks already written

- No batch buffer is necessary. The anchor moves down, so each block is checked when it arrives and written
  immediately. Memory is one block plus `todo` (8 bytes per height; 6.4 MB for 800,000 heights — the same
  `Vec<u64>` that exists today [M] `backfill.rs:172,260`). The `repair` `HashSet` goes away.
- A stop in the middle of a gap leaves the blocks above the stop height in the store. They may stay: each one
  has `hash == prev_hash` of the block above, up to `best_hash`. [I] from the walk; test T-10 proves it.
- The next call scans again. The lowest block written is now the stored upper neighbor (T2). The run resumes
  there. A node restart is the same case: `BackfillState` is in memory only [M] `context.rs:28-31`.
- Yield every 100 blocks stays [M] `backfill.rs:456-459`.

## 5. Stored blocks that are overwritten (divergence repair)

The rule does not delete the overwrite. It anchors it.

| Case | 4cf8be85 / 7b7cdd8e | New rule |
|------|---------------------|----------|
| O-1 Stored block is not on the chain of our own tip (stale index entry, or a block forged before this fix) | Not found unless the tip also differs [M] F-6 | Found by C-2 or by `holds`. Replaced by the block that our chain commits to. |
| O-2 Node on the losing branch: its tip and its stored blocks are its own branch; the peer has the other branch | With `skip_divergence_check=true` (or a preflight that failed open, `backfill.rs:146-161`) the store `fork_point..=tip` is replaced by the peer's branch [M] F-7 | Nothing is written. Reply: `divergence_check = "diverged: …"`. |

Is O-2 a loss for the losing-branch node? No capability of value is lost:
- The overwrite changed the block store only. Chain state, UTXO set and producer set stayed on the old branch.
  The code says so itself [M] `backfill.rs:105-111`, `:140-142`.
- The default preflight already refuses this node [M] `backfill.rs:138-145`.
- The next block that the node applies on its own branch walks back from its tip and writes its own hashes
  into the index again [I] from `writes.rs:135-170`.
- Its recovery paths do not change: reorg, snap sync, wipe + rsync.

Minimal answer: keep Phase 2 as a detector, drop the exemption, and let the same walk do every overwrite.
Test `audit_p1_rpc_005_backfill_repair_set_overwrites_stale_successor` keeps its assertions when the fixture
tip is the peer chain (O-1). A new test pins O-2 (T-8).

## 6. Design — `repairArchiveFromPeer`

The archive mirrors the canonical store. The anchor is the canonical store, not the peer and not older archive
files.

| # | File : function | Change |
|---|-----------------|--------|
| C-6 | `guardian_repair_check.rs:9-30` `admissible` | Compute `want` (below). Refuse when `block.hash() != want`. Then **always** run `verify_block_signature_at` and `body_matches_header`, also for the hash-equal arm (F-10). |
| C-7 | `guardian_repair_check.rs:33-54` `linked` | Replace by `fn anchor(store, h) -> Result<Hash, String>`: index hash at h, and `prev_hash` of the stored block at h+1. Both present and different → refuse ("local store contradicts itself at h"). One present → that hash. None → refuse ("no trusted hash for h; run backfillFromPeer first"). The `prev`-alone branch is deleted. |
| C-8 | `guardian.rs:634-641` reply | Optional: add `first_error: "h=<n>: <why>"`. `errors` already counts each refusal [M] `guardian.rs:584-588`. |

`guardian.rs` (661 lines, over the limit [M]) gets no new logic; the loop at `:496` does not change.

| Pos | Store state | 7b7cdd8e | New |
|-----|-------------|----------|-----|
| A1 | store has h (all full-history archive nodes) | hash must match; body not checked (F-10) | hash, signature and body must match |
| A2 | bottom of a store gap, only h-1 stored | **accepted** (F-11) | refused: no anchor |
| A3 | middle of a store gap | refused | refused |
| A4 | top of a store gap, h+1 stored | accepted if linked | accepted if linked |
| A5 | h = 1, store lacks 1 and 2 | refused | refused |

Rejected: an in-pass chain for the archive (walk down, carry `prev_hash`). It adds a second walk to
`guardian.rs` for nodes that have a documented path: fill the store first, then repair the archive
(M8, `docs/rpc_reference.md`). After the store walk, each height has an index hash, so A1 applies.

## 7. Alternatives (Explorer / Skeptic, one decision)

| Alt | Approach | New parts | Verdict |
|-----|----------|-----------|---------|
| 1 | Do nothing | 0 | ELIMINATED conf(0.0, measured): 6 of 12 forged-write cases stay open (section 10) |
| 2 | Subtract: delete the `prev`-alone acceptance only; refuse when no stored h+1 | −5 lines | ELIMINATED conf(0.0, measured): the upward loop (`backfill.rs:327`) then refuses every honest block of a multi-block gap except the top one; the snap gap `[1, snap_h-1]` can never be filled |
| 3 | **One walk down from `best_hash`; a chain break stops the pass; anchored overwrite** | 0 files (1 replaced) | SURVIVES conf(0.85, observed) |
| 4 | Walk down per gap; any stored upper neighbor is an anchor; continue with the next gap after a break | 0 files | WEAKENED conf(0.55, inferred): a block forged before this fix that sits above an open gap becomes an anchor; the intent then holds only for a clean store |
| 5 | Fill upward, hold the gap in memory, check when the top is reached | buffer | ELIMINATED conf(0.0, inferred): 800,000 blocks in memory, or a second staging store |
| 6 | Ask the peer (or several peers) for a commitment | protocol use | ELIMINATED conf(0.0, measured): the peer answers what it wants; the tip preflight is already defeated this way (`backfill.rs:134-145`) |

Alt 3 against Alt 4: 0.30 apart, same code size (the difference is `return` against `continue` plus the scan
link check). Alt 3 has one trust root. Its cost is in PRE-2 (section 9).

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      +1 hash compare per stored block in the existing scan (observed)
  Memory:   -small: the `repair` HashSet is deleted; `todo` is the existing Vec (observed)
  IO:       +2 point reads per gap (`anchor_above`, `holds`); block reads in the scan do not change (observed)
  Network:  0 per stored block; fewer requests when a pass stops early (observed)
  Disk:     0 (observed)
  Latency:  0 on the reply; the walk is in the background task as today (observed)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: a hash compare against our own chain is the only check a peer cannot satisfy with forged data.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## 8. Security model

- **Trust boundary:** bytes from `rpc_url` enter at `backfill.rs:329-338` (`getBlockRaw`), `:121-131`
  (preflight), `backfill_peer_commitment.rs:12-37`, `backfill.rs:219-232` (reverse scan) and
  `guardian.rs:450-459`, `:507-516`. The caller is an admin (token gate, M1), but the URL is free text.
- **Attack surface and mitigation:**

| Vector | Path | Mitigation |
|--------|------|------------|
| Self-signed block with a copied `prev_hash` | `backfill.rs:423-441` | `admit`: `hash == want`, where `want` comes from T1/T2/T3 only |
| Peer answers the preflight with our tip hash | `backfill.rs:134-145` | The preflight is not an anchor any more |
| Canonical header, forged body or signature | `guardian_repair_check.rs:19` | C-6: signature and `body_matches_header` on every archive write |
| Forged peer commitment to start a repair | `backfill.rs:199-251` | C-3: Phase 2 cannot cause a write |
| Peer sends canonical headers with bad bodies for the whole gap | walk | No write. Time only; bounded by the gap size |
| Forged archive file reaches the store through import | `import.rs:241-260` | Out of this change. New archive writes are anchored (C-6/C-7). Old files: open risk R-3 |

- **Data classification:** all fields of a fetched block are untrusted. After `hash == want` the hashed header
  fields are proved (`block.rs:81-102`). `vdf_proof` (signature) and the body are proved only by the two
  per-block checks. Damage of a wrong block in the store: served to other nodes by `getBlockRaw`, read by
  rebuild and by `maintainer_rewind` (index consumers, `maintainer_rewind/binding.rs:100-143` [M]).

Failure modes:

| Failure | Result | Recovery |
|---------|--------|----------|
| Peer has no block at h | STOP, error names h | Run again with a peer that has h |
| Store read error in `anchor_above` | treated as no anchor → STOP | Run again |
| Reorg in the top 1,000 heights during a pass | a block of the old branch can be indexed [I] | The next call finds the link break (C-2) and repairs it |
| Node restart in the middle | written blocks stay | Run again; it resumes |

Protection mechanism (one): trigger = fetched hash is not the expected hash; action = stop the pass and report;
scale = no threshold, independent of gap size and node count; interactions = one row found in
`v_protection_surface` [M]: PM-025 (rebuild-precondition dense guard, `bins/node/src/node/rewards.rs`). PM-025
refuses a rebuild on a store with holes. After a stop the store keeps its holes, so PM-025 keeps its refusal
until an honest fill completes. Before this rule the store could become dense with forged blocks and pass
PM-025. The two mechanisms do not loop: neither one starts the other.

## Preconditions and refusals

- PRE-1: A block is stored at h only when its hash equals the hash our chain commits to at h (T1/T2/T3).
- PRE-2: The peer must have each block from the top of our gap downward; the first height it does not have, or
  answers with another block, stops the pass.
- PRE-3: `repairArchiveFromPeer` writes h only when the canonical store has the index hash at h or a block at h+1.
- PRE-4: A bodiless index entry that differs from the anchored hash stops the pass; it is never overwritten.

## Population impact

Project population list is missing (`specs/populations.md`). Populations named by this design.
Proof column: test ids are from section 11; "new" means the test does not exist yet.

| Precondition | Population | Meets it | What happens to them | Evidence |
|---|---|---|---|---|
| PRE-1 | archive / full-history node | yes | no gap; a hole is filled from stored h+1 | T-6 (new); `backfill.rs:171-191` |
| PRE-1 | snap-synced external producer, gap `[1, snap_h-1]` or `[1, snap_h]` | yes | whole gap filled top-down from the stored block above the anchor | T-5 (new); `writes.rs:230-244`, `install_contract.rs:62-73` |
| PRE-1 | node with holes | yes | each hole filled from its stored upper neighbor | T-6 (new) |
| PRE-1 | pruned node | yes | gap `[1, floor-1]` filled from the stored floor block; this un-prunes, as today | UNKNOWN — no pruned-store test; `backfill.rs:174` scans from 1 |
| PRE-1 | `--no-snap-sync` node | yes | same as full-history | T-6 (new) |
| PRE-1 | node on the losing branch | partly | gaps below the fork point fill; blocks of the peer's branch are refused; O-2 overwrite is gone (section 5) | T-8 (new); `backfill.rs:138-145` |
| PRE-1 | old-binary peer in a mixed fleet | yes | serves `getBlockRaw` as before; no wire change | `methods/block.rs:61-85`; stub peer `audit_rpc_effects.rs:123-151` |
| PRE-1 | node after a restart in the middle of a pass | yes | resumes below the lowest written block | T-10 (new) |
| PRE-1 | node that stored forged blocks before this fix | yes | forged blocks that contradict the chain are replaced | T-9 (new) |
| PRE-2 | archive / full-history node | yes | a full-history peer has every block | T-6 (new) |
| PRE-2 | snap-synced external producer | partly | fill succeeds from a full-history peer; from a snap-synced peer it fills down to that peer's anchor and stops with an error | T-11 (new) |
| PRE-2 | node with holes | partly | a peer with its own hole above ours stops the pass at that height; lower holes wait for another peer. At 4cf8be85 the pass skipped and continued (`backfill.rs:361-367`) | T-11 (new) |
| PRE-2 | pruned node | partly | same as snap-synced producer | UNKNOWN |
| PRE-2 | `--no-snap-sync` node | yes | same as full-history | T-6 (new) |
| PRE-2 | node on the losing branch | no | above the fork point the canonical peer has other blocks; the pass stops there | T-8 (new) |
| PRE-2 | old-binary peer in a mixed fleet | yes | no new request type | `backfill.rs:329-338` |
| PRE-2 | node after a restart | yes | same peer rule on the next call | T-10 (new) |
| PRE-2 | node whose peer is shorter than the top of its gap (gauntlet GS-052 fixture) | no | nothing is imported; the fixture cannot make its second hole by backfill | `scripts/gauntlet-stranded/fastpath.py:244-250` |
| PRE-3 | archive / full-history node | yes | every archive height has an index hash | T-15 (new) |
| PRE-3 | snap-synced node with `archive_dir` | partly | heights in the store gap are refused until `backfillFromPeer` fills the store; count is in `errors` | T-16 (new) |
| PRE-3 | node with holes and `archive_dir` | partly | same: store first, archive second | T-16 (new) |
| PRE-3 | pruned node with `archive_dir` | partly | only the height below the floor is repairable; same as 7b7cdd8e (verifier P2) | `guardian_repair_check.rs:33-54`; no pruned test: UNKNOWN |
| PRE-3 | node on the losing branch | partly | archive gets its own branch only; peer blocks above the fork are refused | UNKNOWN — no test |
| PRE-3 | old-binary peer | yes | no wire change | `guardian.rs:507-516` |
| PRE-4 | snap-synced external producer | yes | seed and stored h+1 agree by construction (`init.rs:481`) | `audit_p1_rpc_005_backfill_refuses_block_when_height_index_holds_different_hash` |
| PRE-4 | node whose seed and chain disagree (corrupt store) | no | pass stops with an error; operator must resync | same test; count of such nodes: UNKNOWN |

## Intent check

Intent: a hostile or wrong `rpc_url` can waste time but can never put a block into the canonical store or the archive.

| Must | Target cases | Left open | Allowed open |
|---|---|---|---|
| R-1 | 12 | 0 | 0 |

The 12 cases: backfill P1-P6 (6) and archive A1-A5 plus F-10 (6). At `7b7cdd8e` 6 are open: P1, P2, P3, P5
(when no seed is at the tip), A2, F-10. The count 0 is a design claim; no test has run. It depends on
assumption A-1 below. Blocks that were forged **before** this fix and never contradict a stored neighbor that
the walk reaches are outside the 12 cases (R-2 below).

## 10. Assumptions

- A-1: `chain_state.best_hash` is the hash of our applied tip. Plain: we trust our own tip, nothing else.
- A-2: BLAKE3 header hashes do not collide. Plain: two different headers never have the same hash.
- A-3: A height-index entry with no body comes only from `seed_canonical_index`. [M] for the writers found by
  grep (`writes.rs`, `init.rs:481,520`, `install_contract.rs:62`); a body deleted by disk corruption looks
  the same and also stops the pass (safe side).

## 11. Tests for the developer

New file `crates/rpc/tests/it/inc_i_308_r1_anchor.rs`, registered in `crates/rpc/tests/it/main.rs`.
Reuse from `crates/rpc/tests/it/audit_rpc_effects.rs` (make them `pub(super)`): `block` (:44), `local_node`
(:68), `raw` (:86), `call` (:98), `wait_done` (:104), `Peer` + `peer_rpc` + `serve_peer` (:117-161),
`chain3` (:300). Two fixture changes are necessary:
- `local_node` gets the tip hash as a parameter. Today it sets `best_hash = [0x77; 32]` (:73), which is no
  block. The same dummy is in `inc_i_236_m2_backfill.rs:152` and `inc_i_281_ma_backfill.rs:124`; set it to the
  hash of the last chain block there.
- `Peer` gets `claim_tip: Option<Hash>`: the answer to `getBlockByHeight` with the object parameter
  `{height}` (the preflight shape, `backfill.rs:126`). The stub reads only the positional form today (:128-135).

Hostile peer = `Peer { claim_tip: Some(our tip hash), blocks: <self-signed chain by KeyPair::from_seed([0x66; 32])>, .. }`,
called **without** `skip_divergence_check`. Each hostile test asserts: target heights have no index entry,
`imported` does not count them, `backfillStatus.error` names the height.

| Id | Test | Shape | Red at 7b7cdd8e |
|----|------|-------|-----------------|
| T-1 | `r1_backfill_refuses_forged_h1_and_snap_gap` | stored 4..6; peer forged f1,f2,f3 | yes: f1, f2 written |
| T-2 | `r1_backfill_refuses_forged_gap_bottom` | stored 1,5,6; peer honest 4,3, forged f2 (`prev` = h1) | yes: f2 written. New: 4,3 stay, 2 absent |
| T-3 | `r1_backfill_refuses_forged_gap_middle` | stored 1,6; peer honest 5,3,2, forged f4 (`prev` = h3) | yes: f4 written. New: only 5 stored |
| T-4 | `r1_backfill_refuses_forged_tip` | tip 5 not stored, `best_hash` = h5; stored 1..3; peer honest 4, forged f5 | yes: f5 written. New: nothing stored |
| T-5 | `r1_backfill_honest_peer_fills_snap_gap` | (a) stored 4..6; (b) bodiless seed at 4 + stored 5,6 | no (guard against over-refusal) |
| T-6 | `r1_backfill_honest_peer_fills_every_position` | tip 9 absent; stored 1,5,7,8; holes {2-4},{6},{9} | no |
| T-7 | `r1_backfill_reanchors_stale_stored_blocks` | adapted `…repair_set_overwrites_stale_successor`: stored 1,f2,f3; `best_hash` = h3 | fixture change only |
| T-8 | `r1_backfill_does_not_overwrite_own_branch` | stored 1,f2,f3; `best_hash` = f3; peer h1..h3; `skip_divergence_check` | yes: index becomes h2,h3 |
| T-9 | `r1_backfill_heals_forged_low_blocks` | stored f1,f2 (forged), h3..h6; honest peer | yes: nothing repaired |
| T-10 | `r1_backfill_resumes_after_refusal` | T-3, then an honest peer; second pass imports 3, block 5 not fetched again | yes (first half) |
| T-11 | `r1_backfill_stops_where_peer_has_no_block` | stored 1,6; peer has 5,4,2 | yes: 2 written today |
| T-12 | `r1_repair_archive_refuses_forged_gap_bottom` | store 1,4; peer forged f2 (`prev` = h1), honest 3 | yes: `0000000002.block` written |
| T-13 | `r1_repair_archive_refuses_canonical_header_forged_body` | store 1..3; peer h2 header + changed body (copy `presence_variant`, `inc_i_281_ma_backfill.rs:77`) | yes: file written |
| T-14 | `r1_repair_archive_refuses_forged_h1_middle_top` | store 4,5; peer forged f1..f3 | no (regression) |
| T-15 | `r1_repair_archive_honest_peer_full_store` | store 1..5, empty archive → 5 files, `errors` 0 | no |
| T-16 | `r1_repair_archive_after_backfill_fills_store_gap` | store 1,4: repair refuses 2 (`errors` 1); backfill; repair writes 2 | yes: order not needed today |
| T-17 | unit tests in `backfill_anchor.rs` | `admit` three verdicts; `anchor_above` tip / stored / none; `holds` | new code |

Existing tests that must stay green after the fixture change: `req_auth_019_p13_backfill_forged_not_stored`,
`backfill_presence_junk`, the three `inc_i_308_m5_*`, all `audit_p1_rpc_005_*` and `audit_p1_rpc_007_*`.

## 12. Removed behavior and follow-up files

- Removed: the unanchored overwrite O-2 (section 5) and "skip a refused height, continue lower" (PRE-2).
- Docs to align (`/sync-docs`): `docs/rpc_reference.md:1896` (`skip_divergence_check` now skips the preflight
  only), `docs/archiver.md:277,361`, the `repairArchiveFromPeer` text from M8.
- `scripts/gauntlet-stranded/fastpath.py:244-250`: GS-052 needs another way to make its second hole, or it
  reports NOT_MADE.

## 13. Open risks

- R-1: GS-052 fixture breaks (F-16). Decision for the owner: change the fixture or accept NOT_MADE.
- R-2: A node that stored forged blocks before this fix heals only when the walk or the scan link check
  reaches a contradiction. A forged run that is complete from height 1 up to a block that links into the true
  chain cannot exist (the true block above commits to the true parent), so each forged run ends in a link
  break that C-2 finds. [I] — T-9 is the proof to run.
- R-3: `import.rs:241-260` (`check_links`) has the same weak rule for local archive files. A file forged
  before this fix can still be imported. Not a peer write; not in this change.
- R-4: The snap anchor shell with an empty body (`install_contract.rs:66-73`) counts as stored, so its real
  body is never fetched. Existing behavior; not changed.
- R-5: Wall time of the scan plus walk on mainnet (about 900,000 heights) is UNKNOWN. The scan is the
  existing cost; the walk adds 2 reads per gap.
- R-6: Block 1 `prev_hash` convention on mainnet was not read. The design does not depend on it.

## 14. Deploy questions

- Consensus RULES change? No. Block CONTENT change? No. No activation height. No genesis reset (rule #0).
- The change is local to the node that calls the method. A rolling deploy is safe [I] from F-14.

## 15. Quality audit

```
Major decisions: 1   Alternatives: 6   measured 3 / observed 1 / inferred 2 / assumed 0
Winner: Alt 3 conf(0.85, observed)   Flat distribution: no   conf >= 0.8 + assumed: 0
Subtraction alternatives: 1 (Alt 2)   Do nothing: 1 (Alt 1)   Winner cost: 0 new files (1 replaced)
Closest simpler alternative: Alt 4, conf 0.55   Constraint table entries: 0 (no failed approach on record for this path)
```
