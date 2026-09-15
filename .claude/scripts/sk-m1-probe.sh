#!/bin/bash
# SK-M1 outcome-metric probe.
#
# METRIC: the filesystem directory that `sudo doli upgrade` / the node auto-updater
#         actually writes agent skills into, when the process HOME is root-like and the
#         invoking operator is named by SUDO_USER.
#
# Re-runnable, externally observable: it runs the real public installer against a
# synthetic release tarball and then reports which directory on disk received the file.
#
# Usage: bash .claude/scripts/sk-m1-probe.sh
set -uo pipefail

WT="/Users/isudoajl/ownCloud/Projects/doli-network/doli/.claude/worktrees/skills-path"
cd "$WT" || exit 1

OPERATOR="$(id -un)"
PROCESS_HOME="$(mktemp -d /tmp/sk-m1-process-home.XXXXXX)"
CAND_PROCESS="$PROCESS_HOME/.doli/skills"
CAND_OPERATOR="$(eval echo ~"$OPERATOR")/.doli/skills"

# Build first with the NORMAL environment so cargo's own HOME-derived paths are intact,
# then run the test binary under the sudo-shaped environment. Setting HOME for cargo
# itself would move the registry and rebuild the world.
# The probe lives in the consolidated `it` test binary (one binary per crate — see
# .claude/hooks/test-binary-gate.sh), so build `--test it` and filter by test name.
# Ask cargo for the exact executable path. `ls deps/it-*` is NOT safe here: doli-cli also
# consolidates into a test binary named `it`, so a glob can hand back the wrong crate's.
BIN="$(cargo test -p updater --test it --no-run --message-format=json 2>/dev/null \
       | python3 -c 'import sys,json
for line in sys.stdin:
    try: m = json.loads(line)
    except Exception: continue
    if m.get("target", {}).get("name") == "it" and m.get("executable"):
        print(m["executable"])' | tail -1)"
if [ -z "$BIN" ]; then
  echo "SK-M1-RESOLVED=<probe-binary-not-built>"
  exit 1
fi

env HOME="$PROCESS_HOME" SUDO_USER="$OPERATOR" SUDO_UID="$(id -u)" SUDO_GID="$(id -g)" \
    SK_M1_PROBE=1 "$BIN" sk_m1_probe_install_under_sudo_shaped_env \
        --nocapture --test-threads=1 2>&1 | grep -o 'SK-M1-.*'

if [ -f "$CAND_PROCESS/sk-m1-probe/SKILL.md" ]; then
  echo "SK-M1-RESOLVED=$CAND_PROCESS"
elif [ -f "$CAND_OPERATOR/sk-m1-probe/SKILL.md" ]; then
  echo "SK-M1-RESOLVED=$CAND_OPERATOR"
else
  echo "SK-M1-RESOLVED=<nowhere>"
fi
rm -rf "$PROCESS_HOME"
