#!/usr/bin/env bash
# test_gauntlet_gs026.sh — offline companion for GS-026 (INC-I-236 M3). Never starts a node; < 15 s.
# covers: scripts/gauntlet-gs026.sh bins/node/examples/gs026_forge_block.rs scripts/gauntlet.sh scripts/gauntlet-seed.sql scripts/README.md
# REQ-AUTH-024 — Decision: a FAIL means the GS-026 block-signature deliverable cannot enter a gauntlet run cleanly — the runner is missing/unparseable, the forge helper is absent, the seed/emitted/live-DB assertion ids drift from the frozen four, gauntlet.sh has no SKIP arm so the standalone ids FAIL a default run, the README lost its entry, or the runner carries an unscoped kill/rm/ln/pkill/ssh that could reach the live local fleet; the live FAIL-on-control / PASS-on-HEAD proof is the runner itself, so this file guards only registration, SKIP routing, structure and harness safety.
# REQ-AUTH-025 — Decision: a FAIL of c_seed_parity means the GS-026 records never reached memory.db (the gauntlet_scenarios row with runner/expect_fail/incident/assertions), so the scenario is invisible to v_gauntlet_coverage and the INC-I-236 incident carries no protection-scenario row; the records requirement is otherwise verified by the orchestrator's DB row ids, this file checks only the one records artifact that lives in file-checkable form.
# OUTPUT CONTRACT:
#   O1 stdout   `PASS <case>` | `FAIL <case>: <why>` per case; last line `<N> passed, <M> failed`
#   O2 exit     0 all PASS · 1 any FAIL
#   O3 effects  only one mktemp dir (deleted on exit); reads the runner, the forge example,
#               gauntlet.sh assert(), the seed SQL, the live memory.db row (read-only), the README
#   subject outputs read: runner file (present/exec/parse, emitted ids, scratch+kill idiom,
#     forbidden patterns), head of the runner (OUTPUT CONTRACT), seed VALUES block columns,
#     memory.db GS-026 row columns, assert() rc + SKIP_REASONS + FAIL_REASONS, README text.
# INPUT PARTITIONS:
#   runner-present    present · executable · parses under bash -n
#   output-contract   OUTPUT CONTRACT: header in the first 40 lines of the runner
#   forge-example     bins/node/examples/gs026_forge_block.rs present
#   seed-parity       GS-026 seed block (runner, INC-I-236, expect_fail 0, the 4-id set) ·
#                     ids the runner emits · live memory.db row (runner, expect_fail, incident, set);
#                     absent DB row -> FAIL (the expected RED state until the runner registers it)
#   gauntlet-skip-arm each gs026 id -> rc 2 + SKIP_REASONS names scripts/gauntlet-gs026.sh;
#                     control: an unknown token -> rc 1
#   harness-safety    runner clean · planted pkill / foreign kill / testnet rm / ln -> 4 of 4 flagged
#   docs              scripts/README.md carries a GS-026 entry line

# shellcheck disable=SC2016  # literal $VARS are grep patterns, not expansions
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNNER="$ROOT/scripts/gauntlet-gs026.sh"
FORGE="$ROOT/bins/node/examples/gs026_forge_block.rs"
GAUNTLET="$ROOT/scripts/gauntlet.sh"
SEED="$ROOT/scripts/gauntlet-seed.sql"
README="$ROOT/scripts/README.md"
IDS="gs026-mixed-below-height-equal-tips gs026-crossing-height-no-fork gs026-forged-rejected gs026-unsigned-rejected"
T="$(mktemp -d "${TMPDIR:-/tmp}/t-gs026.XXXXXX")" || exit 1
trap 'rm -rf "$T"' EXIT
NP=0
NF=0
pass() { echo "PASS $1${2:+ ($2)}"; NP=$((NP + 1)); }
fail() { echo "FAIL $1: $2"; NF=$((NF + 1)); }
sorted() { tr ' ,' '\n\n' | sed '/^$/d' | sort -u | tr '\n' ' ' | sed 's/ $//'; }
WANT="$(echo "$IDS" | sorted)"

# Resolve memory.db the way the GS-020/025 runners do: env override, then this checkout's
# .omega, then the main checkout found through git's common dir (a worktree has no .omega).
resolve_db() {
  local db common
  db="${GS026_DB:-$ROOT/.omega/memory.db}"
  [ -f "$db" ] && { printf '%s' "$db"; return 0; }
  common="$(git -C "$ROOT" rev-parse --git-common-dir 2>/dev/null)" || return 1
  [ -n "$common" ] || return 1
  case "$common" in /*) ;; *) common="$ROOT/$common" ;; esac
  db="$(cd "$common/.." 2>/dev/null && pwd)/.omega/memory.db"
  [ -f "$db" ] && { printf '%s' "$db"; return 0; }
  return 1
}

c_runner_present() {
  [ -f "$RUNNER" ] || { fail runner-present "$RUNNER missing"; return; }
  [ -x "$RUNNER" ] || { fail runner-present "scripts/gauntlet-gs026.sh not executable (bare-name run would exit 126)"; return; }
  bash -n "$RUNNER" 2>"$T/n.err" || { fail runner-present "bash -n: $(head -1 "$T/n.err")"; return; }
  pass runner-present
}

c_output_contract() {
  [ -f "$RUNNER" ] || { fail output-contract "runner absent"; return; }
  head -n 40 "$RUNNER" | grep -q 'OUTPUT CONTRACT:' ||
    { fail output-contract "no 'OUTPUT CONTRACT:' header in the first 40 lines"; return; }
  pass output-contract
}

c_forge_example_present() {
  [ -f "$FORGE" ] || { fail forge-example "bins/node/examples/gs026_forge_block.rs missing"; return; }
  pass forge-example
}

c_seed_parity() {
  local blk flat got emit dbf row db_runner db_ef db_inc db_assert
  blk="$(awk -v k="VALUES ('GS-026'" 'index($0, k) { on = 1 } on { print } on && /^ON CONFLICT/ { exit }' "$SEED")"
  [ -n "$blk" ] || { fail seed-parity "no VALUES ('GS-026' block in scripts/gauntlet-seed.sql"; return; }
  printf '%s\n' "$blk" | grep -qF "'scripts/gauntlet-gs026.sh'" ||
    { fail seed-parity "GS-026 runner column is not 'scripts/gauntlet-gs026.sh'"; return; }
  printf '%s\n' "$blk" | grep -E "json\('\[" | grep -qF '"INC-I-236"' ||
    { fail seed-parity "GS-026 incident_ids lacks INC-I-236"; return; }
  flat="$(printf '%s\n' "$blk" | tr '\n' ' ' | tr -s ' ')"
  printf '%s' "$flat" | grep -qE "'scripts/gauntlet-gs026\.sh', 'active', 0\)" ||
    { fail seed-parity "GS-026 tail is not runner 'scripts/gauntlet-gs026.sh', status 'active', expect_fail 0"; return; }
  got="$(printf '%s\n' "$blk" | grep -oE "'gs026-[a-z0-9,-]+'" | tr -d "'" | sorted)"
  [ "$got" = "$WANT" ] || { fail seed-parity "seed assertions {${got}} != {${WANT}}"; return; }
  [ -f "$RUNNER" ] || { fail seed-parity "runner absent, cannot compare the ids it emits"; return; }
  emit="$(grep -oE "(PASS|FAIL)[[:space:]]+gs026-[a-z0-9-]+" "$RUNNER" | grep -oE "gs026-[a-z0-9-]+" | sorted)"
  [ -n "$emit" ] || emit="$(grep -oE "gs026-[a-z0-9-]+" "$RUNNER" | sorted)"
  [ "$emit" = "$WANT" ] || { fail seed-parity "ids the runner emits {${emit}} != {${WANT}}"; return; }
  dbf="$(resolve_db)" || { fail seed-parity "memory.db not found via env, $ROOT/.omega or git-common-dir"; return; }
  command -v sqlite3 >/dev/null 2>&1 || { fail seed-parity "sqlite3 unavailable, cannot verify the live GS-026 row"; return; }
  row="$(sqlite3 "$dbf" "SELECT runner||'|'||expect_fail||'|'||incident_ids||'|'||assertions FROM gauntlet_scenarios WHERE scenario_id='GS-026';" 2>/dev/null)"
  [ -n "$row" ] || { fail seed-parity "memory.db has no GS-026 row (scenario not registered yet)"; return; }
  db_runner="$(printf '%s' "$row" | cut -d'|' -f1)"
  db_ef="$(printf '%s' "$row" | cut -d'|' -f2)"
  db_inc="$(printf '%s' "$row" | cut -d'|' -f3)"
  db_assert="$(printf '%s' "$row" | cut -d'|' -f4-)"
  [ "$db_runner" = scripts/gauntlet-gs026.sh ] || { fail seed-parity "DB runner '$db_runner' != scripts/gauntlet-gs026.sh"; return; }
  [ "$db_ef" = 0 ] || { fail seed-parity "DB expect_fail '$db_ef' != 0"; return; }
  printf '%s' "$db_inc" | grep -qF 'INC-I-236' || { fail seed-parity "DB incident_ids lacks INC-I-236: $db_inc"; return; }
  [ "$(printf '%s' "$db_assert" | sorted)" = "$WANT" ] ||
    { fail seed-parity "DB assertions {$(printf '%s' "$db_assert" | sorted)} != {${WANT}}"; return; }
  pass seed-parity "4 ids: seed + runner-emitted + db agree, runner/incident/expect_fail pinned"
}

# Runs gauntlet.sh assert() alone in a fresh bash: prints "<rc>\t<SKIP_REASONS>\t<FAIL_REASONS>".
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
  rc="$(call_assert not-a-gs026-token | cut -f1)"
  [ "$rc" = 1 ] || { fail gauntlet-skip-arm "control: unknown token -> rc ${rc:-none}, want 1 (FAIL)"; return; }
  for id in $IDS; do
    out="$(call_assert "$id")"
    rc="$(printf '%s' "$out" | cut -f1)"
    [ "$rc" = 2 ] || { fail gauntlet-skip-arm "assert $id -> rc ${rc:-none}, want 2 (SKIP):$(printf '%s' "$out" | cut -f3)"; return; }
    printf '%s' "$out" | cut -f2 | grep -qF scripts/gauntlet-gs026.sh ||
      { fail gauntlet-skip-arm "SKIP_REASONS for $id does not name scripts/gauntlet-gs026.sh"; return; }
    [ -z "$(printf '%s' "$out" | cut -f3)" ] ||
      { fail gauntlet-skip-arm "assert $id wrote FAIL_REASONS:$(printf '%s' "$out" | cut -f3)"; return; }
  done
  pass gauntlet-skip-arm "4/4 rc 2 naming the runner, control unknown token rc 1"
}

# Prints one line per harness-unsafe construct in the non-comment lines of $1.
unsafe() {
  local code
  code="$(grep -vE '^[[:space:]]*#' "$1")"
  printf '%s\n' "$code" | grep -E '(^|[^[:alnum:]_])(pkill|killall|launchctl|systemctl|ssh)([^[:alnum:]_]|$)|testnet\.sh' | sed 's/^/forbidden: /'
  printf '%s\n' "$code" | grep -E '(^|[;&|({[:space:]])ln[[:space:]]' | sed 's/^/link: /'
  printf '%s\n' "$code" | grep -E '(^|[^[:alnum:]_-])kill[[:space:]]' | sed -E 's/kill( -[A-Za-z0-9]+)? "\$PID"//g' |
    grep -E '(^|[^[:alnum:]_-])kill[[:space:]]' | sed 's/^/kill: /'
  printf '%s\n' "$code" | grep -F -- 'rm -rf' |
    grep -vE 'gs026\.[^)]*\)[[:space:]]*rm -rf "\$S"' |
    grep -vE '^clone\(\).*rm -rf "\$2"' | sed 's/^/rm: /'
}

c_harness_safety() {
  local v n
  [ -f "$RUNNER" ] || { fail harness-safety "runner absent"; return; }
  v="$(unsafe "$RUNNER")"
  [ -z "$v" ] || { fail harness-safety "$(printf '%s' "$v" | head -3 | tr '\n' ' ')"; return; }
  grep -qE 'mktemp[[:space:]]+-d[[:space:]]+.*gs026\.X+' "$RUNNER" ||
    { fail harness-safety "scratch is not a mktemp -d gs026.XXXXXX dir"; return; }
  grep -qE 'kill[[:space:]]+(-[A-Za-z0-9]+[[:space:]]+)?"\$PID"' "$RUNNER" ||
    { fail harness-safety "runner never signals only its own \$PID"; return; }
  cp "$RUNNER" "$T/planted.sh"
  printf '%s\n' 'pkill -f doli-node' 'kill -TERM "$SEED_PID"' 'rm -rf "$GS026_TESTNET_DIR"' 'ln -s "$CHECKPOINT" "$S/data"' >>"$T/planted.sh"
  n="$(unsafe "$T/planted.sh" | wc -l | tr -d ' ')"
  [ "$n" = 4 ] || { fail harness-safety "checker flagged $n of 4 planted violations"; return; }
  pass harness-safety "runner clean, 4/4 planted violations flagged"
}

c_docs() {
  [ -f "$README" ] || { fail docs "scripts/README.md missing"; return; }
  grep -F 'gauntlet-gs026.sh' "$README" | grep -q 'GS-026' ||
    { fail docs "scripts/README.md has no GS-026 entry line naming gauntlet-gs026.sh"; return; }
  pass docs
}

c_runner_present
c_output_contract
c_forge_example_present
c_seed_parity
c_skip_arm
c_harness_safety
c_docs
echo "$NP passed, $NF failed"
[ "$NF" = 0 ]
