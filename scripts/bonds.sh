#!/usr/bin/env bash
# Usage: scripts/bonds.sh [mainnet|testnet|all]
# Shows bond details for all producers via RPC getProducers from seed node.
set -euo pipefail

AI1="ilozada@72.60.228.233"

query_producers() {
  local bin_path="$1" port="$2" prefix="$3" count="$4"

  local data
  data=$(ssh -o ConnectTimeout=5 "$AI1" "curl -sf --max-time 5 -X POST http://127.0.0.1:${port} \
    -H 'Content-Type: application/json' \
    -d '{\"jsonrpc\":\"2.0\",\"method\":\"getProducers\",\"params\":{},\"id\":1}'" 2>/dev/null)

  if [[ -z "$data" ]]; then
    echo "❌ Seed node not responding on port ${port}"
    return
  fi

  printf "%-5s %8s %14s %10s %s\n" "Node" "Bonds" "Bonded (DOLI)" "Status" "PubKey (short)"
  printf "%-5s %8s %14s %10s %s\n" "-----" "--------" "--------------" "----------" "--------------"

  # Map producers by matching key files
  for N in $(seq 1 "$count"); do
    local server
    if (( N % 2 == 1 )); then server="$AI1"; else server="ilozada@187.124.95.188"; fi

    local key_path
    if [[ "$prefix" == "N" ]]; then
      key_path="/mainnet/n${N}/keys/producer.json"
    else
      key_path="/testnet/nt${N}/keys/producer.json"
    fi

    local pubkey
    pubkey=$(ssh -o ConnectTimeout=5 "$server" "python3 -c \"import json; print(json.load(open('${key_path}'))['public_key'])\"" 2>/dev/null) || pubkey=""

    if [[ -z "$pubkey" ]]; then
      printf "%-5s %8s %14s %10s %s\n" "${prefix}${N}" "?" "?" "no key" "-"
      continue
    fi

    local info
    info=$(echo "$data" | python3 -c "
import sys, json
data = json.load(sys.stdin)
producers = data.get('result', [])
pk = '${pubkey}'
for p in producers:
    if p.get('publicKey','') == pk:
        bonds = p.get('bondCount', 0)
        amount = p.get('bondAmount', 0) / 1e8
        status = p.get('status', '?')
        print(f'{bonds}|{amount:.2f}|{status}|{pk[:16]}')
        break
else:
    print('0|0.00|not registered|${pubkey:0:16}')
" 2>/dev/null)

    local bonds amount status short_key
    IFS='|' read -r bonds amount status short_key <<< "$info"
    printf "%-5s %8s %14s %10s %s\n" "${prefix}${N}" "$bonds" "$amount" "$status" "$short_key"
  done
  echo ""
}

do_mainnet() {
  echo "⛏ Mainnet Bonds"
  query_producers "/mainnet/bin/doli" 8500 "N" 12
}

do_testnet() {
  echo "🧪 Testnet Bonds"
  query_producers "/testnet/bin/doli" 18500 "NT" 12
}

case "${1:-all}" in
  mainnet) do_mainnet ;;
  testnet) do_testnet ;;
  all)     do_mainnet; do_testnet ;;
  *)       echo "Usage: $0 [mainnet|testnet|all]"; exit 1 ;;
esac
