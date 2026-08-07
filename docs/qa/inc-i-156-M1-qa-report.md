# QA Report — INC-I-156 M1 (R1: honest `UtxoSet::clear()`)

```
━━━ FINDINGS ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

QA VERDICT: PASS

Scope     : INC-I-156 M1 — REQ-I156-001, -002, -003, -004, -008, -010
Worktree  : .claude/worktrees/bugfix+inc-i-156-p2-residual-guards @ base f4e6ea69
Milestone : M1 of 2 (M2 = R2 producer-set rebuild guard, NOT validated here)

REQUIREMENTS
  REQ-I156-001  honest clear() on both variants ............. PASS
  REQ-I156-002  Result signature, no swallowed error ........ PASS (doc says "4 sites"; actual = 3)
  REQ-I156-003  legacy rollback leak closed ................. PASS
  REQ-I156-004  legacy reorg leak closed .................... PASS
  REQ-I156-008  doc-comment truth ........................... PASS
  REQ-I156-010  is_rocksdb() still true post-rollback ....... PASS

ROOT CAUSE  : CLOSED, not patched. RocksDb arm delegates to StateDb::clear_utxos()
              (writes.rs:80-102) which deletes cf_utxo AND cf_utxo_by_pubkey in one
              WriteBatch and stores utxo_count = 0.
              evidence: crates/storage/src/utxo/set.rs:78 -> writes.rs:80-102 (read, not assumed)

RED PROOF   : INDEPENDENTLY RE-VERIFIED by neutralizing set.rs:78 to `Ok(())`
              (pre-fix semantics) and re-running all 3 targets:
                storage contract .... 3 passed / 2 FAILED
                rollback leak ....... 2 passed / 1 FAILED
                reorg leak .......... 1 passed / 1 FAILED
              Restored byte-identical (md5 f4ab3231ccb304684fa22c94f349484d, verified).
              The happy-path (REQ-007) and ORACLE tests stayed GREEN under
              neutralization -> the failures are targeted, not blanket.

TESTS       : 10/10 GREEN on the fix. Adjacent: fork_recovery 11/11, recover_replay 4/4,
              inc_i_064_supply_conservation 6/6, inc_i_152_p1_003 3/3, storage --lib 245/245,
              disk_guardian_failsafe 19/19, doli-node --lib 55/55. No new failures.
GATE        : cargo clippy --workspace --all-targets -D warnings -> exit 0, 0 warnings.
              cargo fmt --check -> exit 0.

RESIDUAL 1 (execute_reorg ordering) : ACCEPTABLE-AS-IS
              The partial state is pre-existing in kind (reachable via 3 other `?`
              in the same lock scope, pre- and post-fix) and post-fix is a STRICT
              IMPROVEMENT on it. See §6.

BLOCKING ISSUES : none
OBSERVATIONS    : 4 (1 medium latent, 3 low) — see §8
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

---

## 1. Scope validated

M1 only: `REQ-I156-001, -002, -003, -004, -008, -010`.
Files: `crates/storage/src/utxo/set.rs`, `crates/storage/src/state_db/writes.rs` (read-only —
unchanged by M1), `bins/node/src/node/rollback.rs`, `bins/node/src/node/block_handling.rs`,
`bins/node/src/node/init.rs`.

**Not validated (out of M1 scope):** REQ-I156-005, -006, -007(full-suite clause), -009, -011
(M2 / follow-up). REQ-I156-012/-013/-014 are declared `Won't` by the analysis §7.

## 2. System entrypoint

DOLI is a node binary, not a service QA can meaningfully "start" for this defect: the R1 leak
is only observable on the **RocksDb** `UtxoSet` variant through the legacy no-undo
rollback/reorg branches, which require an engineered undo-gap. The test-writer's harness
(`bins/node/tests/inc_i_156_m1_harness/`) is the correct instrument — it builds a real `Node`
on `UtxoSet::from_state_db` (harness `mod.rs:69`) and asserts the variant at `mod.rs:72`.
Validation was therefore performed through:

```
cargo test -p storage   --test inc_i_156_clear_contract_test
cargo test -p doli-node --test inc_i_156_m1_rocksdb_clear_leak
cargo test -p doli-node --test inc_i_156_m1_reorg_clear_leak
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

Package names confirmed from `crates/storage/Cargo.toml` (`storage`) and
`bins/node/Cargo.toml` (`doli-node`) — the brief's guess `-p doli-storage` was wrong.

## 3. Root-cause verification (not taken on trust)

`crates/storage/src/utxo/set.rs:72-80`:

```rust
pub fn clear(&mut self) -> Result<(), StorageError> {
    match self {
        UtxoSet::InMemory(store) => { store.clear(); Ok(()) }
        UtxoSet::RocksDb(sdb) => sdb.clear_utxos(),
    }
}
```

`crates/storage/src/state_db/writes.rs:80-102` read directly. It does exactly what
REQ-I156-001 requires:

| REQ-001 clause | writes.rs evidence |
|---|---|
| `cf_utxo` emptied | `:85-91` iterate `cf_utxo` from Start, `batch.delete_cf` each key |
| `cf_utxo_by_pubkey` emptied | `:92-98` same loop over `cf_by_pk` — **both** CFs, one batch |
| `utxo_count` reset | `:100` `self.utxo_count.store(0, Ordering::Relaxed)` |
| `Err` leaves the set unchanged | `:99` `self.db.write(batch)?` — RocksDB `WriteBatch` is all-or-nothing, and `:100` sits **after** the `?`, so the counter is not desynchronised on failure |

This is a delegation to an already-correct, already-tested primitive
(`disk_guardian_failsafe_test.rs:283`, `:401`) — not a new implementation and not a symptom
patch. The defect was that the RocksDb arm was an **empty block**; the fix removes the
special case entirely.

## 4. Per-requirement results

### REQ-I156-001 — honest `clear()` — **PASS** (Must)

| Acceptance criterion | Evidence |
|---|---|
| RocksDb: `iter_all().is_empty()` + `utxo_count()==0` after `Ok(())` | `inc_i_156_clear_contract_test.rs:264-315`; test `clear_empties_the_set_rocksdb_variant` PASS |
| both `cf_utxo` and `cf_utxo_by_pubkey` empty | `:326-345` asserts `address_count()==0` (reads `cf_utxo_by_pubkey` directly per `queries.rs:281-304` — the one public path that can *see* an orphan row) + `get_by_pubkey_hash()` empty for both seeded pubkeys |
| InMemory unchanged | `clear_empties_the_set_inmemory_variant` PASS (PASS-LOCK; also passed under neutralization → correctly variant-scoped) |
| failing DB → `Err`, no panic | `clear_on_failing_rocksdb_returns_err_not_panic` PASS; FAILED under neutralization |
| one parameterized body for both variants | `assert_clear_empties(set, variant, seeded)` at `:238`, invoked from `:360` and `:387` |

```
running 5 tests
test clear_empties_the_set_inmemory_variant ... ok
test clear_on_already_empty_rocksdb_is_ok_noop ... ok
test clear_returns_a_result_that_cannot_be_swallowed ... ok
test clear_empties_the_set_rocksdb_variant ... ok
test clear_on_failing_rocksdb_returns_err_not_panic ... ok
test result: ok. 5 passed; 0 failed; 0 ignored
```

### REQ-I156-002 — non-swallowable `Result` — **PASS** (Must)

Signature: `pub fn clear(&mut self) -> Result<(), StorageError>` — `set.rs:72`. ✔

**Call-site census (the "4 vs 3" discrepancy — resolved).** Full-value grep of `.clear()`
over `crates/*/src` and `bins/*/src`, every match read and classified. Non-test call sites on
a **`UtxoSet`** receiver:

| # | Site | Handling | Verdict |
|---|---|---|---|
| 1 | `bins/node/src/node/rollback.rs:201` | `.map_err(...)?` | propagates |
| 2 | `bins/node/src/node/block_handling.rs:809` | `.map_err(...)?` | propagates |
| 3 | `bins/node/src/node/init.rs:118` | `?` | propagates |

**Actual count = 3, not 4.** All 3 propagate → the substantive requirement is **met**.

The "4" in the §5 requirements table is a **documentation error**, and the analysis
contradicts itself: its own detailed acceptance criteria (line 575-576) name exactly three
files — "`rollback.rs`, `block_handling.rs`, `init.rs` propagate with `?`" — and its §1.3
enumeration lists 3 call sites plus 2 rows for the two *arms of the definition itself*
(`set.rs:70` and `:71-76`). The most likely origin of "4" is counting the definition as a
site. `set.rs:75` (`store.clear()`) is `InMemoryUtxoStore::clear`, a different type, inside
the definition. Recorded as OBS-001 (doc fix, non-blocking).

**Grep gate (with positive control, per instrument-truth):**

```
grep -rn --include="*.rs" "let _ = .*\.clear()" crates/   -> exit 1 (zero matches)
grep -rn --include="*.rs" "let _ = .*\.clear()" bins/     -> exit 1 (zero matches)
POSITIVE CONTROL: grep "let _ = " bins/node/src/node/init.rs
  -> init.rs:120,130,133  (the instrument DOES match this shape when present)
```

The zero result is therefore evidence of absence, not an inert command. Zero bare
`.clear();`-as-statement on a `UtxoSet` remain (the full census above is exhaustive).

### REQ-I156-003 — legacy rollback leak — **PASS** (Must)

```
test inc_i_156_req003_legacy_rollback_must_not_leak_rolled_back_block_outputs ... ok
test inc_i_156_req003_oracle_clear_utxos_then_replay_reproduces_canonical ... ok
test inc_i_156_req007_undo_based_rollback_state_root_unchanged ... ok
test result: ok. 3 passed; 0 failed; 0 ignored
```

Construction verified against every acceptance clause:
RocksDb variant (`harness/mod.rs:69` `UtxoSet::from_state_db`, asserted `:72`); dense store
(`inc_i_156_m1_rocksdb_clear_leak.rs:341` asserts `ensure_blocks_present(1, TARGET_HEIGHT)`
passes, so the INC-I-152 guard is *not* what refuses the test); undo erased via the production
API `prune_undo_above(0)` (`:329`); assertions taken on the **persisted** view
(`h::persisted_utxo_content`, `:464`) and cross-checked against the façade view (`:550`);
`utxo_count() == iter_all().len()` (INV-GUARD-001) at `harness/mod.rs:324-330`.

### REQ-I156-004 — legacy reorg leak — **PASS** (Must)

```
test inc_i_156_req004_legacy_reorg_must_not_leak_rolled_back_range_outputs ... ok
test inc_i_156_req007_undo_based_reorg_utxo_state_unchanged ... ok
test result: ok. 2 passed; 0 failed; 0 ignored
```

Multi-block range asserted exactly equal to `canonical_at_target` (`:512`), with the
range-created-then-spent outpoint absent (`:495`) and the pre-range spent outpoint restored
(`:484`) — i.e. the test discriminates a real clear from both a no-op and an over-wipe.

### REQ-I156-008 — comment truth — **PASS** (Should)

`set.rs:64-71`. States the post-condition ("after `Ok(())` the set is empty on EITHER
backend"), names `StateDb::clear_utxos` as the RocksDb implementation, names both column
families and `utxo_count`/INV-GUARD-001, states the `Err` contract, and makes **no claim about
which call sites exist** — which was the falsified clause in the old comment. All three false
clauses of the base comment are gone.

### REQ-I156-010 — variant preserved (INV-SYNC-014) — **PASS** (Should)

`is_rocksdb()` asserted post-rollback/post-reorg in `harness/mod.rs:334`, reached from both
node test files. Independently confirmed by reading the code: `clear()` takes `&mut self` on
the enum and never reassigns the discriminant.

## 5. RED-phase independent re-verification

Rather than reason alone, the claim was **falsified empirically without checking out base**.
`set.rs:78` was temporarily replaced with `UtxoSet::RocksDb(_sdb) => Ok(())` — pre-fix
semantics under the post-fix signature — and all three targets re-run:

| Target | Under neutralization | With fix |
|---|---|---|
| `inc_i_156_clear_contract_test` | **3 passed / 2 FAILED** | 5 passed |
| `inc_i_156_m1_rocksdb_clear_leak` | **2 passed / 1 FAILED** | 3 passed |
| `inc_i_156_m1_reorg_clear_leak` | **1 passed / 1 FAILED** | 2 passed |

Actual failure output (rollback target, neutralized):

```
REQ-I156-003 / O2 — THE LEAK: 2 of 2 outputs CREATED by the rolled-back block (h=20)
are STILL PRESENT in cf_utxo after rolling back to h=19 ...
total_value: canonical(19) = 1900098000 | after rollback = 2000195000
           | inflation = 100097000 || utxo count: 20 -> 22
```

The tests are genuinely RED against pre-fix semantics and the failure is the exact
inflation the incident describes. Note which tests **stayed green** under neutralization:
`inc_i_156_req007_*` (happy-path locks) and
`inc_i_156_req003_oracle_clear_utxos_then_replay_reproduces_canonical` (positive control —
it performs the clear manually, proving the assertion is satisfiable only when a real clear
occurs). The signal discriminates; it is not a blanket failure.

`set.rs` was restored from a scratchpad copy and verified byte-identical
(`md5 f4ab3231ccb304684fa22c94f349484d` before and after; zero `QA-PROBE-TEMP` matches
remain; `git status` shows the same 6 modified + 5 untracked paths as before the probe).
All three targets re-run GREEN after restoration.

## 6. Residual 1 (ordering in `execute_reorg`) — **VERDICT: ACCEPTABLE-AS-IS**

**The fact.** `block_handling.rs:797-815`: `chain_state.best_height/best_hash/best_slot` are
mutated at `:800-802`, `utxo.clear()?` is at `:809`. An `Err` from `clear()` therefore unwinds
with in-memory `chain_state` rewound and the UTXO set not cleared. Confirmed by reading the
code. (`rollback.rs` does **not** have this shape: `clear()?` is at `:201`, the `chain_state`
mutation at `:256-261` — clear-first, so its comment "state left unchanged" is accurate.)

**(a) Is it reachable in practice?** Only on a RocksDB write failure (disk full, IO error,
read-only DB, corruption) *and* only in the legacy no-undo reorg branch. Rare. But the crucial
finding is that **this partial state is not new in kind.** Three other `?` operators inside
the same lock scope already produce it, pre- and post-fix:

- `:831` `utxo.add_transaction(...)?` — on the RocksDb variant this is a direct
  `sdb.insert_utxo(...)?` per outpoint (`set.rs:165`), i.e. a `db.write` on the same disk
- `:840` `consume_genesis_bond_utxos(...)?`
- `:847` `rebuild_producer_set_from_blocks(...)?`

Every one of these fires *after* `:800-802`. Under the identical disk failure that makes
`clear()` fail, **pre-fix** control simply walked past the no-op clear and died at `:831`
instead — reaching the same chain_state-rewound abort, but with the UTXO set *also* partially
mutated (un-cleared old-tip state plus a partial replay of `1..=target`). So the fix does not
create the failure class; it moves the abort one step earlier, into a strictly cleaner state.

**(b) What the node does on that `Err`.** `block_handling.rs:333-335`:
`if let Err(e) = self.execute_reorg(...).await { error!("Failed to execute reorg: {}", e); }`
— logged and swallowed at that call site; `handle_new_block` continues and returns `Ok`. The
other callers (`fork_recovery.rs:75`, `:120`, `wedge_escape.rs:164`) propagate with `?`. No
abort, no exit, no retry of the reorg. Critically, `state_db.atomic_replace` at `:894` **never
runs**, so the *persisted* chain_state is not rewound — a restart reloads the pre-reorg tip
from state_db (`init.rs:317` onward) and the divergence is erased. It is self-healing across
a restart.

**(c) Better, worse, or equivalent?** **Strictly better**, and the honest comparison is not
close:

| | Pre-fix (RocksDb, legacy reorg) | Post-fix |
|---|---|---|
| Healthy disk (100% of real executions) | **Guaranteed silent UTXO leak** of the whole rolled-back range, laundered durably by `atomic_replace`; violates INV-UTXO-001 every run; no error surfaced | Correct |
| Failing disk | abort at `:831` with chain_state rewound **and** UTXO half-mutated | abort at `:809` with chain_state rewound and **UTXO byte-identical** (WriteBatch is atomic; `utxo_count` untouched — `writes.rs:100` is after the `?`), ProducerSet untouched, persisted chain_state untouched, loud error |

The new failure state is a strict *subset* of a pre-existing one, and it replaces a guaranteed
undetectable corruption with a rare, loud, restart-healing in-memory inconsistency.

**(d) Does it need reordering?** No — not in M1.

- Moving `:800-802` after the clear would close only the smallest of four windows; `:831`,
  `:840`, `:847` leave larger ones. It would buy an *appearance* of atomicity, which is the
  worse outcome for a state-corruption class.
- It is already scoped out by name: REQ-I156-014 ("chain_state ↔ `cf_utxo` inconsistency left
  by an `Err` return… **pre-existing, not introduced here**") is a declared `Won't` (§7).
- REQ-I156-006 ("any refused operation must leave both the **UTXO set and the ProducerSet**
  byte-for-byte unchanged") is *already satisfied* on this path — both are untouched at
  `:809`. REQ-006 does not mention chain_state.

**Correction to the developer's framing (intellectual honesty).** The hand-off says the error
"was silently swallowed and the code continued". That is not accurate: pre-fix `clear()`
returned `()`, so there was no error to swallow — it simply could not fail. What *did* exist
pre-fix is the same partial-failure shape via `:831`/`:840`/`:847`. The residual is real, but
"did not exist before" overstates it.

**Recommended follow-up (not a blocker):** fold "hoist the `chain_state` mutation in
`execute_reorg`'s legacy branch to after the UTXO rebuild, or make the whole branch
scratch-set + single `atomic_replace`" into the REQ-I156-012/-014 follow-up incident, where
all four windows can be closed together.

## 7. Adjacent flows probed

| Flow | Probe | Result |
|---|---|---|
| `rollback_one_block()` **undo-based** happy path | `inc_i_156_req007_undo_based_rollback_state_root_unchanged`; code read `rollback.rs:10-140` — clear() is not on this path | **Unchanged.** PASS (also green under neutralization → genuinely untouched) |
| `execute_reorg` **undo** path | `inc_i_156_req007_undo_based_reorg_utxo_state_unchanged`; code read `block_handling.rs:661-780` — no `clear()` | **Unchanged.** PASS |
| `init.rs` **genesis reset** | Code read `init.rs:340-380`. Uses `state_db.clear_and_write_genesis(&chain_state)?` (`:375`) then `*utxo_set.write().await = UtxoSet::new()` (`:377`). It **does not call `UtxoSet::clear()` at all.** | **Not affected.** The brief's description of the init.rs site as "genesis-reset" is imprecise — the actual `clear()` at `init.rs:118` is the *startup undo-gap recovery* path in `recover_body_gaps`, matching the analysis §1.3 row 3 |
| `recover_body_gaps` (`init.rs:118`) new `?` | Fence `if !utxo_set.is_rocksdb()` at `:117` is intact and unchanged from base. `utxo_set` is built as `UtxoSet::from_state_db` at `init.rs:317`, before the only call at `:411`; the sole path that replaces it with `UtxoSet::new()` (`:377`) sets `best_height = 0`, which fails the `if chain_state.best_height > 0` guard at `:405`, so `recover_body_gaps` is not reached. On any production node the fenced branch is therefore **unreachable**, and inside the fence only the InMemory arm exists, where `clear()` is `Ok(())` unconditionally (`set.rs:74-77`) | **No new startup failure mode.** `?` is a signature obligation, not a live error path |
| **Snap sync state install** | Code read `fork_recovery.rs:326-373`. Replaces the set wholesale (`*utxo = new_utxo_set`, `:335`), persists via `atomic_replace` (`:350`), converts back to the RocksDb variant at `:363`. **No `clear()` and no `clear_utxos()`** | **Not affected** |
| Other callers of `StateDb::clear_utxos()` | Per-root grep. `crates/`: only `set.rs:78` (production) + test files. `bins/`: zero non-test. | `UtxoSet::clear()` is now the **sole** production caller |
| `StateDb::clear_and_write_genesis()` callers | Per-root grep. `crates/`: definition + tests only. `bins/`: `init.rs:375` only | Untouched by M1 |
| INC-I-152 rollback guard | `cargo test -p doli-node --test inc_i_152_p1_003_rollback_holed_store` → 3 passed | No regression |
| INV-UTXO-001 conservation | `inc_i_064_supply_conservation` → 6 passed, 1 ignored | No regression |
| Fork recovery integration | `fork_recovery` → 11 passed | No regression |
| `recover_body_gaps` replay tests | `recover_replay` → 4 passed, 1 ignored | No regression |
| Storage core | `storage --lib` 245 passed; `disk_guardian_failsafe_test` 19 passed; `doli-node --lib` 55 passed | No regression |

**Full workspace suite:** not re-run. The prior session established 3164 passed / 3 failed /
43 ignored with the 3 failures confirmed pre-existing on base `f4e6ea69`. M1's blast radius is
2 production files plus 1 startup helper, and every target that touches them (plus the two
adapted test files) was run individually and is green; `cargo clippy --workspace
--all-targets` compiles every test target in the workspace and returned exit 0 with zero
diagnostics, so nothing fails to build. Re-running the full suite would not change the
verdict. **Recommendation:** the reviewer or the M2 close-out should run it once before the
branch merges, to re-baseline the 3 known failures against the combined M1+M2 tree.

## 8. Issues found

No blocking issues.

### OBS-001 — low — requirements doc: "4 production sites" is wrong
`docs/bugfixes/inc-i-156-p2-residual-guards-analysis.md:547`. The actual count of non-test
`UtxoSet::clear()` call sites is **3**, and the doc's own detailed criteria (line 575-576) say
3. **Remediation:** change "all 4 production sites propagate" to "all 3 production call sites
(`rollback.rs`, `block_handling.rs`, `init.rs`) propagate". No code change.

### OBS-002 — medium (latent) — the `init.rs:117` fence has no regression lock
Post-M1 the INC-I-136 fence `if !utxo_set.is_rocksdb()` at `init.rs:117` is *newly*
load-bearing: before M1, deleting it was harmless on RocksDb (clear was a no-op); after M1,
deleting it would **wipe the live production UTXO set at startup**. Verified that no test
exercises it — per-root grep for `recover_body_gaps` finds zero test callers in `bins/`,
`crates/`, `testing/` (only the definition at `init.rs:56`, the re-export at `mod.rs:22`, the
call at `:411`, and comment references in `crates/storage/src/state_db/tests.rs`). The
developer's in-code comment documents the hazard well, but a comment is not a gate.
**Remediation:** add one test that calls `recover_body_gaps` with a RocksDb-variant
`UtxoSet` and a missing undo entry, asserting the set is **not** emptied — and register it as
a `regression_tests` row against the INC-I-136 invariant. Suitable for M2 or the follow-up
incident; not a blocker for M1 (the fence is present and correct).

### OBS-003 — low — REQ-I156-006 straddles the M1/M2 boundary
REQ-I156-006's acceptance criteria explicitly name "R1 clear-failure" as a refusal path to
cover, but §6 assigns REQ-006 to **M2**. Its R1 clause is in fact already satisfied (see §6d:
UTXO set and ProducerSet are both untouched at `block_handling.rs:809`), but M2's QA must not
assume M1 covered it. **Remediation:** note the split in the M2 test plan.

### OBS-004 — low — module size budget (PRE-EXISTING, no action in this bugfix)
`crates/storage/src/utxo/set.rs` 556 → **558** (+2); `bins/node/src/node/block_handling.rs`
973 → **985** (+12); `bins/node/src/node/init.rs` **1478** (+6). All three were already over
the 500-line budget on base `f4e6ea69` (measured via `git show f4e6ea69:<path> | wc -l`). The
growth is entirely explanatory comments on a consensus-critical path, which is the right
trade. **No split demanded in this bugfix**, per the M1 mandate. Flagged only so the budget
debt stays visible.

## 9. Specs / docs drift

| File | Documented behavior | Actual behavior | Severity |
|---|---|---|---|
| `specs/engine-parts.md:2795` | rollback legacy fallback "clears UTXO set and replays all blocks from genesis" | **Now true.** Was false on RocksDb pre-M1 — **M1 heals this drift** | resolved by M1 |
| `specs/engine-parts.md:2795` | (omission) does not mention the INC-I-152 `ensure_blocks_present` guard, unlike the sibling `execute_reorg` entry at `:2760` | guard is live at `rollback.rs:169-181` | low — pre-existing, assigned REQ-I156-009 (M2) |
| `specs/engine-parts.md:2051` | lists `UtxoSet::clear()` among "basic UTXO access" | signature is now `Result<(), StorageError>`; the spec line states no return type | none — no drift introduced |
| `specs/engine-parts.md:2760` | `execute_reorg` "legacy fallback (no undo data): rebuild UTXO and ProducerSet from genesis" | **Now true** on RocksDb | resolved by M1 |
| `crates/storage/src/utxo/set.rs:64-78` (base) | comment false in all 3 clauses | replaced — REQ-I156-008 | resolved by M1 |
| `docs/architecture.md`, `docs/troubleshooting.md` | — | no drift found on these routines | none |

**Net: M1 reduces specs drift.** The one residual item is already owned by REQ-I156-009 in M2.

## 10. Final verdict

**PASS** — all six M1 requirements met (4 Must, 2 Should). The root cause is closed by
delegating to the correct, already-tested primitive rather than patched. The RED phase was
independently re-verified by neutralize-and-restore, not accepted on the developer's word.
Ten M1 tests plus 288 adjacent tests are green; clippy (`-D warnings`) and `fmt --check` both
exit 0. Residual 1 is **ACCEPTABLE-AS-IS**: the partial state it describes is pre-existing in
kind, post-fix is a strict improvement on it, and the general form is a declared `Won't`
(REQ-I156-014). No blocking issues. Four non-blocking observations, of which OBS-002 (missing
regression lock on the now-load-bearing `init.rs:117` fence) is the one worth scheduling.

**Approved for review.** M2 (R2 — `rebuild_producer_set_from_blocks` precondition) remains
unvalidated and out of scope for this report.
