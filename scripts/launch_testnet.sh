#!/bin/bash
# DOLI Testnet - Two Producer Genesis Launch
# This script launches a local testnet with two producer nodes

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
TESTNET_DIR="/tmp/doli-testnet"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}   DOLI Testnet - Two Producer Launch   ${NC}"
echo -e "${BLUE}========================================${NC}"
echo

# Clean up previous testnet
echo -e "${YELLOW}Cleaning up previous testnet data...${NC}"
rm -rf "$TESTNET_DIR"
mkdir -p "$TESTNET_DIR/data1" "$TESTNET_DIR/data2" "$TESTNET_DIR/keys"

# Build the project in release mode
echo -e "${YELLOW}Building doli-node (release)...${NC}"
cd "$PROJECT_ROOT"
cargo build --release -p doli-node 2>&1 | tail -5

NODE_BIN="$PROJECT_ROOT/target/release/doli-node"
if [ ! -f "$NODE_BIN" ]; then
    echo -e "${RED}Error: doli-node binary not found${NC}"
    exit 1
fi

# Generate producer keys using a helper program
echo -e "${YELLOW}Generating producer keys...${NC}"

# Create a small Rust program to generate keys
cat > "$TESTNET_DIR/keygen.rs" << 'KEYGEN_EOF'
use doli_crypto::KeyPair;
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: keygen <output_file>");
        std::process::exit(1);
    }

    let keypair = KeyPair::generate();
    let private_key_hex = hex::encode(keypair.private_key().as_bytes());
    let public_key_hex = hex::encode(keypair.public_key().as_bytes());

    let wallet_json = format!(r#"{{
  "version": 1,
  "addresses": [
    {{
      "address": "tdoli1{}",
      "public_key": "{}",
      "private_key": "{}"
    }}
  ]
}}"#, &public_key_hex[..16], public_key_hex, private_key_hex);

    std::fs::write(&args[1], wallet_json).expect("Failed to write wallet file");
    println!("Generated key: {}", &public_key_hex[..16]);
}
KEYGEN_EOF

# Create Cargo.toml for keygen
cat > "$TESTNET_DIR/Cargo.toml" << CARGO_EOF
[package]
name = "keygen"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "keygen"
path = "keygen.rs"

[dependencies]
doli-crypto = { path = "$PROJECT_ROOT/doli-crypto" }
hex = "0.4"
CARGO_EOF

# Build and run keygen
cd "$TESTNET_DIR"
cargo build --release 2>/dev/null

echo -e -n "  Producer 1: "
./target/release/keygen "$TESTNET_DIR/keys/producer1.json"
echo -e -n "  Producer 2: "
./target/release/keygen "$TESTNET_DIR/keys/producer2.json"

# Extract public keys for display
PRODUCER1_KEY=$(cat "$TESTNET_DIR/keys/producer1.json" | grep public_key | head -1 | cut -d'"' -f4)
PRODUCER2_KEY=$(cat "$TESTNET_DIR/keys/producer2.json" | grep public_key | head -1 | cut -d'"' -f4)

echo
echo -e "${GREEN}Producer Keys Generated:${NC}"
echo -e "  Node 1: ${PRODUCER1_KEY:0:16}..."
echo -e "  Node 2: ${PRODUCER2_KEY:0:16}..."
echo

# Determine ports
NODE1_P2P=40303
NODE1_RPC=18545
NODE1_METRICS=9091

NODE2_P2P=40304
NODE2_RPC=18546
NODE2_METRICS=9092

# Create launch script for node 1 (seed node)
cat > "$TESTNET_DIR/start_node1.sh" << NODE1_EOF
#!/bin/bash
echo "Starting Node 1 (Seed Node)..."
$NODE_BIN \\
    --data-dir "$TESTNET_DIR/data1" \\
    --network devnet \\
    run \\
    --producer \\
    --producer-key "$TESTNET_DIR/keys/producer1.json" \\
    --p2p-port $NODE1_P2P \\
    --rpc-port $NODE1_RPC \\
    --metrics-port $NODE1_METRICS \\
    --no-auto-update \\
    2>&1 | tee "$TESTNET_DIR/node1.log"
NODE1_EOF
chmod +x "$TESTNET_DIR/start_node1.sh"

# Create launch script for node 2 (connects to node 1)
cat > "$TESTNET_DIR/start_node2.sh" << NODE2_EOF
#!/bin/bash
echo "Starting Node 2 (connects to Node 1)..."
sleep 2  # Wait for node 1 to start
$NODE_BIN \\
    --data-dir "$TESTNET_DIR/data2" \\
    --network devnet \\
    run \\
    --producer \\
    --producer-key "$TESTNET_DIR/keys/producer2.json" \\
    --p2p-port $NODE2_P2P \\
    --rpc-port $NODE2_RPC \\
    --metrics-port $NODE2_METRICS \\
    --bootstrap "/ip4/127.0.0.1/tcp/$NODE1_P2P" \\
    --no-auto-update \\
    2>&1 | tee "$TESTNET_DIR/node2.log"
NODE2_EOF
chmod +x "$TESTNET_DIR/start_node2.sh"

# Create a combined launcher
cat > "$TESTNET_DIR/launch_both.sh" << BOTH_EOF
#!/bin/bash
trap 'kill \$(jobs -p) 2>/dev/null' EXIT

echo "=========================================="
echo "   DOLI Devnet - Two Producers Running"
echo "=========================================="
echo
echo "Node 1: P2P=$NODE1_P2P, RPC=$NODE1_RPC"
echo "Node 2: P2P=$NODE2_P2P, RPC=$NODE2_RPC"
echo
echo "Press Ctrl+C to stop both nodes"
echo "=========================================="
echo

# Start node 1 in background
$TESTNET_DIR/start_node1.sh &
NODE1_PID=\$!

# Start node 2 in background
$TESTNET_DIR/start_node2.sh &
NODE2_PID=\$!

# Wait for both
wait
BOTH_EOF
chmod +x "$TESTNET_DIR/launch_both.sh"

# Create status check script
cat > "$TESTNET_DIR/check_status.sh" << STATUS_EOF
#!/bin/bash
echo "=== Node 1 Status ==="
curl -s http://127.0.0.1:$NODE1_RPC -X POST \\
    -H "Content-Type: application/json" \\
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' | jq .

echo
echo "=== Node 2 Status ==="
curl -s http://127.0.0.1:$NODE2_RPC -X POST \\
    -H "Content-Type: application/json" \\
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' | jq .
STATUS_EOF
chmod +x "$TESTNET_DIR/check_status.sh"

echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}   Testnet Setup Complete!${NC}"
echo -e "${GREEN}========================================${NC}"
echo
echo -e "Configuration:"
echo -e "  Network:  ${BLUE}devnet${NC} (5 second slots)"
echo -e "  Data dir: ${BLUE}$TESTNET_DIR${NC}"
echo
echo -e "Node 1 (Seed):"
echo -e "  Data:     ${BLUE}$TESTNET_DIR/data1${NC}"
echo -e "  P2P:      ${BLUE}$NODE1_P2P${NC}"
echo -e "  RPC:      ${BLUE}$NODE1_RPC${NC}"
echo -e "  Key:      ${BLUE}${PRODUCER1_KEY:0:16}...${NC}"
echo
echo -e "Node 2:"
echo -e "  Data:     ${BLUE}$TESTNET_DIR/data2${NC}"
echo -e "  P2P:      ${BLUE}$NODE2_P2P${NC}"
echo -e "  RPC:      ${BLUE}$NODE2_RPC${NC}"
echo -e "  Key:      ${BLUE}${PRODUCER2_KEY:0:16}...${NC}"
echo -e "  Bootstrap: /ip4/127.0.0.1/tcp/$NODE1_P2P"
echo
echo -e "${YELLOW}To launch the testnet:${NC}"
echo -e "  ${GREEN}$TESTNET_DIR/launch_both.sh${NC}"
echo
echo -e "${YELLOW}To check status (in another terminal):${NC}"
echo -e "  ${GREEN}$TESTNET_DIR/check_status.sh${NC}"
echo
echo -e "${YELLOW}To view logs:${NC}"
echo -e "  ${GREEN}tail -f $TESTNET_DIR/node1.log${NC}"
echo -e "  ${GREEN}tail -f $TESTNET_DIR/node2.log${NC}"
echo

# Ask if user wants to launch now
read -p "Launch testnet now? [y/N] " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    exec "$TESTNET_DIR/launch_both.sh"
fi
