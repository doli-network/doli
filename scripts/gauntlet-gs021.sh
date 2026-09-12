#!/usr/bin/env bash
# ============================================================================
# gauntlet-gs021.sh — GS-021 "BLS rotation frozen pre-activation" scenario.
#
# Sourced by scripts/gauntlet.sh, and runnable standalone. Guards INC-I-217:
# TxType::RotateBlsKey = 32 ships in the binary, but every network keeps
# bls_key_rotation_activation_height = u64::MAX, so NO rotation may reach the
# producer set yet. OBSERVATIONAL, READ-ONLY, testnet-only, in the DEFAULT
# gate, NOT opt-in, no confirm-var. It reads one RPC method, the /metrics
# endpoints and files in this checkout. It writes nothing anywhere.
#
#   gs021-no-queued-rotation    — 0 pendingUpdates of kind rotate_bls_key.
#   gs021-rotation-metric-zero  — doli_producer_bls_rotation_total is 0.
#   gs021-below-ah-refused      — a pre-activation rotation answers
#                                 [ERRTX-ROT002]. Needs an offline dry-run
#                                 path in the CLI; none exists, so it SKIPs.
#
# Every precondition SKIPs with rc=2 — never a pass, never a fail. A pinned
# activation height also SKIPs: rotations become legitimate at that point and
# this scenario stops being a falsifier.
#
# Env: GS021_RPC_PORT, GS021_METRICS_PORTS, GS021_DEFAULTS, GS021_TIMEOUT.
# ============================================================================

_GS021_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GS021_DEFAULTS="${GS021_DEFAULTS:-$_GS021_ROOT/crates/core/src/network_params/defaults.rs}"
GS021_ROTATE_CLI="${GS021_ROTATE_CLI:-$_GS021_ROOT/bins/cli/src/cmd_producer/rotate.rs}"
GS021_ROTATE_TX="${GS021_ROTATE_TX:-$_GS021_ROOT/bins/cli/src/rotate_tx.rs}"
GS021_METRICS_PORTS="${GS021_METRICS_PORTS:-9000 9001 9002 9003 9004 9005 9006 9007 9008 9009 9010 9011 9012 9013 9014 9015 9016 9017}"
GS021_TIMEOUT="${GS021_TIMEOUT:-5}"
GS021_GATE_FIELD="bls_key_rotation_activation_height"
GS021_METRIC="doli_producer_bls_rotation_total"
GS021_KIND="rotate_bls_key"

# The runner owns port_of; standalone falls back to the seed RPC port.
_gs021_port() {
  if type port_of >/dev/null 2>&1; then
    port_of seed
  else
    echo "${GS021_RPC_PORT:-8500}"
  fi
}

_gs021_rpc() {
  curl -sf --max-time "$GS021_TIMEOUT" -X POST "http://127.0.0.1:$(_gs021_port)" \
    -H 'Content-Type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":$2}" 2>/dev/null
}

# Gate litmus: no RPC exposes the activation height, so the checkout is the
# only source. Any network carrying a real height ends the scenario premise.
_gs021_gate_frozen() {
  local t="$1" total frozen
  if [ ! -f "$GS021_DEFAULTS" ]; then
    SKIP_REASONS="$SKIP_REASONS; $t: $GS021_DEFAULTS not in this checkout"
    return 2
  fi
  total="$(grep -c "$GS021_GATE_FIELD:" "$GS021_DEFAULTS")"
  frozen="$(grep -c "$GS021_GATE_FIELD: u64::MAX" "$GS021_DEFAULTS")"
  if [ "${total:-0}" -eq 0 ]; then
    SKIP_REASONS="$SKIP_REASONS; $t: no $GS021_GATE_FIELD default found — the field moved"
    return 2
  fi
  if [ "${frozen:-0}" != "${total:-0}" ]; then
    SKIP_REASONS="$SKIP_REASONS; $t: $GS021_GATE_FIELD is pinned on $(( total - frozen )) of $total networks — rotation is activating, premise gone"
    return 2
  fi
  return 0
}

_gs021_queued_check() {
  local t="$1" out counts producers queued
  out="$(_gs021_rpc getProducers '{"active_only":false}')"
  if [ -z "$out" ]; then
    SKIP_REASONS="$SKIP_REASONS; $t: no node answered getProducers on 127.0.0.1:$(_gs021_port)"
    return 2
  fi
  counts="$(printf '%s' "$out" | python3 -c "
import sys, json
try:
    rows = json.load(sys.stdin).get('result') or []
    q = sum(1 for p in rows for u in (p.get('pendingUpdates') or [])
            if u.get('updateType') == '$GS021_KIND')
    print(len(rows), q)
except Exception:
    print('')
" 2>/dev/null)"
  if [ -z "$counts" ]; then
    SKIP_REASONS="$SKIP_REASONS; $t: getProducers answered an unparseable body"
    return 2
  fi
  producers="${counts% *}"; queued="${counts#* }"
  if [ "${producers:-0}" -eq 0 ]; then
    SKIP_REASONS="$SKIP_REASONS; $t: the producer set is empty — nothing to observe"
    return 2
  fi
  if [ "${queued:-0}" -gt 0 ]; then
    FAIL_REASONS="$FAIL_REASONS; $t: $queued $GS021_KIND pendingUpdates entries exist while $GS021_GATE_FIELD is u64::MAX — a rotation reached the queue below its activation height"
    return 1
  fi
  INFO_REASONS="$INFO_REASONS; $t: $producers producers carry 0 $GS021_KIND pending updates"
  return 0
}

_gs021_metric_check() {
  local t="$1" p body val seen=0 hot=0 detail=""
  for p in $GS021_METRICS_PORTS; do
    body="$(curl -sf --max-time "$GS021_TIMEOUT" "http://127.0.0.1:$p/metrics" 2>/dev/null)"
    [ -n "$body" ] || continue
    val="$(printf '%s' "$body" | awk -v m="$GS021_METRIC" '$1 == m {print $2; exit}')"
    [ -n "$val" ] || continue
    seen=$(( seen + 1 ))
    case "$val" in
      0 | 0.0 | 0e+00) ;;
      *) hot=$(( hot + 1 )); detail="$detail :$p=$val" ;;
    esac
  done
  if [ "$seen" -eq 0 ]; then
    SKIP_REASONS="$SKIP_REASONS; $t: no endpoint exposes $GS021_METRIC — metrics off, or a build older than M9"
    return 2
  fi
  if [ "$hot" -gt 0 ]; then
    FAIL_REASONS="$FAIL_REASONS; $t: $GS021_METRIC is above zero on $hot of $seen endpoints —$detail"
    return 1
  fi
  INFO_REASONS="$INFO_REASONS; $t: $GS021_METRIC is 0 on $seen endpoints"
  return 0
}

# The refusal is real (ERRTX-ROT002), but proving it live needs a transaction,
# and this scenario never sends one. It SKIPs until the CLI can build one
# offline. The grep re-opens the check by itself the day that flag lands.
_gs021_refusal_check() {
  local t="$1"
  if grep -qE 'dry[-_]run' "$GS021_ROTATE_CLI" "$GS021_ROTATE_TX" 2>/dev/null; then
    SKIP_REASONS="$SKIP_REASONS; $t: an offline dry-run path now exists in the rotate CLI — wire the [ERRTX-ROT002] check here"
  else
    SKIP_REASONS="$SKIP_REASONS; $t: no offline dry-run path in 'doli producer rotate-bls'; refusing to put a rotation on a live chain to prove [ERRTX-ROT002]"
  fi
  return 2
}

_gs021_assert() {
  local t="${1:-}"
  case "$t" in
    gs021-no-queued-rotation | gs021-rotation-metric-zero | gs021-below-ah-refused) ;;
    *)
      FAIL_REASONS="$FAIL_REASONS; $t: unknown GS-021 assertion token"
      return 1 ;;
  esac
  _gs021_gate_frozen "$t" || return 2
  case "$t" in
    gs021-no-queued-rotation) _gs021_queued_check "$t"; return $? ;;
    gs021-rotation-metric-zero) _gs021_metric_check "$t"; return $? ;;
    gs021-below-ah-refused) _gs021_refusal_check "$t"; return $? ;;
  esac
  FAIL_REASONS="$FAIL_REASONS; $t: unknown GS-021 assertion token"
  return 1
}

# gauntlet.sh has no single-scenario filter, so a solo run comes through here.
_gs021_main() {
  local t rc s_ok=1
  FAIL_REASONS=""; SKIP_REASONS=""; INFO_REASONS=""
  for t in gs021-no-queued-rotation gs021-rotation-metric-zero gs021-below-ah-refused; do
    _gs021_assert "$t"; rc=$?
    { [ "$rc" = "0" ] || [ "$rc" = "2" ]; } || s_ok=0
  done
  if [ "$s_ok" = "1" ]; then
    echo "  PASS  [obs] GS-021  bls-rotation-frozen-pre-activation"
  else
    echo "  FAIL  [obs] GS-021  bls-rotation-frozen-pre-activation"
    echo "       ${FAIL_REASONS# ; }"
  fi
  [ -n "$SKIP_REASONS" ] && echo "       skip:${SKIP_REASONS# ;}"
  [ -n "$INFO_REASONS" ] && echo "       note:${INFO_REASONS# ;}"
  [ "$s_ok" = "1" ]
}

if [ "${BASH_SOURCE[0]}" = "${0}" ]; then
  FAIL_REASONS=""; SKIP_REASONS=""; INFO_REASONS=""
  _gs021_main "$@"
  exit $?
fi
