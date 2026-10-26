#!/usr/bin/env bash
# test_gauntlet_gs028.sh — offline companion for GS-028 (withdraw-then-equivocate and forced
# exit on a throwaway devnet). Never starts a node, never runs the runner; < 15 s.
# covers: scripts/gauntlet-gs028.sh scripts/gauntlet-devnet-lib.sh scripts/gauntlet-devnet.py bins/node/examples/gs028_forge_tx/main.rs bins/node/examples/gs028_forge_tx/shape.rs scripts/gauntlet-gs026.sh scripts/gauntlet-gs027.sh
# REQ-LDP-005 — Decision: a FAIL means the default GS-028 cluster can drop below the producer floor when two producers leave, gives the target too few slots per epoch, or cannot swap node, CLI and forge builds for the control run.
# REQ-LDP-006 — Decision: a FAIL means the runner cannot show the withdrawal was mined before the evidence (no sibling target or minimum height wiring, no withdrawal-before-evidence line), so a slash could have hit an un-withdrawn bond.
# REQ-LDP-007 — Decision: a FAIL means the one-slash assertion does not name the slash height (or none) and the target status is not checked, so a missing or doubled slash reads as a pass.
# REQ-LDP-008 — Decision: a FAIL means the runner does not compute removed(S) through the shared helper or cross-check the burn log line, so the locked-amount burn is asserted on an assumed number.
# REQ-LDP-009 — Decision: a FAIL means removed(W) + removed(S) is not computed from chain data, so the whole-bond claim could rest on a constant.
# REQ-LDP-010 — Decision: a FAIL means the producer-gets-nothing assertion is missing, so a spendable payout surviving the slash would go unreported.
# REQ-LDP-011 — Decision: a FAIL means the forced-exit probe is not run, is matched on the generic error_code field, or its measured line lacks code= and mined=, so a refusal for the wrong reason or a mined forced exit could pass.
# REQ-LDP-012 — Decision: a FAIL means tips-equal or production-continues is not emitted, so a fork or a stall after the slash could ship unseen.
# REQ-LDP-013 — Decision: a FAIL means no conclusive control record on the pre-unbonding build exists (preconditions PASS, a funds assertion FAIL, forced-exit line with mined=), so the HEAD PASS has no RED baseline.
# REQ-LDP-021 — Decision: a FAIL means the control does not declare its expected-FAIL set and preconditions before the cluster starts, or a SKIP is recorded as a conclusive result.
# REQ-LDP-026 — Decision: verified outside this offline test; each live run gets a gauntlet run record with its git sha and status, and its full output is kept with the run.
# REQ-LDP-022 — Decision: a FAIL means the runner breaks the gauntlet output contract (ids, METRIC fields, SKIP exit 2, verdict as the exit code), so the harness counts the wrong outcome.
# REQ-LDP-023 — Decision: a FAIL means the runner can signal a foreign PID, delete a foreign dir, reach a non-loopback host, eval node output, skip hex or digit checks on RPC values, or read an exit code through a pipe.
# REQ-LDP-024 — Decision: a FAIL means a new or changed runner, library, helper or forge file exceeds its line budget, or GS-026 grew.
# OUTPUT CONTRACT:
#   O1 stdout   `PASS <case>` | `FAIL <case>: <why>` per case; last line `<N> passed, <M> failed`
#   O2 exit     0 all PASS · 1 any FAIL
#   O3 effects  one mktemp dir (deleted on exit); reads the runner, library, helper, forge sources
#               never executes the runner
#   subject outputs read: runner file (present/exec/parse, header ids, emitted res ids, METRIC fields,
#     exit idioms, DN_TAG, gate env literals, knob documentation, scratch + kill idiom, start order,
#     CONTROL declaration, measured k= per assertion, safety scans), library cli_as body, line counts
# INPUT PARTITIONS:
#   runner        absent (RED) · present: executable · bash -n · header documents 9 ids + exit 0/1/2
#   ids           exactly the 9 gs028 ids emitted via res (no extra, none missing)
#   control       CONTROL expect_fail = 5 ids · preconditions = 2 ids · declared before start_cluster ·
#                 'control inconclusive' SKIP path
#   gates         four gate literals share one value · pending-root 0
#   safety        runner + library clean · planted pkill / foreign kill / foreign rm / eval -> 4 ·
#                 planted 10.x URL + 192.168.x ip4 -> 2 · planted rc-after-pipe x2 -> 2 · planted
#                 start_node copy -> 1 · planted undocumented knob -> 1 · planted second gate value ·
#                 481-line file flagged, 480-line file not
#   records       head row all 9 PASS · control row conclusive · GS-026/GS-027 head rows pass ·
#                 an exit-2 row never counted (absent rows -> FAIL, expected RED until the runs)

# shellcheck disable=SC2016
set -u
RUNNER="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/gauntlet-gs028.sh"
TAG=gs028
PREFIX=GS028
EXTRA=""
IDS="gs028-withdrawal-before-evidence gs028-one-slash-mined gs028-slash-burns-locked gs028-full-bond-leaves-supply gs028-producer-gets-nothing gs028-forced-exit-refused gs028-tips-equal gs028-production-continues gs028-target-status-slashed"
CONTROL_FAIL="gs028-slash-burns-locked gs028-full-bond-leaves-supply gs028-producer-gets-nothing gs028-forced-exit-refused gs028-target-status-slashed"
CONTROL_PRE="gs028-withdrawal-before-evidence gs028-tips-equal"
FUNDS="gs028-slash-burns-locked|gs028-full-bond-leaves-supply|gs028-producer-gets-nothing"
# shellcheck source=scripts/test_gauntlet_devnet_checks.sh
. "$(dirname "${BASH_SOURCE[0]}")/test_gauntlet_devnet_checks.sh"
FORGE_DIR="$ROOT/bins/node/examples/gs028_forge_tx"

c_cluster_defaults() {
  local c d miss=""
  present cluster-defaults || return
  c="$(code "$RUNNER")"
  for d in GS028_NODES:6 GS028_PRODUCERS:5 GS028_EPOCH_BLOCKS:15 GS028_AH:30; do
    printf '%s\n' "$c" | grep -qE "${d%%:*}=\"?\\\$\\{${d%%:*}:-${d##*:}\\}\"?" || miss="$miss ${d%%:*}:-${d##*:}"
  done
  [ -z "$miss" ] || { fail cluster-defaults "defaults not set:$miss"; return; }
  pass cluster-defaults "6 nodes, 5 producers, epoch 15, one gate height 30"
}

c_forced_exit_probe() {
  local c
  present forced-exit-probe || return
  c="$(code "$RUNNER")"
  for t in '--kind forced-exit' '--payout' '--lock-until' '--ports' 'ERRTX-PAUTH001'; do
    printf '%s\n' "$c" | grep -qF -- "$t" || { fail forced-exit-probe "runner never uses '$t'"; return; }
  done
  printf '%s\n' "$c" | grep -qE '(^|[^[:alnum:]_])GS028_TX_FORGE([^[:alnum:]_]|$)' || { fail forced-exit-probe "tx forge path is not the GS028_TX_FORGE knob"; return; }
  pass forced-exit-probe "forge forced-exit with payout, lock and loopback ports; PAUTH001 matched"
}

c_evidence_wiring() {
  local c
  present evidence-wiring || return
  c="$(code "$RUNNER")"
  for t in 'DN_SIB_PK=' 'DN_SIB_MIN=' 'request-withdrawal' 'cli_as'; do
    printf '%s\n' "$c" | grep -qF -- "$t" || { fail evidence-wiring "runner never uses '$t'"; return; }
  done
  pass evidence-wiring "withdrawal through cli_as, sibling pinned to the target above W"
}

c_present
c_header
c_ids
c_exit_contract
c_metric SLASHED_NODES target W S B U attempts status probe_code control
c_res_kv gs028-forced-exit-refused code mined
c_res_kv gs028-one-slash-mined slash_height
c_sources_lib
c_env_documented GS028_NODE_BIN GS028_CLI GS028_FORGE GS028_TX_FORGE GS028_CONTROL
c_cluster_defaults
c_gates
c_harness_safety
c_order
c_control "$CONTROL_FAIL" "$CONTROL_PRE"
c_forced_exit_probe
c_evidence_wiring
c_tokens burn-cross-check 'SLASH\\?\] burned' 'Deferred slash failed'
c_helper_calls removed
c_rpc_values
c_cli_as
c_planted
c_size "$RUNNER:$RUNNER_MAX" "$LIB:500" "$PYH:500" "$FORGE_DIR/main.rs:500" "$FORGE_DIR/shape.rs:500" \
  "$ROOT/scripts/gauntlet-gs026.sh:517" "$ROOT/scripts/gauntlet-gs027.sh:500"
echo "$NP passed, $NF failed"
[ "$NF" = 0 ]
