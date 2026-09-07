#!/usr/bin/env bash
# INC-I-215 M4 outcome probe.
#
# Question it answers, from OUTSIDE the test suite: for a real host layout
# (network=mainnet, node unit doli-mainnet, data dir /var/lib/doli/mainnet, CLI
# /usr/bin/doli), how many of the three post-conditions of the ROOT HELPER PAIR that
# `doli service install` must render does the shipped code actually satisfy?
#   1 = the .path unit watches EXACTLY the marker string the node stages, and names the
#       .service unit — this is the literal M2's startup preflight scans
#       /etc/systemd/system/*.path for, so anything else means the node still WARNs
#   2 = the .service unit's ExecStart is EXACTLY M3's --from-staged invocation
#       (--network is a global flag, so it precedes the subcommand)
#   3 = the .service unit is a root oneshot with NONE of the node unit's sandbox
#       directives — with User=/NoNewPrivileges/ProtectSystem/ReadWritePaths it could
#       not write /usr/bin, which is the whole INC-I-215 failure
# 0 = nothing is rendered: no helper units exist, so a staged release sits on disk
#     forever and auto-update stays dead (pre-M4 behaviour).
#
# SAFETY: renders into a scratch dir only. It never touches /etc/systemd/system, never
# shells out to systemctl/launchctl/sudo, and never restarts anything.
#
# Re-runnable, no arguments, prints one integer 0..3 on stdout.
set -uo pipefail

WT="/Users/isudoajl/ownCloud/Projects/doli-network/doli/.claude/worktrees/inc-i-215-staged-upgrade"
SCRATCH="$(mktemp -d)"
cleanup() { rm -rf "$SCRATCH"; }
trap cleanup EXIT

UNITS="$SCRATCH/systemd"
mkdir -p "$UNITS"
SVC="doli-mainnet"
NET="mainnet"
DATA="/var/lib/doli/mainnet"
CLI="/usr/bin/doli"

( cd "$WT" && cargo run -q -p doli-cli --example inc_i_215_helper_units_fixture -- \
    "$UNITS" "$SVC" "$NET" "$DATA" "$CLI" ) >/dev/null 2>&1

PATH_UNIT="$UNITS/$SVC-upgrade.path"
SVC_UNIT="$UNITS/$SVC-upgrade.service"

SATISFIED=0

if grep -qxF "PathExists=$DATA/updates/ready" "$PATH_UNIT" 2>/dev/null \
   && grep -qxF "Unit=$SVC-upgrade.service" "$PATH_UNIT" 2>/dev/null; then
  SATISFIED=$((SATISFIED + 1))
fi

if grep -qxF "ExecStart=$CLI --network $NET upgrade --from-staged $DATA/updates --data-dir $DATA --service $SVC --yes" \
     "$SVC_UNIT" 2>/dev/null; then
  SATISFIED=$((SATISFIED + 1))
fi

if [ -f "$SVC_UNIT" ] \
   && grep -qxF "Type=oneshot" "$SVC_UNIT" \
   && ! grep -qE '^(User=|NoNewPrivileges|ProtectSystem|ReadWritePaths)' "$SVC_UNIT"; then
  SATISFIED=$((SATISFIED + 1))
fi

printf '%d\n' "$SATISFIED"
