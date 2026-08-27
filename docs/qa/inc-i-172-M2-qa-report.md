# QA Report — INC-I-172 M2 (maintainer trust-root, Layer 2)

run_id=508 · incident INC-I-172 · milestone M2 · workflow `redesign` · 2026-08-10

## Scope Validated

`crates/core/src/maintainer/` (mod/set/derivation/data), `crates/core/src/network_params/`
(mod/defaults/env_loader), `bins/node/src/node/periodic.rs`,
`bins/node/src/node/apply_block/governance.rs`, and their interaction with M1
(`crates/updater/src/trust_root.rs`, `crates/storage/src/maintainer.rs`).

No git state was modified. One temporary probe file was created in `bins/node/tests/`,
run, and deleted; the tree is as I found it.

## Summary

**CONDITIONAL APPROVAL.** The four gated behaviors (F2 one-shot seed, F2 canonical
derivation, F3 distinct-signer counter, F4 fail-close) are correctly implemented,
correctly gated, and correctly classified for a rolling deploy. Pre-activation parity
holds: I read the diff and the gating code and found no path where post-activation
behavior leaks below the height. All 41 M2 assertions pass; `doli-core` (976 lib + 17
integration binaries) and `doli-node` (except the known fd flake) are green; build,
clippy `-D warnings` and `fmt --check` are clean.

Two things are wrong, both outside the fork-risk surface. (1) `docs/auto_update_system.md`
now asserts a wiped node replays to the same root — the code does the opposite, and I
have a passing probe that shows a removed maintainer key coming back. (2) The spec's
stated snap-sync behavior ("fail closed") is not implemented; a snap-synced node
silently seeds a plausible-but-wrong root. Neither is a regression against pre-M2
behavior and neither can fork the chain, but both must be resolved before merge because
they misstate the security posture of the exact incident this work exists to close.

## System Entrypoint

Library/consensus change with no runnable-service surface of its own. Validated by
execution, not by reading:

```
cargo build                                        # Finished, rc=0
cargo clippy --workspace --all-targets -- -D warnings   # Finished, rc=0
cargo fmt --check                                  # clean (no output)
cargo test -p doli-core --no-fail-fast             # 976 lib pass; 17 int binaries, 0 failed
cargo test -p doli-node --no-fail-fast             # 1 failure: test_cluster_10x100 (known env flake)
cargo test -p doli-node --test test_network test_cluster_10x100  # 1 passed, 13.11s — confirmed env-only
```

## Acceptance Criteria Results

### AC-1 — REQ-172-010 replayability (Should) — **FAIL** (non-blocking, MoSCoW Should)

| Sub-claim | Result | Evidence |
|---|---|---|
| Two nodes, identical block history → identical set | PASS (function) | `crates/core/tests/inc_i_172_m2_canonical_derivation.rs:454` passes |
| Wiped `maintainer_state.bin` re-derives the SAME root | **FAIL (node)** | PROBE-1 below |
| `derive_maintainer_set` revived, not deleted | PASS | `crates/core/src/maintainer/derivation.rs:107` |

⚠ CONTRADICTION: I expected `wiped_node_replay_converges_with_a_node_that_was_online_throughout`
(`inc_i_172_m2_canonical_derivation.rs:512`) to prove the node property. It does not.
It exercises `derive_maintainer_set`, and `derive_maintainer_set` has **zero production
callers** — verified by `grep -rn "derive_maintainer_set" bins crates --include="*.rs"`,
which returns only `crates/core/src/lib.rs:258` (a re-export) and test files. The node's
real path is `periodic.rs:87-108` → `derive_canonical_maintainer_set` over **live
`ProducerSet` state**, not block history. The passing test is true of a function nothing
calls; it is not evidence about any node.

REQ-172-010's own first criterion (`docs/redesigns/maintainer-trust-root-redesign-analysis.md:466`)
is *"derivable by any node from block history alone, **without trusting `maintainer_state.bin`**"*.
The node trusts `maintainer_state.bin` and, when it is absent, substitutes live producer
state. REQ-172-010 is a **Should**, so this is reported and tracked, not blocking.

### AC-2 — REQ-172-005 behavior preservation / fork safety (Must) — **PASS** on parity, **CONCERN** on convergence

**Pre-activation parity: PASS. I verified this against the diff, not against the tests.**

`git diff HEAD -- bins/node/src/node/periodic.rs` shows the `height < AH` branch is the
pre-M2 code verbatim: guard `is_fully_bootstrapped()` (`periodic.rs:156`), then
`all_producers()` (HashMap order) + stable `sort_by_key(registered_at)` + `take(5)`
(`periodic.rs:101-107`). `governance.rs:134-137` reproduces the old
`is_fully_bootstrapped()`-or-`derive_ad_hoc_maintainer_set` selection exactly, and
`derive_ad_hoc_maintainer_set` (`governance.rs:163-175`) is byte-unchanged.

**Is there ANY path where new behavior leaks below the height?** I enumerated every
production consumer of the multisig verifiers:

```
bins/node/src/node/apply_block/governance.rs:37   verify_multisig_at            (gated)
bins/node/src/node/apply_block/governance.rs:72   verify_multisig_excluding_at  (gated)
bins/node/src/node/apply_block/governance.rs:141  verify_multisig_at            (gated)
```

No ungated `verify_multisig` / `verify_multisig_excluding` exists outside
`crates/core/src/maintainer/` and tests. Every gate is `height >= activation_height`
(`set.rs:269`, `set.rs:285`, `governance.rs:108`, `periodic.rs:70`) — one comparison
form, no `>` anywhere. `activation_height` is read from
`self.config.network.params()` at the two node call sites and passed as a plain `u64`;
the height is the chain-derived block height (INV-SYNC-012 respected).

**The one ungated change — I attacked the developer's argument and it is incomplete,
but the conclusion survives for an independent reason.**

The argument in dev-notes §3 is that `is_authorizable()` cannot change any
consensus-visible outcome below the gate because the only consensus-visible consumer,
`ProtocolActivation`, never sees an empty set (it substitutes
`derive_ad_hoc_maintainer_set`). **That argument misses an indirect path.** Below the
gate, on a chain with fewer than 5 producers the seed never fires (`periodic.rs:83-85`),
so the on-chain set stays empty. Pre-M2 an empty set had `threshold == 0` and
`valid_count >= 0` was vacuous, so a **zero-signature `AddMaintainer` was accepted**
(`governance.rs:37` → legacy verifier). That leaves `members = [attacker]`,
`threshold = 1` — and the attacker's own key now satisfies the next add. Four more adds
reach 5 members, at which point `is_fully_bootstrapped()` is true and
`governance.rs:135` **does** use the attacker's set for `ProtocolActivation`. So the
empty set *is* reachable by the consensus-visible consumer, transitively.

The conclusion nevertheless holds, for a reason the notes do not state:

1. `ChainState::serialize_canonical()` (`crates/storage/src/chain_state.rs:143-155`) is a
   fixed 140-byte buffer of `best_hash | best_height | best_slot | total_work |
   genesis_hash | genesis_timestamp | last_registration_hash | registration_sequence |
   total_minted`. It contains **neither** `active_protocol_version` **nor**
   `pending_protocol_activation`. The state root is
   `H(H(chain_state) || H(utxo) || H(producer_set))` (`crates/storage/src/snapshot.rs:17`),
   so a ProtocolActivation accept/reject divergence **cannot** move the state root.
2. `is_protocol_active` (`crates/core/src/consensus/constants.rs:19`) has **zero**
   production callers — grep returns only the re-export, its own doc comment, and
   `crates/core/src/consensus/tests.rs`. `active_protocol_version` currently gates
   nothing.
3. `process_transaction_governance` never rejects a block; on failure it only `warn!`s
   (`governance.rs:58`, `:94`, `:148`). Block validity is untouched.

Net: the ungated `is_authorizable()` is safe to ship, and gating it would have left
FM-02 live until h=172_000 on exactly the nodes most likely to be empty. **But the
stated rationale should be corrected** — a future reader who wires `is_protocol_active`
into a consensus rule will inherit an argument that no longer holds.

**Convergence half (REQ-172-005 criteria at analysis:437-440):**

| Criterion | Result |
|---|---|
| Old binary still verifies legitimate releases | PASS — old binaries never see the new field |
| Fresh-sync from genesis ≡ always-online node | PASS on membership; member ORDER may still differ (both seed below the AH through the frozen HashMap path) |
| Wiped data dir + full resync converges | PASS (full replay re-executes governance) |
| Wiped `maintainer_state.bin` only, chain intact | **FAIL** — PROBE-1 |
| Design states the snapshot-sync path | Design states it (`specs/…architecture.md` §F2, F-7: *"snap-only nodes still fail closed"*); **implementation does not do it** |

### AC-3 — REQ-172-012 / AUDIT-P0-010, N entries from ONE key (Must) — **PASS**

`count_distinct_signers` (`crates/core/src/maintainer/set.rs:130-149`) loops the
**member** list outer and `break`s on the first matching signature, so each member
contributes at most 1. It is the mainnet-live `Condition::Multisig` shape, adopted not
invented. All three tx types route through it above the gate.
Green: `three_entries_from_one_key_must_not_satisfy_threshold_three`,
`three_entries_from_one_key_must_not_authorize_a_removal`,
`protocol_activation_from_one_key_signing_three_times_is_rejected_above_the_gate`.

### AC-4 — AUDIT-P1-010 / FM-02, zero-threshold (Must) — **PASS**

`calculate_threshold(0)` → `MAINTAINER_THRESHOLD` (`set.rs:108`) and every one of the
four verifiers short-circuits on `!is_authorizable()` (`set.rs:165, 182, 203, 231`).
`is_authorizable` = `!members.is_empty() && threshold >= 1 && members.len() >= threshold`
(`set.rs:89`). Note `calculate_threshold(n) <= n` for all `n >= 1`, so the third term is
only reachable for a hand-edited/persisted set — a defensible belt-and-braces.

### AC-5 — AUDIT-P1-013 / FM-01, the reset button (Must) — **PASS above the gate, with a residual**

`maintainer_seed_is_done` (`periodic.rs:152-158`) is one-shot above the gate:
`!members.is_empty() || last_derived_height != 0`. `above_the_gate_a_governance_removal_survives_the_next_block`
and `a_removed_maintainer_must_not_return_on_the_next_block` are green.
Residual: the guard is a function of `maintainer_state.bin` only, so deleting that file
re-arms the reset (PROBE-1). Also, the per-block call at
`apply_block/state_update.rs:214` is retained — correct, since it must still fire below
the gate, and it is a cheap early return above it.

### AC-6 — REQ-172-002 ProtocolActivation fails closed (Must) — **PASS**

`governance.rs:108-128`: above the gate, `on_chain.unwrap_or_default()` (so a `None`
`maintainer_state` yields an empty set), then `!set.is_authorizable()` → `warn!` naming
member count, threshold and the gate → `return None`. `derive_ad_hoc_maintainer_set` is
unreachable at or above the gate. Green:
`protocol_activation_fails_closed_above_the_gate_when_the_root_is_unbootstrapped`.
Correctly scoped: this closes the back-door; the positive property (rotation despite a
hostile quorum) remains open and is honestly marked "M2 partial" at analysis:517.

### AC-7 — Rolling-deploy safety, INV-12 / INC-I-062 (Must) — **PASS**

| Check | Result |
|---|---|
| Block CONTENT unchanged | PASS — no tx shape, coinbase, tx ordering, bitfield, `presence_root` or header field is touched by the diff |
| State root unchanged | PASS — maintainer state is absent from `snapshot.rs`; positive control: the same file names `chain_state`, `utxo_set`, `producer_set` 3×. `maintainer_state.bin` is node-local |
| `HardForkSchedule` entry added | NO — `git diff HEAD -- crates/updater/src/hardfork.rs` is empty |
| `CURRENT_PROTOCOL_VERSION` | 8, unmoved (`crates/network/src/protocols/status.rs:49`) |
| `EPOCH_STATE_FORMAT_VERSION` | 1, unmoved (`status.rs:68`) |
| `MIN_PEER_PROTOCOL_VERSION` | 1, unmoved (`status.rs:83`) |
| `MAINTAINER_STATE_VERSION` | 1, unmoved (`crates/storage/src/maintainer.rs:53`) |
| `Cargo.toml` version | 6.24.1, unmoved (only the `repository` URL changed, unrelated) |

No synchronized fleet stop is required. A mixed fleet across the AH cannot fork: the only
divergence is the node-local install trust root.

### AC-8 — Activation-height sanity (Must) — **PASS**, one CONCERN

mainnet `172_000` (`defaults.rs:264`), testnet `127_200` (`:432`), devnet `0` (`:573`);
env override `DOLI_MAINTAINER_DERIVATION_ACTIVATION_HEIGHT` LOCKED on mainnet
(`env_loader.rs:429-436`). Constant gate, not a `HardForkSchedule` entry. Devnet 0 is
justified — `chainspec.mainnet.json` and `chainspec.testnet.json` both carry
`genesis_producers: []`, and devnet takes a fresh genesis every run, so there is no
history to reinterpret. `mainnet_gate_is_pinned_in_the_future_and_is_not_a_no_op`
asserts `> 162_727`.

CONCERN: 172_000 − 162_727 = 9,273 blocks ≈ **25.8 h** of lead **measured from the pin
time (2026-08-10)**, not from release time. This code is still uncommitted and
unreviewed. With ~20–30 external auto-update producers, 25.8 h is not enough runway for
a release to reach the fleet. Re-pinning forward is still legal *only because the height
has not been crossed* (INC-I-054 immutability starts at the crossing). **Re-verify the
live tip immediately before release and re-pin if the margin has closed.**

## Exploratory Testing Findings

Probes were written as a temporary `bins/node/tests/` file, executed, and deleted.
Raw output:

```
PROBE-3 at_gate_is_canonical=true  below_gate_len=5
PROBE-2 legacy_eq_after=true  canonical_eq_after=false  same_membership_ignoring_order=false
PROBE-1 file_existed=true  after_wipe_len=5  removed_key_back=true
test result: ok. 3 passed; 0 failed
```

| # | What was tried | Expected | Actual | Severity |
|---|---|---|---|---|
| 1 | Seed 5 above the gate, apply a legitimate 3-distinct-signer `RemoveMaintainer` (→4), delete `maintainer_state.bin` + reset the in-memory root, then apply one more block | The root replays to 4 from block history | **Root returns to 5; the removed key is back** (`removed_key_back=true`). `maintainer_seed_is_done` sees `members.is_empty() && last_derived_height == 0` → re-seeds from **live producer state** at the current height | **high** |
| 2 | 8 producers tied at `registered_at == 0`; seed at height 1 (below gate), then call at exactly the gate and at 200_000 | The set converges to the canonical order once past the gate | **Frozen at the legacy HashMap-ordered set forever.** With >5 tied producers even the MEMBERSHIP differs from canonical (`same_membership_ignoring_order=false`). The canonical derivation never runs on a node that seeded pre-AH | medium |
| 3 | First seed at exactly `height == AH`, and at `AH - 1` | `>=` semantics, canonical at the boundary | Correct — canonical at `AH`, legacy at `AH-1`. No off-by-one | none (PASS) |
| 4 | Reorg across the gate (static analysis) | A reorged-out `RemoveMaintainer` is undone | **Not undone.** `grep -i maintainer bins/node/src/node/rollback.rs` → 0 hits; positive control: the same file names `producer_set\|utxo` 21×. Above the gate a governance mutation from an orphaned block persists permanently, so two honest nodes on the same final chain can hold different roots depending on which forks they saw. Below the gate the per-block re-seed masked this; M2 makes the root mutable and therefore reorg-exposed | medium |
| 5 | Snap sync past the gate | Fail closed, per spec §F2 / F-7 | **Does not fail closed.** `maybe_bootstrap_maintainer_set` never consults `ChainState::is_snap_synced()` (`crates/storage/src/chain_state.rs:291`), which exists and is available. A snap-synced node seeds from the snapshot's producer set and never replays governance below the floor — it silently holds the genesis five while the fleet holds the rotated set. Worse, M1's containment then marks the *stale* root usable and the *honest* rotated root refused | **high** |
| 6 | Duplicate entries in `derive_canonical_maintainer_set` (same `registered_at` AND same pubkey) | Total order, one seat per key | Correct. `seated_registrations` (`derivation.rs:50-68`) sorts on `(registered_at, pubkey_bytes)` — total, since distinct keys never compare equal — and de-duplicates before seating. `canonical_derivation_does_not_seat_a_duplicate_twice` covers it | none (PASS) |
| 7 | M1 × M2 interaction after a legitimate rotation above the gate | Rotation works end to end | `TrustRoot::resolve` (`crates/updater/src/trust_root.rs:155`) still refuses any set that is not the compiled bootstrap five, returning `on_chain(vec![], threshold)` → `is_usable()` false. Pre-M2 the rotation self-reverted in one block so this never latched; post-M2 **the first successful rotation permanently disables auto-update fleet-wide** until M1's containment is lifted. Fail-closed, so not a security hole — but an operational trap. Dev-notes §7.1 acknowledges it; it should be a release-note line, not a follow-up | medium |
| 8 | `derive_maintainer_set` vs `derive_canonical_maintainer_set` `last_updated` | Same root from both paths | They differ: `derive_maintainer_set` stamps `max(registered_at)` of the seated five (`derivation.rs:117`), the node stamps the current block height (`periodic.rs:117`). Members and threshold agree. Latent only — `derive_maintainer_set` has no production callers | low |

## Failure Mode Validation

| Scenario | Triggered | Detected | Recovered | Degraded OK | Notes |
|---|---|---|---|---|---|
| FM-01 reset button (per-block revert) | Yes | Yes | Yes | Yes | Above the gate the removal survives; below it, deliberately preserved |
| FM-01 via `maintainer_state.bin` deletion | Yes | **No** | **No** | **No** | PROBE-1 — silent, no warning |
| FM-02 zero-threshold empty set | Yes | Yes | Yes | Yes | `is_authorizable`, ungated |
| AUDIT-P0-010 duplicate-signature quorum | Yes | Yes | Yes | Yes | Distinct-signer counter |
| Unbootstrapped root at activation time | Yes | Yes | n/a | Yes | Fail-closed with a `warn!` naming count/threshold/gate |
| Snap-sync trust-root acquisition | Yes | **No** | **No** | **No** | Finding 5 — silent divergence, not fail-closed |
| Reorg across the gate | Static | **No** | **No** | Unknown | Finding 4 — no rollback hook for maintainer state |

## Security Validation

| Attack surface | Test performed | Result | Notes |
|---|---|---|---|
| Duplicate-signature quorum forgery | 3 entries from 1 key on Add / Remove / ProtocolActivation, at and above the gate | PASS | Rejected above the gate; accepted below it, by design and by frozen parity test |
| Zero-signature `AddMaintainer` on an empty root | `verify_multisig{,_legacy}` with `signatures = []` on `MaintainerSet::new()` | PASS | Refused at **every** height |
| Producer-key authority reclaim | `ProtocolActivation` above the gate with a sub-threshold on-chain root | PASS | Fails closed; ad-hoc producer derivation unreachable |
| `.env` gate override on mainnet | `DOLI_MAINTAINER_DERIVATION_ACTIVATION_HEIGHT=7` | PASS | Mainnet stays 172_000; devnet honours 7 (anti-vacuity leg asserted first) |
| Filesystem write to the data dir | Delete `maintainer_state.bin` above the gate | **FAIL** | PROBE-1 — silently re-arms a governance-removed key as an install trust root. Same reach the M1 `.env`-lock comment already treats as in-scope |
| Snap-sync trust-root substitution | Static: no `is_snap_synced()` consultation in the seed path | **FAIL** | Finding 5 |

## Specs/Docs Drift

| File | Documented behavior | Actual behavior | Severity |
|---|---|---|---|
| `docs/auto_update_system.md:130-136` | *"The **real implementation** is `derive_maintainer_set` … a node whose data directory was wiped **replays to the same root** instead of re-bootstrapping from live producer state (REQ-172-005 / REQ-172-010)."* | `derive_maintainer_set` has **zero production callers**. The node calls `derive_canonical_maintainer_set` over **live producer state**, and PROBE-1 shows a wiped node re-bootstrapping and re-arming a removed key — the precise opposite of the sentence | **high** |
| `specs/maintainer-trust-root-architecture.md` §F2 (F-7) | *"snap-only nodes still fail closed"* | They do not fail closed; they seed from the snapshot's producer set (Finding 5) | **high** |
| `docs/redesigns/…-analysis.md:525` (REQ-172-010 row) | *"**M2 implemented** — `derive_maintainer_set` revived as the replay-complete derivation"* | Revived, but not wired into any node path. The row should read "M2 partial — function landed, node wiring deferred" | medium |
| `crates/updater/src/trust_root.rs:227-230` | *"the chain derivation stable-sorts a `HashMap` iteration and has no pubkey tiebreak … member order is not a stable property"* | True only below the AH now. The set-based comparison remains correct, so this is comment drift, not a defect | low |
| `crates/core/src/network_params/mod.rs:571-577` and dev-notes §3/§4 | ProtocolActivation acceptance described as consensus-visible | It is not currently in the state root, and `is_protocol_active` has no callers. The gate is still the right call (conservative, and it becomes true the moment anything reads `active_protocol_version`), but the stated reason is wrong today | medium |

## Blocking Issues (must fix before merge)

- **[ISSUE-001]** `docs/auto_update_system.md:130-136` — states as fact a wiped-node
  replay property the code does not have, for the exact failure mode
  (AUDIT-P2-015 / F7) this incident exists to close. An operator reading it would
  conclude a data-dir wipe is safe. Either wire `derive_maintainer_set` into
  `maybe_bootstrap_maintainer_set`, or rewrite the paragraph to describe what the node
  actually does and name the residual.
- **[ISSUE-002]** `specs/maintainer-trust-root-architecture.md` §F2 (F-7) states snap-only
  nodes fail closed; `bins/node/src/node/periodic.rs:55-136` never consults
  `ChainState::is_snap_synced()`. Implement the fail-close (the flag already exists at
  `crates/storage/src/chain_state.rs:291`) or amend the spec and the REQ-172-005 row.
  Leaving spec and code disagreeing on a **Must** requirement is the drift CLAUDE.md
  forbids.
- **[ISSUE-003]** `docs/redesigns/maintainer-trust-root-redesign-analysis.md:525` —
  downgrade REQ-172-010 from "M2 implemented" to "M2 partial", and cite the node path
  rather than only the pure function. The cited test proves a property of uncalled code.

## Non-Blocking Observations

- **[OBS-001]** `periodic.rs:152` — make the one-shot guard survive file deletion, e.g.
  re-derive from block history when `maintainer_state.bin` is absent and
  `best_height >= AH`. Today the strongest attacker capability against the trust root is
  `rm maintainer_state.bin`.
- **[OBS-002]** `bins/node/src/node/rollback.rs` — no maintainer-state undo. Above the
  gate a governance mutation from a reorged-out block persists. Consider recording
  Add/Remove in the undo batch, or re-deriving on reorg.
- **[OBS-003]** Release-note the M1×M2 interaction (Finding 7): the first legitimate
  rotation above the AH disables auto-update fleet-wide until `trust_root.rs:155`
  containment is lifted.
- **[OBS-004]** Correct the ungated-change rationale in dev-notes §3 and
  `network_params/mod.rs`: the empty set *is* transitively reachable by
  `ProtocolActivation`; the real safety comes from `ChainState::serialize_canonical`
  excluding activation fields and `is_protocol_active` having no callers.
- **[OBS-005]** Re-verify the mainnet tip against 172_000 immediately before release;
  25.8 h of lead from pin time is thin for ~30 external auto-updaters.
- **[OBS-006]** `crates/core/src/network_params/mod.rs` is 697 lines (budget 500).
  Pre-existing; correctly not bundled into a security fix.

## Modules Not Validated

- `crates/updater/` M1 surface beyond the M2 interaction points (out of scope; covered by
  `docs/reviews/inc-i-157-M1-*` and the M1 QA report).
- Live testnet rehearsal of the 127_200 crossing — not attempted; recommend a scoped
  follow-up on the local testnet before the mainnet AH.

## Final Verdict

**CONDITIONAL APPROVAL.**

The consensus-critical layer — the part that carries fork risk and must be right before
h=172_000 — is correct. Pre-activation parity is proven from the diff, not assumed; every
consensus-reachable verifier is gated; the boundary is `>=` everywhere; block content,
the state root, `HardForkSchedule` and all four version constants are untouched; a mixed
fleet across the gate cannot fork. AC-3, AC-4, AC-5, AC-6, AC-7 and AC-8 pass. The gated
code is approved for activation.

The three blocking issues are documentation and traceability defects, not code defects,
and all three are cheap. They are blocking because each one asserts that a property is
closed when my probes show it is open, on the specific incident whose whole purpose is
to make the trust root trustworthy. REQ-172-010 (a **Should**) fails at the node level
and is tracked, not blocking. The two `high`-severity exploratory findings (wipe re-arms
a removed key; snap-sync silently diverges) are **not regressions** — pre-M2 behavior was
equal or worse — so they do not block the AH; they must be scoped to M3 in writing rather
than left implied by a doc that says they are already fixed.
