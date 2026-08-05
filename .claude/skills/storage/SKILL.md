# storage — DOLI Persistence Layer
<!-- @INDEX
ENTRY-POINTS: lines 21-47
OPERATIONS: lines 48-62
DATA-FLOWS: lines 63-126
STRUCTS: lines 127-279
COLUMN-FAMILIES: lines 280-342
FUNCTIONS-BLOCKSTORE: lines 343-401
FUNCTIONS-STATEDB: lines 402-488
FUNCTIONS-UTXO: lines 489-521
FUNCTIONS-PRODUCERSET: lines 522-543
FUNCTIONS-SNAPSHOT: lines 544-557
FUNCTIONS-ARCHIVER: lines 558-571
FUNCTIONS-MISC: lines 572-581
DEPENDENCIES: lines 582-600
CONSTRAINTS: lines 601-650
PATTERNS: lines 651-732
SERIALIZATION: lines 733-774
-->

## ENTRY-POINTS

Primary structs and their initialization paths:

| Struct | Open/New | File |
|--------|----------|------|
| `BlockStore` | `BlockStore::open(path)` | `block_store/open.rs:59` |
| `StateDb` | `StateDb::open(path)` | `state_db/open.rs:60` |
| `UtxoSet` | `UtxoSet::new()` (in-memory) / `UtxoSet::from_state_db(Arc<StateDb>)` (production, Phase 4) | `utxo/set.rs:41,46` |
| `ProducerSet` | `ProducerSet::new()` / `ProducerSet::load(path)` / `ProducerSet::from_parts(...)` | `producer/set_core.rs:13` / `producer/set_persistence.rs:18` |
| `ChainState` | `ChainState::new(genesis_hash)` / `ChainState::load(path)` | `chain_state.rs:80,98` |
| `StateSnapshot` | `StateSnapshot::create(cs, utxo, ps)` | `snapshot.rs:202` |
| `MaintainerState` | `MaintainerState::load(data_dir)` | `maintainer.rs:40` |
| `BlockArchiver` | `BlockArchiver::new(rx, dir)` + `archiver.run().await` | `archiver.rs:24,29` |
| `ContentStore` | `ContentStore::open(path)` | `content_store.rs:26` |
| `UpdateState` | `UpdateState::load(data_dir)` | `update.rs:62` |
| `UtxoSizeMonitor` | `UtxoSizeMonitor::new(Arc<StateDb>)` | `utxo_size_monitor.rs:37` |
| `CompactMmr` / `IncrementalStateRoot` | `CompactMmr::new()` / `IncrementalStateRoot::new()` (NOT wired into apply_block yet — primitive only) | `mmr.rs:27,122` |

**Startup load order** (`Node::new()` in `bins/node/src/node/init.rs`):
1. `BlockStore::open(blocks_dir)`
2. `StateDb::open(state_dir)` — loads UTXO count from RocksDB, opens 7 CFs (incl. `cf_unique_id`)
3. `UtxoSet::from_state_db(Arc::new(state_db))` — RocksDb-backed variant (Phase 4: state_db is the sole UTXO store)
4. `StateDb::load_producer_set()` — rebuilds `ProducerSet` from `cf_producers` + `cf_exit_history` + `META_PENDING_UPDATES`
5. `StateDb::get_chain_state()` — loads `ChainState` from `cf_meta`
6. `StateDb::get_epoch_state()` — loads persisted `EpochState` bytes (if exists)

## OPERATIONS

| Task | Steps | Commands/Functions | Inputs | Success |
|------|-------|--------------------|--------|---------|
| Open all stores on startup | 1. open block store 2. open state_db (creates 7 CFs incl. cf_unique_id) 3. wrap in `UtxoSet::from_state_db` 4. rebuild ProducerSet 5. load ChainState | `BlockStore::open()`, `StateDb::open()`, `UtxoSet::from_state_db()`, `StateDb::load_producer_set()`, `StateDb::get_chain_state()` | data_dir path | all 3 states loaded, `get_last_applied()` matches `chain_state.best_hash` |
| Write a block batch atomically | 1. `begin_batch()` 2. spend inputs 3. add outputs (stamps Bond/Pool) 4. put producer/chain_state/undo/epoch_state 5. `commit()` | `state_db.begin_batch()`, `batch.spend_transaction_utxos()`, `batch.add_transaction_utxos()`, `batch.put_chain_state()`, `batch.put_undo()`, `batch.commit()` | validated Block | single atomic `WriteBatch.write()`; utxo_count updated |
| Compute the state root | 1. serialize_canonical() each of cs/utxo/ps 2. hash each 3. combine | `compute_state_root(cs, utxo, ps)` | ChainState, UtxoSet, ProducerSet | deterministic `Hash`; logged as `[STATE_ROOT] cs=.. utxo=.. ps=..` |
| Take a snapshot for snap sync | 1. serialize cs (bincode) 2. serialize utxo (canonical) 3. serialize ps (bincode) 4. compute_state_root | `StateSnapshot::create(cs, utxo, ps)` | current in-memory/state_db state | `StateSnapshot{..., state_root}` ready for wire transfer |
| Restore from a snapshot | 1. verify state_root via `compute_state_root_from_bytes` 2. deserialize each component 3. `atomic_replace` 4. `seed_canonical_index` 5. `mark_snap_synced` | `compute_state_root_from_bytes()`, `StateDb::atomic_replace()`, `BlockStore::seed_canonical_index()`, `ChainState::mark_snap_synced()` | received snapshot bytes | state root matches; block store anchored at snap height |
| Roll back N blocks | 1. `get_undo(height)` 2. restore spent_utxos 3. delete created_utxos 4. restore producer_snapshot/epoch_state_snapshot 5. `put_chain_state(prev_cs)` | `StateDb::get_undo()`, `StateDb::insert_utxo()`, `StateDb::remove_utxo()`, `StateDb::put_chain_state()` | target height, undo data present | chain_state rewound; `has_undo_data(from,to)` true before attempting |
| Archive blocks to filesystem | 1. spawn `BlockArchiver::run()` 2. push `ArchiveBlock{height,hash,data}` via mpsc 3. on gap, `catch_up`/`backfill_from_archive` | `BlockArchiver::new()`, `archiver.run()`, `BlockArchiver::catch_up()`, `backfill_from_archive()` | block store + archive dir | `{height:010}.block` + `.blake3` sidecar + updated `manifest.json` |
| Prune old block/undo history | 1. `prune_blocks_below(cutoff, tip)` (block_store) 2. `prune_undo_before/prune_undo_below` (state_db) | `BlockStore::prune_blocks_below()`, `StateDb::prune_undo_before()`, `StateDb::prune_undo_below()` | keep_above_height, chain_tip | old SSTs compacted; MIN_RETENTION=2000 blocks from tip enforced |
| Export RocksDB metrics for Prometheus | 1. call `.metrics()` on BlockStore/StateDb/UtxoSet | `BlockStore::metrics()`, `StateDb::metrics()`, `UtxoSet::metrics()` | none | `RocksDbMetrics` snapshot (memtable, block cache, SST, compaction state) |
| Deduplicate NFT/asset blob content | 1. `ContentStore::open(path)` 2. `put(data)` (ref-counts) 3. `get(hash)` 4. `release(hash)` (deletes at ref=0) | `ContentStore::put/get/release` | raw bytes | same content stored once; ref-counted lifecycle |

## DATA-FLOWS

### Block application (happy path):
```
apply_block()
  → state_db.begin_batch()               # creates BlockBatch
  → batch.spend_transaction_utxos(tx)   # deletes inputs from cf_utxo + cf_utxo_by_pubkey
  → batch.add_transaction_utxos(tx)     # adds outputs to cf_utxo + cf_utxo_by_pubkey; stamps Bond/Pool
  → batch.add/remove_unique_id_for_entry # NFT/Pool/FungibleAsset -> cf_unique_id (Phase 1/3)
  → batch.put_producer()/remove_producer()  # updates cf_producers
  → batch.put_chain_state(cs)           # updates cf_meta["chain_state"]
  → batch.set_last_applied(h, hash, slot) # writes consistency canary
  → batch.put_undo(height, UndoData)    # saves rollback data to cf_undo
  → batch.put_epoch_state(bytes)        # saves EpochState snapshot
  → batch.commit()                      # single atomic RocksDB WriteBatch.write()
  → block_store.put_block(block, height) # headers, bodies, slot/tx/addr indexes
  → block_store.set_canonical_chain(tip_hash, tip_height)  # height_index, hash_to_height
```

### Block rollback (undo path):
```
rollback_one_block()
  → state_db.get_undo(height)           # load UndoData from cf_undo
  → restore spent_utxos to cf_utxo     # undo.spent_utxos → insert back
  → delete created_utxos from cf_utxo  # undo.created_utxos → delete
  → restore producer_snapshot           # undo.producer_snapshot → full replace
  → restore epoch_state_snapshot        # undo.epoch_state_snapshot → optional
  → state_db.put_chain_state(prev_cs)  # rewind chain_state
```

### Snap sync (state transfer):
```
StateSnapshot::create(cs, utxo, ps)
  → chain_state_bytes = bincode::serialize(cs)   # wire format
  → utxo_set_bytes = utxo.serialize_canonical()  # canonical format (sorted)
  → producer_set_bytes = bincode::serialize(ps)  # wire format (bincode)
  → state_root = compute_state_root(cs, utxo, ps) # H(H(cs) || H(utxo) || H(ps))

Apply received snapshot:
  → state_db.atomic_replace(cs, ps, utxo_iter)   # delete all + write all, one batch (CF_META spared)
  → block_store.seed_canonical_index(hash, height) # writes snap_horizon to cf_meta
  → chain_state.mark_snap_synced(height)
```

### State root computation:
```
compute_state_root(cs, utxo, ps)
  → cs_hash = H(cs.serialize_canonical())     # 140 bytes, fixed-field
  → utxo_hash = H(utxo.serialize_canonical()) # sorted by outpoint key (RocksDB iterates lexicographic)
  → ps_hash = H(ps.serialize_canonical())     # sorted by pubkey hash
  → H(cs_hash_bytes || utxo_hash_bytes || ps_hash_bytes)
```

### UTXO read path (Phase 4 — state_db is sole UTXO store):
```
UtxoSet::RocksDb(Arc<StateDb>) reads route directly to StateDb:
  get() -> sdb.get_utxo()
  get_by_pubkey_hash() -> sdb.get_utxos_by_pubkey()  # prefix scan cf_utxo_by_pubkey
  get_bonded_balance/count_bonds/get_bond_entries/get_all_pools/find_nft_by_token_id
    -> mirrored 1:1 on StateDb (queries.rs "Phase 1: Mirrored Query Methods")
  During apply_block: BlockBatch overlay methods (get_utxo/contains_utxo/get_utxos_by_pubkey)
    check pending_utxos + spent_in_batch first, then fall through to committed state_db.
```

## STRUCTS

### ChainState (`chain_state.rs:14`)
```rust
pub struct ChainState {
    pub best_hash: Hash,                           // current chain tip
    pub best_height: u64,
    pub best_slot: u32,
    pub total_work: u64,                           // always = best_height (not accumulated)
    pub genesis_hash: Hash,
    pub genesis_timestamp: u64,                    // 0 until first block
    pub last_registration_hash: Hash,              // anti-Sybil registration chain
    pub registration_sequence: u64,
    pub total_minted: Amount,                      // supply cap enforcement
    pub snap_sync_height: Option<u64>,             // NOT in serialize_canonical()
    pub active_protocol_version: u32,              // NOT in serialize_canonical()
    pub pending_protocol_activation: Option<(u32, u64)>,  // NOT in serialize_canonical()
}
```
- `serialize_canonical()` → fixed 140-byte encoding: `[32B best_hash][8B height][4B slot][8B work][32B genesis_hash][8B genesis_timestamp][32B last_reg_hash][8B reg_seq][8B total_minted]`
- Saved to disk via `bincode` (atomic: write `.bin.tmp` then `rename`)
- Persisted in `StateDb` via `cf_meta["chain_state"]` with `0x01` version prefix

### UtxoEntry (`utxo/types.rs:24`)
```rust
pub struct UtxoEntry {
    pub output: Output,        // output_type, amount, pubkey_hash, lock_until, extra_data
    pub height: BlockHeight,   // block height when created
    pub is_coinbase: bool,
    pub is_epoch_reward: bool, // #[serde(default)] for backward compat
}
```
- `serialize_canonical_bytes()`: `[1B output_type][8B amount][32B pubkey_hash][8B lock_until][8B height][1B is_coinbase][1B is_epoch_reward][2B extra_len (u16 LE)][N extra_data]` — base 61 bytes
- For `extra_data >= 65535`: `0xFFFF` marker + `u32 LE` length (large NFTs)
- Stored in RocksDB via `bincode::serialize()` (not canonical format — canonical only for state root)

### Outpoint (`utxo/types.rs:173`)
```rust
pub struct Outpoint { pub tx_hash: Hash, pub index: u32 }
// Key: 36 bytes = 32 (tx_hash) + 4 (index, LE)
```

### UndoData (`state_db/types.rs:16`)
```rust
pub struct UndoData {
    pub spent_utxos: Vec<(Outpoint, UtxoEntry)>,   // restore on rollback
    pub created_utxos: Vec<Outpoint>,               // delete on rollback
    pub producer_snapshot: Vec<u8>,                 // full ProducerSet bincode snapshot
    pub epoch_state_snapshot: Option<Vec<u8>>,      // EpochState snapshot (added later)
    pub chain_commitment: Option<[u8; 32]>,          // legacy, kept for compat
}
```

### BlockBatch (`state_db/types.rs:101`)
```rust
pub struct BlockBatch<'a> {
    db: &'a StateDb,
    batch: rocksdb::WriteBatch,
    utxo_delta: i64,                              // tracks net UTXO change
    pending_utxos: HashMap<Outpoint, UtxoEntry>, // same-block-spend support
    spent_in_batch: Vec<Outpoint>,
    pending_unique_ids: HashSet<(u8, [u8; 32])>, // Phase 1: same-block uniqueness (NFT/Pool/Asset)
    removed_unique_ids: HashSet<(u8, [u8; 32])>, // Phase 3: spent-in-block IDs no longer visible
}
```
- `commit()` writes the entire batch atomically, then updates `utxo_count` AtomicU64
- Implements `doli_core::validation::UtxoProvider` — lets `validate_transaction_with_utxos` see uncommitted batch state during `apply_block`

### LastApplied (`state_db/types.rs:126`)
```rust
pub struct LastApplied { pub height: u64, pub hash: Hash, pub slot: u32 }
// Serialized as 44 bytes: 8 (height LE) + 32 (hash) + 4 (slot LE)
```
- Consistency canary: written in the same WriteBatch as all state changes
- If `last_applied.hash == chain_state.best_hash`, the DB is consistent

### ProducerInfo (`producer/types.rs`)
```rust
pub struct ProducerInfo {
    pub public_key: PublicKey,
    pub registered_at: u64,
    pub bond_amount: u64,              // total across all bonds
    pub bond_outpoint: (Hash, u32),    // primary bond
    pub status: ProducerStatus,        // Active | Unbonding{started_at} | Exited | Slashed{at}
    pub slots_missed: u64,
    pub registration_era: u32,
    pub has_prior_exit: bool,
    pub last_activity: u64,
    pub bond_count: u32,               // 1-100 bonds (MAX_BONDS_PER_PRODUCER)
    pub additional_bonds: Vec<(Hash, u32)>,
    pub delegated_to: Option<PublicKey>,
    pub delegated_bonds: u32,
    pub received_delegations: Vec<(Hash, u32)>,
    pub bond_entries: Vec<StoredBondEntry>,        // FIFO withdrawal tracking
    pub withdrawal_pending_count: u32,
    pub bls_pubkey: Vec<u8>,           // 48 bytes for BLS12-381
    // DEPRECATED fields (kept for bincode compat): blocks_produced, pending_rewards, activity_gaps
}
```

### ProducerSet (`producer/types.rs`)
```rust
pub struct ProducerSet {
    producers: HashMap<Hash, ProducerInfo>,             // keyed by pubkey_hash
    exit_history: HashMap<Hash, u64>,                   // pubkey_hash → exit_height
    #[serde(skip)] active_cache: Option<(u64, Vec<Hash>)>,     // invalidated on mutation
    #[serde(skip)] unbonding_index: BTreeMap<u64, Vec<Hash>>,  // rebuilt on load
    pending_updates: Vec<PendingProducerUpdate>,        // applied at epoch boundary
}
```
- Persisted to `StateDb` (split: `cf_producers` + `cf_exit_history` + `META_PENDING_UPDATES`)
- Legacy file persistence: JSON first, bincode fallback (`producer/set_persistence.rs:18`)

### StateSnapshot (`snapshot.rs:185`)
```rust
pub struct StateSnapshot {
    pub block_hash: Hash,
    pub block_height: u64,
    pub chain_state_bytes: Vec<u8>,    // bincode
    pub utxo_set_bytes: Vec<u8>,       // canonical format
    pub producer_set_bytes: Vec<u8>,   // bincode
    pub state_root: Hash,
}
```

### MaintainerState (`maintainer.rs:22`) / UpdateState (`update.rs:51`)
```rust
pub struct MaintainerState { pub set: MaintainerSet, pub last_derived_height: u64 }
// Persisted as: {data_dir}/maintainer_state.bin (bincode)

pub struct UpdateState {
    pub pending_releases: HashMap<String, PersistedRelease>,
    pub votes: HashMap<String, Vec<PersistedVote>>,
    pub history: Vec<UpdateHistoryEntry>,
}
// Persisted as: {data_dir}/update_state.bin (bincode)
```

### RocksDbMetrics (`metrics.rs:39`)
```rust
pub struct RocksDbMetrics {
    pub instance: &'static str,  // "block_store" | "state_db"
    // Memory: memtable_bytes, memtable_max_bytes, memtable_cap_bytes,
    //         block_cache_bytes, block_cache_pinned_bytes, block_cache_capacity, table_readers_bytes
    // Data shape: estimate_keys, live_data_bytes, sst_total_bytes, sst_live_bytes
    // Flush/compaction: running_flushes, running_compactions, compaction_pending,
    //                   mem_table_flush_pending, num_immutable_memtable
    // Write health: actual_delayed_write_rate, is_write_stopped, background_errors
    // LSM shape: files_per_level: BTreeMap<u8, u64>  // L0..=L6
}
```
- Built by `collect_db_metrics()` (`metrics.rs:112`) — CF-scoped properties summed across named CFs; block-cache queried directly from `rocksdb::Cache` handle (NOT per-CF property, INC-I-106 root-cause fix)

## COLUMN-FAMILIES

### BlockStore (`block_store/types.rs:89-98`) — 9 CFs, per-CF tuned (`block_store/open.rs`)

| CF Name | Key | Value | Notes |
|---------|-----|-------|-------|
| `headers` | Hash (32B) | bincode BlockHeader | ALL blocks incl. forks. bloom=yes, 8MB wbuf |
| `bodies` | Hash (32B) | bincode BlockBody | txs + BLS sig + attestation_bitfield. bloom=yes, 8MB wbuf |
| `height_index` | height u64 LE (8B) | Hash (32B) | CANONICAL chain only |
| `slot_index` | slot u32 LE (4B) | Hash (32B) | last block per slot |
| `presence` | (deprecated) | (deprecated) | cleaned up on open (`cleanup_presence_cf`), kept in descriptor list |
| `hash_to_height` | Hash (32B) | height u64 LE (8B) | reverse lookup, O(1), bloom=yes |
| `tx_index` | tx_hash (32B) | height u64 LE (8B) | tx → block height, bloom=yes |
| `addr_tx_index` | pubkey_hash(32B) ++ height(8B BE) | empty | address history, sorted by height desc. NO bloom (prefix scan) |
| `meta` | `b"snap_horizon"` | height u64 LE | snap sync anchor |

Shared 32MB LRU block cache across all 9 CFs (`block_cache` field on struct); `DB_WRITE_BUFFER_SIZE_BYTES = 48MB` memtable cap.

**CRITICAL**: `height_index` and `hash_to_height` are written ONLY by `set_canonical_chain()`. `put_block()` does NOT write them — that prevents fork blocks from polluting the canonical index.

### StateDb (`state_db/types.rs:42-51`) — 7 CFs, per-CF tuned + BlobDB on cf_utxo (`state_db/open.rs`)

| CF Name | Key | Value | Notes |
|---------|-----|-------|-------|
| `cf_utxo` | Outpoint (36B) = txhash(32) + index(4 LE) | bincode UtxoEntry | primary UTXO index. BlobDB enabled (Phase 5): values ≥4KB → separate `.blob` files, Zstd, GC at 25% age cutoff. Bloom, 16MB wbuf |
| `cf_utxo_by_pubkey` | pubkey_hash(32B) ++ outpoint(36B) = 68B | `0x00` | secondary index for balance lookups. NO bloom (prefix scan), 8MB wbuf |
| `cf_producers` | pubkey_hash (32B) | bincode ProducerInfo | bloom, 2MB wbuf |
| `cf_exit_history` | pubkey_hash (32B) | height u64 LE (8B) | anti-Sybil, bloom, 1MB wbuf |
| `cf_meta` | string key | varies | chain_state, pending_updates, epoch state, oracle sunset. 4MB wbuf |
| `cf_undo` | height u64 LE (8B) | bincode UndoData | rollback data, Zstd, `periodic_compaction_seconds=3600` (INC-I-108, replaces synchronous compact) |
| `cf_unique_id` | prefix(1B) + id(32B) = 33B | `0x00` | **Phase 1 UTXO consolidation**: NFT/Pool/Asset uniqueness index, mirrors the (now-eliminated) utxo_rocks CF. Bloom, 2MB wbuf |

Shared 48MB LRU block cache across all 7 CFs; `DB_WRITE_BUFFER_SIZE_BYTES = 64MB` memtable cap (must be ≥32MB for snap-sync `atomic_replace` WriteBatch).

**Meta keys** (`state_db/types.rs:54-76`):

| Key | Value | Notes |
|-----|-------|-------|
| `chain_state` | `0x01` prefix + bincode ChainState | versioned format |
| `pending_updates` | bincode Vec<PendingProducerUpdate> | |
| `last_applied` | 44B (height + hash + slot) | consistency canary |
| `epoch_producer_list` | concatenated 32B public keys | frozen at epoch start |
| `active_production_list` | concatenated 32B public keys | round-robin subset |
| `epoch_attested_set` | bincode [HashSet<PublicKey>; 3] | |
| `epoch_attestation_accum` | bincode [HashMap<PublicKey, HashSet<u32>>; 3] | |
| `epoch_blocks_produced` | bincode HashMap<PublicKey, u32> | |
| `epoch_bond_snapshot` | bincode (HashMap<Hash, u64>, epoch) | |
| `epoch_state` | raw bytes (EpochState format) | full scheduler state |
| `epoch_state_version` | u32 LE (4B) | version when epoch_state was written |
| `chain_commitment` | Hash (32B) | incremental chain commitment |
| `chain_commitment_tip` | u64 LE (8B) | height commitment was computed at |
| `oracle_sunset_state` | bincode `OracleSunsetState` | D.3 gradient (warning/halt epoch tracking). NOT in state root |
| `oracle_last_update_height` | u64 LE (8B) | AUDIT-P2-001 cache for `getOracleStatus`. NOT in state root |

### ContentStore (`content_store.rs:16-17`) — separate RocksDB, 2 CFs

| CF Name | Key | Value |
|---------|-----|-------|
| `content` | content_hash (32B, BLAKE3) | raw blob bytes |
| `refcount` | content_hash (32B) | u64 LE ref count |

**ARCHITECTURE NOTE**: `utxo_rocks.rs` / `RocksDbUtxoStore` (the former standalone UTXO RocksDB instance, documented in older skill versions) was **eliminated in Phase 4**. `UtxoSet::RocksDb` now wraps `Arc<StateDb>` directly — `cf_utxo`/`cf_utxo_by_pubkey`/`cf_unique_id` above ARE the production UTXO store. See `specs/utxo-storage-architecture.md`.

## FUNCTIONS-BLOCKSTORE

**`block_store/queries.rs`** — read operations:

| Function | Signature | Notes |
|----------|-----------|-------|
| `get_block` | `(&Hash) → Result<Option<Block>>` | fetches header + body |
| `get_header` | `(&Hash) → Result<Option<BlockHeader>>` | header only |
| `get_height_by_hash` | `(&Hash) → Result<Option<u64>>` | O(1) reverse lookup |
| `get_hash_by_height` | `(u64) → Result<Option<Hash>>` | canonical chain only |
| `get_block_by_height` | `(u64) → Result<Option<Block>>` | via height_index |
| `get_block_by_slot` | `(u32) → Result<Option<Block>>` | via slot_index |
| `get_hash_by_slot` | `(u32) → Result<Option<Hash>>` | |
| `has_block_for_slot` | `(u64) → bool` | fast slot check for producers |
| `get_blocks_in_slot_range` | `(u32, u32) → Result<Vec<Block>>` | [start, end) |
| `has_any_block_in_slot_range` | `(u32, u32) → Result<bool>` | early-exit scan |
| `get_address_heights` | `(&Hash, Option<u64>, usize) → Result<Vec<u64>>` | descending |
| `get_tx_block_height` | `(&Hash) → Result<Option<u64>>` | tx_index lookup |
| `ensure_blocks_present` | `(u64, u64) → Result<()>` | FORK_GUARD backfill invariant |
| `get_snap_horizon` | `() → Result<Option<u64>>` | snap sync anchor height |
| `get_last_rewarded_epoch` | `() → Result<u64>` | scans backwards from tip |
| `has_block` | `(&Hash) → Result<bool>` | |
| `create_checkpoint` | `(&Path) → Result<()>` | RocksDB hard-link checkpoint |

**`block_store/writes.rs`** — write operations:

| Function | Notes |
|----------|-------|
| `put_block(block, height)` | stores header + body + slot/tx/addr indexes. Does NOT write height_index |
| `put_block_canonical(block, height)` | `put_block` + direct height_index update (no-fork convenience) |
| `set_canonical_chain(tip_hash, tip_height)` | walks backward from tip, updates height_index + hash_to_height. Only writer of canonical indexes |
| `seed_canonical_index(hash, height)` | snap sync: writes height_index + hash_to_height + snap_horizon |
| `rebuild_canonical_index()` | emergency: scans all headers, finds true tip by slot, walks back, rewrites both indexes |

**`block_store/maintenance.rs`** — cleanup/pruning/diagnostics (grew significantly since last audit):

| Function | Notes |
|----------|-------|
| `metrics()` | Prometheus `RocksDbMetrics` across 9 named CFs |
| `cleanup_fork_blocks()` | deletes headers/bodies/slot entries not in canonical hash_to_height index |
| `clear_indexes()` | clears height_index/slot_index/hash_to_height only (preserves block data) — used by automatic recovery |
| `clear()` | wipes ALL CFs — manual CLI `recover --yes` last resort |
| `delete_blocks_above(keep_height)` | truncate above a height, compacts affected range |
| `delete_block_by_height(height)` | single-block delete (e.g. stale old-chain block after genesis reset) |
| `prune_blocks_below(keep_above_height, chain_tip)` | bulk prune with `MIN_RETENTION=2000` floor from tip (`const MIN_RETENTION`, `block_store/types.rs:305`); batched writes of 500, compacts 6 CFs after |
| `storage_stats()` | entry counts per CF for diagnostics |
| `height_range()` | `(min_height, max_height)` from height_index |

**Backward-compatible body deserialization** (`block_store/types.rs:38`):
```
deserialize_body(bytes) tries in order:
  1. Current format (v3.7.1+: committed_output_count in Input)
  2. LegacyBlockBodyV2 (v3.6.0: sighash_type, no committed_output_count)
  3. LegacyBlockBodyV1 (v3.5.0: no sighash_type)
  4. Plain Vec<Transaction> (pre-BLS)
  5. Vec<LegacyTransactionV2>
  6. Vec<LegacyTransaction> (oldest)
```

## FUNCTIONS-STATEDB

**`state_db/queries.rs`** — read operations:

| Function | Notes |
|----------|-------|
| `get_utxo(&Outpoint)` | bincode deserialize from cf_utxo |
| `contains_utxo(&Outpoint)` | existence check |
| `get_utxos_by_pubkey(&Hash)` | prefix scan on cf_utxo_by_pubkey |
| `utxo_len()` | O(1) via AtomicU64 counter |
| `utxo_total_value()` | full scan sum of native amounts |
| `get_balance_with_maturity(&Hash, height, maturity)` | filtered scan |
| `get_immature_balance_with_maturity(...)` | coinbase/reward not yet mature |
| `get_bonded_balance` / `count_bonds` / `get_bond_entries` | **Phase 1 mirrored methods** — mirror `RocksDbUtxoStore` (now eliminated); identical logic lives here |
| `get_all_pools()` / `find_nft_by_token_id(&Hash)` | full cf_utxo scan filtered by OutputType |
| `total_confirmed(height, maturity, pool_pkh)` | excludes bonds + pool address |
| `address_count()` | distinct pubkey-hash prefixes in cf_utxo_by_pubkey |
| `has_unique_id/add_unique_id/remove_unique_id` | cf_unique_id direct ops (non-batch, migration/test) |
| `get_producer(&Hash)` | cf_producers lookup |
| `iter_producers()` | full scan of cf_producers |
| `get_exit_height(&Hash)` / `iter_exit_history()` | cf_exit_history lookup / full scan |
| `get_chain_state()` | panics if key exists but can't deserialize (prevents silent data loss) |
| `get_pending_updates()` / `get_last_applied()` | |
| `serialize_canonical_utxo()` | for state root: count + sorted (outpoint, canonical_entry) bytes; count derived from emitted body (not `utxo_len()`) to avoid desync (STOR028) |
| `load_producer_set()` | rebuilds full ProducerSet from cf_producers + cf_exit_history + pending_updates |
| `get_epoch_state()` / `get_epoch_state_version()` | |
| `get_epoch_producer_list()` / `get_active_production_list()` | |
| `get_attestation_accumulators()` | returns tuple of 3 sets + 3 maps + produced map |
| `get_chain_commitment_with_tip()` | |
| `get_oracle_sunset_state()` / `put_oracle_sunset_state()` | D.3 gradient, NOT in state root |
| `get_oracle_last_update_height()` / `put_oracle_last_update_height()` | AUDIT-P2-001, NOT in state root |
| `create_checkpoint(&Path)` | RocksDB hard-link checkpoint |

**`state_db/writes.rs`** — direct (non-batch) write operations:

| Function | Notes |
|----------|-------|
| `insert_utxo(outpoint, entry)` | counter-idempotent: only fetch_add when key is genuinely new (INC-I-136) |
| `remove_utxo(outpoint)` | removes primary + secondary |
| `clear_utxos()` | clears both UTXO CFs |
| `put_chain_state(cs)` | writes `0x01` prefix + bincode to cf_meta |
| `write_producer_set(ps)` | clears + rewrites cf_producers + cf_exit_history |
| `atomic_replace(cs, ps, utxo_iter)` | deletes UTXOs + producers + exit_history; writes all new state + meta. Preserves META epoch keys (Fix #10, 2026-04-15) |
| `clear_and_write_genesis(cs)` | full wipe + genesis ChainState in one batch |
| `import_utxos(entries)` | bulk: batches of 50,000 |
| `iter_utxos()` | full scan for snap sync / migration |
| `add_transaction(tx, height, is_coinbase, slot)` | stamps Bond outputs with slot |
| `spend_transaction(tx)` | returns total input amount |
| `delete_epoch_state()` | forces rebuild from blocks on next startup |
| `put_epoch_state(bytes)` | direct write (outside batch, for rollback paths) |
| `put_chain_commitment_with_tip(hash, scan_tip)` | |
| `flush_cf_utxo()` / `metrics()` | force memtable flush (test); Prometheus snapshot across 7 named CFs |

**`state_db/batch.rs`** — BlockBatch methods:

| Function | Notes |
|----------|-------|
| `begin_batch()` | creates empty BlockBatch (incl. pending/removed unique-id sets) |
| `add_utxo(outpoint, entry)` | adds to batch + pending_utxos map |
| `spend_utxo(outpoint)` | checks pending_utxos first (same-block-spend), then DB; also removes unique IDs for the spent entry |
| `spend_transaction_utxos(tx)` | iterates all inputs |
| `add_transaction_utxos(tx, height, is_coinbase, slot)` | stamps Bonds AND Pool metadata (creation_slot, TWAP accumulation before `last_update_slot` update — BUG-001 fix mirrors both write paths byte-identically); inserts unique IDs |
| `add_pending_unique_id` / `has_unique_id_check` / `remove_pending_unique_id` | Phase 1+3 same-block uniqueness overlay for NFT/Pool/FungibleAsset |
| `put_producer(pubkey_hash, info)` / `remove_producer` / `put_exit_history` | |
| `put_chain_state(cs)` / `put_pending_updates(updates)` | |
| `write_dirty_producers(ps, dirty, removed, dirty_exit)` | O(dirty) instead of O(all) |
| `write_full_producer_set(ps)` | clears + rewrites (for reorg) |
| `put_epoch_state(bytes)` / `put_epoch_state_version(v)` | |
| `put_attestation_accumulators(...)` / `put_epoch_bond_snapshot(...)` | |
| `put_epoch_producer_list(keys)` / `put_active_production_list(keys)` | concatenated 32B pubkeys |
| `set_last_applied(height, hash, slot)` | consistency canary |
| `put_undo(height, undo)` | writes to cf_undo in same batch |
| `put_chain_commitment(hash)` | |
| `get_utxo`/`contains_utxo`/`get_utxos_by_pubkey` | **overlay reads** — pending + committed, excludes spent-in-batch (Phase 3, used by `validate_transaction_with_utxos` via `UtxoProvider` impl) |
| `commit()` | atomic WriteBatch.write() + AtomicU64 update |

**`state_db/undo.rs`** — undo log:

| Function | Notes |
|----------|-------|
| `put_undo(height, undo)` | direct write (not batch) |
| `get_undo(height)` | returns Option<UndoData> |
| `prune_undo_before(keep_height)` | O(1): deletes only `keep_height - 1`; reclamation now via `periodic_compaction_seconds` (INC-I-108), NOT synchronous `compact_range_cf` |
| `prune_undo_above(keep_height)` | deletes all entries above keep_height |
| `has_undo_data(from, to)` | INC-I-136 M2: checkpoint guardian check — `true` iff every height in range has undo data |
| `prune_undo_below(keep_height)` | one-shot bulk delete strictly below keep_height (catch-up after UNDO_KEEP_DEPTH reduction) |

## FUNCTIONS-UTXO

**`UtxoSet` enum** dispatches to `InMemoryUtxoStore` or `Arc<StateDb>` (Phase 4 — NO separate RocksDB instance):

| Function | Notes |
|----------|-------|
| `new()` | InMemory backend |
| `from_state_db(Arc<StateDb>)` | RocksDb backend (production since Phase 4) |
| `get(outpoint)` / `contains(outpoint)` | owned UtxoEntry / existence |
| `add_transaction(tx, height, is_coinbase, slot)` | stamps Bond outputs; RocksDb variant used only in rollback paths (normal blocks use BlockBatch) |
| `spend_transaction(tx)` | returns total Amount |
| `total_value()` / `len()` / `is_empty()` / `address_count()` | |
| `total_confirmed(height, maturity, pool_pkh)` | excludes bonds + pool |
| `get_by_pubkey_hash(hash)` | all UTXOs for address |
| `get_balance_with_maturity` / `get_immature_balance_with_maturity` | |
| `get_bonded_balance` / `count_bonds` / `get_bond_entries` | bond_unit / FIFO-ordered |
| `get_pool_utxo(pool_id)` / `get_all_pools()` | pool lookups |
| `defi_health_inputs()` | `(total_active_bonds, max_pool_by_tvl)` for D4 AC-6 economic-security metric |
| `find_nft_by_token_id(token_id)` | unique_id index fast-path |
| `has_unique_id(prefix, id)` | checks unique_id index |
| `iter_all()` | used by atomic_replace for snap sync |
| `serialize_canonical()` / `deserialize_canonical(bytes)` | sorted by outpoint key, 59B canonical values; always produces InMemory backend |
| `insert(outpoint, entry)` / `remove(outpoint)` | direct (testing/reorgs) |
| `metrics()` | `None` for InMemory, `Some(state_db.metrics())` for RocksDb |
| `is_rocksdb()` | backend check |

**`UtxoEntry` maturity rules** (`utxo/types.rs:151`):
```
is_spendable_at_with_maturity(height, maturity):
  1. output.lock_until <= height (time lock)
  2. if is_coinbase OR is_epoch_reward: (height - self.height) >= maturity
```

## FUNCTIONS-PRODUCERSET

**`producer/set_core.rs`** — core: `new()`, `from_parts()`, `as_parts()`, `rebuild_unbonding_index()`, `get/get_mut/get_by_pubkey`, `active_producers()`, `active_producers_at_height(h)` (cached), `active_producers_for_scheduling_at_height(h, inc_i_068_gate, audit_gate)` (INC-I-075 weight=0 filter), `ensure_active_cache(h)`, `queue_update()`, `apply_pending_updates_with_cap(cap)` (INC-I-078 defensive delegation cap), `pending_updates_by_pubkey()`, `pending_addbond_count()` (INC-I-080).

**`producer/set_registration.rs`** — `register()`, `register_for_network()`, `has_prior_exit()`, `prune_exit_history()`, `register_genesis_producer()`, `with_genesis_producers()`.

**`producer/set_lifecycle.rs`** — `request_exit()`, `cancel_exit()`, `process_unbonding()` (uses unbonding_index, O(k)), `slash_producer()` (100% burn), `renew()`, `cleanup_exited()`, deprecated Pull/Claim reward methods.

**`producer/set_delegation.rs`** — `delegate_bonds()` / `delegate_bonds_capped()` (AUDIT-PROD-002 self-delegation guard, INC-I-078 cap), `cleanup_all_delegations()` (both directions), `revoke_delegation()`.

**`producer/set_governance.rs`** — `total_weight()`, `weighted_veto_threshold()`, `total_effective_weight()` (Active-only, "el silencio no bloquea"), `has_weighted_veto()`, `distribute_weighted_rewards()` (deprecated, Pull/Claim).

**`producer/info.rs`** — `ProducerInfo` impl: `new/new_with_bonds/new_with_prior_exit`, `is_active/can_produce`, `start_unbonding/is_unbonding_complete`, `slash/is_slashed`, `add_bonds/remove_bonds`, `selection_weight_at(height, audit_activation)` (AUDIT-PROD-001: subtracts delegated_bonds post-gate), `calculate_withdrawal_with_quarter()` (FIFO, `calculate_withdrawal_from_bonds()` free fn mirrors it), `apply_withdrawal()` (auto-exit at bond_count=0), `weight_for_network/effective_weight_for_network`, `activity_status_for_network`, `has_governance_power_for_network`.

**`producer/seniority.rs`** — `producer_weight_for_network()` (discrete yearly steps 1-4), `total_weight_for_network()`, `weighted_veto_threshold_for_network()`.

**`PendingProducerUpdate` variants** (`producer/types.rs`): `Register`, `Exit`, `Slash`, `AddBond`, `DelegateBond`, `RevokeDelegation`, `RequestWithdrawal`.

**Constants** (`producer/constants.rs`): `ACTIVATION_DELAY=10`, `MAX_WEIGHT=4`/`MIN_WEIGHT=1`, `VETO_THRESHOLD_PERCENT=40`, `VETO_BOND_AMOUNT=1e9`, `BOND_UNIT=1e9` (mainnet/testnet; devnet uses `bond_unit_for_network()`=1e8), `EXIT_HISTORY_RETENTION=2*2_102_400` (~8yr), `REACTIVATION_THRESHOLD=8_640`. Network-aware variants (`*_for_network`) preferred over deprecated legacy constants.

**Persistence** (`producer/set_persistence.rs:18`): `load(path)`: JSON first → bincode fallback → fresh; calls `rebuild_unbonding_index()` + `migrate_bond_entries()`. `save(path)`: JSON, atomic `.tmp` rename. `serialize_canonical()`: sorted by Hash key; sorts `additional_bonds`, `received_delegations`, `bond_entries` within each entry.

## FUNCTIONS-SNAPSHOT

**`snapshot.rs`**:

| Function | Signature | Notes |
|----------|-----------|-------|
| `compute_state_root` | `(cs, utxo, ps) → Result<Hash>` | `H(H(cs_canon) ‖ H(utxo_canon) ‖ H(ps_canon))` |
| `compute_state_root_with_epoch_state` | `(cs, utxo, ps, Option<Hash>) → Result<Hash>` | M-Choice1/INC-I-034: `None` = bit-identical to legacy; `Some(h)` = 4-component hash. Present but NOT YET WIRED at call sites (Phase 1 only) |
| `compute_state_root_from_bytes` | `(cs_bytes, utxo_bytes, ps_bytes) → Result<Hash>` | for checkpoint verification; per-component error identification |
| `compute_scheduler_root` | `(epoch_bond_snapshot, epoch, producer_list, ...) → Hash` | scheduler state hash — NOT in block header, observational only. Delegates to `doli_core::epoch_state_hash` |
| `StateSnapshot::create` | `(cs, utxo, ps) → Result<StateSnapshot>` | builds bytes + state root |

**State root log format** (searchable in prod): `[STATE_ROOT] cs={:.16} utxo={:.16} ps={:.16} cs_bytes=N utxo_bytes=N ps_bytes=N` and `[STATE_ROOT_HF] ... es={:.16} ...` (4-component, not yet wired).

## FUNCTIONS-ARCHIVER

**`archiver.rs`** — block archiver:

| Function | Notes |
|----------|-------|
| `BlockArchiver::new(rx, dir)` | async receiver + directory |
| `BlockArchiver::run()` | tokio task: receives blocks, calls `archive_block()` |
| `BlockArchiver::catch_up(dir, block_store, tip, genesis_hash)` | static: fills gaps 1..=tip from BlockStore, skips existing files, verifies genesis hash |
| `restore_from_archive` / `backfill_from_archive` / `force_backfill_from_archive` | full import / skip-existing / also-replace-fork-blocks (checksum compare) |
| `manifest_height(dir)` / `manifest_genesis_hash(dir)` | read manifest.json fields |

**Archive file format**: `{height:010}.block` (bincode Block) + `{height:010}.blake3` (checksum sidecar) + `manifest.json` (`{latest_height, latest_hash, genesis_hash}`). All writes atomic (`.tmp` then `rename`).

## FUNCTIONS-MISC

| Module | Key items | Notes |
|--------|-----------|-------|
| `metrics.rs` | `collect_db_metrics(db, instance, cf_names, memtable_cap, cache, cache_capacity) -> RocksDbMetrics` | INC-I-104/106: CF-scoped props summed across named CFs; block cache queried directly from `Cache` handle (not per-CF, avoids N-fold over-report) |
| `mmr.rs` | `CompactMmr` (peaks-only Merkle Mountain Range, O(log n) append/root), `IncrementalStateRoot` (MMR + XOR spent-accumulator) | Primitive for O(log n) state root vs O(n) full UTXO hash — **NOT wired into apply_block**; present as a future optimization only |
| `content_store.rs` | `ContentStore::open/put/get/release/get_refcount/contains/len` | BLAKE3-keyed, ref-counted blob dedup for NFT images/docs |
| `update.rs` | `UpdateState::load/save/add_pending_release/record_vote/record_history` | survives restart for updater crate's vote/release state |
| `utxo_size_monitor.rs` | `UtxoSizeMonitor::new/get_cached_size/compute_size/computation_count` | F1 monitor: 60s-TTL cache of `serialize_canonical_utxo().len()` for the 16MB `MAX_SYNC_SIZE` snap-sync wire limit (alerts at 12MB=75%) |

## DEPENDENCIES

| This Domain Uses | Crate/Skill | What For |
|------------------|-----------|----------|
| `doli_core::{transaction, validation, consensus, network, network_params, oracle, types, maintainer, rewards}` | `crates/core` | Transaction/Output types, `UtxoProvider`/`UtxoInfo` traits, consensus constants, network-aware params, `OracleSunsetState`, `epoch_state_hash()` |
| `crypto::{Hash, PublicKey, hash, Hasher}` | `crates/crypto` | all hashing and key types |
| `rocksdb` (rust-rocksdb 0.22) | external | column families, WriteBatch, Cache, checkpoint |
| `tokio::sync::mpsc` | external | `BlockArchiver` channel |
| `doli_core::rewards::BlockSource` trait | `crates/core` | implemented BY `BlockStore` (`block_store/trait_impls.rs`) so `WeightedRewardCalculator` can fetch blocks by height |

| Used By | Crate/Skill | What For |
|---------|-----------|----------|
| `bins/node` (`apply_block()`, `init.rs`, `rollback.rs`, `fork_recovery.rs`) | node skill | primary consumer — owns `BlockStore`/`StateDb`/`UtxoSet`/`ProducerSet`/`ChainState` instances, drives all batch/rollback/snap-sync operations |
| `crates/network` (sync manager) | network skill | snap sync uses `StateSnapshot`, `compute_state_root_from_bytes`, `atomic_replace`, `seed_canonical_index` |
| `crates/rpc` (methods) | rpc skill | read-only queries: `get_balance_with_maturity`, `get_bond_entries`, `iter_producers`, `get_block_by_height`, oracle status meta keys, `RocksDbMetrics` for `/metrics` |
| `crates/mempool` | mempool skill | UTXO existence/spend checks via `UtxoProvider` |
| `crates/updater` | updater skill | `UpdateState` persistence for vote/release history |
| `bins/cli` | cli skill | `recover --yes` (`BlockStore::clear()`), checkpoint/backup commands |

## CONSTRAINTS

### Atomicity invariant
**One WriteBatch per block** — ALL of UTXO changes, producer changes, chain_state update, last_applied canary, unique-id index, and undo data are written in a single `RocksDB::write(batch)`. A crash leaves the DB either fully applied or not at all. (`state_db/batch.rs:481`)

### Canonical index invariant
`height_index` and `hash_to_height` in BlockStore are EXCLUSIVELY written by `set_canonical_chain()`. `put_block()` does NOT touch them. This prevents fork blocks from appearing in canonical chain queries. (`block_store/writes.rs:14-19`)

### Same-block-spend
`BlockBatch` maintains `pending_utxos: HashMap<Outpoint, UtxoEntry>` for outputs created in the current block but not yet committed. `spend_utxo()` checks this map before the DB so TX2 can spend TX1's output within the same block. Same pattern applies to `pending_unique_ids`/`removed_unique_ids` for NFT/Pool/Asset uniqueness. (`state_db/batch.rs:71`)

### Deferred mutations (epoch boundary rule)
Producer state changes (Register, AddBond, Exit, Slash, Withdrawal, Delegation) are queued as `PendingProducerUpdate` and applied ONLY at epoch boundaries via `apply_pending_updates()`/`apply_pending_updates_with_cap()`. This prevents scheduler divergence between forks. Exception: epoch 0 producers are applied immediately.

### atomic_replace CF_META exclusion
`atomic_replace()` does NOT delete `CF_META` — only deletes `cf_utxo`, `cf_utxo_by_pubkey`, `cf_producers`, `cf_exit_history`. This preserves scheduler meta-state (epoch_producer_list, attestation accumulators, oracle state, etc.) across rollbacks and snap sync. (`state_db/writes.rs:143-186`)

### ChainState versioned format
Written with `0x01` byte prefix. `get_chain_state()` PANICS (not returns None) if the key exists but can't be deserialized — prevents silent state loss. (`state_db/queries.rs:399-416`)

### serialize_canonical() fields
`ChainState.serialize_canonical()` encodes ONLY 9 consensus fields (140 bytes fixed). `snap_sync_height`, `active_protocol_version`, `pending_protocol_activation` are NOT included — they are local bookkeeping. (`chain_state.rs:143`)

### Snap horizon floor
After snap sync, `set_canonical_chain()` stops walking backward when `height <= snap_horizon`. The snap anchor header was never persisted — walking into it crashes with "header missing". (`block_store/writes.rs:129-135`)

### UTXO count caching
`StateDb.utxo_count: AtomicU64` is set on `open()` by counting cf_utxo entries. Updated by `fetch_add/sub` in batch/direct operations, made **counter-idempotent on insert** (only increments on genuinely new keys — INC-I-136 body-gap recovery could otherwise re-insert existing UTXOs and inflate the count). Never scan for count — use `utxo_len()`. (`state_db/types.rs:81`, `state_db/writes.rs:27-47`)

### Bond stamp invariant
Bond outputs get `extra_data = slot.to_le_bytes()` stamped by the NODE at apply time, not trusted from the TX. Applies in `BlockBatch.add_transaction_utxos()`, `StateDb.add_transaction()`, and `UtxoSet::add_transaction` (RocksDb rollback path). (`state_db/batch.rs:146-150`)

### Pool metadata stamp invariant (BUG-001)
Pool UTXOs get `creation_slot`/`last_update_slot`/`cumulative_price` (TWAP) stamped/accumulated by the node at apply time. `BlockBatch::add_transaction_utxos` mirrors `utxo_rocks::add_transaction` byte-for-byte so both write paths (batch vs any legacy direct path) produce identical Pool UTXO bytes. TWAP accumulates BEFORE `last_update_slot` is overwritten. (`state_db/batch.rs:151-185`)

### Total work = height
`ChainState.update()` sets `total_work = height` (assignment, not increment). This prevents divergence between nodes restarted at different heights. (`chain_state.rs:124-132`)

### epoch_state deletion danger
`delete_epoch_state()` forces a non-deterministic rebuild from local blocks on next startup. Snap-synced nodes have incomplete block history → different rebuild results → fork at epoch boundary. Only call when required by `EPOCH_STATE_FORMAT_VERSION` mismatch. (`state_db/queries.rs:598-601`)

### Block-store pruning floor
`prune_blocks_below()` NEVER prunes within `MIN_RETENTION=2000` blocks of chain tip (`block_store/types.rs:305`), independent of `consensus::UNDO_KEEP_DEPTH`. This is the rebuild-from-blocks safety floor, set to 2× `MAX_REORG_DEPTH`.

### Metrics cache-cap coupling
`DB_WRITE_BUFFER_SIZE_BYTES` (per-instance memtable cap) is a SHARED constant between `open()` (sets `db_write_buffer_size`) and `metrics()` (reports the cap) — cannot drift. Same pattern for `block_cache_capacity_bytes` (cannot query `Cache::get_capacity()` in rust-rocksdb 0.22, so the constructor value is remembered on the struct). (`state_db/open.rs:39,206`, `block_store/types.rs:87,109`)

### Never change canonical encoding without a chain reset
`ChainState::serialize_canonical()`, `UtxoEntry::serialize_canonical_bytes()`, `ProducerSet::serialize_canonical()` are all consensus-critical — every node computes the identical state root only if these byte layouts never change. See SERIALIZATION section.

## PATTERNS

### Pattern: read-modify-write producer
```rust
let mut batch = state_db.begin_batch();
batch.put_producer(&pubkey_hash, &updated_info);
// ... other mutations ...
batch.commit()?;
```
Never call `state_db.write_producer_set()` during normal block application — only for reorg/migration.

### Pattern: dirty-only producer writes (normal block)
```rust
let mut dirty_keys: HashSet<Hash> = HashSet::new();
dirty_keys.insert(pubkey_hash);
batch.write_dirty_producers(&producer_set, &dirty_keys, &removed_keys, &dirty_exit_keys);
```
This writes O(dirty) instead of O(all producers), critical for networks with 1000+ producers.

### Pattern: full producer rewrite (reorg/migration)
```rust
batch.write_full_producer_set(&producer_set);  // clears + rewrites all
// OR
state_db.atomic_replace(cs, ps, utxo_iter)?;  // full state swap
```

### Pattern: epoch state persistence
```rust
batch.put_epoch_state(&epoch_state_bytes);
batch.put_epoch_state_version(CURRENT_EPOCH_STATE_FORMAT_VERSION);
batch.commit()?;

// Load on startup:
if let Some(bytes) = state_db.get_epoch_state() {
    let stored_ver = state_db.get_epoch_state_version();
    if stored_ver == Some(CURRENT_EPOCH_STATE_FORMAT_VERSION) {
        // deserialize and use
    } else {
        state_db.delete_epoch_state();  // force rebuild
    }
}
```

### Pattern: same-block-spend / same-block-unique-id
Works automatically via `pending_utxos`/`pending_unique_ids`/`removed_unique_ids` maps in `BlockBatch`. No special handling needed by callers. TX ordering within a block must be topologically sorted before `apply_block()`.

### Pattern: address transaction history (paginated)
```rust
let heights = block_store.get_address_heights(&pubkey_hash, before_height, limit);
// Key format: addr(32) ++ height(8 BE) → empty; iterated in REVERSE for descending order
```

### Pattern: snap sync apply
```rust
let computed = compute_state_root_from_bytes(&cs_bytes, &utxo_bytes, &ps_bytes)?;
assert_eq!(computed, snapshot.state_root);

let cs: ChainState = bincode::deserialize(&cs_bytes)?;
let ps: ProducerSet = bincode::deserialize(&producer_bytes)?;
let utxo = UtxoSet::deserialize_canonical(&utxo_bytes)?;
state_db.atomic_replace(&cs, &ps, utxo.iter_all().into_iter())?;

block_store.seed_canonical_index(cs.best_hash, cs.best_height)?;
cs.mark_snap_synced(cs.best_height);
```

### Pattern: state root diagnosis
Grep logs for `[STATE_ROOT]`:
```
[STATE_ROOT] cs={16 chars} utxo={16 chars} ps={16 chars} cs_bytes=140 utxo_bytes=N ps_bytes=N
```
Compare `cs`, `utxo`, `ps` hashes across nodes — the differing component identifies the divergent state.

### Pattern: per-CF RocksDB tuning
Both `BlockStore::open()` and `StateDb::open()` build a local `cf_opts_*()` helper taking (write_buffer_mb, max_write_buffer_num, bloom, block_size_kb, compression, target_file_size_mb, l0_slowdown, l0_stop) and a SHARED `rocksdb::Cache` passed to every CF's `BlockBasedOptions`. Hot point-lookup CFs get bloom filters; prefix-scan CFs (`cf_utxo_by_pubkey`, `addr_tx_index`) explicitly omit bloom (C-010). See `specs/rocksdb-configuration-architecture.md`.

### Pattern: Prometheus metrics export
```rust
let m: RocksDbMetrics = state_db.metrics(); // or block_store.metrics(), utxo_set.metrics()
```
Always pass the instance's named CF list + the shared `Cache` handle + its configured capacity — do NOT read block-cache properties per-CF (over-reports by N for N CFs sharing one cache, INC-I-106).

## SERIALIZATION

### Format matrix

| Data | Storage format | Canonical format | Wire (snap sync) |
|------|---------------|-----------------|-----------------|
| `BlockHeader` | bincode | — | bincode |
| `Block` body | bincode `BlockBody` | — | bincode |
| `ChainState` (StateDb) | `0x01` prefix + bincode | 140-byte fixed encoding | bincode |
| `ChainState` (file) | bincode | 140-byte fixed encoding | bincode |
| `UtxoEntry` (StateDb) | bincode | 61+N bytes canonical | — |
| `UtxoSet` | bincode (InMemory) / RocksDB via StateDb | sorted outpoints + canonical values | canonical |
| `ProducerInfo` | bincode (StateDb) | bincode (sorted Vec fields) | — |
| `ProducerSet` (file) | JSON first, bincode fallback | sorted by hash key | bincode |
| `UndoData` | bincode (cf_undo) | — | — |
| `MaintainerState` / `UpdateState` | bincode (file) | — | — |
| `EpochState` | raw bytes (format-versioned) | — | — |
| Archive blocks | bincode `Block` | — | — |
| `ContentStore` blobs | raw bytes, BLAKE3-keyed | — | — |

### ChainState format versions
- `0x01` prefix (current): versioned bincode (`state_db/queries.rs:423`)
- Legacy unversioned: raw bincode (v1.0.29 and earlier)
- Legacy migration: raw bincode + appended `1u32 LE` + `0x00` byte (v1.0.28: missing `active_protocol_version` field)

### BlockBody versions (tried in order)
1. `BlockBody` (current, v3.7.1+: `committed_output_count` in Input)
2. `LegacyBlockBodyV2` (v3.6.0: `sighash_type`, no `committed_output_count`)
3. `LegacyBlockBodyV1` (v3.5.0: no `sighash_type`)
4. `Vec<Transaction>` (pre-BLS, no `aggregate_bls_signature`)
5. `Vec<LegacyTransactionV2>` (pre-BLS + old tx format)
6. `Vec<LegacyTransaction>` (oldest)

### Canonical encoding rules (state root safety)
1. **ChainState**: fixed 140 bytes, field positions never change (`chain_state.rs:143`)
2. **UtxoSet**: 8B count LE + entries sorted by outpoint key (RocksDB lexicographic = correct)
3. **UtxoEntry**: fixed base 61 bytes + variable extra_data with length prefix
4. **ProducerSet**: sorted by pubkey Hash key; Vec fields within each entry also sorted
5. **State root**: `H(H(cs_140B) ‖ H(utxo_canonical) ‖ H(ps_canonical))`

**Never change canonical encoding without a chain reset** — all nodes diverge immediately. Per CLAUDE.md #0 RULE, prefer future-height activation gates over genesis resets wherever the change can be made forward-compatible.
