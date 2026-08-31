#!/usr/bin/env bash
# ============================================================================
# gauntlet-gs014.sh — GS-014 "governance-relay-from-non-producer" scenario.
#
# Sourced by scripts/gauntlet.sh. Replays INC-I-195: `submitMaintainerChange`
# ended at `mempool.add_system_transaction` and returned {"status":"accepted"}
# without ever calling `broadcast_tx`, so the transaction never left the node
# that received the RPC. On a NON-PRODUCER endpoint it could therefore never be
# mined — the RPC reported success and the maintainer set never changed.
#
# THE ENDPOINT IS THE WHOLE SCENARIO. Mainnet seeds run `--relay-server`
# WITHOUT `--producer`, and a seed RPC is the endpoint an operator reaches for.
# Submitting to a producer hides the bug completely: the receiving node builds
# the next block itself, so the missing relay costs nothing. This scenario
# therefore submits ONLY to a node it has PROVEN carries no `--producer` flag,
# and refuses to run if it cannot find one. A pass against a producer would be
# a fake green — it would assert the fix while never exercising it.
#
# MEASURED, pre-fix, on this testnet (2026-08-29): a removal submitted to the
# seed returned "accepted" and the set was unchanged ~36 blocks later. After
# the fix the same remove+add submitted TO THE SEED applied at h=57,024 and
# h=57,025 — one block each.
#
# WHY THIS IS THE DETECTION THAT DID NOT EXIST: nothing anywhere reported the
# silent drop. `{"status":"accepted"}` was the only feedback an operator got,
# and it was a lie. The bug surfaced only because a human went looking at the
# maintainer set 36 blocks later. INC-I-175 pushed TEN governance transactions
# through this exact path to rotate the mainnet trust root.
#
# CHAIN-WRITING AND OPT-IN. Like GS-010 this scenario WRITES TO THE CHAIN, so
# it runs only with `--gs014` AND GAUNTLET_GS014_CONFIRM=1, and refuses any
# network that is not testnet. Unlike GS-010 it is STATE-NEUTRAL: it removes a
# maintainer and re-adds the SAME key, so a completed run ends on the digest it
# started from. A bond, by contrast, unwinds only via request-withdrawal.
#
# ORDER IS REMOVE-THEN-ADD, NOT ADD-THEN-REMOVE. The set sits at
# MAX_MAINTAINERS (5 of 5, crates/core/src/maintainer/mod.rs:98), so an "add"
# has nowhere to land and would be rejected before the relay is ever reached.
# The cost of that ordering is the failure mode it creates: if the remove
# applies and the re-add does not, the set is left one member short. That is
# why the scenario refuses to start below 4 members (removing one must still
# leave >= MIN_MAINTAINERS = 3), and why a partial run prints the exact repair
# command instead of exiting quietly.
#
# Env: GS014_CLI ($HOME/testnet/bin/doli-node), GS014_KEYS ($HOME/testnet/keys),
#      GS014_APPLY_MAX_BLOCKS (25), GS014_APPLY_TIMEOUT (240s),
#      GS014_POLL_SECS (5). Uses gauntlet.sh globals/helpers ($WORK, $LIVE,
#      port_of, say, C_*, FAIL_REASONS/SKIP_REASONS/INFO_REASONS).
# ============================================================================

GS014_CLI="${GS014_CLI:-$HOME/testnet/bin/doli-node}"
GS014_KEYS="${GS014_KEYS:-$HOME/testnet/keys}"
GS014_APPLY_MAX_BLOCKS="${GS014_APPLY_MAX_BLOCKS:-25}"
GS014_APPLY_TIMEOUT="${GS014_APPLY_TIMEOUT:-240}"
GS014_POLL_SECS="${GS014_POLL_SECS:-5}"

# ── helpers ─────────────────────────────────────────────────────────────────

# _gs014_rpc <port> <method> [params-json] — raw JSON-RPC POST, empty on failure.
_gs014_rpc() {
    local port="$1" method="$2" params="${3:-}"
    [ -n "$params" ] || params='{}'
    curl -sf --max-time 15 -X POST "http://127.0.0.1:$port" \
        -H 'Content-Type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$method\",\"params\":$params}" 2>/dev/null
}

# _gs014_height <port> — best height, or empty.
_gs014_height() {
    _gs014_rpc "$1" getChainInfo | python3 -c "
import sys, json
try: print(json.load(sys.stdin)['result']['bestHeight'])
except Exception: pass" 2>/dev/null
}

# _gs014_set <port> — 'digest count source enforced' for the maintainer set.
# Empty when the node is unreachable or the response is unparseable.
_gs014_set() {
    _gs014_rpc "$1" getMaintainerSet | python3 -c "
import sys, json
try:
    r = json.load(sys.stdin)['result']
    print('%s %d %s %s' % (r['maintainer_set_digest'], r['member_count'],
                           r['source'], r['enforced']))
except Exception: pass" 2>/dev/null
}

# _gs014_members <port> — member pubkeys, one per line.
_gs014_members() {
    _gs014_rpc "$1" getMaintainerSet | python3 -c "
import sys, json
try:
    for m in json.load(sys.stdin)['result']['maintainers']: print(m['pubkey'])
except Exception: pass" 2>/dev/null
}

# _gs014_is_producer_port <port> — 0 if the LIVE process serving that RPC port
# carries --producer. Read from the actual process args, NOT from a plist: the
# plist is what launchd was asked to start, the args are what is running.
_gs014_is_producer_port() {
    ps -Ao args | grep -- "--rpc-port $1 " | grep -v grep | grep -q -- '--producer'
}

# _gs014_keyfile <pubkey> — wallet json under $GS014_KEYS holding that pubkey.
_gs014_keyfile() {
    python3 - "$GS014_KEYS" "$1" <<'PY' 2>/dev/null
import json, glob, os, sys
keys, want = sys.argv[1], sys.argv[2]
for f in sorted(glob.glob(os.path.join(keys, "*.json"))):
    try: j = json.load(open(f))
    except Exception: continue
    for a in (j.get("addresses") or []):
        if (a.get("public_key") or a.get("publicKey") or "") == want:
            print(f); raise SystemExit
PY
}

# _gs014_sign <add|remove> <target> <keyfile> <height> — '<signer_pk> <sig>'.
# The signer opens no database (it takes the height as an explicit operand
# precisely so it never reads chain state), so it is safe while nodes run.
_gs014_sign() {
    local action="$1" target="$2" keyfile="$3" height="$4" out
    out="$("$GS014_CLI" --network testnet maintainer "$action" \
        --target "$target" --key "$keyfile" --height "$height" 2>/dev/null)"
    local pk sig
    pk="$(printf '%s\n' "$out" | awk '/^pubkey: /{print $2; exit}')"
    sig="$(printf '%s\n' "$out" | awk '/^signature: /{print $2; exit}')"
    [ -n "$pk" ] && [ -n "$sig" ] && echo "$pk $sig"
}

# _gs014_submit <port> <add|remove> <target> <sigfile> — '<status> <tx_hash>'.
# sigfile holds one '<pubkey> <signature>' pair per line.
_gs014_submit() {
    local port="$1" action="$2" target="$3" sigfile="$4" body
    body="$(python3 - "$action" "$target" "$sigfile" <<'PY'
import json, sys
action, target, sigfile = sys.argv[1], sys.argv[2], sys.argv[3]
sigs = []
for line in open(sigfile):
    parts = line.split()
    if len(parts) == 2:
        sigs.append({"pubkey": parts[0], "signature": parts[1]})
params = {"action": action, "target_pubkey": target, "signatures": sigs}
if action == "remove":
    params["reason"] = "GS-014 gauntlet: governance relay from a non-producer endpoint (INC-I-195)"
print(json.dumps({"jsonrpc": "2.0", "id": 1,
                  "method": "submitMaintainerChange", "params": params}))
PY
)"
    curl -sf --max-time 20 -X POST "http://127.0.0.1:$port" \
        -H 'Content-Type: application/json' -d "$body" 2>/dev/null \
    | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    if 'error' in d:
        print('error:%s' % str(d['error'].get('message', d['error']))[:90].replace(' ', '_'))
    else:
        print('%s %s' % (d['result']['status'], d['result']['tx_hash']))
except Exception: print('unparseable')" 2>/dev/null
}

# _gs014_wait_count <port> <want_count> — height at which the set reached
# want_count, or -1 on timeout. Polls the SAME non-producer endpoint the
# submission went to: if the change is visible there, it was mined somewhere
# else and gossiped back, which is precisely the property under test.
_gs014_wait_count() {
    local port="$1" want="$2" waited=0 cur
    while [ "$waited" -lt "$GS014_APPLY_TIMEOUT" ]; do
        cur="$(_gs014_set "$port" | awk '{print $2}')"
        if [ "${cur:-0}" = "$want" ]; then _gs014_height "$port"; return 0; fi
        sleep "$GS014_POLL_SECS"; waited=$(( waited + GS014_POLL_SECS ))
    done
    echo "-1"
}

# ── injection ───────────────────────────────────────────────────────────────
# gs014_inject — remove a maintainer via a NON-PRODUCER RPC, wait for it to
# apply, then re-add the same key the same way. Writes $WORK/gs014_*.txt and a
# gs014_injected marker. No-op (assertions SKIP) unless confirmed and hostable.
gs014_inject() {
    if [ "${GAUNTLET_GS014_CONFIRM:-0}" != "1" ]; then
        say "  ${C_Y}[gs014] GAUNTLET_GS014_CONFIRM=1 not set — SKIPPING governance-relay injection"
        say "         (writes to the chain: two maintainer governance transactions)${C_0}"
        return 0
    fi
    [ -x "$GS014_CLI" ] || { say "  ${C_R}[gs014] node binary not found at $GS014_CLI${C_0}"; return 0; }

    # HARD SAFETY: chain-writing scenario — testnet only, never mainnet.
    local first net
    first="$(echo "$LIVE" | awk '{print $1}')"
    net="$(_gs014_rpc "$(port_of "$first")" getChainInfo | python3 -c "
import sys, json
try: print(json.load(sys.stdin)['result']['network'])
except Exception: pass" 2>/dev/null)"
    if [ "$net" != "testnet" ]; then
        say "  ${C_R}[gs014] network='${net:-unknown}' is not testnet — REFUSING (scenario writes to the chain)${C_0}"
        return 0
    fi

    # ── prerequisite: a live NON-PRODUCER endpoint. This IS the scenario.
    local name ep="" epport=""
    for name in $LIVE; do
        local p; p="$(port_of "$name")"
        if ! _gs014_is_producer_port "$p"; then ep="$name"; epport="$p"; break; fi
    done
    if [ -z "$ep" ]; then
        say "  ${C_Y}[gs014] no live NON-PRODUCER endpoint — every live node carries --producer."
        say "         Submitting to a producer hides the bug entirely (it builds the next"
        say "         block itself), so a pass here would prove nothing. SKIPPING.${C_0}"
        return 0
    fi
    say "  [gs014] non-producer endpoint: ${ep} (rpc ${epport}) — verified no --producer in its args"

    # ── prerequisite: an enforced, on-chain set with room to lose one member ──
    local base count0 digest0 source enforced
    base="$(_gs014_set "$epport")"
    if [ -z "$base" ]; then
        say "  ${C_Y}[gs014] getMaintainerSet unreachable at ${ep} — SKIPPING.${C_0}"; return 0
    fi
    digest0="$(echo "$base" | awk '{print $1}')"
    count0="$(echo "$base"  | awk '{print $2}')"
    source="$(echo "$base"  | awk '{print $3}')"
    enforced="$(echo "$base" | awk '{print $4}')"
    if [ "$source" != "on-chain" ] || [ "$enforced" != "True" ]; then
        say "  ${C_Y}[gs014] maintainer set is source=${source} enforced=${enforced}, not an enforced"
        say "         on-chain set — a governance change cannot apply. SKIPPING.${C_0}"
        return 0
    fi
    if [ "${count0:-0}" -lt 4 ]; then
        say "  ${C_Y}[gs014] set has ${count0} members; removing one would breach MIN_MAINTAINERS=3."
        say "         SKIPPING (refusing to take the set to the floor).${C_0}"
        return 0
    fi

    # ── target + 3 signers, all drawn from the CURRENT set ────────────────────
    # The target is excluded from the signers so the same three keys authorize
    # BOTH halves: after the removal they are still members, so the re-add is
    # signed by a threshold of the reduced set without re-deriving anything.
    local members target="" signers="" kf n=0
    members="$(_gs014_members "$epport")"
    for pk in $members; do
        kf="$(_gs014_keyfile "$pk")"
        [ -n "$kf" ] || continue
        if [ -z "$target" ]; then target="$pk"; continue; fi
        if [ "$n" -lt 3 ]; then signers="$signers $pk"; n=$(( n + 1 )); fi
    done
    signers="$(echo "$signers" | xargs || true)"
    if [ -z "$target" ] || [ "$n" -lt 3 ]; then
        say "  ${C_Y}[gs014] need a target + 3 signer key files under ${GS014_KEYS}; found target='${target:0:12}'"
        say "         and ${n} signer(s). SKIPPING.${C_0}"
        return 0
    fi
    say "  [gs014] target=${target:0:16}… signed by ${n} of the other $(( count0 - 1 )) members"

    local H0; H0="$(_gs014_height "$epport")"
    echo "$digest0 $count0 $target $ep $epport $H0" > "$WORK/gs014_baseline.txt"

    # ── half 1: REMOVE, submitted to the non-producer ────────────────────────
    local sigfile="$WORK/gs014_sig_remove.txt"; : > "$sigfile"
    for pk in $signers; do
        kf="$(_gs014_keyfile "$pk")"
        _gs014_sign remove "$target" "$kf" "$H0" >> "$sigfile"
    done
    if [ "$(wc -l < "$sigfile" | tr -d ' ')" -lt 3 ]; then
        say "  ${C_R}[gs014] could not collect 3 remove signatures — aborting before any write.${C_0}"
        return 0
    fi
    local hsub1 res1
    hsub1="$(_gs014_height "$epport")"
    say "  ${C_R}[gs014] submitting REMOVE to ${ep} (non-producer) at h=${hsub1}${C_0}"
    res1="$(_gs014_submit "$epport" remove "$target" "$sigfile")"
    echo "$res1 $hsub1" > "$WORK/gs014_remove_submit.txt"
    say "  [gs014] remove -> ${res1}"
    case "$res1" in accepted*) ;; *)
        say "  ${C_Y}[gs014] the endpoint did not accept the removal — nothing was written."
        say "         Assertions will report this run as inconclusive.${C_0}"
        touch "$WORK/gs014_injected"; return 0 ;;
    esac

    local happ1; happ1="$(_gs014_wait_count "$epport" $(( count0 - 1 )))"
    echo "$happ1" > "$WORK/gs014_remove_applied.txt"
    if [ "$happ1" = "-1" ]; then
        say "  ${C_R}[gs014] REMOVE never applied within ${GS014_APPLY_TIMEOUT}s — this is the INC-I-195"
        say "         signature itself (accepted by a non-producer, never mined). Set UNCHANGED.${C_0}"
        touch "$WORK/gs014_injected"; return 0
    fi
    say "  [gs014] remove applied at h=${happ1} ($(( happ1 - hsub1 )) blocks after submission)"

    # ── half 2: RE-ADD the same key, same non-producer endpoint ──────────────
    local sigfile2="$WORK/gs014_sig_add.txt"; : > "$sigfile2"
    for pk in $signers; do
        kf="$(_gs014_keyfile "$pk")"
        _gs014_sign add "$target" "$kf" "$happ1" >> "$sigfile2"
    done
    if [ "$(wc -l < "$sigfile2" | tr -d ' ')" -lt 3 ]; then
        say "  ${C_R}[gs014] could not collect 3 add signatures — the set is one member SHORT.${C_0}"
        _gs014_repair_hint "$target"
        touch "$WORK/gs014_injected"; return 0
    fi
    local hsub2 res2
    hsub2="$(_gs014_height "$epport")"
    say "  ${C_C}[gs014] submitting RE-ADD to ${ep} (non-producer) at h=${hsub2}${C_0}"
    res2="$(_gs014_submit "$epport" add "$target" "$sigfile2")"
    echo "$res2 $hsub2" > "$WORK/gs014_add_submit.txt"
    say "  [gs014] add -> ${res2}"
    case "$res2" in accepted*) ;; *)
        say "  ${C_R}[gs014] the endpoint did not accept the re-add — the set is one member SHORT.${C_0}"
        _gs014_repair_hint "$target"
        touch "$WORK/gs014_injected"; return 0 ;;
    esac

    local happ2; happ2="$(_gs014_wait_count "$epport" "$count0")"
    echo "$happ2" > "$WORK/gs014_add_applied.txt"
    if [ "$happ2" = "-1" ]; then
        say "  ${C_R}[gs014] RE-ADD never applied within ${GS014_APPLY_TIMEOUT}s — the set is one member SHORT.${C_0}"
        _gs014_repair_hint "$target"
        touch "$WORK/gs014_injected"; return 0
    fi
    say "  [gs014] re-add applied at h=${happ2} ($(( happ2 - hsub2 )) blocks after submission)"

    # ── final state + fleet agreement ────────────────────────────────────────
    _gs014_set "$epport" > "$WORK/gs014_final.txt"
    : > "$WORK/gs014_fleet.txt"
    for name in $LIVE; do
        local d; d="$(_gs014_set "$(port_of "$name")" | awk '{print $1}')"
        [ -n "$d" ] && echo "$name $d" >> "$WORK/gs014_fleet.txt"
    done
    touch "$WORK/gs014_injected"
}

# _gs014_repair_hint <target> — a partial run left the set short. Print the
# exact command to repair it. NEVER exit quietly on a half-applied governance
# change: the operator must be able to see, and fix, what this scenario did.
_gs014_repair_hint() {
    say "  ${C_Y}[gs014] REPAIR: re-add the key with 3 signatures from the CURRENT set, then"
    say "         POST submitMaintainerChange to any node:"
    say "         $GS014_CLI --network testnet maintainer add --target $1 \\"
    say "             --key $GS014_KEYS/<maintainer>.json --height <tip>${C_0}"
}

# ── assertions ──────────────────────────────────────────────────────────────
# rc 0 pass · 1 fail · 2 skip. Appends to FAIL_REASONS / SKIP_REASONS /
# INFO_REASONS (gauntlet.sh scope).
_gs014_assert() {
    local t="$1"
    if [ ! -f "$WORK/gs014_injected" ]; then
        SKIP_REASONS="$SKIP_REASONS; GS-014 not injected this run (needs --gs014 + GAUNTLET_GS014_CONFIRM=1, a live NON-PRODUCER endpoint, and an enforced on-chain set of >=4) — perturbative, opt-in, writes to the chain"
        return 2
    fi

    local base digest0 count0 ep
    base="$(cat "$WORK/gs014_baseline.txt" 2>/dev/null)"
    digest0="$(echo "$base" | awk '{print $1}')"
    count0="$(echo "$base"  | awk '{print $2}')"
    ep="$(echo "$base"      | awk '{print $4}')"

    local r1 h1 r2 h2 a1 a2
    r1="$(awk '{print $1}' "$WORK/gs014_remove_submit.txt" 2>/dev/null)"
    h1="$(awk '{print $3}' "$WORK/gs014_remove_submit.txt" 2>/dev/null)"
    r2="$(awk '{print $1}' "$WORK/gs014_add_submit.txt"    2>/dev/null)"
    h2="$(awk '{print $3}' "$WORK/gs014_add_submit.txt"    2>/dev/null)"
    a1="$(cat "$WORK/gs014_remove_applied.txt" 2>/dev/null)"
    a2="$(cat "$WORK/gs014_add_applied.txt"    2>/dev/null)"

    case "$t" in
      gs014-relay-accepted)
        # The weakest half of the property, and the one that was ALREADY true
        # before the fix: pre-fix the endpoint answered "accepted" too. It is
        # asserted so that a transport/auth breakage is distinguished from a
        # relay breakage — on its own it proves nothing about the relay.
        if [ -z "$r1" ]; then
          SKIP_REASONS="$SKIP_REASONS; $t: no submission was recorded — nothing measured"; return 2
        fi
        if [ "$r1" != "accepted" ]; then
          FAIL_REASONS="$FAIL_REASONS; $t: the non-producer ${ep} did not accept the removal (${r1})"; return 1
        fi
        if [ -n "$r2" ] && [ "$r2" != "accepted" ]; then
          FAIL_REASONS="$FAIL_REASONS; $t: the non-producer ${ep} did not accept the re-add (${r2})"; return 1
        fi
        INFO_REASONS="$INFO_REASONS; $t: ${ep} accepted both submissions"
        return 0 ;;

      gs014-applies-from-non-producer)
        # THE INC-I-195 PROPERTY. Pre-fix this is exactly what failed: accepted
        # by the non-producer, never mined, set unchanged. A missing apply is a
        # FAILURE, never a skip — "it did not happen" is the regression.
        if [ -z "$r1" ] || [ "$r1" != "accepted" ]; then
          SKIP_REASONS="$SKIP_REASONS; $t: the removal was never accepted, so the relay was never reached"; return 2
        fi
        if [ "${a1:--1}" = "-1" ]; then
          FAIL_REASONS="$FAIL_REASONS; $t: a maintainer change accepted by the non-producer ${ep} NEVER applied within ${GS014_APPLY_TIMEOUT}s — the transaction did not leave the receiving node (INC-I-195)"
          return 1
        fi
        local d1=$(( a1 - h1 ))
        if [ "$d1" -gt "$GS014_APPLY_MAX_BLOCKS" ]; then
          FAIL_REASONS="$FAIL_REASONS; $t: the removal took ${d1} blocks to apply (max ${GS014_APPLY_MAX_BLOCKS}) — relayed, but far slower than the 1-block baseline"
          return 1
        fi
        if [ -z "$r2" ] || [ "$r2" != "accepted" ]; then
          FAIL_REASONS="$FAIL_REASONS; $t: the removal applied in ${d1} block(s) but the re-add was not accepted (${r2:-none}) — the set is one member SHORT"
          return 1
        fi
        if [ "${a2:--1}" = "-1" ]; then
          FAIL_REASONS="$FAIL_REASONS; $t: the re-add was accepted by ${ep} but NEVER applied — the set is one member SHORT"
          return 1
        fi
        local d2=$(( a2 - h2 ))
        if [ "$d2" -gt "$GS014_APPLY_MAX_BLOCKS" ]; then
          FAIL_REASONS="$FAIL_REASONS; $t: the re-add took ${d2} blocks to apply (max ${GS014_APPLY_MAX_BLOCKS})"
          return 1
        fi
        INFO_REASONS="$INFO_REASONS; $t: both changes submitted to the NON-PRODUCER ${ep} reached a producer and applied (remove ${d1} block(s) at h=${a1}, re-add ${d2} block(s) at h=${a2})"
        return 0 ;;

      gs014-set-restored)
        local fin fdig fcnt
        fin="$(cat "$WORK/gs014_final.txt" 2>/dev/null)"
        if [ -z "$fin" ]; then
          FAIL_REASONS="$FAIL_REASONS; $t: the round trip did not complete — final maintainer set was never read; check for a set left one member short"
          return 1
        fi
        fdig="$(echo "$fin" | awk '{print $1}')"; fcnt="$(echo "$fin" | awk '{print $2}')"
        if [ "$fdig" != "$digest0" ] || [ "$fcnt" != "$count0" ]; then
          FAIL_REASONS="$FAIL_REASONS; $t: set NOT restored — digest ${fdig:0:16} count ${fcnt} vs baseline ${digest0:0:16} count ${count0}"
          return 1
        fi
        INFO_REASONS="$INFO_REASONS; $t: digest back to ${digest0:0:16}, ${fcnt} members — state-neutral"
        return 0 ;;

      gs014-fleet-agrees-on-set)
        local n distinct
        if [ ! -s "$WORK/gs014_fleet.txt" ]; then
          SKIP_REASONS="$SKIP_REASONS; $t: no per-node digests collected (round trip did not complete)"; return 2
        fi
        n="$(wc -l < "$WORK/gs014_fleet.txt" | tr -d ' ')"
        distinct="$(awk '{print $2}' "$WORK/gs014_fleet.txt" | sort -u | wc -l | tr -d ' ')"
        if [ "$distinct" != "1" ]; then
          FAIL_REASONS="$FAIL_REASONS; $t: ${distinct} distinct maintainer_set_digest values across ${n} node(s) — governance change diverged the fleet"
          return 1
        fi
        INFO_REASONS="$INFO_REASONS; $t: ${n} node(s) agree on one maintainer_set_digest"
        return 0 ;;

      *)
        FAIL_REASONS="$FAIL_REASONS; unknown gs014 assertion token '$t'"; return 1 ;;
    esac
}
