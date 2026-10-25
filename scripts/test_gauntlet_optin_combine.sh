#!/usr/bin/env bash
# test_gauntlet_optin_combine.sh — gauntlet.sh opt-in flags combine in one call (static checks only; runs nothing).
# covers: scripts/gauntlet.sh scripts/README.md scripts/gauntlet-gs009.sh scripts/gauntlet-gs010.sh scripts/gauntlet-gs014.sh scripts/gauntlet-gs016.sh scripts/gauntlet-gs019.sh scripts/gauntlet-gs022.sh
# REQ-B3-006 — Decision: a FAIL of an own-block or no-elif check means two armed opt-in flags in one call still dispatch only the first scenario.
# REQ-B3-006 — Decision: a FAIL of a banner check means the run banner hides an armed mode, so the operator misreads what the run injected.
# REQ-B3-006 — Decision: a FAIL of a confirm check means an opt-in block lost its confirm-var guard, so a combined call injects without approval.
# OUTPUT CONTRACT: outputs = one `PASS|FAIL <check>` line per check, a `SUMMARY fails=<n>` line, exit code (0 iff fails=0).
#   Reads the dispatch region of gauntlet.sh (between the `# ── perturbation dispatch` and `# ── observation window`
#   section comments), the lines before the banner (`mode=`), chaos_guard, the injector libs and README.md.
# INPUT PARTITIONS: per flag (GS016 GS014 GS010 GS009 GS022 GS019 CHAOS): own `if` block | `elif` rung (FAIL) ·
#   block runs its own injector only | runs another flag's injector (FAIL) · banner reads the flag | not (FAIL) ·
#   confirm-var present in its guard | absent (FAIL).
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GSH="${GAUNTLET_SH:-$ROOT/scripts/gauntlet.sh}"
README="${GAUNTLET_README:-$ROOT/scripts/README.md}"
FAILS=0
FLAGS="GS016 GS014 GS010 GS009 GS022 GS019 CHAOS"

res() {
  if [ "$1" = 0 ]; then echo "PASS $2"; else echo "FAIL $2"; FAILS=$((FAILS + 1)); fi
}
injector() { [ "$1" = CHAOS ] && echo chaos_guard || echo "$(echo "$1" | tr 'A-Z' 'a-z')_inject"; }
confirm_var() { echo "GAUNTLET_${1}_CONFIRM"; }
# A shell reference to the flag variable: $V, ${V}, ${V:-0}, "$V".
ref_re() { echo "\\\$\\{?$1([}:\"[:space:]]|$)"; }

region() {
  awk '/^# ── perturbation dispatch/ {on=1; next} /^# ── observation window/ {on=0} on' "$GSH"
}

# The if-block a flag opens in the dispatch region, up to its matching fi.
block_of() {
  region | RE="$(ref_re "$1")" awk '
    BEGIN { re = ENVIRON["RE"] }
    { l = $0; sub(/^[ \t]+/, "", l) }
    d == 0 && l ~ /^if[ \t]/ && l ~ re {
      d = 1; print
      if (l ~ /;[ \t]*fi[ \t]*$/) exit
      next
    }
    d > 0 {
      print
      if (l ~ /^if[ \t]/) d++
      if (l ~ /^fi([ \t;]|$)/ || l ~ /;[ \t]*fi[ \t]*$/) d--
      if (d == 0) exit
    }'
}

REG="$(region)"
[ -n "$REG" ] && printf '%s\n' "$REG" | grep -q "gs009_inject"; res $? "dispatch region found in gauntlet.sh"

ALL_RE="\\\$\\{?(GS016|GS014|GS010|GS009|GS022|GS019|CHAOS)([}:\"[:space:]]|$)"
bad="$(printf '%s\n' "$REG" | grep -nE "^[[:space:]]*elif[[:space:]]" | grep -E "$ALL_RE")"
[ -z "$bad" ]; res $? "no elif chains an opt-in flag${bad:+ (offending: $(echo "$bad" | tr '\n' ' '))}"

BANNER_AT="$(grep -nE 'mode=\$\{?MODE' "$GSH" | head -1 | cut -d: -f1)"
[ -n "$BANNER_AT" ]; res $? "run banner (mode=\$MODE) found"
# Banner logic: code after the flag-parse loop up to the banner, comments stripped.
PRE="$(awk -v end="${BANNER_AT:-0}" '/^for a in "\$@"/ {p=1} p && /^done/ {s=1; next} s && NR<=end' "$GSH" \
  | grep -vE '^[[:space:]]*#' | sed -E 's/[[:space:]]+#[^"]*$//')"

for f in $FLAGS; do
  inj="$(injector "$f")"
  printf '%s\n' "$REG" | grep -E "^[[:space:]]*if[[:space:]]" | grep -qE "$(ref_re "$f")"
  res $? "$f has its own if block in the dispatch"
  blk="$(block_of "$f")"
  others=""
  for g in $FLAGS; do
    [ "$g" = "$f" ] && continue
    printf '%s\n' "$blk" | grep -qw "$(injector "$g")" && others="$others $(injector "$g")"
  done
  printf '%s\n' "$blk" | grep -qw "$inj" && [ -z "$others" ]
  res $? "$f block runs $inj and no other opt-in injector${others:+ (also:$others)}"
  printf '%s\n' "$PRE" | grep -qw "$f"; res $? "banner logic reads $f"
  if [ "$f" = CHAOS ]; then
    awk '/^chaos_guard\(\)/ {on=1} on {print} on && /^}/ {exit}' "$GSH" | grep -q "$(confirm_var CHAOS)"
  else
    lib="$ROOT/scripts/gauntlet-$(echo "$f" | tr 'A-Z' 'a-z').sh"
    [ -f "$lib" ] && grep -q "$(confirm_var "$f")" "$lib"
  fi
  res $? "$f guard still checks $(confirm_var "$f")"
done

grep -iE "opt-in" "$README" | grep -qiE "combine"; res $? "README says opt-in flags combine in one call"

echo "SUMMARY fails=$FAILS"
[ "$FAILS" = 0 ]
