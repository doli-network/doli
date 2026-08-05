# State-Root / 3-State Commitment — Redesign Analysis (PROPOSAL ONLY)

> Analyst milestone for `/omega-redesign`. Scope THE PROBLEM, not features. Code is the source of truth.
> Every claim below cites `file:line`. No code changes proposed here — this is analysis for the architect.

## Scope
The 3-state commitment (`state_root`) anchored at `crates/storage/src/snapshot.rs`, and its full
blast radius across production apply, validation, snap-sync (build/install), rollback/reorg, and RPC.

---

## 1. Verify Every Code Claim

| # | Claim (from refined prompt) | Verdict | Evidence |
|---|------------------------------|---------|----------|
| a | `compute_state_root()` at `snapshot.rs:24`; `H(H(cs)‖H(utxo)‖H(ps))`, BLAKE3 | **VERIFIED** | `crates/storage/src/snapshot.rs:24-59` |
| b | Sizes cs ~140 B / utxo ~783 KB / ps ~238 KB | **VERIFIED (cs exact); utxo/ps env-dependent** | cs fixed 140 B `chain_state.rs:143`; utxo/ps are variable — a live snap install showed `utxo_bytes=40089` (`docs/bugfixes/inc-i-118-diagnosis.md:29`) |
| c | `:86 compute_state_root_with_epoch_state`, `:164 compute_scheduler_root`, `:253 compute_state_root_from_bytes` | **VERIFIED** | `snapshot.rs:86`, `:164`, `:253` |
| d | "~15 call-sites" | **CORRECTED-TO: 6 non-test invocations** | see §1d table |
| e | UTXO RocksDB-backed via `UtxoSet::from_state_db` at `init.rs:311` | **VERIFIED** | `bins/node/src/node/init.rs:311`; ProducerSet also disk-backed (`init.rs:580 state_db.load_producer_set()`), held in-memory `Arc<RwLock>`; ChainState in-memory |
| f | Metric `doli_utxo_canonical_size_bytes` vs threshold 16 MB | **VERIFIED** | gauge `bins/node/src/metrics.rs:234`; threshold `metrics.rs:243` set to `16*1024*1024` at `metrics.rs:522`; warn alert at `>12582912` (12 MB) `metrics.rs:222,714` |
| g | `EPOCH_SNAPSHOT_HF` (INC-I-034) HF-gated root-change mechanism | **VERIFIED (scaffolding only, unwired)** | `crates/updater/src/hardfork.rs:180-230` |

### 1a — Exact structure of `compute_state_root` (`snapshot.rs:24-59`)
```
cs_bytes  = chain_state.serialize_canonical()      // [u8;140] fixed
utxo_bytes= utxo_set.serialize_canonical()         // Vec<u8>, sorted-by-outpoint
ps_bytes  = producer_set.serialize_canonical()     // Vec<u8>, sorted-by-pubkey-hash
combined  = cs_hash(32) ‖ utxo_hash(32) ‖ ps_hash(32)   // 96 bytes, fixed order
state_root= BLAKE3(combined)
```
It is **nested** `H( H(cs) ‖ H(utxo) ‖ H(ps) )`, NOT a flat `H(cs‖utxo‖ps)`. No length prefixes on the
outer concat (each component hash is fixed 32 B). Component order is fixed cs→utxo→ps. Confirmed `snapshot.rs:53-58`.

### 1b — What each `serialize_canonical()` materializes
- **ChainState** (`chain_state.rs:143`): writes a fixed `[u8;140]` on the stack. No alloc growth, no sort. Negligible.
- **ProducerSet** (`producer/set_persistence.rs:78`): collects `&producers` into a `Vec`, `sort_by_key`, then **clones each `ProducerInfo`**, sorts its 3 sub-vectors (`additional_bonds`, `received_delegations`, `bond_entries`), and `bincode::serialize`s each into a growing `Vec`. O(P·log P) + per-producer clone+bincode. Small at ~14–30 producers; grows with producer count.
- **UtxoSet** dispatch (`utxo/set.rs:421`) → for the production RocksDB backend calls `serialize_canonical_utxo` (`state_db/queries.rs:473`): **full `CF_UTXO` iterator scan from Start**, `bincode::deserialize::<UtxoEntry>` for **every** entry, `collect` into a `Vec`, then re-serialize each canonically into a `Vec::with_capacity(8 + n*95)`. This is a **full RocksDB table scan + N deserializations + full re-serialization + full buffer alloc on every root computation.** The in-memory backend (`utxo/in_memory.rs:360`) sorts explicitly; the RocksDB backend relies on RocksDB's sorted key order (outpoint bytes) to match — this key-order equivalence is the bit-identity guarantee between backends.

### 1d — ACTUAL call-site map (whole-workspace `git grep`)
Non-test invocations of a root function = **6** (the "~15" is a stale/aspirational count from the
`hardfork.rs:203` and `snapshot.rs:81` Phase-2 wiring comments, not the current code).

| file:line | fn | context |
|-----------|-----|---------|
| `bins/node/src/node/apply_block/state_update.rs:139` | `compute_state_root` | **PRODUCTION** — once per applied block, result cached (`cached_state_root`) |
| `bins/node/src/node/event_loop.rs:523` | `compute_state_root` | RPC `GetStateRoot` fallback (cache-miss only, pre-first-block) |
| `bins/node/src/node/validation_checks.rs:1113` | `compute_state_root` | RPC `GetStateRoot` fallback (second handler impl), cache-first |
| `bins/node/src/node/fork_recovery.rs:281` | `compute_state_root_from_bytes` | **snap-sync INSTALL** — verify downloaded snapshot root |
| `bins/node/src/node/fork_recovery.rs:341` | `compute_state_root` | **snap-sync INSTALL** — recompute + cache after state replace |
| `bins/cli/src/cmd_snap.rs:226` | `compute_state_root_from_bytes` | CLI snapshot verify tool |

Re-export only (not a call): `crates/storage/src/lib.rs:116`. Comment references (not calls):
`network/src/protocols/status.rs:31`, `sync/manager/cleanup.rs:189`, `sync/manager/snap_sync.rs:73,133`,
`hardfork.rs:198,203`. Tests: `inc_i_118_snap_utxo_backend.rs`, `utxo/tests_oracle_snapsync.rs`,
`disk_guardian_failsafe_test.rs`.

**Snap-sync BUILD** side (serving a snapshot) computes the root inside `StateSnapshot::create`
(`snapshot.rs:215`) — reachable from the `GetStateSnapshot` handlers (`event_loop.rs:534`,
`validation_checks.rs:1124`).

---

## 2. Architecture Comprehension

### End-to-end lifecycle of the root
- **Produce (apply)**: after every block, `update_chain_state_for_block` computes the root under read
  locks and **caches** it into `cached_state_root` (`state_update.rs:110-141`). One computation per block.
- **Validation expected-root check**: **NONE.** `BlockHeader` (`crates/core/src/block.rs:19`) has **no
  `state_root` field**, and a whole-workspace search for `state_root ==` / `!=` / `expected_root` /
  `declared_root` returns **zero** consensus comparisons. The `state_root` in
  `crates/core/src/transaction/data.rs:581` is an unrelated **L2-rollup** payload field. The 3-state root
  is therefore **observational (logging `[STATE_ROOT]` `snapshot.rs:43`) + snap-sync integrity anchor +
  `GetStateRoot` RPC diagnostic — it is NOT part of the block header, block hash, or block-acceptance rules.**
- **Snap-sync build**: `StateSnapshot::create` serializes all 3 states + computes root (`snapshot.rs:200-235`).
- **Snap-sync install**: downloader verifies `compute_state_root_from_bytes == snapshot.state_root`
  (`fork_recovery.rs:281-303`) against a **quorum-voted root** (`sync/manager/snap_sync.rs:73,133`),
  then replaces state and re-caches (`fork_recovery.rs:341`).
- **Rollback/reorg**: no direct root recomputation in `rollback.rs`; the next applied block re-caches via
  the normal apply path.
- **RPC**: `GetStateRoot` returns the cache, recomputes only on cache-miss (`event_loop.rs:507-531`,
  `validation_checks.rs:1097-1122`).

### `[STATE_ROOT]` frequency
**One full computation per applied block** (the apply path `state_update.rs:139`), then cached; all other
readers hit the cache. So on a healthy node it is exactly 1 O(state) computation per block; extra
computations occur only during snap-sync install and the rare cache-miss RPC fallback.

### Dependency direction
`bins/node` and `bins/cli` depend on `crates/storage::snapshot`. `crates/network` has **no storage dep**
(comment `fork_recovery.rs:280`) — root verification is deliberately node-side. `snapshot.rs` depends only
on `chain_state`, `utxo`, `producer`, `crypto`.

### Invariants
- **INV-SYNC-007** (`sync/recovery`): *every node — including freshly snap-synced or reorged — must converge
  to the canonical chain's bit-identical 3-state (height → stateRoot, csHash, psHash, utxoHash) and never
  remain in a permanent post-snap fork-recovery deadlock.* This is the load-bearing constraint for any redesign.
- **Determinism**: BLAKE3 everywhere (`crates/crypto/src/hash.rs:1`), INC-I-012 F9; canonical ordering via
  explicit sort (in-memory) / RocksDB key order (disk). x86/ARM bit-identity depends on this ordering + LE encoding.
- **INV-EPOCH-003** (adjacent): rebuild-from-blocks must be bit-identical to canonical epoch_state — relevant
  because the EPOCH_SNAPSHOT_HF path folds `H(EpochSnapshot)` into the root.

---

## 3. Capability Inventory (verified baseline — before any "system lacks X")

DOLI **already has**:
- **Merkle tree primitives**: `crates/crypto/src/merkle.rs:49 merkle_root`, `:74 merkle_root_from_hashes`
  (bottom-up BLAKE3 tree with domain-separated leaves). Used for tx trees (`block.rs:255 compute_merkle_root`),
  producer snapshots (`discovery/snapshot.rs:49`), data blobs (`data_root` header field).
- **Canonical serializers** for all 3 states (`chain_state.rs:143`, `producer/set_persistence.rs:78`,
  `utxo/set.rs:421` + `state_db/queries.rs:473` + `utxo/types.rs:57`).
- **HF-gated root-formula switch scaffolding**: `compute_state_root_with_epoch_state` (`snapshot.rs:86`,
  `None`≡legacy, `Some(h)` folds a 4th component) + `EPOCH_SNAPSHOT_HF` schedule (`hardfork.rs:180`).
- **Snapshot/checkpoint machinery**: `StateSnapshot` (`snapshot.rs:184`), guardian checkpoints, archiver.

DOLI **does NOT have** (verified absent):
- No Merkle-Patricia trie, no persistent authenticated state tree, no incremental/running-hash state
  accumulator. The state root is computed by full re-serialization every time — there is no per-entry
  authenticated structure to update incrementally.

---

## 4. Measure Actual Cost — is this a present-day problem?

- **Frequency**: 1 computation / block. Slot time is 10 s.
- **Dominant cost is NOT hashing.** BLAKE3 of ~782 KB at ~1–3 GB/s ≈ **0.3–0.8 ms**. Even at the 16 MB
  threshold, BLAKE3 ≈ **5–16 ms**. The real per-block cost is the **RocksDB full-CF scan + N bincode
  deserializations + full canonical re-serialization + a fresh multi-MB `Vec` allocation** in
  `serialize_canonical_utxo` (`state_db/queries.rs:473`) — I/O read-amplification and allocation churn, not CPU.
- **Present-day headroom**: current size ~782 KB (some networks ~40 KB, `inc-i-118-diagnosis.md:29`) vs
  12 MB warn / 16 MB threshold ⇒ **~15–21× headroom**. Combined per-block cost today is order **single-digit
  ms out of a 10 000 ms slot (≈0.01–0.05%)**. Even at threshold, order tens of ms (<1% of slot).
- **Growth drivers**: `utxo_bytes` grows with live UTXO count (coins + bonds + rewards + pools + NFTs +
  OraclePrice); `ps_bytes` grows with producer/delegation count (`set_persistence.rs:78`).
- **Verdict**: **This is a projected-cost hypothesis, not a measured present-day problem.** The strongest
  real argument for change is RocksDB read-amplification + allocation churn (an INC-I-111-class concern),
  NOT hash CPU, and even that is far from binding at current scale.

---

## 5. Redesign Acceptance Criteria (MoSCoW)

| ID | Requirement | Priority | Acceptance Criteria |
|----|-------------|----------|---------------------|
| REQ-SROOT-001 | Identical root VALUE per height pre-activation | **Must** | - [ ] For every height < activation, new code emits byte-identical root to legacy `compute_state_root` for all reachable inputs (golden-vector test vs 3 consecutive live epochs) |
| REQ-SROOT-002 | Bit-identical cross-node convergence (INV-SYNC-007) | **Must** | - [ ] Two nodes at the same height/state produce the same root; snap-sync install verification (`fork_recovery.rs:281`) still passes across the fleet |
| REQ-SROOT-003 | x86/ARM determinism | **Must** | - [ ] Same root on both arches (BLAKE3 + LE + fixed ordering); cross-arch CI vector |
| REQ-SROOT-004 | Snap-sync build+install intact | **Must** | - [ ] `StateSnapshot::create` and `compute_state_root_from_bytes` still agree; quorum-root voting unaffected |
| REQ-SROOT-005 | Forward-only activation height in `NetworkParams`; no genesis reset; no `CURRENT_PROTOCOL_VERSION` bump unless `EpochState` format changes (INC-I-054) | **Must** | - [ ] Any formula change gated by a new AH; devnet=0, testnet/mainnet future height; no retroactive root change |
| REQ-SROOT-006 | Post-activation initial root derived from on-disk 3-state, never local-block rebuild (safe for snap-synced nodes) | **Must** | - [ ] A snap-synced node with empty block store computes the same activation-height root as a full-history node |
| REQ-SROOT-007 | Remove per-block O(state) full re-serialization; update ∝ CHANGED entries | **Should** | - [ ] Per-block work scales with entries mutated by the block, not total live-set size; no full `CF_UTXO` scan per block |
| REQ-SROOT-008 | **NON-FORECLOSURE** — the chosen simpler structure must not re-create the escaped dead-end | **Should** | - [ ] Document whether an SSF incremental running-hash **forecloses** a later move to proof-based (per-key-provable) snap-sync; record the known evolution path (running-hash → merkelized-per-component → full trie) as an explicit future option **without building it now** |
| REQ-SROOT-009 | Proof-based (Ethereum-style) snap-sync; per-component roots | **Could** | - [ ] Only if §4 cost ever becomes binding; a per-key inclusion proof would let snap-sync verify partial state |
| REQ-SROOT-010 | Genesis reset / changing historical roots / all-nodes-simultaneous stop | **Won't** | N/A — ~30 external producers, no synchronized stop possible; forward-only only |

**Non-foreclosure note (REQ-SROOT-008):** an incremental **running-hash over the 3 component digests**
(the SSF) preserves the *value* of the root but is a single opaque scalar — it does **not** provide
per-key inclusion proofs, so it does not by itself enable Ethereum-style proof-based snap-sync. It does
**not foreclose** that move: proof-based sync would require replacing the per-component digest with a
merkelized-per-component tree, which is an additive, separately-gated step. Record this so a later scale
decision is not blocked by the SSF, and do not build the trie now.

---

## 6. Clarifying Questions (genuine ambiguities only)

1. **Is the goal to cut CPU, or to cut RocksDB read-amplification/alloc churn?** The measured dominant cost
   is the full `CF_UTXO` scan + per-entry deserialize (`state_db/queries.rs:473`), not BLAKE3. An SSF that
   only avoids re-hashing but still scans RocksDB every block would miss the real cost. Which cost are we optimizing?
2. **Given the 3-state root is snap-sync/diagnostic-only (not block-header-validated — §2), is an activation
   height still required, or can a version-negotiated snap-sync formula suffice?** A mixed fleet would not
   fork block production; it would only fail cross-version snap-sync root verification (→ fallback to normal
   sync). This materially relaxes the migration model vs. a true consensus field — confirm the intended risk posture.

---

## Architecture Context (for the architect)

### Module boundaries
- `crates/storage/src/snapshot.rs` — root fns; depends on `chain_state`, `utxo`, `producer`, `crypto`.
- `crates/storage/src/state_db/queries.rs:473` — RocksDB canonical UTXO serializer (the hot path).
- `bins/node/.../apply_block/state_update.rs` — sole production producer of the cached root.
- `bins/node/.../fork_recovery.rs` — snap-sync install consumer/verifier.
- `crates/network` — **no storage dep**; carries the quorum-root vote only.

### Blast radius
- **Direct**: the 6 non-test call-sites (§1d) + `StateSnapshot::create`.
- **Indirect**: snap-sync quorum voting (`sync/manager/snap_sync.rs`, `cleanup.rs`), `GetStateRoot`/
  `GetStateSnapshot` RPC, guardian checkpoints, CLI `cmd_snap`, `EPOCH_SNAPSHOT_HF` scaffolding.
- **NOT in blast radius (verified)**: block validation / acceptance — no header field, no comparison.

### Constraints & invariants
- INV-SYNC-007 (bit-identical 3-state convergence) — load-bearing.
- No `CURRENT_PROTOCOL_VERSION` bump unless `EpochState` format changes (INC-I-054).
- Forward-only AH in `NetworkParams`; initial post-activation root from on-disk 3-state (snap-synced nodes
  have no block history).
- INC-I-111 write-amplification: an incremental design that writes a trie node per mutation must be
  cost-modeled — per-mutation RocksDB writes can silently multiply I/O.

## Assumptions
| # | Technical | Plain language | Confirmed |
|---|-----------|----------------|-----------|
| 1 | utxo/ps sizes are env-dependent; ~782 KB is a mainnet-scale figure, ~40 KB seen on smaller nets | "783 KB" is not universal — it depends on the network's live set | No — user to confirm target network |
| 2 | The dominant per-block cost is RocksDB scan+deserialize, not BLAKE3 | The slow part is reading/rebuilding the UTXO set, not the hashing | No — recommend a flamegraph before committing to a redesign |

## Identified Risks
- **Anchoring risk**: the original ask named an Ethereum MPT. Verified cost (§4) does not justify a trie now;
  MPT is a `Could`, not the mandate.
- **Mixed-fleet snap-sync degradation** (not a fork): cross-version root mismatch → snap-sync rejection →
  normal-sync fallback. Manageable with an AH but must be modeled.
- **Bit-identity fragility**: the RocksDB-vs-in-memory backend equality depends on RocksDB key order matching
  the in-memory sort — any incremental design must preserve this exactly.
