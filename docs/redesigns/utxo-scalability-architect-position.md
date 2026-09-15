# UTXO / extra_data / snap-sync scalability — Architect position paper (DISCUSSION)

**Mode:** discussion with the user, not a locked spec. Nothing here is committed. No `specs/` file was touched.
**Inputs:** `docs/redesigns/utxo-scalability-redesign-analysis.md` (analyst fact base), `docs/.workflow/prompt-refinement.md`,
`specs/l2-settlement.md` §1/§6/§11, `specs/state-root-commitment-architecture.md` (prior decision, Tier 0 shipped M1).
**Date:** 2026-09-14. **Decision row:** `.omega/memory.db decisions.id=124` (status `proposed`, run 558).
**Binding constraints:** no genesis reset; forward activation at a NEW `NetworkParams` height only; no NFT image/file hosting;
government-adjacent conflict-free registries only; snap-sync must stay equivalent to full sync (INV-SYNC-007).

Spot-check of the analyst's file:line claims — 11 checked, 11 hold: `sync.rs:22,294` (16 MiB frame), `snapshot.rs:238`
(whole set, one blob), `output.rs:15,33-40` (512 KiB, era doubling), `utxo/types.rs:48-79` (61 B + extra), `block.rs:395-417`
(`data_root` ≥ 4096), `core.rs:695,711` (fee comment 1000× wrong; fee counts OUTPUT bytes only), `validation/transaction.rs:352-391`
(no dust floor, TOTAL_SUPPLY per output, era-aware extra_data check), `queries.rs:483-501` (2× materialization),
`zk.rs:44` (frozen), `defaults.rs:173,196,204` (defi/amm 0, oracle MAX), `constants.rs:503-511` (block-size era schedule).
No chunked sync path exists (`grep -i chunk crates/network/src/protocols/sync.rs crates/network/src/sync/` → 0 hits).

---

## A. Situation verdict

| # | Constraint | Kind | Who it hurts | Evidence |
|---|---|---|---|---|
| 1 | Snap-sync = ONE frame ≤ 16 MiB carrying the whole canonical UTXO set → ceiling ≈ 172,961 zero-payload UTXOs | **CLIFF** | **New nodes (liveness of joining)** | `sync.rs:22,294`, `snapshot.rs:238`, 97 B/entry |
| 2 | Adversary closes that door for **0.00147 DOLI**: 14,497,619 B headroom ÷ 524,288 = 28 outputs × 5,243 base units, ≤ 10 blocks | CLIFF, cheap | New nodes | `core.rs:711`, §4.3 of analysis |
| 3 | Cheaper still: **dust**. `minimum_fee` = 1 base unit per tx with zero extra_data; no dust floor, no output cap. ~30k Normal outputs per 1.9 MB block ≈ 3 MB of state/block ≈ **26 GB/day** for ~5 base units of fee + a refundable lock-up (output size ≈ 60 B bincode, UNVERIFIED ±30 %) | **GRADIENT that kills every node, then the cliff in ~5 blocks** | Existing nodes (RAM/disk), then new nodes | `transaction.rs:352-360` (`amount != 0` only), `core.rs:711` |
| 4 | Resident state is O(N + Σ extra_data) on every node; state-root compute O(N log N) + 2× transient copy, now lazy/memoized per tip (Tier 0 shipped) | GRADIENT | Existing nodes | `in_memory.rs:22-23,360-370`, `queries.rs:483-501`, `state_root_serve.rs:1-13` |
| 5 | Block archive grows with block bytes, append-only; `prune_blocks_below` is reachable only from an RPC (`rpc/methods/pruning.rs:84`), never from the apply loop | GRADIENT, slow | Archive/seed nodes | `archiver.rs`, analysis §4.4 |
| 6 | Era doubling (block size, extra_data) is unconditional; `MAX_SYNC_SIZE` never grows → the gap widens 2×/4 y | Worsener of #1 | New nodes | `constants.rs:503-511`, `output.rs:33-40` |
| 7 | Throughput: ~300 TPS builder budget (`constants.rs:449`), per-block apply O(tx) | NOT binding for a registry (100k anchors/day = 1.2 TPS) | — | analysis §3 |
| 8 | "Max transferable amount": per-output ≤ TOTAL_SUPPLY = 25,228,800 DOLI, running total `checked_add`; non-native assets exempt from TOTAL_SUPPLY, bounded only by u64 | **NON-ISSUE** (see C.6) | — | `transaction.rs:365-379` |

**Binding order as usage grows:** #3/#2 (cheap adversarial fill) → #1 (benign fill reaches the cliff in months on testnet baseline, days at mainnet producer count) → #4 (RAM/root gradient) → #5 (archive). Nothing about tx throughput binds for the registry use-class.

**Premise corrections, restated from the analyst (all verified by me):**
1. The 512 KiB limit is **per OUTPUT** (`output.rs:15`), there is **no per-tx cap**, and the schedule doubles it to 8 MiB.
2. The "parameter programmed to grow every few years" is the **block size / extra_data** schedule, **not snap-sync**. Snap-sync is a fixed 16 MiB and the schedule makes the mismatch worse.
3. NFTs store a **32-byte content_hash only** (`output.rs:250-268`); bulk payload lives only in `EncryptedContent` (inline ciphertext). **NFT fractionalization does not exist** (TxType 29/30 tombstoned, `types.rs:113-123`); fractional ownership is composable via `Multisig + MintAsset + BurnAsset`.

**My corrections to the analyst:**
- "No new node can ever join" past the ceiling is overstated: **snap-sync dies**; block-by-block sync from an archive seed remains in the code path. Whether a fresh mainnet node completes it today is **UNVERIFIED** (INC-I-143 D6/D7 archive issues are open). Either way the designed entry path is gone.
- `defi_activation_height` is **never read by the validator** (only the 4 AMM types are gated, `transaction.rs:90-110`); `MintAsset`/`BurnAsset` are **ungated**, not merely "active at 0". CLAUDE.md drift confirmed.
- The analyst missed that **`Transaction.extra_data` (tx-level, `core.rs:42`) is accepted on `Transfer` with no validation, no bound beyond block size, no fee** (`transaction.rs:126-129`, `core.rs:711`) — and is **excluded from the signing message** (`core.rs:512-536`). It is a free, unsigned archive-bloat channel today, and the reason a tx-level anchor cannot be the end state (C.2).

---

## B. SSF candidate (one solution, presented alone)

> **Commit and stream, never store-and-ship: a registry anchor becomes a signed zero-amount `OutputType::Anchor` (≤ 64 B) that `apply_block` never inserts into the UTXO set, and snap-sync serves the existing canonical UTXO serialization as sorted-outpoint-range chunks streamed straight into `state_db` instead of one 16 MiB frame.**

**Mechanism (one paragraph).** An anchor is a fact about history, not spendable state: it needs to be in a block (committed by `merkle_root`, `block.rs:26`, since `Transaction::hash` covers every output, `core.rs:499-506`) and provable by inclusion, and it never needs to be spent. So it should cost **zero bytes of consensus state**, exactly as Bitcoin prunes `OP_RETURN` outputs from its UTXO set. The Anchor output is signed (outputs are in the signing message, `core.rs:527-536`), priced by the existing per-output byte fee, bounded to 64 B, and skipped at the single insertion point in `apply_block/tx_processing.rs`; rollback and rebuild replay the same skip, so all three state paths stay bit-identical (INV-SYNC-007). Separately, the transport cliff disappears without touching consensus: the canonical serialization is already `[8 B count][entries sorted by outpoint]` (`in_memory.rs:360-370`), and the RocksDB `CF_UTXO` iterator yields that same order unsorted (`queries.rs:487-491` never sorts yet must equal the InMemory bytes under INV-SYNC-007), so a peer can serve any `[start_key, +1 MiB)` range in O(chunk) and the receiver can hash the stream incrementally, write each chunk into a `state_db` batch (INV-SYNC-014 satisfied by construction), and compare the final `state_root` with the quorum root it already votes on (`snap_sync.rs:83-106`). No new commitment, no activation height, rolling deploy.

**Satisfies:** no genesis reset (both legs are forward-only: Anchor at a NEW AH; chunking is wire-only). Snap ≡ full sync (same bytes, same root). Does not foreclose L2 (`ZKSettle=31`/`ZKRollup=13` untouched; discriminant 16 for Anchor, 23-28/33+ still free) nor registry-at-scale (C.4). Nothing here touches `ProducerSet`, the scheduler, or epoch boundaries.

**conf(0.72, observed)** — every mechanism precondition was read in code; nothing is benchmarked yet (basis cannot be `measured` until a chunk prototype runs on the local testnet).

If you accept this, it is the design. Section C is what you asked for on top: taking it to its end state and showing the numbers.

---

## C. The scaling architecture: "commit-and-stream"

### C.1 Where do registry anchors live? (argued with numbers)

| Home | Bytes of consensus state per anchor | 100k anchors/day, 5 y: state | 100k/day, 5 y: archive | Snap-sync | Provable to a non-node |
|---|---|---|---|---|---|
| **Today** (NFT/Hashlock output, 131-196 B resident) | 131-196 | **24-36 GB** in RAM/state + in every snapshot | 55 GB | dead at 16 MiB | via `getUtxo` while unspent |
| tx-level `extra_data` (works today, 0 code) | **0** | 0 | ~55 GB | fine | inclusion proof — but **unsigned, relayer-malleable** |
| **Anchor output, pruned at apply (proposed)** | **0** | **0** | ~55 GB (≈ 300 B/tx, UNVERIFIED ±30 %) | fine | inclusion proof: header chain + `merkle_root` path |
| Separate committed accumulator (new root in header) | 0 state, +32 B header | 0 | same | fine | proof vs new root — **block CONTENT change, synchronized deploy, header format change** |
| L2 with `ZKSettle` | O(1) per rollup (root + VK) | ~0 | proofs only | fine | needs L2 DA + L1 proof; roadmap gated on a builder (§11) |

Archive math: 100,000 × 300 B × 365 × 5 = 54.75 GB, archive nodes only; state growth **0**. Versus today: 35.8 GB of *state* (analysis §4.4), 426× over the frame ceiling. The anchor's home is the **block**, committed by the existing `merkle_root`; the UTXO set is not the right data structure for it and never was.

**How scalability is achieved, exactly:** documents/day is decoupled from chain load by the registry's own Merkle batching (one anchor per batch, e.g. one per block per registry → ≤ 8,640 anchors/day/registry for any number of documents); anchors/day is decoupled from state by zero residency; the remaining chain cost is archive bytes (linear, archive nodes only) and TPS (100 registries × 8,640 = 864k anchors/day = 10 TPS vs a 300 TPS budget). L1 saturates only at ≈ 26M individual anchor txs/day (300 × 86,400) — that, not state, is the trigger for Stage 5 (L2).

### C.2 Why not the tx-level `extra_data` path (it is free today)?

It is the interim, not the end state. It is excluded from the signing message (`core.rs:512-536`) so a relayer or producer can strip or replace the bytes and the tx stays valid; it is unpriced (`core.rs:711`) and unbounded below block size; the registry's signature never covers the anchor. A registry can self-sign the payload (`[hash | sig]`, 96 B) to reduce this to a DoS, but a pruned Anchor output is signed for free by the existing tx signature, priced by the existing byte fee, and matches the Bitcoin precedent. Stage 3 must bound and price tx-level bytes regardless (they are an archive-bloat channel today).

### C.3 How the UTXO set stays bounded independent of history — options compared

| Option | Mechanism | Q1 consensus RULES? | Q2 block CONTENT? | Blast radius (apply / rollback / rebuild / fork-recovery) | Hardware | Forecloses | Verdict |
|---|---|---|---|---|---|---|---|
| **State expiry / rent** | outputs untouched N blocks are evicted or must pay rent | YES (AH) | NO | **apply: eviction pass; rollback: must restore evicted sets across the boundary (undo window is 100 blocks, `constants.rs:253`); rebuild: snap-synced nodes lack history to know what expired (INV-EPOCH-002 class);** fork-recovery: eviction must be undone on reorg | O(evictions)/block; index by last-touch | breaks REQ-SCALE-005 (every existing UTXO spendable) unless grandfathered → then it bounds nothing old | **REJECTED** for this use-class: anchors are history, not state; expiry solves the wrong object |
| **Commitment-only anchoring (Anchor output, pruned)** | anchor never enters state; document stays with the registry | YES (AH: new valid output type) | only post-AH via txs pre-AH binaries reject | apply: skip insert at one site; rollback: no-op (never inserted; undo must not list it); rebuild: same skip; fork-recovery: none | +0 | nothing | **CHOSEN** for registries |
| **Chunked, Merkle-verified snapshot** | stream the existing canonical bytes by sorted key range; verify stream hash vs quorum root | NO | NO | none in apply/rollback/rebuild; touches `sync.rs`, `snap_sync.rs` (hotspot, 23 incidents), `state_snapshot_serve.rs`, `fork_recovery.rs:318-390`, `queries.rs` | install RAM O(1 MiB chunk) + RocksDB write buffer (INV-STORAGE-001); serve O(chunk) via range iterator | nothing; per-chunk *consensus* proofs come later with the state-root spec's merkelized-per-component step | **CHOSEN** for transport (cliff) |
| **utreexo-style accumulator** | nodes keep a log-size accumulator; spenders carry inclusion proofs | YES | YES (tx format) | wholesale: tx format, mempool, wallet/CLI proof maintenance, every validator | O(log N) hashing per input | nothing, but institutions must run proof-maintenance services | REJECTED now: no consumer, largest surface; revisit at 10^8 UTXOs |
| **Registries on an L2 (ZKSettle)** | L1 holds root + VK per rollup | YES (already reserved) | YES at its AH | per `specs/l2-settlement.md` §6 | ZK verify budget/block | nothing | **Stage 5**, only when volume > L1 budget or privacy/ordering needs; roadmap is builder-gated |
| Raise `MAX_SYNC_SIZE` | constant | NO | NO | none | O(N) allocation per stream (INC-I-012 F13 DoS) | — | REJECTED (REQ-SCALE-021): symptom patch |

**What still grows after the chosen options:** spendable coin/bond/asset/pool outputs — i.e. real users. That growth must be *priced and capped*, which is Stage 3, and it is **not optional**: chunking removes the cliff but the zero-fee dust gradient (#3) would still fill 26 GB/day.

### C.4 Chosen end state and migration path (forward-activated, each stage shippable alone)

| Stage | What | Consensus RULES (Q1) | Block CONTENT (Q2) | Deploy | NetworkParams field | Blast radius (files) | Hardware (CPU / mem / IO / lock) |
|---|---|---|---|---|---|---|---|
| **S1 chunked snapshot** | `GetStateManifest{block_hash}` → `{state_root, cs_bytes, ps_bytes, utxo_count, utxo_bytes_total}`; `GetStateChunk{block_hash, start_key, max_bytes}` → `{entries, next_key}`; receiver streams to a `state_db` batch, hashes incrementally, verifies `state_root` vs quorum at the end; legacy single-frame kept as fallback while ≤ 16 MiB | **NO** | **NO** | **rolling**; protocol negotiation (old peer → `SyncResponse::Error` → legacy path) | none | `protocols/sync.rs` (+2 req/+2 resp), `sync/manager/snap_sync.rs` (download loop only — no admission-logic change), `node/state_snapshot_serve.rs`, `node/fork_recovery.rs:318-390` (install writes to state_db directly, drops the InMemory detour), `state_db/queries.rs` (range serializer) | serve: O(chunk) CPU/IO per request, no full materialization; install: RAM O(chunk) instead of O(N); locks: same read guards as today, held per chunk not per set |
| **S2 Anchor output** | `OutputType::Anchor = 16`; `amount == 0` allowed (join the list at `transaction.rs:352-355`); `extra_data ≤ 64 B`; not inserted at apply; not in undo; `doli anchor <hash>` + `getAnchorProof(txid)` (header chain + merkle path, served by archive nodes) | **YES** | post-AH only, via txs both binaries reject pre-AH | **rolling before AH**; AH pinned after fleet + external producers upgrade (externals need an AH — memory) | `anchor_output_activation_height` (NEW; devnet `u64::MAX` until pinned) | `transaction/types.rs`, `validation/transaction.rs`, `apply_block/tx_processing.rs`, `state_db/batch.rs` (or caller-side skip), `UndoData` shape (UNVERIFIED — must exclude the outpoint), CLI/RPC (non-consensus) | +0 state; +1 branch per output at apply |
| **S3 state pricing + growth cap** | `minimum_fee_at(height)` = BASE + byte_fee(tx bytes incl. tx-level extra_data) + `STATE_FEE × Σ(97 + extra_data)` over *resident* outputs created; per-block `MAX_RESIDENT_BYTES_PER_BLOCK`; per-type resident `extra_data` cap forward-only (EncryptedContent only DOWN per REQ-SCALE-012); tx-level `extra_data` bounded per type (Transfer ≤ 128 B) | **YES** | NO (validation; builder mirrors the budget) | **rolling before AH** | `state_pricing_activation_height` (NEW, own field; may co-activate with S2 at the same height but never the same field) | `transaction/core.rs` (height-aware fee), `validation/transaction.rs`, `validation/block.rs` (per-block resident sum), `production/assembly.rs` (budget), mempool policy | +O(outputs) arithmetic per tx; no new locks |
| **S4 Tier-1 multiset digest** | already decided in `specs/state-root-commitment-architecture.md` (LtHash, folded at `BlockBatch.add_utxo/spend_utxo`); trigger: `doli_utxo_canonical_size_bytes ≥ 4 MB` (`metrics.rs:245-268`) + flamegraph | YES (own AH, **no version bump** — Universal Filter) | NO | per that spec | per that spec | per that spec | O(1) per changed entry; kills the residual O(N) scan on serve |
| **S5 L2 registries** | `ZKSettle`/`ZKRollup` per `specs/l2-settlement.md` §7 | YES | YES at its AH | per spec | `ProtocolActivation` | per spec §6 | ZK budget |

INC-I-075 three-question checklist for S2: (1) user tx reaches the path → YES; (2) producer/attestation reaches it → NO; (3) bit-identical for all inputs → NO (Anchor accepted post-AH). ⇒ **AH required**, own field. Same answers for S3.

Invariants: INV-SYNC-007 — S1 changes no bytes or formula; S2 changes state identically on the single apply path. INV-SYNC-014 — S1 never materializes InMemory. INV-EPOCH-002 — untouched (no rebuild-from-history dependency added). INV-UTXO-001 — Anchor `amount == 0`, conservation arithmetic unchanged. INV-STORAGE-108 — nothing new per block in the apply loop. INV-EPOCH-001 — no `CURRENT_PROTOCOL_VERSION` change anywhere.

Alignment with the prior state-root decision: S1 is the "per-key-provable snap-sync" consumer that REQ-SROOT-009 said did not exist yet — but it does **not** require per-key proofs (stream-hash + quorum suffices), so it does not pull Tier 2 forward. S4 stays exactly as that spec left it.

### C.5 The math: adversarial fill and honest cost

| Attack | Today | After S1 | After S1+S2+S3 (illustrative params: `STATE_FEE` = 1 DOLI/MB resident, cap 64 KiB resident/block) |
|---|---|---|---|
| Close snap-sync for all future nodes | **0.00147 DOLI, ≤ 10 blocks** | impossible (no frame ceiling) | impossible |
| extra_data fill (3 × 512 KiB/block) | 12.66 GiB/day for 1.36 DOLI/day | same bytes, gradient only | capped at 64 KiB/block → **553 MB/day max**, costing 0.0625 DOLI/block = **540 DOLI/day**; resident cap per non-ZKRollup type shrinks the per-output vector further |
| Dust fill (~30k × 97 B outputs/block) | **≈ 26 GB/day for ~5 base units/day + refundable lock-up** | same, gradient only | same cap → 553 MB/day, 540 DOLI/day; a producer paying itself recovers only its bond-weighted share because fees enter the coinbase → reward pool (`assembly.rs:378`) |
| Worst-case state growth / yr | unbounded (cliff first) | unbounded gradient | **≤ 202 GB/yr at 197k DOLI/yr (0.78 % of supply)** — Bitcoin-parity order (Bitcoin worst ≈ 576 MB/day) |
| Honest registry, 100k anchors/day | 19.6 MB/day of *state*, dead in 7 days | 19.6 MB/day of state, ~1 GB RAM in 2 months | **0 state; 30 MB/day archive; fee ≈ 1-2 base units per anchor** (64 B → byte fee rounds to 1) |

Parameters (`STATE_FEE`, resident cap, per-type caps) are the economist's to set; the table shows shape, not values. With `STATE_FEE` = 10 DOLI/MB the annual worst case costs 7.8 % of supply. Note what S3 does *not* need: a dust floor (the resident-byte price is the floor) or rent (no expiry, no rollback hazard).

### C.6 "Max transferable amount" — non-issue for this use-class

Per-output bound is TOTAL_SUPPLY = 25,228,800 DOLI (`transaction.rs:365-370`); the per-tx running total is `checked_add` against the same bound (`:373-379`); there is no per-tx cap below that. A native transfer can move the entire supply in one output. Non-native assets (`MintAsset`) are exempt from TOTAL_SUPPLY and bounded only by u64 (1.8 × 10^19 units); at 8 implied decimals that is 1.8 × 10^11 whole units per output. The only real limit is **precision** (8 decimals fixed, 1 DOLI = 10^8 base units), which is adequate for fiat-denominated registry fees or asset units. Payment *volume* is a TPS question (C.1), not an amount question.

---

## D. Adversarial pass on my own proposal

| # | Attack / failure mode | Survives? | Cost / residual |
|---|---|---|---|
| 1 | **State bloat via dust or extra_data after S1** (cliff gone, gradient remains at 26 GB/day for free) | Only with **S3** — S1 alone is a liveness fix, not a bound | S3 is mandatory, not optional; needs an economist decision on `STATE_FEE` and the per-block cap |
| 2 | **Self-paying producer** bypasses the mempool policy (`max_tx_size` is not consensus, `policy.rs:28`) and fills its own block | Yes, via the **per-block resident-byte cap** (consensus) — price alone would not stop it (fee returns to the pool, producer recovers its share) | cap value calibrates worst-case growth: 64 KiB/block → 553 MB/day; must be tested at small (5 producers) and large (~50-100) scale — the cap is per block, so producer count does not change it |
| 3 | **DA withholding** — registry loses the document; anchor proves "this hash existed at height h" only | Yes: that is the *intended* guarantee (C.1); the chain never promised content. Fractional-ownership state is in UTXOs (asset units), not off-chain. S5 inherits the L2 spec's DA gap (§2: "L1 is not responsible for L2 DA") | none on L1; a registry needs its own retention policy |
| 4 | **Snapshot equivocation with chunks** — a peer serves a manifest/chunks that hash to the wrong root | Yes, detected at the end (final `state_root` vs quorum) — same guarantee as today (`snap_sync.rs:78-82`: "real protection is verification after download"); bounded waste ≤ set size per bad peer; mitigation: manifest `(chunk_count, utxo_bytes_total)` cross-checked across ≥ 2 quorum peers before download, bad peer scored (`scoring.rs`) | per-chunk *consensus* proofs only arrive with the state-root spec's merkelized-per-component step (S4+); until then equivocation costs bandwidth, never correctness |
| 5 | **Rollback across a boundary** | No expiry ⇒ no boundary. Anchor undo: outpoint never inserted ⇒ undo must not delete it (or the delete is a no-op) — `UndoData` shape UNVERIFIED, implementation must assert `Anchor ∉ created_outpoints` | one test: apply → rollback → root identity at AH±1 |
| 6 | **Snap-synced node lacking history** (INV-EPOCH-002 class; "rebuild-from-blocks is unsafe on snap-synced nodes") | Yes: no stage adds a rebuild-from-history dependency; a snap-synced node cannot serve *old* anchor proofs, so **archive nodes are a required role** for `getAnchorProof` — they already exist (seeds + `archiver.rs`) | document the role; light clients need header chain + path from an archive node |
| 7 | **Fee-market failure** (DOLI price ≈ 0, `STATE_FEE` meaningless) | Structurally yes: the per-block cap is the backstop, price is the deterrent | cap must be set as if price were zero |
| 8 | **Governance capture of the registry** | Yes: no chain-level registry allowlist exists or is added; a "registry" is a key (or `Multisig ≤ 127` keys) plus a convention; capture happens only in the registry's own key management | none on chain |
| 9 | **Sync-manager hotspot** (23 incidents; INC-I-143 open) | Partially: S1 adds a download-stage path but **no admission-logic change**; the legacy single-frame stays as fallback | highest implementation risk of the whole plan; ship S1 behind the manifest negotiation, testnet soak with a seed under repeated snap requests |
| 10 | **Era doubling widens the payload vector** after S3 caps | Yes: S3's resident caps are new constants that do **not** follow the era schedule (REQ-SCALE-023: the schedule itself is untouched); block size may keep doubling because bytes no longer imply state | none |
| 11 | **ZKRollup proofs (100-400 KB) resident in the UTXO entry forever** (REQ-SCALE-008 wants `extra_data` to still hold them) | Yes, S3 exempts `ZKRollup` from the per-type resident cap (one entry per rollup, replaced on each settle ⇒ bounded per rollup). Better: the l2 spec could carry the proof in tx-level bytes at settle time and keep only `root + VK` resident — a non-foreclosing refinement for that spec, not decided here | none |

---

## E. External variables (technical only; no jurisdictional claims)

| Variable | Design consequence |
|---|---|
| **Data residency** | Only a 32-64 B commitment leaves the registry; the document never touches the chain. `EncryptedContent` (inline ciphertext) is the opposite posture — S3 shrinks its cap (only down) so DOLI cannot drift into hosting. |
| **Right to erasure** | A raw hash of personal data may be treated as personal data. Recommended anchor payload: `H(doc ‖ salt)` with the salt held by the registry (payload 32 B; or 64 B with a namespace tag). Destroying the salt makes the on-chain value unlinkable; nothing on chain must change. This is why commitment-only anchoring, not payload anchoring, is the only compatible design. |
| **Auditability by non-node parties** | Proof = header chain (14-field header, `block.rs:20-75`; header bytes/day UNVERIFIED) + `merkle_root` path (`compute_merkle_root`, `block.rs:258`) + the registry's batch path. A verifier needs headers and one archive node, not a full node. `getAnchorProof` is non-consensus tooling. |
| **Key custody for institutions** | `Condition::Multisig`/`Threshold` (≤ 127 keys, `conditions/mod.rs`) already give m-of-n; institutional key rotation = spend-to-new-key of a registry identity output (stable `token_id`). HSM signing must produce Ed25519 over the `blake3_256` prehash (`doli sign` convention, memory). Wallet layer, not consensus. |
| **30-year verifiability** | The proof depends on BLAKE3, Ed25519, and the header chain. Rule: any future header/hash change must commit the pre-change header chain root into the first post-change header (checkpoint) so old inclusion proofs stay verifiable. Version the Anchor payload (1 B) so it remains parseable. The archive (not the UTXO set) is the long-horizon artifact — pruning policy for archive nodes must be a stated operator commitment. |
| **Per-anchor cost stability** | Fees are in DOLI; a registry budgeting in fiat needs either a fee schedule denominated in bytes (as today) with a price floor, or the (frozen) oracle. S3's byte-denominated `STATE_FEE` keeps registry cost near zero regardless (anchors are non-resident). |

---

## F. Discussion agenda (answers change the architecture)

1. **Batch or per-document?** Will registries anchor one Merkle batch per block (L1 never saturates) or must every document be its own L1 tx (L1 saturates at ≈ 26M/day ⇒ S5 timing becomes real)?
2. **Accept that an anchor is not a UTXO?** It is provable by inclusion via archive nodes/RPC, not queryable by `getUtxo`. If you need "retrievable from live state forever", the design degrades to a resident `Hashlock` (131 B each) and the bound is lost.
3. **Stage 3 mandate:** are you willing to change fee rules and add a per-block resident-byte cap at a future height (economist sets `STATE_FEE` and the cap)? Without S3 the plan removes the cliff but leaves a free 26 GB/day gradient.
4. **Payload direction:** reduce the state-resident `extra_data` cap for every type except `ZKRollup` at a future height (existing UTXOs grandfathered), `EncryptedContent` only down — yes/no? And may the l2 spec be refined to keep only `root + VK` resident?
5. **Pilot sequencing:** use tx-level `extra_data` anchors *today* (zero code, unsigned — DoS-malleable only if the registry self-signs the payload) for a non-adversarial pilot while S1/S2 ship — acceptable, or wait for S2?
6. **Erasure posture:** salted commitments (registry holds salts) as the documented convention — acceptable to the registry model, and 32 B or 64 B payload?

Resolved by this analysis (dropped from the analyst's §9): scale target (decoupled by batching + zero residency), who-runs-a-node (install RAM becomes O(chunk); production backend is `state_db`, `init.rs:317`), L2 timing (S5, throughput-triggered only).

---

## G. Resource cost

```
━━━ RESOURCE COST — COST-DECLARED ━━━   (SSF stage alone: S1 chunked snapshot + S2 Anchor output)
Dimensions:
  CPU:      +O(chunk) per served chunk instead of O(N) per served snapshot; +1 branch per output at apply (inferred)
  Memory:   install −O(N)→O(1 MiB chunk)+RocksDB write buffer; serve −O(N) materialization→O(chunk); anchors −196 B→0 B each (observed)
  IO:       install writes the same bytes, batched per chunk; serve = range reads (observed); +~300 B archive per anchor tx (assumed)
  Network:  same total bytes as today per snap-sync, more round-trips (1 per MiB); manifest +~100 B (inferred)
  Disk:     state +0 per anchor; archive +~300 B per anchor (assumed)
  Latency:  snap-sync at 10M UTXOs ≈ 970 MB → 97 s at 10 MB/s, ~8 min at 2 MB/s, vs impossible today (inferred)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS — tx-level extra_data anchors are cheaper in code (0) but unsigned/unpriced; raising MAX_SYNC_SIZE is a symptom patch (REQ-SCALE-021)
Why this proposal anyway: it is the smallest change that removes the cliff and makes registry cost independent of anchor count without touching consensus state formulas
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

```
━━━ RESOURCE COST — COST-DECLARED ━━━   (chosen end state: S1 + S2 + S3, with S4 as already decided elsewhere)
Dimensions:
  CPU:      S3 +O(outputs) fee arithmetic per tx and one per-block resident-byte sum (inferred); S4 O(1) per changed entry replacing the O(N) serve scan (per state-root spec)
  Memory:   bounded worst-case state growth ≤ cap × 8,640/day (e.g. 64 KiB → 553 MB/day) instead of unbounded (inferred); install/serve O(chunk) (observed)
  IO:       unchanged per block; worst-case state write rate capped by the same constant (inferred)
  Network:  unchanged beyond S1 (observed)
  Disk:     state ≤ 202 GB/yr adversarial worst case at the illustrative cap, 0 for anchors; archive linear in block bytes (inferred)
  Latency:  no per-block latency added in the apply loop (INV-STORAGE-108 respected) (inferred)
Inevitability: INEVITABLE
Cheaper alternative: S1 alone (cliff only) — rejected because the free dust gradient (26 GB/day) remains
Why this proposal anyway: it is the only combination that bounds state independent of history without expiry, rent, or a new commitment structure
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

---

## Appendix — evidence index used above

| Claim | file:line |
|---|---|
| 16 MiB frame, request + response | `crates/network/src/protocols/sync.rs:22,268,294` |
| whole-set blob in snapshot | `crates/storage/src/snapshot.rs:238`; wire struct `sync.rs:101-130` |
| canonical entry 36 + 61 + extra | `crates/storage/src/utxo/types.rs:48-79`, `in_memory.rs:360-370` |
| RocksDB serializer iterates unsorted, 2× materialization | `crates/storage/src/state_db/queries.rs:483-501` |
| state root = H(H(cs)‖H(utxo)‖H(ps)) | `crates/storage/src/snapshot.rs:68-101` |
| lazy memoized root (Tier 0 M1) | `bins/node/src/node/state_root_serve.rs:1-13` |
| snapshot install path, InMemory detour then state_db import | `bins/node/src/node/fork_recovery.rs:318-390`; production backend `init.rs:317` |
| quorum vote on state_root | `crates/network/src/sync/manager/snap_sync.rs:78-106` |
| Transfer tx-level extra_data unvalidated | `crates/core/src/validation/transaction.rs:126-129` |
| tx hash covers extra_data; signing message excludes it | `crates/core/src/transaction/core.rs:484-508, 512-536` |
| minimum_fee counts output bytes only; comment 1000× off | `crates/core/src/transaction/core.rs:695-714` |
| amount == 0 allow-list; TOTAL_SUPPLY; extra_data era check | `crates/core/src/validation/transaction.rs:352-391` |
| only AMM types are height-gated; defi field unused | `crates/core/src/validation/transaction.rs:90-110` |
| fees → coinbase → pool | `bins/node/src/node/production/assembly.rs:378` |
| header `merkle_root`, `data_root` | `crates/core/src/block.rs:26,66,395-417`; `compute_merkle_root` `:258` |
| undo depth 100, O(1) prune | `crates/core/src/consensus/constants.rs:253`; `state_db/undo.rs:132` |
| block pruning only via RPC | `crates/rpc/src/methods/pruning.rs:84` |
| canonical-size metric + thresholds | `bins/node/src/metrics.rs:245-268,867` |
| era schedules | `crates/core/src/consensus/constants.rs:503-511`; `transaction/output.rs:33-40` |
| ZKSettle frozen | `crates/core/src/validation/zk.rs:44` |
| NetworkParams AH fields (30 existing, own-field discipline) | `crates/core/src/network_params/mod.rs:182-854` |
