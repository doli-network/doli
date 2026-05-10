# mempool — DOLI Transaction Mempool
<!-- @INDEX
ENTRY-POINTS: lines 18-30
STRUCTS: lines 32-62
FUNCTIONS: lines 64-130
DATA-FLOWS: lines 132-152
DEPENDENCIES: lines 154-163
CONSTRAINTS: lines 165-185
PATTERNS: lines 187-210
-->

## ENTRY-POINTS

Public API surface (re-exported from `crates/mempool/src/lib.rs:9-11`):

| Symbol | Module |
|--------|--------|
| `Mempool` | `pool.rs` |
| `MempoolError` | `pool.rs` |
| `MempoolEntry` | `entry.rs` |
| `MempoolPolicy` | `policy.rs` |

Constructor shortcuts (`pool.rs:187-202`):
- `Mempool::mainnet()` — default policy + mainnet `ConsensusParams` + `Network::Mainnet`
- `Mempool::testnet()` — permissive policy + testnet params (max_count=10_000, min_fee_rate=0)
- `Mempool::new(policy, params, network)` — explicit construction

## STRUCTS

### `MempoolEntry` (`entry.rs:11-32`)
```
tx: Transaction
tx_hash: Hash
fee: u64
fee_rate: u64           // fee / size (sat/byte)
size: usize
added_time: u64         // Unix seconds
ancestors: HashSet<Hash>
descendants: HashSet<Hash>
ancestor_fee: u64       // package fee (self + ancestors)
ancestor_size: usize    // package size (self + ancestors)
```

### `MempoolPolicy` (`policy.rs:5-20`)
| Field | Default | Testnet | Local |
|-------|---------|---------|-------|
| `max_count` | 5_000 | 10_000 | 1_000 |
| `max_size` | 10 MB | 10 MB | 1 MB |
| `min_fee_rate` | 0 | 0 | 0 |
| `max_tx_size` | 600 KB | 600 KB | 600 KB |
| `max_ancestors` | 25 | 25 | 25 |
| `max_descendants` | 25 | 25 | 25 |
| `max_age` | 14 days | 14 days | 14 days |

Note: `min_fee_rate=0` across all profiles — flat fee model; any `fee >= minimum_fee()` accepted.

### `Mempool` (`pool.rs:152-168`) — internal state
```
entries: HashMap<Hash, MempoolEntry>
by_fee_rate: BTreeSet<(u64, Hash)>       // eviction index (ascending = lowest first)
by_address: HashMap<Hash, HashSet<Hash>> // pubkey_hash -> tx hashes (outputs only)
spent_outputs: HashMap<Outpoint, Hash>   // double-spend guard
policy: MempoolPolicy
total_size: usize
params: ConsensusParams
network: Network
```

### `MempoolError` (`pool.rs:20-59`)
Error codes returned by `error_code()` and `to_structured_json()`:
| Variant | Code |
|---------|------|
| `AlreadyExists` | `TX_ALREADY_EXISTS` |
| `Full` | `MEMPOOL_FULL` |
| `InvalidTransaction(msg)` | `INVALID_TRANSACTION` or extracted `MPTX0XX` |
| `Validation(e)` | delegates to `ValidationError::error_code()` |
| `FeeTooLow(actual, min)` | `FEE_TOO_LOW` |
| `TooLarge(size, max)` | `TX_TOO_LARGE` |
| `TooManyAncestors(count, max)` | `TOO_MANY_ANCESTORS` |
| `TooManyDescendants(count, max)` | `TOO_MANY_DESCENDANTS` |
| `MissingInput(hash, index)` | `MISSING_INPUT` |
| `DoubleSpend{tx_hash,output_index,spending_tx}` | `DOUBLE_SPEND` |

## FUNCTIONS

### `MempoolEntry` methods (`entry.rs`)
| Method | Signature | Notes |
|--------|-----------|-------|
| `new` | `(tx, fee) -> Self` | stamps `added_time`, computes `fee_rate=fee/size` |
| `effective_fee_rate` | `() -> u64` | `ancestor_fee / ancestor_size` (CPFP package rate) |
| `add_ancestor` | `(hash, fee, size)` | idempotent via `HashSet::insert` |
| `add_descendant` | `(hash)` | |
| `remove_ancestor` | `(hash, fee, size)` | saturating sub on package totals |
| `remove_descendant` | `(hash)` | |
| `age` | `() -> u64` | seconds since `added_time` |

### `Mempool` public methods (`pool.rs`)

**Write path:**
| Method | Signature | Notes |
|--------|-----------|-------|
| `add_transaction` | `(&mut self, tx, utxo_set, current_height) -> Result<Hash, MempoolError>` | Full validation pipeline; see DATA-FLOWS |
| `add_system_transaction` | `(&mut self, tx, current_height) -> Result<Hash, MempoolError>` | Skips UTXO/fee checks; used for SlashProducer; fee_rate=0 |
| `remove_transaction` | `(&mut self, hash) -> Option<MempoolEntry>` | Cleans all indexes; updates ancestor/descendant links |
| `remove_for_block` | `(&mut self, txs: &[Transaction])` | Bulk remove after block commit |
| `remove_registration_txs` | `(&mut self)` | Purge all `TxType::Registration` — called on stale-reg block failure |
| `remove_by_error_pattern` | `(&mut self, err_msg: &str)` | Heuristic purge: NFT/CreatePool/Registration by error string |
| `expire_old` | `(&mut self)` | Removes entries where `age() > max_age` |
| `revalidate` | `(&mut self, utxo_set, current_height)` | Post-reorg: drop any tx whose inputs no longer exist in UTXO set |

**Read path:**
| Method | Signature | Notes |
|--------|-----------|-------|
| `get` | `(&self, hash) -> Option<&MempoolEntry>` | |
| `contains` | `(&self, hash) -> bool` | |
| `len` / `is_empty` / `size` | | count / bytes |
| `max_size` / `max_count` | | policy limits |
| `is_outpoint_spent` | `(&self, outpoint) -> bool` | double-spend check for external callers |
| `min_fee_rate` | `(&self) -> u64` | dynamic: returns `lowest_in_pool` when >90% full |
| `select_for_block` | `(&self, max_size: usize) -> Vec<Transaction>` | CPFP-aware ordering; ancestor prerequisite enforcement |
| `iter` | `(&self) -> impl Iterator<Item=(&Hash, &MempoolEntry)>` | |
| `get_by_address` | `(&self, pubkey_hash) -> Vec<&MempoolEntry>` | indexed by output `pubkey_hash` |
| `calculate_unconfirmed_balance` | `(&self, pubkey_hash, utxo_set) -> (u64, u64)` | returns (incoming, outgoing) |
| `get_unconfirmed_balance` | `(&self, pubkey_hash, utxo_set) -> i64` | `incoming - outgoing`; can be negative |

**Private helpers:**
| Method | Notes |
|--------|-------|
| `calculate_inputs` | Resolves inputs from mempool (parent-in-pool) then UTXO set; builds ancestor set |
| `needs_eviction` | `len >= max_count OR total_size + new_tx_size > max_size` |
| `evict_lowest_fee` | Evicts lowest `(fee_rate, hash)` entry with no descendants |

## DATA-FLOWS

### `add_transaction` validation pipeline (`pool.rs:205-431`)
```
1. Duplicate check — AlreadyExists if hash in entries
2. Size check — TooLarge if tx.size() > max_tx_size
3. Structure validation — validate_transaction(&tx, &ctx)
4. Input signature/pubkey validation (per input):
   a. Normal/Bond outputs at height >= sig_verification_height:
      MPTX001: missing public_key
      MPTX002: pubkey hash mismatch
      MPTX003: invalid signature
   b. Conditioned outputs (covenants):
      MPTX004-007: condition/witness validation
5. calculate_inputs() — resolves input amounts + ancestor set
   MPTX009: coinbase maturity not met (skip for RequestWithdrawal/Exit)
6. Balance check — MPTX008: total_input < total_output
7. Compute fee = total_input - total_output
8. Minimum fee check — FeeTooLow if fee < tx.minimum_fee()
9. Fee rate check — FeeTooLow if policy.min_fee_rate > 0 AND fee_rate < min_fee_rate
10. Ancestor limit — TooManyAncestors
11. Descendant limit — TooManyDescendants
12. Double-spend check — DoubleSpend if outpoint in spent_outputs
13. Eviction loop — evict_lowest_fee() until space; Full if can't
14. Insert: spent_outputs, by_address, by_fee_rate, entries
```

### `select_for_block` ordering (`pool.rs:600-634`)
```
1. Collect all entries, sort descending by effective_fee_rate() (CPFP)
2. For each: skip if selected, too large, or ancestor not yet selected
3. Add to selected set
```

### Post-reorg flow
```
revalidate(utxo_set, height):
  For each entry: check all tx.inputs exist in utxo_set
  Drop entries with missing inputs
```

## DEPENDENCIES

External crates (`pool.rs:1-13`):
- `crypto::Hash` — transaction and outpoint hashing
- `doli_core::consensus::ConsensusParams` — fee/maturity params
- `doli_core::network::Network` — network type + `NetworkParams`
- `doli_core::validation::{validate_transaction, ValidationContext, ValidationError}`
- `doli_core::{BlockHeight, Transaction, TxType, OutputType}` — domain types
- `doli_core::conditions::{Condition, Witness, EvalContext, evaluate, MAX_CONDITION_OPS}`
- `storage::{Outpoint, UtxoSet}` — UTXO lookups

Mempool has NO knowledge of block production, P2P, or storage — pure in-memory data structure.

## CONSTRAINTS

- `max_count=5000` (mainnet), `max_size=10MB`, `max_tx_size=600KB`
- `max_ancestors=25`, `max_descendants=25` — CPFP chain depth limits
- `min_fee_rate=0` everywhere — flat fee model, NOT fee-market
- Dynamic min_fee_rate at >90% capacity returns lowest-in-pool rate
- System transactions bypass ALL fee checks via `add_system_transaction`
- Eviction targets lowest `fee_rate` leaf (no descendants); `Full` if no leaf evictable
- Expiry: `expire_old()` removes entries with `age() > 14 days`
- Signature enforcement enabled at `sig_verification_height`
- `RequestWithdrawal` and `Exit` skip coinbase maturity check
- Double-spend detected at add time via `spent_outputs` HashMap

## PATTERNS

### CPFP (Child-Pays-For-Parent)
- `effective_fee_rate()` = `ancestor_fee / ancestor_size`
- High-fee child promotes low-fee mempool parent in `select_for_block`

### Toxic TX removal (incident-driven)
- `remove_registration_txs()` — purge stale registrations
- `remove_by_error_pattern(err_msg)` — heuristic matching on error string

### Reorg handling
```rust
mempool.remove_for_block(&orphaned_block_txs);
mempool.revalidate(&new_utxo_set, new_height);
```

### Structured errors for agents
`MempoolError::to_structured_json()` returns `error_code` field — match on code, never message string.

### Inline error codes
`InvalidTransaction(msg)` uses `[MPTX0XX]` prefix extracted by `error_code()`:
MPTX001-003 (signature), MPTX004-007 (covenant), MPTX008 (funds), MPTX009 (maturity)
