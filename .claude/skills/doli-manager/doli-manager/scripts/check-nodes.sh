#!/usr/bin/env bash
# Check height/hash/slot of all 6 DOLI mainnet nodes + archiver.
# Usage: ./check-nodes.sh [rpc_host]
# Default: queries omegacortex.ai nodes (N1/N2/N6/Archiver local, N3/N4/N5 remote)

set -euo pipefail

HOST="${1:-omegacortex.ai}"

echo "=== DOLI Mainnet Node Status ==="
echo ""

# Omegacortex nodes: N1=8545, N2=8546, N6=8547, Archiver=8548
declare -A OMEGA_NODES=( [8545]="N1" [8546]="N2" [8547]="N6" [8548]="Archiver" )
for port in 8545 8546 8547 8548; do
    label="${OMEGA_NODES[$port]}"
    result=$(ssh "ilozada@${HOST}" "curl -s -X POST http://127.0.0.1:${port} \
        -H 'Content-Type: application/json' \
        -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}' 2>/dev/null" \
        | jq -r '.result | "height=\(.bestHeight) slot=\(.bestSlot) hash=\(.bestHash[0:16])"' 2>/dev/null \
        || echo "UNREACHABLE")
    echo "${label}: ${result}"
done

# N3 (147.93.84.44 — direct from Mac, NOT via omegacortex)
result=$(ssh -p 50790 "ilozada@147.93.84.44" \
    "curl -s -X POST http://127.0.0.1:8545 \
    -H 'Content-Type: application/json' \
    -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}'" 2>/dev/null \
    | jq -r '.result | "height=\(.bestHeight) slot=\(.bestSlot) hash=\(.bestHash[0:16])"' 2>/dev/null \
    || echo "UNREACHABLE")
echo "N3: ${result}"

# N4 (72.60.115.209 — direct from Mac, NOT via omegacortex)
result=$(ssh -p 50790 "ilozada@72.60.115.209" \
    "curl -s -X POST http://127.0.0.1:8545 \
    -H 'Content-Type: application/json' \
    -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}'" 2>/dev/null \
    | jq -r '.result | "height=\(.bestHeight) slot=\(.bestSlot) hash=\(.bestHash[0:16])"' 2>/dev/null \
    || echo "UNREACHABLE")
echo "N4: ${result}"

# N5 (72.60.70.166 — direct from Mac, NOT via omegacortex)
result=$(ssh -p 50790 "ilozada@72.60.70.166" \
    "curl -s -X POST http://127.0.0.1:8545 \
    -H 'Content-Type: application/json' \
    -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}'" 2>/dev/null \
    | jq -r '.result | "height=\(.bestHeight) slot=\(.bestSlot) hash=\(.bestHash[0:16])"' 2>/dev/null \
    || echo "UNREACHABLE")
echo "N5: ${result}"

echo ""
echo "All producer nodes should show same height and hash prefix."
