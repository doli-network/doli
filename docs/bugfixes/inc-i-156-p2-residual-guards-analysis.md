# INC-I-156 — P2 Residual Guards (R1 `UtxoSet::clear()` no-op, R2 unguarded ProducerSet rebuild)

- **Incident**: INC-I-156 (carried open from INC-I-152 close)
- **Workflow**: `/omega-doctor`, RUN_ID 493
- **Branch**: `bugfix/inc-i-156-p2-residual-guards` (worktree, base `main` f4e6ea69)
- **Agent**: Analyst
- **Status**: analysis only — no source or test code written

---

## Anchor Detection (deterministic-reasoning protocol)

**FIRST READ** of the brief: R2 = "a holed store *silently produces a wrong ProducerSet*". Setting aside.

**SECOND (contradicting) INTERPRETATION**, generated from the code: R2 = "a holed store *destroys* the
in-memory ProducerSet and then errors out mid-rebuild; nothing restores it; the next applied block
persists the destroyed set."

**Chosen**: the second, on evidence — `rewards.rs:1110` calls `producers.clear()` *before* the loop, and
`rewards.rs:1119` `.ok_or_else(...)?` aborts at the first hole. This is the same class of correction
that INC-I-152 entry 1557 had to make to the audit's framing of P1-003, so it is worth stating loudly.
Details and consequence in §2 (R2).

⚠ **SCOPE CORRECTION (code is SoT)**: the brief and the INC-I-152 notes place
`rebuild_producer_set_from_blocks` in `bins/node/src/node/fork_recovery.rs`. It is **not there**.
It is defined at **`bins/node/src/node/rewards.rs:1105`**. `fork_recovery.rs` contains no rebuild
function at all. All line references below are re-verified against this branch.

---

## 1. Architecture Context

### 1.1 Module boundaries

| Module | Responsibility | Depends on | Depended on by |
|---|---|---|---|
| `crates/storage::utxo::set::UtxoSet` (`crates/storage/src/utxo/set.rs`) | Enum-dispatch UTXO façade over two backends: `InMemory(InMemoryUtxoStore)` and `RocksDb(Arc<StateDb>)` | `InMemoryUtxoStore`, `StateDb` | every node path that reads/writes UTXOs: `apply_block`, `rollback`, `block_handling`, `init`, `fork_recovery`, RPC |
| `crates/storage::state_db::StateDb` (`writes.rs`, `batch.rs`, `undo.rs`) | Sole durable UTXO store since Phase 4 (`cf_utxo`, `cf_utxo_by_pubkey`), producers, meta, undo | RocksDB | `UtxoSet::RocksDb`, `apply_block`, rollback persistence |
| `crates/storage::block_store` (`queries.rs`) | Canonical block bodies + `height_index`; `ensure_blocks_present(low, high)` density oracle | RocksDB | rollback, reorg, periodic gap scanner |
| `bins/node/src/node/rollback.rs` | `rollback_one_block()` — single-block fork recovery | `state_db` (undo), `block_store`, `utxo_set`, `producer_set`, `rewards::rebuild_*` | `periodic.rs:718` (RecoveryCoordinator `ShallowRollback`), `production/mod.rs:609` (block-poison) |
| `bins/node/src/node/block_handling.rs` | `execute_reorg()` — multi-block fork switch | same set | `handle_new_block` |
| `bins/node/src/node/rewards.rs` | `rebuild_producer_set_from_blocks()` (**:1105**), `rebuild_epoch_state_from_blocks()`, `rebuild_producer_liveness()` | `block_store`, `config.network`, `params` | `rollback.rs` ×2, `block_handling.rs` ×2 |
| `bins/node/src/node/init.rs` | `Node::new()` — startup state load, genesis reset, undo-gap recovery | all of the above | startup only |
| `bins/node/src/node/fork_recovery.rs` | snap-snapshot install; **:363** converts the post-snap `UtxoSet` back to the `RocksDb` variant (INV-SYNC-014 / INC-I-118) | `state_db` | snap sync completion |

### 1.2 Which `UtxoSet` variant is live per node class

| Node class | Variant | Evidence |
|---|---|---|
| Normal production node (any network) | **`RocksDb`** | `bins/node/src/node/init.rs:311` — `let utxo_set = UtxoSet::from_state_db(state_db.clone());` on the single production constructor path |
| Node mid-snap-install | `InMemory` transiently — the snapshot deserializes into `InMemory` at `fork_recovery.rs:309` | `fork_recovery.rs:309` |
| Node after snap-install completes | **`RocksDb`** (converted back) | `fork_recovery.rs:363` — `*utxo = storage::UtxoSet::from_state_db(self.state_db.clone());` (INV-SYNC-014, INC-I-118) |
| Node immediately after a genesis reset | `InMemory` | `init.rs:371` — `*utxo_set.write().await = UtxoSet::new();` |
| Unit/integration tests via `new_for_test` | `InMemory` | `init.rs:1129`, `init.rs:1338` — `Arc::new(RwLock::new(UtxoSet::new()))` |

**Conclusion**: every node that can reach a rollback or a reorg in production holds the **`RocksDb`**
variant. The one exception (post-genesis-reset `InMemory`) is a node at height 0 that cannot roll back.
This is also why the two hazards were invisible to the existing test suite — `new_for_test` builds the
`InMemory` variant, on which `clear()` is honest.

### 1.3 Every `UtxoSet::clear()` call site, workspace-wide

Enumerated from a scan of `crates/` and of `bins/` (two roots, scanned together; per-file evidence cited below).

| # | Site | Class | Variant reachable there | Verdict |
|---|---|---|---|---|
| 1 | `crates/storage/src/utxo/set.rs:70` | the `InMemory` arm of the definition itself | — | correct |
| 2 | `crates/storage/src/utxo/set.rs:71-76` | the `RocksDb` arm — **empty block, explicit no-op** | RocksDb | **the defect (R1)** |
| 3 | `bins/node/src/node/init.rs:112` | **startup undo-gap recovery**, *not* genesis reset | InMemory only — fenced by `if !utxo_set.is_rocksdb()` at `init.rs:111` (INC-I-136) | correct today; the fence is the workaround that proves the semantics were already known |
| 4 | `bins/node/src/node/rollback.rs:191` | **legacy no-undo rollback rebuild** | RocksDb | **leaking (R1, primary)** |
| 5 | `bins/node/src/node/block_handling.rs:803` | **`execute_reorg` legacy no-undo rebuild** | RocksDb | **leaking (R1, second site — not in the brief)** |
| 6 | `bins/node/tests/recover_replay.rs:193, 256, 293, 355` | test | InMemory | n/a |
| 7 | `bins/node/tests/inc_i_064_supply_conservation.rs:509` | test | InMemory | n/a |

Genesis reset does **not** appear in this list: `init.rs:368` uses
`state_db.clear_and_write_genesis(&chain_state)` and then *replaces* the whole set with
`UtxoSet::new()` at `init.rs:371`. So the comment at `set.rs:73-75` — *"UtxoSet.clear() on the RocksDb
variant is only called during genesis reset (init.rs), which immediately replaces the UtxoSet with a
fresh InMemory variant anyway"* — is false in **all three** of its clauses: (a) it is called from
rollback and reorg, not only init; (b) the init call site is undo-gap recovery, not genesis reset;
(c) neither rollback nor reorg replaces the variant afterwards.

### 1.4 Every caller of `rebuild_producer_set_from_blocks`

| # | Call site | Context | Dense pre-check upstream? |
|---|---|---|---|
| 1 | `bins/node/src/node/rollback.rs:138` | undo path, `producer_snapshot` **deserialize failure** | **NO — this is R2, the only unguarded site** |
| 2 | `bins/node/src/node/rollback.rs:228` | legacy no-undo rebuild | YES — `ensure_blocks_present(1, target_height.max(1))` at `rollback.rs:169-181` (INC-I-152) |
| 3 | `bins/node/src/node/block_handling.rs:717` | reorg undo path, snapshot deserialize failure | YES — `ensure_blocks_present(1, target_height)` at `block_handling.rs:599-615` |
| 4 | `bins/node/src/node/block_handling.rs:835` | reorg legacy rebuild | YES — same guard at `block_handling.rs:599-615` |

### 1.5 Data flows through the two affected paths

**Path A — `rollback_one_block()` legacy no-undo rebuild (R1 primary)**

```
periodic.rs:718 (RecoveryAction::ShallowRollback)  ─┐
production/mod.rs:609 (block-poison after self-produced apply failure) ─┴─► rollback_one_block()
  └─ state_db.get_undo(local_height)  ── None ──► LEGACY BRANCH (rollback.rs:141-229)
       ├─ ensure_blocks_present(1, target.max(1))        [INC-I-152 guard — refuses holed store]
       ├─ utxo_set.write().await                         [write lock]
       ├─ utxo.clear()                                   ← rollback.rs:191  NO-OP on RocksDb
       ├─ for h in 1..=target: spend_transaction + add_transaction
       │     add_transaction on RocksDb == direct StateDb::insert_utxo (writes.rs:27) — NOT batched
       ├─ producer_set.write().await ─► rebuild_producer_set_from_blocks(target)   [rollback.rs:228]
       ├─ chain_state.best_height = target                [rollback.rs:240-245]
       └─ state_db.atomic_replace(cs, ps, utxo.iter_all())  [rollback.rs:276-284]
            on RocksDb, iter_all() re-reads cf_utxo — atomic_replace rewrites the SAME (polluted) keys.
            It cannot heal the leak; it launders it into durable storage.
```

**Path A′ — `execute_reorg()` legacy no-undo rebuild (R1 second site)** — identical shape at
`block_handling.rs:781-836`, guarded by the dense check at `:599`, `utxo.clear()` at `:803`, replay
loop `:804-830`, producer rebuild `:835`.

**Path B — undo-based rollback with a corrupt producer snapshot (R2)**

```
rollback_one_block()
  └─ get_undo(local_height) == Some(undo)
       ├─ utxo_set.write().await
       │    ├─ remove(created_utxos)   ← on RocksDb: direct StateDb::remove_utxo (writes.rs:51), DURABLE NOW
       │    └─ insert(spent_utxos)     ← on RocksDb: direct StateDb::insert_utxo (writes.rs:27), DURABLE NOW
       ├─ bincode::deserialize::<ProducerSet>(&undo.producer_snapshot)  ── Err ──►
       │    producer_set.write().await                                    [rollback.rs:137]
       │    rebuild_producer_set_from_blocks(&mut producers, target)?     [rollback.rs:138] ← NO DENSE CHECK
       │       ├─ producers.clear()                                       [rewards.rs:1110] ← DESTRUCTIVE, FIRST
       │       └─ for h in 1..=target { get_block_by_height(h)?.ok_or_else(..)? }  [rewards.rs:1115-1124]
       │            first hole ⇒ Err  ⇒  `?` unwinds out of rollback_one_block
       └─ (never reached) chain_state rewind, atomic_replace, epoch restore, liveness rebuild
```

### 1.6 Architectural constraints & invariants in force

- **INV-SYNC-014** (INC-I-118): after snap install, `self.utxo_set` **must** be the `state_db`-backed
  variant. Any fix must not leave a rollback/reorg path holding an `InMemory` set as the live set.
- **INV-GUARD-001** (INC-I-136): `utxo_count` must equal the number of distinct `cf_utxo` keys.
  `insert_utxo` is counter-idempotent; `remove_utxo` decrements on real deletion. Any real clear must
  reset the counter — `StateDb::clear_utxos()` already does (`writes.rs:100`).
- **INV-UTXO-001** (INC-I-064, protection level 3): conservation
  `total_after == total_before + coinbase_amount`. R1 violates this every time Path A/A′ runs.
- **INV-STORAGE-002** (INC-I-144): height index mutated atomically with each chain_state rewind
  (`rollback.rs:254-257`). Unaffected by either residual, but both fixes sit upstream of it.
- **INV-EPOCH-003** (INC-I-082): `rebuild_epoch_state_from_blocks` takes an explicit `target_height`
  and never reads chain_state. Precedent for keeping rebuild helpers parameter-driven and side-effect-free.
- **FORK_GUARD / REQ-REDESIGN-011**: `chain_state.best_hash` must never advance past `block_store`
  completeness. `ensure_blocks_present` is the enforcement primitive; both `execute_reorg` and (since
  INC-I-152) the legacy rollback rebuild use it.

### 1.7 Blast radius

*Method note*: graphify was not provisioned per the orchestrator's instruction, and it is known blind
to Rust `self.method()` call edges (memory `reference_graphify_rust_method_blind_spot`) — which is
exactly the edge shape here (`self.rebuild_producer_set_from_blocks(...)`). Blast radius below is from
exhaustive `grep -rn` over `crates/` and `bins/` for each symbol, plus reading each matched site.

**Changing `UtxoSet::clear()` (R1)** — 7 non-test edit points, 5 test edit points:

- *Direct*: `crates/storage/src/utxo/set.rs` (definition), `bins/node/src/node/rollback.rs:191`,
  `bins/node/src/node/block_handling.rs:803`, `bins/node/src/node/init.rs:112`.
- *Indirect (behavior, no edit)*: `StateDb::clear_utxos` (`writes.rs:80`) becomes reachable from the
  node crate for the first time; `utxo_count` atomic (INV-GUARD-001); `atomic_replace`
  (`rollback.rs:281`, `writes.rs:152`) now persists a correctly-rebuilt set instead of a polluted one;
  everything downstream that reads the UTXO set after a rollback — state-root computation
  (`snapshot.rs`), `apply_block` validation, RPC balance/supply methods, the periodic supply gauge.
- *Tests to update for a `Result` signature*: `bins/node/tests/recover_replay.rs:193,256,293,355`,
  `bins/node/tests/inc_i_064_supply_conservation.rs:509`.

**Changing `rebuild_producer_set_from_blocks` (R2)** — 1 edit point, 4 affected call sites:

- *Direct*: `bins/node/src/node/rewards.rs:1105`.
- *Indirect*: `rollback.rs:138` (behavior changes: refuse instead of destroy), `rollback.rs:228`,
  `block_handling.rs:717`, `block_handling.rs:835` (behavior unchanged — already dense-guarded, the new
  check is redundant and passes).
- *Cost*: `ensure_blocks_present` is `O(high-low+1)` height-index point lookups with **no**
  header/body deserialization (`queries.rs:191-192`). The function it guards deserializes every block
  in the same range. The redundant scan is therefore a sub-1% overhead on a path that is already
  `O(chain)`, and it only runs on rollback/reorg — never in the hot path.

### 1.8 Brittleness Check

```
━━━ BRITTLENESS CHECK ━━━
Signals detected: 2/5
Details:
  (1) Cross-module blast radius   — NO. Two crates (`storage`, node bin) with a direct dependency edge.
  (2) Invariant gaps              — NO. The enforcing primitives already exist and are tested
                                     (`StateDb::clear_utxos`, `BlockStore::ensure_blocks_present`);
                                     they are simply not wired to these two call sites.
  (3) Data flow reversal          — NO. Both fixes flow with the existing direction.
  (4) Shared mutable state        — YES. `UtxoSet::RocksDb(Arc<StateDb>)` and `StateDb` are two handles
                                     onto the same `cf_utxo` with no single owner. That dual ownership
                                     is precisely the licence under which `clear()` was allowed to
                                     no-op ("state_db clearing is handled elsewhere").
  (5) Contract absence            — YES. `UtxoSet::clear()` has no post-condition test asserting
                                     `len() == 0` across BOTH variants, so the variants were free to
                                     diverge in meaning.
Verdict: LOCALIZED
━━━━━━━━━━━━━━━━━━━━━━━━━
```

Signals (4) and (5) are the same story told twice, and both are *closed by the R1 fix itself*
(an honest `clear()` gives the façade a single, testable contract). No architectural rework indicated.

---

## 2. Verification of both claims against current code

### R1 — VERIFIED, and broader than reported

`crates/storage/src/utxo/set.rs:64-78`, verbatim on this branch:

```rust
    /// Clear all UTXOs.
    ///
    /// For InMemory: clears the HashMap.
    /// For RocksDb: no-op (state_db clearing is handled by `clear_and_write_genesis`).
    pub fn clear(&mut self) {
        match self {
            UtxoSet::InMemory(store) => store.clear(),
            UtxoSet::RocksDb(_) => {
                // state_db clearing is handled by StateDb::clear_and_write_genesis.
                // UtxoSet.clear() on the RocksDb variant is only called during
                // genesis reset (init.rs), which immediately replaces the UtxoSet
                // with a fresh InMemory variant anyway.
            }
        }
    }
```

- The `RocksDb` arm is an empty block. **No-op confirmed** (`set.rs:71-76`).
- Production variant confirmed: `init.rs:311`, `fork_recovery.rs:363`.
- Falsifying call site #1 confirmed: `bins/node/src/node/rollback.rs:191` — `utxo.clear();` inside the
  write-lock block at `:189-224`, immediately before the `for height in 1..=target_height` replay.
- **Falsifying call site #2, NOT in the brief**: `bins/node/src/node/block_handling.rs:803` —
  `utxo.clear();` inside `execute_reorg`'s legacy no-undo rebuild (`:797-831`), same shape, same
  no-op, same leak but over the whole rolled-back range `target+1..=current` instead of one block.
- `add_transaction` on `RocksDb` is a direct `StateDb::insert_utxo` (`set.rs:127-133` doc comment,
  `writes.rs:27`), which is **counter-idempotent on upsert** (INC-I-136) — so re-inserting an
  already-present output is silent, no counter tripwire fires.

**Leak quantification** (deterministic, not probabilistic): after the no-op clear, the live set still
holds the state at `current_height`. The replay of `1..=target_height` re-spends and re-adds, which
restores every output the rolled-back range *spent*, but nothing removes the outputs the rolled-back
range *created*. Residual = {outputs created in `target+1..=current` and not spent inside that range}.
Every block carries a coinbase output to the reward pool, so this set is **never empty** — the leak
fires on 100% of executions of Path A and Path A′, not on an edge case. `atomic_replace`
(`rollback.rs:281`) then writes it durably. This is the INC-I-041 zombie-UTXO / inflation class and it
violates INV-UTXO-001.

**Not verified / narrower than the comment suggests**: the claim that this is *only* a "dense store"
problem. It is orthogonal to store density. The INC-I-152 guard at `rollback.rs:169` prevents the
*holed-store* variant of the harm; R1 is the *dense-store* residual, and it is unconditional.

### R2 — VERIFIED as an unguarded call site; **harm mechanism corrected**

`bins/node/src/node/rewards.rs:1105-1124`:

```rust
    pub fn rebuild_producer_set_from_blocks(
        &self,
        producers: &mut ProducerSet,
        target_height: u64,
    ) -> Result<()> {
        producers.clear();                                    // ← rewards.rs:1110, DESTRUCTIVE, UNCONDITIONAL
        ...
        for height in 1..=target_height {
            let block = self
                .block_store
                .get_block_by_height(height)?
                .ok_or_else(|| {                              // ← rewards.rs:1119, aborts at first hole
                    anyhow::anyhow!(
                        "Producer set rebuild: missing block at height {} (store corrupted)",
                        height
                    )
                })?;
```

- No `ensure_blocks_present` anywhere in `rewards.rs` (verified by grep over the file).
- `ProducerSet::clear()` is a **real** clear (`crates/storage/src/producer/set_core.rs:66-71`): drops
  `producers`, drops `pending_updates`, invalidates `active_cache`; only `exit_history` survives.

⚠ **CONTRADICTION with the reported framing, resolved**: the brief says a holed store *"silently
produces a wrong ProducerSet"*. It does not. It produces **`Err`** — but only *after* having already
cleared and partially repopulated the caller's `&mut ProducerSet`, which is the live
`self.producer_set` write guard. So the accurate statement is: **destroy-then-abort**, the mirror of
the correction INC-I-152 entry 1557 had to apply to P1-003's framing. Consequences, traced:

1. `rollback.rs:138` `?` unwinds out of `rollback_one_block` **before** the chain_state rewind
   (`:240`), **before** `atomic_replace` (`:281`), and **before** the liveness rebuild (`:237`).
2. The UTXO undo at `rollback.rs:105-117` has **already** run and, on the `RocksDb` variant, has
   already written durably (`remove` → `StateDb::remove_utxo`, `insert` → `StateDb::insert_utxo`).
   So `cf_utxo` is at `target_height` while `META_CHAIN_STATE` still says `local_height`.
3. In memory, `producer_set` is now empty (bar `exit_history`).
4. Neither caller repairs it: `periodic.rs:718` propagates with `?`; `production/mod.rs:609-622` logs
   `[BLOCK_POISON] Rollback failed: … Manual intervention needed.` and returns. The node keeps running.
5. **The destruction becomes durable on the very next applied block**:
   `bins/node/src/node/apply_block/mod.rs:316-318` executes `batch.write_full_producer_set(&producers)`
   inside the block's atomic batch. The empty ProducerSet is then the persisted ProducerSet.

That escalation (transient in-memory destruction → durable, self-inflicted, single-block latency)
makes R2 materially more severe than "wrong ProducerSet", and it is why the guard must sit **before**
`producers.clear()`, not merely at the call site.

### Reachability of both paths (are these live hazards or dead code?)

- `UNDO_KEEP_DEPTH = 100` (`crates/core/src/consensus/constants.rs:259`); `apply_block/mod.rs:371`
  prunes below `height - UNDO_KEEP_DEPTH` after every commit, and `init.rs:538` runs
  `prune_undo_below(horizon)` at startup. Undo coverage is a 100-block sliding window.
- A snap-synced node has **no undo entry for its installed tip** — `get_undo(local_height)` is `None`,
  so the first `ShallowRollback` after a snap install lands directly in Path A. This is the exact
  INC-I-152 fleet scenario.
- `bins/node/src/operations/chain.rs:208` calls `prune_undo_above(new_tip)` during manual recovery,
  another way to arrive with no undo above the tip.
- A `producer_snapshot` that fails `bincode::deserialize` (Path B / R2) arises from a partially-written
  or format-drifted undo entry; the code already treats it as an expected case (it has a
  `warn!` + fallback at `rollback.rs:136`, and a twin at `block_handling.rs:715`).

Both paths are live, reachable, and specifically reachable on the node class the fleet just operated on.

---

## 3. Probable cause + proposed fix direction (SSF — one per residual)

### R1 — cause

`UtxoSet` is a façade over two backends whose `clear()` semantics were allowed to diverge because, at
Phase 4, `cf_utxo` acquired two owners (`StateDb` and the `UtxoSet::RocksDb` handle wrapping it). The
author resolved the ambiguity by *documenting* an assumption about call sites — "only called during
genesis reset" — instead of *enforcing* a post-condition. Assumptions about call sites decay; three
call sites now falsify it. INC-I-136 hit the same decay and patched the *caller* (`init.rs:111`
`if !utxo_set.is_rocksdb()`), which fixed one site and left the misleading primitive in place for the
next two.

### R1 — proposed fix (ONE)

> **Make `UtxoSet::clear()` honest on the `RocksDb` variant by delegating to the already-existing
> `StateDb::clear_utxos()`, and change the return type to `Result<(), StorageError>` so the outcome
> cannot be silently discarded.**

This works because the correct implementation already exists and is already tested:
`crates/storage/src/state_db/writes.rs:80-102` iterate-deletes `cf_utxo` **and** `cf_utxo_by_pubkey`
in a single `WriteBatch` and stores `utxo_count = 0` (INV-GUARD-001-correct), with coverage at
`crates/storage/tests/disk_guardian_failsafe_test.rs:283` (success wipes) and `:401` (failing DB
returns `Err`, does not panic). The change is a ~6-line body in `set.rs` plus `?` propagation at four
production call sites; it repairs `rollback.rs:191` and `block_handling.rs:803` simultaneously and
prevents the *next* caller from inheriting the same lie.

**Decision between the options named in the brief:**

- **(a) real `clear()` on the RocksDb variant — CHOSEN.** Fixes the root cause at the primitive, not
  per-call-site. Reuses tested machinery. Smallest diff that leaves no known-wrong caller behind.
- **(b) error/panic on RocksDb + let the legacy rebuild handle it — REJECTED.** It requires the same
  signature change as (a) but then *also* requires inventing replacement logic at two call sites, and
  it leaves `UtxoSet` unable to express "empty the set" — a legitimate operation on a façade whose
  entire purpose is backend transparency. Strictly more moving parts, strictly less capability.
- **(c) rebuild into an `InMemory` scratch set and publish via the existing `atomic_replace`
  — REJECTED for this iteration, but see the honesty note below.** Mechanically attractive: it makes
  the rebuild crash-atomic and needs no `clear()` at all. But it must be applied at *two* call sites
  independently, it restructures ~40 lines of consensus-adjacent replay logic per site, it introduces a
  transient full-chain UTXO set in RAM (the mainnet set is the largest object the node owns, and RAM
  headroom is a live fleet concern per INC-I-150), and it leaves the lying primitive in place. That is
  more risk and more surface for a benefit that is not the reported defect.

**Honest statement of what (a) changes about failure modes** (required — this is the one place the
chosen fix is not strictly dominant):

The replay loop can still fail *after* the clear, on `get_block_by_height(h)?` (I/O),
`add_transaction(..)?` (I/O), or `consume_genesis_bond_utxos(..)?`. Today, such a failure leaves a
**superset** (the leak plus a partial replay). After the fix it leaves a **subset** (a partial replay
of a truly-emptied set). Both require a resync — node-local recovery is identical. The subset is
strictly preferable for the *network*, because a subset is fail-visible (the node's state root diverges
downward immediately and it cannot construct a valid block) whereas a superset is fail-silent inflation
that satisfies every local check and is exactly the INC-I-041 harm class INC-I-152 measured at
+99,000 minted. Trading silent inflation for loud, local, recoverable emptiness is the right trade.
The missing-block case, which was the original INC-I-152 fear (entry 1553), is already unreachable on
Path A thanks to the dense guard at `rollback.rs:169`, and on Path A′ thanks to `block_handling.rs:599`.

### R2 — cause

`rebuild_producer_set_from_blocks` is a "rebuild from the whole chain" helper that mutates its output
argument **before** establishing that it can complete. Three of its four call sites happen to sit
behind a dense guard for unrelated reasons (they also rebuild the UTXO set or advance chain_state), so
the helper's own lack of a precondition was invisible. The fourth call site — the
snapshot-deserialize-failure fallback at `rollback.rs:138` — has no such incidental protection.

### R2 — proposed fix (ONE)

> **Hoist `self.block_store.ensure_blocks_present(1, target_height.max(1))` to the first statement of
> `rebuild_producer_set_from_blocks` (`rewards.rs:1105`), returning `Err` *before* `producers.clear()`
> ever runs.**

This works because it converts the helper from "destructive then fallible" to "fallible then
destructive" **for the height-index gap class** — the INC-I-152 shape, and the only failure on this
path with a recorded incident — at all four call sites, including any future one, rather than
patching the one site that is currently exposed.

**Scope correction (INC-I-156 M2 review F1 / AUDIT-P3-211).** An earlier draft of this section
claimed the fix makes destroy-then-abort impossible *by construction at all four call sites*. That
is an **overclaim** and is retracted here, at the origin. The guard NARROWS the class; it does not
close it. `ensure_blocks_present` answers a **cheaper question** than the loop it fronts: it reads
the height **INDEX** only (`block_store/queries.rs:199-207`, `get_hash_by_height` per height,
touching neither `cf_headers` nor `cf_bodies`), whereas the replay loop needs a **BODY**
(`get_block_by_height` = index lookup THEN `get_block`, and `get_block` returns `Ok(None)` when
`cf_bodies` has no entry for the hash, `block_store/queries.rs:35-38`). On an **index-dense /
body-absent** store the guard returns `Ok`, `producers.clear()` still runs, and the replay still
aborts — destroy-then-abort survives on exactly that input, at all four call sites. That residual is
production-constructible (three writers produce it — see REQ-I156-015 in §7), is not covered by any
M2 test, and is deferred. The accurate claim is: **impossible by construction for a height-index
gap, at all four call sites.**

**Applicability verification for this call site (required by the brief — the guard is not adopted
verbatim on faith):**

| Question | Finding |
|---|---|
| What locks are held at `rollback.rs:138`? | `self.producer_set.write()` (taken at `:137`) and — from the enclosing block at `:105-117` — the `utxo_set` write guard has already been **dropped** at `:117`. |
| Does the guard take a conflicting lock? | No. `ensure_blocks_present` touches only `BlockStore`'s RocksDB height index (`queries.rs:193-209`), an independent lock domain from `producer_set`/`utxo_set`/`chain_state`. No deadlock risk, and `&self` already exposes `self.block_store` in this function (`rewards.rs:1117`). |
| Is the function `&self`? | Yes (`rewards.rs:1106`) — no re-borrow conflict with the `&mut ProducerSet` argument, which is a separate binding held by the caller. |
| What happens on error, per call site? | `rollback.rs:138`: `?` → `rollback_one_block` returns `Err` with the ProducerSet **untouched** (the improvement). `rollback.rs:228` / `block_handling.rs:717` / `:835`: unreachable-by-construction, since their upstream guard already proved the same range dense over the same height index. |
| Is the failure path atomic? | Yes, post-fix: the guard runs before the first mutation, so a refusal mutates nothing. Pre-fix it is not atomic — that is the defect. |
| Is the range identical to the upstream guards? | Yes. The loop is `1..=target_height` (`rewards.rs:1115`); `rollback.rs:171` checks `1..=target_height.max(1)`; `block_handling.rs:600` checks `1..=target_height`. `.max(1)` keeps the `target_height == 0` case as strict as INC-I-152 made it. Same `height_index`, same oracle (`inc_i_152_p1_003_rollback_holed_store.rs:569` records that equivalence). |
| Cost of the now-redundant scan at the 3 guarded sites? | `O(range)` point lookups, no deserialization (`queries.rs:191-192`), in front of a function that deserializes every block in the same range. Sub-1%, on a non-hot path. |

**Note on the residual `Err` at `rollback.rs:138`.** With the guard in place, a dense store plus a
corrupt snapshot yields a correct rebuild (no change). A holed store plus a corrupt snapshot yields an
`Err` out of `rollback_one_block` with the ProducerSet intact — but the UTXO undo at `:105-117` has
already been applied and durably written, so chain_state and `cf_utxo` are left inconsistent by one
block. That inconsistency **already exists today on every error return from this branch** and is not
introduced by this fix; it is recorded in §7 (Out of Scope) as a follow-up rather than silently folded
into this incident.

---

## 4. Impact analysis

### 4.1 INC-I-075 / INV-12 three-question consensus-shape checklist

**Q1 — Can a user-submittable transaction reach these paths?**
**YES, indirectly.** `production/mod.rs:595-622` is the block-poison path: a transaction that passes
builder validation but fails apply validation makes the node call `rollback_one_block()`. A user can
therefore *initiate* a rollback. The user cannot choose whether undo data is present (that is a
function of `UNDO_KEEP_DEPTH` and snap history), so reaching the legacy branch specifically is not
user-selectable — but honest answer: YES.

**Q2 — Can a producer action or attestation pattern reach it?**
**YES.** An equivocating or fork-producing producer causes reorgs (`execute_reorg` → Path A′) and
`ShallowRollback` recovery actions (`periodic.rs:718` → Path A/B).

**Q3 — Is the new behavior bit-identical for ALL reachable inputs?**
**NO.** On the affected paths the resulting UTXO set (R1) and ProducerSet (R2) differ from pre-fix.

**Literal checklist result: (Q1|Q2) YES + Q3 NO ⇒ "activation height REQUIRED".**
**Assessed result: activation height NOT required.** The exemption argument, stated explicitly so a
reviewer can attack it:

The checklist exists to prevent a *fleet split over validity* — two honest nodes disagreeing about
whether a block or a state is canonical. That requires the pre-fix output to be canonical for at least
some reachable input. It is not, for either residual:

- **R1**: the pre-fix output is `canonical(target) ∪ leaked`, where `leaked` ⊇ {the coinbase output of
  every rolled-back block} and is therefore **never empty** (§2). The pre-fix result is wrong on
  100% of executions, and it is wrong *relative to every peer that did not take this path* — i.e.
  the entire rest of the fleet. The fix changes the output only over inputs where the old output was
  already non-canonical; it moves a node from a divergent state toward the canonical one. It cannot
  create a new disagreement class; it shrinks the existing one.
- **R2**: the pre-fix output is an emptied ProducerSet, which is non-canonical for every
  `target_height > 0` on any live chain. Same argument.

Neither change touches a rule by which a *block* is accepted or produced: no validation predicate, no
scheduler input computation, no `active_producers` derivation, no bond snapshot, no bitfield encoding,
no coinbase shape. Both are node-local recovery hardening — the same classification INC-I-152's
`rollback.rs:169` guard shipped under, on this same code path, without an activation height.

**⚠ Reviewer attention requested on exactly one point**: the argument rests on "the pre-fix output is
never canonical". For R1 that is proven by the always-present coinbase output; for R2 by
`producers.clear()` being unconditional. If either premise is wrong, the exemption collapses and an
activation height is required. Both premises are cited to specific lines above.

### 4.2 The two deploy questions (CLAUDE.md "After Every Modification" #3)

**Deploy Q1 — Does this change consensus RULES?** **NO.** No validation predicate, no activation gate,
no `NetworkParams` field, no `HardForkSchedule` entry, no `CURRENT_PROTOCOL_VERSION` /
`EPOCH_STATE_FORMAT_VERSION` / `MIN_PEER_PROTOCOL_VERSION` change. **⇒ no activation height.**

**Deploy Q2 — Does this change block CONTENT?** **NO.** Nothing in the diff surface reaches the
bitfield encoder/decoder, coinbase construction, transaction ordering, `presence_root`, or any header
field. A node whose state is correct produces byte-identical blocks before and after.
**⇒ no synchronized deploy; a rolling restart is safe.** Mixed-fleet operation is safe *for block
content*: a patched and an unpatched node produce identical blocks from identical state.

**Direction correction (AUDIT-P2-205).** An earlier draft of this section justified the rolling
restart with "…unless the unpatched one has taken a legacy rebuild path, in which case it was already
divergent." That has the asymmetry **backwards** and is retracted. The mixed-fleet delta runs the
other way:

> At `target_height == 0` — reachable on the undo branch (`rollback.rs:138`, `has_undo == true`, when
> the producer snapshot fails to deserialize) and at the `block_handling.rs:739`/`:910` sites — the
> `.max(1)` floor makes the **PATCHED** node check `1..=1` and return `Err` on a store with no
> block 1, while the **UNPATCHED** node runs the empty `1..=0` loop and returns `Ok`. **The patched
> node is the one that stops; the unpatched node completes.**

This is a **fail-closed vs fail-open** difference, not a correctness inversion: what the unpatched
node "completes" is a rebuild that leaves the ProducerSet **empty** — precisely the corruption this
milestone exists to prevent, and one that becomes durable on the next applied block. The patched
node's refusal is the intended outcome. But the operational consequence must be stated honestly: **on
a rolling deploy the newly-patched nodes are the ones that can halt on this input, not the stragglers.**
An operator who sees a node stop rebuilding immediately after upgrading is seeing the guard work, and
the remedy is backfill (or `doli-node reindex --data-dir <DATA_DIR>` on a stopped node if the blocks
are on disk and only the height index is stale) — not a rollback of the binary.

**No `has_undo`-aware carve-out is added** (explicit decision, AUDIT-P2-205). Suppressing the guard
when `has_undo == true` would require threading `has_undo` through
`rebuild_producer_set_from_blocks`'s signature to all four call sites, is covered by no existing
test, and would re-open the fail-open path on the one branch that has no other upstream guard. It is
recorded here as a **known, accepted asymmetry** rather than carved out speculatively in a bugfix
commit.

**Explicit statement**: **no activation height, no synchronized deploy, no version bump.** Standard
rolling restart, testnet first.

### 4.3 Existing code affected

| File | How affected | Risk |
|---|---|---|
| `crates/storage/src/utxo/set.rs` | `clear()` body + signature | **medium** — signature change ripples to 9 sites; caught at compile time |
| `bins/node/src/node/rollback.rs:191` | `clear()` now really clears; `?` added | **medium** — the behavioral fix itself; changes post-rollback state on the legacy path (intended) |
| `bins/node/src/node/block_handling.rs:803` | same | **medium** — same fix, wider range |
| `bins/node/src/node/init.rs:112` | `?`/handling added inside the existing `!is_rocksdb()` fence | low — `InMemory`-only, semantics unchanged |
| `bins/node/src/node/rewards.rs:1105` | dense guard prepended | low — pure precondition; the guarded body is unchanged |
| `bins/node/tests/recover_replay.rs`, `bins/node/tests/inc_i_064_supply_conservation.rs` | mechanical `?`/`unwrap` at 5 sites | low |
| `crates/storage/src/state_db/writes.rs` | **unchanged** — reused as-is | none |

### 4.4 What breaks if this changes

- **`atomic_replace` after a legacy rollback** (`rollback.rs:281`) will now persist a *smaller*, correct
  set. Any test that asserted the pre-fix (inflated) supply after a legacy rollback will fail — that is
  the intended signal, not a regression. Mitigation: expect and inspect such failures rather than
  adjusting expectations reflexively.
- **`utxo_count`** drops to 0 and is rebuilt during the replay. `insert_utxo` is counter-idempotent
  (`writes.rs:44-46`), so the post-rebuild count equals the distinct-key count — INV-GUARD-001 holds.
  Mitigation: assert `utxo_count == iter_all().len()` after a legacy rebuild.
- **Nothing breaks on the happy path.** The undo-based rollback (the overwhelmingly common case) never
  calls `clear()` and, with a valid `producer_snapshot`, never calls the rebuild helper.

### 4.5 Regression risk areas

- **Post-clear replay failure ⇒ empty/partial persisted set** — analyzed and accepted in §3 (fail-visible
  beats fail-silent); must be called out in the developer's `Path-Coverage:` block.
- **INV-SYNC-014** — the fix must not swap the live variant. Neither proposed change touches variant
  construction; a regression assertion (`utxo_set.is_rocksdb()` still true post-rollback) is cheap.
- **Reorg path (`block_handling.rs:803`)** carries the same fix but a *wider* range and, unlike
  `rollback.rs`, silently skips absent blocks (`:805-807` uses `.ok().flatten()`); it relies entirely on
  the upstream guard at `:599` for density. The R1 fix does not change that reliance, but a real clear
  makes the consequence of a guard bypass worse (empty instead of stale). The guard at `:599` must be
  confirmed intact by the reviewer.
- **`rebuild_producer_liveness` / `rebuild_epoch_state_from_blocks`** run after both fixed paths and are
  themselves `O(chain)` block readers with no dense guard. Out of scope (§7) but named so QA does not
  mistake a failure there for one of these two fixes.

---

## 5. Requirements

### Summary (plain language)

Two recovery routines in the node quietly do the wrong thing. The first says "empty the coin database"
but on real nodes that command does nothing, so when the node rebuilds after undoing a block it stacks
the rebuilt coins on top of the old ones and invents money that never existed. The second wipes the
producer list before checking that it has all the blocks it needs to rebuild it — if blocks are
missing it erases the list and gives up, and the very next block it accepts saves that empty list to
disk. Fix one: make "empty the database" actually empty it. Fix two: check first, wipe second.

### User stories

- As a **node operator**, I want a rollback after a snap sync to leave my node with the same coin set as
  the rest of the network, so that my node does not silently fork on invented supply.
- As a **node operator**, I want a recovery routine that cannot complete to leave my node exactly as it
  found it, so that a transient block-store gap does not destroy my producer set.
- As a **protocol maintainer**, I want `UtxoSet::clear()` to mean the same thing on both backends, so
  that the next caller cannot inherit a silently wrong assumption.

### Requirements table

| ID | Requirement | Priority | Acceptance criteria (summary) |
|---|---|---|---|
| REQ-I156-001 | `UtxoSet::clear()` must empty the set on **both** variants; on `RocksDb` it delegates to `StateDb::clear_utxos()` | **Must** | post-condition `len()==0` on both variants; `utxo_count==0`; both `cf_utxo` and `cf_utxo_by_pubkey` emptied |
| REQ-I156-002 | `clear()` must return `Result<(), StorageError>`; no production call site may discard the error | **Must** | signature changed; all 3 production call sites (`rollback.rs`, `block_handling.rs`, `init.rs`) propagate; no `let _ =` on a `clear()` call anywhere in `crates/` or `bins/` non-test code |
| REQ-I156-003 | The legacy no-undo rollback rebuild (`rollback.rs:~191`) must not leak the rolled-back block's outputs on a `RocksDb`-variant node with a dense store | **Must** | red test proves the leak pre-fix and passes post-fix (see detail) |
| REQ-I156-004 | The `execute_reorg` legacy rebuild (`block_handling.rs:~803`) must not leak the rolled-back range's outputs on a `RocksDb`-variant node | **Must** | same shape as 003, over a multi-block reorg range |
| REQ-I156-005 | `rebuild_producer_set_from_blocks` must verify `ensure_blocks_present(1, target_height.max(1))` **before** any mutation of the passed `ProducerSet` | **Must** | red test proves destruction pre-fix and refusal post-fix |
| REQ-I156-006 | Any refused operation must leave both the UTXO set and the ProducerSet byte-for-byte unchanged | **Must** | pre/post snapshots compared by full content, not just counts |
| REQ-I156-007 | No behavior change on the dense/happy path | **Must** | undo-based rollback and dense-store reorg produce identical state roots pre- and post-fix |
| REQ-I156-008 | The falsified doc comment at `set.rs:64-78` must be replaced by the true post-condition | **Should** | comment states the post-condition, names both backends, cites `clear_utxos` |
| REQ-I156-009 | `specs/engine-parts.md` drift corrected for both routines | **Should** | see §8 |
| REQ-I156-010 | Regression assertion: the live `UtxoSet` variant is still `RocksDb` after a legacy rollback/reorg (INV-SYNC-014) | **Should** | `is_rocksdb()` asserted post-rollback in the R1 tests |
| REQ-I156-011 | Extract/refresh an invariant record for "façade methods must have variant-uniform post-conditions" | **Could** | `invariants` row + linked `regression_tests` rows |
| REQ-I156-012 | Make the legacy rebuild crash-atomic (scratch set + single `atomic_replace`) | **Won't** | deferred — §7 |
| REQ-I156-013 | Guard `rebuild_epoch_state_from_blocks` / `rebuild_producer_liveness` similarly | **Won't** | deferred — §7 |
| REQ-I156-014 | Repair the chain_state ↔ `cf_utxo` inconsistency left by an `Err` return from the undo branch | **Won't** | pre-existing, not introduced here — §7 |

### Detailed acceptance criteria

**REQ-I156-001 — honest `clear()`**
- [ ] Given a `UtxoSet::RocksDb` holding N > 0 UTXOs, when `clear()` returns `Ok(())`, then
      `iter_all().is_empty()` and `utxo_count() == 0`.
- [ ] Given the same, then `cf_utxo` **and** `cf_utxo_by_pubkey` are both empty (no orphaned index rows).
- [ ] Given a `UtxoSet::InMemory`, behavior is unchanged (`len()==0` after clear).
- [ ] Given a read-only / failing DB, `clear()` returns `Err` and does not panic
      (mirrors `disk_guardian_failsafe_test.rs:401`).
- [ ] The same post-condition test runs against **both** variants from one parameterized body — the
      contract-absence signal from §1.8 is closed by construction.

**REQ-I156-002 — non-swallowable result**
- [ ] `pub fn clear(&mut self) -> Result<(), StorageError>`.
- [ ] `rollback.rs`, `block_handling.rs`, `init.rs` propagate with `?` (or an explicit, commented
      handling for the `InMemory`-fenced `init.rs` site).
- [ ] Grep over non-test `crates/` and `bins/` finds zero `let _ = ….clear()` and zero
      `.clear();`-as-statement on a `UtxoSet`.

**REQ-I156-003 — R1 red test, rollback path (the leak proof)**
- [ ] Test node is built on the **`RocksDb`** variant (`UtxoSet::from_state_db`) — not `new_for_test`'s
      `InMemory`, which cannot express this bug. Recording this explicitly: an `InMemory`-variant test
      will PASS on the broken code, exactly as INC-I-152's first P1-003 test did.
- [ ] Block store is **dense** over `1..=target_height` — the INC-I-152 guard at `rollback.rs:169` must
      not be what refuses this test; the leak must be shown on a store that guard admits.
- [ ] Undo data for the tip is absent (`prune_undo_above(0)`, the production API used at
      `inc_i_152_p1_003_rollback_holed_store.rs:544`), forcing the legacy branch.
- [ ] Given the above, when `rollback_one_block()` runs, then **pre-fix** the total supply after
      rollback is strictly greater than the canonical supply at `target_height`, and the specific
      outpoints created by the rolled-back block are still present.
- [ ] **Post-fix**, the UTXO set is exactly the canonical set at `target_height`: same total supply,
      same outpoint set; none of the rolled-back block's created outpoints remain.
- [ ] Post-fix, `utxo_count() == iter_all().len()` (INV-GUARD-001).
- [ ] Post-fix, `utxo_set.is_rocksdb()` is still true (INV-SYNC-014).
- [ ] The test asserts on the **persisted** state (after `atomic_replace`), not only in memory.

**REQ-I156-004 — R1 red test, reorg path**
- [ ] Same construction over `execute_reorg`'s legacy branch: `RocksDb` variant, dense store,
      `has_undo == false` over the rollback range (`block_handling.rs:658-659`), reorg depth ≥ 2.
- [ ] Pre-fix: outputs created in `target+1..=current` and unspent within that range survive the reorg.
- [ ] Post-fix: they do not; the set equals the canonical set at `target_height`.

**REQ-I156-005 — R2 red test (destroy-then-abort proof)**
- [ ] Block store is **holed** over `1..=target_height` (drop a canonical height-index entry, the
      technique recorded at `inc_i_152_p1_003_rollback_holed_store.rs:569`), and the precondition
      `ensure_blocks_present(1, target)` is asserted to already report the hole.
- [ ] Undo data for the tip **exists** but its `producer_snapshot` is non-empty and **not**
      deserializable as a `ProducerSet` — this is the only route to `rollback.rs:138`.
- [ ] Pre-fix: after `rollback_one_block()` returns `Err`, the in-memory `ProducerSet` is **empty**
      (`producers.len() == 0`) despite having been non-empty before — i.e. the rebuild was *accepted
      into a mutation* it could not complete. (⚠ This is the corrected form of the brief's
      "wrong/accepted ProducerSet"; the observable pre-fix harm is destruction, not a plausible-looking
      wrong set. See §2 R2.)
- [ ] Pre-fix escalation, asserted: applying one further block persists the emptied set
      (`apply_block/mod.rs:316-318`), so the destruction survives a restart.
- [ ] Post-fix: `rollback_one_block()` returns `Err` (or refuses) with the `ProducerSet` **identical**
      to its pre-call value — same producer count, same pubkeys, same bond totals, same
      `pending_updates` length.
- [ ] Post-fix: the error message names the first missing height (`ensure_blocks_present` already does
      this, `queries.rs:201-205`) so an operator can act.

**REQ-I156-006 — unchanged-after-refusal**
- [ ] For every refusal path introduced or touched (R1 clear-failure, R2 density refusal), the test
      compares a **full content snapshot** taken before the call with one taken after: UTXO outpoint
      set + amounts, and ProducerSet pubkeys + bonds + pending update count. Count-only assertions are
      insufficient.

**REQ-I156-007 — happy path unchanged**
- [ ] Undo-based rollback with a valid `producer_snapshot`: state root at `target_height` is identical
      pre- and post-fix.
- [ ] Dense-store reorg with undo data: state root identical pre- and post-fix.
- [ ] Full workspace suite: no new failures beyond the 3 known pre-existing ones recorded at INC-I-152
      close.

**REQ-I156-008 — comment truth**
- [ ] `set.rs` doc comment states the post-condition ("after `Ok(())`, the set is empty on either
      backend"), names `StateDb::clear_utxos` as the `RocksDb` implementation, and contains no claim
      about which call sites exist.

---

## 6. Milestones assessment

**Confirmed: 2 milestones**, as proposed by the orchestrator — with one adjustment to M1's contents.

**M1 — R1: honest `UtxoSet::clear()`**
`REQ-I156-001, -002, -003, -004, -008, -010`.
Touches `crates/storage/src/utxo/set.rs`, `bins/node/src/node/rollback.rs:191`,
`bins/node/src/node/block_handling.rs:803`, `bins/node/src/node/init.rs:112`, and 5 test call sites.

> **Adjustment**: M1 must include `block_handling.rs:803` (REQ-I156-004), which the task brief did not
> list. Splitting it out would be *incorrect*, not merely inconvenient: the signature change to
> `clear()` forces that file to be edited in the same commit anyway, so leaving its behavior unfixed
> would mean knowingly compiling a call site whose semantics just changed under it. One commit, both
> leak sites.

**M2 — R2: precondition before mutation in `rebuild_producer_set_from_blocks`**
`REQ-I156-005, -006, -007, -009`.
Touches `bins/node/src/node/rewards.rs:1105` only (plus its new test).

**Independence**: verified. M1 and M2 share the file `rollback.rs` but not the region — M1 edits line
191 (legacy branch), M2's behavior change is observed at line 138 (undo branch) with the code edit in
`rewards.rs`. Either can land, be reverted, or be deployed alone. Recommended order **M1 → M2** only
because M1's `RocksDb`-variant test harness is the harder piece to build and M2 can reuse the
holed-store technique already in `inc_i_152_p1_003_rollback_holed_store.rs`; the reverse order is also
valid. No third milestone: the brief excludes PM-016 re-derivation (an orchestrator-owned post-fix
governance step), and §7 items are explicitly `Won't`.

---

## 7. Out of scope (Won't, this iteration)

- **Crash-atomic legacy rebuild** (REQ-I156-012): replay into a scratch set and publish through one
  `atomic_replace`. Real improvement, rejected as fix-direction in §3, and a candidate for its own
  incident. Do not fold in.
- **Dense guards for `rebuild_epoch_state_from_blocks` and `rebuild_producer_liveness`**
  (REQ-I156-013): both are `O(chain)` block readers that run after the fixed paths, and both are
  plausibly exposed to the same class of defect. **Not investigated** in this analysis — named so a
  future incident can pick them up, and so QA does not attribute a failure there to M1/M2.
- **chain_state ↔ `cf_utxo` inconsistency on an `Err` return from the undo branch** (REQ-I156-014):
  pre-existing (§3 R2 note), not introduced by either fix, and not reported.
- **Body-density guard for `rebuild_producer_set_from_blocks`** (REQ-I156-015): the M2 guard is
  **height-index-only**, so an index-dense / body-absent store still reaches `producers.clear()` and
  still aborts in the replay (§3 R2 scope correction). Closing it needs either a body-density variant
  of `ensure_blocks_present` (one that resolves each height to a hash and probes `cf_bodies`, paying
  an O(range) body lookup the current guard deliberately avoids) or the REQ-I156-012 scratch-set
  shape, which subsumes it. **Deferred, not dismissed** — the shape is production-constructible by
  three distinct writers, all verified against `crates/storage/src/block_store/writes.rs`:
  1. `seed_canonical_index` (`writes.rs:230-238`) — writes `height_index` + `hash_to_height` +
     `snap_horizon` in one batch, and no header and no body. The snap-sync anchor.
  2. `set_canonical_chain` (`writes.rs:107-170`) — indexes canonical heights on **header presence
     alone**: the tip-down walk reads `get_header` (`:161`) to follow `prev_hash` and writes both
     index CFs (`:145-146`) without ever consulting `cf_bodies`.
  3. `put_block` (`writes.rs:20-70`) — writes header (`:31`), body (`:42`) and slot index (`:46`) as
     three **separate, un-batched** `put_cf` calls. No `WriteBatch`, so a crash or error return
     between `:31` and `:42` leaves a durable header with no body.

  This bullet is the deferral target cited by the code comment in
  `bins/node/src/node/rewards.rs` and by the `specs/engine-parts.md` entries for
  `rebuild_producer_set_from_blocks`, `execute_reorg` and `rollback_one_block`.
- **PM-016 re-derivation**: orchestrator-owned governance step, performed after the code fixes.
- **FM3 fleet-wipe snap herd** (INC-I-152 entry 1561): separate incident.

---

## 8. Specs drift detected

- **`specs/engine-parts.md:2795`** — describes `rollback_one_block()`'s legacy fallback as *"clears UTXO
  set and replays all blocks from genesis"*. On production (`RocksDb`) nodes it does **not** clear
  (R1). The line also omits the INC-I-152 `ensure_blocks_present` guard, which the sibling entry for
  `execute_reorg` at **`:2760`** does document. Two corrections needed; both become accurate once M1
  lands, so update with the fix rather than ahead of it (REQ-I156-009).
- **`crates/storage/src/utxo/set.rs:64-78`** — in-code doc comment, false in all three clauses (§1.3).
  Covered by REQ-I156-008.
- No drift found in `docs/architecture.md` or `docs/troubleshooting.md` on these two routines.

---

## 9. Assumptions

| # | Assumption (technical) | Plain language | Confirmed |
|---|---|---|---|
| 1 | `StateDb::clear_utxos()` is safe to call while a `UtxoSet::RocksDb` write guard is held — it takes no additional node-level lock, only RocksDB's internal one | Emptying the database from inside the rollback lock will not deadlock | Read: `writes.rs:80-102` — no node locks. **Yes** |
| 2 | `ensure_blocks_present` and `rebuild_producer_set_from_blocks` consult the same `height_index` | The density check answers the exact question the rebuild loop asks | `queries.rs:200` `get_hash_by_height` vs `rewards.rs:1118` `get_block_by_height`; equivalence recorded at `inc_i_152_p1_003_rollback_holed_store.rs:569`. **Yes** |
| 3 | Every block carries at least one coinbase/reward output, so the R1 leak set is never empty | The bug fires every time, not occasionally | Inferred from the reward-pool coinbase design (CLAUDE.md mental model) + `is_reward_minting()` handling at `rollback.rs:203`. **Not directly re-verified** — the exemption argument in §4.1 depends on it. Flagged for the architect. |
| 4 | No non-DOLI consumer depends on `UtxoSet::clear()`'s current `()` return | Changing the signature only breaks our own code | Workspace-internal type, no public API surface. **Yes** |
| 5 | The 3 known pre-existing test failures from INC-I-152 close are still the only pre-existing failures on this base | The baseline is what we think it is | Not re-run (analysis is read-only). **No** — the developer must establish the baseline before the red tests. |

---

## 10. What I don't understand (intellectual-honesty gate)

1. **Whether `rollback.rs`'s legacy branch was ever *intended* to clear.** The `.max(1)` reasoning at
   `:162-166` and the INC-I-152 comment block show careful thought about density, but nothing in the
   comment acknowledges that the very next statement (`utxo.clear()`) is inert on the variant the
   comment names ("the production RocksDb backend", `:154`). Either the author knew and considered it
   out of scope, or the no-op was not noticed. I could not determine which, and it does not change the
   fix.
2. **Why `block_handling.rs:805-807` uses `.ok().flatten()`** (skip missing blocks silently) while
   `rollback.rs:196-201` uses `.ok_or_else(…)?` (hard error) for the same replay loop. Both are behind
   dense guards today so the difference is currently unobservable, but the asymmetry is unexplained and
   would matter if a guard were ever bypassed.
3. **The exact production frequency of the legacy no-undo branch.** I established it is reachable
   (snap-installed tip has no undo; `UNDO_KEEP_DEPTH=100`; `prune_undo_above` in manual recovery) but I
   did not measure how often the fleet actually enters it. If it is entered routinely, R1's supply
   impact on real nodes may already be measurable in explorer supply figures — worth a check I did not
   perform.
4. **Whether any snapshot/state-root consumer caches a UTXO-derived value across a rollback** such that
   a now-smaller set would surface a stale cache. I traced `atomic_replace` and
   `delete_chain_commitment()` (`rollback.rs:348`) but did not exhaustively enumerate UTXO-derived
   caches.

None of these gaps blocks M1 or M2; items 3 and 4 are worth a line of the architect's attention.

---

## 11. Traceability matrix

Test-file legend (M1, run 493):
- **SC** = `crates/storage/tests/inc_i_156_clear_contract_test.rs`
- **RB** = `bins/node/tests/inc_i_156_m1_rocksdb_clear_leak.rs`
- **RG** = `bins/node/tests/inc_i_156_m1_reorg_clear_leak.rs`
- shared fixture: `bins/node/tests/inc_i_156_m1_harness/mod.rs`
- **[RED]** = fails on base f4e6ea69, must pass post-fix. **[LOCK]** = passes before AND after.

| Requirement ID | Priority | Milestone | Test IDs | Architecture section | Implementation module |
|---|---|---|---|---|---|
| REQ-I156-001 | Must | M1 | SC `clear_empties_the_set_rocksdb_variant` **[RED]**; SC `clear_empties_the_set_inmemory_variant` **[LOCK]**; SC `clear_on_already_empty_rocksdb_is_ok_noop` **[LOCK]** (one shared body `assert_clear_empties`, both variants) | (architect) | ✅ `UtxoSet::clear` @ `crates/storage/src/utxo/set.rs:64-80` — RocksDb arm delegates to `StateDb::clear_utxos` (`state_db/writes.rs:80`) |
| REQ-I156-002 | Must | M1 | SC `clear_returns_a_result_that_cannot_be_swallowed` **[RED]**; SC `clear_on_failing_rocksdb_returns_err_not_panic` **[RED]** | (architect) | ✅ `UtxoSet::clear -> Result<(), StorageError>` @ `set.rs:72`; consumed at all 3 non-test sites: `rollback.rs:201` (`map_err(..)?`), `block_handling.rs:809` (`map_err(..)?`), `init.rs:118` (`?`, inside the INC-I-136 fence). Grep over `crates` and `bins` (per-root): zero `let _ = ….clear()`, zero bare `.clear();`-as-statement |
| REQ-I156-003 | Must | M1 | RB `inc_i_156_req003_legacy_rollback_must_not_leak_rolled_back_block_outputs` **[RED — PRIMARY]**; RB `inc_i_156_req003_oracle_clear_utxos_then_replay_reproduces_canonical` **[LOCK, oracle]** | (architect) | ✅ `rollback_one_block` legacy branch @ `bins/node/src/node/rollback.rs:201` |
| REQ-I156-004 | Must | M1 | RG `inc_i_156_req004_legacy_reorg_must_not_leak_rolled_back_range_outputs` **[RED]** | (architect) | ✅ `execute_reorg` legacy branch @ `bins/node/src/node/block_handling.rs:809` |
| REQ-I156-005 | Must | M2 | (test-writer, M2) | (architect) | `bins/node/src/node/rewards.rs` |
| REQ-I156-006 | Must | M1+M2 | M1: SC `clear_on_failing_rocksdb_returns_err_not_panic` (full content snapshot via `content()` — outpoints + amounts + owners, not counts) **[RED on O5]**. M2 portion pending. | (architect) | both |
| REQ-I156-007 | Must | M1+M2 | RB `inc_i_156_req007_undo_based_rollback_state_root_unchanged` **[LOCK]**; RG `inc_i_156_req007_undo_based_reorg_utxo_state_unchanged` **[LOCK]**. M2 portion pending. | (architect) | both |
| REQ-I156-008 | Should | M1 | n/a (doc comment; its post-condition is executable as REQ-I156-001) | (architect) | ✅ `set.rs:64-71` — the falsified comment is deleted; the replacement states the post-condition, names `StateDb::clear_utxos`, and makes NO claim about which call sites exist |
| REQ-I156-009 | Should | M2 | n/a | (architect) | `specs/engine-parts.md` |
| REQ-I156-010 | Should | M1 | `inc_i_156_m1_harness::assert_utxo_invariants` — invoked by all 5 node tests; plus SC `clear_empties_the_set_rocksdb_variant` and SC `clear_on_already_empty_rocksdb_is_ok_noop` | (architect) | ✅ test-only; verified green — `clear()` empties the backend, it never replaces the variant (no `*self = …` in `set.rs:72-80`) |
| REQ-I156-011 | Could | M2 | (test-writer, M2) | (architect) | memory.db |
| REQ-I156-012/013/014 | Won't | — | — | — | — |

### M1 red-phase status (measured on base f4e6ea69)

| File | RED | LOCK (green pre-fix) |
|---|---|---|
| SC | 3 | 2 |
| RB | 1 | 2 |
| RG | 1 | 1 |

**Note on the RED shape.** All 5 red tests fail on an ASSERTION, not on a compile error.
The `clear()` signature change would normally make SC fail to compile — a weak signal that
hides every behavioural assertion behind a type mismatch. SC therefore carries a
`ClearOutcome` adapter trait (implemented for both `()` and `Result<(), StorageError>`, with
an associated `RETURNS_RESULT` const) so the file compiles on both sides of the fix; the
signature requirement is then pinned as its own runtime assertion.

---

## 12. Triage Verdict

━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.92, verified — both defects read directly at file:line on this branch; both fix primitives already exist and are tested; blast radius enumerated exhaustively by grep over crates/ and bins/)
Reasoning: Two localized, already-root-caused precondition defects with existing tested primitives (StateDb::clear_utxos, BlockStore::ensure_blocks_present) and a 2/5 brittleness score — no architectural investigation required.
━━━━━━━━━━━━━━━━━━━━━━

**Code did not contradict the prior INC-I-152 findings** — it sharpened them in three ways, none of
which argues for DEEP: (1) `rebuild_producer_set_from_blocks` lives in `rewards.rs:1105`, not
`fork_recovery.rs`; (2) R1 has a second leaking call site at `block_handling.rs:803`; (3) R2's harm is
destroy-then-abort with next-block persistence, not a silently-wrong set. All three are absorbed by
the same two fixes.
