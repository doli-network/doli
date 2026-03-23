#!/bin/bash
# auto-bond.sh — Auto-bond spendable DOLI for genesis producers N1-N5
# Runs hourly via cron. Target: 3000 bonds each.
# Bond unit: 0.1 DOLI (local devnet)

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
LOG="${SCRIPT_DIR}/auto-bond.log"

# Log rotation: trim to 512KB if over 1MB
MAX_LOG=1048576
if [ -f "$LOG" ] && [ "$(stat -c%s "$LOG" 2>/dev/null || echo 0)" -gt "$MAX_LOG" ]; then
    tail -c 524288 "$LOG" > "${LOG}.tmp" && mv "${LOG}.tmp" "$LOG"
fi

CLI="$HOME/repos/doli/target/release/doli"
KEYS_DIR="$HOME/mainnet/keys"
BOND_UNIT="0.1"

if [ ! -x "$CLI" ]; then
    echo "$(date +%Y-%m-%d\ %H:%M:%S) ERROR: CLI not found at $CLI" >> "$LOG"
    exit 1
fi

for NUM in 1 2 3 4 5; do
    KEY="${KEYS_DIR}/producer_${NUM}.json"
    PORT=$((8500 + NUM))
    [ ! -f "$KEY" ] && continue

    OUTPUT=$("$CLI" -w "$KEY" -r "http://127.0.0.1:${PORT}" balance 2>&1)
    SPENDABLE=$(echo "$OUTPUT" | grep -i "Spendable:" | awk '{print $2}')
    BONDED=$(echo "$OUTPUT" | grep -i "Bonded:" | awk '{print $2}')
    [ -z "$SPENDABLE" ] && {
        echo "$(date +%Y-%m-%d\ %H:%M:%S) N${NUM}: balance check failed" >> "$LOG"
        continue
    }

    CURRENT_BONDS=$(awk "BEGIN {print int(${BONDED:-0} / $BOND_UNIT)}")
    MAX_BONDS=3000
    REMAINING=$((MAX_BONDS - CURRENT_BONDS))

    if [ "$REMAINING" -le 0 ]; then
        echo "$(date +%Y-%m-%d\ %H:%M:%S) N${NUM}: at max bonds (${CURRENT_BONDS}/${MAX_BONDS}), skipping" >> "$LOG"
        continue
    fi

    BONDS=$(awk "BEGIN {b=int(($SPENDABLE - 0.01) / $BOND_UNIT); if(b>100) b=100; if(b>$REMAINING) b=$REMAINING; print b}")
    if [ "$BONDS" -gt 0 ]; then
        echo "$(date +%Y-%m-%d\ %H:%M:%S) N${NUM}: spendable=${SPENDABLE}, bonds=${CURRENT_BONDS}/${MAX_BONDS}, adding ${BONDS}" >> "$LOG"
        "$CLI" -w "$KEY" -r "http://127.0.0.1:${PORT}" producer add-bond --count "$BONDS" >> "$LOG" 2>&1
    else
        echo "$(date +%Y-%m-%d\ %H:%M:%S) N${NUM}: spendable=${SPENDABLE}, bonds=${CURRENT_BONDS}/${MAX_BONDS}, skipping" >> "$LOG"
    fi
done

# Others disabled — stress test producers keep 1 bond each
# for NUM in $(seq 13 162); do
#     ...
# done
