━━━ FINDINGS — 9 total (Major:3 Minor:6) ━━━

  [F1] MAJOR conf(0.90, observed) — crates/core/src/maintainer/derivation.rs:125-146 — the revived replay derivation verifies signatures UNGATED while holding the height in hand; it cannot reproduce pre-AH history, which is exactly what M3/R1 is told to wire it for
  [F2] MAJOR conf(0.95, measured) — bins/node/src/node/periodic.rs:139-157 + bins/node/src/node/apply_block/governance.rs:108-128 — M2's two new protection mechanisms are unregistered in protection_mechanisms; the M1×M2 auto-update-kill interaction exists in prose only
  [F3] MAJOR conf(0.85, observed) — crates/core/src/network_params/defaults.rs:573 + bins/node/src/node/periodic.rs:83-85 — devnet AH=0 × the hardcoded 5-producer seed precondition × F4 fail-close makes governance and ProtocolActivation permanently dead on the repo's own 2-producer devnet
  [F4] MINOR conf(0.90, observed) — crates/rpc/src/methods/governance.rs:99-140 — a fourth, ungated, HashMap-ordered derivation survives in getMaintainerSet, and the new runbook tells operators to trust it
  [F5] MINOR conf(0.85, observed) — crates/core/tests/inc_i_172_m2_canonical_derivation.rs:516-571 — a green test named for the wiped-node SYSTEM property only exercises a function with zero production callers
  [F6] MINOR conf(0.90, observed) — crates/core/src/maintainer/derivation.rs:98-102,148-152 — the new "replay-complete, root = f(seed, every governance action <= H)" claim is false: there is no H parameter and slashing is applied out of chronological order
  [F7] MINOR conf(1.00, measured) — bins/node/src/node/periodic.rs:1-2021 — M2 resolved the 790-line maintainer.rs violation but enlarged four pre-existing ones; the one-shot seed guard now lives in a 2021-line module
  [F8] MINOR conf(0.90, measured) — crates/core/src/maintainer/set.rs:88-116 — the UNGATED deviation's safety argument SURVIVES attack, but its load-bearing leg is a zero-callers fact with no tripwire test and no invariant record
  [F9] MINOR conf(0.85, observed) — bins/node/src/node/periodic.rs:139-157 × PM-172-02 — the protection halts the node on a CORRUPTED trust-root file but re-seeds silently on a DELETED one; the louder response is reserved for the harder attack

  Speculative: 1 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Code Review: INC-I-172 M2 — Layer 2 maintainer trust-root derivation

Reviewer pass, run_id=508, 2026-08-10. Scope restricted to M2 per brief; M1's
`crates/updater/` surface was NOT re-reviewed except where M1 and M2 interact (F2, F9).

## Scope Reviewed

| Area | Files |
|---|---|
| Activation height | `crates/core/src/network_params/{mod.rs,defaults.rs,env_loader.rs,tests.rs}` |
| Derivation module (split from 790-line `maintainer.rs`) | `crates/core/src/maintainer/{mod,set,derivation,data,tests}.rs` |
| Re-exports | `crates/core/src/lib.rs` |
| One-shot seed guard | `bins/node/src/node/periodic.rs` |
| Fail-close | `bins/node/src/node/apply_block/governance.rs` |
| Tests | `crates/core/tests/inc_i_172_m2_*.rs` (3), `bins/node/tests/inc_i_172_m2_*.rs` (2) |
| Cross-cutting (to bound blast radius) | `crates/core/src/validation/`, `crates/storage/src/chain_state.rs`, `crates/storage/src/snapshot.rs`, `crates/rpc/src/methods/governance.rs`, `bins/node/src/node/apply_block/{mod,state_update}.rs` |
| Docs/specs | `docs/auto_update_system.md`, `docs/troubleshooting.md`, `specs/maintainer-trust-root-architecture.md`, `specs/{protocol,engine-parts,l2-settlement}.md`, `docs/rpc_reference.md`, `docs/.workflow/inc-i-172-M3-scope.md` |

**Test evidence (measured, this session):** all 36 M2 tests pass.
`cargo test -p doli-core --test inc_i_172_m2_activation_height --test inc_i_172_m2_canonical_derivation --test inc_i_172_m2_maintainer_governance` → 11 + 11 + 14 passed, 0 failed.
`cargo test -p doli-node --test inc_i_172_m2_fail_close --test inc_i_172_m2_maintainer_reset` → 7 + 4 passed, 0 failed.

## Summary

⚠️ **APPROVED WITH OBSERVATIONS.** The fork-safety layer — the part that can hurt a live
chain — is correct, and I could not break it. The three Major findings are a latent M3 trap
(F1), a registry/process gap around a real composite failure (F2), and a devnet functional
regression (F3). None of them threatens mainnet consensus.

**Is this a redesign or a reshuffle?** Genuine redesign for two of the four root causes,
partial for the third, and honestly labelled as partial in-tree.

* **Reset button (F2 of the spec) — HARDER, not dead.** Above the gate the *per-block*
  re-derivation is genuinely removed: `maintainer_seed_is_done` (`periodic.rs:151-157`)
  makes the seed one-shot, and `above_the_gate_a_governance_removal_survives_the_next_block`
  proves it. But the guard reads `maintainer_state.bin` alone, so `rm maintainer_state.bin`
  still re-arms a removed key (QA PROBE-1, `removed_key_back=true`). The reset button moved
  from "any block" to "one filesystem write". That is a real improvement and it is NOT sold
  as more than it is — `docs/auto_update_system.md:145-152` and
  `docs/.workflow/inc-i-172-M3-scope.md` R1 state it plainly. Credit where due: this is the
  rare case where the residual is documented before a reviewer finds it.
* **ONE derivation — TRUE for consensus, FALSE as written.** Above the gate exactly one
  derivation runs on the node path (`derive_canonical_maintainer_set`). But four derivations
  exist in tree: canonical, the frozen pre-AH stable sort (`periodic.rs:97-107`), the frozen
  pre-AH `derive_ad_hoc_maintainer_set` (`governance.rs:163-175`), and an **ungated** one in
  the RPC (F4). `crates/core/src/maintainer/mod.rs:49` says "is now the ONE derivation".
* **Distinct signers (F3 of the spec) — DONE and correct.** `count_distinct_signers`
  (`set.rs:130-149`) adopts the mainnet-live covenant shape from `conditions/eval.rs`
  rather than inventing a HashSet dedup. Correct call.
* **Fail-close (F4 of the spec) — DONE.** `governance.rs:108-128` removes the producer-key
  fallback above the gate. Verified reachable and tested both sides.

## Fork Safety — the thing that matters most

I hunted for leaks of new behavior below the gate and found none.

* **Boundary comparison is `>=` at every site, with no exceptions.** Four sites:
  `periodic.rs:70`, `governance.rs:108`, `set.rs:269`, `set.rs:285`. Tested at `AH-1`, `AH`,
  and `AH+1` (`inc_i_172_m2_canonical_derivation.rs:342-404`).
* **The AH is read at exactly two production sites** (`periodic.rs:65-69`,
  `governance.rs:23-27`), both from `self.config.network.params()` — chain-derived height
  against a constant, never a per-process counter (INV-SYNC-012 respected).
* **Pre-activation parity is preserved by construction, not by comment.** The historical
  entry-counting verifier survives verbatim as `verify_multisig_legacy` /
  `verify_multisig_excluding_legacy` (`set.rs:198-251`), the HashMap-ordered stable sort
  survives verbatim in the `else` branch (`periodic.rs:97-107`), and
  `derive_ad_hoc_maintainer_set` is kept "unchanged, including its HashMap-ordered
  non-determinism". `pre_activation_counter_preserves_entry_counting_exactly` asserts the
  *defect* still reproduces below the gate — the right test to write.
* **No `HardForkSchedule` entry.** `git diff crates/updater/src/hardfork.rs` is empty.
  Correct per CLAUDE.md — `current_fork_id(u64::MAX)` would activate it immediately.
* **No version-constant movement.** No change to `CURRENT_PROTOCOL_VERSION`,
  `EPOCH_STATE_FORMAT_VERSION`, `MIN_PEER_PROTOCOL_VERSION`, or
  `MAINTAINER_STATE_VERSION`. The only `Cargo.toml` change is the repository URL
  (`e-weil/doli` → `doli-network/doli`), not `version = "6.24.1"`.
* **No block-content change.** Transaction shapes, coinbase, header fields, bitfield
  encoding untouched. INV-DEPLOY-001 / INC-I-062 not triggered; rolling deploy is safe.
* **Mainnet AH is in the future** (172_000 vs tip 162_727 at pin), so no already-executed
  `ProtocolActivation` is reinterpreted — the spec's own stated precondition (§F2:186) is met.
* **Mainnet env override is LOCKED** (`env_loader.rs:429-436`), so no operator can move
  the gate on a mainnet node and fork itself off. Tested
  (`inc_i_172_m2_activation_height.rs:142-175`) with an anti-vacuity control.

## The deliberately UNGATED change — I attacked it; it holds

`calculate_threshold(0)` now returns 3 (was 0) and every verifier short-circuits on
`!is_authorizable()`, at ALL heights. The dev notes' ORIGINAL argument was wrong and was
corrected; I attacked the CORRECTED argument independently against the tree.

**The deviation is real.** Below the gate with an empty derived set, the OLD code returned
`valid_count (0) >= threshold (0)` = **true**, i.e. it ACCEPTED a zero-signature
`ProtocolActivation` and a zero-signature `AddMaintainer`. The new code refuses. That is a
pre-activation behavior change and it is not gated. So the only question is whether the
outcome is consensus-visible. Three legs, all verified:

1. **State root.** `ChainState::serialize_canonical` (`crates/storage/src/chain_state.rs:143-155`)
   is a fixed 140-byte buffer holding best_hash / best_height / best_slot / total_work /
   genesis_hash / genesis_timestamp / last_registration_hash / registration_sequence /
   total_minted. Neither `active_protocol_version` nor `pending_protocol_activation` appears.
   `crates/storage/src/snapshot.rs:30-32` is the only state-root construction and it consumes
   exactly `serialize_canonical()`. **Leg holds.**
2. **Consumer.** `is_protocol_active` (`crates/core/src/consensus/constants.rs:19`) has zero
   non-test callers (grep over `crates` and `bins`: only the `lib.rs:163` re-export, a doc
   comment, and `consensus/tests.rs`). `active_protocol_version` is written at
   `state_update.rs:83` and compared at `:64`, and read nowhere else in production.
   **Leg holds.**
3. **Block validity.** `apply_block/mod.rs:216-221` consumes the
   `process_transaction_governance` `Option` only to assign
   `pending_protocol_activation_data`. No `?`, no `Err`, no rejection.
   **I added a fourth check the dev notes do not make:** could the ungated refusal flip
   *transaction* validation? No — `crates/core/src/validation/tx_types.rs:739-776`
   (`validate_maintainer_change_data`) and `:938-980` (`validate_protocol_activation_data`)
   are purely structural: input/output count, `extra_data` deserialization, version != 0,
   signatures non-empty. Neither consults a `MaintainerSet`. The file even says so at
   `:737-738`. **Leg holds.**

**Verdict: the ungated change is NOT a fork risk in this tree.** It is safe on three facts,
not three invariants — see F8 for the missing tripwire.

## Semantics moved in the `maintainer.rs` → `maintainer/` split

Diffed against `git show HEAD:crates/core/src/maintainer.rs` (790 lines). All 16 inline
tests survive verbatim (`comm -23` of old vs new `fn test_*` names is empty), plus two new.
`MaintainerChangeData` / `ProtocolActivationData` / `MaintainerSignature` / `MaintainerError`
moved byte-equivalent. `MaintainerSet::new()` deliberately keeps `threshold: 0` (not
`calculate_threshold(0)`) so the M1 versioned decoder round-trip still holds — correct and
documented at `set.rs:35-39`.

**Two semantics DID change inside `derive_maintainer_set`, both deliberate, both undocumented
as changes:** the old code walked `take(INITIAL_MAINTAINER_COUNT)` of reader order and let
`add_maintainer` silently reject duplicates (so a duplicate in the first five yielded a
FOUR-member set); the new `seated_registrations` de-duplicates first and then seats five, so
a sixth-position key takes the freed seat. And the seed `last_updated` changed from "height
of the last successful add in reader order" to `max(registered_at)` over the seated five.
Both are improvements; neither is a live-path change because the function has zero
production callers. Noted for completeness, not raised as a finding.

## `crates/core` boundary — clean

`derive_canonical_maintainer_set(&[(PublicKey, u64)], u64)` (`derivation.rs:80-89`) takes a
value slice. The `ProducerInfo -> (public_key, registered_at)` map happens at the call site
in `bins/node` (`periodic.rs:91-94`). `crates/core::maintainer` imports only `crypto` and
`serde`. No edge to `storage`. C-R4 satisfied.

---

# Findings

### [F1] MAJOR — the revived replay derivation is not height-gated

- **Location:** `crates/core/src/maintainer/derivation.rs:125-146`
- **Evidence:** `for (height, change) in changes` binds `height` and passes it to
  `add_maintainer(data.target, height)` (`:131`) and `remove_maintainer(&data.target, height)`
  (`:142`) — but the authorization calls are `maintainer_set.verify_multisig(...)` (`:130`)
  and `verify_multisig_excluding(...)` (`:137`), the ungated post-activation forms, NOT
  `verify_multisig_at(..., height, activation_height)`. The doc comment at `:104-106` claims
  "this path has no chain height to gate on"; line `:125` contradicts it — the height is in
  the tuple and is already used two lines later.
- **Impact:** `docs/.workflow/inc-i-172-M3-scope.md:32-36` names this function as R1's fix
  ("wire the already-revived `derive_maintainer_set` … into the seed path"). The moment that
  happens, a wiped node replaying any pre-AH `AddMaintainer`/`RemoveMaintainer` that the live
  fleet ACCEPTED under entry-counting will REJECT it under distinct-signer counting, and
  derive a different trust root than the node that stayed online. That is the precise
  divergence class the activation height exists to prevent, pre-built into the function M3 is
  instructed to use. `set.rs:191` already states the rule this violates: "**MUST NOT be called
  ungated**".
- **Confidence:** conf(0.90, observed)
- **Suggested fix:** give `derive_maintainer_set` an `activation_height: u64` parameter and
  call `verify_multisig_at(&data.signatures, &message, height, activation_height)` /
  `verify_multisig_excluding_at(...)`. Add a test replaying a history that SPANS the gate:
  an entry-counting-only change below AH plus a distinct-signer change above it, asserting
  the replay matches an incrementally-applied online node.

### [F2] MAJOR — M2's protection mechanisms are unregistered; a known composite failure lives in prose only

- **Location:** `bins/node/src/node/periodic.rs:139-157` (one-shot seed guard);
  `bins/node/src/node/apply_block/governance.rs:108-128` (ProtocolActivation fail-close);
  `crates/core/src/maintainer/set.rs:88-90` (ungated `is_authorizable` refusal)
- **Evidence:** `sqlite3 .omega/memory.db "SELECT mechanism_id,name FROM protection_mechanisms WHERE mechanism_id LIKE '%172%'"`
  returns exactly two rows, PM-172-01 and PM-172-02, both M1. Total registry: 27 mechanisms,
  none for M2. All three constructs above match the protocol's definition of a protection
  mechanism (trigger condition + constraining action + scale assumption).
  Meanwhile `docs/.workflow/inc-i-172-M3-scope.md:91-103` (R4) records a genuine composite
  failure: M2 makes rotation durable, and M1's PM-172-01 containment
  (`crates/updater/src/trust_root.rs:155`) refuses any set differing from the compiled
  bootstrap five, so **the first legitimate rotation above the gate disables auto-update
  fleet-wide** — the exact action this incident exists to enable. Pre-M2 the rotation
  self-reverted within one block, so the refusal never latched.
- **Impact:** the interaction is understood and written down, but `v_protection_surface` — the
  query every future change is required to run — cannot see it. The next agent touching the
  update path will find PM-172-01 with no recorded interaction to M2's one-shot seed.
- **Confidence:** conf(0.95, measured)
- **Suggested fix:** register three mechanisms (PM-172-03 one-shot seed guard, PM-172-04
  ProtocolActivation fail-close, PM-172-05 unauthorizable-set refusal) with trigger, action,
  scale assumption (`INITIAL_MAINTAINER_COUNT = 5` producers — see F3), and
  `interacts_with = ["PM-172-01","PM-172-02"]`; update PM-172-01's `interacts_with` to name
  PM-172-03 and record the R4 latch in its notes.

### [F3] MAJOR — devnet AH=0 kills governance and ProtocolActivation on the repo's own devnet

- **Location:** `crates/core/src/network_params/defaults.rs:573` (devnet AH = 0);
  `bins/node/src/node/periodic.rs:83-85`; `bins/node/src/node/apply_block/governance.rs:114-127`;
  `crates/core/src/maintainer/set.rs:88-90`
- **Evidence:** `scripts/launch_testnet.sh:2-3` — "DOLI Testnet - Two Producer Genesis Launch";
  `:94-97` generates exactly `producer1.json` and `producer2.json`. `periodic.rs:83-85` returns
  early when `all.len() < INITIAL_MAINTAINER_COUNT` (5), so with 2 producers the root NEVER
  seeds and stays `members: []`. Devnet AH is 0, so `height >= activation_height` is true from
  block 0: `governance.rs:114-127` takes the fail-close branch and returns `None` for every
  `ProtocolActivation`, permanently. `AddMaintainer` cannot rescue it either —
  `is_authorizable()` is false on an empty set, so `verify_multisig_at` refuses at
  `set.rs:165-167` before counting. Pre-M2 the same devnet accepted a `ProtocolActivation` via
  `derive_ad_hoc_maintainer_set(2 producers)` → `with_members` → `calculate_threshold(2) = 2`.
- **Impact:** a functional regression, absorbing until a 5th producer registers, on the one
  network the project deliberately keeps env-overridable *because* "devnet's accelerated timing
  is what makes the update path testable at all" (`crates/core/src/network_params/tests.rs`,
  AUDIT-P3-012 control-test rationale). It is also a **scale-sensitivity** defect by the
  protocol's own definition: `INITIAL_MAINTAINER_COUNT = 5` is a constant with no derivation
  from observed network size, and no test covers a sub-5-producer network above the gate.
  Mainnet (30+ registered producers) and the 12-node local testnet are unaffected.
- **Confidence:** conf(0.85, observed) — derived from script + code, not from a booted devnet.
- **Suggested fix:** simplest first — raise `scripts/launch_testnet.sh` to 5 producers and
  state the `>= INITIAL_MAINTAINER_COUNT` precondition in `docs/auto_update_system.md`. If
  2-producer devnets must keep working, pin devnet to a small future height instead of 0.
  Either way add a test: `<5` producers above the gate → assert the intended dead-end
  explicitly, so it is a decision rather than a surprise.

### [F4] MINOR — a fourth, ungated derivation survives in the RPC the new runbook trusts

- **Location:** `crates/rpc/src/methods/governance.rs:99-140`
- **Evidence:** `:113-118` — `producers.all_producers()` then `sort_by_key(|p| p.registered_at)`
  then `.take(INITIAL_MAINTAINER_COUNT)`: the identical HashMap-ordered stable-sort shape that
  M2 froze as pre-activation-only at `periodic.rs:97-107`, still live at ALL heights on the RPC
  fallback. `:129` calls `calculate_threshold(member_count)`. Meanwhile
  `docs/troubleshooting.md` (new INC-I-172 block after `:751`) instructs: "verify the root with
  `getMaintainerSet` against a known-good node before trusting auto-update on that host."
- **Impact:** when `maintainer_state` is `None` the RPC reports a non-deterministic
  producer-derived set, so two honest nodes on the same chain can print different `maintainers`
  arrays and the runbook comparison yields a false mismatch. The response does carry
  `"source": "derived"` vs `"on-chain"`, so it is distinguishable — but the runbook does not
  say to check it, and `crates/core/src/maintainer/mod.rs:49` claims the canonical function "is
  now the ONE derivation", which is false for this path.
- **Confidence:** conf(0.90, observed)
- **Suggested fix:** route the fallback through `derive_canonical_maintainer_set` (it is
  read-only, so no gate is needed for correctness — only for determinism), and amend the
  troubleshooting step to say "compare only when `source` is `on-chain` on both nodes".

### [F5] MINOR — a green test asserts a system property the system does not have

- **Location:** `crates/core/tests/inc_i_172_m2_canonical_derivation.rs:516-571`
- **Evidence:** the doc comment reads "A node whose data directory was wiped replays (genesis
  seed + every governance action <= H) from block data. A node that was online throughout …
  Both must reach the same root." The body (`:551-555`) calls `derive_maintainer_set(&Chain{…})`
  against a hand-built `BlockchainReader` and compares to a hand-mutated `MaintainerSet`
  (`:540-548`). It never touches `maybe_bootstrap_maintainer_set`. The production wipe path
  measurably does the OPPOSITE — `docs/.workflow/inc-i-172-M3-scope.md:23-26`, QA PROBE-1,
  `after_wipe_len=5`, `removed_key_back=true`. Test passes (measured, 11/11 in that file).
- **Impact:** a green test named for R1's property, in a file M3 will read while implementing
  R1. The unit property is real; the framing invites the reader to conclude the system
  property is closed.
- **Confidence:** conf(0.85, observed)
- **Suggested fix:** rename to `replay_function_converges_with_incremental_application` and put
  "NOT the production wipe path — see R1" in the doc comment. Zero code change.

### [F6] MINOR — the "replay-complete, actions <= H" claim is false in two ways

- **Location:** `crates/core/src/maintainer/derivation.rs:98-102` (claim), `:148-152` (slashing)
- **Evidence:** the function signature is `derive_maintainer_set<R: BlockchainReader>(reader: &R)`
  — there is no `H` parameter, so it replays whatever the reader returns with no height bound.
  And `get_slashed_producers()` (trait `:25`) returns `Vec<PublicKey>` with **no heights**, and
  step 3 applies all of them AFTER every Add/Remove, so a slash that chronologically preceded a
  legitimate re-add is applied after it. Ordering is inherited from HEAD; the "replay-complete"
  and "<= H" claims are new in M2.
- **Impact:** doc asserts a property the code lacks (CLAUDE.md rule 7, code is SoT). The missing
  `<= H` bound is also a prerequisite for R2 (reorg undo) — you cannot re-derive "the root as of
  the reorg target" from a function with no height bound.
- **Confidence:** conf(0.90, observed)
- **Suggested fix:** add `up_to_height: u64` and filter `changes` and registrations by it;
  extend `BlockchainReader::get_slashed_producers` to `Vec<(u64, PublicKey)>` and merge slashes
  into the chronological change stream. Or, if that is M3, downgrade the doc claim now.

### [F7] MINOR — module-size budget: one violation resolved, four enlarged

- **Location:** `bins/node/src/node/periodic.rs`, `crates/core/src/network_params/{mod,defaults,env_loader}.rs`
- **Evidence:** `wc -l` now vs `git show HEAD:<path> | wc -l`:

  | File | HEAD | Now | Budget | Class |
  |---|---|---|---|---|
  | `crates/core/src/maintainer.rs` | 790 | — (deleted) | 500 | **RESOLVED by M2** |
  | `crates/core/src/maintainer/{mod,set,derivation,data}.rs` | — | 79 / 443 / 155 / 116 | 500 | compliant |
  | `crates/core/src/maintainer/tests.rs` | — | 392 | 800 | compliant |
  | `bins/node/src/node/periodic.rs` | 1955 | **2021** | 500 | pre-existing, M2 +66 |
  | `crates/core/src/network_params/mod.rs` | 654 | **711** | 500 | pre-existing, M2 +57 |
  | `crates/core/src/network_params/defaults.rs` | 558 | **584** | 500 | pre-existing, M2 +26 |
  | `crates/core/src/network_params/env_loader.rs` | 530 | **557** | 500 | pre-existing, M2 +27 |

  All five M2 test files are under the 800-line test budget (largest: 572).
- **Impact:** M2 created no new violation and fixed the biggest one in its own scope — net
  positive. The concern is placement, not arithmetic: the one-shot seed guard is a security
  control living in a 2021-line grab-bag alongside DeFi health caching and archive flushing.
- **Confidence:** conf(1.00, measured)
- **Suggested fix:** extract `maybe_bootstrap_maintainer_set` + `maintainer_seed_is_done` into
  `bins/node/src/node/maintainer_seed.rs` (~110 lines). Separate commit; not an M2 blocker.

### [F8] MINOR — the ungated safety argument holds, but has no tripwire

- **Location:** `crates/core/src/maintainer/set.rs:88-116`; argument recorded at
  `crates/core/src/network_params/mod.rs:568-584`
- **Evidence:** see "The deliberately UNGATED change" above — all three legs verified against
  code, plus a fourth (structural-only transaction validation) that the dev notes do not make.
  The argument survives. **Residual:** leg 2 is "`is_protocol_active` has zero production
  callers", a fact with nothing that fails when it stops being true. Leg 1 is partially guarded
  — `test_serialize_canonical_fixed_size` (`chain_state.rs:540`) fails if the buffer length
  changes, so adding a field to it trips a test. Leg 3 is unguarded. The dev notes themselves
  say "Points 1-2 are facts about the tree, NOT invariants -- they expire the moment anything
  reads `active_protocol_version`", which is the right diagnosis with no mechanism attached.
- **Impact:** the deviation is measurably behavior-changing below the gate (an empty derived set
  ACCEPTED a zero-signature `ProtocolActivation` before M2, refuses after). Its safety is
  entirely contingent. A future consensus rule reading `active_protocol_version` — which the
  field exists for — makes it retroactively fork-relevant over history this gate already governs.
- **Confidence:** conf(0.90, measured)
- **Suggested fix:** record an `invariants` row ("no production code may read
  `active_protocol_version` / call `is_protocol_active` until the maintainer-root divergence
  axis is closed") with a linked `regression_tests` entry, and a grep-based tripwire test in
  `crates/core` that fails when a non-test caller appears. Cheap and it converts a fact into a
  guard.

### [F9] MINOR — PM-172-02 is strictly weaker against the strictly easier attack

- **Location:** `bins/node/src/node/periodic.rs:139-157` interacting with PM-172-02
- **Evidence:** PM-172-02's registered trigger explicitly excludes an absent file — "A MISSING
  file is NOT a trigger (fresh node)" — while corruption is fatal at startup.
  `maintainer_seed_is_done` reads only `state.set.members.is_empty()` and
  `state.last_derived_height != 0`, both of which read false when the file is absent. So
  `rm maintainer_state.bin` → silent re-seed from live producer state (R1), no log, no halt;
  flipping one byte in the same file → node refuses to start.
- **Impact:** the loud response is reserved for the harder action. An attacker with filesystem
  write already prefers delete over corrupt; M2's one-shot guard makes delete strictly more
  valuable than it was pre-M2 (the re-seed now sticks instead of being overwritten next block).
- **Confidence:** conf(0.85, observed)
- **Suggested fix:** at startup, if `maintainer_state.bin` is absent AND
  `best_height >= maintainer_derivation_activation_height`, emit an `error!` naming the
  re-seed risk and the `getMaintainerSet` cross-check. One log line, no behavior change; the
  real fix is R1.

## Speculative Findings (low-confidence, not actionable)

- **R5 activation-lead erosion.** `maintainer_derivation_activation_height = 172_000` was pinned
  on 2026-08-10 against a mainnet tip of 162_727 — 9,273 blocks ≈ 25.8 h at 10 s slots. I did
  not query mainnet, so I cannot say what the lead is now. `docs/.workflow/inc-i-172-M3-scope.md`
  R5 already mandates a pre-release re-query and a forward re-pin *while the height is uncrossed*.
  Flagging only to confirm R5 is a release gate, not a suggestion. conf(0.50, inferred).

## Specs/Docs Drift

QA's three blocking issues were documentation defects. **All three are now corrected and the
corrections are TRUE against code**, verified independently:

* `docs/auto_update_system.md:129-165` — states plainly that `derive_maintainer_set` has zero
  production callers, that deleting `maintainer_state.bin` re-arms a removed key (with the
  measured PROBE-1 citation), and carries a convergence table marking snap-sync and
  file-delete as **NO**. Matches code.
* `specs/maintainer-trust-root-architecture.md:195-213` — an explicit "F2 as SHIPPED vs F2 as
  PROPOSED" amendment retracting the F-7 snap claim and the replay claim. Matches code.
* `docs/troubleshooting.md` (new block after `:751`) — tells operators not to delete the file
  alone, and distinguishes full-sync (converges) from snap-sync (does not). Matches code.

Remaining drift, all Minor, none blocking:

| File | Issue |
|---|---|
| `crates/core/src/maintainer/mod.rs:49` | "is now the ONE derivation" — false for `crates/rpc/src/methods/governance.rs:113-118` (F4) |
| `crates/core/src/maintainer/derivation.rs:98-106` | "replay-complete … <= H" and "no chain height to gate on" — both false (F1, F6) |
| `specs/protocol.md:693,720` | "Requires 3/5 multisig from current maintainers (first 5 registered producers)" — no mention of `maintainer_derivation_activation_height`, and "first 5 registered producers" describes only the genesis seed, not the post-rotation set |
| `specs/l2-settlement.md:50` | path updated to `crates/core/src/maintainer/data.rs` ✓, but `governance.rs:80` is stale — the `ProtocolActivation` branch is now `:101` |
| `specs/engine-parts.md:1762-1830` | section header updated to `crates/core/src/maintainer/` ✓, but `derive_canonical_maintainer_set` / `verify_multisig_at` / `verify_multisig_excluding_at` / `is_authorizable` / `verify_multisig_legacy` are absent from the symbol list |
| `docs/rpc_reference.md:1018` | "Falls back to ad-hoc derivation if `MaintainerState` is not yet available" — accurate but does not say the fallback is ungated and non-deterministic (F4) |
| `docs/auto_update_system.md:245` | "avoids re-deriving from genesis on every restart" — there is no re-derive-from-genesis path in the node at all |

## Deferral judgement (R1-R5)

**Defensible.** R1 needs a real `BlockchainReader` over the block store — new production code
on the consensus-adjacent seed path after QA signed off on the gated layer; that is genuinely
M3-sized. R3's argument is the strongest of the five: making snap-synced nodes fail closed
introduces a NEW divergence axis on a currently-inert field, which would invalidate the
fork-safety review the approval rests on. Deferring a change *because* it would widen the
blast radius past what was reviewed is correct engineering, not avoidance. R2 is honest about
being a filter (F-6 ROLLBACK PARITY) the design listed and the implementation did not meet.

**One thing belongs in M2, not M3:** F1. Not the wiring — the *signature*. Leaving
`derive_maintainer_set` ungated is a landmine planted directly under M3's stated task, and
fixing it is a parameter plus two call-site edits in a function with zero production callers,
i.e. zero deploy risk. It costs less now than the divergence it will cause later.

## Architecture escalation

None. No `[ARCHITECTURE]` finding. The layered design (Layer 1 node-local no-AH, Layer 2
AH-gated consensus-adjacent) is sound, the AH is a constant gate rather than a
`HardForkSchedule` entry as CLAUDE.md requires, and the three unmet failure filters are
recorded against the design rather than hidden.

## Contradiction check (intellectual honesty)

I looked for an unacknowledged contradiction in the upstream artifacts and found none. The
dev notes §3 explicitly retract their own original safety argument and replace it — the
retraction is labelled as a retraction, and the replacement is correct. The spec carries an
"as SHIPPED vs as PROPOSED" amendment rather than quietly editing the original claim. That is
the behavior the protocol asks for.

## Final Verdict

**Approved with observations.** Merge-safe for mainnet: fork safety verified, 36/36 tests
green, no version movement, no block-content change, no `HardForkSchedule` entry, mainnet env
lock in place, activation height in the future. Recommend F1 (2-line signature change) and a
decision on F3 (devnet) before commit; F2 (registry) before the milestone closes; the rest are
M3 or follow-up.

---

━━━ RESOURCE COST — LOW ━━━
Dimensions:
  CPU:      +O(1) per replayed governance change for F1's added `height >= AH` branch; F4 routes the RPC fallback through one O(n log n) sort it already performs (observed)
  Memory:   0 — F1 adds one u64 parameter; F4 reuses the existing Vec; F7 moves lines between files (observed)
  IO:       +1 log line per node start in the absent-file case (F9); zero on the normal path (observed)
  Network:  0 (observed)
  Disk:     0 — no new persisted field, no MAINTAINER_STATE_VERSION bump implied by any fix (observed)
  Latency:  0 on block apply and block production; F1/F6 touch only a function with zero production callers, F4 only an RPC read path (measured — the fixes do not touch periodic.rs:55-136 or governance.rs:17-154 hot paths)
Inevitability: AVOIDABLE
Cheaper alternative: fix F1 alone (parameter + two call sites) and downgrade F4/F6 to doc corrections
Why this proposal anyway: F1 is the only fix that must land before M3 touches the seed path — it costs two lines against a zero-caller function and prevents the wiped-node/online-node root divergence M3 is otherwise being set up to build; F3 is a shipped functional regression on the one network where the update path is testable, and its fix is a script or a constant, not a design change
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

━━━ SECURITY AUDIT VERDICT ━━━
Verdict: AUDIT-REQUIRED
Signals: authorization logic (who may sign AddMaintainer / RemoveMaintainer / ProtocolActivation) on a live chain whose maintainer keys are known-leaked (INC-I-170); cryptographic quorum counting rewritten (entry-count → distinct-signer, `set.rs:130-149`); trust-boundary change on the enforcement surface itself — the maintainer set M2 governs IS the binary-install trust root wired by M1 (`crates/updater/src/trust_root.rs`); consensus-adjacent activation-height gate on user-submittable transactions; state integrity (a durable trust root that survives a block, and a reorg path that does not undo it — R2); fail-close authorization path with a measured absorbing state on sub-5-producer networks (F3)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Security Audit Verdict: AUDIT-REQUIRED
