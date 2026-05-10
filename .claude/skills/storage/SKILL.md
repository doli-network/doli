# storage — DOLI Persistence Layer
<!-- @INDEX
ENTRY-POINTS: lines 16-38
DATA-FLOWS: lines 40-73
STRUCTS: lines 75-158
COLUMN-FAMILIES: lines 160-205
FUNCTIONS-BLOCKSTORE: lines 207-267
FUNCTIONS-STATEDB: lines 269-360
FUNCTIONS-UTXO: lines 362-420
FUNCTIONS-PRODUCERSET: lines 422-490
FUNCTIONS-SNAPSHOT: lines 492-525
FUNCTIONS-ARCHIVER: lines 527-560
CONSTRAINTS: lines 562-620
PATTERNS: lines 622-670
SERIALIZATION: lines 672-720
-->

## ENTRY-POINTS

Primary structs and their initialization paths:

| Struct | Open/New | File |
|--------|----------|------|
| `BlockStore` | `BlockStore::open(path)` | `block_store/open.rs:16` |
| `StateDb` | `StateDb::open(path)` | `state_db/open.rs:14` |
| `UtxoSet` | `UtxoSet::new()` (in-memory) / `UtxoSet::open_rocksdb(path)` | `utxo/set.rs:29,34` |
| `RocksDbUtxoStore` | `RocksDbUtxoStore::open(path)` | `utxo_rocks.rs:37` |
| `ProducerSet` | `ProducerSet::new()` / `ProducerSet::load(path)` / `ProducerSet::from_parts(...)` | `producer/set_core.rs:13` / `producer/set_persistence.rs:18` |
| `ChainState` | `ChainState::new(genesis_hash)` / `ChainState::load(path)` | `chain_state.rs:80,98` |
| `StateSnapshot` | `StateSnapshot::create(cs, utxo, ps)` | `snapshot.rs:202` |
| `MaintainerState` | `MaintainerState::load(data_dir)` | `maintainer.rs:40` |
| `BlockArchiver` | `BlockArchiver::new(rx, dir)` + `archiver.run().await` | `archiver.rs:24,29` |

**Startup load order** (`Node::new()` in `bins/node/src/node/init.rs`):
1. `BlockStore::open(blocks_dir)`
2. `StateDb::open(state_dir)` — loads UTXO count from RocksDB
3. `StateDb::load_producer_set()` — rebuilds `ProducerSet` from `cf_producers` + `cf_exit_history` + `META_PENDING_UPDATES`
4. `StateDb::get_chain_state()` — loads `ChainState` from `cf_meta`
5. `StateDb::get_epoch_state()` — loads persisted `EpochState` bytes (if exists)

## DATA-FLOWS

### Block application (happy path):
```
apply_block()
  → state_db.begin_batch()               # creates BlockBatch
  → batch.spend_transaction_utxos(tx)   # deletes inputs from cf_utxo + cf_utxo_by_pubkey
  → batch.add_transaction_utxos(tx)     # adds outputs to cf_utxo + cf_utxo_by_pubkey
  → batch.put_producer()/remove_producer()  # updates cf_producers
  → batch.put_chain_state(cs)           # updates cf_meta["chain_state"]
  → batch.set_last_applied(h, hash, slot) # writes consistency canary
  → batch.put_undo(height, UndoData)    # saves rollback data to cf_undo
  → batch.put_epoch_state(bytes)        # saves EpochState snapshot
  → batch.commit()                      # single atomic RocksDB WriteBatch.write()
  → block_store.put_block(block, height) # headers, bodies, slot/tx indexes
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
  → state_db.atomic_replace(cs, ps, utxo_iter)   # delete all + write all, one batch
  → block_store.seed_canonical_index(hash, height) # writes snap_horizon to cf_meta
  → chain_state.mark_snap_synced(height)
```

### State root computation:
```
compute_state_root(cs, utxo, ps)
  → cs_hash = H(cs.serialize_canonical())     # 140 bytes, fixed-field
  → utxo_hash = H(utxo.serialize_canonical()) # sorted by outpoint key
  → ps_hash = H(ps.serialize_canonical())     # sorted by pubkey hash
  → H(cs_hash_bytes || utxo_hash_bytes || ps_hash_bytes)
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

### BlockBatch (`state_db/types.rs:70`)
```rust
pub struct BlockBatch<'a> {
    db: &'a StateDb,
    batch: rocksdb::WriteBatch,
    utxo_delta: i64,                              // tracks net UTXO change
    pending_utxos: HashMap<Outpoint, UtxoEntry>, // same-block-spend support
    spent_in_batch: Vec<Outpoint>,
}
```
- `commit()` writes the entire batch atomically, then updates `utxo_count` AtomicU64

### LastApplied (`state_db/types.rs:85`)
```rust
pub struct LastApplied { pub height: u64, pub hash: Hash, pub slot: u32 }
// Serialized as 44 bytes: 8 (height LE) + 32 (hash) + 4 (slot LE)
```
- Consistency canary: written in the same WriteBatch as all state changes
- If `last_applied.hash == chain_state.best_hash`, the DB is consistent

### ProducerInfo (`producer/types.rs:72`)
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
    pub bond_count: u32,               // 1-100 bonds
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

### ProducerSet (`producer/types.rs:214`)
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

### MaintainerState (`maintainer.rs:22`)
```rust
pub struct MaintainerState { pub set: MaintainerSet, pub last_derived_height: u64 }
// Persisted as: {data_dir}/maintainer_state.bin (bincode)
```

## COLUMN-FAMILIES

### BlockStore (`block_store/types.rs:85-94`)

| CF Name | Key | Value | Notes |
|---------|-----|-------|-------|
| `headers` | Hash (32B) | bincode BlockHeader | ALL blocks incl. forks |
| `bodies` | Hash (32B) | bincode BlockBody | txs + BLS sig + attestation_bitfield |
| `height_index` | height u64 LE (8B) | Hash (32B) | CANONICAL chain only |
| `slot_index` | slot u32 LE (4B) | Hash (32B) | last block per slot |
| `presence` | (deprecated) | (deprecated) | cleaned up on open |
| `hash_to_height` | Hash (32B) | height u64 LE (8B) | reverse lookup, O(1) |
| `tx_index` | tx_hash (32B) | height u64 LE (8B) | tx → block height |
| `addr_tx_index` | pubkey_hash(32B) ++ height(8B BE) | empty | address history, sorted by height desc |
| `meta` | `b"snap_horizon"` | height u64 LE | snap sync anchor |

**CRITICAL**: `height_index` and `hash_to_height` are written ONLY by `set_canonical_chain()`. `put_block()` does NOT write them — that prevents fork blocks from polluting the canonical index.

### StateDb (`state_db/types.rs:37-57`)

| CF Name | Key | Value | Notes |
|---------|-----|-------|-------|
| `cf_utxo` | Outpoint (36B) = txhash(32) + index(4 LE) | bincode UtxoEntry | primary UTXO index |
| `cf_utxo_by_pubkey` | pubkey_hash(32B) ++ outpoint(36B) = 68B | `0x00` | secondary index for balance lookups |
| `cf_producers` | pubkey_hash (32B) | bincode ProducerInfo | |
| `cf_exit_history` | pubkey_hash (32B) | height u64 LE (8B) | anti-Sybil |
| `cf_meta` | string key | varies | chain_state, pending_updates, epoch state |
| `cf_undo` | height u64 LE (8B) | bincode UndoData | rollback data |

**Meta keys** (`state_db/types.rs:45-57`):

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

### RocksDbUtxoStore (`utxo_rocks.rs:18-24`)

| CF Name | Key | Value |
|---------|-----|-------|
| `utxo` | Outpoint (36B) | bincode UtxoEntry |
| `utxo_by_pubkey` | pubkey_hash(32B) ++ outpoint(36B) = 68B | empty |
| `unique_id` | prefix(1B) + id(32B) = 33B | empty |

UID prefixes: `0x01`=NFT, `0x02`=Asset, `0x03`=Pool, `0x04`=Channel

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
| `get_balance_with_maturity(&Hash, height, maturity)` | filtered scan |
| `get_producer(&Hash)` | cf_producers lookup |
| `iter_producers()` | full scan of cf_producers |
| `get_exit_height(&Hash)` | cf_exit_history lookup |
| `get_chain_state()` | panics if key exists but can't deserialize (prevents silent data loss) |
| `get_pending_updates()` | |
| `get_last_applied()` | consistency canary |
| `serialize_canonical_utxo()` | for state root: count + sorted (outpoint, canonical_entry) bytes |
| `load_producer_set()` | rebuilds full ProducerSet from cf_producers + cf_exit_history + pending_updates |
| `get_epoch_state()` / `get_epoch_state_version()` | |
| `get_epoch_producer_list()` / `get_active_production_list()` | |
| `get_attestation_accumulators()` | returns tuple of 3 sets + 3 maps + produced map |
| `get_chain_commitment_with_tip()` | |
| `create_checkpoint(&Path)` | RocksDB hard-link checkpoint |

**`state_db/writes.rs`** — direct (non-batch) write operations:

| Function | Notes |
|----------|-------|
| `insert_utxo(outpoint, entry)` | also writes secondary index |
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

**`state_db/batch.rs`** — BlockBatch methods:

| Function | Notes |
|----------|-------|
| `begin_batch()` | creates empty BlockBatch |
| `add_utxo(outpoint, entry)` | adds to batch + pending_utxos map |
| `spend_utxo(outpoint)` | checks pending_utxos first (same-block-spend), then DB |
| `spend_transaction_utxos(tx)` | iterates all inputs |
| `add_transaction_utxos(tx, height, is_coinbase, slot)` | stamps Bonds |
| `put_producer(pubkey_hash, info)` | |
| `remove_producer(pubkey_hash)` | |
| `put_exit_history(pubkey_hash, height)` | |
| `put_chain_state(cs)` | `0x01` prefix + bincode |
| `put_pending_updates(updates)` | |
| `write_dirty_producers(ps, dirty, removed, dirty_exit)` | O(dirty) instead of O(all) |
| `write_full_producer_set(ps)` | clears + rewrites (for reorg) |
| `put_epoch_state(bytes)` / `put_epoch_state_version(v)` | |
| `put_attestation_accumulators(...)` | |
| `put_epoch_bond_snapshot(snapshot, epoch)` | |
| `put_epoch_producer_list(keys)` | concatenated 32B pubkeys |
| `put_active_production_list(keys)` | |
| `set_last_applied(height, hash, slot)` | consistency canary |
| `put_undo(height, undo)` | writes to cf_undo in same batch |
| `put_chain_commitment(hash)` | |
| `commit()` | atomic WriteBatch.write() + AtomicU64 update |

**`state_db/undo.rs`** — undo log:

| Function | Notes |
|----------|-------|
| `put_undo(height, undo)` | direct write (not batch) |
| `get_undo(height)` | returns Option<UndoData> |
| `delete_undo(height)` | |
| `prune_undo_before(keep_height)` | O(1): deletes only `keep_height - 1`, compacts every 100 blocks |
| `prune_undo_above(keep_height)` | deletes all entries above keep_height |

## FUNCTIONS-UTXO

**`UtxoSet` enum** dispatches to `InMemoryUtxoStore` or `RocksDbUtxoStore`:

| Function | Notes |
|----------|-------|
| `new()` | InMemory backend |
| `open_rocksdb(path)` | RocksDb backend |
| `get(outpoint)` | owned UtxoEntry |
| `contains(outpoint)` | |
| `add_transaction(tx, height, is_coinbase, slot)` | stamps Bond outputs |
| `spend_transaction(tx)` | returns total Amount |
| `total_value()` | sum of all native amounts |
| `len()` / `is_empty()` | |
| `total_confirmed(height, maturity, pool_pkh)` | excludes bonds + pool |
| `get_by_pubkey_hash(hash)` | all UTXOs for address |
| `get_balance_with_maturity(hash, height, maturity)` | spendable only |
| `get_immature_balance_with_maturity(hash, height, maturity)` | coinbase/reward not yet mature |
| `get_bonded_balance(hash)` | sum of Bond UTXOs |
| `count_bonds(hash, bond_unit)` | bond_amount / bond_unit |
| `get_bond_entries(hash)` | (outpoint, creation_slot, amount), FIFO-ordered |
| `get_pool_utxo(pool_id)` | finds Pool output by pubkey_hash = pool_id |
| `get_all_pools()` / `get_all_collateral()` | |
| `find_nft_by_token_id(token_id)` | uses unique_id index fast-path |
| `has_unique_id(prefix, id)` | checks unique_id index |
| `iter_all()` | used by atomic_replace for snap sync |
| `serialize_canonical()` | sorted by outpoint key, 59B canonical values |
| `deserialize_canonical(bytes)` | always produces InMemory backend |
| `insert(outpoint, entry)` | direct insert (testing/reorgs) |
| `remove(outpoint)` | |

**`UtxoEntry` maturity rules** (`utxo/types.rs:151`):
```
is_spendable_at_with_maturity(height, maturity):
  1. output.lock_until <= height (time lock)
  2. if is_coinbase OR is_epoch_reward: (height - self.height) >= maturity
```

## FUNCTIONS-PRODUCERSET

**`producer/set_core.rs`** — core operations:

| Function | Notes |
|----------|-------|
| `new()` | empty set |
| `from_parts(producers, exit_history, pending_updates)` | used by StateDb loading; rebuilds unbonding_index |
| `as_parts()` | borrows raw maps for StateDb writes |
| `rebuild_unbonding_index()` | called after deserialization |
| `get(pubkey_hash)` | O(1) lookup |
| `get_by_pubkey(pubkey)` | hashes pubkey first |
| `active_producers()` | all Active-status producers |
| `active_producers_at_height(height)` | Active + past ACTIVATION_DELAY (10 blocks). Uses cache |
| `ensure_active_cache(height)` | pre-build cache for hot path |
| `queue_update(update)` | adds to pending_updates |
| `apply_pending_updates()` | applies all pending at epoch boundary, clears cache |
| `has_pending_updates()` / `pending_update_count()` | |
| `pending_updates_for(pubkey)` | O(M) scan |
| `pending_updates_by_pubkey()` | O(M) single-pass grouping |
| `pending_registration_keys()` / `pending_registrations()` | |

**`PendingProducerUpdate` variants** (`producer/types.rs:178`):
- `Register { info: Box<ProducerInfo>, height }` — deferred at epoch boundary
- `Exit { pubkey, height }` — triggers unbonding
- `Slash { pubkey, height }` — 100% bond burn
- `AddBond { pubkey, outpoints, bond_unit, creation_slot }` — bond stacking
- `DelegateBond { delegator, delegate, bond_count }` — tier delegation
- `RevokeDelegation { delegator }` — removes delegation
- `RequestWithdrawal { pubkey, bond_count, bond_unit }` — FIFO withdrawal

**Producer weight constants** (`producer/constants.rs`):
- `ACTIVATION_DELAY = 10` blocks before eligible for scheduling
- `BOND_UNIT = 1_000_000_000` (10 DOLI, mainnet/testnet)
- `MAX_WEIGHT = 4`, `MIN_WEIGHT = 1` (seniority steps by year)
- `EXIT_HISTORY_RETENTION = 4_204_800` (~8 years)
- `INACTIVITY_THRESHOLD` (mainnet default: 60,480 blocks ≈ 7 days)

**ProducerSet persistence** (`producer/set_persistence.rs`):
- `load(path)`: JSON first → bincode fallback → fresh. Calls `rebuild_unbonding_index()` + `migrate_bond_entries()`
- `save(path)`: JSON, atomic via `.tmp` rename
- `serialize_canonical()`: sorted by Hash key; sorts `additional_bonds`, `received_delegations`, `bond_entries` within each entry

## FUNCTIONS-SNAPSHOT

**`snapshot.rs`**:

| Function | Signature | Notes |
|----------|-----------|-------|
| `compute_state_root` | `(cs, utxo, ps) → Result<Hash>` | `H(H(cs_canon) ‖ H(utxo_canon) ‖ H(ps_canon))` |
| `compute_state_root_with_epoch_state` | `(cs, utxo, ps, Option<Hash>) → Result<Hash>` | Phase-1: `None` = identical to legacy; `Some(h)` = 4-component hash |
| `compute_state_root_from_bytes` | `(cs_bytes, utxo_bytes, ps_bytes) → Result<Hash>` | for checkpoint verification |
| `compute_scheduler_root` | `(epoch_bond_snapshot, epoch, producer_list, ...)  → Hash` | scheduler state hash — NOT in block header, observational only |
| `StateSnapshot::create` | `(cs, utxo, ps) → Result<StateSnapshot>` | builds bytes + state root |

**State root log format** (searchable in prod):
```
[STATE_ROOT] cs={:.16} utxo={:.16} ps={:.16} cs_bytes=N utxo_bytes=N ps_bytes=N
[STATE_ROOT_HF] ... es={:.16} ...   (4-component, not yet wired)
```

## FUNCTIONS-ARCHIVER

**`archiver.rs`** — block archiver:

| Function | Notes |
|----------|-------|
| `BlockArchiver::new(rx, dir)` | async receiver + directory |
| `BlockArchiver::run()` | tokio task: receives blocks, calls `archive_block()` |
| `BlockArchiver::catch_up(dir, block_store, tip, genesis_hash)` | static: fills gaps 1..=tip from BlockStore |
| `restore_from_archive(dir, block_store, genesis_hash)` | imports all archive blocks |
| `backfill_from_archive(dir, block_store, genesis_hash)` | skips existing blocks |
| `force_backfill_from_archive(dir, block_store, genesis_hash)` | also replaces fork blocks (checksum comparison) |
| `manifest_height(dir)` | reads latest_height from manifest.json |
| `manifest_genesis_hash(dir)` | reads genesis_hash from manifest.json |

**Archive file format**:
```
{height:010}.block      — bincode-serialized Block
{height:010}.blake3     — BLAKE3 hex checksum (sidecar)
manifest.json           — { latest_height, latest_hash, genesis_hash }
```
All writes are atomic: write `.tmp` then `rename`.

## CONSTRAINTS

### Atomicity invariant
**One WriteBatch per block** — ALL of UTXO changes, producer changes, chain_state update, last_applied canary, and undo data are written in a single `RocksDB::write(batch)`. A crash leaves the DB either fully applied or not at all. (`state_db/batch.rs:280`)

### Canonical index invariant
`height_index` and `hash_to_height` in BlockStore are EXCLUSIVELY written by `set_canonical_chain()`. `put_block()` does NOT touch them. This prevents fork blocks from appearing in canonical chain queries. (`block_store/writes.rs:14-19`)

### Same-block-spend
`BlockBatch` maintains `pending_utxos: HashMap<Outpoint, UtxoEntry>` for outputs created in the current block but not yet committed. `spend_utxo()` checks this map before the DB so TX2 can spend TX1's output within the same block. (`state_db/batch.rs:67`)

### Deferred mutations (epoch boundary rule)
Producer state changes (Register, AddBond, Exit, Slash, Withdrawal, Delegation) are queued as `PendingProducerUpdate` and applied ONLY at epoch boundaries via `apply_pending_updates()`. This prevents scheduler divergence between forks. Exception: epoch 0 producers are applied immediately.

### atomic_replace CF_META exclusion
`atomic_replace()` does NOT delete `CF_META` — only deletes UTXO + producer CFs. This preserves scheduler meta-state (epoch_producer_list, attestation accumulators, etc.) across rollbacks and snap sync. (`state_db/writes.rs:149-165`)

### ChainState versioned format
Written with `0x01` byte prefix. `get_chain_state()` PANICS (not returns None) if the key exists but can't be deserialized — prevents silent state loss. (`state_db/queries.rs:207`)

### serialize_canonical() fields
`ChainState.serialize_canonical()` encodes ONLY 9 consensus fields (140 bytes fixed). `snap_sync_height`, `active_protocol_version`, `pending_protocol_activation` are NOT included — they are local bookkeeping. (`chain_state.rs:143`)

### Snap horizon floor
After snap sync, `set_canonical_chain()` stops walking backward when `height <= snap_horizon`. The snap anchor header was never persisted — walking into it crashes with "header missing". (`block_store/writes.rs:130-135`)

### UTXO count caching
`StateDb.utxo_count: AtomicU64` is set on `open()` by counting cf_utxo entries. Updated by `fetch_add/sub` in batch operations. Never scan for count — use `utxo_len()`. (`state_db/types.rs:62`, `state_db/open.rs:38-44`)

### Bond stamp invariant
Bond outputs get `extra_data = slot.to_le_bytes()` stamped by the NODE at apply time, not trusted from the TX. Applies in both `BlockBatch.add_transaction_utxos()` and `StateDb.add_transaction()`. (`state_db/batch.rs:135`, `state_db/writes.rs:372`)

### Total work = height
`ChainState.update()` sets `total_work = height` (assignment, not increment). This prevents divergence between nodes restarted at different heights. (`chain_state.rs:128-131`)

### epoch_state deletion danger
`delete_epoch_state()` forces a non-deterministic rebuild from local blocks on next startup. Snap-synced nodes have incomplete block history → different rebuild results → fork at epoch boundary. Only call when required by `EPOCH_STATE_FORMAT_VERSION` mismatch. (`state_db/queries.rs:402`)

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
// Track which producers changed in this block
let mut dirty_keys: HashSet<Hash> = HashSet::new();
let mut removed_keys: HashSet<Hash> = HashSet::new();
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
// Save in same WriteBatch as block:
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

### Pattern: same-block-spend
Works automatically via `pending_utxos` map in `BlockBatch`. No special handling needed by callers. TX ordering within a block must be topologically sorted before `apply_block()`.

### Pattern: address transaction history (paginated)
```rust
let heights = block_store.get_address_heights(
    &pubkey_hash,
    before_height,  // exclusive upper bound (None = from tip)
    limit,
);
// Key format: addr(32) ++ height(8 BE) → empty
// Iterated in REVERSE for descending order
```

### Pattern: snap sync apply
```rust
// 1. Verify state root
let computed = compute_state_root_from_bytes(&cs_bytes, &utxo_bytes, &ps_bytes)?;
assert_eq!(computed, snapshot.state_root);

// 2. Atomic replace
let cs: ChainState = bincode::deserialize(&cs_bytes)?;
let ps: ProducerSet = bincode::deserialize(&producer_bytes)?;
let utxo = UtxoSet::deserialize_canonical(&utxo_bytes)?;
state_db.atomic_replace(&cs, &ps, utxo.iter_all().into_iter())?;

// 3. Anchor the block store
block_store.seed_canonical_index(cs.best_hash, cs.best_height)?;
cs.mark_snap_synced(cs.best_height);
```

### Pattern: state root diagnosis
When state roots diverge, grep logs for `[STATE_ROOT]`:
```
[STATE_ROOT] cs={16 chars} utxo={16 chars} ps={16 chars} cs_bytes=140 utxo_bytes=N ps_bytes=N
```
Compare `cs`, `utxo`, `ps` hashes across nodes — the differing component identifies the divergent state.

## SERIALIZATION

### Format matrix

| Data | Storage format | Canonical format | Wire (snap sync) |
|------|---------------|-----------------|-----------------|
| `BlockHeader` | bincode | — | bincode |
| `Block` body | bincode `BlockBody` | — | bincode |
| `ChainState` (StateDb) | `0x01` prefix + bincode | 140-byte fixed encoding | bincode |
| `ChainState` (file) | bincode | 140-byte fixed encoding | bincode |
| `UtxoEntry` (StateDb) | bincode | 61+N bytes canonical | — |
| `UtxoSet` | bincode (InMemory) / RocksDB | sorted outpoints + canonical values | canonical |
| `ProducerInfo` | bincode (StateDb) | bincode (sorted Vec fields) | — |
| `ProducerSet` (file) | JSON first, bincode fallback | sorted by hash key | bincode |
| `UndoData` | bincode (cf_undo) | — | — |
| `MaintainerState` | bincode (file) | — | — |
| `UpdateState` | bincode (file) | — | — |
| `EpochState` | raw bytes (format-versioned) | — | — |
| Archive blocks | bincode `Block` | — | — |

### ChainState format versions
- `0x01` prefix (current): versioned bincode (`state_db/queries.rs:233`)
- Legacy unversioned: raw bincode (v1.0.29 and earlier)
- Legacy migration: raw bincode + appended `1u32 LE` + `0x00` byte (v1.0.28: missing `active_protocol_version` field)

### BlockBody versions (tried in order):
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

**Never change canonical encoding without a chain reset** — all nodes diverge immediately.
