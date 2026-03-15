#!/usr/bin/env bash
# rotate-logs.sh — Rotate all testnet logs at 1MB
# Keeps current .log and one .log.1 backup per node.
# Run periodically or via: scripts/testnet.sh rotate-logs
set -euo pipefail

TESTNET_DIR="$HOME/testnet"
MAX_SIZE=1048576  # 1MB

rotated=0

# Genesis node logs
for f in "$TESTNET_DIR"/logs/*.log; do
  [[ -f "$f" ]] || continue
  size=$(stat -f%z "$f" 2>/dev/null || echo 0)
  if (( size > MAX_SIZE )); then
    mv "$f" "${f}.1"
    rotated=$((rotated + 1))
  fi
done

# Stress batch logs
for batch_dir in "$TESTNET_DIR"/logs/nodes*/; do
  [[ -d "$batch_dir" ]] || continue
  for f in "$batch_dir"*.log; do
    [[ -f "$f" ]] || continue
    size=$(stat -f%z "$f" 2>/dev/null || echo 0)
    if (( size > MAX_SIZE )); then
      mv "$f" "${f}.1"
      rotated=$((rotated + 1))
    fi
  done
done

echo "Rotated $rotated log files"
