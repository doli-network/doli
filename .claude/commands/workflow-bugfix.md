---
name: workflow:bugfix
description: Fix a bug with a reduced chain. Accepts optional --scope to limit context.
---

# Workflow: Bugfix

Optional: `--scope="file or module"` to point directly at the suspected area.

## Step 1: Analyst
Analyze the reported bug.
1. If `--scope` provided, read only that file/module and its spec
2. If no `--scope`, use Grep to locate the relevant code from the bug description
3. Read only the affected code and related spec files
4. Identify the probable cause
5. Perform impact analysis — what else might be affected by the fix
6. Flag if the bug reveals a specs/docs drift
7. Generate requirements with IDs, priorities, and acceptance criteria for the fix

Save output to `specs/bugfixes/[name]-analysis.md`.

## Step 2: Test Writer
Write a test that REPRODUCES the bug (it must fail).
1. Reference the requirement ID from the analyst's document
2. Read only the affected module's existing tests to match conventions
3. Add related edge case tests
4. Consider: does this bug pattern exist elsewhere? If so, note it.

## Step 3: Developer
Fix the bug. Read only the affected module.
The reproduction test must pass.
Run all existing tests to check for regression.

## Step 4: QA
Invoke the `qa` subagent.
1. Verify the bug is actually fixed — reproduce the original scenario
2. Verify acceptance criteria from the analyst's document
3. Test related flows — ensure the fix didn't break adjacent functionality
4. Verify the fix addresses the root cause, not just the symptom
5. Generate QA report

## Step 5: QA Iteration
If QA finds the bug is not fully fixed or the fix broke something else:
- Developer fixes → QA re-validates
- Repeat until QA approves

## Step 6: Reviewer
Review only the changed files.
Verify it's not a superficial patch but a root cause fix.
Verify that relevant specs/docs are updated if the bug revealed incorrect documentation.
Clean up `docs/.workflow/` temporary files.
