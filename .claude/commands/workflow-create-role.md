---
name: workflow:create-role
description: Create a new agent role definition with automatic audit and remediation. Role-creator designs the role, role-auditor audits it adversarially, then findings are auto-fixed (max 2 cycles).
---

# Workflow: Create Role

Three-phase workflow: (1) `role-creator` designs the agent definition, (2) `role-auditor` runs a full adversarial audit, (3) corrections are applied automatically if needed.
Input: a description of the desired role (can be vague or detailed — the agent adapts).

## Phase 1: Role Creation

The role-creator follows this sequence:

1. **Analyze the request**
   - Read the user's description of the desired role
   - Glob `.claude/agents/*.md` to study existing agent patterns and detect potential overlap
   - Read `.claude/commands/*.md` to understand orchestration patterns
   - Read `CLAUDE.md` for workflow rules and constraints the new agent must respect

2. **Clarify (if needed)**
   - If the role description is vague, ask targeted questions about identity, boundaries, triggers, output, tools, and integration
   - If the description is detailed enough, proceed without unnecessary questions

3. **Research the domain**
   - Use WebSearch to research best practices, methodologies, and pitfalls for the role's domain
   - 2-4 targeted searches, not exhaustive research

4. **Design the role architecture**
   - Walk through the Role Anatomy Checklist (identity, boundaries, prerequisites, directory safety, source of truth, context management, process, output, rules, anti-patterns, failure handling, integration)
   - Perform overlap analysis against existing agents
   - Select minimal tools (least privilege)
   - Select appropriate model

5. **Write the agent definition**
   - Produce the complete `.claude/agents/[name].md` file
   - Follow the standard frontmatter format (name, description, tools, model)

6. **Validate**
   - Completeness check (all anatomy items addressed)
   - Consistency check (doesn't contradict CLAUDE.md or existing agents)
   - Clarity check (unambiguous to another LLM)
   - Boundary check (sharp enough to prevent scope creep)
   - Failure check (handles missing prerequisites, empty input, context exhaustion)

7. **Present and confirm**
   - Show the complete agent definition to the user
   - Explain key design decisions
   - Wait for explicit approval before saving to disk

8. **Save and create companion artifacts**
   - Save the approved agent definition to `.claude/agents/[name].md`
   - If applicable, create a companion command at `.claude/commands/workflow-[name].md`
   - Note any existing commands that should be updated to integrate the new agent

---

## Phase 2: Automatic Adversarial Audit

Once the role-creator saves the agent definition, **immediately** invoke the `role-auditor` subagent on the newly created role file. Do NOT ask the user — this is automatic.

9. **Invoke role-auditor**
   - Run a full D1-D12 adversarial audit on `.claude/agents/[name].md`
   - The auditor follows its standard process: pre-audit integrity check → context loading → dimensional audit → back-propagation → final report
   - Save the audit report to `docs/.workflow/role-audit-[name].md`

10. **Evaluate audit verdict**
    - If verdict is **deployable** or **hardened** with no major/critical findings → Phase 2 complete, report results to user
    - If verdict is **broken** or **degraded** → proceed to Phase 3 (remediation)

---

## Phase 3: Automatic Remediation

If the audit finds critical or major issues, **immediately** apply corrections. Do NOT ask the user — this is automatic.

11. **Apply corrections**
    - Read the audit report from `docs/.workflow/role-audit-[name].md`
    - For each CRITICAL finding: apply the fix directly to `.claude/agents/[name].md`
    - For each MAJOR finding: apply the fix directly to `.claude/agents/[name].md`
    - MINOR findings: apply fixes if straightforward, skip if cosmetic-only
    - Each fix must address the specific audit finding — no speculative changes

12. **Re-audit after remediation**
    - Invoke the `role-auditor` again on the corrected `.claude/agents/[name].md`
    - Save the new audit report (overwrites the previous one)
    - Maximum **2 remediation cycles** (audit → fix → re-audit → fix → final audit)
    - If still broken after 2 cycles → STOP and report remaining issues to the user for manual review

13. **Final report to user**
    - Summarize what was created, what the audit found, what was fixed, and the final verdict
    - If companion command was created, include that in the summary
    - Show the final audit verdict clearly

---

## Iteration Limits
- **Remediation cycles:** Maximum **2** (create → audit → fix → re-audit → fix → final audit)
- If the role is still **broken** after 2 cycles, stop and escalate to the user
- A role reaching **degraded** after remediation is acceptable — report the remaining minors to the user

## What the Workflow Produces
- A complete agent definition file (`.claude/agents/[name].md`)
- Optionally, a companion command file (`.claude/commands/workflow-[name].md`)
- An audit report at `docs/.workflow/role-audit-[name].md`
- Design rationale for key decisions (tools, model, boundaries)

## Quality Standards
Every role produced must have:
- Clear identity and purpose (first 3 lines tell you what it does)
- Sharp boundaries (what it does NOT do is explicit)
- Prerequisite gate (stops with clear error if upstream input is missing)
- Directory safety (creates directories before writing)
- Source of truth hierarchy (reads code before docs)
- Context management strategy (never reads entire codebase)
- Step-by-step process (phases with numbered steps)
- Output format template (predictable structure)
- Hard rules (non-negotiable constraints)
- Anti-patterns (explicit "don't do this" list)
- Failure handling (missing input, context limits, upstream failures)
- **Passed adversarial audit** with verdict of hardened or better
