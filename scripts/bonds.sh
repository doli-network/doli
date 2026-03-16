#!/usr/bin/env bash
# Usage: scripts/bonds.sh [devnet]
# Shows bond/producer info for the local devnet via RPC.
set -euo pipefail

RPC_PORT=28500

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
RED='\033[0;31m'
NC='\033[0m'

rpc_call() {
  local method="$1" params="${2:-{}}"
  curl -sf --max-time 5 -X POST "http://127.0.0.1:${RPC_PORT}" \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"${method}\",\"params\":${params},\"id\":1}" 2>/dev/null
}

# Check RPC is up
if ! rpc_call "getChainInfo" >/dev/null 2>&1; then
  echo -e "${RED}No devnet node responding on port ${RPC_PORT}${NC}"
  echo "Start devnet first: doli-node devnet start"
  exit 1
fi

echo -e "${CYAN}Producer & Bond Info (devnet)${NC}"
echo ""

# Get producers list
producers=$(rpc_call "getProducers" '{"includeInactive": true}')

if [[ -z "$producers" ]]; then
  echo "No response from getProducers."
  exit 1
fi

# Parse and display
python3 -c "
import json, sys

data = json.loads('''${producers}''')
result = data.get('result', {})
producers = result.get('producers', [])

if not producers:
    print('No producers registered.')
    sys.exit(0)

print(f'{'Producer':<20} {'Bonds':>6} {'Total Staked':>14} {'Status':<12} {'Pubkey':<20}')
print(f'{\"---\"*7:<20} {\"---\"*2:>6} {\"---\"*5:>14} {\"---\"*4:<12} {\"---\"*7:<20}')

total_staked = 0
for p in producers:
    name = p.get('name', '?')
    pubkey = p.get('publicKey', p.get('public_key', '?'))
    bonds = p.get('bondCount', p.get('bond_count', 0))
    staked = p.get('totalStaked', p.get('total_staked', 0))
    active = p.get('active', p.get('is_active', False))
    status = 'Active' if active else 'Inactive'

    staked_doli = staked / 1e8 if isinstance(staked, (int, float)) and staked > 1000 else staked
    total_staked += staked_doli if isinstance(staked_doli, (int, float)) else 0

    short_key = pubkey[:16] + '...' if len(str(pubkey)) > 16 else pubkey
    print(f'{name:<20} {bonds:>6} {staked_doli:>13.2f}D {status:<12} {short_key:<20}')

print()
print(f'Total producers: {len(producers)}')
print(f'Active: {sum(1 for p in producers if p.get(\"active\", p.get(\"is_active\", False)))}')
print(f'Total staked: {total_staked:.2f} DOLI')
" 2>/dev/null || {
  # Fallback: just dump raw JSON
  echo "Raw producer data:"
  echo "$producers" | python3 -m json.tool 2>/dev/null || echo "$producers"
}

echo ""

# Epoch info
epoch=$(rpc_call "getEpochInfo")
if [[ -n "$epoch" ]]; then
  python3 -c "
import json
data = json.loads('''${epoch}''')
r = data.get('result', {})
print(f'Epoch: {r.get(\"currentEpoch\", r.get(\"epoch\", \"?\"))}  Slot: {r.get(\"currentSlot\", r.get(\"slot\", \"?\"))}  Blocks in epoch: {r.get(\"blocksInEpoch\", r.get(\"blocks_in_epoch\", \"?\"))}')
" 2>/dev/null || true
fi
