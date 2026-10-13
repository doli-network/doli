━━━ FINDINGS — 11 total (BLOCKING:0 MEDIUM:0 LOW:11) ━━━

  [F1] LOW conf(0.95, observed) — memory.db protection_mechanisms PM-237-12.action vs crates/mempool/src/zero_flow_mined.rs:190-201 — QA-574-R2-01 (registry drift, the registry half of QA-574-03): the PM-237-12 `action` text still reads "snapshot_for -> mempool read lock: check_system_transaction (...) -> guard dropped -> DelegateBond only: MPTX014 ...", so every zero-flow type appears to take the read-lock phase. In code only a DelegateBond does (`if tx.tx_type == TxType::DelegateBond` at :191); the other four types go straight to `add_system_transaction` under the write lock (:199-201). M12.3 fixed this text in `specs/protocol.md` §7.2, `specs/security_model.md` and the status spec, and updated only `scale_assumptions` and `interacts_with` of PM-237-12. Cosmetic in the same rows: PM-237-14 `domain` is `mempool`, PM-237-12 is `mempool-admission`. Registry text only, no behaviour.
  [F2] LOW conf(0.9, observed) — docs/error-codes.md:264 + docs/rpc_reference.md:361-363 vs crates/mempool/src/zero_flow_mined.rs:88-96 — QA-574-R2-02 (doc precision on `MPTX017`): both texts give the `MPTX017` condition as a DelegateBond "whose txid the tx index places in a canonical block". The code refuses `MPTX017` for any tx-index hit whose indexed height has a canonical hash, with the bucket empty; that includes a txid indexed only on a side block at that height, which the canonical block does not carry. PM-237-14 `trigger_condition` ("at a height that has a canonical block hash") and `docs/cli.md:1321-1323` ("a canonical block may carry") state it correctly, and the M12.3 dev-notes Residuals name the side-block case. Same error code and same retry advice, so no contradiction on the code or the flow. Two smaller gaps: `docs/rpc_reference.md:1273-1274` says `submitMaintainerChange` never returns `MPTX014` or `MPTX016` and does not name `MPTX017` (also DelegateBond only; true by code); the "order inside the mined check" sentences in `specs/protocol.md` §7.2 and the status M12.3 row omit the defensive arm "index hit whose height has no canonical hash -> admitted, 0 budget" (`zero_flow_mined.rs:91-93`, untested, same as at f46aa949).
  [F3] LOW conf(0.9, observed) — bins/node/tests/it/inc_i_237_m12_budget.rs:173-179 + bins/node/tests/it/inc_i_237_m12_common.rs:109-116 — QA-574-R2-03 (test strength, row (f)): `m12_3_sec_020_send_transaction_with_an_empty_bucket_returns_mptx017` asserts `contains("MPTX017")` on the joined string "`data.error_code` + message", so it would also pass if only the message carried the code. `data.error_code = "MPTX017"` holds by code reading: `refuse` builds `"[MPTX017] DelegateBond mined-check budget exhausted, retry"` (`zero_flow_policy.rs:83-88`), `MempoolError::error_code` returns the bracket prefix (`pool.rs:77-89`), `to_structured_json` puts it in `error_code` (`pool.rs:104-110`), and `mempool_error_to_rpc` sends `INVALID_TX` (-32002) with that data (`rpc/methods/transaction.rs:282-284`, `rpc/error.rs:25`). Same shape as MPTX011..016, which round 1 confirmed live (QA-R1). The exact refill rate (2/s) is pinned by the constant and the docs test phrase; the tests bound it from below only (refill after 600 ms gives 1 read, so rate >= 1.67/s) and loosely from above (the probe allows at most 2 reads in about 81 ms).
  [F4] LOW conf(0.9, observed) — docs/error-codes.md:261,272 — QA-574-R2-04 (rest of QA-574-01): the 1-bond case is now documented with real workarounds (AddBond first, or another delegate), so QA-574-01 is fixed. Its second point is still open: the case "a mined copy whose apply had no effect" (for example the delegate was not active at that boundary) is not named. The `MPTX014` row says "already mined", which covers it, and the flow (a) workarounds apply unchanged. Wording only.
  [F5] LOW conf(0.85, observed) — crates/mempool/src/contention_tests.rs:34-36 — QA-574-R2-05 (information): the in-crate cap test `zero_flow_mined_tests.rs` is included by `#[path]` from `contention_tests.rs`, so nextest names it `contention_tests::zero_flow_mined_tests::m12_2_sec_017_cache_keeps_the_newest_10_000_per_instance`. It runs and passes (R2c); only discoverability suffers. The dev-notes record this as the test-writer's include.
  [F6] LOW conf(0.9, observed) — common.md D5 + M12.3 dev-notes Residuals — QA-574-R2-06 (information, for the reviewer): the read budget is global per node. A flood of index-hit DelegateBonds from one registered key can make another peer's replay or side-block-indexed DelegateBond get `MPTX017` / Ignore. Only those two cases ever need a read; honest new delegations spend nothing (row (d)). A refused copy is not forwarded by this hop (Ignore, as for every refusal); an RPC client gets `MPTX017` and retries. The commit's FM1 says the waiting tx "is retried by gossip / RPC": on gossip that is not a retry by this node. The gossip message id is BLAKE3 of the raw message bytes (`crates/network/src/gossip/config.rs:272-275`), so a byte-identical resend inside the duplicate-cache window is dropped before admission, and the node has no pending-tx rebroadcast (grep: none). Recovery on gossip is another peer's admission (each node has its own bucket) or a resend in a different message. The docs make no gossip-retry claim.
  [F7] LOW conf(0.9, observed) — bins/node/src/metrics_gossip.rs — OBS-002 (carried, Should): the REQ-GRV-016 remainder is still not built (file unchanged in this round).
  [F8] LOW conf(0.9, measured) — crates/network/src/service/ — OBS-003 (carried, Could): REQ-GRV-020 not met; directory unchanged in this round.
  [F9] LOW conf(0.95, observed) — common.md working assumptions — OBS-004 (carried): Q1/Q2/Q3 and D1-D5 are orchestrator assumptions, not user-confirmed. The soak verdict depends on Q3 (node-only retained heap); the M12.3 budget scope depends on D5.
  [F10] LOW conf(0.95, measured) — test:inc_i_237_m5_soak::soak — OBS-005/OBS-006 (carried, unchanged): a name filter does not select the 30 s soak (`-E 'test(=soak)'` is needed); the M9 drift probe lives in the gitignored workflow dir (re-run here: `stale_drift_rows=0`).
  [F11] LOW conf(0.9, observed) — bins/node/src/node/startup.rs — OBS-007 (carried, pre-existing): INC-I-247 (devnet init node key / BLS) is still open.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# QA Report: gossip-relay-verdict, RELEASE pass (RUN 574, round 2, INC-I-237 follow-up 3: M12.1 + M12.2 + M12.3)

## Scope Validated
- V = `548b3d68240d83ae1564475b4e3b9975065c266e` (M12.3). Baseline `d0e15686` (v6.43.3). Branch = `git diff d0e15686..V` (M0-M12.3).
- Round 1 of this pass ran at `f46aa949` (M12.2) and passed. The fix diff for this round is `git diff f46aa949..V` = M12.3 only: 16 files, +681/-94, one commit `548b3d68` (REL-574-R1-01..05, QA-574-01..03).
- Criteria: REQ-GRV-SEC-020 rows (a)-(f), REQ-GRV-SEC-021, REQ-GRV-SEC-022, REQ-GRV-043, REQ-GRV-042 as stated in the round-1 review (REL-574-R1-01 SSF) and the M12.3 dev-notes; `docs/error-codes.md` MPTX010-017; every earlier branch criterion (re-run by the full suite, the milestone filters, the four probes and both soaks).
- Round-1 evidence is cited only for paths unchanged in `f46aa949..V` (proof below). Every changed path was re-run or re-read at V.
- Not in scope: testnet deploy, mixed-version run and gauntlet (orchestrator), the security fix-verification (parallel audit), a live devnet run.

### Unchanged paths (proof for citations)
| Path | `git diff --stat f46aa949..V` |
|---|---|
| `crates/core`, `crates/storage`, `bins/node/src/node/apply_block` | empty (0 lines) |
| `crates/core/src/validation`, `crates/core/src/network_params` (and the two above) over `8cc50d33..V` | empty (0 lines) |
| `crates/network`, `bins/node/src/metrics_gossip.rs` | not in the 16-file list |
| `bins/node/src` outside `node/validation_checks/gossip_txs.rs` | not in the 16-file list |
| `bins/node/tests/inc_i_237_m5_soak*` (vs `8cc50d33`) | empty |
| `crates/mempool/src/pool.rs`, `zero_flow_policy.rs`, `zero_flow_standing.rs`, `crates/rpc` | not in the 16-file list |
| any `Cargo.toml` | 0 lines |

## Summary
**Release verdict: PASS.** Every Must criterion is met at V, and the 600 s soak passes.
- REL-574-R1-01 (one key buying unbounded block reads with distinct mined DelegateBonds) is closed. Probe at V: `M12.3-PROBE handle_ms=81.1 body_reads=2` for 1,000 distinct mined DelegateBonds in one message on a 1,900,126-byte block of 7,550 txs. The outcome-metric file has before 23,307.4 ms / 1,000 reads (measured at `f46aa949` + test files) and after 81.3 ms / 2 reads. This run matches the "after" values.
- All six REQ-GRV-SEC-020 acceptance rows (a)-(f) map to passing tests; the code reading agrees with each row. REQ-GRV-SEC-021 (`record` is `pub(crate)`, cap test in `crates/mempool`) and REQ-GRV-SEC-022 (dedupe keyed on BLAKE3 of the full bytes) are met.
- QA-574-03 is fixed in the specs: `specs/protocol.md` §7.2, `specs/security_model.md` and the status M12.2 row match `admit_zero_flow` step by step. QA-574-01 (1-bond workaround) and QA-574-02 (flow (f)) are fixed in the docs.
- Full suite 5,170/5,170 on the first run (R1, the last big run; no `m3_state_session_serve` SIGABRT, so no re-run). clippy and fmt clean.
- The 600 s soak: PASS, node-under-test retained heap 35.8 MiB (bound 64 MiB, node-only per Q3; round 1: 35.8 MiB), honest 600/600, 0 forged zero-flow txs at C.
- No consensus-visible code changed in this round. Rolling deploy, no activation height.

Non-blocking: F1 (PM-237-12 `action` text still has the old read-lock order), F2 (`MPTX017` condition wording vs the side-block case), F3 (row (f) test strength), F4 (rest of QA-574-01), F5/F6 (information), and the carried F7-F11. Agent scale: CONDITIONAL APPROVAL (carried Should gap REQ-GRV-016 remainder and Could gap REQ-GRV-020; every RUN 574 Must and Should is met). Release gate: PASS.

## System Entrypoint
- Harness: `cargo nextest` (test profile) in the worktree at V. The probes, the M8/M11/M12 relay tests and both soaks start real `Node` instances and real libp2p swarms on loopback. The RPC rows call the real `RpcContext::handle_request("sendTransaction")` on the node's own mempool (production wiring passes `self.mempool.clone()` to the RPC context; unchanged since round 1).
- No scratch test file was added this round: the parallel-agent rule allows writing only the report, and a new file under `bins/node/tests/` would be picked up by the other agents' builds. Exploratory checks were done by reading code at V and by the existing tests.
- The 600 s soak ran in the background while only reading (git, sqlite, grep) happened on this agent's side; the reviewer and the auditor ran their own commands on the same host in parallel (host load average was about 10-24 during the soak).

## Mandatory Runs
| # | Command | Wall | Exit | Key numbers |
|---|---|---|---|---|
| R0 | `git status --short` / `git rev-parse HEAD` (start) | <1 s | 0 | 0 lines / `548b3d68240d83ae1564475b4e3b9975065c266e` |
| R2a | `cargo nextest run -p doli-node --test it --no-fail-fast -E 'test(/^inc_i_237_m(8\|11\|12)_/)'` | 38 s | 0 | **92/92** (M8 17, M11 22, M12 53 = round-1 44 - 1 cap row moved + 10 new) |
| R2b | `cargo nextest run -p mempool --no-fail-fast` | 1 s | 0 | **70/70**, incl. `contention_tests::zero_flow_mined_tests::m12_2_sec_017_cache_keeps_the_newest_10_000_per_instance` |
| R2c | `cargo nextest run -p rpc --no-fail-fast` | 1 s | 0 | 117/117 |
| R3a | `cargo nextest run -p doli-node --test it -E 'test(=inc_i_237_m11_probe::m11_audit_p1_509_variant_flood_probe)' --no-capture` | 64 s (test 5.8 s) | 0 | `M11-PROBE variant_forwards=0 middle_residents=1` |
| R3b | same with `test(=inc_i_237_m12_probe::m12_1_audit_p1_515_throwaway_delegation_probe)` | 63 s (test 4.8 s) | 0 | `M12.1-PROBE middle_forwards=0 middle_residents=0` |
| R3c | same with `test(=inc_i_237_m12_mined_probe::m12_2_audit_p1_516_mined_copies_probe)` | 61 s (test 0.26 s) | 0 | `M12.2-PROBE-SETUP block_bytes=1900325 block_txs=7170`; `M12.2-PROBE handle_ms=26.2 body_reads=1` (round 1: 25.7 / 1) |
| R3d | same with `test(=inc_i_237_m12_budget_probe::m12_3_rel_574_r1_01_distinct_mined_probe)` | 66 s (test 0.33 s) | 0 | `M12.3-PROBE-SETUP block_bytes=1900126 block_txs=7550`; **`M12.3-PROBE handle_ms=81.1 body_reads=2`** (outcome file: before 23307.4 / 1000, after 81.3 / 2) |
| R4a | `cargo nextest run -p doli-node --test inc_i_237_m5_soak -E 'test(=soak)' --no-capture` (30 s smoke) | 96 s (test 50.2 s) | 0 | `b_retained_mib=10.4`; ATTRIBUTION coverage 0.988; b_dup 54,103/218,000; deliveries 50,000/50,000; ingress 1,802.4 ids/s; retention 120.9 s; honest 30/30; zero_flow_at_c 0; family 1; junk 1; genuine_txs_at_c 1; max_verdict 0.98 ms; late 0 (round 1: 10.3 MiB) |
| R4b | `SOAK_SECS=600 cargo nextest run -p doli-node --test inc_i_237_m5_soak --run-ignored only -E 'test(soak_10min)' --no-capture` (background; no other command of this agent used cargo until it ended) | test 620.2 s; process 652 s incl. ~31 s build | 0 | `b_retained_mib=35.8`; `BOUND retained_mib=130.892 harness_mib=95.107 (printed, not counted)`; `ATTRIBUTION retained_mib=130.892 attributed_mib=126.717 unattributed_mib=4.175 coverage=0.968`; B rows: dup_cache 218,000 entries, 137.4 B/entry, 25.346 MiB; peer_score_deliveries 50,000, 6.250 MiB; attest_seen 602, 0.026 MiB; mempool 0; total B 31.610 MiB, harness 95.107 MiB; `BOUND b_dup entries=218000 max_entries=218000 b_deliveries entries=50000 max_entries=50000 ingress_ids_per_s=1799.7 effective_retention_s=121.1 stale_window_s=60`; honest 600/600; zero_flow_at_c 0; family 1; junk 1; genuine_txs_at_c 1; votes_at_c 10,327; max_verdict 11.7 ms (bound 2 s; round 1 15.7 ms); late 0; lane_max [20,644, 2,616, 0] B; peak 1,986.3 MiB (harness, not retained). Same command, same bound, same retained-heap measurement as round 1 |
| R5 | `cargo nextest run -p network --test it -E 'test(disclosure)'` | 1 s | 0 | 2/2 PASS |
| R6a | `cargo nextest run -p network --test it -E 'test(m9) \| test(m4_doc_drift)'` | <1 s | 0 | 14/14 |
| R6b | M9 drift probe (gitignored, `<root>` workflow dir), run from the worktree | <1 s | 0 | `stale_drift_rows=0` |
| R7 | `cargo nextest run -p doli-node --test inc_i_173_m3_f4_routing` | 25 s | 0 | 10/10 |
| R1 | `cargo nextest run --workspace --no-fail-fast --status-level fail --final-status-level fail` (foreground, unpiped, the last big run) | 229 s (build 59.5 s, suite 169.0 s) | 0 | **5,170 run, 5,170 passed** (1 slow), 26 skipped. No `m3_state_session_serve` SIGABRT, so no re-run. Count = round-1 5,160 + 10 M12.3 tests |
| R9 | `cargo clippy --workspace --all-targets -- -D warnings` (after R1) | 13 s | 0 | 0 warnings from workspace crates; the 2 pre-existing vendored `libp2p-gossipsub` warnings print, as at round 1 |
| R10 | `cargo fmt --check` | 1 s | 0 | 0 lines |
| R11 | `git status --short` (end) | <1 s | 0 | 3 untracked files: this report, the parallel review and the parallel audit; no tracked file modified |

## Round 2 delta (M12.3)

### What changed (`git diff f46aa949..V`)
- `crates/mempool/src/zero_flow_mined.rs` (202 lines): `ReadBudget { tokens: f64, at: Instant }` (`:31-58`), `Default` = full at `BODY_READ_BURST` (`:36-43`), `take(now)` refills `elapsed_s * BODY_READS_PER_SEC`, caps at the burst, spends 1 iff `tokens >= 1` (`:47-57`). Constants `BODY_READS_PER_SEC = 2` (`:24`), `BODY_READ_BURST = 2` (`:27`). `MinedCheck` owns `budget: Mutex<ReadBudget>` (`:71`) next to the pins. `refuse_if_mined` (`:79-102`): pin hit -> `MPTX014` (`:85-87`); index miss -> `Ok` (`:88-90`); index height with no canonical hash -> `Ok` (`:91-93`); no token -> `MPTX017` (`:94-96`); token -> one body read, carried -> pin + `MPTX014` (`:97-100`), else `Ok`. `record` is `pub(crate)` (`:132`). `admit_zero_flow` (`:182-202`) is unchanged in shape.
- `bins/node/src/node/validation_checks/gossip_txs.rs:15`: `seen.insert(crypto::hash::hash(&tx.serialize()))`; `crypto::hash::hash` is `blake3::hash` (`crates/crypto/src/hash.rs:320-322`).
- Tests: `inc_i_237_m12_budget.rs` (6), `inc_i_237_m12_budget_docs.rs` (3), `inc_i_237_m12_budget_probe.rs` (1); the cap row moved from `inc_i_237_m12_mined_cache.rs` to `crates/mempool/src/zero_flow_mined_tests.rs`.
- Docs/specs: `docs/error-codes.md` (MPTX014 1-bond text, MPTX017 row, flows (a) and (f)), `docs/rpc_reference.md`, `docs/cli.md`, `specs/gossip-relay-verdict-status.md`, `specs/protocol.md` §7.2, `specs/security_model.md`; worktree `docs/.workflow/wiring-debt.md` (4 rows, reason text).

### REQ-GRV-SEC-020 (Must) acceptance rows
| Row | Criterion | Code at V | Test(s) at V (all pass in R2a / R3d / R1) | Met |
|---|---|---|---|---|
| (a) | index hit + empty bucket -> `MPTX017`, no body read, no cache entry, gossip Ignore with no peer penalty | `zero_flow_mined.rs:94-96` returns before `carries` (the only `body_reads` increment, `:123`) and before `record` (`:98`); gossip: every admission `Err` -> `false` (`validation_checks/mod.rs:1033-1036`) -> `Verdict::Ignore` (`gossip_txs.rs:23-27`); `Verdict` has only `Accept`/`Ignore` (`crates/network/src/service/verdict.rs:22-25`), Ignore -> gossipsub `MessageAcceptance::Ignore` (`:75`), no penalty | `inc_i_237_m12_budget::m12_3_sec_020_index_hit_with_an_empty_bucket_reads_no_block_then_refills` (Ignore, not resident, 0 reads), `::m12_3_sec_020_budget_refusal_leaves_no_cache_entry` (Ignore, 0 reads, `is_cached` false; after the refill the same tx costs 1 read, `MPTX014`, pinned), `::m12_3_sec_020_send_transaction_with_an_empty_bucket_returns_mptx017` (code `MPTX017`, not `MPTX014`, 0 reads) | Yes |
| (b) | refill 2 reads/s, burst 2, starts full, monotonic clock | `:24`, `:27`, `:36-43` (full at construction), `:47-57`; `std::time::Instant` (`:9`), `saturating_duration_since` (`:48`), `Instant::now()` at each call (`:94`); `pool.rs` builds `mined_check: Default::default()` (unchanged), so every node starts full | `drain` fixture in `inc_i_237_m12_budget.rs:57-66` (a fresh node spends exactly `BURST` = 2 reads: starts full); `..._reads_no_block_then_refills` (after 600 ms one more read: rate >= 1.67/s); `m12_3_sec_020_distinct_mined_message_reads_at_most_the_burst` and the probe (at most 2 reads in one call); `inc_i_237_m12_budget_docs::m12_3_grv_043_docs_name_mptx017_and_the_read_budget` pins the phrase "2 reads/s with burst 2". The exact rate is pinned by the constant (F3) | Yes |
| (c) | cache hit spends 0 budget | `:85-87` returns `MPTX014` before `budget()` | `::m12_3_sec_020_cache_hit_refused_mptx014_with_an_empty_bucket` (bucket empty, RPC `MPTX014` not `MPTX017`, gossip Ignore, 0 reads) | Yes |
| (d) | index miss (honest new delegation) spends 0 budget and is admitted with an empty bucket | `:88-90` returns `Ok` before `budget()` | `::m12_3_sec_020_fresh_delegate_bond_admitted_with_an_empty_bucket` (fixture asserts `boundary_effect` and `get_tx_block_height == None`; gossip Accept + resident; RPC Ok + resident; 0 reads) | Yes |
| (e) | 1,000 distinct mined DelegateBonds in one message -> 2 reads | the budget caps reads per call at the burst plus refill | `inc_i_237_m12_budget_probe::m12_3_rel_574_r1_01_distinct_mined_probe` (R3d: `body_reads=2`, `handle_ms=81.1`; asserts `1..=BURST`, `BURST` hard-coded 2 in the test), `inc_i_237_m12_budget::m12_3_sec_020_distinct_mined_message_reads_at_most_the_burst` (Ignore, 0 resident, reads in `1..=2`) | Yes |
| (f) | RPC `sendTransaction` returns the `MPTX017` error code string | `refuse("MPTX017", ...)` -> `"[MPTX017] DelegateBond mined-check budget exhausted, retry"` (`zero_flow_policy.rs:83-88`); `error_code()` (`pool.rs:77-89`) -> `to_structured_json` `error_code` (`pool.rs:104-110`) -> `INVALID_TX` -32002 (`rpc/methods/transaction.rs:282-284`, `rpc/error.rs:25`) | `::m12_3_sec_020_send_transaction_with_an_empty_bucket_returns_mptx017` (asserts the joined `error_code` + message string contains `MPTX017` and not `MPTX014`; see F3 on strength) | Yes |

### Other M12.3 requirements
- [x] **REQ-GRV-SEC-021 (Should): PASS.** `MinedCheck::record` is `pub(crate)` (`zero_flow_mined.rs:132`); `grep -n "pub fn record" crates/mempool/src/zero_flow_mined.rs` = no match. `refuse_if_mined` is `pub(crate)` (`:79`). The cap test `m12_2_sec_017_cache_keeps_the_newest_10_000_per_instance` lives in `crates/mempool/src/zero_flow_mined_tests.rs`, included under `#[cfg(test)]` from `contention_tests.rs:34-36` (F5); R2b runs it: PASS. It asserts cap 10,000, 10,001 records -> 10,000 kept, oldest gone, newest kept, a second instance empty. The only `pub` methods left on `MinedCheck` are the four doc-hidden read-only accessors.
- [x] **REQ-GRV-SEC-022 (Could): PASS.** Key = BLAKE3-256 of `tx.serialize()` (`gossip_txs.rs:15`), 32 bytes per entry, never `tx.hash()`. `inc_i_237_m12_mined_dedupe::m12_2_sec_018_*` (4) pass, including `m12_2_sec_018_same_txid_different_input_signature_both_attempted` (INV-NETWORK-004). No test can tell "hash of bytes" from "bytes" (same behaviour by design); code reading confirms the key.
- [x] **REQ-GRV-043 (Should): PASS.** `inc_i_237_m12_budget_docs::m12_3_grv_043_*` (3) pass. Each doc/spec/registry row is read against the code in "Specs/Docs Drift" and "Registry". Remaining drift: F1 (registry text), F2 (wording), F4 (one unnamed case); none is a code/doc contradiction on an error code or an admission flow.
- [x] **REQ-GRV-042 (Must): PASS.** `git diff f46aa949..V -- crates/core crates/storage bins/node/src/node/apply_block | wc -l` = 0; over `8cc50d33..V` the four consensus dirs are empty (0 lines). No `Cargo.toml` change, `CURRENT_PROTOCOL_VERSION` = 8 (`crates/network/src/protocols/status.rs:49`). M12.3 touches only mempool admission and the gossip dedupe key.

### Round-1 notes disposed
| Note | Disposition at V | Evidence |
|---|---|---|
| QA-574-01 (MPTX014 workaround impossible at 1 bond) | **Fixed** (main point); second point still open as F4 | `docs/error-codes.md:261` MPTX014 row and `:272` flow (a) name the 1-bond limit and "AddBond first ... or delegate to a different delegate"; `docs/rpc_reference.md:372-377`; `docs/cli.md:1315-1320`; test `m12_3_grv_043_docs_state_the_one_bond_limit` PASS. Matches round-1 QA-R4 (1-bond same tuple `MPTX014`; 2 bonds `MPTX016`; other delegate `Ok`). The "mined copy whose apply had no effect" case is still unnamed (F4) |
| QA-574-02 (queued no-op DelegateBond makes admission stricter) | **Fixed by documentation** (code unchanged by design, as the round-1 review suggested) | `docs/error-codes.md:277` flow (f), `docs/rpc_reference.md:386-389` flow (f), status spec AUDIT-P1-515 row "Not looser only (REL-574-R1-03 / QA-574-02) ..." with the workaround; test `m12_3_grv_043_error_codes_list_flow_f` PASS. `zero_flow_policy.rs` is unchanged since round 1, so round-1 QA-R5 still describes the behaviour. The example "fully withdraws" is right: a full withdrawal sets `ProducerStatus::Exited` (`crates/storage/src/producer/info.rs:442-445`) |
| QA-574-03 (brief wording: consensus-dir diff, over-cap code, read-lock phase text) | **Fixed** in the brief and the specs; registry half open as F1 | common.md now states the `post_commit.rs` +3 hit and the empty `8cc50d33..V` check, and maps over-cap to `MPTX013`. `specs/protocol.md:2096` ("A DelegateBond then takes three phases ... Registration, RevokeDelegation, AddMaintainer and RemoveMaintainer take no read-lock phase and no mined check"), `specs/security_model.md:637` and the status M12.2 row (`:22`) match `admit_zero_flow` (`zero_flow_mined.rs:190-201`) and the check order in `check_system_transaction` (`pool.rs:919-1009`: residency, size, `validate_transaction`, `zero_flow_policy::check` = `MPTX012`/`MPTX011` -> `MPTX010` -> `MPTX016` -> `MPTX013`, then newness `probe`). PM-237-12 `action` still has the old text (F1) |
| QA-574-04 (`AUDIT-P1-516` still open in memory.db) | **Fixed** (orchestrator) | `findings`: `AUDIT-P1-516 fixed P1`, `REL-573-R2-01 fixed P1` |
| QA-574-05 (information) | Superseded | the double signature check per admitted DelegateBond is unchanged; M12.3 adds no per-tx cost besides one BLAKE3 hash per tx in a message |

## Traceability Matrix Status (all rows re-run at V)
| Requirement | Priority | Tests / probes (run at V) | Has tests | Pass | Met |
|---|---|---|---|---|---|
| REQ-GRV-SEC-020 | Must | `inc_i_237_m12_budget::m12_3_sec_020_*` (6), R3d | Yes | Yes | Yes (rows (a)-(f)) |
| REQ-GRV-SEC-021 | Should | `contention_tests::zero_flow_mined_tests::m12_2_sec_017_cache_keeps_the_newest_10_000_per_instance` (R2b) | Yes | Yes | Yes |
| REQ-GRV-SEC-022 | Could | `inc_i_237_m12_mined_dedupe::m12_2_sec_018_*` (4) | Yes | Yes | Yes |
| REQ-GRV-043 | Should (priority not stated in the review or the dev-notes; read as Should like REQ-GRV-039/041) | `inc_i_237_m12_budget_docs::m12_3_grv_043_*` (3) + reading | Yes | Yes | Yes (F1, F2, F4 low) |
| REQ-GRV-042 | Must | consensus-shape diffs (R0 script) + R7 | Yes | Yes | Yes |
| REQ-GRV-SEC-012 / -013 / -014 / -015, REQ-GRV-037 / -038 / -039 (M12.1) | Must/Should | `inc_i_237_m12_standing::*` (10), `inc_i_237_m12_bound::*` (4), `inc_i_237_m12_guards::*` (5), M8 admission/block/relay (17), R3b | Yes | Yes | Yes (QA-574-02 now documented as flow (f)) |
| REQ-GRV-SEC-016 / -017 / -018 / -019, REQ-GRV-040 / -041 (M12.2) | Must/Should | `inc_i_237_m12_mined_cost::*` (11), `inc_i_237_m12_mined_cache::*` (5), `inc_i_237_m12_mined_dedupe::*` (4), `inc_i_237_m12_mined_docs::*` (2), cap row in `mempool`, R3a, R3c | Yes | Yes | Yes |
| REQ-GRV-033..036, REQ-GRV-SEC-011 (M11) | Must | M11 suites (22) in R2a, rpc admission rows in R2c, R3a | Yes | Yes | Yes |
| REQ-GRV-SEC-002 / SEC-006, REQ-GRV-023/024 (M6) | Must/Should | M6 tests in R1; R4 honest delivery | Yes | Yes | Yes |
| REQ-GRV-007 | Must | R4a, R4b | Yes | Yes | Yes (35.8 MiB < 64) |
| REQ-GRV-026 / 027 / 028 (M7) | Must/Must/Should | R4 attribution and retention lines; M7.2 tests in R1 | Yes | Yes | Yes (35.8 MiB < 64) |
| REQ-GRV-SEC-007 / SEC-008, REQ-GRV-029/030/031 (M8) | Must/Should | R2a, R2c, R1 | Yes | Yes | Yes |
| REQ-GRV-032 (M9) | Should | R6a, R6b | Yes | Yes | Yes |
| REQ-GRV-SEC-010 | Must | R5 + Disclosure section | Yes | Yes | Yes |
| REQ-GRV-SEC-009 | Should | INC-I-246 open + stop-gaps | Yes | Yes | Yes (residuals documented) |
| REQ-GRV-025 | Won't | status-spec open item | — | — | Deferred |
| REQ-GRV-016 | Should | reader tests (R1) | Yes | Yes | **No** (F7), non-blocking |
| REQ-GRV-020 | Could | `m4_source_pins` (R1) | Yes | Yes | **No** (F8), non-blocking |

### Gaps Found
- Row (f): the RPC test does not pin `data.error_code` apart from the message (F3); code reading closes it.
- Row (b): the exact refill rate is pinned by the constant and a docs phrase, not by a test that would fail at, say, 5 reads/s (F3).
- The defensive arm "index hit whose height has no canonical hash -> admitted, 0 budget" (`zero_flow_mined.rs:91-93`) has no test (unchanged from round 1; recorded in the M12.2 and M12.3 dev-notes).
- Cross-peer starvation (D5) has no test; it is a documented residual for the reviewer (F6).

## Acceptance Criteria Results
### M12.3 (REL-574-R1-01..05, QA-574-01..03)
- [x] **REQ-GRV-SEC-020 (Must): PASS** — rows (a)-(f) above.
- [x] **REQ-GRV-SEC-021 (Should): PASS.**
- [x] **REQ-GRV-SEC-022 (Could): PASS.**
- [x] **REQ-GRV-043 (Should): PASS** as written (F1, F2, F4 low).
- [x] **REQ-GRV-042 (Must): PASS.**

### M12.1 and M12.2
- Re-run by R2a (92/92), R2b, R2c, R3b, R3c and R1. `zero_flow_policy.rs`, `zero_flow_standing.rs`, `zero_flow_newness.rs`, `zero_flow_auth.rs` and `pool.rs` are unchanged in this round, so the round-1 code reading of REQ-GRV-SEC-012..019 and REQ-GRV-037..041 still holds. In `zero_flow_mined.rs` the cache logic moved into `pinned_canonical` / `carries` with the same outcomes; REQ-GRV-SEC-017 rows (`m12_2_sec_017_*`, 5 node rows + the cap row) pass, and R3c gives `body_reads=1` for 1,000 copies as in round 1.

### Earlier milestones (M0-M11 + docs commit `8cc50d33`)
- Re-run by R1, R2a, R3a, R4a/R4b, R5, R6, R7. The network crate, the node source outside `gossip_txs.rs`, `crates/core`, `crates/storage` and the soak test are unchanged in this round.

## End-to-End Flow Results
| Flow | Result | Notes |
|---|---|---|
| 1,000 distinct mined DelegateBonds from one key in one message (R3d) | PASS | 2 body reads, 81.1 ms, Ignore, 0 resident (before M12.3: 1,000 reads, 23.3 s) |
| Budget drained by 2 index hits, then an honest new DelegateBond on gossip and RPC (row (d) test) | PASS | Accept + resident on both; 0 reads |
| Budget drained, then a replay on RPC (row (f) test) | PASS | `MPTX017`, not resident, 0 reads; 600 ms later the same tx: 1 read, `MPTX014`, pinned |
| 1,000 copies of one mined DelegateBond in one message (R3c) | PASS | 1 read, 26.2 ms |
| 100 throwaway DelegateBonds A -> B -> C (R3b) | PASS | B forwards 0, holds 0 |
| Keyless maintainer variant flood A -> B -> C (R3a) | PASS | 0 forwards, 1 resident |
| 600 s flood + honest load on the M12.3 admission path (R4b) | PASS | every bound holds; honest 600/600; genuine_txs_at_c 1; retained 35.8 MiB |

## Exploratory Testing Findings
No scratch test this round (see System Entrypoint). Code-reading probes at V:
| # | Tried | Expected | Actual | Severity |
|---|---|---|---|---|
| 1 | Concurrent takers (event loop + RPC tasks) on the bucket | one lock, no lost update | `budget()` is a `Mutex` held only for the arithmetic (`:47-57`, `:168-170`); `Instant::now()` is read after the lock is taken, so the stored `at` never moves backward between holders; `saturating_duration_since` guards it anyway | none |
| 2 | Budget refusal followed by a cache lookup | no pin from a refusal | `record` is reached only after `carries` returns true (`:97-99`); row (a) test confirms | none |
| 3 | Index hit on a side block with tokens left | 1 read, admitted | `carries` false -> `Ok` (`:97-101`); `m11_tx_indexed_only_on_a_side_block_is_still_admitted` PASS | none |
| 4 | Same, bucket empty | per D5: `MPTX017` / Ignore | code `:94-96` | low (F2 wording, F6) |
| 5 | Two copies with one txid but different input signatures in one message | both attempted | key = BLAKE3 of full bytes; `m12_2_sec_018_same_txid_different_input_signature_both_attempted` PASS | none |
| 6 | Re-sending the same gossip message after an Ignore | not re-processed inside the duplicate window | message id = BLAKE3 of the raw data (`gossip/config.rs:272-275`); no pending-tx rebroadcast in the node | low (F6) |
| 7 | `submitMaintainerChange` and `MPTX017` | never returned | maintainer changes never reach the mined check (`zero_flow_mined.rs:191`) | low (F2: not named in the doc) |
| 8 | Restart | full bucket, empty cache | `MinedCheck` is `#[derive(Default)]` inside `Mempool`; no persisted state | none |

## Failure Mode Validation
### M12.3 (commit `Failure-Modes:` block)
| Failure mode | Triggered | Detected | Recovered | Degraded OK | Notes |
|---|---|---|---|---|---|
| FM1 honest replay / side-block-indexed DelegateBond waits | Yes (row (a)/(f) tests) | — | Yes on retry after one refill (600 ms in the tests) | Yes | on gossip the retry is not by this node (F6) |
| FM2 reorg burst of side-block-indexed txs | No (read) | — | drains at 2/s | Yes | no pin from a budget refusal (row (a) test) |
| FM3 restart | No (read) | — | full bucket | Yes | `Default` at construction |
| FM4 attacker cycling distinct mined txids | Yes (R3d, row (e) test) | — | — | Yes | 2 reads, 81.1 ms per 1,000-tx message |
| FM5 RPC `MPTX017` vs `MPTX014` | Yes (row (f), (c) tests) | — | — | Yes | distinct codes |
| FM6 mixed-version window | No (read) | — | — | Yes | verdict shape unchanged (Ignore) |
| FM7 clock | No (read) | — | — | Yes | `std::time::Instant` only |

### M12.1 / M12.2
Unchanged from round 1 (code paths unchanged except the moved cache helpers): FM1 reorg row `m12_2_sec_017_reorged_out_carrier_readmits_the_next_copy` PASS; FM5 lock row `m12_2_sec_016_mined_refusal_completes_under_a_held_mempool_read_guard` PASS; the others read only.

## Security Validation
The parallel fix-verification audit is the primary assessment. QA probes:
| Surface | Test | Result |
|---|---|---|
| Block-read amplification by distinct mined DelegateBonds (REL-574-R1-01) | R3d, row (e) test | PASS (2 reads per message, about 2 reads/s sustained) |
| Budget refusal turned into a permanent "mined" verdict | row (a) no-cache test | PASS (no pin; later full check) |
| Honest new delegations starved by a drained bucket | row (d) test | PASS (0 budget, admitted) |
| Cross-peer starvation of replays / side-block txs | read | Accepted residual (D5, F6), for the reviewer |
| Block-read amplification by copies (AUDIT-P1-516) | R3c | PASS (1 read) |
| Throwaway-key delegation relay (AUDIT-P1-515) | R3b | PASS (0 forwards) |
| Dedupe suppressing a distinct tx (INV-NETWORK-004) | `m12_2_sec_018_same_txid_different_input_signature_both_attempted` | PASS |
| Cache writer reachable from outside the crate | `record` is `pub(crate)` | PASS |
| Memory exhaustion by distinct ids | R4b | PASS (35.8 MiB < 64 MiB) |

## Consensus-Shape Check (INV-12) and Deploy Classification
| Check | Result |
|---|---|
| `git diff --stat 8cc50d33..V -- crates/core/src/validation bins/node/src/node/apply_block crates/storage crates/core/src/network_params` | empty (0 lines) |
| Positive control: same four dirs over `d0e15686..V` | `bins/node/src/node/apply_block/post_commit.rs` +3 only |
| The +3 hunk | `:80` `crate::metrics_gossip::observe_attestation_coverage(idx.len(), decode_len);` (M0: one histogram observation, `metrics_gossip.rs:228-232`) and `:450-451` `self.attest_seen.prune(block.header.slot);` (M5: prunes the attestation gossip first-seen set `AttestSeen`, used only by `gossip_verdicts.rs`). Neither writes chain, UTXO or producer state or the state root. Not consensus-visible |
| `git diff f46aa949..V -- crates/core crates/storage bins/node/src/node/apply_block` | 0 lines |
| `CURRENT_PROTOCOL_VERSION` | 8, unchanged; no `Cargo.toml` change in `f46aa949..V` |
| Q1 user-tx reach YES; Q2 producer reach NO (mempool only); Q3 bit-identical validity and apply YES (admission can now Ignore an index-hit DelegateBond with the bucket empty; block validity and apply are untouched) | **Rolling deploy, no activation height, no block-content change** |

## Disclosure Check (REQ-GRV-SEC-010)
| Scan | Result |
|---|---|
| `git log --format=%B f46aa949..V` (135 lines) with the brief pattern | 0 |
| added lines of `git diff f46aa949..V` (697) with the same pattern; same over `d0e15686..V` | 0 / 0 |
| IPv4 addresses and the brief's host-log word list in the M12.3 added lines | 0 / 0 |
| Positive controls (the pattern and the IPv4 regex on a known matching string) | 1 / 1 |
| R5 | 2/2 PASS |
| This report: the brief's grep on the written file | 0 hits (run after writing) |

## Specs/Docs Drift (code read first, then the text; the code wins)
| File | Documented | Actual (code at V) | Severity |
|---|---|---|---|
| `docs/error-codes.md:264` MPTX017 row | DelegateBond index hit with the budget spent; one bucket per node, "2 reads/s with burst 2", memory only, full at start; no read, no cache entry, not "mined"; retry (one read refills every 500 ms); gossip Ignore, no penalty; cache hits and index misses never spend | matches `zero_flow_mined.rs:24-57,79-102`, except "places in a canonical block" (side-block-indexed txid at a canonical height also gets `MPTX017`) | low (F2) |
| `docs/error-codes.md:261` MPTX014 row + `:272` flow (a) | deterministic signer; 1-bond delegator has no new tx for the same delegate; AddBond first or another delegate; a different `bond_count` only while one is unused | matches round-1 QA-R4 and `delegate_bonds_capped` (`set_delegation.rs`: `bond_count <= bond_count - withdrawal_pending_count`) | none (QA-574-01 fixed; F4 rest) |
| `docs/error-codes.md:277` flow (f) | queued DelegateBond counted as effective without its own boundary preconditions -> `MPTX016`; stricter for up to one epoch; revoke-then-delegate workaround | matches round-1 QA-R5; `zero_flow_policy.rs` unchanged | none (QA-574-02 fixed) |
| `docs/rpc_reference.md:354-366` `sendTransaction` errors | `-32002` with `data.error_code`; `MPTX017` (index hit, budget spent, retry, nothing cached, not "mined"); body reads draw on one per-node budget (2 reads/s, burst 2); cache hits and index misses spend none | matches (`transaction.rs:282-284`, `pool.rs:77-110`), same F2 wording | low (F2) |
| `docs/rpc_reference.md:372-377,386-389` flows (a), (f) | as error-codes | matches | none |
| `docs/rpc_reference.md:1273-1274` `submitMaintainerChange` | never returns `MPTX014` / `MPTX016` | true; `MPTX017` is not named (also never returned) | low (F2) |
| `docs/cli.md:1315-1323` delegation bullets | 1-bond limit + AddBond / other delegate; "Node busy checking a replay ... `MPTX017`. Retry after a second" | matches (refill one token per 500 ms) | none |
| `specs/protocol.md:2096` §7.2 | per-type admission order (DelegateBond three phases; the other four types straight to the write lock with the listed checks); M12.3 read budget "2 reads/s with burst 2"; order inside the mined check | matches `admit_zero_flow` (`:190-201`) and `check_system_transaction` (`pool.rs:919-1009`) step by step; the "order inside the mined check" omits the no-canonical-hash arm (`:91-93`) | none / low (F2) |
| `specs/security_model.md:637` | per-type admission order; budget "2 reads/s with burst 2", `MPTX017` (retry), no read, no cache entry, Ignore; "at most about 2 block bodies per second" | matches | none |
| `specs/gossip-relay-verdict-status.md:22` M12.2 row | per-type order (QA-574-03) | matches | none |
| `specs/gossip-relay-verdict-status.md:23` M12.3 row | bucket, order inside the mined check, `MPTX017` gossip Ignore / RPC -32002 `data.error_code`, `record` crate-private, BLAKE3 dedupe key, probe 1,000 -> at most 2 | matches (no-canonical-hash arm omitted) | none |
| status AUDIT-P1-515 row FM2, AUDIT-P1-516 row FM3 | FM2 "Not looser only" + flow (f); FM3 corpus "not finite", bounded by the budget | matches round-1 QA-R5 and REL-574-R1-01 | none |
| status traceability `:67-70` | SEC-020..022, GRV-043 rows name the files and tests | all named tests exist and pass | none |

## Registry (memory.db `protection_mechanisms`; `.schema protection_mechanisms` run first)
| Row | Field | Claim (dev-notes) | At V | Drift vs code |
|---|---|---|---|---|
| PM-237-14 | exists, `status` active, `incident_id` INC-I-237, `domain` `mempool` | INSERT | present | `domain` differs from PM-237-12 `mempool-admission` (cosmetic, F1) |
| PM-237-14 | `trigger_condition` | index-hit DelegateBond not pinned, past every cheaper refusal, canonical hash present | "not pinned ... and the chain tx index places its txid at a height that has a canonical block hash" | none (matches `:85-93`) |
| PM-237-14 | `action` | token -> read/pin; empty -> `MPTX017` no read no pin; Ignore / RPC -32002 | same, plus "Cache hits (MPTX014) and tx-index misses never take a token" | none |
| PM-237-14 | `scale_assumptions` (limits) | 2/s burst 2, ~25 ms/read -> <= ~50 ms/s, wait <= 500 ms per token, new delegations never spend, global per node, memory only, restart full, `Instant` | all present | none (the ~25 ms/read is consistent with R3c 26.2 ms for one read + 1,000 index gets) |
| PM-237-14 | `interacts_with` | [PM-237-02, -08, -12, -13] | `["PM-237-02","PM-237-08","PM-237-12","PM-237-13"]` | none |
| PM-237-14 | `source_files` | `zero_flow_mined.rs`, `gossip_txs.rs`, rpc `transaction.rs` | same three | none |
| PM-237-12 | `scale_assumptions` | "finite corpus" dropped; corpus unbounded per key; budget PM-237-14; BLAKE3 dedupe | all present ("The corpus ... is NOT finite") | none |
| PM-237-12 | `interacts_with` | += PM-237-14 | `[..., "PM-237-13", "PM-237-14"]` | none |
| PM-237-12 | `action` | not claimed as updated | still "snapshot_for -> mempool read lock: check_system_transaction (...) -> guard dropped -> DelegateBond only: MPTX014 ..." | **drift**: only a DelegateBond takes the read-lock phase (`zero_flow_mined.rs:191`) (F1) |

## Wiring Census (`git diff f46aa949..V -- '*.rs'`)
| Item | Result |
|---|---|
| Added `pub fn` lines | 3, all in `bins/node/tests/it/inc_i_237_m12_budget_probe.rs` (test code: `Corpus::resident`, `Corpus::deliver`, `distinct_corpus`), not counted |
| Changed visibility | `MinedCheck::record`: `pub fn` -> `pub(crate) fn` (1 removed `pub fn` line) |
| Added non-test `pub fn` | **0** |
| Added `pub const` (non-test) | `BODY_READS_PER_SEC`, `BODY_READ_BURST` (`zero_flow_mined.rs:24,27`), used by `ReadBudget` in production |
| Positive control (`8cc50d33..f46aa949`, same grep on `zero_flow_mined.rs`) | 7 added `pub fn` lines found |
| Worktree `docs/.workflow/wiring-debt.md:27-30` | `body_reads`, `index_gets`, `cached_len`, `is_cached`: `test-infra`, reason "the only pub accessors left on `MinedCheck` (the cache writer `record` is pub(crate) since M12.3)". Accurate: these four are the only `pub` methods on `MinedCheck` at V (`zero_flow_mined.rs:144-162`; `refuse_if_mined` and `record` are `pub(crate)`), and a grep of non-test files finds no production caller of any of them |
Census: PASS.

## Blocking Issues (must fix before merge)
- None.

## Non-Blocking Observations
- **F1 (QA-574-R2-01):** update the PM-237-12 `action` text to the per-type order (orchestrator / registry; no code).
- **F2 (QA-574-R2-02):** say "whose txid the tx index places at a height that has a canonical block (the block may not carry it)" in `docs/error-codes.md:264` and `docs/rpc_reference.md:361-363`; name `MPTX017` in the `submitMaintainerChange` never-returns sentence; optionally add the no-canonical-hash arm to the order sentences.
- **F3 (QA-574-R2-03):** assert `data.error_code == "MPTX017"` on its own in the row (f) test.
- **F4 (QA-574-R2-04):** name the "mined copy whose apply had no effect" case under MPTX014 / flow (a).
- **F5, F6:** information. F6 is the D5 residual for the reviewer, plus the commit FM1 wording ("retried by gossip").
- **F7-F11:** carried.

## Modules Not Validated
- **Live devnet re-run.** Not repeated. The changed path is the tx admission path; it runs in-process over real loopback swarms (R3a-R3d, R4) and through the real RPC handler.
- **Release binary rebuild.** Not done. R1 and R9 compile the whole workspace at V.
- **Testnet deploy, mixed-version check and gauntlet.** Orchestrator steps.
- **Cross-peer starvation (D5) and FM2 reorg burst.** Read only; no test drives two peers against one bucket.
- **Exploratory scratch rows.** Not run this round (parallel-agent write rule); round-1 QA-R1..R6 still describe unchanged code paths.

## Final Verdict
All Must requirements of this round (REQ-GRV-SEC-020 rows (a)-(f), REQ-GRV-042) and every earlier Must requirement, including REQ-GRV-007 (600 s soak, 35.8 MiB < 64 MiB node-only), are met at V. The full suite passes 5,170/5,170, clippy and fmt are clean, the M12.3 probe prints `handle_ms=81.1 body_reads=2`, and no code/doc contradiction on an error code or an admission flow was found. There are no blocking issues. REQ-GRV-SEC-021, REQ-GRV-SEC-022 and REQ-GRV-043 are met; F1-F4 are low, non-blocking (registry text, wording, test strength, one unnamed case). The carried Should gap is the REQ-GRV-016 remainder (F7) and the Could gap REQ-GRV-020 (F8). Agent scale: CONDITIONAL APPROVAL. Release gate: PASS. Q1/Q2/Q3 and D1-D5 stay orchestrator assumptions, not user-confirmed (F9).

Release-Validated: 548b3d68240d83ae1564475b4e3b9975065c266e
Release-Verdict: PASS
