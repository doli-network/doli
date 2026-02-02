# DOLI Coin Blockchain - Justfile
# VDF-based Proof of Time (PoT) consensus blockchain
# All commands run inside Nix environment

# Default recipe: show available commands
default:
    @just --list

# ============================================================================
# ALIASES (short versions of common commands)
# ============================================================================

alias b := build
alias br := build-release
alias c := check
alias t := test
alias l := lint
alias f := fmt
alias d := doc
alias q := quick
alias w := watch
alias wt := watch-test

# Node aliases
alias nm := node-mainnet
alias nt := node-testnet
alias nd := node-devnet
alias nr := node-release

# Test aliases
alias tc := test-core
alias tv := test-vdf
alias ts := test-storage
alias tn := test-network
alias ti := test-integration
alias te := test-e2e

# Deploy aliases
alias d1 := deploy-single
alias d2 := deploy-two
alias d3 := deploy-three
alias dk := deploy-kill

# ============================================================================
# NIX ENVIRONMENT
# ============================================================================

# Enter Nix development shell (interactive)
nix-shell:
    nix --extra-experimental-features "nix-command flakes" develop

# Run command inside Nix environment
[private]
nix-run cmd:
    nix --extra-experimental-features "nix-command flakes" develop --command bash -c "{{cmd}}"

# ============================================================================
# BUILD COMMANDS
# ============================================================================

# Build all crates (debug)
build:
    @just nix-run "cargo build 2>&1" | grep -iE "(compiling|finished|error|warning)" | head -20

# Build all crates (release)
build-release:
    @just nix-run "cargo build --release 2>&1" | grep -iE "(compiling|finished|error|warning)" | head -20

# Build specific crate
build-crate crate:
    @just nix-run "cargo build -p {{crate}} 2>&1" | grep -iE "(compiling|finished|error|warning)" | head -20

# Check all crates without building
check:
    @just nix-run "cargo check 2>&1" | grep -iE "(checking|finished|error|warning)" | head -20

# Clean build artifacts
clean:
    @just nix-run "cargo clean"

# ============================================================================
# TESTING
# ============================================================================

# Run all tests
test:
    @just nix-run "cargo test 2>&1" | grep -iE "(running|passed|failed|error|test result)" | awk '!seen[$0]++' | head -30

# Run tests for specific crate
test-crate crate:
    @just nix-run "cargo test -p {{crate}} 2>&1" | grep -iE "(running|passed|failed|error|test result)" | awk '!seen[$0]++' | head -30

# Run a single test by name
test-single crate test_name:
    @just nix-run "cargo test -p {{crate}} {{test_name}} 2>&1" | grep -iE "(running|passed|failed|error|test result)" | head -20

# Run tests for core crates
test-core:
    @just test-crate doli-core

test-crypto:
    @just test-crate crypto

test-vdf:
    @just test-crate vdf

test-storage:
    @just test-crate storage

test-network:
    @just test-crate network

test-mempool:
    @just test-crate mempool

test-rpc:
    @just test-crate rpc

# Run integration tests
test-integration:
    @just nix-run "cargo test --manifest-path testing/integration/Cargo.toml 2>&1" | grep -iE "(running|passed|failed|error|test result)" | head -30

# Run e2e tests
test-e2e:
    @just nix-run "cargo test --manifest-path testing/e2e/Cargo.toml 2>&1" | grep -iE "(running|passed|failed|error|test result)" | head -30

# ============================================================================
# FUZZ TESTING
# ============================================================================

# Run block deserialization fuzzer
fuzz-block:
    cd testing/fuzz && cargo +nightly fuzz run fuzz_block_deserialize

# Run transaction deserialization fuzzer
fuzz-tx:
    cd testing/fuzz && cargo +nightly fuzz run fuzz_tx_deserialize

# Run VDF verification fuzzer
fuzz-vdf:
    cd testing/fuzz && cargo +nightly fuzz run fuzz_vdf_verify

# List available fuzz targets
fuzz-list:
    cd testing/fuzz && cargo +nightly fuzz list

# ============================================================================
# BENCHMARKS
# ============================================================================

# Run benchmarks
bench:
    @just nix-run "cargo bench --manifest-path testing/benchmarks/Cargo.toml 2>&1" | head -50

# ============================================================================
# LINTING & FORMATTING
# ============================================================================

# Run clippy lints
lint:
    @just nix-run "cargo clippy 2>&1" | grep -iE "(checking|warning|error|finished)" | awk '!seen[$0]++' | head -30

# Run clippy with strict warnings (deny all warnings)
lint-strict:
    @just nix-run "cargo clippy -- -D warnings 2>&1" | grep -iE "(checking|warning|error|finished)" | awk '!seen[$0]++' | head -30

# Check code formatting
fmt-check:
    @just nix-run "cargo fmt --check 2>&1"

# Format code
fmt:
    @just nix-run "cargo fmt"

# Run all quality checks (lint + format check + test)
qa: lint fmt-check test
    @echo "✅ All quality checks passed"

# ============================================================================
# DOCUMENTATION
# ============================================================================

# Generate API documentation
doc:
    @just nix-run "cargo doc --workspace --no-deps 2>&1" | grep -iE "(documenting|finished|error|warning)" | head -20

# Generate and open API documentation
doc-open:
    @just nix-run "cargo doc --workspace --no-deps --open"

# ============================================================================
# NODE OPERATIONS
# ============================================================================

# Run node on mainnet
node-mainnet:
    @just nix-run "cargo run -p doli-node -- run"

# Run node on testnet
node-testnet:
    @just nix-run "cargo run -p doli-node -- --network testnet run"

# Run node on devnet (local development)
node-devnet:
    @just nix-run "cargo run -p doli-node -- --network devnet run"

# Run node with custom config
node-config config_path:
    @just nix-run "cargo run -p doli-node -- --config {{config_path}} run"

# Run node in release mode (mainnet)
node-release:
    @just nix-run "cargo run --release -p doli-node -- run"

# ============================================================================
# CLI WALLET
# ============================================================================

# Create new wallet
wallet-new:
    @just nix-run "cargo run -p doli-cli -- wallet new"

# Check wallet balance
wallet-balance address:
    @just nix-run "cargo run -p doli-cli -- wallet balance {{address}}"

# Run CLI command
cli *args:
    @just nix-run "cargo run -p doli-cli -- {{args}}"

# ============================================================================
# NETWORK SCRIPTS
# ============================================================================

# Launch testnet using script
launch-testnet:
    bash scripts/launch_testnet.sh

# Run stress test (600 nodes simulation)
stress-test:
    bash scripts/stress_test_600.sh

# ============================================================================
# DEVELOPMENT HELPERS
# ============================================================================

# Watch for changes and rebuild
watch:
    @just nix-run "cargo watch -x build"

# Watch for changes and run tests
watch-test:
    @just nix-run "cargo watch -x test"

# Show crate dependency tree
deps:
    @just nix-run "cargo tree --depth 2"

# Show workspace members
workspace:
    @just nix-run "cargo metadata --format-version 1 2>/dev/null | jq -r '.workspace_members[]'"

# Update dependencies
update:
    @just nix-run "cargo update"

# Audit dependencies for security vulnerabilities
audit:
    @just nix-run "cargo audit 2>&1" | head -50

# ============================================================================
# RELEASE
# ============================================================================

# Build release binaries for all targets
release-build:
    @just nix-run "cargo build --release 2>&1" | grep -iE "(compiling|finished|error|warning)" | head -20

# Create release artifacts
release-artifacts: release-build
    @mkdir -p dist
    @cp target/release/doli-node dist/ 2>/dev/null || echo "doli-node not found"
    @cp target/release/doli-cli dist/ 2>/dev/null || echo "doli-cli not found"
    @echo "✅ Release artifacts copied to dist/"

# ============================================================================
# QUICK RECIPES
# ============================================================================

# Full development cycle: format, lint, test
dev: fmt lint test
    @echo "✅ Development checks complete"

# Quick check: just verify it compiles
quick: check
    @echo "✅ Quick check passed"

# CI pipeline simulation
ci: fmt-check lint-strict test doc
    @echo "✅ CI pipeline passed"

# ============================================================================
# NETWORK INFO
# ============================================================================

# Display network configuration table
networks:
    @echo "┌─────────┬─────┬───────┬──────────┬──────────┬────────────────┐"
    @echo "│ Network │ ID  │ Slot  │ P2P Port │ RPC Port │ Address Prefix │"
    @echo "├─────────┼─────┼───────┼──────────┼──────────┼────────────────┤"
    @echo "│ Mainnet │ 1   │ 60s   │ 30303    │ 8545     │ doli           │"
    @echo "│ Testnet │ 2   │ 10s   │ 40303    │ 18545    │ tdoli          │"
    @echo "│ Devnet  │ 99  │ 5s    │ 50303    │ 28545    │ ddoli          │"
    @echo "└─────────┴─────┴───────┴──────────┴──────────┴────────────────┘"

# Display architecture diagram
arch:
    @echo "DOLI Crate Dependency Flow:"
    @echo ""
    @echo "bins/node (doli-node)          bins/cli (doli-cli)"
    @echo "    │                              │"
    @echo "    ├─→ network ─┐                 │"
    @echo "    ├─→ rpc ─────┤                 │"
    @echo "    ├─→ mempool ─┤                 │"
    @echo "    ├─→ storage ─┤                 │"
    @echo "    ├─→ updater ─┤                 │"
    @echo "    │            ▼                 │"
    @echo "    └─────────→ core ←─────────────┘"
    @echo "                 │"
    @echo "                 ▼"
    @echo "         ┌───────┴───────┐"
    @echo "         ▼               ▼"
    @echo "      crypto            vdf"

# Display time structure info
time-info:
    @echo "DOLI Time Structure:"
    @echo "  • Slot    = 10 seconds (mainnet/testnet) / 1s (devnet)"
    @echo "  • Epoch   = 60 slots (1 hour on mainnet)"
    @echo "  • Era     = 2,102,400 slots (~4 years) - triggers reward halving"

# ============================================================================
# NODE DEPLOYMENT (Testing)
# ============================================================================

# Deploy single devnet node (no producer)
deploy-single:
    @echo "Starting single devnet node..."
    @just nix-run "cargo run -p doli-node -- --network devnet --data-dir /tmp/doli-single run --p2p-port 50303 --rpc-port 28545 --no-dht --no-auto-update"

# Deploy single producer node on devnet
deploy-producer:
    @echo "Starting single producer node on devnet..."
    @mkdir -p /tmp/doli-producer
    @just nix-run "cargo run -p doli-cli -- -w /tmp/doli-producer/wallet.json new" 2>/dev/null || true
    @just nix-run "cargo run -p doli-node -- --network devnet --data-dir /tmp/doli-producer run --producer --producer-key /tmp/doli-producer/wallet.json --p2p-port 50303 --rpc-port 28545 --no-dht --no-auto-update"

# Deploy two-node devnet for sync testing
deploy-two:
    @echo "Launching two-node devnet..."
    @bash scripts/launch_testnet.sh

# Deploy three-node devnet cluster
deploy-three:
    @echo "Starting 3-node devnet cluster..."
    @mkdir -p /tmp/doli-cluster/{node1,node2,node3}
    @echo "Node 1: P2P=50303 RPC=28545 (seed)"
    @echo "Node 2: P2P=50304 RPC=28546"
    @echo "Node 3: P2P=50305 RPC=28547"
    @echo ""
    @echo "Run in separate terminals:"
    @echo "  just deploy-node1"
    @echo "  just deploy-node2"
    @echo "  just deploy-node3"

# Deploy node 1 (seed node for cluster)
deploy-node1:
    @just nix-run "cargo run -p doli-node -- --network devnet --data-dir /tmp/doli-cluster/node1 run --p2p-port 50303 --rpc-port 28545 --no-dht --no-auto-update"

# Deploy node 2 (connects to node 1)
deploy-node2:
    @sleep 2
    @just nix-run "cargo run -p doli-node -- --network devnet --data-dir /tmp/doli-cluster/node2 run --p2p-port 50304 --rpc-port 28546 --bootstrap /ip4/127.0.0.1/tcp/50303 --no-dht --no-auto-update"

# Deploy node 3 (connects to node 1)
deploy-node3:
    @sleep 2
    @just nix-run "cargo run -p doli-node -- --network devnet --data-dir /tmp/doli-cluster/node3 run --p2p-port 50305 --rpc-port 28547 --bootstrap /ip4/127.0.0.1/tcp/50303 --no-dht --no-auto-update"

# Kill all running doli nodes
deploy-kill:
    @echo "Stopping all DOLI nodes..."
    @pkill -f "doli-node" 2>/dev/null || echo "No nodes running"
    @echo "Done"

# Clean deployment data directories
deploy-clean:
    @echo "Cleaning deployment data..."
    @rm -rf /tmp/doli-single /tmp/doli-producer /tmp/doli-cluster /tmp/doli-testnet
    @echo "Done"

# Run stress test with configurable producers (default: 10)
deploy-stress count="10":
    @echo "Starting stress test with {{count}} producers..."
    @PRODUCER_COUNT={{count}} bash scripts/stress_test_600.sh

# ============================================================================
# WALLET OPERATIONS
# ============================================================================

# Create a new test wallet
wallet-create name="test":
    @mkdir -p /tmp/doli-wallets
    @just nix-run "cargo run -p doli-cli -- -w /tmp/doli-wallets/{{name}}.json new --name {{name}}"

# Show wallet info
wallet-info wallet="~/.doli/wallet.json":
    @just nix-run "cargo run -p doli-cli -- -w {{wallet}} info"

# List wallet addresses
wallet-list wallet="~/.doli/wallet.json":
    @just nix-run "cargo run -p doli-cli -- -w {{wallet}} addresses"

# Generate new address in wallet
wallet-addr wallet="~/.doli/wallet.json" label="":
    @just nix-run "cargo run -p doli-cli -- -w {{wallet}} address --label '{{label}}'"

# Check balance (uses devnet RPC by default)
wallet-bal address="" rpc="http://127.0.0.1:28545":
    @just nix-run "cargo run -p doli-cli -- -r {{rpc}} balance {{address}}"

# Send coins
wallet-send to amount fee="" rpc="http://127.0.0.1:28545":
    @just nix-run "cargo run -p doli-cli -- -r {{rpc}} send {{to}} {{amount}} {{fee}}"

# Sign a message
wallet-sign message wallet="~/.doli/wallet.json":
    @just nix-run "cargo run -p doli-cli -- -w {{wallet}} sign '{{message}}'"

# Show transaction history
wallet-history limit="10" rpc="http://127.0.0.1:28545":
    @just nix-run "cargo run -p doli-cli -- -r {{rpc}} history --limit {{limit}}"

# ============================================================================
# RPC QUERIES
# ============================================================================

# Get chain info from node
rpc-chain rpc="http://127.0.0.1:28545":
    @curl -s -X POST {{rpc}} -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' | jq .

# Get network info from node
rpc-network rpc="http://127.0.0.1:28545":
    @curl -s -X POST {{rpc}} -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":[],"id":1}' | jq .

# Get block by height
rpc-block height rpc="http://127.0.0.1:28545":
    @curl -s -X POST {{rpc}} -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"getBlock","params":{"height":{{height}}},"id":1}' | jq .

# Get mempool info
rpc-mempool rpc="http://127.0.0.1:28545":
    @curl -s -X POST {{rpc}} -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"getMempoolInfo","params":[],"id":1}' | jq .

# Get producer set
rpc-producers rpc="http://127.0.0.1:28545":
    @curl -s -X POST {{rpc}} -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"getProducers","params":[],"id":1}' | jq .

# Get balance for address
rpc-balance address rpc="http://127.0.0.1:28545":
    @curl -s -X POST {{rpc}} -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"getBalance","params":{"address":"{{address}}"},"id":1}' | jq .

# Get UTXOs for address
rpc-utxos address rpc="http://127.0.0.1:28545":
    @curl -s -X POST {{rpc}} -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"getUtxos","params":{"address":"{{address}}"},"id":1}' | jq .

# Ping all cluster nodes
rpc-ping-all:
    @echo "Node 1 (28545):" && curl -s -X POST http://127.0.0.1:28545 -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' | jq -r '.result.best_height // "offline"'
    @echo "Node 2 (28546):" && curl -s -X POST http://127.0.0.1:28546 -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' | jq -r '.result.best_height // "offline"'
    @echo "Node 3 (28547):" && curl -s -X POST http://127.0.0.1:28547 -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' | jq -r '.result.best_height // "offline"'

# ============================================================================
# INTEGRATION TEST RUNNERS
# ============================================================================

# Run specific integration test
test-int name:
    @just nix-run "cargo test --manifest-path testing/integration/Cargo.toml {{name}} 2>&1" | grep -iE "(running|passed|failed|error|test result)" | head -20

# Run two-node sync test
test-sync:
    @just test-int two_node_sync

# Run reorg test
test-reorg:
    @just test-int reorg_test

# Run partition heal test
test-partition:
    @just test-int partition_heal

# Run mempool stress test
test-mempool-stress:
    @just test-int mempool_stress

# Run malicious peer test
test-malicious:
    @just test-int malicious_peer

# Run bond stacking test
test-bond:
    @just test-int bond_stacking

# Run two producer PoP test
test-pop:
    @just test-int two_producer_pop

# Run attack reorg test
test-attack:
    @just test-int attack_reorg

# ============================================================================
# E2E TEST RUNNERS
# ============================================================================

# Run specific e2e test
test-e2e-name name:
    @just nix-run "cargo test --manifest-path testing/e2e/Cargo.toml {{name}} 2>&1" | grep -iE "(running|passed|failed|error|test result)" | head -20

# Run full cycle e2e test
test-full-cycle:
    @just test-e2e-name full_cycle

# Run wallet flow e2e test
test-wallet-flow:
    @just test-e2e-name wallet_flow

# ============================================================================
# LOG MONITORING
# ============================================================================

# Tail node logs (assumes standard log location)
logs-node1:
    @tail -f /tmp/doli-testnet/node1.log 2>/dev/null || echo "Log file not found. Start testnet first with: just deploy-two"

logs-node2:
    @tail -f /tmp/doli-testnet/node2.log 2>/dev/null || echo "Log file not found. Start testnet first with: just deploy-two"

# Follow all testnet logs
logs-all:
    @tail -f /tmp/doli-testnet/*.log 2>/dev/null || echo "Log files not found. Start testnet first with: just deploy-two"

# ============================================================================
# PROFILING & DEBUGGING
# ============================================================================

# Run with debug logging
run-debug:
    @just nix-run "RUST_LOG=debug cargo run -p doli-node -- --network devnet --log-level debug run --no-dht --no-auto-update"

# Run with trace logging
run-trace:
    @just nix-run "RUST_LOG=trace cargo run -p doli-node -- --network devnet --log-level trace run --no-dht --no-auto-update"

# Profile VDF performance
profile-vdf:
    @just nix-run "cargo run --release -p doli-node -- --network devnet run --no-dht --no-auto-update" &
    @sleep 5
    @echo "VDF profiling: Check metrics at http://localhost:9090/metrics"

# ============================================================================
# CODE QUALITY EXTENDED
# ============================================================================

# Run clippy with all features
lint-all:
    @just nix-run "cargo clippy --all-features 2>&1" | grep -iE "(checking|warning|error|finished)" | awk '!seen[$0]++' | head -40

# Run clippy on specific crate
lint-crate crate:
    @just nix-run "cargo clippy -p {{crate}} 2>&1" | grep -iE "(checking|warning|error|finished)" | awk '!seen[$0]++' | head -30

# Check for unused dependencies
check-deps:
    @just nix-run "cargo +nightly udeps 2>&1" | head -50 || echo "Install cargo-udeps: cargo install cargo-udeps"

# Check for outdated dependencies
check-outdated:
    @just nix-run "cargo outdated 2>&1" | head -30 || echo "Install cargo-outdated: cargo install cargo-outdated"

# Run all security checks
security: audit
    @echo "Security audit complete"

# ============================================================================
# GENESIS & CHAIN TOOLS
# ============================================================================

# Initialize new devnet data directory
init-devnet:
    @just nix-run "cargo run -p doli-node -- --network devnet init"

# Initialize new testnet data directory
init-testnet:
    @just nix-run "cargo run -p doli-node -- --network testnet init"

# Export blocks to file
export-blocks path from="0" to="":
    @just nix-run "cargo run -p doli-node -- export {{path}} --from {{from}} {{to}}"

# Import blocks from file
import-blocks path:
    @just nix-run "cargo run -p doli-node -- import {{path}}"

# ============================================================================
# PRODUCER COMMANDS
# ============================================================================

# Check producer status
producer-status pubkey="" rpc="http://127.0.0.1:28545":
    @just nix-run "cargo run -p doli-cli -- -r {{rpc}} producer status {{pubkey}}"

# Show producer registration info
producer-info:
    @echo "Producer Registration Requirements:"
    @echo "  1. Complete VDF proof (~10 minutes)"
    @echo "  2. Bond: 1,000 DOLI (Era 0)"
    @echo "  3. Submit registration transaction"
    @echo ""
    @echo "See: docs/BECOMING_A_PRODUCER.md"

# ============================================================================
# VERSION & INFO
# ============================================================================

# Show version info
version:
    @just nix-run "cargo run -p doli-node -- --version" 2>/dev/null || cargo pkgid -p doli-node | cut -d'#' -f2

# Show all crate versions
versions:
    @just nix-run "cargo metadata --format-version 1 2>/dev/null | jq -r '.packages[] | select(.source == null) | \"\(.name): \(.version)\"'"

# Show help for node CLI
help-node:
    @just nix-run "cargo run -p doli-node -- --help"

# Show help for wallet CLI
help-cli:
    @just nix-run "cargo run -p doli-cli -- --help"

# ============================================================================
# BACKEND DEVELOPMENT
# ============================================================================

# Build release binary (for devnet testing)
backend-build:
    @just nix-run "cargo build --release 2>&1" | grep -iE "(compiling|finished|error|warning)" | head -30

# ============================================================================
# DEVNET MANAGEMENT
# ============================================================================

# Initialize devnet with N nodes (cleans existing first)
devnet-init nodes:
    @just nix-run "./target/release/doli-node devnet stop 2>/dev/null || true"
    @just nix-run "./target/release/doli-node devnet clean 2>/dev/null || true"
    @just nix-run "./target/release/doli-node devnet init --nodes {{nodes}}"

# Start all devnet nodes
devnet-start:
    @just nix-run "./target/release/doli-node devnet start 2>&1" | tail -15

# Stop all devnet nodes
devnet-stop:
    @just nix-run "./target/release/doli-node devnet stop"

# Check devnet status
devnet-status:
    @just nix-run "./target/release/doli-node devnet status 2>&1" | grep -E "^(Node|---|-|[0-9])" | head -30

# Clean devnet data
devnet-clean:
    @just nix-run "./target/release/doli-node devnet clean"

# ============================================================================
# CONVENIENCE COMBOS
# ============================================================================

# Full rebuild and test
rebuild: clean build test
    @echo "✅ Full rebuild complete"

# Pre-commit checks (format, lint, test)
pre-commit: fmt lint test
    @echo "✅ Pre-commit checks passed"

# Release preparation (full QA + docs)
release-prep: ci release-build doc
    @echo "✅ Release preparation complete"
