---
name: sync-docs
description: Use this skill after completing a code implementation, bug fix, or feature to test, sync documentation, and commit. Run with "/sync-docs <crate>" or "/sync-docs" for full workspace.
version: 1.0.0
---

# Sync Documentation Skill

This skill automates the post-implementation workflow: test → update docs → commit.

## Git Author (Required)

All commits MUST use:
```
Name:  E. Weil
Email: weil@doli.network
```

Always include `--author="E. Weil <weil@doli.network>"` in git commit commands.

## Workflow

When invoked, follow these steps **in order**:

### Step 1: Run Unit Tests

Run tests for the specified crate or full workspace:

```bash
# If crate specified:
just test-crate <crate> 2>&1 | grep -i "pass\|fail\|error\|ok\|FAILED" | awk '!seen[$0]++' | head -30

# If no crate specified (full workspace):
just test 2>&1 | grep -i "pass\|fail\|error\|ok\|FAILED" | awk '!seen[$0]++' | head -30
```

**STOP HERE if tests fail.** Report the failure and do not proceed.

### Step 2: Analyze Changes

If tests pass, analyze what was changed:

1. Run `git diff --name-only` to see modified files
2. Run `git diff` to understand the nature of changes
3. Categorize changes:
   - **Consensus changes**: VDF, producer selection, block validation, fork choice
   - **Protocol changes**: Wire format, message types, network behavior
   - **Economic changes**: Rewards, bonds, fees, slashing
   - **API changes**: RPC endpoints, CLI commands
   - **Architecture changes**: New crates, component interactions

### Step 3: Check Documentation Updates

Based on the changes, determine if documentation needs updating:

#### WHITEPAPER.md (Root Level)
Update if changes affect:
- Consensus mechanism (Sections 4, 7, 8)
- Transaction model (Section 2)
- Network protocol (Section 5)
- Producer registration/selection (Section 6, 7)
- Economic model: rewards, bonds, fees (Section 9)
- Security model (Section 11)
- Time structure: slots, epochs, eras (Section 4.2)

**Do NOT update WHITEPAPER.md for:**
- Internal refactoring
- Bug fixes that don't change behavior
- Test additions
- Performance optimizations
- Code style changes

#### specs/* Files
| File | Update When |
|------|-------------|
| `specs/protocol.md` | Wire format, message types, handshake changes |
| `specs/architecture.md` | New crates, component interactions, data flow |
| `specs/security_model.md` | New attack vectors, threat model changes |
| `specs/SPECS.md` | New spec files added (master index) |

#### docs/* Files
| File | Update When |
|------|-------------|
| `docs/running_a_node.md` | Node configuration, CLI flags |
| `docs/becoming_a_producer.md` | Registration, bonds, requirements |
| `docs/rpc_reference.md` | New/changed RPC endpoints |
| `docs/troubleshooting.md` | New error conditions |
| `docs/roadmap.md` | Milestone completion |
| `docs/attack_analysis.md` | Security findings |
| `docs/DOCS.md` | New documentation files added (master index) |

### Step 4: Apply Updates

If updates are needed:

1. **Read the target file(s)** first to understand current content
2. **Make minimal, precise edits** - only update affected sections
3. **Maintain consistency** with existing style and format
4. **Update version/date** if the file has one

### Step 5: Commit Changes

If documentation was updated OR code changes are ready:

```bash
# Stage specific files (never use git add -A)
git add <specific-files>

# Commit with author explicitly set (required for commits)
git commit --author="E. Weil <weil@doli.network>" -m "$(cat <<'EOF'
<type>(<scope>): <description>

<body explaining what changed>

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

**Git Author Configuration:**
- Author: `E. Weil <weil@doli.network>`
- Always use `--author` flag to ensure commits have correct attribution
- This is required even if global git config exists

**Commit types:**
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation only
- `refactor`: Code refactoring
- `test`: Test additions
- `chore`: Maintenance

### Step 6: Report Summary

Output a summary:
- Tests: PASSED/FAILED
- Whitepaper: UPDATED/NO CHANGE NEEDED
- Specs updated: list or "none"
- Docs updated: list or "none"
- Commit: hash or "not committed"

## Usage Examples

```
/sync-docs core      # Test core crate, sync docs, commit
/sync-docs vdf       # Test vdf crate, sync docs, commit
/sync-docs crypto    # Test crypto crate, sync docs, commit
/sync-docs           # Test all, sync docs, commit
```

## Decision Tree

```
Tests Pass?
├─ NO → Report failure, STOP
└─ YES → Analyze changes
         │
         ├─ Consensus/Protocol/Economic change?
         │  └─ YES → Update WHITEPAPER.md
         │
         ├─ Wire format/Architecture change?
         │  └─ YES → Update relevant specs/*
         │
         ├─ User-facing behavior change?
         │  └─ YES → Update relevant docs/*
         │
         └─ Commit all changes
```

## Important Rules

1. **Never update docs for internal-only changes** (refactoring, optimization)
2. **Always read files before editing** - understand context
3. **Minimal edits** - don't rewrite entire sections
4. **Test must pass first** - no documentation for broken code
5. **One commit** - bundle code + docs together
