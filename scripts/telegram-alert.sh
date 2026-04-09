#!/usr/bin/env bash
# telegram-alert.sh — Send an alert message to a Telegram chat via the Bot API.
#
# Generic helper used by guardian scripts (fork-monitor, etc.) to push
# alerts to an operator Telegram channel. Safe to call when alerting is
# unconfigured — exits 0 silently if the env vars are missing so it can be
# used unconditionally by callers.
#
# Required environment variables:
#   DOLI_TELEGRAM_BOT_TOKEN   Bot token from @BotFather (e.g. 123456:ABC-...)
#   DOLI_TELEGRAM_CHAT_ID     Target chat ID — numeric, may be negative for
#                             groups/supergroups. Get it by messaging the bot
#                             and calling getUpdates, or via @userinfobot.
#
# Usage:
#   scripts/telegram-alert.sh \
#       --severity critical \
#       --title   "FORK DETECTED" \
#       --body    "testnet — 2 chain tips across 13 nodes..."
#
# Flags:
#   --severity  critical | warning | info | recovery   (default: info)
#   --title     one-line bold headline                 (required)
#   --body      multi-line body, rendered as <pre>     (optional)
#   -h, --help  this help
#
# Message format: HTML parse mode. Title is bold with a text severity prefix
# ([CRITICAL], [WARNING], [INFO], [RECOVERED]). Body is wrapped in <pre>. All
# user-provided strings are HTML-escaped.
#
# Exit codes:
#   0  sent, or no-op when alerting is unconfigured
#   1  send failed (network/API error)
#   2  bad arguments

set -euo pipefail

SEVERITY="info"
TITLE=""
BODY=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --severity) SEVERITY="$2"; shift 2 ;;
    --title)    TITLE="$2"; shift 2 ;;
    --body)     BODY="$2"; shift 2 ;;
    -h|--help)  grep '^#' "$0" | sed 's/^# \?//'; exit 0 ;;
    *)          echo "telegram-alert: unknown arg: $1" >&2; exit 2 ;;
  esac
done

if [[ -z "$TITLE" ]]; then
  echo "telegram-alert: --title is required" >&2
  exit 2
fi

TOKEN="${DOLI_TELEGRAM_BOT_TOKEN:-}"
CHAT="${DOLI_TELEGRAM_CHAT_ID:-}"

if [[ -z "$TOKEN" || -z "$CHAT" ]]; then
  # No-op when unconfigured so callers can use it unconditionally.
  echo "telegram-alert: DOLI_TELEGRAM_BOT_TOKEN or DOLI_TELEGRAM_CHAT_ID not set — skipping" >&2
  exit 0
fi

case "$SEVERITY" in
  critical) PREFIX="[CRITICAL]" ;;
  warning)  PREFIX="[WARNING]"  ;;
  recovery) PREFIX="[RECOVERED]" ;;
  info|*)   PREFIX="[INFO]"     ;;
esac

# HTML-escape via python (Telegram HTML parse mode requires < > & escaped).
html_escape() {
  python3 -c 'import sys, html; sys.stdout.write(html.escape(sys.stdin.read()))'
}

TITLE_ESC=$(printf '%s' "$TITLE" | html_escape)
BODY_ESC=$(printf '%s' "$BODY"   | html_escape)

if [[ -n "$BODY_ESC" ]]; then
  MSG="<b>${PREFIX} ${TITLE_ESC}</b>
<pre>${BODY_ESC}</pre>"
else
  MSG="<b>${PREFIX} ${TITLE_ESC}</b>"
fi

RESPONSE_FILE=$(mktemp -t telegram-alert.XXXXXX)
trap 'rm -f "$RESPONSE_FILE"' EXIT

HTTP_CODE=$(curl -sS --max-time 10 \
  -o "$RESPONSE_FILE" \
  -w '%{http_code}' \
  "https://api.telegram.org/bot${TOKEN}/sendMessage" \
  --data-urlencode "chat_id=${CHAT}" \
  --data-urlencode "parse_mode=HTML" \
  --data-urlencode "text=${MSG}" \
  2>/dev/null || echo "000")

if [[ "$HTTP_CODE" == "200" ]]; then
  echo "telegram-alert: sent ${PREFIX} ${TITLE}" >&2
  exit 0
else
  echo "telegram-alert: send FAILED (http=$HTTP_CODE) — ${PREFIX} ${TITLE}" >&2
  if [[ -s "$RESPONSE_FILE" ]]; then
    echo "telegram-alert: response body:" >&2
    head -c 500 "$RESPONSE_FILE" >&2
    echo "" >&2
  fi
  exit 1
fi
