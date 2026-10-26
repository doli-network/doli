#!/usr/bin/env bash
# test_gauntlet_gs027.sh — offline companion for GS-027 (INC-I-255 M3). Never starts a node; < 15 s.
# covers: scripts/gauntlet-gs027.sh scripts/gauntlet-devnet-lib.sh bins/node/examples/gs026_forge_block.rs scripts/gauntlet.sh scripts/gauntlet-seed.sql scripts/README.md specs/protocol.md specs/security_model.md docs/architecture.md docs/rewards.md docs/troubleshooting.md bins/node/src/node/snapshot_install.rs
# REQ-SLASH-005 — Decision: a FAIL means the GS-027 live equivocation proof cannot be trusted or cannot run — the runner is missing, its exit code is not its verdict, a pure helper miscounts mined slash txs, producer status, the supply delta, tip equality or the shared slash id (so the control-FAIL / HEAD-PASS split could flip for the wrong reason), library mode has side effects, a gate export or an env var is missing or undocumented, the forge helper cannot sign a sibling block with the producer's own key or prints key material, ids drift between runner, seed and DB, gauntlet.sh has no SKIP arm, or the runner can reach the live local fleet.
# REQ-SLASH-006 — Decision: a FAIL means the INC-I-181 snap tests that refuse a fabricated queued Slash were renamed or lost their Slash fixture, so snap sync could install a slash the chain never applied with no test turning red.
# REQ-LDP-025 — Decision: a FAIL means the shared devnet library split broke a GS-027 contract idiom, has side effects when sourced, reads a runner-specific variable or is not sourced at all, so GS-026/GS-027 regressions and the new runners would rest on an unproven harness.
# REQ-LDP-026 — Decision: no offline check here; a GS-027 regression run after the library split with no gauntlet_runs row (git sha, status, runner lines) leaves the split unproven live, so M1 cannot close.
# REQ-SLASH-007 — Decision: a FAIL means the slashing records drifted from the code — no GS-027 scenario row in memory.db, or the protocol, security and architecture docs do not name the slashing gate, the per-block cap and the burn and admission modules — so operators read a slashing model the binary does not run.
# OUTPUT CONTRACT:
#   O1 stdout   `PASS <case>` | `FAIL <case>: <why>` per case; last line `<N> passed, <M> failed`
#   O2 exit     0 all PASS · 1 any FAIL
#   O3 effects  only one mktemp dir (deleted on exit); the runner is sourced only with
#               GS027_LIB_ONLY=1, after a static guard check, with TMPDIR inside that dir
#   subject outputs read: runner file (present/exec/parse, header, emitted ids, METRIC,
#     verdict tail, env reads, gate exports, log level, scratch+kill idiom, forbidden
#     patterns), lib helper stdout + rc, forge example text, seed VALUES block, memory.db
#     GS-027 row (read-only), assert() rc + SKIP_REASONS + FAIL_REASONS, README GS-027
#     entry, three docs, the INC-I-181 snap test file.
# INPUT PARTITIONS:
#   runner            present · executable · bash -n · OUTPUT CONTRACT in the first 40 lines ·
#                     pipefail|PIPESTATUS · `exit 2` SKIP path · METRIC SLASHED_NODES= ·
#                     last code line calls verdict
#   lib-mode          GS027_LIB_ONLY guard before any top-level mktemp/trap/curl/lsof/
#                     start_node/sleep/nohup · sourcing writes nothing under TMPDIR, starts
#                     no job, defines the 6 helpers + res
#   count_slash_txs   block result with txs expanded to getTransaction objects: 1 slash_producer
#                     among coinbase+transfer (envelope) -> 1 · 2 slashes (bare result) -> 2 ·
#                     none -> 0 · empty list -> 0 · raw tx-hash strings (type unknown) -> rc!=0 ·
#                     error envelope -> rc!=0 · malformed JSON -> rc!=0
#   producer_status   getProducers list hit (slashed, active) · getProducer object hit ·
#                     getProducer object of another key -> empty+rc!=0 · unknown key -> empty+rc!=0 ·
#                     `Producer not found` envelope -> empty+rc!=0
#   supply_delta_ok   burn (delta = reward-bond < 0) -> 0 · large base units -> 0 · no burn
#                     (delta = +reward) -> 1 · off by one -> 1 · non-numeric / empty -> 1
#   all_equal         3 equal -> 0 · 2 differ -> !=0 · 1 arg -> !=0 · 0 args -> !=0 · an empty arg -> !=0
#   slash_ids_from_log  handler `slash <id> for <pk> admitted` (plain + ANSI, one id twice) -> 2 ids;
#                     not admitted / already reported / Added transaction / Queued SLASH /
#                     EQUIVOCATION / `[UTXO] FAIL type=SlashProducer` lines -> no id · empty log -> none
#   verdict           no res -> 0 · only PASS -> 0 · any FAIL -> 1; `res <id> 1|0 <kv>` prints
#                     `PASS|FAIL <id> <kv>`
#   env / node-env    every GS027_* and DOLI_*BIN* the runner reads is in its header and the
#                     README GS-027 entry · both gate vars set for the nodes · --log-level info
#   forge-sibling     "sibling" kind · key-file flag · no print naming private/secret/seed ·
#                     planted secret print -> flagged
#   seed-parity       seed block (runner, INC-I-255, expect_fail 0, the 7-id set) · ids the runner
#                     emits · live memory.db row; absent row -> FAIL (expected RED until registered)
#   gauntlet-skip-arm each gs027 id -> rc 2 + SKIP_REASONS names the runner; unknown token -> rc 1
#   harness-safety    runner + library clean · planted pkill / foreign kill / foreign rm / ln /
#                     ~/testnet write / .doli/devnet write -> 6 of 6 flagged · library cleanup arm
#                     `*/"$DN_TAG".*) rm -rf "$S"` exempt like the runner's gs027 arm
#   devnet-lib        present · bash -n · sourced by the runner · no top-level statement ·
#                     sourcing writes nothing under TMPDIR, starts no job, defines the cluster
#                     functions · reads no GS0NN_* variable, no gs0NN. glob · planted top-level
#                     sleep / trap and a GS028_* read -> 3 of 3 flagged
#   docs              README GS-027 line · protocol.md gate + MAX_SLASH_TXS_PER_BLOCK ·
#                     security_model.md gate · architecture.md slash_burn.rs + slash_admission.rs
#   r6-snap-pin       INC-I-181 snap test file defines test_181_001 + test_181_005, builds a Slash

# shellcheck disable=SC2016
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNNER="$ROOT/scripts/gauntlet-gs027.sh"
FORGE="$ROOT/bins/node/examples/gs026_forge_block.rs"
GAUNTLET="$ROOT/scripts/gauntlet.sh"
SEED="$ROOT/scripts/gauntlet-seed.sql"
README="$ROOT/scripts/README.md"
LIB="$ROOT/scripts/gauntlet-devnet-lib.sh"
R6="$ROOT/bins/node/tests/it/inc_i_181_snap_pending_updates.rs"
IDS="gs027-one-slash-mined gs027-producer-slashed-everywhere gs027-bond-burned-once gs027-tips-equal gs027-production-continues gs027-mempool-slash-bounded gs027-one-shared-slash-id"
HELPERS="count_slash_txs producer_status supply_delta_ok all_equal slash_ids_from_log verdict res"
LIB_FUNCS="skip alive ours stop_one cleanup start_node start_cluster seed_gset gset_ready wait_height inject_sibling all_equal write_chainspec gen_wallets check_ports restart_node cli_as wait_tx_mined sampler_start sampler_stop"
T="$(mktemp -d "${TMPDIR:-/tmp}/t-gs027.XXXXXX")" || exit 1
trap 'rm -rf "$T"' EXIT
NP=0
NF=0
LIB_OK=0
pass() { echo "PASS $1${2:+ ($2)}"; NP=$((NP + 1)); }
fail() { echo "FAIL $1: $2"; NF=$((NF + 1)); }
sorted() { tr ' ,' '\n\n' | sed '/^$/d' | sort -u | tr '\n' ' ' | sed 's/ $//'; }
code() { grep -vE '^[[:space:]]*#' "$1"; }
code_all() { code "$RUNNER"; [ -f "$LIB" ] && code "$LIB"; return 0; }
hx() { printf '%064d' 0 | tr 0 "$1"; }
WANT="$(echo "$IDS" | sorted)"

resolve_db() {
  local db common
  db="${GS027_DB:-$ROOT/.omega/memory.db}"
  [ -f "$db" ] && { printf '%s' "$db"; return 0; }
  common="$(git -C "$ROOT" rev-parse --git-common-dir 2>/dev/null)" || return 1
  [ -n "$common" ] || return 1
  case "$common" in /*) ;; *) common="$ROOT/$common" ;; esac
  db="$(cd "$common/.." 2>/dev/null && pwd)/.omega/memory.db"
  [ -f "$db" ] && { printf '%s' "$db"; return 0; }
  return 1
}

lib() {
  TMPDIR="$T/libtmp" GS027_LIB_ONLY=1 bash -c 'src=$1; shift; a=("$@"); set --
    . "$src" >/dev/null 2>&1 || exit 98
    "${a[@]}"' _ "$RUNNER" "$@"
}
lib_ready() { [ "$LIB_OK" = 1 ] && return 0; fail "$1" "library mode unavailable (see lib-mode)"; return 1; }

c_runner_present() {
  [ -f "$RUNNER" ] || { fail runner-present "scripts/gauntlet-gs027.sh missing"; return; }
  [ -x "$RUNNER" ] || { fail runner-present "scripts/gauntlet-gs027.sh not executable (bare-name run would exit 126)"; return; }
  bash -n "$RUNNER" 2>"$T/n.err" || { fail runner-present "bash -n: $(head -1 "$T/n.err")"; return; }
  pass runner-present
}

c_output_contract() {
  [ -f "$RUNNER" ] || { fail output-contract "runner absent"; return; }
  head -n 40 "$RUNNER" | grep -q 'OUTPUT CONTRACT:' ||
    { fail output-contract "no 'OUTPUT CONTRACT:' header in the first 40 lines"; return; }
  pass output-contract
}

c_exit_contract() {
  local c last
  [ -f "$RUNNER" ] || { fail exit-contract "runner absent"; return; }
  c="$(code "$RUNNER")"
  printf '%s\n' "$c" | grep -qE 'set -[a-z]*o[[:space:]]+pipefail|PIPESTATUS' ||
    { fail exit-contract "neither set -o pipefail nor PIPESTATUS: a failed probe in a pipe would read as success"; return; }
  code_all | grep -qE '(^|[^[:alnum:]_])exit[[:space:]]+2([^0-9]|$)' ||
    { fail exit-contract "no 'exit 2' SKIP path"; return; }
  printf '%s\n' "$c" | grep -qF 'METRIC SLASHED_NODES=' ||
    { fail exit-contract "no 'METRIC SLASHED_NODES=' line"; return; }
  last="$(printf '%s\n' "$c" | sed '/^[[:space:]]*$/d' | tail -1)"
  printf '%s' "$last" | grep -qE '(^|[^[:alnum:]_])verdict([^[:alnum:]_(]|$)' ||
    { fail exit-contract "last code line '$last' does not call verdict, so the exit code is not the verdict"; return; }
  pass exit-contract
}

c_lib_mode() {
  local guard first out
  [ -f "$RUNNER" ] || { fail lib-mode "runner absent"; return; }
  guard="$(awk '/^[^#]*GS027_LIB_ONLY/ { c = NR } c && NR - c <= 2 && /(^|[^A-Za-z0-9_])return([^A-Za-z0-9_]|$)/ { print NR; exit }' "$RUNNER")"
  [ -n "$guard" ] || { fail lib-mode "no GS027_LIB_ONLY guard followed by return; not sourced"; return; }
  first="$(grep -nE '^[^[:space:]#]' "$RUNNER" | grep -vE '^[0-9]+:[A-Za-z_][A-Za-z0-9_]*[[:space:]]*\(\)' |
    grep -E '(^|[^[:alnum:]_])(mktemp|trap|curl|lsof|start_node|sleep|nohup)([^[:alnum:]_]|$)' | head -1 | cut -d: -f1)"
  if [ -n "$first" ] && [ "$first" -lt "$guard" ]; then
    fail lib-mode "top-level side effect at line $first precedes the library guard at line $guard; not sourced"
    return
  fi
  mkdir -p "$T/libtmp"
  out="$(TMPDIR="$T/libtmp" GS027_LIB_ONLY=1 bash -c 'src=$1; h=$2; set --
    . "$src" >/dev/null 2>&1 || { echo "source-rc:$?"; exit 0; }
    for f in $h; do declare -F "$f" >/dev/null || echo "missing:$f"; done
    [ -z "$(jobs -p)" ] || echo "jobs:$(jobs -p | tr "\n" " ")"
    echo sourced' _ "$RUNNER" "$HELPERS" 2>&1)"
  printf '%s\n' "$out" | grep -qx sourced || { fail lib-mode "sourcing with GS027_LIB_ONLY=1 did not return: $(printf '%s' "$out" | tr '\n' ' ')"; return; }
  printf '%s\n' "$out" | grep -q '^source-rc:' && { fail lib-mode "sourcing returned non-zero: $(printf '%s' "$out" | tr '\n' ' ')"; return; }
  printf '%s\n' "$out" | grep -q '^missing:' && { fail lib-mode "helpers undefined:$(printf '%s\n' "$out" | sed -n 's/^missing:/ /p' | tr -d '\n')"; return; }
  printf '%s\n' "$out" | grep -q '^jobs:' && { fail lib-mode "library mode started background jobs"; return; }
  [ -z "$(ls -A "$T/libtmp")" ] || { fail lib-mode "library mode created $(ls -A "$T/libtmp" | head -1) under TMPDIR"; return; }
  LIB_OK=1
  pass lib-mode "guard at line $guard, no side effect, 7 functions defined"
}

mk_fixtures() {
  local PKA PKB PKC H1 H2 H3 H4 HB HP HM CB TR SL SL2
  PKA="$(hx a)"; PKB="$(hx b)"; PKC="$(hx c)"
  H1="$(hx 1)"; H2="$(hx 2)"; H3="$(hx 3)"; H4="$(hx 4)"; HB="$(hx 5)"; HP="$(hx 6)"; HM="$(hx 7)"
  CB="{\"hash\":\"$H1\",\"version\":1,\"txType\":\"coinbase\",\"inputs\":[],\"outputs\":[],\"size\":120}"
  TR="{\"hash\":\"$H2\",\"version\":1,\"txType\":\"transfer\",\"inputs\":[],\"outputs\":[],\"size\":250,\"fee\":1000}"
  SL="{\"hash\":\"$H3\",\"version\":1,\"txType\":\"slash_producer\",\"inputs\":[],\"outputs\":[],\"size\":700,\"fee\":0}"
  SL2="{\"hash\":\"$H4\",\"version\":1,\"txType\":\"slash_producer\",\"inputs\":[],\"outputs\":[],\"size\":700,\"fee\":0}"
  blk() { printf '{"hash":"%s","prevHash":"%s","height":%s,"slot":%s,"timestamp":%s,"producer":"%s","merkleRoot":"%s","txCount":%s,"transactions":[%s],"size":900}' \
    "$HB" "$HP" "$1" "$1" "$((1000 + $1))" "$PKA" "$HM" "$2" "$3"; }
  env_ok() { printf '{"jsonrpc":"2.0","result":%s,"id":1}' "$1"; }
  env_ok "$(blk 41 3 "$CB,$TR,$SL")" >"$T/blk_one.json"
  blk 42 3 "$CB,$SL,$SL2" >"$T/blk_two.json"
  env_ok "$(blk 43 2 "$CB,$TR")" >"$T/blk_none.json"
  env_ok "$(blk 44 0 "")" >"$T/blk_empty.json"
  env_ok "$(blk 45 2 "\"$H1\",\"$H3\"")" >"$T/blk_hashes.json"
  printf '{"jsonrpc":"2.0","error":{"code":-32000,"message":"Block not found","data":{"height":99}},"id":1}' >"$T/blk_err.json"
  printf '{"jsonrpc":"2.0","result":{"hash":"%s","transactions":[{"txType":"slash_' "$HB" >"$T/blk_bad.json"
  prod() { printf '{"publicKey":"%s","addressHash":"%s","registrationHeight":0,"bondAmount":%s,"bondCount":%s,"producerSetBondCount":%s,"status":"%s","era":0,"pendingWithdrawals":[],"delegatedBonds":0,"selectionWeight":%s}' \
    "$1" "$HM" "$3" "$4" "$4" "$2" "$4"; }
  env_ok "[$(prod "$PKA" active 100000000 1),$(prod "$PKB" slashed 0 0)]" >"$T/prod_list.json"
  env_ok "$(prod "$PKB" slashed 0 0)" >"$T/prod_one.json"
  printf '{"jsonrpc":"2.0","error":{"code":-32006,"message":"Producer not found"},"id":1}' >"$T/prod_err.json"
  printf '%s\n' "$PKA" >"$T/pka"; printf '%s\n' "$PKB" >"$T/pkb"; printf '%s\n' "$PKC" >"$T/pkc"
}

c_count_slash_txs() {
  local spec f want got rc
  lib_ready count-slash-txs || return
  for spec in blk_one.json:1 blk_two.json:2 blk_none.json:0 blk_empty.json:0; do
    f="${spec%%:*}"
    want="${spec##*:}"
    got="$(lib count_slash_txs "$T/$f" 2>/dev/null)"
    rc=$?
    { [ "$rc" = 0 ] && [ "$got" = "$want" ]; } ||
      { fail count-slash-txs "$f -> '$got' rc $rc, want '$want' rc 0"; return; }
  done
  for f in blk_hashes.json blk_err.json blk_bad.json; do
    got="$(lib count_slash_txs "$T/$f" 2>/dev/null)"
    rc=$?
    [ "$rc" != 0 ] || { fail count-slash-txs "$f -> '$got' rc 0, want rc != 0 (an unclassifiable block must not count as 0)"; return; }
  done
  pass count-slash-txs "4 counts, 3 fail-loud shapes"
}

c_producer_status() {
  local spec f k want got rc
  lib_ready producer-status || return
  for spec in prod_list.json:pkb:slashed prod_list.json:pka:active prod_one.json:pkb:slashed; do
    IFS=: read -r f k want <<<"$spec"
    got="$(lib producer_status "$T/$f" "$(cat "$T/$k")" 2>/dev/null)"
    rc=$?
    { [ "$rc" = 0 ] && [ "$got" = "$want" ]; } ||
      { fail producer-status "$f $k -> '$got' rc $rc, want '$want' rc 0"; return; }
  done
  for spec in prod_list.json:pkc prod_one.json:pka prod_err.json:pkb; do
    IFS=: read -r f k <<<"$spec"
    got="$(lib producer_status "$T/$f" "$(cat "$T/$k")" 2>/dev/null)"
    rc=$?
    { [ "$rc" != 0 ] && [ -z "$got" ]; } ||
      { fail producer-status "$f $k (absent) -> '$got' rc $rc, want empty + rc != 0"; return; }
  done
  pass producer-status "3 hits, 3 absent"
}

c_supply_delta_ok() {
  local spec rc args
  lib_ready supply-delta-ok || return
  for spec in "100 1000 5000 4100" "100000000 1000000000 2500000000000 2499100000000" "100000000 100000000 100000000000000 100000000000000"; do
    read -r -a args <<<"$spec"
    lib supply_delta_ok "${args[@]}" >/dev/null 2>&1
    rc=$?
    [ "$rc" = 0 ] || { fail supply-delta-ok "supply_delta_ok $spec -> rc $rc, want 0"; return; }
  done
  for spec in "100 1000 5000 5100" "100 1000 5000 4101" "100 1000 x5000 4100" "100 abc 5000 4100"; do
    read -r -a args <<<"$spec"
    lib supply_delta_ok "${args[@]}" >/dev/null 2>&1
    rc=$?
    [ "$rc" = 1 ] || { fail supply-delta-ok "supply_delta_ok $spec -> rc $rc, want 1"; return; }
  done
  lib supply_delta_ok 100 1000 "" 4100 >/dev/null 2>&1
  rc=$?
  [ "$rc" = 1 ] || { fail supply-delta-ok "empty BEFORE -> rc $rc, want 1"; return; }
  pass supply-delta-ok "3 equal deltas, 5 rejects"
}

c_all_equal() {
  local a b
  lib_ready all-equal || return
  a="$(hx 9)"
  b="$(hx 8)"
  lib all_equal "$a" "$a" "$a" >/dev/null 2>&1 || { fail all-equal "three equal tips -> rc $?, want 0"; return; }
  lib all_equal "$a" "$b" >/dev/null 2>&1 && { fail all-equal "two different tips -> rc 0"; return; }
  lib all_equal "$a" >/dev/null 2>&1 && { fail all-equal "one arg -> rc 0, want != 0"; return; }
  lib all_equal >/dev/null 2>&1 && { fail all-equal "no args -> rc 0, want != 0"; return; }
  lib all_equal "$a" "" >/dev/null 2>&1 && { fail all-equal "an empty value -> rc 0, want != 0"; return; }
  lib all_equal "" "" >/dev/null 2>&1 && { fail all-equal "two empty values -> rc 0, want != 0"; return; }
  pass all-equal "1 equal, 5 rejects"
}

c_slash_ids_from_log() {
  local TS E PKA PKB S1 S2 S8 S9 HX PH HH1 HH2 got want
  lib_ready slash-ids-from-log || return
  TS="$(date -u +%Y-%m-%dT%H:%M:%S.000000Z)"
  E="$(printf '\033')"
  PKA="$(hx a)"; PKB="$(hx b)"; S1="$(hx d)"; S2="$(hx e)"; S8="$(hx 8)"; S9="$(hx 9)"
  HX="$(hx f)"; PH="$(hx 5)"; HH1="$(hx 6)"; HH2="$(hx 7)"
  {
    echo "$TS  INFO doli_node::node::validation_checks: Added transaction $HX to mempool"
    echo "$TS  WARN doli_node::node::rewards: EQUIVOCATION: producer $PH made two headers for slot 44: $HH1 and $HH2"
    echo "$TS  INFO doli_node::node::validation_checks: Added transaction $S1 to mempool"
    echo "$TS  INFO doli_node::node::slash_submit: slash $S1 for $PKB admitted"
    echo "$TS DEBUG doli_node::node::slash_submit: slash $S9 for $PKB not admitted"
    echo "$TS DEBUG doli_node::node::slash_submit: slash for $PKB already reported; not resubmitted"
    echo "${E}[2m$TS${E}[0m ${E}[32m INFO${E}[0m ${E}[2mdoli_node::node::slash_submit${E}[0m${E}[2m:${E}[0m slash $S2 for $PKA admitted"
    echo "$TS  INFO doli_node::node::slash_submit: slash $S1 for $PKB admitted"
    echo "$TS  WARN doli_node::node::apply_block::tx_processing: Queued SLASH for producer $PH at height 45 (deferred to epoch boundary)"
    echo "$TS  WARN doli_node::node::apply_block::tx_processing: [UTXO] FAIL h=45 tx=$S8 type=SlashProducer inputs=0 outputs=0 error=invalid slash"
  } >"$T/node.log"
  : >"$T/empty.log"
  got="$(lib slash_ids_from_log "$T/node.log" 2>/dev/null | sed '/^$/d' | sort)"
  want="$(printf '%s\n' "$S1" "$S2" | sort)"
  [ "$got" = "$want" ] ||
    { fail slash-ids-from-log "got {$(printf '%s' "$got" | cut -c1-8 | tr '\n' ' ')} want the 2 handler-admitted ids {$(printf '%s' "$want" | cut -c1-8 | tr '\n' ' ')}"; return; }
  got="$(lib slash_ids_from_log "$T/empty.log" 2>/dev/null | sed '/^$/d')"
  [ -z "$got" ] || { fail slash-ids-from-log "empty log -> '$got', want nothing"; return; }
  pass slash-ids-from-log "2 distinct admitted ids, 6 decoy lines ignored"
}

c_verdict() {
  local out rc
  lib_ready verdict || return
  lib verdict >/dev/null 2>&1
  rc=$?
  [ "$rc" = 0 ] || { fail verdict "no recorded line -> rc $rc, want 0"; return; }
  out="$(lib eval 'res gs027-tips-equal 1 tips=3/3; res gs027-production-continues 1 advanced=3/3; verdict' 2>/dev/null)"
  rc=$?
  [ "$rc" = 0 ] || { fail verdict "only PASS lines -> rc $rc, want 0"; return; }
  printf '%s\n' "$out" | grep -qx 'PASS gs027-tips-equal tips=3/3' ||
    { fail verdict "res did not print 'PASS gs027-tips-equal tips=3/3': $(printf '%s' "$out" | head -1)"; return; }
  out="$(lib eval 'res gs027-tips-equal 1 tips=3/3; res gs027-bond-burned-once 0 burned=0/3; res gs027-production-continues 1 advanced=3/3; verdict' 2>/dev/null)"
  rc=$?
  [ "$rc" = 1 ] || { fail verdict "one FAIL among PASS lines -> rc $rc, want 1"; return; }
  printf '%s\n' "$out" | grep -qx 'FAIL gs027-bond-burned-once burned=0/3' ||
    { fail verdict "res did not print 'FAIL gs027-bond-burned-once burned=0/3'"; return; }
  pass verdict "none 0, PASS-only 0, any FAIL 1"
}

c_env_documented() {
  local names hdr entry n miss=""
  [ -f "$RUNNER" ] || { fail env-documented "runner absent"; return; }
  names="$(code "$RUNNER" | grep -oE 'GS027_[A-Z0-9_]+|\$\{DOLI_[A-Z0-9_]*BIN[A-Z0-9_]*' | sed 's/^\${//' | sort -u)"
  [ -n "$names" ] || { fail env-documented "the runner reads no GS027_* variable (GS027_LIB_ONLY at least)"; return; }
  hdr="$(awk 'NR == 1 { next } /^#/ { print; next } { exit }' "$RUNNER")"
  entry="$(awk 'index($0, "gauntlet-gs027.sh") && index($0, "GS-027") { on = 1; print; next } on && (/^[ \t]*$/ || /^- / || /^#/) { exit } on { print }' "$README" 2>/dev/null)"
  [ -n "$entry" ] || { fail env-documented "scripts/README.md has no GS-027 entry naming gauntlet-gs027.sh"; return; }
  for n in $names; do
    printf '%s\n' "$hdr" | grep -qw -- "$n" || miss="$miss header:$n"
    printf '%s\n' "$entry" | grep -qw -- "$n" || miss="$miss readme:$n"
  done
  [ -z "$miss" ] || { fail env-documented "undocumented:$miss"; return; }
  pass env-documented "$(printf '%s\n' "$names" | wc -l | tr -d ' ') vars in the runner header and the README entry"
}

c_node_env() {
  local c v
  [ -f "$RUNNER" ] || { fail node-env "runner absent"; return; }
  c="$(code_all)"
  for v in DOLI_INC_I_236_BLOCK_SIGNATURE_ACTIVATION_HEIGHT DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT; do
    printf '%s\n' "$c" | grep -qE "(^|[^[:alnum:]_])$v=" || { fail node-env "never sets $v for the nodes"; return; }
  done
  printf '%s\n' "$c" | grep -qE -- '--log-level[[:space:]=]+"?info' ||
    { fail node-env "nodes not started with --log-level info (the admission line is INFO)"; return; }
  pass node-env "both gates exported, --log-level info"
}

secret_prints() {
  grep -vE '^[[:space:]]*//' "$1" | grep -E '(e?print(ln)?|write(ln)?)!' | grep -iE 'private|secret|seed'
}

c_forge_sibling() {
  local c v n
  [ -f "$FORGE" ] || { fail forge-sibling "bins/node/examples/gs026_forge_block.rs missing"; return; }
  c="$(grep -vE '^[[:space:]]*//' "$FORGE")"
  printf '%s\n' "$c" | grep -qF '"sibling"' || { fail forge-sibling "no \"sibling\" --kind arm"; return; }
  printf '%s\n' "$c" | grep -qE '"--[a-z0-9-]*(key|wallet)[a-z0-9-]*"' ||
    { fail forge-sibling "no key-file / wallet path flag for the producer's own key"; return; }
  v="$(secret_prints "$FORGE")"
  [ -z "$v" ] || { fail forge-sibling "prints key material: $(printf '%s' "$v" | head -1 | sed 's/^[[:space:]]*//')"; return; }
  cp "$FORGE" "$T/planted.rs"
  printf '%s\n' '    eprintln!("signing with private key {}", hex::encode(sk));' >>"$T/planted.rs"
  n="$(secret_prints "$T/planted.rs" | wc -l | tr -d ' ')"
  [ "$n" = 1 ] || { fail forge-sibling "checker flagged $n of 1 planted secret print"; return; }
  pass forge-sibling "sibling kind + key-file flag, no secret print, planted print flagged"
}

c_seed_parity() {
  local blk flat got emit dbf row db_runner db_ef db_inc db_assert
  blk="$(awk -v k="VALUES ('GS-027'" 'index($0, k) { on = 1 } on { print } on && /^ON CONFLICT/ { exit }' "$SEED")"
  [ -n "$blk" ] || { fail seed-parity "no VALUES ('GS-027' block in scripts/gauntlet-seed.sql"; return; }
  printf '%s\n' "$blk" | grep -qF "'scripts/gauntlet-gs027.sh'" ||
    { fail seed-parity "GS-027 runner column is not 'scripts/gauntlet-gs027.sh'"; return; }
  printf '%s\n' "$blk" | grep -E "json\('\[" | grep -qF '"INC-I-255"' ||
    { fail seed-parity "GS-027 incident_ids lacks INC-I-255"; return; }
  flat="$(printf '%s\n' "$blk" | tr '\n' ' ' | tr -s ' ')"
  printf '%s' "$flat" | grep -qE "'scripts/gauntlet-gs027\.sh', 'active', 0\)" ||
    { fail seed-parity "GS-027 tail is not runner 'scripts/gauntlet-gs027.sh', status 'active', expect_fail 0"; return; }
  got="$(printf '%s\n' "$blk" | grep -oE "'gs027-[a-z0-9,-]+'" | tr -d "'" | sorted)"
  [ "$got" = "$WANT" ] || { fail seed-parity "seed assertions {${got}} != {${WANT}}"; return; }
  [ -f "$RUNNER" ] || { fail seed-parity "runner absent, cannot compare the ids it emits"; return; }
  emit="$(grep -oE "(PASS|FAIL)[[:space:]]+gs027-[a-z0-9-]+" "$RUNNER" | grep -oE "gs027-[a-z0-9-]+" | sorted)"
  [ -n "$emit" ] || emit="$(grep -oE "gs027-[a-z0-9-]+" "$RUNNER" | sorted)"
  [ "$emit" = "$WANT" ] || { fail seed-parity "ids the runner emits {${emit}} != {${WANT}}"; return; }
  dbf="$(resolve_db)" || { fail seed-parity "memory.db not found via env, .omega or git-common-dir"; return; }
  command -v sqlite3 >/dev/null 2>&1 || { fail seed-parity "sqlite3 unavailable, cannot verify the live GS-027 row"; return; }
  row="$(sqlite3 "$dbf" "SELECT runner||'|'||expect_fail||'|'||incident_ids||'|'||assertions FROM gauntlet_scenarios WHERE scenario_id='GS-027';" 2>/dev/null)"
  [ -n "$row" ] || { fail seed-parity "memory.db has no GS-027 row (scenario not registered yet)"; return; }
  db_runner="$(printf '%s' "$row" | cut -d'|' -f1)"
  db_ef="$(printf '%s' "$row" | cut -d'|' -f2)"
  db_inc="$(printf '%s' "$row" | cut -d'|' -f3)"
  db_assert="$(printf '%s' "$row" | cut -d'|' -f4-)"
  [ "$db_runner" = scripts/gauntlet-gs027.sh ] || { fail seed-parity "DB runner '$db_runner' != scripts/gauntlet-gs027.sh"; return; }
  [ "$db_ef" = 0 ] || { fail seed-parity "DB expect_fail '$db_ef' != 0"; return; }
  printf '%s' "$db_inc" | grep -qF 'INC-I-255' || { fail seed-parity "DB incident_ids lacks INC-I-255: $db_inc"; return; }
  [ "$(printf '%s' "$db_assert" | sorted)" = "$WANT" ] ||
    { fail seed-parity "DB assertions {$(printf '%s' "$db_assert" | sorted)} != {${WANT}}"; return; }
  pass seed-parity "7 ids: seed + runner-emitted + db agree, runner/incident/expect_fail pinned"
}

call_assert() {
  bash -c 'jget() { echo 0; }; . "$1" || exit 99
    SKIP_REASONS=""; FAIL_REASONS=""; INFO_REASONS=""
    assert "$2"; rc=$?
    printf "%s\t%s\t%s\n" "$rc" "$SKIP_REASONS" "$FAIL_REASONS"' _ "$T/assert.sh" "$1" 2>/dev/null
}

c_skip_arm() {
  local id out rc
  awk '/^assert\(\)[[:space:]]*\{/ { on = 1 } on { print } on && /^}/ { exit }' "$GAUNTLET" >"$T/assert.sh"
  grep -q '^assert()' "$T/assert.sh" || { fail gauntlet-skip-arm "assert() is not extractable from scripts/gauntlet.sh"; return; }
  rc="$(call_assert not-a-gs027-token | cut -f1)"
  [ "$rc" = 1 ] || { fail gauntlet-skip-arm "control: unknown token -> rc ${rc:-none}, want 1 (FAIL)"; return; }
  for id in $IDS; do
    out="$(call_assert "$id")"
    rc="$(printf '%s' "$out" | cut -f1)"
    [ "$rc" = 2 ] || { fail gauntlet-skip-arm "assert $id -> rc ${rc:-none}, want 2 (SKIP):$(printf '%s' "$out" | cut -f3)"; return; }
    printf '%s' "$out" | cut -f2 | grep -qF scripts/gauntlet-gs027.sh ||
      { fail gauntlet-skip-arm "SKIP_REASONS for $id does not name scripts/gauntlet-gs027.sh"; return; }
    [ -z "$(printf '%s' "$out" | cut -f3)" ] ||
      { fail gauntlet-skip-arm "assert $id wrote FAIL_REASONS:$(printf '%s' "$out" | cut -f3)"; return; }
  done
  pass gauntlet-skip-arm "7/7 rc 2 naming the runner, control unknown token rc 1"
}

unsafe() {
  local c p
  c="$(code "$1")"
  p='(~|\$HOME|\$\{HOME\})/testnet|\.doli/(devnet|testnet|mainnet)'
  printf '%s\n' "$c" | grep -E '(^|[^[:alnum:]_])(pkill|killall|launchctl|systemctl|ssh)([^[:alnum:]_]|$)|testnet\.sh' | sed 's/^/forbidden: /'
  printf '%s\n' "$c" | grep -E '(^|[;&|({[:space:]])ln[[:space:]]' | sed 's/^/link: /'
  printf '%s\n' "$c" | grep -E '(^|[^[:alnum:]_-])kill[[:space:]]' | sed -E 's/kill( -[A-Za-z0-9]+)? "\$PID"//g' |
    grep -E '(^|[^[:alnum:]_-])kill[[:space:]]' | sed 's/^/kill: /'
  printf '%s\n' "$c" | grep -E '(^|[^[:alnum:]_-])rm[[:space:]]+-[A-Za-z]*[rR]' |
    grep -vE '(gs027|"\$DN_TAG")\.[^)]*\)[[:space:]]*rm -rf "\$S"' | sed 's/^/rm: /'
  printf '%s\n' "$c" | grep -E "$p" |
    grep -E "(^|[^[:alnum:]_-])(rm|cp|mv|mkdir|touch|tee|rsync|tar|install|dd)([^[:alnum:]_-]|$)|>>?[[:space:]]*\"?[^[:space:]]*($p)" |
    sed 's/^/path: /'
}

c_harness_safety() {
  local v n
  [ -f "$RUNNER" ] || { fail harness-safety "runner absent"; return; }
  v="$(unsafe "$RUNNER"; [ -f "$LIB" ] && unsafe "$LIB")"
  [ -z "$v" ] || { fail harness-safety "$(printf '%s' "$v" | head -3 | tr '\n' ' ')"; return; }
  grep -qE 'mktemp[[:space:]]+-d[[:space:]]+.*gs027\.X+' "$RUNNER" ||
    { fail harness-safety "scratch is not a mktemp -d gs027.XXXXXX dir"; return; }
  code_all | grep -qE 'kill[[:space:]]+(-[A-Za-z0-9]+[[:space:]]+)?"\$PID"' ||
    { fail harness-safety "runner never signals only its own \$PID"; return; }
  cp "$RUNNER" "$T/planted.sh"
  printf '%s\n' 'pkill -f doli-node' 'kill -TERM "$SEED_PID"' 'rm -rf "$GS027_TESTNET_DIR"' 'ln -s "$CHECKPOINT" "$S/data"' \
    'cp "$S/chainspec.json" ~/testnet/n1/' 'mkdir -p "$HOME/.doli/devnet/keys"' >>"$T/planted.sh"
  n="$(unsafe "$T/planted.sh" | wc -l | tr -d ' ')"
  [ "$n" = 6 ] || { fail harness-safety "checker flagged $n of 6 planted violations"; return; }
  pass harness-safety "runner clean, 6/6 planted violations flagged"
}

# Top-level statements (outside any function body) and runner-specific reads of a sourced library.
lib_violations() {
  awk '
    /^[[:space:]]*(#|$)/ { next }
    d == 0 && /^[A-Za-z_][A-Za-z0-9_]*[[:space:]]*\(\)[[:space:]]*\{/ { if ($0 !~ /\}[[:space:]]*$/) d = 1; next }
    d == 0 && /^[A-Za-z_][A-Za-z0-9_]*[[:space:]]*\(\)[[:space:]]*$/ { d = 1; next }
    d == 1 && /^\}/ { d = 0; next }
    d == 0 { print "top-level: " NR ": " $0 }' "$1"
  code "$1" | grep -E 'GS0[0-9]{2}_[A-Z0-9_]+|gs0[0-9]{2}\.' | sed 's/^/runner-var: /'
  code "$1" | grep -q 'DN_TAG' || echo "no-dn-tag: the library never reads DN_TAG"
}

c_devnet_lib() {
  local v out n
  [ -f "$LIB" ] || { fail devnet-lib "scripts/gauntlet-devnet-lib.sh missing"; return; }
  bash -n "$LIB" 2>"$T/l.err" || { fail devnet-lib "bash -n: $(head -1 "$T/l.err")"; return; }
  [ -f "$RUNNER" ] && code "$RUNNER" |
    grep -qE '^[[:space:]]*(\.|source)[[:space:]]+"?\$\{?ROOT\}?/scripts/gauntlet-devnet-lib\.sh"?' ||
    { fail devnet-lib "the runner does not source \$ROOT/scripts/gauntlet-devnet-lib.sh"; return; }
  v="$(lib_violations "$LIB")"
  [ -z "$v" ] || { fail devnet-lib "$(printf '%s' "$v" | head -2 | tr '\n' ' ')"; return; }
  mkdir -p "$T/srctmp"
  out="$(TMPDIR="$T/srctmp" bash -c 'src=$1; fs=$2; set --
    . "$src" >/dev/null 2>&1 || { echo "source-rc:$?"; exit 0; }
    for f in $fs; do declare -F "$f" >/dev/null || echo "missing:$f"; done
    [ -z "$(jobs -p)" ] || echo "jobs:$(jobs -p | tr "\n" " ")"
    echo sourced' _ "$LIB" "$LIB_FUNCS" 2>&1)"
  printf '%s\n' "$out" | grep -qx sourced || { fail devnet-lib "sourcing did not complete: $(printf '%s' "$out" | tr '\n' ' ')"; return; }
  printf '%s\n' "$out" | grep -q '^source-rc:' && { fail devnet-lib "sourcing returned non-zero"; return; }
  printf '%s\n' "$out" | grep -q '^missing:' && { fail devnet-lib "functions undefined:$(printf '%s\n' "$out" | sed -n 's/^missing:/ /p' | tr -d '\n')"; return; }
  printf '%s\n' "$out" | grep -q '^jobs:' && { fail devnet-lib "sourcing started background jobs"; return; }
  [ -z "$(ls -A "$T/srctmp")" ] || { fail devnet-lib "sourcing created $(ls -A "$T/srctmp" | head -1) under TMPDIR"; return; }
  cp "$LIB" "$T/planted-lib.sh"
  printf '%s\n' 'sleep 1' 'trap cleanup EXIT' 'leak() { echo "${GS028_AH:-}"; }' >>"$T/planted-lib.sh"
  n="$(lib_violations "$T/planted-lib.sh" | wc -l | tr -d ' ')"
  [ "$n" = 3 ] || { fail devnet-lib "checker flagged $n of 3 planted library violations"; return; }
  pass devnet-lib "sourced by the runner, functions only, DN_* only, 3/3 planted violations flagged"
}

c_docs_readme() {
  [ -f "$README" ] || { fail docs-readme "scripts/README.md missing"; return; }
  grep -F 'gauntlet-gs027.sh' "$README" | grep -q 'GS-027' ||
    { fail docs-readme "scripts/README.md has no GS-027 entry line naming gauntlet-gs027.sh"; return; }
  pass docs-readme
}

doc_names() {
  local case="$1" file="$2" n
  shift 2
  [ -f "$ROOT/$file" ] || { fail "$case" "$file missing"; return; }
  for n in "$@"; do
    grep -qF -- "$n" "$ROOT/$file" || { fail "$case" "$file does not name $n"; return; }
  done
  pass "$case"
}

c_r6_snap_pin() {
  local t
  [ -f "$R6" ] || { fail r6-snap-pin "bins/node/tests/it/inc_i_181_snap_pending_updates.rs missing"; return; }
  for t in test_181_001_legacy_forged_pending_update_refused test_181_005_staged_forged_pending_update_refused; do
    grep -qE "fn $t[[:space:]]*\(" "$R6" || { fail r6-snap-pin "$t no longer defined"; return; }
  done
  grep -qF 'PendingProducerUpdate::Slash' "$R6" || { fail r6-snap-pin "no PendingProducerUpdate::Slash fixture"; return; }
  pass r6-snap-pin "legacy + staged forged-Slash refusals present"
}

mk_fixtures
c_runner_present
c_output_contract
c_exit_contract
c_lib_mode
c_count_slash_txs
c_producer_status
c_supply_delta_ok
c_all_equal
c_slash_ids_from_log
c_verdict
c_env_documented
c_node_env
c_forge_sibling
c_seed_parity
c_skip_arm
c_harness_safety
c_devnet_lib
c_docs_readme
doc_names docs-protocol specs/protocol.md inc_i_255_slashing_activation_height MAX_SLASH_TXS_PER_BLOCK
doc_names docs-security-model specs/security_model.md inc_i_255_slashing_activation_height
doc_names docs-architecture docs/architecture.md slash_burn.rs slash_admission.rs
c_r6_snap_pin
echo "$NP passed, $NF failed"
[ "$NF" = 0 ]
