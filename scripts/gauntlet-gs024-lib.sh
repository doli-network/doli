#!/usr/bin/env bash
# gauntlet-gs024-lib.sh — helpers for gauntlet-gs024.sh (GS-024, INC-I-235). Sourced, never run.
# Config, RPC wrappers, preflight, pool selection/upkeep, throttle and the hole scanner wrapper.

_GS024_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# shellcheck disable=SC2034  # gs_* are read by gauntlet-gs024.sh
_gs024_cfg() {
    gs_pool="${GS024_POOL:-$HOME/testnet/drill-inc-i-235}"
    gs_seed="${GS024_SEED_RPC:-http://127.0.0.1:8500}"
    gs_arch="${GS024_SEED_ARCHIVE:-$HOME/testnet/seed/blocks}"
    gs_ckpt="${GS024_SEED_CKPT_DIR:-$HOME/testnet/seed/data/checkpoints}"
    gs_bin="${GS024_BIN:-$_GS024_ROOT/target/release/doli-node}"
    case "$gs_bin" in /*) ;; *) gs_bin="$PWD/$gs_bin" ;; esac
    gs_min="${GS024_MIN_GAP:-500}"
    gs_max="${GS024_MAX_GAP:-20000}"
    gs_p2p="${GS024_P2P_PORT:-30390}"
    gs_rpc="${GS024_RPC_PORT:-8590}"
    gs_met="${GS024_METRICS_PORT:-9090}"
    gs_local="http://127.0.0.1:$gs_rpc"
    gs_py="${GS024_PYTHON:-python3}"
    gs_scan="$_GS024_ROOT/scripts/gauntlet-gs024-scan.py"
    gs_win="${GS024_THROTTLE_WINDOW:-600}"
    gs_lock="${GS024_LOCKOUT_EVAL:-90}"
    _gs024_num "$gs_lock" || gs_lock=90
    gs_thr=0
    [ "${GS024_THROTTLE:-1}" = 1 ] && gs_thr=1
    return 0
}

_gs024_num() { case "${1:-}" in '' | *[!0-9]*) return 1 ;; esac; }

# gs024_rpc URL METHOD [PARAMS] — raw JSON-RPC reply of URL on stdout (one attempt; seed calls).
gs024_rpc() {
    curl -s --max-time "${GS024_RPC_TIMEOUT:-10}" -X POST "$1" -H 'Content-Type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$2\",\"params\":${3:-[]}}" 2>/dev/null
}

# gs024_irpc URL METHOD [PARAMS] — instance call: up to GS024_RPC_TRIES attempts; rc 1 when none answers.
gs024_irpc() {
    local n=0 tries="${GS024_RPC_TRIES:-5}" out
    { _gs024_num "$tries" && [ "$tries" -ge 1 ]; } || tries=5
    while [ "$n" -lt "$tries" ]; do
        n=$((n + 1))
        if out="$(curl -s --max-time "${GS024_RPC_TIMEOUT:-30}" -X POST "$1" -H 'Content-Type: application/json' \
            -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$2\",\"params\":${3:-[]}}" 2>/dev/null)" && [ -n "$out" ]; then
            printf '%s\n' "$out"
            return 0
        fi
        [ "$n" -lt "$tries" ] && sleep 2
    done
    return 1
}

# _gs024_jget KEY < JSON — first scalar value of KEY.
_gs024_jget() {
    tr -d '\n' | grep -oE "\"$1\"[[:space:]]*:[[:space:]]*(\"[^\"]*\"|[^],}[:space:]]+)" | head -n 1 \
        | sed -E 's/^"[^"]*"[[:space:]]*:[[:space:]]*"?//; s/"$//'
}

# _gs024_kv KEY < LINE — value of KEY=VALUE in a space-separated line.
_gs024_kv() { sed -nE "s/^(.* )?$1=([^ ]*).*\$/\\2/p" | head -n 1; }

_gs024_height() { gs024_rpc "$1" getChainInfo '{}' | _gs024_jget bestHeight; }
_gs024_iheight() { gs024_irpc "$1" getChainInfo '{}' | _gs024_jget bestHeight; }

# _gs024_build_running — prints cargo|rustc when a build of THIS checkout runs (its args name ROOT/).
_gs024_build_running() {
    ps -axww -o args= 2>/dev/null | awk -v r="$_GS024_ROOT/" -v w="$_GS024_ROOT/.claude/worktrees/" '
        { b = $1; sub(/.*\//, "", b) }
        (b == "cargo" || b == "rustc") && index($0, r) && !index($0, w) { print b; exit }'
}

# gs024_preflight — rc 0 when ports, build state, seed, binary and scanner allow a run.
gs024_preflight() {
    _gs024_cfg
    local p bad=0 b
    for p in "$gs_p2p" "$gs_rpc" "$gs_met"; do
        if ! _gs024_num "$p"; then
            echo "NOT READY: invalid port '$p'"; bad=1
        elif lsof -nP -iTCP:"$p" -sTCP:LISTEN >/dev/null 2>&1; then
            echo "NOT READY: TCP port $p is busy"; bad=1
        fi
    done
    if _gs024_num "$gs_p2p" && lsof -nP -iUDP:"$((gs_p2p + 1))" >/dev/null 2>&1; then
        echo "NOT READY: UDP discovery port $((gs_p2p + 1)) is busy"; bad=1
    fi
    if [ "${GS024_ALLOW_BUILD:-0}" != 1 ]; then
        b="$(_gs024_build_running)"
        if [ -n "$b" ]; then
            echo "NOT READY: a build of this checkout is running ($b); GS024_ALLOW_BUILD=1 overrides"; bad=1
        fi
    fi
    if [ ! -f "$gs_bin" ] || [ ! -x "$gs_bin" ]; then
        echo "NOT READY: binary $gs_bin missing or not executable"; bad=1
    fi
    if ! command -v "$gs_py" >/dev/null 2>&1 || [ ! -f "$gs_scan" ]; then
        echo "NOT READY: scanner needs $gs_py and $gs_scan"; bad=1
    fi
    if ! _gs024_num "$(_gs024_height "$gs_seed")"; then
        echo "NOT READY: seed RPC $gs_seed does not answer getChainInfo"; bad=1
    fi
    return "$bad"
}

# gs024_pick_base TIP — OLDEST canonical pool ckpt-h* dir with MIN_GAP <= TIP-h <= MAX_GAP.
gs024_pick_base() {
    _gs024_cfg
    local tip="${1:-}" tab d h hash cands="" why="" n=0 gap sh reply
    tab="$(printf '\t')"
    if ! _gs024_num "$tip"; then echo "NOT READY: seed tip unknown"; return 1; fi
    for d in "$gs_pool"/ckpt-h*; do
        { [ -d "$d" ] && [ ! -L "$d" ]; } || continue
        n=$((n + 1))
        if [ ! -f "$d/health.json" ] || [ ! -d "$d/state_db" ] || [ ! -d "$d/blocks" ]; then
            why="$why; ${d##*/} incomplete (needs blocks, state_db, health.json)"; continue
        fi
        h="$(_gs024_jget height < "$d/health.json")"
        hash="$(_gs024_jget hash < "$d/health.json")"
        case "$hash" in *[!0-9a-f]*) hash="" ;; esac
        if ! _gs024_num "$h" || [ "${#hash}" -ne 64 ]; then
            why="$why; ${d##*/} has no height/hash in health.json"; continue
        fi
        cands="$cands$h$tab$hash$tab$d
"
    done
    if [ "$n" -eq 0 ]; then echo "NOT READY: no base checkpoint (ckpt-h*) in $gs_pool"; return 1; fi
    while IFS="$tab" read -r h hash d; do
        [ -n "$d" ] || continue
        if [ "$h" -gt "$tip" ]; then why="$why; ${d##*/} above seed tip $tip"; continue; fi
        gap=$((tip - h))
        if [ "$gap" -lt "$gs_min" ]; then why="$why; ${d##*/} too young (gap $gap < $gs_min)"; continue; fi
        if [ "$gap" -gt "$gs_max" ]; then why="$why; ${d##*/} too old (gap $gap > $gs_max)"; continue; fi
        reply="$(gs024_rpc "$gs_seed" getBlockByHeight "{\"height\":$h}")"
        if [ -z "$reply" ]; then
            echo "NOT READY: seed RPC $gs_seed did not answer getBlockByHeight $h"; return 1
        fi
        sh="$(printf '%s' "$reply" | _gs024_jget hash)"
        if [ "$sh" != "$hash" ]; then why="$why; ${d##*/} not canonical (seed hash at $h: ${sh:-none})"; continue; fi
        printf '%s\n' "$d"
        return 0
    done <<EOF
$(printf '%s' "$cands" | sort -t "$tab" -k1,1n)
EOF
    echo "NOT READY: no admissible base in $gs_pool (tip $tip, gap $gs_min..$gs_max):${why#;}"
    return 1
}

# gs024_ranges < ints — `a..b,c..c` or `none`.
gs024_ranges() {
    sort -n -u | awk '$1 ~ /^[0-9]+$/ { v = $1 + 0
            if (n == 0) { s = v; e = v; n = 1 } else if (v == e + 1) { e = v }
            else { o = o (o == "" ? "" : ",") s ".." e; s = v; e = v } }
        END { if (n) print o (o == "" ? "" : ",") s ".." e; else print "none" }'
}

# _gs024_expand < "a..b,c" or "a-b,c" — one integer per line.
_gs024_expand() {
    tr ',' '\n' | sed -E 's/\.\./-/' | awk -F- '$1 ~ /^[0-9]+$/ { e = (NF == 2 && $2 ~ /^[0-9]+$/) ? $2 : $1
        for (i = $1; i <= e; i++) print i }'
}

# gs024_prune_pool TIP — removes pool ckpt-h* dirs whose health.json height is > MAX_GAP behind TIP.
gs024_prune_pool() {
    _gs024_cfg
    local tip="${1:-}" d h
    if ! _gs024_num "$tip"; then echo "gs024: prune skipped, seed tip unknown"; return 1; fi
    for d in "$gs_pool"/ckpt-h*; do
        { [ -d "$d" ] && [ ! -L "$d" ] && [ -f "$d/health.json" ]; } || continue
        h="$(_gs024_jget height < "$d/health.json")"
        _gs024_num "$h" || continue
        [ $((tip - h)) -gt "$gs_max" ] || continue
        echo "gs024: pruning ${d##*/} (gap $((tip - h)) > $gs_max)"
        rm -rf -- "$d"
    done
    return 0
}

# _gs024_copy SRC DST — APFS clone, plain copy only when the clone left nothing behind.
_gs024_copy() {
    cp -cR "$1" "$2" 2>/dev/null && return 0
    [ -e "$2" ] && return 1
    cp -R "$1" "$2"
}

# gs024_refresh_pool — clones the oldest seed checkpoint into the pool as ckpt-h<H>-<ts>.
gs024_refresh_pool() {
    _gs024_cfg
    local d n best="" src="" dst tmp
    for d in "$gs_ckpt"/h*-*; do
        [ -d "$d" ] || continue
        n="${d##*/}"; n="${n#h}"; n="${n%%-*}"
        _gs024_num "$n" || continue
        if [ -z "$best" ] || [ "$n" -lt "$best" ]; then best="$n"; src="$d"; fi
    done
    if [ -z "$src" ]; then echo "gs024: no seed checkpoint in $gs_ckpt"; return 1; fi
    dst="$gs_pool/ckpt-${src##*/}"
    [ -e "$dst" ] && return 0
    tmp="$gs_pool/.clone-${src##*/}"
    [ -e "$tmp" ] && rm -rf -- "$tmp"
    if mkdir -p "$tmp" && _gs024_copy "$src/blocks" "$tmp/blocks" && _gs024_copy "$src/state_db" "$tmp/state_db" \
        && cp "$src/health.json" "$tmp/health.json" && mv "$tmp" "$dst"; then
        echo "gs024: pool refreshed with ${dst##*/}"
        return 0
    fi
    rm -rf -- "$tmp"
    return 1
}

# gs024_throttle on|off PID — darwin background policy on PID (-b) or off (-B); no-op unless GS024_THROTTLE=1.
gs024_throttle() {
    [ "${GS024_THROTTLE:-1}" = 1 ] || return 0
    local flag
    case "${1:-}" in on) flag=-b ;; off) flag=-B ;; *) return 2 ;; esac
    _gs024_num "${2:-}" || return 2
    "${GS024_TASKPOLICY:-/usr/sbin/taskpolicy}" "$flag" -p "$2"
}

# gs024_scan URL LO HI — scanner line on stdout; rc 0 only when every height LO..HI answered a block.
gs024_scan() {
    _gs024_cfg
    "$gs_py" -I "$gs_scan" "$1" "$2" "$3"
}
