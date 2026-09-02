#!/usr/bin/env bash
# ============================================================================
# gauntlet-gs017.sh — GS-017 "over-cap-addbond-refused" scenario (INC-I-203).
#
# Sourced by scripts/gauntlet.sh. OBSERVATIONAL, STATE-NEUTRAL, chain-read-only,
# no confirm-var, part of the DEFAULT run.
#
#   gs017-cli-refuses-before-signing — invokes `producer add-bond` with EXACTLY
#     headroom+1 bonds. The count is derived, never hardcoded: any count at or
#     below the headroom is one the node ACCEPTS, which would bond real funds on
#     an unattended default run. Exercises the M3 CLIENT path; the node
#     admission path is covered by the INV-BOND-002 regression tests.
#   gs017-no-addbond-residency — no `addbond` resident in any mempool on
#     8500-8517. A port with no RPC is tolerated, never a failure.
#   gs017-no-cap-poison-in-window — no NEW `[BLOCK_POISON] ADDBOND_CAP_EXCEEDED`
#     past the runner's per-node log byte offsets. Every log still carries
#     pre-fix events, so only GROWTH is a finding.
#
# Every precondition is a SKIP (rc 2), never a FAIL: one false FAIL is how a
# scenario earns a standing waiver and stops guarding anything.
#
# Env: GS017_KEYS_DIR, GS017_LOG_DIR, GS017_PORTS, GS017_SEED_PORT,
#      GS017_MAX_BONDS, GS017_NETWORK, GS017_OFFSETS, DOLI_CLI, NODECFG.
# ============================================================================

GS017_MAX_BONDS="${GS017_MAX_BONDS:-3000}"
GS017_SEED_PORT="${GS017_SEED_PORT:-8500}"
GS017_KEYS_DIR="${GS017_KEYS_DIR:-$HOME/testnet/keys}"
GS017_LOG_DIR="${GS017_LOG_DIR:-$HOME/testnet/logs}"
GS017_NETWORK="${GS017_NETWORK:-testnet}"
GS017_PORTS="${GS017_PORTS:-8500 8501 8502 8503 8504 8505 8506 8507 8508 8509 8510 8511 8512 8513 8514 8515 8516 8517}"

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

# _gs017_addbond_count — read a getMempoolTransactions reply on stdin, echo the
# number of resident `addbond` entries, or -1 when the reply does not parse.
_gs017_addbond_count() {
    python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    r = d.get('result', d) if isinstance(d, dict) else d
    txs = r.get('transactions', r) if isinstance(r, dict) else r
    n = 0
    for t in (txs if isinstance(txs, list) else []):
        t = t if isinstance(t, dict) else {}
        ty = str(t.get('txType') or t.get('tx_type') or '').lower().replace('_', '')
        if ty == 'addbond':
            n += 1
    print(n)
except Exception:
    print(-1)" 2>/dev/null
}

# _gs017_live_ports — ports on GS017_PORTS answering JSON-RPC, space separated.
_gs017_live_ports() {
    local p out=""
    for p in $GS017_PORTS; do
        _gs017_rpc "$p" getChainInfo '{}' >/dev/null 2>&1 && out="$out $p"
    done
    printf '%s' "${out# }"
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

# ── assertion 1: the CLI refuses before it signs ────────────────────────────

_gs017_cli_check() {
    local t="$1" bin port pick kind wallet pubkey bonds count head out msg tmp rc=0 n
    bin="$(_gs017_doli)" || bin=""
    if [ -z "$bin" ]; then
        SKIP_REASONS="$SKIP_REASONS; $t: doli CLI not resolvable (DOLI_CLI, \$HOME/testnet/bin/doli, PATH) — the client-side refusal cannot be exercised"
        return 2
    fi
    if [ -z "$(ls "$GS017_KEYS_DIR"/producer_*.json 2>/dev/null)" ]; then
        SKIP_REASONS="$SKIP_REASONS; $t: no producer_*.json wallet under $GS017_KEYS_DIR — nothing to submit with"
        return 2
    fi
    port="$(_gs017_first_producer_port)"
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
    if ! printf '%s' "$out" | grep -Eq 'headroom|cap|ADDBOND_CAP_EXCEEDED'; then
        FAIL_REASONS="$FAIL_REASONS; $t: add-bond --count $count exited $rc with no cap/headroom refusal — a transport error is not an admission decision: ${msg:-no output}"
        return 1
    fi
    n="$(_gs017_rpc "$port" getMempoolTransactions '[]' | _gs017_addbond_count)"
    n="$(printf '%s' "$n" | tr -dc '0-9-')"
    if [ -n "$n" ] && [ "$n" -gt 0 ]; then
        FAIL_REASONS="$FAIL_REASONS; $t: port $port holds $n addbond entr(ies) after the refusal — the CLI printed a refusal and submitted anyway"
        return 1
    fi
    INFO_REASONS="$INFO_REASONS; $t: CLI refused --count $count (headroom $head +1) for producer ${pubkey:0:8} at bond_count=$bonds on port $port, no addbond resident: ${msg:-refused}"
    return 0
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

# ── assertion 2: no addbond resident anywhere on the fleet ──────────────────

_gs017_residency_check() {
    local t="$1" p mem n answered=0 hits=""
    for p in $GS017_PORTS; do
        mem="$(_gs017_rpc "$p" getMempoolTransactions '[]')" || continue
        [ -n "$mem" ] || continue
        n="$(printf '%s' "$mem" | _gs017_addbond_count)"
        n="$(printf '%s' "$n" | tr -dc '0-9-')"
        { [ -n "$n" ] && [ "$n" -ge 0 ]; } || continue
        answered=$(( answered + 1 ))
        [ "$n" -gt 0 ] && hits="$hits $p($n)"
    done
    if [ "$answered" -eq 0 ]; then
        SKIP_REASONS="$SKIP_REASONS; $t: no node on ${GS017_PORTS%% *}-${GS017_PORTS##* } answered getMempoolTransactions — no mempool inspected"
        return 2
    fi
    if [ -n "$hits" ]; then
        FAIL_REASONS="$FAIL_REASONS; $t: addbond resident on port(s)$hits of $answered live mempool(s) — an AddBond one node rejected is still gossiped to the rest"
        return 1
    fi
    INFO_REASONS="$INFO_REASONS; $t: no addbond in any of $answered live mempool(s)"
    return 0
}

# ── assertion 3: no NEW cap poison in the observation window ────────────────

# _gs017_baseline_src — nodecfg (the runner's own pre-window offsets) · offsets
# (standalone GS017_OFFSETS file) · snapshot (no baseline, take one now).
_gs017_baseline_src() {
    if [ -n "${NODECFG:-}" ] && [ -r "${NODECFG:-}" ]; then echo nodecfg
    elif [ -n "${GS017_OFFSETS:-}" ] && [ -r "${GS017_OFFSETS:-}" ]; then echo offsets
    else echo snapshot; fi
}

# _gs017_baseline — 'name|logfile|byte-offset' per producer node.
_gs017_baseline() {
    local name off f
    case "$(_gs017_baseline_src)" in
        nodecfg)
            python3 -c "
import json
try:
    d = json.load(open('$NODECFG'))
except Exception:
    raise SystemExit(0)
for n in (d.get('nodes') or []):
    lf = n.get('logfile') or ''
    if lf:
        print('%s|%s|%s' % (n.get('name', ''), lf, n.get('offset', 0)))" 2>/dev/null ;;
        offsets)
            while IFS=: read -r name off; do
                [ -n "$name" ] || continue
                printf '%s|%s/%s.log|%s\n' "$name" "$GS017_LOG_DIR" "$name" "${off:-0}"
            done < "$GS017_OFFSETS" ;;
        *)
            for f in "$GS017_LOG_DIR"/n*.log; do
                [ -r "$f" ] || continue
                name="$(basename "$f" .log)"
                printf '%s|%s|%s\n' "$name" "$f" "$(wc -c < "$f" 2>/dev/null | tr -d ' ')"
            done ;;
    esac
}

_gs017_poison_check() {
    local t="$1" name logf off n src scanned=0 total=0 hits=""
    src="$(_gs017_baseline_src)"
    while IFS='|' read -r name logf off; do
        [ -n "$name" ] && [ -n "$logf" ] || continue
        case "$name" in n[0-9]*) ;; *) continue ;; esac
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
    if [ "$scanned" -eq 0 ]; then
        SKIP_REASONS="$SKIP_REASONS; $t: no readable node log under $GS017_LOG_DIR — no observation window to scan"
        return 2
    fi
    if [ -n "$hits" ]; then
        FAIL_REASONS="$FAIL_REASONS; $t: $total new [BLOCK_POISON] ADDBOND_CAP_EXCEEDED event(s) past the $src baseline on$hits — block assembly packed an over-cap AddBond again"
        return 1
    fi
    INFO_REASONS="$INFO_REASONS; $t: 0 new [BLOCK_POISON] ADDBOND_CAP_EXCEEDED events past the $src baseline across $scanned node log(s)"
    return 0
}

_gs017_assert() {
    local t="$1"
    case "$t" in
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

_gs017_main() {
    local t rc s_ok=1
    GS017_ECHO_CMD=1
    FAIL_REASONS=""; SKIP_REASONS=""; INFO_REASONS=""
    for t in gs017-cli-refuses-before-signing gs017-no-addbond-residency gs017-no-cap-poison-in-window; do
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
    _gs017_main
    exit $?
fi
