#!/bin/bash
# auto-bond-others.sh — Auto-bond for stress-test producers (N13-N112)
# Runs at minute 5 each hour via cron. Adds bonds every 10 seconds, cycling through all producers.
# Bond unit: 0.1 DOLI

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
LOG="${SCRIPT_DIR}/auto-bond-others.log"

# Log rotation: trim to 512KB if over 1MB
MAX_LOG=1048576
if [ -f "$LOG" ] && [ "$(stat -f%z "$LOG" 2>/dev/null || echo 0)" -gt "$MAX_LOG" ]; then
    tail -c 524288 "$LOG" > "${LOG}.tmp" && mv "${LOG}.tmp" "$LOG"
fi

CLI="$HOME/repos/localdoli/target/release/doli"
KEYS_DIR="$HOME/mainnet/keys"
BOND_UNIT="0.1"
RPC="http://127.0.0.1:8501"
SLEEP_INTERVAL=10

if [ ! -x "$CLI" ]; then
    echo "$(date +%Y-%m-%d\ %H:%M:%S) ERROR: CLI not found at $CLI" >> "$LOG"
    exit 1
fi

echo "$(date +%Y-%m-%d\ %H:%M:%S) === auto-bond-others starting (producers 13-112) ===" >> "$LOG"

for NUM in $(seq 13 162); do
    KEY="${KEYS_DIR}/producer_${NUM}.json"
    [ ! -f "$KEY" ] && continue

    OUTPUT=$("$CLI" -w "$KEY" -r "$RPC" balance 2>&1)
    SPENDABLE=$(echo "$OUTPUT" | grep -i "Spendable:" | awk '{print $2}')
    BONDED=$(echo "$OUTPUT" | grep -i "Bonded:" | awk '{print $2}')
    if [ -z "$SPENDABLE" ]; then
        echo "$(date +%Y-%m-%d\ %H:%M:%S) P${NUM}: balance check failed" >> "$LOG"
        sleep "$SLEEP_INTERVAL"
        continue
    fi

    # Current bond count and max cap
    CURRENT_BONDS=$(awk "BEGIN {print int(${BONDED:-0} / $BOND_UNIT)}")
    MAX_BONDS=3000
    REMAINING=$((MAX_BONDS - CURRENT_BONDS))

    if [ "$REMAINING" -le 0 ]; then
        echo "$(date +%Y-%m-%d\ %H:%M:%S) P${NUM}: at max bonds (${CURRENT_BONDS}/${MAX_BONDS}), skipping" >> "$LOG"
        sleep "$SLEEP_INTERVAL"
        continue
    fi

    # Calculate bonds: spendable / 0.1, leave 0.01 for fees, cap at 100 per TX, respect max 3000
    BONDS=$(awk "BEGIN {b=int(($SPENDABLE - 0.01) / $BOND_UNIT); if(b>100) b=100; if(b>$REMAINING) b=$REMAINING; print b}")
    if [ "$BONDS" -gt 0 ]; then
        echo "$(date +%Y-%m-%d\ %H:%M:%S) P${NUM}: spendable=${SPENDABLE}, bonds=${CURRENT_BONDS}/${MAX_BONDS}, adding ${BONDS}" >> "$LOG"
        "$CLI" -w "$KEY" -r "$RPC" producer add-bond --count "$BONDS" >> "$LOG" 2>&1
    else
        echo "$(date +%Y-%m-%d\ %H:%M:%S) P${NUM}: spendable=${SPENDABLE}, bonds=${CURRENT_BONDS}/${MAX_BONDS}, skipping" >> "$LOG"
    fi

    sleep "$SLEEP_INTERVAL"
done

echo "$(date +%Y-%m-%d\ %H:%M:%S) === auto-bond-others finished ===" >> "$LOG"
