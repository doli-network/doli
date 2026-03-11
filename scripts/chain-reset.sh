#!/usr/bin/env bash
# chain-reset.sh — Safe chain reset with pre-flight validation
#
# Usage: scripts/chain-reset.sh [mainnet|testnet] [--skip-backup]
#
# Sequence:
#   1. Validate ALL service files have --bootstrap (blocks reset if not)
#   2. Back up current data directories
#   3. Stop all services
#   4. Wipe data directories
#   5. Reload systemd (picks up any service file changes)
#   6. Start seeds first, wait, then producers
#   7. Run health-check.sh (blocks until all checks pass or fails)
#
# See: docs/postmortems/2026-03-11-network-partition.md
set -euo pipefail

AI1="ilozada@72.60.228.233"
AI2="ilozada@187.124.95.188"
AI3="ilozada@187.124.148.93"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m'

NETWORK="${1:-}"
SKIP_BACKUP="${2:-}"

if [[ -z "$NETWORK" || ! "$NETWORK" =~ ^(mainnet|testnet)$ ]]; then
  echo "Usage: $0 [mainnet|testnet] [--skip-backup]"
  exit 1
fi

echo -e "${YELLOW}=== DOLI Chain Reset: ${NETWORK} ===${NC}"
echo ""

# ── Phase 0: Pre-flight validation ─────────────────────────────────────
echo "Phase 0: Validating service files..."

VALIDATION_ERRORS=0

validate_service() {
  local server="$1" svc_name="$2" needs_bootstrap="$3"

  local has_bootstrap
  has_bootstrap=$(ssh -o ConnectTimeout=5 "$server" \
    "grep -c 'bootstrap' /etc/systemd/system/${svc_name}.service 2>/dev/null || echo 0" 2>/dev/null) || has_bootstrap=0

  if [[ "$needs_bootstrap" == "true" && "$has_bootstrap" -lt 1 ]]; then
    echo -e "  ${RED}FAIL${NC} ${svc_name} on ${server} — MISSING --bootstrap"
    VALIDATION_ERRORS=$((VALIDATION_ERRORS + 1))
  else
    echo -e "  ${GREEN}OK${NC}   ${svc_name} on ${server}"
  fi
}

if [[ "$NETWORK" == "mainnet" ]]; then
  validate_service "$AI1" "doli-mainnet-seed" "false"
  validate_service "$AI2" "doli-mainnet-seed" "true"
  validate_service "$AI3" "doli-mainnet-seed" "true" 2>/dev/null || true
  for N in $(seq 1 12); do
    if (( N % 2 == 1 )); then server="$AI1"; else server="$AI2"; fi
    validate_service "$server" "doli-mainnet-n${N}" "true"
  done
else
  validate_service "$AI1" "doli-testnet-seed" "false"
  validate_service "$AI2" "doli-testnet-seed" "true"
  validate_service "$AI3" "doli-testnet-seed" "true" 2>/dev/null || true
  for N in $(seq 1 12); do
    if (( N % 2 == 1 )); then server="$AI1"; else server="$AI2"; fi
    validate_service "$server" "doli-testnet-nt${N}" "true"
  done
fi

if (( VALIDATION_ERRORS > 0 )); then
  echo ""
  echo -e "${RED}BLOCKED: ${VALIDATION_ERRORS} service files are misconfigured.${NC}"
  echo "Fix with: scripts/install-services.sh ${NETWORK}"
  echo "Then re-run this script."
  exit 1
fi

echo -e "${GREEN}All service files validated.${NC}"
echo ""

# ── Confirmation ────────────────────────────────────────────────────────
echo -e "${RED}WARNING: This will DESTROY all ${NETWORK} chain data on ai1, ai2, and ai3.${NC}"
echo -n "Type 'RESET' to confirm: "
read -r confirm
if [[ "$confirm" != "RESET" ]]; then
  echo "Aborted."
  exit 1
fi
echo ""

# ── Phase 1: Backup ────────────────────────────────────────────────────
if [[ "$SKIP_BACKUP" != "--skip-backup" ]]; then
  echo "Phase 1: Backing up data directories..."
  TIMESTAMP=$(date +%Y%m%d-%H%M%S)

  if [[ "$NETWORK" == "mainnet" ]]; then
    BACKUP_DIR="/mainnet/backups/${TIMESTAMP}"
    for server in "$AI1" "$AI2"; do
      ssh -o ConnectTimeout=10 "$server" "
        sudo mkdir -p ${BACKUP_DIR}
        for d in /mainnet/seed/data /mainnet/n*/data; do
          if [ -d \"\$d\" ] && [ \"\$(ls -A \$d 2>/dev/null)\" ]; then
            name=\$(echo \$d | sed 's|/mainnet/||;s|/data||;s|/|-|g')
            sudo cp -r \"\$d\" \"${BACKUP_DIR}/\${name}\"
          fi
        done
        echo \"  Backed up to ${BACKUP_DIR} on \$(hostname)\"
      " 2>/dev/null || echo "  Warning: backup failed on $server"
    done
  else
    BACKUP_DIR="/testnet/backups/${TIMESTAMP}"
    for server in "$AI1" "$AI2"; do
      ssh -o ConnectTimeout=10 "$server" "
        sudo mkdir -p ${BACKUP_DIR}
        for d in /testnet/seed/data /testnet/nt*/data; do
          if [ -d \"\$d\" ] && [ \"\$(ls -A \$d 2>/dev/null)\" ]; then
            name=\$(echo \$d | sed 's|/testnet/||;s|/data||;s|/|-|g')
            sudo cp -r \"\$d\" \"${BACKUP_DIR}/\${name}\"
          fi
        done
        echo \"  Backed up to ${BACKUP_DIR} on \$(hostname)\"
      " 2>/dev/null || echo "  Warning: backup failed on $server"
    done
  fi
  echo ""
else
  echo "Phase 1: Skipping backup (--skip-backup)."
  echo ""
fi

# ── Phase 2: Stop all services ─────────────────────────────────────────
echo "Phase 2: Stopping all ${NETWORK} services..."

if [[ "$NETWORK" == "mainnet" ]]; then
  for server in "$AI1" "$AI2" "$AI3"; do
    ssh -o ConnectTimeout=10 "$server" "
      sudo systemctl stop doli-mainnet-seed 2>/dev/null || true
      for N in \$(seq 1 12); do
        sudo systemctl stop doli-mainnet-n\${N} 2>/dev/null || true
      done
    " 2>/dev/null && echo "  Stopped on $server" || echo "  Warning: stop failed on $server"
  done
else
  for server in "$AI1" "$AI2" "$AI3"; do
    ssh -o ConnectTimeout=10 "$server" "
      sudo systemctl stop doli-testnet-seed 2>/dev/null || true
      for N in \$(seq 1 12); do
        sudo systemctl stop doli-testnet-nt\${N} 2>/dev/null || true
      done
    " 2>/dev/null && echo "  Stopped on $server" || echo "  Warning: stop failed on $server"
  done
fi
echo ""

# ── Phase 3: Wipe data ─────────────────────────────────────────────────
echo "Phase 3: Wiping data directories..."

if [[ "$NETWORK" == "mainnet" ]]; then
  for server in "$AI1" "$AI2" "$AI3"; do
    ssh -o ConnectTimeout=10 "$server" "
      sudo find /mainnet/seed/data -mindepth 1 -delete 2>/dev/null || true
      for N in \$(seq 1 12); do
        sudo find /mainnet/n\${N}/data -mindepth 1 -delete 2>/dev/null || true
      done
    " 2>/dev/null && echo "  Wiped on $server" || echo "  Warning: wipe failed on $server"
  done
else
  for server in "$AI1" "$AI2" "$AI3"; do
    ssh -o ConnectTimeout=10 "$server" "
      sudo find /testnet/seed/data -mindepth 1 -delete 2>/dev/null || true
      for N in \$(seq 1 12); do
        sudo find /testnet/nt\${N}/data -mindepth 1 -delete 2>/dev/null || true
      done
    " 2>/dev/null && echo "  Wiped on $server" || echo "  Warning: wipe failed on $server"
  done
fi
echo ""

# ── Phase 4: Reload systemd ────────────────────────────────────────────
echo "Phase 4: Reloading systemd..."
for server in "$AI1" "$AI2" "$AI3"; do
  ssh -o ConnectTimeout=10 "$server" "sudo systemctl daemon-reload" 2>/dev/null \
    && echo "  Reloaded on $server" || echo "  Warning: reload failed on $server"
done
echo ""

# ── Phase 5: Start seeds first ─────────────────────────────────────────
echo "Phase 5: Starting seeds..."
for server in "$AI1" "$AI2" "$AI3"; do
  ssh -o ConnectTimeout=10 "$server" "sudo systemctl start doli-${NETWORK}-seed" 2>/dev/null \
    && echo "  Seed started on $server" || echo "  Warning: seed start failed on $server"
done

echo "  Waiting 15 seconds for seeds to initialize..."
sleep 15
echo ""

# ── Phase 6: Start producers ───────────────────────────────────────────
echo "Phase 6: Starting producers..."

if [[ "$NETWORK" == "mainnet" ]]; then
  for N in $(seq 1 12); do
    if (( N % 2 == 1 )); then server="$AI1"; else server="$AI2"; fi
    ssh -o ConnectTimeout=10 "$server" "sudo systemctl start doli-mainnet-n${N}" 2>/dev/null \
      && echo "  N${N} started on $server" || echo "  Warning: N${N} start failed on $server"
    sleep 2
  done
else
  for N in $(seq 1 12); do
    if (( N % 2 == 1 )); then server="$AI1"; else server="$AI2"; fi
    ssh -o ConnectTimeout=10 "$server" "sudo systemctl start doli-testnet-nt${N}" 2>/dev/null \
      && echo "  NT${N} started on $server" || echo "  Warning: NT${N} start failed on $server"
    sleep 2
  done
fi
echo ""

# ── Phase 7: Health check ──────────────────────────────────────────────
echo "Phase 7: Waiting 30 seconds before health check..."
sleep 30

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
echo "Running health check..."
echo ""
"${SCRIPT_DIR}/health-check.sh" "$NETWORK"
