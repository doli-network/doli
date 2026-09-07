#!/usr/bin/env bash
# INC-I-171 M7 outcome probe — externally observable duplication of the vesting
# penalty ladder outside crates/core.
#
# LADDER_SITES      independent re-derivations of `amount * pct / 100` in the
#                   RPC and CLI presentation layers (must reach 0: one ladder).
# LOSSY_SLOT_CASTS  unguarded `as u32` narrowings of a u64 slot/quarter feeding
#                   the ladder in the RPC (anti-pattern AP-6; must reach 0).
#
# Re-runnable from the repo root, no node and no network required.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 1

CLI=bins/cli/src/cmd_producer/common.rs
RPC=crates/rpc/src/methods/producer.rs

LADDER=$(grep -c 'amount \* .*pct\|\* pct as u128 / 100\|/ 100' "$CLI" "$RPC" 2>/dev/null | awk -F: '{s+=$2} END {print s+0}')
CASTS=$(grep -c 'age as u32\|quarter as u32\|oldest_age as u32' "$RPC" 2>/dev/null)

echo "LADDER_SITES=${LADDER}"
echo "LOSSY_SLOT_CASTS=${CASTS}"
