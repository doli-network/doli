#!/usr/bin/env bash
# rotate-logs.sh — Rotate all mainnet logs at 1MB
# Keeps current .log and one .log.1 backup per node.
# Run periodically or via: scripts/mainnet.sh rotate-logs
set -euo pipefail

MAINNET_DIR="$HOME/mainnet"
MAX_SIZE=1048576  # 1MB

rotated=0

# Genesis node logs
for f in "$MAINNET_DIR"/logs/*.log; do
  [[ -f "$f" ]] || continue
  size=$(stat -f%z "$f" 2>/dev/null || echo 0)
  if (( size > MAX_SIZE )); then
    mv "$f" "${f}.1"
    rotated=$((rotated + 1))
  fi
done

# Stress batch logs
for batch_dir in "$MAINNET_DIR"/logs/nodes*/; do
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
