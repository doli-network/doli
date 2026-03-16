#!/usr/bin/env bash
# chain-reset.sh — Reset local devnet chain data
#
# Usage: scripts/chain-reset.sh [devnet] [--skip-backup]
#
# Sequence:
#   1. Kill all local doli-node processes
#   2. Optionally back up ~/.doli/devnet/ data
#   3. Wipe data directories
#   4. Report completion
set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m'

NETWORK="${1:-devnet}"
SKIP_BACKUP="${2:-}"

if [[ "$NETWORK" != "devnet" ]]; then
  echo "Usage: $0 [devnet] [--skip-backup]"
  echo "  This script only manages local devnet. For remote networks, use the server scripts."
  exit 1
fi

DEVNET_DIR="$HOME/.doli/devnet"

echo -e "${YELLOW}=== DOLI Local Chain Reset: ${NETWORK} ===${NC}"
echo ""

# ── Confirmation ────────────────────────────────────────────────────────
echo -e "${RED}WARNING: This will DESTROY all local devnet chain data in ${DEVNET_DIR}.${NC}"
echo -n "Type 'RESET' to confirm: "
read -r confirm
if [[ "$confirm" != "RESET" ]]; then
  echo "Aborted."
  exit 1
fi
echo ""

# ── Phase 1: Kill doli-node processes ─────────────────────────────────
echo "Phase 1: Stopping all local doli-node processes..."

PIDS=$(pgrep -f "doli-node" 2>/dev/null || true)
if [[ -n "$PIDS" ]]; then
  echo "  Found doli-node PIDs: $PIDS"
  kill $PIDS 2>/dev/null || true
  sleep 2
  # Force kill any remaining
  REMAINING=$(pgrep -f "doli-node" 2>/dev/null || true)
  if [[ -n "$REMAINING" ]]; then
    echo "  Force killing remaining: $REMAINING"
    kill -9 $REMAINING 2>/dev/null || true
    sleep 1
  fi
  echo -e "  ${GREEN}All doli-node processes stopped.${NC}"
else
  echo "  No doli-node processes found."
fi

# Clean up pid files
if [[ -d "$DEVNET_DIR/pids" ]]; then
  rm -f "$DEVNET_DIR/pids"/*.pid 2>/dev/null || true
fi
echo ""

# ── Phase 2: Backup ────────────────────────────────────────────────────
if [[ "$SKIP_BACKUP" != "--skip-backup" ]]; then
  if [[ -d "$DEVNET_DIR/data" ]]; then
    echo "Phase 2: Backing up data directories..."
    TIMESTAMP=$(date +%Y%m%d-%H%M%S)
    BACKUP_DIR="$DEVNET_DIR/backups/${TIMESTAMP}"
    mkdir -p "$BACKUP_DIR"

    # Back up data directories
    for d in "$DEVNET_DIR"/data/node*; do
      if [[ -d "$d" ]]; then
        name=$(basename "$d")
        cp -r "$d" "$BACKUP_DIR/$name"
      fi
    done

    echo -e "  ${GREEN}Backed up to ${BACKUP_DIR}${NC}"
    echo ""
  else
    echo "Phase 2: No data directories to back up."
    echo ""
  fi
else
  echo "Phase 2: Skipping backup (--skip-backup)."
  echo ""
fi

# ── Phase 3: Wipe data ─────────────────────────────────────────────────
echo "Phase 3: Wiping data directories..."

# Wipe node data but preserve keys, chainspec, logs structure
if [[ -d "$DEVNET_DIR/data" ]]; then
  rm -rf "$DEVNET_DIR/data"/node*/
  echo "  Wiped node data directories."
fi

# Wipe logs
if [[ -d "$DEVNET_DIR/logs" ]]; then
  rm -f "$DEVNET_DIR/logs"/*.log
  echo "  Wiped log files."
fi

echo ""

# ── Done ──────────────────────────────────────────────────────────────
echo -e "${GREEN}=== Chain reset complete ===${NC}"
echo ""
echo "Preserved:"
echo "  Keys:      $DEVNET_DIR/keys/"
echo "  Chainspec: $DEVNET_DIR/chainspec.json"
if [[ "$SKIP_BACKUP" != "--skip-backup" && -n "${BACKUP_DIR:-}" ]]; then
  echo "  Backup:    $BACKUP_DIR"
fi
echo ""
echo "To restart devnet:"
echo "  doli-node devnet start"
echo "  # or: scripts/launch_testnet.sh"
