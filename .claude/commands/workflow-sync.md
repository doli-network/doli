---
name: workflow:sync
description: Detect and fix drift between codebase and specs/docs. Accepts optional --scope to limit to a milestone/module.
---

# Workflow: Sync

Invoke ONLY the `architect` subagent in sync mode.
Optional: `--scope="milestone or module"` to sync a specific area.

## Without scope (full sync)
The architect MUST work in chunks to avoid context limits:
1. Read `specs/SPECS.md` to get the full list of milestones/domains
2. For each milestone:
   a. Read the code for that milestone
   b. Read corresponding spec files
   c. Read corresponding doc files
   d. Log drift findings to `docs/.workflow/sync-[milestone].md`
   e. Fix drift found
3. Compile all findings into the final drift report
4. Update both master indexes
5. Save report to `docs/sync/sync-[date].md`
6. Clean up `docs/.workflow/sync-*.md` temporary files

## With scope (targeted sync)
1. Read only the scoped code, specs, and docs
2. Find and fix drift for that area
3. Update master indexes if files were added/removed
4. Save report to `docs/sync/sync-[scope]-[date].md`

## Drift Report Format
```markdown
# Sync Report — [date]

## Scope
[Full project / specific milestone or module]

## Specs Drift
| Spec File | Status | Issue |
|-----------|--------|-------|
| specs/x.md | ⚠️ Stale | Function Y was renamed to Z in code |
| specs/y.md | ❌ Missing | Module Y has no spec |
| specs/z.md | 🗑️ Orphan | References deleted code |

## Docs Drift
| Doc File | Status | Issue |
|----------|--------|-------|
| docs/x.md | ⚠️ Stale | Config option removed |
| docs/y.md | ❌ Missing | New feature undocumented |

## Index Drift
| Index | Issue |
|-------|-------|
| SPECS.md | Missing entry for specs/new-module.md |
| DOCS.md | Links to deleted docs/old-feature.md |

## Actions Taken
- [What was fixed]

## Remaining Issues (if any)
- [What couldn't be fixed automatically]
```
