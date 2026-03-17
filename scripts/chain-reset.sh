#!/usr/bin/env bash
# chain-reset.sh — Safe chain reset with pre-flight validation
#
# Usage: scripts/chain-reset.sh [mainnet|testnet] [--skip-backup]
#
# Architecture v8 (2026-03-17):
#   ai1 = mainnet seed+N1-N3 + testnet seed+NT1-NT5
#   ai2 = mainnet seed+N4-N5 + testnet seed + build + explorer
#   ai3 = seeds only (both networks), ai4 = mainnet N6-N8, ai5 = mainnet N9-N12 + testnet NT6-NT12
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

AI1="${DOLI_AI1:?Set DOLI_AI1=user@host}"    # Mainnet seed+N1-N3 + Testnet seed+NT1-NT5
AI2="${DOLI_AI2:?Set DOLI_AI2=user@host}"   # Mainnet seed+N4-N5 + Testnet seed + build + explorer
AI3="${DOLI_AI3:?Set DOLI_AI3=user@host}"   # Seeds only (both networks)
AI4="${DOLI_AI4:?Set DOLI_AI4=user@host}"  # Mainnet N6-N8
AI5="${DOLI_AI5:?Set DOLI_AI5=user@host}"    # Mainnet N9-N12 + Testnet NT6-NT12

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

# SSH wrapper
do_ssh() {
  local server="$1"; shift
  ssh -p "${DOLI_SSH_PORT:-22}" -o ConnectTimeout=10 "$server" "$@"
}

echo -e "${YELLOW}=== DOLI Chain Reset: ${NETWORK} ===${NC}"
echo ""

# Determine which servers to target
if [[ "$NETWORK" == "mainnet" ]]; then
  PRIMARY="$AI1"       # Mainnet seed + N1-N3
  SECONDARY="$AI2"     # Mainnet seed + N4-N5
  TERTIARY="$AI4"      # Mainnet N6-N8
  QUATERNARY="$AI5"    # Mainnet N9-N12
  PRODUCERS="N"
  PRIMARY_LIST="1 2 3"
  SECONDARY_LIST="4 5"
  TERTIARY_LIST="6 7 8"
  QUATERNARY_LIST="9 10 11 12"
  NODE_LIST="1 2 3 4 5 6 7 8 9 10 11 12"
else
  PRIMARY="$AI1"       # Testnet NT1-NT5 + seed
  SECONDARY="$AI5"     # Testnet NT6-NT12
  TERTIARY=""
  QUATERNARY=""
  PRODUCERS="NT"
  PRIMARY_LIST="1 2 3 4 5"
  SECONDARY_LIST="6 7 8 9 10 11 12"
  TERTIARY_LIST=""
  QUATERNARY_LIST=""
  NODE_LIST="1 2 3 4 5 6 7 8 9 10 11 12"
fi

# ── Phase 0: Pre-flight validation ─────────────────────────────────────
echo "Phase 0: Validating service files..."

VALIDATION_ERRORS=0

validate_service() {
  local server="$1" svc_name="$2" needs_bootstrap="$3"

  local has_bootstrap
  has_bootstrap=$(do_ssh "$server" \
    "grep -c 'bootstrap' /etc/systemd/system/${svc_name}.service 2>/dev/null || echo 0" 2>/dev/null) || has_bootstrap=0

  if [[ "$needs_bootstrap" == "true" && "$has_bootstrap" -lt 1 ]]; then
    echo -e "  ${RED}FAIL${NC} ${svc_name} on ${server} — MISSING --bootstrap"
    VALIDATION_ERRORS=$((VALIDATION_ERRORS + 1))
  else
    echo -e "  ${GREEN}OK${NC}   ${svc_name} on ${server}"
  fi
}

# Primary seed (no bootstrap needed — it IS the bootstrap)
validate_service "$PRIMARY" "doli-${NETWORK}-seed" "false"

# Seed on ai3 (needs bootstrap)
validate_service "$AI3" "doli-${NETWORK}-seed" "true" 2>/dev/null || true

# Seed on the OTHER primary server (mainnet has seed on ai1, testnet has no extra)
if [[ "$NETWORK" == "mainnet" ]]; then
  validate_service "$AI1" "doli-mainnet-seed" "true" 2>/dev/null || true
fi

# All producers on primary server
for N in $NODE_LIST; do
  local_prefix=$(echo "$PRODUCERS" | tr '[:upper:]' '[:lower:]')
  validate_service "$PRIMARY" "doli-${NETWORK}-${local_prefix}${N}" "true"
done

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
echo -e "${RED}WARNING: This will DESTROY all ${NETWORK} chain data on $(hostname) and ai3.${NC}"
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
  BACKUP_DIR="/${NETWORK}/backups/${TIMESTAMP}"

  # Primary server (all producers + seed)
  do_ssh "$PRIMARY" "
    sudo mkdir -p ${BACKUP_DIR}
    for d in /${NETWORK}/seed/data /${NETWORK}/*/data; do
      if [ -d \"\$d\" ] && [ \"\$(ls -A \$d 2>/dev/null)\" ]; then
        name=\$(echo \$d | sed 's|/${NETWORK}/||;s|/data||;s|/|-|g')
        sudo cp -r \"\$d\" \"${BACKUP_DIR}/\${name}\"
      fi
    done
    echo \"  Backed up to ${BACKUP_DIR} on \$(hostname)\"
  " 2>/dev/null || echo "  Warning: backup failed on primary"

  # ai3 (seed only)
  do_ssh "$AI3" "
    sudo mkdir -p ${BACKUP_DIR}
    if [ -d /${NETWORK}/seed/data ] && [ \"\$(ls -A /${NETWORK}/seed/data 2>/dev/null)\" ]; then
      sudo cp -r /${NETWORK}/seed/data ${BACKUP_DIR}/seed
    fi
    echo \"  Backed up to ${BACKUP_DIR} on \$(hostname)\"
  " 2>/dev/null || echo "  Warning: backup failed on ai3"

  echo ""
else
  echo "Phase 1: Skipping backup (--skip-backup)."
  echo ""
fi

# ── Phase 2: Stop all services ─────────────────────────────────────────
echo "Phase 2: Stopping all ${NETWORK} services..."

# Stop producers on primary
local_prefix=$(echo "$PRODUCERS" | tr '[:upper:]' '[:lower:]')
do_ssh "$PRIMARY" "
  for N in ${PRIMARY_LIST}; do
    sudo systemctl stop doli-${NETWORK}-${local_prefix}\${N} 2>/dev/null || true
  done
  sudo systemctl stop doli-${NETWORK}-seed 2>/dev/null || true
" 2>/dev/null && echo "  Stopped on primary" || echo "  Warning: stop failed on primary"

# Stop producers on secondary
do_ssh "$SECONDARY" "
  for N in ${SECONDARY_LIST}; do
    sudo systemctl stop doli-${NETWORK}-${local_prefix}\${N} 2>/dev/null || true
  done
  sudo systemctl stop doli-${NETWORK}-seed 2>/dev/null || true
" 2>/dev/null && echo "  Stopped on secondary" || echo "  Warning: stop failed on secondary"

# Stop producers on tertiary (mainnet N6-N8 on ai4)
if [[ -n "$TERTIARY" ]]; then
  do_ssh "$TERTIARY" "
    for N in ${TERTIARY_LIST}; do
      sudo systemctl stop doli-${NETWORK}-${local_prefix}\${N} 2>/dev/null || true
    done
  " 2>/dev/null && echo "  Stopped on tertiary" || echo "  Warning: stop failed on tertiary"
fi

# Stop producers on quaternary (mainnet N9-N12 on ai5)
if [[ -n "$QUATERNARY" ]]; then
  do_ssh "$QUATERNARY" "
    for N in ${QUATERNARY_LIST}; do
      sudo systemctl stop doli-${NETWORK}-${local_prefix}\${N} 2>/dev/null || true
    done
  " 2>/dev/null && echo "  Stopped on quaternary" || echo "  Warning: stop failed on quaternary"
fi

# Stop seed on ai3
do_ssh "$AI3" "sudo systemctl stop doli-${NETWORK}-seed 2>/dev/null || true" \
  && echo "  Stopped seed on ai3" || echo "  Warning: stop failed on ai3"

echo ""

# ── Phase 3: Wipe data ─────────────────────────────────────────────────
echo "Phase 3: Wiping data directories..."

do_ssh "$PRIMARY" "
  sudo find /${NETWORK}/seed/data -mindepth 1 -delete 2>/dev/null || true
  sudo find /${NETWORK}/seed/blocks -mindepth 1 -delete 2>/dev/null || true
  for N in ${PRIMARY_LIST}; do
    sudo find /${NETWORK}/${local_prefix}\${N}/data -mindepth 1 -delete 2>/dev/null || true
  done
" 2>/dev/null && echo "  Wiped on primary" || echo "  Warning: wipe failed on primary"

do_ssh "$SECONDARY" "
  sudo find /${NETWORK}/seed/data -mindepth 1 -delete 2>/dev/null || true
  sudo find /${NETWORK}/seed/blocks -mindepth 1 -delete 2>/dev/null || true
  for N in ${SECONDARY_LIST}; do
    sudo find /${NETWORK}/${local_prefix}\${N}/data -mindepth 1 -delete 2>/dev/null || true
  done
" 2>/dev/null && echo "  Wiped on secondary" || echo "  Warning: wipe failed on secondary"

if [[ -n "$TERTIARY" ]]; then
  do_ssh "$TERTIARY" "
    for N in ${TERTIARY_LIST}; do
      sudo find /${NETWORK}/${local_prefix}\${N}/data -mindepth 1 -delete 2>/dev/null || true
    done
  " 2>/dev/null && echo "  Wiped on tertiary" || echo "  Warning: wipe failed on tertiary"
fi

if [[ -n "$QUATERNARY" ]]; then
  do_ssh "$QUATERNARY" "
    for N in ${QUATERNARY_LIST}; do
      sudo find /${NETWORK}/${local_prefix}\${N}/data -mindepth 1 -delete 2>/dev/null || true
    done
  " 2>/dev/null && echo "  Wiped on quaternary" || echo "  Warning: wipe failed on quaternary"
fi

do_ssh "$AI3" "
  sudo find /${NETWORK}/seed/data -mindepth 1 -delete 2>/dev/null || true
  sudo find /${NETWORK}/seed/blocks -mindepth 1 -delete 2>/dev/null || true
" 2>/dev/null && echo "  Wiped seed on ai3" || echo "  Warning: wipe failed on ai3"

echo ""

# ── Phase 4: Reload systemd ────────────────────────────────────────────
echo "Phase 4: Reloading systemd..."
do_ssh "$PRIMARY" "sudo systemctl daemon-reload" 2>/dev/null \
  && echo "  Reloaded on primary" || echo "  Warning: reload failed on primary"
do_ssh "$SECONDARY" "sudo systemctl daemon-reload" 2>/dev/null \
  && echo "  Reloaded on secondary" || echo "  Warning: reload failed on secondary"
if [[ -n "$TERTIARY" ]]; then
  do_ssh "$TERTIARY" "sudo systemctl daemon-reload" 2>/dev/null \
    && echo "  Reloaded on tertiary" || echo "  Warning: reload failed on tertiary"
fi
if [[ -n "$QUATERNARY" ]]; then
  do_ssh "$QUATERNARY" "sudo systemctl daemon-reload" 2>/dev/null \
    && echo "  Reloaded on quaternary" || echo "  Warning: reload failed on quaternary"
fi
do_ssh "$AI3" "sudo systemctl daemon-reload" 2>/dev/null \
  && echo "  Reloaded on ai3" || echo "  Warning: reload failed on ai3"
echo ""

# ── Phase 5: Start seeds first ─────────────────────────────────────────
echo "Phase 5: Starting seeds..."
do_ssh "$PRIMARY" "sudo systemctl start doli-${NETWORK}-seed" 2>/dev/null \
  && echo "  Seed started on primary" || echo "  Warning: seed start failed on primary"
do_ssh "$SECONDARY" "sudo systemctl start doli-${NETWORK}-seed" 2>/dev/null \
  && echo "  Seed started on secondary" || echo "  Warning: seed start failed on secondary"
do_ssh "$AI3" "sudo systemctl start doli-${NETWORK}-seed" 2>/dev/null \
  && echo "  Seed started on ai3" || echo "  Warning: seed start failed on ai3"

echo "  Waiting 15 seconds for seeds to initialize..."
sleep 15
echo ""

# ── Phase 6: Start producers ───────────────────────────────────────────
echo "Phase 6: Starting producers..."

# Start primary producers
for N in $PRIMARY_LIST; do
  do_ssh "$PRIMARY" "sudo systemctl start doli-${NETWORK}-${local_prefix}${N}" 2>/dev/null \
    && echo "  ${PRODUCERS}${N} started (primary)" || echo "  Warning: ${PRODUCERS}${N} start failed"
  sleep 2
done

# Start secondary producers
for N in $SECONDARY_LIST; do
  do_ssh "$SECONDARY" "sudo systemctl start doli-${NETWORK}-${local_prefix}${N}" 2>/dev/null \
    && echo "  ${PRODUCERS}${N} started (secondary)" || echo "  Warning: ${PRODUCERS}${N} start failed"
  sleep 2
done

# Start tertiary producers (mainnet N6-N8 on ai4)
if [[ -n "$TERTIARY" && -n "$TERTIARY_LIST" ]]; then
  for N in $TERTIARY_LIST; do
    do_ssh "$TERTIARY" "sudo systemctl start doli-${NETWORK}-${local_prefix}${N}" 2>/dev/null \
      && echo "  ${PRODUCERS}${N} started (tertiary)" || echo "  Warning: ${PRODUCERS}${N} start failed"
    sleep 2
  done
fi

# Start quaternary producers (mainnet N9-N12 on ai5)
if [[ -n "$QUATERNARY" && -n "$QUATERNARY_LIST" ]]; then
  for N in $QUATERNARY_LIST; do
    do_ssh "$QUATERNARY" "sudo systemctl start doli-${NETWORK}-${local_prefix}${N}" 2>/dev/null \
      && echo "  ${PRODUCERS}${N} started (quaternary)" || echo "  Warning: ${PRODUCERS}${N} start failed"
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
