---
name: workflow:audit-role
description: Adversarial audit of an agent role definition. Audits across 12 dimensions at 2 levels. Assumes broken until proven safe. Accepts a role file path or audits all roles. Accepts optional --scope to limit to specific dimensions.
---

# Workflow: Audit Role

Invoke ONLY the `role-auditor` subagent to perform an adversarial audit of agent role definition(s).
Input: a path to a specific `.claude/agents/*.md` file, or "all" to audit every agent.
Optional: `--scope="dimensions"` to audit only specific dimensions.

## Single Role Audit (default)

The role-auditor runs a full D1-D12 adversarial audit on the specified role:

1. **Pre-audit integrity check**
   - Read the target role definition file completely
   - Verify YAML frontmatter is present and well-formed (name, description, tools, model)
   - Verify body content exists after frontmatter
   - If corrupted/incomplete → flag and determine if audit can proceed

2. **Context loading**
   - Read ALL existing agents in `.claude/agents/*.md` (needed for overlap detection)
   - Read `CLAUDE.md` for pipeline rules and conventions
   - This context is mandatory — overlap analysis is impossible without full pipeline knowledge

3. **Dimensional audit (D1-D12, sequential, no skipping)**
   - D1: Identity Integrity — clear, single-responsibility, non-contradictory
   - D2: Boundary Soundness — explicit boundaries, no overlap, scope creep resistance
   - D3: Prerequisite Gate Completeness — all inputs checked, stops on missing, content validation
   - D4: Process Determinism — specific steps, explicit decision points, no ambiguity
   - D5: Output Predictability — concrete template, save location, parseable by consumers
   - D6: Failure Mode Coverage — 5 common failures + role-specific failures handled
   - D7: Context Management Soundness — scoping strategy, checkpoints, actionable limits
   - D8: Rule Enforceability — every rule passes the observability test, no aspirational language
   - D9: Anti-Pattern Coverage — domain-specific, explains why, covers actual failure modes
   - D10: Tool & Permission Analysis — least privilege, no missing tools, no excess
   - D11: Integration & Pipeline Fit — upstream/downstream clear, handoff formats compatible
   - D12: Self-Audit — auditor's own limitations acknowledged
   - Output one `audit()` block per dimension

4. **Back-propagation**
   - After D12, re-read all earlier dimension verdicts
   - Revise any invalidated by later findings
   - Record revisions in `back_propagation`

5. **Final report**
   - Output `final_report()` with severity counts, anatomy checklist score, verdict, and deployment conditions
   - Save to `docs/.workflow/role-audit-[name].md`

## Audit All Roles

When input is "all" or no specific file is given:
1. Glob `.claude/agents/*.md` to find all agent definitions
2. Audit each role SEPARATELY with a full D1-D12 pass
3. Produce individual audit reports for each role
4. Produce a COMPARATIVE summary noting cross-role issues:
   - Overlapping responsibilities
   - Pipeline gaps (no agent covers area X)
   - Inconsistent conventions across agents
   - Tool privilege inconsistencies

## Severity Classification

- **CRITICAL**: Role will malfunction, silently degrades, overlaps without disambiguation, unbounded loops, privilege escalation
- **MAJOR**: Aspirational rules, missing failure handling, implicit boundaries, ambiguous decision points
- **MINOR**: Redundant rules, generic anti-patterns, suboptimal model selection

## Verdict Scale

- **broken**: Any critical finding OR 3+ major OR anatomy score < 8/14 → must not deploy
- **degraded**: No critical, 1-2 major, anatomy >= 8/14 → deploy with acknowledged limitations
- **hardened**: No critical, no major, minor only, anatomy >= 11/14 → solid, minor improvements possible
- **deployable**: No findings, anatomy = 14/14 → meets all quality standards (rare)

## Scope Parameter

The `--scope` parameter limits which dimensions are audited:

```bash
# By dimension number
/workflow:audit-role ".claude/agents/analyst.md" --scope="D1-D3"
/workflow:audit-role ".claude/agents/analyst.md" --scope="D6"
/workflow:audit-role ".claude/agents/analyst.md" --scope="D1-D3,D8,D10"

# By dimension name
/workflow:audit-role ".claude/agents/analyst.md" --scope="boundaries,tools,rules"

# Dimension name mapping
# identity=D1, boundaries=D2, prerequisites=D3, process=D4,
# output=D5, failures=D6, context=D7, rules=D8,
# antipatterns=D9, tools=D10, integration=D11, self=D12
```

When scoped:
- Only specified dimensions are audited
- D12 (self-audit) is ALWAYS included regardless of scope
- Scoped audits CANNOT produce "deployable" verdict — full D1-D12 required
- Cross-dimension findings outside scope are noted but not fully audited

When no scope is provided:
- All D1-D12 are audited. No exceptions.

## What the Auditor Does NOT Do
- Does not MODIFY role definitions (read-only)
- Does not audit code, tests, or runtime behavior
- Does not fix findings — that's the role-creator's job
- Does not audit non-role files (specs, docs, source code)
