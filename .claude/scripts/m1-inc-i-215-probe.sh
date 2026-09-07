#!/usr/bin/env bash
# INC-I-215 M1 outcome probe.
#
# Question it answers, from OUTSIDE the test suite: when the updater's install
# target is NOT writable (the sandboxed `doli service install` unit of
# INC-I-215), how many verified release artifacts does the updater actually
# write into {data_dir}/updates/ ?
#
# It runs the shipped library through `cargo run --example` against a real
# read-only directory and counts the files that land on disk.
#   0 = the updater stages nothing (pre-M1: no staging path exists at all)
#   4 = release.tar.gz + CHECKSUMS.txt + SIGNATURES.json + ready
#
# Re-runnable, no arguments, prints one integer on stdout.
set -uo pipefail

WT="/Users/isudoajl/ownCloud/Projects/doli-network/doli/.claude/worktrees/inc-i-215-staged-upgrade"
SCRATCH="$(mktemp -d)"
cleanup() { chmod -R u+w "$SCRATCH" >/dev/null 2>&1; rm -rf "$SCRATCH"; }
trap cleanup EXIT

mkdir -p "$SCRATCH/opt/bin" "$SCRATCH/data"
chmod 0555 "$SCRATCH/opt/bin"

( cd "$WT" && cargo run -q -p updater --example inc_i_215_stage_probe -- \
    "$SCRATCH/opt/bin/doli-node" "$SCRATCH/data" ) >/dev/null 2>&1

ls -1 "$SCRATCH/data/updates" 2>/dev/null | wc -l | tr -d ' '
