━━━ FINDINGS — 4 total (P3:4) ━━━

  [F1] P3 conf(0.85, observed) — crates/mempool/src/zero_flow_mined.rs:94-96 + specs/gossip-relay-verdict-status.md:23,46 — REL-574-R2-01: decision D5 residual (one global read budget per node, so a flood of index-hit DelegateBonds makes another peer's side-block-indexed DelegateBond get MPTX017 / Ignore for as long as the flood lasts) is ACCEPTED for this release as a P3 follow-up; the shipped status spec does not record it
  [F2] P3 conf(0.85, inferred) — crates/mempool/src/zero_flow_mined.rs:47-57 — REL-574-R2-02: the burst cap `.min(BODY_READ_BURST)` (the property that bounds reads after an idle period) is pinned by no test; every M12.3 test spends the budget less than 0.5 s after node construction, and no unit test drives `ReadBudget::take` with synthetic instants
  [F3] P3 conf(0.85, observed) — git:548b3d68 commit message, Failure-Modes FM1/FM2 + Path-Coverage — REL-574-R2-03: FM1 says an honest replayed or side-block-indexed DelegateBond waits at most 500 ms and is retried by gossip, which is false under the D5 flood and false for gossip (300 s duplicate cache); the let-else early return at zero_flow_mined.rs:91 has no Path-Coverage row
  [F4] P3 conf(0.90, observed) — crates/mempool/src/contention_tests.rs:34-36 — REL-574-R2-04: the in-crate cap test module is included through `#[path]` from the unrelated contention_tests.rs, which is already over the 800-line test-file budget (1,234 -> 1,238 lines); the test runs as `contention_tests::zero_flow_mined_tests::...`

  Speculative: 1 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Code Review (RELEASE gate, round 2 of 2): INC-I-237 gossip relay verdict, RUN 574

## Scope Reviewed

- V = `548b3d68240d83ae1564475b4e3b9975065c266e` (M12.3). Fix diff = `git diff f46aa949..V`: 16 files, +681/-94. The tree was clean at the start of the review.
- Read in full at V:
  - `crates/mempool/src/zero_flow_mined.rs` (202 lines).
  - `crates/mempool/src/zero_flow_mined_tests.rs` (46 lines).
  - The `contention_tests.rs` hunk.
  - `bins/node/src/node/validation_checks/gossip_txs.rs` (29 lines).
  - The admission sites: `validation_checks/mod.rs:999-1037` and `rpc/methods/transaction.rs:220-229`.
  - The RPC mapping: `transaction.rs:264-287`, `MempoolError::error_code` (`pool.rs:77-92`), `to_structured_json` (`pool.rs:104-116`).
  - `delegation_view` (`zero_flow_policy.rs:26-60`, to check the flow (f) workaround).
  - The tx-index write in `put_block` (`storage/block_store/writes.rs:48-55`).
- Tests read: the 10 new tests (`inc_i_237_m12_budget.rs` 6, `inc_i_237_m12_budget_probe.rs` 1, `inc_i_237_m12_budget_docs.rs` 3), the moved cap test, and the `inc_i_237_m12_mined_cache.rs` deletion hunk.
- Docs and specs: the 7 docs/spec files of the diff. memory.db: PM-237-14 and PM-237-12.
- Commit message of 548b3d68. Dev notes and the outcome-metric file of the M12.3 runner.
- Cargo jobs run by this reviewer at V (full suites are QA's job in this pass):
  - Probe, `-E 'test(m12_3_rel_574_r1_01_distinct_mined_probe)' --no-capture`: `M12.3-PROBE-SETUP block_bytes=1900126 block_txs=7550` and `M12.3-PROBE handle_ms=81.5 body_reads=2`, PASS.
  - `-E 'test(/m12_3_/) | test(/m12_2_sec_017/)'`: 13 run, 13 passed (`test` profile, optimized). The probe and the 1,000-tx row took 0.369 s. The drain-based rows took 0.048 s. The two rows that sleep 600 ms took 0.651 s.
  - `-p mempool -E 'test(/m12_2_sec_017/)'`: 1 run, 1 passed (`contention_tests::zero_flow_mined_tests::m12_2_sec_017_cache_keeps_the_newest_10_000_per_instance`).

## Summary

**PASS. There is no P0 or P1 at V.**

M12.3 closes REL-574-R1-01 as prescribed:
- The bucket is correct by construction. Refill and spend happen under one mutex, and the lock is never held across I/O. The bucket starts full, saturates at the burst, and reads a monotonic clock.
- A body read now needs a token. A cache hit and an index miss cost no tokens.
- An empty bucket refuses with `MPTX017`. It does not read the block and does not cache anything.
- The event-loop cost of mined-check reads is bounded by burst + 2/s whatever the corpus. The measured stall for 1,000 distinct mined DelegateBonds drops from 23,307 ms to 81 ms (my re-run: 81.5 ms, 2 reads).

REL-574-R1-02..05 are all drained.

The four new findings are P3. Two are documentation and attestation accuracy about the D5 residual. One is a regression-test gap on the burst cap. One is test-module placement. I rule the D5 residual acceptable for this release (section C).

## A. Fix diff, file by file

### `crates/mempool/src/zero_flow_mined.rs` (151 -> 202 lines)

- **How the bucket is shared.** It is one `Mutex<ReadBudget>` field of `MinedCheck` (`:71`). `MinedCheck` is per mempool behind `Arc` (`:175-177`), so there is one bucket per node. All three sites share it: gossip, the fetch path, and RPC.
- **Race on the last token.** None. `take` refills and spends under the same guard (`:47-57`, called at `:94`), so two concurrent admissions cannot both spend the last token or corrupt `at`.
- **Lock order.** In `self.budget().take(Instant::now())` the receiver is evaluated first, so the lock is taken before the clock is read. `at` therefore never moves backwards, and `saturating_duration_since` (`:48`) covers it anyway.
- **Lock scope.** The guard is a temporary of an `if` condition, so it is dropped before `refuse` / `carries` run. No I/O or `.await` runs under the budget lock. The pin mutex is never held while the budget mutex is taken, so there is no lock-order edge.
- **Arithmetic.**
  - `f64` tokens, refill = `elapsed_s * 2`, then `.min(2.0)` (`:49-50`).
  - No overflow: a huge elapsed time saturates at the burst.
  - No NaN source.
  - Fractional refill accumulates across calls, so frequent calls do not starve the refill. Rounding can delay a token by at most one call.
  - `at = now` is updated on every call (`:51`), also when the bucket is full, so idle time is never counted twice.
  - The bucket starts full: `Default` sets `BODY_READ_BURST` tokens (`:36-43`), and `MinedCheck` keeps `#[derive(Default)]`, so `pool.rs` is untouched.
- **The three outcomes, checked against `refuse_if_mined` (`:79-102`):**
  - Cache hit -> `MPTX014`, 0 budget (`:85-86`; `pinned_canonical` makes one height-index get).
  - Index miss -> `Ok`, 0 budget (`:88-90`).
  - Index hit at a height with no canonical hash -> `Ok`, 0 budget (`:91-93`). This is the old `carrier` `?`, moved.
  - Index hit with a token -> one body read (`carries`, `:122-129`). If the block carries the tx: pin + `MPTX014` (`:97-99`). Otherwise `Ok` (`:101`).
  - Index hit without a token -> `MPTX017` with no read and no pin (`:94-96`).
  - The token is taken only after BOTH index lookups hit, as the brief requires.
- **`record`** is `pub(crate)` (`:132`). A grep of the workspace finds no caller outside the crate (production caller `:98`, test caller `zero_flow_mined_tests.rs:27`).
- **Module size.** 202 lines, under 500.

### `bins/node/src/node/validation_checks/gossip_txs.rs` (28 -> 29)

- **Dedupe key.** The per-message dedupe is now `seen.insert(crypto::hash::hash(&tx.serialize()))` (`:15`). That is BLAKE3-256 of the full serialized bytes: 32 bytes per entry instead of a whole copy of each tx.
- **Cost per tx.** One BLAKE3 pass over the serialized bytes, well under 1 µs for a zero-flow tx. The `serialize()` allocation was already there.
- **Collisions.** A collision would need a BLAKE3-256 collision. If one happened, the second tx would be skipped and the verdict would be `Ignore`, which is harmless.
- **What did not change.** The key is still never `tx.hash()`, so input-signature variants are both attempted (`m12_2_sec_018_*` green at V per the runner).
- **MPTX017 on gossip.** The verdict is `Ignore` with no penalty. Every admission `Err` becomes `false` (`validation_checks/mod.rs:1033-1036`), and any `false` gives `Verdict::Ignore` (`gossip_txs.rs:21-27`). The tx path has no `Reject` arm. Rows (a), (d) and (f) assert `Ignore`.

### RPC: `crates/rpc/src/methods/transaction.rs` (not in the diff)

- `MPTX017` goes through the generic arm of `mempool_error_to_rpc` (`:282-285`): `-32002` with `data.error_code` = `MPTX017`, extracted by `error_code()` (`pool.rs:81-87`). This is the same path as `MPTX011..016`.
- `m12_3_sec_020_send_transaction_with_an_empty_bucket_returns_mptx017` asserts `MPTX017`, not `MPTX014`, no residency, and 0 reads.
- `governance.rs:283` builds maintainer changes only, and those never reach the mined check (`zero_flow_mined.rs:191`).
- Observation, no change proposed: a retryable refusal shares `-32002` ("Invalid transaction", `rpc_reference.md:1704`) with permanent refusals. Clients must read `data.error_code`. `rpc_reference.md:361` documents this.

### Performance of the event loop at V

- **Reads per gossip message.** At most `2 + 2 × elapsed_s`, at about 24.6 ms each (measured at f46aa949). One 2,000-tx message pays at most the burst plus about one refill: roughly 50-75 ms of body reads.
- **Sustained cost.** About 50 ms per second plus the burst, whatever the corpus size and the number of peers.
- **Index gets.** These stay at M12.2 cost: at most 2 point gets per DelegateBond (index, height) plus 1 per pin hit.
- **Measured.** The outcome probe gives 81.5 ms for 1,000 distinct mined DelegateBonds (23,307.4 ms before).

### Module-size budgets

- Non-test files touched: `zero_flow_mined.rs` 202 and `gossip_txs.rs` 29.
- `pool.rs` is 1,983 at both f46aa949 and V, so it did not grow. Its pre-existing excess is REL-573-R-06, deferred.
- Test files: `inc_i_237_m12_budget.rs` 213, `_probe.rs` 160, `_docs.rs` 121, `_mined_cache.rs` 223, `zero_flow_mined_tests.rs` 46.
- `contention_tests.rs` was already over the 800 budget and grew from 1,234 to 1,238 (F4).

### Tests

- **Rows (a)-(f) of REQ-GRV-SEC-020:**
  - (a) `m12_3_sec_020_distinct_mined_message_reads_at_most_the_burst` and the probe `m12_3_rel_574_r1_01_distinct_mined_probe`.
  - (b) `..._fresh_delegate_bond_admitted_with_an_empty_bucket`: index miss, Accept plus RPC ok, 0 reads.
  - (c) `..._cache_hit_refused_mptx014_with_an_empty_bucket`.
  - (d) `..._index_hit_with_an_empty_bucket_reads_no_block_then_refills`.
  - (e) `..._send_transaction_with_an_empty_bucket_returns_mptx017`.
  - (f) `..._budget_refusal_leaves_no_cache_entry`: not pinned, then 1 read + `MPTX014` + pinned after the refill.
- **SEC-021.** The cap row moved to `zero_flow_mined_tests.rs:20-46`.
- **SEC-022.** Covered by the existing `m12_2_sec_018_*` rows. The change keeps the same behaviour, so no new row is needed.
- **`inc_i_237_m12_mined_cache.rs` -38 lines.** The hunk removes the `txid` helper, the cap test, its P7 header line, and two imports (`Hash`, `MinedCheck`, `MINED_CACHE_CAP`). The moved test body is byte-identical: the same 5 checks (cap = 10,000, `cached_len`, oldest evicted, second/newest kept, other instance empty). Nothing was weakened.
- **Timing margin.** The drain-based rows assume less than 500 ms between the second draining read and the next attempt. They measured 0.048 s in total, so the margin is about 10×. A flake is unlikely. The burst cap itself is not pinned (F2).
- **Gaps.** No row pins the D5 residual (side-block-indexed tx with an empty bucket -> `MPTX017`). No row pins `:91`.

### Docs and specs (7 files)

- **`docs/error-codes.md`:**
  - `:261`: the `MPTX014` row now states the 1-bond limit.
  - `:264`: new `MPTX017` row with the budget, no read, no cache, `Ignore`, retry.
  - `:272`: flow (a) gives the deterministic-signer reason, the 1-bond limit, the B-uses bound, and AddBond / other-delegate workarounds.
  - `:277`: new flow (f).
- **Flow (f) workaround checked against code.** A queued RevokeDelegation sets `effective = None` in the fold (`zero_flow_policy.rs:43`). So once the revoke is mined, a new DelegateBond is admitted. The workaround is correct.
- **`docs/rpc_reference.md`:** `:356-386`. **`docs/cli.md`:** `:1315-1323`. The `add-bond` subcommand exists (`cli.md:956`, `commands.rs:856`).
- **`specs/gossip-relay-verdict-status.md`:**
  - `:22`: the M12.2 row gives the per-type order (QA-574-03).
  - `:23`: new M12.3 row.
  - `:46`: AUDIT-P1-516 FM3 now says "not finite: bounded since M12.3".
  - The traceability rows for SEC-020..022 and GRV-043 are present.
  - It lacks the D5 residual (F1).
- **`specs/protocol.md` §7.2 and `specs/security_model.md`:** they name the budget, `MPTX017`, and the per-type order. `protocol.md:2096` still says the dedupe is "keyed on the full serialized tx bytes". With a collision-resistant hash of those bytes that statement stays true. No drift.
- **`docs/.workflow/wiring-debt.md`:** reason text only. It names the four remaining pub accessors.

## B. Dispositions of REL-574-R1-01..05 at V

| Id | Ruling | Evidence at V | memory.db status set |
|---|---|---|---|
| REL-574-R1-01 (P1) | FIXED | Budget `zero_flow_mined.rs:23-58,71,94-96`. Tests: `m12_3_sec_020_distinct_mined_message_reads_at_most_the_burst`, `m12_3_rel_574_r1_01_distinct_mined_probe` (my re-run `handle_ms=81.5 body_reads=2`; the runner measured 23,307.4 ms / 1,000 reads at f46aa949), and rows (b)-(f) above. Registered as PM-237-14. The PM-237-12 "finite corpus" text is corrected. The residual is ruled in section C. | `fixed`, `fixed_in_run=574` |
| REL-574-R1-02 (P2) | FIXED (docs) | `error-codes.md:261,272`, `rpc_reference.md:372-377`, `cli.md:1315-1320` state the deterministic signer, the 1-bond limit, and the AddBond / other-delegate workarounds. The test `m12_3_grv_043_docs_state_the_one_bond_limit` pins the text. Rule D3 / Q2 is still not user-confirmed, as stated in round 1. | `fixed`, `fixed_in_run=574` |
| REL-574-R1-03 (P3) | FIXED (docs, as suggested) | Flow (f) at `error-codes.md:277` and `rpc_reference.md:386`. The status spec AUDIT-P1-515 row (`:45`) now says "Not looser only". The test `m12_3_grv_043_error_codes_list_flow_f` pins it. I verified the workaround against `zero_flow_policy.rs:43`. The behavioural guards row suggested in round 1 was not added; that is a test gap with no defect. | `fixed`, `fixed_in_run=574` |
| REL-574-R1-04 (P3) | FIXED | `record` is `pub(crate)` (`zero_flow_mined.rs:132`). No caller outside the crate. The cap row is in-crate (`zero_flow_mined_tests.rs:20-46`, run green). The wiring-debt reason text is updated. | `fixed`, `fixed_in_run=574` |
| REL-574-R1-05 (P3) | FIXED (correction note) | The 548b3d68 message carries "pool.rs:896 Full branch: pre-existing code moved in M12.2; the f46aa949 Path-Coverage row cited the wrong test (REL-574-R1-05)". This is the round-1 option "mark the row moved, pre-existing". The f46aa949 text itself is unchanged in history. | `fixed`, `fixed_in_run=574` |

Round-1 Speculative S1 (an RPC-side TOCTOU between `refuse_if_mined` and the write lock) is unchanged by M12.3. The budget sits inside the same window.

## C. Ruling on decision D5's residual (global per-node budget)

**Ruling: acceptable for this release, as a P3 follow-up (F1). It is not a blocker.**

- **Which honest txs can be affected.** Only DelegateBonds whose txid is in the node's tx index at a height that has a canonical hash. Every new delegation is an index miss and spends nothing (row (b), `zero_flow_mined.rs:88-90`). Two kinds remain:
  1. **A replay of a mined tx.** It is refused either way: `MPTX014` after a read, or `MPTX017` without one. The honest user loses nothing.
  2. **A side-block-indexed tx.** `put_block` indexes the txs of every stored block, side blocks included (`storage/block_store/writes.rs:48-55`). When a block carrying an honest DelegateBond loses a fork, the reorg path only revalidates the mempool (`block_handling.rs:1071-1079`); it does not re-add the block's txs. The tx is usually still resident on nodes that never applied the losing block, and gets mined there. If the user must resubmit, the resubmission is an index hit on every node that stored the side block. This is the only real loss.
- **What the user loses, and for how long.** At a flooded node that stored the side block:
  - RPC returns `MPTX017` ("retry").
  - Gossip gets `Ignore` at that hop only. The same message bytes are not re-processed there for 300 s (duplicate cache, `protocol.md:2085`).
  - This lasts as long as the attacker keeps the bucket empty. A retry competes for 2 tokens/s against a sender limited to 500 tx/s per peer, so in practice the retry fails until the flood stops.
  - Nothing persists. `MPTX017` writes no pin, so admission works as soon as the flood stops.
  - Workarounds: submit through any node that did not store the side block (an index miss there, 0 budget), or wait.
  - No funds, no consensus state, and no new delegation are affected.
- **What the attack costs.** Every read pins its txid (`:97-99`), so a corpus is used up at 2 txids/s. Sustained starvation needs more than 10,000 distinct mined DelegateBonds that each pass every cheaper check. Only then does the FIFO cap (`MINED_CACHE_CAP`, `:21`) recycle them. The attacker also needs a registered, active, undelegated delegator key with bonds (real stake). The corpus has to be pre-mined:
  - at the baseline, where any zero-fee DelegateBond is admitted, bounded by block space (about 7,550 txs per builder-budget block in the probe);
  - or, once M12 is deployed, by a malicious scheduled producer, because M12.1 keeps one resident delegation tx per delegator.

  On top of that, about 2 tx/s per victim node.
- **Why not a blocker.** The harm is a liveness delay for a rare honest class (a reorged-out DelegateBond that needs a resubmission), at the flooded nodes only, with a retry signal and a route around it. The alternative is a per-peer or per-key budget. The round-1 resource-cost note recorded why the SSF rejected those: they need per-key state and scale with the number of keys an attacker funds. The global budget is what bounds event-loop time absolutely, and that was the P1.
- **What is owed (F1, F3).** Record the residual in the shipped status spec. Correct the commit's FM1/FM2, which state a bound that does not hold under the flood. PM-237-14 `scale_assumptions` in memory.db already names it ("one flooding peer can make other peers' index-hit DelegateBonds wait").

## D. Commit message of 548b3d68

- **Consensus checklist (INV-12).**
  - (1) YES (gossip, fetch, both RPC sites).
  - (2) NO.
  - (3) YES for consensus, because only the cost and the refusal of an admission change.
  - Conclusion: no activation height, rolling deploy. I agree with the conclusion. Over 8cc50d33..V the four consensus dirs are untouched: `git diff --stat 8cc50d33..HEAD -- crates/core/src/validation bins/node/src/node/apply_block crates/storage crates/core/src/network_params` prints nothing.
  - (2) could fairly be YES: block content written by a producer feeds the tx index that this path reads. That changes nothing, because (3) is YES. The Path-Coverage q3 = NO answers are per-branch admission behaviour, not consensus, and the message explains this.
- **Failure-Modes.** FM3-FM7 are accurate. FM1 and FM2 overstate the bound (F3).
- **Path-Coverage.**
  - The rows are at `:52`, `:85`, `:88`, `:94`, `:97`, `:106`, `:110`, `:114`. All line numbers match V.
  - The two new guards (`:52` `tokens < 1.0`, `:94` `MPTX017`) cite tests that reach them (`..._reads_no_block_then_refills`, `..._returns_mptx017`).
  - The let-else early return at `:91` has no row. The dev notes say no test reaches it (F3).
- **Outcome-Metric.** The metric is external (event-loop wall time plus body reads). The probe can be re-run. The before value (23,307.4 ms / 1,000) and the after value (81.3 ms / 2) are both measured and differ. My re-run gives 81.5 ms / 2.
- **REL-574-R1-05 note.** Present (section B).
- **Reword before push.** Recommended, not required for the release verdict. If the history is reworded anyway, fix FM1/FM2 and add the `:91` row (F3).

## E. Scope (REQ-GRV-042)

- **M12.3 files.** All 16 are admission code, its tests, docs/specs, or the wiring ledger. The four consensus dirs are empty over 8cc50d33..V. M12.3 adds no activation height, version bump, storage or index change.
- **Whole branch d0e15686..V.** The only hit in those dirs is `bins/node/src/node/apply_block/post_commit.rs`, +3 lines:
  - `crate::metrics_gossip::observe_attestation_coverage(idx.len(), decode_len);`, a metric.
  - `self.attest_seen.prune(block.header.slot);`, which prunes the node-local relay-newness cache.
  - Neither writes ChainState, UtxoSet, ProducerSet, or block content. This matches the RUN 573 F5 ruling: non-consensus.

## Findings

### [F1] P3 — REL-574-R2-01 — D5 cross-peer starvation residual: accepted, not recorded in the shipped spec

- **Location:** `crates/mempool/src/zero_flow_mined.rs:94-96` (one global bucket, `:71`); `specs/gossip-relay-verdict-status.md:23` (M12.3 row) and `:46` (AUDIT-P1-516 residuals).
- **Evidence:**
  - `MPTX017` is returned for any index hit when the per-node bucket is empty, whoever sent the tx (`:94-96`).
  - `put_block` indexes side-block txs (`storage/block_store/writes.rs:48-55`).
  - The M12.3 row and the AUDIT-P1-516 row describe the budget, but neither names the cross-peer residual. A grep for `starv|another peer|cross-peer|flood` in the status spec finds nothing on M12.3.
  - The residual is recorded only in PM-237-14 (memory.db) and the runner's dev notes, which are not shipped.
- **Confidence:** conf(0.85, observed) for the mechanism and the missing spec text. The honest-impact analysis in section C is inferred.
- **Disposition:** P3 follow-up, non-blocking. Accepted for release under D5. D5 is an orchestrator decision, not user-confirmed.
- **Suggested fix:** add one residual sentence to the M12.3 row or to the AUDIT-P1-516 residuals in `specs/gossip-relay-verdict-status.md`: "global per node: a flood of index-hit DelegateBonds delays another peer's side-block-indexed DelegateBond (MPTX017 / Ignore at that node) for as long as the flood lasts; replays lose nothing; new delegations never spend". If the user later wants it closed, the follow-up design is a per-peer fair share of the budget.
- **Test strategy:** extend `m12_3_grv_043_*` to require the residual sentence in the status spec.

```
━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      0 (docs only) (observed)
  Memory:   0 (docs only) (observed)
  IO:       0 (docs only) (observed)
  Network:  0 (docs only) (observed)
  Disk:     +one sentence of specification text (observed)
  Latency:  0 (docs only) (observed)
Inevitability: AVOIDABLE
Cheaper alternative: leave the residual only in the memory.db registry row; later reviewers of the shipped spec then miss it
Why this proposal anyway: the spec is the projection later reviews and operators read; one sentence keeps the accepted residual visible at zero runtime cost
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### [F2] P3 — REL-574-R2-02 — the burst cap of the read budget is not pinned by any test

- **Location:** `crates/mempool/src/zero_flow_mined.rs:47-57` (`.min(f64::from(BODY_READ_BURST))` at `:50`).
- **Evidence:**
  - The bucket starts full at node construction (`:36-43`).
  - Every M12.3 row spends it soon after construction. My run at V: the drain-based rows took 0.048 s in total, and the probe and 1,000-tx rows 0.369 s in total.
  - Without the cap, the extra refill before the first spend is under 1 token (less than 0.5 s × 2/s). Every assertion would still see 0, 1, or at most 2 reads.
  - Only the 600 ms rows idle after a spend, and each makes one attempt.
  - No unit test calls `ReadBudget::take` with synthetic `Instant`s, although `take(now)` takes the clock as an argument.
  - The cap is the property that bounds reads after an idle period. Without it, a node idle for an hour would grant about 7,200 reads, roughly 177 s of loop time at 24.6 ms/read: the R1-01 stall on a quiet node.
- **Confidence:** conf(0.85, inferred). The timings and the code are observed. The mutation was not run, because this reviewer may not modify tracked files.
- **Disposition:** P3 follow-up, non-blocking. The code at V is correct.
- **Suggested fix:** add an in-crate unit test in `zero_flow_mined_tests.rs` that drives `ReadBudget::take` with `t0` and `t0 + Duration`. The test must check five things:
  1. 2 spends succeed and the 3rd fails.
  2. At +500 ms, 1 more spend succeeds.
  3. At +1 h, exactly 2 spends succeed and the 3rd fails (the cap).
  4. A `now` earlier than `at` neither panics nor adds tokens.
  5. Fractional refills accumulate.
- **Test strategy:** the unit test above. It is deterministic and needs no wall-clock sleep. Removing `.min(..)` makes step 3 fail.

```
━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      0 at runtime (test only); +microseconds of test time (observed)
  Memory:   0 at runtime (test only) (observed)
  IO:       0 (test only) (observed)
  Network:  0 (test only) (observed)
  Disk:     +about 40 lines of test code (inferred)
  Latency:  0 at runtime (test only) (observed)
Inevitability: AVOIDABLE
Cheaper alternative: an it row that idles more than 1 s after draining and then sends 3 index hits; slower and wall-clock dependent
Why this proposal anyway: a synthetic-instant unit test pins the cap deterministically, and the cap is what keeps an idle node from granting a large read burst
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### [F3] P3 — REL-574-R2-03 — commit 548b3d68: FM1/FM2 overstate the bound, and one early return has no Path-Coverage row

- **Location:** `git:548b3d68` message, Failure-Modes FM1 and FM2, and the Path-Coverage block. The code is at `crates/mempool/src/zero_flow_mined.rs:91-93`.
- **Evidence:**
  - FM1 says: "honest replayed or side-block-indexed DelegateBond: waits at most one refill interval (500 ms per token) and is retried by gossip / RPC".
    - Under a flood, each refilled token goes to whichever index hit arrives first (`:94`). An honest tx waits as long as the flood lasts (section C).
    - Gossip does not retry an `Ignore`d message at that hop. The same message id is dropped for 300 s (`specs/protocol.md:2085`).
  - FM2 ("the burst drains at 2/s") assumes the txs are resubmitted. The reorg path does not re-add them (`block_handling.rs:1071-1079` revalidates only).
  - Path-Coverage lists `:52,85,88,94,97,106,110,114`. The let-else `return Ok(())` at `:91-93` (index height without a canonical hash) is an early return in the added lines and has no row. The dev notes say no test reaches it.
- **Confidence:** conf(0.85, observed).
- **Disposition:** P3, non-blocking. A reword is recommended before push and is not required for this verdict.
- **Suggested fix (on reword):**
  - FM1 -> "waits for a token; under a flood of index-hit DelegateBonds from any peer, until the flood stops (D5 residual); gossip Ignore at that hop, RPC MPTX017 retry".
  - FM2 -> "resubmitted txs each wait for a token".
  - Add the row `line: 91 q1: YES q2: NO q3: YES test: none (moved defensive path, same outcome as f46aa949)`, or cite a test that rolls back a tip whose txs stay indexed.
- **Test strategy:** NOT_TESTABLE (commit metadata). Verify by reading the reworded message against `zero_flow_mined.rs:88-96`.

```
━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      0 (commit metadata only) (observed)
  Memory:   0 (commit metadata only) (observed)
  IO:       0 (commit metadata only) (observed)
  Network:  0 (commit metadata only) (observed)
  Disk:     0 (commit metadata only) (observed)
  Latency:  0 (commit metadata only) (observed)
Inevitability: AVOIDABLE
Cheaper alternative: leave the message; the status spec sentence of F1 then carries the correct bound alone
Why this proposal anyway: Failure-Modes blocks are what later gates and reviewers rely on; a false bound on the accepted residual misleads the next review of this mechanism
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### [F4] P3 — REL-574-R2-04 — the in-crate test module hangs off an unrelated, over-budget test file

- **Location:** `crates/mempool/src/contention_tests.rs:34-36` (`#[cfg(test)] #[path = "zero_flow_mined_tests.rs"] mod zero_flow_mined_tests;`).
- **Evidence:**
  - `wc -l` of `contention_tests.rs`: 1,234 at f46aa949, 1,238 at V. The test-file budget is 800.
  - nextest reports the test as `mempool contention_tests::zero_flow_mined_tests::m12_2_sec_017_cache_keeps_the_newest_10_000_per_instance`.
  - `lib.rs` is 26 lines and `zero_flow_mined.rs` is 202. Either could declare the module.
- **Confidence:** conf(0.90, observed).
- **Disposition:** P3, non-blocking (maintainability: someone looking for the mined-check tests will not look in contention tests).
- **Suggested fix:** move the three lines to the end of `zero_flow_mined.rs` as `#[cfg(test)] #[path = "zero_flow_mined_tests.rs"] mod tests;`. Change the import in the test file to `use super::{MinedCheck, MINED_CACHE_CAP};`, or keep the `crate::` path.
- **Test strategy:** the moved cap row still runs and passes under `-p mempool`. `contention_tests.rs` is back to 1,234 lines.

```
━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      0 (test-module placement only) (observed)
  Memory:   0 (test-module placement only) (observed)
  IO:       0 (test-module placement only) (observed)
  Network:  0 (test-module placement only) (observed)
  Disk:     0 net (three lines move between files) (observed)
  Latency:  0 (test-module placement only) (observed)
Inevitability: AVOIDABLE
Cheaper alternative: leave it; the test runs either way
Why this proposal anyway: the tests sit next to the code they cover and stop growing a file that is already over budget, at zero runtime cost
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Speculative Findings (low-confidence, not actionable)

- **S1, conf(0.60, inferred).** Honest traffic also uses up the budget. Late gossip or fetch copies of DelegateBonds that a just-applied block carries are index hits. The first 2 per refill window spend tokens and get pinned. The rest get `MPTX017`, cost no read, and are not pinned. Right after a block with many DelegateBonds, a resubmitted side-block-indexed DelegateBond may therefore get `MPTX017` without any attacker. The effect is bounded and short: the copies stop once peers apply the block. No test or measurement covers it.

## Specs/Docs Drift

- `specs/gossip-relay-verdict-status.md`: the D5 residual is missing (F1).
- No other drift found in the 7 files. The docs test `m12_3_grv_043_*` (3/3) passed at V in my run.

## Security Audit Verdict

```
━━━ SECURITY AUDIT VERDICT ━━━
Verdict: AUDIT-REQUIRED
Signals: trust boundary (gossip and RPC tx admission of peer- and user-supplied DelegateBonds); DoS protection mechanism on the event loop (read budget PM-237-14); external data (tx index and block-body reads keyed by attacker-chosen txids)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Final Verdict

Approved for release. There is no reproducible P0 or P1 at V. REL-574-R1-01..05 are drained. The D5 residual is accepted as a P3 follow-up. F1-F4 are non-blocking. The F1/F3 text fixes can ride on a pre-push reword if one happens.

Release-Validated: 548b3d68240d83ae1564475b4e3b9975065c266e
Release-Verdict: PASS
