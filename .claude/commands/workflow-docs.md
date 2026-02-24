---
name: workflow:docs
description: Generate/update specs and docs from the codebase. Accepts optional --scope to limit to a milestone/module.
---

# Workflow: Documentation

Invoke ONLY the `architect` subagent in documentation mode.
Optional: `--scope="milestone or module"` to document a specific area.

## Without scope (full documentation)
The architect MUST work in chunks to avoid context limits:
1. Read `specs/SPECS.md` to get the list of milestones/domains
2. For each milestone:
   a. Read the code files for that milestone
   b. Compare against existing specs
   c. Update stale specs, create missing ones
   d. Update related docs if needed
   e. Save progress to `docs/.workflow/docs-progress.md`
3. Update `specs/SPECS.md` master index
4. Update `docs/DOCS.md` master index
5. Generate summary of all changes
6. Clean up `docs/.workflow/` temporary files

## With scope (targeted documentation)
1. Read only the scoped code and its specs/docs
2. Update or create specs/docs for that area
3. Update master indexes if new files were created
