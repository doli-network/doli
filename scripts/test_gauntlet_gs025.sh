#!/usr/bin/env bash
# test_gauntlet_gs025.sh — offline RED tests for GS-025 (INC-I-253, REQ-INTEG-008). Never starts a node.
# covers: gauntlet-gs025.sh integrity_hole_punch.rs gauntlet.sh gauntlet-seed.sql README.md Cargo.toml
# REQ-INTEG-008 — Decision: a FAIL means GS-025 cannot tell the phantom height-0 scan from a clean one, or its runner/helper can touch the live testnet.
# OUTPUT CONTRACT: scripts/gauntlet-gs025.sh (sourced with GS025_LIB_ONLY=1) + bins/node/examples/integrity_hole_punch.rs
#   O1 gs025_scan_stats LOG       -- stdout: ONE line `scans= complete= missing_scans= full_backfills= targeted=
#                                    repair_passes= imported= heights_read= backoffs= successes=`; absent LOG -> all 0
#   O2 gs025_verdict_a LOG        -- stdout: PASS|FAIL gs025-a-three-scans | -a-no-missing | -a-no-repair; rc 0 iff all PASS
#   O3 gs025_verdict_b LOG HOLE   -- stdout: PASS|FAIL gs025-b-repaired-once | -b-names-hole | -b-complete-after | -b-no-loop;
#                                    rc 0 iff all PASS
#   O4 gs025_pick_checkpoint DIR  -- stdout: path of the oldest h<digits>-<digits> subdir, rc 0 | empty, rc 1
#   O5 gs025_pick_hole ARCH LO HI -- stdout: height nearest (LO+HI)/2 (up, then down) with <010d>.block AND .blake3,
#                                    never LO, HI or <= 1 | empty, rc 1
#   O6 gs025_metric A B HOLE      -- stdout: ONE `METRIC partA_scans= partA_repair_passes= partA_heights_read=
#                                    partB_repair_passes= partB_imported= partB_heights_read= hole=` line; empty part -> none
#   O7 GS025_LIB_ONLY=1 source    -- six functions defined; prints nothing; starts no process; creates no scratch dir
#   O8 gauntlet-gs025.sh static   -- no pkill/killall/launchctl/ssh; kill only "$PID"; rm -r only in a gs025. case arm;
#                                    no write, node dir or punch target under testnet; no --producer; no node_key copy;
#                                    OUTPUT CONTRACT header; build running -> SKIP exit 2; names integrity_hole_punch; bash -n
#   O9 gauntlet.sh assert() + gauntlet-seed.sql GS-025 row -- row tokens == the 7 verdict ids; each SKIPs rc 2 naming the runner
#   O10 integrity_hole_punch DIR H -- `REFUSE ...` + exit 1, DIR unchanged, no DB created
# INPUT PARTITIONS:
#   ST1 phantom (3 full passes, 0 imported) · ST2 fixed (3 complete + noise) · ST3 hole repaired · ST4 mixed (full pass
#   imported, targeted, backoff, BRIDGE/After/Scan-at decoys) · ST5 absent LOG
#   VA1 fixed · VA2 phantom · VA3 two scans · VA4 missing scan · VA5 targeted repair · VA6 absent LOG
#   VB1 repaired · VB2 complete only before · VB3 second repair · VB4 missing scan after · VB5 backoff · VB6 missing scan before
#   VB7 HOLE prefix 14000 · VB8 suffix 40000 · VB9 range tip 281500 · VB10 timestamp digits · VB11 list first/mid/last
#   VB12 phantom · VB13 absent LOG
#   PC1 mixed names + file decoy · PC2 99999 vs 100000 · PC3 empty · PC4 absent · PC5 decoys only
#   PH1 dense · PH2 mid absent, up · PH3 below only · PH4 half pairs · PH5 unpadded names · PH6 only LO/HI · PH7 LO 0, <= 1
#   PH8 empty · PH9 LO 1, HI 3
#   MT1 distinct values · MT2 A empty · MT3 B empty · MT4 stats pipeline
#   F1 planted violations flagged · F2 GS-023-style lines clean
#   H1 protected root · H2 $HOME/testnet + other root (additive) · H3 symlink · H4 ../ path · H5 empty dir · H6 absent dir
#   H7 LOCK held
# MATRIX: O1:ST1-ST5 · O2:VA1-VA6 · O3:VB1-VB13 · O4:PC1-PC5 · O5:PH1-PH9 · O6:MT1-MT4 · O7:L1 · O8:S1-S10,F1-F2
#         O9:GT1-GT2,SQ1 · O10:H1-H7

# shellcheck disable=SC2016  # check conditions are single-quoted on purpose: ck/sck/tck/pck eval them
# shellcheck source=scripts/test_gauntlet_gs025_lib.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/test_gauntlet_gs025_lib.sh"

print_header "gauntlet-gs025.sh RED tests (INC-I-253, GS-025, REQ-INTEG-008)"
echo -e "${CYAN}Test directory: $TEST_DIR${NC}"
write_fixtures

# L1 — REQ-INTEG-008 — Decision: a library that runs the scenario on source starts a scratch node whenever a test or gauntlet.sh loads it.
new_case l1_lib_only; : > "$MUTATE_LOG"; bounded _load_body
NFN="$(cat "$CASE_DIR/nfn" 2>/dev/null || echo 0)"; OUT="$(cat "$CASE_DIR/out" 2>/dev/null)"; ERR="$(cat "$CASE_DIR/err" 2>/dev/null)"
DEFINED=yes; RC=0
ck "L1 lib-only source: all six gs025_* functions defined" '[[ "$NFN" -eq 6 ]]' "defined=$NFN/6 lib=$LIB"
ck "L1 lib-only source: prints nothing" '[[ -z "$OUT" && -z "$ERR" ]]'
ck "L1 lib-only source: starts no process, creates no scratch dir" \
   '[[ ! -s "$MUTATE_LOG" && -z "$(ls -A "$CASE_DIR/tmp")" ]] && not_timed_out' \
   "procs=[$(tr '\n' '|' < "$MUTATE_LOG")] tmp=[$(ls -A "$CASE_DIR/tmp" | tr '\n' ' ')]"

# ST1 — REQ-INTEG-008 — Decision: a miscount of phantom passes makes the ccc2ca37 control METRIC show no repair cost, so before/after proves nothing.
call st1 gs025_scan_stats "$FIX/phantom.log"
ck "ST1 stats phantom: scans=3 missing_scans=3 full_backfills=3 repair_passes=3 heights_read=844794 imported=0" \
   'stats_is "scans=3 complete=0 missing_scans=3 full_backfills=3 targeted=0 repair_passes=3 imported=0 heights_read=844794 backoffs=0 successes=0"'

# ST2 — REQ-INTEG-008 — Decision: counting archiver noise as scans or repairs turns a clean fixed run into a FAIL.
call st2 gs025_scan_stats "$FIX/fixed.log"
ck "ST2 stats fixed: scans=3 complete=3, every repair counter 0" \
   'stats_is "scans=3 complete=3 missing_scans=0 full_backfills=0 targeted=0 repair_passes=0 imported=0 heights_read=0 backoffs=0 successes=0"'

# ST3 — REQ-INTEG-008 — Decision: Part B is judged on these counters; a targeted pass read wrong hides a second repair or a 0-import repair.
call st3 gs025_scan_stats "$FIX/hole_ok.log"
ck "ST3 stats hole repaired: scans=2 complete=1 targeted=1 repair_passes=1 imported=1 heights_read=1 successes=1" \
   'stats_is "scans=2 complete=1 missing_scans=0 full_backfills=0 targeted=1 repair_passes=1 imported=1 heights_read=1 backoffs=0 successes=1"'

# ST4 — REQ-INTEG-008 — Decision: counting BRIDGE imports, "Auto-repair imported" echoes or After/Scan-at lines double-counts the work a repair did.
call st4 gs025_scan_stats "$FIX/mixed.log"
ck "ST4 stats mixed: imported=3 (2 full + 1 targeted; BRIDGE 5 excluded) heights_read=301 backoffs=1 scans=4" \
   'stats_is "scans=4 complete=1 missing_scans=2 full_backfills=1 targeted=1 repair_passes=2 imported=3 heights_read=301 backoffs=1 successes=1"'

# ST5 — REQ-INTEG-008 — Decision: a node that never wrote its log must read as zero work, not as an error that aborts the verdict.
call st5 gs025_scan_stats "$FIX/no-such.log"
ck "ST5 stats absent log: all ten counters 0" \
   'stats_is "scans=0 complete=0 missing_scans=0 full_backfills=0 targeted=0 repair_passes=0 imported=0 heights_read=0 backoffs=0 successes=0"'

# VA1 — REQ-INTEG-008 — Decision: a Part A verdict that fails a clean store blocks the fix it is meant to certify.
call va1 gs025_verdict_a "$FIX/fixed.log"
ck "VA1 verdict_a fixed: three PASS lines, rc 0" \
   '[[ "$RC" -eq 0 ]] && is_pass gs025-a-three-scans && is_pass gs025-a-no-missing && is_pass gs025-a-no-repair'

# VA2 — REQ-INTEG-008 — Decision: Part A must FAIL on ccc2ca37, or GS-025 cannot see INC-I-253 at all.
call va2 gs025_verdict_a "$FIX/phantom.log"
ck "VA2 verdict_a phantom: all three FAIL, rc 1" \
   '[[ "$RC" -eq 1 ]] && is_fail gs025-a-three-scans && is_fail gs025-a-no-missing && is_fail gs025-a-no-repair'

# VA3 — REQ-INTEG-008 — Decision: fewer than 3 scans proves no cadence; a pass on 2 lets a run that ended early certify the fix.
call va3 gs025_verdict_a "$FIX/two_scans.log"
ck "VA3 verdict_a 2 complete scans: three-scans FAIL, others PASS, rc 1" \
   '[[ "$RC" -eq 1 ]] && is_fail gs025-a-three-scans && is_pass gs025-a-no-missing && is_pass gs025-a-no-repair'

# VA4 — REQ-INTEG-008 — Decision: one missing scan among complete ones is the phantom coming back at a new floor.
call va4 gs025_verdict_a "$FIX/fixed_missing.log"
ck "VA4 verdict_a 3 complete + 1 missing scan: no-missing FAIL, three-scans PASS, rc 1" \
   '[[ "$RC" -eq 1 ]] && is_fail gs025-a-no-missing && is_pass gs025-a-three-scans'

# VA5 — REQ-INTEG-008 — Decision: any repair pass on a complete store is wasted archive I/O, the cost INC-I-253 measures.
call va5 gs025_verdict_a "$FIX/fixed_repair.log"
ck "VA5 verdict_a complete + 1 targeted pass: no-repair FAIL, no-missing PASS, rc 1" \
   '[[ "$RC" -eq 1 ]] && is_fail gs025-a-no-repair && is_pass gs025-a-no-missing'

# VA6 — REQ-INTEG-008 — Decision: an absent log must not PASS by having nothing to count.
call va6 gs025_verdict_a "$FIX/no-such.log"
ck "VA6 verdict_a absent log: three-scans FAIL, rc 1" '[[ "$RC" -eq 1 ]] && is_fail gs025-a-three-scans'

# VB1 — REQ-INTEG-008 — Decision: a Part B verdict that fails one clean targeted repair blocks the fix it certifies.
call vb1 gs025_verdict_b "$FIX/hole_ok.log" 140000
ck "VB1 verdict_b repaired once: four PASS lines, rc 0" \
   '[[ "$RC" -eq 0 ]] && is_pass gs025-b-repaired-once && is_pass gs025-b-names-hole && is_pass gs025-b-complete-after && is_pass gs025-b-no-loop'

# VB2 — REQ-INTEG-008 — Decision: a complete scan before the repair says nothing about whether the repair closed the hole.
call vb2 gs025_verdict_b "$FIX/hole_before.log" 140000
ck "VB2 verdict_b complete only before the repair: complete-after FAIL, repaired-once PASS, rc 1" \
   '[[ "$RC" -eq 1 ]] && is_fail gs025-b-complete-after && is_pass gs025-b-repaired-once'

# VB3 — REQ-INTEG-008 — Decision: a second pass over a repaired height is the repeat-repair loop the back-off must stop.
call vb3 gs025_verdict_b "$FIX/hole_twice.log" 140000
ck "VB3 verdict_b second targeted pass (0 imported): repaired-once FAIL, rc 1" '[[ "$RC" -eq 1 ]] && is_fail gs025-b-repaired-once'
ck "VB3 verdict_b second targeted pass: no-loop FAIL" 'is_fail gs025-b-no-loop'

# VB4 — REQ-INTEG-008 — Decision: a missing scan after the repair means the imported block did not close the hole.
call vb4 gs025_verdict_b "$FIX/hole_loop.log" 140000
ck "VB4 verdict_b missing scan after the repair: no-loop FAIL, complete-after PASS, rc 1" \
   '[[ "$RC" -eq 1 ]] && is_fail gs025-b-no-loop && is_pass gs025-b-complete-after'

# VB5 — REQ-INTEG-008 — Decision: a back-off line in Part B means a repair failed and the node is parked, not repaired.
call vb5 gs025_verdict_b "$FIX/hole_backoff.log" 140000
ck "VB5 verdict_b backoff line: no-loop FAIL, repaired-once PASS, rc 1" \
   '[[ "$RC" -eq 1 ]] && is_fail gs025-b-no-loop && is_pass gs025-b-repaired-once'

# VB6 — REQ-INTEG-008 — Decision: a missing scan BEFORE the repair is the hole being found; failing it fails every honest run.
call vb6 gs025_verdict_b "$FIX/hole_pre_missing.log" 140000
ck "VB6 verdict_b missing scan before the first repair: no-loop PASS, rc 0" '[[ "$RC" -eq 0 ]] && is_pass gs025-b-no-loop'

# VB7-VB10 — REQ-INTEG-008 — Decision: a substring match credits the wrong height (14000 in 140000, the tip, a timestamp), so a repair of another block passes.
for hc in "VB7 14000 prefix" "VB8 40000 suffix" "VB9 281500 range-tip-outside-brackets" "VB10 712253 timestamp-digits"; do
    read -r id h why <<< "$hc"
    call "$id" gs025_verdict_b "$FIX/hole_ok.log" "$h"
    ck "$id verdict_b HOLE=$h ($why) vs [140000]: names-hole FAIL, rc 1" '[[ "$RC" -eq 1 ]] && is_fail gs025-b-names-hole'
done

# VB11 — REQ-INTEG-008 — Decision: a whole-number match that misses the first or last list element fails a correct run.
for h in 139998 140000 140002; do
    call "vb11_$h" gs025_verdict_b "$FIX/hole_list.log" "$h"
    ck "VB11 verdict_b HOLE=$h in [139998, 140000, 140002]: names-hole PASS" 'is_pass gs025-b-names-hole'
done

# VB12 — REQ-INTEG-008 — Decision: the ccc2ca37 full pass that imports 0 must never read as a repair.
call vb12 gs025_verdict_b "$FIX/phantom.log" 140000
ck "VB12 verdict_b phantom log: repaired-once FAIL, names-hole FAIL, rc 1" \
   '[[ "$RC" -eq 1 ]] && is_fail gs025-b-repaired-once && is_fail gs025-b-names-hole'

# VB13 — REQ-INTEG-008 — Decision: a Part B node that wrote no log must not PASS.
call vb13 gs025_verdict_b "$FIX/no-such.log" 140000
ck "VB13 verdict_b absent log: repaired-once FAIL, rc 1" '[[ "$RC" -eq 1 ]] && is_fail gs025-b-repaired-once'

# PC1 — REQ-INTEG-008 — Decision: a younger checkpoint is < 300 blocks behind, so fewer than 3 scans fire and Part A cannot run.
D="$TEST_DIR/pc1"; mk_ckpts "$D" h281646-1792448949 h281596-1792448450 h281796-1792450449 junk h12x-1 h281000-abc; : > "$D/h100-1"
call pc1 gs025_pick_checkpoint "$D"
ck "PC1 pick_checkpoint: oldest h281596 wins; junk, h12x-1, h281000-abc and file h100-1 ignored" \
   '[[ "$RC" -eq 0 && "${OUT%/}" == "$D/h281596-1792448450" ]]'

# PC2 — REQ-INTEG-008 — Decision: a lexical sort ranks h100000 before h99999 and picks the younger checkpoint.
D="$TEST_DIR/pc2"; mk_ckpts "$D" h100000-2 h99999-1
call pc2 gs025_pick_checkpoint "$D"
ck "PC2 pick_checkpoint: numeric order picks h99999 over h100000" '[[ "$RC" -eq 0 && "${OUT%/}" == "$D/h99999-1" ]]'

# PC3-PC5 — REQ-INTEG-008 — Decision: no checkpoint must stop the run, not clone an empty or junk dir as the node's data.
mkdir -p "$TEST_DIR/pc3"; call pc3 gs025_pick_checkpoint "$TEST_DIR/pc3"
ck "PC3 pick_checkpoint empty dir: rc 1, empty stdout" '[[ "$RC" -eq 1 && -z "$OUT" ]]'
call pc4 gs025_pick_checkpoint "$TEST_DIR/pc4-absent"
ck "PC4 pick_checkpoint absent dir: rc 1, empty stdout" '[[ "$RC" -eq 1 && -z "$OUT" ]]'
D="$TEST_DIR/pc5"; mk_ckpts "$D" junk h12x-1 h281000-abc; : > "$D/h100-1"
call pc5 gs025_pick_checkpoint "$D"
ck "PC5 pick_checkpoint only decoys: rc 1, empty stdout" '[[ "$RC" -eq 1 && -z "$OUT" ]]'

# PH1 — REQ-INTEG-008 — Decision: a hole at the store edge is repaired by catch-up, not by the integrity scan, so Part B tests the wrong path.
mk_arch "$TEST_DIR/ph1" both $(seq 100 200)
call ph1 gs025_pick_hole "$TEST_DIR/ph1" 100 200
ck "PH1 pick_hole dense 100..200: 150" '[[ "$RC" -eq 0 && "$OUT" == 150 ]]'

# PH2-PH3 — REQ-INTEG-008 — Decision: a punched height with no archive file can never be repaired, so Part B FAILs for the fixture, not the fix.
mk_arch "$TEST_DIR/ph2" both 100 140 160 170 200
call ph2 gs025_pick_hole "$TEST_DIR/ph2" 100 200
ck "PH2 pick_hole mid 150 absent, 140/160 present: 160 (upward first)" '[[ "$RC" -eq 0 && "$OUT" == 160 ]]'
mk_arch "$TEST_DIR/ph3" both 100 120 140 200
call ph3 gs025_pick_hole "$TEST_DIR/ph3" 100 200
ck "PH3 pick_hole nothing in 151..199: downward to 140, never HIGH 200" '[[ "$RC" -eq 0 && "$OUT" == 140 ]]'

# PH4 — REQ-INTEG-008 — Decision: a .block without its .blake3 is rejected by the targeted import, so a hole there imports 0.
mk_arch "$TEST_DIR/ph4" block 150; mk_arch "$TEST_DIR/ph4" blake3 151; mk_arch "$TEST_DIR/ph4" both 100 152 200
call ph4 gs025_pick_hole "$TEST_DIR/ph4" 100 200
ck "PH4 pick_hole 150 .block only, 151 .blake3 only: 152" '[[ "$RC" -eq 0 && "$OUT" == 152 ]]'

# PH5 — REQ-INTEG-008 — Decision: the archiver reads only <010d> names; an unpadded file is invisible to it.
mk_arch "$TEST_DIR/ph5" both 100 160 200; : > "$TEST_DIR/ph5/150.block"; : > "$TEST_DIR/ph5/150.blake3"
call ph5 gs025_pick_hole "$TEST_DIR/ph5" 100 200
ck "PH5 pick_hole unpadded 150.block/.blake3 ignored: 160" '[[ "$RC" -eq 0 && "$OUT" == 160 ]]'

# PH6-PH8 — REQ-INTEG-008 — Decision: punching LO, HI or block <= 1 removes the scan floor or tip, which the scan does not report as a hole.
mk_arch "$TEST_DIR/ph6" both 100 200
call ph6 gs025_pick_hole "$TEST_DIR/ph6" 100 200
ck "PH6 pick_hole only LO/HI present: rc 1, empty stdout" '[[ "$RC" -eq 1 && -z "$OUT" ]]'
mk_arch "$TEST_DIR/ph7" both 0 1 4
call ph7 gs025_pick_hole "$TEST_DIR/ph7" 0 4
ck "PH7 pick_hole LO 0 HI 4, only 0/1/4 present: rc 1 (never <= 1, never HI)" '[[ "$RC" -eq 1 && -z "$OUT" ]]'
mkdir -p "$TEST_DIR/ph8"; call ph8 gs025_pick_hole "$TEST_DIR/ph8" 100 200
ck "PH8 pick_hole empty archive: rc 1, empty stdout" '[[ "$RC" -eq 1 && -z "$OUT" ]]'

# PH9 — REQ-INTEG-008 — Decision: an off-by-one exclusion of LO+1 leaves no hole on a short store.
mk_arch "$TEST_DIR/ph9" both 1 2 3
call ph9 gs025_pick_hole "$TEST_DIR/ph9" 1 3
ck "PH9 pick_hole LO 1 HI 3: 2" '[[ "$RC" -eq 0 && "$OUT" == 2 ]]'

SA="scans=4 complete=1 missing_scans=3 full_backfills=2 targeted=5 repair_passes=7 imported=6 heights_read=844794 backoffs=8 successes=9"
SB="scans=12 complete=11 missing_scans=10 full_backfills=13 targeted=14 repair_passes=27 imported=15 heights_read=16 backoffs=17 successes=18"
# MT1 — REQ-INTEG-008 — Decision: a field read from the wrong key misreports the before/after metric the milestone is judged on.
call mt1 gs025_metric "$SA" "$SB" 140000
ck "MT1 metric: one line, each field from its own key" \
   '[[ "$OUT" == "METRIC partA_scans=4 partA_repair_passes=7 partA_heights_read=844794 partB_repair_passes=27 partB_imported=15 partB_heights_read=16 hole=140000" ]]'

# MT2-MT3 — REQ-INTEG-008 — Decision: a part that did not run must print none, never 0, which reads as a clean result.
call mt2 gs025_metric "" "$SB" 140000
ck "MT2 metric Part A empty: partA_* = none" \
   '[[ "$OUT" == "METRIC partA_scans=none partA_repair_passes=none partA_heights_read=none partB_repair_passes=27 partB_imported=15 partB_heights_read=16 hole=140000" ]]'
call mt3 gs025_metric "$SA" "" ""
ck "MT3 metric Part B empty: partB_* = none, one line" \
   '[[ "$(printf "%s\n" "$OUT" | grep -c .)" -eq 1 && "$OUT" == "METRIC partA_scans=4 partA_repair_passes=7 partA_heights_read=844794 partB_repair_passes=none partB_imported=none partB_heights_read=none hole="* ]]'

# MT4 — REQ-INTEG-008 — Decision: stats and metric must agree end to end, or the control run's METRIC line misstates the phantom cost.
_t_pipe() { gs025_metric "$(gs025_scan_stats "$1")" "$(gs025_scan_stats "$2")" "$3"; }
call mt4 _t_pipe "$FIX/phantom.log" "$FIX/hole_ok.log" 140000
ck "MT4 metric(stats(phantom), stats(hole repaired)): the ccc2ca37-shape METRIC line" \
   '[[ "$OUT" == "METRIC partA_scans=3 partA_repair_passes=3 partA_heights_read=844794 partB_repair_passes=1 partB_imported=1 partB_heights_read=1 hole=140000" ]]'

# F1 — REQ-INTEG-008 — Decision: a static checker that flags nothing passes any runner; planted violations must each be caught.
REAL_LIB="$LIB"; LIB="$TEST_DIR/planted_bad.sh"
cat > "$LIB" <<'BAD'
pkill doli-node
kill -9 "$SEED_PID"
rm -rf "$S"
case "$S" in "$TMPROOT"/gs025.?*) rm -rf "$HOME/testnet" ;; esac
case "$X" in "$TMPROOT"/gs025.?*) rm -rf "$S" ;; esac
echo x > "$HOME/testnet/seed/marker"
cp -R "$S/data" "$GS025_TESTNET_DIR/seed/data"
"$GS025_BINARY" --network testnet run --data-dir "$GS025_TESTNET_DIR/seed/data"
"$GS025_PUNCH_BIN" "$GS025_TESTNET_DIR/seed/data/blocks" 140000
BAD
tck "F1 checker self-test: planted kill / 3 rm -r / 5 testnet writes are flagged" \
    '[[ "$(bad_kills | grep -c .)" -eq 1 && "$(bad_rms | grep -c .)" -eq 3 && "$(testnet_writes | sort -u | grep -c .)" -eq 5 ]]' \
    "kills=$(bad_kills | grep -c .) rms=$(bad_rms | grep -c .) tn=$(testnet_writes | sort -u | grep -c .)"

# F2 — REQ-INTEG-008 — Decision: a checker that flags the GS-023 cleanup idiom forces an unsafe workaround.
LIB="$TEST_DIR/planted_good.sh"
cat > "$LIB" <<'GOOD'
case "$S" in "$TMPROOT"/gs025.?*) rm -rf "$S" ;; esac
kill -TERM "$PID" 2>/dev/null
clone "$GS025_CHECKPOINT" "$S/data" || skip "cannot clone checkpoint"
GS025_TESTNET_DIR="${GS025_TESTNET_DIR:-$HOME/testnet}"
args=(--network testnet --data-dir "$S/data" --archive-to "$S/archive")
GS025_PROTECTED_ROOTS="$GS025_TESTNET_DIR" "$GS025_PUNCH_BIN" "$S/data/blocks" "$HOLE"
"$GS025_BINARY" "${args[@]}" > "$S/node.log" 2>&1 &
GOOD
tck "F2 checker self-test: GS-023-style cleanup, clone, args and punch lines are clean" \
    '[[ -z "$(bad_kills)$(bad_rms)$(testnet_writes)" ]]' "flagged=[$(bad_kills; bad_rms; testnet_writes)]"
LIB="$REAL_LIB"

# S1 — REQ-INTEG-008 — Decision: pkill, killall, launchctl or ssh can stop live testnet nodes or reach mainnet hosts.
sck "S1 static: no pkill / killall / launchctl / ssh" \
    '! code_lines | grep -qE "(^|[^A-Za-z0-9_-])(pkill|killall|launchctl|ssh)([^A-Za-z0-9_-]|\$)"' \
    "$(code_lines | grep -nE '(^|[^A-Za-z0-9_-])(pkill|killall|launchctl|ssh)([^A-Za-z0-9_-]|$)' | head -3)"

# S2 — REQ-INTEG-008 — Decision: a kill of any PID but the scratch node's can stop the live seed.
sck "S2 static: every kill targets only \"\$PID\"" '[[ -z "$(bad_kills)" ]]' "$(bad_kills | head -3 | tr '\n' '|')"

# S3 — REQ-INTEG-008 — Decision: an rm -r outside a gs025. scratch arm can wipe the seed's data or keys.
sck "S3 static: rm -r only as \`<gs025. pattern>) rm -rf \"\$VAR\"\` case arm" '[[ -z "$(bad_rms)" ]]' "$(bad_rms | head -3 | tr '\n' '|')"

# S4 — REQ-INTEG-008 — Decision: a write, node data dir or punch under testnet corrupts the live seed's store.
sck "S4 static: no redirect/cp/rm/--data-dir/--archive-to/punch into a testnet path" '[[ -z "$(testnet_writes)" ]]' \
    "$(testnet_writes | head -3 | tr '\n' '|')"

# S5 — REQ-INTEG-008 — Decision: a --producer scratch node signs blocks beside the live producers.
sck "S5 static: scratch node never started with --producer" '! code_lines | grep -qE -- "--producer"' \
    "$(code_lines | grep -nE -- '--producer' | head -3)"

# S6 — REQ-INTEG-008 — Decision: a copied node_key gives the scratch node the seed's peer ID.
sck "S6 static: node_key never copied" \
    '! code_lines | grep "node_key" | grep -qE "(^|[^A-Za-z0-9_-])(cp|rsync|ditto|ln|mv|install|clone)[[:space:]]"' \
    "$(code_lines | grep -n node_key | head -3)"

# S7 — REQ-INTEG-008 — Decision: without the OUTPUT CONTRACT header gauntlet-run.sh readers cannot map PASS/FAIL ids to evidence.
sck "S7 static: OUTPUT CONTRACT: header in the first 40 lines" 'head -n 40 "$LIB" | grep -q "OUTPUT CONTRACT:"'

# S8 — REQ-INTEG-008 — Decision: a build starves the scratch node and slow scans read as a fix failure.
sck "S8 static: a running build -> skip, and skip prints SKIP gs025 and exits 2" \
    'code_lines | grep "build_running" | grep -q "skip" && awk "/skip\\(\\)[[:space:]]*\\{/{f=1} f{print} f&&/}/{exit}" "$LIB" | grep -q "exit 2" && grep -q "SKIP gs025" "$LIB"'

# S9 — REQ-INTEG-008 — Decision: Part B without the helper never punches a hole, so it measures nothing.
sck "S9 static: runner calls the integrity_hole_punch helper" 'code_lines | grep -q "integrity_hole_punch"'

# S10 — REQ-INTEG-008 — Decision: gauntlet-run.sh runs the file directly; a syntax error is a permanent FAIL unrelated to the fix.
sck "S10 hygiene: runner parses under bash -n" 'bash -n "$LIB" 2>/dev/null' "$(bash -n "$LIB" 2>&1 | head -2)"

# S11 — REQ-INTEG-008 — Decision: gauntlet-run.sh runs the bare name via bash -c; a non-executable file exits 126, a FAIL unrelated to the fix.
sck "S11 hygiene: runner file is executable" '[[ -x "$LIB" ]]' "$(ls -l "$LIB" 2>&1 | cut -c1-10)"

# GT1 — REQ-INTEG-008 — Decision: gauntlet.sh reads every active seed row; a GS-025 token it cannot route FAILs every gauntlet.sh run.
for tok in $TOKENS; do
    GA="$(ga_assert "$tok")"
    tck "GT1 gauntlet.sh assert() SKIPs standalone $tok (rc 2, names scripts/gauntlet-gs025.sh)" \
        '[[ "${GA%%|*}" == 2 && "$GA" == *"$tok: standalone scenario, run scripts/gauntlet-gs025.sh (not part of gauntlet.sh)"* && "$GA" == *"|" ]]' \
        "assert $tok -> [$GA]"
done

# GT2 — REQ-INTEG-008 — Decision: a gs025-* glob SKIPs a mistyped seed token instead of failing it.
GA="$(ga_assert gs025-no-such-token)"
tck "GT2 gauntlet.sh assert() still FAILs an unknown gs025-like token" \
    '[[ "${GA%%|*}" == 1 && "$GA" == *"unknown assertion token"* ]]' "assert gs025-no-such-token -> [$GA]"

# SQ1 — REQ-INTEG-008 — Decision: a seed row whose tokens differ from the verdict ids records assertions the runner never prints.
SQ="$(seed_tokens | tr '\n' ' ')"; WANT="$(printf '%s\n' $TOKENS | sort -u | tr '\n' ' ')"
tck "SQ1 gauntlet-seed.sql GS-025 row: assertions == the 7 gs025-* verdict ids" '[[ -n "$SQ" && "$SQ" == "$WANT" ]]' \
    "row=[$SQ] want=[$WANT]"

P="$TEST_DIR/punch"; mkdir -p "$P/home1" "$P/unrelated" "$P/x"
CANON="(canonicalize the input path AND \$HOME/testnet AND each GS025_PROTECTED_ROOTS entry; TMPDIR may sit under /var -> /private/var)"
# H1 — REQ-INTEG-008 — Decision: a helper that opens a protected store before checking the root rotates or corrupts a live node's DB.
fake_db "$P/prot/blocks"; B="$(snap "$P/prot/blocks")"
punch "$P/home1" "$P/prot" "$P/prot/blocks" 140000
pck "H1 punch dir under GS025_PROTECTED_ROOTS: REFUSE exit 1, dir untouched" '[[ "$(snap "$P/prot/blocks")" == "$B" ]] && refused' \
    "rc=$PRC out=[$(printf '%s' "$POUT" | tr '\n' '|')] $CANON"

# H2 — REQ-INTEG-008 — Decision: GS025_PROTECTED_ROOTS that REPLACES \$HOME/testnet unprotects the live seed whenever it is set.
T2="$P/home2/testnet/seed/data/blocks"; fake_db "$T2"; B="$(snap "$T2")"
punch "$P/home2" "$P/unrelated" "$T2" 140000
pck "H2 punch \$HOME/testnet store with an unrelated protected root set: REFUSE exit 1, untouched" \
    '[[ "$(snap "$T2")" == "$B" ]] && refused' "rc=$PRC out=[$(printf '%s' "$POUT" | tr '\n' '|')] $CANON"

# H3-H4 — REQ-INTEG-008 — Decision: a prefix check on the raw path lets a symlink or ../ path reach the testnet store.
ln -s "$T2" "$P/link"
punch "$P/home2" "" "$P/link" 140000
pck "H3 punch symlink into \$HOME/testnet: REFUSE exit 1, untouched" '[[ "$(snap "$T2")" == "$B" ]] && refused' \
    "rc=$PRC out=[$(printf '%s' "$POUT" | tr '\n' '|')] $CANON"
punch "$P/home2" "" "$P/x/../home2/testnet/seed/data/blocks" 140000
pck "H4 punch ../ path into \$HOME/testnet: REFUSE exit 1, untouched" '[[ "$(snap "$T2")" == "$B" ]] && refused' \
    "rc=$PRC out=[$(printf '%s' "$POUT" | tr '\n' '|')] $CANON"

# H5-H6 — REQ-INTEG-008 — Decision: RocksDB open writes LOG/LOCK even with create_if_missing=false, so a mistyped path becomes a new empty DB.
mkdir -p "$P/empty"
punch "$P/home1" "" "$P/empty" 140000
pck "H5 punch empty dir (no CURRENT): REFUSE exit 1, dir still empty" '[[ -z "$(ls -A "$P/empty")" ]] && refused' \
    "rc=$PRC out=[$(printf '%s' "$POUT" | tr '\n' '|')] contents=[$(ls -A "$P/empty" | tr '\n' ' ')]"
punch "$P/home1" "" "$P/absent" 140000
pck "H6 punch absent dir: REFUSE exit 1, dir still absent" '[[ ! -e "$P/absent" ]] && refused'

# H7 — REQ-INTEG-008 — Decision: punching a store a running node holds mutates it under the node, the exact fault the helper must refuse.
fake_db "$P/locked"
# shellcheck disable=SC2034  # B is read by the pck condition (eval)
B="$(cat "$P/locked/CURRENT")"; HOLD=""
if [[ -x "$PUNCH" ]]; then
    python3 -I -c 'import fcntl, sys, time
f = open(sys.argv[1], "r+")
fcntl.lockf(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
open(sys.argv[2], "w").close()
time.sleep(120)' "$P/locked/LOCK" "$P/lock.ready" &
    HOLD=$!; HOLD_PIDS="$HOLD_PIDS $HOLD"
    n=0; while [[ ! -f "$P/lock.ready" && "$n" -lt 50 ]]; do sleep 0.1; n=$((n + 1)); done
    punch "$P/home1" "" "$P/locked" 140000
    kill "$HOLD" 2>/dev/null; wait "$HOLD" 2>/dev/null
fi
pck "H7 punch store whose LOCK another process holds: REFUSE (names the lock) exit 1, no DB written" \
    '[[ -f "$P/lock.ready" && "$(cat "$P/locked/CURRENT")" == "$B" ]] && ! db_created "$P/locked" && refused && printf "%s\n" "$POUT" | grep -E "^REFUSE " | grep -qi lock' \
    "rc=$PRC ready=$([[ -f "$P/lock.ready" ]] && echo yes || echo no) out=[$(printf '%s' "$POUT" | tr '\n' '|')]"

print_header "TEST SUMMARY"
echo -e "  Tests Passed: ${GREEN}$TESTS_PASSED${NC}"
echo -e "  Tests Failed: ${RED}$TESTS_FAILED${NC}"
echo -e "  Total Tests:  $TESTS_TOTAL"
echo

if [ "$TESTS_FAILED" -eq 0 ] && [ "$TESTS_TOTAL" -gt 0 ]; then EXIT_CODE=0; else EXIT_CODE=1; fi
exit $EXIT_CODE
