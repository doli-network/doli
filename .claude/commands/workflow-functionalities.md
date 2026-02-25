---
name: workflow:functionalities
description: Analyze the codebase and produce a structured inventory of all functionalities. Accepts optional --scope to limit to a module/area.
---

# Workflow: Functionalities

Invoke ONLY the `functionality-analyst` subagent to map what the codebase does.
Optional: `--scope="module or area"` to analyze a specific part.

## Without scope (full analysis)
The functionality-analyst MUST work in chunks to avoid context limits:
1. Use Glob to map the project directory structure
2. Identify top-level modules/domains from the directory layout
3. For each module:
   a. Discover all functionalities (endpoints, services, models, handlers, etc.)
   b. Record findings with file paths and line numbers
   c. Save progress to `docs/.workflow/functionality-analyst-progress.md`
   d. Clear mental context before next module
4. Detect cross-module dependencies
5. Flag dead code and unused exports
6. Compile all module findings into per-domain files at `docs/functionalities/[domain]-functionalities.md`
7. Generate master index at `docs/functionalities/FUNCTIONALITIES.md`
8. Clean up `docs/.workflow/functionality-analyst-*.md` temporary files

## With scope (targeted analysis)
The functionality-analyst works only within the specified area:
1. Map structure of the scoped area only
2. Discover and catalog all functionalities in that area
3. Note dependencies on modules outside the scope (but don't analyze them)
4. Save to `docs/functionalities/[scope]-functionalities.md`
5. Update `docs/functionalities/FUNCTIONALITIES.md` if it exists

## Analysis Covers
- Public APIs and endpoints (REST, GraphQL, gRPC, WebSocket)
- Services and business logic
- Data models and schemas
- CLI commands and subcommands
- Event handlers and listeners
- Scheduled tasks and cron jobs
- Middleware and interceptors
- Integrations with external services
- Background workers and queues
- Dead code and unused exports
- Cross-module dependency chains
