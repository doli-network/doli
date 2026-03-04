---
name: workflow:new
description: Start a new project from scratch with the full workflow
---

# Workflow: New Project

The user wants to create something new from scratch. Execute the full chain.

**This is a greenfield project.** There may be no existing code, no `specs/`, and no `docs/`. Each agent must handle this gracefully — creating structure instead of reading it.

## Fail-Safe Controls

### Iteration Limits (per milestone)
- **QA ↔ Developer iterations (Steps 6-7):** Maximum **3 iterations** per milestone. If QA still finds blocking issues after 3 rounds, STOP and report to user: "QA iteration limit reached (3/3) for milestone M[N]. Remaining issues: [list]. Requires human decision on how to proceed."
- **Reviewer ↔ Developer iterations (Steps 8-9):** Maximum **2 iterations** per milestone. If the reviewer still finds critical issues after 2 rounds, STOP and report to user: "Review iteration limit reached (2/2) for milestone M[N]. Remaining issues: [list]. Requires human decision."

### Inter-Step Output Validation
Before invoking each agent, verify the previous agent produced its expected output:
- Before Analyst (Step 2): verify `docs/.workflow/idea-brief.md` exists
- Before Architect (Step 3): verify `specs/*-requirements.md` exists
- Before Milestone Extraction (Step 3.5): verify `specs/*-architecture.md` exists
- Before Test Writer (Step 4, each milestone): verify `specs/*-architecture.md` contains the milestone's modules
- Before Developer (Step 5, each milestone): verify test files exist for the milestone's modules
- Before Compilation Validation (Step 5.5, each milestone): verify source code files exist
- Before QA (Step 6, each milestone): verify compilation & lint validation passed (build + lint + tests clean)
- Before Reviewer (Step 8, each milestone): verify QA report exists in `docs/qa/`

**If any expected output is missing, STOP the chain** and report: "CHAIN HALTED at Step [N] (Milestone M[X]): Expected output from [agent] not found. [What's missing]. Previous agent may have failed silently."

### Error Recovery
If any agent fails mid-chain:
1. Save the chain state to `docs/.workflow/chain-state.md` with:
   - Which steps completed successfully (and their output files)
   - Which step failed and why
   - What remains to be done
2. Report to user with the chain state
3. The user can resume with `/workflow:resume` which auto-detects the resume point, or `/workflow:resume --from="[step]"` to resume from a specific step

## Step 1: Discovery
Invoke the `discovery` subagent with the user's raw idea.
The discovery agent is the ONLY agent that has extended back-and-forth conversation with the user.
1. Let it explore the idea with the user — what problem it solves, who uses it, what's essential vs. nice-to-have
2. It will challenge the idea itself — is this the right thing to build?
3. It will help the user find the MVP scope
4. Wait for the discovery agent to produce the Idea Brief at `docs/.workflow/idea-brief.md`
5. If the user's description is already detailed and specific, the discovery agent will move quickly

**Do NOT skip this step.** Even "obvious" ideas benefit from a brief validation pass.

## Step 2: Analyst
Once the Idea Brief is ready, invoke the `analyst` subagent passing both the original idea AND the Idea Brief.
1. The analyst reads `docs/.workflow/idea-brief.md` to understand the validated concept
2. If `specs/SPECS.md` exists, read it to understand any existing project layout
3. If `specs/SPECS.md` does NOT exist, skip codebase reading — this is a new project
4. Now focus on turning the Idea Brief into formal requirements — questioning technical details, edge cases, and dependencies
5. Generate the requirements document with:
   - Unique requirement IDs (REQ-[DOMAIN]-[NNN])
   - MoSCoW priorities (Must/Should/Could/Won't)
   - Acceptance criteria for every requirement
   - User stories where applicable
   - Impact analysis (if any existing code)
   - Initial traceability matrix
6. Create `specs/` directory if it doesn't exist
7. Save output to `specs/[domain]-requirements.md` and create/update `specs/SPECS.md` index

## Step 3: Architect
Once the analyst completes, invoke the `architect` subagent passing the requirements document.
1. If this is a new project (no existing code), design the full project structure:
   - Create `backend/` (and `frontend/` if needed) directory layout
   - Define module structure, interfaces, dependencies
2. Design failure modes and recovery strategies per module
3. Define security considerations and trust boundaries
4. Set performance budgets and complexity targets
5. Plan graceful degradation behavior
6. Create `specs/` and `docs/` scaffolding if they don't exist
7. Save specs to `specs/[domain].md` and create/update `specs/SPECS.md`
8. Save docs to `docs/[topic].md` and create/update `docs/DOCS.md`
9. Update the traceability matrix with architecture sections

## Step 3.5: Milestone Plan Extraction
After the Architect completes, parse the architecture document for milestones:

1. Read `specs/*-architecture.md` and look for a **Milestones** section (table with ID, Name, Scope (Modules), Scope (Requirements), Dependencies)
2. **If milestones are defined** (M1, M2, M3...): extract them into an ordered list respecting dependency order. A milestone cannot start until all its dependencies are complete.
3. **If no milestones are defined** (small project): treat the entire project as a single milestone — wrap all modules into one pass through Steps 4-9.5.
4. Save the milestone plan to `docs/.workflow/milestone-progress.md` with all milestones listed as `PENDING`.

### Milestone Progress File Format
```markdown
# Milestone Progress

| Milestone | Name | Status | Started | Completed |
|-----------|------|--------|---------|-----------|
| M1 | Core Infrastructure | PENDING | — | — |
| M2 | Service Layer | PENDING | — | — |
| M3 | Integration Layer | PENDING | — | — |
```

## Steps 4-9: Milestone Loop
**For EACH milestone in dependency order**, execute the following steps. After completing all steps for a milestone, **auto-continue to the next milestone without user intervention**.

> **60% Context Budget:** Each agent invocation for a milestone must complete within 60% of its context window. The Architect sized milestones to respect this budget (max 3 modules each). If an agent stops due to budget exhaustion, save chain state to `docs/.workflow/chain-state.md` and use `/workflow:resume` to continue.

### Step 4: Test Writer (scoped to current milestone)
Invoke the `test-writer` subagent passing the architecture, **scoped to the current milestone's modules and requirements only**.
1. Tests Must requirements first (exhaustive coverage + edge cases)
2. Tests Should requirements second (solid coverage)
3. Tests Could requirements last (basic happy path)
4. Every test references a requirement ID for traceability
5. Tests cover failure modes and security from the architect's design
6. Updates the traceability matrix with test IDs
7. All tests must fail initially (red phase)

### Step 5: Developer (scoped to current milestone)
Invoke the `developer` subagent, **scoped to the current milestone's modules only**.
The developer works one module at a time: read tests → implement → run tests → commit → next.
Must implement module by module until all tests pass.
If context gets heavy mid-implementation, commit progress and continue.

### Step 5.5: Compilation & Lint Validation
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

### Step 6: QA (scoped to current milestone)
Invoke the `qa` subagent, **scoped to the current milestone**.
1. Verify the traceability matrix is complete for this milestone's requirements
2. Verify acceptance criteria for every Must requirement in this milestone
3. Verify acceptance criteria for every Should requirement in this milestone
4. Run end-to-end flows that cross module boundaries within this milestone
5. Perform exploratory testing — try what no test anticipated
6. Validate failure modes and recovery strategies
7. Validate security considerations
8. Generate QA report at `docs/qa/[name]-M[N]-qa-report.md`

### Step 7: QA Iteration
If QA finds blocking issues (Must requirements failing, broken flows):
- Return to the developer with the QA findings
- The developer fixes them (scoped to the affected area only)
- QA re-validates (scoped to the fix only)
- Repeat until QA approves

### Step 8: Reviewer (scoped to current milestone)
Once QA approves, invoke the `reviewer` subagent, **scoped to the current milestone**.
The reviewer works module by module, saving findings incrementally.
Wait for the review report, including specs/docs drift check.
Save output to `docs/reviews/[name]-M[N]-review.md`.

### Step 9: Review Iteration
If the reviewer finds critical issues:
- Return to the developer with the findings
- The developer fixes them (scoped to the affected module only)
- The reviewer reviews again (scoped to the fix only)
- Repeat until approved

### Step 9.5: Milestone Commit & Push
After the reviewer approves the current milestone:
1. `git add` all files relevant to this milestone (source code, tests, specs, docs, QA reports, review reports)
2. `git commit` with conventional message: `feat: complete [milestone name] (M[N])`
3. `git push` to the remote
4. Update `docs/.workflow/milestone-progress.md` — mark this milestone as `COMPLETE` with timestamp

**Then AUTO-CONTINUE to the next milestone.** No user intervention needed between milestones.

### Milestone Loop Termination
The loop ends when ALL milestones are marked `COMPLETE` in the progress file. If any milestone fails (agent error, retry limit exceeded), save the chain state and report which milestones completed and which remain.

## Step 10: Final Versioning
Once ALL milestones are complete:
1. Run the full test suite one final time to verify cross-milestone integration
2. Create the final version tag
3. Final `git push --tags`
4. Clean up `docs/.workflow/` temporary files (but keep `milestone-progress.md` as a record)
