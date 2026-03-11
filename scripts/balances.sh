#!/usr/bin/env bash
# Usage: scripts/balances.sh [mainnet|testnet|all]
# Shows complete balance table for all nodes using CLI (not RPC).
# All work happens on remote Linux servers via SSH.
set -euo pipefail

AI1="ilozada@72.60.228.233"
AI2="ilozada@187.124.95.188"

print_header() {
  printf "%-5s %12s %12s %12s %12s\n" "Node" "Spendable" "Bonded" "Immature" "Total"
  printf "%-5s %12s %12s %12s %12s\n" "-----" "------------" "------------" "------------" "------------"
}

# Single SSH call per server. All parsing on remote Linux (grep -P safe).
mainnet_ai1() {
  ssh -o ConnectTimeout=10 "$AI1" 'for N in 1 3 5 7 9 11; do
    BAL=$(/mainnet/bin/doli --wallet /mainnet/n${N}/keys/producer.json -r http://127.0.0.1:$((8500+N)) balance 2>&1) || BAL=""
    if [ -z "$BAL" ]; then
      printf "N%-4s %12s %12s %12s %12s\n" "$N" "OFFLINE" "-" "-" "-"
    else
      SP=$(echo "$BAL" | grep -oP "Spendable:\s+\K[\d.]+" || echo "0")
      BO=$(echo "$BAL" | grep -oP "Bonded:\s+\K[\d.]+" || echo "0")
      IM=$(echo "$BAL" | grep -oP "Immature:\s+\K[\d.]+" || echo "0")
      TO=$(echo "$BAL" | grep -oP "Total:\s+\K[\d.]+" || echo "0")
      printf "N%-4s %12s %12s %12s %12s\n" "$N" "$SP" "$BO" "$IM" "$TO"
    fi
  done'
}

mainnet_ai2() {
  ssh -o ConnectTimeout=10 "$AI2" 'for N in 2 4 6 8 10 12; do
    BAL=$(/mainnet/bin/doli --wallet /mainnet/n${N}/keys/producer.json -r http://127.0.0.1:$((8500+N)) balance 2>&1) || BAL=""
    if [ -z "$BAL" ]; then
      printf "N%-4s %12s %12s %12s %12s\n" "$N" "OFFLINE" "-" "-" "-"
    else
      SP=$(echo "$BAL" | grep -oP "Spendable:\s+\K[\d.]+" || echo "0")
      BO=$(echo "$BAL" | grep -oP "Bonded:\s+\K[\d.]+" || echo "0")
      IM=$(echo "$BAL" | grep -oP "Immature:\s+\K[\d.]+" || echo "0")
      TO=$(echo "$BAL" | grep -oP "Total:\s+\K[\d.]+" || echo "0")
      printf "N%-4s %12s %12s %12s %12s\n" "$N" "$SP" "$BO" "$IM" "$TO"
    fi
  done'
}

testnet_ai1() {
  ssh -o ConnectTimeout=10 "$AI1" 'for N in 1 3 5 7 9 11; do
    BAL=$(/testnet/bin/doli -n testnet --wallet /testnet/nt${N}/keys/producer.json -r http://127.0.0.1:$((18500+N)) balance 2>&1) || BAL=""
    if [ -z "$BAL" ]; then
      printf "NT%-3s %12s %12s %12s %12s\n" "$N" "OFFLINE" "-" "-" "-"
    else
      SP=$(echo "$BAL" | grep -oP "Spendable:\s+\K[\d.]+" || echo "0")
      BO=$(echo "$BAL" | grep -oP "Bonded:\s+\K[\d.]+" || echo "0")
      IM=$(echo "$BAL" | grep -oP "Immature:\s+\K[\d.]+" || echo "0")
      TO=$(echo "$BAL" | grep -oP "Total:\s+\K[\d.]+" || echo "0")
      printf "NT%-3s %12s %12s %12s %12s\n" "$N" "$SP" "$BO" "$IM" "$TO"
    fi
  done'
}

testnet_ai2() {
  ssh -o ConnectTimeout=10 "$AI2" 'for N in 2 4 6 8 10 12; do
    BAL=$(/testnet/bin/doli -n testnet --wallet /testnet/nt${N}/keys/producer.json -r http://127.0.0.1:$((18500+N)) balance 2>&1) || BAL=""
    if [ -z "$BAL" ]; then
      printf "NT%-3s %12s %12s %12s %12s\n" "$N" "OFFLINE" "-" "-" "-"
    else
      SP=$(echo "$BAL" | grep -oP "Spendable:\s+\K[\d.]+" || echo "0")
      BO=$(echo "$BAL" | grep -oP "Bonded:\s+\K[\d.]+" || echo "0")
      IM=$(echo "$BAL" | grep -oP "Immature:\s+\K[\d.]+" || echo "0")
      TO=$(echo "$BAL" | grep -oP "Total:\s+\K[\d.]+" || echo "0")
      printf "NT%-3s %12s %12s %12s %12s\n" "$N" "$SP" "$BO" "$IM" "$TO"
    fi
  done'
}

do_mainnet() {
  echo "⛏ Mainnet (N1-N12)"
  print_header
  local tmp1 tmp2
  tmp1=$(mktemp); tmp2=$(mktemp)
  mainnet_ai1 > "$tmp1" &
  mainnet_ai2 > "$tmp2" &
  wait
  sort -t'N' -k1.2 -n "$tmp1" "$tmp2"
  rm -f "$tmp1" "$tmp2"
  echo ""
}

do_testnet() {
  echo "🧪 Testnet (NT1-NT12)"
  print_header
  local tmp1 tmp2
  tmp1=$(mktemp); tmp2=$(mktemp)
  testnet_ai1 > "$tmp1" &
  testnet_ai2 > "$tmp2" &
  wait
  sort -k1.3 -n "$tmp1" "$tmp2"
  rm -f "$tmp1" "$tmp2"
  echo ""
}

case "${1:-all}" in
  mainnet) do_mainnet ;;
  testnet) do_testnet ;;
  all)     do_mainnet; do_testnet ;;
  *)       echo "Usage: $0 [mainnet|testnet|all]"; exit 1 ;;
esac
