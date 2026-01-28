#!/bin/bash
#
# DOLI Stress Test: 600 Producers Simulation
#
# Este script simula un escenario extremo con 600 productores en devnet.
# Basado en las fórmulas del whitepaper.
#
# ADVERTENCIA: Este script consume muchos recursos. Asegúrate de tener:
# - Al menos 64GB RAM (recomendado 128GB)
# - Al menos 32 cores CPU (recomendado 64+)
# - 500GB+ de espacio en disco SSD
#

set -e

# ==================== Configuración ====================

PRODUCER_COUNT=${PRODUCER_COUNT:-600}
BASE_P2P_PORT=${BASE_P2P_PORT:-50303}
BASE_RPC_PORT=${BASE_RPC_PORT:-28545}
BASE_DATA_DIR=${BASE_DATA_DIR:-~/.doli/stress-test}
DOLI_NODE=${DOLI_NODE:-./target/release/doli-node}
LOG_LEVEL=${LOG_LEVEL:-warn}

# Parámetros derivados del whitepaper para 600 productores
SLOT_DURATION=1        # 1 segundo por slot (extremo)
VDF_ITERATIONS=100000  # ~100ms VDF
BOOTSTRAP_BLOCKS=10    # Bootstrap muy corto

# ==================== Funciones ====================

print_header() {
    echo ""
    echo "╔══════════════════════════════════════════════════════════════════╗"
    echo "║              DOLI STRESS TEST: $PRODUCER_COUNT PRODUCTORES                  ║"
    echo "╠══════════════════════════════════════════════════════════════════╣"
    echo "║                                                                  ║"
    echo "║  Basado en el whitepaper DOLI v1.0                              ║"
    echo "║                                                                  ║"
    echo "║  Parámetros calculados:                                         ║"
    echo "║  • Probabilidad por slot: $(printf "%.4f" $(echo "scale=4; 1/$PRODUCER_COUNT" | bc))%                           ║"
    echo "║  • Tiempo entre bloques propios: ${PRODUCER_COUNT}s (~$(echo "$PRODUCER_COUNT/60" | bc) min)             ║"
    echo "║  • Eficiencia de red: $(printf "%.4f" $(echo "scale=4; 100/$PRODUCER_COUNT" | bc))%                            ║"
    echo "║  • Productores para 51% attack: $((PRODUCER_COUNT/2 + 1))                         ║"
    echo "║                                                                  ║"
    echo "╚══════════════════════════════════════════════════════════════════╝"
    echo ""
}

check_requirements() {
    echo "[*] Verificando requisitos..."

    # Verificar binario
    if [ ! -f "$DOLI_NODE" ]; then
        echo "[!] Error: No se encuentra $DOLI_NODE"
        echo "    Ejecuta: cargo build --release"
        exit 1
    fi

    # Verificar RAM
    if [ "$(uname)" == "Darwin" ]; then
        TOTAL_RAM_GB=$(sysctl -n hw.memsize | awk '{print int($1/1024/1024/1024)}')
    else
        TOTAL_RAM_GB=$(free -g | awk '/^Mem:/{print $2}')
    fi

    REQUIRED_RAM_GB=$((PRODUCER_COUNT / 5))  # ~200MB per node
    if [ "$TOTAL_RAM_GB" -lt "$REQUIRED_RAM_GB" ]; then
        echo "[!] Advertencia: RAM insuficiente"
        echo "    Disponible: ${TOTAL_RAM_GB}GB"
        echo "    Recomendado: ${REQUIRED_RAM_GB}GB para $PRODUCER_COUNT nodos"
        read -p "    ¿Continuar de todos modos? (y/n) " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            exit 1
        fi
    fi

    # Verificar ulimit
    CURRENT_ULIMIT=$(ulimit -n)
    REQUIRED_ULIMIT=$((PRODUCER_COUNT * 100))
    if [ "$CURRENT_ULIMIT" -lt "$REQUIRED_ULIMIT" ]; then
        echo "[!] Advertencia: ulimit muy bajo"
        echo "    Actual: $CURRENT_ULIMIT"
        echo "    Recomendado: $REQUIRED_ULIMIT"
        echo "    Ejecuta: ulimit -n $REQUIRED_ULIMIT"
    fi

    echo "[✓] Requisitos verificados"
}

cleanup() {
    echo ""
    echo "[*] Limpiando procesos..."
    pkill -f "doli-node.*stress-test" 2>/dev/null || true
    echo "[✓] Limpieza completada"
}

trap cleanup EXIT

launch_seed_node() {
    echo "[*] Lanzando nodo semilla..."

    mkdir -p "$BASE_DATA_DIR-0"

    $DOLI_NODE --network devnet run \
        --data-dir "$BASE_DATA_DIR-0" \
        --p2p-port "$BASE_P2P_PORT" \
        --rpc-port "$BASE_RPC_PORT" \
        --log-level "$LOG_LEVEL" \
        --producer &

    SEED_PID=$!
    echo "[✓] Nodo semilla lanzado (PID: $SEED_PID)"
    sleep 2

    BOOTSTRAP_ADDR="/ip4/127.0.0.1/tcp/$BASE_P2P_PORT"
}

launch_producer_batch() {
    local START=$1
    local END=$2
    local BATCH_NAME=$3

    echo "[*] Lanzando batch $BATCH_NAME: nodos $START-$END..."

    for i in $(seq "$START" "$END"); do
        local P2P_PORT=$((BASE_P2P_PORT + i))
        local RPC_PORT=$((BASE_RPC_PORT + i))
        local DATA_DIR="$BASE_DATA_DIR-$i"

        mkdir -p "$DATA_DIR"

        $DOLI_NODE --network devnet run \
            --data-dir "$DATA_DIR" \
            --p2p-port "$P2P_PORT" \
            --rpc-port "$RPC_PORT" \
            --bootstrap "$BOOTSTRAP_ADDR" \
            --log-level "$LOG_LEVEL" \
            --producer &

        # Pequeña pausa cada 10 nodos para no saturar
        if [ $((i % 10)) -eq 0 ]; then
            sleep 0.1
        fi
    done
}

monitor_network() {
    echo ""
    echo "[*] Monitoreando red..."
    echo "    (Presiona Ctrl+C para detener)"
    echo ""

    while true; do
        # Consultar estado del nodo semilla
        RESPONSE=$(curl -s -X POST "http://localhost:$BASE_RPC_PORT" \
            -H "Content-Type: application/json" \
            -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' 2>/dev/null || echo '{}')

        HEIGHT=$(echo "$RESPONSE" | grep -o '"best_height":[0-9]*' | cut -d: -f2 || echo "?")
        SLOT=$(echo "$RESPONSE" | grep -o '"best_slot":[0-9]*' | cut -d: -f2 || echo "?")

        NETWORK_RESPONSE=$(curl -s -X POST "http://localhost:$BASE_RPC_PORT" \
            -H "Content-Type: application/json" \
            -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":{},"id":1}' 2>/dev/null || echo '{}')

        PEERS=$(echo "$NETWORK_RESPONSE" | grep -o '"peer_count":[0-9]*' | cut -d: -f2 || echo "?")

        RUNNING_NODES=$(pgrep -f "doli-node.*stress-test" | wc -l | tr -d ' ')

        printf "\r[📊] Altura: %s | Slot: %s | Peers: %s | Nodos: %s/%s     " \
            "$HEIGHT" "$SLOT" "$PEERS" "$RUNNING_NODES" "$PRODUCER_COUNT"

        sleep 2
    done
}

# ==================== Main ====================

print_header
check_requirements

echo "[*] Preparando directorio de datos..."
rm -rf "$BASE_DATA_DIR"*
mkdir -p "$BASE_DATA_DIR"

launch_seed_node

# Lanzar en batches de 50 para no saturar
BATCH_SIZE=50
for batch_start in $(seq 1 $BATCH_SIZE $((PRODUCER_COUNT - 1))); do
    batch_end=$((batch_start + BATCH_SIZE - 1))
    if [ $batch_end -ge $PRODUCER_COUNT ]; then
        batch_end=$((PRODUCER_COUNT - 1))
    fi
    launch_producer_batch "$batch_start" "$batch_end" "$((batch_start / BATCH_SIZE + 1))"
    sleep 1
done

echo ""
echo "[✓] Todos los $PRODUCER_COUNT nodos lanzados"
echo ""

# Esperar a que se estabilice
echo "[*] Esperando estabilización de la red (30 segundos)..."
sleep 30

monitor_network
