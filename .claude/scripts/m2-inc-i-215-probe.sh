#!/usr/bin/env bash
# INC-I-215 M2 outcome probe.
#
# Question it answers, from OUTSIDE the test suite: when a node starts and its
# own install target is NOT writable (the sandboxed `doli service install` unit
# of INC-I-215) and no privileged helper unit watches the staging handoff, how
# many actionable startup diagnostics does the node emit about it?
#   0 = the node says nothing; the operator learns the updater is dead only when
#       the version-enforcement halt stops block production (pre-M2 behaviour)
#   1 = one startup line carrying UPDATE_TARGET_NOT_WRITABLE, the target path,
#       the reason and the fix command
#
# It runs the SHIPPED node preflight through `cargo run --example` against a
# real 0555 directory and a real (empty) systemd units directory.
# Re-runnable, no arguments, prints one integer on stdout.
set -uo pipefail

WT="/Users/isudoajl/ownCloud/Projects/doli-network/doli/.claude/worktrees/inc-i-215-staged-upgrade"
SCRATCH="$(mktemp -d)"
cleanup() { chmod -R u+w "$SCRATCH" >/dev/null 2>&1; rm -rf "$SCRATCH"; }
trap cleanup EXIT

mkdir -p "$SCRATCH/opt/bin" "$SCRATCH/data" "$SCRATCH/units"
: > "$SCRATCH/opt/bin/doli-node"
chmod 0555 "$SCRATCH/opt/bin"

OUT="$( cd "$WT" && cargo run -q -p doli-node --example inc_i_215_preflight_probe -- \
    "$SCRATCH/opt/bin/doli-node" "$SCRATCH/data" "$SCRATCH/units" 2>/dev/null )"

printf '%s\n' "$OUT" | grep -c 'UPDATE_TARGET_NOT_WRITABLE' | tr -d ' '
