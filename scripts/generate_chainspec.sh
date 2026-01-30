#!/bin/bash
#
# Generate chainspec JSON from wallet files
#
# This script reads producer wallet JSON files and generates a chainspec
# with the correct public keys. This avoids manual pubkey copying errors.
#
# Usage:
#   ./scripts/generate_chainspec.sh <network> <wallet_dir> [output_file]
#
# Examples:
#   ./scripts/generate_chainspec.sh testnet ~/.doli/testnet/producer_keys
#   ./scripts/generate_chainspec.sh mainnet ~/.doli/mainnet/producer_keys mainnet.json
#
# The script expects wallet files named producer_1.json, producer_2.json, etc.
#

set -e

NETWORK="${1:-testnet}"
WALLET_DIR="${2:-}"
OUTPUT="${3:-}"

# Validate arguments
if [ -z "$WALLET_DIR" ]; then
    echo "Usage: $0 <network> <wallet_dir> [output_file]"
    echo ""
    echo "Arguments:"
    echo "  network     Network type: mainnet, testnet, or devnet"
    echo "  wallet_dir  Directory containing producer_N.json wallet files"
    echo "  output_file Optional output file (default: stdout)"
    echo ""
    echo "Example:"
    echo "  $0 testnet ~/.doli/testnet/producer_keys testnet.json"
    exit 1
fi

if [ ! -d "$WALLET_DIR" ]; then
    echo "Error: Wallet directory not found: $WALLET_DIR"
    exit 1
fi

# Network-specific parameters
case "$NETWORK" in
    mainnet)
        GENESIS_TIMESTAMP=1769904000
        GENESIS_MESSAGE="Time is the only fair currency. 01/Feb/2026"
        CHAIN_NAME="DOLI Mainnet"
        SLOT_DURATION=10
        SLOTS_PER_EPOCH=360
        BOND_AMOUNT=100000000000
        ;;
    testnet)
        GENESIS_TIMESTAMP=1769738400
        GENESIS_MESSAGE="DOLI Testnet v2 Genesis - Time is the only fair currency"
        CHAIN_NAME="DOLI Testnet"
        SLOT_DURATION=10
        SLOTS_PER_EPOCH=360
        BOND_AMOUNT=100000000000
        ;;
    devnet)
        GENESIS_TIMESTAMP=0
        GENESIS_MESSAGE="DOLI Devnet - Development and Testing"
        CHAIN_NAME="DOLI Devnet"
        SLOT_DURATION=5
        SLOTS_PER_EPOCH=60
        BOND_AMOUNT=100000000
        ;;
    *)
        echo "Error: Unknown network: $NETWORK"
        echo "Valid networks: mainnet, testnet, devnet"
        exit 1
        ;;
esac

# Find wallet files
WALLET_FILES=$(ls "$WALLET_DIR"/producer_*.json 2>/dev/null | sort -V)
if [ -z "$WALLET_FILES" ]; then
    echo "Warning: No producer_*.json files found in $WALLET_DIR"
    WALLET_FILES=""
fi

# Extract pubkeys from wallet files
PRODUCERS_JSON=""
FIRST=true
for wallet in $WALLET_FILES; do
    if [ -f "$wallet" ]; then
        # Extract producer name from filename (producer_1.json -> producer_1)
        NAME=$(basename "$wallet" .json)

        # Extract public key from wallet JSON
        PUBKEY=$(cat "$wallet" | python3 -c "import sys,json; w=json.load(sys.stdin); print(w['addresses'][0]['public_key'])" 2>/dev/null)

        if [ -z "$PUBKEY" ]; then
            echo "Warning: Could not extract pubkey from $wallet" >&2
            continue
        fi

        # Validate pubkey length
        if [ ${#PUBKEY} -ne 64 ]; then
            echo "Warning: Invalid pubkey length in $wallet: ${#PUBKEY} chars (expected 64)" >&2
            continue
        fi

        # Add comma separator
        if [ "$FIRST" = true ]; then
            FIRST=false
        else
            PRODUCERS_JSON="$PRODUCERS_JSON,"
        fi

        PRODUCERS_JSON="$PRODUCERS_JSON
    {
      \"name\": \"$NAME\",
      \"public_key\": \"$PUBKEY\",
      \"bond_count\": 1
    }"

        echo "Found: $NAME -> $PUBKEY" >&2
    fi
done

# Generate the chainspec JSON
CHAINSPEC=$(cat <<EOF
{
  "name": "$CHAIN_NAME",
  "id": "$NETWORK",
  "network": "$(echo $NETWORK | sed 's/./\U&/')",
  "genesis": {
    "timestamp": $GENESIS_TIMESTAMP,
    "message": "$GENESIS_MESSAGE",
    "initial_reward": 100000000
  },
  "consensus": {
    "slot_duration": $SLOT_DURATION,
    "slots_per_epoch": $SLOTS_PER_EPOCH,
    "bond_amount": $BOND_AMOUNT
  },
  "genesis_producers": [$PRODUCERS_JSON
  ],
  "maintainer_keys": []
}
EOF
)

# Output
if [ -n "$OUTPUT" ]; then
    echo "$CHAINSPEC" > "$OUTPUT"
    echo "" >&2
    echo "Chainspec written to: $OUTPUT" >&2
    echo "Producers found: $(echo "$WALLET_FILES" | wc -w | tr -d ' ')" >&2
else
    echo "$CHAINSPEC"
fi
