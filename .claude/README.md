# Claude Code Quality Workflow

A multi-agent orchestration system for [Claude Code](https://docs.anthropic.com/en/docs/claude-code) that produces high-quality code through structured validation layers. Instead of asking an AI to "build X" and hoping for the best, this workflow forces every piece of code through questioning, architecture design, test-driven development, implementation, and review — each handled by a specialized agent with its own context window.

## The Problem

When you ask an AI to write code directly, it:
- **Assumes things** instead of asking — leading to silent bugs
- **Writes tests after code** — biasing tests toward what was built, not what should be built
- **Skips architecture** — jumping straight to implementation without thinking through design
- **Ignores context** — not reading existing code conventions, patterns, or documentation
- **Lets documentation rot** — specs and docs drift out of sync with the actual codebase

This workflow solves all of that.

## How It Works

Five specialized agents execute in chain, each with a single responsibility:

```
Your Idea
  ↓
🔍 Analyst       → Questions your idea, reads existing code, eliminates ambiguity
  ↓
🏗️ Architect     → Designs the architecture, updates specs/ and docs/
  ↓
🧪 Test Writer   → Writes tests BEFORE code exists (TDD)
  ↓
💻 Developer     → Implements module by module until all tests pass
  ↓
🔨 Compiler      → Automatic validation (Rust recommended)
  ↓
👁️ Reviewer      → Audits for bugs, security, performance, and documentation drift
  ↓
📦 Git           → Conventional commits and versioning
```

Each agent runs as a Claude Code subagent with its own isolated context window. The analyst's heavy reading doesn't eat into the developer's context. Work is scoped, incremental, and saved to disk at every step.

## Source of Truth

```
Codebase  →  specs/  →  docs/
(ultimate)   (technical)  (user-facing)
```

The codebase always wins. When specs or docs are outdated, agents flag the discrepancy and fix it. Every agent reads the actual code before trusting any documentation.

## Agents

### 🔍 Analyst (`analyst.md`)
**Model:** Opus | **Tools:** Read, Grep, Glob, WebFetch, WebSearch

The gatekeeper. Reads `specs/SPECS.md` to understand the project, scopes to the relevant area, reads the actual code, then questions everything that isn't clear. Never assumes — always asks. Generates a requirements document with explicit assumptions in both technical and plain language formats. Flags any drift between code and specs.

**Output:** `specs/[domain]-requirements.md`

### 🏗️ Architect (`architect.md`)
**Model:** Opus | **Tools:** Read, Write, Edit, Grep, Glob

The designer. Takes the analyst's requirements and designs the system architecture before any code is written. Defines modules, interfaces, dependencies, and implementation order. Creates and updates spec files in `specs/` and documentation in `docs/`. Maintains both master indexes (`SPECS.md` and `DOCS.md`).

Also handles `/workflow:docs` and `/workflow:sync` — reading the codebase and bringing specs/docs back in sync.

**Output:** `specs/[domain]-architecture.md`, updated specs and docs

### 🧪 Test Writer (`test-writer.md`)
**Model:** Sonnet | **Tools:** Read, Write, Edit, Bash, Glob, Grep

The contract writer. Writes all tests BEFORE any implementation exists. Defines what the code must do through tests — happy paths, error handling, and edge cases. Follows existing test conventions found in the codebase. Works one module at a time, saving to disk after each.

Always considers the 10 worst scenarios: empty input, negative numbers, overflow, unicode, concurrency, disk full, network failure, huge input, inconsistent data, and interrupted operations.

**Output:** Test files that must fail initially (red phase of TDD)

### 💻 Developer (`developer.md`)
**Model:** Sonnet | **Tools:** Read, Write, Edit, Bash, Glob, Grep

The builder. Implements the minimum code needed to pass all tests, one module at a time in the order defined by the architect. Matches existing code conventions by grepping the codebase. Never advances to the next module until the current one's tests all pass. Commits after each module.

**Cycle:** Red → Green → Refactor → Commit → Next

### 👁️ Reviewer (`reviewer.md`)
**Model:** Opus | **Tools:** Read, Grep, Glob (read-only)

The auditor. Reviews all implemented code looking for bugs, security vulnerabilities, performance issues, technical debt, and specs/docs drift. Uses Grep for cross-cutting scans (`unwrap()`, `unsafe`, `TODO`, `HACK`). Works module by module, saving findings incrementally. Brutally honest — doesn't approve out of courtesy.

**Output:** Review report with critical/minor findings, specs drift, and final verdict

## Commands

| Command | Description | Agents Used |
|---------|-------------|-------------|
| `/workflow:new "idea"` | Build something from scratch | All 5 in chain |
| `/workflow:feature "feature"` | Add to existing project | All 5 in chain |
| `/workflow:improve "improvement"` | Refactor, optimize, or enhance existing code | analyst → test-writer → developer → reviewer |
| `/workflow:bugfix "bug"` | Fix a bug | analyst → test-writer → developer → reviewer |
| `/workflow:audit` | Full code + specs audit | Reviewer only |
| `/workflow:docs` | Generate/update specs & docs | Architect only |
| `/workflow:sync` | Fix drift between code and specs/docs | Architect only |

### Scope Parameter

All commands accept `--scope` to limit context usage on large codebases:

```bash
/workflow:feature "add retry logic" --scope="network"
/workflow:audit --scope="core"
/workflow:sync --scope="storage"
/workflow:bugfix "scheduler crash" --scope="crates/core/src/scheduler.rs"
```

When no scope is provided, the analyst determines the minimal scope needed.

## Context Window Management

This workflow is designed for real-world codebases that exceed a single context window. Every agent follows these rules:

- **Read indexes first** — `specs/SPECS.md` gives the project layout without reading every file
- **Grep before Read** — search for symbols and patterns before loading whole files
- **Work one module at a time** — never load everything into context simultaneously
- **Save to disk incrementally** — tests, code, and findings are written to files after each module
- **Checkpoint on large operations** — audit, docs, and sync process one milestone at a time with progress saved to `docs/.workflow/`
- **Never silently degrade** — if an agent can't finish, it states exactly what was skipped and recommends a scoped follow-up
- **Clean up** — temporary `docs/.workflow/` files are removed after workflow completion

## Installation

### Quick Install (existing project)

```bash
# Clone the workflow repo
git clone <repo-url> claude-workflow

# Copy agents and commands into your project
mkdir -p .claude/agents .claude/commands
cp claude-workflow/.claude/agents/*.md .claude/agents/
cp claude-workflow/.claude/commands/*.md .claude/commands/
cp claude-workflow/CLAUDE.md ./CLAUDE.md
```

### Setup Script (new project)

```bash
git clone <repo-url> claude-workflow
cd my-project
bash ../claude-workflow/scripts/setup.sh
```

The setup script creates `specs/SPECS.md` and `docs/DOCS.md` if they don't exist, and never overwrites existing files.

## Project Structure

The workflow expects (and creates if missing) this structure:

```
your-project/
├── CLAUDE.md                  ← Workflow rules (read by Claude Code on startup)
├── specs/
│   ├── SPECS.md               ← Master index of all technical specs
│   ├── domain-a.md            ← Per-domain spec files
│   └── domain-b.md
├── docs/
│   ├── DOCS.md                ← Master index of all documentation
│   ├── quickstart.md          ← Topic-oriented guides
│   ├── architecture.md
│   ├── .workflow/             ← Temporary agent checkpoints (auto-cleaned)
│   ├── reviews/               ← Code review reports
│   ├── audits/                ← Audit reports
│   └── sync/                  ← Sync/drift reports
├── .claude/
│   ├── README.md              ← This file
│   ├── agents/                ← Subagent definitions
│   │   ├── analyst.md
│   │   ├── architect.md
│   │   ├── test-writer.md
│   │   ├── developer.md
│   │   └── reviewer.md
│   └── commands/              ← Slash commands
│       ├── workflow-new.md
│       ├── workflow-feature.md
│       ├── workflow-improve.md
│       ├── workflow-bugfix.md
│       ├── workflow-audit.md
│       ├── workflow-docs.md
│       └── workflow-sync.md
└── .gitignore
```

## Customization

### Add Custom Agents
Create a `.md` file in `.claude/agents/` with the frontmatter format:
```yaml
---
name: your-agent
description: When to invoke this agent
tools: Read, Write, Edit, Bash, Glob, Grep
model: sonnet
---

Your agent instructions here...
```

### Modify Workflows
Edit commands in `.claude/commands/` to change agent chain order, add steps, or create new workflow modes.

### Integrate With Existing CLAUDE.md
If your project already has a `CLAUDE.md`, merge the workflow rules — specifically the Implicit Workflow Routing section and the Source of Truth Hierarchy.

## Workflow Details

### `/workflow:new` — Full Pipeline

```
Step 1: Analyst    → reads specs index, scopes, questions user, generates requirements
Step 2: Architect  → reads scoped code, designs architecture, updates specs/ and docs/
Step 3: Test Writer→ writes failing tests for each module (TDD red phase)
Step 4: Developer  → implements module by module until green, commits each
Step 5: Compiler   → cargo build + clippy + fmt + test (automatic gate)
Step 6: Reviewer   → audits code + specs drift, approves or sends back
Step 7: Iteration  → developer fixes → reviewer re-reviews (scoped to fix only)
Step 8: Versioning → final commit, version tag, cleanup temp files
```

### `/workflow:feature` — Same as New, Context-Aware

Same pipeline but every agent reads existing code first. The analyst checks for specs drift. The test-writer matches existing test conventions. All previous tests must continue passing (regression).

### `/workflow:improve` — Refactor and Optimize

```
Step 1: Analyst    → reads current code, identifies what to improve (no new requirements)
Step 2: Test Writer→ writes regression tests to lock in existing behavior
Step 3: Developer  → refactors/optimizes, all tests must still pass
Step 4: Reviewer   → verifies improvement is real, no behavior changes slipped in
```

Skips the architect since the architecture already exists. The analyst focuses on code quality, performance, and patterns rather than questioning new requirements. Behavior stays the same — only the implementation gets better.

### `/workflow:bugfix` — Reduced Chain

```
Step 1: Analyst    → locates bug in code (Grep), reads affected area only
Step 2: Test Writer→ writes a test that reproduces the bug (must fail)
Step 3: Developer  → fixes bug, reproduction test passes, no regression
Step 4: Reviewer   → verifies root cause fix (not a patch), checks specs
```

### `/workflow:audit` — Read-Only Analysis

Reviewer scans the codebase looking for security issues, performance problems, technical debt, dead code, missing tests, and documentation drift. On large codebases, works one milestone at a time with checkpoints. Produces a comprehensive report.

### `/workflow:docs` — Documentation Generation

Architect reads the codebase (source of truth) and creates or updates specs and docs to match reality. Works one milestone at a time on large projects.

### `/workflow:sync` — Drift Detection and Fix

Architect compares every spec and doc file against the actual code. Produces a drift report showing stale specs, missing specs, orphaned docs, and index gaps. Then fixes everything found.

## Philosophy

> "The best code is the one that went through multiple layers of questioning before it existed."

This workflow exists because:
- Without constraints, AI assumes things and generates silent bugs
- Tests written after code are biased toward what was built, not what should be built
- A strict compiler (Rust) compensates for AI weaknesses in ways a dynamic language can't
- Code review by a separate instance catches what the original missed
- Documenting before coding forces clarity of thought
- Specs and docs drift silently — automated sync catches it before it becomes a liability
- Context limits are real — scoping and chunking prevent quality degradation on large codebases

## License

MIT
