#!/usr/bin/env bash
# INC-I-178 M7.6 outcome probe — REQ-BLS-007.
# Measures the rate of the `[ATTEST_INGEST] valid bls` line in a live testnet
# node log, at the node's CONFIGURED level (all 18 units run --log-level info).
#
# Reads only the bytes appended during the window, so it is O(window) and not
# O(logfile) on the ~1.5 GB testnet logs, and it is immune to the historical
# backlog. Re-runnable; prints one line: "<rate> lines/s (<n> lines over <w>s)".
#
#   usage: bash .claude/scripts/m76-probe.sh [log] [window_secs]
set -u
LOG="${1:-$HOME/testnet/logs/n5.log}"
W="${2:-300}"
[ -r "$LOG" ] || { echo "PROBE-ERROR: cannot read $LOG"; exit 1; }
a="$(wc -c < "$LOG" | tr -d ' ')"
sleep "$W"
b="$(wc -c < "$LOG" | tr -d ' ')"
if [ "$b" -lt "$a" ]; then echo "PROBE-ERROR: $LOG rotated mid-window"; exit 1; fi
n="$(tail -c "+$(( a + 1 ))" "$LOG" 2>/dev/null | head -c "$(( b - a ))" \
     | grep -ac 'valid bls' || true)"
n="$(printf '%s' "$n" | tr -dc '0-9')"
awk -v n="${n:-0}" -v w="$W" 'BEGIN{printf "%.3f lines/s (%d lines over %ds)\n", n/w, n, w}'
