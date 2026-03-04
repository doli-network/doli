---
name: workflow:new-feature
description: Add a feature to an existing project. Accepts optional --scope to limit context.
---

# Workflow: New Feature

The user wants to add functionality to existing code.
Optional: `--scope="area"` to limit which part of the codebase is analyzed.

## Existing Project Validation
Before starting the chain, verify this is an existing project:
1. Check for source code files in the project. If none exist, suggest `/workflow:new` instead.
2. If `specs/SPECS.md` does not exist, **don't fail** — proceed but note: "No specs/SPECS.md found. The analyst will work from code and user description only. Specs will be created as part of this workflow."

## Fail-Safe Controls

### Iteration Limits (per milestone)
- **QA ↔ Developer iterations (Steps 5-6):** Maximum **3 iterations** per milestone. If QA still finds blocking issues after 3 rounds, STOP and report to user: "QA iteration limit reached (3/3) for milestone M[N]. Remaining issues: [list]. Requires human decision on how to proceed."
- **Reviewer ↔ Developer iterations (Steps 7-8):** Maximum **2 iterations** per milestone. If the reviewer still finds critical issues after 2 rounds, STOP and report to user: "Review iteration limit reached (2/2) for milestone M[N]. Remaining issues: [list]. Requires human decision."

### Inter-Step Output Validation
Before invoking each agent, verify the previous agent produced its expected output:
- Before Feature Evaluator (Step 0.5): if discovery ran, verify `docs/.workflow/idea-brief.md` exists
- Before Analyst (Step 1): verify `docs/.workflow/feature-evaluation.md` exists with a GO verdict or user override
- Before Architect (Step 2): verify `specs/*-requirements.md` exists
- Before Milestone Extraction (Step 2.5): verify `specs/*-architecture.md` exists
- Before Test Writer (Step 3, each milestone): verify `specs/*-architecture.md` contains the milestone's modules
- Before Developer (Step 4, each milestone): verify test files exist for the milestone's modules
- Before Compilation Validation (Step 4.5, each milestone): verify source code files exist
- Before QA (Step 5, each milestone): verify compilation & lint validation passed (build + lint + tests clean)
- Before Reviewer (Step 7, each milestone): verify QA report exists in `docs/qa/`

**If any expected output is missing, STOP the chain** and report: "CHAIN HALTED at Step [N] (Milestone M[X]): Expected output from [agent] not found. [What's missing]. Previous agent may have failed silently."

### Error Recovery
If any agent fails mid-chain:
1. Save the chain state to `docs/.workflow/chain-state.md` with:
   - Which steps completed successfully (and their output files)
   - Which step failed and why
   - What remains to be done
2. Report to user with the chain state
3. The user can resume with `/workflow:resume` which auto-detects the resume point, or `/workflow:resume --from="[step]"` to resume from a specific step

## Step 0: Discovery (conditional)
**Evaluate whether discovery is needed.** Invoke the `discovery` subagent if the feature description is vague or underspecified — for example:
- "add a dashboard" (what kind? for whom? showing what?)
- "we need notifications" (what triggers them? how are they delivered?)
- "improve the user experience" (which part? what's wrong with it?)

**Skip discovery if** the feature is specific and well-scoped:
- "add CSV export to the contacts list page"
- "add OAuth2 login with Google"
- "add rate limiting to the /api/search endpoint at 100 req/min"

If invoking discovery:
1. The discovery agent scans the project structure to understand what exists
2. It has a conversation with the user to clarify the feature concept
3. It produces the Idea Brief at `docs/.workflow/idea-brief.md`
4. The Analyst then uses the Idea Brief as input

If skipping discovery, proceed directly to Step 0.5.

## Step 0.5: Feature Evaluation (always runs)
**Always invoke the `feature-evaluator` subagent** before proceeding to the Analyst. This is a mandatory gate that evaluates whether the feature is worth building.

1. The feature-evaluator reads the Idea Brief (if discovery ran) or the feature description from command arguments
2. It evaluates the feature across 7 dimensions: necessity, impact, complexity cost, alternatives, alignment, risk, and timing
3. It produces a scored evaluation with a **GO / CONDITIONAL / NO-GO** verdict
4. It saves the evaluation to `docs/.workflow/feature-evaluation.md`

**Based on the verdict:**
- **GO** → proceed to Step 1 (Analyst)
- **CONDITIONAL** → present the conditions to the user. If the user accepts the conditions and wants to proceed, continue to Step 1. If the user wants to modify the feature scope based on the conditions, return to Step 0 (Discovery) with the modified scope. If the user aborts, STOP the chain
- **NO-GO** → present the evaluation to the user. If the user **explicitly overrides** the NO-GO and wants to proceed anyway, document the override in the evaluation report and continue to Step 1. Otherwise, STOP the chain and report: "Feature evaluation resulted in NO-GO. The pipeline has been stopped. See docs/.workflow/feature-evaluation.md for details."

**The user always has the final say.** The feature-evaluator is advisory, not a veto.

## Step 1: Analyst
Invoke the `analyst` subagent. It MUST:
1. Read `docs/.workflow/idea-brief.md` if it exists (from discovery phase)
2. Read `specs/SPECS.md` index (not all files)
3. If `--scope` provided, read only that area's specs and code
4. If no `--scope`, determine minimal scope from the task description
5. Flag any drift between code and specs/docs
6. Perform impact analysis — what existing code/behavior is affected
7. Ask questions considering the current architecture
8. Generate requirements with IDs, MoSCoW priorities, acceptance criteria, and user stories
9. Build the traceability matrix
10. Explicitly state the scope in the requirements document

Save output to `specs/[domain]-requirements.md` and update `specs/SPECS.md`.

## Step 2: Architect
Invoke the `architect` subagent.
1. Read the Analyst's requirements (scope, priorities, and IDs are defined there)
2. Read only the scoped codebase and specs
3. Design the architecture including failure modes, security, and performance budgets
4. Update existing spec files or create new ones in `specs/`
5. Define how the new feature integrates with what already exists
6. Plan graceful degradation where applicable
7. Update `docs/` with new documentation
8. Update both master indexes (SPECS.md and DOCS.md)
9. Update the traceability matrix with architecture sections

## Step 2.5: Milestone Plan Extraction
After the Architect completes, parse the architecture document for milestones:

1. Read `specs/*-architecture.md` and look for a **Milestones** section (table with ID, Name, Scope (Modules), Scope (Requirements), Dependencies)
2. **If milestones are defined** (M1, M2, M3...): extract them into an ordered list respecting dependency order. A milestone cannot start until all its dependencies are complete.
3. **If no milestones are defined** (typical for small features): treat the entire feature as a single milestone — wrap all modules into one pass through Steps 3-8.5.
4. Save the milestone plan to `docs/.workflow/milestone-progress.md` with all milestones listed as `PENDING`.

## Steps 3-8: Milestone Loop
**For EACH milestone in dependency order**, execute the following steps. After completing all steps for a milestone, **auto-continue to the next milestone without user intervention**.

> **60% Context Budget:** Each agent invocation for a milestone must complete within 60% of its context window. The Architect sized milestones to respect this budget (max 3 modules each). If an agent stops due to budget exhaustion, save chain state to `docs/.workflow/chain-state.md` and use `/workflow:resume` to continue.

### Step 3: Test Writer (scoped to current milestone)
Invoke the `test-writer` subagent, **scoped to the current milestone's modules and requirements only**.
1. Read requirements with IDs, priorities, and acceptance criteria
2. Test Must requirements first, then Should, then Could
3. Every test references a requirement ID
4. Cover failure modes and security from the architect's design
5. Update the traceability matrix with test IDs
6. All previous tests must continue passing (regression)

### Step 4: Developer (scoped to current milestone)
Invoke the `developer` subagent, **scoped to the current milestone's modules only**.
Work within the scope defined by the analyst.
Module by module: read tests → implement → run tests → commit → next.

### Step 4.5: Compilation & Lint Validation
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

### Step 5: QA (scoped to current milestone)
Invoke the `qa` subagent, **scoped to the current milestone**.
1. Verify traceability matrix completeness for this milestone's requirements
2. Verify acceptance criteria for Must and Should requirements in this milestone
3. Run end-to-end flows including integration with existing functionality
4. Perform exploratory testing
5. Validate failure modes and security
6. Generate QA report at `docs/qa/[name]-M[N]-qa-report.md`

### Step 6: QA Iteration
If QA finds blocking issues:
- Developer fixes → QA re-validates (scoped to fix only)
- Repeat until QA approves

### Step 7: Reviewer (scoped to current milestone)
Invoke the `reviewer` subagent, **scoped to the current milestone**.
The reviewer specifically checks that specs/ and docs/ were updated for the new feature.
All work within the scope defined by the analyst.
Save output to `docs/reviews/[name]-M[N]-review.md`.

### Step 8: Review Iteration
If the reviewer finds critical issues:
- Developer fixes → reviewer re-reviews (scoped to fix only)
- Repeat until approved

### Step 8.5: Milestone Commit & Push
After the reviewer approves the current milestone:
1. `git add` all files relevant to this milestone (source code, tests, specs, docs, QA reports, review reports)
2. `git commit` with conventional message: `feat: complete [milestone name] (M[N])`
3. `git push` to the remote
4. Update `docs/.workflow/milestone-progress.md` — mark this milestone as `COMPLETE` with timestamp

**Then AUTO-CONTINUE to the next milestone.** No user intervention needed between milestones.

### Milestone Loop Termination
The loop ends when ALL milestones are marked `COMPLETE` in the progress file. If any milestone fails (agent error, retry limit exceeded), save the chain state and report which milestones completed and which remain.

## Step 9: Final Versioning
Once ALL milestones are complete:
1. Run the full test suite one final time to verify cross-milestone integration
2. Create the final version tag
3. Final `git push --tags`
4. Clean up `docs/.workflow/` temporary files (but keep `milestone-progress.md` as a record)
