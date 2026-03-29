#!/usr/bin/env bash
# seed-backup.sh — Create RocksDB checkpoint on seed nodes
#
# Uses the createCheckpoint RPC method. Checkpoints use hard links,
# so they're near-instant and use minimal extra disk space.
#
# Usage:
#   scripts/seed-backup.sh                     # backup local seed (28500)
#   scripts/seed-backup.sh --testnet           # backup testnet seed (8500)
#   scripts/seed-backup.sh --endpoint HOST:PORT # backup specific node
#   scripts/seed-backup.sh --all               # backup all seeds (devnet)
#   scripts/seed-backup.sh --max-keep N        # keep last N checkpoints (default: 5)
#
# For cron:
#   0 * * * * /path/to/scripts/seed-backup.sh --testnet >> /var/log/doli-backup.log 2>&1
set -euo pipefail

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
BOLD='\033[1m'
NC='\033[0m'

MODE="devnet"
ENDPOINT=""
ALL=false
MAX_KEEP=5

while [[ $# -gt 0 ]]; do
  case "$1" in
    --testnet)   MODE="testnet"; shift ;;
    --devnet)    MODE="devnet"; shift ;;
    --endpoint)  ENDPOINT="$2"; shift 2 ;;
    --all)       ALL=true; shift ;;
    --max-keep)  MAX_KEEP="$2"; shift 2 ;;
    *) echo "Unknown arg: $1"; exit 1 ;;
  esac
done

rpc_call() {
  local endpoint="$1" method="$2" params="${3:-[]}"
  curl -sf --max-time 30 -X POST "http://${endpoint}" \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"${method}\",\"params\":${params},\"id\":1}" 2>/dev/null
}

create_checkpoint() {
  local endpoint="$1"
  local name="$2"

  echo -n "  Creating checkpoint on $name ($endpoint)... "

  local result
  result=$(rpc_call "$endpoint" "createCheckpoint") || {
    echo -e "${RED}FAIL${NC} (unreachable)"
    return 1
  }

  local status path height
  status=$(echo "$result" | python3 -c "import sys,json; print(json.load(sys.stdin).get('result',{}).get('status','error'))" 2>/dev/null)
  path=$(echo "$result" | python3 -c "import sys,json; print(json.load(sys.stdin).get('result',{}).get('path',''))" 2>/dev/null)
  height=$(echo "$result" | python3 -c "import sys,json; print(json.load(sys.stdin).get('result',{}).get('height','?'))" 2>/dev/null)

  if [[ "$status" == "ok" ]]; then
    echo -e "${GREEN}OK${NC} — height=$height path=$path"
    return 0
  else
    local error_msg
    error_msg=$(echo "$result" | python3 -c "import sys,json; r=json.load(sys.stdin); print(r.get('error',{}).get('message',str(r)))" 2>/dev/null)
    echo -e "${RED}FAIL${NC} — $error_msg"
    return 1
  fi
}

echo -e "${BOLD}=== SEED BACKUP (RocksDB Checkpoint) ===${NC}"
echo ""

success=0
failed=0

if [[ -n "$ENDPOINT" ]]; then
  create_checkpoint "$ENDPOINT" "custom" && success=$((success+1)) || failed=$((failed+1))
elif [[ "$ALL" == true ]]; then
  if [[ "$MODE" == "testnet" ]]; then
    create_checkpoint "127.0.0.1:8500" "Seed" && success=$((success+1)) || failed=$((failed+1))
  else
    for ((i=0; i<=50; i++)); do
      ep="127.0.0.1:$((28500 + i))"
      result=$(rpc_call "$ep" "getChainInfo" 2>/dev/null) || continue
      [[ -z "$result" || "$result" == "null" ]] && continue
      if [[ $i -eq 0 ]]; then name="Seed"; else name="N${i}"; fi
      create_checkpoint "$ep" "$name" && success=$((success+1)) || failed=$((failed+1))
    done
  fi
else
  # Default: just the seed
  if [[ "$MODE" == "testnet" ]]; then
    create_checkpoint "127.0.0.1:8500" "Seed" && success=$((success+1)) || failed=$((failed+1))
  else
    create_checkpoint "127.0.0.1:28500" "Seed" && success=$((success+1)) || failed=$((failed+1))
  fi
fi

echo ""
echo -e "${BOLD}Results:${NC} $success checkpoints created, $failed failed"

if [[ $success -gt 0 ]]; then
  echo ""
  echo "Checkpoints stored in {data_dir}/checkpoints/"
  echo "Each checkpoint is a full RocksDB snapshot (hard-linked, minimal disk overhead)."
  echo ""
  echo "To restore from a checkpoint:"
  echo "  1. Stop the node"
  echo "  2. cp -r checkpoints/h{HEIGHT}-{TS}/state_db data_dir/state_db"
  echo "  3. cp -r checkpoints/h{HEIGHT}-{TS}/blocks data_dir/blocks"
  echo "  4. Start the node"
fi
