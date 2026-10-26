#!/usr/bin/env bash
# shellcheck disable=SC2034,SC2016,SC2012,SC2086,SC2218
# test_gauntlet_chaos_restore.sh — offline test of the chaos B full-history restore in gauntlet.sh (scratch tree; no node, no RPC).
# covers: scripts/gauntlet.sh
# INC-I-262 — Decision: a FAIL means a wipe injection can leave the chaos target without block history, or the restore can touch the wrong files or node.
# OUTPUT CONTRACT: fn chaos_full_history_restore (globals RESTART_NODE CHAOS_DONOR LIVE TESTNET_HOME TESTNET_SH WORK)
#   O3: return — 0 iff the target block 1 equals the seed's and both nodes rejoined; else 1
#   O4: target data/ — own signed_slots.db + maintainer_state.bin byte-identical, all else removed, blocks/ + state_db/ = donor's
#   O4: donor data/, target data.bak.*, target node_key — unchanged on every path
#   O4: $WORK/chaos_full_history.txt — `donor=<n> restored=<node> block1_equal=<0|1> rejoin_s=<n>`
#   O5: CHAOS_RECOVERED (kept 1 | set 0, never reset to 1) · CHAOS_REASON (one reason appended per failure)
#   O6: testnet.sh calls — stop target, stop donor, start donor, start target (no call when refused)
# PATHS: P1 restored · P2 block-1 mismatch · P3 rejoin timeout · P4 donor/target refused · P5 donor not full-history · P6 a node did not stop · P7 rsync failed
# INPUT PARTITIONS: P1a clean entry · P1b entry CHAOS_RECOVERED=0 · P1c target has no maintainer_state.bin · P2a hash differs · P2b hash empty ·
#   P3a target timeout · P3b donor timeout · P4 donor seed|target|non-nN|nN not live, target seed · P5a donor hash differs · P5b seed hash empty ·
#   P5c donor has no state_db/ · W wait_stopped: matching process | other node's process (n10 vs n1)
# MATRIX: 6 outputs x 17 partitions; static: wipe calls the restore after its rejoin wait, guard checks the donor, header + record carry donor + reason.
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GSH="${GAUNTLET_SH:-$ROOT/scripts/gauntlet.sh}"
S="$(mktemp -d "${TMPDIR:-/tmp}/chaos-restore.XXXXXX")"
BGPID=""
trap '[ -n "$BGPID" ] && kill "$BGPID" 2>/dev/null; rm -rf "$S"' EXIT
FAILS=0
res() { if [ "$1" = 0 ]; then echo "PASS $2"; else echo "FAIL $2"; FAILS=$((FAILS + 1)); fi; }

SECTION="$S/section.sh"
awk '/^# ── chaos full-history restore/{f=1} f && /^# ── readiness/{exit} f' "$GSH" > "$SECTION"
grep '^port_of()' "$GSH" >> "$SECTION"
# shellcheck source=/dev/null
. "$SECTION"
type chaos_full_history_restore >/dev/null 2>&1; res $? "section defines chaos_full_history_restore"

C_R=""; C_Y=""; C_0=""; C_C=""; C_G=""
say() { printf '%s\n' "$*"; }
sleep() { :; }
wait_rejoin() { eval "echo \"\${REJOIN_$2:-0}\""; }
block1_hash() { eval "echo \"\${HASH_$1-H1}\""; }
mkdir -p "$S/bin" "$S/failbin"
printf '#!/usr/bin/env bash\necho "$1 $2" >> "$CALLS"\n' > "$S/bin/testnet.sh"
printf '#!/usr/bin/env bash\nexit 23\n' > "$S/failbin/rsync"
chmod +x "$S/bin/testnet.sh" "$S/failbin/rsync"
TESTNET_SH="$S/bin/testnet.sh"

setup() {
  C="$S/$1"; TESTNET_HOME="$C/testnet"; WORK="$C/work"; CALLS="$C/calls.txt"; export CALLS
  local d="$TESTNET_HOME/n1/data" t="$TESTNET_HOME/n5/data" f
  mkdir -p "$WORK" "$d/blocks" "$d/state_db/sub" "$t/blocks" "$t/state_db" "$TESTNET_HOME/n5/data.bak.1/blocks"
  : > "$CALLS"
  echo donor-sst > "$d/blocks/000001.sst"; echo donor-manifest > "$d/blocks/MANIFEST-000001"
  echo donor-cur > "$d/state_db/CURRENT"; echo donor-nested > "$d/state_db/sub/x.log"
  echo donor-slots > "$d/signed_slots.db"; echo donor-maint > "$d/maintainer_state.bin"; echo donor-peers > "$d/peers.cache"
  echo stale-sst > "$t/blocks/999.sst"; echo stale-cur > "$t/state_db/CURRENT"
  printf 'target-slots\001\002' > "$t/signed_slots.db"; printf 'target-maint\003' > "$t/maintainer_state.bin"
  for f in peers.cache producer_gset.bin producer.lock; do echo "t-$f" > "$t/$f"; done
  echo bak > "$TESTNET_HOME/n5/data.bak.1/blocks/old.sst"; echo key > "$TESTNET_HOME/n5/node_key"
  case "${2:-}" in nomaint) rm "$t/maintainer_state.bin" ;; nostate) rm -rf "$d/state_db" ;; esac
  cp -R "$TESTNET_HOME" "$C/orig"
  RESTART_NODE=n5; CHAOS_DONOR=n1; LIVE="seed n1 n2 n3 n5"; CHAOS_RECOVERED=1; CHAOS_REASON=""
  unset HASH_8500 HASH_8501 HASH_8505
  REJOIN_n1=4; REJOIN_n5=6
}
run() { chaos_full_history_restore > "$C/out.txt" 2>&1; RC=$?; }
calls_are() { [ "$(tr '\n' ',' < "$CALLS")" = "$1" ]; }
record_is() { [ "$(cat "$WORK/chaos_full_history.txt" 2>/dev/null)" = "$1" ]; }
listing_is() { [ "$(LC_ALL=C ls -A "$TESTNET_HOME/n5/data" | tr '\n' ' ')" = "$1" ]; }
reason_has() { case "$CHAOS_REASON" in *"$1"*) return 0 ;; *) return 1 ;; esac; }
restored() {
  local t="$TESTNET_HOME/n5/data" o="$C/orig/n5/data" f
  for f in signed_slots.db maintainer_state.bin; do
    if [ -f "$o/$f" ]; then cmp -s "$o/$f" "$t/$f" || return 1; else [ ! -e "$t/$f" ] || return 1; fi
  done
  diff -r "$TESTNET_HOME/n1/data/blocks" "$t/blocks" >/dev/null && diff -r "$TESTNET_HOME/n1/data/state_db" "$t/state_db" >/dev/null
}
side_safe() {
  diff -r "$C/orig/n1" "$TESTNET_HOME/n1" >/dev/null && diff -r "$C/orig/n5/data.bak.1" "$TESTNET_HOME/n5/data.bak.1" >/dev/null \
    && cmp -s "$C/orig/n5/node_key" "$TESTNET_HOME/n5/node_key"
}
untouched() { diff -r "$C/orig/n5" "$TESTNET_HOME/n5" >/dev/null && side_safe; }
OK_CALLS="stop n5,stop n1,start n1,start n5,"
FULL_LIST="blocks maintainer_state.bin signed_slots.db state_db "

setup p1a; run
[ "$RC" = 0 ]; res $? "P1a returns 0"
calls_are "$OK_CALLS"; res $? "P1a stops target then donor, starts donor then target"
restored && listing_is "$FULL_LIST"; res $? "P1a keeps own slots+maintainer, drops peers/gset/lock, blocks+state_db = donor"
side_safe; res $? "P1a donor data, data.bak and node_key unchanged"
record_is "donor=n1 restored=n5 block1_equal=1 rejoin_s=10"; res $? "P1a record file"
[ "$CHAOS_RECOVERED" = 1 ] && [ -z "$CHAOS_REASON" ]; res $? "P1a CHAOS_RECOVERED stays 1, no reason"
grep -q "donor=n1 restored=n5 block1_equal=1 rejoin_s=10" "$C/out.txt"; res $? "P1a say line carries the record"

setup p1b; CHAOS_RECOVERED=0; CHAOS_REASON="; snap rejoin timeout"; run
[ "$RC" = 0 ] && restored && [ "$CHAOS_RECOVERED" = 0 ] && [ "$CHAOS_REASON" = "; snap rejoin timeout" ]
res $? "P1b restore succeeds but never resets an earlier CHAOS_RECOVERED=0"

setup p1c nomaint; run
[ "$RC" = 0 ] && restored && listing_is "blocks signed_slots.db state_db " && calls_are "$OK_CALLS"
res $? "P1c target without maintainer_state.bin: restored, donor's copy not taken"

for p in "p2a OTHER" "p2b "; do
  set -- $p; setup "$1"; HASH_8505="${2:-}"; run
  [ "$RC" = 1 ] && [ "$CHAOS_RECOVERED" = 0 ] && reason_has "block 1"; res $? "$1 hash mismatch sets CHAOS_RECOVERED=0 with a block-1 reason"
  record_is "donor=n1 restored=n5 block1_equal=0 rejoin_s=10" && calls_are "$OK_CALLS" && restored && side_safe
  res $? "$1 record block1_equal=0, data still restored, order kept"
done

setup p3a; REJOIN_n5=-1; run
[ "$RC" = 1 ] && [ "$CHAOS_RECOVERED" = 0 ] && reason_has "rejoin" && record_is "donor=n1 restored=n5 block1_equal=1 rejoin_s=-1"
res $? "P3a target rejoin timeout: CHAOS_RECOVERED=0, rejoin_s=-1"
setup p3b; REJOIN_n1=-1; run
[ "$RC" = 1 ] && [ "$CHAOS_RECOVERED" = 0 ] && reason_has "rejoin" && record_is "donor=n1 restored=n5 block1_equal=1 rejoin_s=-1"
res $? "P3b donor rejoin timeout: CHAOS_RECOVERED=0, rejoin_s=-1"

refused_ok() {
  [ "$RC" = 1 ] && [ ! -s "$CALLS" ] && untouched && [ "$CHAOS_RECOVERED" = 0 ] && reason_has "refused" \
    && record_is "donor=$CHAOS_DONOR restored=$RESTART_NODE block1_equal=0 rejoin_s=-1"
}
for d in seed n5 foo n9; do
  setup "p4-$d"; CHAOS_DONOR="$d"; run
  refused_ok && reason_has "$d"; res $? "P4 donor '$d' refused: no stop/start, target untouched, CHAOS_RECOVERED=0"
done
setup p4-tseed; RESTART_NODE=seed; run
refused_ok; res $? "P4 target seed refused: no stop/start, nothing touched"

setup p4-direct
chaos_donor_check n1 >/dev/null && chaos_donor_check n2 >/dev/null; res $? "P4 donor check accepts live producers n1, n2"
for d in seed n5 foo n9 ""; do
  ! chaos_donor_check "$d" >/dev/null; res $? "P4 donor check refuses '$d'"
done

setup p5a; HASH_8501=OTHER; run
refused_ok && reason_has "not full-history"; res $? "P5a donor block 1 differs from the seed: refused before any stop"
setup p5b; HASH_8500=""; run
refused_ok; res $? "P5b seed block 1 unknown: refused before any stop"
setup p5c nostate; run
refused_ok; res $? "P5c donor without state_db/: refused before any stop"

setup p7; PATH="$S/failbin:$PATH" run
[ "$RC" = 1 ] && [ "$CHAOS_RECOVERED" = 0 ] && reason_has "rsync" && calls_are "$OK_CALLS"
res $? "P7 rsync failure: CHAOS_RECOVERED=0 with an rsync reason, both nodes restarted"

setup w; TN="$TESTNET_HOME"
bash -c 'command sleep 30' _ --data-dir "$TN/n10/data" --log-level info & BGPID=$!
command sleep 0.3
! wait_stopped n10 1; res $? "W wait_stopped fails while a node process holds its data dir"
wait_stopped n1 1; res $? "W wait_stopped n1 is not fooled by n10's process"
kill "$BGPID" 2>/dev/null; wait "$BGPID" 2>/dev/null; BGPID=""

setup p6; wait_stopped() { [ "$1" != n1 ]; }; run
[ "$RC" = 1 ] && [ "$CHAOS_RECOVERED" = 0 ] && reason_has "did not stop" && calls_are "$OK_CALLS" && untouched \
  && record_is "donor=n1 restored=n5 block1_equal=0 rejoin_s=-1"
res $? "P6 donor did not stop: nothing removed or copied, both restarted, CHAOS_RECOVERED=0"

body_of() { awk -v f="$1" '$0 ~ "^"f"\\(\\)\\{" {p=1} p {print} p && /^}/ {exit}' "$GSH"; }
body_of chaos_data_wipe | awk '/wait_rejoin/{w=NR} /chaos_full_history_restore/{r=NR} END{exit !(w && r > w)}'
res $? "S1 chaos_data_wipe calls the restore after its rejoin wait"
body_of chaos_guard | grep -q 'chaos_donor_check "\$CHAOS_DONOR"'; res $? "S2 chaos_guard checks the donor before any injection"
head -170 "$GSH" | grep -q GAUNTLET_CHAOS_DONOR && head -170 "$GSH" | grep -qi "full-history restore"
res $? "S3 header names the full-history restore and GAUNTLET_CHAOS_DONOR"
grep -qF 'CHAOS_DONOR="${GAUNTLET_CHAOS_DONOR:-n1}"' "$GSH"; res $? "S4 donor defaults to n1"
awk '/CHAOS_RECOVERED" != "1" \]; then/,/^fi/' "$GSH" | grep -q CHAOS_REASON; res $? "S5 CHAOS-RECOVERY record carries CHAOS_REASON"
! grep -Eq 'data\.bak|pkill' "$SECTION"; res $? "S6 restore section never names data.bak or pkill"

echo "SUMMARY fails=$FAILS"
[ "$FAILS" = 0 ]
