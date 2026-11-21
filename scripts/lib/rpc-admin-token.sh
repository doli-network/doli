#!/usr/bin/env bash
# Admin RPC token lookup (INC-I-308). Source this file; it defines functions only.
# Order: env DOLI_RPC_ADMIN_TOKEN (non-empty), then <data_dir>/rpc-admin.token.

# rpc_admin_token [data_dir] — print the token; return 1 when none is found.
rpc_admin_token() {
  local dir="${1:-}" tok=""
  if [ -n "${DOLI_RPC_ADMIN_TOKEN:-}" ]; then
    tok="$DOLI_RPC_ADMIN_TOKEN"
  elif [ -n "$dir" ] && [ -r "$dir/rpc-admin.token" ]; then
    tok="$(tr -d '[:space:]' < "$dir/rpc-admin.token")"
  fi
  [ -n "$tok" ] || return 1
  printf '%s\n' "$tok"
}

# rpc_admin_curl_args [data_dir] — print "-H" and "Authorization: Bearer <tok>" one per line, or nothing.
rpc_admin_curl_args() {
  local tok
  tok="$(rpc_admin_token "${1:-}")" || return 0
  printf '%s\n' "-H" "Authorization: Bearer $tok"
}

# rpc_admin_port_data_dir <port> — print the --data-dir of the local process listening on <port>.
rpc_admin_port_data_dir() {
  local pid
  for pid in $(lsof -nP -iTCP:"$1" -sTCP:LISTEN -t 2>/dev/null); do
    ps -o command= -p "$pid" 2>/dev/null | sed -n 's/.*--data-dir[ =]\([^ ]*\).*/\1/p' | head -1
    return 0
  done
}

RPC_ADMIN_METHODS=" pauseProduction resumeProduction createCheckpoint pruneBlocks backfillFromPeer enterRecoveryMode exitRecoveryMode bridgeFromArchive getUtxoDiff getStateSnapshot getStateRootDebug verifyChainIntegrity repairArchiveFromPeer forceReorgTo "

# rpc_admin_is_loopback <host> — exact match: localhost, ::1, [::1], 127.a.b.c.
rpc_admin_is_loopback() {
  case "$1" in
    localhost|::1|\[::1\]) return 0 ;;
  esac
  printf '%s\n' "$1" | grep -Eq '^127\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}$'
}

# rpc_admin_header_for <method> <url|host:port> — print the Authorization line for an admin method (use: curl -H @<(...)).
rpc_admin_header_for() {
  local auth="${2#*://}" host port dir="" tok
  case "$RPC_ADMIN_METHODS" in *" $1 "*) ;; *) return 0 ;; esac
  auth="${auth%%/*}"
  case "$auth" in
    *@*) host="" ;;
    \[*) host="${auth%%]*}]" ;;
    *:*) host="${auth%:*}" ;;
    *) host="$auth" ;;
  esac
  port="${auth##*:}"
  port="${port%%[!0-9]*}"
  rpc_admin_is_loopback "$host" || host=""
  [ -n "${DOLI_RPC_ADMIN_TOKEN:-}" ] || [ -z "$host" ] || [ -z "$port" ] || dir="$(rpc_admin_port_data_dir "$port")"
  tok="$(rpc_admin_token "$dir")" && printf 'Authorization: Bearer %s\n' "$tok"
  return 0
}

# rpc_admin_ssh <host> <service> <port> <json> — admin RPC to 127.0.0.1:<port> on <host>; the token stays off argv.
rpc_admin_ssh() {
  local remote='read -r t
if [ -z "$t" ]; then
  d=$(systemctl show -p ExecStart --value "$1" 2>/dev/null | sed -n "s/.*--data-dir[ =]\([^ ;]*\).*/\1/p")
  t=$(cat "$d/rpc-admin.token" 2>/dev/null || sudo -n cat "$d/rpc-admin.token" 2>/dev/null)
fi
if [ -n "$t" ]; then printf "Authorization: Bearer %s\n" "$t"; fi |
  curl -sf --max-time 5 -X POST "http://127.0.0.1:$2" -H "Content-Type: application/json" -H @- -d "$3"'
  printf '%s\n' "${DOLI_RPC_ADMIN_TOKEN:-}" | ssh -o ConnectTimeout=5 -o BatchMode=yes "$1" \
    "bash -c $(printf %q "$remote") _ $(printf '%q ' "$2" "$3" "$4")"
}
