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

---

# M-RC12-full Reviewer (2026-04-16)

- Milestone: M-RC12-full — Complete asymmetric blacklist in sync manager empty-headers path
- Incident: INC-I-034 (santiago/ivan/seed3 cascade 2026-04-16 05:11 UTC)
- Branch: `synmgrefactor` (uncommitted)
- Scope reviewed:
  - `crates/network/src/sync/manager/sync_engine/response.rs` (net -4 LOC at lines 317-347)
  - `crates/network/src/sync/manager/tests.rs` (+343 LOC asymmetric-blacklist module + tests.rs:2213 VerifiedSnapshot compile-gate fix)
  - `docs/bugfixes/inc-i-034-m-rc12-full-asymmetric-blacklist-fix.md` (new, 176 lines)
  - `docs/.workflow/milestone-progress.md` (M-RC12-full status flip)

## M-RC12-full Reviewer Verdict
Verdict: APPROVE
Correctness:  pass — insert() deleted; use_height_based_headers=true set unconditionally; counter preserved; no new locks/awaits; recently_snapped fully dead-removed; idempotent with other set-sites (response.rs:228, :296)
Scope:        pass — 4 files exactly; no BlacklistDecision enum (Phase-2 excluded); pre-existing CLAUDE.md/fundamentals-check.md drift belongs to prior sessions
Tests:        4/4 asymmetric module + 302/302 network lib suite (FAIL->PASS); output contract matches (4 outputs x 4 paths); regression-safe for test_post_snap_empty_headers_triggers_height_fallback (tests.rs:1339) and test_blacklist_escalation_uses_signal_not_counter (tests.rs:1006)
Regression:   clean — decision.rs:90, cleanup.rs:410, snap_sync.rs:262, mod.rs:309, block_lifecycle.rs:152 all safe under empty blacklist; site #8 test (tests.rs:3414) manually seeds blacklist, path unchanged
Docs:         pass — three-layer coverage, FAIL->PASS evidence, no-version-bump + no-HF justifications, deployment checklist present; spec scheduler-state-architecture.md lines 216/228/276/294/305/463 all align with fix
Issues:
  - (nit) bugfix doc file list omits tests.rs (+343 LOC tests + tests.rs:2213 compile-gate fix) — non-blocking
  - (nit) warn! at response.rs:337 could arguably be info! — structured-correct path under minority-fork conditions. Defer to operator preference.
Recommendation: green-light

## Handoff
- Author: `Antonio Lozada <antonio@omegacortex.ai>`.
- Post-commit: testnet deployment per CLAUDE.md checklist (step 5: cp + codesign).
- Monitor post-deploy for new log line: `local hash not recognized by peer. NOT blacklisting (asymmetric invariant)`. Expected on minority-fork nodes; benign.

---

# M-Choice1 Reviewer (2026-04-16)

- Milestone: M-Choice1 — Phase-1 EpochState-in-state-root HardForkSchedule entry (NO call-site wiring)
- Incident: INC-I-034
- Branch: `synmgrefactor` (uncommitted)
- Scope reviewed:
  - `crates/storage/src/snapshot.rs` (+71 LOC: new `compute_state_root_with_epoch_state` + 3 m_choice1 tests; legacy `compute_state_root` verified byte-identical)
  - `crates/network/src/protocols/status.rs` (`CURRENT_PROTOCOL_VERSION: 3 -> 4`, MIN_PEER held at 1, +2 tests)
  - `crates/updater/src/hardfork.rs` (`for_network` seeds Mainnet+Testnet EPOCH_SNAPSHOT_HF at h=10_000_080, min_version 7.0.0; Devnet empty; +2 tests)
  - `docs/bugfixes/inc-i-034-m-choice1-state-root-hf.md` (new)
  - `docs/.workflow/milestone-progress.md` (M-Choice1 row flipped)
- Spec: `specs/scheduler-state-architecture.md` — "State-root inclusion (timing: SAME HF)" + Migration Phase-1 items 3 + 6

## M-Choice1 Reviewer Verdict
Verdict: APPROVE
Legacy compute_state_root untouched: yes (snapshot.rs:24-59 byte-identical; signature, doc, canonical-encoding, individual hashes, tracing::info!, 96-byte combined buffer, all preserved)
4-component ordering: matches legacy prefix (cs || utxo || ps || es) — spec prose ordering differs but legacy order is locked by committed mainnet state roots; reviewer confirms correctness of developer choice.
CURRENT_PROTOCOL_VERSION bump: 3 -> 4
MIN_PEER unchanged: yes (held at 1 — essential for v3/v4 Phase-1 coexistence)
No call-site wiring: yes — zero callers outside m_choice1 test module. 4 production callers of legacy `compute_state_root` (event_loop, apply_block/state_update, validation_checks, fork_recovery) unchanged.
HF entries: correct — Mainnet + Testnet at h=10_000_080 (= 27778*360, epoch-aligned); Devnet no entry; consensus_changes mentions EpochState + state root + INC-I-034.
Tests: 7/7 m_choice1 (storage 3/3, updater 2/2, network 2/2) + regression (storage 173/173, updater 36/36, network 304/304 +1 pre-existing ignored, doli-node lib 20/20).
Build gates: clean (cargo build --release --workspace + cargo clippy --workspace -- -D warnings).
Scope: pass — exactly 5 files; no apply_block/snap_sync/cleanup/Node-struct edits.
Docs: pass — bugfix report covers Phase-1-only scope (repeated), three-layer coverage table with Phase-2 deferrals, FAIL->PASS per crate, placeholder rationale + operator checklist, version-bump justification.

## Consensus-layer safety summary
- State root unchanged on every existing chain: legacy function byte-identical; every production caller still invokes 3-component; new function exists as a linked-but-uncalled primitive. Phase-1 state roots bit-identical to pre-M-Choice1.
- Activation height safely future: placeholder 10_000_080 is ~250x current tip; accidental ship-as-is = HF never fires = no consensus change.
- fork_id boundary sharp: pre-activation Hash::ZERO, at-and-after activation non-ZERO and stable.
- Protocol version signals capability without partitioning: v4 advertises "can switch formulas at gate"; MIN_PEER=1 keeps v3 peers connected until operators cross the mainnet boundary.
- 4-component formula pinned by explicit byte recomputation in Test 2: any silent re-ordering or extra framing would break the test.

## Issues (non-blocking)
- (nit) Developer bugfix cites "15 call sites" — actual production count is 4 (bookkeeping number, not a defect).
- (nit) `specs/protocol.md:1479` has pre-existing stale CURRENT_PROTOCOL_VERSION=2 note (out of scope; `/sync-docs` follow-up).
- (nit) `cargo fmt --check` drift in test modules is pre-existing from test-writer pass.

Recommendation: green-light for commit as `Antonio Lozada <antonio@omegacortex.ai>` on `synmgrefactor`.
