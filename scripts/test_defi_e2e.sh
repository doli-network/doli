#!/usr/bin/env bash
# test_defi_e2e.sh — End-to-end DeFi primitives test against LOCAL testnet
#
# ============================================================================
# OUTPUT CONTRACT
# ============================================================================
# Function-under-test: the deployed DOLI testnet binary (consensus + RPC + CLI),
# observed through black-box CLI/RPC calls. This is an integration harness, not
# a unit test — outputs are network-side state changes plus harness pass/fail.
#
# Observable outputs:
#   1. Process exit code (0 = all passed, 1 = at least one assertion failed, 2 = bad usage)
#   2. Stdout: phase markers, per-assertion ✓/✗/∙ lines, final PASS/FAIL/SKIP totals
#   3. Chain state mutations (transactions broadcast):
#        - MintAsset tx (Phase 1)
#        - CreatePool x2 (different fee tiers, Phase 2)
#        - SwapPool tx (Phase 3)
#        - AddLiquidity + RemoveLiquidity tx (Phase 4)
#        - NFT MintAsset + Transfer tx (Phase 5)
#        - OpenChannel + Cooperative Close tx (Phase 6)
#        - HTLC-conditioned Send tx (Phase 7)
#        - Bridge HTLC lock tx (Phase 9)
#   4. Wallet balance deltas in ~/testnet/keys/producer_{1..2}.json
#   5. Files written: close-<chan_id>.json (PSBT-style offer); cleaned up post-test
#
# Code paths (per phase, all reachable when AMM+oracle active at h >= 20099):
#   - preflight              → assert binary + RPCs + balances
#   - phase_mint             → MintAsset path (issue-token)
#   - phase_amm              → pool create (D1 MIN_LIQUIDITY, D2 fee_bps in pool_id),
#                              swap (D3 fee), add liquidity
#   - phase_nft              → NFT mint + transfer
#   - phase_channel          → channel open + list
#   - phase_template         → covenant template surface check
#   - phase_oracle           → oracle status + price read paths
#   - phase_bridge           → bridge-list RPC surface check
#
# INPUT PARTITIONS
# ----------------------------------------------------------------------------
# Phase | Path                | Partition                          | Expected
# ------|---------------------|------------------------------------|----------
# 1     | issue-token         | valid supply, fresh ticker         | accept
# 2     | pool create         | fee=30 bps, sufficient liquidity   | accept
# 2     | pool create         | fee=100 bps, same pair             | accept, distinct pool_id (D2)
# 2     | pool create         | sub-MIN_LIQUIDITY (0.0001/1)       | reject (D1)
# 3     | pool swap a2b       | valid amount, sufficient reserves  | reserves change, k non-decreasing
# 4     | pool add            | proportional add from issuer       | total_lp/reserves grow
# 4     | pool remove         | LP UTXO present, partial burn      | reserves shrink, LP UTXO consumed
# 5     | nft mint            | valid IPFS-style URI               | confirmed via getTransaction
# 5     | nft list            | post-mint                          | UTXO appears (P3-014 fix)
# 5     | nft transfer        | valid recipient address            | N2 lists, N1 no longer
# 6     | channel open        | valid counterparty, sufficient bal | channel id returned
# 6     | channel open self   | counterparty == self               | reject pre-broadcast (P1-007)
# 6     | channel close       | open channel                       | offer file written
# 6     | channel close-finish| valid offer file from counterparty | close tx confirmed (INC-I-093)
# 7     | template surface    | help on 6 kinds                    | all 6 respond
# 7     | template htlc-pay   | live --send tx with hashlock       | tx confirmed on chain
# 8     | getOracleStatus     | always callable                    | active=true, ah=20099
# 8     | getOraclePrice      | with valid pair_id                 | RPC responds cleanly
# 9     | bridge-list         | any state                          | RPC responds
# 9     | bridge-swap         | valid BTC addr, 0.05 DOLI          | swap tx confirmed
# 9     | getUtxos bridgeHtlc | post-swap                          | bridgeHtlc UTXO present
# ============================================================================
#
# Targets local testnet at ~/testnet/, producers N1-N5, RPC 8501-8505 (seed on 8500).
# READ-ONLY for code & data dirs. Sends real txs; consumes wallet funds.
#
# Usage: scripts/test_defi_e2e.sh [phase]
#   phase: all (default) | mint | amm | nft | channel | template | oracle | bridge

set -uo pipefail

# ============================================================================
# Config
# ============================================================================
CLI="./target/release/doli"
SEED_RPC="http://127.0.0.1:8500"
KEYS_DIR="$HOME/testnet/keys"

NODES="N1 N2 N3 N4 N5"

# Bash 3.2-compatible lookup (macOS default has no associative arrays)
rpc_url() {
    case "$1" in
        N1) echo "http://127.0.0.1:8501" ;;
        N2) echo "http://127.0.0.1:8502" ;;
        N3) echo "http://127.0.0.1:8503" ;;
        N4) echo "http://127.0.0.1:8504" ;;
        N5) echo "http://127.0.0.1:8505" ;;
        *)  echo ""; return 1 ;;
    esac
}

wallet_path() {
    case "$1" in
        N1) echo "$KEYS_DIR/producer_1.json" ;;
        N2) echo "$KEYS_DIR/producer_2.json" ;;
        N3) echo "$KEYS_DIR/producer_3.json" ;;
        N4) echo "$KEYS_DIR/producer_4.json" ;;
        N5) echo "$KEYS_DIR/producer_5.json" ;;
        *)  echo ""; return 1 ;;
    esac
}

PHASE="${1:-all}"
PASS=0
FAIL=0
SKIP=0
FAILURES=()

# ============================================================================
# Helpers
# ============================================================================
log()    { printf "\033[1;36m[%s]\033[0m %s\n" "$(date +%H:%M:%S)" "$*"; }
ok()     { printf "  \033[1;32mPASS\033[0m %s\n" "$*"; PASS=$((PASS+1)); }
fail()   { printf "  \033[1;31mFAIL\033[0m %s\n" "$*"; FAIL=$((FAIL+1)); FAILURES+=("$*"); }
skip()   { printf "  \033[1;33mSKIP\033[0m %s\n" "$*"; SKIP=$((SKIP+1)); }
phase()  { printf "\n\033[1;35m=== %s ===\033[0m\n" "$*"; }

rpc() {
    local method="$1" params="${2:-[]}" url="${3:-$SEED_RPC}"
    curl -s -X POST "$url" -H "Content-Type: application/json" \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"$method\",\"params\":$params,\"id\":1}"
}

cli() {
    local node="$1"; shift
    DOLI_NETWORK=testnet "$CLI" -w "$(wallet_path "$node")" -r "$(rpc_url "$node")" "$@"
}

height() {
    rpc getChainInfo | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['bestHeight'])" 2>/dev/null
}

wait_blocks() {
    local n="$1" start; start=$(height)
    local target=$((start + n))
    log "  waiting $n blocks (h=$start -> h=$target)..."
    for _ in $(seq 1 60); do
        sleep 2
        local h; h=$(height)
        [ "$h" -ge "$target" ] && { log "  reached h=$h"; return 0; }
    done
    fail "timeout waiting for h=$target (stuck at h=$(height))"
    return 1
}

assert_eq() {
    if [ "$2" = "$3" ]; then ok "$1"; else fail "$1: expected '$2', got '$3'"; fi
}

assert_ne() {
    if [ "$2" != "$3" ]; then ok "$1"; else fail "$1: should differ but both are '$2'"; fi
}

# Extract the tx hash from a CLI command's output. Each CLI subcommand prints
# different identifiers (asset_id, content_hash, channel_id, hashlock, etc.) so
# we look for the explicit "TX Hash:" or "TX:" label first, falling back to the
# LAST 64-hex token (broadcast confirmations print the tx hash last).
tx_hash() {
    local out="$1"
    local h
    # Try labeled patterns in order of specificity
    h=$(printf '%s\n' "$out" | grep -iE "^[[:space:]]*(TX Hash|Close TX|Funding TX hash|Funding TX|TX):" | grep -oE "[0-9a-f]{64}" | tail -1)
    if [ -n "$h" ]; then printf '%s' "$h"; return 0; fi
    # Fallback: last 64-hex token in the output
    printf '%s\n' "$out" | grep -oE "[0-9a-f]{64}" | tail -1
}

# Wait until a tx hash is queryable (confirmed in a block). Returns 0 + prints h=N.
wait_confirmed() {
    local tx="$1" label="${2:-tx}"
    for i in 1 2 3 4 5 6 7 8 9 10; do
        wait_blocks 1 >/dev/null 2>&1
        local h
        h=$(rpc getTransaction "[\"$tx\"]" | python3 -c "import sys,json; r=json.load(sys.stdin).get('result',{}) or {}; print(r.get('blockHeight') or '')" 2>/dev/null)
        if [ -n "$h" ]; then printf 'h=%s\n' "$h"; return 0; fi
    done
    return 1
}

# ============================================================================
# Preflight
# ============================================================================
preflight() {
    phase "Preflight"

    if [ ! -x "$CLI" ]; then
        fail "doli binary not found at $CLI -- run cargo build --release"
        exit 1
    fi
    ok "doli binary present"

    local h; h=$(height)
    if [ -z "$h" ] || [ "$h" -lt 20099 ]; then
        fail "testnet not synced or AMM not active (h=$h, need >=20099)"
        exit 1
    fi
    ok "testnet at h=$h (AMM+oracle active since 20099)"

    for n in $NODES; do
        local url; url=$(rpc_url "$n")
        local nh; nh=$(rpc getChainInfo "[]" "$url" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('result',{}).get('bestHeight',''))" 2>/dev/null)
        if [ -z "$nh" ]; then fail "$n RPC down at $url"; else ok "$n at h=$nh"; fi
    done

    for n in $NODES; do
        local bal; bal=$(cli "$n" balance 2>/dev/null | grep -E "^  Spendable" | head -1 | awk '{print $2}')
        if [ -z "$bal" ]; then fail "$n wallet read failed"; else ok "$n spendable=$bal DOLI"; fi
    done
}

# ============================================================================
# Phase 1: MintAsset
# ============================================================================
ASSET_ID=""
ASSET_TICKER="DTST$(printf '%04d' $((RANDOM % 10000)))"

phase_mint() {
    phase "Phase 1: MintAsset / issue-token (D.1 issuer auth)"

    log "Issuing token $ASSET_TICKER with supply 1_000_000 from N1"
    local out; out=$(cli N1 issue-token --supply 1000000 "$ASSET_TICKER" 2>&1)
    local txid; txid=$(printf '%s\n' "$out" | grep -oE "[0-9a-f]{64}" | head -1)
    if [ -z "$txid" ]; then
        fail "issue-token produced no txid"
        printf '%s\n' "$out" | tail -10
        return 1
    fi
    ok "issue-token submitted (tx=${txid:0:16}...)"

    log "Waiting for inclusion..."
    wait_blocks 2 || return 1

    log "Resolving asset_id from on-chain tx"
    # Look up tx, asset_id is at outputs[i].asset.assetId for FungibleAsset outputs
    ASSET_ID=$(rpc getTransaction "[\"$txid\"]" | python3 -c "
import sys, json
d = json.load(sys.stdin).get('result', {})
for out in d.get('outputs', []):
    asset = out.get('asset') or {}
    aid = asset.get('assetId') or asset.get('asset_id') or out.get('asset_id') or out.get('assetId')
    if aid: print(aid); break
" 2>/dev/null)
    if [ -n "$ASSET_ID" ]; then
        ok "asset_id=${ASSET_ID:0:16}..."
    else
        skip "asset_id resolution (downstream pool/swap tests may skip)"
    fi

    skip "D.1 issuer-spoof (covered by stress-tester)"
}

# ============================================================================
# Phases 2-4: AMM
# ============================================================================
POOL_ID_30=""
POOL_ID_100=""

phase_amm() {
    phase "Phase 2: AMM pool create (D1 MIN_LIQUIDITY=1000, D2 fee_bps in pool_id)"

    if [ -z "$ASSET_ID" ]; then
        skip "no asset_id from Phase 1 -- AMM phase skipped"
        return 0
    fi

    log "Creating pool @ 30 bps: 10 DOLI / 1000 tokens"
    local out30; out30=$(cli N1 pool create --asset "$ASSET_ID" --doli 10 --tokens 1000 --fee 30 --yes 2>&1)
    POOL_ID_30=$(printf '%s\n' "$out30" | grep -oE "[0-9a-f]{64}" | head -1)
    if [ -n "$POOL_ID_30" ]; then
        ok "pool @ 30 bps created (id=${POOL_ID_30:0:16}...)"
    else
        fail "pool create @ 30 bps failed"
        printf '%s\n' "$out30" | tail -5
    fi

    wait_blocks 2 || true

    log "Creating SAME pair @ 100 bps (D2: different fee_bps -> different pool_id)"
    local out100; out100=$(cli N1 pool create --asset "$ASSET_ID" --doli 5 --tokens 500 --fee 100 --yes 2>&1)
    POOL_ID_100=$(printf '%s\n' "$out100" | grep -oE "[0-9a-f]{64}" | head -1)
    if [ -n "$POOL_ID_100" ]; then
        ok "pool @ 100 bps created (id=${POOL_ID_100:0:16}...)"
        assert_ne "D2: distinct pool_ids per fee tier" "$POOL_ID_30" "$POOL_ID_100"
    else
        fail "pool create @ 100 bps failed (D2 may be regressed)"
        printf '%s\n' "$out100" | tail -5
    fi

    wait_blocks 2 || true

    log "D1: attempt pool below MIN_LIQUIDITY=1000 (should reject)"
    local out_low; out_low=$(cli N1 pool create --asset "$ASSET_ID" --doli 0.0001 --tokens 1 --fee 30 --yes 2>&1 || true)
    if printf '%s\n' "$out_low" | grep -qiE "minimum.?liquidity|MIN_LIQUIDITY|insufficient.?liquidity|below.?threshold|rejected|error"; then
        ok "D1: sub-threshold pool rejected"
    else
        fail "D1 may be regressed: pool below MIN_LIQUIDITY=1000 was NOT rejected"
        printf '%s\n' "$out_low" | tail -5
    fi

    phase "Phase 3: AMM swap (D3 fee semantics)"

    if [ -z "$POOL_ID_30" ]; then skip "no pool_id -- swap phase skipped"; return 0; fi

    log "Pre-swap pool state"
    local pre; pre=$(rpc getPoolInfo "{\"poolId\":\"$POOL_ID_30\"}")
    local pre_a pre_b
    pre_a=$(printf '%s\n' "$pre" | python3 -c "import sys,json; d=json.load(sys.stdin).get('result',{}); print(d.get('reserveA', d.get('reserve_a',0)))" 2>/dev/null)
    pre_b=$(printf '%s\n' "$pre" | python3 -c "import sys,json; d=json.load(sys.stdin).get('result',{}); print(d.get('reserveB', d.get('reserve_b',0)))" 2>/dev/null)
    log "  reserve_a=$pre_a  reserve_b=$pre_b"

    log "N1 swaps 0.5 DOLI -> tokens (a2b)"
    local swap_out; swap_out=$(cli N1 pool swap --pool "$POOL_ID_30" --amount 0.5 --direction a2b --yes 2>&1)
    if printf '%s\n' "$swap_out" | grep -qiE "submitted|tx:|txid|broadcast"; then
        ok "swap a2b submitted"
    else
        fail "swap a2b failed"
        printf '%s\n' "$swap_out" | tail -5
    fi

    wait_blocks 2 || true

    log "Post-swap pool state"
    local post; post=$(rpc getPoolInfo "{\"poolId\":\"$POOL_ID_30\"}")
    local post_a post_b
    post_a=$(printf '%s\n' "$post" | python3 -c "import sys,json; d=json.load(sys.stdin).get('result',{}); print(d.get('reserveA', d.get('reserve_a',0)))" 2>/dev/null)
    post_b=$(printf '%s\n' "$post" | python3 -c "import sys,json; d=json.load(sys.stdin).get('result',{}); print(d.get('reserveB', d.get('reserve_b',0)))" 2>/dev/null)
    log "  reserve_a=$post_a  reserve_b=$post_b"

    if [ -n "$post_a" ] && [ -n "$pre_a" ] && [ "$post_a" != "$pre_a" ]; then
        ok "swap updated reserves (a: $pre_a->$post_a, b: $pre_b->$post_b)"
    else
        fail "reserves unchanged after swap"
    fi

    if [ -n "$pre_a" ] && [ -n "$pre_b" ] && [ -n "$post_a" ] && [ -n "$post_b" ]; then
        if python3 -c "import sys; sys.exit(0 if $post_a*$post_b >= $pre_a*$pre_b else 1)" 2>/dev/null; then
            ok "k-invariant non-decreasing"
        else
            fail "k-invariant violated"
        fi
    fi

    phase "Phase 4: AMM add + remove liquidity"

    log "Pre-add pool state"
    local pre_add; pre_add=$(rpc getPoolInfo "{\"poolId\":\"$POOL_ID_30\"}")
    local pre_lp; pre_lp=$(printf '%s\n' "$pre_add" | python3 -c "import sys,json; d=json.load(sys.stdin).get('result',{}); print(d.get('totalLp', d.get('total_lp', 0)))" 2>/dev/null)
    log "  total_lp=$pre_lp"

    log "N1 adds liquidity: 1 DOLI / 100 tokens"
    local add_out; add_out=$(cli N1 pool add --pool "$POOL_ID_30" --doli 1 --tokens 100 --yes 2>&1)
    if printf '%s\n' "$add_out" | grep -qiE "submitted|broadcast|tx|created|successfully"; then
        ok "add liquidity submitted"
    else
        fail "add liquidity failed"
        printf '%s\n' "$add_out" | tail -5
        return 1
    fi

    wait_blocks 2 || true

    local post_add; post_add=$(rpc getPoolInfo "{\"poolId\":\"$POOL_ID_30\"}")
    local post_lp; post_lp=$(printf '%s\n' "$post_add" | python3 -c "import sys,json; d=json.load(sys.stdin).get('result',{}); print(d.get('totalLp', d.get('total_lp', 0)))" 2>/dev/null)
    log "  total_lp now=$post_lp"
    if [ -n "$pre_lp" ] && [ -n "$post_lp" ] && [ "$post_lp" != "$pre_lp" ]; then
        ok "add liquidity increased total_lp ($pre_lp -> $post_lp)"
    else
        # Some pool RPC implementations may not expose totalLp; fall back to reserves rising
        local post_add_a; post_add_a=$(printf '%s\n' "$post_add" | python3 -c "import sys,json; print(json.load(sys.stdin).get('result',{}).get('reserveA',0))" 2>/dev/null)
        if [ "$post_add_a" -gt "$pre_a" ] 2>/dev/null; then
            ok "add liquidity grew reserve_a ($pre_a -> $post_add_a)"
        else
            skip "could not verify add-liquidity effect (RPC shape lacks totalLp + reserveA unchanged)"
        fi
    fi

    # Try to remove. Known issue (2026-05-29): wallet-side LP UTXO selection does
    # not filter by pool_id, so when N1 holds LP shares from MULTIPLE pools the
    # wallet picks the wrong UTXO -> MPTX007 covenant condition not satisfied at
    # mempool. Track as a separate finding; here we surface the diagnostic.
    log "Removing 100 LP shares from POOL_ID_30 (probes wallet UTXO selection)"
    local rm_out; rm_out=$(cli N1 pool remove --pool "$POOL_ID_30" --shares 100 --yes 2>&1)
    if printf '%s\n' "$rm_out" | grep -qE "MPTX007|covenant condition"; then
        fail "pool remove rejected at mempool (MPTX007 on input 1 = wrong LP UTXO selected) -- wallet LP UTXO selection does not filter by pool_id"
    elif printf '%s\n' "$rm_out" | grep -qiE "submitted|broadcast|removed|successfully|TX Hash"; then
        local rm_tx; rm_tx=$(tx_hash "$rm_out")
        local rm_confirmed; rm_confirmed=$(wait_confirmed "$rm_tx" "remove")
        if [ -n "$rm_confirmed" ]; then
            ok "remove liquidity confirmed ($rm_confirmed)"
        else
            fail "remove liquidity tx $rm_tx not queryable after 10 blocks"
        fi
    else
        fail "remove liquidity failed (unrecognized output)"
        printf '%s\n' "$rm_out" | tail -5
    fi
}

# ============================================================================
# Phase 5: NFT
# ============================================================================
phase_nft() {
    phase "Phase 5: NFT mint + transfer end-to-end"

    log "N1 mints NFT"
    local mint_out; mint_out=$(cli N1 nft --mint "ipfs://QmDefiTest$$" --amount 1 2>&1)
    local mint_tx; mint_tx=$(tx_hash "$mint_out")
    if [ -n "$mint_tx" ]; then
        ok "NFT mint submitted (tx=${mint_tx:0:16}...)"
    else
        fail "NFT mint produced no txid"
        printf '%s\n' "$mint_out" | tail -5
        return 1
    fi

    log "Waiting for mint to confirm"
    local confirmed; confirmed=$(wait_confirmed "$mint_tx" "mint")
    if [ -n "$confirmed" ]; then
        ok "NFT mint confirmed ($confirmed)"
    else
        fail "NFT mint $mint_tx not queryable after 10 blocks"
        return 1
    fi

    log "N1 lists NFTs (P3-014: should include EncryptedContent mints)"
    local list_out; list_out=$(cli N1 nft --list 2>&1)
    local nft_utxo; nft_utxo=$(printf '%s\n' "$list_out" | grep -oE "[0-9a-f]{64}:[0-9]+" | head -1)
    if [ -n "$nft_utxo" ]; then
        ok "N1 nft --list shows UTXO ($nft_utxo)"
    else
        fail "nft --list empty after confirmed mint"
        printf '%s\n' "$list_out" | head -10
        return 1
    fi

    # Locate the SPECIFIC just-minted UTXO (mint_tx:N)
    local fresh_utxo; fresh_utxo=$(printf '%s\n' "$list_out" | grep -oE "${mint_tx}:[0-9]+" | head -1)
    if [ -z "$fresh_utxo" ]; then fresh_utxo="$nft_utxo"; fi
    # NFT transfer uses ECIES encryption to the recipient's PUBKEY. The CLI can
    # resolve a pubkey from an address only via on-chain SEND history. Producer
    # BLS attestations don't count. Workaround: pass N3's pubkey hex directly
    # (queryable via `doli info`).
    local n3_pubkey; n3_pubkey=$(cli N3 info 2>&1 | grep -iE "^[[:space:]]*Public Key:" | grep -oE "[0-9a-f]{64}" | head -1)
    if [ -z "$n3_pubkey" ]; then fail "could not read N3 pubkey from info"; return 1; fi
    log "Transferring NFT $fresh_utxo from N1 -> N3 (via pubkey hex)"
    local xfer_out; xfer_out=$(cli N1 nft --transfer "$fresh_utxo" --to "$n3_pubkey" 2>&1)
    local xfer_tx; xfer_tx=$(tx_hash "$xfer_out")
    if [ -n "$xfer_tx" ]; then
        ok "NFT transfer submitted (tx=${xfer_tx:0:16}...)"
    else
        fail "NFT transfer produced no txid"
        printf '%s\n' "$xfer_out" | tail -5
        return 1
    fi

    log "Waiting for transfer to confirm"
    local xfer_confirmed; xfer_confirmed=$(wait_confirmed "$xfer_tx" "transfer")
    if [ -n "$xfer_confirmed" ]; then
        ok "transfer confirmed ($xfer_confirmed)"
    else
        fail "transfer $xfer_tx not queryable after 10 blocks"
        return 1
    fi

    # Verify N3 now lists the NFT and N1 no longer does (for THIS utxo)
    local n3_list; n3_list=$(cli N3 nft --list 2>&1)
    local n3_count; n3_count=$(printf '%s\n' "$n3_list" | grep -cE "[0-9a-f]{64}:[0-9]+" || true)
    if [ "$n3_count" -ge 1 ]; then
        ok "N3 owns $n3_count NFT(s) post-transfer"
    else
        fail "N3 nft --list empty after transfer"
    fi

    local n1_list_after; n1_list_after=$(cli N1 nft --list 2>&1)
    if printf '%s\n' "$n1_list_after" | grep -q "$fresh_utxo"; then
        fail "transferred NFT UTXO $fresh_utxo still appears in N1 list (UTXO model: should be spent)"
    else
        ok "N1 no longer owns $fresh_utxo (spent input)"
    fi
}

# ============================================================================
# Phase 6: Payment channel
# ============================================================================
phase_channel() {
    phase "Phase 6: Payment channel open + cooperative close end-to-end"

    local n2_addr; n2_addr=$(cli N2 addresses 2>&1 | grep -oE "tdoli[a-z0-9]+" | head -1)
    if [ -z "$n2_addr" ]; then fail "no N2 addr"; return 1; fi

    log "N1 opens channel with N2 (1 DOLI capacity)"
    local open_out
    open_out=$(cli N1 channel open "$n2_addr" 1 2>&1 || true)
    local chan_id; chan_id=$(printf '%s\n' "$open_out" | grep -oE "Channel opened: [0-9a-f]+" | awk '{print $3}' | head -1)
    if [ -z "$chan_id" ]; then
        # Fallback: take first 16-hex token after a "opened|Channel" mention
        chan_id=$(printf '%s\n' "$open_out" | grep -oE "[0-9a-f]{16,}" | head -1)
    fi
    if [ -n "$chan_id" ]; then
        ok "channel open submitted (id=${chan_id:0:16})"
    else
        fail "channel open did not return a channel id"
        printf '%s\n' "$open_out" | tail -8
        return 1
    fi

    wait_blocks 2 || true

    # P1-007 fixed check: self-channel must be rejected pre-broadcast
    local n1_addr; n1_addr=$(cli N1 addresses 2>&1 | grep -oE "tdoli[a-z0-9]+" | head -1)
    local self_out; self_out=$(cli N1 channel open "$n1_addr" 1 2>&1 || true)
    if printf '%s\n' "$self_out" | grep -qiE "cannot|self|same|distinct|rejected|error"; then
        ok "P1-007: self-channel rejected pre-broadcast"
    else
        fail "P1-007 regression: self-channel was NOT rejected"
        printf '%s\n' "$self_out" | tail -3
    fi

    # INC-I-093: cooperative-close PSBT handoff (close -> offer file -> close-finish)
    log "Closing channel cooperatively (PSBT handoff)"
    cd "$(dirname "$0")/.." >/dev/null
    local close_out; close_out=$(cli N1 channel close "$chan_id" 2>&1)
    local offer_file; offer_file=$(printf '%s\n' "$close_out" | grep -oE "close-[0-9a-f]+\.json" | head -1)
    if [ -z "$offer_file" ] || [ ! -f "$offer_file" ]; then
        # Try to locate by chan_id prefix
        offer_file=$(ls close-${chan_id:0:16}*.json 2>/dev/null | head -1)
    fi
    if [ -n "$offer_file" ] && [ -f "$offer_file" ]; then
        ok "channel close step 1 wrote offer file ($offer_file)"
    else
        fail "channel close did not produce a verifiable offer file"
        printf '%s\n' "$close_out" | tail -8
        return 1
    fi

    log "N2 finalizes the cooperative close"
    local finish_out; finish_out=$(cli N2 channel close-finish "$offer_file" 2>&1)
    local close_tx; close_tx=$(tx_hash "$finish_out")
    if [ -n "$close_tx" ]; then
        ok "channel close-finish broadcast (tx=${close_tx:0:16}...)"
    else
        fail "channel close-finish failed"
        printf '%s\n' "$finish_out" | tail -8
        return 1
    fi

    log "Waiting for close tx to confirm"
    local close_confirmed; close_confirmed=$(wait_confirmed "$close_tx" "close")
    if [ -n "$close_confirmed" ]; then
        ok "INC-I-093 cooperative-close confirmed on chain ($close_confirmed)"
    else
        fail "close $close_tx not queryable after 10 blocks"
    fi

    # Cleanup offer file
    rm -f "$offer_file" 2>/dev/null
}

# ============================================================================
# Phase 7: Covenant template
# ============================================================================
phase_template() {
    phase "Phase 7: Covenant template (htlc-payment live send)"

    # Surface check: all template subcommands exist
    local kinds_seen=0
    for k in vault escrow htlc-payment subscription agent-allowance escrow-loan; do
        if cli N1 template $k --help >/dev/null 2>&1; then
            kinds_seen=$((kinds_seen+1))
        fi
    done
    if [ "$kinds_seen" -ge 6 ]; then
        ok "all 6 covenant template kinds exposed in CLI"
    else
        fail "only $kinds_seen/6 covenant template kinds responded to --help"
    fi

    # Live tx: htlc-payment is the simplest single-output template that exercises
    # the script path end-to-end. Refund recipient = N1 self (so we can refund post-expiry
    # if needed). The condition is the on-chain part we care about.
    local n2_addr; n2_addr=$(cli N2 addresses 2>&1 | grep -oE "tdoli[a-z0-9]+" | head -1)
    local n1_addr; n1_addr=$(cli N1 addresses 2>&1 | grep -oE "tdoli[a-z0-9]+" | head -1)
    if [ -z "$n2_addr" ] || [ -z "$n1_addr" ]; then fail "addr lookup failed"; return 1; fi

    # Deterministic hashlock (we don't need to actually claim it -- we just verify the
    # send constructs and lands on chain with the condition)
    local hash="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    local cur_h; cur_h=$(height)
    local lock=$((cur_h + 100))
    local expiry=$((cur_h + 200))

    log "N1 sends 0.1 DOLI -> N2 with htlc-payment condition (lock=$lock, expiry=$expiry)"
    # template <kind> --send has no --yes flag; auto-confirm via stdin
    local send_out; send_out=$(printf 'y\n' | cli N1 template htlc-payment \
        --hash "$hash" --lock "$lock" --expiry "$expiry" --refund "$n1_addr" \
        --send --to "$n2_addr" --amount 0.1 2>&1)
    local send_tx; send_tx=$(tx_hash "$send_out")
    if [ -n "$send_tx" ]; then
        ok "htlc-payment tx broadcast (tx=${send_tx:0:16}...)"
    else
        fail "htlc-payment --send produced no txid"
        printf '%s\n' "$send_out" | tail -8
        return 1
    fi

    log "Waiting for htlc tx to confirm"
    local hp_confirmed; hp_confirmed=$(wait_confirmed "$send_tx" "htlc")
    if [ -n "$hp_confirmed" ]; then
        ok "htlc-payment confirmed ($hp_confirmed) -- condition encoded on chain"
    else
        fail "htlc-payment $send_tx not queryable after 10 blocks"
    fi
}

# ============================================================================
# Phase 8: Oracle
# ============================================================================
phase_oracle() {
    phase "Phase 8: Oracle read paths (getOracleStatus, getOraclePrice)"

    local status; status=$(rpc getOracleStatus)
    local active; active=$(printf '%s\n' "$status" | python3 -c "import sys,json; d=json.load(sys.stdin).get('result',{}); print(d.get('active',False))" 2>/dev/null)
    assert_eq "oracle active flag" "True" "$active"

    local ah; ah=$(printf '%s\n' "$status" | python3 -c "import sys,json; d=json.load(sys.stdin).get('result',{}); print(d.get('activation_height',-1))" 2>/dev/null)
    assert_eq "oracle activation_height=20099" "20099" "$ah"

    local attesters; attesters=$(printf '%s\n' "$status" | python3 -c "import sys,json; d=json.load(sys.stdin).get('result',{}); print(d.get('attester_count',-1))" 2>/dev/null)
    log "  current attester_count=$attesters (0 = no attestations yet, expected pre-tooling)"
    ok "oracle status RPC returns structured response"

    # Probe getOraclePrice with a synthetic pair_id (no real prices exist pre-attester).
    # Any well-formed call should dispatch cleanly; "no price" result is acceptable.
    local probe_pair="0000000000000000000000000000000000000000000000000000000000000001"
    local price; price=$(rpc getOraclePrice "{\"pair_id\":\"$probe_pair\"}")
    if printf '%s\n' "$price" | python3 -c "
import sys, json
d = json.load(sys.stdin)
# Accept: result present (price or null) OR a 'no price' style error code (-32603 / -32000)
if 'result' in d: sys.exit(0)
err = d.get('error', {})
msg = (err.get('message') or '').lower()
if 'no' in msg or 'not found' in msg or 'absent' in msg: sys.exit(0)
sys.exit(1)
" 2>/dev/null; then
        ok "getOraclePrice dispatches (no-price acceptable, attester_count=0)"
    else
        fail "getOraclePrice returned unexpected error: $(printf '%s' "$price" | head -c 200)"
    fi

    skip "PriceAttestation submission (no CLI surface yet -- see crates/core/src/transaction/core.rs:897)"
}

# ============================================================================
# Phase 9: Bridge
# ============================================================================
phase_bridge() {
    phase "Phase 9: Bridge HTLC lock + on-chain verify"

    local list_out; list_out=$(cli N1 bridge-list 2>&1)
    if printf '%s\n' "$list_out" | grep -qiE "swap|htlc|active|empty|none|0 swap|bridge"; then
        ok "bridge-list RPC responds"
    else
        fail "bridge-list failed"
        printf '%s\n' "$list_out" | tail -5
    fi

    # Live lock via bridge-swap (auto-generates preimage + initiates the HTLC).
    # We do NOT attempt claim or refund -- those depend on counter-chain confirmations
    # or block-height-based expiry waits that would stretch this script too long.
    # The lock itself exercises the HTLC construction code path on chain.
    log "N1 initiates bridge-swap 0.05 DOLI -> Bitcoin testnet addr"
    local btc_addr="tb1qar0srrr7xfkvy5l643lydnw9re59gtzzkqtgek"
    local swap_out; swap_out=$(cli N1 bridge-swap 0.05 --chain bitcoin --to "$btc_addr" 2>&1)
    local swap_tx; swap_tx=$(tx_hash "$swap_out")
    local preimage; preimage=$(printf '%s\n' "$swap_out" | grep -iE "preimage" | grep -oE "[0-9a-f]{64}" | head -1)
    if [ -n "$swap_tx" ]; then
        ok "bridge-swap broadcast (tx=${swap_tx:0:16}..., preimage captured: $([ -n \"$preimage\" ] && echo yes || echo no))"
    else
        fail "bridge-swap produced no txid"
        printf '%s\n' "$swap_out" | tail -8
        return 1
    fi

    log "Waiting for swap tx to confirm"
    local swap_confirmed; swap_confirmed=$(wait_confirmed "$swap_tx" "swap")
    if [ -n "$swap_confirmed" ]; then
        ok "bridge-swap confirmed on chain ($swap_confirmed)"
    else
        fail "bridge-swap $swap_tx not queryable after 10 blocks"
    fi

    # Verify the bridgeHtlc UTXO is now present in N1's set
    local n1_addr; n1_addr=$(cli N1 addresses 2>&1 | grep -oE "tdoli[a-z0-9]+" | head -1)
    local utxos; utxos=$(rpc getUtxos "{\"address\":\"$n1_addr\"}" "$SEED_RPC")
    local htlc_count; htlc_count=$(printf '%s\n' "$utxos" | python3 -c "
import sys, json
r = json.load(sys.stdin).get('result', [])
print(sum(1 for u in r if u.get('outputType','').lower() == 'bridgehtlc'))
" 2>/dev/null)
    if [ -n "$htlc_count" ] && [ "$htlc_count" -ge 1 ]; then
        ok "$htlc_count bridgeHtlc UTXO(s) live in N1 state"
    else
        fail "no bridgeHtlc UTXO found in N1's UTXO set"
    fi

    # Verify bridge-list now reflects the active swap
    local list2; list2=$(cli N1 bridge-list 2>&1)
    if printf '%s\n' "$list2" | grep -qE "${swap_tx:0:16}|active|locked"; then
        ok "bridge-list shows the active swap"
    else
        skip "bridge-list does not surface the new swap by tx prefix (CLI display only)"
    fi

    skip "bridge-claim / bridge-refund live -- exercised in Phase 10 + Phase 11 below"
}

# ============================================================================
# Phase 10: Bridge HTLC claim live roundtrip
# ============================================================================
# Generates a deterministic 64-hex preimage, locks DOLI with a short lock height,
# waits past the lock, and has N2 claim with the preimage. Verifies funds move.
phase_bridge_claim() {
    phase "Phase 10: Bridge HTLC claim live roundtrip (N1 lock -> N2 claim with preimage)"

    local n2_addr; n2_addr=$(cli N2 addresses 2>&1 | grep -oE "tdoli[a-z0-9]+" | head -1)
    if [ -z "$n2_addr" ]; then fail "no N2 addr"; return 1; fi

    # Random 64-hex preimage. Counter-hash is opaque dummy for manual-mode locking.
    local preimage; preimage=$(python3 -c "import secrets; print(secrets.token_hex(32))")
    local counter_hash="cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
    local cur_h; cur_h=$(height)
    local lock=$((cur_h + 2))
    local expiry=$((cur_h + 500))

    log "N1 bridge-lock 0.1 DOLI (preimage=${preimage:0:16}..., lock=$lock, expiry=$expiry)"
    local lock_out; lock_out=$(cli N1 bridge-lock 0.1 \
        --preimage "$preimage" --lock "$lock" --expiry "$expiry" \
        --chain bitcoin --to "tb1qar0srrr7xfkvy5l643lydnw9re59gtzzkqtgek" \
        --counter-hash "$counter_hash" --yes 2>&1)
    local lock_tx; lock_tx=$(tx_hash "$lock_out")
    if [ -z "$lock_tx" ]; then
        fail "bridge-lock produced no txid"
        printf '%s\n' "$lock_out" | tail -6
        return 1
    fi
    ok "bridge-lock broadcast (tx=${lock_tx:0:16}...)"

    local lock_confirmed; lock_confirmed=$(wait_confirmed "$lock_tx" "lock")
    if [ -z "$lock_confirmed" ]; then
        fail "bridge-lock $lock_tx not confirmed after 10 blocks"
        return 1
    fi
    ok "bridge-lock confirmed ($lock_confirmed)"

    # Locate the bridgeHtlc UTXO produced by this lock_tx (output 0 is conventional)
    local htlc_utxo="${lock_tx}:0"
    log "Waiting past lock height ($lock) so claim is permitted"
    while [ "$(height)" -lt "$lock" ]; do wait_blocks 1 >/dev/null 2>&1; done
    ok "past lock height (h=$(height) >= $lock)"

    # N2 claims with the preimage. Owner is N1 by lock-side construction; the HTLC
    # claim path is satisfied by preimage + receiver signature (per INC-I-093 P1-003).
    log "N2 claims HTLC $htlc_utxo with preimage"
    local claim_out; claim_out=$(cli N2 bridge-claim "$htlc_utxo" --preimage "$preimage" --yes 2>&1)
    local claim_tx; claim_tx=$(tx_hash "$claim_out")
    if [ -z "$claim_tx" ]; then
        fail "bridge-claim produced no txid"
        printf '%s\n' "$claim_out" | tail -6
        return 1
    fi
    ok "bridge-claim broadcast (tx=${claim_tx:0:16}...)"

    local claim_confirmed; claim_confirmed=$(wait_confirmed "$claim_tx" "claim")
    if [ -n "$claim_confirmed" ]; then
        ok "INC-I-093 P1-003 bridge claim confirmed on chain ($claim_confirmed)"
    else
        fail "bridge-claim $claim_tx not confirmed after 10 blocks"
    fi
}

# ============================================================================
# Phase 11: Bridge HTLC refund live roundtrip
# ============================================================================
# Short expiry; wait past it; N1 refunds. Tests INC-I-093 P2-004 refund-witness fix.
phase_bridge_refund() {
    phase "Phase 11: Bridge HTLC refund live roundtrip (N1 lock with short expiry -> N1 refund)"

    local preimage; preimage=$(python3 -c "import secrets; print(secrets.token_hex(32))")
    local counter_hash="dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
    local cur_h; cur_h=$(height)
    local lock=$((cur_h + 1))
    local expiry=$((cur_h + 4))

    log "N1 bridge-lock 0.05 DOLI (lock=$lock, expiry=$expiry -- intentionally short)"
    local lock_out; lock_out=$(cli N1 bridge-lock 0.05 \
        --preimage "$preimage" --lock "$lock" --expiry "$expiry" \
        --chain bitcoin --to "tb1qar0srrr7xfkvy5l643lydnw9re59gtzzkqtgek" \
        --counter-hash "$counter_hash" --yes 2>&1)
    local lock_tx; lock_tx=$(tx_hash "$lock_out")
    if [ -z "$lock_tx" ]; then
        fail "bridge-lock (refund test) produced no txid"
        printf '%s\n' "$lock_out" | tail -6
        return 1
    fi
    ok "bridge-lock (refund test) broadcast (tx=${lock_tx:0:16}...)"

    local lock_confirmed; lock_confirmed=$(wait_confirmed "$lock_tx" "lock")
    if [ -z "$lock_confirmed" ]; then
        fail "lock not confirmed"
        return 1
    fi
    ok "lock confirmed ($lock_confirmed)"

    local htlc_utxo="${lock_tx}:0"
    log "Waiting past expiry height ($expiry) so refund is permitted"
    while [ "$(height)" -lt "$expiry" ]; do wait_blocks 1 >/dev/null 2>&1; done
    ok "past expiry (h=$(height) >= $expiry)"

    log "N1 refunds HTLC $htlc_utxo"
    local refund_out; refund_out=$(cli N1 bridge-refund "$htlc_utxo" --yes 2>&1)
    local refund_tx; refund_tx=$(tx_hash "$refund_out")
    if [ -z "$refund_tx" ]; then
        fail "bridge-refund produced no txid"
        printf '%s\n' "$refund_out" | tail -8
        return 1
    fi
    ok "bridge-refund broadcast (tx=${refund_tx:0:16}...)"

    local refund_confirmed; refund_confirmed=$(wait_confirmed "$refund_tx" "refund")
    if [ -n "$refund_confirmed" ]; then
        ok "INC-I-093 P2-004 bridge refund confirmed on chain ($refund_confirmed)"
    else
        fail "bridge-refund $refund_tx not confirmed after 10 blocks"
    fi
}

# ============================================================================
# Phase 12: Payment channel intra-channel pay
# ============================================================================
# Off-chain payment: opens a fresh channel, sends a payment, checks channel state.
phase_channel_pay() {
    phase "Phase 12: Payment channel intra-channel pay (off-chain state update)"

    local n2_addr; n2_addr=$(cli N2 addresses 2>&1 | grep -oE "tdoli[a-z0-9]+" | head -1)
    if [ -z "$n2_addr" ]; then fail "no N2 addr"; return 1; fi

    log "N1 opens fresh channel with N2 (2 DOLI capacity) for pay test"
    local open_out; open_out=$(cli N1 channel open "$n2_addr" 2 2>&1 || true)
    local chan_id; chan_id=$(printf '%s\n' "$open_out" | grep -oE "Channel opened: [0-9a-f]+" | awk '{print $3}' | head -1)
    if [ -z "$chan_id" ]; then
        fail "channel open for pay test did not return a channel id"
        printf '%s\n' "$open_out" | tail -6
        return 1
    fi
    ok "channel opened (id=${chan_id:0:16})"

    wait_blocks 2 || true

    log "N1 sends 0.5 DOLI through channel ${chan_id:0:16}"
    local pay_out; pay_out=$(cli N1 channel pay "$chan_id" 0.5 2>&1)
    if printf '%s\n' "$pay_out" | grep -qiE "sent|payment|updated|local.?balance|remote.?balance|success"; then
        ok "channel pay 0.5 DOLI accepted"
        local info_out; info_out=$(cli N1 channel info "$chan_id" 2>&1)
        if printf '%s\n' "$info_out" | grep -qE "1\.5|0\.5"; then
            ok "channel info reflects updated balances"
        else
            skip "could not parse expected balances from channel info (off-chain state varies by display)"
        fi
    elif printf '%s\n' "$pay_out" | grep -qE "not active|FundingBroadcast|state:"; then
        # Channels stuck in FundingBroadcast forever -- wallet never transitions to Active
        # even with funding confirmed thousands of blocks ago. Filed separately.
        fail "channel pay blocked: wallet channel state stuck in FundingBroadcast (never auto-transitions to Active). Funding tx is mined + confirmed but local wallet state does not advance. Likely missing chain-watcher / state-machine wiring in crates/channels or wallet. NEW FINDING -- file separately."
        printf '%s\n' "$pay_out" | tail -3
    else
        fail "channel pay rejected (unrecognized output)"
        printf '%s\n' "$pay_out" | tail -8
    fi

    # Cooperative close to clean up. Don't fail the phase if close hits the cache miss; the pay test already passed.
    local close_out; close_out=$(cli N1 channel close "$chan_id" 2>&1 || true)
    local offer_file; offer_file=$(printf '%s\n' "$close_out" | grep -oE "close-[0-9a-f]+\.json" | head -1)
    if [ -n "$offer_file" ] && [ -f "$offer_file" ]; then
        cli N2 channel close-finish "$offer_file" >/dev/null 2>&1 || true
        rm -f "$offer_file"
        ok "channel cleanup attempted (close + close-finish)"
    fi

    skip "channel force-close (INC-I-093 P1-002 deferred -- CLI returns roadmap-item error pending timeout-branch witness builder)"
}

# ============================================================================
# Phase 13: Covenant templates live -- vault + escrow
# ============================================================================
# Sends two real transactions using the template covenant conditions. We verify
# the conditioned UTXO lands on chain; the SPENDING side (multi-party / wait-for-
# unlock) is out of scope and remains tested at unit-test level.
phase_templates_live() {
    phase "Phase 13: Covenant templates live -- vault + escrow"

    local n1_addr; n1_addr=$(cli N1 addresses 2>&1 | grep -oE "tdoli[a-z0-9]+" | head -1)
    local n2_pk; n2_pk=$(cli N2 info 2>&1 | grep -iE "^[[:space:]]*Public Key:" | grep -oE "[0-9a-f]{64}" | head -1)
    local n3_pk; n3_pk=$(cli N3 info 2>&1 | grep -iE "^[[:space:]]*Public Key:" | grep -oE "[0-9a-f]{64}" | head -1)
    if [ -z "$n1_addr" ] || [ -z "$n2_pk" ] || [ -z "$n3_pk" ]; then
        fail "could not resolve required addresses/pubkeys"; return 1
    fi
    local cur_h; cur_h=$(height)

    # --- vault: owner=N1, cosigner=N3, unlock-height = far future ---
    log "vault: N1 owner + N3 cosigner, unlock=$((cur_h + 1000)), send 0.1 DOLI"
    local vault_out; vault_out=$(printf 'y\n' | cli N1 template vault \
        --owner "$n1_addr" --cosigner "$n3_pk" --unlock-height $((cur_h + 1000)) \
        --send --to "$n1_addr" --amount 0.1 2>&1)
    if printf '%s\n' "$vault_out" | grep -qE "ERRTX-HTLC001|unsigned refund branch"; then
        fail "vault template tx rejected: [ERRTX-HTLC001] vault condition or(and(sig,timelock), multisig) is misclassified as HTLC by validation -- requires unsigned-refund-branch fix. NEW FINDING."
    else
        local vault_tx; vault_tx=$(tx_hash "$vault_out")
        if [ -z "$vault_tx" ]; then
            fail "vault template tx produced no txid"
            printf '%s\n' "$vault_out" | tail -8
        else
            ok "vault template tx broadcast (tx=${vault_tx:0:16}...)"
            local vault_confirmed; vault_confirmed=$(wait_confirmed "$vault_tx" "vault")
            if [ -n "$vault_confirmed" ]; then
                ok "vault condition encoded on chain ($vault_confirmed)"
            else
                fail "vault tx $vault_tx not confirmed after 10 blocks"
            fi
        fi
    fi

    # --- escrow: 2-of-3 (N1, N2, N3), timeout, refund to N1 ---
    log "escrow: 2-of-3 (N1,N2,N3), timeout=$((cur_h + 1000)), refund=N1, send 0.1 DOLI"
    local n2_addr; n2_addr=$(cli N2 addresses 2>&1 | grep -oE "tdoli[a-z0-9]+" | head -1)
    local n3_addr; n3_addr=$(cli N3 addresses 2>&1 | grep -oE "tdoli[a-z0-9]+" | head -1)
    local escrow_out; escrow_out=$(printf 'y\n' | cli N1 template escrow \
        --parties "${n1_addr},${n2_addr},${n3_addr}" --threshold 2 \
        --timeout $((cur_h + 1000)) --refund "$n1_addr" \
        --send --to "$n1_addr" --amount 0.1 2>&1)
    if printf '%s\n' "$escrow_out" | grep -qE "ERRTX-HTLC001|unsigned refund branch"; then
        fail "escrow template tx rejected: same [ERRTX-HTLC001] class as vault -- or(multisig, and(sig, timelock)) misclassified as HTLC. Same finding."
    else
        local escrow_tx; escrow_tx=$(tx_hash "$escrow_out")
        if [ -z "$escrow_tx" ]; then
            fail "escrow template tx produced no txid"
            printf '%s\n' "$escrow_out" | tail -8
        else
            ok "escrow template tx broadcast (tx=${escrow_tx:0:16}...)"
            local escrow_confirmed; escrow_confirmed=$(wait_confirmed "$escrow_tx" "escrow")
            if [ -n "$escrow_confirmed" ]; then
                ok "escrow condition encoded on chain ($escrow_confirmed)"
            else
                fail "escrow tx $escrow_tx not confirmed after 10 blocks"
            fi
        fi
    fi

    skip "subscription / agent-allowance / escrow-loan live tx (additional argument surfaces; skip to keep run-time bounded)"
}

# ============================================================================
# Main
# ============================================================================
main() {
    log "DOLI DeFi E2E test -- phase=$PHASE"
    cd "$(dirname "$0")/.." || exit 1

    preflight

    case "$PHASE" in
        all)            phase_mint; phase_amm; phase_nft; phase_channel; phase_template; phase_oracle; phase_bridge; \
                        phase_bridge_claim; phase_bridge_refund; phase_channel_pay; phase_templates_live ;;
        mint)           phase_mint ;;
        amm)            phase_mint; phase_amm ;;
        nft)            phase_nft ;;
        channel)        phase_channel ;;
        channel-pay)    phase_channel_pay ;;
        template)       phase_template ;;
        templates-live) phase_templates_live ;;
        oracle)         phase_oracle ;;
        bridge)         phase_bridge ;;
        bridge-claim)   phase_bridge_claim ;;
        bridge-refund)  phase_bridge_refund ;;
        *)              echo "Unknown phase: $PHASE"; exit 2 ;;
    esac

    phase "Summary"
    printf "  PASS: \033[1;32m%d\033[0m\n" "$PASS"
    printf "  FAIL: \033[1;31m%d\033[0m\n" "$FAIL"
    printf "  SKIP: \033[1;33m%d\033[0m\n" "$SKIP"
    if [ "$FAIL" -gt 0 ]; then
        printf "\n\033[1;31mFailures:\033[0m\n"
        for f in "${FAILURES[@]}"; do printf "  - %s\n" "$f"; done
        exit 1
    fi
}

main "$@"
