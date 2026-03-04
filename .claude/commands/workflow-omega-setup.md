---
name: workflow:omega-setup
description: Design and configure OMEGA infrastructure for a business domain. Maps user goals to OMEGA primitives (projects, skills, topologies, schedules, heartbeats).
---

# Workflow: OMEGA Setup

The user wants to configure OMEGA for a new business domain or restructure an existing setup.
Input: a description of what the user wants OMEGA to help with (can be vague or detailed — the agent adapts).

## Single Agent Workflow

This workflow invokes the `omega-topology-architect` subagent, which handles the full lifecycle:

1. **Understand** the user's business domain through targeted questions (if needed)
2. **Discover** existing OMEGA infrastructure to avoid duplication
3. **Map** the domain to OMEGA primitives (projects, skills, topologies, schedules, heartbeats)
4. **Design** a concrete proposal with minimum viable setup
5. **Present** the proposal and wait for human approval
6. **Execute** the approved setup (create files, configure schedules)

## Invocation

Invoke the `omega-topology-architect` subagent with the user's business domain description and any `--scope` parameter.

The agent reads from `~/.omega/` to understand existing state and writes configuration files to `~/.omega/projects/`, `~/.omega/topologies/`, and `~/.omega/skills/` after human approval.

## Fail-Safe Controls

### Human Approval Gate
The agent MUST present a structured proposal and receive explicit user approval before creating any files. This is non-negotiable.

### Progress Recovery
If the conversation is interrupted or context limits are reached, the agent saves progress to `docs/.workflow/topology-architect-progress.md`. The user can resume by re-invoking this command — the agent will read the progress file and continue from where it left off.

### Iteration Limits
- **Clarification rounds:** Maximum 2. If the domain is still unclear after 2 rounds, the agent proceeds with what it knows and flags uncertainties.
- **Proposal revisions:** No hard limit — the user can iterate on the proposal as many times as needed before approval.

## What It Produces

- `~/.omega/projects/<name>/ROLE.md` — project role definition with domain expertise
- `~/.omega/projects/<name>/HEARTBEAT.md` — heartbeat monitoring items (if applicable)
- `~/.omega/topologies/<name>/TOPOLOGY.toml` — custom topology (only if standard development topology doesn't fit)
- `~/.omega/skills/<name>/SKILL.md` — new skill definitions (only if needed)
- Scheduling markers for the system to process (SCHEDULE_ACTION, SCHEDULE_REMINDER)
