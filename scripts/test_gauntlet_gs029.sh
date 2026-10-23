#!/usr/bin/env bash
# test_gauntlet_gs029.sh — offline companion for GS-029 (mixed-version devnet: old and HEAD nodes,
# producer transactions, same-dir restart, gate crossing). Never starts a node, never runs the runner.
# covers: scripts/gauntlet-gs029.sh scripts/gauntlet-gs029-probes.sh scripts/gauntlet-devnet-lib.sh scripts/gauntlet-devnet.py
# REQ-LDP-014 — Decision: a FAIL means the mixed cluster cannot select the old node binary and CLI, does not default to six nodes, or gives the gates more than one height, so the run does not model a rolling upgrade.
# REQ-LDP-015 — Decision: a FAIL means the mixed phase never sends the transfer, Registration, AddBond and withdrawal, or never reports old-producer blocks, so cross-version agreement is shown on an empty chain.
# REQ-LDP-016 — Decision: a FAIL means the boundary check does not compare a normalised producer set, so a mixed-version boundary that splits the set (INC-I-075 class) could pass.
# REQ-LDP-017 — Decision: a FAIL means full hash history, state roots or sampled-tip canonicity are not checked, or the fork line does not name the first differing height and both sides.
# REQ-LDP-018 — Decision: a FAIL means the restart is not by own PID on the same data dir, or epoch rebuilds, snap syncs and double production after it go unchecked (INC-I-054 class).
# REQ-LDP-019 — Decision: a FAIL means the crossing probes are missing or vacuous: the unsigned tx line lacks its code, or the unsigned block can pass without an INJECTED hash.
# REQ-LDP-020 — Decision: a FAIL means the negative control does not move the HEAD gates to BOOT+2, does not classify fork versus stall with the helper, or no conclusive negative record names the fork, so the HEAD PASS has no RED baseline.
# REQ-LDP-021 — Decision: a FAIL means the negative control does not declare its expected-FAIL set before the cluster starts, or a SKIP is recorded as its result.
# REQ-LDP-026 — Decision: verified outside this offline test; each live run gets a gauntlet run record with its git sha and status, and its full output is kept with the run.
# REQ-LDP-022 — Decision: a FAIL means the runner breaks the gauntlet output contract (ids, METRIC fields, SKIP exit 2, verdict as the exit code).
# REQ-LDP-023 — Decision: a FAIL means the runner or its probes can signal a foreign PID, delete a foreign dir, reach a non-loopback host, eval node output, or read an exit code through a pipe.
# REQ-LDP-024 — Decision: a FAIL means the runner or its probes file exceeds its line budget.
# REQ-LDP-027 — Decision: a FAIL means the post-gate withdrawal assertion is not emitted, so a HEAD CLI that still pays a Normal output above the gate goes unseen.
# REQ-LDP-028 — Decision: a FAIL means the METRIC line omits the producer-tx count in old-built blocks, so the reader cannot tell whether old producers ever packed the new tail.
# REQ-LDP-031 — Decision: a FAIL means the METRIC line omits the new producer's block count, so a registration that never produces is not visible.
# REQ-LDP-033 — Decision: a FAIL means the METRIC line omits the longest tip-divergence run, so transient splits stay invisible.
# OUTPUT CONTRACT:
#   O1 stdout   `PASS <case>` | `FAIL <case>: <why>` per case; last line `<N> passed, <M> failed`
#   O2 exit     0 all PASS · 1 any FAIL
#   O3 effects  one mktemp dir (deleted on exit); reads the runner, the probes file when present,
#               the library and the helper; never executes the runner
#   subject outputs read: runner + probes (present/exec/parse, header ids, emitted res ids, METRIC
#     fields, exit idioms, DN_TAG, gate literals, knob documentation, defaults, negative-gate idiom,
#     scratch + kill idiom, start order, CONTROL declaration, measured k= per assertion, restart and
#     probe tokens, helper subcommand calls, safety scans), line counts
# INPUT PARTITIONS:
#   runner        absent (RED) · present: executable · bash -n · header documents 12 ids + exit 0/1/2
#   ids           exactly the 12 gs029 ids emitted via res across runner + probes
#   negative      GS029_NEGATIVE read · gates at BOOT+2 · CONTROL expect_fail = {mixed-no-fork,
#                 old-blocks-canonical} declared before start_cluster · forkcheck called ·
#                 'control inconclusive' SKIP · GS029_STALL_SLOTS default 6
#   safety        same planted controls as GS-028 (4 + 2 + 2 + 1 + 1 + gates + size)
#   records       head row all 12 PASS · negative row conclusive with first_diff= or absent= ·
#                 an exit-2 row never counted (absent rows -> FAIL, expected RED until the runs)

# shellcheck disable=SC2016,SC2046
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUNNER="$HERE/gauntlet-gs029.sh"
TAG=gs029
PREFIX=GS029
EXTRA="$HERE/gauntlet-gs029-probes.sh"
IDS="gs029-mixed-txs-mined gs029-old-producer-built gs029-mixed-producer-set-equal gs029-mixed-no-fork gs029-mixed-state-equal gs029-old-blocks-canonical gs029-restart-same-datadir gs029-crossing-no-fork gs029-unsigned-tx-refused gs029-unsigned-block-refused gs029-signed-tx-mined gs029-post-gate-unbonding"
NEG_FAIL="gs029-mixed-no-fork gs029-old-blocks-canonical"
# shellcheck source=scripts/test_gauntlet_devnet_checks.sh
. "$HERE/test_gauntlet_devnet_checks.sh"

c_cluster_defaults() {
  local c d miss=""
  present cluster-defaults || return
  c="$(code "$RUNNER")"
  for d in GS029_NODES:6 GS029_EPOCH_BLOCKS:8 GS029_AH:72 GS029_STALL_SLOTS:6; do
    printf '%s\n' "$c" | grep -qE "${d%%:*}=\"?\\\$\\{${d%%:*}:-${d##*:}\\}\"?" || miss="$miss ${d%%:*}:-${d##*:}"
  done
  [ -z "$miss" ] || { fail cluster-defaults "defaults not set:$miss"; return; }
  pass cluster-defaults "6 nodes, epoch 8, one gate height 72, stall after 6 slots"
}

c_negative_gates() {
  local c
  present negative-gates || return
  c="$(code "$RUNNER")"
  printf '%s\n' "$c" | grep -qE '(^|[^[:alnum:]_])GS029_NEGATIVE([^[:alnum:]_]|$)' || { fail negative-gates "never reads GS029_NEGATIVE"; return; }
  printf '%s\n' "$c" | grep -qE 'BOOT[A-Za-z_]*\}?[[:space:]]*\+[[:space:]]*2([^0-9]|$)' ||
    { fail negative-gates "no gate height computed as BOOT + 2 for the negative control"; return; }
  pass negative-gates
}

c_probes_sourced() {
  [ -f "$EXTRA" ] || { pass probes-sourced "no probes file (runner holds the probes)"; return; }
  present probes-sourced || return
  code "$RUNNER" | grep -qE '^[[:space:]]*(\.|source)[[:space:]]+"?\$\{?ROOT\}?/scripts/gauntlet-gs029-probes\.sh"?' ||
    { fail probes-sourced "scripts/gauntlet-gs029-probes.sh exists but the runner does not source it"; return; }
  pass probes-sourced
}

# The transfer recipient must be the wallet's bech32 address (network HRP + "1" + data), which
# only the CLI `addresses` command prints; the wallet JSON holds a hex pubkey hash that `send`
# refuses as an unrecognized format (observed live, twice: a 64-hex check SKIPped the run, a
# 40-hex value was refused by the CLI).
c_wallet_address() {
  local c
  present wallet-address || return
  c="$(code "$RUNNER")"
  printf '%s\n' "$c" | grep -qE 'is_hex[[:space:]]+"?\$\{?NEW_ADDR\}?"?|64-hex address|\$\{#NEW_ADDR\}"?[[:space:]]*(=|-eq)[[:space:]]*"?40' &&
    { fail wallet-address "the fresh wallet address is taken as hex (send needs the bech32 form)"; return; }
  printf '%s\n' "$c" | grep -qE 'NEW_ADDR=.*(cli_as|\$DN_CLI|\$GS029_CLI).*[[:space:]]addresses' ||
    { fail wallet-address "the fresh wallet address is not read from the CLI addresses command"; return; }
  printf '%s\n' "$c" | grep -qE 'NEW_ADDR.*1\[0-9a-z\]|\[a-z\]\+1\[0-9a-z\]' ||
    { fail wallet-address "no bech32 shape check (hrp1data) on the fresh wallet address"; return; }
  pass wallet-address "fresh wallet address read from the CLI as bech32"
}

c_present
c_header
c_ids
c_wallet_address
c_exit_contract
c_metric old_built_producer_txs new_producer_blocks max_tip_diverge_slots
c_res_kv gs029-mixed-no-fork first_diff sides
c_res_kv gs029-old-blocks-canonical absent
c_res_kv gs029-unsigned-tx-refused code mined
c_res_kv gs029-unsigned-block-refused injected
c_sources_lib
c_probes_sourced
c_env_documented GS029_NODE_BIN GS029_OLD_NODE_BIN GS029_CLI GS029_OLD_CLI GS029_FORGE GS029_NEGATIVE GS029_STALL_SLOTS
c_cluster_defaults
c_negative_gates
c_gates
c_harness_safety
c_order
c_control "$NEG_FAIL" ""
c_tokens mixed-tx-wiring 'request-withdrawal' 'add-bond' 'register' '(^|[[:space:]])send[[:space:]]' 'cli_as'
c_tokens restart-checks 'restart_node' 'EPOCH_REBUILD' 'SNAP_SYNC|SnapSync' 'EQUIVOCATION DETECTED'
c_tokens crossing-probes 'ERRTX-PAUTH001' '--kind unsigned' 'INJECTED' 'pauseProduction' 'getStateRootDebug'
c_tokens role-identity 'sha256sum|shasum -a 256'
c_helper_calls producers-norm history attrib forkcheck
c_rpc_values
c_cli_as
c_planted
c_size "$RUNNER:$RUNNER_MAX" "$LIB:500" "$PYH:500" $([ -f "$EXTRA" ] && echo "$EXTRA:500")
echo "$NP passed, $NF failed"
[ "$NF" = 0 ]
