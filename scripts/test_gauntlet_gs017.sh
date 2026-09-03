#!/usr/bin/env bash
# OUTPUT CONTRACT: scripts/gauntlet-gs017.sh — `_gs017_assert <token>` (INC-I-203, GS-017)
#   O1 return code   — 0 PASS · 1 FAIL · 2 SKIP (gauntlet.sh:639 treats 0 and 2 alike,
#                       so O1 alone can never separate "checked green" from "not checked")
#   O2 FAIL_REASONS  — caller-owned global, appended on rc 1 only
#   O3 SKIP_REASONS  — caller-owned global, appended on rc 2 only; the ONLY signal that
#                       distinguishes a skip from a pass in the runner's output
#   O4 INFO_REASONS  — caller-owned global, appended on rc 0 only
#   O5 doli/curl log — every stubbed `doli` and `curl` invocation. GS-017 submits ONE
#                       deliberately over-cap AddBond, so the safety property is checked
#                       here: no `--count` at or below the discovered headroom (such a
#                       count is one the node ACCEPTS, bonding real funds) and no mutating
#                       RPC method (sendTransaction/submitTransaction/broadcast), ever.
#                       Carries the REQUEST BODY too: getMempoolTransactions must ask for the
#                       maximum fetchable page (`"limit":500`, the RPC hard cap of
#                       crates/rpc/src/methods/stats.rs:188) — there is no offset and no
#                       cursor, so a smaller limit makes the sweep a silent sample.
#   O6 registration  — the gauntlet.sh `assert` dispatch arm + the gauntlet-seed.sql
#                       assertions column. A green suite over a library the host runner
#                       never reaches is false assurance: the 2026-09-01 run failed GS-015
#                       with "unknown assertion token" while its unit suite was 100% green.
#   O7 precondition  — `gs017-cli-carries-m3`: its rc/reasons AND the gate it imposes on
#                       gs017-cli-refuses-before-signing. GS-017 is a DEFAULT-run scenario
#                       that fires a REAL add-bond, so an unverified CLI means the gauntlet
#                       injects the very incident it exists to detect.
#   PATHS:
#     token gs017-cli-refuses-before-signing
#       -> preflight: doli resolvable? · a producer_*.json wallet on disk? · that wallet's
#          bech32 address maps to an ACTIVE producer with bond_count < 3000? · a live
#          producer RPC?   -> any NO => rc 2 + O3
#       -> `doli ... producer add-bond --count (3000 - bond_count + 1)`, one over headroom
#          -> exit 0                                  => rc 1 + O2 (the pre-M3 false success)
#          -> exit != 0, no cap/headroom text         => rc 1 + O2 (a connection error is
#             not a refusal; reading it as one is a vacuous green)
#          -> exit != 0 WITH `headroom|cap` or ADDBOND_CAP_EXCEEDED
#             -> submitting node's getMempoolTransactions holds an `addbond` => rc 1 + O2
#             -> holds none                                                  => rc 0 + O4
#     token gs017-cli-carries-m3
#       -> `<doli> --version` -> `<name> <semver> (<sha>)`; rc 0 only when
#          `git merge-base --is-ancestor $GS017_M3_COMMIT <sha>` holds in this repo
#          (GS017_M3_COMMIT default f250f274). No sha in the version string · sha not an
#          object here · no git · CLI predates M3 => rc 2 + O3 naming the binary and its
#          version string. Never rc 1, never a submit: an old CLI is not a chain defect.
#          getNodeInfo carries no commit, so the node's M2 status is NOT determinable over
#          RPC — the node version is recorded informationally only.
#     token gs017-no-addbond-residency
#       -> sweep RPC 8500..8517; a port with no RPC is tolerated, not a failure
#       -> nothing answered   => rc 2 + O3 ; any `addbond` resident => rc 1 + O2 (names the
#          port) ; none => rc 0 + O4
#     token gs017-no-cap-poison-in-window
#       -> baseline per-node log byte offsets: $NODECFG when set and readable, else the
#          self-snapshot file $GS017_OFFSETS, else snapshot now. History holds pre-fix
#          events, so an absolute count is meaningless — only GROWTH past the baseline is
#          a finding.
#       -> no readable n*.log => rc 2 + O3 ; a `[BLOCK_POISON]` line carrying
#          ADDBOND_CAP_EXCEEDED past the baseline => rc 1 + O2 (names the node) ; none => rc 0 + O4
# INPUT PARTITIONS:
#   S1:  CLI refuses with the M3 client-side text         — O1=0, O4 non-empty, O2/O3 empty
#   S2:  RETIRED by REV-203-004 — superseded by S35: the M2 node text proves the CLI reached
#        the node, so the M3 client guard is absent or bypassed — O1=1, O2 non-empty
#   S3:  requested count vs a 2999 headroom               — O5 never --count <= 2999, no mutation
#   S4:  CLI exits 0 (the pre-M3 false success)           — O1=1, O2 non-empty, O4 empty
#   S5:  CLI exits non-zero, unrelated message            — O1=1, O2 non-empty, O4 empty
#   S6:  addbond resident on the submitting node after    — O1=1, O2 non-empty, O4 empty
#   S7:  no producer_*.json wallet on disk                — O1=2, O3 non-empty, O2/O4 empty, O5 no add-bond
#   S8:  the mapped producer already sits at the cap      — O1=2, O3 non-empty, O2 empty, O5 no add-bond
#   S9:  no live producer RPC                             — O1=2, O3 non-empty, O2/O4 empty
#   S10: `doli` not resolvable                            — O1=2, O3 names doli, O2 empty
#   S11: headroom 500 (bond_count 2500)                   — O5 never --count <= 500, no mutation
#   S12: every live mempool empty                         — O1=0, O4 non-empty, O2/O3 empty
#   S13: an addbond resident on a NON-submitting node     — O1=1, O2 names the port, O4 empty
#   S13b:a non-addbond tx resident                        — O1=0, O2 empty
#   S14: only some ports answer                           — O1=0, O2 empty (a dead port is tolerated)
#   S15: no port answers at all                           — O1=2, O3 non-empty, O2/O4 empty
#   S16: residency sweep                                  — O5 has no add-bond, no mutation
#   S17: cap poison in history, none past the baseline    — O1=0, O4 non-empty, O2 empty
#   S18: cap poison appended past the baseline            — O1=1, O2 names the node, O4 empty
#   S19: NON-cap poison appended past the baseline        — O1=0, O2 empty
#   S20: NODECFG unset, $GS017_OFFSETS present, growth    — O1=1, O2 non-empty, O4 empty
#   S21: NODECFG unset, no offsets file, history only     — O1=0, O2 empty
#   S22: no readable n*.log                               — O1=2, O3 non-empty, O2/O4 empty
#   S23: poison window scan                               — O5 has no add-bond, no mutation
#   S24: unknown gs017-* token                            — O1!=0 (never a silent pass)
#   S25: gauntlet.sh sources scripts/gauntlet-gs017.sh    — O6
#   S26: gauntlet.sh `assert` dispatches all three tokens — O6
#   S27: gauntlet-seed.sql registers GS-017 active        — O6
#   S28: `bash -n` on the library                         — parses cleanly
#   S29: CLI version carries an M3-or-later commit        — O7 O1=0, O4 names the node version
#   S30: CLI version predates M3                          — O7 O1=2, O3 non-empty, O2 empty
#   S31: version string carries no `(sha)`                — O7 O1=2, O3 names binary + version
#   S32: sha is not an object in this repo                — O7 O1=2, O3 non-empty, O2 empty
#   S33: precondition probe is read-only                  — O5 --version only, no add-bond
#   S34: pre-M3 CLI + the submit token                    — O7 O1=2, O5 has NO add-bond
#   S35: M2 node text (carries `RPC error`)               — O1=1, O2 says the CLI reached the node
#   S35b:bare [ADDBOND_CAP_EXCEEDED], no `RPC error`      — O1=1, O2 non-empty
#   S36: message with only `capacity`/`escape`/`capture`  — O1=1 (bare `cap` no longer suffices)
#   S37: M3 text AND `Submitting add-bond transaction`    — O1=1, O2 non-empty
#   S38: poison past baseline in an n*.log outside NODECFG— O1=1, O2 names n5 + count + window
#   S39: history only, 5 logs on disk, NODECFG lists 3    — O1=0, O4 names 5 logs + window
#   S40: residency sweep page size                        — O5 every request carries limit 500
#   S41: CLI read-back page size                          — O5 every request carries limit 500
#   S42: getMempoolInfo.txCount 1200 > one 500 page       — O1=1, O2 names the port + remainder
#   S43: getMempoolInfo.txCount 120 within one page       — O1=0, O4 names the observed txCount
#   S44: addbond only in the post-window sweep            — O1=0, O5 sweeps the port twice
#   S45: addbond only in the pre-window snapshot          — O1=0, O2 empty
#   S46: the SAME hash in both sweeps                     — O1=1, O2 names the port and the hash
#   S47: DIFFERENT addbond hashes in the two sweeps       — O1=0, O2 empty
#   S48: default settle spans >= 2 slots                  — elapsed >= 20s
#   S49: gauntlet.sh assert() routes gs017-cli-carries-m3 — O6
#   S50: gauntlet-seed.sql registers the 4th token        — O6
# MATRIX: 6 outputs x 28 partitions (only cells the path reaches are asserted)
#   S1: O1 O2 O3 O4 | S2: O1 O2 O4 | S3: O5 | S4: O1 O2 O4 | S5: O1 O2 O4 | S6: O1 O2 O4
#   S7: O1 O2 O3 O4 O5 | S8: O1 O2 O3 O5 | S9: O1 O2 O3 O4 | S10: O1 O2 O3 | S11: O5
#   S12: O1 O2 O3 O4 | S13: O1 O2 O4 | S13b: O1 O2 | S14: O1 O2 | S15: O1 O2 O3 O4 | S16: O5
#   S17: O1 O2 O4 | S18: O1 O2 O4 | S19: O1 O2 | S20: O1 O2 O4 | S21: O1 O2 | S22: O1 O2 O3 O4
#   S23: O5 | S24: O1 | S25: O6 | S26: O6 | S27: O6 | S28: parse
#   S29: O7 O1 O2 O3 O4 | S30: O7 O1 O2 O3 | S31: O7 O1 O3 | S32: O7 O1 O2 O3 | S33: O5
#   S34: O7 O1 O2 O3 O5 | S35: O1 O2 O4 | S35b: O1 | S36: O1 | S37: O1
#   S38: O1 O2 O4 | S39: O1 O2 O4 | S40: O5 | S41: O5 | S42: O1 O2 | S43: O1 O4
#   S44: O1 O5 | S45: O1 O2 | S46: O1 O2 | S47: O1 O2 | S48: timing | S49: O6 | S50: O6
#
# TDD RED tests for scripts/gauntlet-gs017.sh, which DOES NOT EXIST YET.
# `doli` and `curl` are stubbed on PATH and HOME is redirected into the sandbox, so every
# default the library derives from it ($HOME/testnet/{keys,logs,bin}) resolves to fixtures:
# no live testnet, no network, no chain, and no node started, stopped or perturbed.
# Sandbox PATH is $BIN_DIR:/usr/bin:/bin — python3 present, jq absent, the same parsing
# environment gauntlet.sh's own _gs012/_gs013 asserts run in.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
GS017_LIB="$PROJECT_ROOT/scripts/gauntlet-gs017.sh"
GAUNTLET="$PROJECT_ROOT/scripts/gauntlet.sh"
SEED_FILE="$PROJECT_ROOT/scripts/gauntlet-seed.sql"
TEST_DIR="/tmp/doli-gauntlet-gs017-test-$$"

TOKEN_CLI="gs017-cli-refuses-before-signing"
TOKEN_RESIDENCY="gs017-no-addbond-residency"
TOKEN_POISON="gs017-no-cap-poison-in-window"
TOKEN_M3="gs017-cli-carries-m3"

# f250f274 is INC-I-203 M3 ("refuse add-bond beyond the remaining cap headroom before
# signing"); 00f91933 is its parent merge, the newest commit that still ships the pre-M3 CLI.
M3_COMMIT="f250f274"
PRE_M3_COMMIT="00f91933"
ABSENT_COMMIT="0badc0de"

MAX_BONDS=3000
ADDBOND_HASH="988630d9c1a24f6b"
ADDR5="tdoli1x5f9k2m7q0w8ev3n6r4t2y8u1p5s9d3g7h2j4k6l8z0cq7v"
PORTS="8500 8501 8502 8503 8504 8505 8506 8507 8508 8509 8510 8511 8512 8513 8514 8515 8516 8517"
SUBMIT_PORT=8501

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

TESTS_PASSED=0
TESTS_FAILED=0
TESTS_TOTAL=0

print_header() {
    echo
    echo -e "${BLUE}========================================${NC}"
    echo -e "${BLUE}  $1${NC}"
    echo -e "${BLUE}========================================${NC}"
    echo
}

test_result() {
    local test_name=$1 result=$2 detail=$3
    TESTS_TOTAL=$((TESTS_TOTAL + 1))
    if [ "$result" = "pass" ]; then
        TESTS_PASSED=$((TESTS_PASSED + 1))
        echo -e "  ${GREEN}[PASS]${NC} $test_name"
    else
        TESTS_FAILED=$((TESTS_FAILED + 1))
        echo -e "  ${RED}[FAIL]${NC} $test_name"
        [ -n "$detail" ] && echo -e "         ${RED}$detail${NC}"
    fi
}

# shellcheck disable=SC2329  # invoked indirectly via trap below
cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT

rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR"

# A missing gauntlet-gs017.sh means _gs017_assert is never called, RC keeps the 99 sentinel
# and the *_REASONS stay empty — vacuously satisfying "rc non-zero", "INFO empty" and "no
# add-bond". `ck` conjoins every behavioural cell with this, so nothing goes green without it.
lib_ok() { [[ -f "$GS017_LIB" ]] && [[ "$FUNC_DEFINED" == "yes" ]]; }

# ck <name> <condition> [detail] — one asserted output cell per line.
ck() {
    local name="$1" cond="$2" det="${3:-}"
    if lib_ok && eval "$cond"; then test_result "$name" "pass" ""
    else test_result "$name" "fail" "${det:-$(detail)}"; fi
}

# --- stub writers ---

# `doli` stub. DOLI_ADDBOND_MODE drives the `producer add-bond` arm:
#   refuse_m3 (default) — the M3 client-side text of bins/cli/src/producer_ledger.rs:57
#   refuse_m2           — the M2 node-side text captured live 2026-09-02 (RPC -32002)
#   accept              — the pre-M3 shape: built, signed, submitted, exit 0
#   unrelated           — non-zero for a reason that has nothing to do with the cap
#   m2_bare             — the M2 marker alone, without the `RPC error` envelope
#   vague_cap           — non-zero text whose only `cap` match is capture/capacity/escape
#   refuse_then_submit  — the M3 refusal text AFTER the submit line was already printed
# DOLI_VERSION_MODE drives `--version`: m3 (default) · pre_m3 · no_sha · unknown_sha.
write_doli_stub() {
    cat > "$1/doli" <<'DOLI_STUB'
#!/usr/bin/env bash
echo "doli $*" >> "${DOLI_LOG:?DOLI_LOG not set}"
case "$*" in
    --version|*" --version"*|*--version)
        case "${DOLI_VERSION_MODE:-m3}" in
            pre_m3)      echo "doli 6.26.1 (00f91933)" ;;
            no_sha)      echo "doli 6.26.3" ;;
            unknown_sha) echo "doli 6.26.3 (0badc0de)" ;;
            *)           echo "doli 6.26.3 (f250f274)" ;;
        esac
        exit 0 ;;
    *addresses*)
        w=""; prev=""
        for a in "$@"; do
            [ "$prev" = "--wallet" ] && w="$a"
            prev="$a"
        done
        addr="$(python3 -c "import json;print(json.load(open('$w')).get('gs017_address',''))" 2>/dev/null)"
        [ -n "$addr" ] || exit 1
        echo "Address: $addr"; exit 0 ;;
    *"producer list"*)
        cat "${RPC_DIR:?RPC_DIR not set}/producers.json"; exit 0 ;;
    *add-bond*)
        case "${DOLI_ADDBOND_MODE:-refuse_m3}" in
            accept)
                echo "Transaction submitted: 988630d9c1a24f6b"; exit 0 ;;
            unrelated)
                echo "Error: error sending request for url (http://127.0.0.1:8502/): Connection refused (os error 61)" >&2
                exit 1 ;;
            refuse_m2)
                echo "Error: Error adding bonds: RPC error -32002 (INVALID_TRANSACTION): invalid transaction: [ADDBOND_CAP_EXCEEDED] producer=PublicKey(3047e96b) current=1 pending=0 in_block_prior=0 requested=3000 max=3000" >&2
                exit 1 ;;
            m2_bare)
                echo "Error: invalid transaction: [ADDBOND_CAP_EXCEEDED] producer=PublicKey(3047e96b) current=1 pending=0 in_block_prior=0 requested=3000 max=3000" >&2
                exit 1 ;;
            vague_cap)
                echo "Error: node at capture capacity, no escape route for this request" >&2
                exit 1 ;;
            refuse_then_submit)
                echo "Submitting add-bond transaction..."
                echo "Error: Bond cap exceeded: current=1 pending=0 requested=3000 cap=3000. You may still add 2999 bond(s)." >&2
                exit 1 ;;
            *)
                echo "Error: Bond cap exceeded: current=1 pending=0 requested=3000 cap=3000. You may still add 2999 bond(s). Re-run with --count 2999 or less; to grow beyond the cap, use delegation." >&2
                exit 1 ;;
        esac ;;
esac
exit 0
DOLI_STUB
    chmod +x "$1/doli"
}

# `curl` stub. A port with no $RPC_DIR/<port>.up exits 7 (curl's "couldn't connect"),
# which is how a node that is simply not running presents itself.
# getMempoolTransactions is SEQUENCED per port: call 1 serves <port>.mempool.json, call 2+
# serves <port>.mempool2.json when it exists, so a two-sweep settle filter sees the mempool
# actually change across the window. Absent mempool2.json every call answers the same, which
# is what every partition written before REV-203-007 expects.
write_curl_stub() {
    cat > "$1/curl" <<'CURL_STUB'
#!/usr/bin/env bash
echo "curl $*" >> "${CURL_LOG:?CURL_LOG not set}"
url=""; data=""; prev=""
for a in "$@"; do
    case "$a" in http://*) url="$a" ;; esac
    if [ "$prev" = "-d" ] || [ "$prev" = "--data" ] || [ "$prev" = "--data-binary" ]; then data="$a"; fi
    prev="$a"
done
port="${url##*:}"; port="${port%%/*}"
method="$(printf '%s' "$data" | sed -n 's/.*"method"[[:space:]]*:[[:space:]]*"\([A-Za-z]*\)".*/\1/p')"
[ -f "${RPC_DIR:?RPC_DIR not set}/$port.up" ] || exit 7
case "$method" in
    getMempoolTransactions)
        seq=$(( $(cat "$RPC_DIR/$port.mpseq" 2>/dev/null || echo 0) + 1 ))
        printf '%s' "$seq" > "$RPC_DIR/$port.mpseq"
        src="$RPC_DIR/$port.mempool.json"
        if [ "$seq" -ge 2 ] && [ -f "$RPC_DIR/$port.mempool2.json" ]; then
            src="$RPC_DIR/$port.mempool2.json"
        fi
        body="$(cat "$src" 2>/dev/null)"
        printf '{"jsonrpc":"2.0","id":1,"result":%s}\n' "${body:-[]}" ;;
    getMempoolInfo)
        if [ -f "$RPC_DIR/$port.mempoolinfo.json" ]; then
            printf '{"jsonrpc":"2.0","id":1,"result":%s}\n' "$(cat "$RPC_DIR/$port.mempoolinfo.json")"
        else
            n="$(python3 -c "import json;print(len(json.load(open('$RPC_DIR/$port.mempool.json'))))" 2>/dev/null || echo 0)"
            printf '{"jsonrpc":"2.0","id":1,"result":{"txCount":%s,"totalSize":420,"minFeeRate":1,"maxSize":10000000,"maxCount":5000}}\n' "${n:-0}"
        fi ;;
    getNodeInfo)
        printf '{"jsonrpc":"2.0","id":1,"result":{"version":"6.26.2","network":"testnet","peerId":"12D3KooWgs017","peerCount":7,"platform":"linux","arch":"x86_64"}}\n' ;;
    getProducers)
        body="$(cat "$RPC_DIR/producers.json" 2>/dev/null)"
        printf '{"jsonrpc":"2.0","id":1,"result":{"producers":%s}}\n' "${body:-[]}" ;;
    *)
        printf '{"jsonrpc":"2.0","id":1,"result":{"height":94444,"bestHash":"050fd33e543d"}}\n' ;;
esac
exit 0
CURL_STUB
    chmod +x "$1/curl"
}

# --- fixtures ---

# Sets CASE_DIR/WORK_DIR/BIN_DIR/HOME_DIR/KEYS_DIR/LOGS_DIR/RPC_DIR/DOLI_LOG/CURL_LOG.
# with_doli=0 leaves `doli` off the sandbox PATH AND out of $HOME/testnet/bin (S10).
new_sandbox() {
    local name="$1" with_doli="${2:-1}" p
    CASE_DIR="$TEST_DIR/$name"
    WORK_DIR="$CASE_DIR/work"; BIN_DIR="$CASE_DIR/bin"; HOME_DIR="$CASE_DIR/home"
    KEYS_DIR="$HOME_DIR/testnet/keys"; LOGS_DIR="$HOME_DIR/testnet/logs"
    RPC_DIR="$CASE_DIR/rpc"
    DOLI_LOG="$CASE_DIR/doli.log"; CURL_LOG="$CASE_DIR/curl.log"; LOG_FILE="$CASE_DIR/run.log"
    TEST_NODECFG=""
    DOLI_VERSION_MODE="m3"
    TEST_SETTLE="0"
    rm -rf "$CASE_DIR"
    mkdir -p "$WORK_DIR" "$BIN_DIR" "$KEYS_DIR" "$LOGS_DIR" "$RPC_DIR" "$HOME_DIR/testnet/bin"
    : > "$DOLI_LOG"; : > "$CURL_LOG"; : > "$LOG_FILE"
    write_curl_stub "$BIN_DIR"
    [ "$with_doli" = "1" ] && write_doli_stub "$BIN_DIR"
    write_wallet producer_5 "$ADDR5"
    write_producers active 1
    for p in $PORTS; do rpc_up "$p"; mempool_empty "$p"; done
    write_logs
}

write_wallet() {
    cat > "$KEYS_DIR/$1.json" <<WALLET
{"gs017_address":"$2","addresses":[{"public_key":"3047e96b","label":"$1"}]}
WALLET
}

# One producer row in the shape both discovery paths read: `producer list --format json` (cmd_producer/status.rs:457) and getProducers.
write_producers() {
    cat > "$RPC_DIR/producers.json" <<PRODUCERS
[{"status":"$1","address":"$ADDR5","publicKey":"3047e96b","bondCount":$2,"bondAmount":100000000000,"era":1,"registrationHeight":100,"pendingUpdates":[]},
 {"status":"active","address":"tdoli1othernodeaddressq9w8e7r6t5y4u3i2o1p0asdfghjkl","publicKey":"aa11bb22","bondCount":42,"bondAmount":4200000000,"era":1,"registrationHeight":120,"pendingUpdates":[]}]
PRODUCERS
}

rpc_up()        { : > "$RPC_DIR/$1.up"; }
rpc_down()      { rm -f "$RPC_DIR/$1.up"; }
mempool_empty() { echo '[]' > "$RPC_DIR/$1.mempool.json"; }

_mp_addbond_json() {
    cat > "$1" <<MEMPOOL
[{"hash":"$2","txType":"addbond","size":420,"fee":1000,"feeRate":2,"addedTime":1756848000}]
MEMPOOL
}

mempool_addbond() { _mp_addbond_json "$RPC_DIR/$1.mempool.json" "${2:-$ADDBOND_HASH}"; }

mempool_transfer() {
    cat > "$RPC_DIR/$1.mempool.json" <<MEMPOOL
[{"hash":"11aa22bb33cc44dd","txType":"transfer","size":250,"fee":1000,"feeRate":4,"addedTime":1756848000}]
MEMPOOL
}

# History that PREDATES the window: cap poison is non-zero on every node from the incident onward, so a scenario counting absolutely is permanently red.
write_logs() {
    local n
    for n in n1 n2 n3; do
        {
            echo "2026-09-02T20:00:00Z INFO Applied block h=94400 hash=050fd33e"
            echo "2026-09-02T20:01:00Z WARN [BLOCK_POISON] apply_block failed on self-produced block at h=94401: block economics invalid: [ADDBOND_CAP_EXCEEDED] producer=PublicKey(3047e96b) current=1 pending=0 in_block_prior=0 requested=3000 max=3000. Purged 1 TXs from mempool."
            echo "2026-09-02T20:02:00Z INFO Applied block h=94402 hash=1a2b3c4d"
        } > "$LOGS_DIR/$n.log"
    done
}

append_poison() {
    echo "2026-09-02T21:30:00Z WARN [BLOCK_POISON] apply_block failed on self-produced block at h=94500: $2. Purged 1 TXs from mempool." >> "$LOGS_DIR/$1.log"
}

# gauntlet.sh:200 build_nodecfg shape — written BEFORE the window, so its `offset` is the only baseline matching what the runner observed.
write_nodecfg() {
    local n off p=8501 first=1
    TEST_NODECFG="$CASE_DIR/nodes.json"
    {
        printf '{"nodes":['
        for n in n1 n2 n3; do
            off="$(wc -c < "$LOGS_DIR/$n.log" 2>/dev/null | tr -d ' ')"
            [ "$first" = 1 ] || printf ','
            first=0
            printf '{"name":"%s","port":%d,"pid":"0","logfile":"%s","offset":%s,"baseline_height":94444,"rss_mb":100}' \
                "$n" "$p" "$LOGS_DIR/$n.log" "${off:-0}"
            p=$((p + 1))
        done
        printf ']}\n'
    } > "$TEST_NODECFG"
}

# The standalone fallback: name:byte-offset, the shape _gs016_mark_offsets already writes.
write_offsets_file() {
    local n
    : > "$CASE_DIR/offsets.txt"
    for n in n1 n2 n3; do
        printf '%s:%s\n' "$n" "$(wc -c < "$LOGS_DIR/$n.log" 2>/dev/null | tr -d ' ')" >> "$CASE_DIR/offsets.txt"
    done
}

# --- runner ---
# Sources the lib in a subshell (PATH/HOME/env mutation cannot leak between partitions), resets
# the caller-owned globals, calls the entry point and ships rc + the *_REASONS out through files.
run_assert() {
    local token="$1"
    (
        set +e
        unset NODECFG DOLI_CLI GS017_M3_COMMIT GS017_SETTLE_SECS
        cd "$WORK_DIR" || exit 1
        export PATH="$BIN_DIR:/usr/bin:/bin"
        export HOME="$HOME_DIR"
        export DOLI_LOG CURL_LOG RPC_DIR
        export DOLI_ADDBOND_MODE="${DOLI_ADDBOND_MODE:-refuse_m3}"
        export DOLI_VERSION_MODE="${DOLI_VERSION_MODE:-m3}"
        # cwd is the sandbox, not a git worktree: the M3 ancestry check has to resolve this
        # repo on its own (BASH_SOURCE) or from GS017_REPO, never from $PWD.
        export GS017_REPO="$PROJECT_ROOT"
        export GAUNTLET_WINDOW="${TEST_WINDOW:-45}"
        export GS017_KEYS_DIR="$KEYS_DIR" GS017_LOG_DIR="$LOGS_DIR"
        export GS017_PORTS="$PORTS" GS017_SEED_PORT=8500
        export GS017_OFFSETS="$CASE_DIR/offsets.txt"
        [ -n "$TEST_NODECFG" ] && export NODECFG="$TEST_NODECFG"
        [ -n "${TEST_SETTLE:-}" ] && export GS017_SETTLE_SECS="$TEST_SETTLE"
        FAIL_REASONS=""; SKIP_REASONS=""; INFO_REASONS=""
        func="no"
        if [[ -f "$GS017_LIB" ]]; then
            # shellcheck disable=SC1090  # path is a test parameter, resolved at runtime
            . "$GS017_LIB" >/dev/null 2>&1
        fi
        declare -F _gs017_assert >/dev/null 2>&1 && func="yes"
        rc=99
        if [[ "$func" == "yes" ]]; then
            rc=0
            _gs017_assert "$token" > "$LOG_FILE" 2>&1 || rc=$?
        fi
        printf '%s' "$func" > "$CASE_DIR/func"
        printf '%s' "$rc"   > "$CASE_DIR/rc"
        printf '%s' "$FAIL_REASONS" > "$CASE_DIR/fail"
        printf '%s' "$SKIP_REASONS" > "$CASE_DIR/skip"
        printf '%s' "$INFO_REASONS" > "$CASE_DIR/info"
    )
    FUNC_DEFINED="$(cat "$CASE_DIR/func" 2>/dev/null || echo no)"
    RC="$(cat "$CASE_DIR/rc" 2>/dev/null || echo 99)"
    R_FAIL="$(cat "$CASE_DIR/fail" 2>/dev/null || true)"
    R_SKIP="$(cat "$CASE_DIR/skip" 2>/dev/null || true)"
    R_INFO="$(cat "$CASE_DIR/info" 2>/dev/null || true)"
}

# --- O5 predicates ---

addbond_counts() { grep -oE -- '--count[= ]+[0-9]+' "$DOLI_LOG" 2>/dev/null | grep -oE '[0-9]+'; }
addbond_attempted() { grep -q -- 'add-bond' "$DOLI_LOG" 2>/dev/null; }
mutating_rpc() { grep -qEi 'sendTransaction|submitTransaction|broadcast' "$CURL_LOG" 2>/dev/null; }

# 0 when SOME add-bond used a count that fits the headroom — a fitting count is one the node ACCEPTS, which bonds real funds on a default, no-confirm-var run.
fitting_count_used() {
    local c
    for c in $(addbond_counts); do [ "$c" -le "$1" ] && return 0; done
    return 1
}

o5_detail() { echo "counts=[$(addbond_counts | tr '\n' ' ')] doli=$DOLI_LOG curl=$CURL_LOG"; }
detail() { echo "rc=$RC func=$FUNC_DEFINED lib=$GS017_LIB fail=[$R_FAIL] skip=[$R_SKIP] info=[$R_INFO] log=$LOG_FILE"; }
lc() { printf '%s' "$1" | tr '[:upper:]' '[:lower:]'; }

print_header "gauntlet-gs017.sh RED tests (INC-I-203, GS-017)"
echo -e "${CYAN}Test directory: $TEST_DIR${NC}"

# S1 — REQ-BOND-007 (Should) — Decision: a scenario that cannot report a healthy post-M3 refusal is permanently red, gets waived, and stops guarding the admission gap.
new_sandbox "s1_cli_refuses_m3"
DOLI_ADDBOND_MODE=refuse_m3 run_assert "$TOKEN_CLI"
ck "S1 cli_refuses_m3_text: rc 0 (PASS)" '[[ "$RC" -eq 0 ]]'
ck "S1 cli_refuses_m3_text: INFO_REASONS records the refusal" '[[ -n "$R_INFO" ]]'
# A pass carrying a skip reason prints as PASS + a yellow skip: line — indistinguishable, to an operator, from a scenario that never ran.
ck "S1 cli_refuses_m3_text: SKIP_REASONS and FAIL_REASONS empty" '[[ -z "$R_SKIP" && -z "$R_FAIL" ]]'

# S3 — REQ-BOND-007 (Must, safety) — Decision: one bond less than headroom+1 is a count the node ACCEPTS, so the gauntlet would bond 2999 real DOLI on every default run.
ck "S3 count_over_headroom: no --count at or below the 2999-bond headroom" \
   'addbond_attempted && ! fitting_count_used $((MAX_BONDS - 1))' "$(o5_detail)"
ck "S3 count_over_headroom: no mutating RPC method invoked" '! mutating_rpc' "$(o5_detail)"

# S2 — RETIRED by REV-203-004. It pinned "the M2 node-side refusal text is a PASS", which the
# narrowed acceptance contradicts: reaching the node at all means the M3 client guard did not
# fire, so an M3 revert would be invisible. Replaced by S35/S35b in the hardening file.

# S4 — REQ-BOND-007 (Must) — Decision: exit 0 is the INC-I-203 shape (built, signed, submitted, gossiped); reading it as green certifies the defect GS-017 exists to catch.
new_sandbox "s4_cli_exits_zero"
DOLI_ADDBOND_MODE=accept run_assert "$TOKEN_CLI"
ck "S4 cli_exits_zero: rc 1 (FAIL)" '[[ "$RC" -eq 1 ]]'
ck "S4 cli_exits_zero: FAIL non-empty, INFO empty" '[[ -n "$R_FAIL" && -z "$R_INFO" ]]'

# S5 — REQ-BOND-007 (Must) — Decision: a connection refusal also exits non-zero, so accepting any non-zero exit as proof reports green on a fleet where the guard was reverted.
new_sandbox "s5_cli_unrelated_error"
DOLI_ADDBOND_MODE=unrelated run_assert "$TOKEN_CLI"
ck "S5 cli_unrelated_error: rc 1 (FAIL, not a cap refusal)" '[[ "$RC" -eq 1 ]]'
ck "S5 cli_unrelated_error: FAIL non-empty, INFO empty" '[[ -n "$R_FAIL" && -z "$R_INFO" ]]'

# S6 — REQ-BOND-002 (Must) — Decision: a CLI that prints a refusal and submits anyway leaves the toxic tx resident (the whole harm) while GS-017 reads the text and reports green.
new_sandbox "s6_addbond_resident_after"
mempool_addbond "$SUBMIT_PORT"
DOLI_ADDBOND_MODE=refuse_m3 run_assert "$TOKEN_CLI"
ck "S6 addbond_resident_after_refusal: rc 1 (FAIL)" '[[ "$RC" -eq 1 ]]'
ck "S6 addbond_resident_after_refusal: FAIL non-empty, INFO empty" '[[ -n "$R_FAIL" && -z "$R_INFO" ]]'

# S7 — REQ-BOND-007 (Must) — Decision: GS-017 runs by default on hosts with no local testnet, and a FAIL with no wallet is how a scenario earns a standing waiver.
new_sandbox "s7_no_wallet"
rm -f "$KEYS_DIR"/producer_*.json
run_assert "$TOKEN_CLI"
ck "S7 no_wallet_key: rc 2 (SKIP, not FAIL)" '[[ "$RC" -eq 2 ]]'
ck "S7 no_wallet_key: SKIP non-empty, FAIL empty" '[[ -n "$R_SKIP" && -z "$R_FAIL" ]]'
# rc 2 alone prints as PASS (gauntlet.sh:639); only a non-empty SKIP with an empty INFO says the check did not happen.
ck "S7 no_wallet_key: INFO empty and no add-bond attempted" '[[ -z "$R_INFO" ]] && ! addbond_attempted'

# S8 — REQ-BOND-007 (Must) — Decision: at the cap, headroom+1 is --count 1, which the node ACCEPTS — there is no safe probe for a producer at 3000, so it must be skipped.
new_sandbox "s8_producer_at_cap"
write_producers active "$MAX_BONDS"
run_assert "$TOKEN_CLI"
ck "S8 producer_at_cap: rc 2 (SKIP, not FAIL)" '[[ "$RC" -eq 2 ]]'
ck "S8 producer_at_cap: SKIP non-empty, FAIL empty" '[[ -n "$R_SKIP" && -z "$R_FAIL" ]]'
ck "S8 producer_at_cap: no add-bond attempted against a producer at the cap" \
   '! addbond_attempted' "$(o5_detail)"

# S9 — REQ-BOND-007 (Must) — Decision: with no node answering there is nothing to submit to and no mempool to read back, so a FAIL reports a chain defect on a stopped testnet.
new_sandbox "s9_no_live_rpc"
for p in $PORTS; do rpc_down "$p"; done
run_assert "$TOKEN_CLI"
ck "S9 no_live_rpc: rc 2 (SKIP, not FAIL)" '[[ "$RC" -eq 2 ]]'
ck "S9 no_live_rpc: SKIP non-empty, FAIL and INFO empty" '[[ -n "$R_SKIP" && -z "$R_FAIL" && -z "$R_INFO" ]]'

# S10 — REQ-BOND-007 (Must) — Decision: no `doli` means the refusal was never exercised, so rendering an absent binary as "the CLI did not refuse" is a vacuous FAIL.
new_sandbox "s10_doli_absent" 0
run_assert "$TOKEN_CLI"
ck "S10 doli_unresolvable: rc 2 (SKIP, not FAIL)" '[[ "$RC" -eq 2 ]]'
ck "S10 doli_unresolvable: SKIP names doli, FAIL empty" '[[ "$(lc "$R_SKIP")" == *doli* && -z "$R_FAIL" ]]'

# S11 — REQ-BOND-007 (Must, safety) — Decision: a hardcoded 3000 is over headroom only at bond_count 1; at 2500 it FITS and bonds 3000 real DOLI, so the count must be derived.
new_sandbox "s11_headroom_500"
write_producers active 2500
DOLI_ADDBOND_MODE=refuse_m3 run_assert "$TOKEN_CLI"
ck "S11 derived_count: no --count at or below the 500-bond headroom" \
   'addbond_attempted && ! fitting_count_used 500' "$(o5_detail)"
ck "S11 derived_count: no mutating RPC method invoked" '! mutating_rpc' "$(o5_detail)"

# S12 — REQ-BOND-002 (Must) — Decision: the fleet sweep is the only check that would have seen 988630d9 sit in 13 of 18 mempools while every node reported itself healthy.
new_sandbox "s12_all_mempools_empty"
run_assert "$TOKEN_RESIDENCY"
ck "S12 all_mempools_empty: rc 0 (PASS)" '[[ "$RC" -eq 0 ]]'
ck "S12 all_mempools_empty: INFO non-empty, FAIL and SKIP empty" \
   '[[ -n "$R_INFO" && -z "$R_FAIL" && -z "$R_SKIP" ]]'

# S13 — REQ-BOND-002 (Must) — Decision: gossip spreads an AddBond the submitting node rejected, so a sweep of one node reproduces the blind spot that hid it on seed, n2, n6, n13.
new_sandbox "s13_addbond_elsewhere"
mempool_addbond 8507
run_assert "$TOKEN_RESIDENCY"
ck "S13 addbond_on_another_node: rc 1 (FAIL)" '[[ "$RC" -eq 1 ]]'
ck "S13 addbond_on_another_node: FAIL names port 8507, INFO empty" '[[ "$R_FAIL" == *8507* && -z "$R_INFO" ]]'

# S13b — REQ-BOND-002 (Should) — Decision: a non-AddBond resident tx is normal traffic, and flagging it makes the sweep red on any busy fleet — the same waiver failure mode.
new_sandbox "s13b_transfer_resident"
mempool_transfer 8507
run_assert "$TOKEN_RESIDENCY"
ck "S13b transfer_resident: rc 0 (a non-addbond tx is not a finding)" '[[ "$RC" -eq 0 && -z "$R_FAIL" ]]'

# S14 — REQ-BOND-002 (Should) — Decision: the fleet is routinely partly down, so treating an unanswered port as a finding makes the default run red for a non-bond reason.
new_sandbox "s14_partial_fleet"
for p in 8509 8510 8511 8512 8513 8514 8515 8516 8517; do rpc_down "$p"; done
run_assert "$TOKEN_RESIDENCY"
ck "S14 some_ports_dead: rc 0 (a dead port is tolerated, not a FAIL)" '[[ "$RC" -eq 0 && -z "$R_FAIL" ]]'

# S15 — REQ-BOND-002 (Must) — Decision: zero answering nodes means zero mempools inspected, and "none found" over an empty set is the vacuous green this token must prevent.
new_sandbox "s15_fleet_down"
for p in $PORTS; do rpc_down "$p"; done
run_assert "$TOKEN_RESIDENCY"
ck "S15 no_port_answers: rc 2 (SKIP, not a vacuous PASS)" '[[ "$RC" -eq 2 ]]'
ck "S15 no_port_answers: SKIP non-empty, INFO and FAIL empty" \
   '[[ -n "$R_SKIP" && -z "$R_INFO" && -z "$R_FAIL" ]]'

# S16 — REQ-BOND-002 (Must, safety) — Decision: if a read-only sweep can reach add-bond or a mutating RPC it submits transactions unattended on every default run, guarded by nothing.
new_sandbox "s16_residency_read_only"
run_assert "$TOKEN_RESIDENCY"
ck "S16 residency_read_only: no add-bond and no mutating RPC" \
   '! addbond_attempted && ! mutating_rpc' "$(o5_detail)"

# S17 — REQ-BOND-001 (Must) — Decision: every log already carries pre-fix cap poison, so an absolute count is red forever and only growth past the baseline is live evidence.
new_sandbox "s17_history_only"
write_nodecfg
run_assert "$TOKEN_POISON"
ck "S17 history_before_baseline: rc 0 (history is not a finding)" '[[ "$RC" -eq 0 ]]'
ck "S17 history_before_baseline: INFO non-empty, FAIL empty" '[[ -n "$R_INFO" && -z "$R_FAIL" ]]'

# S18 — REQ-BOND-001 (Must) — Decision: new cap poison in the window means the builder packed an over-cap AddBond again, losing a slot and discarding the block.
new_sandbox "s18_poison_growth"
write_nodecfg
append_poison n2 "block economics invalid: [ADDBOND_CAP_EXCEEDED] producer=PublicKey(3047e96b) current=1 requested=3000 max=3000"
run_assert "$TOKEN_POISON"
ck "S18 cap_poison_after_baseline: rc 1 (FAIL)" '[[ "$RC" -eq 1 ]]'
ck "S18 cap_poison_after_baseline: FAIL names n2, INFO empty" '[[ "$R_FAIL" == *n2* && -z "$R_INFO" ]]'

# S19 — REQ-BOND-001 (Must) — Decision: [BLOCK_POISON] is the shared INC-I-204 arm, so blaming another cause on the bond cap sends the operator to the wrong subsystem.
new_sandbox "s19_unrelated_poison_growth"
write_nodecfg
append_poison n3 "state root mismatch: expected 050fd33e, got 9c1a24f6"
run_assert "$TOKEN_POISON"
ck "S19 non_cap_poison_after_baseline: rc 0 (not a bond-cap finding)" '[[ "$RC" -eq 0 && -z "$R_FAIL" ]]'

# S20 — REQ-BOND-001 (Should) — Decision: without a fallback baseline the delta is computed against nothing and the FAIL branch is unreachable outside gauntlet.sh.
new_sandbox "s20_standalone_offsets_file"
write_offsets_file
append_poison n1 "block economics invalid: [ADDBOND_CAP_EXCEEDED] producer=PublicKey(3047e96b) current=1 requested=3000 max=3000"
run_assert "$TOKEN_POISON"
ck "S20 standalone_offsets_growth: rc 1 (FAIL without NODECFG)" '[[ "$RC" -eq 1 ]]'
ck "S20 standalone_offsets_growth: FAIL non-empty, INFO empty" '[[ -n "$R_FAIL" && -z "$R_INFO" ]]'

# S21 — REQ-BOND-001 (Should) — Decision: with no baseline the library must snapshot one and see nothing, never count the whole log and report every pre-fix event.
new_sandbox "s21_standalone_snapshot"
rm -f "$CASE_DIR/offsets.txt"
run_assert "$TOKEN_POISON"
ck "S21 standalone_snapshot: rc 0 (history invisible to a fresh snapshot)" '[[ "$RC" -eq 0 && -z "$R_FAIL" ]]'

# S22 — REQ-BOND-001 (Must) — Decision: no readable log means no window was observed, and "zero cap poison" over zero log bytes is a green built on nothing.
new_sandbox "s22_no_logs"
rm -f "$LOGS_DIR"/n*.log
run_assert "$TOKEN_POISON"
ck "S22 no_readable_logs: rc 2 (SKIP, not a vacuous PASS)" '[[ "$RC" -eq 2 ]]'
ck "S22 no_readable_logs: SKIP non-empty, INFO and FAIL empty" \
   '[[ -n "$R_SKIP" && -z "$R_INFO" && -z "$R_FAIL" ]]'

# S23 — REQ-BOND-001 (Must, safety) — Decision: the poison scan reads log files, so if it can reach add-bond or a mutating RPC then a log scan submits transactions.
new_sandbox "s23_poison_read_only"
write_nodecfg
run_assert "$TOKEN_POISON"
ck "S23 poison_scan_read_only: no add-bond and no mutating RPC" \
   '! addbond_attempted && ! mutating_rpc' "$(o5_detail)"

# S24 — REQ-BOND-002 (Should) — Decision: a typo in the assertions column would otherwise make GS-017 report PASS while asserting nothing at all.
new_sandbox "s24_unknown_token"
run_assert "gs017-not-a-real-token"
ck "S24 unknown_token: rc non-zero (never a silent pass)" '[[ "$RC" -ne 0 ]]'

# --- registration-edge helpers (O6) ---

# The runner dispatches only tokens from gauntlet_scenarios (gauntlet.sh:658) and .omega/ is
# gitignored, so gauntlet-seed.sql is the only registration. Applied to a throwaway DB.
seed_gs017_assertions() {
    local db="$TEST_DIR/seed-check.db"
    rm -f "$db"
    if command -v sqlite3 >/dev/null 2>&1; then
        sqlite3 "$db" "CREATE TABLE gauntlet_scenarios (scenario_id TEXT PRIMARY KEY, name TEXT, description TEXT, incident_ids TEXT, assertions TEXT, scale_params TEXT, runner TEXT, status TEXT);" >/dev/null 2>&1
        sqlite3 "$db" < "$SEED_FILE" >/dev/null 2>&1
        sqlite3 "$db" "SELECT assertions FROM gauntlet_scenarios WHERE scenario_id='GS-017' AND status='active';" 2>/dev/null
    else
        grep -A8 "'GS-017'," "$SEED_FILE" | grep -oE "'gs017-[a-z0-9,-]+'" | tr -d "'" | head -1
    fi
}

# A `case` arm carrying the token whose next line dispatches to _gs017_assert. Matching the token
# anywhere would also match the header comment, which dispatches nothing (the 2026-09-01 shape).
dispatches_token() {
    awk -v tok="$1" '
        $0 ~ /^[[:space:]]*[a-z0-9|_-]+\)[[:space:]]*$/ && index($0, tok) { armed = 1; next }
        armed { if ($0 ~ /_gs017_assert/) ok = 1; armed = 0 }
        END { exit ok ? 0 : 1 }
    ' "$GAUNTLET"
}

# S25 — REQ-BOND-002 (Must) — Decision: an unsourced library leaves _gs017_assert undefined and every token falls through to "unknown assertion token", unit suite green.
if grep -q 'gauntlet-gs017\.sh' "$GAUNTLET" 2>/dev/null && grep -q '\. "\$GS017_LIB"' "$GAUNTLET" 2>/dev/null; then
    test_result "S25 wiring: gauntlet.sh sources scripts/gauntlet-gs017.sh" "pass" ""
else
    test_result "S25 wiring: gauntlet.sh sources scripts/gauntlet-gs017.sh" "fail" \
        "no GS017_LIB source block in $GAUNTLET"
fi

# S26 — REQ-BOND-002 (Must) — Decision: a seeded token that lands on the unknown-token arm asserts nothing while still being counted as a scenario in the run summary.
for tok in "$TOKEN_CLI" "$TOKEN_RESIDENCY" "$TOKEN_POISON"; do
    if dispatches_token "$tok"; then
        test_result "S26 dispatch: gauntlet.sh assert() routes $tok" "pass" ""
    else
        test_result "S26 dispatch: gauntlet.sh assert() routes $tok" "fail" \
            "no case arm in $GAUNTLET dispatches $tok to _gs017_assert"
    fi
done

# S27 — REQ-BOND-002 (Must) — Decision: assert() is only called with tokens from gauntlet_scenarios, so a scenario absent from the seed is dead code on every other machine.
SEED_ASSERTIONS="$(seed_gs017_assertions)"
if [ -n "$SEED_ASSERTIONS" ]; then
    test_result "S27 seed_registration: gauntlet-seed.sql registers GS-017 as active" "pass" ""
else
    test_result "S27 seed_registration: gauntlet-seed.sql registers GS-017 as active" "fail" \
        "no active GS-017 row parsed from $SEED_FILE"
fi

if [[ "$SEED_ASSERTIONS" == *"$TOKEN_CLI"* ]] && [[ "$SEED_ASSERTIONS" == *"$TOKEN_RESIDENCY"* ]] \
   && [[ "$SEED_ASSERTIONS" == *"$TOKEN_POISON"* ]]; then
    test_result "S27 seed_registration: registered assertions list all three tokens" "pass" ""
else
    test_result "S27 seed_registration: registered assertions list all three tokens" "fail" \
        "assertions=[$SEED_ASSERTIONS]"
fi

# S28 — REQ-BOND-002 (Must) — Decision: gauntlet.sh sources the library unconditionally, so a syntax error there takes the whole gauntlet down, not GS-017 alone.
if [ -f "$GS017_LIB" ] && bash -n "$GS017_LIB" 2>/dev/null; then
    test_result "S28 hygiene: scripts/gauntlet-gs017.sh parses under bash -n" "pass" ""
else
    test_result "S28 hygiene: scripts/gauntlet-gs017.sh parses under bash -n" "fail" \
        "$(bash -n "$GS017_LIB" 2>&1 | head -3 || echo "missing: $GS017_LIB")"
fi

# REV-203-003..007 cells live in a second file: this one is already at the 800-line test
# budget of MODULE-SIZE-BUDGET, and every helper they need is defined above.
# shellcheck source=scripts/test_gauntlet_gs017_hardening.sh
. "$SCRIPT_DIR/test_gauntlet_gs017_hardening.sh"

print_header "TEST SUMMARY"
echo -e "  Tests Passed: ${GREEN}$TESTS_PASSED${NC}"
echo -e "  Tests Failed: ${RED}$TESTS_FAILED${NC}"
echo -e "  Total Tests:  $TESTS_TOTAL"
echo

if [ "$TESTS_FAILED" -eq 0 ]; then
    EXIT_CODE=0
else
    EXIT_CODE=1
fi

exit $EXIT_CODE
