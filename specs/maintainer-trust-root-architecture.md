━━━ FINDINGS — 8 total (DECISION:8) ━━━

  [F1] DECISION conf(0.85, converged) — bins/node/src/run.rs:461 + crates/updater/src/verification.rs:66-79 — fail-closed on-chain trust-root read; delete the fail-open fallback to leaked constants
  [F2] DECISION conf(0.85, converged) — bins/node/src/node/periodic.rs:44-72 — kill the bootstrap reset button; one-shot genesis seed + single canonical replayable derivation (AH-gated)
  [F3] DECISION conf(0.70, converged) — crates/updater/src/verification.rs:83-123 + crates/core/src/maintainer.rs:145-159 — distinct-signer k-of-n counter; adopt conditions/eval.rs:51-68 shape
  [F4] DECISION conf(0.70, converged) — bins/node/src/node/apply_block/governance.rs:112-124 — fail-close derive_ad_hoc; ProtocolActivation must not revert to producer keys (AH-gated)
  [F5] DECISION conf(0.70, measured) — bins/node/src/run.rs:452-454 + crates/storage/src/maintainer.rs:44-47 — versioned, fail-closed MaintainerState decoder; remove unwrap_or_default
  [F6] DECISION conf(0.68, converged) — bins/cli/src/cmd_upgrade.rs:82-106 — enforce verification at install; remove hardcoded Network::Mainnet
  [F7] DECISION conf(0.70, converged) — crates/core/src/maintainer.rs:323-326 + crates/updater/src/verification.rs:62 — replay domain (DOLI_MAINTAINER_V1), re-verify at install, sign release timing (AH-gated for governance msgs)
  [F8] DECISION conf(0.65, converged) — crates/updater/src/vote.rs + params.rs — delete dead weighted-veto machinery; revive derive_maintainer_set as the replayable derivation

  Speculative: 4 (report-only, not actionable) — weight-hatch override, binary transparency+timelock, cold-key role-separation ceremony, deferred consensus-state anchor
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Maintainer / Update-Signing Trust Root Architecture

> **Incident**: INC-I-172 (open, high, domain `updater/governance`). **Proposal-only** — no `--fix`.
> **Related**: INC-I-170 (key exposure, sequencing dependency), INC-I-171 (vesting penalty),
> INC-I-157 (release origin pinning), INC-I-153 (privileged install).
> **Method**: 5-evaluator parallel design synthesis (subtraction, restructure, patterns, failures,
> radical). Every claim carries `file:line` evidence. Layered, subtraction-first.
>
> **Implementation status** (code is the source of truth — the `file:line` references below
> describe the tree BEFORE each milestone landed):
> * **M1 (Layer 1, node-local, no AH)** — F1, F5, F6, and the release-verification half of F3/F7,
>   plus the F8 veto deletion. Shipped.
> * **M2 (Layer 2, gated on `NetworkParams::maintainer_derivation_activation_height`:
>   mainnet `172_000`, testnet `127_200`, devnet `0`)** — F2, the governance half of F3, F4,
>   and the F8 `derive_maintainer_set` revival. Shipped. `crates/core/src/maintainer.rs` was
>   split into the directory module `crates/core/src/maintainer/`
>   (`mod` / `set` / `data` / `derivation` / `tests`), so §F3's and §F7's
>   `crates/core/src/maintainer.rs:NNN` references now resolve to `maintainer/set.rs` and
>   `maintainer/data.rs`.
>   **M2 shipped PARTIAL against F2/F-6/F-7** — see the amendments in §F2, §Failure Filters
>   (F-7) and §Proposed Architecture, and the scope file
>   `docs/.workflow/inc-i-172-M3-scope.md`. Summary: the replay derivation exists but has no
>   production callers (**R1**), reorg had no maintainer undo (**R2**, now CLOSED — see the
>   next bullet), and snap-only nodes do NOT fail closed (**R3**). None is a regression
>   against pre-M2 behavior.
> * **R2 CLOSED by INC-I-174 M1 (2026-08-11).** Reorg now HAS a maintainer undo. The
>   pre-block trust root is recorded in a separate 9-byte-prefixed `cf_undo` key family
>   (`crates/storage/src/state_db/undo.rs`, `MaintainerUndoSnapshot`) — `UndoData` itself is
>   byte-unchanged, so no re-encode and no state-root effect. Both rewind loops call the
>   same two functions in `bins/node/src/node/maintainer_rewind/` (plan = pure reads, run
>   early; commit = mutate, run after `atomic_replace`), so `rollback_one_block` and
>   `execute_reorg` cannot drift (INC-I-040). A rewind that CANNOT restore keeps the live
>   root and announces on the `MAINTAINER_REWIND_UNRESTORED` anchor with a machine-readable
>   `reason=` sub-token, and increments `maintainer_rewind_unrestored_count`. **R1 and R3
>   remain OPEN.**
> * **The record is authority for a BLOCK, never for a HEIGHT (`AUDIT-P1-001`, SYS-001).**
>   `MaintainerUndoSnapshot` carries a `MUND` magic, a `u16` version, the `block_hash` it was
>   captured for, and the `maintainer_set_digest` of the set it holds. `plan_maintainer_rewind`
>   routes every candidate through `maintainer_rewind/binding.rs::check_snapshot_binding`,
>   which refuses on any of the three before promoting to `Restore`. This is required because
>   the planner resolves its block through `CF_HEIGHT_INDEX`, and `put_block_canonical`
>   (`backfillFromPeer`, `doli-node restore`, the archiver, `rebuild_canonical_index`)
>   rewrites that index with no `apply_block` and no record refresh — so a routine operator
>   recovery, with no data-dir write, could otherwise install this host's own former set
>   (under INC-I-175, the publicly leaked bootstrap five) through the `info!` SUCCESS exit.
>   The record is node-local: no activation height, no `*_VERSION` bump, no new column family.
>   Defence in depth: snap-sync `install_snapshot` calls `prune_undo_above(block_height)` on
>   its success arm so a chain replacement cannot leave records describing the chain it
>   replaced. `validate_persisted_set` is unchanged and remains ONE function shared by the
>   load and restore paths — the load path has no block, so it cannot ask this question.
> * **The binding is STALENESS/DRIFT detection, NOT tamper detection (`AUDIT-P3-401`).** All
>   three checks run on PUBLIC, UNKEYED inputs: `MUND`/`1` are compiled constants, the
>   `block_hash` comparand is recomputed from a block in the same data dir as the record, and
>   `maintainer_set_digest` is `BLAKE3(domain ‖ genesis_hash ‖ threshold ‖ sorted members)`
>   with no node secret. It therefore detects a FOSSIL record, a record for a DIFFERENT
>   BLOCK, a record from ANOTHER CHAIN, a member list edited in place after capture, and a
>   record from a different BINARY GENERATION — which is exactly the `AUDIT-P1-001` class,
>   reachable with NO data-dir write. It does NOT detect an actor who can WRITE the data dir:
>   that actor recomputes a matching `block_hash` and `set_digest` in one BLAKE3 call. The
>   residual is ACCEPTED because the same access rewrites `maintainer_state.bin` — the LIVE
>   root, documented as unsigned and attacker-writable (`crates/storage/src/lib.rs`,
>   `StorageError::MalformedPersistedValue`) — a strictly shorter path to the same authority.
>   Do not describe this as authentication, tamper-proofing or integrity protection against
>   an attacker, and do not retire another control on the strength of the record.
>   **UPDATE (INC-I-196):** the `TrustRoot::resolve` M1 containment guard named here as an
>   example has since been DELETED — but not "on the strength of the record". It was
>   retired because its own stated exit condition was met: the distinct-signer governance
>   counter activated at `maintainer_derivation_activation_height` (mainnet 172_000). The
>   caution above still stands for every other control.
> * **Compiled bootstrap arrays rotated (INC-I-175 Phase 5 / INC-I-196).** Both
>   `BOOTSTRAP_MAINTAINER_KEYS_MAINNET` and `BOOTSTRAP_MAINTAINER_KEYS_TESTNET` now carry
>   signing-only wallets whose private halves are held outside this repository. Neither array
>   is the genesis producer five any more: mainnet cut over at h=331_457, testnet followed and
>   is compiled-only (the testnet on-chain maintainer set is `testnet/keys/producer_{508..512}`,
>   also tracked, so it could not be reused). The two arrays are DISJOINT — which narrows FM-12
>   (§F7) without closing it, because the signed message still carries no network term.
>   `req_196_004_no_network_ships_a_publicly_compromised_key`
>   (`crates/updater/tests/trust_root_fail_closed.rs`) is the standing guard.
> * **M3 (Layer 3)** — F7's replay-domain binding on governance messages, plus residuals
>   **R1** and **R3** from M2 (**R2** is closed, see above). NOT implemented. Scope:
>   `docs/.workflow/inc-i-172-M3-scope.md`.
> * Option A (weight hatch) and Option C (cold-key role separation) remain user decisions.

## Problem Statement

DOLI must be able to **rotate its software-update signing keys ("maintainer" keys) after the current
keys are compromised — a public leak (INC-I-170) or a future server hack — without a genesis reset and
without a synchronized fleet redeploy**. Assume the current 5 maintainer keys are ALREADY compromised
when the fix ships: all 5 private keys are byte-identical to committed
`testnetlinux/keys/producer_{1..5}.json` (analyst F1, INC-I-170 12/12 proof), and `REQUIRED_SIGNATURES=3`.
That premise was the state of BOTH compiled arrays until the INC-I-175/196 rotations; it no longer
describes either one (see Implementation status). It is retained because it is the problem this
architecture was built to solve, and the rotation is the exercise of that capability, not its retirement.

The root cause is architectural, not a single bug: no crate owns "who are the maintainers"; the on-chain
trust root exists but is never read (`run.rs:461` returns `Vec::new()`); the effective root is 5
compile-time constants; and a free `RemoveMaintainer` transaction wholesale-reverts any rotation back to
the leaked keys within the same block (FM-01). The synthesis below is **layered** because the SSF
one-liner ("wire `run.rs:461`") is necessary but demonstrably insufficient and is actively reverted by
FM-01.

━━━ RESOURCE COST — SUMMARY — COST-DECLARED ━━━
Dimensions:
  CPU:      -small; deletions remove a per-block sort and up to 2 redundant verifies; weight lookups only on rare rotation txs, Option A only (measured)
  Memory:   0 (observed)
  IO:       -small; removes spurious maintainer_state.bin rewrites; +1 replay only when the file is absent, which fresh-sync already pays (measured)
  Network:  0 (observed)
  Disk:     +~8B/node; a MaintainerState version tag; +~150B/release only if Option B ships (inferred)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: the analyst SSF one-liner — wire run.rs:461 to the on-chain set, 3 lines, zero resource delta
Why this proposal anyway: the one-liner leaves the on-chain set equal to the leaked N1–N5, leaves rotation authorized only by the compromised quorum, and is reverted by FM-01 in the same block — it satisfies REQ-172-001 while failing REQ-172-002 and REQ-172-005 outright
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Evaluation Summary

| Evaluator | Lens | Top Proposal | Confidence | Key Finding |
|-----------|------|-------------|------------|-------------|
| Subtractionist | removal | Delete fail-open fallback + reset re-derivation; ~955 lines + 1 module removable | conf(0.68, measured) | FIVE rival "who are the maintainers" impls; `force_remove_maintainer` has 0 production callers → brief §3.5 FALSE |
| Restructurer | boundaries | Move trust-root selection to composition root; anchor convergence in UtxoSet not state root | conf(0.68, measured) | No sync path reconstructs the root — "the single most important wrong boundary"; state-root verdict NO |
| Pattern Matcher | patterns | Adopt in-repo k-of-n (`conditions/eval.rs:51-68`), domain sep, set-version | conf(0.70, measured) | HARD CONTRADICTION: `verify_multisig` counts entries not signers → 3-of-5 is 1-of-5 on all paths |
| Failure Analyst | failures | 18 hard filters (F-1..F-18); disjoint+monotone anchor mandatory | conf(0.70, measured) | FM-01: one free `RemoveMaintainer` resets the whole set to leaked keys in the same block — rotation is self-reverting, not racy |
| Radical Simplifier | minimal | P1: seed-once, read-from-chain, weight-overrules; −2 fns, −80 lines, 0 AH | conf(0.62, observed) | No uncompromised on-chain anchor exists today; import exactly one, once — REQ-172-002 conditional on INC-I-170 |

## Convergence Matrix

Independence verified: each converging evaluator reached the claim from a DISTINCT lens/evidence source
(graph dependents, boundary tracing, anti-pattern scan, adversarial error-path, first-principles).

| Change / Correction | Sub | Res | Pat | Fail | Rad | Count | Verdict |
|---|:--:|:--:|:--:|:--:|:--:|:--:|---|
| Fail-open fallback → fail closed (`verification.rs:66-79`) | Y | Y | Y | Y | Y | **5/5** | DEFINITE |
| Kill the reset button (`periodic.rs:44-72`, FM-01) | Y | Y | (sig) | Y | Y | **4/5** | DEFINITE |
| Wire `run.rs:461` (necessary baseline) | Y | Y | Y | Y | Y | **5/5** | DEFINITE (in F1) |
| Brief §3.5 FALSE — slashing does NOT force-remove maintainers | Y | Y | (A10) | (FM-15 inf) | Y | **2 strong + verified** | CORRECTION |
| Duplicate-signer counter (3-of-5 = 1-of-5) | (sig) | (sig) | Y | Y | (sig) | **1 strong + corrob** | RECOMMENDED |
| No sync reconstruction (wipe/snap reverts) | Y | Y | (gap) | Y | Y | **4/5** | DEFINITE (in F2) |
| State-root inclusion of MaintainerSet: **NO** | (impl) | Y | — | Y (FM-16) | Y | **3/5** | OPTION D (deferred) |
| Advisory-only install (`cmd_upgrade.rs`) | Y | Y | — | (FM-08) | (P3) | **2 strong** | RECOMMENDED |
| Replay domain / domain separation | Y | (flag) | Y | Y | Y | **4/5** | RECOMMENDED |
| Sign `published_at` timing (FM-09/A6) | (V4) | — | Y | Y | — | **2/5** | RECOMMENDED (in F7) |
| Revive `derive_maintainer_set` as replayable | (tension) | (replace) | Y | Y | Y | **4/5** | RECOMMENDED (in F8) |
| Canonical deterministic ordering (FM-03) | (sig) | Y | — | Y | — | **3/5** | DEFINITE (in F2) |
| Format-safety versioned decoder (FM-16) | (serde) | (C-R9) | (unk) | Y | — | **2/5** | RECOMMENDED |
| Weight-hatch is ONLY answer to REQ-172-002, DEAD until INC-I-170 | — | — | Y | Y | Y | **3/5** | OPTION A |
| First delivery irreducibly out-of-band | — | — | Y | Y | Y | **3/5** | Constraint + plan |
| INC-I-170 sequencing dependency | — | (sig) | Y | Y | Y | **3/5** | First-class plan input |

## Definite Changes (High Convergence)

These execute as **Layer 1** (node-local, no AH, rolling-deploy safe) and **Layer 2** (consensus-visible
via `ProtocolActivation`, AH-gated). Both ship together as the "minimum that actually works."

### [F1] ARCHITECTURAL: Fail-closed on-chain trust-root read

Convergence: 5/5 (subtraction P1, restructure P1, patterns P5/A3, failures F-5/F-6, radical P1 #1-2).
Evidence: `bins/node/src/run.rs:461` returns `move || -> Vec<String> { Vec::new() }` (verified verbatim
this session, with the false BLAKE3 TODO at `:457-459` — C14); `crates/updater/src/verification.rs:66-79`
selects `bootstrap_maintainer_keys(network)` (the leaked constants `constants.rs:37-48`) whenever the
list is empty, logged only at `debug!`.
Confidence: conf(0.85, converged).

What changes architecturally: introduce a `TrustRoot { keys, threshold, provenance }` value type resolved
ONCE at the composition root (`bins/node/src/run.rs`), replacing the fail-open `Vec<String>` empty-sentinel
that conflates "no set yet" / "empty set" / "wire not connected" (restructure P1). Wire `run.rs:461` to
return the on-chain `MaintainerSet.members` as that type. **Delete** the `on_chain_keys.is_empty()`
fallback branch (`verification.rs:73-79`): an empty or sub-threshold root **fails verification** (do not
auto-install) — never falls back to the compile-time keys. Surface `provenance` + the empty-set condition
in `getUpdateStatus` at `error!` level. This is the single highest-leverage deletion for the incident: it
removes the one line that keeps `constants.rs:37-48` authoritative.
Consensus classification (INV-12): Q1 (user tx reaches it?) **NO** — no transaction reaches
`verify_release`. Q2 (producer action/attestation?) **NO**. Q3 **N/A** — nothing consensus-visible is
computed. ⇒ **No activation height. Block content: NO. No synchronized deploy. Genesis reset: NO.**
REQ coverage: REQ-172-001 (edge case), REQ-172-005 (preserves un-upgraded nodes), REQ-172-011.
Musts NOT yet satisfied by this layer: 002 (still authorized by the compromised quorum), and the on-chain
set is still the leaked N1–N5 until F2 + Option A.
Failure filters passed: F-5 (fail closed, never to leaked constants), F-6 (rollback parity — read is
stateless), F-18 (subtraction). Exploits radical constraint 4 (`enforcement.rs:170-205` fails open when
`binary_ready==false`, so fail-closed verification cannot halt production).

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  0 (observed)
  Disk:     0 (observed)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: the SSF one-liner — return on-chain hex and keep the sentinel, one line
Why this proposal anyway: the one-liner leaves the fail-open sentinel in place and repairs 1 of 3 call sites; deleting the fallback is what makes an absent or empty root fail closed instead of re-arming the leaked keys
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### [F2] ARCHITECTURAL: Kill the reset button + single canonical replayable derivation

Convergence: 4/5 (subtraction P2, restructure defect B + P4, failures F-3/F-4/F-7 top filter, radical
P1 #3). Structurally verified this session: `force_remove_maintainer` has zero references in `bins/` and
its only non-test `crates/` caller is inside the dead `derive_maintainer_set` (`maintainer.rs:527`).
Evidence: `periodic.rs:44-49` guard is `is_fully_bootstrapped()` = `len>=5`; `:70-72` assigns
`state.set = set` **wholesale**; `apply_block/state_update.rs:214` runs it on **every** applied block,
**after** the governance tx loop at `apply_block/mod.rs:221`. `all_producers()` is HashMap-random
(`set_core.rs:398-400`) + stable sort + genesis `registered_at:0` (`set_registration.rs:192`) ⇒
non-deterministic. `derive_maintainer_set` (`maintainer.rs:490`) — the only replay path — has 0
production callers.
Confidence: conf(0.85, converged). Runtime caveat (failures gap): a FAIL→PASS integration test for FM-01
(apply a lone `RemoveMaintainer`, assert the set returns to the genesis five one block later) is a
**precondition to implementation** — if FM-01 does not reproduce on the target network, this finding's
force weakens.

What changes architecturally: **delete** the re-derivation branch. `maybe_bootstrap_maintainer_set`
becomes a **one-shot genesis seed** guarded by "has this root ever been mutated?", never by `len>=5`
(F-3). Replace the three rival derivations (`periodic.rs:52-64`, `governance.rs:112-124`,
`maintainer.rs:490`) with ONE canonical, totally-ordered pure function in `crates/core::maintainer` that
sorts by `(registered_at, pubkey_bytes)` — no HashMap iteration, no stable-sort ties (F-4, restructure
P4). Preserve `crates/core`'s no-edge-to-`storage` boundary: the function takes a value slice
`&[(PublicKey, u64)]`, never `ProducerInfo` (C-R4). Make the root a replay-complete function of (genesis
seed, all governance actions ≤ H) — revive the replay semantics of `derive_maintainer_set` (REQ-172-010)
so a wiped/backfilled node re-derives the same root instead of re-bootstrapping to N1–N5.
Consensus classification (INV-12): the maintainer set feeds `ProtocolActivation` acceptance
(`governance.rs:80-106`), so changing the derivation changes which governance/activation txs take effect.
Q1 **YES** (`AddMaintainer`/`RemoveMaintainer`/`ProtocolActivation` are user-submittable). Q2 **YES**
(producer `registered_at` is an input). Q3 **NO** (a governance tx that mutated the node-local set before
may be gated differently after). ⇒ **(1|2) YES + (3) NO ⇒ ACTIVATION HEIGHT REQUIRED** — new
`maintainer_derivation_activation_height` in `crates/core/src/network_params/`, a future height (never 0,
never crossed), a **constant gate NOT a HardForkSchedule entry** (CLAUDE.md rolling-deploy rule). Block
content: block bytes unchanged (tx shapes identical) ⇒ **no synchronized deploy** beyond the forward-only
AH; late upgraders keep old node-local behavior until the AH. **Genesis reset: NO.** Pre-condition to
setting the height: verify `ProtocolActivation` has never been exercised on mainnet (restructure gap #3)
so the AH does not rewrite live consensus history.
REQ coverage: REQ-172-005 (fresh-sync/wipe convergence for full-sync/backfill), REQ-172-010, and it is the
structural precondition for REQ-172-001 to be durable.
Musts NOT yet satisfied by this layer: 002 (the seed is still the leaked set until Option A); snap-only
nodes still fail closed until Option D or a stated snap path (F-7).
Failure filters passed: F-3 (reset button removed), F-4 (determinism, proven by a shuffled-input test),
F-7 (snap/wipe convergence stated: full-sync/backfill replay; snap-only fail closed), F-17 (slashing can
no longer reset the root because it is no longer producer-order-derived post-seed).

> **AMENDED 2026-08-10, F2 as SHIPPED vs F2 as PROPOSED (code is the source of truth).**
> Two claims above did not survive implementation and are corrected here rather than left
> to drift:
> * **F-7 is NOT satisfied for the snap path.** Snap-only nodes do **not** fail closed —
>   nothing in `bins/node/src/node/periodic.rs` reads `ChainState::is_snap_synced()`. See the
>   F-7 amendment in *Failure Filters* below. Residual **R3**.
> * **F2's replay claim landed as a pure function only.** `derive_maintainer_set` was revived
>   and is replay-complete, but it has **zero production callers**. The node seeds from LIVE
>   `ProducerSet` state via `derive_canonical_maintainer_set`, so deleting
>   `maintainer_state.bin` on a node with an intact chain re-seeds the root and can re-arm a
>   governance-removed key (measured: M2 QA PROBE-1). REQ-172-010 is **M2 partial**.
>   Residual **R1**.
> * **F-6 ROLLBACK PARITY was likewise open at M2.** M2 makes the root mutable and therefore
>   reorg-exposed; `bins/node/src/node/rollback.rs` had no maintainer-state undo.
>   Residual **R2** — **CLOSED by INC-I-174 M1 (2026-08-11)**: `rollback_one_block` and
>   `execute_reorg` both plan the rewind before purging the height index and commit it after
>   `atomic_replace`, restoring `set` AND `last_derived_height` from a `cf_undo` snapshot, or
>   announcing on `MAINTAINER_REWIND_UNRESTORED` when they cannot. See
>   `bins/node/src/node/maintainer_rewind/`.
>
> None of the three is a regression against pre-M2 behavior, which is why they do not block
> the M2 activation height. All three are scoped in
> `docs/.workflow/inc-i-172-M3-scope.md`; **R2 is now closed, R1 and R3 remain open.**

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      -small; removes a per-block clone-and-sort; +one-time seed sort and one replay when absent (measured)
  Memory:   -small; removes a producer-sized transient Vec per block; +≤5×32B for the root (observed)
  IO:       -small; removes spurious rewrites; +1 replay only when the file is absent, which fresh-sync already pays (measured)
  Network:  0 (observed)
  Disk:     0 (observed)
  Latency:  -small; removes a sort under the producer_set read lock; no change on production or validation (measured)
Inevitability: AVOIDABLE
Cheaper alternative: delete only the dead BlockchainReader and derive_maintainer_set, leaving the two live copies duplicated
Why this proposal anyway: the re-derivation branch is an attacker-triggerable one-tx reset of the trust root; deleting it is both the security fix and a per-block CPU saving, and de-duplicating the derivation stops a future edit silently diverging ProtocolActivation acceptance across nodes
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Recommended Changes (Medium Convergence)

### [F3] ARCHITECTURAL: Distinct-signer k-of-n counter

Convergence: patterns P4 (hard contradiction with the brief, A1), corroborated by radical (constants
`REQUIRED_SIGNATURES=3` vs `MaintainerSet::calculate_threshold` mismatch) and failures ("attacker needs 1
key, not 3"). Structurally verified this session: `verify_multisig` is `.filter(...).count()` with
`valid_count >= self.threshold` — no `HashSet`, no dedup, no per-key `break`.
Evidence: `crates/updater/src/verification.rs:83-123` and `crates/core/src/maintainer.rs:145-159,165-188`
count signature ENTRIES; the correct k-of-n shape (outer loop over expected keys, inner over witnesses,
`break`) is mainnet-live at `crates/core/src/conditions/eval.rs:51-68` since covenant activation h=9150.
Confidence: conf(0.70, converged/measured).

What changes architecturally: replace both entry-counting loops with the covenant distinct-signer shape
(adoption, not invention — patterns C-PAT-3). This makes every "3-of-5" threshold mean 3 distinct signers
on release signing, `AddMaintainer`, `RemoveMaintainer`, and `ProtocolActivation`. Reconcile the two
thresholds (`constants.rs:29` vs `maintainer.rs:125-135`) so a shrunk set cannot lock out the recovery
channel (radical constraint 7 / FM-07).
Consensus classification (INV-12): the **release-verification** counter (`verification.rs`) is
**node-local** — Q1/Q2 NO, Q3 N/A ⇒ no AH (ships in Layer 1). The **governance** counter
(`maintainer.rs:145-159`) is **consensus-adjacent** — it changes which `AddMaintainer`/`ProtocolActivation`
txs take effect: Q1 YES, Q2 YES, Q3 NO ⇒ **AH REQUIRED** (fold into
`maintainer_derivation_activation_height`, Layer 2). Block content: unchanged shape ⇒ no sync deploy.
Genesis reset: NO.
REQ coverage: REQ-172-012, and it is a **precondition for the correctness of** 001, 002, 008, 010.
Honest limit (patterns risk): raising the bar from 1 key to 3 changes NOTHING for the current adversary
(who holds all 5) — it must not be presented as remediation, only as making thresholds mean what they say.
Verify against the live `SIGNATURES.json` before shipping (patterns unknown #5).
Failure filters passed: C-PAT-2 (no authorization through an entry-counting threshold), F-2 (supports
monotone revocation once combined with the deny-list in F7).

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      -small; up to 2 fewer Ed25519 verifies per check, break stops at the first satisfying witness (observed)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  0 (observed)
  Disk:     +4B; a set-version field on the persisted MaintainerSet (inferred)
  Latency:  -small; sub-ms less on the 6-hourly poll (observed)
Inevitability: AVOIDABLE
Cheaper alternative: insert a HashSet<PublicKey> guard in front of the existing counting loops
Why this proposal anyway: the covenant loop at conditions/eval.rs:51-68 is mainnet-proven, allocates nothing, and is less code than adding a set
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### [F4] ARCHITECTURAL: Fail-close `derive_ad_hoc_maintainer_set`

Convergence: restructure C-R5 + P5 sub-change, failures F-16, patterns A9.
Evidence: `bins/node/src/node/apply_block/governance.rs:112-124` — whenever `is_fully_bootstrapped()` is
false, `ProtocolActivation` verification silently reverts to producer-key authority; combined with role
separation (Option C) this is a silent downgrade path back to the compromised keys.
Confidence: conf(0.70, converged).

What changes architecturally: remove the ad-hoc producer-key fallback; when the seeded root is absent or
sub-threshold, `ProtocolActivation` **fails closed** (does not accept) rather than deriving authority from
producers. Severs the consensus-decision-from-node-local-state coupling (F-16).
Consensus classification (INV-12): Q1 YES, Q2 YES, Q3 NO ⇒ **AH REQUIRED** (fold into
`maintainer_derivation_activation_height`, Layer 2). Block content: unchanged ⇒ no sync deploy. Genesis
reset: NO.
REQ coverage: REQ-172-002 (removes a hostile-quorum back-door), REQ-172-005.
Failure filters passed: F-16 (no consensus decision from node-local state), F-5 (fail closed), F-1 (closes
a non-disjoint authorization path).

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  0 (observed)
  Disk:     0 (observed)
  Latency:  0 (observed)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: the fallback is the downgrade path; leaving it lets a hostile actor drive the set sub-threshold and reclaim ProtocolActivation authority through producer keys
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### [F5] ARCHITECTURAL: Versioned, fail-closed MaintainerState decoder

Convergence: failures F-14/P5, restructure C-R9, subtraction serde-default caution.
Evidence: `bins/node/src/run.rs:452-454` `MaintainerState::load(&data_dir).unwrap_or_default()`;
`crates/storage/src/maintainer.rs:44-47` is plain unversioned bincode; `MaintainerState::default()`
(`:29-35`) yields the empty set → threshold 0 → zero-signature `AddMaintainer` accepted (FM-02) AND the
leaked-constants fallback (FM-06), fleet-wide, on any format change.
Confidence: conf(0.70, measured).

What changes architecturally: add a version tag to the persisted struct and a **fail-closed** load path —
an unknown/old format is a loud, defined migration, never a silent empty root. Remove `unwrap_or_default()`
on the trust root. This MUST land **before** any layer adds a field to `MaintainerSet`/`MaintainerState`
(the natural first move of almost every design). **No** `CURRENT_PROTOCOL_VERSION`/`EPOCH_STATE_FORMAT_VERSION`
bump (INV-4/INC-I-054, REQ-172-019). Encoder/decoder index parity if the format is touched.
Consensus classification (INV-12): Q1 NO, Q2 NO, Q3 N/A — the file is node-local, never gossiped, never
hashed (`crates/network/src` has zero `maintainer` refs). ⇒ **No AH, no block content, no sync deploy.
Genesis reset: NO.**
REQ coverage: REQ-172-005 (upgrade-day safety), REQ-172-011 (non-foreclosure of a later field add).
Failure filters passed: F-14 (format safety).

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      +small; one version-tag comparison per node start, once per process lifetime (observed)
  Memory:   +small; a few bytes for a version field in the persisted struct (observed)
  IO:       0 (observed)
  Network:  0 (observed)
  Disk:     +~8B; a version tag per node (observed)
  Latency:  +small; microseconds once at node start (observed)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: adding a field to MaintainerSet today wipes the trust root on every node at the same moment per FM-16; a version tag plus a fail-closed load is already the minimal safe form
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### [F6] ARCHITECTURAL: Enforce verification at `cmd_upgrade` install

Convergence: restructure defect A + C-R7, subtraction (three call sites, one port).
Evidence: `bins/cli/src/cmd_upgrade.rs:82-106` calls `verify_release_signatures(&sig_release,
Network::Mainnet)` — network **hardcoded**, always uses the compiled leaked keys, and **installs
regardless** of the verdict (Err arms only `println!` at `:86-95`; install proceeds at `:106`; comment at
`:70` "informational — never blocks manual upgrade"). `doli upgrade` is the documented INC-I-153
remediation path and stays exposed even after `run.rs:461` is fixed.
Confidence: conf(0.68, converged).

What changes architecturally: route the CLI path through the same `TrustRoot` port (F1); **block** install
on verification failure; derive the network from the argument, not a hardcoded constant. Repairs the 2 of
3 verification sites that F1 alone leaves exposed.
Consensus classification (INV-12): node-local CLI change — Q1/Q2 NO, Q3 N/A ⇒ **No AH, no block content,
no sync deploy. Genesis reset: NO.**
REQ coverage: REQ-172-001, REQ-172-006 (makes the operator-facing path's root explicit).
Failure filters passed: F-5, F-9 (no in-band-only trust on the manual path).

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  0 (observed)
  Disk:     0 (observed)
  Latency:  0 (observed)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: an advisory verify that installs anyway is not a control; leaving it keeps the documented upgrade path pinned to the leaked constants and installing on failure
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### [F7] ARCHITECTURAL: Replay domain + re-verify at install + signed timing

Convergence: patterns P4/CA-2/A6/A8, failures F-10/F-12/F-13/FM-08/FM-09/FM-12, radical P1 #5,
subtraction (V4).
Evidence: `crates/core/src/maintainer.rs:323-326,376-382` bind no domain (`"add:{hex}"`,
`"activate:{v}:{epoch}"`); the signed release message carries no network term, so a testnet signature is a
mainnet authorization wherever that signer is in the resolved array (FM-12). The mainnet/testnet key arrays
were byte-identical when this was written; INC-I-196 made them disjoint, which narrows FM-12 without
closing it — F7 stays open. `crates/updater/src/verification.rs:62` signs only
`"{version}:{binary_sha256}"`; `service.rs:316` uses unsigned `published_at` (`download.rs:206-209`
`.unwrap_or(0)`) to set the veto deadline (FM-09). `service.rs:55` restores a pending update with no
re-verification (FM-08). The domain-separation API already exists at
`crates/crypto/src/signature.rs:269,334` with 6 `_V1` domains at `lib.rs:88-103` — the trust root is the
only signing surface that skipped it.
Confidence: conf(0.70, converged).

What changes architecturally: (a) bind `{network id, genesis hash, target, set-version/height}` into every
governance and release signing message via `sign_with_domain`/`verify_with_domain` under new
`DOLI_MAINTAINER_V1`/`DOLI_RELEASE_V1` domains; reject old-format signatures after activation with a stated
transition (F-13). (b) Re-verify release signatures against the CURRENT root immediately before install
and invalidate any pending update whose signers are no longer trusted (F-10). (c) Sign the timing field
that gates the install decision; never key it off unsigned `published_at` (F-12).
Consensus classification (INV-12): the **governance-message** domain change is consensus-visible (it
changes which txs mutate state): Q1 YES, Q2 YES, Q3 NO ⇒ **AH REQUIRED** (Layer 3,
`maintainer_replay_domain_activation_height`). The **release-verification** re-verify + signed-timing
changes are **node-local**: Q1/Q2 NO, Q3 N/A ⇒ no AH (ship in Layer 1). Block content: unchanged shape ⇒
no sync deploy. Genesis reset: NO.
REQ coverage: REQ-172-012 (primary); closes the cross-network replay that would otherwise defeat
revocation independently of everything else (FM-12).
Failure filters passed: F-10, F-12, F-13.

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      +small; 3 to 5 Ed25519 verifies per rare install event (measured)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  0 (observed)
  Disk:     0 (observed)
  Latency:  +small; single-digit ms on a path that already spends seconds downloading (measured)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: revocation cannot reach an in-flight update without checking the current root at install time, and a testnet rehearsal hands the attacker mainnet-valid authorization without the domain bind
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

### [F8] ARCHITECTURAL: Delete dead veto machinery; revive `derive_maintainer_set`

Convergence: subtraction P3, patterns P5, radical (dead-code signal), failures F-18, restructure
(subtractionist signal).
Evidence: weighted-veto functions (`vote.rs:126-233`, `params.rs` `calculate_vote_weight`/
`seniority_multiplier`/`is_eligible_to_vote`) have callers ONLY in `#[cfg(test)]` (`lib.rs:102` onward);
`is_eligible_to_vote` has zero references repo-wide. Four docs + one log line assert a "7-day
seniority-weighted veto" that never executes (analyst C12/C13) — operators price risk against an absent
control (anti-pattern A5).
Confidence: conf(0.65, converged).

What changes architecturally: **delete** the dead weighted-veto machinery (making the drift impossible to
reintroduce and forcing the docs to tell the truth), BUT **revive** `derive_maintainer_set`
(`maintainer.rs:490`) rather than delete it — it is publicly re-exported (`core/src/lib.rs:258`) and is
the replayable derivation REQ-172-010 needs and F2 depends on (patterns caveat, do NOT apply the deletion
uniformly).
Consensus classification (INV-12): node-local dead-code removal — Q1/Q2 NO, Q3 N/A ⇒ **No AH, no block
content, no sync deploy. Genesis reset: NO.**
REQ coverage: REQ-172-008 (removes the false veto control), REQ-172-010, REQ-172-011, REQ-172-015.
Failure filters passed: F-11 (no veto in either direction), F-18 (subtraction).

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      0 (observed)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  0 (observed)
  Disk:     -small; binary size only, roughly 9 fewer functions (inferred)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: keep the dead functions and only correct the four false docs and the one false log line
Why this proposal anyway: removing nine unreferenced functions costs nothing at runtime and eliminates a documented-but-absent control that operators budget risk against
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Options for User Decision

These are ADDITIONS and consequential choices. They are below conf(0.7), or they are genuine user
decisions (operational cost, sequencing, consensus-adjacent power). Presented as options, not defaults.

### OPTION A — Weight-hatch economic override (the ONLY answer to REQ-172-002)  [low-evidence, conditional]
What: add a second authorization branch in `process_transaction_governance` — an `AddMaintainer`/
`RemoveMaintainer` signed by producers holding ≥ `FINALITY_THRESHOLD_PCT` (67%) of **`active_producers()`**
effective weight is authorized **regardless of the maintainer quorum** (radical P1 #4, patterns P3
UASF/BIP8-LOT). Reuses the existing `producers: &ProducerSet` param and `effective_weight()` — no new
module, no new tx type, no new state store. MUST read `active_producers()` (status-filtered), NEVER
`all_producers()` (radical constraint 5, F-5). MUST be monotone: pair with a revocation deny-list so a
removed key cannot be re-added by any replayed/old-format signature (F-2, CRL pattern).
Evidence: patterns P3 conf(0.5, inferred); radical P1 #4 within conf(0.62) bundle; failures F-1 names it
the only disjoint-ish anchor available.
Confidence: conf(0.60, conditional).
**HARD SEQUENCING DEPENDENCY on INC-I-170**: at the current ~89.85% leaked weight, the hatch is the
ATTACKER's door, not the defender's (patterns kill test FOUND it DEAD as-is; failures F-1 corollary). The
`maintainer_weight_override_activation_height` MUST be set to a height crossed **only after** the INC-I-170
bond-side producer-retirement completes on the target network AND honest weight is re-measured above 67%.
Using `active_producers()` lets the hatch self-heal for free as leaked identities exit. This is a
first-class part of the plan, not a footnote.
Consensus classification: Q1 YES, Q2 YES, Q3 NO ⇒ **AH REQUIRED**, block content unchanged ⇒ no sync
deploy, no HardForkSchedule, no genesis reset.
Failure filters: passes F-1 ONLY under the sequencing precondition; passes F-2 with the deny-list.
Why it is an Option not a Definite: it is the heaviest, most consequential change; it grants a
consensus-adjacent governance power that cannot be un-granted without another AH; and its safety rests on
an unmeasured post-retirement weight split. This is the user's decision, per Rule 18 and
`feedback_gate_three_options`.

### OPTION B — Binary transparency + block-height install delay (REQ-172-008 replacement)  [addition]
What: a release digest must have been announced on-chain at height H; a node installs only when
`current_height ≥ H + RELEASE_DELAY_BLOCKS` (patterns P2, CA-3 timelock). Replaces the dead veto's assumed
"humans get time to react" function with a deterministic, unvotable delay and an adversary-resistant clock
(block height, not attacker-settable `published_at`). Detection + latency, NOT authorization. Creates a
monitoring obligation — the design must name who watches the window.
Evidence: patterns P2 conf(0.6, observed); `conditions/eval.rs:79-81` height-timelock already in-repo.
Confidence: conf(0.60, observed).
Carrier constraint (C-PAT-6): reuse an existing `TxType` discriminant; a NEW discriminant is very likely a
block-content break → AH → fork risk → collides with REQ-172-004 (radical P2 kill test). With an existing
carrier and a node-local install-gate, this stays node-local (no AH). No genesis reset.

### OPTION C — Cold-key role-separation ceremony (REQ-172-009)  [operational decision]
What: rotate the seed set off producer (hot) keys onto fresh offline/cold maintainer keys via an
interleaved `Remove`/`Add` ceremony (restructure P5; radical "role separation is free —
`maintainer.rs:196-212` never checked producer registration"). Reduces FUTURE blast radius (a host
compromise yields producer power, not release-signing power). Does NOT satisfy REQ-172-002 by itself:
`MAX_MAINTAINERS=5`/`MIN_MAINTAINERS=3` force a 10-tx interleave in which the outgoing (compromised) quorum
authorizes every step (C-R8) — front-runnable. Depends on F4 (fail-close `derive_ad_hoc`).
Evidence: restructure P5 conf(0.65, measured).
Confidence: conf(0.65, operational).
User inputs required (REQ-172-009 acceptance): key custody (HSM / air-gap / threshold), signing latency,
quorum availability, and what happens if an offline holder is unreachable during an emergency.

### OPTION D — Deferred consensus-state anchor (non-foreclosure, do NOT ship now)  [deferred]
What: eventually make trust-root divergence fork-detectable. **State-root verdict: NO** (restructure P3,
3/5) — `MaintainerSet` has no canonical encoding (hashing it forks the fleet, C-R1), and the 4th
state-root slot is reserved for `EPOCH_SNAPSHOT_HF` (C-R6). If pursued, prefer the **UtxoSet anchor**
(maintainer slots as UTXOs — already state-root-covered, converges on ALL sync paths including snap, zero
state-root format change; the repo already does this for `ZKRollup` verifying keys). Downgrade risk (C-R3):
a UTXO anchor makes a compromised-key slot-move irreversible on-chain, removing the compile-time recovery
backstop — keep an explicit, logged operator override.
Evidence: restructure P3 conf(0.55, inferred).
Confidence: conf(0.55, inferred). Keep reachable per REQ-172-011; do not build now.

## Constraints (from Failure Analyst — F-1..F-18, MANDATORY gates)

Every candidate — including all Definite/Recommended changes and all Options — must pass these. They are
rejection rules, not tradeoffs.

- **F-1 DISJOINT ANCHOR** — no rotation authorized solely by ≥threshold sigs from current maintainers; a
  producer-weight override is NOT disjoint until INC-I-170 retirement lands (Option A precondition).
- **F-2 MONOTONE REVOCATION** — a revoked key can never return via any new/replayed/old-format signature;
  survives reorg/snap/wipe; no outcome decided by a repeated tx-inclusion race (governance txs are free,
  attacker holds production weight — FM-14).
- **F-3 KILL THE RESET BUTTON** — no re-deriving the root from producer order after any governance action;
  one-shot seed guarded by "ever mutated?", never `len>=5` (F2 satisfies).
- **F-4 DETERMINISM, PROVEN** — pure totally-ordered function of committed chain data; proven by a
  shuffled-input byte-identical-output test (F2 satisfies).
- **F-5 FAIL CLOSED, NEVER TO LEAKED CONSTANTS** — empty/sub-threshold root fails verification (F1
  satisfies); behavior for `len(root) < REQUIRED_SIGNATURES` stated explicitly (F3 reconciles thresholds).
- **F-6 ROLLBACK PARITY** — every trust-root mutation needs a rollback inverse or must derive from state
  rollback already restores (at M2: zero matches in rollback.rs/fork_recovery.rs/block_handling.rs).
  > **SATISFIED 2026-08-11 by INC-I-174 M1 — residual R2 closed.** The rollback inverse now
  > exists. `rollback.rs` and `block_handling.rs` each call
  > `Node::plan_maintainer_rewind` (pure reads, before the height index is purged) and
  > `Node::commit_maintainer_rewind` (mutates, after `atomic_replace`) from
  > `bins/node/src/node/maintainer_rewind/`. One shared pair of functions, deliberately, so
  > the two independent rewind loops cannot drift — that drift is the INC-I-040 precedent.
  > The inverse restores `set` and `last_derived_height` together; restoring the set alone
  > would leave the one-shot seed armed. When no snapshot can undo a rotation in the range,
  > the live root is KEPT and the divergence is announced on `MAINTAINER_REWIND_UNRESTORED`
  > with a `reason=` sub-token that separates "cannot prove" (`block_unreadable`) from
  > "provably diverged". `fork_recovery.rs` needs no match because it owns no rewind loop —
  > every rewind it triggers delegates to `Node::execute_reorg` (`fork_recovery.rs:75`,
  > `:120`), which carries the rewind. Note that the rebuild-from-genesis fallback in
  > `rollback.rs:239-270` replays UTXO and producer state ONLY (`utxo.spend_transaction` /
  > `utxo.add_transaction` then `rebuild_producer_set_from_blocks`) and never calls
  > `process_transaction_governance`, so `AddMaintainer` / `RemoveMaintainer` are NOT
  > replayed there; the maintainer rewind is what covers that path, which is why
  > `plan_maintainer_rewind` is called OUTSIDE the `cached_undo` branch.
- **F-7 SNAP-SYNC/WIPE CONVERGENCE, STATED** — full-sync/backfill replay; snap-only nodes fail closed until
  Option D (F2 states this).
  > **AMENDED 2026-08-10 after M2 shipped — code is the source of truth (CLAUDE.md).** The
  > "snap-only nodes fail closed" half is **NOT implemented and was never implemented**.
  > `maybe_bootstrap_maintainer_set` (`bins/node/src/node/periodic.rs:55-136`) never consults
  > `ChainState::is_snap_synced()` (`crates/storage/src/chain_state.rs:291`), which exists and
  > is available. A snap-synced node seeds the root from the **snapshot's** producer set and
  > never replays governance below the snapshot floor, so it silently holds a
  > plausible-but-possibly-stale root instead of refusing to seed. Worse, M1's containment
  > (`crates/updater/src/trust_root.rs:155`) then marks that stale root usable while refusing
  > the honestly-rotated one.
  >
  > **Deliberately NOT implemented in M2, with reason.** Making snap-synced nodes fail closed
  > on `ProtocolActivation` while full-sync nodes accept it introduces a NEW divergence axis,
  > on a field that is currently inert: `ChainState::serialize_canonical`
  > (`crates/storage/src/chain_state.rs:143-155`) excludes both `active_protocol_version` and
  > `pending_protocol_activation` from the state root, and `is_protocol_active`
  > (`crates/core/src/consensus/constants.rs:19`) has zero production callers. That trade needs
  > its own review and its own evidence; bolting it onto an already-approved consensus layer
  > would invalidate the M2 fork-safety review. Tracked as **R3** in
  > `docs/.workflow/inc-i-172-M3-scope.md`.
  >
  > The full-sync / backfill half of F-7 **does** hold: a full data-dir wipe followed by a full
  > resync from genesis re-executes the whole governance history and converges.
- **F-8 OUT-OF-BAND FIRST DELIVERY, NEW ANCHOR** — see First-Delivery Plan; must not depend on
  `is_newer_version`/`/releases/latest` (attacker-pinnable).
- **F-9 NO IN-BAND-ONLY TRUST** — reproducible build + independently published digest, or nothing.
- **F-10 REVOCATION REACHES IN-FLIGHT UPDATES** — re-verify against the current root at install (F7).
- **F-11 NO VETO, EITHER DIRECTION** — the veto is not a control and the recovery release is not placed
  behind a veto tally (F8 deletes the dead machinery; Option B replaces the function).
- **F-12 SIGN EVERYTHING THAT GATES A DECISION** — unsigned `published_at` must not set timing (F7).
- **F-13 REPLAY DOMAIN** — bind network id/genesis hash/target/nonce on every message (F7).
- **F-14 FORMAT SAFETY** — versioned fail-closed decoder; no `unwrap_or_default` on a trust root; no
  protocol/epoch-format version bump (F5 satisfies).
- **F-15 CONSENSUS-VISIBLE ELEMENTS ARE AH-GATED** — every consensus-visible change names an AH; constant
  gate, never HardForkSchedule (F2/F3-governance/F4/F7-governance/Option A satisfy).
- **F-16 NO CONSENSUS DECISION FROM NODE-LOCAL STATE** — `ProtocolActivation` must not read node-local
  state; sever or move into consensus state (F4 satisfies; Option D is the eventual full fix).
- **F-17 SLASHING MUST NOT MUTATE THE SIGNING ROOT** — role separation as a safety property (F2 satisfies
  by decoupling the root from producer order).
- **F-18 SUBTRACTION CHECK** — before adding, check whether a deletion removes the failure (F1/F2/F8 are
  deletions that remove FM-01/FM-06 outright).

## First-Delivery Bootstrap Plan (REQ-172-006 — irreducibly out-of-band)

**Trust bootstrapping cannot re-root a hierarchy using only channels that hierarchy secures** (patterns
C-PAT-1, failures F-8/F-9). Every matched industry pattern (TUF root rotation, CA rollover, distro
keyring, PGP) ends in an out-of-band step. This is the irreducible core of the problem, not a gap. The
compromised channel will also deliver the honest fix — so REQ-172-006 is a **race-and-residue** problem,
not a delivery problem. Three tiers, zero new channels:

- **Tier 1 — the ~15 operator-controlled nodes (N1–N12 + 3 seeds):** build the trust-root-fixing binary
  from reviewed source on the operator's own machine (memory: "compile only on ai2") and install that.
  Trusts git history, not the leaked keys, not the release assets. Cost: 0 new machinery.
- **Tier 2 — the ~20–30 external auto-update nodes:** publish through the normal channel; the leak cuts
  both ways so the honest release installs automatically, and on install the door closes (constants
  demoted to seed-only). Surviving verification factor: **write access to `doli-network/doli` releases**
  (the INC-I-157 origin invariant, `constants.rs:132-138`). Publish the commit SHA + `CHECKSUMS` hash on
  the explorer and in release notes as an independent comparison anchor. **Unverified gap** (radical #4):
  who holds GitHub org write access — if org access is also compromised, this tier's factor collapses.
- **Tier 3 — nodes that never update:** they keep the old (leaked) root permanently; residual protection
  is Tier 2's origin factor and nothing else. State this plainly — it is a real, permanent residue no
  design removes.

**Unremovable residual risk (all proposals):** the front-running race — an attacker holding the same keys
can publish a competing release as fast as the honest side. The only surviving factor is control of write
access to the release origin. No design closes this; any claim otherwise is wrong.

## INC-I-170 Sequencing Dependency (first-class plan input)

Three evaluators converged (patterns C-PAT-5, failures F-1 corollary + sequencing signal, radical
precondition) that the bond-side and update-side remediations are **mutually dependent and must be
sequenced deliberately, not run in parallel**:
1. **Bond retirement must land FIRST** for Option A (weight-hatch) to be honest-controlled. At ~89.85%
   leaked selection weight, the override is the attacker's door until the leaked identities exit
   `active_producers()`.
2. **Retirement alone does NOT revoke maintainer power** (analyst F5, verified: `all_producers()` is
   status-unfiltered, `set_core.rs:398-400`; bootstrap takes first-5 by `registered_at`). Worse, retiring
   N1–N5 *strengthens* FM-01's reset while it still exists — so F2 (kill-reset) must land before or with
   retirement.
Recommended order: **F1+F5+F6+F7(node-local) (Layer 1) → F2+F3(gov)+F4+F7(gov) (Layer 2, one AH) →
[INC-I-170 retirement completes; re-measure honest weight] → Option A (Layer 3, weight-override AH) →
Option B/C as chosen.** Option A before retirement arms the attacker.

## Architecture Maps

### Current Architecture

```
GitHub release ──► download.rs (CHECKSUMS.txt + SIGNATURES.json) ──► service.rs:221
                                                                        │
   maintainer_keys_fn() ──► Vec::new()  ◄── THE GAP (run.rs:461)        │
                                                                        ▼
   verification.rs:66-79: on_chain empty ⇒ bootstrap_maintainer_keys(network)
                          = 5 LEAKED compile-time constants (constants.rs:37-48)
                          counter = .filter().count()  ⇒ effective threshold 1
                                                                        ▼
   veto (300s, node-local, unweighted — DEAD control) ──► auto_apply ──► install ──► restart

Governance:  AddMaintainer/RemoveMaintainer/ProtocolActivation (structural-only validation)
   ──► governance.rs (warn!+skip on sig failure) ──► maintainer_state.bin (NODE-LOCAL, not state root)
   ──► periodic.rs:44-72 re-bootstrap EVERY block: len<5 ⇒ wholesale replace with first-5 producers
       = the leaked N1–N5  (FM-01 reset button)
   Consumers: ProtocolActivation (consensus) · release verify (intended) · RPC · slashing force-remove
   Sync:  crates/network/src has ZERO maintainer refs ⇒ wipe/snap ⇒ default() ⇒ re-bootstrap ⇒ N1–N5
```

### Proposed Architecture (Definite + Recommended)

```
GitHub release ──► download.rs ──► service.rs / cmd_upgrade (BOTH enforce)
                                        │
   resolve_trust_root(&MaintainerState, network) ──► TrustRoot{ keys, threshold, provenance }
                                        │  (F1: no empty-sentinel; empty/sub-threshold ⇒ FAIL CLOSED)
                                        ▼
   verify_release(&Release, &TrustRoot): distinct-signer k-of-n (F3), domain-tagged (F7),
      re-verified against CURRENT root at install (F7)  ──► install
   NO constants fallback path.  Timing signed, not published_at (F7).

Governance (AH-gated, Layer 2):
   ONE canonical derive_bootstrap_set(&[(PublicKey,u64)], height) — total order, no ties (F2)
   maybe_bootstrap = ONE-SHOT genesis seed, "ever mutated?" guard (F2)  ── reset button DELETED
   replay-complete: root = f(seed, all governance actions ≤ H) (F2, revived derive_maintainer_set F8)
   ProtocolActivation FAILS CLOSED when unbootstrapped (F4) — no producer-key downgrade
   MaintainerState: versioned, fail-closed decoder (F5)
   Sync: wipe/backfill re-derive same root; snap-only fail closed until Option D
```

> **AS-SHIPPED DELTA (2026-08-10, M2).** The `Governance (AH-gated, Layer 2)` block above is
> the PROPOSAL. Three lines of it are not what the code does:
> * `replay-complete: root = f(seed, all governance actions ≤ H)` — the FUNCTION exists
>   (`derive_maintainer_set`) but has zero production callers; the node seeds from live
>   `ProducerSet` state. Residual **R1**.
> * `Sync: … snap-only fail closed` — snap-only nodes do NOT fail closed; they seed from the
>   snapshot's producer set. Residual **R3**.
> * Rollback parity (F-6) was unimplemented at M2: a governance mutation from a reorged-out
>   block persisted above the gate. Residual **R2** — **CLOSED by INC-I-174 M1
>   (2026-08-11)**; the line now IS what the code does. See F-6 in *Failure Filters*.
>
> The rest of the block — one canonical totally-ordered derivation, one-shot seed,
> `ProtocolActivation` fail-close, versioned fail-closed decoder — shipped as drawn.

## Migration Path

Preserve existing behavior during transition. Un-upgraded nodes keep verifying with the compiled keys
until they take the new binary; upgraded nodes with an unbootstrapped set also use the compiled keys, so
there is **no behavior change until the on-chain set is authoritative** (restructure P1 second kill test).

1. **Milestone 1 (Layer 1, node-local, rolling-safe):** F1 + F5 + F6 + F7(node-local: re-verify at
   install, signed timing) + F3(release path). No AH, no sync deploy. Ship as an ordinary binary upgrade.
   - `BRIDGE:` while F2 has not yet activated, `maybe_bootstrap_maintainer_set` still re-derives — so the
     on-chain set remains the leaked N1–N5 and rotation is not yet durable. This is a TEMPORARY state,
     acknowledged: Layer 1 hardens the read path (fail-closed, enforced install, fixed release counter) but
     rotation only becomes durable when Layer 2 ships. Remove this bridge when F2 activates.
2. **Milestone 2 (Layer 2, AH-gated):** F2 + F3(governance) + F4 + F7(governance domain) under ONE new
   `maintainer_derivation_activation_height` (constant gate, future height, NOT HardForkSchedule). Pre-req:
   FAIL→PASS FM-01 and FM-02 reproduction tests (failures mandate); verify `ProtocolActivation` never used
   on mainnet before choosing the height. Rolling deploy — no synchronized stop; late upgraders converge at
   the AH.
3. **Milestone 3 (INC-I-170):** complete bond-side producer-retirement; **re-measure** honest
   `active_producers()` weight; confirm > 67%.
4. **Milestone 4 (Option A, AH-gated) — user-gated:** weight-hatch under
   `maintainer_weight_override_activation_height`, set to a height crossed only after Milestone 3. Adds the
   deny-list (F-2). This is the layer that actually satisfies REQ-172-002.
5. **Milestone 5 (Options B/C/D) — as chosen:** transparency+timelock, cold-key ceremony, deferred anchor.

## Complexity Comparison

Counting moving parts (radical Simplifier's floor is P1). Lower is better unless a Must is violated.

| Metric | Current | Radical Minimum (P1) | Proposed (Definite+Recommended) | +Option A |
|--------|--------|----------------------|-----------------------------|-----------|
| Trust-key stores | 2 (constants effective, on-chain inert) | 1 on-chain + 1 seed | 1 on-chain + 1 seed | 1 on-chain + 1 seed |
| Trust-key accessor fns | 4 | 2 | 2 | 2 |
| Derivation paths | 3 (1 dead) | 1 (seed + replay) | 1 (canonical, replay-complete) | 1 |
| Authorization models | 1 (compromised) | 2 (quorum + weight) | 1 (quorum, hardened) | 2 (quorum + weight) |
| New tx types | — | 0 | 0 | 0 |
| New modules | — | 0 | 0 | 0 |
| Activation heights | — | 0 | 1 (derivation) | 2 (+ weight-override) |
| Synchronized deploys | — | 0 | 0 | 0 |
| Genesis resets | — | 0 | **0** | **0** |
| Dead functions | ≥9 | −2 | −9 (revive derive_maintainer_set) | −9 |
| Net lines | — | −~80 +35 | −~200 +~120 | +~35 |
| Effective threshold | 1 (counter bug) | 1 | **3 distinct signers** | 3 |
| Rotatable without redeploy | **NO** | YES | YES | YES |
| REQ-172-002 (hostile quorum) | NO | conditional | NO | **YES (post-retirement)** |

## Milestones

Touches 6+ modules (`updater`, `core/maintainer`, `storage/maintainer`, `bins/node/{run,periodic,
apply_block/governance}`, `bins/cli`, `crates/rpc`) — milestones defined above (Migration Path M1–M5).
M1 and M2 are the "minimum that actually works"; M3–M5 satisfy REQ-172-002 and the Should/Could reqs.

## Corrections to the Architect Prompt / Brief

Every disproved claim, with `file:line` and the evaluators that disproved it:

| # | Disproved claim | Verdict | Evidence | Disproved by |
|---|---|---|---|---|
| 1 | Brief §3.5: "slashing force-removes maintainers, mutating the update trust root" | **FALSE** | `force_remove_maintainer` (`maintainer.rs:242`) has ZERO references in `bins/`; its only non-test `crates/` caller is `:527` inside the dead `derive_maintainer_set` (0 graph dependents); doc `maintainer.rs:28` also false. Verified this session. | restructure, subtraction, + synthesizer verification |
| 2 | Brief capability inventory: "the system does not lack a multisig verifier" | **FALSE** | `verify_multisig` (`maintainer.rs:145-159`) and `verification.rs:83-123` count signature ENTRIES via `.filter().count()`, not distinct signers — no HashSet/dedup/break. Effective threshold is **1**, not 3. Verified this session. Correct shape exists at `conditions/eval.rs:51-68`. | patterns (hard contradiction), + synthesizer verification |
| 3 | "Wiring `run.rs:461` alone makes the leaked constants stop mattering" | **INSUFFICIENT** | 2 further live call sites (`cmd_upgrade.rs:82`, `commands/update.rs:384`) use the constants; FM-01 (`periodic.rs:44-72`) reverts any rotation in the same block; `run.rs:457-459` TODO's BLAKE3 blocker is FALSE (C14 — `ProducerInfo.public_key` is raw Ed25519). | subtraction, restructure, failures, radical |
| 4 | Analyst F2 residual: `assert_production_keys` is the production gate | **FALSE** | `assert_production_keys` has 0 callers; the live guard is `is_using_placeholder_keys` at `startup.rs:9,16`. It only rejects a `"00000000"` prefix — cannot detect the leaked keys (anti-pattern A11). | subtraction, restructure, radical |
| 5 | Veto is a usable control (any direction) | **FALSE** | 300s window (not 7 days — 4 false docs + log at `service.rs:227`), node-local unweighted tally, symmetric (blocks honest recovery too), plus V4: attacker sets the deadline via unsigned `published_at` (FM-09). Seniority weighting is dead code (C13). | failures, all (analyst confirmed) |
| 6 | Threat model: "attacker needs ≥3 maintainer keys" | **WRONG by a factor** | Because of the entry-counting bug, the attacker needs **1** key; because all 5 are public, difficulty is 0. | patterns |

## Design Synthesis Quality Gate

```
━━━ DESIGN SYNTHESIS QUALITY GATE ━━━
Evaluators completed:             5/5
Deletion convergence items:       3 at 4+/5 (fail-open fallback 5/5, kill-reset 4/5, dead-veto 4/5)
Restructuring convergence:        2 (canonical derivation 3/5, advisory-install 2 strong)
Addition options presented:       4 (weight-hatch, transparency+timelock, cold-key ceremony, deferred anchor)
Failure modes identified:         16 (FM-01..FM-16) + 18 filters (F-1..F-18)
Failure modes applied as filters: 18/18 (every Definite/Recommended/Option checked)
Radical floor gap:                current 3 derivation paths / threshold 1 → radical min (P1: 1 path, 0 AH)
                                  → proposed (1 canonical path, 1 AH, threshold 3, REQ-172-002 via Option A)
Contradictions found:             2 (brief §3.5 slashing; brief "has a multisig verifier") — both resolved
Contradictions resolved:          2/2 (structurally verified this session)
Evidence independence verified:   YES (5 distinct lenses; convergence cross-checked against code graph + reads)
Genesis reset (every layer):      NO
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```
