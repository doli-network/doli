---
name: workflow:audit
description: "Audit existing code. Read-only by default; with --fix, auto-fixes findings by priority. Accepts --scope, --fix, --include-p3."
---

# Workflow: Audit

Audit existing code for security, performance, technical debt, and specs/docs drift.

**Two modes:**
- **Without `--fix`** (default): Read-only audit. Reviewer scans and produces a report. No code changes.
- **With `--fix`**: Auto-fix pipeline. Reviewer produces a prioritized report, then the pipeline loops through each priority level — writing regression tests, fixing findings, validating, and committing per priority pass.

**Parameters:**
- `--scope="area"` — limit audit to a specific milestone, module, or file path
- `--fix` — enable the auto-fix pipeline after the audit
- `--include-p3` — include P3 (suggestions) in auto-fix (skipped by default)

---

## Mode 1: Read-Only Audit (no --fix)

Invoke ONLY the `reviewer` subagent in full audit mode.
Optional: `--scope="milestone or module"` to audit a specific area.

### Prerequisite Fallback
If `specs/SPECS.md` does not exist:
- **Don't fail.** Proceed with a code-only audit.
- Note in the audit report: "No specs/SPECS.md found. Audit is based on codebase analysis only. Specs drift checks were skipped."
- The reviewer should still check for the existence of docs and report their absence as a finding.

### Without scope (full audit)
The reviewer MUST work in chunks to avoid context limits:
1. Read `specs/SPECS.md` to get the list of milestones/domains (if it exists; if not, derive structure from directory layout)
2. For each milestone:
   a. Review the code for that milestone
   b. Review corresponding specs and docs
   c. Save findings to `docs/.workflow/audit-[milestone].md`
   d. Clear mental context before next milestone
3. Compile all milestone findings into the final report
4. Save to `docs/audits/audit-[date].md`
5. Clean up `docs/.workflow/audit-*.md` temporary files

### With scope (targeted audit)
The reviewer works only within the specified area:
1. Read only the scoped code, specs, and docs
2. Generate the audit report for that area
3. Save to `docs/audits/audit-[scope]-[date].md`

### Audit Covers
- Security vulnerabilities
- Performance issues
- Technical debt
- Dead code
- Missing tests
- Suggested improvements
- Specs/docs drift (specs that don't match code, missing specs, orphaned docs)

---

## Mode 2: Auto-Fix Pipeline (--fix)

When `--fix` is present, the audit becomes an active remediation pipeline. The reviewer produces a structured, prioritized report, then the pipeline automatically loops through each priority level.

```
Step 1:   Reviewer           → structured audit with P0/P1/P2/P3 findings (AUDIT-PX-NNN IDs)
Step 1.5: Progress Tracker   → creates docs/.workflow/audit-fix-progress.md

  ┌─── FOR EACH PRIORITY LEVEL (P0 → P1 → P2 → [P3]) ───────────────────┐
  │ Step 2:   Test Writer    → regression tests for this priority's       │
  │                            findings (proves issues exist)             │
  │ Step 3:   Developer      → fixes findings scoped to this priority    │
  │ Step 3.5: Build & Lint   → full compilation + lint + test suite      │
  │ Step 4:   Verification   → regression tests pass + no regressions    │
  │ Step 4.5: Commit & Push  → git commit + push per priority pass       │
  └───────────────────────────────────────────────────────────────────────┘

Step 5:   Final Summary      → total fixes, escalated findings, cleanup
```

### Fail-Safe Controls

#### Iteration Limits
- **Developer fix attempts:** max **5** per individual finding. After 5 failed attempts, mark finding as `ESCALATED` and move to the next.
- **Build/lint retries:** max **3** per priority pass. If build/lint still fails after 3 rounds of fixes, STOP the priority pass and escalate all remaining findings in that priority.
- **Verification iterations:** max **2** per priority pass. If regression tests or existing tests still fail after 2 verification rounds, escalate remaining findings.

#### Priority Treatment
- **P0 (Critical):** All findings MUST be attempted. Cannot skip.
- **P1 (Major):** All findings MUST be attempted. Cannot skip.
- **P2 (Minor):** All findings SHOULD be attempted. Individual findings may be escalated after retry limits.
- **P3 (Suggestions):** SKIPPED by default. Only processed when `--include-p3` is specified.

#### Error Recovery
If the pipeline fails mid-chain (agent error, unexpected state):
1. Save chain state to `docs/.workflow/chain-state.md` with:
   - Which priority levels completed
   - Which priority level failed and at which step
   - Which findings were fixed, which were escalated, which remain
2. Update `docs/.workflow/audit-fix-progress.md` with current status
3. Report to user with the chain state summary

### Step 1: Reviewer (Structured Audit)

Invoke the `reviewer` subagent in **structured audit mode** (inform it that `--fix` was specified).

**Critical instruction to the reviewer:** "This audit uses `--fix` mode. You MUST produce findings in the structured AUDIT-PX-NNN format with Location, Category, Description, Impact, Suggested Fix, and Test Strategy fields. Classify every finding into P0/P1/P2/P3. This format is required for the auto-fix pipeline to process your findings."

The reviewer follows the same scoping rules as Mode 1 (chunks for large codebases, targeted for scoped audits) but outputs in the structured format.

**Expected output:** `docs/audits/audit-[scope]-[date].md` (or `audit-[date].md` for full audit) with the structured format containing AUDIT-PX-NNN IDs.

**Validation:** After the reviewer completes, verify the audit report exists and contains at least one finding. If zero findings, skip the auto-fix pipeline and report: "Audit complete. No findings to fix."

### Step 1.5: Progress Tracker

After the reviewer completes, parse the audit report and create the progress tracking file:

1. Read the audit report and extract all AUDIT-PX-NNN findings
2. Count findings per priority level
3. Create `docs/.workflow/audit-fix-progress.md` with the following format:

```markdown
# Audit Fix Progress

## Summary
- **Audit report:** docs/audits/audit-[name].md
- **Total findings:** [count]
- **P0 (Critical):** [count]
- **P1 (Major):** [count]
- **P2 (Minor):** [count]
- **P3 (Suggestions):** [count] [SKIPPED / INCLUDED]

## Priority Pass Status

| Priority | Status | Findings | Fixed | Escalated | Commit |
|----------|--------|----------|-------|-----------|--------|
| P0 | PENDING | [count] | 0 | 0 | — |
| P1 | PENDING | [count] | 0 | 0 | — |
| P2 | PENDING | [count] | 0 | 0 | — |
| P3 | SKIPPED | [count] | 0 | 0 | — |

## Findings Detail

### P0: Critical
| ID | Title | Status | Test File | Fix Commit |
|----|-------|--------|-----------|------------|
| AUDIT-P0-001 | [title] | PENDING | — | — |

### P1: Major
(same table)

### P2: Minor
(same table)

### P3: Suggestions
(same table, all SKIPPED unless --include-p3)
```

4. If P3 is present and `--include-p3` was NOT specified, mark all P3 findings as `SKIPPED`
5. If there are no P0 findings, the P0 pass is auto-completed (Status: `COMPLETE`, 0 findings)
6. Same for any empty priority level

### Steps 2-4.5: Priority Loop

**For EACH priority level with findings** (P0 → P1 → P2 → P3 if included), execute the following steps. After completing all steps for a priority level, **auto-continue to the next priority level without user intervention**.

Skip priority levels with zero findings (mark as COMPLETE in progress file).

#### Step 2: Test Writer (scoped to current priority's findings)

Invoke the `test-writer` subagent with a **special prerequisite override**: instead of requiring an architect design + analyst requirements, the **audit report serves as the requirements source**.

**Instruction to test-writer:** "You are writing regression tests for audit findings, not for new requirements. Your requirements source is the audit report at `docs/audits/audit-[name].md`. For each finding in priority P[X], write a test that **proves the issue exists** (the test should fail or demonstrate the problematic behavior). Use the finding's Test Strategy field as guidance. Reference finding IDs (AUDIT-PX-NNN) in test names/comments for traceability."

Pass to the test-writer:
- The audit report (as requirements source)
- The current priority level (e.g., "P0 findings only")
- The scope (if `--scope` was specified)

**Expected output:** Test files that demonstrate the audit findings exist.

**Note:** Some findings (e.g., dead code, documentation gaps, style issues) may not be testable. The test-writer should note these as `NOT_TESTABLE` and the developer should still fix them based on the audit description alone.

#### Step 3: Developer (scoped to current priority's findings)

Invoke the `developer` subagent with a **special prerequisite override**: instead of requiring an architect design, the **audit report serves as the architecture reference**.

**Instruction to developer:** "You are fixing audit findings, not implementing new features. Your reference is the audit report at `docs/audits/audit-[name].md`. Fix each finding in priority P[X] using the Suggested Fix field as guidance. For testable findings, ensure the regression tests now pass. For non-testable findings (dead code removal, documentation fixes, style changes), fix based on the audit description. Work one finding at a time. Max 5 attempts per finding — if a finding resists fixing after 5 attempts, mark it as ESCALATED and move to the next."

Pass to the developer:
- The audit report (as architecture reference)
- The regression tests from Step 2
- The current priority level
- The scope (if `--scope` was specified)

**Tracking:** After the developer completes, update `docs/.workflow/audit-fix-progress.md`:
- Mark each fixed finding as `FIXED`
- Mark each escalated finding as `ESCALATED`
- Record which findings were addressed

#### Step 3.5: Build & Lint Validation

**Mandatory gate before verification.** Run full compilation, lint, and test suite for the project's detected language:

**Rust projects** (detected via `Cargo.toml`):
1. `cargo build` — fix any compilation errors
2. `cargo clippy -- -D warnings` — fix all lint warnings
3. `cargo test` — full test suite (including previous priority passes)

**Elixir projects** (detected via `mix.exs`):
1. `mix compile --warnings-as-errors` — fix compilation warnings
2. `mix test` — full test suite

**Node.js/TypeScript projects** (detected via `package.json` + `tsconfig.json`):
1. `npx tsc --noEmit` (TypeScript) or build step — fix type errors
2. `npx eslint .` (if configured) — fix lint issues
3. `npm test` or `npx jest` — full test suite

**General pattern** (adapt to detected language):
1. Build/compile step
2. Lint/static analysis step
3. Full test suite

If any step fails, return to the developer to fix. Max **3 build/lint retry rounds** per priority pass. If retries exhausted, STOP this priority pass and escalate remaining findings.

#### Step 4: Verification

After build & lint passes, verify:
1. **Regression tests pass** — all tests written in Step 2 for this priority's findings now pass (proving issues are fixed)
2. **No regressions** — all pre-existing tests still pass
3. **No new issues introduced** — quick scan for obvious problems in changed files

If verification fails:
- Return to developer with specific failures
- Max **2 verification iterations** per priority pass
- After 2 failed verifications, escalate remaining issues

#### Step 4.5: Commit & Push

After verification passes for this priority level:
1. `git add` all files changed during this priority pass (source code, tests, docs)
2. `git commit` with conventional message: `fix: resolve P[X] audit findings ([count] fixed, [count] escalated)`
3. `git push` to the remote
4. Update `docs/.workflow/audit-fix-progress.md`:
   - Mark this priority level as `COMPLETE`
   - Record the commit hash for each fixed finding
   - Update the Priority Pass Status table

**Then AUTO-CONTINUE to the next priority level.**

### Step 5: Final Summary

After all priority levels have been processed:

1. Read `docs/.workflow/audit-fix-progress.md` for final tallies
2. Produce a final summary to the user:

```
## Audit Fix Summary

**Total findings:** [count]
**Fixed:** [count]
**Escalated (need human review):** [count]
**Skipped (P3):** [count]

### Commits
- P0: [commit hash] — [count] fixed, [count] escalated
- P1: [commit hash] — [count] fixed, [count] escalated
- P2: [commit hash] — [count] fixed, [count] escalated

### Escalated Findings (require human review)
- AUDIT-P0-NNN: [title] — [reason for escalation]
- AUDIT-P1-NNN: [title] — [reason for escalation]

### Next Steps
[Recommendations for escalated findings]
```

3. Clean up temporary `docs/.workflow/` files EXCEPT:
   - Keep `docs/.workflow/audit-fix-progress.md` as a record
   - Keep `docs/.workflow/chain-state.md` if any findings were escalated
4. The audit report at `docs/audits/` is permanent — do not delete it
