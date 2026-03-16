#!/usr/bin/env bash
# Usage: scripts/balances.sh [devnet]
# Shows wallet balances for all producer keys in the local devnet.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
CLI_BIN="$PROJECT_ROOT/target/release/doli"
KEYS_DIR="$HOME/.doli/devnet/keys"
RPC_PORT=28500
RPC_ENDPOINT="http://127.0.0.1:${RPC_PORT}"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
RED='\033[0;31m'
NC='\033[0m'

# Check CLI binary
if [[ ! -f "$CLI_BIN" ]]; then
  echo -e "${RED}CLI binary not found at ${CLI_BIN}${NC}"
  echo "Run: cargo build --release"
  exit 1
fi

# Check keys directory
if [[ ! -d "$KEYS_DIR" ]]; then
  echo -e "${RED}Keys directory not found: ${KEYS_DIR}${NC}"
  echo "Initialize devnet first: doli-node devnet init --nodes 5"
  exit 1
fi

# Check RPC is up
if ! curl -sf --max-time 2 "http://127.0.0.1:${RPC_PORT}" -X POST \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' >/dev/null 2>&1; then
  echo -e "${RED}No devnet node responding on port ${RPC_PORT}${NC}"
  echo "Start devnet first: doli-node devnet start"
  exit 1
fi

echo -e "${CYAN}Wallet Balances (devnet)${NC}"
echo ""
printf "%-20s %15s %s\n" "Wallet" "Balance" "Address"
printf "%-20s %15s %s\n" "--------------------" "---------------" "--------"

total=0
shopt -s nullglob
for wallet in "$KEYS_DIR"/producer_*.json; do
  name=$(basename "$wallet" .json)
  address=$(python3 -c "import json; w=json.load(open('$wallet')); print(w['addresses'][0]['address'])" 2>/dev/null || echo "?")

  balance=$("$CLI_BIN" -r "$RPC_ENDPOINT" -w "$wallet" balance 2>/dev/null | grep -oE '[0-9]+\.[0-9]+' | head -1 || echo "0.00")

  printf "%-20s %15s %s\n" "$name" "${balance} DOLI" "${address:0:16}..."
  total=$(echo "$total + $balance" | bc 2>/dev/null || echo "$total")
done
shopt -u nullglob

echo ""
echo -e "Total: ${GREEN}${total} DOLI${NC}"
