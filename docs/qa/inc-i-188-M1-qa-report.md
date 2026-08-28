# QA Report: INC-I-188 M1 — `reset-failed` precedes `restart` in the CLI upgrade restart path

**Run**: 530 · **Agent**: qa · **Milestone**: M1 · **Date**: 2026-08-28
**Subject**: uncommitted working tree, package `doli-cli` (`git diff HEAD -- bins/cli/` + 2 untracked files)

━━━ FINDINGS — 7 total (0 BLOCKING, 0 HIGH, 0 MEDIUM, 7 LOW) ━━━

  [F1] LOW conf(0.95, measured) — `bins/cli/src/upgrade_restart.rs:82,189` — the printed operator hint chains with `&&`, which does NOT match the best-effort semantics the executed code deliberately implements for `reset-failed`.
  [F2] LOW conf(0.95, measured) — `bins/cli/src/upgrade_restart.rs:82,189` — the failure-hint string is hand-duplicated verbatim at both call sites and is NOT derived from the plan, so it can drift from `systemd_restart_plan` while every test still passes.
  [F3] LOW conf(0.9, measured) — `bins/cli/src/upgrade_restart.rs:11-20` — `run_systemd_plan` returns the LAST required step's status, not the conjunction of all required steps, and returns `false` for a plan with zero required steps; correct for today's 1-required-step plan, latent if the plan grows.
  [F4] LOW conf(0.9, measured) — `bins/cli/src/cmd_service.rs:683-689`, `bins/cli/src/cmd_snap.rs:516`, `bins/cli/src/cmd_chain.rs:276` — sibling systemd restart sites outside the upgrade path still issue a bare `systemctl restart` and remain unable to recover a start-limit-locked unit; out of REQ-188-001 scope, same failure class.
  [F5] LOW conf(0.95, measured) — `bins/cli/src/upgrade_restart.rs:132-136` — unit discovery matches systemd TEMPLATE units (`doli-mainnet@.service`), which cannot be started without an instance name; M1 now spends 2 doomed `sudo` calls per template unit instead of 1. Pre-existing discovery defect, amplified not introduced.
  [F6] LOW conf(0.95, measured) — `bins/cli/src/upgrade_restart.rs:76-86` — an empty or whitespace-only `--service` value is accepted and reported as `Restarted .`; pre-existing, behaviour byte-identical to pre-fix except for the extra `reset-failed` call.
  [F7] LOW conf(0.9, measured) — `docs/bugfixes/inc-i-188-analysis.md:22`, `crates/updater/src/apply.rs:229-244` — M1 fixes RECOVERY from the start-limit lock, not the `rm`-then-`cp` window that CREATES it; the upgrade can still produce the 203/EXEC burst on every run, now self-healing. Explicitly deferred in the analysis, restated here so it is not lost.

  Speculative: 0 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Summary

**PASS.** Both requirements are met. REQ-188-001 is satisfied *structurally*, not cosmetically:
the `reset-failed` → `restart` ordering exists in exactly ONE place
(`upgrade_systemd_plan::systemd_restart_plan`), and both live call sites consume it through the
same executor, so the two sites cannot drift apart on ordering. REQ-188-002 is confirmed at
RUNTIME, not just as a data flag. No blocking issues. Seven low-severity observations, none of
which prevent the milestone from shipping; five of the seven are pre-existing or explicitly
out of scope.

## Scope Validated

`bins/cli/src/upgrade_systemd_plan.rs` (new), `bins/cli/src/upgrade_restart.rs`,
`bins/cli/src/lib.rs`, `bins/cli/tests/it/inc_i_188_upgrade_reset_failed_test.rs` (new),
`bins/cli/tests/it/main.rs`. Consumer surface `bins/cli/src/cmd_upgrade.rs` read but unmodified.

Out of scope per task constraints and NOT reported as M1 defects: `crates/updater/**`,
`REV2-172-001` (`sh -c` root RCE in `try_restart_process`), `AUDIT-P2-011`
(`find_doli_node_path` pgrep-derived target). The adversarial unit-name string literal in the
test file is a Rust literal fed to a pure `&str -> Vec<String>` builder; treated as intended, not a defect.

## System Entrypoint

`doli upgrade` cannot be exercised end to end on this host: it requires root, a systemd
service manager, and a signed release. Two substitute harnesses were built in the scratchpad
(NOT in the repo, nothing in the repo was modified):

1. **Type-check harness** — `bins/cli/src/upgrade_restart.rs` copied with
   `#[cfg(target_os = "linux")]` attributes STRIPPED and the macOS ones disabled, compiled with
   `rustc --edition 2021 --crate-type lib`. Exit 0, `libharness.rlib` produced. This is the only
   evidence that the Linux-gated `try_restart_systemd` body compiles with the M1 edit applied.
2. **Execution probe** — same harness built as a binary with a stubbed `sudo` and stubbed
   `systemctl` earlier on `PATH`, logging argv in order. Used for cases 1-6 below.

Gates were run against the real workspace with the real cargo toolchain
(`rustc 1.95.0 (59807616e 2026-04-14)`, host `aarch64-apple-darwin`).

## Gate Results (observed, not narrated)

| Gate | Command | Result | Observed numbers |
|---|---|---|---|
| Build | `cargo build -p doli-cli` | **PASS** | after `touch` of all 3 changed sources: `Compiling doli-cli v6.25.0` → `Finished dev profile in 2.08s`; 0 errors, 0 warnings |
| Clippy | `cargo clippy -p doli-cli --all-targets -- -D warnings` | **PASS** | `Finished dev profile in 7.42s`; 0 warnings, 0 errors |
| Format | `cargo fmt --check` (workspace) | **PASS** | exit code 0, empty output |
| Tests | `cargo test -p doli-cli` | **PASS** | **245 passed, 0 failed, 0 ignored** across 12 test binaries + 0 doc-tests |
| Linux-body type-check | `rustc` forced-cfg harness (see above) | **PASS** | exit 0 |

Per-binary test counts: `src/lib.rs` 0 · `src/main.rs` 195 · `cmd_wallet_balance_address_no_wallet_read` 3 ·
`delegation_bond_cap` 4 · `inc_i_095_lp_select` 5 · `inc_i_095_lp_witness` 2 ·
`inc_i_167_wallet_overwrite_guard` 2 · `inc_i_172_cli_trust_root_resolution_test` 4 ·
`inc_i_172_upgrade_verify_blocks_test` 5 · **`tests/it/main.rs` 19 (13 INC-I-180 + 6 new INC-I-188)** ·
`logrotate_dropin_test` 3 · `register_submit_reporting` 3 · Doc-tests 0.

All 6 new tests pass:
`test_systemd_restart_plan_marks_reset_failed_best_effort_and_restart_required`,
`test_systemd_restart_plan_orders_reset_failed_before_restart`,
`test_systemd_restart_plan_threads_a_different_unit_verbatim`,
`test_systemd_restart_plan_threads_adversarial_unit_verbatim`,
`test_upgrade_restart_source_has_no_raw_restart_argv_literal`,
`test_upgrade_restart_source_routes_both_call_sites_through_plan`.

## Traceability Matrix Status

| Requirement ID | Priority | Has Tests | Tests Pass | Acceptance Met | Notes |
|---|---|---|---|---|---|
| REQ-188-001 | Must | Yes (5) | Yes | **Yes** | Ordering single-sourced in `upgrade_systemd_plan.rs:14-33`; both sites consume it at `upgrade_restart.rs:78` and `:184`. Confirmed at runtime for BOTH sites (cases 1, 6). |
| REQ-188-002 | Should | Yes (1) | Yes | **Yes** | `required: false` at `upgrade_systemd_plan.rs:22`; executor ignores non-required exit status at `upgrade_restart.rs:15-17`. Confirmed at runtime: `reset-failed` exited 1, `restart` still ran (case 1). |

### RED-state verification (test-before-fix)

Verified without touching the working tree, by reading `HEAD`:
- `git show HEAD:bins/cli/src/upgrade_restart.rs` contains `.args(["systemctl", "restart", service])` at line 64
  and `.args(["systemctl", "restart", unit])` at line 172 → `test_upgrade_restart_source_has_no_raw_restart_argv_literal` FAILS on pre-fix source.
- `git show HEAD:bins/cli/src/upgrade_restart.rs | grep -c "systemd_restart_plan("` → **0** → `test_upgrade_restart_source_routes_both_call_sites_through_plan` FAILS (needs ≥ 2).
- `bins/cli/src/upgrade_systemd_plan.rs` does not exist at HEAD → the 4 behavioural tests are a compile-failure RED, which is the weaker form of RED but acceptable for a new pure module.

### Gaps Found

- None for REQ-188-001 / REQ-188-002. Both requirements have tests, the tests correspond to real
  test functions in a registered test binary, and the code they cover is reachable from
  `cmd_upgrade.rs:88`, `:255`, `:257`.
- Wiring census (rule 32): new `pub fn systemd_restart_plan` and `pub struct SystemdStep` both have
  production callers in `bins/cli/src/upgrade_restart.rs`. No wiring debt row required.
- Module size budget (rule 19): `upgrade_restart.rs` 347 lines, `upgrade_systemd_plan.rs` 33 lines,
  test file 239 lines. All within budget.

## Acceptance Criteria Results

### Must Requirements

#### REQ-188-001: reset-failed precedes every systemd restart in the upgrade restart path — **PASS**

- [x] **The plan is exactly `reset-failed <unit>` then `restart <unit>`, in that order.**
  `upgrade_systemd_plan.rs:14-33` returns a 2-element `Vec<SystemdStep>`: index 0 is
  `["systemctl", "reset-failed", unit]`, index 1 is `["systemctl", "restart", unit]`. Asserted
  positionally (not by "contains") in `inc_i_188_upgrade_reset_failed_test.rs:82-103`, so a future
  swap is caught.
- [x] **BOTH call sites consume the same plan.**
  `restart_specific_service` → `upgrade_restart.rs:78`;
  `try_restart_systemd` → `upgrade_restart.rs:184`. Both are literally
  `run_systemd_plan(&systemd_restart_plan(X))`. No raw `["systemctl", "restart", ...]` literal
  remains anywhere in the file (grep confirms 0 matches).
- [x] **Runtime confirmation, site 1** (probe case 1, `restart_specific_service`), argv in order:
  `systemctl reset-failed doli-mainnet.service` then `systemctl restart doli-mainnet.service`.
- [x] **Runtime confirmation, site 2** (probe case 6, `try_restart_systemd`, Linux body forced on),
  argv in order, per discovered unit:
  `reset-failed doli-mainnet.service`, `restart doli-mainnet.service`,
  `reset-failed doli-mainnet@.service`, `restart doli-mainnet@.service`.

**Doctor-mode judgement — is the ordering STRUCTURAL or two hand-written sequences?**
**Structural.** There is exactly one construction of the ordering, in one pure function, and both
sites call it. Neither site can reorder, drop, or special-case a step, because neither site names a
`systemctl` subcommand at all any more — the only `"systemctl"` string literals left in
`upgrade_restart.rs` are the *read-only* discovery calls (`list-unit-files` at `:123`, `cat` at
`:149`, `is-active` at `:165`), which are untouched. The one residual duplication is the
failure-hint TEXT, not the command sequence — see [F1]/[F2].

### Should Requirements

#### REQ-188-002: a failing `reset-failed` must not abort the restart — **PASS**

- [x] **Data-level**: `SystemdStep.required` is `false` for `reset-failed`
  (`upgrade_systemd_plan.rs:22`) and `true` for `restart` (`:30`); asserted in
  `inc_i_188_upgrade_reset_failed_test.rs:112-125`.
- [x] **Execution-level** (stronger; the test suite alone does not prove this):
  `run_systemd_plan` (`upgrade_restart.rs:11-20`) iterates ALL steps unconditionally and only
  reads `status` when `step.required`. There is no early `return`, `?`, or `break`.
  Probe case 1, with a stubbed `sudo` returning **exit 1** for `reset-failed`: the log shows the
  `restart` invocation was still issued, and the function reported success
  (`Restarted doli-mainnet.service.`). This is the criterion, verified against running code.

### Could Requirements

None defined for M1.

## End-to-End Flow Results

| Flow | Steps | Result | Notes |
|---|---|---|---|
| `restart_specific_service` on a locked unit (`reset-failed` rc=1, `restart` rc=0) | 2 | **PASS** | Both commands issued in order; success line printed once |
| `restart_specific_service`, restart genuinely fails (`restart` rc=1) | 2 | **PASS** | Both commands issued; single failure line printed; `run_systemd_plan` returned `false` |
| `restart_doli_service(None)` → `try_restart_systemd` (Linux body forced on) | discovery + 2 per unit | **PASS** | Per-unit `reset-failed`+`restart` in order for both discovered units; `any_ok` set correctly |
| `cmd_upgrade.rs:88` already-up-to-date + `--service` path | 1 | **PASS (behaviour change, benign)** | This path now also clears a failed-state counter. That is exactly the operator recovery action, so it is an improvement, but it IS a new side effect on a previously read-only-ish path. Noted, not a defect. |

## Exploratory Testing Findings

| # | What Was Tried | Expected | Actual | Severity |
|---|---|---|---|---|
| 1 | `sudo` stub returns **exit 1** for `reset-failed`, 0 for `restart` | restart still runs, success reported | Both issued in order; `Restarted doli-mainnet.service.` | none (PASS) |
| 2 | `sudo` stub returns 1 for `reset-failed` AND 1 for `restart` | failure reported once, hint printed | `Failed to restart doli-mainnet.service. Run: sudo systemctl reset-failed doli-mainnet.service && sudo systemctl restart doli-mainnet.service` | LOW ([F1]) |
| 3 | **Empty** unit string (`doli upgrade --service ""`) | rejected, or at least not reported as success | `Restarting service: ` / `Restarted .`; argv `systemctl reset-failed ` (empty 3rd element) | LOW ([F6]) — pre-existing; identical shape pre-fix |
| 4 | **Whitespace-only** unit (`"   "`) | same as above | `Restarted    .`; whitespace threaded verbatim as one argv element | LOW ([F6]) |
| 5 | Adversarial unit `doli-mainnet.service; touch /tmp/PWNED_INC188 #` executed for real through `Command::args` | no shell interpretation | Single argv element, no split, no escape; `/tmp/PWNED_INC188` **not created** — `NO - no shell interpretation` | none (PASS) |
| 6 | `systemctl list-unit-files` stub emitting a real-shaped table incl. header, footer, `sshd.service`, and a **template** unit `doli-mainnet@.service` | only real doli units restarted | Header/footer/`sshd` correctly filtered out; **`doli-mainnet@.service` selected** and given `reset-failed`+`restart`, which systemd refuses for an instance-less template | LOW ([F5]) — pre-existing discovery defect at `:132-136`, untouched by M1, cost doubled from 1 to 2 wasted calls |
| 7 | Plan with **no required step** (read of `run_systemd_plan`) | conservative `false` | `let mut ok = false` → returns `false`. Correct. Multi-required plan would return only the LAST step's status | LOW ([F3]) |
| 8 | `sudo` binary **absent** (read of the executor) | reported as failure, no panic | `Command::new("sudo").status()` yields `Err(NotFound)`; `matches!(status, Ok(s) if s.success())` → `false` → failure hint. No unwrap, no panic. Two failed spawns instead of one — harmless | none (PASS) |
| 9 | Shipped sudoers whitelist (`scripts/install.sh:192-199`) grants NOPASSWD only for `rm -f`/`cp` of the two binaries — **no systemctl verb at all** | `sudo systemctl reset-failed` might be denied for a non-root caller | True, but `doli upgrade` runs as root (`docs/cli.md:1940`, INC-I-153). As root, `sudo` succeeds regardless of sudoers. Pre-fix `sudo systemctl restart` had the identical dependency, so M1 introduces no new privilege class | none (PASS) |

## Regression Check — adjacent behaviour M1 must NOT have changed

Verified from `git diff HEAD -- bins/cli/src/upgrade_restart.rs`: the diff contains exactly three
hunks — the new `use` + `run_systemd_plan` block, the `restart_specific_service` body, and the
per-unit loop body of `try_restart_systemd`. Nothing else.

| Adjacent surface | Location | Touched? | Still reachable? |
|---|---|---|---|
| `find_doli_node_path` | `upgrade_restart.rs:23-73` | **No** — zero diff hunks | Yes: `cmd_upgrade.rs:215` |
| `restart_doli_service` tier selection | `upgrade_restart.rs:91-116` | **No** | Yes: `cmd_upgrade.rs:257`; tier order systemd → launchd → process unchanged |
| `try_restart_launchd` (macOS tier 2) | `upgrade_restart.rs:199-244` | **No** | Yes: `restart_doli_service:105`; kickstart + stop/start fallback intact |
| `try_restart_process` (tier 3, all platforms) | `upgrade_restart.rs:247-315` | **No** | Yes: `restart_doli_service:111` |
| `get_uid` | `upgrade_restart.rs:319-347` | **No** | Yes: `try_restart_launchd:223` |
| systemd unit **discovery** (`list-unit-files`, `cat`, `is-active`) | `upgrade_restart.rs:123-179` | **No** | Yes — verified live in probe case 6 |

**Operator-facing output flow — preserved exactly.**

| | pre-fix | post-fix |
|---|---|---|
| `restart_specific_service` | 1 header line + exactly 1 of 2 outcome lines | identical count and trigger conditions |
| `try_restart_systemd` | per unit: 1 header + exactly 1 of 2 outcome lines; `any_ok = true` only on success | identical; `if/else` replaces `match` with the same two arms |
| return semantics | `try_restart_systemd -> bool` = "any unit restarted" | unchanged; probe case 6 confirms |

The only output delta is the hint TEXT, assessed next.

## Assessment of the deliberate hint-text change

Old: `Failed to restart X. Run: sudo systemctl restart X`
New: `Failed to restart X. Run: sudo systemctl reset-failed X && sudo systemctl restart X`

**Correct, and the change was necessary.** The old hint recommended the exact command that fails
for the same reason as the bug — the "detection-without-action" shape the root-cause analysis
named (`inc-i-188-analysis.md:47-48`). The new hint matches the manual recovery verified live on
vm-server (`inc-i-188-analysis.md:15`, `reset-failed` + `start`); `restart` is equivalent to `start`
for a stopped or failed unit, so substituting it is consistent with the plan the code itself runs.
**Consistent** with `systemd_restart_plan` in verb, order, and unit threading.

Two residual nits, both LOW and both non-blocking:

- **[F1]** The hint chains with `&&`, so if `reset-failed` exits non-zero the operator's shell
  SKIPS the `restart` — the opposite of the best-effort semantics the code was written to have
  (`required: false`). Practical impact is small: `systemctl reset-failed` returns 0 for a loaded
  non-failed unit and only fails for an unknown unit, where `restart` would fail anyway. Being
  honest about the bound: this is an inconsistency between the executed contract and the printed
  contract, not a defect that reproduces the incident. `;` would express the intent exactly.
- **[F2]** The hint is a hand-written literal duplicated at `:82` and `:189`. It is NOT derived
  from `systemd_restart_plan`, so if the plan ever gains or reorders a step, both strings go stale
  and **every existing test still passes** — the structural tests only guard the argv literal and
  the call count, never the hint text. That is the one place where the two call sites can still
  drift apart.

## Failure Mode Validation

| Failure Scenario | Triggered | Detected | Recovered | Degraded OK | Notes |
|---|---|---|---|---|---|
| Unit in start-limit lock (the incident) | Simulated (stubbed `sudo`) | Yes | Yes | Yes | `reset-failed` issued first at both sites — the lock is cleared before `restart`. Cannot be triggered for real on macOS |
| `reset-failed` fails (unit never failed / unknown) | Yes (case 1) | n/a by design | Yes | Yes | Restart proceeds; REQ-188-002 |
| `restart` fails after a successful `reset-failed` | Yes (case 2) | Yes | No (by design) | Yes | Single failure line + hint; `any_ok`/return `false` |
| `sudo` binary missing | Yes (read + reasoning) | Yes | No | Yes | `Err` → `false` → hint. No panic |
| `systemctl list-unit-files` unavailable / non-zero | Not triggered | Yes | n/a | Yes | `upgrade_restart.rs:127-129` returns `false` → falls through to tier 3. Untouched by M1 |
| Template unit selected by discovery | Yes (case 6) | **No** | No | Partially | Two doomed `sudo` calls per template unit; operator sees a spurious `Restarted doli-mainnet@.service.` if the stub-equivalent returns 0. Pre-existing ([F5]) |
| Real systemd start-limit lock on a real host | **Untestable in this environment** | — | — | — | macOS build host, no systemd. Requires a Linux staging node |

## Security Validation

| Attack Surface | Test Performed | Result | Notes |
|---|---|---|---|
| Command injection via unit name (plan builder) | `systemd_restart_plan("doli-mainnet-n3.service; rm -rf / #")` | **PASS** | 3-element argv, verbatim, no split/escape (`inc_i_188_upgrade_reset_failed_test.rs:156-185`) |
| Command injection via unit name (real execution) | Probe case 5 — ran the binary with `doli-mainnet.service; touch /tmp/PWNED_INC188 #` and a stubbed `sudo` | **PASS** | Single argv element; `/tmp/PWNED_INC188` NOT created. `Command::args` never invokes a shell |
| Privilege escalation via the new `sudo` verb | Reviewed `scripts/install.sh:192-199` sudoers whitelist vs. the new `sudo systemctl reset-failed` | **PASS** | No new privilege class: the pre-fix code already ran `sudo systemctl restart`. `doli upgrade` runs as root anyway |
| Sensitive data in new output | Reviewed both hint strings and `run_systemd_plan` | **PASS** | Only the unit name is echoed; no paths, tokens, or keys |
| Unit-name confusion (`reset-failed` on an unintended unit) | Probe case 6 discovery output | **PASS** | `reset-failed` targets exactly the unit that `restart` targets — same `unit` string, same plan. No widening |
| `sh -c` root RCE in `try_restart_process` | — | **Out of Scope** | Banked as `REV2-172-001` (P1) per task constraints; code untouched by M1 |
| `find_doli_node_path` pgrep-derived target | — | **Out of Scope** | Banked as `AUDIT-P2-011` (P2); code untouched by M1 |

## Platform-Coverage Risk (explicit assessment)

**The risk is real and was NOT covered by the developer's evidence. I closed it myself.**

`try_restart_systemd` is `#[cfg(target_os = "linux")]` (`upgrade_restart.rs:121`). On this
aarch64-apple-darwin host, `cargo build`, `cargo clippy --all-targets`, and `cargo test` do **not
compile, type-check, lint, or execute a single line of its body** — including the M1 edit at
`:184`. `rustup target list --installed` shows only `aarch64-apple-darwin` and
`aarch64-apple-ios`, so no cross-target `cargo check` was available.

Evidence that existed BEFORE this QA pass, and why each is insufficient on its own:

1. `test_upgrade_restart_source_routes_both_call_sites_through_plan` counts `systemd_restart_plan(`
   occurrences ≥ 2 in the SOURCE TEXT. This proves a string is present. It does not prove the
   Linux body compiles, that the call type-checks, or that `unit: &String` coerces to `&str`.
2. `test_upgrade_restart_source_has_no_raw_restart_argv_literal` proves no
   `["systemctl","restart",` literal remains. Also text-only, and it is a partial guard: it would
   NOT catch `.args(["systemctl".to_string(), "restart".to_string(), ...])` or a chained
   `.arg("systemctl").arg("restart")`.
3. CI (`.github/workflows/ci.yml`) runs clippy, test, and build on `ubuntu-latest`, so the Linux
   body IS type-checked and linted — **but only after a push**. The change is uncommitted; that
   evidence does not exist yet.

**Sufficiency verdict: the pre-existing evidence was NOT sufficient.** I therefore produced the
missing evidence directly:

- **Type-check**: compiled a scratchpad copy of `upgrade_restart.rs` with the `linux` cfg attributes
  stripped (macOS ones disabled) under `rustc --edition 2021 --crate-type lib` → **exit 0**, rlib
  produced. The Linux body, including `run_systemd_plan(&systemd_restart_plan(unit))` where `unit`
  is `&String` from `for unit in &units`, compiles.
- **Execution**: built the same harness as a binary and ran `restart_doli_service(None)` with
  stubbed `systemctl` and `sudo` → the Linux path produced the correct `reset-failed`-then-`restart`
  argv sequence per unit (probe case 6 above).

Residual risk after this: **low**. The only remaining unverified difference between the harness and
a real Linux build is the platform std behaviour of `std::process::Command`, which is not
platform-divergent for the constructs used here. Recommendation: let CI's ubuntu-latest clippy/test
job be the confirming gate before merge — it will be the first real Linux compile of this code.

## Specs/Docs Drift

| File | Documented Behavior | Actual Behavior | Severity |
|---|---|---|---|
| `docs/cli.md:1918-1940` | Describes `doli upgrade` options and the maintainer-signature trust root only; makes no claim about the post-install restart path | Matches — no claim to contradict | none |
| `docs/troubleshooting.md` | No mention of `start-limit`, `reset-failed`, or "Start request repeated too quickly" | Matches (nothing stale) | none |
| `docs/bugfixes/inc-i-188-analysis.md:71` | Milestone M1 status: `PENDING` | M1 is implemented and passing | low — status field should move to DONE at commit time |

**Recommendation (non-blocking, not a defect):** `docs/troubleshooting.md` has no entry for the
start-limit-lock symptom, which is now a known, reproducible mode of this system. Adding one during
`/sync-docs bins/cli` would be net-positive. This is an ADDITION, not drift.

## Blocking Issues

**None.**

## Non-Blocking Observations

- **[OBS-188-001]** `bins/cli/src/upgrade_restart.rs:82,189` — change `&&` to `;` in the hint so the
  printed recovery matches the best-effort semantics the code implements. ([F1])
- **[OBS-188-002]** `bins/cli/src/upgrade_restart.rs:82,189` — derive the hint text from
  `systemd_restart_plan` (e.g. a `fn plan_as_shell(unit) -> String` next to the plan) so the last
  hand-duplicated pair cannot drift. Would also make the hint test-covered, which it currently is
  not. ([F2])
- **[OBS-188-003]** `bins/cli/src/upgrade_restart.rs:11-20` — if `run_systemd_plan` ever carries more
  than one required step, make `ok` a conjunction (`ok &= ...` with `ok` seeded per-required-step)
  rather than an overwrite. Correct today; the doc comment says "the required step" (singular), which
  is honest. ([F3])
- **[OBS-188-004]** `bins/cli/src/cmd_service.rs:683-689` (`doli service restart`),
  `bins/cli/src/cmd_snap.rs:516` (post-snap-restore restart), `bins/cli/src/cmd_chain.rs:276` — same
  start-limit-lock failure class, still on a bare `systemctl restart`. `doli service restart` is the
  one an operator reaches for first, so it is the highest-value follow-up. Out of REQ-188-001 scope
  (which names the upgrade restart path); worth a follow-up milestone now that the plan exists. ([F4])
- **[OBS-188-005]** `bins/cli/src/upgrade_restart.rs:132-136` — filter out systemd template units
  (unit names containing `@.`) during discovery. Pre-existing; M1 doubles the wasted work per
  template unit and can print a misleading `Restarted doli-...@.service.` ([F5])
- **[OBS-188-006]** `bins/cli/src/upgrade_restart.rs:76` — reject an empty/whitespace `--service`
  value early instead of reporting `Restarted .`. Pre-existing. ([F6])
- **[OBS-188-007]** `crates/updater/src/apply.rs:229-244` — the `rm`-then-`cp` window that CREATES the
  203/EXEC burst is still open (explicitly deferred in `inc-i-188-analysis.md:22`). M1 makes each
  upgrade self-heal from the lock, which is the right first fix, but the fleet will keep entering the
  lock on every upgrade until the installer/sudoers rollout decision is made. ([F7])

## Modules Not Validated

None within scope. `crates/updater/**` was read for context only and is excluded by task constraint.

## Final Verdict

**PASS** — Both REQ-188-001 (Must) and REQ-188-002 (Should) are met, verified against the code and
against running binaries rather than against narration. The fix addresses the stated root cause
structurally: the command ordering exists in exactly one pure function that both call sites consume,
so the two sites cannot drift. All four gates pass with 245/245 tests green, 0 clippy warnings, and
`cargo fmt --check` clean. The platform-coverage gap on the Linux-gated body was real and is now
closed by a forced-cfg type-check plus an execution probe. Seven low-severity observations are
recorded; none block the milestone, and five are pre-existing or explicitly deferred. Approved for
review.
