#!/usr/bin/env bash
# Check height/hash/slot of all 5 DOLI mainnet nodes.
# Usage: ./check-nodes.sh [rpc_host]
# Default: queries omegacortex.ai nodes (N1/N2/N3 local, N4/N5 remote)

set -euo pipefail

HOST="${1:-omegacortex.ai}"

echo "=== DOLI Mainnet Node Status ==="
echo ""

# N1, N2, N3 (omegacortex - ports 8545, 8546, 8547)
for port in 8545 8546 8547; do
    node_num=$((port - 8544))
    result=$(ssh "ilozada@${HOST}" "curl -s -X POST http://127.0.0.1:${port} \
        -H 'Content-Type: application/json' \
        -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}' 2>/dev/null" \
        | jq -r '.result | "height=\(.bestHeight) slot=\(.bestSlot) hash=\(.bestHash[0:16])"' 2>/dev/null \
        || echo "UNREACHABLE")
    echo "N${node_num}: ${result}"
done

# N4 (72.60.70.166 via jump)
result=$(ssh "ilozada@${HOST}" "ssh -p 50790 ilozada@72.60.70.166 \
    'curl -s -X POST http://127.0.0.1:8545 \
    -H \"Content-Type: application/json\" \
    -d \"{\\\"jsonrpc\\\":\\\"2.0\\\",\\\"method\\\":\\\"getChainInfo\\\",\\\"params\\\":{},\\\"id\\\":1}\"' 2>/dev/null" \
    | jq -r '.result | "height=\(.bestHeight) slot=\(.bestSlot) hash=\(.bestHash[0:16])"' 2>/dev/null \
    || echo "UNREACHABLE")
echo "N4: ${result}"

# N5 (72.60.115.209 via jump)
result=$(ssh "ilozada@${HOST}" "ssh -p 50790 ilozada@72.60.115.209 \
    'curl -s -X POST http://127.0.0.1:8545 \
    -H \"Content-Type: application/json\" \
    -d \"{\\\"jsonrpc\\\":\\\"2.0\\\",\\\"method\\\":\\\"getChainInfo\\\",\\\"params\\\":{},\\\"id\\\":1}\"' 2>/dev/null" \
    | jq -r '.result | "height=\(.bestHeight) slot=\(.bestSlot) hash=\(.bestHash[0:16])"' 2>/dev/null \
    || echo "UNREACHABLE")
echo "N5: ${result}"

echo ""
echo "All nodes should show same height and hash prefix."
