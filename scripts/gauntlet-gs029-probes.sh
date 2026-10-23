#!/usr/bin/env bash
# gauntlet-gs029-probes.sh — phase, probe and evidence functions for scripts/gauntlet-gs029.sh.
# Sourced by the runner after the devnet library; defines functions only and reads the
# runner's globals. Knobs are documented in the runner header.
# shellcheck disable=SC2034,SC2154

cap() {
  local mh
  mh="$(min_height)" || mh=0
  echo $((($1 > mh ? $1 - mh : 0) * DN_SLOT * 3 / 2 + 60))
}
nb() { echo $((($1 / E + 1) * E)); }
pfield() { producer_json "$1" "$2" | jfield "$3"; }
bfield() { call "$1" getBalance "{\"address\":\"$2\"}" | jfield "$3"; }
txid_in() { sed -nE 's/.*(TX|TX Hash|Transaction hash): ([0-9a-f]{64}).*/\2/p' "$1" | head -1; }
mined_at() { call "$1" getTransaction "{\"hash\":\"$2\"}" | jfield blockHeight; }
cli_err() { tail -1 "$S/cli.$1.out" 2>/dev/null | tr -d '"' | cut -c1-60; }
codes_in() { grep -oE 'ERRTX-[A-Z0-9-]*[A-Z0-9]' "$1" | head -1; }
n_pauth() { printf '%s' "$1" | tr ' ' '\n' | grep -c '^ERRTX-PAUTH001$'; }
distinct() { printf '%s' "$1" | tr ' ' '\n' | sed '/^$/d' | sort -u | tr '\n' ',' | sed 's/,$//'; }
blk_dirs() { local i; for ((i = 0; i < N; i++)); do printf '%s ' "$S/n$i/blk"; done; }

# "<outputs> <type> <amount> <lock>" of tx $1 on node 0; "0 none 0 0" when unreadable.
tx_out() {
  call "$R0" getTransaction "{\"hash\":\"$1\"}" | python3 -I -c '
import json, sys
try:
    o = json.load(sys.stdin)["result"]["outputs"]
    print(len(o), o[0]["outputType"], int(o[0]["amount"]), int(o[0]["lockUntil"]))
except Exception:
    print("0 none 0 0")'
}

# cli_tx TAG WALLET NODE ARGS...: the CLI in DN_CLI as wallet WALLET against node NODE; txid.
cli_tx() {
  local tag="$1"
  shift
  cli_as "$@" </dev/null >"$S/cli.$tag.out" 2>&1
  txid_in "$S/cli.$tag.out"
}

# reach H: every node at height >= H; rc 1 with WHY on a stall of GS029_STALL_SLOTS slots, a
# dead node or the time cap.
reach() {
  local mh last=-1 t0 tp now lim
  t0="$(date +%s)" tp="$t0" lim="$(cap "$1")"
  while :; do
    mh="$(min_height)" || mh=-1
    [ "$mh" -ge "$1" ] && return 0
    now="$(date +%s)"
    [ "$mh" -gt "$last" ] && last="$mh" tp="$now"
    all_alive || { WHY="a node exited (min height $last)"; return 1; }
    [ $((now - tp)) -ge $((GS029_STALL_SLOTS * DN_SLOT)) ] && { WHY="stall at min height $last"; return 1; }
    [ $((now - t0)) -ge "$lim" ] && { WHY="time cap at min height $last"; return 1; }
    sleep 2
  done
}

# --- phase 1: funding and the four producer-side transactions through mixed nodes ---------
mixed_phase() {
  local h f p1 i hs=""
  PHASE=funding
  while :; do
    f="$(bfield "$R0" "$F_ADDR" confirmed)" p1="$(bfield "$R0" "$P1_ADDR" confirmed)"
    is_num "$f" && is_num "$p1" && [ "$f" -ge $((3 * BOND_UNIT)) ] && [ "$p1" -ge $((2 * BOND_UNIT)) ] && break
    h="$(min_height)" || h=0
    [ "$h" -ge $((AH - 38)) ] && { WHY="funder ${f:-none}, producer 1 ${p1:-none} at height $h"; return 1; }
    reach $((h + 1)) || return 1
  done
  PHASE=mixed-txs
  TX_T="$(cli_tx transfer 0 5 send "$NEW_ADDR" 2 --yes)"
  TXH_T="$(wait_tx_mined "$TX_T" $((10 * DN_SLOT)))" || TXH_T=none
  TX_R="$(cli_tx register 4 4 producer register --bonds 1 --yes)"
  TX_A="$(cli_tx addbond 1 2 producer add-bond --count 1 --yes)"
  TX_W="$(cli_tx withdraw 0 0 producer request-withdrawal --count 1)"
  TXH_R="$(wait_tx_mined "$TX_R" $((10 * DN_SLOT)))" || TXH_R=none
  TXH_A="$(wait_tx_mined "$TX_A" $((4 * DN_SLOT)))" || TXH_A=none
  TXH_W="$(wait_tx_mined "$TX_W" $((4 * DN_SLOT)))" || TXH_W=none
  read -r _ TXW_TYPE _ _ <<<"$(tx_out "$TX_W")"
  for i in "$TXH_T" "$TXH_R" "$TXH_A" "$TXH_W"; do is_num "$i" && hs="$hs $i"; done
  # shellcheck disable=SC2086
  MAXH="$(printf '%s\n' $hs | sort -n | tail -1)"
  is_num "$MAXH" || MAXH="$(min_height)" || MAXH=0
  if is_num "$TXH_R"; then
    DN_KEY[4]="$S/w4.json"
    restart_node 4
  fi
  PHASE=boundary
  B3="$(nb "$MAXH")"
  [ "$B3" -le $((AH - 16)) ] || { WHY="last mixed tx at $MAXH, boundary $B3 too close to the gate"; return 1; }
  reach $((B3 + 1)) || return 1
  snap_producers
  root_check
}

snap_producers() {
  local i f=""
  for ((i = 0; i < N; i++)); do
    call "${DN_RPC[$i]}" getProducers >"$S/n$i/producers.json"
    f="$f $S/n$i/producers.json"
  done
  # shellcheck disable=SC2086
  devnet_py producers-norm $f >"$S/norm.json"
  NORM_EQ="$(jfield equal <"$S/norm.json")"
  NORM_SHOW="$(python3 -I -c '
import json, sys
try:
    m = {p[0]: p[1:] for p in json.load(open(sys.argv[1]))["set"]}
except Exception:
    m = {}
g = lambda k: "%s/%s" % tuple(m[k]) if k in m else "absent"
print("new=%s p0=%s p1=%s" % (g(sys.argv[2]), g(sys.argv[3]), g(sys.argv[4])))' "$S/norm.json" "${DN_PK[4]}" "${DN_PK[0]}" "${DN_PK[1]}")"
}

# State roots where every node reports one (height, bestHash); appends <h>:ok|MISMATCH|unaligned.
root_check() {
  local t i h hs root comp keys roots comps
  for ((t = 0; t < 5; t++)); do
    keys="" roots="" comps=""
    for ((i = 0; i < N; i++)); do
      read -r h hs root comp <<<"$(call "${DN_RPC[$i]}" getStateRootDebug | python3 -I -c '
import json, sys
try:
    r = json.load(sys.stdin)["result"]
    print(r["height"], r["bestHash"], r["stateRoot"], "/".join(r[k][:8] for k in ("csHash", "utxoHash", "psHash")))
except Exception:
    print("- - - -")')"
      keys="$keys $h:$hs" roots="$roots $root" comps="$comps n$i=$comp"
    done
    # shellcheck disable=SC2086
    if all_equal $keys; then
      # shellcheck disable=SC2086
      if all_equal $roots; then ROOTS="$ROOTS $h:ok"; else ROOTS="$ROOTS $h:MISMATCH[${comps# }]"; fi
      return 0
    fi
    sleep 2
  done
  ROOTS="$ROOTS unaligned"
}

# --- phase 2: stop an old node by PID, wait 2 slots, start it on HEAD on the same data dir ---
restart_old() {
  local i="$1" h t
  RSTOP[i]="$(best_height "${DN_RPC[$i]}")" || RSTOP[i]=0
  stop_one "${DN_PID[$i]}"
  sleep $((2 * DN_SLOT))
  restart_node "$i" "$GS029_NODE_BIN" "$S/n$i/node.head.log"
  for ((t = 0; t < 90; t++)); do
    h="$(best_height "${DN_RPC[$i]}")" && RFIRST[i]="$h" && return 0
    alive "${DN_PID[$i]}" || break
    sleep 1
  done
  WHY="node $i did not answer after its restart"
  return 1
}

restart_counts() {
  local i
  REBUILD=0 SNAPS=0 EQUIV=0
  for i in 3 5; do
    REBUILD=$((REBUILD + $(grep -c '\[EPOCH_REBUILD\]' "$S/n$i/node.head.log" 2>/dev/null)))
    SNAPS=$((SNAPS + $(grep -cE '\[SNAP_SYNC\]|action=SnapSync' "$S/n$i/node.head.log" 2>/dev/null)))
  done
  for ((i = 0; i < N; i++)); do
    EQUIV=$((EQUIV + $(cat "$S/n$i/"node*.log 2>/dev/null | grep -c 'EQUIVOCATION DETECTED')))
  done
}

# --- phase 7 probes above the gate ---------------------------------------------------------
probe_unsigned_tx() {
  local j id tip
  UTX_CODES="" UTX_IDS="" UTX_FORGE=skipped
  DN_CLI="$GS029_OLD_CLI"
  for ((j = 0; j < N; j++)); do
    id="$(cli_tx "utx$j" 2 "$j" producer add-bond --count 1 --yes)"
    UTX_CODES="$UTX_CODES $(codes_in "$S/cli.utx$j.out" || echo none)"
    is_hex "$id" && UTX_IDS="$UTX_IDS $id"
  done
  DN_CLI="$GS029_CLI"
  [ "$(n_pauth "$UTX_CODES")" = "$N" ] || [ ! -x "$GS029_TX_FORGE" ] && return 0
  tip="$(best_height "$R0")" || tip="$AH"
  "$GS029_TX_FORGE" --kind addbond-untailed --payout normal --payer-key "$S/w2.json" --victim "${DN_PK[2]}" \
    --bonds 1 --lock-until $((tip + 1576)) --ports "$PORTS" >"$S/utx.forge.out" 2>>"$S/txforge.log"
  UTX_FORGE="$(sed -n 's/^SUBMIT port=[0-9]* accepted=[01] code=\([A-Za-z0-9-]*\).*/\1/p' "$S/utx.forge.out" | tr '\n' ' ')"
  id="$(sed -n 's/^FORGED .*txid=\([0-9a-f]*\).*/\1/p' "$S/utx.forge.out" | head -1)"
  is_hex "$id" && UTX_IDS="$UTX_IDS $id"
}

# An unsigned block for the slot after the tip, built while production is paused on every node.
probe_unsigned_block() {
  local i tip slot owner b64 dial="" out
  for ((i = 0; i < N; i++)); do call "${DN_RPC[$i]}" pauseProduction >/dev/null; done
  sleep 2
  tip="$(best_height "$R0")" && slot="$(block_json "$R0" "$tip" | jfield slot)" && is_num "$slot" &&
    b64="$(block_raw "$R0" "$tip")" && UB_H=$((tip + 1))
  if is_num "$UB_H"; then
    owner="$(call "$R0" getSlotSchedule "{\"fromSlot\":$((slot + 1)),\"count\":1}" |
      python3 -I -c 'import json, sys; print(json.load(sys.stdin)["result"]["slots"][0]["producer"])' 2>/dev/null)"
    is_hex "$owner" || owner="${DN_PK[0]}"
    for ((i = 0; i < N; i++)); do dial="$dial,/ip4/127.0.0.1/tcp/${DN_P2P[$i]}"; done
    out="$("$GS029_FORGE" --kind unsigned --tip-b64 "$b64" --height "$UB_H" --slot $((slot + 1)) --producer "$owner" \
      --genesis-time "$DN_GEN_TS" --slot-duration "$DN_SLOT" --dial "${dial#,}" 2>>"$S/forge.log")"
    UB_HASH="$(printf '%s\n' "$out" | sed -n 's/^INJECTED .*hash=\([0-9a-f]*\).*/\1/p' | head -1)"
    is_hex "$UB_HASH" || UB_HASH=none
    sleep 6
    for ((i = 0; i < N; i++)); do [ "$(block_hash "${DN_RPC[$i]}" "$UB_H")" = "$UB_HASH" ] && UB_HELD=$((UB_HELD + 1)); done
  fi
  for ((i = 0; i < N; i++)); do call "${DN_RPC[$i]}" resumeProduction >/dev/null; done
}

probe_signed() {
  SIG_TX="$(cli_tx signed 2 0 producer add-bond --count 1 --yes)"
  PG_TX="$(cli_tx postgate 1 1 producer request-withdrawal --count 1)"
  SIG_H="$(wait_tx_mined "$SIG_TX" $((12 * DN_SLOT)))" || SIG_H=none
  PG_H="$(wait_tx_mined "$PG_TX" $((6 * DN_SLOT)))" || PG_H=none
  read -r PG_NOUT PG_TYPE _ PG_LOCK <<<"$(tx_out "$PG_TX")"
}

head_phases() {
  mixed_phase || return 1
  PHASE=restart
  LIM13="$(min_height)" || LIM13=$((B3 + 1))
  restart_old 3 || return 1
  restart_old 5 || return 1
  RDONE="$(min_height)" || RDONE="$LIM13"
  [ "$RDONE" -le $((AH - 10)) ] || { WHY="restarts done at height $RDONE, past $((AH - 10))"; return 1; }
  BR="$(nb "$RDONE")"
  PHASE=boundary-after-restart
  reach $((BR + 1)) || return 1
  root_check
  PHASE=crossing
  reach $((AH + 1)) || return 1
  PHASE=probes
  probe_unsigned_tx
  probe_unsigned_block
  probe_signed
  reach $((AH + 12)) || return 1
}

# --- evidence ------------------------------------------------------------------------------
final_scan() {
  local i
  sampler_stop
  for ((i = 0; i < N; i++)); do
    FTIP[i]="$(best_height "${DN_RPC[$i]}")" || FTIP[i]=0
    [ "${FTIP[$i]}" -ge 1 ] && devnet_py scan "$S/n$i/blk" "${DN_RPC[$i]}" 1 "${FTIP[$i]}" >/dev/null
  done
  FMIN="$(printf '%s\n' "${FTIP[@]}" | sort -n | head -1)"
}

# forkeval LIM: hash history 1..LIM on every node plus fork-vs-stall over the tips sampled <= LIM.
forkeval() {
  local i f=""
  for ((i = 0; i < N; i++)); do
    awk -v m="$1" '$1 == "T" && $3 <= m' "$S/n$i/samples.txt" >"$S/n$i/t.lim.txt" 2>/dev/null
    f="$f $S/n$i/t.lim.txt"
  done
  # shellcheck disable=SC2086
  devnet_py forkcheck "$S/n0/blk" $f >"$S/fork.json"
  # shellcheck disable=SC2046
  devnet_py history 1 "$1" $(blk_dirs) >"$S/hist.json"
  FORK="$(jfield fork <"$S/fork.json")" HIST_EQ="$(jfield equal <"$S/hist.json")"
  HFIRST="$(jfield first_diff <"$S/hist.json")" FIRST="$HFIRST"
  is_num "$FIRST" || FIRST="$(jfield first_height <"$S/fork.json")"
  is_num "$FIRST" || FIRST=none
  SIDES="$(jlines sides <"$S/fork.json" | awk '{ printf "%s%s:%s", (NR > 1 ? "," : ""), substr($1, 1, 12), substr($2, 1, 8) }')"
  ABSENT="$(jlines absent <"$S/fork.json" | sed '/^$/d' | wc -l | tr -d ' ')"
  ABSENT1="$(jlines absent <"$S/fork.json" | awk 'NR == 1 { print "n" $1 ":" $2 ":" substr($3, 1, 12) }')"
  [ "$FORK" = True ] || is_num "$HFIRST"
}

# Supply: "<heights compared> <first height <= $1 whose per-node most frequent sample differs>".
supply_check() {
  awk -v m="$1" '$1 == "S" && $2 <= m { c[FILENAME SUBSEP $2 SUBSEP $3]++ }
    END {
      for (k in c) { split(k, a, SUBSEP); f = a[1] SUBSEP a[2]; if (c[k] > b[f]) { b[f] = c[k]; v[f] = a[3] } }
      for (f in v) { split(f, a, SUBSEP); h = a[2]; if ((h in r) && r[h] != v[f]) bad[h] = 1; r[h] = v[f] }
      n = 0; for (h in r) n++
      x = ""; for (h in bad) if (x == "" || h + 0 < x + 0) x = h
      print n, (x == "" ? "none" : x) }' "$S"/n*/samples.txt
}

# Longest run of polls whose tips differ across nodes, in slots (no verdict).
diverge_slots() {
  awk -v n="$N" -v sl="$DN_SLOT" '$1 == "T" { v[++c[$2], $2] = $3 " " $4 }
    END {
      m = c[0]; for (i = 1; i < n; i++) if (c[i] < m) m = c[i]
      for (k = 1; k <= m; k++) {
        d = 0; for (i = 1; i < n; i++) if (v[k, i] != v[k, 0]) d = 1
        r = d ? r + 1 : 0; if (r > b) b = r
      }
      print int((b * 2 + sl - 1) / sl) }' "$S"/n*/samples.txt
}
