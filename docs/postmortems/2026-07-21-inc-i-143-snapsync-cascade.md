# INC-I-143 — Fleet-wide SnapSync cascade after the 2026-07-21 mainnet deploy

**Status:** OPEN — root cause NOT fully established
**Severity:** critical · **Protection level:** 3
**Network:** mainnet (3 seeds + N1-N12 + ~30 external producers)
**Workflow run:** 465 (`/omega-swarm --deep`)
**Author:** investigation orchestrated with operator (Antonio Lozada)

---

## 1. Executive summary

A rolling deploy of a new node binary across the entire mainnet fleet was followed within ~19 minutes
by Seed1 falling out of sync, and subsequently by a fleet-wide SnapSync cascade. The operator rolled
the producers back to 6.23.10. The fleet is now **synced but not sound**: all 15 nodes agree on height
and hash, but **every node reports `BLOCK 1` missing and `INTEGRITY -1`** — permanent damage that did
not exist before the deploy.

**What is proven:** the deployed binary changed node behavior at ~07:00Z, and the first failure
followed at 07:19Z. The operator's sequencing (healthy → deploy → seed1 behind → cascade) is correct
and is corroborated by the nodes' own logs.

**What is NOT proven:** the mechanism connecting the binary change to the initial block starvation on
Seed1 at 07:19Z. Two candidate mechanisms were investigated and **both were falsified by measurement**
(see §6). This report does not assert a root cause, because one has not been established.

---

## 2. Version discrepancy (operator-facing, important)

The deployed binary reports version string **6.23.11** but is actually commit **`1c510919`** — four
commits past the `v6.23.11` tag. The version string reads 6.23.11 only because `version.workspace`
bumps in the following commit.

Consequences:
- The INC-I-142 gossip staleness gate **is** live in the deployed binary (it was assumed absent).
- Any "roll back to the previous binary" reasoning must account for the fact that the deployed artifact
  was never the tagged 6.23.11.
- **Action item:** the deploy pipeline must stamp the actual commit SHA, not the workspace version.

Current fleet version state (10:35Z): **seeds still on 6.23.11, producers rolled back to 6.23.10.**
The fleet is running mixed versions.

---

## 3. Evidence-based timeline (2026-07-21, UTC)

| Time | Event | Evidence |
|---|---|---|
| pre-07:00 | Fleet fully healthy since genesis. `BLOCK 1 ✓`, `INTEGRITY ✓` on all 15 nodes | operator dashboards |
| ~07:00 | **Deploy lands.** Eager per-block state-root compute stops | `[STATE_ROOT]` rate drops **360/hour → 3/hour** on ai1 |
| 07:19 | **Seed1 stops receiving blocks** | `[SILENCE_PULL] No block for 382s` @07:25:31 |
| 07:25:30 | Seed1 stuck at h=108456, orphan gossip | `[SYNC] 3 consecutive orphan gossip blocks (local_h=108456, tip_h=108457, gap=1)` |
| 07:26:06–07:33:39 | **454 byte-identical refusals to self-recover** | `[FINALITY_GUARD] refusing StuckFork ShallowRollback`, `target_h=108455 < finality=108456 = local_tip` (`recovery.rs:359-378`) |
| 07:33:41 | Seed1 asks 24 peers for state roots; **quorum fails** | `[SNAP_SYNC] Batch-sending GetStateRoot to 24 peers simultaneously` → `No majority best_hash among 29 peers — network too fragmented for snap sync` |
| 07:33:44 | **Snap sync installs a bad anchor** | `[SNAP_SYNC] … hash=35574faf… height=108505` — but `35574faf` is canonically at **108506** (verified on ai2 by hash and by height) |
| 07:33:47 | Integrity hole opens | `[INTEGRITY] 45 missing blocks in 1..=108500`; auto-repair imports **0** blocks |
| 08:09:36 | **Epoch 302** — 12 min late, first epoch with **37 producers** (was 36 for E296-E301), bonds 9269→9344 | `[EPOCH] Bond snapshot fingerprint: epoch=302 producers=37 total_bonds=9344` |
| 08:09:50 | **seed2 / seed3 / n11 halt** | `[ECON_EPOCH_INPUTS_MISMATCH] EpochReward pool inputs mismatch at height=108720: expected 360 inputs, got 360` |
| ~10:00 | Operator rolls producers back to 6.23.10 | — |
| 10:35 | Fleet synced at h=109215 `a7adc1ba…e67687` — **but all 15 nodes show `BLOCK 1 ✗` and `INTEGRITY ✗ -1`** | operator dashboard |

### Fleet state at deploy completion (08:30Z dashboard)

Only **Seed2 (108,480)** and **N11 (108,480)** held the correct tip, both showing a distinct integrity
value `b0a300c4..`. All other nodes were at 108,479. Seed1 and Seed3 were `BEHIND -24`, degrading to
`-39` minutes later. **Divergence was already present the moment the deploy finished.**

---

## 4. Confirmed defects (contributing, not proven-causal)

These are real, evidenced code defects surfaced by the investigation. Each is a genuine bug that made
the incident worse or unrecoverable. **None is established as the initiating cause.**

### D1 — Snap-sync admits a peer-supplied anchor height without validation
`crates/network/src/sync/manager/snap_sync.rs:173-178` computes a `quorum_root`, compares it against
the peer's `response_root`, logs the mismatch at `info!` — **and accepts the snapshot anyway**.
`bins/node/src/node/fork_recovery.rs:377` then persists the peer-supplied height verbatim.
The one independent reference that would have caught the bad anchor is computed and discarded.

### D2 — Block height is derived from local tip, not from the block
`bins/node/src/node/apply_block/mod.rs:51` assigns height as `best_height + 1`. `BlockHeader` carries
**no height field**. Therefore a single bad anchor shifts every subsequent block's height permanently.

### D3 — The offset cannot self-correct
`crates/storage/src/block_store/writes.rs:129-135` — the snap_horizon floor blocks backward correction
below the sync floor. Once installed, the offset is durable.

### D4 — The stuck-fork remedy is structurally unreachable
`recovery.rs:359-378` — the ShallowRollback target is hard-coded `local_height - 1`, but mainnet
finality reaches the tip within ~1s. Whenever a node stalls on a finalized tip, its only corrective
action is refused. Seed1 hit this 454 times; Seed3 escaped only via a timing race.

### D5 — Misleading validation error
`[ECON_EPOCH_INPUTS_MISMATCH] … expected 360 inputs, got 360` reports **equal** counts on failure. The
real comparison is on input identity/content, not count. The message actively misdirects diagnosis.

### D6 — Archive auto-repair is self-referential
ai1's archive is short exactly the same 49 blocks (`0000108457`–`0000108505`) that the node itself is
missing, so `--archive-to` auto-repair structurally cannot close the hole. It reported
"imported 0 blocks" on every attempt.

### D7 — Seed1 bootstraps to itself
ai1 **is** `seed1.doli.network`. All three seeds pass `--bootstrap /dns4/seed1.doli.network/...`,
so Seed1 has zero external bootstrap peers (measured: ai1 31 established connections vs ai3 43, ai1's
outbound set mostly same-host n1-n3 loopback, no outbound to ai2).

---

## 5. What the deploy demonstrably changed

The binary removed the eager per-block 3-state-root computation (`apply_block/state_update.rs`,
commit `df974e06`, "M2"), making the root fully lazy — computed on demand in `serve_state_root()`
(`bins/node/src/node/state_root_serve.rs:33`) and memoized keyed on `best_hash`.

Measured effect on ai1: `[STATE_ROOT]` computations fell from **360/hour (exactly 1 per block) to
3/hour**. This is the deploy's fingerprint and dates it precisely.

**Evidenced downstream consequence:** the memo is `best_hash`-keyed, so it is stale after every new
block. Peers asked "what is your state root?" now compute it fresh at whatever instant they are asked,
rather than returning a value fixed at the block boundary. Peers polled at slightly different moments
return different answers. This is consistent with the observed
`No majority best_hash among 29 peers — network too fragmented for snap sync` and explains **why
recovery failed**.

⚠️ This explains the *failure to recover*. It does **not** explain why blocks stopped arriving at
Seed1 at 07:19Z. That gap is the open question.

---

## 6. Falsified hypotheses (recorded so they are not re-derived)

| # | Hypothesis | Verdict | Falsifying evidence |
|---|---|---|---|
| H1 | The lazy-state-root change is causal via a removed *validation* | **FALSE** | The deleted code was compute + log only; root bytes byte-identical at every height |
| H2 | A height-index schism predated the deploy by ~108k blocks; the deploy merely silenced the canary | **FALSE — and methodologically invalid** | Derived by comparing live node state *after* multiple nodes had already snap-synced. Post-incident state cannot establish what predated the incident. Contradicted by operator's direct observation of a healthy fleet pre-deploy |
| H3 | Epoch boundaries forced a latent split into consensus | **FALSE** | Epochs are 360 blocks = exactly 1 hour. A split tested hourly cannot wait 301 epochs to surface |
| H4 | A `GetStateRoot` recompute storm starved block application | **FALSE** | State-root computes went **down** (360/hr → 3/hr), not up. Only 1 batch fan-out logged on seed1 |

---

## 7. Open questions

1. **PRIMARY — why did blocks stop arriving at Seed1 between 07:00 and 07:19Z?** This is the true first
   domino and remains unexplained. All established findings are downstream of it.
2. Was the fleet-wide stall at h=108455 (all 21 peers pinned for 34 slots, `gap=0`, `sync_fails=0`,
   `[PROD_DIAG] eligible_len=1`) a cause or a consequence? A production halt with `eligible_len=1` is
   its own serious fragility and may warrant a separate incident.
3. Did the 37th producer joining at epoch 302 change the reward input set in a way the two node groups
   computed differently — or is D5 masking a different failure entirely?
4. **Why did INC-I-139's fix not hold?** INC-I-139 ("block 1 missing, integrity -1" after SnapSync
   escalation) is marked *resolved*; this is the same signature at fleet scale.
5. What does fleet-wide `INTEGRITY -1` actually cost operationally — is reward accounting affected
   going forward, or is it archive-availability reporting only?

---

## 8. Why pre-deploy testing did not catch this

The change passed `gauntlet 8/8` on the local testnet. The local testnet does not reproduce:
- ~30 external producers outside the structural fleet
- a seed that is also the fleet's DNS bootstrap target (D7)
- finality reaching the tip within ~1s, which is what makes D4 unreachable
- snap-sync quorum formation across ~30 heterogeneous peers

The three-question consensus-shape checklist (INC-I-075) was answered in the commit and answered
**correctly** — this change genuinely does not alter consensus rules or block content. The checklist
does not ask whether a change alters the **timing or availability of data that recovery paths depend
on**, which is what happened here.

**Proposed gap closure:** add a fourth question — *"Does this change when, how often, or under what
conditions any value consumed by a recovery/sync path is produced?"*

---

## 9. Current state (as of 10:35Z)

- All 15 nodes synced: h=109,215, hash `a7adc1ba…e67687`
- **All 15 nodes: `BLOCK 1 ✗`, `INTEGRITY ✗ -1`** — does not self-heal
- Mixed versions: seeds 6.23.11, producers 6.23.10
- Chain is live and producing; no active halt

**No remediation has been applied.** An earlier recommendation in this investigation (re-snap the
minority group onto the majority) was **retracted before execution** — it was based on falsified
hypothesis H2, and acting on it would have destroyed evidence.

---

## 10. Recommended next steps

1. **Do not** re-snap, wipe, or resync any node yet — current state is the evidence.
2. Investigate the **07:00–07:19Z window** on Seed1 to establish the initiating mechanism (§7 Q1).
3. Resolve the mixed-version state deliberately, one node at a time, once the mechanism is understood.
4. Determine the operational cost of fleet-wide `INTEGRITY -1` (§7 Q5) — this decides urgency.
5. Fix D1–D6 independently of root cause; each is a real defect with its own failure mode.
6. Fix the deploy pipeline to stamp the actual commit SHA (§2).

---

## 11. Investigation quality note

This investigation produced **three** incorrect root-cause statements before arriving at "not yet
established". Each was corrected by the operator, not by the investigation:

1. Asserted the height schism predated the deploy — based on contaminated post-incident state (H2).
2. Asserted epoch boundaries were the trigger — corrected by the operator's knowledge that epochs are
   hourly (H3).
3. Repeatedly presented downstream cascade mechanisms as "root cause" when the operator was asking
   about the first deviation from healthy.

Behavioral learnings recorded in `.omega/memory.db`:
- Root cause = the **first** deviation from healthy, not the most dramatic failure in the chain.
- When an operator reports a clean before/after boundary around a deploy, that temporal evidence
  **outranks** any "this predates the deploy" conclusion derived from post-incident state, which is
  contaminated by the recovery actions the incident itself triggered.

**Related:** INC-I-139, INC-I-138, INC-I-103, INC-I-142, INC-I-082, INC-I-075

---

## 12. Re-investigation update (2026-07-21, run 466)

The follow-up domain synthesis (4/4 lens convergence, conf 0.95) **established the root cause** that §1
and §11 left as "not yet established":

- **Root cause = the deploy execution, not the binary's logic.** An unserialized, activation-height-less
  fleet-wide **rolling restart** onto artifact `1c510919` (~07:00–07:13Z) shipped the block-content
  commit `427d5050` (Ed25519-only attestations, empty BLS aggregate) rolling — the **3rd recurrence of the
  INC-I-062 shape** (after INC-I-062, INC-I-075). Enough scheduled producers were simultaneously state-less
  behind `STARTUP_GATE` to stall production for 34 slots (108567–108599). Onset is **07:13:30Z** (the
  `[HEALTH]` production stall), not 07:19Z — 07:19 was the arrival of the first sparse block. Production
  resumed on the stale parent `bb956e85`@108455 → a **genuine sibling split** at h=108456
  (`57ff018e`@slot108600 vs `830e319c`@slot108637). No commit in `v6.23.10..1c510919` is a code-logic or
  parameter regression; the deployed binary's logic is exonerated (4/4).
- **Report §5 refuted.** The lazy-state-root → best_hash quorum-failure wiring hypothesized here is refuted:
  the snap `best_hash` quorum is **STATUS-keyed** and independent of the lazy-root memo. The 07:33 quorum
  failure was genuine fork-induced tip fragmentation, not a memo-starvation artifact.
- **Fixes implemented.** F1 (deploy-content gate: block ungated fleet deploys touching producer-content
  paths; serialize structural restarts), F2 (INC-I-139 Phase-2 wedge-escape, `wedge_escape.rs`), F3
  (D4 StuckFork→`SiblingFetch`, `recovery.rs`), F4 (D1/D2 snap-anchor integrity gates, `snap_sync.rs`),
  F5 (D5 content-aware `ECON_EPOCH_INPUTS_MISMATCH`), and the **GS-009** gauntlet fleet-rolling-restart
  scenario (`scripts/gauntlet-gs009.sh`) are all implemented and tested.

Full evidence, causal chain, and per-commit archeology: `docs/.workflow/domain-diagnosis-report.md`.
