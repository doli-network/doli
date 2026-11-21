#!/usr/bin/env bash
# gauntlet-gs058.sh — GS-058 rpc-admin-gate-scratch-node (INC-I-308; AUDIT-P1-RPC-001, -002).
#
# Usage: bash scripts/gauntlet-gs058.sh            (GA_BIN = node binary, default target/release/doli-node)
# Env:   GA_BIN, GS058_RPC (8591), GS058_P2P (30391), GS058_METRICS (9091), GA_WORK (scratch root)
#
# Starts ONE isolated scratch node (see scripts/gauntlet-rpc-admin-lib.sh) and asserts, from loopback:
#   gs058-no-token-refused     every ADMIN_METHODS name with no Authorization -> -32008
#   gs058-wrong-token-refused  every ADMIN_METHODS name with a wrong 64-hex token -> -32008
#   gs058-valid-token-passes   every ADMIN_METHODS name with the node's own token (read from
#                              <data_dir>/rpc-admin.token) -> NOT -32008. State-changing methods get
#                              params the handler refuses ({}), or are paired and undone
#                              (pauseProduction->resumeProduction, enterRecoveryMode->exitRecoveryMode).
#   gs058-guardian-unchanged   getGuardianStatus is identical before and after the refused sweeps
#   gs058-public-unchanged     getChainInfo and getGuardianStatus answer with no token
#   gs058-token-file-0600      the token file exists, mode 600, 64 chars; not on the node argv
# The scratch node is stopped by PID at exit and its data dir deleted (node.log kept under GA_WORK).
set -uo pipefail
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/gauntlet-rpc-admin-lib.sh"
mkdir -p "$GA_WORK"
trap 'ga_scratch_stop' EXIT

FAILS=""; fail(){ FAILS="$FAILS; $*"; echo "  ✗ $*"; }
echo "GS-058 rpc-admin-gate-scratch-node  bin=$GA_BIN  version=$("$GA_BIN" --version 2>/dev/null)"
echo "  bin_sha256=$(shasum -a 256 "$GA_BIN" | awk '{print $1}')"
METHODS=$(ga_admin_methods)
N=$(printf '%s\n' "$METHODS" | grep -c .)
ga_metric admin_methods=$N "$(printf '%s ' $METHODS)"
[ "$N" -ge 14 ] || { echo "VERDICT: NOT_MADE (ADMIN_METHODS parse gave $N names)"; exit 3; }

ga_scratch_start gs058 "${GS058_RPC:-8591}" "${GS058_P2P:-30391}" "${GS058_METRICS:-9091}" \
  || { echo "VERDICT: NOT_MADE (scratch node did not start)"; exit 3; }
URL="$SCRATCH_RPC"

WRONG=$(printf 'f%.0s' $(seq 1 64))   # never equals a random 64-hex token in practice; checked below
G0=$(ga_rpc "$URL" getGuardianStatus '{}')
for pm in getChainInfo getGuardianStatus; do
  c=$(ga_code "$(ga_rpc "$URL" $pm '{}')"); ga_metric public $pm no_token=$c
  [ "$c" = "ok" ] || fail "gs058-public-unchanged: $pm no token -> $c"
done

for m in $METHODS; do
  c1=$(ga_code "$(ga_rpc "$URL" "$m" '{}')")
  c2=$(ga_code "$(ga_rpc "$URL" "$m" '{}' "$WRONG")")
  ga_metric refused $m no_token=$c1 wrong_token=$c2
  [ "$c1" = "$GA_UNAUTH" ] || fail "gs058-no-token-refused: $m -> $c1"
  [ "$c2" = "$GA_UNAUTH" ] || fail "gs058-wrong-token-refused: $m -> $c2"
done
G1=$(ga_rpc "$URL" getGuardianStatus '{}')
if [ "$G0" = "$G1" ]; then ga_metric guardian_unchanged=yes
else fail "gs058-guardian-unchanged: before=$G0 after=$G1"; fi

# Token file: checked after the refused sweeps so a pre-INC-I-308 control binary still reports them.
TF=$(find "$SCRATCH_DIR/data" -name rpc-admin.token 2>/dev/null | head -1)
if [ -z "$TF" ]; then
  fail "gs058-token-file-0600: no rpc-admin.token under the scratch data dir"
  echo "VERDICT: FAIL$FAILS"; exit 1
fi
TOKEN=$(tr -d '[:space:]' < "$TF")
MODE=$(stat -f '%Lp' "$TF")
ga_metric token_file="$TF" mode=$MODE len=${#TOKEN}
[ "$MODE" = "600" ] && [ "${#TOKEN}" -eq 64 ] || fail "gs058-token-file-0600: mode=$MODE len=${#TOKEN}"
ga_argv "$SCRATCH_PID" | grep -q -- "$TOKEN" && fail "gs058-token-file-0600: token on argv"
[ "$WRONG" = "$TOKEN" ] && fail "gs058-wrong-token-refused: the wrong token equals the real one (rerun)"

# Valid token: paired methods first (do, then undo), then the rest with handler-refused params.
for m in $METHODS; do
  # resumeProduction / exitRecoveryMode are sent below as the undo of their pair.
  case "$m" in resumeProduction|exitRecoveryMode) continue ;; esac
  # {} only: on a fresh scratch node at h=0 the handlers have nothing to prune/bridge/fetch,
  # and params-requiring methods (backfillFromPeer, repairArchiveFromPeer, forceReorgTo) refuse.
  b=$(ga_rpc "$URL" "$m" '{}' "$TOKEN"); c=$(ga_code "$b")
  ga_metric valid $m code=$c "answer=$(printf '%s' "$b" | head -c 140)"
  [ "$c" != "$GA_UNAUTH" ] && [ "$c" != "none" ] || fail "gs058-valid-token-passes: $m -> $c"
  undo=""
  [ "$m" = pauseProduction ] && undo=resumeProduction
  [ "$m" = enterRecoveryMode ] && undo=exitRecoveryMode
  if [ -n "$undo" ]; then
    b=$(ga_rpc "$URL" "$undo" '{}' "$TOKEN"); c=$(ga_code "$b")
    ga_metric valid $undo code=$c "answer=$(printf '%s' "$b" | head -c 140)"
    [ "$c" != "$GA_UNAUTH" ] && [ "$c" != "none" ] || fail "gs058-valid-token-passes: $undo -> $c"
  fi
done
G2=$(ga_rpc "$URL" getGuardianStatus '{}')
ga_metric guardian_after_valid_pairs="$(printf '%s' "$G2" | head -c 200)"

if [ -z "$FAILS" ]; then echo "VERDICT: PASS"; exit 0; fi
echo "VERDICT: FAIL$FAILS"; exit 1
