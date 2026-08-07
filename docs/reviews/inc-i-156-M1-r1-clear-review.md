# Code Review — INC-I-156 M1 (R1: honest `UtxoSet::clear()`)

```
━━━ FINDINGS — 5 total (Major:2 Minor:3) ━━━

  [F1] MAJOR conf(0.85, observed) — bins/node/src/node/block_handling.rs:815-817 — the replay that now follows a REAL clear still swallows block-read failures with `.ok().flatten()`; a read Err converts a formerly-masked defect into silent permanent UTXO loss
  [F2] MINOR conf(0.95, measured) — crates/storage/src/utxo/set.rs:64-68 — the NEW doc post-condition "no secondary index row survives" is false: `cf_unique_id` survives, and `UtxoSet::has_unique_id` (set.rs:120-125) still returns true after `clear()`. REQ-I156-008 partially unmet
  [F3] MAJOR conf(0.85, observed) — bins/node/src/node/init.rs:317 vs :377 — PRE-EXISTING: the genesis-reset branch replaces the production `RocksDb` UtxoSet with an `InMemory` one and never restores it until restart (variant swap, the exact shape the M1 harness guards against)
  [F4] MINOR conf(0.90, observed) — crates/storage/src/state_db/writes.rs:84-99 — `clear_utxos()` builds ONE unbounded in-memory WriteBatch over the whole UTXO set; this cost is genuinely NEW because the path was a no-op
  [F5] MINOR conf(0.95, measured) — crates/storage/src/utxo/set.rs (pre-fix comment) + bins/node/src/node/init.rs:105-123 — the `init.rs` call site is `recover_body_gaps` (STARTUP body-gap recovery), NOT the genesis-reset path; the pre-fix comment was false on BOTH of its clauses

  Speculative: 1 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

REVIEW VERDICT: APPROVED
```

**Approval qualifier (non-bouncing):** F2 is a one-line doc correction serving REQ-I156-008 (the
requirement whose purpose is "the comment must be TRUE") — apply before commit, no re-review needed.
F1 and F3 are tracked to M2 / a new incident; rationale in each finding.

```
━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      +O(N) per legacy-rebuild invocation, N = live UTXO count (observed — writes.rs:85-98 iterates cf_utxo and cf_utxo_by_pubkey once each; previously this arm did zero work)
  Memory:   +~130-150 bytes x N transient (observed — one WriteBatch entry per key: Outpoint key 36B in cf_utxo + 68B composite key in cf_utxo_by_pubkey, plus rocksdb per-record framing; at N=1e6 that is ~150 MB held until db.write returns)
  IO:       +1 WriteBatch commit + 2 full CF scans per invocation (observed — writes.rs:99 `self.db.write(batch)`; scans are block-cache/SST reads)
  Network:  N-A (single-node storage operation, no peer traffic)
  Disk:     +2N tombstones written, reclaimed by background compaction (observed — delete_cf x2 per key; INV-STORAGE-108 forbids synchronous compact_range here and none is added)
  Latency:  +full-CF-scan + batch-commit added INSIDE the `utxo_set` write guard (observed — rollback.rs:190 / block_handling.rs:799 hold the guard; marginal against the O(chain-length) replay that already runs in the same scope)
Inevitability: AVOIDABLE
Cheaper alternative: `delete_range_cf` (or a chunked WriteBatch flushed every K keys) would wipe both CFs in O(1) app-side memory instead of O(N)
Why this proposal anyway: `clear_utxos()` is PRE-EXISTING, already exercised by `clear_and_write_genesis` on the same CFs, and already covered by tests (disk_guardian_failsafe_test.rs:283-296, :401-407). Reusing it is the minimal root-cause fix; introducing a new deletion primitive inside a P2 bugfix would add an untested storage path to a consensus-state wipe. The O(N) memory spike is recorded as F4 for M2.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

**Boundedness answer (explicitly requested):** the WriteBatch is bounded only by the live UTXO count.
It is NOT bounded by rollback depth, chain length, or any constant. On a node with a large mainnet
UTXO set this is a transient allocation proportional to the whole set, held under the `utxo_set` write
guard, and it is not capped by `db_write_buffer_size` (INV-STORAGE-001) — a WriteBatch is built
application-side, before any memtable. Reachability is narrow: only the legacy no-undo rebuild
branches, additionally gated by `ensure_blocks_present(1, target_height)` requiring the ENTIRE chain
dense in block_store, so on an archiving node the path is refused before it allocates. Not blocking.

**SECURITY AUDIT VERDICT: AUDIT-REQUIRED**

The diff changes when and whether the durable UTXO set — the ledger's balance/supply state — is
wiped, on paths reachable from user-submittable transactions (block-poison rollback,
`production/mod.rs:609`) and from adversarial producer behaviour (fork-driven reorg,
`block_handling.rs:333`). That is squarely "state integrity: financial calculations, balance
operations, multi-step transactions" plus an external-data trust boundary (peer-supplied blocks drive
the reorg). A defect here is a silent supply/inflation change, which is the INC-I-041 class. Even
though the fix moves state toward canonical, the full 5-auditor sweep is warranted.

---

## Scope Reviewed

Diff vs base `f4e6ea69` in worktree `.claude/worktrees/bugfix+inc-i-156-p2-residual-guards`:

| File | Read | Note |
|---|---|---|
| `crates/storage/src/utxo/set.rs` | full impl block + `is_rocksdb` | signature + RocksDb arm + doc |
| `crates/storage/src/state_db/writes.rs:80-102` | full | UNCHANGED — the delegate |
| `bins/node/src/node/rollback.rs:140-270` | full | legacy no-undo rebuild |
| `bins/node/src/node/block_handling.rs:585-900` | full | `execute_reorg` guard + legacy rebuild |
| `bins/node/src/node/init.rs:56-150, 310-420` | full | `recover_body_gaps` + genesis reset |
| 3 new test targets + harness | structure + assertions | 2308 lines total |
| 2 adapted test files | full diff | mechanical only |
| analysis §4.1/§4.2/§4.5/§8/§9, QA §1-§2 + FINDINGS | targeted | |

Not reviewed: M2 surface (`rewards.rs` rebuild), full workspace suite (spot-checked per §6).

---

## 1. Root cause: CLOSED, not patched

The pre-fix `UtxoSet::RocksDb` arm was an empty match arm. The fix routes it to
`StateDb::clear_utxos()` (`set.rs:78` → `writes.rs:80-102`), which I read directly: it deletes every
key in `CF_UTXO` and every key in `CF_UTXO_BY_PUBKEY` in a single `WriteBatch`, `?`s on
`db.write(batch)`, and only then stores `utxo_count = 0` (`writes.rs:100`) — so a failed write cannot
leave a zeroed counter over a populated CF.

This is the correct minimal fix and NOT a paper-over. Evidence it is the root cause and not a symptom:
- The defect is a *missing implementation*, not a missing guard. There is no upstream condition to
  fix — the arm simply did nothing.
- The delegate already exists, is already exercised on the same two CFs by `clear_and_write_genesis`
  (`writes.rs:109-130`), and already has success + failing-DB coverage
  (`crates/storage/tests/disk_guardian_failsafe_test.rs:283-296`, `:401-407`).
- No new abstraction, no new flag, no activation gate, no state field. 22 lines net in `set.rs`.

**SSF check: PASS.** I could not construct a simpler fix that resolves the root cause.

**Fix/cause alignment check: PASS.** The stated cause ("RocksDb arm is a silent no-op") and the change
("RocksDb arm now wipes") are the same statement. No contradiction between commit intent and diff.

---

## 2. Unintended behaviour changes — the three call sites traced

### 2a. `init.rs:118` (`recover_body_gaps`) — the STARTUP path. Scrutinised hardest. SAFE.

`recover_body_gaps` is called at `init.rs:411` inside `Node::new()` with `?`. An `Err` there **aborts
node startup**. So the `?` added at `:118` is the one genuinely dangerous edit in the diff.

It is provably inert:
- `:117` fences the branch behind `if !utxo_set.is_rocksdb()`.
- `is_rocksdb()` is `matches!(self, UtxoSet::RocksDb(..))` (`set.rs:538-540`) — total, no side effects.
- Inside the fence only `UtxoSet::InMemory` is reachable, and that arm is
  `{ store.clear(); Ok(()) }` (`set.rs:74-77`) — it returns `Ok` unconditionally with no fallible
  call in it.

Therefore `utxo_set.clear()?` at `:118` can never yield `Err`, and **no new startup-abort path is
created**. The developer's comment says exactly this and the comment is accurate.

Equally important: the fence is now LOAD-BEARING in a way it was not before. Pre-fix, removing the
fence would have been harmless (no-op either way). Post-fix, removing it would WIPE the live
production UTXO set at startup and then re-insert from `state_db.iter_utxos()` — reading the store it
just emptied. The developer's comment flags this explicitly. I verified there is no other
`is_rocksdb()` caller anywhere in `bins/node/src` or `crates/storage/src` that could be refactored
away by accident (single call site, `init.rs:117`).

### 2b. `rollback.rs:201` — legacy no-undo rebuild. SAFE, error handling correct.

Callers: `periodic.rs:718` (`ShallowRollback` recovery, `?` into `run_periodic_tasks`, whose Err is
consumed by `if let Err(e)` at `event_loop.rs:92` and `:147` — logged, loop continues) and
`production/mod.rs:609` (block-poison, `match` → logs `[BLOCK_POISON] Rollback failed` and returns the
original error; the node keeps running, that block is not broadcast).

Neither caller halts the node. An `Err` from the new `clear()` therefore degrades to "this rollback
did not happen, loudly" — with UTXO, chain_state and producer_set all untouched, because the `?` fires
BEFORE any mutation in this function (chain_state is written later, at `:256-261`). Ordering here is
correct and strictly better than the reorg sibling. No behaviour change on the undo-based path, which
never calls `clear()` (verified: `rollback.rs:100-139` uses per-outpoint `remove`/`insert`).

### 2c. `block_handling.rs:809` — `execute_reorg` legacy rebuild. See Residual 1 below.

Callers: `block_handling.rs:333` (`if let Err(e)` — logged, non-fatal), `wedge_escape.rs:164`,
`fork_recovery.rs:75` and `:120` (all `?` into their own non-fatal recovery drivers). No node halt.

### 2d. Density guard confirmed intact (analysis §4.5 asked the reviewer to confirm)

`block_handling.rs:599-615`: `ensure_blocks_present(1, target_height)` runs BEFORE any mutation and
covers BOTH the undo and legacy branches. `rollback.rs:169-181` carries the sibling guard with
`.max(1)`. Both intact, unmodified by M1. **Confirmed.**

---

## 3. INV-12 CONSENSUS-SHAPE CHECKLIST

The analyst's classification (analysis §4.1): Q1 YES, Q2 YES, Q3 NO → literal rule says "activation
height REQUIRED"; assessed result "NOT required" on the argument that the pre-fix output is never
canonical, with the premise flagged for reviewer attack.

**My answers:**

**Q1 — Can a user-submittable tx reach this path? YES.** `production/mod.rs:595-622`: a transaction
that passes builder validation but fails apply validation triggers `rollback_one_block()`. A user can
initiate a rollback; the user cannot select the *legacy* branch (that depends on undo availability).

**Q2 — Can a producer action or attestation pattern reach it? YES.** Fork-producing or equivocating
producers drive `execute_reorg` (`block_handling.rs:333`) and `ShallowRollback`
(`periodic.rs:716-722`).

**Q3 — Is the new behaviour bit-identical for ALL reachable inputs? NO** (for the subset of inputs
where the rolled-back range left surviving outputs).

**Literal rule: (Q1|Q2) YES + Q3 NO ⇒ activation height REQUIRED.
Assessed: NOT required. I AGREE with the analyst — and I strengthen the argument, because as written
it rests on a premise the analyst himself marked unverified.**

The analyst's exemption depends on assumption #3 (§9): "every block carries at least one
coinbase/reward output, so the leak set is never empty." He records it as **not directly re-verified**.
That dependency is unnecessary. Partition every reachable input:

- **(a) the rolled-back range left ≥1 surviving output.** Pre-fix state =
  `canonical(target) ∪ residual`, which differs from every peer that did not take this path — i.e.
  pre-fix is non-canonical. Post-fix moves toward canonical. No *new* disagreement class is created;
  an existing one shrinks.
- **(b) the rolled-back range left 0 surviving outputs.** Then `clear()` + replay(1..=target) and
  no-clear + replay(1..=target) produce the same set: the replay re-adds every output created in
  `1..=target` (`add_transaction`, upsert semantics via `insert_utxo`, `writes.rs:32-46`) and re-spends
  every one spent in `1..=target`; outputs created in `1..=target` but spent inside the rolled-back
  range are restored by the replay in both cases. Pre-fix and post-fix are **identical**.

So for every reachable input the fix either leaves the state unchanged or replaces a provably
non-canonical state with a closer-to-canonical one. It can never turn a canonical pre-fix state into a
different state. **The exemption holds independently of assumption #3.** That is a stronger footing
than the analysis had, and it retires the analyst's own "⚠ reviewer attention requested" flag.

Nothing in the diff touches a validation predicate, `active_producers`, scheduler input, bond
snapshot, bitfield encoding, coinbase shape, or `presence_root`. Same classification INC-I-152's
`rollback.rs:169` guard shipped under, on this same code path.

**Deploy Q1 — does this change consensus RULES? NO.** No validation predicate, no `NetworkParams`
field, no `HardForkSchedule` entry, no `CURRENT_PROTOCOL_VERSION` / `EPOCH_STATE_FORMAT_VERSION` /
`MIN_PEER_PROTOCOL_VERSION` change (verified: none of these identifiers appear in the diff).
**⇒ no activation height.**

**Deploy Q2 — does this change block CONTENT? NO.** Nothing in the diff reaches the bitfield
encoder/decoder, coinbase construction, transaction ordering, `presence_root`, or any header field. A
node with correct state produces byte-identical blocks before and after. **⇒ rolling restart is safe;
no synchronized deploy.** Mixed-fleet is safe: patched and unpatched nodes differ only when the
unpatched one has taken a legacy rebuild path, in which case it was already divergent.

**I AGREE with the analyst on both deploy questions.** No version bump required (and none is in the
diff — Cargo.toml untouched).

---

## 4. Residual 1 (`execute_reorg` ordering) — INDEPENDENT EVALUATION: **AGREE with QA, ACCEPTABLE-AS-IS**

**The facts, re-derived from code:** `block_handling.rs:797-809` opens a scope taking BOTH the
`chain_state` and `utxo_set` write guards, writes `state.best_height/best_hash/best_slot` at
`:800-802`, and only then calls `utxo.clear()?` at `:809`. On `Err`, both guards drop and the function
returns `Err` with chain_state rewound in memory and the UTXO set untouched — and `atomic_replace`
(`:889-891`) never runs, so nothing is persisted.

**QA's argument, tested rather than accepted:** QA claims the same partial state is already reachable
via three other `?` in the same lock scope, pre- and post-fix. I verified the claim by enumerating the
fallible operations inside that scope *after* `:800-802`:
- `utxo.add_transaction(tx, height, is_reward_tx, block.header.slot)?` (`:826`) — on the RocksDb
  variant this is `sdb.insert_utxo(...)?` (`set.rs:165`), fallible on any RocksDB write error;
- `Self::consume_genesis_bond_utxos(...)?` (`:831-836`);
- the same `add_transaction` again for each of N transactions.

So yes: **pre-fix, an `Err` after `:800-802` was already reachable, and the resulting state was
strictly WORSE** — chain_state rewound *and* the UTXO set half-rebuilt, versus post-fix chain_state
rewound and the UTXO set completely untouched. The two failures share one environmental cause (a
RocksDB write failure), so the new `Err` adds no new trigger class.

Weighing the alternative to shipping: pre-fix on a production RocksDb node this path leaked the ENTIRE
rolled-back range on **100% of executions**, silently. Post-fix it is correct except under a RocksDB
write failure that would have corrupted the same reorg anyway. Blocking M1 to reorder three lines
would preserve a guaranteed-corruption path in order to avoid a rarer, louder one. **I agree with QA.**

**But the reorder is still right, and it is cheap.** The replay loop and `consume_genesis_bond_utxos`
never read `chain_state`; I checked — no use of `state` between `:802` and the end of the scope.
Moving the three assignments to immediately after the `clear()?` is therefore behaviour-preserving on
the success path and strictly better on the failure path:

```rust
// concrete fix (M2): move these three lines to AFTER the utxo.clear()? call
state.best_height = target_height;
state.best_hash   = common_ancestor_hash;
state.best_slot   = common_ancestor_slot;
```

**Recorded as an M2 item, not a blocker.**

---

## 5. Findings (bodies)

### [F1] MAJOR — replay after a real clear still swallows read failures
- **Location:** `bins/node/src/node/block_handling.rs:815-817`
- **Evidence:** `if let Some(block) = self.block_store.get_block_by_height(height).ok().flatten()`.
  `.ok()` discards an `Err` (RocksDB read failure); `.flatten()` discards `None`. Either way the loop
  **silently continues to the next height**. The sibling in `rollback.rs:209-217` does the opposite —
  `.ok_or_else(|| anyhow!("Rollback UTXO rebuild: missing block at height {}"))?`.
- **Why M1 changes its weight:** pre-fix the set was never cleared, so a skipped block's outputs
  survived in the stale set and the skip was masked. Post-fix the set is genuinely empty, so a skipped
  block's outputs are **permanently absent** from the rebuilt set and then laundered to disk by
  `atomic_replace` (`:889-891`). The consequence flips from "stale" to "silently short". This is also
  internally inconsistent with M1's own stated philosophy ("a failed wipe must abort") — the wipe is
  now fail-loud while the very next loop is fail-silent.
- **Severity rationale (why not blocking):** reachability is narrow. `ensure_blocks_present(1,
  target_height)` at `:599` proves density before the scope opens, so only a read error or a
  concurrent store mutation between the guard and the replay can trigger it. Shipping M1 removes a
  100%-of-executions corruption; this finding is a rarer one. Net risk strictly decreases.
- **Suggested fix (M2):** mirror `rollback.rs:212-217` — replace `.ok().flatten()` with
  `?` + `ok_or_else(...)` so a missing/unreadable block aborts the reorg instead of shortening the set.
- **Confidence:** conf(0.85, observed)

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — replaces two combinator calls with a `?`; identical work on the success path)
  Memory:   0 (observed — no new allocation; the error string is built only on the failure path)
  IO:       0 (observed — same single get_block_by_height call)
  Network:  N-A (local storage read)
  Disk:     0 (observed — the change only prevents a write that would have been wrong)
  Latency:  0 (observed — success path unchanged)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: correctness only; it converts silent state loss into a visible abort at zero steady-state cost.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### [F2] MINOR — the NEW doc comment contains a false clause (REQ-I156-008 partially unmet)
- **Location:** `crates/storage/src/utxo/set.rs:64-68`
- **Evidence:** the comment asserts "…`len() == 0`, and **no secondary index row survives**".
  `StateDb::clear_utxos` touches exactly two CFs — `CF_UTXO` and `CF_UTXO_BY_PUBKEY`
  (`writes.rs:81-98`). A third UTXO-adjacent index exists: `CF_UNIQUE_ID`
  (`crates/storage/src/state_db/types.rs:51`), read through `StateDb::has_unique_id`
  (`queries.rs:314-321`) and **exposed on the UtxoSet façade itself** at `set.rs:120-125`
  (`UtxoSet::has_unique_id`). After a successful `clear()`, `utxo.has_unique_id(UID_PREFIX_NFT, id)`
  still returns `true` for every id minted before the wipe. The stated post-condition is therefore
  false against the type's own public API.
- **Why it matters beyond pedantry:** REQ-I156-008 exists *because* the previous comment was false.
  Replacing one false comment with a differently-false one does not close that requirement. QA marked
  REQ-I156-008 `PASS`; that grade is over-generous and I disagree with it.
- **Suggested fix (pre-commit, one line):** replace the clause with an explicit scope statement, e.g.
  "…and no `cf_utxo_by_pubkey` row survives. `cf_unique_id` (NFT/pool-id uniqueness) is deliberately
  NOT touched — it is not rolled back on any path (`UndoData` has no unique-id field), so wiping it
  here would make previously-minted ids re-mintable."
- **Test strategy:** `crates/storage/tests/inc_i_156_clear_contract_test.rs` — add
  `add_unique_id(UID_PREFIX_NFT, id)` before `clear()`, assert `has_unique_id` still `true` after.
- **Confidence:** conf(0.95, measured)

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — comment text only, no codegen)
  Memory:   0 (observed — comment text only)
  IO:       N-A (comment-only change)
  Network:  N-A (comment-only change)
  Disk:     0 (observed — source bytes only)
  Latency:  0 (observed — no runtime path affected)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: REQ-I156-008 requires the comment to be true; a false post-condition on a state-wipe primitive is how INC-I-156 started.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### [F3] MAJOR (PRE-EXISTING, not caused by M1) — genesis reset swaps the production UtxoSet variant
- **Location:** `bins/node/src/node/init.rs:317` vs `bins/node/src/node/init.rs:377`
- **Evidence:** `:317` builds the production variant — `let utxo_set = UtxoSet::from_state_db(state_db.clone());`
  (RocksDb). The genesis-reset branch at `:377` then does `*utxo_set.write().await = UtxoSet::new();`
  and `UtxoSet::new()` returns `UtxoSet::InMemory(InMemoryUtxoStore::new())` (`set.rs:41-43`). Nothing
  between `:377` and the end of `Node::new()` restores `from_state_db` (the only other `UtxoSet::new()`
  sites are `:1135` and `:1344`, both `new_for_test`). A production node that takes a genesis reset
  therefore runs until the next restart with a detached in-memory UTXO set alongside its durable
  `state_db`.
- **Why it belongs in this review:** it is the exact hazard the M1 harness was written to catch —
  `bins/node/tests/inc_i_156_m1_harness/mod.rs:317-337` warns that "a fix that rebuilt into a scratch
  `InMemory` set and published it would satisfy every content assertion while silently detaching the
  production backend", and asserts `is_rocksdb()`. `init.rs:377` already does that in production. It
  also plausibly contributes RAM growth (a full second copy of the UTXO set), which is live context in
  `docs/bugfixes/family-ram-growth-architecture-context.md`.
- **Not M1's fault, not M1's job:** the line is untouched by this diff and the behaviour is identical
  pre- and post-fix. Do NOT expand M1 to cover it.
- **Suggested fix (new incident):** `*utxo_set.write().await = UtxoSet::from_state_db(state_db.clone());`
  after `clear_and_write_genesis`, plus a regression assertion that `is_rocksdb()` holds across a reset.
- **Test strategy:** drive `Node::new()` through the genesis-reset branch with a mismatched chainspec
  genesis and assert `node.utxo_set.read().await.is_rocksdb()`.
- **Confidence:** conf(0.85, observed)

```
━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      0 (observed — one enum assignment at startup, replacing another)
  Memory:   -O(N) after reset (inferred — removes a full duplicate in-memory UTXO set that the InMemory variant accumulates as the node re-syncs)
  IO:       +per-UTXO-read routed to RocksDB instead of a HashMap (inferred — reads served by state_db block cache rather than heap)
  Network:  N-A (single-node startup path)
  Disk:     0 (observed — apply_block already writes the same BlockBatch either way)
  Latency:  +small per UTXO read after a reset (inferred — RocksDB point-get vs HashMap get; identical to every non-reset node's steady state)
Inevitability: AVOIDABLE
Cheaper alternative: leave as-is and accept a detached in-memory set until the next restart
Why this proposal anyway: the cheaper path is the defect — it violates the single-source-of-truth for the UTXO set and doubles resident memory on a path already implicated in the family RAM-growth investigation.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### [F4] MINOR — unbounded WriteBatch on a path that used to cost nothing
- **Location:** `crates/storage/src/state_db/writes.rs:84-99` (unchanged code, newly reachable from
  `crates/storage/src/utxo/set.rs:78`)
- **Evidence:** one `rocksdb::WriteBatch::default()` accumulates `delete_cf` for **every** key in
  `cf_utxo` and **every** key in `cf_utxo_by_pubkey` before a single `db.write(batch)` at `:99`. Key
  sizes: `Outpoint::to_bytes()` = 36 B; the by-pubkey composite is 32 B pubkey_hash + 36 B outpoint =
  68 B (`writes.rs:38-41`). App-side memory is therefore ~130-150 B x N with framing.
- **Impact:** a transient O(N) allocation held under the `utxo_set` write guard, on nodes whose memory
  budget is already tight (INC-I-150: 3.8 GiB family host). Not a regression of a previously-cheap
  operation — the operation previously did not exist on this arm — which is exactly why it must be
  stated rather than inherited silently.
- **Suggested fix (M2, optional):** chunk the batch (flush every K keys) or use `delete_range_cf`.
  Both change durability shape (no longer one atomic wipe), so this needs its own analysis — do not
  bundle it into M1.
- **Test strategy:** NOT_TESTABLE as a unit assertion (memory-ceiling behaviour); measurable via a
  benchmark that clears a synthetic N=1e6 set and samples RSS.
- **Confidence:** conf(0.90, observed)

*(Resource cost for F4's own fix is deferred with the fix — the finding as filed proposes analysis,
not a change; the cost of the change under review is stated in the report-level block above.)*

### [F5] MINOR — the pre-fix comment misdescribed its own call sites; the milestone brief inherited it
- **Location:** `crates/storage/src/utxo/set.rs` (pre-fix comment, now deleted) and
  `bins/node/src/node/init.rs:105-123`
- **Evidence:** the deleted comment claimed `clear()` on the RocksDb variant "is only called during
  genesis reset (init.rs), which immediately replaces the UtxoSet with a fresh InMemory variant
  anyway." Both clauses are false: (i) it was also called from `rollback.rs:191` and
  `block_handling.rs:803`; (ii) the genesis-reset branch (`init.rs:373-377`) never calls `clear()` at
  all — it assigns `UtxoSet::new()`. The `init.rs` call site that M1 actually touches is inside
  `recover_body_gaps` (`init.rs:56-150`), the **startup body-gap recovery** path, not genesis reset.
- **Why filed:** the M1 task framing repeats the same mislabel ("init.rs genesis-reset path"). The
  mislabel matters because the two paths have opposite risk profiles: `recover_body_gaps` runs inside
  `Node::new()` where an `Err` aborts startup (analysed in §2a); genesis reset does not call `clear()`
  at all. Anyone reasoning from the label rather than the code would scrutinise the wrong function.
  The new comment does not repeat the error — it is correct on this point.
- **Suggested fix:** none in code. Correct the label in the M1 commit message / M2 brief.
- **Confidence:** conf(0.95, measured)

---

## Speculative Findings (low-confidence, not actionable)

- **conf(0.55, inferred)** — `bins/node/src/node/block_handling.rs:599` → `:815`: a concurrent
  archiver prune between `ensure_blocks_present` and the replay could open F1's failure window without
  any read error. I did not verify the archiver's prune predicate or whether it can run concurrently
  with `execute_reorg`, and the full-chain density requirement makes the legacy path unreachable on an
  archiving node in the first place. Reported so F1's fix is not scoped to read errors alone.

---

## 6. Tests — verified with real command output, not narration

All commands run in the worktree.

| Command | Result |
|---|---|
| `cargo test -p storage --test inc_i_156_clear_contract_test` | **5 passed / 0 failed** |
| `cargo test --test inc_i_156_m1_rocksdb_clear_leak` | **3 passed / 0 failed** |
| `cargo test --test inc_i_156_m1_reorg_clear_leak` | **2 passed / 0 failed** |
| `cargo clippy --workspace --all-targets -- -D warnings` | **exit 0**, 0 warnings |
| `cargo fmt --check` | **exit 0** |

10/10 new tests green. I did not re-run the full workspace suite; QA's 3164/3/43 with the 3 failures
verified pre-existing on `f4e6ea69` stands unchallenged, and nothing in this diff plausibly touches
them (the diff is confined to one storage method and three call sites, all compile-checked).

**Test quality — real proofs, not tautologies:**
- The harness does the thing that justifies its existence: `Node::new_for_test` leaves an `InMemory`
  set, so `inc_i_156_m1_harness/mod.rs:69` swaps in `UtxoSet::from_state_db(node.state_db.clone())`
  and `:71-73` **asserts** `is_rocksdb()` before the scenario runs. Without that swap the bug is
  inexpressible — the `InMemory` arm was always honest. This is the correct instrument.
- `assert_utxo_invariants` (`mod.rs:321-338`) additionally re-asserts `is_rocksdb()` **after** the
  scenario, closing the "rebuilt into a scratch InMemory set and published it" cheat that would satisfy
  every content assertion. That is a genuinely adversarial assertion, not decoration.
- The tests would fail if the fix were reverted. QA proved it by neutralising `set.rs:78` to `Ok(())`
  — the faithful pre-fix semantics with the post-fix signature — and observed 2/1/1 targeted failures
  with the happy-path and ORACLE tests staying GREEN (i.e. not a blanket break). **I confirm the
  method is sound**: reverting the arm body while keeping the signature is the only way to test-revert
  this change, since the pre-fix `-> ()` signature makes
  `clear_returns_a_result_that_cannot_be_swallowed` fail to compile rather than fail to pass. I did not
  re-run the neutralisation myself — a reviewer must not edit source — and I accept QA's md5-verified
  restore (`f4ab3231ccb304684fa22c94f349484d`) as evidence the tree was returned intact.
- `inc_i_156_req003_oracle_clear_utxos_then_replay_reproduces_canonical` is a real oracle: it applies
  `StateDb::clear_utxos()` directly and shows the replay reproduces the canonical set, proving the
  delegate is the right primitive *before* the façade is trusted.
- The two adapted test files are mechanical `?`/`.expect()` adaptations at 5 sites with **no assertion
  weakened or removed** (verified against the full diff).

---

## 7. Specs / docs drift

- `crates/storage/src/utxo/set.rs:64-68` — see **[F2]**. Not fully closed.
- `specs/engine-parts.md:2795` (rollback legacy fallback described as "clears UTXO set and replays all
  blocks from genesis"; also omits the INC-I-152 `ensure_blocks_present` guard that the sibling entry
  at `:2760` documents). The analysis §8 assigns the correction to **REQ-I156-009 = M2**.
  **Deferral is DEFENSIBLE and I agree**, for a reason worth stating precisely: as of this diff the
  `:2795` sentence has become **TRUE** — the routine now really does clear on both backends. The
  remaining inaccuracy is the *omitted guard*, which is a documentation gap, not a falsehood, and it
  affects both routines symmetrically, so fixing them together in M2 keeps the spec edit coherent.
  Deferring a falsehood would not be acceptable; deferring an omission is.
- No drift in `docs/architecture.md` / `docs/troubleshooting.md` per analysis §8 (not re-verified).

---

## 8. Missed opportunities / went-too-far

**Went too far: nothing.** The diff is 62 insertions / 21 deletions across 6 files. No refactor, no
opportunistic cleanup, no signature change beyond the one the fix requires. `writes.rs` was correctly
left untouched. The `init.rs` fence was correctly left in place rather than "simplified".

**Left undone that arguably belonged in M1:** only F2 (one line). F1, F3 and F4 are correctly outside
a minimal root-cause fix.

**Pre-existing, noted only, no action demanded in this bugfix (Global Rule 19):**
`crates/storage/src/utxo/set.rs` = 558 lines and `bins/node/src/node/block_handling.rs` = 985 lines
both exceed the 500-line budget. **Both were already over on base `f4e6ea69`** and M1 adds 1 net line
to `set.rs` and 13 to `block_handling.rs` (all comment + error-mapping). Do not split in a bugfix.

**Cross-cutting scan** (`unwrap()` / `TODO` / `HACK` / bare swallows in the diff): the only error
swallows introduced or left in the changed hunks are the pre-existing `let _ = utxo_set.insert(...)`
at `init.rs:120` (inside the InMemory-only fence, unchanged) and F1's `.ok().flatten()`. No new
`unwrap()`, no `TODO`, no `unsafe`. No injection surface: no SQL, no shell, no `format!` reaching an
interpreter — the diff is pure Rust storage/state code.

---

## Final Verdict

**APPROVED for commit**, with the F2 one-line doc correction applied first (no re-review needed).
F1 and Residual 1's reorder go to **M2**. F3 warrants its **own incident** — it is pre-existing and
must not be folded into INC-I-156's scope.

The root cause is closed rather than patched, the startup path is provably unchanged, the
consensus-shape exemption holds on stronger footing than the analysis claimed, and the tests are
adversarial rather than confirmatory.
