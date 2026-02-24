---
name: workflow:new
description: Start a new project from scratch with the full workflow
---

# Workflow: New Project

The user wants to create something new from scratch. Execute the full chain.

**This is a greenfield project.** There may be no existing code, no `specs/`, and no `docs/`. Each agent must handle this gracefully — creating structure instead of reading it.

## Step 1: Analyst
Invoke the `analyst` subagent with the user's description.
1. If `specs/SPECS.md` exists, read it to understand any existing project layout
2. If `specs/SPECS.md` does NOT exist, skip codebase reading — this is a new project
3. Focus entirely on questioning the idea, clarifying requirements, and identifying risks
4. Generate the requirements document with confirmed assumptions
5. Create `specs/` directory if it doesn't exist
6. Save output to `specs/[domain]-requirements.md` and create/update `specs/SPECS.md` index

## Step 2: Architect
Once the analyst completes, invoke the `architect` subagent passing the requirements document.
1. If this is a new project (no existing code), design the full project structure:
   - Create `backend/` (and `frontend/` if needed) directory layout
   - Define module structure, interfaces, dependencies
2. Create `specs/` and `docs/` scaffolding if they don't exist
3. Save specs to `specs/[domain].md` and create/update `specs/SPECS.md`
4. Save docs to `docs/[topic].md` and create/update `docs/DOCS.md`

## Step 3: Test Writer
Once the architect completes, invoke the `test-writer` subagent passing the architecture.
The test-writer works one module at a time, saving tests to disk after each.
Wait for it to generate all tests (they must fail initially).

## Step 4: Developer
Once tests are written, invoke the `developer` subagent.
The developer works one module at a time: read tests → implement → run tests → commit → next.
Must implement module by module until all tests pass.
If context gets heavy mid-implementation, commit progress and continue.

## Step 5: Reviewer
Once all code passes the tests, invoke the `reviewer` subagent.
The reviewer works module by module, saving findings incrementally.
Wait for the review report, including specs/docs drift check.
Save output to `docs/reviews/[name]-review.md`.

## Step 6: Iteration
If the reviewer finds critical issues:
- Return to the developer with the findings
- The developer fixes them (scoped to the affected module only)
- The reviewer reviews again (scoped to the fix only)
- Repeat until approved

## Step 7: Versioning
Once approved, create the final commit and version tag.
Clean up `docs/.workflow/` temporary files.
