# UTXO/Data Handling Optimization Roadmap

> Goal: Make DOLI the most efficient UTXO chain for on-chain data — simple, powerful, brutally effective.

## Current State (v6.5.x)

- `extra_data` per output: 512KB base, grows with era (cap 8MB)
- NFT mint with `--data-file`: raw bytes embedded on-chain
- No per-block data limit → large NFTs cause gossip latency → missed slots
- Fee: 1 sat per 100 bytes (`FEE_PER_BYTE / FEE_DIVISOR`)
- UTXO set: in-memory `HashMap` + RocksDB persistence via `StateDb`
- Canonical serialization: bincode with u16→u32 escape for >64KB entries

## Phase 1: Block Data Budget (next release)

**Problem:** A 408KB NFT mint caused 1 missed slot. 20 simultaneous would cause ~10.

**Fix:** Soft cap on user data per block in the builder (not consensus).

```
assembly.rs: track cumulative_user_bytes
  → if next tx would exceed 1MB, skip it (stays in mempool for next block)
  → coinbase and epoch_reward exempt (protocol txs, not user data)
```

- ~5 lines in `assembly.rs`
- Not consensus-breaking — builder policy only
- Large NFTs still mint, just queued across blocks
- 20 × 408KB = distributed over ~8 blocks (~80 seconds) instead of 1

**Metrics:**
- Max block propagation time: <2s (currently ~0.5s for normal blocks)
- Zero missed slots from data-heavy blocks

## Phase 2: RocksDB Default (eliminate RAM bottleneck)

**Problem:** The entire UTXO set lives in RAM (`InMemoryUtxoStore`). With 1000 NFTs of 408KB each, that's 400MB of RAM on every node. Doesn't scale.

**Fix:** Activate `UtxoSet::RocksDb` as default. Already implemented, never used in production.

```
init.rs: use UtxoSet::RocksDb(state_db) instead of UtxoSet::InMemory(HashMap)
```

- RocksDB block cache automatically keeps hot UTXOs (recent transfers) in RAM
- Cold UTXOs (old NFTs) stay on disk — accessed only when spent or exported
- No tiered storage needed — RocksDB + OS page cache does the tiering automatically
- Lookup cost: ~1-5μs (vs 50ns HashMap). 40 lookups per block = 120μs. Slot is 10 seconds.

**Eliminates old Phase 2 (tiered storage)** — simpler, same result, zero new code.

**Impact:**
- RAM: proportional to block cache size (configurable), not UTXO set size
- 10,000 NFTs: ~4GB on disk, ~64MB block cache in RAM
- 100,000 UTXOs: same 64MB RAM, just more on disk
- Bitcoin Core made the same migration (in-memory → LevelDB) for the same reason

## Phase 3: Lazy Data Propagation (producers skip blobs)

**Problem:** Every node stores every NFT's full data. A producer that only validates transfers doesn't need the 408KB image bytes.

**Fix:** Role-based data availability. Three node tiers with different storage responsibilities.

```
Block header includes: data_root = BLAKE3(blob_hashes)

Producers/validators:
  - Do NOT download blobs
  - Validate data_root via attestation quorum — no blob download required
  - Produce blocks, attest, earn rewards without touching blob data
  - UTXO set contains blob hashes (32 bytes), not blob content
  - State root includes blob hashes, not blobs — identical across all nodes

Full archivers (seeds with --archive-to, explorers):
  - Download and store everything — blocks, blobs, full history
  - Serve GetBlockData() requests when users export NFTs
  - The canonical source of truth for on-chain data retrieval
  - Run by seed operators and explorer services

Light nodes:
  - Verify data_root from header
  - Download blobs on-demand via RPC from archivers
  - Minimal storage footprint
```

- New gossip message: `GetBlockData(hash, output_index)` → returns extra_data bytes (served by archivers only)
- Producers can prune blob data immediately — they never download it
- Data availability guaranteed by archiver attestation quorum, not by every node storing every blob
- State root does NOT change — includes hashes of blobs, not blobs. All nodes compute the same state root regardless of what data they store locally.
- Snap sync transfers UTXO hashes (32 bytes each), not blobs — fast regardless of NFT count

**Impact:**
- Producers: RAM and disk proportional to UTXO metadata only (~100 bytes per NFT, not 408KB)
- Archivers: full data, same as today's seeds with `--archive-to`
- Light nodes: sync headers + UTXO set without downloading NFT blobs
- Storage reduction for producers: 1000x for NFT-heavy chains
- Gossip bandwidth: proportional to transfers, not data size
- Network scales data storage with archiver count, not producer count

## Phase 4: MMR Incremental State Root (O(log n))

**Problem:** `compute_state_root()` iterates ALL UTXOs. With RocksDB (Phase 2), this means scanning disk. At 100K UTXOs = ~500ms. At 2TB = minutes. Doesn't scale.

**Fix:** Incremental state root via append-only Merkle Mountain Range (MMR).

```
Each apply_block:
  - Remove spent UTXOs from MMR (mark as spent, don't rebuild)
  - Append new UTXOs to MMR
  - State root = MMR root (O(log n) update, not O(n) rebuild)
```

- No full UTXO iteration at any point
- Snap sync: transfer MMR peaks + UTXO set
- Proof of inclusion: O(log n) for any UTXO
- Required before UTXO set exceeds ~100K entries

**Impact:**
- State root computation: O(log n) instead of O(n)
- 1M UTXOs: ~20 hashes instead of iterating 1M entries
- 2TB UTXO set: same ~20 hashes, no disk scan
- Snap sync proofs: compact, verifiable without full UTXO set

## Phase 5: Content-Addressed Data Deduplication

**Problem:** If 100 NFTs reference the same image (e.g., a collection), it's stored 100 times in the block store.

**Fix:** Separate content store with reference counting.

```
UTXO.extra_data = content_hash (32 bytes)
content_store[content_hash] = raw_bytes (408KB)
ref_count[content_hash] = number of UTXOs referencing it
```

- Mint: if content_hash exists, increment ref_count (no duplicate storage)
- Spend/burn: decrement ref_count, delete content when ref_count = 0
- Wire format: block carries content blobs only for NEW content_hashes
- Most useful for NFT collections (10K PFPs with shared layers)

**Impact:**
- 100 copies of same image: 408KB once, not 40MB
- Block size for duplicate mints: ~300 bytes instead of 408KB

## Phase 6: Parallel Transaction Validation

**Problem:** `apply_block` processes transactions sequentially. Large blocks with many txs bottleneck on single-core.

**Fix:** Dependency graph + parallel validation.

```
1. Build tx dependency graph (which txs spend which UTXOs)
2. Independent txs (no shared inputs/outputs) → validate in parallel
3. Dependent txs → sequential within dependency chain
4. UTXO mutations: apply in topological order after all validations pass
```

- Most blocks: 90%+ txs are independent (different senders)
- Signature verification (Ed25519): embarrassingly parallel
- BLS aggregate verification: already O(1)

**Impact:**
- Block validation: scales with CPU cores
- 100 txs on 8 cores: ~12x faster than sequential
- Enables higher throughput without reducing slot time

## Priority Order

| Phase | Effort | Impact | When |
|-------|--------|--------|------|
| 1. Block data budget | 5 lines | Prevents missed slots from large NFTs | Next release |
| 2. RocksDB default | ~50 lines | Eliminates RAM bottleneck, scales to any UTXO set size | v7 |
| 3. Lazy data propagation | ~500 lines | Producers skip blobs, snap sync lightweight | v7-v8 |
| 4. MMR incremental | ~400 lines | O(log n) state roots, scales to 2TB+ | v8 |
| 5. Content dedup | ~300 lines | Storage efficiency for NFT collections | v8-v9 |
| 6. Parallel validation | ~600 lines | Multi-core block processing | v9 |

## Scaling Profile

| UTXO Count | Phase 1 only | + Phase 2 | + Phase 3 | + Phase 4 |
|------------|-------------|-----------|-----------|-----------|
| 1K | 50MB RAM | 64MB cache | 64MB cache | 64MB cache |
| 10K | 4GB RAM | 64MB cache | 64MB cache | 64MB cache |
| 100K | 40GB RAM (OOM) | 64MB cache | 64MB cache | 64MB cache |
| 1M | impossible | 64MB cache, 500ms state root | 64MB cache, 500ms | 64MB cache, 20 hashes |
| 10M+ | impossible | impossible (state root) | impossible | 64MB cache, 24 hashes |

## Design Principles

1. **UTXO-native** — no accounts, no VM, no state rent. Data lives in outputs.
2. **Pay once, own forever** — no recurring storage fees. Fee at mint covers permanent inclusion.
3. **Producer decides** — block data budget is builder policy, not consensus. No hard forks for tuning.
4. **Prune without breaking** — data can be pruned from non-archival nodes without affecting consensus. State root proves it existed.
5. **Simple beats clever** — each phase is independent. Skip any phase without breaking the others.
6. **Let the OS work** — RocksDB block cache + OS page cache do automatic tiering. Don't reinvent LRU.
