---
name: workflow:proto-audit
description: Audit a protocol specification using PROTO-AUDITOR v2.0. Runs 12 dimensions across 3 levels. Accepts protocol file paths and optional --scope to limit to specific dimensions.
---

# Workflow: Protocol Audit (PROTO-AUDITOR v2.0)

Audit a protocol specification across 12 dimensions at 3 levels (protocol, enforcement, self-audit). Uses the `proto-auditor` subagent in full adversarial mode.

**Input:** Protocol file path(s). If an enforcement layer file exists, include it for L2 analysis.
**Optional:** `--scope="D1,D2,D6"` to audit only specific dimensions, or `--scope="L1"` to audit only at a specific level.

## Step 0: Locate Protocol Files

1. Parse the user's input for protocol file paths and `--scope` parameter
2. If no paths provided, search `c2c-protocol/` for protocol and enforcement layer files
3. Read all protocol files to verify they are complete and parseable
4. If `--scope` is provided, validate dimension/level references (D1-D12, L1-L3)
5. Create output directory: `c2c-protocol/audits/` (if it doesn't exist)

## Step 1: Launch PROTO-AUDITOR

Invoke the `proto-auditor` subagent (subagent_type: `general-purpose`, model: `opus`).

**Prompt construction:**
- Include the FULL content of the `proto-auditor` agent definition as the system identity
- Include the COMPLETE text of all protocol files being audited
- If an enforcement layer file is provided, include it and instruct L2 analysis
- If no enforcement layer, instruct the auditor to skip L2 and flag `enforcement_gap`

**Prompt template:**
```
You are operating under the following identity:

[FULL CONTENT OF proto-auditor agent]

=== PROTOCOL SPECIFICATION TO AUDIT ===

[COMPLETE TEXT OF PROTOCOL FILE(S)]

=== ENFORCEMENT LAYER (if present) ===

[COMPLETE TEXT OF ENFORCEMENT LAYER FILE, or "NOT PROVIDED — skip L2 checks, flag enforcement_gap in every dimension"]

=== INSTRUCTIONS ===

Execute the full audit:
1. Verify document integrity
2. Run D1 through D12 sequentially at all applicable levels
3. Output one audit() block per dimension
4. After D12, run back-propagation check
5. Output final_report() with cross-references

Do not skip dimensions. Do not merge dimensions.
Output structured audit() blocks only — no prose outside finding fields.

{IF SCOPE PROVIDED}
SCOPE RESTRICTION: Only audit the following dimensions/levels: [SCOPE]
Skip all other dimensions but note them as "skipped — out of scope" in the final report.
{/IF}
```

## Step 2: Save Audit Report

1. Save the complete audit output to `c2c-protocol/audits/audit-[protocol-name]-[date].md`
2. Display a summary to the user:
   - Total findings by severity (CRITICAL / MAJOR / MINOR)
   - Cross-layer findings count
   - Overall verdict
   - Top 3 most critical findings

## Step 3: Suggest Next Steps

If findings were found, suggest:
```
To generate improvement patches based on this audit, run:
/workflow:proto-improve
```

## Important Notes

- The proto-auditor agent produces TEXT OUTPUT ONLY — structured audit() and final_report() blocks
- The agent does NOT modify any files — the orchestrator saves the output
- Each audit is a separate subagent invocation with full protocol context
- For large protocols, ensure the full text is passed — do not summarize or truncate
