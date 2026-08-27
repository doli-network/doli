━━━ FINDINGS — 7 total (Major:1 Minor:6) ━━━

  [F1] MAJOR conf(0.90, observed) — bins/node/src/node/validation_checks.rs:664-896 + bins/node/src/node/production/withdrawal_holdings.rs:93-209 + crates/mempool/src/withdrawal_holdings.rs:68-135 — the banked residual FIND-I180-M2-TRANSCRIPTION-001 covers R1 only; R0/R2/R3 and `bond_input_split` are THREE verbatim transcriptions with no term-exact parity lock
  [F2] MINOR conf(0.95, measured) — bins/node/src/node/apply_block/mod.rs:373 vs bins/node/src/node/block_handling.rs:1080 — the "≤1 block stale" holdings-snapshot bound is false: 1 refresh site against 7 `producer_set.write()` sites; `execute_reorg`'s `revalidate` has no preceding refresh
  [F3] MINOR conf(0.95, measured) — crates/mempool/src/pool.rs:566-572,1293-1305 + bins/node/src/node/production/assembly.rs:324-334 — three new protection mechanisms are unregistered in `protection_mechanisms`; PM-021/PM-022 are the identical precedent shape and ARE registered
  [F4] MINOR conf(1.00, measured) — .omega/gauntlet.conf:24 (`domain: bins/node/src/node/`) — no `gauntlet_runs` row exists for run 525; last pass is `3f8bf185` (INC-I-174), and two M2 changes are live BELOW AH #23 on every network today
  [F5] MINOR conf(0.95, observed) — crates/mempool/src/holdings.rs:92-94 — the `is_empty()` early return is absent from the `Path-Coverage:` block and is driven by no test in the tree (QA's PROBE-EMPTY was deleted)
  [F6] MINOR conf(0.95, observed) — docs/.workflow/inc-i-180-M2-implementation.md:19-32 — the per-change AH classification table omits `bins/node/src/node/apply_block/mod.rs` (+11 −9), one of only two changes that are live below the AH today
  [F7] MINOR conf(0.90, observed) — docs/.workflow/inc-i-180-M2-implementation.md:238 + specs/protocol.md + docs/architecture.md — four surviving prose imprecisions in shipped files, carried unfixed from QA round 3 (OBS-010/011/012/013)

  Speculative: 1 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Code Review: INC-I-180 M2 — mempool + block-builder parity for the withdrawal-holdings gate

**Run 525 · incident INC-I-180 · branch `bugfix/inc-i-180-withdrawal-holdings-gate` · base `e6c066c7` · UNCOMMITTED.**
Reviewer pass over the full milestone after three QA rounds (current QA verdict: PASS).

## Scope Reviewed

Every M2 source file, re-derived against the code rather than against the reports:

| File | What I re-derived |
|---|---|
| `bins/node/src/node/validation_checks.rs:610-901` | the whole M1 rule table + both S3 Replay guards, term by term |
| `bins/node/src/node/production/withdrawal_holdings.rs` (209 L) | R0/R1/R4/R3/R2 against the gate, term by term and operation by operation |
| `bins/node/src/node/production/assembly.rs:176-339` | guard discipline, skip placement, in-block accounting, block ordering |
| `crates/mempool/src/holdings.rs` (114 L), `withdrawal_holdings.rs` (135 L) | the decidable subset, the substitution premise, the fail-open ladder |
| `crates/mempool/src/pool.rs` (+94) | both admission entry points, `revalidate`, height gating |
| `bins/node/src/node/holdings.rs` (41 L), `mod.rs` (+9), `init.rs` (+29 −1) | snapshot publication and all three constructors |
| `bins/node/src/node/rewards.rs:1356-1429` vs `apply_block/tx_processing.rs:380-449` | the S5 mirror against the live branch |
| `bins/node/src/node/apply_block/mod.rs` (+11 −9) | the OBS-002 hygiene reordering, and every `?` between the old and new position |
| `specs/protocol.md` (+97 −6), `docs/architecture.md` (+48 −4) | every load-bearing sentence, re-derived by `grep`/`sed`/read |
| `crates/core/src/validation/utxo.rs`, `transaction/types.rs` | reachability of the R4 asymmetry and of `TxType::Exit` |
| `.omega/memory.db` | `v_protection_surface`, `gauntlet_runs`, open findings, active invariants |

Not re-derived (accepted from the measured gate state in the brief): build/clippy/fmt, the 3728/3/43 workspace figures, the byte-identical failing set.

## Summary

**⚠️ Approved with observations — conditional on discharging F3, F4, F5 before commit/close.**

The milestone does what it was written to do. I attacked the primary question — *post-AH, is there still any mempool-admissible `RequestWithdrawal` that a builder will select and `validate_block_economics` will reject?* — and could not construct one. That is the whole defect class (Reviewer F1, INV-VALIDATION-001, INV-PROD-003), and it is closed. No CRITICAL finding. No injection pattern. No `unwrap()`/`panic!`/`unsafe`/TODO in any of the four new modules. No version bump, no activation height added, moved or reused, no out-of-scope file touched.

What I found is one Major scoping defect in the *residual* the developer banked, and six Minor items — three of which are OMEGA protocol gates that are unmet as of this moment.

---

## 1. Root-cause completeness — CLOSED, verified by enumeration

I enumerated every term of every rule in all three transcriptions and looked for a reachable divergence.

| Rule | Gate | Builder | Divergence? |
|---|---|---|---|
| R0 registered | `producers.get_by_pubkey(pk)` (`validation_checks.rs:749`) | `self.holdings.get(&pk)`, populated by `mempool::holdings::of_producer_set` which is `set.get_by_pubkey(pk)` (`holdings.rs:106`) | none |
| R1 allowance | inline 5-term chain (`:771-776`) | `ProducerHoldings::allowance_with` (`holdings.rs:36-42`) | none — identical term order AND operation order; locked two-sided by `inc_i180_m2_the_gate_allowance_equals_the_shared_function*` |
| R4 same-block input | `earlier_tx_hashes`, ALL lower-index txs incl. coinbase/epoch-coinbase/genesis-reg (`:696-699`) | `earlier_hashes`, mempool txs only (`withdrawal_holdings.rs:138`) | **structural asymmetry, unreachable** — see below |
| R3 exclusivity | `bond_inputs_by_tx` over the pre-block view (`:664-680`) | `bond_input_split` over the same guard-held view (`:194-208`) | none — byte-identical function bodies |
| R2 split | `:868-895` | `:113-128` | none |
| in-block `AddBond` | `count() as u32` (`:709`) | `u32::try_from(..).unwrap_or(u32::MAX)` (`:142-148`) | different overflow semantics, unreachable at any block size (QA OBS-008) |
| in-block `Exit` | `producers.get_by_pubkey(..).bond_count` (`:733-736`) | `self.holdings.get(..).bond_count` (`:159-163`) | none — `load()` resolves Exit-named producers (`:55`) |

**The R4 asymmetry is genuinely unreachable, and I proved it independently of QA.** `crates/core/src/validation/utxo.rs:125-130` resolves *every* input through `utxo_provider.get_utxo(..).ok_or(ValidationError::OutputNotFound)?` — there is no carve-out for `RequestWithdrawal`. The builder runs that function FIRST (`assembly.rs:259`), against the pre-block view, so a transaction naming an outpoint created by any earlier transaction of the same block — protocol-generated or not — is skipped before `wd_parity.allow()` at `:324` is ever reached.

**Ordering is preserved end to end**, which the in-block accounting depends on: `BlockBuilder::add_transaction` (`crates/core/src/block.rs:338`) appends without sorting, `add_coinbase_with_extra` (`:361`) inserts at index 0, and `wd_parity.accept(tx)` (`assembly.rs:335`) is the last statement before `included_txs.push` / `builder.add_transaction` with no `continue` between them. Selection order IS block order, so `in_block_addbond` / `in_block_withdrawn` / `earlier_hashes` are accumulated over exactly the set the gate will see at exactly the indices the gate will see them.

**No admission bypass exists.** Both mempool entry points route on `tx.is_zero_flow()` (`crates/rpc/src/methods/transaction.rs:210`, `bins/node/src/node/validation_checks.rs:1253`), and `TxType::RequestWithdrawal => false` in `allows_empty_io` (`crates/core/src/transaction/types.rs:198`), so every withdrawal lands in `add_transaction` where the new check sits. `add_system_transaction` is unreachable for this type.

**No build/apply interleaving window.** `try_produce_block()` is awaited inline in the single `run_event_loop` `tokio::select!` task (`event_loop.rs:88,123`), and the self-produced block is applied within that same call (the `[BLOCK_POISON]` handler at `production/mod.rs:624` proves it). So the producer-set and UTXO views the builder read are the views the gate reads.

## 2. Unintended behaviour changes — pre-activation byte-identity holds, with two live exceptions

- **Gate:** the entire section is inside `if height >= withdrawal_gate_ah` (`validation_checks.rs:621`) and `git diff --numstat e6c066c7` is `27 0` — zero deletions. Both S3 guards are `mode == ValidationMode::Replay`, so Full/Light are untouched by construction.
- **Builder:** `WithdrawalParity::new(active, height)` short-circuits `load()` (`:47-49`) and `allow()` (`:70-72`) when inactive. `allowance_with` is never reached below the AH.
- **Mempool:** `withdrawal_holdings_verdict` returns `Ok` before any lookup when `current_height < AH` (`pool.rs:325-331`).

Two changes ARE live below AH #23 on every network today:

1. **S5 `rewards.rs` auto-revoke mirror (+22 −0).** Deliberately ungated. I verified the mirror against live term by term: rebuild's `available` (`rewards.rs:1391-1394`) IS live's `remaining` (`tx_processing.rs:401-403`), and `available.saturating_sub(delegated)` IS live's `available` (`:404`), so the flattened condition `delegated > 0 && bond_count == available && bond_count > available.saturating_sub(delegated)` is exactly live's nested `if bond_count > available { if delegated > 0 && bond_count == remaining }`. The `RevokeDelegation` is queued BEFORE the withdrawal on both sides, which matters because `apply_pending_updates_with_cap` is order-dependent. **I agree with QA's assessment and with the developer's disproof attempt.** The pre-fix rebuild value was already divergent from live on a `serialize_canonical()` field, and rebuild runs only on reorg/recovery, so the fleet never converged on it. The change strictly reduces the divergent-outcome set; a rolling restart is sufficient. It must still be stated at the deploy gate, and §9's third deploy answer does state it.

2. **The OBS-002 hygiene reordering in `apply_block/mod.rs` (+11 −9).** I checked the range between the old (`:265`) and new (`:375`) positions myself: every exit is a `?` propagation (`maybe_complete_genesis(..).await?`, `put_block(..)?`, `set_canonical_chain(..)?`, `batch.commit()`) — there is no unconditional early `return Ok`. So the only behavioural delta is the declared one, and it is the safe direction. See F6 for the classification gap.

## 3. AH classification — correct as argued, incomplete as tabulated

The reasoning is right. The builder and the mempool emit no consensus verdict: a skip yields a strict subset block that every node still accepts, and a refused admission keeps a transaction out of one node's pool while any other node may relay and mine it. Both are nevertheless height-aware and strict no-ops below the AH, which is the correct discipline — skipping below the gate would be censorship. S3 is the one AH-visible change and it rides the existing AH #23, which is right because the whole gate does.

Verified structurally: `git diff --stat e6c066c7 -- crates/core/network_params/ crates/core/src/consensus/ bins/cli/ crates/updater/` is **empty**; no `Cargo.toml` version change; no `CURRENT_PROTOCOL_VERSION` / `EPOCH_STATE_FORMAT_VERSION` / `MIN_PEER_PROTOCOL_VERSION` token in the diff. **No height added, moved or reused. No version bump.** Confirmed.

The gap is F6: the classification TABLE the commit body draws from omits one file.

## 4. The `HoldingsSources` design deviation — fail-open is safe; the staleness bound is not what is claimed

- **Deadlock / blocking / livelock: none, and I agree with QA here for the right reason.** `HoldingsSources::lookup` uses `tokio::sync::RwLock::try_read()` (`holdings.rs:77`), which is a non-async, non-queueing call. `Mempool::add_transaction` is a synchronous `&mut self` method reached under `mempool.write()` + `utxo_set.read()` (`validation_checks.rs:1255-1264`); adding a non-blocking producer-set probe under those two guards cannot park, cannot queue behind tokio's write-preferring writer, and therefore cannot join `apply_block`'s `utxo→producers` (`apply_block/mod.rs:196-198`) to `rollback`'s `producers→utxo` (`rollback.rs:324-326`). I re-read all four line ranges.
- **Builder lock discipline is correct.** `wd_parity.load()` runs under `producer_set.read()` alone at `assembly.rs:192-195` and the guard is dropped before `let utxo = self.utxo_set.read().await` at `:199`. Only one guard is ever held. The stated conclusion is true.
- **Sustained write contention degrades to the snapshot, then to `Unavailable` → admit.** Both failure directions are non-safety: over-rejection is a bounded liveness cost, under-rejection is pre-M2 behaviour and is still caught by the builder and by consensus. **Fail-open is the right direction and it is now true in all three constructors** (`is_empty()` ⇒ `Unavailable`, `holdings.rs:92-94`).
- **The one-block staleness bound is NOT enforced.** This is F2 — QA validated it as "enforced" and that is the one substantive thing QA got wrong across three rounds.

## 5. The residual routed forward (FIND-I180-M2-TRANSCRIPTION-001) — right call, wrong scope

**The layering argument is correct.** Routing `validate_block_economics` through `crates/mempool` would make a consensus rule depend on the mempool crate. `bins/node` already links `mempool` (`production/withdrawal_holdings.rs:16`), so this is an architectural objection rather than a compile constraint — and it is the right objection. Relocating `ProducerHoldings` to `crates/storage`, which `bins/node` and `crates/mempool` both already depend on, is the correct structural close.

**The R1 lock is genuinely strong.** It asserts the allowance the gate REPORTS plus the terms the same message echoes, so it is two-sided: an up-drift stops the probe rejecting at all (the test panics on `Ok`), a down-drift prints a different N, and a right-number-from-wrong-terms gate fails the echoed tail. QA reproduced both mutations and restored `validation_checks.rs` to `sha256 b411b0bb…` / `numstat 27 0`. I accept that as sufficient for R1 until the relocation lands.

**But the residual is scoped to R1 only, and that is F1.** The R1 arithmetic was one of *several* duplicated expressions; ISSUE-001 was a drift in a duplicated expression, and the remaining duplicates carry no equivalent lock.

## 6. S4 routing quality — SOUND, and I agree with the destination

I tried to break each rejection rather than accept it:

- **(a) add `pending_updates` to `serialize_canonical()`** — blast radius stated accurately. `set_persistence.rs:78-113` is the producer-set input to `psHash`; adding a field moves the state root of every block on every network at the height, needs its own AH (never AH #23 — bundling is the INC-I-054 error), a synchronized deploy, and a NEW canonical ordering rule for an order-sensitive queue. Correctly sized as its own incident.
- **(b) re-derive from the block range** — correctly rejected for *availability*, not cost. A snap-synced node's store starts above the last epoch boundary and would refetch from the same untrusted peer. INV-EPOCH-002 verbatim.
- **(c) restructure the read** — I probed the variant the doc dismisses in one line and confirm it is not there: no field inside `serialize_canonical()` carries in-flight AddBonds, so "keep the term, source it from the state root" collapses back into (a). And dropping the term breaks M1's green `req_i180_002_post_ah_pending_addbond_makes_the_434th_withdrawable`. Genuinely unavailable.
- **A fourth option not considered, and it also fails:** covering `pending_updates` in the snapshot *transport* checksum instead of the state root. It cannot work — the hostile peer produces both the object and the checksum; only a chain-derived root cross-checks.

**Residual-exposure statement: honest.** The corrected framing ("one field lacks the integrity check its siblings have") is the accurate one, the "NOT a bound" clause about self-healing is exactly right, and the conclusion that this is a prerequisite for pinning a real mainnet value for AH #23 is correct and must be carried into the AUDIT-P2-001/002 height-pinning session. One residual: the routing doc describes the owning incident in prose but no `INC-*` ticket has been opened for it, so the constraint on the height-pinning session has no ID to hang from. Open it before close.

## 7. Specs / docs accuracy — re-derived, not read-and-agreed

Every load-bearing sentence, checked against source:

| Claim | Verdict |
|---|---|
| "The gate does NOT call it: it holds the reference expression inline" | **TRUE** — `grep -rn "allowance_with\|\.allowance()" bins crates \| grep -v /tests/` returns 4 lines, none in `validation_checks.rs`; `sed -n '771,776p'` is the 5-term chain |
| "the gate is evaluated BEFORE the EpochReward section, so no early return can make it mode-dependent" | **TRUE** — withdrawal section `:621-901`, EpochReward section begins `:903` |
| "`OVER_HOLDINGS` and `SAME_BLOCK_INPUT` stay strict in all three modes" | **TRUE** — R1 at `:777`, R4 at `:807-821`, both above the Replay guard at `:828` |
| "the skipped transaction still charges the allowance" | **TRUE** — `:834-835`, inside the guard, before `continue` |
| the R0 Replay carve-out does NOT charge | **TRUE and correct** — live apply's `if let Some(info) = producers.get_by_pubkey(..)` (`tx_processing.rs:379`) queues nothing for an unregistered producer either |
| "`in_block_withdrawn` is replaced by `in_mempool_withdrawn`" | **TRUE** — `pool.rs:573` passes `count_residents = true`, `:1301` passes `false`; `withdrawal_holdings.rs:51` is `allowance_with(0, in_mempool_withdrawn)` |
| "admission OVER-rejects whenever the substitute raises the allowance or exceeds the debit" | **substantively TRUE, quantifier over-broad** — F7/OBS-011b |
| "`revalidate` evicts a held withdrawal the ledger moved out from under" | **TRUE** — `pool.rs:1293-1305` |
| "falling back to a **per-block** snapshot" / "≤1 block stale" | **FALSE** on the rollback/reorg/fork-recovery paths — **F2** |
| "drives the gate over **eight** allowance shapes" attributed to one function | **imprecise** — `grep -c 'name: "IP-'` is 8 but the file carries two test functions; F7/OBS-010 |
| S5 mirror description (auto-revoke queued before the withdrawal, ungated, inherits both `in_flight` forms) | **TRUE** — verified term by term against `tx_processing.rs:401-437` |

The round-1 and round-2 false claims are all gone. I found no NEW false claim beyond F2 and the F7 set.

## 8. Scope discipline — clean

Nothing in brief §3 was touched. `bins/cli/` (AUDIT-P2-003), `crates/core/src/network_params/` (AUDIT-P2-001/002), `crates/updater/` and `crates/core/src/consensus/` are all absent from `git diff --stat e6c066c7`. `FIND-I081-EPOCH-SKIP-001` remains open and untouched at `validation_checks.rs:838`'s sibling site. AUDIT-P0-001 (registration-declared `bond_count`) and AUDIT-P1-003/P1-005 are not addressed, correctly.

The milestone did not go too far. It went slightly further than the brief in one place — the OBS-002 apply_block reordering, which is a QA-requested fix rather than brief scope — and that extension is justified and correctly bounded.

## 9. Module size

| File | Lines | Budget | Status |
|---|---|---|---|
| `crates/mempool/src/pool.rs` | **1818** | 500 | 3.64x over; grew +94 this milestone; **not banked anywhere** |
| `bins/node/src/node/validation_checks.rs` | **1614** | 500 | 3.23x over; grew +27; banked as `DEV-I173-M3-002` |
| `bins/node/src/node/rewards.rs` | **1561** | 500 | 3.12x over; grew +22; not banked |
| `bins/node/src/node/production/assembly.rs` | 669 | 500 | 1.34x over; grew +30 |
| `bins/node/src/node/apply_block/mod.rs` | **500** | 500 | exactly at budget after +11 −9 — the next line over it is a violation |
| new: `production/withdrawal_holdings.rs` / `mempool/withdrawal_holdings.rs` / `mempool/holdings.rs` / `node/holdings.rs` | 209 / 135 / 114 / 41 | 500 | all well under — **correct discipline: all new logic went into new focused modules** |
| test files, largest | `inc_i_180_builder_parity.rs` 755, `inc_i_180_gate_bindings.rs` 734, `inc_i_180_withdrawal_holdings_gate.rs` 725 | 800 | all under |

The developer's choice to put every new rule into a new module rather than growing the three over-budget files is the right one and is why this is an observation, not a finding. `apply_block/mod.rs` landing on exactly 500 is worth naming: it has no headroom left.

---

# Findings

## [F1] MAJOR — the banked residual covers R1 only; the whole rule table and `bond_input_split` are three unlocked transcriptions

- **Severity:** Major
- **Location:** `bins/node/src/node/validation_checks.rs:664-680, 838-896` · `bins/node/src/node/production/withdrawal_holdings.rs:106-128, 194-208` · `crates/mempool/src/withdrawal_holdings.rs:68-99, 115-135`
- **Confidence:** `conf(0.90, observed)`
- **Evidence:**
  - `FIND-I180-M2-TRANSCRIPTION-001` in `.omega/memory.db` names exactly one duplicated expression: *"the allowance R1 exists as TWO transcriptions — `ProducerHoldings::allowance_with` … and an inline expression in `validate_block_economics`"*.
  - It is not two, and it is not only R1. `bond_input_split` exists in three places with **byte-identical bodies**: `validation_checks.rs:664-679` (as an inline closure), `production/withdrawal_holdings.rs:194-208`, `crates/mempool/src/withdrawal_holdings.rs:120-134`. Each does the same `utxo.get(&Outpoint::new(..))` → `output_type != Bond` → `saturating_add` → `pubkey_hash == owner` walk.
  - The R2 shape split is likewise triplicated: gate `validation_checks.rs:868-895`, builder `production/withdrawal_holdings.rs:113-128`, mempool `crates/mempool/src/withdrawal_holdings.rs:87-99`. R3 exclusivity: gate `:846`, builder `:107`, mempool `:83`. The address derivation `hash_with_domain(ADDRESS_DOMAIN, pk.as_bytes())` appears at `validation_checks.rs:653-656`, `production/withdrawal_holdings.rs:105`, `crates/mempool/src/withdrawal_holdings.rs:116`.
  - Only R1 has a term-exact lock. QA's own round-2 mutation table (`docs/qa/inc-i-180-M2-qa-report.md:757-767`) lists exactly one gate-drift detector before the lock was added, and the lock that was added (`inc_i_180_allowance_parity.rs`, 8 shapes, 2 tests) asserts **only** the R1 allowance and its echoed terms — R2/R3/R4 are never reached in those rows by construction (every probe declares `allowance + 1` and bails at R1).
  - The historical cost of exactly this class is on the record: ISSUE-001 was a `saturating_sub`/`saturating_add` order drift between two copies of the R1 expression, shipped, and caught only because QA wrote a partition with a non-zero `in_block_addbond`.
- **Impact:** No runtime defect today — I compared all three copies and they agree for every input. The defect is that the follow-up work item, as banked, would close roughly one third of the duplication. A future edit to the R2 full-exit branch or to `bond_input_split` in one layer and not the others reproduces INC-I-180's own root cause, and no test would go red until an end-to-end partition happens to straddle the drift.
- **Suggested fix (documentation-scoped, no code change in M2):** widen `FIND-I180-M2-TRANSCRIPTION-001` to name the full surface — the R0/R2/R3 arms, `bond_input_split` and the address derivation, not just `allowance_with` — and state that the `crates/storage` relocation must move the whole decidable rule body, not one function. Add to the routed-residual text that the follow-up is a prerequisite for pinning a real mainnet AH #23, alongside AUDIT-P2-004.
- **Test Strategy:** the relocation milestone should carry a single shared `fn bond_input_split(tx, utxo, owner) -> (u32, u32)` in `crates/storage` with all three layers calling it, plus a parity row per R2/R3 arm of the same two-sided shape as `inc_i180_m2_the_gate_allowance_equals_the_shared_function` (assert the gate's echoed counts equal the shared function's output). Until then, the lock is R1-only and should be documented as such.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — the proposal edits one memory.db finding row and one workflow document; no code path changes)
  Memory:   0 (observed — no allocation, no data structure touched)
  IO:       0 (observed — one SQLite UPDATE against a local file already open in this session)
  Network:  N-A (single-node bookkeeping; no peer traffic)
  Disk:     0 (observed — a few hundred bytes rewritten in memory.db and one .md file)
  Latency:  0 (observed — no request path, no consensus path, no block path is touched)
Inevitability: AVOIDABLE
Cheaper alternative: leave the residual scoped to `allowance_with` and rely on the next reader to re-derive the other two thirds.
Why this proposal anyway: the cheaper path is exactly how ISSUE-001 shipped — a duplicated expression that "currently agrees" is not a closed hazard, and a follow-up scoped to one third of the duplication will be marked done while the other two thirds stay unlocked.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## [F2] MINOR — the published holdings snapshot's "≤1 block stale" bound is false on every rollback/reorg path

- **Severity:** Minor
- **Location:** `bins/node/src/node/apply_block/mod.rs:373` (the only refresh) vs `bins/node/src/node/block_handling.rs:1076-1081` (`execute_reorg`'s `revalidate`) · claimed in `docs/.workflow/inc-i-180-M2-implementation.md:172-173`, `specs/protocol.md` (mempool bullet, "per-block snapshot"), `docs/architecture.md` ("The refresh is published BEFORE the `revalidate` pass that may fall back to it")
- **Confidence:** `conf(0.95, measured)`
- **Evidence:**
  - `grep -rn "refresh_mempool_producer_snapshot" bins/node/src/` returns exactly **two** lines: the definition site's comment at `init.rs:706` and **one** call site, `apply_block/mod.rs:373`.
  - `grep -rn "producer_set\.write()" bins/node/src/` returns **seven** mutation sites: `rollback.rs:147`, `rollback.rs:151`, `rollback.rs:274`, `fork_recovery.rs:337`, `apply_block/mod.rs:198`, `apply_block/genesis_completion.rs:46`, `apply_block/state_update.rs:184`, `block_handling.rs:747`, `block_handling.rs:751`, `block_handling.rs:927`. Only the ones under `apply_block/` are followed by a refresh.
  - `grep -rn "\.revalidate(" bins/node/src/` returns **two** production call sites: `apply_block/mod.rs:383` (refreshed at `:373` immediately above — the OBS-002 fix) and **`block_handling.rs:1080`, inside `execute_reorg` (`:498`), with no preceding refresh**.
  - The implementation report states the bound as fact: *"The node keeps it fresh in `refresh_mempool_producer_snapshot`, so it can never be more than one block stale."* QA validated it as *"The one-block staleness bound is enforced"* (`docs/qa/inc-i-180-M2-qa-report.md:419-421`) by checking the apply path only.
- **Impact:** Bounded and non-safety. The snapshot is consulted only when `try_read()` on the live `ProducerSet` fails, and both outcomes are benign: over-rejection is a one-confirmation liveness cost (the owner resubmits, no fee, no input spent) and under-rejection is pre-M2 behaviour still caught by the builder and by the gate. But the window is the reorg path — the path INV-VALIDATION-001 and INC-I-147 both come from — and a reader who trusts "≤1 block" will size the risk wrongly when pinning AH #23 or when reviewing the next change to this channel.
- **Suggested fix:** documentation-only for M2. Replace "per-block snapshot" / "never more than one block stale" with the measured statement: *the fallback snapshot is refreshed once per APPLIED block and is not refreshed by rollback, reorg, fork recovery or snapshot install, so after a rewind of depth N it is up to N blocks stale until the next block is applied; both staleness directions are non-safety because the live handle is tried first and both failure directions are caught downstream.* Optionally add a `refresh_mempool_producer_snapshot` call at the end of `execute_reorg` in a follow-up milestone — do NOT add it in M2, since it touches the reorg path and INV-PROD-002's sequencing caveat binds this milestone.
- **Test Strategy:** a node `it` row that rolls back N blocks through `rollback_one_block`, holds `node.producer_set.write()` across `Mempool::add_transaction`, and asserts the verdict is computed against the pre-rollback holdings (i.e. that the snapshot is stale) — this makes the true bound executable instead of asserted.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — the M2-scoped fix rewrites three prose sentences; no code path is added or removed)
  Memory:   0 (observed — no allocation)
  IO:       0 (observed — three markdown/source-comment edits)
  Network:  N-A (documentation change; no peer traffic)
  Disk:     0 (observed — a few hundred bytes across three files)
  Latency:  0 (observed — no runtime path affected)
Inevitability: AVOIDABLE
Cheaper alternative: leave the sentence and rely on the follow-up milestone to notice.
Why this proposal anyway: the sentence is a safety-relevant bound that a height-pinning decision will be sized against, and it is falsifiable in one grep — shipping it is the same defect class QA blocked twice in rounds 1 and 2.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## [F3] MINOR — three new protection mechanisms are unregistered

- **Severity:** Minor (protocol gate — must be discharged before close)
- **Location:** `crates/mempool/src/pool.rs:566-572` (admission filter), `crates/mempool/src/pool.rs:1293-1305` (revalidate eviction), `bins/node/src/node/production/assembly.rs:322-334` (builder skip gate)
- **Confidence:** `conf(0.95, measured)`
- **Evidence:**
  - `SELECT mechanism_id, name, status FROM protection_mechanisms WHERE mechanism_id LIKE '%180%' OR name LIKE '%withdrawal%' OR name LIKE '%holdings%';` returns exactly one row: `PM-180-01|withdrawal-holdings gate (pre-mutation RequestWithdrawal cap)|active` — the M1 consensus gate.
  - The precedent is registered and is the identical shape: `PM-021|duplicate-registration admission filter|A TxType::Registration arrives at Mempool::add_transaction / add_system_transaction …` and `PM-022|revalidate duplicate-registration eviction|Mempool::revalidate runs (after every apply_block and after reorg) …`. M2 adds the withdrawal analogue of both, plus a third at block assembly, and registers none.
  - `.omega/gauntlet.conf:24` configures `domain: bins/node/src/node/`, so the system-impact protocol is armed for this milestone.
- **Impact:** `v_protection_surface` is the query the next agent runs to find interacting protections. Three unregistered mechanisms are invisible to that query, so the next change to mempool admission, block assembly or the rollback ladder will reason about a protection surface that is missing the ones M2 just added. This is precisely the composite-failure gap the registry exists to close.
- **Suggested fix:** register three rows before close. Trigger conditions, actions and scale assumptions are all already measured and stated in the implementation report §6 "Resource cost" and its Failure-Modes blocks — this is transcription, not new analysis:
  - `PM-180-02 · builder withdrawal-holdings skip gate` — trigger: at `height >= AH #23`, a candidate `RequestWithdrawal` fails R0/R1/R4/R3/R2 against the in-block accounting; action: `warn!` + `continue` (never `Err`, never abort, never evict); scale: O(candidate set) producer-set reads once per block plus O(tx.inputs) per withdrawal; interacts-with: PM-180-01 (must be a strict subset of its verdict), INV-PROD-002 rollback ladder (must never reach it).
  - `PM-180-03 · mempool withdrawal-holdings admission filter` — trigger: `RequestWithdrawal` at `add_transaction` with `current_height >= AH #23` and a resolvable holdings source; action: reject with the gate's bracketed code before fee work; scale: one `try_read` + `get_by_pubkey` + `pending_addbond_count` per withdrawal only; interacts-with: PM-021 (same entry point), PM-180-02, PM-180-04.
  - `PM-180-04 · revalidate withdrawal eviction` — trigger: `Mempool::revalidate` (after every applied block AND after reorg) holds a `RequestWithdrawal` the ledger has moved out from under; action: evict, logged with the bracketed reason, residents counted as 0 deliberately (INC-I-147 both-members lesson); scale: O(resident withdrawals) per revalidate; interacts-with: PM-022 (same pass), PM-180-03 (strictly weaker — residents are 0 here, so nothing admitted can flap).
- **Interaction analysis I ran, for the record (`v_protection_surface`, 39 active rows):** PM-180-02/03/04 share a trigger surface with PM-180-01, PM-021 and PM-022. (i) *Can two fire on one event?* Yes — PM-180-03 and PM-021 both evaluate at `add_transaction`, but on disjoint tx types (`RequestWithdrawal` vs `Registration`), so neither shadows the other. (ii) *Can one's action create another's trigger?* No — a builder skip removes a transaction from one candidate block without touching the mempool (`req_i180_003_skip_never_fails_aborts_or_rolls_back` locks the receiver cells), and an eviction cannot cause an admission. (iii) *Can one starve the input another needs to disarm?* The only candidate is PM-180-04 evicting a withdrawal that PM-180-01 would have accepted — bounded to a resubmission, and the resident charge is deliberately what keeps `[partial(P), full-exit(P)]` off any builder (SEC-FIXVERIFY2-001). **No unanswered interaction.**

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — three INSERTs into a local SQLite table read only by agents, never by the node)
  Memory:   0 (observed — no runtime data structure)
  IO:       0 (observed — three row writes to .omega/memory.db)
  Network:  N-A (local institutional memory; never transmitted)
  Disk:     0 (observed — a few kilobytes)
  Latency:  0 (observed — memory.db is not on any node code path)
Inevitability: AVOIDABLE
Cheaper alternative: register them in the follow-up milestone that relocates ProducerHoldings.
Why this proposal anyway: the registry's whole value is being complete AT THE MOMENT the next agent queries it, and the next agent to touch mempool admission or block assembly will query it before this milestone's follow-up exists.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## [F4] MINOR — no gauntlet run exists for run 525, and two M2 changes are live below the AH today

- **Severity:** Minor (protocol gate — blocks workflow close, not code correctness)
- **Location:** `.omega/gauntlet.conf:24` (`domain: bins/node/src/node/`) · `.omega/memory.db` table `gauntlet_runs`
- **Confidence:** `conf(1.00, measured)`
- **Evidence:**
  - `SELECT id, run_id, status, scenarios_run, scenarios_passed, git_sha, created_at FROM gauntlet_runs ORDER BY id DESC LIMIT 3;` → `39||pass|11|11|…|3f8bf185|2026-08-12`, `38||pass|11|11|…|7f917e7a`, `37||fail|11|10|…|7f917e7a`. **No row references run 525, and the newest row is INC-I-174's commit `3f8bf185`.**
  - `.omega/gauntlet.conf` lists `domain: bins/node/src/node/`; M2 modifies eight files under that prefix.
  - The gate config's own text: *"Workflow close (status=completed/partial) requires a fresh gauntlet_runs pass row when the run touched a configured domain."*
- **Impact:** This is not bookkeeping in this milestone's case. AH #23 is unreached on testnet (`230_000`), so the gauntlet cannot exercise any post-AH path — but two M2 changes ARE live below the AH on the local testnet today: the `apply_block/mod.rs` mempool-hygiene reordering (which moves `remove_for_block` + `revalidate` across `put_block` and `batch.commit()` on the hot apply path for every block) and the `rewards.rs` auto-revoke mirror (which changes `rebuild_producer_set_from_blocks` output, i.e. the reorg-recovery path). GS-005 (block-store completeness) and the rolling-restart scenario (GS-009) are exactly the scenarios that would surface a regression in those two. A pre-existing `fail` row at `37` for GS-005 makes the case stronger, not weaker.
- **Suggested fix:** run `scripts/gauntlet.sh` against the M2 tree before close and record the pass row against run 525. `--gs009` (fleet rolling restart) is the scenario with the highest relevance to the apply-path reordering and should be included if the operator consents; the default observational set is the minimum.
- **Test Strategy:** the gauntlet IS the test; the acceptance criterion is a `gauntlet_runs` row with `run_id=525`, `status='pass'` and a `git_sha` matching the committed M2 tree.

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      +moderate, bounded to one run (observed — `scripts/gauntlet.sh` drives the 20-node local testnet through 10 scenarios; the last three recorded runs took 83, 88 and 85 seconds)
  Memory:   +moderate, transient (observed — the local testnet fleet is already resident; the runner adds one Python collector process)
  IO:       +moderate, transient (observed — RPC polls across ports 8500-8512 plus one launchd service restart in the default set)
  Network:  +loopback only (observed — every node is on 127.0.0.1; no external peer traffic)
  Disk:     +small (observed — one `gauntlet_runs` row plus the runner's log; `--gs010` would write to the testnet CHAIN and is opt-in, not part of this recommendation)
  Latency:  +~85s one-off (measured — duration_seconds of gauntlet_runs rows 37/38/39)
Inevitability: AVOIDABLE
Cheaper alternative: close run 525 on the workspace suite alone (3728 passed, failing set byte-identical to baseline) and skip the fleet-level run.
Why this proposal anyway: the workspace suite runs one node in one process; the two changes that are live below AH #23 are an apply-path reordering and a reorg-recovery path, and neither failure mode (mempool state after a failed commit, producer-set divergence after a rewind) is observable without a multi-node fleet that actually reorgs. The gate config was armed for exactly this domain.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## [F5] MINOR — the `is_empty()` early return has no Path-Coverage entry and no test

- **Severity:** Minor (protocol gate — `path-coverage-gate.sh` fires on commit)
- **Location:** `crates/mempool/src/holdings.rs:92-94`
- **Confidence:** `conf(0.95, observed)`
- **Evidence:**
  - The branch is `if guard.is_empty() { return HoldingsLookup::Unavailable; }` — a new early-return guard in non-test Rust, which OMEGA rule 24 and `.claude/protocols/path-coverage.md` bind.
  - `docs/.workflow/inc-i-180-M2-implementation.md:422` lists three `HoldingsSources::lookup` outcomes (live hit / snapshot hit / `Unavailable` unwired). The fourth — `Unavailable` because the snapshot is populated-but-empty — is absent. That block is the commit body's `Path-Coverage:` material.
  - No test drives it: `crates/mempool/tests/it/inc_i_180_admission_parity.rs` seeds `case.holdings` in every row (a POPULATED snapshot), and every `bins/node` row resolves through an uncontended live handle. QA's `PROBE-EMPTY` covered it and was deleted (`docs/qa/inc-i-180-M2-qa-report.md:906-915`, OBS-006, still open at round 3).
  - The branch is the fail-open/fail-closed decision point: without it, a contended `try_read()` under `Node::new_for_test` (`init.rs:1141`) or `Node::new_for_replay` (`init.rs:1353`) makes every producer read `Unregistered` and reject — the opposite of what the layer documents.
- **Impact:** Not production-reachable through `Node::new` (which seeds the snapshot at `init.rs:740-752`), so no live-network consequence. But `new_for_replay` backs the operator reindex tool, and the branch that decides whether admission censors every producer under contention is currently protected by nothing.
- **Suggested fix:** add the fourth outcome to the `Path-Coverage:` block, and add one test that write-holds `node.producer_set` across `Mempool::add_transaction` on a `new_for_test` node and asserts `Ok` for a withdrawal declaring more bonds than any source would allow. QA already wrote and ran exactly that probe; re-land it as a permanent row rather than re-deriving it.
- **Test Strategy:** `PROBE-EMPTY` verbatim — hold the live write guard, seed nothing, submit `RequestWithdrawal(P, 99)` against `bond_count = 1`, assert admitted. RED if `is_empty()` is removed (every producer reads `Unregistered` → reject).

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — one added `#[tokio::test]` row; the production code path is unchanged)
  Memory:   0 (observed — test-only allocation, torn down per test)
  IO:       0 (observed — one test file edit plus one line in the commit body)
  Network:  N-A (in-process test; no peer traffic)
  Disk:     0 (observed — a few kilobytes of test source)
  Latency:  +~1 test (observed — one more row in a target that currently runs 68)
Inevitability: AVOIDABLE
Cheaper alternative: add the Path-Coverage line only and leave the branch untested.
Why this proposal anyway: the Path-Coverage protocol exists because an unexercised early return is where fail-open silently becomes fail-closed, and this specific branch was ADDED to fix exactly that inversion — leaving it untested reopens the defect it closed.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## [F6] MINOR — the per-change AH classification table omits `apply_block/mod.rs`

- **Severity:** Minor
- **Location:** `docs/.workflow/inc-i-180-M2-implementation.md:19-32` (the `| File | Lines | Item | Class |` table) · the omitted file is `bins/node/src/node/apply_block/mod.rs` (+11 −9)
- **Confidence:** `conf(0.95, observed)`
- **Evidence:**
  - `git diff --numstat e6c066c7` lists `11 9 bins/node/src/node/apply_block/mod.rs`. The §1 classification table lists eleven files and this is not one of them; the file appears only in the round-1 "Files changed in this round" list at line 626 with the note `(OBS-002 ordering)` and no `AH-VISIBLE` / `NODE-LOCAL` class.
  - Brief §4 makes the classification a hard rule: *"Consensus-visible parity rules ride the SAME AH #23; builder and mempool layers are node-local policy. **Say which is which, per change, in the commit body.**"*
  - This is not a harmless omission. The reordering moves `remove_for_block` + `revalidate` from before `put_block`/`batch.commit()` to after them, on the apply path of **every block on every network today**, below AH #23. It is one of only two changes in the milestone with that property (the other, the S5 mirror, IS classified and IS carried into a third deploy answer at §9).
- **Impact:** The commit body the runner transcribes will assert a per-change classification that does not cover one of the two pre-AH-live changes. A reader reconciling the commit against the diff will find an unclassified file in a domain where classification is the gate.
- **Suggested fix:** add the row — `bins/node/src/node/apply_block/mod.rs | +11 −9 | OBS-002 mempool-hygiene reordering | NODE-LOCAL (mempool policy; pre-AH-LIVE on every network)` — and extend §9's third deploy answer to name it alongside the S5 mirror, with its one declared delta (on a `put_block` / `batch.commit()` error the block's transactions now stay in the mempool).
- **Test Strategy:** NOT_TESTABLE — this is a commit-body completeness defect, not a behavioural one. The behaviour itself is covered by the existing suite and by the F4 gauntlet run.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — one table row and one sentence in a workflow document)
  Memory:   0 (observed — no code touched)
  IO:       0 (observed — one markdown edit)
  Network:  N-A (documentation change)
  Disk:     0 (observed — under 500 bytes)
  Latency:  0 (observed — no runtime path)
Inevitability: AVOIDABLE
Cheaper alternative: rely on §9's existing third deploy answer, which already flags the S5 mirror, to cover the deploy question generically.
Why this proposal anyway: §9's third answer names the S5 mirror specifically and says nothing about the apply-path reordering, so a deploy-gate reader weighing "what in this milestone is live today" gets one of the two.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## [F7] MINOR — four prose imprecisions survive in shipped files (QA OBS-010/011/012/013, all still open)

- **Severity:** Minor
- **Location:** `docs/.workflow/inc-i-180-M2-implementation.md:238` · `specs/protocol.md` (builder bullet, mempool bullet) · `docs/architecture.md` (Builder-parity paragraph, "Admission is not contained…" paragraph)
- **Confidence:** `conf(0.90, observed)`
- **Evidence:**
  - **OBS-012, and I re-confirmed it survives:** `docs/.workflow/inc-i-180-M2-implementation.md:238` still reads *"the peer already controls the entire `ProducerSet` and `UtxoSet` the node installs"*. The routing document was corrected for exactly this sentence in round 1 (OBS-005) and now carries the accurate form at `docs/.workflow/inc-i-180-M2-audit-p2-004-routing.md:106-107` (*"a field inside an object the syncing node installs from a peer — minus the state-root cross-check the object's other fields get"*). Read alone the surviving copy is false: those objects ARE cross-checked by the state root, which is what makes `pending_updates`' exclusion the finding at all.
  - **OBS-010:** `grep -c 'name: "IP-' bins/node/tests/it/inc_i_180_allowance_parity.rs` → 8, but the file defines two test functions (`inc_i180_m2_the_gate_allowance_equals_the_shared_function` at :281 and `…_at_the_ceiling` at :297). Three shipped files attribute "eight allowance shapes" to the first name alone. True as a `cargo test` filter (the attributed name is a strict prefix), imprecise as a sentence.
  - **OBS-011:** two `whenever` quantifiers state a necessary condition as sufficient. `specs/protocol.md`: *"a second order silently raises one layer's allowance above the other's whenever `withdrawal_pending > bond_count + pending_addbond`"* — it also requires `in_block_addbond > 0`, and QA's own MUT-A run is the counter-example (IP-DEFICIT is that ledger with `in_block_addbond = 0` and stayed GREEN).
  - **OBS-013:** `docs/architecture.md` shortens the boundedness claim to *"one confirmation deep"* while `specs/protocol.md` states the accurate form, *"the resident confirms **or expires**"*. A resident no builder ever selects holds the second withdrawal out until the 14-day mempool expiry, not until a confirmation.
- **Impact:** None asserts a false safety property and none would make a reader less careful — QA's judgement not to block on them is correct. But three QA rounds were consumed by prose in these same two files being falsifiable in one command, and OBS-012 is a sentence that was already corrected once in a sibling document and left standing here.
- **Suggested fix:** four one-line edits. (a) replace the §5 sentence with the routing document's corrected form verbatim; (b) "…locked by the two `inc_i180_m2_the_gate_allowance_equals_the_shared_function*` rows, eight shapes in total"; (c) "can over-reject only when…" in both `whenever` sites; (d) add "or expires" to `docs/architecture.md`.
- **Test Strategy:** NOT_TESTABLE — prose. (a) is verifiable by `grep -rn "entire \`ProducerSet\`" docs/` returning nothing.

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — four sentence edits across three markdown files)
  Memory:   0 (observed — no code touched)
  IO:       0 (observed — three file edits)
  Network:  N-A (documentation change)
  Disk:     0 (observed — under 1 KB)
  Latency:  0 (observed — no runtime path)
Inevitability: AVOIDABLE
Cheaper alternative: ship as-is; QA already judged all four non-blocking and the safety properties described are real.
Why this proposal anyway: OBS-012 is the one that matters — the same sentence was corrected in the sibling document and left standing here, and it is the sentence a height-pinning session will read when sizing the AUDIT-P2-004 residual.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

---

## Speculative Findings (low-confidence, not actionable)

- **[S1] `warn!` amplification on the builder skip path.** `conf(0.60, inferred)`. `assembly.rs:325-333` emits a `warn!` per skipped withdrawal per build attempt, and the skip does not evict, so a withdrawal that the mempool admits but the builder repeatedly skips produces one `warn!` per producer per slot until the 14-day mempool expiry. I could not construct a cheap way to populate that set at scale: admission counts residents, so a second same-producer withdrawal is rejected at admission; `in_block_addbond` only ever raises the builder's allowance; and R4 is unreachable per §1. The reachable population is confined to transactions admitted while the holdings source was `Unavailable`, or admitted below the AH and still resident when it activates. The idiom also matches the pre-existing NFT/Pool skip gates the brief told the developer to copy. Recorded for the height-pinning session, not proposed as a change.

## Observations (no change proposed)

- **`TxType::Exit` is currently unmineable and un-admittable**, which makes both the M1 gate's Exit charge (`validation_checks.rs:714-739`) and the M2 builder's Exit mirror (`production/withdrawal_holdings.rs:157-171`) dead code today. I verified this independently of QA: `validate_exit_data` (`crates/core/src/validation/tx_types.rs:11-24`) requires zero inputs AND zero outputs, `allows_empty_io(Exit)` is `false` (`transaction/types.rs:190`), so `is_zero_flow()` is false and every Exit routes to `add_transaction`, where the non-state-only branch of `validate_transaction_with_utxos` applies a fee check against `total_input = 0`. Keeping the mirror is the right conservative choice, and IP-EXIT's `!TxType::Exit.allows_empty_io()` assertion converts a future change into a test failure rather than a silent divergence. Pre-existing, out of M2 scope, worth naming so the next reader does not re-derive it.
- **Mempool height gating is off by one at the AH boundary, in the safe direction.** `add_transaction` receives `current_height = chain_state.best_height` while the transaction will be included at `best_height + 1`, so at exactly `height == AH − 1` admission is a no-op while the block at `AH` applies the gate. Brief §S2 explicitly sanctions the weaker direction (*"Being weaker than the builder is correct here"*), and the builder — which is armed against the height being built — closes it. Not a defect.
- **QA OBS-007/008 (overflow-semantics transcriptions) and OBS-009 (builder R4 omits protocol-generated transactions)** are correctly classified as unreachable. OBS-009 in particular deserves the one-line code comment QA asked for, since the unreachability rests on a property of a different function in a different crate (`OutputNotFound` at `crates/core/src/validation/utxo.rs:127`).
- **`apply_block/mod.rs` is now exactly 500 lines.** No headroom remains against the module budget.

## Specs/Docs Drift

Two rows, both covered above: F2 (the staleness bound, in `specs/protocol.md` + `docs/architecture.md` + the implementation report) and F7 (four prose imprecisions). Everything else added to `specs/protocol.md` and `docs/architecture.md` in this milestone was re-derived TRUE against the code — see §7's table. `specs/SPECS.md` and `docs/DOCS.md` index entries for `protocol.md` and `architecture.md` already exist and need no update (no new file was added to either tree).

## Modules Not Reviewed

None within M2 scope. Out-of-scope items per brief §3 were confirmed untouched, not reviewed.

## Contradiction Check

No self-contradiction found in the implementation report, the QA report or the routing decision. The one superseded claim (round-1 §ISSUE-002's containment enumeration) is explicitly marked SUPERSEDED at `docs/.workflow/inc-i-180-M2-implementation.md:515-520` and points forward, which is the correct handling — the round-1 record is preserved rather than rewritten. The stated fix matches the actual code change in every case I checked: ISSUE-001 was described as a saturating-order defect and was fixed by unifying the order, not by a retry or a tolerance.

## Invariant Compliance

| Invariant | Verdict |
|---|---|
| **INV-VALIDATION-001** (three-path parity, contract over the whole context) | **MET** — all three paths drive the same rule table; the containment relation between them is now stated correctly and locked by rows that drive `Mempool::add_transaction` AND `validate_block_economics` on one node. The `[BLOCK_POISON]` monitoring signal exists at `bins/node/src/node/production/mod.rs:624` and was not touched. |
| **INV-PROD-003** (builder no weaker than apply) | **MET** — see §1; the one structural asymmetry (R4 hash set) is unreachable, proven from `crates/core/src/validation/utxo.rs:125-130`. |
| **INV-PROD-002** (poison-rollback path must not be removed or altered) | **MET** — `git diff --stat e6c066c7 -- bins/node/src/node/rollback.rs bins/node/src/node/block_handling.rs` shows `rollback.rs` absent entirely; `block_handling.rs` is absent from the M2 numstat. `production/mod.rs` is `1 0` (one module declaration). The refusal is `warn!` + `continue`, never `Err`. |
| **INV-EPOCH-002** (no state rebuild from local block history on a snap-synced node) | **RESPECTED** — it is the stated reason S4 option (b) was rejected, applied verbatim. |

## Final Verdict

**Approved for commit, conditional on three protocol discharges and one residual widening.**

The engineering is sound and the milestone closes its root cause. Before the commit and the workflow close:

1. **F3** — register `PM-180-02/03/04` in `protection_mechanisms` (content is already written above; transcription only).
2. **F4** — run `scripts/gauntlet.sh` against the M2 tree and record the pass row against run 525.
3. **F5** — add the fourth `HoldingsSources::lookup` outcome to the `Path-Coverage:` block and re-land QA's `PROBE-EMPTY` as a permanent test row.
4. **F1** — widen `FIND-I180-M2-TRANSCRIPTION-001` to name the full duplicated surface, not just `allowance_with`.
5. **F2, F6, F7** — five documentation edits, all one line each.

None of these is a code-correctness defect and none requires a Developer iteration on the implementation. The commit body must also carry, verbatim from the implementation report §9: the three-question consensus-shape checklist, BOTH deploy questions plus the third answer (extended per F6), the `Path-Coverage:` block (extended per F5), and the `Failure-Modes:` block — the last of which is mandatory because `bins/node/src/node/` is a configured gauntlet domain.

━━━ SECURITY AUDIT VERDICT ━━━
Verdict: AUDIT-REQUIRED
Signals: (1) **External data / trust boundary** — unauthenticated mempool admission gains a new rejection path reachable by any network peer through `handle_new_transaction` and by any RPC client through `submitTransaction`; (2) **State integrity** — the change decides bond-withdrawal allowances, i.e. which financial state transitions a block may carry, and the S5 mirror moves a `serialize_canonical()` field (`received_delegations`) that feeds the producer-set contribution to the state root; (3) **Consensus-adjacent validation** — `validate_block_economics` gains 27 lines of mode-dependent behaviour under AH #23, and block assembly gains a new selection-time refusal; (4) **Concurrency / lock acquisition** — a non-blocking `try_read()` on a shared `Arc<tokio::sync::RwLock<ProducerSet>>` is now taken from the mempool's synchronous context under two other guards; (5) **Known unresolved trust gap in scope** — AUDIT-P2-004 (`pending_updates` outside the state root) was routed out rather than closed, and post-AH it lets a hostile snapshot-serving peer move a syncing node's withdrawal allowance.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
