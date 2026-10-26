#!/usr/bin/env bash
# test_gauntlet_seed_optin.sh — registry honesty of scripts/gauntlet-seed.sql and the gauntlet.sh token arms.
# covers: scripts/gauntlet-seed.sql scripts/gauntlet.sh
# REQ-LDP-050 — Decision: a FAIL means an opt-in row (GS-009/010/014/016/019/022 or --chaos) is registered so a run counts it without its injection armed and without a manual-only mark.
# REQ-LDP-051 — Decision: a FAIL means a row asks a runner to FAIL (expect_fail 1), so a broken scenario reads as a pass.
# REQ-LDP-050 — Decision: a FAIL of the manual-evaluation check means gauntlet.sh reads only active rows, so a manual row's own --gsNNN run injects and judges nothing.
# REQ-LDP-060 — Decision: a FAIL means GS-028 or GS-029 is missing, not active, has another runner, its assertion ids differ from its runner's output contract, or an id has no standalone SKIP arm in gauntlet.sh (the shared run would fail on an unknown token).
# REQ-LDP-062 — Decision: a FAIL means GS-028 or GS-029 does not link INC-I-236, INC-I-255, INC-I-257 and INC-I-182.
# REQ-LDP-052 — Decision: a FAIL means an active registered runner carries --gs010/--gs014/--gs016/--chaos, so an unattended gauntlet run writes a chain or wipes data without operator approval.
# REQ-LDP-053 — Decision: a FAIL means a registered runner script calls pkill, killall or ssh, so a gauntlet run can kill fleet nodes or reach a remote host.
# REQ-LDP-054 — Decision: a FAIL means the README lacks a GS-028/GS-029 bullet, one of their env knobs, an armed opt-in command or a manual-only mark, so an operator runs a scenario the wrong way.
# OUTPUT CONTRACT: outputs = one `PASS|FAIL|SKIP <check>` line per check, a `SUMMARY fails=<n>` line, exit code.
#   The seed is applied to a scratch sqlite DB; checks read its gauntlet_scenarios rows (status, runner,
#   description, assertions, incident_ids, expect_fail), the seed text, the runner headers and gauntlet.sh.
#   Memory agreement compares the same columns with .omega/memory.db, SKIP when that DB is absent.
# INPUT PARTITIONS: armed opt-in row (runner has flag + confirm var) · manual row (status manual, runner NULL,
#   MANUAL-ONLY with the exact command) · unarmed active row (FAIL) · missing row (FAIL) · expect_fail 1 (FAIL) ·
#   assertion ids equal / different from the runner header · token with / without a case arm.
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SEED="${SEED_SQL:-$ROOT/scripts/gauntlet-seed.sql}"
GSH="${GAUNTLET_SH:-$ROOT/scripts/gauntlet.sh}"
MEM="${MEMORY_DB:-$ROOT/.omega/memory.db}"
FAILS=0
TMP="$(mktemp -d "${TMPDIR:-/tmp}/seedoptin.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
DB="$TMP/scratch.db"

res() {
  if [ "$1" = 0 ]; then echo "PASS $2"; else echo "FAIL $2"; FAILS=$((FAILS + 1)); fi
}
q() { sqlite3 -separator '|' "$1" "$2" 2>/dev/null; }

sqlite3 "$DB" "CREATE TABLE gauntlet_scenarios (id INTEGER PRIMARY KEY AUTOINCREMENT,
  scenario_id TEXT UNIQUE NOT NULL, name TEXT NOT NULL, description TEXT, incident_ids TEXT,
  assertions TEXT, scale_params TEXT, status TEXT DEFAULT 'active',
  created_at TEXT DEFAULT (datetime('now')), runner TEXT, expect_fail INTEGER DEFAULT 0);"
sqlite3 "$DB" < "$SEED" >/dev/null 2>"$TMP/err"
[ ! -s "$TMP/err" ]; res $? "seed applies cleanly"

for n in 009 010 014 016 019 022; do
  sid="GS-$n"; cmd="GAUNTLET_GS${n}_CONFIRM=1 bash scripts/gauntlet.sh --gs$n"
  row="$(q "$DB" "SELECT status, COALESCE(runner,''), COALESCE(description,'') FROM gauntlet_scenarios WHERE scenario_id='$sid';")"
  st="${row%%|*}"; rest="${row#*|}"; rn="${rest%%|*}"; ds="${rest#*|}"
  ok=1
  if [ "$st" = active ] && [ "$rn" = "$cmd" ]; then ok=0; fi
  if [ "$st" = manual ] && [ -z "$rn" ] && case "$ds" in *"MANUAL-ONLY:"*"$cmd"*) true ;; *) false ;; esac; then ok=0; fi
  res $ok "$sid armed runner or manual-only (status=${st:-missing} runner=${rn:-NULL})"
done

grep -qF "status='active' OR (status='manual' AND scenario_id IN (\$ARMED_MANUAL))" "$GSH"
res $? "gauntlet.sh evaluates a manual row when its own flag is set"
for sid in $(q "$DB" "SELECT scenario_id FROM gauntlet_scenarios WHERE status='manual' ORDER BY scenario_id;"); do
  n="${sid#GS-}"
  grep -qF "[ \"\$GS$n\" = 1 ] && ARMED_MANUAL=\"\$ARMED_MANUAL,'$sid'\"" "$GSH"
  res $? "gauntlet.sh arms manual $sid only under --gs$n"
done

[ "$(q "$DB" "SELECT COUNT(*) FROM gauntlet_scenarios WHERE COALESCE(expect_fail,0) <> 0;")" = 0 ]
res $? "no row has expect_fail 1"
[ "$(q "$DB" "SELECT COUNT(*) FROM gauntlet_scenarios WHERE COALESCE(runner,'') LIKE '%--chaos%';")" = 0 ]
res $? "no registered runner runs --chaos"
grep -qF -- "MANUAL-ONLY: --chaos" "$SEED" && grep -qF -- "GAUNTLET_CHAOS_CONFIRM=1 bash scripts/gauntlet.sh --chaos" "$SEED"
res $? "seed marks --chaos manual-only with its exact command"

check_standalone() {
  local sid="$1" runner="$2" incs="$3" want got row st rn inc
  want="$(grep -oE "^#   ${4}-[a-z0-9-]+" "$ROOT/$runner" | sed 's/^#   //' | paste -sd, -)"
  row="$(q "$DB" "SELECT status, COALESCE(runner,''), COALESCE(assertions,'') FROM gauntlet_scenarios WHERE scenario_id='$sid';")"
  st="${row%%|*}"; rn="$(printf '%s' "$row" | cut -d'|' -f2)"; got="$(printf '%s' "$row" | cut -d'|' -f3)"
  [ "$st" = active ] && [ "$rn" = "$runner" ]; res $? "$sid active with runner $runner (status=${st:-missing} runner=${rn:-NULL})"
  [ -n "$want" ] && [ "$got" = "$want" ]; res $? "$sid assertions equal the runner header ids ($(printf '%s' "$want" | tr ',' '\n' | grep -c .))"
  for inc in $incs; do
    [ "$(q "$DB" "SELECT COUNT(*) FROM gauntlet_scenarios, json_each(incident_ids) WHERE scenario_id='$sid' AND value='$inc';")" = 1 ]
    res $? "$sid links $inc"
  done
  q "$DB" "SELECT description FROM gauntlet_scenarios WHERE scenario_id='$sid';" | grep -q "STANDALONE ONLY.*INJECTS:.*ASSERTS"
  res $? "$sid description in the STANDALONE/INJECTS/ASSERTS form"
  for t in $(printf '%s' "$want" | tr ',' ' '); do
    grep -Eq "(^|[ |])$t(\||\))" "$GSH"; res $? "gauntlet.sh has a case arm for $t"
  done
  grep -qF "run $runner (not part of gauntlet.sh)" "$GSH"; res $? "gauntlet.sh standalone reason names $runner"
}
check_standalone GS-028 scripts/gauntlet-gs028.sh "INC-I-236 INC-I-255 INC-I-257 INC-I-182 INC-I-259" gs028
check_standalone GS-029 scripts/gauntlet-gs029.sh "INC-I-236 INC-I-255 INC-I-257 INC-I-182" gs029

[ "$(q "$DB" "SELECT COUNT(*) FROM gauntlet_scenarios WHERE status='active' AND (runner LIKE '%--gs010%' OR runner LIKE '%--gs014%' OR runner LIKE '%--gs016%' OR runner LIKE '%--chaos%');")" = 0 ]
res $? "no active runner writes a chain or wipes data"

for r in $( { echo scripts/gauntlet.sh; q "$DB" "SELECT runner FROM gauntlet_scenarios WHERE status='active' AND runner IS NOT NULL;" | grep -oE '[A-Za-z0-9_./-]+\.sh'; } | sort -u); do
  f="$ROOT/$r"; [ -f "$f" ] || f="$ROOT/scripts/$r"
  [ -f "$f" ] && ! grep -vE '^[[:space:]]*#' "$f" | grep -qE '(^|[^a-zA-Z_-])(pkill|killall|ssh)([^a-zA-Z_-]|$)'
  res $? "runner $r present, no pkill/killall/ssh"
done

README="$ROOT/scripts/README.md"
for n in 028 029; do
  grep -qF -- "- \`scripts/gauntlet-gs$n.sh\` — GS-$n" "$README"; res $? "README has the gauntlet-gs$n.sh bullet"
  for k in $(awk '/^# Env:/ { f = 1 } f && !/^# Env:/ && !/^#   / { exit } f' "$ROOT/scripts/gauntlet-gs$n.sh" | grep -oE "GS${n}_[A-Z0-9_]+" | sort -u); do
    grep -qF -- "\`$k\`" "$README"; res $? "README names GS-$n env knob $k"
  done
done
for n in 009 022; do
  grep -qF -- "GAUNTLET_GS${n}_CONFIRM=1 bash scripts/gauntlet.sh --gs$n" "$README"; res $? "README gives the armed GS-$n command"
done
for sid in $(q "$DB" "SELECT scenario_id FROM gauntlet_scenarios WHERE status='manual' ORDER BY scenario_id;") --chaos; do
  grep -F -- "$sid" "$README" | grep -qF "MANUAL-ONLY"; res $? "README marks $sid MANUAL-ONLY"
done
grep -qF "one pass" "$README"; res $? "README says the shared gauntlet.sh rows are one pass"

COLS="scenario_id, status, COALESCE(runner,''), COALESCE(expect_fail,0), assertions, json(incident_ids), description"
IDS="'GS-009','GS-010','GS-014','GS-016','GS-019','GS-022','GS-028','GS-029'"
if [ -f "$MEM" ]; then
  a="$(q "$DB" "SELECT $COLS FROM gauntlet_scenarios WHERE scenario_id IN ($IDS) ORDER BY scenario_id;")"
  b="$(q "$MEM" "SELECT $COLS FROM gauntlet_scenarios WHERE scenario_id IN ($IDS) ORDER BY scenario_id;")"
  [ -n "$a" ] && [ "$a" = "$b" ]; res $? "memory rows agree with the seed for $IDS"
else
  echo "SKIP memory rows agree (no $MEM)"
fi

echo "SUMMARY fails=$FAILS"
[ "$FAILS" = 0 ]
