#!/bin/bash
# DOLI Devnet - Interactive Producer Deployment Script
#
# This script interactively deploys N new producer nodes to a running devnet.
# It handles the complete workflow:
#   1. Creates wallets for new producers
#   2. Funds each producer from a selected genesis wallet
#   3. Registers each producer with the specified number of bonds
#   4. Starts each producer node with unique ports
#
# Prerequisites:
#   - A running devnet (started with `doli-node devnet start`)
#   - Genesis producer wallets in ~/.doli/devnet/keys/
#   - Built binaries (cargo build --release)
#
# Usage:
#   ./scripts/deploy_producers.sh
#
# The script will prompt for:
#   - Number of nodes to deploy
#   - DOLI amount to fund each producer
#   - Number of bonds per producer
#   - Wait time between registrations
#   - Source wallet selection from genesis producers

set -e

# =============================================================================
# Configuration
# =============================================================================

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
DEVNET_DIR="$HOME/.doli/devnet"
KEYS_DIR="$DEVNET_DIR/keys"
DATA_DIR="$DEVNET_DIR/data"
LOGS_DIR="$DEVNET_DIR/logs"
PIDS_DIR="$DEVNET_DIR/pids"
CHAINSPEC="$DEVNET_DIR/chainspec.json"

# Binaries
NODE_BIN="$PROJECT_ROOT/target/release/doli-node"
CLI_BIN="$PROJECT_ROOT/target/release/doli"

# Network defaults (devnet)
BASE_P2P_PORT=50303
BASE_RPC_PORT=28545
BASE_METRICS_PORT=9090
RPC_ENDPOINT="http://127.0.0.1:$BASE_RPC_PORT"
BOOTSTRAP="/ip4/127.0.0.1/tcp/$BASE_P2P_PORT"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
NC='\033[0m'

# =============================================================================
# Utility Functions
# =============================================================================

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[OK]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_step() {
    echo -e "${CYAN}[STEP]${NC} $1"
}

prompt() {
    echo -en "${MAGENTA}$1${NC}"
    read -r REPLY
    echo "$REPLY"
}

# Check if a port is in use
port_in_use() {
    lsof -i ":$1" >/dev/null 2>&1
}

# Kill process on port
kill_port() {
    local port=$1
    local pid=$(lsof -ti ":$port" 2>/dev/null)
    if [ -n "$pid" ]; then
        kill "$pid" 2>/dev/null && log_warn "Killed process on port $port (PID $pid)"
        return 0
    fi
    return 1
}

# Get pubkey hash from wallet (required for sending)
get_pubkey_hash() {
    local wallet=$1
    $CLI_BIN -w "$wallet" info 2>/dev/null | grep "Pubkey Hash (32-byte):" | sed 's/.*: //'
}

# Get wallet balance
get_balance() {
    local wallet=$1
    $CLI_BIN -r "$RPC_ENDPOINT" -w "$wallet" balance 2>/dev/null | grep -oE '[0-9]+\.[0-9]+' | head -1 || echo "0"
}

# Get next available node index
get_next_node_index() {
    local max_index=-1
    shopt -s nullglob
    for f in "$KEYS_DIR"/producer_*.json; do
        local idx=$(basename "$f" | sed 's/producer_//' | sed 's/.json//')
        if [[ "$idx" =~ ^[0-9]+$ ]] && [ "$idx" -gt "$max_index" ]; then
            max_index=$idx
        fi
    done
    shopt -u nullglob
    echo $((max_index + 1))
}

# Count existing genesis producers
count_genesis_producers() {
    shopt -s nullglob
    local files=("$KEYS_DIR"/producer_*.json)
    shopt -u nullglob
    echo "${#files[@]}"
}

# =============================================================================
# Validation Functions
# =============================================================================

validate_prerequisites() {
    log_step "Validating prerequisites..."

    # Check binaries exist
    if [ ! -f "$NODE_BIN" ]; then
        log_error "doli-node binary not found at $NODE_BIN"
        log_info "Run: cargo build --release"
        exit 1
    fi

    if [ ! -f "$CLI_BIN" ]; then
        log_error "doli-cli binary not found at $CLI_BIN"
        log_info "Run: cargo build --release"
        exit 1
    fi

    # Check devnet directory exists
    if [ ! -d "$DEVNET_DIR" ]; then
        log_error "Devnet directory not found: $DEVNET_DIR"
        log_info "Initialize devnet first: doli-node devnet init --nodes 5"
        exit 1
    fi

    # Check chainspec exists
    if [ ! -f "$CHAINSPEC" ]; then
        log_error "Chainspec not found: $CHAINSPEC"
        log_info "Initialize devnet first: doli-node devnet init --nodes 5"
        exit 1
    fi

    # Check at least one node is running (RPC responsive)
    if ! curl -s "$RPC_ENDPOINT" -X POST -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' | grep -q "result"; then
        log_error "No running devnet node found at $RPC_ENDPOINT"
        log_info "Start devnet first: doli-node devnet start"
        exit 1
    fi

    # Check genesis producer keys exist
    local genesis_count=$(count_genesis_producers)
    if [ "$genesis_count" -eq 0 ]; then
        log_error "No genesis producer keys found in $KEYS_DIR"
        exit 1
    fi

    log_success "All prerequisites met (found $genesis_count genesis producers)"
}

# =============================================================================
# Interactive Prompts
# =============================================================================

prompt_node_count() {
    echo
    echo -e "${CYAN}┌─────────────────────────────────────────────┐${NC}"
    echo -e "${CYAN}│       DOLI Producer Deployment Script       │${NC}"
    echo -e "${CYAN}└─────────────────────────────────────────────┘${NC}"
    echo

    while true; do
        echo -en "${MAGENTA}How many producer nodes to deploy? [1-50]: ${NC}"
        read -r NODE_COUNT
        if [[ "$NODE_COUNT" =~ ^[0-9]+$ ]] && [ "$NODE_COUNT" -ge 1 ] && [ "$NODE_COUNT" -le 50 ]; then
            break
        fi
        log_error "Please enter a number between 1 and 50"
    done
}

prompt_fund_amount() {
    echo
    log_info "For devnet: Bond unit = 1 DOLI (each bond requires 1 DOLI)"
    log_info "Recommended: Fund at least (bonds + 1) DOLI to cover fees"
    echo

    while true; do
        echo -en "${MAGENTA}How much DOLI to fund each producer? [e.g., 10]: ${NC}"
        read -r FUND_AMOUNT
        if [[ "$FUND_AMOUNT" =~ ^[0-9]+(\.[0-9]+)?$ ]] && [ "$(echo "$FUND_AMOUNT > 0" | bc)" -eq 1 ]; then
            break
        fi
        log_error "Please enter a positive number"
    done
}

prompt_bond_count() {
    echo
    local max_bonds=$(echo "$FUND_AMOUNT - 1" | bc | cut -d'.' -f1)
    if [ "$max_bonds" -lt 1 ]; then
        max_bonds=1
        log_warn "With $FUND_AMOUNT DOLI, you can only stake 1 bond (need fees)"
    fi

    while true; do
        echo -en "${MAGENTA}How many bonds per producer? [1-$max_bonds]: ${NC}"
        read -r BOND_COUNT
        if [[ "$BOND_COUNT" =~ ^[0-9]+$ ]] && [ "$BOND_COUNT" -ge 1 ]; then
            local required=$(echo "$BOND_COUNT + 0.1" | bc)
            if [ "$(echo "$FUND_AMOUNT >= $required" | bc)" -eq 1 ]; then
                break
            else
                log_error "Not enough funds: $BOND_COUNT bonds require at least $required DOLI"
            fi
        else
            log_error "Please enter a positive integer"
        fi
    done
}

prompt_wait_time() {
    echo
    log_info "Wait time between registrations prevents double-spend errors"
    log_info "Minimum: 12 seconds (one block confirmation)"
    log_info "Note: Funding always waits at least 12s regardless of this setting"
    echo

    while true; do
        echo -en "${MAGENTA}Seconds to wait between registrations? [12-60, default=12]: ${NC}"
        read -r WAIT_TIME
        if [ -z "$WAIT_TIME" ]; then
            WAIT_TIME=12
            break
        fi
        if [[ "$WAIT_TIME" =~ ^[0-9]+$ ]] && [ "$WAIT_TIME" -ge 12 ] && [ "$WAIT_TIME" -le 60 ]; then
            break
        fi
        log_error "Please enter a number between 12 and 60"
    done
}

prompt_source_wallet() {
    echo
    log_info "Available genesis producer wallets:"
    echo

    local i=0
    declare -a WALLETS
    for wallet in "$KEYS_DIR"/producer_*.json; do
        if [ -f "$wallet" ]; then
            local name=$(basename "$wallet" .json)
            local balance=$(get_balance "$wallet")
            local pubkey_hash=$(get_pubkey_hash "$wallet")
            echo -e "  ${CYAN}[$i]${NC} $name - Balance: ${GREEN}$balance DOLI${NC}"
            echo -e "      Pubkey Hash: ${pubkey_hash:0:16}...${pubkey_hash: -16}"
            WALLETS[$i]="$wallet"
            ((i++))
        fi
    done

    echo
    local max_idx=$((i - 1))

    while true; do
        echo -en "${MAGENTA}Select source wallet [0-$max_idx]: ${NC}"
        read -r WALLET_IDX
        if [[ "$WALLET_IDX" =~ ^[0-9]+$ ]] && [ "$WALLET_IDX" -ge 0 ] && [ "$WALLET_IDX" -le "$max_idx" ]; then
            SOURCE_WALLET="${WALLETS[$WALLET_IDX]}"
            break
        fi
        log_error "Please enter a number between 0 and $max_idx"
    done

    # Verify source wallet has enough funds
    local source_balance=$(get_balance "$SOURCE_WALLET")
    local total_needed=$(echo "$FUND_AMOUNT * $NODE_COUNT" | bc)

    echo
    log_info "Source wallet: $(basename "$SOURCE_WALLET" .json)"
    log_info "Balance: $source_balance DOLI"
    log_info "Total needed: $total_needed DOLI ($FUND_AMOUNT x $NODE_COUNT nodes)"

    if [ "$(echo "$source_balance < $total_needed" | bc)" -eq 1 ]; then
        log_error "Insufficient balance! Need $total_needed DOLI but have $source_balance DOLI"
        exit 1
    fi

    log_success "Sufficient balance confirmed"
}

confirm_deployment() {
    echo
    echo -e "${CYAN}┌─────────────────────────────────────────────┐${NC}"
    echo -e "${CYAN}│           Deployment Summary                │${NC}"
    echo -e "${CYAN}└─────────────────────────────────────────────┘${NC}"
    echo
    echo -e "  Nodes to deploy:    ${GREEN}$NODE_COUNT${NC}"
    echo -e "  Fund per node:      ${GREEN}$FUND_AMOUNT DOLI${NC}"
    echo -e "  Bonds per node:     ${GREEN}$BOND_COUNT${NC}"
    echo -e "  Wait between regs:  ${GREEN}${WAIT_TIME}s${NC}"
    echo -e "  Source wallet:      ${GREEN}$(basename "$SOURCE_WALLET" .json)${NC}"
    echo -e "  Starting index:     ${GREEN}$START_INDEX${NC}"
    echo -e "  Ending index:       ${GREEN}$((START_INDEX + NODE_COUNT - 1))${NC}"
    echo

    echo -en "${MAGENTA}Proceed with deployment? [y/N]: ${NC}"
    read -r CONFIRM
    if [[ ! "$CONFIRM" =~ ^[Yy]$ ]]; then
        log_info "Deployment cancelled"
        exit 0
    fi
}

# =============================================================================
# Deployment Functions
# =============================================================================

clean_ports() {
    log_step "Checking and cleaning ports for nodes $START_INDEX to $((START_INDEX + NODE_COUNT - 1))..."

    local cleaned=0
    for ((i = START_INDEX; i < START_INDEX + NODE_COUNT; i++)); do
        local p2p=$((BASE_P2P_PORT + i))
        local rpc=$((BASE_RPC_PORT + i))
        local metrics=$((BASE_METRICS_PORT + i))

        for port in $p2p $rpc $metrics; do
            if port_in_use "$port"; then
                kill_port "$port"
                ((cleaned++))
            fi
        done
    done

    if [ "$cleaned" -gt 0 ]; then
        log_warn "Cleaned $cleaned ports, waiting 2 seconds..."
        sleep 2
    else
        log_success "All ports are free"
    fi
}

create_wallets() {
    log_step "Creating wallets for $NODE_COUNT new producers..."

    mkdir -p "$KEYS_DIR"

    for ((i = START_INDEX; i < START_INDEX + NODE_COUNT; i++)); do
        local wallet="$KEYS_DIR/producer_$i.json"
        if [ -f "$wallet" ]; then
            log_warn "Wallet already exists: producer_$i.json (skipping)"
        else
            $CLI_BIN -w "$wallet" new -n "producer_$i" >/dev/null 2>&1
            log_success "Created wallet: producer_$i.json"
        fi
    done
}

fund_producers() {
    log_step "Funding $NODE_COUNT producers with $FUND_AMOUNT DOLI each..."

    for ((i = START_INDEX; i < START_INDEX + NODE_COUNT; i++)); do
        local wallet="$KEYS_DIR/producer_$i.json"
        local pubkey_hash=$(get_pubkey_hash "$wallet")

        if [ -z "$pubkey_hash" ]; then
            log_error "Failed to get pubkey hash for producer_$i"
            exit 1
        fi

        echo -en "  Funding producer_$i... "

        if ! $CLI_BIN -r "$RPC_ENDPOINT" -w "$SOURCE_WALLET" send "$pubkey_hash" "$FUND_AMOUNT" >/dev/null 2>&1; then
            log_error "Failed to send funds to producer_$i"
            log_info "Pubkey hash: $pubkey_hash"
            exit 1
        fi

        echo -e "${GREEN}OK${NC}"

        # Always wait at least 12 seconds between transactions from same wallet
        local actual_wait=$WAIT_TIME
        if [ "$actual_wait" -lt 12 ]; then
            actual_wait=12
        fi

        if [ "$i" -lt "$((START_INDEX + NODE_COUNT - 1))" ]; then
            echo -en "  Waiting ${actual_wait}s for confirmation... "
            sleep "$actual_wait"
            echo "done"
        fi
    done

    log_success "All producers funded"
}

register_producers() {
    log_step "Registering $NODE_COUNT producers with $BOND_COUNT bonds each..."

    for ((i = START_INDEX; i < START_INDEX + NODE_COUNT; i++)); do
        local wallet="$KEYS_DIR/producer_$i.json"

        echo -en "  Registering producer_$i... "

        if ! $CLI_BIN -r "$RPC_ENDPOINT" -w "$wallet" producer register -b "$BOND_COUNT" >/dev/null 2>&1; then
            log_error "Failed to register producer_$i"
            exit 1
        fi

        echo -e "${GREEN}OK${NC}"

        if [ "$WAIT_TIME" -gt 0 ] && [ "$i" -lt "$((START_INDEX + NODE_COUNT - 1))" ]; then
            echo -en "  Waiting ${WAIT_TIME}s for confirmation... "
            sleep "$WAIT_TIME"
            echo "done"
        fi
    done

    log_success "All producers registered"
}

start_nodes() {
    log_step "Starting $NODE_COUNT producer nodes..."

    mkdir -p "$DATA_DIR" "$LOGS_DIR" "$PIDS_DIR"

    for ((i = START_INDEX; i < START_INDEX + NODE_COUNT; i++)); do
        local wallet="$KEYS_DIR/producer_$i.json"
        local data_dir="$DATA_DIR/node$i"
        local log_file="$LOGS_DIR/node$i.log"
        local pid_file="$PIDS_DIR/node$i.pid"

        local p2p=$((BASE_P2P_PORT + i))
        local rpc=$((BASE_RPC_PORT + i))
        local metrics=$((BASE_METRICS_PORT + i))

        mkdir -p "$data_dir"

        echo -en "  Starting node $i (P2P: $p2p, RPC: $rpc, Metrics: $metrics)... "

        $NODE_BIN \
            --network devnet \
            --data-dir "$data_dir" \
            run \
            --producer \
            --producer-key "$wallet" \
            --p2p-port "$p2p" \
            --rpc-port "$rpc" \
            --metrics-port "$metrics" \
            --bootstrap "$BOOTSTRAP" \
            --chainspec "$CHAINSPEC" \
            --no-dht \
            --yes \
            > "$log_file" 2>&1 &

        local pid=$!
        echo "$pid" > "$pid_file"

        # Wait for node to initialize (2 seconds)
        sleep 2

        if kill -0 "$pid" 2>/dev/null; then
            echo -e "${GREEN}OK${NC} (PID: $pid)"
        else
            # Double-check after a brief moment
            sleep 1
            if kill -0 "$pid" 2>/dev/null; then
                echo -e "${GREEN}OK${NC} (PID: $pid)"
            else
                echo -e "${RED}FAILED${NC}"
                log_error "Node $i failed to start. Check log: $log_file"
                tail -5 "$log_file" 2>/dev/null || true
            fi
        fi
    done

    log_success "All nodes started"
}

verify_deployment() {
    log_step "Verifying deployment (waiting 10s for nodes to sync)..."
    sleep 10

    echo
    echo -e "${CYAN}Node Status:${NC}"
    echo

    local success_count=0
    local fail_count=0

    for ((i = START_INDEX; i < START_INDEX + NODE_COUNT; i++)); do
        local rpc=$((BASE_RPC_PORT + i))
        local pid_file="$PIDS_DIR/node$i.pid"

        # Check if process is running
        local running="no"
        if [ -f "$pid_file" ]; then
            local pid=$(cat "$pid_file")
            if kill -0 "$pid" 2>/dev/null; then
                running="yes"
            fi
        fi

        # Check RPC
        local height="N/A"
        local slot="N/A"
        local rpc_resp=$(curl -s "http://127.0.0.1:$rpc" -X POST -H "Content-Type: application/json" \
            -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' 2>/dev/null)

        if echo "$rpc_resp" | grep -q "result"; then
            height=$(echo "$rpc_resp" | grep -o '"bestHeight":[0-9]*' | cut -d':' -f2)
            slot=$(echo "$rpc_resp" | grep -o '"bestSlot":[0-9]*' | cut -d':' -f2)
            ((success_count++))
            echo -e "  Node $i: ${GREEN}Running${NC} | Height: $height | Slot: $slot"
        else
            ((fail_count++))
            if [ "$running" = "yes" ]; then
                echo -e "  Node $i: ${YELLOW}Starting...${NC} (RPC not ready)"
            else
                echo -e "  Node $i: ${RED}Not running${NC}"
            fi
        fi
    done

    echo
    echo -e "${CYAN}Summary:${NC}"
    echo -e "  Deployed: $NODE_COUNT nodes (producer_$START_INDEX to producer_$((START_INDEX + NODE_COUNT - 1)))"
    echo -e "  Running:  ${GREEN}$success_count${NC} | Failed: ${RED}$fail_count${NC}"

    if [ "$fail_count" -gt 0 ]; then
        log_warn "Some nodes may need more time to start. Check logs in $LOGS_DIR"
    fi
}

show_management_tips() {
    echo
    echo -e "${CYAN}┌─────────────────────────────────────────────┐${NC}"
    echo -e "${CYAN}│           Management Commands               │${NC}"
    echo -e "${CYAN}└─────────────────────────────────────────────┘${NC}"
    echo
    echo "Check node status:"
    echo "  curl -s http://127.0.0.1:\$RPC_PORT -X POST -H 'Content-Type: application/json' \\"
    echo "    -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":[],\"id\":1}' | jq"
    echo
    echo "View node logs:"
    echo "  tail -f $LOGS_DIR/node\$N.log"
    echo
    echo "Stop a node:"
    echo "  kill \$(cat $PIDS_DIR/node\$N.pid)"
    echo
    echo "List producers:"
    echo "  $CLI_BIN -r $RPC_ENDPOINT producer list"
    echo
    echo "Check producer balance:"
    echo "  $CLI_BIN -r $RPC_ENDPOINT -w $KEYS_DIR/producer_\$N.json balance"
    echo
}

# =============================================================================
# Main
# =============================================================================

main() {
    echo
    validate_prerequisites

    # Get next available node index
    START_INDEX=$(get_next_node_index)

    # Interactive prompts
    prompt_node_count
    prompt_fund_amount
    prompt_bond_count
    prompt_wait_time
    prompt_source_wallet
    confirm_deployment

    echo
    echo -e "${CYAN}═══════════════════════════════════════════════${NC}"
    echo -e "${CYAN}  Starting Deployment                          ${NC}"
    echo -e "${CYAN}═══════════════════════════════════════════════${NC}"
    echo

    # Execute deployment steps
    clean_ports
    create_wallets
    fund_producers
    register_producers
    start_nodes
    verify_deployment
    show_management_tips

    echo
    log_success "Deployment complete!"
    echo
}

# Run main function
main "$@"
