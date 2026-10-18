#!/usr/bin/env bash
# OUTPUT CONTRACT: scripts/gauntlet-gs024.sh + gauntlet-gs024-lib.sh + gauntlet-gs024-scan.py (INC-I-235, GS-024)
#   O1 gs024_rpc URL METHOD PARAMS -- stdout: raw JSON reply of URL; the request carries METHOD + PARAMS
#   O2 gs024_pick_base TIP         -- rc 0 + stdout == OLDEST admissible canonical pool ckpt-h* dir | rc!=0 + `NOT READY: <reason>`
#   O3 gs024_preflight             -- rc 0 + no NOT READY line | rc!=0 + `NOT READY: <reason>`; a build = a cargo/rustc
#                                     process whose args hold "$ROOT/" (not "$ROOT/.claude/worktrees/"); GS024_PYTHON runs
#   O4 gs024_ranges < ints         -- stdout: `a..b,c..c` | `none`
#   O5 gs024_prune_pool TIP        -- filesystem: removes exactly the pool ckpt-h* dirs with TIP-height > MAX_GAP
#   O6 gs024_main (executed)       -- exit 3 + `NOT READY:`; node never launched; base kept; no pkill/launchctl
#   O7 GS024_LIB_ONLY=1 source     -- all functions defined; nothing printed, queried or launched
#   O8 runner + lib source (static) -- no pkill/killall/launchctl; kill only "$PID"; rm -r targets bounded;
#                                     no node_key copy; no --producer / --no-snap-sync; no /Users/ path;
#                                     names the seven assertion tokens; parses under bash -n / python ast
#   O9 gs024_throttle on|off PID   -- GS024_THROTTLE (default 1) !=1: rc 0, no taskpolicy | =1: one call
#                                     ${GS024_TASKPOLICY:-/usr/sbin/taskpolicy} -b|-B -p PID | bad PID: rc!=0, no call
#   O10 gs024_irpc URL METHOD PARAMS -- rc 0 + first non-empty reply | rc!=0 after ${GS024_RPC_TRIES:-5} curl
#                                     attempts; every attempt carries max time ${GS024_RPC_TIMEOUT:-30}
#   O11 _gs024_stop                -- SIGTERM, ${GS024_STOP_GRACE:-180} s grace, then SIGKILL + rc 2
#   O12 gs024_main (full run)      -- RESULT line throttle=0|1; THROTTLE=1: -b on the node PID after its RPC
#                                     answers, -B before verifyChainIntegrity and the stop; instance RPC max time >= 30
#   O13 gs024_main throttle window -- -B no earlier than ${GS024_THROTTLE_WINDOW:-600} s after -b; gs024-gap-held PASS iff
#                                     seed - instance >= MIN_GAP at the window end or at the first snap order
#                                     (end=snap_ordered), or a logged [COORDINATOR] action=X gap=N with N >= MIN_GAP
#                                     (end=coordinator), or the post-lockout gap (first sample >= RPC-up +
#                                     ${GS024_LOCKOUT_EVAL:-90} s) >= MIN_GAP; detail post_lockout_gap=N|none
#                                     (none = tip reached first); RESULT carries window_gap=N, post_window_s=S|none
#   O14 gs024_main hole scan       -- scanner ($GS024_PYTHON) on the instance URL, lo = base+1 | 1 (GS024_FULL_SCAN=1),
#                                     hi = H; scanner missing>0, errors>0 or rc!=0 -> FAIL gs024-no-missing-height + range
#   O15 gauntlet-gs024-scan.py URL LO HI [--compare URL2] -- one line `scan LO..HI checked= missing= ranges=
#                                     mismatched= mismatch_ranges= errors=`; exit 0 iff missing=errors=mismatched=0
# INPUT PARTITIONS (TIP 50000, MIN_GAP 500, MAX_GAP 20000 unless stated):
#   B1 gap 499 · B2 gap 500 · B3 gap 20000 · B4 gap 20001 · B5 hash != seed · B6 empty/absent pool
#   B7 oldest valid wins · B7b oldest non-canonical skipped · B8 9800 vs 10200 (numeric order) · B9 no-hash dirs
#   B10 base above tip · B11 non-ckpt dirs only · B12 seed down · B13 SEED_RPC + MIN_GAP overrides
#   P1 all clear · P2 each port busy · P3a rustc in ROOT/target · P3b cargo --manifest-path ROOT · P3c other repo
#   P3d cargo without a path · P3e build under ROOT/.claude/worktrees · P3f non-build process in ROOT
#   P4 ALLOW_BUILD=1 + build · P5 ALLOW_BUILD=1 + busy · P6 seed down · P7 BIN missing / not executable / a directory
#   P8 port overrides · P9 GS024_PYTHON missing
#   R1-R6 ranges · X1 mixed pool + cwd decoy · X2 MAX_GAP override · X3 empty TIP
#   M1 too-young base · M2 empty pool · M3 busy port · M4 build running · L1 lib-only load
#   T1 unset (default on) · T2 0 · T3 on · T4 off · T5 empty PID · T6 non-numeric PID · L2 new functions defined
#   I1 first answers · I2 fail x2 · I3 empty x2 · I4 down · I5 TRIES=2 · I6 default 30 · I7 TIMEOUT=7
#   I8 forwards METHOD+PARAMS · I9 fail x4 · G1 default 180 · G2 GRACE=1 + TERM-ignoring instance
#   MT0 unthrottled, instance calls 2-3 fail · MT1 throttled, tip reached · MT2 throttled, catch-up timeout
#   W1 default window static · GH1 throttled, 20 behind through a 2 s window, catches up on -B
#   GH2 throttled, at tip inside the window · GH3 snap ordered 20 behind · GH4 coordinator gap 20 · GH5 coordinator gap 9 · FS1 default lo · FS2 FULL_SCAN=1 · FS3 scanner missing · FS4 scanner errors
#   GH6 post-lockout gap 20, window-end gap 5, no line · GH7 post-lockout gap 5, window-end 5 · GH8 20 behind, tip before the
#   post-lockout sample · GH9 default LOCKOUT_EVAL static
#   SC1 holes · SC2 none · SC3 --compare mismatch · SC4 node down · SC5 usage error
#   GT1 each gs024-* token · GT2 each gs023-* token · GT3 unknown gs024-like token
# O16 gauntlet.sh assert() TOKEN (real function, extracted) -- standalone gs023-*/gs024-* token: rc 2, SKIP reason
#                                     names its runner, no FAIL reason | unknown token: rc 1 + FAIL "unknown assertion token"
# MATRIX: O1:R0a-R0b · O2:B1-B13 · O3:P1-P9 · O4:R1-R6 · O5:X1-X3 · O6:M1-M4 · O7:L1-L2 · O8:S1-S12
#         O9:T1-T6,S10 · O10:I1-I9 · O11:G1-G2 · O12:MT0-MT2 · O13:W1,GH1-GH9 · O14:FS1-FS4 · O15:SC1-SC5
#         O16:GT1-GT3

# shellcheck source=scripts/test_gauntlet_gs024_lib.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/test_gauntlet_gs024_lib.sh"

print_header "gauntlet-gs024.sh RED tests (INC-I-235, GS-024)"
echo -e "${CYAN}Test directory: $TEST_DIR${NC}"

# L1 — REQ-ARCHSYNC-004 — Decision: a library that runs gs024_main on source launches a drill node whenever a test or gauntlet.sh loads it.
new_sandbox l1_lib_only; bounded _load_body
NFN="$(cat "$CASE_DIR/nfn" 2>/dev/null || echo 0)"; OUT="$(cat "$CASE_DIR/out" 2>/dev/null)"; ERR="$(cat "$CASE_DIR/err" 2>/dev/null)"
DEFINED=yes; RC=0
ck "L1 lib_only: all six gs024_* functions defined" '[[ "$NFN" -eq 6 ]]' "defined=$NFN/6 lib=$LIB"
ck "L1 lib_only: sourcing prints nothing" '[[ -z "$OUT" && -z "$ERR" ]]'
ck "L1 lib_only: sourcing queries no RPC and launches nothing" \
   '[[ ! -s "$CURL_LOG" && ! -s "$BIN_LOG" ]] && not_timed_out' "curl=$(tr '\n' ' ' < "$CURL_LOG") bin=$(tr '\n' ' ' < "$BIN_LOG")"

# R0 — REQ-ARCHSYNC-004 — Decision: an rpc helper that ignores its URL argument reads the seed when the caller asked the drill node, so seed-vs-drill commitment checks compare the seed with itself.
new_sandbox r0_rpc; mkdir -p "$FIX_DIR/$ALT_PORT"; echo 777 > "$FIX_DIR/$ALT_PORT/tip"
call_fn gs024_rpc "http://127.0.0.1:$ALT_PORT" getChainInfo '[]'
ck "R0a rpc: returns the raw reply of the URL it was given" '[[ "$OUT" == *"\"bestHeight\":777"* ]]'
seed_has 49000
call_fn gs024_rpc "http://127.0.0.1:$SEED_PORT" getBlockByHeight '{"height":49000}'
ck "R0b rpc: forwards METHOD and PARAMS to the node" \
   '[[ "$OUT" == *"$(hx 49000)"* ]] && grep -q getBlockByHeight "$CURL_LOG"' "$(det) curl=$(tr '\n' ' ' < "$CURL_LOG")"

# B1 — REQ-ARCHSYNC-004 — Decision: a base 499 behind sits under the 500 snap threshold, so the old binary header-syncs too and the control run cannot reproduce the hole.
new_sandbox b1_gap499; canon 49501; call_fn gs024_pick_base "$TIP"
ck "B1 gap 499: rc != 0" '[[ "$RC" -ne 0 ]]'
ck "B1 gap 499: NOT READY names too young" 'reason_has "too young"'

# B2 — REQ-ARCHSYNC-004 — Decision: refusing a base at exactly gap 500 makes the scenario NOT READY on the youngest base that still crosses the snap boundary.
new_sandbox b2_gap500; canon 49500; call_fn gs024_pick_base "$TIP"
ck "B2 gap 500: picks the base, stdout is its path only" 'picked 49500'

# B3 — REQ-ARCHSYNC-004 — Decision: an exclusive upper bound drops the oldest admissible base one block early and shortens the pool's usable life.
new_sandbox b3_gap20000; canon 30000; call_fn gs024_pick_base "$TIP"
ck "B3 gap 20000: picks the base" 'picked 30000'

# B4 — REQ-ARCHSYNC-004 — Decision: a base past MAX_GAP turns the run into a multi-hour catch-up that times out and reports the fix as broken.
new_sandbox b4_gap20001; canon 29999; call_fn gs024_pick_base "$TIP"
ck "B4 gap 20001: rc != 0" '[[ "$RC" -ne 0 ]]'
ck "B4 gap 20001: NOT READY names too old" 'reason_has "too old"'

# B5 — REQ-ARCHSYNC-004 — Decision: a base from a fork or a reset chain starts the drill node off-chain, so gs024-commitment-matches-seed fails for a reason unrelated to the fix.
new_sandbox b5_noncanon; mk_base 49000; seed_has 49000 "$(hx 77)"; call_fn gs024_pick_base "$TIP"
ck "B5 hash != seed: rc != 0" '[[ "$RC" -ne 0 ]]'
ck "B5 hash != seed: NOT READY names not canonical" 'reason_has "not canonical"'

# B6 — REQ-ARCHSYNC-004 — Decision: an empty or absent pool must stop the run as NOT READY instead of launching from an empty data dir, which is a genesis sync, not this scenario.
new_sandbox b6_empty; call_fn gs024_pick_base "$TIP"
ck "B6a empty pool: rc != 0" '[[ "$RC" -ne 0 ]]'
ck "B6a empty pool: NOT READY names no base" 'reason_has "no base"'
new_sandbox b6_absent; POOL_DIR="$CASE_DIR/no-such-pool"; call_fn gs024_pick_base "$TIP"
ck "B6b absent pool: rc != 0 with NOT READY" '[[ "$RC" -ne 0 ]] && not_ready'

# B7 — REQ-ARCHSYNC-004 — Decision: the newest base starts barely over 500 behind; on a fast host header-first closes that gap before the 60 s lockout ends, so Rule 2 is never reachable (measured: gap 593 caught up in 80 s).
new_sandbox b7_oldest; canon 45000 48000 49800; call_fn gs024_pick_base "$TIP"
ck "B7 several bases: oldest valid one wins, too-young one skipped" 'picked 45000'

# B7b — REQ-ARCHSYNC-004 — Decision: an oldest-first pick that stops at a non-canonical base reports NOT READY while a canonical base exists.
new_sandbox b7b_oldest_noncanon; mk_base 45000; seed_has 45000 "$(hx 77)"; canon 48000; call_fn gs024_pick_base "$TIP"
ck "B7b oldest base not canonical: next oldest canonical one wins" 'picked 48000'

# B8 — REQ-ARCHSYNC-004 — Decision: a lexical sort ranks ckpt-h10200 before ckpt-h9800 once the height gains a digit, so the runner picks the younger base.
new_sandbox b8_numeric; canon 9800 10200; call_fn gs024_pick_base 11000
ck "B8 9800 vs 10200: numeric order picks the older 9800" 'picked 9800'

# B9 — REQ-ARCHSYNC-004 — Decision: a half-cloned base (no health.json, no hash) must be skipped; an empty hash compared with an absent seed block reads as canonical.
new_sandbox b9_broken
mkdir -p "$POOL_DIR/ckpt-h49000-$TS/state_db"
printf '{\n  "healthy": true,\n  "height": 49000\n}\n' > "$POOL_DIR/ckpt-h49000-$TS/health.json"
mkdir -p "$POOL_DIR/ckpt-h48000-$TS/blocks"
canon 47000
call_fn gs024_pick_base "$TIP"
ck "B9a broken dirs + valid 47000: picks 47000" 'picked 47000'
rm -rf "$(base_path 47000)"; call_fn gs024_pick_base "$TIP"
ck "B9b only broken dirs: rc != 0 with NOT READY" '[[ "$RC" -ne 0 ]] && not_ready'

# B10 — REQ-ARCHSYNC-004 — Decision: a base above the seed tip comes from another chain; an abs() or unsigned gap would accept it.
new_sandbox b10_above_tip; canon 50100; call_fn gs024_pick_base "$TIP"
ck "B10 base above tip: rc != 0 with NOT READY" '[[ "$RC" -ne 0 ]] && not_ready'

# B11 — REQ-ARCHSYNC-004 — Decision: a run dir or a non-ckpt snapshot picked as base starts the drill from a node that already ran, not from a pristine checkpoint.
new_sandbox b11_non_ckpt; mk_health_at "$POOL_DIR/runs/run-1" 49000; mk_health_at "$POOL_DIR/snapshot-h49000-$TS" 49000
seed_has 49000; call_fn gs024_pick_base "$TIP"
ck "B11 only non-ckpt dirs: rc != 0 with NOT READY" '[[ "$RC" -ne 0 ]] && not_ready'

# B12 — REQ-ARCHSYNC-004 — Decision: a canonical check that passes when the seed does not answer starts a drill whose base was never verified.
new_sandbox b12_seed_down; canon 49000; : > "$FIX_DIR/$SEED_PORT/down"; call_fn gs024_pick_base "$TIP"
ck "B12 seed down: rc != 0 with NOT READY" '[[ "$RC" -ne 0 ]] && not_ready'

# B13 — REQ-ARCHSYNC-004 — Decision: ignored env overrides make the runner check a seed and a threshold other than the ones the operator set.
new_sandbox b13_overrides; T_SEED_URL="http://127.0.0.1:$ALT_PORT"; T_MIN=100
mk_base 49850; mk_base 49960; seed_has 49850 "" "$ALT_PORT"; seed_has 49960 "" "$ALT_PORT"
call_fn gs024_pick_base "$TIP"
ck "B13 GS024_SEED_RPC + GS024_MIN_GAP=100: picks 49850 (gap 150) via the alternate seed" 'picked 49850'

# P1 — REQ-ARCHSYNC-004 — Decision: a preflight that refuses a clean environment makes GS-024 NOT READY on every run, so it never guards anything.
new_sandbox p1_clear; call_fn gs024_preflight
ck "P1 all clear: rc 0, no NOT READY" '[[ "$RC" -eq 0 ]] && ! not_ready'

# P2 — REQ-ARCHSYNC-004 — Decision: a busy port means another process (possibly a live node) owns it; launching anyway fails the bind or hits the wrong RPC.
for port in 30390 8590 9090; do
    new_sandbox "p2_busy_$port"; : > "$FIX_DIR/busy.$port"; call_fn gs024_preflight
    ck "P2 port $port busy: rc != 0 with NOT READY" '[[ "$RC" -ne 0 ]] && not_ready'
done

# P3 — REQ-ARCHSYNC-004 — Decision: a concurrent build of THIS repo starves the drill node of CPU and disk, and a slow catch-up reads as a fix failure.
RUSTC_HERE="$HOME/.rustup/toolchains/stable/bin/rustc --crate-name doli_core crates/core/src/lib.rs --out-dir $PROJECT_ROOT/target/release/deps"
build_case() {
    new_sandbox "$1"; printf '%s\n' "$2" > "$FIX_DIR/ps.txt"; call_fn gs024_preflight
}
build_case p3a_rustc_here "$RUSTC_HERE"
ck "P3a rustc with $PROJECT_ROOT/target in its args: rc != 0 with NOT READY build" '[[ "$RC" -ne 0 ]] && reason_has build'
build_case p3b_cargo_here "/opt/cargo/bin/cargo build --release --manifest-path $PROJECT_ROOT/Cargo.toml"
ck "P3b cargo --manifest-path $PROJECT_ROOT/Cargo.toml: rc != 0 with NOT READY build" '[[ "$RC" -ne 0 ]] && reason_has build'

# P3c — REQ-ARCHSYNC-004 — Decision: other sessions build other checkouts on this machine all day; counting them makes GS-024 NOT READY forever.
build_case p3c_other_repo "rustc --crate-name x src/lib.rs --out-dir /tmp/other-repo/target/debug/deps
cargo test --manifest-path /tmp/other-repo/Cargo.toml"
ck "P3c cargo/rustc of another repo: rc 0, no NOT READY" '[[ "$RC" -eq 0 ]] && ! not_ready'
build_case p3d_cargo_nopath "/opt/cargo/bin/cargo test --workspace --no-fail-fast"
ck "P3d cargo with no path in its args: rc 0, no NOT READY" '[[ "$RC" -eq 0 ]] && ! not_ready'

# P3e — REQ-ARCHSYNC-004 — Decision: a worktree path begins with the main checkout path, so a plain substring match counts worktree builds against the main checkout.
build_case p3e_worktree "rustc --crate-name y src/lib.rs --out-dir $PROJECT_ROOT/.claude/worktrees/other/target/release/deps"
ck "P3e rustc under $PROJECT_ROOT/.claude/worktrees: rc 0, no NOT READY" '[[ "$RC" -eq 0 ]] && ! not_ready'

# P3f — REQ-ARCHSYNC-004 — Decision: a running node or test binary under ROOT/target is not a build; counting it blocks the drill on its own node.
build_case p3f_not_build "$PROJECT_ROOT/target/release/doli-node --network testnet run
/bin/zsh -c cargo build --manifest-path $PROJECT_ROOT/Cargo.toml"
ck "P3f non-cargo/rustc processes naming $PROJECT_ROOT: rc 0, no NOT READY" '[[ "$RC" -eq 0 ]] && ! not_ready'

# P4 — REQ-ARCHSYNC-004 — Decision: without the ALLOW_BUILD escape the scenario can never run on a machine with a resident build.
new_sandbox p4_allow_build; T_ALLOW=1; printf '%s\n' "$RUSTC_HERE" > "$FIX_DIR/ps.txt"; call_fn gs024_preflight
ck "P4 GS024_ALLOW_BUILD=1 with a build of this repo: rc 0" '[[ "$RC" -eq 0 ]] && ! not_ready'

# P5 — REQ-ARCHSYNC-004 — Decision: an ALLOW_BUILD that also skips the port check lets the drill collide with a live node's port.
new_sandbox p5_allow_busy; T_ALLOW=1; : > "$FIX_DIR/busy.8590"; call_fn gs024_preflight
ck "P5 GS024_ALLOW_BUILD=1 with port 8590 busy: rc != 0 with NOT READY" '[[ "$RC" -ne 0 ]] && not_ready'

# P6 — REQ-ARCHSYNC-004 — Decision: without a live seed there is no tip, no canonical check and no commitment reference, so every assertion would be vacuous.
new_sandbox p6_seed_down; : > "$FIX_DIR/$SEED_PORT/down"; call_fn gs024_preflight
ck "P6 seed RPC down: rc != 0 with NOT READY" '[[ "$RC" -ne 0 ]] && not_ready'

# P7 — REQ-ARCHSYNC-004 — Decision: a missing or non-executable binary is found only after the pool clone, leaving a half-built run dir and a confusing launch failure.
new_sandbox p7_bin_missing; rm -f "$NODE_BIN"; call_fn gs024_preflight
ck "P7a GS024_BIN missing: rc != 0 with NOT READY" '[[ "$RC" -ne 0 ]] && not_ready'
new_sandbox p7_bin_noexec; chmod -x "$NODE_BIN"; call_fn gs024_preflight
ck "P7b GS024_BIN not executable: rc != 0 with NOT READY" '[[ "$RC" -ne 0 ]] && not_ready'
new_sandbox p7_bin_dir; rm -f "$NODE_BIN"; mkdir -p "$NODE_BIN"; call_fn gs024_preflight
ck "P7c GS024_BIN is a directory: rc != 0 with NOT READY" '[[ "$RC" -ne 0 ]] && not_ready'

# P8 — REQ-ARCHSYNC-004 — Decision: hard-coded ports check the defaults while the node binds the overrides, so a busy override port goes undetected.
new_sandbox p8_override_free; T_P2P=41390; T_RPC=48590; T_MET=49090; : > "$FIX_DIR/busy.8590"; call_fn gs024_preflight
ck "P8a overridden ports free, default 8590 busy: rc 0" '[[ "$RC" -eq 0 ]] && ! not_ready'
new_sandbox p8_override_busy; T_P2P=41390; T_RPC=48590; T_MET=49090; : > "$FIX_DIR/busy.48590"; call_fn gs024_preflight
ck "P8b overridden RPC port 48590 busy: rc != 0 with NOT READY" '[[ "$RC" -ne 0 ]] && not_ready'

# P9 — REQ-ARCHSYNC-004 — Decision: without the scanner interpreter gs024-no-missing-height cannot scan, and a missing scan must stop the run before the clone, not read as a hole.
new_sandbox p9_no_python; T_PY="$CASE_DIR/no-such-python3"; call_fn gs024_preflight
ck "P9 GS024_PYTHON missing: rc != 0 with NOT READY naming it" '[[ "$RC" -ne 0 ]] && reason_has "no-such-python3"'

# R1 — REQ-ARCHSYNC-004 — Decision: gs024-no-missing-height reports holes through this; a wrong range sends the operator to backfill heights that are not missing.
ranges_case() {
    new_sandbox "$1"; STDIN_FILE="$CASE_DIR/in"; WANT="$3"
    # shellcheck disable=SC2059  # $2 is a fixture escape sequence
    printf "$2" > "$STDIN_FILE"; call_fn gs024_ranges
    ck "$1: $2 -> $3" '[[ "$OUT" == "$WANT" ]]' "$(det) want=[$WANT]"
}
ranges_case R1_mixed '5\n6\n7\n10\n' '5..7,10..10'
ranges_case R2_empty '' 'none'
ranges_case R3_single '42\n' '42..42'
ranges_case R4_one_run '1\n2\n3\n4\n5\n' '1..5'
ranges_case R5_no_runs '2\n4\n6\n' '2..2,4..4,6..6'
ranges_case R6_no_final_newline '269097\n269098\n270000' '269097..269098,270000..270000'

# X1 — REQ-ARCHSYNC-004 — Decision: a prune that deletes a young base, a run dir, an unreadable clone or anything outside the pool destroys the only reusable drill base or unrelated data.
new_sandbox x1_prune
canon 29999 30000 49000 49900
mkdir -p "$POOL_DIR/ckpt-h1000-noh/blocks"
mk_health_at "$POOL_DIR/keep-me" 1
mkdir -p "$POOL_DIR/runs/run-1"; echo m > "$POOL_DIR/runs/run-1/marker"
mk_health_at "$CWD_DIR/ckpt-h100-$TS" 100
call_fn gs024_prune_pool "$TIP"
ck "X1 gap 20001 base removed" '[[ ! -e "$(base_path 29999)" ]]'
ck "X1 gap 20000 base kept" '[[ -f "$(base_path 30000)/health.json" ]]'
ck "X1 young bases 49000/49900 kept" '[[ -f "$(base_path 49000)/state_db/000001.sst" && -f "$(base_path 49900)/health.json" ]]'
ck "X1 ckpt dir without health.json kept" '[[ -d "$POOL_DIR/ckpt-h1000-noh/blocks" ]]'
ck "X1 non-ckpt sibling dir kept" '[[ -f "$POOL_DIR/keep-me/health.json" ]]'
ck "X1 runs/ dir and contents kept" '[[ -f "$POOL_DIR/runs/run-1/marker" ]]'
ck "X1 old ckpt dir in the cwd (outside the pool) kept" '[[ -f "$CWD_DIR/ckpt-h100-$TS/health.json" ]]'

# X2 — REQ-ARCHSYNC-004 — Decision: an ignored GS024_MAX_GAP prunes against 20000 while pick_base admits against the override, so the two disagree on which bases exist.
new_sandbox x2_prune_max; T_MAX=1000; canon 48999 49000; call_fn gs024_prune_pool "$TIP"
ck "X2 GS024_MAX_GAP=1000: gap 1001 removed" '[[ ! -e "$(base_path 48999)" ]]'
ck "X2 GS024_MAX_GAP=1000: gap 1000 kept" '[[ -f "$(base_path 49000)/health.json" ]]'

# X3 — REQ-ARCHSYNC-004 — Decision: a seed that stops answering yields an empty tip, and a prune that treats it as huge wipes every base in the pool.
new_sandbox x3_prune_no_tip; canon 29999 1000; call_fn gs024_prune_pool ""
ck "X3 empty TIP: nothing removed" '[[ -d "$(base_path 29999)" && -d "$(base_path 1000)" ]]'

# M1 — REQ-ARCHSYNC-004 — Decision: gauntlet-run.sh scores exit 0 as PASS, so a runner that exits 0 without running certifies the fix with no evidence.
main_checks() {
    ck "$1: exit code 3" '[[ "$RC" -eq 3 ]]'
    ck "$1: prints a NOT READY: line" 'not_ready'
    ck "$1: drill node never launched, no pkill/launchctl" '! launched && ! procs_touched && not_timed_out' \
       "bin=$(tr '\n' ' ' < "$BIN_LOG") mutate=$(tr '\n' ' ' < "$MUTATE_LOG") timedout=$([ -f "$CASE_DIR/timedout" ] && echo yes || echo no)"
}
new_sandbox m1_young; canon 49800; run_main
main_checks "M1 main, only a too-young base"
ck "M1 main: the base is not deleted" '[[ -f "$(base_path 49800)/state_db/000001.sst" ]]'

# M2 — REQ-ARCHSYNC-004 — Decision: an empty pool must be NOT READY, never a launch from an empty data dir.
new_sandbox m2_empty; run_main
main_checks "M2 main, empty pool"

# M3 — REQ-ARCHSYNC-004 — Decision: main that skips preflight launches onto a port another node owns, even with a valid base.
new_sandbox m3_busy; canon 49000; : > "$FIX_DIR/busy.30390"; run_main
main_checks "M3 main, valid base + port 30390 busy"
ck "M3 main: the base is not deleted" '[[ -f "$(base_path 49000)/state_db/000001.sst" ]]'

# M4 — REQ-ARCHSYNC-004 — Decision: main that skips the build check measures catch-up under a cargo build and reports CPU starvation as a fix failure.
new_sandbox m4_build; canon 49000; printf '%s\n' "$RUSTC_HERE" > "$FIX_DIR/ps.txt"; run_main
main_checks "M4 main, valid base + cargo running"

# S1 — REQ-ARCHSYNC-004 — Decision: pkill, killall or launchctl stop live testnet nodes, and launchd respawns them into a chain split (CLAUDE.md never-pkill rule).
sck "S1 static: no pkill / killall / launchctl" \
    '! code_lines | grep -qE "(^|[^A-Za-z0-9_-])(pkill|killall|launchctl)([^A-Za-z0-9_-]|\$)"' \
    "$(code_lines | grep -nE '(^|[^A-Za-z0-9_-])(pkill|killall|launchctl)([^A-Za-z0-9_-]|$)' | head -3)"

# S2 — REQ-ARCHSYNC-004 — Decision: a kill of any PID but the runner's own drill node can stop a live seed or producer.
sck "S2 static: every kill targets only \$PID" '[[ -z "$(bad_kills)" ]]' "$(bad_kills | head -3 | tr '\n' '|')"

# S3 — REQ-ARCHSYNC-004 — Decision: an rm -r of the pool root, \$HOME, a seed dir, a glob or an unquoted variable wipes data the runner must never touch.
sck "S3 static: rm -r targets are quoted, glob-free, not pool/HOME/seed roots" '[[ -z "$(bad_rms)" ]]' "$(bad_rms | head -3 | tr '\n' '|')"

# S4 — REQ-ARCHSYNC-004 — Decision: a copied node_key gives the drill node the seed's peer ID, and peers then drop or misroute the real seed.
sck "S4 static: node_key is never copied" \
    '! code_lines | grep "node_key" | grep -qE "(^|[^A-Za-z0-9_-])(cp|rsync|ditto|ln|mv|install)[[:space:]]"' \
    "$(code_lines | grep -n node_key | head -3)"

# S5 — REQ-ARCHSYNC-004 — Decision: a hard-coded home path ignores HOME and GS024_POOL, so prune and pick act on the operator's real pool from any sandbox.
sck "S5 static: no hard-coded /Users/ or /home/ path (runner, lib, scanner)" \
    '! { code_lines; cat "$SCAN" 2>/dev/null; } | grep -qE "/Users/|/home/"' \
    "$({ code_lines; cat "$SCAN" 2>/dev/null; } | grep -nE '/Users/|/home/' | head -3)"

# S6 — REQ-ARCHSYNC-004 — Decision: a --producer drill instance signs blocks beside a live producer and can be slashed for equivocation.
sck "S6 static: drill node is never started with --producer" '! code_lines | grep -qE -- "--producer"' \
    "$(code_lines | grep -nE -- '--producer' | head -3)"

# S7 — REQ-ARCHSYNC-004 — Decision: an explicit --no-snap-sync hides a regression of the archive-role no-snap rule, so GS-024 would also pass on the 9eda87b4 binary.
sck "S7 static: drill node is never started with --no-snap-sync" '! code_lines | grep -qE -- "--no-snap-sync"' \
    "$(code_lines | grep -nE -- '--no-snap-sync' | head -3)"

# S8 — REQ-ARCHSYNC-004 — Decision: a token the runner never evaluates leaves that failure mode (the archive hole itself for no-missing-height) unchecked while the scenario reports PASS.
for tok in $TOKENS; do
    sck "S8 static: runner evaluates $tok" 'code_lines | grep -q -- "$tok"' "token $tok absent from $LIB"
done

# S9 — REQ-ARCHSYNC-004 — Decision: gauntlet-run.sh executes the runner directly, so a syntax error turns GS-024 into a permanent FAIL unrelated to the fix.
sck "S9 hygiene: runner and lib parse under bash -n" 'bash -n "$LIB" 2>/dev/null && bash -n "$HLIB" 2>/dev/null' \
    "$(bash -n "$LIB" 2>&1 | head -2) $(bash -n "$HLIB" 2>&1 | head -2)"

# S11 — REQ-ARCHSYNC-004 — Decision: a scanner that does not parse fails every hole scan, so gs024-no-missing-height fails for a reason unrelated to the fix.
sck "S11 hygiene: scripts/gauntlet-gs024-scan.py parses" \
    'python3 -I -c "import ast, sys; ast.parse(open(sys.argv[1]).read())" "$SCAN" 2>/dev/null' "scanner $SCAN missing or does not parse"

# S12 — REQ-ARCHSYNC-004 — Decision: a file over the module budget is blocked by the size gate and hides logic reviewers cannot read in one pass.
sck "S12 hygiene: runner, lib and scanner are <= 500 lines each" \
    '[[ -f "$SCAN" && "$(cat "$LIB" | wc -l)" -le 500 && "$(cat "$HLIB" | wc -l)" -le 500 && "$(cat "$SCAN" | wc -l)" -le 500 ]]' \
    "lines: $(wc -l "$LIB" "$HLIB" "$SCAN" 2>&1 | tr '\n' ' ')"

# GT1-GT3 — REQ-ARCHSYNC-004 — Decision: gauntlet.sh reads every active scenario row; a standalone token it cannot evaluate turns into "unknown assertion token" and fails every gauntlet.sh run.
GAUNTLET="$PROJECT_ROOT/scripts/gauntlet.sh"
GS023_TOKENS="gs023-rpc-30s gs023-rpc-10s gs023-second-call-1s gs023-no-failed-probe gs023-no-respawn gs023-reaches-tip gs023-clean-shutdown"
ga_assert() {
    ( SKIP_REASONS=""; FAIL_REASONS=""
      eval "$(awk '/^assert\(\)\{/{f=1} f{print} f&&/^}/{exit}' "$GAUNTLET")" 2>/dev/null || exit 99
      assert "$1"; rc=$?
      printf '%s|%s|%s\n' "$rc" "$SKIP_REASONS" "$FAIL_REASONS" )
}
for tok in $TOKENS $GS023_TOKENS; do
    GA="$(ga_assert "$tok")"; run="scripts/gauntlet-${tok%%-*}.sh"; gt=GT1; [[ "$tok" == gs023-* ]] && gt=GT2
    sck "$gt gauntlet.sh assert() SKIPs standalone $tok (rc 2, names $run)" \
        '[[ "${GA%%|*}" == 2 && "$GA" == *"$tok: standalone scenario, run $run (not part of gauntlet.sh)"* && "$GA" == *"|" ]]' \
        "assert $tok -> [$GA]"
done
GA="$(ga_assert gs024-no-such-token)"
sck "GT3 gauntlet.sh assert() still FAILs an unknown gs024-like token" \
    '[[ "${GA%%|*}" == 1 && "$GA" == *"unknown assertion token"* ]]' "assert gs024-no-such-token -> [$GA]"

# L2 —REQ-ARCHSYNC-004 — Decision: without gs024_throttle and gs024_irpc the throttled control run cannot be driven or unit-tested.
LOAD_FNS="gs024_throttle gs024_irpc"; new_sandbox l2_lib_only_new; bounded _load_body; LOAD_FNS=""
NFN="$(cat "$CASE_DIR/nfn" 2>/dev/null || echo 0)"; DEFINED=yes; RC=0
ck "L2 lib_only: gs024_throttle and gs024_irpc defined" '[[ "$NFN" -eq 2 ]]' "defined=$NFN/2 lib=$LIB"

# T1 — REQ-ARCHSYNC-004 — Decision: the throttle is the slow-host model; a default-off throttle makes a plain `gauntlet-gs024.sh` run prove nothing at gaps under ~1,200.
new_sandbox t1_unset; call_fn gs024_throttle on "$DUMMY_PID"
ck "T1 GS024_THROTTLE unset (default on), on: rc 0, one call '-b -p PID'" \
   '[[ "$RC" -eq 0 && "$(tp_calls)" == "taskpolicy -b -p $DUMMY_PID" ]]' "$(det) tp=$(tp_calls | tr '\n' '|')"

# T2 — REQ-ARCHSYNC-004 — Decision: a non-empty test instead of == 1 throttles when the operator set GS024_THROTTLE=0.
new_sandbox t2_zero; T_THR=0; call_fn gs024_throttle on "$DUMMY_PID"
ck "T2 GS024_THROTTLE=0, on: rc 0, taskpolicy never called" '[[ "$RC" -eq 0 ]] && no_tp' "$(det) tp=$(tp_calls | tr '\n' '|')"

# T3 — REQ-ARCHSYNC-004 — Decision: without -b -p on the node PID the old binary syncs fast, never snaps, and the control run passes.
new_sandbox t3_on; T_THR=1; call_fn gs024_throttle on "$DUMMY_PID"
ck "T3 GS024_THROTTLE=1, on: rc 0, one GS024_TASKPOLICY call '-b -p PID'" \
   '[[ "$RC" -eq 0 && "$(tp_calls)" == "taskpolicy -b -p $DUMMY_PID" ]] && ! grep -q "^taskpolicy " "$MUTATE_LOG"' \
   "$(det) tp=$(tp_calls | tr '\n' '|') path_taskpolicy=$(grep -c '^taskpolicy ' "$MUTATE_LOG")"

# T4 — REQ-ARCHSYNC-004 — Decision: an off that does not send -B leaves the node throttled through RPC checks and stop, the measured timeout and SIGKILL defects.
new_sandbox t4_off; T_THR=1; call_fn gs024_throttle off "$DUMMY_PID"
ck "T4 GS024_THROTTLE=1, off: rc 0, one GS024_TASKPOLICY call '-B -p PID'" \
   '[[ "$RC" -eq 0 && "$(tp_calls)" == "taskpolicy -B -p $DUMMY_PID" ]]' "$(det) tp=$(tp_calls | tr '\n' '|')"

# T5 — REQ-ARCHSYNC-004 — Decision: an empty PID turns the call into "taskpolicy -b -p" with no target, a silent no-throttle or a usage error.
for mode in on off; do
    new_sandbox "t5_empty_$mode"; T_THR=1; call_fn gs024_throttle "$mode" ""
    ck "T5 GS024_THROTTLE=1, $mode with empty PID: rc != 0, no taskpolicy call" '[[ "$RC" -ne 0 ]] && no_tp' "$(det) tp=$(tp_calls | tr '\n' '|')"
done

# T6 — REQ-ARCHSYNC-004 — Decision: a non-numeric PID reaches taskpolicy as a flag or a word list and throttles a process the runner does not own.
i=0
for bad in abc 12a -1 '1 2' '$(id)'; do
    i=$((i + 1)); new_sandbox "t6_pid_$i"; T_THR=1; call_fn gs024_throttle on "$bad"
    ck "T6 GS024_THROTTLE=1, on with PID '$bad': rc != 0, no taskpolicy call" '[[ "$RC" -ne 0 ]] && no_tp' "$(det) tp=$(tp_calls | tr '\n' '|')"
done

# S10 — REQ-ARCHSYNC-004 — Decision: a bare or other default taskpolicy path breaks the throttle on a PATH without /usr/sbin and ignores the test override.
sck "S10 static: taskpolicy resolves as \${GS024_TASKPOLICY:-/usr/sbin/taskpolicy}" \
    'code_lines | grep -qF "GS024_TASKPOLICY:-/usr/sbin/taskpolicy"' "GS024_TASKPOLICY:-/usr/sbin/taskpolicy absent from $LIB"

irpc_case() { new_sandbox "$1"; fast_sleep; mkdir -p "$FIX_DIR/$ALT_PORT"; echo 777 > "$FIX_DIR/$ALT_PORT/tip"; }
IURL="http://127.0.0.1:$ALT_PORT"

# I1 — REQ-ARCHSYNC-004 — Decision: a wrapper that retries after a good reply multiplies RPC load on a throttled node.
irpc_case i1_first; call_fn gs024_irpc "$IURL" getChainInfo '[]'
ck "I1 first attempt answers: rc 0, raw reply of URL" '[[ "$RC" -eq 0 && "$OUT" == *"\"bestHeight\":777"* ]]'
ck "I1 first attempt answers: exactly one curl call" '[[ "$(ncurl "$ALT_PORT")" -eq 1 ]]' "$(det) calls=$(ncurl "$ALT_PORT")"

# I2 — REQ-ARCHSYNC-004 — Decision: two transient timeouts on the throttled instance produced "no common height"; a retry must absorb them.
irpc_case i2_fail2; echo 2 > "$FIX_DIR/$ALT_PORT/failn"; call_fn gs024_irpc "$IURL" getChainInfo '[]'
ck "I2 two timeouts then a reply: rc 0, one reply, 3 curl calls" \
   '[[ "$RC" -eq 0 && "$(printf "%s\n" "$OUT" | grep -c bestHeight)" -eq 1 && "$(ncurl "$ALT_PORT")" -eq 3 ]]' "$(det) calls=$(ncurl "$ALT_PORT")"

# I3 — REQ-ARCHSYNC-004 — Decision: an empty reply with curl rc 0 taken as success returns no height, the same false FAIL.
irpc_case i3_empty2; echo 2 > "$FIX_DIR/$ALT_PORT/emptyn"; call_fn gs024_irpc "$IURL" getChainInfo '[]'
ck "I3 two empty rc-0 replies then a reply: rc 0, reply, 3 curl calls" \
   '[[ "$RC" -eq 0 && "$OUT" == *"\"bestHeight\":777"* && "$(ncurl "$ALT_PORT")" -eq 3 ]]' "$(det) calls=$(ncurl "$ALT_PORT")"

# I4 — REQ-ARCHSYNC-004 — Decision: a wrapper that loops forever or returns rc 0 on a dead instance hides a crashed node.
irpc_case i4_down; : > "$FIX_DIR/$ALT_PORT/down"; call_fn gs024_irpc "$IURL" getChainInfo '[]'
ck "I4 instance down: rc != 0, empty stdout, 5 curl calls" \
   '[[ "$RC" -ne 0 && -z "$OUT" && "$(ncurl "$ALT_PORT")" -eq 5 ]] && not_timed_out' "$(det) calls=$(ncurl "$ALT_PORT")"

# I6 — REQ-ARCHSYNC-004 — Decision: the 10 s seed default on the throttled instance is the measured timeout defect.
ck "I6 default: every attempt carries curl max time 30" \
   '[[ "$(ncurl "$ALT_PORT")" -ge 1 && -z "$(mt_bad "$ALT_PORT" ne 30)" ]]' "bad=[$(mt_bad "$ALT_PORT" ne 30 | head -2 | tr '\n' '|')]"

# I5 — REQ-ARCHSYNC-004 — Decision: an ignored GS024_RPC_TRIES makes a known-dead instance cost the default attempts.
irpc_case i5_tries2; T_TRIES=2; : > "$FIX_DIR/$ALT_PORT/down"; call_fn gs024_irpc "$IURL" getChainInfo '[]'
ck "I5 GS024_RPC_TRIES=2, down: rc != 0, exactly 2 curl calls" '[[ "$RC" -ne 0 && "$(ncurl "$ALT_PORT")" -eq 2 ]]' "$(det) calls=$(ncurl "$ALT_PORT")"

# I7 — REQ-ARCHSYNC-004 — Decision: an ignored GS024_RPC_TIMEOUT leaves the operator no knob when a throttle is harsher than 30 s.
irpc_case i7_timeout7; T_RTO=7; echo 1 > "$FIX_DIR/$ALT_PORT/failn"; call_fn gs024_irpc "$IURL" getChainInfo '[]'
ck "I7 GS024_RPC_TIMEOUT=7: rc 0, every attempt carries max time 7" \
   '[[ "$RC" -eq 0 && "$(ncurl "$ALT_PORT")" -eq 2 && -z "$(mt_bad "$ALT_PORT" ne 7)" ]]' "$(det) bad=[$(mt_bad "$ALT_PORT" ne 7 | head -2 | tr '\n' '|')]"

# I8 — REQ-ARCHSYNC-004 — Decision: a wrapper that drops METHOD or PARAMS returns another block, so the hole scan reads the wrong height.
new_sandbox i8_forward; fast_sleep; seed_has 49000
call_fn gs024_irpc "http://127.0.0.1:$SEED_PORT" getBlockByHeight '{"height":49000}'
ck "I8 forwards URL, METHOD and PARAMS" '[[ "$RC" -eq 0 && "$OUT" == *"$(hx 49000)"* ]]'

# I9 — REQ-ARCHSYNC-004 — Decision: an off-by-one attempt loop drops the last try and fails a node that answers on attempt 5.
irpc_case i9_fail4; echo 4 > "$FIX_DIR/$ALT_PORT/failn"; call_fn gs024_irpc "$IURL" getChainInfo '[]'
ck "I9 four timeouts then a reply: rc 0 on attempt 5" '[[ "$RC" -eq 0 && "$(ncurl "$ALT_PORT")" -eq 5 ]]' "$(det) calls=$(ncurl "$ALT_PORT")"

# G1 — REQ-ARCHSYNC-004 — Decision: the 60 s grace force-killed the throttled node, so gs024-clean-exit failed on a healthy shutdown.
sck "G1 static: SIGTERM grace defaults to \${GS024_STOP_GRACE:-180}" \
    'code_lines | grep -qE "GS024_STOP_GRACE:-180([^0-9]|\$)"' "GS024_STOP_GRACE:-180 absent from $LIB"

# G2 — REQ-ARCHSYNC-004 — Decision: a stop routine that ignores GS024_STOP_GRACE cannot be tuned, and a hung node keeps the run blocked.
new_sandbox g2_grace; T_GRACE=1; R="$CASE_DIR/runs/run-1"
bash -c 'trap "" TERM; n=0; while [ "$n" -lt 600 ]; do /bin/sleep 0.1; n=$((n + 1)); done' "$R" &
HOLD=$!; echo "$HOLD" > "$CASE_DIR/hold.pid"
# shellcheck disable=SC2034  # read by the sourced _gs024_stop
PID="$HOLD"; /bin/sleep 0.3
call_fn _gs024_stop
ck "G2 GS024_STOP_GRACE=1, TERM ignored: SIGKILL after the grace, rc 2" '[[ "$RC" -eq 2 ]] && not_timed_out' \
   "$(det) timedout=$([ -f "$CASE_DIR/timedout" ] && echo yes || echo no)"
ck "G2 GS024_STOP_GRACE=1: the instance is gone after stop" '[[ -z "$(ps -p "$HOLD" -o stat= 2>/dev/null | tr -d " Z")" ]]'
reap; wait "$HOLD" 2>/dev/null; unset PID R

# MT0 — REQ-ARCHSYNC-004 — Decision: a run without the throttle must say so in RESULT and never touch taskpolicy, or control and fix runs are not comparable.
run_sandbox mt0_plain 50000; T_THR=0; echo "2 3" > "$FIX_DIR/8590/failat"; run_full
ck "MT0 unthrottled: RESULT carries throttle=0, taskpolicy never called" 'result_has throttle=0 && no_tp && not_timed_out' "$(mt_det)"

# MT0b — REQ-ARCHSYNC-004 — Decision: instance calls 2-3 timing out reproduce "FAIL gs024-no-missing-height: no common height".
ck "MT0 instance calls 2-3 time out: common height H still computed" \
   '! printf "%s\n%s\n" "$OUT" "$ERR" | grep -q "no common height" && printf "%s\n" "$OUT" | grep -qE "H=[0-9]+ complete="' \
   "$(mt_det) holes=[$(printf '%s\n' "$OUT" | grep -m 1 'gs024-no-missing-height')]"

# MT0c — REQ-ARCHSYNC-004 — Decision: any instance call under 30 s max time is the throttled-RPC timeout defect.
ck "MT0 every instance RPC call carries max time >= 30" \
   '[[ "$(ncurl 8590)" -ge 1 && -z "$(mt_bad 8590 lt 30)" ]]' "bad=[$(mt_bad 8590 lt 30 | head -2 | tr '\n' '|')]"

# MT1 — REQ-ARCHSYNC-004 — Decision: without throttle=1 in RESULT a throttled control PASS is indistinguishable from an unthrottled one.
run_sandbox mt1_throttled 50000; T_THR=1; run_full
ck "MT1 throttled: RESULT carries throttle=1" 'result_has throttle=1 && not_timed_out' "$(mt_det)"

# MT1b — REQ-ARCHSYNC-004 — Decision: a throttle on any PID but the launched node slows another process and leaves the drill node fast.
ck "MT1 throttled: exactly '-b -p NODE' then '-B -p NODE', none via PATH" \
   '[[ -n "$NPID" && "$(tp_calls | tr "\n" "|")" == "taskpolicy -b -p $NPID|taskpolicy -B -p $NPID|" ]] && ! grep -q "^taskpolicy " "$MUTATE_LOG"' "$(mt_det)"

# MT1c — REQ-ARCHSYNC-004 — Decision: throttling before the RPC answers stretches the archiver catch-up from minutes to 15 min.
ck "MT1 throttled: -b only after the instance answered getChainInfo" \
   'before "^curl .*127\.0\.0\.1:8590.*getChainInfo" "^taskpolicy -b -p "' "$(mt_det)"

# MT1d — REQ-ARCHSYNC-004 — Decision: integrity checks and the stop under throttle are the measured RPC-timeout and SIGKILL defects.
ck "MT1 throttled: -B before verifyChainIntegrity and before the stop" \
   'before "^taskpolicy -B -p " "verifyChainIntegrity" && before "^taskpolicy -B -p " "^node TERM"' "$(mt_det)"

# MT2 — REQ-ARCHSYNC-004 — Decision: an unthrottle only on the tip-reached branch leaves a timed-out node throttled for every check.
run_sandbox mt2_timeout 49985; T_THR=1; T_TIMEOUT=1; run_full
ck "MT2 catch-up timeout: '-b -p NODE' then '-B -p NODE'" \
   '[[ -n "$NPID" && "$(tp_calls | tr "\n" "|")" == "taskpolicy -b -p $NPID|taskpolicy -B -p $NPID|" ]] && not_timed_out' "$(mt_det)"
ck "MT2 catch-up timeout: -B before verifyChainIntegrity and before the stop" \
   'before "^taskpolicy -B -p " "verifyChainIntegrity" && before "^taskpolicy -B -p " "^node TERM"' "$(mt_det)"

# W1 — REQ-ARCHSYNC-004 — Decision: the throttled 9eda87b4 control snapped ~450 s after its RPC answered; a shorter default window unthrottles first and the control stops reproducing H-1.
sck "W1 static: throttle window defaults to \${GS024_THROTTLE_WINDOW:-600}" \
    'code_lines | grep -qE "GS024_THROTTLE_WINDOW:-600([^0-9]|\$)"' "GS024_THROTTLE_WINDOW:-600 absent from $LIB / $HLIB"

# GH1 — REQ-ARCHSYNC-004 — Decision: without a held gap the run never reached Rule 2, so a no-snap PASS proves nothing about the fix.
run_sandbox gh1_held 49980; T_THR=1; T_WIN=2; T_TP=tpcatch; run_full
tp_span() {
    local b B
    b="$(awk '$2 == "-b" { print $1; exit }' "$FIX_DIR/tp.time" 2>/dev/null)"
    B="$(awk '$2 == "-B" { print $1; exit }' "$FIX_DIR/tp.time" 2>/dev/null)"
    [[ -n "$b" && -n "$B" ]] && echo $((B - b))
}
ck "GH1 20 behind at the window end (MIN_GAP 10): PASS gs024-gap-held" \
   'fail_line gs024-gap-held | grep -qE "^ *PASS "' "$(mt_det) line=[$(fail_line gs024-gap-held)]"
ck "GH1 RESULT carries window_gap=20 and throttle=1" 'result_has window_gap=20 && result_has throttle=1' "$(mt_det)"
ck "GH1 -B comes no earlier than the 2 s window after -b" '[[ "$(tp_span)" -ge 2 ]]' "span=$(tp_span) $(mt_det)"
ck "GH1 catches up after -B: every assertion PASS, exit 0" \
   '[[ "$RC" -eq 0 ]] && out_has "^GS024 RESULT: PASS " && ! out_has "^ *FAIL "' "$(mt_det) fails=[$(printf '%s\n' "$OUT" | grep -E '^ *FAIL' | tr '\n' '|')]"

# FS1 — REQ-ARCHSYNC-004 — Decision: a scan of the seed instead of the instance, or from the wrong base, never sees the hole the snap leaves above the base.
ck "FS1 default scan: instance URL, lo = base+1 = 49981, hi = H = 49998" \
   'grep -qE "^pyscan .*http://127\.0\.0\.1:8590 49981 49998\$" "$CURL_LOG"' "scan=[$(grep '^pyscan' "$CURL_LOG" | tr '\n' '|')]"

# GH2 — REQ-ARCHSYNC-004 — Decision: an instance that reaches the tip inside the window closed the gap before Rule 2 could fire; reporting PASS certifies an unexercised path.
run_sandbox gh2_at_tip 50000; T_THR=1; run_full
ck "GH2 at tip inside the window: FAIL gs024-gap-held, RESULT FAIL, exit 1" \
   '[[ "$RC" -eq 1 ]] && fail_line gs024-gap-held | grep -qE "^ *FAIL " && out_has "^GS024 RESULT: FAIL "' "$(mt_det)"

# GH3 — REQ-ARCHSYNC-004 — Decision: on the old binary the snap fires inside the window and closes the gap; judging the gap only at the window end calls the H-1 reproduction "Rule 2 not reached".
run_sandbox gh3_snap 49980; T_THR=1; T_WIN=2; T_TP=tpcatch
echo "INFO network::sync::manager::block_lifecycle: [COORDINATOR] action=SnapSync gap=20 peers=7" > "$FIX_DIR/snapline"; run_full
ck "GH3 snap ordered 20 behind: PASS gs024-gap-held end=snap_ordered window_gap=20" \
   'fail_line gs024-gap-held | grep -qE "^ *PASS " && out_has "end=snap_ordered" && result_has window_gap=20' "$(mt_det) line=[$(fail_line gs024-gap-held)]"
ck "GH3 snap ordered: FAIL gs024-no-snap, RESULT FAIL, exit 1" \
   '[[ "$RC" -eq 1 ]] && fail_line gs024-no-snap | grep -qE "^ *FAIL " && out_has "^GS024 RESULT: FAIL "' "$(mt_det)"

# GH4 — REQ-ARCHSYNC-004 — Decision: measured, the fixed binary logged "[COORDINATOR] action=HeaderFirstSync gap=1238" and then caught up inside the window while throttled; the coordinator line is the direct proof that Rule 2 was evaluated, so window-end timing alone fails a correct fix.
run_sandbox gh4_coord 50000; T_THR=1
echo "INFO network::sync::manager::block_lifecycle: [COORDINATOR] action=HeaderFirstSync gap=20 last_applied=0s peers=7" > "$FIX_DIR/snapline"; run_full
ck "GH4 at tip inside the window, coordinator decided at gap 20 >= 10: PASS gs024-gap-held end=coordinator" \
   'fail_line gs024-gap-held | grep -qE "^ *PASS " && out_has "end=coordinator" && out_has "coordinator=HeaderFirstSync:20"' "$(mt_det) line=[$(fail_line gs024-gap-held)]"
ck "GH4 HeaderFirstSync is not a snap: PASS gs024-no-snap, exit 0, post_window_s=none" \
   '[[ "$RC" -eq 0 ]] && fail_line gs024-no-snap | grep -qE "^ *PASS " && result_has post_window_s=none' "$(mt_det)"

# GH5 — REQ-ARCHSYNC-004 — Decision: a coordinator decision below MIN_GAP is the medium-gap path; counting it as Rule 2 reach repeats the unthrottled gap-593 false PASS.
run_sandbox gh5_coord_small 50000; T_THR=1
echo "INFO network::sync::manager::block_lifecycle: [COORDINATOR] action=HeaderFirstSync gap=9 last_applied=0s peers=7" > "$FIX_DIR/snapline"; run_full
ck "GH5 coordinator decided at gap 9 < 10, at tip inside the window: FAIL gs024-gap-held" \
   '[[ "$RC" -eq 1 ]] && fail_line gs024-gap-held | grep -qE "^ *FAIL "' "$(mt_det) line=[$(fail_line gs024-gap-held)]"

# GH6 — REQ-ARCHSYNC-004 — Decision: measured, a fixed binary was ~1,750 behind when its post-restart lockout ended, logged no coordinator line and was 594 behind at the window end; judging only the window end makes a correct run pass or fail on host speed.
run_sandbox gh6_post_lockout 49980; T_THR=1; T_WIN=6; T_LOCK=1; T_TP=tpcatch; echo "25 49995" > "$FIX_DIR/tip.later"; run_full
ck "GH6 post-lockout gap 20 >= 10, window-end gap 5, no coordinator line: PASS gs024-gap-held post_lockout_gap=20" \
   'fail_line gs024-gap-held | grep -qE "^ *PASS " && out_has "post_lockout_gap=20( |\$)" && result_has window_gap=5' "$(mt_det) line=[$(fail_line gs024-gap-held)] detail=[$(printf '%s\n' "$OUT" | grep -m 1 -A 1 'gs024-gap-held' | tail -n 1)]"
ck "GH6 no snap, catches up after -B: RESULT PASS, exit 0" '[[ "$RC" -eq 0 ]] && out_has "^GS024 RESULT: PASS "' "$(mt_det)"

# GH7 — REQ-ARCHSYNC-004 — Decision: a gap under MIN_GAP when the lockout ends is the medium-gap path; Rule 2 was never reachable.
run_sandbox gh7_post_lockout_small 49995; T_THR=1; T_WIN=3; T_LOCK=1; T_TP=tpcatch; run_full
ck "GH7 post-lockout gap 5 < 10, window-end gap 5, no coordinator line: FAIL gs024-gap-held post_lockout_gap=5" \
   '[[ "$RC" -eq 1 ]] && fail_line gs024-gap-held | grep -qE "^ *FAIL .*post_lockout_gap=5( |\$)"' "$(mt_det) line=[$(fail_line gs024-gap-held)]"

# GH8 — REQ-ARCHSYNC-004 — Decision: a gap sampled before the lockout ends was never seen by the coordinator; an instance at the tip by then never reached Rule 2.
run_sandbox gh8_tip_before_lockout 49980; T_THR=1; T_LOCK=3; echo "5 50000" > "$FIX_DIR/tip.later"; run_full
ck "GH8 20 behind at RPC-up, at tip before the post-lockout sample: FAIL gs024-gap-held post_lockout_gap=none" \
   '[[ "$RC" -eq 1 ]] && fail_line gs024-gap-held | grep -qE "^ *FAIL .*post_lockout_gap=none( |\$)"' "$(mt_det) line=[$(fail_line gs024-gap-held)]"

# GH9 — REQ-ARCHSYNC-004 — Decision: the node holds a 60 s post-restart lockout and ticks every ~30 s, so Rule 2 is first evaluated by ~90 s; an earlier default samples a gap the coordinator never saw.
sck "GH9 static: post-lockout sample defaults to \${GS024_LOCKOUT_EVAL:-90}, named in the runner Env header" \
    'code_lines | grep -qE "GS024_LOCKOUT_EVAL:-90([^0-9]|\$)" && head -n 16 "$LIB" | grep -qE "^#.*GS024_LOCKOUT_EVAL"' "GS024_LOCKOUT_EVAL:-90 absent from $LIB / $HLIB or from the Env header"

# FS2 — REQ-ARCHSYNC-004 — Decision: holes below the base are invisible to a base+1 scan; the drills need genesis..H.
run_sandbox fs2_full 50000; T_THR=0; T_FULL=1; run_full
ck "FS2 GS024_FULL_SCAN=1: scanner lo = 1, hi = H = 49998" \
   'grep -qE "^pyscan .*http://127\.0\.0\.1:8590 1 49998\$" "$CURL_LOG"' "scan=[$(grep '^pyscan' "$CURL_LOG" | tr '\n' '|')]"

# FS3 — REQ-ARCHSYNC-004 — Decision: dropping the scanner's missing count passes the exact hole INC-I-235 is about.
run_sandbox fs3_missing 50000; T_THR=0
echo "scan 49981..49998 checked=18 missing=3 ranges=49985..49987 mismatched=0 mismatch_ranges=none errors=0 seconds=0" > "$FIX_DIR/scan.out"
echo 1 > "$FIX_DIR/scan.rc"; run_full
ck "FS3 scanner missing=3: FAIL gs024-no-missing-height, RESULT missing=3 range=49985..49987" \
   'fail_line gs024-no-missing-height | grep -qE "^ *FAIL " && result_has missing=3 && result_has range=49985..49987' "$(mt_det)"

# FS4 — REQ-ARCHSYNC-004 — Decision: heights the instance never answered are unknown, not present; counting them as present hides a hole behind RPC errors.
run_sandbox fs4_errors 50000; T_THR=0
echo "scan 49981..49998 checked=18 missing=0 ranges=none mismatched=0 mismatch_ranges=none errors=4 seconds=0" > "$FIX_DIR/scan.out"
echo 1 > "$FIX_DIR/scan.rc"; run_full
ck "FS4 scanner errors=4 (missing=0): FAIL gs024-no-missing-height" \
   'fail_line gs024-no-missing-height | grep -qE "^ *FAIL "' "$(mt_det) line=[$(fail_line gs024-no-missing-height)]"

# SC1 — REQ-ARCHSYNC-004 — Decision: the scanner is the hole metric for H-1/H-3/H-2; a wrong count or range misstates the evidence.
srv_start a 30 "5,6,7,12" ""; SRV_A="$SRV_URL"
scan_run "$SRV_A" 1 30
sc_ck "SC1 heights 5-7,12 absent: missing=4 ranges=5..7,12..12, exit 1" \
      '[[ "$RC" -eq 1 ]] && [[ "$OUT" =~ ^scan\ 1\.\.30\ checked=30\ missing=4\ ranges=5\.\.7,12\.\.12\ .*errors=0 ]]'
scan_run "$SRV_A" 13 30 --threads 3
sc_ck "SC2 13..30 all present: missing=0 ranges=none errors=0, exit 0" \
      '[[ "$RC" -eq 0 ]] && [[ "$OUT" =~ ^scan\ 13\.\.30\ checked=18\ missing=0\ ranges=none\ mismatched=0\ .*errors=0 ]]'

# SC3 — REQ-ARCHSYNC-004 — Decision: H-3 needs instance == verifier per height; a compare that skips heights absent on the verifier reports a false repair.
srv_start b 30 "20" "9"; SRV_B="$SRV_URL"
scan_run "$SRV_A" 13 30 --compare "$SRV_B"
sc_ck "SC3 --compare: 9 altered (outside), 20 absent on the verifier: mismatched=1 mismatch_ranges=20..20, exit 1" \
      '[[ "$RC" -eq 1 ]] && [[ "$OUT" == *" mismatched=1 mismatch_ranges=20..20 "* ]]'
scan_run "$SRV_B" 1 10 --compare "$SRV_A"
sc_ck "SC3b --compare: 5-7 absent on the verifier, 9 differs: mismatched=4 mismatch_ranges=5..7,9..9" \
      '[[ "$RC" -eq 1 ]] && [[ "$OUT" == *" missing=0 "* && "$OUT" == *" mismatched=4 mismatch_ranges=5..7,9..9 "* ]]'

# SC4 — REQ-ARCHSYNC-004 — Decision: a dead node read as "no block" or as "all present" turns an outage into a hole or into a PASS.
scan_run "http://127.0.0.1:9" 1 5 --timeout 2 --tries 2
sc_ck "SC4 node down: errors=5 missing=0, exit != 0" '[[ "$RC" -ne 0 ]] && [[ "$OUT" == *" missing=0 "* && "$OUT" == *" errors=5"* ]]'

# SC5 — REQ-ARCHSYNC-004 — Decision: a scanner that runs on bad arguments scans nothing and prints a clean line.
scan_run "$SRV_A" 9 3
sc_ck "SC5 LO > HI: exit 2, no scan line" '[[ "$RC" -eq 2 ]] && [[ "$OUT" != scan* ]]'

print_header "TEST SUMMARY"
echo -e "  Tests Passed: ${GREEN}$TESTS_PASSED${NC}"
echo -e "  Tests Failed: ${RED}$TESTS_FAILED${NC}"
echo -e "  Total Tests:  $TESTS_TOTAL"
echo

if [ "$TESTS_FAILED" -eq 0 ] && [ "$TESTS_TOTAL" -gt 0 ]; then EXIT_CODE=0; else EXIT_CODE=1; fi
exit $EXIT_CODE
