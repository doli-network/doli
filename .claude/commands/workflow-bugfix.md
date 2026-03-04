---
name: workflow:bugfix
description: Fix a bug with a reduced chain. Accepts optional --scope to limit context.
---

# Workflow: Bugfix

Optional: `--scope="file or module"` to point directly at the suspected area.

## Fail-Safe Controls

### Bug Verification
Before starting the chain, verify the bug is reproducible:
1. If the bug description includes reproduction steps, try them first
2. If the bug cannot be reproduced from the description alone, the Analyst should note this and proceed with code analysis to identify the probable cause
3. If no relevant code can be found from the bug description (even with Grep), STOP and ask the user for more context

### Iteration Limits
- **QA ↔ Developer iterations (Steps 4-5):** Maximum **3 iterations**. If QA still finds the bug is not fully fixed after 3 rounds, STOP and report to user: "QA iteration limit reached (3/3). Bug status: [description]. Requires human decision."
- **Reviewer ↔ Developer iterations (Steps 6-7):** Maximum **2 iterations**. If the reviewer still finds critical issues after 2 rounds, STOP and report to user: "Review iteration limit reached (2/2). Remaining issues: [list]. Requires human decision."

### Inter-Step Output Validation
Before invoking each agent, verify the previous agent produced its expected output:
- Before Test Writer (Step 2): verify `specs/bugfixes/*-analysis.md` exists
- Before Developer (Step 3): verify reproduction test file exists
- Before QA (Step 4): verify code changes exist
- Before Reviewer (Step 6): verify QA report exists in `docs/qa/`

**If any expected output is missing, STOP the chain** and report: "CHAIN HALTED at Step [N]: Expected output from [agent] not found. [What's missing]."

### Error Recovery
If any agent fails mid-chain:
1. Save the chain state to `docs/.workflow/chain-state.md` with:
   - Which steps completed successfully (and their output files)
   - Which step failed and why
   - What remains to be done
2. Report to user with the chain state
3. The user can resume by re-invoking the failed step's agent manually

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
- Repeat until QA approves (max 3 iterations — see Fail-Safe Controls above)

## Step 6: Reviewer
Review only the changed files.
Verify it's not a superficial patch but a root cause fix.
Verify that relevant specs/docs are updated if the bug revealed incorrect documentation.

## Step 7: Review Iteration
If the reviewer finds critical issues:
- Return to the developer with the findings
- The developer fixes them (scoped to the affected area only)
- The reviewer reviews again (scoped to the fix only)
- Repeat until approved (max 2 iterations — see Fail-Safe Controls above)

## Step 8: Versioning
Once approved, create the final commit with `fix:` prefix.
Clean up `docs/.workflow/` temporary files.
