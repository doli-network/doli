━━━ FINDINGS — 6 total (Major:1 Minor:5) ━━━

  [F1] MAJOR conf(0.90, observed) — bins/cli/src/cmd_upgrade.rs:253-263 — `doli upgrade` prints "Upgrade to vX complete!" and returns `Ok(())` whether the restart succeeded, failed, or found nothing; the reporting half of INC-I-188 is untouched by M1. PRE-EXISTING, not a regression.
  [F2] MINOR conf(0.85, observed) — bins/cli/src/upgrade_restart.rs:160-175 — ADJUDICATION of the logic(P1 INTRODUCED) vs auth(P3 INTRODUCED) split on the `is-active` pre-filter: it is PRE-EXISTING (commit b5f68bba, 2026-08-10; zero diff hunks in that range) and unreachable on the incident path. My ruling: P2/Minor, PRE-EXISTING.
  [F3] MINOR conf(0.85, observed) — bins/cli/src/upgrade_restart.rs:82-83,189-190 — the new operator hint chains with `&&`, which short-circuits exactly the restart the code deliberately makes unconditional; the hint is also near-redundant because the tool just ran both commands. Ruling on developer deviation (b): use `;` and add a diagnostic pointer.
  [F4] MINOR conf(0.80, observed) — bins/cli/src/upgrade_systemd_plan.rs:14-33 — the diff clears systemd's StartLimitBurst circuit breaker (an external protection mechanism) with no `protection_mechanisms` row; interaction analysis completed here and is clean, but the traceability record is missing.
  [F5] MINOR conf(0.75, inferred) — docs/bugfixes/inc-i-188-analysis.md:71 + docs/troubleshooting.md — M1 status row still reads `PENDING`; the operator troubleshooting doc has no start-limit-lock entry although docs/postmortems/2026-04-19-orphan-fork-rollback-cascade.md:156 already records `reset-failed` as the manual remedy.
  [F6] MINOR conf(0.80, observed) — bins/cli/src/cmd_service.rs:677-692, cmd_snap.rs:509-520, cmd_chain.rs:269-277 — three sibling restart/start sites share the failure class and the M1 drift guard is file-scoped to upgrade_restart.rs; ruling: deferring is CORRECT for M1, and QA F4's characterisation of two of the three sites is imprecise.

  Speculative: 1 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Code Review: INC-I-188 M1 — `reset-failed` precedes `restart` in the CLI upgrade restart path

- **Reviewer:** OMEGA reviewer (doctor-mode, post-QA, post-5-auditor-sweep)
- **Run:** 530 · **Incident:** INC-I-188 · **Milestone:** M1
- **State under review:** working tree at `HEAD = 9e27bd19`, all M1 changes UNCOMMITTED
- **Package:** `doli-cli` · **Consensus impact:** NONE (CLI-only, no node, no state root, no activation height)
- **Verdict:** **APPROVED** — commit M1 as-is, or with the two-line F3 nit folded in first.

---

## Scope Reviewed

| Artifact | Lines | State |
|---|---|---|
| `bins/cli/src/upgrade_systemd_plan.rs` | 33 | NEW (untracked) |
| `bins/cli/src/lib.rs` | +1 | modified |
| `bins/cli/src/upgrade_restart.rs` | +30 / -21 | modified |
| `bins/cli/tests/it/inc_i_188_upgrade_reset_failed_test.rs` | 239 | NEW (untracked) |
| `bins/cli/tests/it/main.rs` | +1 | modified |

Read for context, not reviewed as change: `bins/cli/src/cmd_upgrade.rs:70-100,185-264`, `bins/cli/src/cmd_service.rs:675-700`, `bins/cli/src/cmd_snap.rs:505-524`, `bins/cli/src/cmd_chain.rs:265-284`, `bins/cli/Cargo.toml`, `bins/cli/src/main.rs:12-36`.

**Out of scope, honoured:** `crates/updater/**`; banked pre-existing findings `REV2-172-001` (P1) and `AUDIT-P2-011` (P2) — both re-reported by the injection and auth auditors, both correctly labelled PRE-EXISTING by them, neither re-litigated here.

**Prior stages consumed, not re-derived:** `docs/qa/inc-i-188-M1-qa-report.md` (PASS, 0 blocking, gates green, forced-cfg Linux-body execution harness); `docs/.workflow/security-audit-{injection,auth,crypto,logic}-M1.md`.

---

## 1. Doctor-mode root-cause test — **PASS (root-cause, not a patch)**

### 1.1 Does `reset-failed` before `restart` actually resolve start-limit lock?

**Yes, and it is the only operation that does.** A unit that has hit `StartLimitBurst` within `StartLimitIntervalSec` enters `failed (start-limit-hit)`. In that state systemd *refuses* `systemctl restart` (and `systemctl start`) with "start request repeated too quickly" — the refusal is issued by the job engine before ExecStart is ever evaluated, so no amount of retrying `restart` can clear it. `systemctl reset-failed <unit>` zeroes the start-rate counter and the failed state; the very next `start`/`restart` is then admitted.

The fix is therefore aimed at the actual blocking mechanism, not at a downstream symptom.

**Corroborating evidence that this is the known remedy and was simply never automated:** `docs/postmortems/2026-04-19-orphan-fork-rollback-cascade.md:156` already prescribes "`systemctl reset-failed` for services that hit StartLimitBurst" as a manual operator step — recorded four months before INC-I-188. An institutional lesson existed in the postmortem corpus and was never wired into the code path that creates the condition. M1 closes that loop.

**The lock is transient state, not a persistent fault.** By the time `restart_doli_service` runs (`cmd_upgrade.rs:257`), `install_binary` has already returned `Ok` (`cmd_upgrade.rs:218-230`) — a valid binary is on disk. So clearing the counter is *sufficient*: the next start attempt executes a good ExecStart. That is what makes this a root-cause fix rather than a retry loop dressed up as one.

### 1.2 Does single-sourcing genuinely prevent the two call sites from drifting?

**Within `upgrade_restart.rs`: yes, and it is test-enforced. Outside it: no, and that limit should be stated rather than assumed away.**

Three independent guards, all in `bins/cli/tests/it/inc_i_188_upgrade_reset_failed_test.rs`:

1. `test_systemd_restart_plan_orders_reset_failed_before_restart` (:82-103) asserts `plan.len() == 2` **and** the exact argv **and** the `required` flag of each step positionally. A future edit that swaps the two steps — the failure that would silently restore the deadlock while still "containing reset-failed" — fails this test. This is the assertion that matters most and it is correctly positional rather than a `contains` check.
2. `test_upgrade_restart_source_has_no_raw_restart_argv_literal` (:204-215) forbids the whitespace-stripped literal `["systemctl","restart",` anywhere in the source file.
3. `test_upgrade_restart_source_routes_both_call_sites_through_plan` (:223-239) requires ≥ 2 occurrences of `systemd_restart_plan(`. Verified no false positive from the `use` line: `use doli_cli::upgrade_systemd_plan::{systemd_restart_plan, SystemdStep};` strips to `...{systemd_restart_plan,SystemdStep};` — the needle requires a following `(`, which the import does not supply.

Guard (2) is not airtight — a literal written as `vec!["systemctl".to_string(), "restart".to_string(), unit]` evades the substring check. That is acceptable: guard (3) still requires both sites to reach the plan, and guard (1) still pins the plan's shape. The realistic drift (someone adds a third restart in this file, or reorders the plan) is caught.

The honest limit: all three guards are scoped to one file. A new `systemctl restart` added in `cmd_service.rs` or anywhere else trips nothing. See F6.

### 1.3 What the fix does NOT do — stated, not hidden

The failure chain is:

1. `install_binary` overwrites the node binary while the unit is running (`cmd_upgrade.rs:218`).
2. systemd's `Restart=` auto-restarts, ExecStart hits `203/EXEC` 4× inside the swap window → start-limit lock.
3. The upgrade issues `systemctl restart` → **refused**.
4. `cmd_upgrade` prints "Upgrade to vX complete!" and exits 0 while the node is DOWN.

**M1 fixes step 3 completely.** Steps 1-2 (the lock being created at all) and step 4 (the false success report) remain.

- On **steps 1-2**: preventing the lock would require `stop → swap → start`, which deliberately takes the node down for the duration of a download-and-install and changes the command's contract. Under SSF (rule 18), `reset-failed` is the correct minimal remedy — it makes the existing contract work rather than replacing it. **Not a defect; no change recommended.**
- On **step 4**: this is a real residual and it is the *stated symptom of the incident title*. See **F1**. It is pre-existing and out of REQ-188-001/002 scope, so it does not block M1, but it must not be left unowned.

**Contradiction check (rule 20):** the stated fix ("reset-failed precedes every restart, single-sourced, both sites consume one plan") matches the shipped code exactly — `upgrade_restart.rs:78` and `:184` both call `run_systemd_plan(&systemd_restart_plan(unit))`. No contradiction between claim and diff.

---

## 2. Unintended behaviour change — **NONE. Verified.**

`git diff HEAD -- bins/cli/src/upgrade_restart.rs` contains exactly three hunks: `@@ -4,6 +4,21 @@` (the `use` line + new `run_systemd_plan`), `@@ -60,15 +75,13 @@` (`restart_specific_service` body), `@@ -168,18 +181,14 @@` (the `try_restart_systemd` restart loop). Everything else in the file has **zero** hunks:

| Untouched | Location | Evidence |
|---|---|---|
| `find_doli_node_path` (all 3 tiers) | `upgrade_restart.rs:23-73` | appears only as diff context; pgrep tier, `which` tier and the 4 fixed paths byte-identical |
| `restart_doli_service` tier selection | `:91-116` | no hunk; systemd → launchd → process order unchanged |
| launchd tier `try_restart_launchd` | `:198-244` | no hunk; `kickstart -k` + stop/start fallback unchanged |
| process tier `try_restart_process` | `:247-315` | no hunk; SIGTERM + `sh -c nohup` respawn unchanged |
| `get_uid` | `:318-347` | no hunk |
| `try_restart_systemd` unit discovery + both filter branches | `:123-179` | no hunk; only the loop at `:181-193` changed |

Arithmetic corroborates: diffstat is 32 insertions / 21 deletions across 3 files, of which `lib.rs` is +1 and `tests/it/main.rs` is +1, leaving `upgrade_restart.rs` at +30/-21 — exactly the 15-line helper + `use` line + the two rewritten blocks, with nothing left over for a silent edit elsewhere.

**Module wiring is correct and follows the existing project pattern.** `bins/cli/src/main.rs:12-36` declares `mod upgrade_restart;` but **not** `mod upgrade_systemd_plan;`, so the plan module compiles into the `doli_cli` lib target only and the binary consumes it via `use doli_cli::upgrade_systemd_plan::...`. Single definition, no duplicate type. This mirrors `producer_ledger` (INC-I-180 M3), which `lib.rs:3` already exposes the same way. `Cargo.toml` has no `[lib]` stanza, so the lib name defaults to `doli_cli` — correct.

**Blast radius (code graph, not grep):** `blast.py --hops 2` on `bins/cli/src/upgrade_restart.rs` and on `restart_doli_service` both return a single dependent, `bins/cli/src/cmd_upgrade.rs`. No consumer outside `cmd_upgrade` was overlooked.

### One genuinely new behaviour, and it is an improvement

There is a **third** entry point QA correctly spotted: `cmd_upgrade.rs:80-89`, the "binary already at vX, restarting service" branch taken when `--service` is supplied and no upgrade is needed. That path now also issues `reset-failed`. QA called it "behaviour change, benign". **I go further: it is the single most valuable new capability in this milestone for INC-I-188 recovery.** An operator whose unit is start-limit locked after a failed upgrade re-runs `sudo doli upgrade --service doli-mainnet.service`, hits the already-up-to-date branch, and now gets an unlock+restart instead of a refused restart. `reset-failed` on a healthy or never-failed loaded unit is a no-op. Explicitly approved.

---

## 3. Rulings on the two deliberate developer deviations

### 3.1 Deviation (a) — the developer reformatted the test file it was told not to touch — **CORRECT CALL. Approved.**

**Ruling: the developer made the right decision, and the verification method used to clear it was the right one.**

The "developer does not modify the test-writer's tests" rule exists to stop a developer from weakening assertions to make a red test go green. It is a rule about *semantic* tampering. `cargo fmt --check` is a hard gate (project Law 4) and it applies to every file in the workspace including test files. Leaving the gate red is not an option, and escalating a whitespace conflict would burn a round-trip for zero information. Reformatting is the correct resolution — **provided** semantic identity is proven, which it was.

**The evidence offered is sufficient and, for this particular file, unusually strong.** The whitespace-stripped hash being unchanged is a complete proof that no non-whitespace character was added, removed, or reordered — which, for Rust, is a proof of semantic identity for everything except whitespace *inside* string literals. Independently confirmed: `cargo fmt --check` on the workspace now exits 0 with empty output, so the gate the deviation was taken for is genuinely satisfied.

**One honest caveat, harmless here.** rustfmt can re-wrap `\`-continued string literals, which changes whitespace *inside* an assertion message without changing the stripped hash. In this file the only such literals are `assert!`/`assert_eq!` failure messages (`:68-69`, `:117-118`, `:209-213`, `:233-237`) — diagnostic text, never load-bearing. The two raw/needle literals that *are* load-bearing (`r#"["systemctl","restart","#` at `:208` and the `"systemd_restart_plan("` needle at `:225`) contain no whitespace and are re-stripped at runtime by `upgrade_restart_source_normalised()` (`:189-195`) anyway. So the proof holds where it needs to.

**Recommendation:** record the reformat in the M1 commit message body (one line: "test file reformatted for `cargo fmt --check`; whitespace-only, stripped-hash unchanged") so a future reader cannot mistake it for test tampering. Not a code change.

### 3.2 Deviation (b) — the `&&` in the operator failure hint — **QA F1 is right in principle; my ruling goes one step further than `;`**

**Ruling: yes, `&&` is wrong. But swapping it for `;` fixes half the problem. Replace the hint, do it now, and it still does not block approval.**

Two defects, not one:

**(i) The `&&` contradicts the semantics the code implements.** `run_systemd_plan` (`upgrade_restart.rs:11-20`) runs `reset-failed` and discards its status *by design* — that is REQ-188-002, and the test at `:112-125` pins it. The hint then tells the operator to run the same pair with `&&`, which aborts the restart if `reset-failed` returns non-zero. The tool and its own advice disagree about the one semantic the milestone exists to establish. The practical harm is small (`systemctl reset-failed` returns 0 for any loaded unit, failing only on an unknown unit — where restart would fail too), which is why this is Minor and not Major. But the *documented* semantics diverging from the *implemented* semantics is exactly the drift class M1 was chartered to eliminate, so leaving it is inconsistent with the milestone's own thesis.

**(ii) The hint is now largely redundant, which `;` does not fix.** The hint prints only after the required `restart` step failed — i.e. after the tool already ran `reset-failed` *and* `restart` itself. Telling the operator to re-run the exact pair that just failed is low-value advice. It retains *marginal* value for the auth-failure case (`sudo` absent, polkit denial, non-interactive sudo prompt), where re-running under a working root shell would succeed. So do not delete it — improve it.

**Concrete recommendation** (two string literals, `upgrade_restart.rs:82-83` and `:189-190`):

> `Failed to restart {u}. reset-failed and restart were both attempted. Retry as root: sudo systemctl reset-failed {u}; sudo systemctl restart {u} — then diagnose: sudo systemctl status {u}; sudo journalctl -u {u} -n 50 --no-pager`

`;` instead of `&&` (matches the best-effort semantics), plus a diagnostic pointer (restores real value), plus an explicit statement that both were already tried (removes the false impression that the tool did not try).

**Worth fixing now?** Yes — the file is uncommitted, the change is two string literals, the runtime risk is zero, and the injection auditor's SEC-INJECTION-001 (P3, unquoted unit name in copy-pasteable root shell text) is *reduced* by dropping to a single-verb-per-statement form. **But it does not gate approval:** it is cosmetic operator text with no behavioural coupling, and the milestone is correct without it. If the orchestrator prefers a clean M1 boundary, fold it into M2.

---

## 4. Adjudication — the `is-active` pre-filter contradiction (logic P1 INTRODUCED vs auth P3)

Two auditors reported the same code at `bins/cli/src/upgrade_restart.rs:160-175` with incompatible severities:

- **logic** `[F1] P1 conf(0.70, observed)` — SEC-LOGIC-001: "the `is-active` pre-filter removes exactly the failed units the new `reset-failed` step exists to recover, so on the `installed_path == None` branch M1's fix can never reach a down unit."
- **auth** `[F4] P3 conf(0.7, observed)` — SEC-AUTH-004: same mechanism, labelled **INTRODUCED**, "so REQ-188-001 does not hold on that path."

Note both auditors call it INTRODUCED; they disagree only on severity. Both are wrong on the classification, and neither is right on severity.

### 4.1 The classification: **NOT INTRODUCED — PRE-EXISTING since 2026-08-10. Both auditors are refuted.**

`git log -1 -L 160,175:bins/cli/src/upgrade_restart.rs` attributes the entire range to commit **`b5f68bba` (2026-08-10) "refactor(updater): INC-I-172 M1+M2 fail-closed maintainer trust root"** — the commit that created the file. The M1 diff contains **no hunk** touching `:160-175`; the sole hunk inside `try_restart_systemd` is `@@ -168,18 +181,14 @@`, which rewrites the restart loop at `:181-193` only.

The logic auditor's own note (`security-audit-logic-M1.md:39-41`) concedes the uncertainty — "I cannot tell whether 'stopped on purpose' is a real operational state on these hosts, and that decides…" — and both auditors then defaulted to INTRODUCED under the "if you cannot tell, it is INTRODUCED" convention. That convention is correct as a *safety* default for reachability questions, but INTRODUCED-vs-PRE-EXISTING is not a judgement call: it is decided by one `git log -L` invocation. What M1 introduced is a new *reason to care* about a pre-existing filter. That is not the same as introducing the defect, and conflating the two inflates a milestone's apparent regression count.

### 4.2 The reachability trace: **the orchestrator's trace is VERIFIED, not refuted.**

`installed_node_path` (`cmd_upgrade.rs:211`) is `Some` **iff all three hold**: (a) `extract_named_binary_from_tarball(&tarball, "doli-node")` returned `Ok` (`:212`), (b) `doli_node_path.or_else(find_doli_node_path)` resolved to a path (`:215-216`), and (c) `install_binary` returned `Ok` — otherwise the function `return Err`s at `:222-228` and never reaches the restart at all. So `installed_node_path == Some(p)` ⟺ **a node binary was actually written to disk on this run**.

The INC-I-188 scenario *is* the binary-swap scenario: the 4× `203/EXEC` failures are caused by the swap, so the swap demonstrably happened, so `installed_node_path` is `Some`. `cmd_upgrade.rs:257` passes it to `restart_doli_service`, which forwards it to `try_restart_systemd`, where `upgrade_restart.rs:143` `if let Some(bin_path) = installed_path` selects the **ExecStart-match** filter (`:145-159`). The `is-active` filter at `:160-175` is on the `else` arm and is **not on the incident path**. Trace confirmed.

### 4.3 What is actually left on the `None` branch — the part both auditors under-analysed

The `None` branch is reached only when (a) `doli-node` was absent from the tarball (`cmd_upgrade.rs:236-238`) or (b) `find_doli_node_path()` returned `None` with no `--doli-node-path` (`:231-234`). **In both cases `cmd_upgrade` has already printed an explicit skip message** — "doli-node not in tarball, skipping node binary install" or "doli-node not found on system, skipping node binary install. / Hint: use --doli-node-path <PATH>". No binary was installed, so no swap-induced lock was created by this run, and a restart on that branch would only re-launch an *unchanged* binary.

There is one real correlation that makes the logic auditor's instinct non-crazy and that I want on record: `find_doli_node_path` Tier 1 is `pgrep -a doli-node` (`upgrade_restart.rs:27-43`), which **fails precisely when the node is down**. Tiers 2-3 (`which`, plus four fixed paths at `:60-65`) can also miss on a per-service-binary mainnet layout. So "node is down" correlates with "land on the `None` branch". But the consequence there is *not* a silently skipped restart — it is a loudly skipped **install**, with an actionable hint naming the exact flag that fixes it. The operator is not misled.

### 4.4 My severity ruling: **P2 / Minor — PRE-EXISTING, non-blocking, tracked for M2.**

- Not **P1**: unreachable on the incident path (§4.2), not introduced by this diff (§4.1), no regression, and the branch where it applies emits a visible actionable message rather than failing silently. A P1 on this milestone would be a false regression signal.
- Not **P3**: the down-node ↔ `None`-branch correlation in §4.3 is real, and on that branch the filter genuinely defeats the intent of the new step. It deserves a tracked follow-up, not a shrug.

**Concrete M2 remedy** (recorded so the follow-up is not re-derived): the filter's purpose is to avoid starting units an operator deliberately stopped on a multi-node host — a legitimate goal, so do **not** simply delete it. Change the predicate from `is-active` to `is-active OR is-failed`: a unit in `failed`/`start-limit-hit` is by definition *not* a deliberately-stopped unit, so admitting it is safe and restores REQ-188-001's intent on that branch. Optionally also make ExecStart matching independent of `pgrep` (e.g. `systemctl show -p FragmentPath`), which removes the down-node correlation at its source.

---

## 5. Specs / docs accuracy

| File | Needs update? | Basis |
|---|---|---|
| `docs/cli.md` §18 (`:1910-1958`) | **No** | It documents flags and the trust-root/provenance model. It never documented post-upgrade restart behaviour, so M1 introduces **no drift**. Adding a sentence would be an improvement, not a correction. |
| `specs/*.md` | **No** | `grep -n "start-limit\|reset-failed\|StartLimit" specs/*.md` → no matches. No spec asserts anything about the CLI restart path; nothing to bring back into sync. `specs/SPECS.md` needs no new entry (no new spec domain). |
| `docs/troubleshooting.md` | **Yes — gap, not drift** | Same grep → no matches for `start-limit`, `203/EXEC`, or `reset-failed`. Meanwhile `docs/postmortems/2026-04-19-orphan-fork-rollback-cascade.md:156` already lists `reset-failed` as the manual remedy. The operator-facing troubleshooting doc lacks the failure mode that just produced an incident. Add a short entry: symptom (`status=203/EXEC`, unit `failed (start-limit-hit)`, `restart` refused), cause (binary swap window), remedy (`sudo systemctl reset-failed <unit>` — now automatic in `doli upgrade` ≥ this release). |
| `docs/bugfixes/inc-i-188-analysis.md:71` | **Yes — required before close-out** | The M1 row still reads `Status = PENDING`. Flip to `DONE` (or `IMPLEMENTED (uncommitted)` until the commit lands) and add the M2 scope items from F1, F2 §4.4, F3 and F6 so they are not lost. |
| `docs/DOCS.md` index | **No** | No new doc file created by M1. |

*(F5 covers the two "Yes" rows.)*

---

## 6. Missed opportunities and changes that went too far

### 6.1 Did the change go too far? **No.**

The diff adds 33 lines of new module + a 15-line private executor and rewires two call sites. It introduces no abstraction that is not consumed, no configuration, no feature flag, no trait. `SystemdStep` has exactly the two fields the plan needs. Wiring census (rule 32): `pub fn systemd_restart_plan` and `pub struct SystemdStep` both have production callers in `upgrade_restart.rs:78,184` — no `wiring-debt.md` row required (QA independently confirmed this). Module budget (rule 19): `upgrade_restart.rs` 347 lines, `upgrade_systemd_plan.rs` 33 lines, test file 239 lines — all within budget. **SSF-compliant; nothing to remove.**

### 6.2 `run_systemd_plan`'s last-write-wins `ok` — QA F3 / logic F3 flagged it; **I rule it a non-issue, with evidence neither found**

`upgrade_restart.rs:11-20` sets `ok = matches!(...)` (assignment, not `&&=`) inside the `required` branch, so a plan with two required steps would report only the *last* step's status and could mask an earlier failure.

This is defended by the test suite, which is why it needs no code change: `test_systemd_restart_plan_orders_reset_failed_before_restart` (`:86-90`) asserts `plan.len() == 2`, and `test_systemd_restart_plan_marks_reset_failed_best_effort_and_restart_required` (`:112-125`) pins `!plan[0].required && plan[1].required`. Any future edit that adds a second required step changes `plan.len()`, fails the first assertion, and is caught in CI. The function's doc comment (`:9-10`) also states the singular contract explicitly. Adding `saw_required` bookkeeping for a shape the tests already forbid would be complexity for a hypothetical — an SSF violation. **No change recommended.**

### 6.3 QA F4's sibling sites — **deferring is CORRECT for M1. Two of QA's three descriptions need correcting.**

| Site | What it actually is | Ruling |
|---|---|---|
| `cmd_service.rs:677-692` (`doli service restart`) | Bare `systemctl restart`, then a `sudo` retry. Genuinely the same failure class. | **Highest-value M2 target.** This is the command an operator reaches for *after* INC-I-188, and it will be refused on a locked unit. Mitigating factor: it is interactive and surfaces the systemd error, so the operator is not silently misled the way `doli upgrade` misled them. Deferring is defensible; M2 should start here. |
| `cmd_snap.rs:509-520` | `let _ = Command::new("sudo").args(["systemctl","restart",service]).output();` then **unconditionally** `println!("  Restarted {}", service)`. | **QA under-rated this.** It is not merely a missing `reset-failed` — it is a *stronger* instance of F1: the exit status is discarded and success is printed regardless. Post-snap-restore is exactly when a unit is most likely to be in a failed state. M2 should fix both halves here. |
| `cmd_chain.rs:269-277` | **`systemctl start`, not `restart`** — QA F4's label is imprecise. | The start-limit failure class *does* apply (`start` is refused identically), so the concern is valid, but the code already has a `sudo` fallback and prints a neutral message. Lowest priority of the three. |

**Why deferring is right:** REQ-188-001 is scoped to "the upgrade restart path", the three siblings live in different commands with different operator contexts and different correct remedies (`cmd_snap` needs a status check *and* `reset-failed`; `cmd_chain` needs neither urgently), and pulling them in would turn a 33-line surgical fix into a cross-command refactor mid-incident. The developer's design choice pays off here: `upgrade_systemd_plan` now lives in the `doli_cli` **lib**, so extending it to `cmd_service`/`cmd_snap` in M2 is a `use` line plus a call — the cheap-extension property is a point in favour of approving M1 as shipped.

**Caveat to record (this is F6's substance):** the anti-drift guarantee is file-scoped. Tests 2 and 3 read only `src/upgrade_restart.rs`. A future `systemctl restart` added in any other file trips nothing. If M2 extends the plan to the siblings, the structural test should be widened to scan `src/**` for the raw literal, or replaced by an injectable-runner unit test under `#[cfg(target_os = "linux")]` (which QA's forced-cfg harness has already demonstrated is feasible).

---

## 7. System impact (rule 29 — `.omega/gauntlet.conf` present)

**Is a protection mechanism added or tuned?** Not one of ours — but the diff **clears an external one**. systemd's `StartLimitBurst`/`StartLimitIntervalSec` is a circuit breaker that stops a crash-looping unit from thrashing the host. `systemd_restart_plan` now trips that breaker open before every restart on this path. That is squarely in scope for this section, so I answered the interaction questions rather than skipping them.

**`SELECT * FROM v_protection_surface;` → 40 active mechanisms.** All 40 are node-internal (consensus, gossip, sync, mempool, maintainer-state) except `PM-020` (installer logrotate) and `PM-172-07` (release-signing argument shape, CLI). **No registered mechanism shares a trigger surface with "an operator runs `doli upgrade` and the CLI invokes `systemctl` on a host unit"** — the CLI runs out-of-process and can only start/stop a unit; it emits no block, no gossip, no mempool entry, no consensus verdict.

The one adjacency worth naming and answering:

- **`PM-003` (memory watchdog) and `PM-012` (GOSSIP_WATCHDOG)** run *inside* the node process and can terminate it. Repeated terminations feed systemd's start-limit counter. So systemd's breaker sits **downstream** of PM-003/PM-012, and M1 now clears that downstream breaker.
- **Can one's action create the other's trigger (feedback loop)? NO — and this is the question that actually mattered.** Clearing the breaker is reachable **only** from an operator-initiated `doli upgrade`: `blast.py --hops 2` on both `upgrade_restart.rs` and `restart_doli_service` returns exactly one dependent, `cmd_upgrade.rs`. The node's own auto-update path does **not** use it — it calls `updater::restart_node()` (`crates/updater/src/apply.rs:599`), an in-process re-exec that never shells out to `systemctl`. There is therefore no automated agent that can clear the breaker in a loop. A genuinely broken binary crash-loops, the breaker re-arms after `StartLimitBurst`, and it stays armed until a human runs the command again. **Bounded. No amplification.**
- **Can one starve the input the other needs to disarm? NO** — `reset-failed` mutates only systemd unit state; it consumes no resource PM-003/PM-012 depend on.
- **Scale sensitivity: N-A.** The diff introduces **no numeric threshold**. The plan is a fixed two-step constant; `StartLimitBurst` is systemd's own value, unread and unwritten by this code. There is no constant here that could be calibrated for one fleet size and misbehave at another.

**Gap (F4):** no `protection_mechanisms` row records this. The interaction is clean *today* precisely because the only caller is a human-initiated command — an invariant that is nowhere written down and that a future automated caller (a cron, a fleet-orchestration script, an auto-update mode that shells to the CLI) would silently break. Register **`PM-188-01`** with trigger = "operator runs `doli upgrade`, per discovered unit"; action = "`systemctl reset-failed <unit>` (status ignored) then `systemctl restart <unit>`"; scale assumptions = "no thresholds; bounded by human invocation frequency"; interacts-with = "systemd StartLimitBurst (external), downstream of PM-003 / PM-012". Non-blocking, but it converts an unwritten invariant into a checked one.

---

## Findings

### F1 — MAJOR — `doli upgrade` reports success regardless of restart outcome

- **Location:** `bins/cli/src/cmd_upgrade.rs:253-263`; sinks at `bins/cli/src/upgrade_restart.rs:76` and `:91`
- **Severity:** Major · **Confidence:** `conf(0.90, observed)` · **Classification:** PRE-EXISTING (not a regression)
- **Evidence:** `restart_specific_service(svc);` (`:255`) and `restart_doli_service(installed_node_path.as_deref());` (`:257`) are both statements — the functions are declared `pub(crate) fn ... ()` at `upgrade_restart.rs:76` and `:91` and return unit. `run_systemd_plan`'s `bool` and `try_restart_systemd`'s `any_ok` are consumed only by `println!` and then dropped. Control flow then reaches `:261` `println!("Upgrade to v{} complete!", release.version);` and `:263` `Ok(())` unconditionally. `restart_doli_service`'s worst case prints "No doli-node service or process found. Restart manually if needed." (`upgrade_restart.rs:115`) and still exits 0.
- **Impact:** This is the *literal incident title* — "`sudo doli upgrade` completed while the node's systemd unit stayed DOWN". M1 makes the restart *succeed* in the start-limit case, so the specific reported failure is closed. But for **any other** restart failure (polkit denial, missing `sudo`, an ExecStart that is genuinely broken, a unit list that matched nothing) the operator still sees "complete!" and a zero exit code, and any automation wrapping `doli upgrade` still reads success. Independently identified by the logic auditor as SEC-LOGIC-005 / `[F2] P1`; I concur with the substance.
- **Suggested fix (M2):** return `bool`/`Result` from both restart entry points; in `cmd_upgrade`, print "Upgrade to vX complete!" only when the restart succeeded, and otherwise print an explicit "binary installed, service NOT restarted" line and return a non-zero exit. Keep the binary install itself successful — the two outcomes must be separable so a wrapper can tell "installed but down" from "installed and up".
- **Test strategy:** extend QA's forced-cfg harness — stub `sudo` to fail both steps, assert the process exit code is non-zero and that stdout does **not** contain "complete!".
- **Blocks M1?** **No.** Out of REQ-188-001/002 scope, pre-existing, and M1 strictly improves the situation. Must be the headline item of M2.

### F2 — MINOR — `is-active` pre-filter: adjudicated PRE-EXISTING / P2, not P1-INTRODUCED

- **Location:** `bins/cli/src/upgrade_restart.rs:160-175`; branch selector at `:143`; producer at `bins/cli/src/cmd_upgrade.rs:211,230,257`
- **Severity:** Minor (P2) · **Confidence:** `conf(0.85, observed)` · **Classification:** PRE-EXISTING (commit `b5f68bba`, 2026-08-10)
- **Evidence:** `git log -1 -L 160,175:bins/cli/src/upgrade_restart.rs` → `b5f68bba 2026-08-10 refactor(updater): INC-I-172 M1+M2 fail-closed maintainer trust root`, file creation. `git diff HEAD -- bins/cli/src/upgrade_restart.rs` shows the only hunk inside `try_restart_systemd` is `@@ -168,18 +181,14 @@` (the restart loop at `:181-193`); `:160-175` is untouched. Reachability chain verified at `cmd_upgrade.rs:212-230` (`installed_node_path = Some(path)` only after a successful `install_binary`; failure paths `return Err` at `:222-228`) → `:257` → `upgrade_restart.rs:143` selects the ExecStart-match arm.
- **Impact:** On the `installed_path == None` branch only, down/failed units are filtered out before the plan runs — but that branch is only reached when **no node binary was installed**, and it emits an explicit skip message plus a `--doli-node-path` hint (`cmd_upgrade.rs:232-233`). Not reachable in the INC-I-188 scenario.
- **Suggested fix (M2):** widen the predicate from `is-active` to `is-active OR is-failed` (see §4.4). Do not delete the filter — it correctly protects deliberately-stopped units on multi-node hosts.
- **Test strategy:** in the forced-cfg harness, stub `systemctl is-active --quiet` to exit 3 (inactive) and `is-failed` to exit 0 for one unit; assert the unit is still selected and receives `reset-failed`+`restart`.
- **Blocks M1?** No.

### F3 — MINOR — operator failure hint contradicts the code's own best-effort semantics and is near-redundant

- **Location:** `bins/cli/src/upgrade_restart.rs:82-83` and `:189-190`
- **Severity:** Minor · **Confidence:** `conf(0.85, observed)`
- **Evidence:** Hint text: `"  Failed to restart {}. Run: sudo systemctl reset-failed {} && sudo systemctl restart {}"`. Contrast `run_systemd_plan` at `:15-17`, which evaluates `if step.required` and therefore *ignores* the non-required step's status by construction; that behaviour is pinned by `test_systemd_restart_plan_marks_reset_failed_best_effort_and_restart_required` (test file `:112-125`). QA reproduced the exact printed string with both steps stubbed to exit 1 (QA exploratory row 2, severity LOW `[F1]`). Redundancy: the branch is reachable only after `run_systemd_plan` returned `false`, i.e. after both commands already ran.
- **Impact:** Cosmetic, no behavioural coupling. Divergence between implemented and advertised semantics on the one point M1 exists to establish; plus advice that repeats an action that just failed.
- **Suggested fix:** replace with the `;`-separated retry **plus** a `systemctl status` / `journalctl -u ... -n 50 --no-pager` diagnostic pointer, and state that both were already attempted (full text in §3.2). Two string literals; also narrows the injection auditor's P3 SEC-INJECTION-001 surface by removing the shell operator from copy-pasteable root text.
- **Test strategy:** `NOT_TESTABLE` as a behavioural assertion (operator-facing text, no code path depends on it). A string assertion in the forced-cfg harness would pin the wording, which is not worth the brittleness.
- **Blocks M1?** No. Cheap enough to fold in before commit; equally fine in M2.

### F4 — MINOR — the diff clears an external circuit breaker with no `protection_mechanisms` record

- **Location:** `bins/cli/src/upgrade_systemd_plan.rs:14-33`; executor `bins/cli/src/upgrade_restart.rs:11-20`
- **Severity:** Minor · **Confidence:** `conf(0.80, observed)`
- **Evidence:** `systemd_restart_plan` emits `["systemctl","reset-failed",unit]` unconditionally before every restart. `SELECT mechanism_id, name FROM v_protection_surface;` returns 40 rows, none covering the CLI systemd path. Loop-freedom established by `blast.py --hops 2` (single dependent, `cmd_upgrade.rs`) plus `crates/updater/src/apply.rs:599` `pub fn restart_node() -> !` (in-process re-exec, no `systemctl`), so no automated caller exists.
- **Impact:** None today — the interaction analysis in §7 is clean. The risk is future: "only a human can clear this breaker" is a load-bearing, unwritten invariant. A future cron/orchestration/auto-update caller of this path would create an unbounded breaker-clearing loop against a crash-looping binary, and nothing would flag it.
- **Suggested fix:** insert `PM-188-01` into `protection_mechanisms` with the trigger/action/scale/interacts-with fields given in §7. DB row only, no code change.
- **Test strategy:** `NOT_TESTABLE` (registry/traceability record, not behaviour).
- **Blocks M1?** No.

### F5 — MINOR — docs: stale milestone status row, and a missing troubleshooting entry

- **Location:** `docs/bugfixes/inc-i-188-analysis.md:71`; `docs/troubleshooting.md` (absent entry); cross-ref `docs/postmortems/2026-04-19-orphan-fork-rollback-cascade.md:156`
- **Severity:** Minor · **Confidence:** `conf(0.75, inferred)`
- **Evidence:** `docs/bugfixes/inc-i-188-analysis.md:71` → `| M1 | reset-failed precedes restart in CLI upgrade restart path | bins/cli/src/upgrade_restart.rs | REQ-188-001, REQ-188-002 | PENDING |`. `grep -n "start-limit\|start limit\|reset-failed\|203/EXEC\|StartLimit" docs/troubleshooting.md specs/*.md` → **zero matches**, while the postmortem at `:156` already prescribes `systemctl reset-failed` for `StartLimitBurst`.
- **Impact:** The analysis doc misreports milestone state to any later reader. The troubleshooting doc omits a failure mode that has now caused a live incident and that has an established remedy elsewhere in the corpus — an operator searching `troubleshooting.md` for "203/EXEC" finds nothing.
- **Suggested fix:** flip the M1 row to `DONE` (or `IMPLEMENTED (uncommitted)`) and append the M2 scope (F1, F2 §4.4, F3, F6); add a short `troubleshooting.md` entry (symptom / cause / remedy / "now automatic in `doli upgrade`"). `docs/cli.md` and `specs/` need **no** change — see §5.
- **Test strategy:** `NOT_TESTABLE` (documentation).
- **Blocks M1?** No, but the analysis-doc row is **required before incident close-out**.

### F6 — MINOR — sibling restart sites share the failure class; the drift guard is file-scoped

- **Location:** `bins/cli/src/cmd_service.rs:677-692`; `bins/cli/src/cmd_snap.rs:509-520`; `bins/cli/src/cmd_chain.rs:269-277`; guard scope at `bins/cli/tests/it/inc_i_188_upgrade_reset_failed_test.rs:190,204-239`
- **Severity:** Minor · **Confidence:** `conf(0.80, observed)`
- **Evidence:** `grep -rn '"restart"' bins/cli/src/` → `cmd_snap.rs:516`, `cmd_service.rs:682`, `cmd_service.rs:688`, plus the plan module. `cmd_snap.rs:514-518` is `let _ = Command::new("sudo").args(["systemctl","restart",service]).output(); println!("  Restarted {}", service);` — status discarded, success printed unconditionally. `cmd_chain.rs:269-277` issues `systemctl start` (with a `sudo` retry), **not** `restart`. Guard scope: `upgrade_restart_source_normalised()` (test file `:190`) reads only `src/upgrade_restart.rs`.
- **Impact:** M1's single-source property does not extend past one file, so a new unguarded restart elsewhere is undetected. `cmd_snap.rs` additionally reproduces F1's false-success shape in a stronger form. Corrects two imprecisions in QA F4 (`cmd_chain` is `start`; `cmd_snap` is worse than "missing reset-failed").
- **Suggested fix (M2, in this order):** (1) `cmd_service.rs` `cmd_restart` — route through `doli_cli::upgrade_systemd_plan`; (2) `cmd_snap.rs:514-518` — route through the plan **and** stop printing success on a discarded status; (3) `cmd_chain.rs` — lower priority, `start` after `reset-failed`. Then widen the structural test to scan `src/**`, or replace it with an injectable-runner `#[cfg(target_os = "linux")]` unit test (QA's forced-cfg harness proves this is feasible).
- **Test strategy:** for each site, a forced-cfg harness run with a stubbed `systemctl` asserting `reset-failed` precedes the start/restart verb; for `cmd_snap`, assert no "Restarted" line when the stub exits non-zero.
- **Blocks M1?** **No — deferring is the correct call.** Rationale in §6.3.

---

## Speculative Findings (low-confidence, not actionable)

### S1 — structural source-text test is brittle against a legitimate M2 refactor

- **Location:** `bins/cli/tests/it/inc_i_188_upgrade_reset_failed_test.rs:223-239`
- **Confidence:** `conf(0.65, inferred)` — below the actionable floor; reported only so M2 is not surprised.
- **Reasoning:** the test requires ≥ 2 occurrences of `systemd_restart_plan(` in `src/upgrade_restart.rs`. If M2 consolidates both call sites behind a single shared helper (the natural refactor when extending the plan to `cmd_service`/`cmd_snap`), occurrences drop to 1 and the test fails even though the behaviour is strictly better. No change now — the test is currently the only cross-call-site guard available on a macOS build host and it is earning its keep. Flagged so that a future red is read as "the guard needs rehoming", not "the fix regressed".

---

## Verdict

**FINAL — APPROVED for commit.**

M1 is a root-cause fix, not a superficial patch. `reset-failed` is the only systemd operation that clears start-limit lock, the lock is transient state over an already-valid binary, and the ordering is single-sourced in a pure function that three tests pin positionally. Both requirements are met (QA independently verified REQ-188-001 and REQ-188-002 PASS, gates green, RED-state confirmed pre-fix). No unintended behaviour change: the launchd tier, the process tier, `find_doli_node_path`, and `restart_doli_service`'s tier selection carry zero diff hunks, corroborated by diffstat arithmetic and by a code-graph blast radius of exactly one dependent. The one new behaviour — `reset-failed` on the already-up-to-date `--service` path — is an improvement, not a regression.

Both developer deviations are ruled **correct**: (a) reformatting the test file to clear `cargo fmt --check` was right, and the whitespace-stripped-hash verification is a complete proof of semantic identity for this file; (b) the `&&` hint is genuinely wrong and should become `;` plus a diagnostic pointer, but it is operator text with no behavioural coupling and does not gate the milestone.

The auditor contradiction is adjudicated against **both** auditors' classification: the `is-active` filter is **PRE-EXISTING** (`b5f68bba`, 2026-08-10, zero hunks in range), not INTRODUCED, and it is **P2/Minor**, not P1 — unreachable on the incident path because `installed_node_path` is `Some` whenever a binary was swapped.

Six findings, none blocking. The residual that matters is **F1**: `doli upgrade` still exits 0 and prints "complete!" whatever the restart did. M1 closes the reported failure; F1 is the remaining half of the incident title and should headline M2.

**Recommended before commit (all optional, none blocking):** F3 (two string literals), F4 (one DB row), F5 (analysis-doc status row — required before incident close-out).

**Architecture escalation:** none. No `[ARCHITECTURE]` finding. The design is sound and the choice to put the plan in the `doli_cli` lib makes the M2 extension cheap.

---

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      +constant per unit per upgrade (observed) — one extra fork/exec of `sudo systemctl reset-failed` at upgrade_restart.rs:14, plus 2 SystemdStep + 6 String constructions in systemd_restart_plan. My proposed fixes (F3 string literals, F4 DB row, F5 docs) add 0.
  Memory:   +constant, short-lived (observed) — 6 heap Strings + a 2-element Vec per unit, dropped at the end of run_systemd_plan; no long-lived buffer, no cache, no static.
  IO:       +1 process spawn and its syscalls per unit per upgrade (observed) — doubles the systemctl invocations in this path from 1 to 2 per unit; QA measured exactly 2 calls per unit in the forced-cfg harness.
  Network:  N-A (observed) — the restart path performs no network I/O; `doli upgrade`'s GitHub fetch is upstream at cmd_upgrade.rs:76 and is byte-for-byte unchanged.
  Disk:     +1 journal record per unit per upgrade (observed) — systemd logs the reset-failed action; bytes are negligible and PM-020 (installer logrotate drop-in) already bounds log growth on these hosts.
  Latency:  +one sub-second systemctl round trip per unit (observed) — on `doli upgrade`, a human-initiated command with no SLO that has already spent seconds downloading and verifying a release tarball.
Inevitability: AVOIDABLE
Cheaper alternative: issue `reset-failed` only after a `restart` has already been observed to fail — one spawn in the common (healthy) case instead of two.
Why this proposal anyway: the conditional form re-creates two divergent execution paths, which is the exact drift REQ-188-001 exists to eliminate, and it buys back one sub-second spawn on a manually-invoked command; `systemctl reset-failed` is a documented no-op on a loaded healthy unit, so the unconditional form is correct and strictly simpler.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

━━━ SECURITY AUDIT VERDICT ━━━
Verdict: AUDIT-REQUIRED
Signals: enforcement & deploy surface — the diff is inside the `doli upgrade` installer/restart path, which runs as root and is the mechanism by which every host receives new binaries; external data — unit names sourced from `systemctl list-unit-files` output (upgrade_restart.rs:131-136) and from the operator-supplied `--service` flag are threaded into root-privileged `sudo systemctl` argv; state integrity — the change clears systemd's StartLimitBurst circuit breaker, altering host-level failure containment. Recorded as confirmation only: the 5-auditor sweep has already run (injection, auth, crypto, logic on disk) and no finding is discarded by this verdict either way.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
