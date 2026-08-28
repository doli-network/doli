# INC-I-188 — `doli upgrade` restart path cannot clear a systemd start-limit lock

**Run**: 530 · **Branch**: main · **Triage**: FAST · **Date**: 2026-08-28

## Bug Statement

`sudo doli upgrade` completed a binary install on vm-server (6.24.1 → 6.25.0) and printed
"Upgrade to v6.25.0 complete!" while `doli-mainnet.service` stayed down. The unit was in
systemd **start-limit lock** (`start-limit-hit`): 4 rapid `status=203/EXEC` failures during
the binary-swap window exhausted `StartLimitBurst`. From that state, plain
`systemctl restart` / `systemctl start` are refused with "Start request repeated too
quickly" — only `systemctl reset-failed <unit>` clears the lock. The upgrade's post-install
restart path never issues `reset-failed`, so its restart failed, and the fallback command it
printed to the operator (`sudo systemctl restart doli-mainnet`) failed identically.
Manual recovery: `sudo systemctl reset-failed doli-mainnet && sudo systemctl start doli-mainnet`.

## Live Evidence (captured 2026-08-27/28, vm-server)

- `systemctl status doli-mainnet`: `Active: failed (Result: exit-code)`, `Main PID: 1779 (code=exited, status=203/EXEC)`, `Scheduled restart job, restart counter is at 4`, then repeated `Start request repeated too quickly.`
- Journal 23:50:59: two consecutive refused start requests — matching `restart_doli_service`'s attempt plus the operator retrying the printed fallback command.
- Recovery verified: after `reset-failed` + `start`, unit active (pid 2524), node snap-synced to network tip h=313289.
- Contributing race (separate, NOT in scope here): the privileged install fallback is `sudo rm -f <target>` then `sudo cp` (`crates/updater/src/apply.rs:229-244`, shape forced by the deployed sudoers whitelist). Between rm and cp the binary does not exist; `Restart=` firing in that window produces exactly the observed 203/EXEC burst. Fixing that window needs an installer/sudoers rollout decision — deferred, recorded in the incident.

## Architecture Context

- `bins/cli/src/cmd_upgrade.rs` (`cmd_upgrade`) verifies + installs binaries, then hands off restart:
  - `cmd_upgrade.rs:255` → `restart_specific_service(svc)` (operator passed `--service`)
  - `cmd_upgrade.rs:257` → `restart_doli_service(installed_node_path)` (auto-detect)
  - `cmd_upgrade.rs:88` → `restart_specific_service(svc)` (already-up-to-date + `--service` path)
- `bins/cli/src/upgrade_restart.rs` implements the restart tier logic. The two systemd-touching sites:
  - `restart_specific_service` (`upgrade_restart.rs:61-73`): `sudo systemctl restart <service>`
  - `try_restart_systemd` (`upgrade_restart.rs:169-184`): per detected unit, `sudo systemctl restart <unit>`
- Non-systemd tiers (`try_restart_launchd` macOS, `try_restart_process` pgrep/kill) have no
  start-limit concept — out of scope.
- Blast radius (verified by direct read of both files, all call sites enumerated above): the
  change is contained to `bins/cli/src/upgrade_restart.rs`; its only production consumer is
  `cmd_upgrade.rs`. No node-side code imports this module (`bins/cli` is a binary crate; the
  node's auto-updater restart goes through process exit + systemd `Restart=`, not this file).
- Consensus impact: none. CLI-only operational tooling. No activation height, no block
  content, no synchronized deploy. Rolling deploy safe.

## Root Cause

`restart_specific_service` and `try_restart_systemd` assume `systemctl restart` can always
start a unit. That assumption is false for a unit in start-limit lock — the state a failed
upgrade window reliably produces. The restart path therefore cannot recover from the very
failure mode the upgrade itself can create. Detection-without-action: the code prints a
fallback command that fails for the same reason.

## Fix Requirements

| ID | Priority | Requirement | Acceptance Criteria |
|----|----------|-------------|---------------------|
| REQ-188-001 | Must | Every systemd restart issued by the upgrade restart path MUST be preceded by `systemctl reset-failed <unit>` for the same unit, so a start-limit-locked unit is recoverable. | Unit test proves the systemd command plan for a unit is exactly `reset-failed <unit>` then `restart <unit>`, in that order; both call sites (`restart_specific_service`, `try_restart_systemd`) consume the same plan. Test FAILS on current code, PASSES after fix. |
| REQ-188-002 | Should | A failing `reset-failed` (unit not failed, unit unknown) MUST NOT abort the restart attempt — it is best-effort preparation. | The restart command is executed regardless of the `reset-failed` exit status; verified by unit test on the plan-execution contract or by reviewer inspection of the call sequence. |

Testable seam: the command sequence must be derivable by a pure function (command plan)
so ordering is assertable without systemd on the build host (macOS/CI).

### Traceability — Implementation Modules (M1)

| Requirement | Test | Implementation Module |
|---|---|---|
| REQ-188-001 | `test_systemd_restart_plan_orders_reset_failed_before_restart`, `test_systemd_restart_plan_threads_a_different_unit_verbatim`, `test_systemd_restart_plan_threads_adversarial_unit_verbatim`, `test_upgrade_restart_source_has_no_raw_restart_argv_literal`, `test_upgrade_restart_source_routes_both_call_sites_through_plan` | `systemd_restart_plan` @ `bins/cli/src/upgrade_systemd_plan.rs`; `restart_specific_service` + `try_restart_systemd` @ `bins/cli/src/upgrade_restart.rs` |
| REQ-188-002 | `test_systemd_restart_plan_marks_reset_failed_best_effort_and_restart_required` | `SystemdStep::required` @ `bins/cli/src/upgrade_systemd_plan.rs`; `run_systemd_plan` @ `bins/cli/src/upgrade_restart.rs` |

## Milestones

| ID | Name | Scope (Modules) | Scope (Requirements) | Status |
|----|------|-----------------|----------------------|--------|
| M1 | reset-failed precedes restart in CLI upgrade restart path | bins/cli/src/upgrade_restart.rs | REQ-188-001, REQ-188-002 | PENDING |

## Specs/Docs Drift

None found: `docs/cli.md` describes `doli upgrade` behavior generically; no doc claims the
restart path recovers locked units. Post-fix, `/sync-docs bins/cli` should confirm.

━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.9, measured — failure mode observed live on vm-server this session; manual reset-failed+start recovered the unit; both code call sites read end-to-end)
Reasoning: Deterministic, localized to one module with one production consumer; root cause identified and remedy verified operationally on the affected host.
━━━━━━━━━━━━━━━━━━━━━━
