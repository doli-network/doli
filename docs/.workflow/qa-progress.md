# QA Progress — INC-I-034 M-RC9 (2026-04-16)

- Milestone: M-RC9 silent `vec![]` fail-fast fix
- File under test: `bins/node/src/node/rewards.rs::Node::calculate_epoch_rewards`
- Test binary: `bins/node/tests/m_rc9_silent_vec_regression.rs`
- REQ: REQ-REDESIGN-002 row 9 — silent `vec![]` deleted; fail-fast on incomplete block_store OR snapshot use.
- Chosen branch of the contract: **FAIL-FAST**

## Entrypoint
- Library/test binary; no `cargo run` needed.
- Run: `cargo test -p doli-node --test m_rc9_silent_vec_regression` and the other requested suites.
- Build: `cargo build --release -p doli-node`, `cargo clippy -p doli-node -- -D warnings`, `cargo fmt --check`.

## Checklist

- [x] cargo test --test m_rc9_silent_vec_regression -p doli-node  — 3/3 PASS
- [x] cargo test -p doli-node --lib  — 10/10 PASS
- [x] cargo test -p doli-node --test epoch_reward_explicit_inputs  — 7/7 PASS (2 ignored)
- [x] cargo test -p doli-node --test fork_recovery  — 11/11 PASS
- [x] cargo test -p doli-node --test checkpoint_rotation  — 16/16 PASS (12 ignored stress)
- [x] cargo test -p doli-node --test test_network  — 13/13 PASS (12 ignored stress)
- [x] cargo build --release -p doli-node  — clean
- [x] cargo clippy -p doli-node -- -D warnings  — clean
- [x] cargo fmt --check  — clean
- [x] Exploratory findings — all 7 logged in QA report

## Final verdict: APPROVE.
Report: docs/qa/inc-i-034-M-RC9-qa-report.md

---

# M-Choice3 QA (2026-04-16)

- Milestone: M-Choice3 — `doli chain-repair` CLI subcommand
- Incident: INC-I-034
- Branch: `synmgrefactor` (uncommitted)
- Scope: `bins/cli/src/cmd_chain.rs`, `bins/cli/src/rpc_client.rs`, `bins/cli/src/commands.rs`, `bins/cli/src/main.rs`
- Spec: `specs/scheduler-state-architecture.md` — "What ADDS" → `bins/cli/src/repair_chain.rs (~100 lines)`
- Test-writer report: `docs/.workflow/m-choice3-test-writer.md`
- Developer report: `docs/bugfixes/inc-i-034-m-choice3-chain-repair.md`

## Entrypoint
- CLI binary: `./target/release/doli chain-repair --help`
- Build: `cargo build --release -p doli-cli`
- Test: `cargo test -p doli-cli --bins`

## Acceptance criteria results (10/10 met)

| # | Criterion | Result |
|---|-----------|--------|
| 1 | FAIL→PASS evidence (15 pure helper tests) | PASS — 15/15 `repair_chain_tests` pass |
| 2 | Regression tests (10 `wipe_tests`) | PASS — 10/10 still green |
| 3 | All bins tests | PASS — 56/56 `cargo test -p doli-cli --bins` |
| 4 | Build gates (release/clippy/fmt/workspace) | PASS — all 4 gates clean |
| 5 | Output × Path matrix coverage (15 cells) | PASS — validate_peer_url 6/6, format_gap_summary 3/3, BackfillPhase::from_status 3/3, format_progress 3/3 |
| 6 | CLI semantic correctness (4 validation paths) | PASS — empty/peer-id/missing-scheme/self all rejected pre-RPC with actionable messages |
| 7 | Scope discipline (whitelist respected) | PASS — only the 4 source files + bugfix report + milestone row touched; CLAUDE.md / fundamentals-check.md mods are pre-existing and unrelated |
| 8 | Agentic error design | PASS — all 4 error classes are stage-aware, classify the mistake, and suggest remediation |
| 9 | MEMORY.md rule #1 defense | PASS — `validate_peer_url` checks peer-ID BEFORE scheme (line 788 vs 811), so paste-a-peer-id gets the helpful error not the generic scheme error |
| 10 | Bugfix doc completeness | PASS — includes REQ, FAIL→PASS evidence, no-version-bump rationale, no-HardFork-entry rationale, deployment checklist, operator surface |
| 11 (bonus) | Milestone row updated | PASS — M-Choice3 shows `COMPLETE (local, pending commit)` |

## Test evidence (reproduced by QA)

- `cargo test -p doli-cli repair_chain_tests` → **15 passed; 0 failed** (3 `BackfillPhase::from_status`, 3 `format_gap_summary`, 3 `format_progress`, 6 `validate_peer_url`).
- `cargo test -p doli-cli wipe_tests` → **10 passed; 0 failed**.
- `cargo test -p doli-cli --bins` → **56 passed; 0 failed**.
- `cargo build --release -p doli-cli` → clean (32.36s).
- `cargo clippy -p doli-cli --all-targets -- -D warnings` → clean (no warnings).
- `cargo fmt --check -p doli-cli` → exit 0.
- `cargo build --release --workspace` → clean.

## CLI surface validation (pre-RPC validation paths)

```
$ ./target/release/doli chain-repair --help
Usage: doli chain-repair [OPTIONS] --peer <PEER>
  --peer, --yes, --poll-interval-secs (default 5), --max-wait-secs (default 3600)

$ ./target/release/doli chain-repair --peer 12D3KooWAbCdEfGhIjKlMnOpQrStUvWxYz
Error: '12D3KooW...' looks like a libp2p peer ID, not an RPC URL. backfillFromPeer requires an RPC URL like http://HOST:PORT, never a peer id.

$ ./target/release/doli chain-repair --peer 127.0.0.1:8500
Error: peer URL '127.0.0.1:8500' is missing a scheme — use http:// or https://

$ ./target/release/doli --rpc http://127.0.0.1:8500 chain-repair --peer http://127.0.0.1:8500
Error: peer URL 'http://127.0.0.1:8500' is the same as the local node endpoint — cannot backfill from self

$ ./target/release/doli chain-repair --peer ""
Error: peer RPC URL is required (empty string not allowed)
```

All 4 exit with code 1. All fire BEFORE any RPC call. MEMORY.md rule #1 trap is hard-blocked.

## Scope audit

Pure additions only across the 4 whitelisted source files:
- `bins/cli/src/cmd_chain.rs`: +629 lines (helpers +149 + orchestrator +141 + test module +290 + doc headers +49), 0 deletions. Pre-existing `cmd_chain`, `cmd_chain_verify`, `cmd_rewards`, `cmd_wipe`, `WIPE_PRESERVE`, `WipeResult`, `collect_deletable`, `wipe_data_dir`, and `wipe_tests` byte-identical.
- `bins/cli/src/commands.rs`: +27 (task spec said +24 — 3 extra are docstring comments, not scope creep).
- `bins/cli/src/main.rs`: +15 (task spec said +16, matches).
- `bins/cli/src/rpc_client.rs`: +44 (task spec said +40 — 4 extra are docstring comments).

No retry logic, no progress bars, no peer discovery. Orchestrator is ~140 LOC of thin glue exactly as designed.

## Final verdict: **PASS** — green-light for commit.

Report: `docs/.workflow/qa-progress.md` (this section)

---

# M-Choice2 Reviewer (2026-04-16)

- Milestone: M-Choice2 — Phase-1 RUNTIME PERIODIC block-store integrity check (observability-only)
- Incident: INC-I-034
- Branch: `synmgrefactor` (uncommitted)
- Scope reviewed: `bins/node/src/node/periodic.rs`, `bins/node/src/node/mod.rs`, `bins/node/src/node/init.rs`, `docs/bugfixes/inc-i-034-m-choice2-periodic-integrity-check.md`, `docs/.workflow/milestone-progress.md` (row M-Choice2)
- Spec: `specs/scheduler-state-architecture.md` — "Block-store integrity contract" + locked CHOICE 2 (RUNTIME PERIODIC)

## Verdict: **APPROVE** — green-light for commit.

| Category       | Result |
|----------------|--------|
| Correctness (helper) | pass — 9 paths x 1 output, all asserted (periodic.rs:863-880) |
| Correctness (async)  | pass — minimal lock, spawn_blocking offload, correct log-spam guard (periodic.rs:715-759) |
| Correctness (struct) | pass — field added at mod.rs:226, initialized at init.rs:920 and init.rs:1105 |
| Scope                | pass — 3 source files touched, no drive-bys, no Phase-2 leakage |
| Tests                | 10/10 PASS (`integrity_check_tests` + `default_interval_constant_is_1000`) |
| Regression           | clean (lib 20/20, clippy/fmt clean, m_rc11 compiles) |
| Docs                 | pass — bugfix report complete, milestone row flipped |
| Issues               | none blocking |

## Key findings
1. `saturating_sub` is belt-and-braces (the caller already guards `current_tip > last`), but harmless; P8 at `u64::MAX` proves no overflow.
2. TOCTOU between tip read and scan is a known Phase-1 design choice — subsequent gaps caught next tick.
3. `last_integrity_check_tip` updated regardless of scan outcome — deliberate anti-spam guard (1 CRITICAL per ~3 h instead of per 5 s tick).
4. CRITICAL log preserves first-missing height from `ensure_blocks_present` and names `doli chain-repair` as remediation.
5. Zero Phase-2 leakage: no `HALT_PRODUCTION`, no `BackfillRequest`, no auto-recovery added.

## Informational (non-blocking, deferred)
- `docs/troubleshooting.md` entry for `[INTEGRITY_CHECK] CRITICAL` deferred to operator-docs sweep (noted in bugfix report section 4).

## Recommendation: green-light for commit as `Antonio Lozada <antonio@omegacortex.ai>` on `synmgrefactor`.
