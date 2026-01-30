---
name: sync-docs
description: Use this skill after completing a code implementation, bug fix, or feature to verify documentation alignment and commit. Enforces the Documentation Alignment Principle. Run with "/sync-docs <crate>" or "/sync-docs" for full workspace. Use "/sync-docs --audit" for alignment check only (no commit).
version: 2.0.0
---

# Sync Documentation Skill

This skill enforces the **Documentation Alignment Principle** by verifying and updating documentation after code changes.

**Key change in v2.0**: Documentation alignment is now MANDATORY, not discretionary. This skill verifies the truth hierarchy before allowing commits.

## Git Author (Required)

All commits MUST use:
```
Name:  E. Weil
Email: weil@doli.network
```

Always include `--author="E. Weil <weil@doli.network>"` in git commit commands.

## Truth Hierarchy

```
1. WHITEPAPER.md  →  Defines WHAT the protocol IS
2. specs/*        →  Defines HOW it works technically
3. docs/*         →  Defines HOW to use it
4. Code           →  Implements the above
```

**Rule**: Higher levels govern lower levels. Code must match WHITEPAPER. Specs must match code. Docs must match reality.

---

## Workflow

### Step 1: Run Tests

Run tests for the specified crate or full workspace:

```bash
# If crate specified:
cargo test -p <crate> 2>&1 | grep -i "pass\|fail\|error\|ok\|FAILED" | awk '!seen[$0]++' | head -30

# If no crate specified (full workspace):
cargo test 2>&1 | grep -i "pass\|fail\|error\|ok\|FAILED" | awk '!seen[$0]++' | head -30
```

**STOP if tests fail.** Report failure and do not proceed.

---

### Step 2: Analyze Changes

Understand what was changed:

```bash
git diff --name-only HEAD
git diff HEAD
```

**Categorize each change:**

| Category | Examples | Documentation Impact |
|----------|----------|---------------------|
| **Consensus** | VDF, producer selection, fork choice, block validation | WHITEPAPER + specs + maybe docs |
| **Economic** | Rewards, bonds, fees, slashing | WHITEPAPER + specs + docs |
| **Protocol** | Wire format, message types, handshake | specs |
| **Architecture** | New crates, component interactions | specs |
| **API** | RPC endpoints, response formats | specs + docs |
| **CLI** | Commands, flags, output | docs |
| **Node operation** | Config, startup, networking | docs |
| **Internal** | Refactoring, optimization, tests | NONE |

---

### Step 3: WHITEPAPER Alignment Check (MANDATORY)

**This step cannot be skipped for consensus, economic, or protocol changes.**

#### 3.1 Identify Relevant WHITEPAPER Sections

Read the sections that govern the changed functionality:

| If change affects... | Read WHITEPAPER section... |
|---------------------|---------------------------|
| Block structure | Section 3 (Block Structure) |
| Transactions | Section 2 (Transaction Model) |
| Consensus/VDF | Sections 4, 7, 8 (Proof of Time) |
| Producer selection | Sections 6, 7 (Producers) |
| Rewards/economics | Section 9 (Economic Model) |
| Network protocol | Section 5 (Network) |
| Security properties | Section 11 (Security) |

#### 3.2 Verify Compliance

Answer explicitly:

```
WHITEPAPER Alignment Check:
- Relevant section(s): <list sections>
- Code implements spec: YES / NO / PARTIAL
- Discrepancy found: YES / NO
- If YES, describe: <discrepancy>
```

#### 3.3 Handle Discrepancies

```
Code differs from WHITEPAPER?
├─ Code is WRONG → Fix the code, not the whitepaper
├─ WHITEPAPER needs update → STOP, escalate to user
└─ WHITEPAPER is silent → Document in specs, note gap
```

**STOP AND ESCALATE** if code intentionally differs from WHITEPAPER. User must approve whitepaper changes.

---

### Step 4: Specs Alignment Check (MANDATORY)

For changes affecting protocol, architecture, or security:

#### 4.1 Check Each Relevant Spec

| File | Check when... |
|------|---------------|
| `specs/protocol.md` | Wire format, messages, encoding changed |
| `specs/architecture.md` | Crate structure, component interaction changed |
| `specs/security_model.md` | Attack vectors, threat model affected |

#### 4.2 Verify Accuracy

For each relevant spec file:

1. **Read the current spec**
2. **Compare to new code behavior**
3. **Document findings:**

```
Specs Alignment Check:
- specs/protocol.md: ACCURATE / NEEDS UPDATE / N/A
- specs/architecture.md: ACCURATE / NEEDS UPDATE / N/A
- specs/security_model.md: ACCURATE / NEEDS UPDATE / N/A
```

---

### Step 5: Docs Alignment Check (MANDATORY)

For changes affecting user-facing behavior:

#### 5.1 Check Each Relevant Doc

| File | Check when... |
|------|---------------|
| `docs/cli.md` | CLI commands, flags, output changed |
| `docs/rpc_reference.md` | RPC endpoints, request/response changed |
| `docs/running_a_node.md` | Node config, startup, operation changed |
| `docs/becoming_a_producer.md` | Producer registration, requirements changed |
| `docs/troubleshooting.md` | New error conditions, failure modes |

#### 5.2 Verify Accuracy

```
Docs Alignment Check:
- docs/cli.md: ACCURATE / NEEDS UPDATE / N/A
- docs/rpc_reference.md: ACCURATE / NEEDS UPDATE / N/A
- docs/running_a_node.md: ACCURATE / NEEDS UPDATE / N/A
- docs/becoming_a_producer.md: ACCURATE / NEEDS UPDATE / N/A
- docs/troubleshooting.md: ACCURATE / NEEDS UPDATE / N/A
```

---

### Step 6: Apply Updates

For each file marked "NEEDS UPDATE":

1. **Read the file first** - Understand current content and style
2. **Make minimal, precise edits** - Only update affected sections
3. **Maintain consistency** - Match existing format and tone
4. **No aspirational content** - Only document what EXISTS

#### Update Checklist

- [ ] WHITEPAPER.md updates (if approved by user)
- [ ] specs/* updates
- [ ] docs/* updates
- [ ] Index files updated (SPECS.md, DOCS.md) if new files added

---

### Step 7: Present Alignment Report

Before committing, present this report to the user:

```
## Documentation Alignment Report

### Changes Analyzed
- Files modified: <list>
- Change category: <consensus|economic|protocol|api|cli|internal>

### WHITEPAPER Compliance
- Status: COMPLIANT / DISCREPANCY FOUND / N/A
- Sections verified: <list>
- Notes: <any concerns>

### Specs Alignment
- protocol.md: ✓ accurate | ✗ updated | - n/a
- architecture.md: ✓ accurate | ✗ updated | - n/a
- security_model.md: ✓ accurate | ✗ updated | - n/a

### Docs Alignment
- cli.md: ✓ accurate | ✗ updated | - n/a
- rpc_reference.md: ✓ accurate | ✗ updated | - n/a
- running_a_node.md: ✓ accurate | ✗ updated | - n/a
- <other relevant docs>

### Documentation Changes Made
- <file>: <what was updated>
- <file>: <what was updated>

### Ready to Commit
- [ ] Tests pass
- [ ] WHITEPAPER compliance verified
- [ ] Specs accurate
- [ ] Docs accurate

Proceed with commit? (waiting for confirmation)
```

---

### Step 8: Commit (After User Confirmation)

**Only proceed after user confirms the alignment report.**

```bash
# Stage specific files (never use git add -A)
git add <code-files> <doc-files>

# Commit with proper attribution
git commit --author="E. Weil <weil@doli.network>" -m "$(cat <<'EOF'
<type>(<scope>): <description>

<body explaining changes>

Documentation: <specs|docs> updated for alignment

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

---

## Audit Mode

When invoked with `--audit` flag, perform Steps 1-7 only (no commit). Use this to:
- Verify documentation alignment before major releases
- Check for documentation drift
- Audit after multiple changes

```
/sync-docs --audit        # Audit full workspace
/sync-docs core --audit   # Audit specific crate
```

---

## Quick Reference

```
/sync-docs Workflow:

1. TEST      → Run tests, stop if fail
2. ANALYZE   → Categorize changes
3. WHITEPAPER → Verify compliance (MANDATORY for consensus/economic)
4. SPECS     → Verify accuracy (MANDATORY for protocol/arch)
5. DOCS      → Verify accuracy (MANDATORY for user-facing)
6. UPDATE    → Make necessary changes
7. REPORT    → Present alignment report
8. COMMIT    → After user confirms
```

## Prohibited Actions

1. **NO commits without alignment verification** - Every commit must pass the checks
2. **NO outdated documentation** - If you change behavior, you update docs
3. **NO WHITEPAPER changes without approval** - Escalate discrepancies
4. **NO aspirational documentation** - Only document what exists
5. **NO skipping steps** - All mandatory checks must be performed

## Decision Tree

```
Change made?
│
├─ Affects consensus/economic?
│  └─ MUST verify WHITEPAPER alignment
│     └─ Discrepancy? → STOP, escalate
│
├─ Affects protocol/architecture?
│  └─ MUST verify specs/* accuracy
│     └─ Outdated? → UPDATE before commit
│
├─ Affects user-facing behavior?
│  └─ MUST verify docs/* accuracy
│     └─ Outdated? → UPDATE before commit
│
└─ Internal only?
   └─ Verify no doc impact, then commit
```
