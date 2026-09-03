#!/usr/bin/env bash
# ============================================================================
# gauntlet-gs017.sh — GS-017 "over-cap-addbond-refused" scenario (INC-I-203).
#
# Sourced by scripts/gauntlet.sh. OBSERVATIONAL, STATE-NEUTRAL, chain-read-only,
# no confirm-var, part of the DEFAULT run.
#
#   gs017-cli-carries-m3 — precondition, read-only. `<doli> --version` prints
#     `<name> <semver> (<sha>)`; rc 0 only when GS017_M3_COMMIT is an ancestor
#     of that sha. An "unreachable --rpc" capability probe CANNOT stand in for
#     it: the M3 guard (bins/cli/src/cmd_producer/bonds.rs:48) runs AFTER
#     get_network_params, so a pre-M3 and an M3 CLI die at the same connection
#     error. getNodeInfo carries no commit, so the node's M2 status is NOT
#     determinable over RPC — the node version is recorded informationally.
#   gs017-cli-refuses-before-signing — invokes `producer add-bond` with EXACTLY
#     headroom+1 bonds. The count is derived, never hardcoded: any count at or
#     below the headroom is one the node ACCEPTS, which would bond real funds on
#     an unattended default run. Runs ONLY when the precondition passed —
#     GS-017 never lets a pre-M3 CLI reach a node. rc 0 demands the M3 CLIENT
#     text with no node round-trip and no submit line.
#   gs017-no-addbond-residency — settle-and-retry: only an addbond HASH resident
#     in BOTH the pre- and post-window sweeps is a finding. getMempoolTransactions
#     exposes no producer, no pubkey and no bond count, so over-cap cannot be
#     filtered directly and one snapshot would fail on ordinary in-flight
#     traffic. That RPC also has no offset and no cursor, so every request asks
#     for the 500-tx hard cap and getMempoolInfo.txCount is the guard against a
#     silent sample.
#   gs017-no-cap-poison-in-window — no NEW `[BLOCK_POISON] ADDBOND_CAP_EXCEEDED`
#     past a per-log byte offset. Every log still carries pre-fix events, so only
#     GROWTH is a finding. The scan set is EVERY n*.log on disk: NODECFG stops at
#     n12 and the fleet is 17 nodes.
#
# Every precondition is a SKIP (rc 2), never a FAIL: one false FAIL is how a
# scenario earns a standing waiver and stops guarding anything.
#
# Env: GS017_KEYS_DIR, GS017_LOG_DIR, GS017_PORTS, GS017_SEED_PORT,
#      GS017_MAX_BONDS, GS017_NETWORK, GS017_OFFSETS, GS017_M3_COMMIT,
#      GS017_REPO, GS017_SETTLE_SECS, GAUNTLET_WINDOW, DOLI_CLI, NODECFG.
# ============================================================================

GS017_MAX_BONDS="${GS017_MAX_BONDS:-3000}"
GS017_SEED_PORT="${GS017_SEED_PORT:-8500}"
GS017_KEYS_DIR="${GS017_KEYS_DIR:-$HOME/testnet/keys}"
GS017_LOG_DIR="${GS017_LOG_DIR:-$HOME/testnet/logs}"
GS017_NETWORK="${GS017_NETWORK:-testnet}"
GS017_PORTS="${GS017_PORTS:-8500 8501 8502 8503 8504 8505 8506 8507 8508 8509 8510 8511 8512 8513 8514 8515 8516 8517}"
GS017_M3_COMMIT="${GS017_M3_COMMIT:-f250f274}"
GS017_REPO="${GS017_REPO:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." 2>/dev/null && pwd)}"
# The RPC hard cap of crates/rpc/src/methods/stats.rs:188 — limit.min(500).
GS017_PAGE="${GS017_PAGE:-500}"
# 2 x SLOT_DURATION (crates/core/src/consensus/constants.rs:175).
GS017_SETTLE_SECS="${GS017_SETTLE_SECS:-20}"
GS017_WINDOW_SECS="${GAUNTLET_WINDOW:-45}"
GS017_M3_OK=""

# Pre-window baseline for EVERY node log, captured at SOURCE time: gauntlet.sh
# sources this lib before it sleeps its window, so the whole run is the window.
GS017_SRC_OFFSETS="$(
    for _gs017_f in "$GS017_LOG_DIR"/n*.log; do
        [ -r "$_gs017_f" ] || continue
        printf '%s:%s\n' "$(basename "$_gs017_f" .log)" \
            "$(wc -c < "$_gs017_f" 2>/dev/null | tr -d ' ')"
    done
)"

# ── helpers ─────────────────────────────────────────────────────────────────

# _gs017_rpc <port> <method> [params-json] — raw JSON-RPC POST, empty on failure.
_gs017_rpc() {
    local port="$1" method="$2" params="${3:-[]}"
    curl -sf --max-time 5 -X POST "http://127.0.0.1:$port" \
        -H 'Content-Type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$method\",\"params\":$params}" 2>/dev/null
}

# _gs017_doli — echo a usable doli CLI path, non-zero when none resolves.
# $HOME/testnet/bin/doli is preferred over PATH: the fleet binary is the one the
# operator actually runs, and /usr/local/bin/doli is routinely an older build.
_gs017_doli() {
    if [ -n "${DOLI_CLI:-}" ]; then
        [ -x "$DOLI_CLI" ] || return 1
        printf '%s' "$DOLI_CLI"
        return 0
    fi
    if [ -x "$HOME/testnet/bin/doli" ]; then
        printf '%s' "$HOME/testnet/bin/doli"
        return 0
    fi
    command -v doli 2>/dev/null
}

# _gs017_addbond_hashes — read a getMempoolTransactions reply on stdin, echo
# 'OK[ <hash>...]' for the resident addbond entries, or 'ERR' if it did not parse.
_gs017_addbond_hashes() {
    python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    r = d.get('result', d) if isinstance(d, dict) else d
    txs = r.get('transactions', r) if isinstance(r, dict) else r
    if not isinstance(txs, list):
        raise ValueError('shape')
    out = ['OK']
    for t in txs:
        t = t if isinstance(t, dict) else {}
        ty = str(t.get('txType') or t.get('tx_type') or '').lower().replace('_', '')
        if ty == 'addbond':
            out.append(str(t.get('hash') or 'unknown'))
    print(' '.join(out))
except Exception:
    print('ERR')" 2>/dev/null
}

# _gs017_page <port> — one maximum-size mempool page, hashes only.
_gs017_page() {
    _gs017_rpc "$1" getMempoolTransactions "{\"limit\":$GS017_PAGE}" | _gs017_addbond_hashes
}

# _gs017_txcount <port> — getMempoolInfo.txCount, 0 when unreadable.
_gs017_txcount() {
    local n
    n="$(_gs017_rpc "$1" getMempoolInfo \
        | sed -n 's/.*"txCount"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p' | head -1)"
    n="$(printf '%s' "${n:-0}" | tr -dc '0-9')"
    printf '%s' "${n:-0}"
}

# _gs017_first_producer_port — lowest answering port that is not the seed.
_gs017_first_producer_port() {
    local p
    for p in $GS017_PORTS; do
        [ "$p" = "$GS017_SEED_PORT" ] && continue
        _gs017_rpc "$p" getChainInfo '{}' >/dev/null 2>&1 && { printf '%s' "$p"; return 0; }
    done
    return 1
}

# _gs017_pick <producers-json-file> — map a wallet on disk to a live producer and
# echo 'OK <wallet> <pubkey> <bond_count>'. getProducers answers either a bare
# list (live) or {"producers":[...]}, so both shapes are unwrapped. Also echoes
# CAP (best candidate already at the cap), NOMATCH or NOWALLET, each a SKIP.
_gs017_pick() {
    python3 - "$1" "$GS017_KEYS_DIR" "$GS017_MAX_BONDS" <<'PY' 2>/dev/null
import glob, json, os, sys
raw, keys, cap = sys.argv[1], sys.argv[2], int(sys.argv[3])
plist = []
try:
    d = json.load(open(raw))
    r = d.get('result', d) if isinstance(d, dict) else d
    r = r.get('producers', r) if isinstance(r, dict) else r
    plist = r if isinstance(r, list) else []
except Exception:
    plist = []
by_key = {}
for p in plist:
    if not isinstance(p, dict):
        continue
    k = str(p.get('publicKey') or p.get('public_key') or '').lower()
    if k:
        by_key[k] = p
files = sorted(glob.glob(os.path.join(keys, 'producer_*.json')))
if not files:
    print('NOWALLET')
    sys.exit(0)
capped = None
for f in files:
    try:
        w = json.load(open(f))
    except Exception:
        continue
    for a in (w.get('addresses') or []):
        if not isinstance(a, dict):
            continue
        p = by_key.get(str(a.get('public_key') or '').lower())
        if not p or str(p.get('status', '')).lower() != 'active':
            continue
        n = int(p.get('bondCount', p.get('bond_count', 0)) or 0)
        k = str(p.get('publicKey') or p.get('public_key') or '')
        if n < cap:
            print('OK %s %s %d' % (f, k, n))
            sys.exit(0)
        if capped is None:
            capped = (f, k, n)
print('CAP %s %s %d' % capped if capped else 'NOMATCH')
PY
}

# ── precondition: the resolved CLI carries the M3 client guard ──────────────

# _gs017_m3_check <token> [quiet] — sets GS017_M3_OK to 1 (carries M3) or 0.
# quiet=1 suppresses the informational note when called as a gate, not a token.
_gs017_m3_check() {
    local t="$1" quiet="${2:-0}" bin ver sha port nodever
    GS017_M3_OK=0
    bin="$(_gs017_doli)" || bin=""
    if [ -z "$bin" ]; then
        SKIP_REASONS="$SKIP_REASONS; $t: doli CLI not resolvable (DOLI_CLI, \$HOME/testnet/bin/doli, PATH) — its M3 status cannot be read"
        return 2
    fi
    ver="$("$bin" --version 2>&1 | head -1)"
    sha="$(printf '%s' "$ver" | sed -n 's/.*(\([0-9a-fA-F][0-9a-fA-F]*\)).*/\1/p')"
    if [ -z "$sha" ]; then
        SKIP_REASONS="$SKIP_REASONS; $t: $bin reports \"$ver\", which carries no (commit) — M3 ancestry is unreadable"
        return 2
    fi
    if ! command -v git >/dev/null 2>&1; then
        SKIP_REASONS="$SKIP_REASONS; $t: git unavailable — $bin (\"$ver\") cannot be placed against M3 $GS017_M3_COMMIT"
        return 2
    fi
    if ! git -C "$GS017_REPO" rev-parse --verify --quiet "${sha}^{commit}" >/dev/null 2>&1 \
       || ! git -C "$GS017_REPO" rev-parse --verify --quiet "${GS017_M3_COMMIT}^{commit}" >/dev/null 2>&1; then
        SKIP_REASONS="$SKIP_REASONS; $t: $bin (\"$ver\") — $sha or M3 $GS017_M3_COMMIT is not an object in $GS017_REPO"
        return 2
    fi
    if ! git -C "$GS017_REPO" merge-base --is-ancestor "$GS017_M3_COMMIT" "$sha" >/dev/null 2>&1; then
        SKIP_REASONS="$SKIP_REASONS; $t: $bin (\"$ver\") predates M3 $GS017_M3_COMMIT — GS-017 will not let a pre-M3 CLI reach a node"
        return 2
    fi
    GS017_M3_OK=1
    [ "$quiet" = "1" ] && return 0
    port="$(_gs017_first_producer_port)" || port=""
    nodever="$(_gs017_rpc "${port:-$GS017_SEED_PORT}" getNodeInfo \
        | sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
    INFO_REASONS="$INFO_REASONS; $t: $bin (\"$ver\") carries M3 $GS017_M3_COMMIT; node ${port:-none} reports version ${nodever:-unknown} — getNodeInfo exposes no commit, so the node's M2 status is not determinable over RPC"
    return 0
}

# ── assertion: the CLI refuses before it signs ──────────────────────────────

_gs017_cli_check() {
    local t="$1" bin port pick kind wallet pubkey bonds count head out msg tmp rc=0 res
    bin="$(_gs017_doli)" || bin=""
    if [ -z "$bin" ]; then
        SKIP_REASONS="$SKIP_REASONS; $t: doli CLI not resolvable (DOLI_CLI, \$HOME/testnet/bin/doli, PATH) — the client-side refusal cannot be exercised"
        return 2
    fi
    if [ -z "$GS017_M3_OK" ]; then
        _gs017_m3_check gs017-cli-carries-m3 1 || true
    fi
    if [ "$GS017_M3_OK" != "1" ]; then
        SKIP_REASONS="$SKIP_REASONS; $t: precondition gs017-cli-carries-m3 did not pass for $bin — a CLI that is not proven post-M3 is never allowed to reach a node"
        return 2
    fi
    if [ -z "$(ls "$GS017_KEYS_DIR"/producer_*.json 2>/dev/null)" ]; then
        SKIP_REASONS="$SKIP_REASONS; $t: no producer_*.json wallet under $GS017_KEYS_DIR — nothing to submit with"
        return 2
    fi
    port="$(_gs017_first_producer_port)" || port=""
    if [ -z "$port" ]; then
        SKIP_REASONS="$SKIP_REASONS; $t: no live producer RPC on ${GS017_PORTS% *} (seed $GS017_SEED_PORT excluded) — nothing to submit to"
        return 2
    fi
    tmp="$(mktemp -t gs017prod)" || return 2
    _gs017_rpc "$port" getProducers '[]' > "$tmp" 2>/dev/null
    pick="$(_gs017_pick "$tmp")"
    rm -f "$tmp"
    read -r kind wallet pubkey bonds <<EOF
$pick
EOF
    case "${kind:-NOMATCH}" in
        OK) ;;
        CAP)
            SKIP_REASONS="$SKIP_REASONS; $t: producer ${pubkey:0:8} is already at bond_count=$bonds/$GS017_MAX_BONDS — headroom+1 would be a count the node ACCEPTS, so there is no safe probe"
            return 2 ;;
        NOWALLET)
            SKIP_REASONS="$SKIP_REASONS; $t: no producer_*.json wallet under $GS017_KEYS_DIR — nothing to submit with"
            return 2 ;;
        *)
            SKIP_REASONS="$SKIP_REASONS; $t: no wallet under $GS017_KEYS_DIR maps to an active producer on port $port — the refusal cannot be exercised"
            return 2 ;;
    esac
    head=$(( GS017_MAX_BONDS - bonds ))
    count=$(( head + 1 ))
    if [ "$count" -le "$head" ]; then
        SKIP_REASONS="$SKIP_REASONS; $t: derived count $count fits the $head-bond headroom — refusing to submit an AddBond the node would accept"
        return 2
    fi
    [ "${GS017_ECHO_CMD:-0}" = "1" ] && printf '  producer=%s bond_count=%s headroom=%s port=%s wallet=%s\n  cmd: %s --network %s --rpc http://127.0.0.1:%s --wallet %s producer add-bond --count %s\n' \
        "$pubkey" "$bonds" "$head" "$port" "$wallet" "$bin" "$GS017_NETWORK" "$port" "$wallet" "$count"
    out="$("$bin" --network "$GS017_NETWORK" --rpc "http://127.0.0.1:$port" \
        --wallet "$wallet" producer add-bond --count "$count" 2>&1 </dev/null)" || rc=$?
    msg="$(printf '%s' "$out" | tr '\n' ' ' | cut -c1-200)"
    if [ "$rc" -eq 0 ]; then
        FAIL_REASONS="$FAIL_REASONS; $t: add-bond --count $count exited 0 for producer ${pubkey:0:8} at bond_count=$bonds (cap $GS017_MAX_BONDS) — the CLI built, signed and submitted an over-cap AddBond: ${msg:-no output}"
        return 1
    fi
    if printf '%s' "$out" | grep -q 'Submitting add-bond transaction'; then
        FAIL_REASONS="$FAIL_REASONS; $t: add-bond --count $count printed the submit line before exiting $rc — the guard has to refuse BEFORE signing, and this transaction was already built and sent: ${msg}"
        return 1
    fi
    if ! printf '%s' "$out" | grep -q 'Bond cap exceeded'; then
        if printf '%s' "$out" | grep -q 'ADDBOND_CAP_EXCEEDED'; then
            FAIL_REASONS="$FAIL_REASONS; $t: the only refusal is the node text [ADDBOND_CAP_EXCEEDED] — the CLI reached the node, so the M3 client guard is absent or bypassed: ${msg}"
            return 1
        fi
        FAIL_REASONS="$FAIL_REASONS; $t: add-bond --count $count exited $rc with no 'Bond cap exceeded' client refusal — a transport error is not an admission decision: ${msg:-no output}"
        return 1
    fi
    if printf '%s' "$out" | grep -q 'RPC error'; then
        FAIL_REASONS="$FAIL_REASONS; $t: the refusal came back inside an 'RPC error' envelope — the CLI reached the node, so the M3 client guard did not fire first: ${msg}"
        return 1
    fi
    res="$(_gs017_page "$port")"
    case "$res" in
        OK\ *)
            FAIL_REASONS="$FAIL_REASONS; $t: port $port holds addbond${res#OK} after the refusal — the CLI printed a refusal and submitted anyway"
            return 1 ;;
    esac
    INFO_REASONS="$INFO_REASONS; $t: CLI refused --count $count (headroom $head +1) for producer ${pubkey:0:8} at bond_count=$bonds on port $port, no addbond resident: ${msg:-refused}"
    return 0
}

# ── assertion: no addbond SURVIVES the window anywhere on the fleet ─────────

_gs017_residency_check() {
    local t="$1" p res tc h answered=0 maxtc=0 over="" pre="" stuck=""
    for p in $GS017_PORTS; do
        res="$(_gs017_page "$p")"
        case "$res" in OK|OK\ *) ;; *) continue ;; esac
        answered=$(( answered + 1 ))
        tc="$(_gs017_txcount "$p")"
        [ "$tc" -gt "$maxtc" ] && maxtc="$tc"
        if [ "$tc" -gt "$GS017_PAGE" ]; then
            over="$over $p(txCount=$tc, $(( tc - GS017_PAGE )) tx past the ${GS017_PAGE}-tx page)"
        fi
        for h in ${res#OK}; do pre="$pre $p:$h"; done
    done
    if [ "$answered" -eq 0 ]; then
        SKIP_REASONS="$SKIP_REASONS; $t: no node on ${GS017_PORTS%% *}-${GS017_PORTS##* } answered getMempoolTransactions — no mempool inspected"
        return 2
    fi
    if [ -n "$over" ]; then
        FAIL_REASONS="$FAIL_REASONS; $t: getMempoolTransactions is hard-capped at $GS017_PAGE with no offset and no cursor, and$over — that remainder is unfetchable, so a clean sweep here would be a sample, not a result"
        return 1
    fi
    if [ "${GS017_SETTLE_SECS:-0}" -gt 0 ]; then
        sleep "$GS017_SETTLE_SECS"
    fi
    for p in $GS017_PORTS; do
        res="$(_gs017_page "$p")"
        case "$res" in OK|OK\ *) ;; *) continue ;; esac
        for h in ${res#OK}; do
            case " $pre " in *" $p:$h "*) stuck="$stuck $p:$h" ;; esac
        done
    done
    if [ -n "$stuck" ]; then
        FAIL_REASONS="$FAIL_REASONS; $t: addbond still resident after a ${GS017_SETTLE_SECS}s settle (>= 2 slots) on$stuck — an AddBond one node rejected is still gossiped and re-admitted across the fleet"
        return 1
    fi
    INFO_REASONS="$INFO_REASONS; $t: no addbond survived a ${GS017_SETTLE_SECS}s settle across $answered live mempool(s); largest getMempoolInfo txCount observed $maxtc, inside the ${GS017_PAGE}-tx page"
    return 0
}

# ── assertion: no NEW cap poison in the observation window ──────────────────

# _gs017_baseline — 'name|logfile|byte-offset' for EVERY readable n*.log. The
# offset comes from GS017_OFFSETS, else NODECFG, else the source-time snapshot,
# else 0 (a log that appeared mid-run is new in its entirety). NODECFG cannot
# choose the scan SET: gauntlet.sh:196 builds it from `seq 1 12`.
_gs017_baseline() {
    GS017_SRC_OFFSETS="$GS017_SRC_OFFSETS" \
    python3 - "$GS017_LOG_DIR" "${GS017_OFFSETS:-}" "${NODECFG:-}" <<'PY' 2>/dev/null
import glob, json, os, sys
logdir, offs, cfg = sys.argv[1], sys.argv[2], sys.argv[3]

def pairs(text):
    d = {}
    for line in text.splitlines():
        n, _, o = line.strip().partition(':')
        if n:
            d[n] = o or '0'
    return d

snap = pairs(os.environ.get('GS017_SRC_OFFSETS', ''))
filep = {}
if offs:
    try:
        filep = pairs(open(offs).read())
    except Exception:
        filep = {}
cfgp = {}
if cfg:
    try:
        for n in (json.load(open(cfg)).get('nodes') or []):
            nm = n.get('name') or ''
            if nm:
                cfgp[nm] = str(n.get('offset', 0))
    except Exception:
        cfgp = {}
for f in sorted(glob.glob(os.path.join(logdir, 'n*.log'))):
    if not os.access(f, os.R_OK):
        continue
    nm = os.path.basename(f)[:-4]
    print('%s|%s|%s' % (nm, f, filep.get(nm, cfgp.get(nm, snap.get(nm, '0')))))
PY
}

# _gs017_nodecfg_count — how many nodes the runner itself named, for cross-check.
_gs017_nodecfg_count() {
    if [ -n "${NODECFG:-}" ] && [ -r "${NODECFG:-}" ]; then
        grep -o '"name"' "$NODECFG" 2>/dev/null | wc -l | tr -d ' '
    else
        printf '0'
    fi
}

_gs017_poison_check() {
    local t="$1" name logf off n cfg scanned=0 total=0 hits=""
    while IFS='|' read -r name logf off; do
        [ -n "$name" ] && [ -n "$logf" ] || continue
        [ -r "$logf" ] || continue
        scanned=$(( scanned + 1 ))
        # grep -c prints 0 AND exits 1 on no match, so swallow the status and
        # hard-normalise or the arithmetic below dies.
        n="$(tail -c "+$(( ${off:-0} + 1 ))" "$logf" 2>/dev/null \
            | grep -acE '\[BLOCK_POISON\].*ADDBOND_CAP_EXCEEDED' || true)"
        n="$(printf '%s' "$n" | tr -dc '0-9')"
        n="${n:-0}"
        [ "$n" -gt 0 ] && hits="$hits $name($n)"
        total=$(( total + n ))
    done <<EOF
$(_gs017_baseline)
EOF
    cfg="$(_gs017_nodecfg_count)"
    if [ "$scanned" -eq 0 ]; then
        SKIP_REASONS="$SKIP_REASONS; $t: no readable n*.log under $GS017_LOG_DIR — no observation window to scan"
        return 2
    fi
    if [ -n "$hits" ]; then
        FAIL_REASONS="$FAIL_REASONS; $t: $total new [BLOCK_POISON] ADDBOND_CAP_EXCEEDED event(s) past the pre-window baseline on$hits, scanned $scanned n*.log over a ${GS017_WINDOW_SECS}s window (NODECFG named $cfg) — block assembly packed an over-cap AddBond again"
        return 1
    fi
    INFO_REASONS="$INFO_REASONS; $t: 0 new [BLOCK_POISON] ADDBOND_CAP_EXCEEDED events past the pre-window baseline, scanned $scanned n*.log over a ${GS017_WINDOW_SECS}s window (NODECFG named $cfg)"
    return 0
}

_gs017_assert() {
    local t="$1"
    case "$t" in
        gs017-cli-carries-m3) _gs017_m3_check "$t"; return $? ;;
        gs017-cli-refuses-before-signing) _gs017_cli_check "$t"; return $? ;;
        gs017-no-addbond-residency) _gs017_residency_check "$t"; return $? ;;
        gs017-no-cap-poison-in-window) _gs017_poison_check "$t"; return $? ;;
    esac
    FAIL_REASONS="$FAIL_REASONS; $t: unknown GS-017 assertion token"
    return 1
}

# ── standalone ──────────────────────────────────────────────────────────────
# gauntlet.sh has no single-scenario filter, so running GS-017 on its own goes
# through here. Prints the runner's own result shape (gauntlet.sh:673-682).
# The baseline is already taken (source time); the residency settle IS the
# observation window, and the poison scan runs after it.

_gs017_main() {
    local t rc s_ok=1
    GS017_ECHO_CMD=1
    if [ "${1:-}" = "--quick" ]; then
        GS017_WINDOW_SECS=20
    fi
    GS017_SETTLE_SECS="$GS017_WINDOW_SECS"
    FAIL_REASONS=""; SKIP_REASONS=""; INFO_REASONS=""
    for t in gs017-cli-carries-m3 gs017-cli-refuses-before-signing \
             gs017-no-addbond-residency gs017-no-cap-poison-in-window; do
        _gs017_assert "$t"; rc=$?
        { [ "$rc" = "0" ] || [ "$rc" = "2" ]; } || s_ok=0
    done
    if [ "$s_ok" = "1" ]; then
        printf "  PASS %-5s %-32s %s\n" "[obs]" "GS-017" "over-cap-addbond-refused"
    else
        printf "  FAIL %-5s %-32s %s\n" "[obs]" "GS-017" "over-cap-addbond-refused"
        printf "       %s\n" "${FAIL_REASONS# ; }"
    fi
    [ -n "$SKIP_REASONS" ] && printf "       skip:%s\n" "${SKIP_REASONS# ;}"
    [ -n "$INFO_REASONS" ] && printf "       note:%s\n" "${INFO_REASONS# ;}"
    [ "$s_ok" = "1" ] || return 1
    return 0
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    _gs017_main "$@"
    exit $?
fi
