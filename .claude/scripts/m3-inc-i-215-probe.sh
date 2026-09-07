#!/usr/bin/env bash
# INC-I-215 M3 outcome probe.
#
# Question it answers, from OUTSIDE the test suite: given a REAL staged handoff on disk
# (3-of-5 on-chain trust root, signed CHECKSUMS.txt, a signed tarball and a `ready`
# marker for v99.0.0), how many of the three post-conditions of a privileged install does
# ONE `doli upgrade --from-staged` run actually satisfy?
#   1 = the node target holds the bytes that were staged
#   2 = <target>.backup exists holding the OLD bytes (a recoverable host)
#   3 = the staging dir is gone (M4's `.path` unit is disarmed, no retrigger loop)
# 0 = nothing happens: the flag does not exist, so a staged release sits on disk forever
#     and the node stays on the old binary (pre-M3 behaviour).
#
# SAFETY: `restart_doli_service` on macOS kickstarts EVERY launchd label containing
# "doli", which on this machine is an 18-node testnet. This probe therefore passes
# `--service <fake>` (routing the CLI to `sudo systemctl …`) and runs it with PATH
# prefixed by a scratch shim dir holding an inert `sudo` and `systemctl`. Never drop
# either half.
#
# Re-runnable, no arguments, prints one integer 0..3 on stdout.
set -uo pipefail

WT="/Users/isudoajl/ownCloud/Projects/doli-network/doli/.claude/worktrees/inc-i-215-staged-upgrade"
SCRATCH="$(mktemp -d)"
cleanup() { chmod -R u+w "$SCRATCH" >/dev/null 2>&1; rm -rf "$SCRATCH"; }
trap cleanup EXIT

# --- restart shims: sudo runs its argv, systemctl does nothing and succeeds -----------
SHIM="$SCRATCH/shim"
mkdir -p "$SHIM"
printf '#!/bin/sh\nexec "$@"\n' > "$SHIM/sudo"
printf '#!/bin/sh\nexit 0\n'    > "$SHIM/systemctl"
chmod 0755 "$SHIM/sudo" "$SHIM/systemctl"

# --- lay out the staged handoff with the shipped libraries ----------------------------
( cd "$WT" && cargo run -q -p doli-cli --example inc_i_215_from_staged_fixture -- \
    "$SCRATCH" ) >/dev/null 2>&1

TARGET="$SCRATCH/bin/doli-node"
STAGED_PAYLOAD="$SCRATCH/expected-new"
OLD_PAYLOAD="$SCRATCH/expected-old"
printf '#!/bin/sh\n# INC-I-215 STAGED PAYLOAD v99.0.0\nexit 0\n'    > "$STAGED_PAYLOAD"
printf '#!/bin/sh\n# INC-I-215 PRE-EXISTING BINARY v6.29.0\nexit 0\n' > "$OLD_PAYLOAD"

# --- the one run under test ------------------------------------------------------------
( cd "$WT" && PATH="$SHIM:$PATH" cargo run -q -p doli-cli --bin doli -- \
    --network devnet upgrade \
    --from-staged "$SCRATCH/updates" \
    --data-dir "$SCRATCH/data" \
    --doli-node-path "$TARGET" \
    --service doli-inc-i-215-probe.service \
    --yes ) >/dev/null 2>&1

SATISFIED=0
cmp -s "$TARGET" "$STAGED_PAYLOAD"        && SATISFIED=$((SATISFIED + 1))
cmp -s "$TARGET.backup" "$OLD_PAYLOAD"    && SATISFIED=$((SATISFIED + 1))
[ ! -d "$SCRATCH/updates" ]               && SATISFIED=$((SATISFIED + 1))

printf '%d\n' "$SATISFIED"
