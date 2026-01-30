---
name: fix-bug
description: Use this skill when fixing any bug. It enforces root cause analysis, whitepaper compliance, and requires user validation before commit. Invoked with "/fix-bug <description>" or "/fix-bug" to start the protocol.
version: 1.0.0
---

# Bug Fixing Protocol Skill

**CRITICAL**: This is state-of-the-art blockchain technology. Quick fixes that mask symptoms are strictly prohibited. This skill enforces rigorous bug-fixing discipline.

## Git Author (Required)

All commits MUST use:
```
Name:  E. Weil
Email: weil@doli.network
```

## Invocation

```
/fix-bug <bug description or reference>
/fix-bug REPORT_*.md
/fix-bug
```

## Interactive Workflow

When this skill is invoked, follow **ALL phases in order**. Use checkboxes to track progress and **STOP** at mandatory checkpoints.

---

## Phase 1: Root Cause Analysis

### Checklist

- [ ] **1.1 Reproduce the bug** - Confirm the bug exists and understand its symptoms
- [ ] **1.2 Trace to origin** - Identify WHERE in the codebase the bug originates (not just where it manifests)
- [ ] **1.3 Understand WHY** - Determine the logical error or missing condition that causes the bug
- [ ] **1.4 Map affected components** - List all components that interact with the buggy code

### Component Impact Assessment

For each affected component, answer:

| Component | Question | Answer |
|-----------|----------|--------|
| **Consensus** | Does it affect PoT, VDF validation, producer selection, fork choice? | |
| **State** | Does it affect UTXO set, bond registry, reward calculations? | |
| **Network** | Does it affect message propagation, sync, peer management? | |
| **Crypto** | Does it affect signatures, hashes, key derivation? | |
| **Storage** | Does it affect persistence, state recovery, chain reorganization? | |

### Architectural Implications

Answer these questions BEFORE writing any fix:

1. **Invariants**: Does this fix change any invariant or assumption in the protocol?
2. **Race conditions**: Could this fix introduce timing issues or state inconsistencies?
3. **Security**: Does it affect any property defined in `specs/security_model.md`?
4. **Consensus**: Will it impact finality, fork choice, or producer selection?
5. **Compatibility**: Does it change wire format or break existing nodes?

**Output**: Present root cause analysis to user in this format:

```
## Root Cause Analysis

**Bug Location**: <file:line>
**Root Cause**: <explanation of why the bug occurs>
**Affected Components**: <list>
**Architectural Impact**: <none | low | medium | high> - <explanation>
```

---

## Phase 2: Whitepaper Compliance

### MANDATORY CHECK

**You are strictly prohibited from implementing anything not specified in the WHITEPAPER.**

- [ ] **2.1 Read relevant WHITEPAPER sections** - Identify which sections govern the affected functionality
- [ ] **2.2 Verify fix aligns with spec** - The fix MUST implement what the whitepaper specifies
- [ ] **2.3 Check for gaps** - If the bug reveals implementation differs from whitepaper, whitepaper is truth

### Decision Tree

```
Is the fix specified in WHITEPAPER.md?
├─ YES → Proceed to implementation
├─ NO, but whitepaper is silent → STOP, escalate to user
└─ NO, contradicts whitepaper → STOP, this is not a bug, it's a feature gap
```

### If Escalation Required

Ask the user:
```
The proposed fix involves behavior not specified in the WHITEPAPER:

- Affected section: <whitepaper section>
- Current spec says: <quote or "silent on this matter">
- Proposed behavior: <what the fix would do>

How should I proceed?
1. Update WHITEPAPER first, then implement
2. Implement as specified (provide spec)
3. Abandon this approach
```

---

## Phase 3: Implementation

### Pre-Implementation Checklist

- [ ] **3.1 Root cause identified** (Phase 1 complete)
- [ ] **3.2 Whitepaper compliant** (Phase 2 complete)
- [ ] **3.3 Minimal fix designed** - Only fix what's broken, no refactoring

### Implementation Rules

1. **Minimal change** - Fix only the root cause, nothing more
2. **No side effects** - Don't "improve" surrounding code
3. **Add regression test** - Write a test that would have caught this bug
4. **Preserve existing tests** - Don't modify tests to make them pass

### Execute Implementation

```bash
# After implementing the fix:

# Run tests for affected crate
cargo test -p <crate> 2>&1 | grep -i "pass\|fail\|error\|ok" | awk '!seen[$0]++' | head -30

# Run full test suite
cargo test 2>&1 | grep -i "pass\|fail\|error\|ok" | awk '!seen[$0]++' | head -30

# Lint and format
cargo clippy 2>&1 | grep -i "error\|warning" | head -20
cargo fmt --check
```

### Post-Implementation Checklist

- [ ] **3.4 All tests pass**
- [ ] **3.5 No clippy warnings introduced**
- [ ] **3.6 Code formatted**
- [ ] **3.7 Regression test added**

---

## Phase 4: User Validation

### MANDATORY CHECKPOINT - DO NOT SKIP

**You are strictly prohibited from committing without explicit user validation.**

Present this summary to the user:

```
## Bug Fix Summary - Awaiting Validation

### Root Cause
<Brief explanation of the root cause>

### Fix Applied
<Description of the code changes>

### Files Modified
- <file1>: <what changed>
- <file2>: <what changed>

### Whitepaper Compliance
- Relevant section: <section number and title>
- Compliance: VERIFIED / ESCALATED

### Test Results
- Unit tests: PASSED / FAILED
- Regression test: ADDED at <file:line>

### Architectural Impact
<none | low | medium | high> - <brief explanation>

---

**Have you validated that this bug is fixed?**

Please confirm by testing the fix yourself, then respond with:
- "yes" or "confirmed" - to proceed to commit
- "no" or describe issues - to iterate on the fix
```

### WAIT FOR USER RESPONSE

- [ ] **4.1 User has validated the fix** - Received explicit confirmation

**DO NOT PROCEED TO PHASE 5 WITHOUT USER CONFIRMATION**

---

## Phase 5: Documentation and Commit

### Only After User Confirms

- [ ] **5.1 Check if documentation update needed**

Documentation update required ONLY if:
- The fix changes user-facing behavior
- The fix changes API/RPC responses
- The fix changes CLI behavior
- The fix reveals undocumented behavior that should be documented

**Do NOT update docs for:**
- Internal-only fixes
- Performance fixes
- Code that already matched the spec

### If REPORT_*.md Exists

- [ ] **5.2 Update the report with:**
  - Root cause analysis
  - Fix description
  - Resolution date
- [ ] **5.3 Move report to `docs/legacy/bugs/`**

### Commit

```bash
# Stage specific files only
git add <modified-files>

# If REPORT file was moved
git add docs/legacy/bugs/REPORT_*.md
git rm <original-report-location>

# Commit with full attribution
git commit --author="E. Weil <weil@doli.network>" -m "$(cat <<'EOF'
fix(<scope>): <brief description of root cause fix>

Root cause: <one-line explanation>
Fix: <one-line explanation of solution>

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

### Final Checklist

- [ ] **5.4 Commit created**
- [ ] **5.5 No uncommitted changes** (`git status` clean)

---

## Quick Reference Card

```
/fix-bug Protocol:

Phase 1: ROOT CAUSE ANALYSIS
  → Reproduce → Trace origin → Map impact → Document

Phase 2: WHITEPAPER CHECK
  → Read spec → Verify alignment → Escalate if silent

Phase 3: IMPLEMENTATION
  → Minimal fix → Add test → Run tests → Lint

Phase 4: USER VALIDATION [MANDATORY STOP]
  → Present summary → Wait for "confirmed"

Phase 5: COMMIT (only after user confirms)
  → Update docs if needed → Commit
```

## Prohibited Actions

1. **NO quick fixes** - Never fix symptoms without understanding root cause
2. **NO commits without validation** - User must explicitly confirm
3. **NO whitepaper deviations** - Cannot invent behavior not in spec
4. **NO unnecessary changes** - Fix only what's broken
5. **NO test modifications** - Don't change tests to make them pass
6. **NO silent commits** - Always present summary and wait

## Error Recovery

If at any point you realize:
- Root cause was misidentified → Return to Phase 1
- Fix violates whitepaper → Return to Phase 2
- Tests fail after fix → Return to Phase 3
- User rejects fix → Return to appropriate phase based on feedback
