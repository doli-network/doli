# INC-I-034 / M-Choice2 — RUNTIME PERIODIC Block-store Integrity Check (Phase 1, Observability-only)

> **UPDATE 2026-04-16 (post-M-Choice3 revert):** the CRITICAL log message
> referenced throughout this report originally pointed operators at
> `doli chain-repair --peer <RPC_URL>`. That subcommand was reverted
> alongside commit 8caea821 because it wrapped the existing `backfillFromPeer`
> RPC with no new capability. The live code (commit a1334818) now points
> operators at the RPC directly — call it via curl or the doli-ops
> backfill skill (see MEMORY.md rule #1 for the RPC-URL format). The
> body of this report below is preserved as a historical snapshot for
> commit 953a7c3d; all runtime behaviour (interval, spam guard,
> observability-only stance) is unchanged.

- **Incident**: INC-I-034
- **Milestone**: M-Choice2 (locked user decision: "RUNTIME PERIODIC")
- **Branch**: `synmgrefactor`
- **Requirement**: REQ-REDESIGN-011 (block-store completeness invariant)
- **Spec**: `specs/scheduler-state-architecture.md` — "Block-store integrity contract" + locked CHOICE 2
- **Files touched**:
  - `bins/node/src/node/periodic.rs` (+92 LOC: constant + pure helper + async method + call site + tests were already landed by test-writer)
  - `bins/node/src/node/mod.rs` (+5 LOC: `last_integrity_check_tip: Option<u64>` field)
  - `bins/node/src/node/init.rs` (+2 LOC: field init at 2 construction sites)
  - `docs/.workflow/milestone-progress.md` (status update)
- **Tests**: `bins/node/src/node/periodic.rs :: integrity_check_tests` (10 unit tests, all green)

---

## What was added

Three things wired together:

1. **Constant** `INTEGRITY_CHECK_INTERVAL_BLOCKS = 1000` — the default scan interval (~3 h at 10 s slot time).
2. **Pure helper** `should_run_integrity_check(current_tip, last_checked_tip, min_interval_blocks) -> bool` — the scheduling predicate, exercised by the 10 tests written by the test-writer in TDD step 1.
3. **Async method** `Node::maybe_run_integrity_check(&mut self)` — the stateful glue that:
   - reads `self.chain_state.best_height`,
   - consults the pure helper to decide whether to scan,
   - on `true`, clones `self.block_store` and dispatches `BlockStore::ensure_blocks_present(1, tip)` on a `tokio::task::spawn_blocking` handle (so the O(tip) CF point-lookup sweep does not starve the async runtime),
   - logs success at `INFO` level, gap at `ERROR` level with an operator-facing instruction to run `doli chain-repair --peer <RPC_URL>`,
   - updates `self.last_integrity_check_tip = Some(current_tip)` unconditionally — see "Log-spam guard" below.
4. **Wiring** — a single `self.maybe_run_integrity_check().await` call at the bottom of `Node::run_periodic_tasks`, right before the `Ok(())` return.
5. **Field** — `Node.last_integrity_check_tip: Option<u64>`, initialized to `None` at both construction sites (`Node::new`, `Node::new_for_test`).

---

## Design notes

### Phase 1 — Observability-only
No `HALT_PRODUCTION`, no automatic `BackfillRequest` emission, no `chain_state` rollback, no peer scoring change. A gapped node keeps producing and serving blocks; the operator sees a single CRITICAL log line per interval until they run `doli chain-repair`. Phase 2 (M-Choice1, `HardForkSchedule`-gated) is where the halt lands.

### Reuses existing primitives
The scan uses `BlockStore::ensure_blocks_present(1, tip)` which landed in M-RC11 (commit `5f1565c0`). That function is exactly the right shape for this call site — genesis-safe (`low.max(1)`), no header/body deserialization, O(range) hot-path point lookups on the height index, returns `Err(StorageError::NotFound(...))` with the first missing height on failure.

### Blocking-task dispatch
At mainnet tip heights (~40 k blocks today, ~millions later) a full 1..=tip sweep is not free. Running it on the async runtime would stall the event loop long enough to miss slot deadlines or drop gossip messages. `tokio::task::spawn_blocking` moves the sweep onto the blocking thread pool where it belongs.

### 1000-block interval
At 10 s slot time this is ~2 h 47 min. Overhead: one CF sweep per ~3 h, purely read-only. Contention with production/gossip: negligible.

### Log-spam guard
`last_integrity_check_tip` is updated **regardless of scan outcome** (Ok, Err, or join-error). On success this is the obvious "don't re-scan for another 1000 blocks" marker. On failure this is a deliberate anti-spam guard: the operator sees one CRITICAL every ~3 h, not one per periodic-task tick (~5 s). They see the same message repeatedly until they fix the gap; they don't see a log flood. If this guard were absent, a gapped producer would emit ~700 CRITICAL lines per hour, drowning out every other log.

### Defensive `should_run_integrity_check`
The pure helper returns `false` in three non-obvious cases:
- `current_tip == 0` (genesis — nothing to scan yet; avoid cold-start spam).
- `current_tip <= last_checked_tip` (rollback or stall — never re-run on a backward move; no `u64` underflow).
- `min_interval_blocks == 0` AND no advance (guard against a misconfigured/zero-initialized interval turning the scan into a busy loop).

These are the P1, P6, P7, P9 cases in the test matrix.

---

## Test evidence (FAIL -> PASS)

**FAIL baseline (test-writer's handoff, TDD step 1 red)**:
```
$ cargo test -p doli-node --bin doli-node integrity_check_tests --no-run
error[E0425]: cannot find function `should_run_integrity_check` in this scope
error[E0425]: cannot find value `INTEGRITY_CHECK_INTERVAL_BLOCKS` in this scope
error: could not compile `doli-node` (bin "doli-node" test) due to 11 previous errors
```

**PASS evidence (after this change)**:
```
$ cargo test -p doli-node --bin doli-node integrity_check_tests

running 10 tests
test node::periodic::integrity_check_tests::default_interval_constant_is_1000 ... ok
test node::periodic::integrity_check_tests::p1_genesis_tip_zero_no_prior_scan_returns_false ... ok
test node::periodic::integrity_check_tests::p2_first_run_tip_past_interval_returns_true ... ok
test node::periodic::integrity_check_tests::p3_last_scan_recent_returns_false ... ok
test node::periodic::integrity_check_tests::p4_exact_boundary_returns_true ... ok
test node::periodic::integrity_check_tests::p5_just_past_boundary_returns_true ... ok
test node::periodic::integrity_check_tests::p6_zero_interval_no_advance_returns_false ... ok
test node::periodic::integrity_check_tests::p7_tip_backward_returns_false_no_underflow ... ok
test node::periodic::integrity_check_tests::p8_u64_max_boundary_returns_true ... ok
test node::periodic::integrity_check_tests::p9_u64_max_no_advance_returns_false ... ok

test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 15 filtered out
```

Paths covered: P1 genesis, P2 first-run crossed, P3 too-soon, P4 exact boundary (inclusive >=), P5 past boundary, P6 zero-interval no-advance, P7 backward tip, P8 `u64::MAX` boundary, P9 `u64::MAX` no-advance, plus a sanity test pinning the constant = 1000.

**Full `doli-node` lib test suite (regression guard on the new field)**:
```
$ cargo test -p doli-node --lib
test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

**Integration test compile (field-init regression guard)**:
```
$ cargo test -p doli-node --test m_rc11_fork_guard_backfill_regression --no-run
Finished `test` profile [optimized + debuginfo] target(s) in 19.95s
Executable tests/m_rc11_fork_guard_backfill_regression.rs
```

---

## Why no protocol version bump

This change is **pure observability inside a single node**. No wire format changes, no gossip message changes, no consensus rule changes, no state-root change, no new RPC. A node running this binary produces and validates exactly the same bytes as a node without it; the only externally-visible differences are (a) one extra log line per ~3 h per node, and (b) slightly more RocksDB read load once per interval. `CURRENT_PROTOCOL_VERSION` stays at its current value.

## Why no HardForkSchedule entry

Phase 1 is observability-only by design. `HALT_PRODUCTION` on gap detection is a consensus-breaking behavior change (a gapped node stops signing blocks, so its stake stops voting) and lives in Phase 2 / M-Choice1. That is when a `HardForkSchedule` entry is warranted. Adding one now would be premature: there is nothing activation-gated in this patch.

---

## Deployment checklist (per CLAUDE.md "After Every Modification")

1. **Build gate**: `cargo build --release -p doli-node && cargo clippy -p doli-node -- -D warnings && cargo fmt --check` — PASS for modified files (pre-existing fmt tech-debt in `m_rc10_*`, `m_rc11_*` regression tests is out of scope).
2. **Test**: `cargo test -p doli-node --lib` (20/20), `cargo test -p doli-node --bin doli-node integrity_check_tests` (10/10).
3. **Version protection**: not applicable (no wire/consensus/validation change). `CURRENT_PROTOCOL_VERSION` and `HardForkSchedule` unchanged.
4. **Documentation alignment**:
   - `specs/scheduler-state-architecture.md` — already documents the "Block-store integrity contract" + CHOICE 2 (RUNTIME PERIODIC). No edit needed; this change is the implementation of the already-specified behavior.
   - `specs/protocol.md` / `specs/security_model.md` / `docs/architecture.md` — no wire/consensus/component-interaction change, no edit needed.
   - `docs/troubleshooting.md` — a future follow-up could add a `[INTEGRITY_CHECK] CRITICAL` troubleshooting entry; deferred to operator-docs sweep.
   - `docs/rpc_reference.md` / `docs/cli.md` — no change.
5. **Copy binary to testnet**: standard post-build (`cp target/release/doli-node ~/testnet/bin/ && codesign --force --sign - ~/testnet/bin/doli-node`) at operator's discretion.
6. **Commit and push**: orchestrator commits as `Antonio Lozada <antonio@omegacortex.ai>` on `synmgrefactor`. Developer does NOT commit.
7. **Deploy consideration**: testnet first. Observational-only — safe to enable on mainnet after one testnet epoch if no `[INTEGRITY_CHECK] CRITICAL` log is unexpected.

---

## Operator-visible surface

### Healthy node (expected case)
```
INFO  node::periodic: [INTEGRITY_CHECK] block_store complete 1..=42000 (next scan in 1000 blocks)
```

### Gapped node (the case this scan exists to catch)
```
ERROR node::periodic: [INTEGRITY_CHECK] CRITICAL: [FORK_GUARD_BACKFILL] block_store missing canonical block at height 38214 (range checked: 1..=42000). This node's block_store has a gap. Run `doli chain-repair --peer <RPC_URL>` against a known-good peer to heal. Production will continue for now; at the M-Choice1 HardForkSchedule activation height, gapped nodes will enter HALT_PRODUCTION.
```

### Extreme edge (runtime starved out the blocking thread — should not happen in practice)
```
WARN  node::periodic: [INTEGRITY_CHECK] scan task join error at tip=42000: <JoinError>
```

The operator-action cue is explicit and points at the already-shipped CLI subcommand (`doli chain-repair`, landed in M-Choice3, commit pending).

---

## Non-goals (for the reviewer / QA)

This patch does NOT:
- halt production on gap detection (Phase 2 / M-Choice1)
- dispatch an automatic `BackfillRequest` to peers
- rewrite or modify `BlockStore::ensure_blocks_present` (reused as-is from M-RC11)
- change any wire/consensus rule, state-root, or RPC
- add or modify any field other than `Node.last_integrity_check_tip`
- touch any file outside `periodic.rs`, `mod.rs`, `init.rs`, the bugfix report, and `milestone-progress.md`
