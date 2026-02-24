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
5. Flag if the bug reveals a specs/docs drift

Save output to `specs/bugfixes/[name]-analysis.md`.

## Step 2: Test Writer
Write a test that REPRODUCES the bug (it must fail).
Read only the affected module's existing tests to match conventions.
Add related edge case tests.

## Step 3: Developer
Fix the bug. Read only the affected module.
The reproduction test must pass.
Run all existing tests to check for regression.

## Step 4: Reviewer
Review only the changed files.
Verify it's not a superficial patch but a root cause fix.
Verify that relevant specs/docs are updated if the bug revealed incorrect documentation.
Clean up `docs/.workflow/` temporary files.
