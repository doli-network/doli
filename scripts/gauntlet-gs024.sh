#!/usr/bin/env bash
# gauntlet-gs024.sh — GS-024 "archive-node-far-behind-restart" (INC-I-235).
# Standalone runner. Starts ONE scratch non-producer archive instance from the OLDEST pool
# checkpoint MIN_GAP..MAX_GAP blocks behind the local testnet seed. Once its RPC answers it is
# put under darwin background policy for GS024_THROTTLE_WINDOW s (the slow-host model: the gap
# is still >= MIN_GAP when the 60 s post-restart lockout ends), then released to catch up.
# Assertions: gs024-gap-held gs024-no-snap gs024-reaches-tip gs024-no-missing-height
#   gs024-commitment-matches-seed gs024-archive-files-present gs024-clean-exit
# Usage: gauntlet-gs024.sh [--keep]      exit: 0 PASS · 1 FAIL · 3 NOT READY
# Env: GS024_POOL GS024_SEED_RPC GS024_SEED_ARCHIVE GS024_SEED_CKPT_DIR GS024_BIN GS024_PYTHON
#   GS024_MIN_GAP GS024_MAX_GAP GS024_P2P_PORT GS024_RPC_PORT GS024_METRICS_PORT GS024_ALLOW_BUILD
#   GS024_THROTTLE (1) GS024_THROTTLE_WINDOW (600) GS024_TASKPOLICY GS024_FULL_SCAN (1 = scan 1..H)
#   GS024_LOCKOUT_EVAL (90 = s after RPC-up of the post-lockout gap sample) GS024_TIMEOUT GS024_RPC_TIMEOUT GS024_RPC_TRIES GS024_STOP_GRACE GS024_LIB_ONLY (1 = define only)
# --keep keeps the run dir (the instance is still stopped); restart it with its start.sh.
# Safety: signals only the PID it started; deletes only its run dir and pool ckpt-h* dirs.

# shellcheck source=scripts/gauntlet-gs024-lib.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/gauntlet-gs024-lib.sh"

# --- instance control (own PID only) ---

_gs024_alive() {
    local st
    [ -n "$PID" ] || return 1
    st="$(ps -p "$PID" -o stat= 2>/dev/null | tr -d ' ')"
    [ -n "$st" ] && [ "${st#Z}" = "$st" ]
}

_gs024_ours() { [ -n "$PID" ] && [ -n "$R" ] && ps -p "$PID" -o command= 2>/dev/null | grep -qF -- "$R"; }

_gs024_launch() {
    bash "$R/start.sh" </dev/null &
    PID=$!
    echo "$PID" >"$R/pid"
}

# _gs024_unthrottle — releases the background policy once.
_gs024_unthrottle() {
    [ "${GS_THR_ON:-0}" = 1 ] || return 0
    GS_THR_ON=0
    _gs024_alive && _gs024_ours && gs024_throttle off "$PID"
    return 0
}

# _gs024_stop — rc 0 exited on SIGTERM · 1 not running · 2 needed SIGKILL · 3 still running.
_gs024_stop() {
    local n=0 rc=1 grace="${GS024_STOP_GRACE:-180}"
    _gs024_num "$grace" || grace=180
    [ -n "$PID" ] || return 1
    if _gs024_alive && _gs024_ours; then
        rc=0
        kill -TERM "$PID" 2>/dev/null
        while _gs024_alive && [ "$n" -lt "$grace" ]; do sleep 1; n=$((n + 1)); done
        if _gs024_alive && _gs024_ours; then
            kill -KILL "$PID" 2>/dev/null
            GS_FORCED=1; rc=2; sleep 2
        fi
        _gs024_alive && rc=3
    fi
    if ! _gs024_alive; then
        wait "$PID" 2>/dev/null
        PID=""
    fi
    return "$rc"
}

_gs024_wait_rpc() {
    local end=$(($(date +%s) + $1))
    while [ "$(date +%s)" -lt "$end" ]; do
        _gs024_alive || return 1
        _gs024_num "$(_gs024_iheight "$gs_local")" && return 0
        sleep 5
    done
    return 1
}

_gs024_log() { [ -n "$R" ] && [ -f "$R/node.log" ] && sed $'s/\x1b\\[[0-9;]*m//g' "$R/node.log"; }
_gs024_snap_line() { _gs024_log | grep -m 1 -E '\[SNAP_SYNC\] Starting snap sync|\[COORDINATOR\] action=SnapSync'; }

_gs024_write_start() {
    local p
    {
        echo '#!/usr/bin/env bash'
        printf 'exec %q --network testnet --data-dir %q --log-level info run' "$gs_bin" "$R/data"
        printf ' --p2p-port %s --rpc-port %s --rpc-bind 127.0.0.1 --metrics-port %s --archive-to %q' \
            "$gs_p2p" "$gs_rpc" "$gs_met" "$R/blocks"
        for p in 30300 30301 30302 30303; do printf ' --bootstrap /ip4/127.0.0.1/tcp/%s' "$p"; done
        printf ' --no-auto-update --yes >> %q 2>&1\n' "$R/node.log"
    } >"$R/start.sh" && chmod +x "$R/start.sh"
}

# _gs024_trim_archive BASE_H BASE_HASH — drop $R/blocks files above BASE_H; manifest -> base.
_gs024_trim_archive() {
    local dir="$R/blocks" gen
    find "$dir" -maxdepth 1 -type f -name '[0-9]*' | sed 's|.*/||' \
        | awk -v b="$1" '{ n = substr($0, 1, 10); s = substr($0, 11) }
            n ~ /^[0-9]+$/ && length(n) == 10 && s ~ /^\.(block|blake3)(\.tmp)?$/ && n + 0 > b + 0' \
        | (cd "$dir" && xargs rm -f) || return 1
    gen="$(_gs024_jget genesis_hash <"$dir/manifest.json")"
    {
        echo "{"
        [ -n "$gen" ] && printf '  "genesis_hash": "%s",\n' "$gen"
        printf '  "latest_hash": "%s",\n  "latest_height": %s\n}\n' "$2" "$1"
    } >"$dir/manifest.json.tmp" && mv "$dir/manifest.json.tmp" "$dir/manifest.json"
}

# --- assertions ---

_gs024_res() {
    if [ "$2" = 1 ]; then echo "  PASS $1"; [ -n "${3:-}" ] && echo "       $3"
    else echo "  FAIL $1: ${3:-}"; GS_FAIL=1; fi
    return 0
}

# Rule 2 was reachable: >= MIN_GAP behind at the window end, at a snap order or when the post-restart
# lockout ended (first coordinator evaluation), or the coordinator logged a decision at gap >= MIN_GAP.
_gs024_check_gap() {
    local ok=0 why="${GS_WWHY:-none}" c
    case "$why" in
        window_end | snap_ordered) _gs024_num "$GS_WGAP" && [ "$GS_WGAP" -ge "$gs_min" ] && ok=1 ;;
    esac
    c="$(_gs024_log | grep -oE '\[COORDINATOR\] action=[A-Za-z]+ gap=[0-9]+' | awk -v m="$gs_min" '
        { a = $2; g = $3; sub(/^action=/, "", a); sub(/^gap=/, "", g); if (g + 0 >= m + 0) { print a ":" g; exit } }')"
    if [ "$ok" = 0 ] && [ -n "$c" ]; then ok=1; why=coordinator; fi
    if [ "$ok" = 0 ] && _gs024_num "$GS_PLGAP" && [ "$GS_PLGAP" -ge "$gs_min" ]; then ok=1; why=post_lockout; fi
    _gs024_res gs024-gap-held "$ok" "window_gap=${GS_WGAP:-none} post_lockout_gap=${GS_PLGAP:-none} min_gap=$gs_min end=$why throttle_end=${GS_WWHY:-none} coordinator=${c:-none} window_s=$gs_win lockout_eval_s=$gs_lock throttle=$gs_thr"
}

# _gs024_check_holes BASE_H H — verifyChainIntegrity [H,1] + scanner LO..H (LO = base+1 | 1).
_gs024_check_holes() {
    local bh="$1" H="$2" vci complete vmiss lo line src smiss serr smis
    GS_MISS_N=unknown; GS_MISS_R=none
    if ! _gs024_num "$H"; then
        _gs024_res gs024-no-missing-height 0 "no common height (instance or seed RPC down)"; return
    fi
    vci="$(GS024_RPC_TIMEOUT=600 gs024_irpc "$gs_local" verifyChainIntegrity "[$H,1]")"
    complete="$(printf '%s' "$vci" | _gs024_jget complete)"
    vmiss="$(printf '%s' "$vci" | _gs024_jget missingCount)"
    printf '%s' "$vci" | tr -d '\n' | grep -oE '"missing"[[:space:]]*:[[:space:]]*\[[^]]*\]' | head -n 1 \
        | sed -E 's/^[^[]*\[//; s/\]$//' | tr -d '" ' | _gs024_expand >"$R/missing.txt"
    lo=$((bh + 1))
    [ "${GS024_FULL_SCAN:-0}" = 1 ] && lo=1
    echo "gs024: scanning getBlockByHeight $lo..$H on the instance"
    line="$(gs024_scan "$gs_local" "$lo" "$H")"
    src=$?
    echo "gs024: ${line:-scanner printed nothing (rc $src)}"
    smiss="$(printf '%s' "$line" | _gs024_kv missing)"
    serr="$(printf '%s' "$line" | _gs024_kv errors)"
    smis="$(printf '%s' "$line" | _gs024_kv mismatched)"
    printf '%s' "$line" | _gs024_kv ranges | _gs024_expand >>"$R/missing.txt"
    GS_MISS_N="$(sort -n -u "$R/missing.txt" | grep -c .)"
    GS_MISS_R="$(gs024_ranges <"$R/missing.txt")"
    local ok=0
    [ "$complete" = true ] && [ "$vmiss" = 0 ] && [ "$src" = 0 ] && [ "$smiss" = 0 ] && [ "$serr" = 0 ] \
        && [ "${smis:-0}" = 0 ] && [ "$GS_MISS_N" = 0 ] && ok=1
    _gs024_res gs024-no-missing-height "$ok" \
        "H=$H complete=${complete:-none} missingCount=${vmiss:-none} scan=$lo..$H scan_missing=${smiss:-none} scan_errors=${serr:-none} missing=$GS_MISS_N range=$GS_MISS_R"
}

_gs024_check_commit() {
    local H="$1" ci cs
    if ! _gs024_num "$H"; then _gs024_res gs024-commitment-matches-seed 0 "no common height"; return; fi
    ci="$(GS024_RPC_TIMEOUT=600 gs024_irpc "$gs_local" verifyChainIntegrity "[$H]" | _gs024_jget chainCommitment)"
    cs="$(GS024_RPC_TIMEOUT=600 gs024_rpc "$gs_seed" verifyChainIntegrity "[$H]" | _gs024_jget chainCommitment)"
    local ok=0
    [ -n "$ci" ] && [ "$ci" = "$cs" ] && ok=1
    _gs024_res gs024-commitment-matches-seed "$ok" "H=$H instance=${ci:-none} seed=${cs:-none}"
}

_gs024_archive_missing() {
    local h="$1" n=0 f
    while [ "$h" -le "$2" ]; do
        printf -v f '%s/blocks/%010d.block' "$R" "$h"
        [ -f "$f" ] || n=$((n + 1))
        h=$((h + 1))
    done
    echo "$n"
}

# _gs024_check_archive BASE_H H — .block files for BASE_H+1..F (F = H-10), one restart allowed.
_gs024_check_archive() {
    local bh="$1" F miss end
    GS_RESTART=0
    if ! _gs024_num "$2" || [ $(($2 - 10)) -le "$bh" ]; then
        _gs024_res gs024-archive-files-present 0 "no finalized height above base $bh (H=${2:-none}) restart_used=0"
        return
    fi
    F=$(($2 - 10))
    miss="$(_gs024_archive_missing $((bh + 1)) "$F")"
    if [ "$miss" -gt 0 ]; then
        echo "gs024: $miss .block files missing in $((bh + 1))..$F; restarting the instance once"
        GS_RESTART=1
        _gs024_stop
        if [ -z "$PID" ]; then
            _gs024_launch
            echo "gs024: restarted pid=$PID"
            if _gs024_wait_rpc 900; then
                end=$(($(date +%s) + 600))
                while [ "$(date +%s)" -lt "$end" ]; do
                    miss="$(_gs024_archive_missing $((bh + 1)) "$F")"
                    [ "$miss" -eq 0 ] && break
                    sleep 10
                done
            fi
        fi
        miss="$(_gs024_archive_missing $((bh + 1)) "$F")"
    fi
    local ok=0
    [ "$miss" -eq 0 ] && ok=1
    _gs024_res gs024-archive-files-present "$ok" "missing_files=$miss range=$((bh + 1))..$F restart_used=$GS_RESTART"
}

_gs024_check_exit() {
    local was=0 rc pan ok=0
    _gs024_alive && was=1
    _gs024_stop
    rc=$?
    pan="$(_gs024_log | grep -m 1 'panicked')"
    [ "$was" = 1 ] && [ "$rc" = 0 ] && [ "$GS_FORCED" = 0 ] && [ -z "$pan" ] && ok=1
    _gs024_res gs024-clean-exit "$ok" "running_before_stop=$was stop_rc=$rc forced=$GS_FORCED panic=${pan:-none}"
}

_gs024_check_snap() {
    local s ok=0
    s="$(_gs024_snap_line)"
    [ -z "$s" ] && ok=1
    _gs024_res gs024-no-snap "$ok" "${s:-no snap lines in node.log}"
}

# --- teardown ---

_gs024_teardown() {
    [ "${GS_TORN:-0}" = 1 ] && return 0
    GS_TORN=1
    if [ -n "$R" ] && [ -d "$R" ]; then
        GS_RUNDIR="$R"
        if [ "${GS_KEEP:-0}" != 1 ] && [ -z "$PID" ]; then
            case "$R" in
                "$gs_pool"/runs/run-*)
                    rm -rf -- "$R"
                    [ -e "$R" ] || GS_RUNDIR=removed ;;
            esac
        fi
    fi
    gs024_refresh_pool >&2 || echo "gs024: warning: pool refresh failed" >&2
    if _gs024_num "$TIP"; then
        gs024_prune_pool "$TIP" >&2 || echo "gs024: warning: pool prune failed" >&2
    fi
    return 0
}

# shellcheck disable=SC2329  # invoked via trap
_gs024_on_exit() {
    _gs024_unthrottle >/dev/null 2>&1
    _gs024_stop >/dev/null 2>&1
    _gs024_teardown
}

# _gs024_prepare — picks the base and builds the run dir; sets base bh bhash gap R. rc 3 = NOT READY.
_gs024_prepare() {
    local free
    gs024_preflight || return 3
    TIP="$(_gs024_height "$gs_seed")"
    if ! base="$(gs024_pick_base "$TIP")"; then printf '%s\n' "$base"; return 3; fi
    bh="$(_gs024_jget height <"$base/health.json")"
    bhash="$(_gs024_jget hash <"$base/health.json")"
    gap=$((TIP - bh))
    free="$(df -Pk "$gs_pool" 2>/dev/null | awk 'NR == 2 { print $4 }')"
    if ! _gs024_num "$free" || [ "$free" -lt 5242880 ]; then
        echo "NOT READY: less than 5 GB free under $gs_pool (${free:-?} KB)"; return 3
    fi
    if [ ! -f "$gs_arch/manifest.json" ]; then
        echo "NOT READY: seed archive $gs_arch has no manifest.json"; return 3
    fi
    R="$gs_pool/runs/run-$(date +%s)"
    [ -e "$R" ] && R="$R-$$"
    echo "gs024: base=${base##*/} height=$bh tip=$TIP gap=$gap run_dir=$R"
    if ! mkdir -p "$R/data" || ! _gs024_copy "$base/blocks" "$R/data/blocks" \
        || ! _gs024_copy "$base/state_db" "$R/data/state_db"; then
        echo "NOT READY: cannot clone base ${base##*/} into $R"; return 3
    fi
    echo "gs024: cloning seed archive $gs_arch (1-3 min)"
    if ! _gs024_copy "$gs_arch" "$R/blocks"; then echo "NOT READY: cannot clone seed archive"; return 3; fi
    if ! _gs024_trim_archive "$bh" "$bhash"; then echo "NOT READY: cannot trim archive clone to $bh"; return 3; fi
    if ! _gs024_write_start; then echo "NOT READY: cannot write $R/start.sh"; return 3; fi
    return 0
}

# _gs024_catchup — launch, throttle window, wait for the tip. Sets reached tcatch hn st GS_WGAP GS_WWHY GS_PLGAP.
_gs024_catchup() {
    local now h last=0 tthr="" snap=""
    t0="$(date +%s)"
    _gs024_launch
    echo "gs024: started pid=$PID rpc=$gs_local timeout=${timeout}s throttle=$gs_thr window=${gs_win}s (restart by hand: bash $R/start.sh &)"
    while :; do
        now="$(date +%s)"
        if ! _gs024_alive; then echo "gs024: instance exited after $((now - t0))s"; [ -z "$GS_WWHY" ] && GS_WWHY=exited; break; fi
        h="$(_gs024_iheight "$gs_local")"
        _gs024_num "$h" && hn="$h"
        st="$(_gs024_height "$gs_seed")"
        _gs024_num "$st" || st="$TIP"
        if [ -z "$tthr" ] && _gs024_num "$h"; then
            [ "$gs_thr" = 1 ] && gs024_throttle on "$PID" && GS_THR_ON=1
            tthr="$(date +%s)"
            echo "gs024: RPC up at t=$((tthr - t0))s height=$h seed=$st; throttle=$GS_THR_ON for ${gs_win}s"
        fi
        if [ -n "$tthr" ] && [ -z "$GS_PLGAP" ] && [ "$now" -ge $((tthr + gs_lock)) ] && _gs024_num "$h"; then
            GS_PLGAP=$((st - h))
            echo "gs024: post-lockout sample at t=$((now - t0))s height=$h seed=$st gap=$GS_PLGAP"
        fi
        if [ -z "$snap" ]; then
            snap="$(_gs024_snap_line)"
            if [ -n "$snap" ]; then
                echo "gs024: t=$((now - t0))s snap observed: $snap"
                [ -z "$GS_WWHY" ] && { GS_WGAP=$((st - ${hn:-$bh})); GS_WWHY=snap_ordered; }
            fi
        fi
        if [ -n "$tthr" ] && [ "$now" -ge $((tthr + gs_win)) ] && [ -z "$GS_TWEND" ]; then
            [ -z "$GS_WWHY" ] && { GS_WGAP=$((st - hn)); GS_WWHY=window_end; }
            echo "gs024: window end at t=$((now - t0))s height=$hn seed=$st gap=$((st - hn))"
            _gs024_unthrottle
            GS_TWEND="$now"
        fi
        if _gs024_num "$h" && [ "$h" -ge $((st - 2)) ]; then reached="$h"; tcatch=$((now - t0)); break; fi
        [ "$now" -ge $((t0 + timeout)) ] && break
        if [ $((now - last)) -ge 60 ]; then
            echo "gs024: t=$((now - t0))s height=${hn:-none} seed=$st"; last="$now"
        fi
        sleep 5
    done
    if [ -z "$GS_WWHY" ]; then
        GS_WWHY=early_tip; [ -z "$reached" ] && GS_WWHY=timeout
        _gs024_num "$hn" && GS_WGAP=$((st - hn))
    fi
    _gs024_unthrottle
}

gs024_main() {
    _gs024_cfg
    local a base bh bhash gap timeout t0 hn="" st="" reached="" tcatch="" bps hi hs H="" pw=none
    GS_KEEP=0
    for a in "$@"; do
        case "$a" in
            --keep) GS_KEEP=1 ;;
            *) echo "usage: gauntlet-gs024.sh [--keep]" >&2; return 2 ;;
        esac
    done
    R=""; PID=""; TIP=""; GS_TORN=0; GS_FORCED=0; GS_FAIL=0; GS_RUNDIR=none; GS_RESTART=0
    GS_MISS_N=unknown; GS_MISS_R=none; GS_THR_ON=0; GS_TWEND=""; GS_WGAP=""; GS_WWHY=""; GS_PLGAP=""
    trap _gs024_on_exit EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM

    _gs024_prepare || return 3
    timeout="${GS024_TIMEOUT:-}"
    if ! _gs024_num "$timeout"; then
        timeout=$((2 * gap + gs_win)); [ "$timeout" -lt 1800 ] && timeout=1800
    fi
    echo "gs024: binary $("$gs_bin" --version 2>/dev/null | head -n 1)"
    _gs024_catchup
    if [ -n "$reached" ]; then
        bps="$(awk -v a=$((reached - bh)) -v t="$tcatch" 'BEGIN { printf "%.2f", (t > 0 ? a / t : 0) }')"
        _gs024_num "$GS_TWEND" && pw=$((t0 + tcatch - GS_TWEND))
        echo "gs024: reached $reached in ${tcatch}s (post_window_s=$pw); waiting 30s for the archiver"
        sleep 30
    else
        bps="$(awk -v a=$((${hn:-$bh} - bh)) -v t=$(($(date +%s) - t0)) 'BEGIN { printf "%.2f", (t > 0 ? a / t : 0) }')"
    fi

    hi="$(_gs024_iheight "$gs_local")"
    hs="$(_gs024_height "$gs_seed")"
    if _gs024_num "$hi" && _gs024_num "$hs"; then H=$(((hi < hs ? hi : hs) - 2)); fi
    _gs024_check_gap
    a=0
    [ -n "$reached" ] && a=1
    _gs024_res gs024-reaches-tip "$a" \
        "height=${hn:-none} seed=$st catchup_s=${tcatch:-none} post_window_s=$pw elapsed_s=$(($(date +%s) - t0)) timeout_s=$timeout bps=$bps"
    _gs024_check_holes "$bh" "$H"
    _gs024_check_commit "$H"
    _gs024_check_archive "$bh" "$H"
    _gs024_check_exit
    _gs024_check_snap
    _gs024_teardown

    a=PASS; [ "$GS_FAIL" = 0 ] || a=FAIL
    echo "GS024 RESULT: $a base=$bh tip=$TIP gap=$gap throttle=$gs_thr window_gap=${GS_WGAP:-none} catchup_s=${tcatch:-none} post_window_s=$pw bps=$bps missing=$GS_MISS_N range=$GS_MISS_R restart_used=$GS_RESTART run_dir=$GS_RUNDIR"
    [ "$GS_FAIL" = 0 ] || return 1
    return 0
}

if [ "${BASH_SOURCE[0]}" = "$0" ] && [ "${GS024_LIB_ONLY:-0}" != 1 ]; then
    gs024_main "$@"
    exit $?
fi
