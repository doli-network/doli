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

## Phase 2: UTXO Pruning & Compaction

**Problem:** NFT UTXOs with 408KB of data live in the UTXO set forever. With 1000 NFTs, that's 400MB in memory.

**Fix:** Tiered UTXO storage.

```
Hot tier:  In-memory HashMap — only UTXOs with extra_data < 4KB
Cold tier: RocksDB direct — UTXOs with extra_data >= 4KB (NFTs, large covenants)
```

- `get()` checks hot first, falls back to cold
- `spend()` works on both tiers
- NFT data is on-chain (in blocks) but not in RAM
- State root computation: stream cold UTXOs from disk, don't load all into memory

**Impact:**
- Memory usage: O(normal UTXOs) instead of O(all UTXOs including NFT blobs)
- 10,000 NFTs = ~4GB on disk, ~50MB in RAM (metadata only)

## Phase 3: Content-Addressed Data Deduplication

**Problem:** If 100 NFTs reference the same image, it's stored 100 times.

**Fix:** Separate content store with reference counting.

```
UTXO.extra_data = content_hash (32 bytes)
content_store[content_hash] = raw_bytes (408KB)
ref_count[content_hash] = number of UTXOs referencing it
```

- Mint: if content_hash exists, increment ref_count (no duplicate storage)
- Spend/burn: decrement ref_count, delete content when ref_count = 0
- State root: includes content_store merkle root
- Wire format: block carries content blobs only for NEW content_hashes

**Impact:**
- 100 copies of same image: 408KB once, not 40MB
- Block size for duplicate mints: ~300 bytes instead of 408KB

## Phase 4: Lazy Data Propagation

**Problem:** Every node stores every NFT's full data. A node that only validates transfers doesn't need the image bytes.

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

**Impact:**
- Producers: RAM and disk proportional to UTXO metadata only (~100 bytes per NFT, not 408KB)
- Archivers: full data, same as today's seeds with `--archive-to`
- Light nodes: sync headers + UTXO set without downloading NFT blobs
- Storage reduction for producers: 1000x for NFT-heavy chains
- Gossip bandwidth: proportional to transfers, not data size
- Network scales data storage with archiver count, not producer count

## Phase 5: Streaming UTXO Commitments

**Problem:** `compute_state_root()` iterates ALL UTXOs. At 1M UTXOs, this takes seconds.

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

**Impact:**
- State root computation: O(log n) instead of O(n)
- 1M UTXOs: ~20 hashes instead of iterating 1M entries
- Snap sync proofs: compact, verifiable without full UTXO set

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
| 2. UTXO tiered storage | ~200 lines | Memory scales with normal txs, not NFT data | v7 |
| 3. Content dedup | ~300 lines | Storage efficiency for collections | v7 |
| 4. Lazy data propagation | ~500 lines | Light nodes, bandwidth reduction | v8 |
| 5. Streaming commitments | ~400 lines | O(log n) state roots | v8 |
| 6. Parallel validation | ~600 lines | Multi-core block processing | v9 |

## Design Principles

1. **UTXO-native** — no accounts, no VM, no state rent. Data lives in outputs.
2. **Pay once, own forever** — no recurring storage fees. Fee at mint covers permanent inclusion.
3. **Producer decides** — block data budget is builder policy, not consensus. No hard forks for tuning.
4. **Prune without breaking** — data can be pruned from non-archival nodes without affecting consensus. State root proves it existed.
5. **Simple beats clever** — each phase is independent. Skip any phase without breaking the others.
