#!/usr/bin/env bash
# GS-017 hardening cells — sourced by scripts/test_gauntlet_gs017.sh, never run alone.
# Pins the reviewer findings the shipped library does not satisfy:
#   REV-203-003  no M3 precondition -> a DEFAULT run with a stale CLI injects INC-I-203
#   REV-203-004  M2 text and bare `cap` accepted -> an M3 revert is undetectable
#   REV-203-005  window scan covers seq 1 12 only, reports neither scope nor duration
#   REV-203-006  getMempoolTransactions asked with params [] -> a 100-tx sample, no txCount
#   REV-203-007  any addbond FAILs -> in-flight traffic is a blocking false FAIL
# Every helper and fixture it uses comes from the sourcing file.
# shellcheck shell=bash
# shellcheck disable=SC2154  # globals (RC, R_FAIL, CURL_LOG, ...) belong to the sourcing file

# --- O5 predicates: the getMempoolTransactions request body ---

# getMempoolTransactions is hard-capped at limit 500 and has no offset and no cursor
# (crates/rpc/src/methods/stats.rs:188-211), so anything under 500 is an unrepeatable sample.
mempool_reqs()        { grep -cE 'getMempoolTransactions' "$CURL_LOG" 2>/dev/null || true; }
mempool_reqs_at_500() { grep -E 'getMempoolTransactions' "$CURL_LOG" 2>/dev/null | grep -cE '"limit"[[:space:]]*:[[:space:]]*500' || true; }
mempool_reqs_port()   { grep -cE "127\.0\.0\.1:$1 .*getMempoolTransactions" "$CURL_LOG" 2>/dev/null || true; }
version_probed()      { grep -qE -- '--version' "$DOLI_LOG" 2>/dev/null; }

max_page_always() {
    local t; t="$(mempool_reqs)"
    [ "${t:-0}" -gt 0 ] && [ "$t" = "$(mempool_reqs_at_500)" ]
}

page_detail() { echo "getMempoolTransactions reqs=$(mempool_reqs) at_limit_500=$(mempool_reqs_at_500) curl=$CURL_LOG"; }

# --- O2/O4 predicates: a reason an operator can act on ---
# "0 events past the baseline" is unfalsifiable without the scope and the duration it covers.
reason_names_logs()   { printf '%s' "$1" | grep -Eq "(^|[^0-9])$2 [A-Za-z*.()]{0,14}log"; }
reason_names_window() { printf '%s' "$1" | grep -Eiq 'window[^0-9]{0,30}[0-9]+ ?s|[0-9]+ ?s(ec[a-z]*)?[^0-9]{0,30}window'; }

# --- fixtures ---

write_logs_extra() {
    local n
    for n in "$@"; do
        {
            echo "2026-09-02T20:00:00Z INFO Applied block h=94400 hash=050fd33e"
            echo "2026-09-02T20:01:00Z WARN [BLOCK_POISON] apply_block failed on self-produced block at h=94401: block economics invalid: [ADDBOND_CAP_EXCEEDED] producer=PublicKey(3047e96b) current=1 pending=0 in_block_prior=0 requested=3000 max=3000. Purged 1 TXs from mempool."
            echo "2026-09-02T20:02:00Z INFO Applied block h=94402 hash=1a2b3c4d"
        } > "$LOGS_DIR/$n.log"
    done
}

# A pre-window baseline for EVERY n*.log on disk, not just the ones gauntlet.sh named.
write_offsets_file_all() {
    local f n
    : > "$CASE_DIR/offsets.txt"
    for f in "$LOGS_DIR"/n*.log; do
        [ -r "$f" ] || continue
        n="$(basename "$f" .log)"
        printf '%s:%s\n' "$n" "$(wc -c < "$f" 2>/dev/null | tr -d ' ')" >> "$CASE_DIR/offsets.txt"
    done
}

mempool_addbond2() { _mp_addbond_json "$RPC_DIR/$1.mempool2.json" "${2:-$ADDBOND_HASH}"; }
mempool_empty2()   { echo '[]' > "$RPC_DIR/$1.mempool2.json"; }

mempool_info() {
    cat > "$RPC_DIR/$1.mempoolinfo.json" <<INFO
{"txCount":$2,"totalSize":$(( $2 * 420 )),"minFeeRate":1,"maxSize":10000000,"maxCount":5000}
INFO
}

print_header "GS-017 hardening cells (REV-203-003 .. REV-203-007)"

# --- REV-203-003: the M3 precondition (O7) ---

# S29 — REQ-BOND-007 (Must) — Decision: REV-203-003, a precondition that cannot certify a healthy post-M3 CLI is either always-skip (guarding nothing) or always-submit, and GS-017 fires a REAL add-bond on a default run.
new_sandbox "s29_cli_carries_m3"
DOLI_VERSION_MODE=m3 run_assert "$TOKEN_M3"
ck "S29 REV-203-003 cli_carries_m3: rc 0, INFO non-empty, SKIP and FAIL empty" \
   '[[ "$RC" -eq 0 && -n "$R_INFO" && -z "$R_SKIP" && -z "$R_FAIL" ]]'
# getNodeInfo carries no commit hash, so the node's M2 status is unknowable over RPC; recording
# the node version is the only honest thing the precondition can say about the far side.
ck "S29 REV-203-003 cli_carries_m3: INFO records the node getNodeInfo version 6.26.2" \
   '[[ "$R_INFO" == *6.26.2* ]]'

# S30 — REQ-BOND-007 (Must) — Decision: REV-203-003, a pre-M3 CLI submits the over-cap AddBond for real, so calling it a FAIL both waives the scenario and leaves the toxic tx on the fleet.
new_sandbox "s30_cli_predates_m3"
DOLI_VERSION_MODE=pre_m3 run_assert "$TOKEN_M3"
ck "S30 REV-203-003 cli_predates_m3: rc 2 (SKIP, never FAIL), SKIP non-empty, FAIL empty" \
   '[[ "$RC" -eq 2 && -n "$R_SKIP" && -z "$R_FAIL" ]]' \
   "$(detail) pre_m3=$PRE_M3_COMMIT m3=$M3_COMMIT"

# S31 — REQ-BOND-007 (Must) — Decision: REV-203-003, "could not determine the CLI version" is an unactionable skip unless it says WHICH binary answered WHAT, since several doli builds sit on PATH.
new_sandbox "s31_version_without_sha"
DOLI_VERSION_MODE=no_sha run_assert "$TOKEN_M3"
ck "S31 REV-203-003 version_without_sha: rc 2 and SKIP names the binary and its version string" \
   '[[ "$RC" -eq 2 && "$R_SKIP" == *"$BIN_DIR/doli"* && "$R_SKIP" == *6.26.3* ]]'

# S32 — REQ-BOND-007 (Must) — Decision: REV-203-003, an operator running a build from another checkout has a sha this repo cannot resolve, and a FAIL there reports a chain defect for a local git miss.
new_sandbox "s32_sha_not_an_object"
DOLI_VERSION_MODE=unknown_sha run_assert "$TOKEN_M3"
ck "S32 REV-203-003 sha_not_an_object: rc 2 (SKIP), SKIP non-empty, FAIL empty" \
   '[[ "$RC" -eq 2 && -n "$R_SKIP" && -z "$R_FAIL" ]]' \
   "$(detail) absent=$ABSENT_COMMIT"

# S33 — REQ-BOND-007 (Must, safety) — Decision: REV-203-003, a precondition that can reach add-bond turns the gate meant to PREVENT the submit into a second submit path.
new_sandbox "s33_precondition_read_only"
run_assert "$TOKEN_M3"
ck "S33 REV-203-003 precondition_read_only: probed --version, no add-bond, no mutating RPC" \
   'version_probed && ! addbond_attempted && ! mutating_rpc' "$(o5_detail)"

# S34 — REQ-BOND-007 (Must, safety) — Decision: REV-203-003, this is the whole harm: an unguarded CLI on a default run builds, signs and gossips the over-cap AddBond that GS-017 exists to detect.
new_sandbox "s34_pre_m3_cli_must_not_submit"
# shellcheck disable=SC2034  # consumed by run_assert in the sourcing file
DOLI_VERSION_MODE=pre_m3
DOLI_ADDBOND_MODE=accept run_assert "$TOKEN_CLI"
ck "S34 REV-203-003 pre_m3_cli_must_not_submit: rc 2, SKIP non-empty, FAIL empty, NO add-bond invoked" \
   '[[ "$RC" -eq 2 && -n "$R_SKIP" && -z "$R_FAIL" ]] && ! addbond_attempted' \
   "$(detail) $(o5_detail)"

# --- REV-203-004: the refusal must be the CLIENT-side refusal ---

# S35 — REQ-BOND-007 (Must) — Decision: REV-203-004, the M2 node text proves the CLI reached the node, so accepting it reports green on a fleet where the M3 guard was reverted or bypassed.
new_sandbox "s35_m2_text_is_not_a_client_refusal"
DOLI_ADDBOND_MODE=refuse_m2 run_assert "$TOKEN_CLI"
ck "S35 REV-203-004 m2_text_not_client_refusal: rc 1 (FAIL), FAIL non-empty, INFO empty" \
   '[[ "$RC" -eq 1 && -n "$R_FAIL" && -z "$R_INFO" ]]'
ck "S35 REV-203-004 m2_text_not_client_refusal: FAIL says the CLI reached the node / the M3 guard is missing" \
   '[[ "$(lc "$R_FAIL")" == *node* ]] && [[ "$(lc "$R_FAIL")" == *m3* || "$(lc "$R_FAIL")" == *guard* ]]'

# S35b — REQ-BOND-007 (Must) — Decision: REV-203-004, stripping the `RPC error` envelope must not rescue it — the marker itself is emitted by the NODE, so its presence is the finding.
new_sandbox "s35b_bare_m2_marker"
DOLI_ADDBOND_MODE=m2_bare run_assert "$TOKEN_CLI"
ck "S35b REV-203-004 bare_m2_marker: rc 1 (FAIL), FAIL non-empty" '[[ "$RC" -eq 1 && -n "$R_FAIL" ]]'

# S36 — REQ-BOND-007 (Must) — Decision: REV-203-004, `cap` matches capacity/capture/escape, so an unrelated non-zero exit whose text happens to carry any of them reads as a cap refusal.
new_sandbox "s36_vague_cap_words"
DOLI_ADDBOND_MODE=vague_cap run_assert "$TOKEN_CLI"
ck "S36 REV-203-004 vague_cap_words: rc 1 (capacity/capture/escape is not a refusal)" \
   '[[ "$RC" -eq 1 && -n "$R_FAIL" ]]' "$(detail)"

# S37 — REQ-BOND-007 (Must) — Decision: REV-203-004, "refuses BEFORE signing" is the property; a CLI that submits and then fails printed the refusal too late and the tx is already gossiped.
new_sandbox "s37_refusal_after_submit"
DOLI_ADDBOND_MODE=refuse_then_submit run_assert "$TOKEN_CLI"
ck "S37 REV-203-004 refusal_after_submit: rc 1 (the submit line means it did not refuse before signing)" \
   '[[ "$RC" -eq 1 && -n "$R_FAIL" ]]' "$(detail)"

# --- REV-203-005: the window scan must cover the whole fleet and say so ---

# S38 — REQ-BOND-001 (Must) — Decision: REV-203-005, gauntlet.sh builds NODECFG from seq 1 12 while 17 n*.log sit on disk, so poison on n13-n17 is invisible and the token reports green.
new_sandbox "s38_poison_outside_nodecfg"
write_logs_extra n4 n5
write_nodecfg
write_offsets_file_all
append_poison n5 "block economics invalid: [ADDBOND_CAP_EXCEEDED] producer=PublicKey(3047e96b) current=1 requested=3000 max=3000"
run_assert "$TOKEN_POISON"
ck "S38 REV-203-005 poison_outside_nodecfg: rc 1 (FAIL)" '[[ "$RC" -eq 1 ]]'
ck "S38 REV-203-005 poison_outside_nodecfg: FAIL names n5, a log NODECFG never listed" \
   '[[ "$R_FAIL" == *n5* ]]'
ck "S38 REV-203-005 poison_outside_nodecfg: FAIL states 5 logs scanned and the window in seconds" \
   'reason_names_logs "$R_FAIL" 5 && reason_names_window "$R_FAIL"'

# S39 — REQ-BOND-001 (Must) — Decision: REV-203-005, "0 events" is unfalsifiable without the scope and the duration; standalone the baseline equals the tail, so the token can report green over ZERO scanned bytes.
new_sandbox "s39_history_across_all_logs"
write_logs_extra n4 n5
write_nodecfg
write_offsets_file_all
run_assert "$TOKEN_POISON"
ck "S39 REV-203-005 history_across_all_logs: rc 0 and the note states 5 logs scanned" \
   '[[ "$RC" -eq 0 && -z "$R_FAIL" ]] && reason_names_logs "$R_INFO" 5'
ck "S39 REV-203-005 history_across_all_logs: note states the window length in seconds" \
   'reason_names_window "$R_INFO"'

# --- REV-203-006: the sweep must fetch a full page and reconcile it with txCount ---

# S40 — REQ-BOND-002 (Must) — Decision: REV-203-006, params [] means limit 100 over HashMap order with no offset and no cursor, so on a busy node the sweep samples and misses the resident AddBond.
new_sandbox "s40_residency_full_page"
run_assert "$TOKEN_RESIDENCY"
ck "S40 REV-203-006 residency_full_page: every getMempoolTransactions asks for limit 500" \
   'max_page_always' "$(page_detail)"

# S41 — REQ-BOND-002 (Must) — Decision: REV-203-006, the post-refusal read-back is the check that catches "printed a refusal and submitted anyway"; a 100-tx sample makes that green by luck.
new_sandbox "s41_cli_readback_full_page"
DOLI_ADDBOND_MODE=refuse_m3 run_assert "$TOKEN_CLI"
ck "S41 REV-203-006 cli_readback_full_page: the read-back also asks for limit 500" \
   'max_page_always' "$(page_detail)"

# S42 — REQ-BOND-002 (Must) — Decision: REV-203-006, above 500 resident txs one page CANNOT see the mempool, and reporting "no addbond found" over an unfetchable remainder is the vacuous green.
new_sandbox "s42_txcount_exceeds_one_page"
mempool_info 8503 1200
run_assert "$TOKEN_RESIDENCY"
ck "S42 REV-203-006 txcount_exceeds_one_page: rc 1 (FAIL loudly, never sample silently)" \
   '[[ "$RC" -eq 1 ]]' "$(detail)"
ck "S42 REV-203-006 txcount_exceeds_one_page: FAIL names port 8503 and the unfetchable remainder" \
   '[[ "$R_FAIL" == *8503* ]] && [[ "$R_FAIL" == *1200* || "$R_FAIL" == *700* ]]'

# S43 — REQ-BOND-002 (Must) — Decision: REV-203-006, a note that does not say how much of the mempool it saw cannot be told apart from one that saw nothing at all.
new_sandbox "s43_txcount_within_one_page"
mempool_info 8503 120
run_assert "$TOKEN_RESIDENCY"
ck "S43 REV-203-006 txcount_within_one_page: rc 0 and the note states the observed txCount" \
   '[[ "$RC" -eq 0 && -z "$R_FAIL" && "$R_INFO" == *120* ]]' "$(detail)"

# --- REV-203-007: only a tx that SURVIVES the window is a finding ---

# S44 — REQ-BOND-002 (Must) — Decision: REV-203-007, an AddBond that arrives mid-run is normal traffic; failing on a single snapshot makes the DEFAULT gate red for anyone bonding during the run.
new_sandbox "s44_addbond_only_in_second_sweep"
mempool_addbond2 8507
run_assert "$TOKEN_RESIDENCY"
ck "S44 REV-203-007 addbond_only_in_second_sweep: rc 0 and port 8507 was swept twice" \
   '[[ "$RC" -eq 0 && -z "$R_FAIL" ]] && [[ "$(mempool_reqs_port 8507)" -eq 2 ]]' \
   "$(detail) reqs_8507=$(mempool_reqs_port 8507)"

# S45 — REQ-BOND-002 (Must) — Decision: REV-203-007, a tx present at the start and mined during the window is the HEALTHY outcome, and flagging it inverts the assertion's meaning.
new_sandbox "s45_addbond_mined_during_window"
mempool_addbond 8507
mempool_empty2 8507
run_assert "$TOKEN_RESIDENCY"
ck "S45 REV-203-007 addbond_mined_during_window: rc 0 (it settled, that is the pass condition)" \
   '[[ "$RC" -eq 0 && -z "$R_FAIL" ]]' "$(detail)"

# S46 — REQ-BOND-002 (Must) — Decision: REV-203-007, a hash stuck across two slots is the INC-I-203 shape (the node keeps re-admitting it), and the operator needs the hash to trace it.
new_sandbox "s46_same_hash_survives_window"
mempool_addbond 8507
mempool_addbond2 8507
run_assert "$TOKEN_RESIDENCY"
ck "S46 REV-203-007 same_hash_survives_window: rc 1 and FAIL names port 8507 and the stuck hash" \
   '[[ "$RC" -eq 1 && "$R_FAIL" == *8507* && "$R_FAIL" == *988630d9* ]]' "$(detail)"

# S47 — REQ-BOND-002 (Must) — Decision: REV-203-007, matching on COUNT rather than hash makes ordinary churn (one mined, another arrived) look identical to one tx stuck for two slots.
new_sandbox "s47_addbond_churn_is_not_residency"
mempool_addbond 8507
mempool_addbond2 8507 "c0ffee11c0ffee22"
run_assert "$TOKEN_RESIDENCY"
ck "S47 REV-203-007 addbond_churn_is_not_residency: rc 0 (different hashes, nothing survived)" \
   '[[ "$RC" -eq 0 && -z "$R_FAIL" ]]' "$(detail)"

# S48 — REQ-BOND-002 (Must) — Decision: REV-203-007, with a zero settle both sweeps read the same instant, every in-flight tx looks stuck and the false FAIL the filter exists to remove comes straight back.
new_sandbox "s48_settle_spans_two_slots"
mempool_addbond 8507
mempool_addbond2 8507
# shellcheck disable=SC2034  # empty => run_assert leaves GS017_SETTLE_SECS unset (library default)
TEST_SETTLE=""
SETTLE_T0=$SECONDS
run_assert "$TOKEN_RESIDENCY"
SETTLE_ELAPSED=$(( SECONDS - SETTLE_T0 ))
ck "S48 REV-203-007 settle_spans_two_slots: default settle >= 20s (2 x SLOT_DURATION)" \
   '[[ "$SETTLE_ELAPSED" -ge 20 ]]' "elapsed=${SETTLE_ELAPSED}s rc=$RC"

# --- O6: the 4th token has to be reachable from the host runner ---

# S49 — REQ-BOND-007 (Must) — Decision: REV-203-003, a precondition token that lands on the unknown-token arm turns the gate into a FAIL on every run, and the fix for that is a waiver.
if dispatches_token "$TOKEN_M3"; then
    test_result "S49 REV-203-003 dispatch: gauntlet.sh assert() routes $TOKEN_M3" "pass" ""
else
    test_result "S49 REV-203-003 dispatch: gauntlet.sh assert() routes $TOKEN_M3" "fail" \
        "no case arm in $GAUNTLET dispatches $TOKEN_M3 to _gs017_assert"
fi

# S50 — REQ-BOND-007 (Must) — Decision: REV-203-003, assert() is only called with tokens from gauntlet_scenarios, so an unseeded precondition never runs and the submit fires unguarded.
if [[ "$SEED_ASSERTIONS" == *"$TOKEN_M3"* ]]; then
    test_result "S50 REV-203-003 seed_registration: gauntlet-seed.sql lists $TOKEN_M3" "pass" ""
else
    test_result "S50 REV-203-003 seed_registration: gauntlet-seed.sql lists $TOKEN_M3" "fail" \
        "assertions=[$SEED_ASSERTIONS]"
fi
