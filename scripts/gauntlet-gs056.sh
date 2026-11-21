#!/usr/bin/env bash
# gauntlet-gs056.sh — GS-056 rpc-admin-gate-by-source (INC-I-308; AUDIT-P1-RPC-001, -003).
#
# Usage: bash scripts/gauntlet-gs056.sh
# OBSERVATIONAL over the running fleet. It only reads: ps, lsof, the token FILE's existence and
# length (never its value), and one read-only admin probe (getUtxoDiff, {}: no state change, cheap)
# sent with NO token. It never restarts, stops or writes to a node.
# Per running doli-node process:
#   gs056-no-open-nonloopback  no RPC socket on a non-loopback address unless a token is set
#                              (env DOLI_RPC_ADMIN_TOKEN, or <data_dir>/rpc-admin.token >= 32 chars)
#   gs056-probe-nonloopback    the probe sent to the host's non-loopback address on the node's RPC
#                              port returns -32008 or no connection
# INFO (not a verdict): the same probe on 127.0.0.1. A node on a pre-INC-I-308 binary answers it
# (gate open from loopback: the parent behaviour); a node on the END binary answers -32008.
set -uo pipefail
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/gauntlet-rpc-admin-lib.sh"
mkdir -p "$GA_WORK"
FAILS=""; fail(){ FAILS="$FAILS; $*"; echo "  ✗ $*"; }
PROBE="${GS056_PROBE:-getUtxoDiff}"
HOSTIP="${GS056_HOST_IP:-$(ifconfig 2>/dev/null | awk '/inet /&&$2!~/^127\./{print $2;exit}')}"
echo "GS-056 rpc-admin-gate-by-source  probe=$PROBE  host_nonloopback_ip=${HOSTIP:-none}"
PIDS=$(ga_fleet_pids)
[ -n "$PIDS" ] || { echo "VERDICT: NOT_MADE (no running doli-node)"; exit 3; }
N=0
for pid in $PIDS; do
  argv=$(ga_argv "$pid"); [ -n "$argv" ] || continue
  rpc=$(ga_flag "$argv" --rpc-port); dd=$(ga_flag "$argv" --data-dir)
  [ -n "$rpc" ] || { ga_metric pid=$pid rpc_port=none skipped; continue; }
  N=$((N+1))
  addrs=$(ga_listen_addrs "$pid" "$rpc" | tr '\n' ' ')
  tok=$(ga_env_has_token "$pid")
  if [ -z "$tok" ] && [ -n "$dd" ] && [ -f "$dd/rpc-admin.token" ]; then
    l=$(tr -d '[:space:]' < "$dd/rpc-admin.token" 2>/dev/null | wc -c | tr -d ' ')
    [ "${l:-0}" -ge 32 ] && tok="file(len=$l)"
  fi
  nonloop=""
  for a in $addrs; do ga_is_loopback "$a" || nonloop="$nonloop $a"; done
  if [ -n "$nonloop" ] && [ -z "$tok" ]; then fail "gs056-no-open-nonloopback: pid=$pid rpc=$rpc listen=$nonloop token=none"; fi
  pn="none"
  if [ -n "$HOSTIP" ]; then
    pn=$(ga_code "$(GA_RPC_TIMEOUT=5 ga_rpc "http://$HOSTIP:$rpc" "$PROBE" '{}')")
    case "$pn" in none|"$GA_UNAUTH") ;; *) fail "gs056-probe-nonloopback: pid=$pid $HOSTIP:$rpc $PROBE -> $pn" ;; esac
  fi
  pl=$(ga_code "$(GA_RPC_TIMEOUT=10 ga_rpc "http://127.0.0.1:$rpc" "$PROBE" '{}')")
  ga_metric pid=$pid rpc=$rpc listen="${addrs% }" token=${tok:-none} nonloopback_probe=$pn loopback_probe_info=$pl \
    bin="$(printf '%s' "$argv" | awk '{print $1}' | xargs basename)"
done
[ "$N" -ge 1 ] || { echo "VERDICT: NOT_MADE (no node with --rpc-port)"; exit 3; }
ga_metric nodes_checked=$N
if [ -z "$FAILS" ]; then echo "VERDICT: PASS"; exit 0; fi
echo "VERDICT: FAIL$FAILS"; exit 1
