---
name: workflow:improve
description: Improve existing code — refactor, optimize, or enhance without adding new features. Accepts optional --scope to limit context.
---

# Workflow: Improve Existing Code

The user wants to improve code that already works — refactoring, performance optimization, code quality enhancement, or simplification.
This is NOT for adding new features or fixing bugs. The behavior should stay the same; the implementation gets better.
Optional: `--scope="area"` to limit which part of the codebase is analyzed.

## Step 1: Analyst (improvement-focused)
Invoke the `analyst` subagent. It MUST:
1. Read `specs/SPECS.md` index (not all files)
2. If `--scope` provided, read only that area's specs and code
3. If no `--scope`, determine minimal scope from the improvement description
4. Read the **actual code** in the scoped area — focus on:
   - Code smells (duplication, long functions, deep nesting, unclear naming)
   - Performance issues (unnecessary allocations, O(n²) where O(n) is possible, blocking calls)
   - Complexity (can this be simplified without losing functionality?)
   - Pattern violations (code that doesn't match the project's established conventions)
5. Ask clarifying questions about the desired improvement direction
6. Generate a requirements document that specifies:
   - What the current code does (behavior to preserve)
   - What specifically will be improved
   - What will NOT change (explicit boundaries)

Save output to `specs/improvements/[domain]-improvement.md`.

## Step 2: Test Writer (regression-focused)
Invoke the `test-writer` subagent. It MUST:
1. Read the analyst's improvement document
2. Read existing tests for the affected modules
3. Write **regression tests** that lock in current behavior BEFORE any changes
4. Cover edge cases that the improvement might accidentally break
5. If existing tests already cover the behavior well, state that and add only missing edge cases

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

## Step 4: Reviewer (improvement-focused)
Invoke the `reviewer` subagent. It MUST:
1. Verify the improvement actually improves things (not just reshuffling)
2. Confirm no behavior changes slipped in
3. Check that all tests pass (regression + existing)
4. Verify specs/docs are still accurate after the changes
5. Look for opportunities missed or improvements that went too far

Save output to `docs/reviews/[name]-improvement-review.md`.

## Step 5: Iteration
If the reviewer finds issues:
- Return to the developer with findings (scoped to affected module only)
- Developer fixes → reviewer re-reviews (scoped to fix only)
- Repeat until approved

## Step 6: Versioning
Once approved, create the final commit and version tag.
Clean up `docs/.workflow/` temporary files.
