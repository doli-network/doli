#!/usr/bin/env bash
# INC-I-215 M5 outcome probe.
#
# Question it answers, from OUTSIDE the test suite: of the eleven shipped
# documentation surfaces an operator or an agent actually reads (docs/, specs/,
# .claude/skills/, CLAUDE.md), how many describe the staged upgrade handoff at
# all — i.e. name the root entry point `--from-staged` or the preflight token
# `UPDATE_TARGET_NOT_WRITABLE`?
#
# 0  = the shipped docs still describe the pre-INC-I-215 world: an operator whose
#      node logs "Read-only file system (os error 30)" finds nothing in cli.md,
#      troubleshooting.md, releases.md or any skill, and the node stays dead.
# 11 = every surface in the §4.4 drift table documents the handoff.
#
# SAFETY: read-only. Greps files. Touches no node, no chain, no network.
#
# Re-runnable, no arguments, prints one integer 0..11 on stdout.
set -uo pipefail

WT="/Users/isudoajl/ownCloud/Projects/doli-network/doli/.claude/worktrees/inc-i-215-staged-upgrade"

FILES="
docs/cli.md
docs/troubleshooting.md
docs/releases.md
docs/producer_node_quickstart.md
docs/architecture.md
specs/security_model.md
.claude/skills/auto-update/SKILL.md
.claude/skills/updater/SKILL.md
.claude/skills/release/SKILL.md
.claude/skills/SKILLS-INDEX.md
CLAUDE.md
"

COUNT=0
for f in $FILES; do
  p="$WT/$f"
  [ -f "$p" ] || continue
  if grep -q -e '--from-staged' -e 'UPDATE_TARGET_NOT_WRITABLE' "$p"; then
    COUNT=$((COUNT + 1))
  fi
done

printf '%d\n' "$COUNT"
