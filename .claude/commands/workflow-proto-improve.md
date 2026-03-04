---
name: workflow:proto-improve
description: Improve a protocol specification based on PROTO-AUDITOR findings. Generates structured patches via PROTO-ARCHITECT. Accepts protocol + audit report paths and optional --scope to limit to specific findings.
---

# Workflow: Protocol Improvement (PROTO-ARCHITECT)

Generate structured improvement patches for a protocol specification based on audit findings from PROTO-AUDITOR. Uses the `proto-architect` subagent.

**Input:** Protocol file path(s) + audit report path. If no audit report path provided, look for the latest in `c2c-protocol/audits/`.
**Optional:** `--scope="D1,D2,D6"` to patch only specific dimension findings, or `--scope="critical"` to patch only CRITICAL severity findings.

## Step 0: Locate Files

1. Parse the user's input for protocol file paths, audit report path, and `--scope` parameter
2. If no audit report path provided:
   - Search `c2c-protocol/audits/` for the most recent audit report
   - If none found, STOP and instruct the user to run `/workflow:proto-audit` first
3. Read all protocol files completely
4. Read the audit report completely
5. Verify the audit report contains valid `audit()` blocks with finding IDs
6. Create output directory: `c2c-protocol/patches/` (if it doesn't exist)

## Step 1: Launch PROTO-ARCHITECT

Invoke the `proto-architect` subagent (subagent_type: `general-purpose`, model: `opus`).

**Prompt construction:**
- Include the FULL content of the `proto-architect` agent definition as the system identity
- Include the COMPLETE text of all protocol files
- Include the COMPLETE audit report

**Prompt template:**
```
You are operating under the following identity:

[FULL CONTENT OF proto-architect agent]

=== PROTOCOL SPECIFICATION ===

[COMPLETE TEXT OF PROTOCOL FILE(S)]

=== AUDIT REPORT (from PROTO-AUDITOR) ===

[COMPLETE TEXT OF AUDIT REPORT]

=== INSTRUCTIONS ===

IMPROVE: [protocol_name_and_version] AUDIT: [audit_report_reference]

Execute the full improvement pipeline:
1. P1: Triage incoming audit report
2. P2: Root cause isolation
3. P3: Patch generation
4. P4: Patch self-audit
5. P5: Version increment
6. P6: Regression check

Do not skip steps. Do not merge steps.
Output each step block before proceeding to the next.
Output structured patch() blocks only — no prose outside rule_text fields.

{IF SCOPE PROVIDED}
SCOPE RESTRICTION: Only process findings matching: [SCOPE]
Skip all other findings but list them as "out of scope — not patched" in the triage output.
{/IF}
```

## Step 2: Save Patch Report

1. Save the complete patch output to `c2c-protocol/patches/patches-[protocol-name]-[date].md`
2. Display a summary to the user:
   - Findings triaged (count by classification)
   - Patches generated (count by type: amend/extend/add/deprecate/axiom/define)
   - Patches rejected in self-audit (count + reasons)
   - Proposed version bump
   - Regressions found (if any)
   - Patch quality tier distribution

## Step 3: Operator Decision Points

Flag any items requiring operator decision:
- Structural changes requiring approval (from P2)
- CRITICAL findings closed with TIER 3 patches (from P3)
- Conflicting patches (from Rules of Engagement #4)
- Regressions requiring expanded patches (from P6)

Present these as a numbered list for the user to resolve.

## Step 4: Suggest Next Steps

After patches are generated, suggest:
```
To validate the patches, re-audit the patched protocol:
/workflow:proto-audit

To apply patches to the protocol, review the patch report at:
c2c-protocol/patches/patches-[protocol-name]-[date].md
```

## Important Notes

- PROTO-ARCHITECT REFUSES input without audit references — do not bypass
- The agent produces TEXT OUTPUT ONLY — structured patch() blocks
- The agent does NOT modify protocol files directly — patches are proposals
- Patch batches are atomic: all pass or none apply
- Operator must review structural changes and TIER 3 patches on CRITICAL findings
- After applying patches, always re-audit with `/workflow:proto-audit` to verify
