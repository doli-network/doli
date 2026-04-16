# QA Re-validation Progress (2026-03-13)

## Scope: Re-validate 3 blocking issues from GUI Desktop QA

### ISSUE-001: TxBuilder build_for_signing() and sign_and_build()
- Status: RESOLVED
- build_for_signing(): Signing message format verified identical to doli-core::Transaction::signing_message()
- sign_and_build(): Full implementation present, no todo!() macros
- New concern (OBS-008): bincode serialization compatibility -- Hash/Signature custom serde uses serialize_bytes (with u64 length prefix) in bincode, but wallet writes raw bytes without prefix

### ISSUE-002: Registration fee tiered calculation
- Status: RESOLVED
- fee_multiplier_x100() in wallet matches core line-by-line (all 8 tiers)
- registration_fee() formula matches core exactly
- Constants verified: BASE=100,000, MAX=1,000,000
- test_fr020_registration_fee_matches_protocol passes

### ISSUE-003: CI pipeline for GUI builds
- Status: RESOLVED
- 3 new jobs: build-gui-linux, build-gui-macos, build-gui-windows
- Each installs: Rust, Node.js 20, platform deps, Tauri CLI v2
- Artifacts: .AppImage/.deb, .dmg, .msi
- Release job depends on all 5 build jobs (2 existing + 3 new GUI)

### Test Results
- wallet: 162 tests pass (146 unit + 8 tx_builder integration + 8 wallet_compat integration)
- doli-gui: 14 tests pass
- Total: 176 tests, 0 failures

### Final Verdict
Changed from CONDITIONAL APPROVAL to PASS.

---

<!--
  2026-04-16: The M-RC9 QA section and the M-Choice3 QA section previously
  occupied this position. Both were introduced by commit 8caea821 (M-Choice3)
  which replaced the 2026-03-13 QA content above. When 8caea821 was reverted
  alongside the drop of the doli chain-repair subcommand, those QA sections
  were removed and the 2026-03-13 content restored. The three Reviewer sections
  below were appended by later commits (M-Choice2 = 953a7c3d, M-RC12-full =
  fdadfeab, M-Choice1 = 1f28193e) and remain untouched.

  The M-RC9 work itself is intact (commit 69893d83); its QA report lives at
  docs/qa/inc-i-034-M-RC9-qa-report.md.
-->

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
