#!/usr/bin/env bash
# test_gauntlet_gs023.sh — offline companion for GS-023 (INC-I-249 M4). Never starts a node; < 15 s.
# covers: scripts/gauntlet-gs023.sh scripts/gauntlet.sh scripts/gauntlet-seed.sql docs/archiver.md docs/troubleshooting.md specs/architecture.md
# REQ-ARCH-010 — Decision: a FAIL means the default gauntlet run cannot carry GS-023 cleanly (seed or memory.db row drift, no gauntlet.sh SKIP arm so the unknown-token arm FAILs the scenario, a missing precondition reported as exit 0, or an unscoped kill/rm/link in the runner); the live RED/GREEN proof is the runner itself, so this file guards only registration, SKIP routing and harness safety.
# REQ-ARCH-011 — Decision: a FAIL means docs/archiver.md, docs/troubleshooting.md or specs/architecture.md drifted back to the pre-INC-I-249 story (existence-only archive check, no verified_height, no watchdog cold-start grace) that an operator reads during a restart loop, so grep checks pin the post-fix wording.
# OUTPUT CONTRACT:
#   O1 stdout   `PASS <case>` | `FAIL <case>: <why>` per case; last line `<N> passed, <M> failed`
#   O2 exit     0 all PASS · 1 any FAIL
#   O3 effects  only mktemp dirs (deleted on exit); the runner only ever sees a stub binary
#   subject outputs read: runner exit code, `SKIP gs023:` line, scratch dirs, stub-binary calls;
#   assert() rc + SKIP_REASONS + FAIL_REASONS; seed block + memory.db row columns; doc text.
# INPUT PARTITIONS:
#   runner-present     present · executable · parses
#   seed-parity        GS-023 block: runner, INC-I-249, 7-id set; ids emitted by the runner; DB row
#   gauntlet-skip-arm  each gs023 id -> rc 2 + SKIP_REASONS names the runner; control: unknown -> rc 1
#   skip-exit-2        (a) binary missing · (b) stub binary + empty testnet dir -> exit 2, stub unused
#   build-scope        cargo/rustc of another repo (u) · under ROOT/.claude/worktrees (w) -> no build SKIP ·
#                      cargo/rustc naming this checkout ROOT/ (r) -> `SKIP gs023: a build is running (cargo/rustc)` naming ROOT
#   harness-safety     runner clean · planted pkill / foreign kill / testnet rm / ln -> 4 of 4 flagged
#   docs-synced        post-fix wording present, pre-fix wording gone
#   manifest-strip     key present -> removed, rest kept · key absent -> unchanged ·
#                      unparsable -> rc != 0 · unwritable with key -> rc != 0 · called after the clone

# shellcheck disable=SC2016,SC2020  # literal $VARS are grep patterns, tr maps two chars
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNNER="$ROOT/scripts/gauntlet-gs023.sh"
GAUNTLET="$ROOT/scripts/gauntlet.sh"
SEED="$ROOT/scripts/gauntlet-seed.sql"
DB="$ROOT/.omega/memory.db"
IDS="gs023-rpc-30s gs023-rpc-10s gs023-second-call-1s gs023-no-failed-probe gs023-no-respawn gs023-reaches-tip gs023-clean-shutdown"
T="$(mktemp -d "${TMPDIR:-/tmp}/t-gs023.XXXXXX")" || exit 1
trap 'rm -rf "$T"' EXIT
NP=0
NF=0
pass() { echo "PASS $1${2:+ ($2)}"; NP=$((NP + 1)); }
fail() { echo "FAIL $1: $2"; NF=$((NF + 1)); }
sorted() { tr ' ,' '\n\n' | sed '/^$/d' | sort -u | tr '\n' ' ' | sed 's/ $//'; }
WANT="$(echo "$IDS" | sorted)"

c_runner_present() {
  [ -f "$RUNNER" ] || { fail runner-present "$RUNNER missing"; return; }
  [ -x "$RUNNER" ] || { fail runner-present "$RUNNER not executable"; return; }
  bash -n "$RUNNER" 2>"$T/n.err" || { fail runner-present "bash -n: $(head -1 "$T/n.err")"; return; }
  pass runner-present
}

c_seed_parity() {
  local blk got id row db=absent
  blk="$(awk -v k="VALUES ('GS-023'" 'index($0, k) { on = 1 } on { print } on && /^ON CONFLICT/ { exit }' "$SEED")"
  [ -n "$blk" ] || { fail seed-parity "no VALUES ('GS-023' block in scripts/gauntlet-seed.sql"; return; }
  printf '%s\n' "$blk" | grep -qF "'scripts/gauntlet-gs023.sh'" ||
    { fail seed-parity "GS-023 runner column is not 'scripts/gauntlet-gs023.sh'"; return; }
  printf '%s\n' "$blk" | grep -E "json\('\[" | grep -qF '"INC-I-249"' ||
    { fail seed-parity "GS-023 incident_ids lacks INC-I-249"; return; }
  got="$(printf '%s\n' "$blk" | grep -oE "^[[:space:]]*'gs023-[a-z0-9,-]+'" | tr -d " '" | sorted)"
  [ "$got" = "$WANT" ] || { fail seed-parity "assertions {${got}} != {${WANT}}"; return; }
  for id in $IDS; do
    grep -qE "(^|[^a-z0-9-])$id([^a-z0-9-]|$)" "$RUNNER" ||
      { fail seed-parity "$id is never emitted by the runner"; return; }
  done
  if [ -f "$DB" ] && command -v sqlite3 >/dev/null 2>&1; then
    row="$(sqlite3 "$DB" "SELECT runner || '|' || assertions FROM gauntlet_scenarios WHERE scenario_id = 'GS-023';" 2>/dev/null)"
    if [ -n "$row" ]; then
      if [ "${row%%|*}" != scripts/gauntlet-gs023.sh ] || [ "$(echo "${row#*|}" | sorted)" != "$WANT" ]; then
        fail seed-parity "memory.db GS-023 row drifts from the seed: $row"
        return
      fi
      db=checked
    fi
  fi
  pass seed-parity "7 ids, db=$db"
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
  if ! grep -q '^assert()' "$T/assert.sh" || [ "$(call_assert not-a-token | cut -f1)" != 1 ]; then
    for id in $IDS; do
      grep -A2 -E "(^|[|[:space:]])$id([|)])" "$GAUNTLET" | grep -q 'ok=2' ||
        { fail gauntlet-skip-arm "static fallback: no arm for $id that sets ok=2"; return; }
    done
    pass gauntlet-skip-arm "static fallback: assert() extraction failed"
    return
  fi
  for id in $IDS; do
    out="$(call_assert "$id")"
    rc="$(printf '%s' "$out" | cut -f1)"
    [ "$rc" = 2 ] || { fail gauntlet-skip-arm "assert $id -> rc ${rc:-none}, want 2 (SKIP):$(printf '%s' "$out" | cut -f3)"; return; }
    printf '%s' "$out" | cut -f2 | grep -qF gauntlet-gs023.sh ||
      { fail gauntlet-skip-arm "SKIP_REASONS for $id does not name scripts/gauntlet-gs023.sh"; return; }
    [ -z "$(printf '%s' "$out" | cut -f3)" ] ||
      { fail gauntlet-skip-arm "assert $id wrote FAIL_REASONS:$(printf '%s' "$out" | cut -f3)"; return; }
  done
  pass gauntlet-skip-arm "functional: 7/7 rc 2, control unknown token rc 1"
}

mk_stubs() {
  mkdir -p "$T/bin" "$T/empty" "$T/tmp"
  cat >"$T/bin/curl" <<'EOF'
#!/bin/sh
printf '%s' '{"jsonrpc":"2.0","result":{"bestHeight":1},"id":1}'
EOF
  printf '#!/bin/sh\nexit 1\n' >"$T/bin/pgrep"
  cp "$T/bin/pgrep" "$T/bin/lsof"
  printf '#!/bin/sh\necho "$*" >>"%s/stub-ran"\n' "$T" >"$T/stubnode"
  chmod +x "$T/bin/curl" "$T/bin/pgrep" "$T/bin/lsof" "$T/stubnode"
}

# Runs the runner (stubbed curl/pgrep/lsof, scratch TMPDIR) capped at 20 s; returns its exit code.
guarded() {
  local log="$1" p i=0
  shift
  env -u GS023_CHECKPOINT -u GS023_ARCHIVE_SRC PATH="$T/bin:$PATH" TMPDIR="$T/tmp" \
    GS023_PROBE_TIMEOUT=2 "$@" bash "$RUNNER" >"$log" 2>&1 &
  p=$!
  while kill -0 "$p" 2>/dev/null && [ "$i" -lt 100 ]; do
    sleep 0.2
    i=$((i + 1))
  done
  if kill -0 "$p" 2>/dev/null; then
    kill "$p"
    wait "$p" 2>/dev/null
    return 124
  fi
  wait "$p"
}

check_skip() {
  if [ "$2" != 2 ]; then fail skip-exit-2 "($1) exit $2, want 2: $(tail -1 "$3")"; return 1; fi
  grep -q "^SKIP gs023: .*$4" "$3" || { fail skip-exit-2 "($1) no 'SKIP gs023: ...$4' line: $(tail -1 "$3")"; return 1; }
  if grep -qE '^(PASS|FAIL) gs023-|^METRIC' "$3"; then fail skip-exit-2 "($1) printed a verdict on a SKIP path"; return 1; fi
}

c_skip_exit_2() {
  local rc
  mk_stubs
  guarded "$T/a.log" GS023_BINARY=/nonexistent/doli-node
  rc=$?
  check_skip a "$rc" "$T/a.log" binary || return
  guarded "$T/b.log" GS023_BINARY="$T/stubnode" GS023_TESTNET_DIR="$T/empty"
  rc=$?
  check_skip b "$rc" "$T/b.log" checkpoint || return
  [ ! -e "$T/stub-ran" ] || { fail skip-exit-2 "(b) the stub binary was run: $(head -1 "$T/stub-ran")"; return; }
  [ -z "$(ls -A "$T/tmp")" ] || { fail skip-exit-2 "(b) scratch created before preconditions: $(ls "$T/tmp")"; return; }
  pass skip-exit-2 "a+b exit 2, stub never run"
}

# Process-listing stubs driven by $T_PROCS (one args line per process); `ps -p` calls reach /bin/ps.
mk_proc_stubs() {
  mk_stubs
  cat >"$T/bin/pgrep" <<'EOF'
#!/bin/sh
for a in "$@"; do n="$a"; done
[ -f "${T_PROCS:-}" ] || exit 1
awk -v n="$n" '{ b = $1; sub(/.*\//, "", b) } b == n { f = 1 } END { exit !f }' "$T_PROCS"
EOF
  cat >"$T/bin/ps" <<'EOF'
#!/bin/sh
case " $* " in *" -p "*) exec /bin/ps "$@" ;; esac
[ -f "${T_PROCS:-}" ] && cat "$T_PROCS"
exit 0
EOF
  chmod +x "$T/bin/pgrep" "$T/bin/ps"
}

# REQ-ARCH-010 — Decision: a FAIL means GS-023 SKIPs (rc 2, counted as FAIL by gauntlet-run.sh) whenever ANY cargo/rustc runs on the machine (measured: another project's `cargo test`), or runs while this checkout is being built.
c_build_scope() {
  local rc c
  mk_proc_stubs
  for c in u w; do
    if [ "$c" = u ]; then
      printf '%s\n' '/opt/tools/cargo test --manifest-path /opt/other-project/Cargo.toml' \
        'rustc --crate-name other --edition=2021 /opt/other-project/src/lib.rs' >"$T/procs.$c"
    else
      printf '%s\n' "rustc --crate-name wt --edition=2021 $ROOT/.claude/worktrees/other/src/lib.rs" >"$T/procs.$c"
    fi
    guarded "$T/bs-$c.log" T_PROCS="$T/procs.$c" GS023_BINARY=/nonexistent/doli-node
    rc=$?
    if grep -q '^SKIP gs023: a build is running' "$T/bs-$c.log"; then
      fail build-scope "($c) a cargo/rustc outside this checkout caused the build SKIP: $(grep -m 1 '^SKIP' "$T/bs-$c.log")"; return
    fi
    { [ "$rc" = 2 ] && grep -q '^SKIP gs023: binary ' "$T/bs-$c.log"; } ||
      { fail build-scope "($c) exit $rc, want 2 at the next precondition (binary): $(tail -1 "$T/bs-$c.log")"; return; }
  done
  printf '%s\n' "/opt/tools/cargo build --release --manifest-path $ROOT/Cargo.toml" >"$T/procs.r"
  guarded "$T/bs-r.log" T_PROCS="$T/procs.r" GS023_BINARY=/nonexistent/doli-node
  rc=$?
  { [ "$rc" = 2 ] && grep -q '^SKIP gs023: a build is running (cargo/rustc)' "$T/bs-r.log"; } ||
    { fail build-scope "(r) exit $rc, want 2 + 'SKIP gs023: a build is running (cargo/rustc)': $(tail -1 "$T/bs-r.log")"; return; }
  grep '^SKIP gs023: a build is running' "$T/bs-r.log" | grep -qF -- "$ROOT" ||
    { fail build-scope "(r) the build SKIP reason does not name this checkout $ROOT: $(grep -m 1 '^SKIP' "$T/bs-r.log")"; return; }
  pass build-scope "other repo + worktree: no build SKIP; this checkout: SKIP naming ROOT"
}

# Prints one line per unsafe construct in the non-comment lines of $1.
unsafe() {
  local code
  code="$(grep -vE '^[[:space:]]*#' "$1")"
  printf '%s\n' "$code" | grep -E '(^|[^[:alnum:]_])(pkill|killall|launchctl|systemctl)([^[:alnum:]_]|$)|testnet\.sh' | sed 's/^/forbidden: /'
  printf '%s\n' "$code" | grep -E '(^|[;&|({[:space:]])ln[[:space:]]' | sed 's/^/link: /'
  printf '%s\n' "$code" | grep -E '(^|[^[:alnum:]_-])kill[[:space:]]' | sed -E 's/kill -[A-Z0-9]+ "\$PID"//g' |
    grep -E '(^|[^[:alnum:]_-])kill[[:space:]]' | sed 's/^/kill: /'
  printf '%s\n' "$code" | grep -F 'rm -rf' |
    grep -vE '^[[:space:]]*case "\$S" in "\$TMPROOT"/gs023\.\?\*\) rm -rf "\$S" ;; esac$' |
    grep -vE '^clone\(\) \{ .*rm -rf "\$2"' | sed 's/^/rm: /'
  printf '%s\n' "$code" | grep -E '^[[:space:]]*clone "' |
    grep -vE '^[[:space:]]*clone "[^"]+" "\$S/[a-z]+"' | sed 's/^/clone: /'
}

c_harness_safety() {
  local v n
  v="$(unsafe "$RUNNER")"
  [ -z "$v" ] || { fail harness-safety "$(printf '%s' "$v" | head -3 | tr '\n' ' ')"; return; }
  grep -qF 'mktemp -d "$TMPROOT/gs023.XXXXXX"' "$RUNNER" || { fail harness-safety "scratch is not mktemp \$TMPROOT/gs023.XXXXXX"; return; }
  grep -qE 'kill -TERM "\$PID"' "$RUNNER" || { fail harness-safety "runner never SIGTERMs its own \$PID"; return; }
  cp "$RUNNER" "$T/planted.sh"
  printf '%s\n' 'pkill -f doli-node' 'kill -TERM "$SEED_PID"' 'rm -rf "$GS023_TESTNET_DIR"' 'ln -s "$GS023_CHECKPOINT" "$S/data"' >>"$T/planted.sh"
  n="$(unsafe "$T/planted.sh" | wc -l | tr -d ' ')"
  [ "$n" = 4 ] || { fail harness-safety "checker flagged $n of 4 planted violations"; return; }
  pass harness-safety "runner clean, 4/4 planted violations flagged"
}

c_docs() {
  local why=""
  grep -qF verified_height "$ROOT/docs/archiver.md" || why="${why}docs/archiver.md lacks verified_height; "
  grep -qF 'file existence check, not manifest-based' "$ROOT/docs/archiver.md" &&
    why="${why}docs/archiver.md still says 'file existence check, not manifest-based'; "
  grep -qF DOLI_WATCHDOG_COLD_START_GRACE "$ROOT/docs/troubleshooting.md" ||
    why="${why}docs/troubleshooting.md lacks DOLI_WATCHDOG_COLD_START_GRACE; "
  grep -qF 'Restarts in a Loop' "$ROOT/docs/troubleshooting.md" || why="${why}docs/troubleshooting.md lacks 'Restarts in a Loop'; "
  grep -qF verified_height "$ROOT/specs/architecture.md" || why="${why}specs/architecture.md lacks verified_height; "
  if [ -z "$why" ]; then pass docs-synced; else fail docs-synced "$why"; fi
}

# Calls the runner's gs023_strip_verified_height (extracted to $T/strip.sh) in a fresh bash; prints its rc.
call_strip() {
  bash -c '. "$1" || exit 99; gs023_strip_verified_height "$2"' _ "$T/strip.sh" "$1" >"$T/strip.out" 2>&1
  echo $?
}

# Exit 0 when JSON file $1 equals JSON text $2 (python3, exact ints).
json_eq() { python3 -I -c 'import json, sys; sys.exit(json.load(open(sys.argv[1])) != json.loads(sys.argv[2]))' "$1" "$2" 2>/dev/null; }

# REQ-ARCH-010 — Decision: a FAIL means a cloned seed manifest keeps verified_height, so GS-023 never sees a background full check (REVIEW-P1-001).
c_manifest_strip() {
  local rc cl cs m="$T/manifest.json" ro="$T/ro"
  local rest='{"latest_height": 9007199254740993, "latest_hash": "ab12", "genesis_hash": "cd34", "extra": [1, {"k": null}]}'
  local full='{"latest_height": 9007199254740993, "latest_hash": "ab12", "genesis_hash": "cd34", "verified_height": 9007199254740993, "extra": [1, {"k": null}]}'
  awk '/^gs023_strip_verified_height\(\)[[:space:]]*\{/ { on = 1 } on { print } on && /^}/ { exit }' "$RUNNER" >"$T/strip.sh"
  grep -q '^gs023_strip_verified_height()' "$T/strip.sh" ||
    { fail manifest-strip "gs023_strip_verified_height() is not defined in the runner"; return; }
  printf '%s' "$full" >"$m"
  rc="$(call_strip "$m")"
  [ "$rc" = 0 ] || { fail manifest-strip "(with key) rc $rc: $(head -1 "$T/strip.out")"; return; }
  json_eq "$m" "$rest" || { fail manifest-strip "(with key) want verified_height removed and the rest kept, got: $(head -c 200 "$m")"; return; }
  printf '%s' "$rest" >"$m"
  rc="$(call_strip "$m")"
  [ "$rc" = 0 ] || { fail manifest-strip "(without key) rc $rc: $(head -1 "$T/strip.out")"; return; }
  json_eq "$m" "$rest" || { fail manifest-strip "(without key) keys changed: $(head -c 200 "$m")"; return; }
  printf '%s' '{not json' >"$m"
  rc="$(call_strip "$m")"
  [ "$rc" != 0 ] || { fail manifest-strip "(unparsable) rc 0, want non-zero"; return; }
  [ -s "$T/strip.out" ] || { fail manifest-strip "(unparsable) failed without a message"; return; }
  mkdir -p "$ro"
  printf '%s' "$full" >"$ro/manifest.json"
  chmod 0444 "$ro/manifest.json"
  chmod 0555 "$ro"
  if touch "$ro/.probe" 2>/dev/null; then
    rm -f "$ro/.probe"
  else
    rc="$(call_strip "$ro/manifest.json")"
    chmod 0755 "$ro"
    chmod 0644 "$ro/manifest.json"
    [ "$rc" != 0 ] || { fail manifest-strip "(unwritable, key kept) rc 0, want non-zero"; return; }
  fi
  chmod 0755 "$ro" 2>/dev/null
  cl="$(grep -nF 'clone "$GS023_ARCHIVE_SRC" "$S/archive"' "$RUNNER" | head -1 | cut -d: -f1)"
  cs="$(grep -nE '^[^#]*gs023_strip_verified_height "\$S/archive/manifest\.json"' "$RUNNER" | grep -E '\|\||:[[:space:]]*if !' | head -1 | cut -d: -f1)"
  { [ -n "$cl" ] && [ -n "$cs" ] && [ "$cs" -gt "$cl" ]; } ||
    { fail manifest-strip "runner does not call gs023_strip_verified_height \"\$S/archive/manifest.json\" with a failure arm after the archive clone"; return; }
  pass manifest-strip "key stripped, rest kept, absent key unchanged, unparsable/unwritable rc != 0"
}

c_runner_present
c_seed_parity
c_skip_arm
c_skip_exit_2
c_build_scope
c_harness_safety
c_docs
c_manifest_strip
echo "$NP passed, $NF failed"
[ "$NF" = 0 ]
