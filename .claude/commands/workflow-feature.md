---
name: workflow:feature
description: Add a feature to an existing project. Accepts optional --scope to limit context.
---

# Workflow: New Feature

The user wants to add functionality to existing code.
Optional: `--scope="area"` to limit which part of the codebase is analyzed.

## Step 1: Analyst
Invoke the `analyst` subagent. It MUST:
1. Read `specs/SPECS.md` index (not all files)
2. If `--scope` provided, read only that area's specs and code
3. If no `--scope`, determine minimal scope from the task description
4. Flag any drift between code and specs/docs
5. Ask questions considering the current architecture
6. Explicitly state the scope in the requirements document

Save output to `specs/[domain]-requirements.md` and update `specs/SPECS.md`.

## Step 2: Architect
Invoke the `architect` subagent.
1. Read the Analyst's requirements (scope is defined there)
2. Read only the scoped codebase and specs
3. Update existing spec files or create new ones in `specs/`
4. Define how the new feature integrates with what already exists
5. Update `docs/` with new documentation
6. Update both master indexes (SPECS.md and DOCS.md)

## Steps 3-7: Same as workflow:new
The test-writer, developer, and reviewer follow the same flow.
All work within the scope defined by the analyst.
All previous tests must continue passing (regression).
The reviewer specifically checks that specs/ and docs/ were updated for the new feature.
