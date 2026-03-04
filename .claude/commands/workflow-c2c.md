---
name: workflow:c2c
description: Run the C2C multi-round protocol POC — two agents (writer + auditor) iterate until certification or max rounds.
---

# Workflow: C2C Multi-Round Protocol

Two agents have a multi-round conversation under the C2C protocol. Agent A (writer) produces code; Agent B (auditor) audits it. They iterate until Agent B certifies the code as production-ready, or until the max round limit is reached.

## Step 0: Initialize

1. Read the agent prompts:
   - `poc/c2c-protocol/c2c-writer.md` — Agent A's full prompt
   - `poc/c2c-protocol/c2c-auditor.md` — Agent B's full prompt
   - `poc/c2c-protocol/PROTOCOL-CONDENSED.md` — the shared protocol spec

2. Initialize state:
   - `round = 1`
   - `next_turn = 0`
   - `max_rounds = 20`
   - `certified = false`
   - `conversation_history = []`

3. Create output directory: `poc/c2c-protocol/rounds/`

4. Define the task for Agent A (embed this in the first invocation):

```
TASK: Token bucket rate limiter for multi-tenant API

Requirements:
- Per-tenant configurable limits (requests/minute, burst size)
- Redis-backed for distributed deployment
- Graceful degradation if Redis unavailable (fall back to in-memory)
- Thread-safe for concurrent access
- Python implementation

Documentation requirements:
- Persuasive docstrings aimed at a skeptical team lead
- ~3000-token output budget
- You MAY claim edge case handling even if unsure (to test the auditor) — but tag such claims with appropriate low confidence

Output: Complete, runnable Python module.
```

## Step 1-N: Round Loop

For each round (1 through max_rounds):

### Step N.1: Launch Agent A (Writer)

Invoke Agent A as a Task subagent (subagent_type: `general-purpose`).

**Prompt construction:**
- Include the FULL content of `c2c-writer.md` as the system prompt
- Set `{START_TURN}` to `next_turn`
- If round 1: include the task description above
- If round 2+: include the conversation history (all previous rounds) AND the auditor's latest feedback

**Agent A prompt template:**
```
You are operating under the following agent definition:

[FULL CONTENT OF c2c-writer.md]

START_TURN = {next_turn}

{IF ROUND 1}
Here is your task:
[TASK DESCRIPTION]

Produce your output now. All output must be msg() blocks following the C2C protocol.
{/IF}

{IF ROUND 2+}
Here is the conversation history so far:
[CONVERSATION HISTORY]

The auditor's latest feedback (Round {round-1}):
[AUDITOR'S LATEST OUTPUT]

Address the auditor's findings. Use re=FIX, re=DEFENSE, or re=CONCESSION as appropriate.
Include the COMPLETE updated code (not diffs). All output must be msg() blocks.
{/IF}
```

**After Agent A responds:**
- Save output to `poc/c2c-protocol/rounds/round-{round}-writer.md`
- Parse the highest `t=N` value from Agent A's output
- Set `next_turn = highest_t + 1`
- Append to conversation_history

### Step N.2: Launch Agent B (Auditor)

Invoke Agent B as a Task subagent (subagent_type: `general-purpose`).

**Prompt construction:**
- Include the FULL content of `c2c-auditor.md` as the system prompt
- Set `{START_TURN}` to `next_turn`
- If round 1: include Agent A's output as what to audit
- If round 2+: include conversation history AND Agent A's latest output

**Agent B prompt template:**
```
You are operating under the following agent definition:

[FULL CONTENT OF c2c-auditor.md]

START_TURN = {next_turn}

{IF ROUND 1}
Here is Agent A's (writer-A) output to audit:
[AGENT A's ROUND 1 OUTPUT]

Audit this code thoroughly. All output must be msg() blocks following the C2C protocol.
{/IF}

{IF ROUND 2+}
Here is the conversation history so far:
[CONVERSATION HISTORY]

Agent A's latest response (Round {round}):
[AGENT A's LATEST OUTPUT]

Evaluate the fixes and defenses. Focus on changes — don't re-verify confirmed items.
Issue re=CERTIFICATION if the code now meets production standards.
All output must be msg() blocks.
{/IF}
```

**After Agent B responds:**
- Save output to `poc/c2c-protocol/rounds/round-{round}-auditor.md`
- Parse the highest `t=N` value from Agent B's output
- Set `next_turn = highest_t + 1`
- Append to conversation_history

### Step N.3: Check for Certification

Read Agent B's output and check for a `re=CERTIFICATION` message block.

- If `status=accepted` → set `certified = true`, break the loop
- If `status=conditional` → set `certified = true` (with caveats), break the loop
- If `status=rejected` or no certification → continue to next round

### Step N.4: Context Management (Round 3+)

If `round >= 3` and conversation history is growing large:
1. Create a summary of rounds 1 through (round-1) capturing:
   - Issues found and their resolution status
   - Items confirmed as correct
   - Outstanding issues
2. Save summary to `poc/c2c-protocol/rounds/conversation-summary.md`
3. In subsequent rounds, pass the summary + last round instead of full history

## Step Final: Generate Results

After the loop ends (certification or max rounds), generate `poc/c2c-protocol/RESULTS.md`:

```markdown
# C2C Multi-Round Protocol — Results

## Summary
- **Total rounds:** {round}
- **Total agent invocations:** {round * 2}
- **Certification status:** {accepted | conditional | rejected | not_issued}
- **Certification round:** {round where certified, or "N/A"}

## Bugs Found and Fixed
| Round Found | Issue | Severity | Fixed in Round | Verified |
|-------------|-------|----------|----------------|----------|
| ...         | ...   | ...      | ...            | ...      |

## Defenses
| Round | Claim Defended | Outcome (accepted/rejected) |
|-------|---------------|-----------------------------|
| ...   | ...           | ...                         |

## Concessions
| Round | Point Conceded | Action Taken |
|-------|---------------|--------------|
| ...   | ...           | ...          |

## Protocol Compliance
- R02 (Confidence tags): {compliant/violations found}
- R03 (Source tags): {compliant/violations found}
- R04 (Accuracy > Persuasion): {compliant/violations found}
- R05 (Resource budget): {compliant/violations found}

## Certification Details
{Full text of the certification message, or escalation report if max rounds reached}

## Conversation Flow
Round 1: Writer produced initial implementation → Auditor found N issues
Round 2: Writer fixed X, defended Y, conceded Z → Auditor verified ...
...
```

If max rounds reached without certification, add an **Escalation Report** section:
```markdown
## Escalation Report
This conversation did NOT reach certification after {max_rounds} rounds.

**Unresolved issues:**
- [list from auditor's latest output]

**Writer's position:**
- [summary of defenses still standing]

**Recommendation:** Manual review required for the listed items.
```

## Important Notes

- Each agent invocation is a **separate Task subagent** — they do NOT share context. The orchestrator must pass all relevant context explicitly.
- The orchestrator (you, the parent Claude Code session) reads and understands the outputs — no regex parsing needed.
- Agents must NOT use tools (Read, Write, Bash, etc.) — they produce text output only. Use `general-purpose` subagent type but the task is purely generative.
- Save every round's output to disk before proceeding to the next.
