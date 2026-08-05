# mempool — DOLI Transaction Mempool
<!-- @INDEX
ENTRY-POINTS    11-99
OPERATIONS      101-114
DATA-FLOW       116-162
DEPENDENCIES    164-182
CONSTRAINTS     184-202
PATTERNS        204-217
@/INDEX -->

## ENTRY POINTS

Public API surface (re-exported `crates/mempool/src/lib.rs:11-14`):

| Symbol | Location | Description |
|--------|----------|-------------|
| `Mempool` | `pool.rs:158-196` | Core mempool struct |
| `MempoolError` | `pool.rs:26-65` | Error enum, 9 variants |
| `MempoolEntry` | `entry.rs:11-32` | Per-tx pool entry (CPFP-aware) |
| `MempoolPolicy` | `policy.rs:5-20` | Limits config (mainnet/testnet/local) |
| `AddTransactionResult` | `contention.rs:40-46` | `{tx_hash, diagnostic}` — return type of `add_transaction` |
| `ContentionInfo` | `contention.rs:24-30` | `{competing_count, pool_utxo_tx, pool_utxo_index}` |
| `MempoolDiagnostic` | `contention.rs:34-37` | `{contention: Option<ContentionInfo>}` |

Constructor shortcuts:
- `Mempool::mainnet()` — `pool.rs:243-249` — default policy + mainnet `ConsensusParams` + `Network::Mainnet`
- `Mempool::testnet()` — `pool.rs:252-258` — permissive policy + testnet params (`max_count=10_000`, `min_fee_rate=0`)
- `Mempool::new(policy, params, network)` — `pool.rs:200-214` — explicit construction

Node-wiring methods (bind mempool to Node's shared state at init; exact call site not located — `rg` binary was unavailable this session, grep `bins/node/src/node/` for `share_oracle_sunset_flag`/`share_active_producers_weighted` to confirm):
- `share_oracle_sunset_flag(&mut self, flag: Arc<AtomicBool>)` — `pool.rs:220-225` — binds mempool's M8 oracle-sunset check to the Node's `Arc<AtomicBool>`
- `share_active_producers_weighted(&mut self, snapshot: Arc<RwLock<Vec<(PublicKey,u64)>>>)` — `pool.rs:235-240` — binds AUDIT-P1-001 active-producer snapshot (fixes universal PriceAttestation rejection)

Write path (`pool.rs`):

| Method | Signature | Location |
|--------|-----------|----------|
| `add_transaction` | `(&mut self, tx: Transaction, utxo_set: &UtxoSet, current_height: BlockHeight) -> Result<AddTransactionResult, MempoolError>` | `pool.rs:261-627` |
| `add_system_transaction` | `(&mut self, tx: Transaction, current_height: BlockHeight) -> Result<Hash, MempoolError>` | `pool.rs:633-707` |
| `remove_transaction` | `(&mut self, tx_hash: &Hash) -> Option<MempoolEntry>` | `pool.rs:710-764` |
| `remove_for_block` | `(&mut self, transactions: &[Transaction])` | `pool.rs:767-771` |
| `remove_registration_txs` | `(&mut self)` | `pool.rs:885-895` |
| `remove_by_error_pattern` | `(&mut self, err_msg: &str)` | `pool.rs:901-929` |
| `expire_old` | `(&mut self)` | `pool.rs:1070-1082` |
| `revalidate` | `(&mut self, utxo_set: &UtxoSet, current_height: BlockHeight)` | `pool.rs:1086-1112` |

Read path (`pool.rs`):

| Method | Signature | Location |
|--------|-----------|----------|
| `get` | `(&self, tx_hash: &Hash) -> Option<&MempoolEntry>` | `pool.rs:774-776` |
| `contains` | `(&self, tx_hash: &Hash) -> bool` | `pool.rs:779-781` |
| `len`/`is_empty`/`size` | count/count/bytes | `pool.rs:784-796` |
| `max_size`/`max_count` | policy limits | `pool.rs:799-806` |
| `is_outpoint_spent` | `(&self, outpoint: &Outpoint) -> bool` | `pool.rs:809-811` |
| `pool_contention_count` | `(&self, outpoint: &Outpoint) -> usize` | `pool.rs:816-820` — pending AMM txs contending for this Pool UTXO |
| `pool_contention_index_len` | `(&self) -> usize` | `pool.rs:825-827` — distinct contested Pool outpoints |
| `min_fee_rate` | `(&self) -> u64` | `pool.rs:830-839` — dynamic: returns lowest-in-pool rate when >90% full |
| `select_for_block` | `(&self, max_size: usize) -> Vec<Transaction>` | `pool.rs:845-879` — CPFP-aware ordering |
| `iter` | `(&self) -> impl Iterator<Item=(&Hash,&MempoolEntry)>` | `pool.rs:932-934` |
| `get_by_address` | `(&self, pubkey_hash: &Hash) -> Vec<&MempoolEntry>` | `pool.rs:937-942` |
| `calculate_unconfirmed_balance` | `(&self, pubkey_hash, utxo_set) -> (u64,u64)` | `pool.rs:951-978` — (incoming, outgoing) |
| `get_unconfirmed_balance` | `(&self, pubkey_hash, utxo_set) -> i64` | `pool.rs:985-988` — `incoming - outgoing`, can be negative |

`MempoolEntry` methods (`entry.rs`):

| Method | Signature | Notes |
|--------|-----------|-------|
| `new` | `(tx, fee) -> Self` | `entry.rs:36-58` stamps `added_time`, computes `fee_rate=fee/size` |
| `effective_fee_rate` | `() -> u64` | `entry.rs:64-70` `ancestor_fee/ancestor_size` (CPFP package rate) |
| `add_ancestor` / `remove_ancestor` | `(hash, fee, size)` | `entry.rs:73-78`, `entry.rs:86-91` idempotent via `HashSet::insert`/`remove` |
| `add_descendant` / `remove_descendant` | `(hash)` | `entry.rs:81-83`, `entry.rs:94-96` |
| `age` | `() -> u64` | `entry.rs:99-105` seconds since `added_time` |

`MempoolError` variants + `error_code()` (`pool.rs:26-155`):

| Variant | Code |
|---------|------|
| `AlreadyExists` | `TX_ALREADY_EXISTS` |
| `Full` | `MEMPOOL_FULL` |
| `InvalidTransaction(msg)` | `INVALID_TRANSACTION` or extracted `[MPTX0XX]` |
| `Validation(e)` | delegates to `ValidationError::error_code()` |
| `FeeTooLow(actual,min)` | `FEE_TOO_LOW` |
| `TooLarge(size,max)` | `TX_TOO_LARGE` |
| `TooManyAncestors(count,max)` | `TOO_MANY_ANCESTORS` |
| `TooManyDescendants(count,max)` | `TOO_MANY_DESCENDANTS` |
| `MissingInput(hash,index)` | `MISSING_INPUT` |
| `DoubleSpend{tx_hash,output_index,spending_tx}` | `DOUBLE_SPEND` |

`to_structured_json()` (`pool.rs:96-154`) — merges variant-specific fields into a JSON object with `error_code`/`message`/`stage`; agents MUST match on `error_code`, never the message string.

Contention helpers (`contention.rs`, new since 2026-05-11 skill):

| Function | Signature | Location |
|----------|-----------|----------|
| `is_pool_contention_type` | `(tx_type: TxType) -> bool` | `contention.rs:49-56` — true for Swap/AddLiquidity/RemoveLiquidity |
| `find_pool_input` | `(tx: &Transaction, utxo_set: &UtxoSet) -> Option<Outpoint>` | `contention.rs:62-75` — first input whose UTXO is `OutputType::Pool` |

RPC entry point consuming `add_transaction`/`add_system_transaction`: `send_transaction` — `crates/rpc/src/methods/transaction.rs:165-237`. Reads mempool via `getTransaction` — `crates/rpc/src/methods/transaction.rs:16-76`.

## OPERATIONS

| Task | Steps | Commands/Functions | Inputs | Success |
|------|-------|--------------------|--------|---------|
| Submit a transaction | 1. RPC decodes hex tx 2. If `tx.is_state_only()` call `add_system_transaction` else `add_transaction` 3. Broadcast to network 4. Return hash + optional warnings | `sendTransaction` RPC → `Mempool::add_transaction`/`add_system_transaction` | Signed `Transaction`, current UTXO set, current height | `Ok(AddTransactionResult{tx_hash,..})`; response includes `hash` and optional `warnings[].type=POOL_CONTENTION` — `crates/rpc/src/methods/transaction.rs:196-236` |
| Select transactions for a block | 1. `select_for_block(max_size)` sorts by `effective_fee_rate()` descending 2. Skip if already selected, too large, or an ancestor not yet selected 3. Stop when `selected_size >= max_size` | `Mempool::select_for_block` | `max_size` budget (gated by `large_block_activation_height` — `LARGE_BLOCK_SELECT_BUDGET` vs `LEGACY_BLOCK_SELECT_BUDGET`) | `Vec<Transaction>` respecting size cap + ancestor ordering — `pool.rs:845-879`, called from `bins/node/src/node/production/assembly.rs:176-179` |
| CPFP: promote a stuck low-fee parent | 1. Submit high-fee child spending the parent's output while parent still unconfirmed 2. `calculate_inputs` records parent as ancestor, `add_ancestor` sums `ancestor_fee`/`ancestor_size` onto the child's entry 3. `select_for_block` sorts by `effective_fee_rate()` (package rate), pulling parent along once child is chosen | `Mempool::add_transaction` (child) → `MempoolEntry::add_ancestor`/`effective_fee_rate` | Child tx spending an in-mempool parent's output, child fee high enough that package rate beats other candidates | Parent+child both included in the built block; verified by `test_cpfp_effective_fee_rate` — `pool.rs:1335-1381` |
| Detect Pool-UTXO contention before submitting an AMM tx | 1. Tx type ∈ {Swap, AddLiquidity, RemoveLiquidity} 2. `find_pool_input` locates the Pool-type input outpoint 3. Look up `pool_utxo_contention[outpoint]` for existing pending txs 4. If found, non-fatal diagnostic returned to submitter; if the SAME outpoint is already spent by another pending tx, admission still fails with `DoubleSpend` (no silent replacement) | `Mempool::add_transaction` internal (`pool.rs:532-549`) → `ContentionInfo` | AMM tx referencing a Pool UTXO also targeted by another pending tx | `AddTransactionResult.diagnostic.contention = Some(ContentionInfo{competing_count,..})` on first-in acceptance; `Err(DoubleSpend)` on the actual conflicting spend attempt |
| Purge toxic mempool TXs after a failed self-produced block | 1. `apply_block` fails on a self-produced block (e.g. stale Registration, NFT/Pool ID collision) 2. `rollback_one_block()` 3. Iterate `block.transactions`, call `remove_transaction` for each | `Node::try_produce_block` (`bins/node/src/node/production/mod.rs:589-638`) → `Mempool::remove_transaction` | Failed block's tx list | Mempool purged of all TXs from the poisoned block; next production tick retries clean — see testnet incident 2026-03-25 |
| Purge specific toxic tx classes by error heuristic | 1. Error string matched against `token_id`/`NFT`, `ool` (Pool), `egistration` substrings 2. Matching entries removed | `Mempool::remove_by_error_pattern(err_msg)` | Error message string from a failed apply/validate | All matching NFT/CreatePool/Registration TXs removed — `pool.rs:901-929` |
| Clear stale Registration TXs | 1. Filter entries where `tx_type == Registration` 2. Remove all | `Mempool::remove_registration_txs()` | none | All Registration-type mempool entries removed — `pool.rs:885-895`; prevents infinite retry loops on stale/duplicate registrations |
| Revalidate mempool after reorg | 1. For each entry, check every input outpoint still exists in the new canonical `UtxoSet` 2. Remove entries with any missing input | `Mempool::revalidate(utxo_set, current_height)` | Post-reorg `UtxoSet` | Entries spending now-invalid inputs removed — `pool.rs:1086-1112` |
| Expire aged-out transactions | 1. Periodic task computes `age() > policy.max_age` (14 days) 2. Remove matching entries | `Mempool::expire_old()` called from `bins/node/src/node/periodic.rs:451` | none | Entries older than 14 days removed |
| Query unconfirmed balance for an address | 1. `get_by_address` sums outputs to `pubkey_hash` as incoming 2. Scan `spent_outputs` for UTXOs owned by `pubkey_hash` as outgoing | `Mempool::calculate_unconfirmed_balance` / `get_unconfirmed_balance` | `pubkey_hash`, current `UtxoSet` | `(incoming, outgoing)` tuple or signed net `i64` — `pool.rs:951-988` |

## DATA FLOW

| Input | Transform | Output | Location |
|-------|-----------|--------|----------|
| Raw hex tx (RPC) | Deserialize → route by `tx.is_state_only()` | `Transaction` fed to `add_transaction`/`add_system_transaction` | `crates/rpc/src/methods/transaction.rs:169-214` |
| `Transaction` + `UtxoSet` + height | 14-step `add_transaction` pipeline (below) | `Ok(AddTransactionResult)` or typed `MempoolError` | `pool.rs:261-627` |
| Mempool entries (BTreeMap by fee) | `select_for_block(budget)` — CPFP sort + ancestor-gated greedy pack | `Vec<Transaction>` for `BlockBuilder` | `pool.rs:845-879` → `bins/node/src/node/production/assembly.rs:176-179` |
| Reorg's new canonical `UtxoSet` | `revalidate()` drops entries whose inputs vanished | Pruned `entries`/`spent_outputs`/`pool_utxo_contention` | `pool.rs:1086-1112` |
| Applied/failed block's tx list | `remove_for_block`/`remove_transaction` loop | Cleared `entries`, `by_fee_rate`, `by_address`, `spent_outputs`, `pool_utxo_contention`, ancestor/descendant links | `pool.rs:767-771`, `710-764` |

`add_transaction` validation pipeline (`pool.rs:261-627`):
```
1.  Duplicate check — AlreadyExists if hash in entries
2.  Size check — TooLarge if tx.size() > max_tx_size
3.  Build ValidationContext (ALL activation heights + oracle_sunset_triggered +
    active_producers_weighted snapshot) → validate_transaction(&tx, &ctx)
4.  Per-input signature/covenant validation (skipped for AMM Pool-input INC-I-092
    exemption — see CONSTRAINTS):
    a. Normal/Bond at height >= sig_verification_height:
       MPTX001 missing public_key, MPTX002 pubkey hash mismatch, MPTX003 bad signature
    b. Conditioned (covenant) outputs: MPTX004-007 condition/witness validation
5.  calculate_inputs() — resolves input amounts + ancestor set (mempool-parent-aware)
    MPTX009: coinbase maturity not met (skipped for RequestWithdrawal/Exit)
6.  Fee computation branches on AMM gate (inc_i_096_activation_height):
    - AMM-gated Swap/AddLiquidity/RemoveLiquidity: resolve consumed outputs,
      delegate to shared verify_amm_conservation() (same fn as consensus),
      fee = result.doli_surplus. MPTX008 on InsufficientFunds.
    - Otherwise: naive fee = total_input - total_output; MPTX008 if input < output.
7.  Min-fee / min-fee-rate floor checks — SKIPPED entirely for AMM-gated txs
    (fee_exempt, mirrors consensus validation/utxo.rs)
8.  Ancestor limit — TooManyAncestors
9.  Descendant limit — TooManyDescendants (checked against each ancestor's set)
10. Pool-UTXO contention lookup (pre-simulation diagnostic, BEFORE double-spend
    check) — is_pool_contention_type + find_pool_input + pool_utxo_contention index
11. Double-spend check — DoubleSpend if outpoint already in spent_outputs
    (no RBF: same tx_hash re-submission is a no-op via step 1, NOT a replacement)
12. Eviction loop — evict_lowest_fee() until space; Full if no evictable leaf
13. Insert: spent_outputs, by_address, ancestor/descendant links,
    pool_utxo_contention, by_fee_rate, entries, total_size
14. Return AddTransactionResult{tx_hash, diagnostic:{contention}}
```

`select_for_block` ordering (`pool.rs:845-879`):
```
1. Collect all entries, sort descending by effective_fee_rate() (CPFP)
2. For each: skip if selected, too large for remaining budget, or an ancestor not yet selected
3. Add to selected set; stop when selected_size >= max_size
```

## DEPENDENCIES

| This Domain Uses | Skill File | What For |
|-------------------|-----------|----------|
| `crypto::Hash`, `crypto::signature::verify_hash`, `crypto::hash::hash_with_domain` | (crypto crate, no skill file found) | Tx/outpoint hashing, per-input signature verification |
| `doli_core::validation::{validate_transaction, validate_transaction_with_utxos, verify_amm_conservation, ValidationContext, ValidationError}` | none found for `crates/core` domain | Structure validation, AMM conservation (shared with consensus), activation-height-gated context |
| `doli_core::consensus::ConsensusParams`, `doli_core::network::Network`/`NetworkParams` | none found | Fee params, activation heights (`sig_verification_height`, `amm_activation_height`, `inc_i_092_activation_height`, `inc_i_096_activation_height`, `oracle_activation_height`, `defi_activation_height`, etc.) |
| `doli_core::conditions::{Condition, Witness, EvalContext, evaluate, MAX_CONDITION_OPS}` | none found | Covenant condition/witness evaluation for conditioned outputs (MPTX004-007) |
| `storage::{Outpoint, UtxoSet, UtxoEntry}` | none found | UTXO lookups for input resolution, maturity checks, Pool-type detection |

| Used By | Skill File | What For |
|---------|-----------|----------|
| `bins/node/src/node/production/assembly.rs:176-179,235` | none found (node/production not a separate skill) | `mempool.select_for_block(select_budget)` to fill block body; `oracle_sunset_triggered` also read directly by the Node in the same builder context |
| `bins/node/src/node/production/mod.rs:619-623` | none found | `mempool.remove_transaction(&tx.hash())` bulk purge after a self-produced block fails `apply_block` (block-poison recovery) |
| `bins/node/src/node/periodic.rs:451` | none found | `mempool.expire_old()` every periodic tick |
| `crates/rpc/src/methods/transaction.rs:16-76,165-263` | `.claude/skills/doli-network/SKILL.md` (RPC domain) | `getTransaction` reads `mempool.get()`; `sendTransaction` calls `add_transaction`/`add_system_transaction`, converts `MempoolError` → structured `RpcError` |
| Node init (exact site not confirmed — `rg` unavailable) | none found | `share_oracle_sunset_flag`, `share_active_producers_weighted` — wire mempool's `ValidationContext` construction to live Node state |

Mempool has NO knowledge of block production internals, P2P, or storage persistence — pure in-memory data structure operated on by the Node and RPC layers.

## CONSTRAINTS

| Constraint | Type | Location | Detail |
|-----------|------|----------|--------|
| No RBF (Replace-By-Fee) | invariant | `pool.rs:551-563` | Double-spend of an outpoint already in `spent_outputs` by a DIFFERENT tx hash is unconditionally rejected with `DoubleSpend` — there is no fee-comparison replacement path. Confirmed per `project_no_rbf_by_design.md`. Re-submitting the SAME tx hash is caught earlier as `AlreadyExists` (step 1), also not a replacement. |
| `max_count=5000` / `max_size=10MB` (mainnet) | invariant | `policy.rs:22-34` | Testnet: `max_count=10_000`. Local: `max_count=1_000`, `max_size=1MB`. |
| `max_tx_size=600KB` | invariant | `policy.rs:28` | Covers 512KB NFT content + inputs/outputs overhead |
| `max_ancestors=25` / `max_descendants=25` | invariant | `policy.rs:29-30` | CPFP chain depth limits, checked at `pool.rs:512-530` |
| `min_fee_rate=0` on ALL policy profiles | invariant | `policy.rs:26-27,44-58` | Flat fee model, NOT a fee market — floors are `minimum_fee()` (base + per-byte), not per-byte-rate driven |
| Dynamic min_fee_rate at >90% capacity | performance | `pool.rs:830-839` | Returns `max(lowest_in_pool_rate, policy.min_fee_rate)` once `len() > max_count*9/10` |
| AMM txs exempt from min-fee/min-fee-rate floors | edge-case | `pool.rs:494-510` | Post `inc_i_096_activation_height`, Swap/AddLiquidity/RemoveLiquidity fee = `doli_surplus` from `verify_amm_conservation`; conservation itself is the spam guard, so per-byte floor is skipped (mirrors consensus `validation/utxo.rs` `fee_exempt`) |
| AMM Pool-input signature exemption (INC-I-092 RC-A) | security | `pool.rs:322-343` | Input 0 of a Swap/AddLiquidity/RemoveLiquidity consuming an `OutputType::Pool` UTXO skips the pubkey/signature check — authorized instead by the x·y=k invariant enforced later in `verify_amm_conservation`. Gated by `inc_i_092_activation_height`; MUST mirror consensus's carve-out in `validation/utxo.rs` or admission/block-validation disagree. |
| `system_transaction` bypasses ALL UTXO/fee checks | security | `pool.rs:633-707` | Used for `SlashProducer` and other state-only txs; entered with `fee=0`, lowest eviction priority. Same `ValidationContext` (activation heights + producer snapshot) still applied via `validate_transaction`. |
| Eviction targets lowest `fee_rate` leaf (no descendants) | invariant | `pool.rs:1048-1067` | `Full` returned if no leaf entry is evictable (all remaining have descendants) |
| Expiry: `expire_old()` at `age() > 14 days` | invariant | `policy.rs:31`, `pool.rs:1070-1082` | Same across all policy profiles |
| `RequestWithdrawal`/`Exit` skip both coinbase-maturity AND lock checks | edge-case | `pool.rs:1016-1020` | They unlock Bond UTXOs; mirrors `validation.rs:2527` exemption |
| Pool-UTXO contention diagnostic is read-only, non-mutating of selection | invariant | `contention.rs:8-12` | Does not change which TXs the producer includes; no competing tx hashes leaked to the submitter (MEV-safety design constraint, AC-12 false-positive rate <=0.1%) |
| Contention check runs BEFORE the double-spend check | invariant | `pool.rs:532-549` vs `551-563` | So a rejected double-spend on a Pool UTXO can still carry contention context in logs, even though the caller only sees `DoubleSpend` |
| `active_producers_weighted` snapshot may be empty pre-oracle-activation | edge-case | `pool.rs:284-290` | Harmless because `oracle_activation_height = u64::MAX` rejects `PriceAttestation` at the height gate first (Phase 2.1, frozen per CLAUDE.md) |

## PATTERNS

| Pattern | Example Location | Usage |
|---------|-------------------|-------|
| CPFP (Child-Pays-For-Parent) | `entry.rs:64-70`, `pool.rs:845-879` | `effective_fee_rate() = ancestor_fee/ancestor_size`; high-fee child promotes a low-fee mempool parent in `select_for_block`. Verified: `pool.rs:1334-1381` `test_cpfp_effective_fee_rate` |
| Toxic TX removal (incident-driven) | `pool.rs:885-929` | `remove_registration_txs()` purges stale registrations; `remove_by_error_pattern(err_msg)` heuristically matches NFT/Pool/Registration substrings in an apply-time error string — born from testnet incident 2026-03-25 |
| Block-poison recovery loop | `bins/node/src/node/production/mod.rs:584-638` | On self-produced-block `apply_block` failure: rollback → purge every tx in the failed block from mempool via `remove_transaction` → return `Ok(())` so next tick retries clean |
| Reorg handling | `pool.rs:1086-1112` | `mempool.remove_for_block(&orphaned_block_txs); mempool.revalidate(&new_utxo_set, new_height);` |
| Structured errors for agents | `pool.rs:67-155` | `MempoolError::to_structured_json()` returns `error_code` field — match on code, never the message string; `MempoolError::error_code()` extracts `[MPTX0XX]` prefixes from `InvalidTransaction` |
| Inline error codes | `pool.rs:73-81` | `InvalidTransaction(msg)` uses `[MPTX0XX]` prefix parsed by `error_code()`: MPTX001-003 (signature), MPTX004-007 (covenant), MPTX008 (funds/AMM conservation), MPTX009 (maturity) |
| Consensus-parity delegation for AMM math | `pool.rs:436-491` | Mempool calls the SAME `verify_amm_conservation()` function consensus uses in `apply_block`, rather than reimplementing pool-aware fee math — guarantees a tx admitted by mempool is never later rejected by consensus for conservation reasons (INC-I-096 M3 parity) |
| Pool-UTXO contention index, keyed by outpoint | `pool.rs:167-170`, `604-609`, `732-744` | `HashMap<Outpoint, HashSet<Hash>>` tracks which pending AMM txs reference each Pool UTXO; populated on insert, cleaned on `remove_transaction`, queried via `pool_contention_count`/`pool_contention_index_len` |
| Shared-state wiring via `Arc<AtomicBool>`/`Arc<RwLock<..>>` | `pool.rs:179-195, 220-240` | Node owns the canonical `oracle_sunset_triggered` flag and `active_producers_weighted` snapshot; mempool holds a clone of the `Arc` and reads it fresh on every `add_transaction`/`add_system_transaction` call rather than caching a stale copy |
| Output Contract test scaffolding | `contention_tests.rs:1-32` | Header comment enumerates Observable outputs (O1-O4), Code paths (P1-P5), Input partitions (IP1-IP6), and a full IPxP matrix before any test body — pattern to replicate for new contention-adjacent test files |
