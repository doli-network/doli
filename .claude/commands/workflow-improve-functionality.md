---
name: workflow:improve-functionality
description: Improve existing code — refactor, optimize, or enhance without adding new features. Accepts optional --scope to limit context.
---

# Workflow: Improve Existing Code

The user wants to improve code that already works — refactoring, performance optimization, code quality enhancement, or simplification.
This is NOT for adding new features or fixing bugs. The behavior should stay the same; the implementation gets better.
Optional: `--scope="area"` to limit which part of the codebase is analyzed.

**Note: This workflow does NOT invoke the Architect.** The architecture already exists — only the implementation changes.

## Existing Code Validation
Before starting, verify there is code to improve:
1. Check for source code files in the project. If none exist, this workflow is inapplicable — inform the user.
2. If `specs/SPECS.md` does not exist, proceed but note: "No specs found. Improvement analysis will be based solely on codebase reading."

## Fail-Safe Controls

### Iteration Limits
- **QA ↔ Developer iterations (Steps 4-5):** Maximum **3 iterations**. If QA still finds behavioral changes or broken flows after 3 rounds, STOP and report to user: "QA iteration limit reached (3/3). Remaining issues: [list]. Requires human decision."
- **Reviewer ↔ Developer iterations (Steps 6-7):** Maximum **2 iterations**. If the reviewer still finds issues after 2 rounds, STOP and report to user: "Review iteration limit reached (2/2). Remaining issues: [list]. Requires human decision."

### Inter-Step Output Validation
Before invoking each agent, verify the previous agent produced its expected output:
- Before Test Writer (Step 2): verify `specs/improvements/*-improvement.md` exists
- Before Developer (Step 3): verify test files exist (new regression tests or confirmation that existing tests suffice)
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

## Step 1: Analyst (improvement-focused)
Invoke the `analyst` subagent. It MUST:
1. Read `specs/SPECS.md` index (not all files)
2. If `--scope` provided, read only that area's specs and code
3. If no `--scope`, determine minimal scope from the improvement description
4. Read the **actual code** in the scoped area — focus on:
   - Code smells (duplication, long functions, deep nesting, unclear naming)
   - Performance issues (unnecessary allocations, O(n^2) where O(n) is possible, blocking calls)
   - Complexity (can this be simplified without losing functionality?)
   - Pattern violations (code that doesn't match the project's established conventions)
5. Perform impact analysis — what other modules depend on the code being improved
6. Ask clarifying questions about the desired improvement direction
7. Generate a requirements document with IDs, priorities, and acceptance criteria that specifies:
   - What the current code does (behavior to preserve)
   - What specifically will be improved
   - What will NOT change (explicit boundaries)

Save output to `specs/improvements/[domain]-improvement.md`.

## Step 2: Test Writer (regression-focused)
Invoke the `test-writer` subagent. It MUST:
1. Read the analyst's improvement document (IDs, priorities, acceptance criteria)
2. Read existing tests for the affected modules
3. Write **regression tests** that lock in current behavior BEFORE any changes
4. Reference requirement IDs for traceability
5. Cover edge cases that the improvement might accidentally break
6. If existing tests already cover the behavior well, state that and add only missing edge cases

The goal is a safety net: after the improvement, all tests must still pass.

## Step 3: Developer (refactor-focused)
Invoke the `developer` subagent. It MUST:
1. Read the analyst's improvement document and the test suite
2. Read the scoped codebase to understand current conventions
3. Implement the improvement one module at a time
4. After each change, run ALL tests (new regression tests + existing tests)
5. Never change behavior — only implementation
6. Commit after each module with `refactor:` or `perf:` prefix

**Cycle:** Understand → Improve → Test → Commit → Next

## Step 4: QA (regression-focused)
Invoke the `qa` subagent. It MUST:
1. Verify that behavior has NOT changed — run end-to-end flows before and after comparison
2. Verify acceptance criteria (the improvement targets were met)
3. Check that no functionality was accidentally removed or altered
4. Validate that performance improvements are measurable (if applicable)
5. Generate QA report

## Step 5: QA Iteration
If QA finds behavioral changes or broken flows:
- Developer fixes → QA re-validates
- Repeat until QA confirms behavior is preserved

## Step 6: Reviewer (improvement-focused)
Invoke the `reviewer` subagent. It MUST:
1. Verify the improvement actually improves things (not just reshuffling)
2. Confirm no behavior changes slipped in
3. Check that all tests pass (regression + existing)
4. Verify specs/docs are still accurate after the changes
5. Look for opportunities missed or improvements that went too far

Save output to `docs/reviews/[name]-improvement-review.md`.

## Step 7: Review Iteration
If the reviewer finds issues:
- Return to the developer with findings (scoped to affected module only)
- Developer fixes → reviewer re-reviews (scoped to fix only)
- Repeat until approved

## Step 8: Versioning
Once approved, create the final commit and version tag.
Clean up `docs/.workflow/` temporary files.
