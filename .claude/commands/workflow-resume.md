---
name: workflow:resume
description: Resume a stopped or failed workflow from the last completed step. Accepts optional --from to specify a milestone or step.
---

# Workflow: Resume

The user wants to resume a workflow that was interrupted — by context limits, agent errors, retry exhaustion, or manual stop. This command reads saved state and picks up where the workflow left off.

Optional: `--from="M3"` or `--from="test-writer"` to override the auto-detected resume point.

## When to Use

This command resumes **milestone-based workflows only** (`/workflow:new`, `/workflow:new-feature`). If you need to resume a non-milestone workflow (`/workflow:bugfix`, `/workflow:improve-functionality`), re-invoke the failed step's agent directly instead.

## Prerequisite Gate

### Required Files
- `docs/.workflow/milestone-progress.md` MUST exist — if not, STOP: "No milestone progress file found. There is no workflow to resume. Use `/workflow:new` or `/workflow:new-feature` to start a workflow."
- `specs/*-architecture.md` MUST exist — needed to understand milestone scopes and module definitions
- `specs/*-requirements.md` MUST exist — needed for agent context (requirement IDs, acceptance criteria, traceability)

### Optional Files
- `docs/.workflow/chain-state.md` — if it exists, read it to understand what step failed within the milestone and why. If it doesn't exist (user stopped manually), infer the resume point from milestone progress alone.

## Parameters

- `--from="M[N]"` — resume from a specific milestone (skip completed ones before it). Starts from the first step (test-writer) of that milestone.
- `--from="test-writer"` / `--from="developer"` / `--from="qa"` / `--from="reviewer"` — resume from a specific step within the next pending milestone.
- **No parameter** — auto-detect: find the first non-COMPLETE milestone and the first pending step.

## Fail-Safe Controls

### Iteration Limits (per milestone)
- **QA ↔ Developer iterations:** Maximum **3 iterations** per milestone. If QA still finds blocking issues after 3 rounds, STOP and report to user: "QA iteration limit reached (3/3) for milestone M[N]. Remaining issues: [list]. Requires human decision on how to proceed."
- **Reviewer ↔ Developer iterations:** Maximum **2 iterations** per milestone. If the reviewer still finds critical issues after 2 rounds, STOP and report to user: "Review iteration limit reached (2/2) for milestone M[N]. Remaining issues: [list]. Requires human decision."

### Inter-Step Output Validation
Before invoking each agent, verify the previous agent produced its expected output:
- Before Test Writer (each milestone): verify `specs/*-architecture.md` contains the milestone's modules
- Before Developer (each milestone): verify test files exist for the milestone's modules
- Before Compilation Validation (each milestone): verify source code files exist
- Before QA (each milestone): verify compilation & lint validation passed (build + lint + tests clean)
- Before Reviewer (each milestone): verify QA report exists in `docs/qa/`

**If any expected output is missing, STOP the chain** and report: "CHAIN HALTED at [step] (Milestone M[X]): Expected output from [agent] not found. [What's missing]. Previous agent may have failed silently."

### Error Recovery
If any agent fails mid-chain during resume:
1. Save the chain state to `docs/.workflow/chain-state.md` with:
   - Which steps completed successfully (and their output files)
   - Which step failed and why
   - What remains to be done
2. Report to user with the chain state
3. The user can resume again with `/workflow:resume` or `/workflow:resume --from="[step]"`

## Step 1: Read Milestone Progress

1. Read `docs/.workflow/milestone-progress.md` and parse the milestone table
2. Identify the first milestone with status `PENDING` or `IN_PROGRESS`
3. If ALL milestones are `COMPLETE`, report: "All milestones are already complete. Nothing to resume." and STOP

## Step 2: Read Chain State (if exists)

1. If `docs/.workflow/chain-state.md` exists:
   - Read it to determine which step failed within the current milestone
   - Note the failure reason for context
   - Delete the chain-state file after reading — a new one will be created if the resume also fails
2. If it does not exist:
   - The workflow was likely stopped manually
   - The resume point will be inferred from existing outputs

## Step 3: Validate Completed Milestones

For every milestone marked `COMPLETE` in the progress file:
1. Verify source code files exist for that milestone's modules
2. Verify test files exist for that milestone's modules
3. If any completed milestone is missing outputs, WARN the user but do not block — previous milestones may have been committed and are in version control

## Step 4: Determine Resume Point

Apply the following logic in priority order:

### If `--from="M[N]"` is provided:
- Start from that milestone's first step (test-writer)
- Mark any `IN_PROGRESS` milestones before M[N] as needing re-evaluation
- Update milestone progress to show M[N] as `IN_PROGRESS`

### If `--from="[agent-name]"` is provided:
- Use the first `PENDING` or `IN_PROGRESS` milestone
- Start from the specified agent step within that milestone
- Valid agent names: `test-writer`, `developer`, `qa`, `reviewer`

### If chain-state exists (no --from):
- Use the milestone and step identified in the chain state
- Resume from the failed step

### Default (no --from, no chain-state):
- Find the first `PENDING` or `IN_PROGRESS` milestone
- Check what outputs already exist for that milestone to determine the furthest completed step:
  - Test files exist for the milestone's modules → skip test-writer, start at developer
  - Source code exists and tests pass → skip developer, start at QA
  - QA report exists → skip QA, start at reviewer
  - None of the above → start from test-writer
- Update milestone progress to show this milestone as `IN_PROGRESS`

## Step 5: Report Resume Plan

Before executing, report the resume plan to the user:
```
RESUMING WORKFLOW
  Current milestone: M[N] — [name]
  Resume from: [step name]
  Remaining milestones: [count]
  Reason: [auto-detected / user specified / chain-state recovery]
```

## Steps 6-11: Milestone Loop

**For EACH milestone starting from the resume point**, execute the following steps. After completing all steps for a milestone, **auto-continue to the next milestone without user intervention**.

### Step 6: Test Writer (scoped to current milestone)
Invoke the `test-writer` subagent passing the architecture, **scoped to the current milestone's modules and requirements only**.
1. Tests Must requirements first (exhaustive coverage + edge cases)
2. Tests Should requirements second (solid coverage)
3. Tests Could requirements last (basic happy path)
4. Every test references a requirement ID for traceability
5. Tests cover failure modes and security from the architect's design
6. Updates the traceability matrix with test IDs
7. All new tests must fail initially (red phase)
8. All previous tests (from completed milestones) must continue passing

**Skip this step if resuming from a later step within this milestone.**

### Step 7: Developer (scoped to current milestone)
Invoke the `developer` subagent, **scoped to the current milestone's modules only**.
The developer works one module at a time: read tests → implement → run tests → commit → next.
Must implement module by module until all tests pass.
If context gets heavy mid-implementation, commit progress and continue.

**Skip this step if resuming from QA or reviewer within this milestone.**

### Step 7.5: Compilation & Lint Validation
**Mandatory gate before QA.** The developer MUST run a full compilation and lint validation pass after implementing all modules for the current milestone:

**Rust projects** (detected via `Cargo.toml`):
1. `cargo build` — fix any compilation errors
2. `cargo clippy -- -D warnings` — fix all lint warnings
3. `cargo test` — run full test suite, ensure ALL tests pass (including previous milestones)

**Elixir projects** (detected via `mix.exs`):
1. `mix compile --warnings-as-errors` — fix compilation warnings
2. `mix dialyzer` (if configured) — fix type issues
3. `mix test` — run full test suite

**Node.js/TypeScript projects** (detected via `package.json` + `tsconfig.json`):
1. `npx tsc --noEmit` (TypeScript) or build step — fix type/compilation errors
2. `npx eslint .` (if configured) — fix lint issues
3. `npm test` or `npx jest` — run the full test suite

**General pattern** (adapt to detected language):
1. Build/compile step
2. Lint/static analysis step
3. Full test suite

If any step fails, fix and re-run. This is subject to the developer's **max 5 retry limit**. If all 3 steps pass clean, proceed to QA. If retries are exhausted, STOP and escalate.

**Skip this step if resuming from QA or reviewer within this milestone.**

### Step 8: QA (scoped to current milestone)
Invoke the `qa` subagent, **scoped to the current milestone**.
1. Verify the traceability matrix is complete for this milestone's requirements
2. Verify acceptance criteria for every Must requirement in this milestone
3. Verify acceptance criteria for every Should requirement in this milestone
4. Run end-to-end flows that cross module boundaries within this milestone
5. Perform exploratory testing — try what no test anticipated
6. Validate failure modes and recovery strategies
7. Validate security considerations
8. Generate QA report at `docs/qa/[name]-M[N]-qa-report.md`

**Skip this step if resuming from reviewer within this milestone.**

### Step 9: QA Iteration
If QA finds blocking issues (Must requirements failing, broken flows):
- Return to the developer with the QA findings
- The developer fixes them (scoped to the affected area only)
- QA re-validates (scoped to the fix only)
- Repeat until QA approves

**Skip this step if resuming from reviewer within this milestone.**

### Step 10: Reviewer (scoped to current milestone)
Once QA approves, invoke the `reviewer` subagent, **scoped to the current milestone**.
The reviewer works module by module, saving findings incrementally.
Wait for the review report, including specs/docs drift check.
Save output to `docs/reviews/[name]-M[N]-review.md`.

### Step 11: Review Iteration
If the reviewer finds critical issues:
- Return to the developer with the findings
- The developer fixes them (scoped to the affected module only)
- The reviewer reviews again (scoped to the fix only)
- Repeat until approved

### Step 11.5: Milestone Commit & Push
After the reviewer approves the current milestone:
1. `git add` all files relevant to this milestone (source code, tests, specs, docs, QA reports, review reports)
2. `git commit` with conventional message: `feat: complete [milestone name] (M[N])`
3. `git push` to the remote
4. Update `docs/.workflow/milestone-progress.md` — mark this milestone as `COMPLETE` with timestamp

**Then AUTO-CONTINUE to the next milestone.** No user intervention needed between milestones.

### Milestone Loop Termination
The loop ends when ALL milestones are marked `COMPLETE` in the progress file. If any milestone fails (agent error, retry limit exceeded), save the chain state and report which milestones completed and which remain.

## Step 12: Final Versioning

Once ALL milestones are complete:
1. Run the full test suite one final time to verify cross-milestone integration
2. Create the final version tag
3. Final `git push --tags`
4. Clean up `docs/.workflow/` temporary files (but keep `milestone-progress.md` as a record)
