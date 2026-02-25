---
name: workflow:feature
description: Add a feature to an existing project. Accepts optional --scope to limit context.
---

# Workflow: New Feature

The user wants to add functionality to existing code.
Optional: `--scope="area"` to limit which part of the codebase is analyzed.

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

If skipping discovery, proceed directly to Step 1.

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

## Step 3: Test Writer
Invoke the `test-writer` subagent.
1. Read requirements with IDs, priorities, and acceptance criteria
2. Test Must requirements first, then Should, then Could
3. Every test references a requirement ID
4. Cover failure modes and security from the architect's design
5. Update the traceability matrix with test IDs
6. All previous tests must continue passing (regression)

## Step 4: Developer
Invoke the `developer` subagent.
Work within the scope defined by the analyst.
Module by module: read tests → implement → run tests → commit → next.

## Step 5: QA
Invoke the `qa` subagent.
1. Verify traceability matrix completeness
2. Verify acceptance criteria for Must and Should requirements
3. Run end-to-end flows including integration with existing functionality
4. Perform exploratory testing
5. Validate failure modes and security
6. Generate QA report

## Step 6: QA Iteration
If QA finds blocking issues:
- Developer fixes → QA re-validates (scoped to fix only)
- Repeat until QA approves

## Step 7: Reviewer
Invoke the `reviewer` subagent.
The reviewer specifically checks that specs/ and docs/ were updated for the new feature.
All work within the scope defined by the analyst.

## Step 8: Review Iteration
If the reviewer finds critical issues:
- Developer fixes → reviewer re-reviews (scoped to fix only)
- Repeat until approved

## Step 9: Versioning
Once approved, create the final commit and version tag.
Clean up `docs/.workflow/` temporary files.
