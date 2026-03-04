---
name: workflow:understand
description: Deep comprehension of a codebase — architecture, domain logic, data flows, patterns, and risk areas. Accepts optional --scope to limit to a module/area.
---

# Workflow: Understand

Invoke ONLY the `codebase-expert` subagent to build a deep understanding of the project.
Optional: `--scope="module or area"` to focus on a specific part.

## Without scope (full understanding)
The codebase-expert MUST work in progressive layers to handle any codebase size:

1. **Layer 1 — Project Shape**
   - Glob the directory tree for top-level structure
   - Read config files (Cargo.toml, package.json, go.mod, etc.) for tech stack
   - Read README and specs/docs indexes if they exist
   - Identify languages, frameworks, build systems

2. **Layer 2 — Architecture & Boundaries**
   - Read entry points (main.*, index.*, app.*)
   - Trace application bootstrap and wiring
   - Identify module boundaries and dependency direction
   - Save progress to `docs/.workflow/expert-structure.md`

3. **Layer 3 — Domain & Business Logic**
   - Read core domain models and entities
   - Understand main business workflows
   - Identify domain-specific patterns (DDD, event sourcing, etc.)

4. **Layer 4 — Data Flow & State**
   - Trace data from entry to storage to exit
   - Map state management (databases, caches, queues)
   - Understand configuration flow
   - Save progress to `docs/.workflow/expert-domain.md`

5. **Layer 5 — Patterns & Conventions**
   - Identify coding conventions (naming, error handling, testing)
   - Map architectural patterns (middleware, plugins, DI, etc.)
   - Find the "template" for adding new features
   - Note convention breaks

6. **Layer 6 — Complexity & Risk Map**
   - Identify high-complexity areas
   - Map security-sensitive and performance-sensitive paths
   - Catalog technical debt
   - Save progress to `docs/.workflow/expert-patterns.md`

7. **Compile** all layer findings into `docs/understanding/PROJECT-UNDERSTANDING.md`
8. **Clean up** `docs/.workflow/expert-*.md` temporary files

## With scope (targeted understanding)
The codebase-expert focuses on the specified area:
1. Quick Layer 1 (project shape — needed for context)
2. Layers 2-6 focused exclusively on the scoped area
3. Note external dependencies and what depends on the scoped area
4. Save to `docs/understanding/[scope]-understanding.md`

## Understanding Covers
- Tech stack and framework identification
- Architecture and module boundaries
- Domain model and core business logic
- Data flows (entry → processing → storage → exit)
- Coding conventions and architectural patterns
- The "template" for adding new features
- High-complexity and high-risk areas
- Security-sensitive and performance-sensitive paths
- Technical debt and convention breaks
- Key files for onboarding
- Specs/docs drift detection
