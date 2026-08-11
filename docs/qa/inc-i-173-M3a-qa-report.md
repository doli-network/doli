# QA Report: INC-I-173 M3a — salvage/reduction of M3 to F4+F5+F6+F7

```
━━━ FINDINGS ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
VERDICT: PASS — the reduction is complete, the four kept items are green, and
         no removed-item artefact survives anywhere in bins/ or crates/.

  [P1] PASS conf(1.00, measured) — all 15 must-be-untouched paths are at ZERO
       diff vs 32e0a650. Evidence: per-path loop over `git diff 32e0a650 --stat --`
       + `git status --porcelain --` → diff_lines=0 / porcelain=0 for every one,
       including `crates/updater` and `bins/node/src/updater` (whole trees).

  [P2] PASS conf(1.00, measured) — the INC-I-172 blanket-refusal guard is
       byte-identical status quo. Evidence: `git show 32e0a650:crates/updater/src/trust_root.rs
       | shasum -a 256` = `0cdde54184495886ac606d8a3ee7d10b9f4e71360ba5b59d6b1eb4773c539d3b`
       == `shasum -a 256 crates/updater/src/trust_root.rs` (identical).

  [P3] PASS conf(1.00, measured) — ZERO dangling references to the removed
       items, per source root. Evidence: `grep -rn --include='*.rs' -F` for
       `Option E` / `bound_to` / `rotation journal` / `MaintainerRotation` /
       `replay_rotation` / `MAX_ROTATION` / `maintainer_journal` →
       bins/: 0, 0, 0, 0, 0, 0, 0 ; crates/: 0, 0, 0, 0, 0, 0, 0.

  [P4] PASS conf(1.00, measured) — REQ-173-014 (F5) holds, including the
       below-gate INERTNESS partition and its anti-vacuity discriminator.
       Evidence: `cargo test -p doli-core --test inc_i_173_m3_payload_bounds`
       → 19 passed; 0 failed. `crates/core/tests/inc_i_173_m3_payload_bounds.rs:666-690`
       asserts `!is_size_cap_rejection(&e)` below the gate.

  [P5] PASS conf(1.00, measured) — F6 is purely ADDITIVE and the `none` branch
       publishes `genesis_hash` with NO digest. Evidence: `git diff 32e0a650 --
       crates/rpc/src/methods/governance.rs` contains only `+` field lines (no
       `-` on any JSON key); live testnet base binary at RPC 8500/8501 returns
       the 10 pre-existing fields and NEITHER new field. 7/7 + 10/10 tests pass.

  [P6] PASS conf(1.00, measured) — F4: `fn is_state_only` is DEFINED nowhere.
       Evidence: `grep -rn --include='*.rs' 'fn is_state_only' bins crates` →
       1 hit, and it is the negative assertion at
       `bins/node/tests/inc_i_173_m3_f4_routing.rs:147`. 10/10 tests pass.

  [P7] PASS conf(1.00, measured) — F7 / REQ-173-011 is TOTAL over all 24
       variants. Evidence: `TxType` enum = 24 discriminants
       (`crates/core/src/transaction/types.rs`); the test pins
       `rows.len() == 24` at `inc_i_173_m3_f7_cross_list.rs:341-343`; 4/4 pass.

  [P8] PASS conf(1.00, measured) — the 3 full-suite failures are EXACTLY the
       named known set, no M3a regression. Evidence: `grep -n ' FAILED$'
       /tmp/m3a-fulltest.log` → lines 2142 `test_network::test_cluster_10x100`,
       2716 `test_cluster_10x100`, 2909
       `contention_tests::tests::inc_i_096_below_gate_rejects_remove_liquidity`.

  [O1] OBS medium conf(1.00, measured) — `specs/SPECS.md:43` index DRIFT: still
       reads "F4-F7 + Options A-E PROPOSAL" while
       `specs/state-only-fee-gate-architecture.md:17` reads "M3a IMPLEMENTED
       (F4+F5+F6+F7 only)". Non-blocking; the spec body itself is correct.

  [O2] OBS medium conf(1.00, measured) — the testnet gate is ALREADY CROSSED
       and the in-source deploy note cites a STALE tip. Evidence: `getChainInfo`
       @127.0.0.1:8500 → `bestHeight: 135642`; `inc_i_173_activation_height`
       testnet = `133_000` (`network_params/defaults.rs:480`);
       `bins/node/src/node/validation_checks.rs:322` still says "tip measured
       134_159". Deploy is a SYNCHRONIZED stop-all/start-all (INV-8), and the
       comment's own "M2 must re-pin the testnet height" action is still OPEN.

  [O3] OBS low conf(1.00, measured) — stale workflow artefact:
       `docs/.workflow/inc-i-173-M3-implementation.md:19` still records
       "Item 4 — Rotation journal + guard redesign … **DONE**" and lists
       `crates/storage/src/maintainer_journal.rs` (:40), a file that does not
       exist on this branch.

  [O4] OBS low conf(1.00, measured) — `MAX_MAINTAINER_CHANGE_ENCODED_BYTES`
       (`crates/core/src/maintainer/mod.rs:177`) is `pub` with test-only
       consumers (`inc_i_173_m3_payload_bounds.rs:112,272`). Deliberate: it is
       the derived-arithmetic anchor that replaced drifting prose. No action.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Scope Validated

The M3a **reduction** on `bugfix/inc-i-173-state-only-fee-gate`, working tree vs base
`32e0a650` (HEAD is `32e0a650`; nothing committed). Validated: reduction completeness,
absence of removed-item traces, and the four kept items F4/F5/F6/F7 against REQ-173-011
and REQ-173-014.

**Not re-validated** (out of scope, by instruction): the original six-item M3, the
security-audit findings that invalidated items 3 and 4, and M1 (F1+F2+F3, already in base).

## Summary

**PASS.** The reduction is complete and leaves no residue. Every one of the 15 paths that
had to return to base is byte-identical, `crates/updater/src/trust_root.rs` hash-matches
`32e0a650` exactly, and all seven removed-item grep patterns return zero hits in both
`bins/` and `crates/`. The four kept items are green across 50 targeted tests. The
reduction did not orphan any constant, did not introduce any `#[allow(dead_code)]`, and
did not touch a single activation-height value. Four non-blocking observations, all
documentation or deploy-sequencing, none of which prevent commit.

## System Entrypoint

No system start was required or attempted — M3a is source-level and the pre-captured gates
(`cargo build --release`, workspace clippy, full workspace test) already prove the tree
compiles and runs. Validation used:

```bash
cargo test -p doli-core --test inc_i_173_m3_payload_bounds
cargo test -p doli-core --test inc_i_173_m3_maintainer_digest
cargo test -p doli-core --test inc_i_173_m3_f7_cross_list
cargo test -p rpc       --test inc_i_173_m3_maintainer_set_rpc
cargo test -p doli-node --test inc_i_173_m3_f4_routing
```

Live **read-only** RPC probe against the local testnet (`127.0.0.1:8500`, `:8501`) using
`getMaintainerSet` and `getChainInfo`. No write, no SSH, no deploy.

## 1. Reduction Completeness

`git diff 32e0a650 --stat` → **14 files, +512 / -79**, plus 6 untracked `.rs`
(5 new test files + `crates/core/src/maintainer/digest.rs`). **0 deleted files**
(`git diff 32e0a650 --diff-filter=D --name-only` is empty).

Every modified file maps to a kept item:

| File | Item |
|---|---|
| `crates/core/src/maintainer/mod.rs` | F5 constants + F6 `digest` mod export |
| `crates/core/src/validation/tx_types.rs` | F5 bounds |
| `crates/core/src/validation/transaction.rs` | F5 `ctx` plumbing (2 call sites) |
| `crates/core/src/transaction/core.rs` | F4 `is_state_only` deletion |
| `crates/rpc/src/methods/governance.rs` | F6 |
| `bins/node/src/node/apply_block/governance.rs` | F6 |
| `crates/rpc/src/methods/transaction.rs` | F4 |
| `bins/node/src/node/validation_checks.rs` | F4 |
| `crates/mempool/src/pool.rs` | F4 |
| `crates/core/src/transaction/tests_price_attestation.rs` | F4 (test update) |
| `bins/node/tests/oracle_integration.rs` | F4 (test update) |
| `docs/rpc_reference.md`, `specs/engine-parts.md`, `specs/state-only-fee-gate-architecture.md` | docs |

### Must-be-untouched paths — all ZERO diff

Per-path `git diff 32e0a650 --stat -- <p>` and `git status --porcelain -- <p>`:

| Path | diff lines | porcelain | exists |
|---|---|---|---|
| `crates/updater` (tree) | 0 | 0 | yes |
| `bins/node/src/updater` (tree) | 0 | 0 | yes |
| `crates/core/src/maintainer/data.rs` | 0 | 0 | yes |
| `crates/core/src/maintainer/derivation.rs` | 0 | 0 | yes |
| `bins/node/src/commands/maintainer.rs` | 0 | 0 | yes |
| `bins/node/src/cli.rs` | 0 | 0 | yes |
| `bins/cli/src/cmd_upgrade.rs` | 0 | 0 | yes |
| `crates/storage/src/lib.rs` | 0 | 0 | yes |
| `crates/storage/src/maintainer.rs` | 0 | 0 | yes |
| `crates/storage/src/maintainer_wellformed.rs` | 0 | 0 | yes |
| `bins/node/src/run.rs` | 0 | 0 | yes |
| `bins/node/src/node/startup.rs` | 0 | 0 | yes |
| `crates/core/src/consensus/params.rs` | 0 | 0 | yes |
| `docs/cli.md` | 0 | 0 | yes |
| `specs/maintainer-trust-root-architecture.md` | 0 | 0 | yes |

Additionally verified beyond the required list: **`crates/core/src/network_params/`** is at
zero diff (0 porcelain, 0 diff lines), and `crates/core/src/maintainer/tests.rs` is at zero
diff. Activation-height values are unchanged: mainnet `u64::MAX`
(`defaults.rs:275`), testnet `133_000` (`:480`), devnet `0` (`:631`).

### Item 3 (Option E) — withdrawn cleanly

`MaintainerChangeData::signing_message` exists at `crates/core/src/maintainer/data.rs:46`
with signature `pub fn signing_message(&self, is_add: bool) -> Vec<u8>` — and
`git show 32e0a650:crates/core/src/maintainer/data.rs | grep -n signing_message` returns the
**same line 46**. `data.rs` is at zero diff, so the function is the PRE-EXISTING INC-I-172
M2 bearer-token form, not the Option E chain-bound form. `bound_to` = 0 hits in both roots.
Its test file `crates/core/tests/inc_i_172_m2_maintainer_governance.rs` is untouched.

### Item 4 (updater guard + rotation journal) — withdrawn cleanly

`crates/core/src/maintainer/` contains `data.rs derivation.rs digest.rs mod.rs set.rs
tests.rs` — **no `journal.rs`**. `crates/storage/src/` contains `maintainer.rs` and
`maintainer_wellformed.rs` only — **no `maintainer_journal.rs`**. No untracked `.rs` file
mentions `journal` / `Option E` / `bound_to` / `MaintainerRotation`.

## 2. Traceability Matrix Status

| Requirement | Priority | Has Tests | Tests Pass | Acceptance Met | Notes |
|---|---|---|---|---|---|
| REQ-173-011 (F7 cross-list totality) | Must | Yes | Yes (4/4) | Yes | `crates/core/tests/inc_i_173_m3_f7_cross_list.rs`; pins 24/24 variants |
| REQ-173-014 (F5 payload bounds) | Must | Yes | Yes (19/19) | Yes | `crates/core/tests/inc_i_173_m3_payload_bounds.rs`; both gate branches covered |
| F6 (AUDIT-P1-003, digest) | Recommended | Yes | Yes (17/17) | Yes | 10 core + 7 RPC |
| F4 (AUDIT-P3-002/003, routing) | Recommended | Yes | Yes (10/10) | Yes | `bins/node/tests/inc_i_173_m3_f4_routing.rs` |

**Gaps found:** none for the kept items. `MAX_MAINTAINER_CHANGE_ENCODED_BYTES` has only
test consumers, which is its stated purpose (O4).

## 3. Acceptance Criteria Results

### F5 / REQ-173-014 — payload bounds (Must) — PASS

`cargo test -p doli-core --test inc_i_173_m3_payload_bounds` → **19 passed, 0 failed.**

Gate implementation, `crates/core/src/validation/tx_types.rs:783-828`:

- Bound 1 (`extra_data` ≤ 1024 B) at `:784-792`, guarded by
  `ctx.current_height >= ctx.inc_i_173_activation_height` and evaluated **before**
  `MaintainerChangeData::from_bytes` — bincode never sees an attacker-sized buffer.
- Bound 2 (signature count ≤ `MAX_MAINTAINERS` = 5) at `:804-810`, inside the same gate.
- Bound 3 (`reason.len()` ≤ 256 **bytes**) at `:817-825`, inside the same gate.
- Call sites updated at `crates/core/src/validation/transaction.rs:171,174` for both
  `AddMaintainer` and `RemoveMaintainer`.

**Below-gate inertness (the load-bearing partition) — VERIFIED:**

| Test | What it proves |
|---|---|
| `req_173_014_oversized_extra_data_is_still_accepted_below_the_gate` | over-cap-but-decodable payload → `Ok` below the gate |
| `req_173_014_signature_count_over_the_cap_is_still_accepted_below_the_gate` | signature cap inert below the gate |
| `req_173_014_oversized_reason_is_still_accepted_below_the_gate` | reason cap inert below the gate |
| `req_173_014_emoji_reason_is_still_accepted_below_the_gate` | byte/char distinction absent below the gate |
| `req_173_014_garbage_is_rejected_by_the_decoder_below_the_gate` | 64 KiB garbage → `InvalidMaintainerChange` **from the DECODER**, asserting `!is_size_cap_rejection(&e)` so the message does **not** name the 1024-byte bound |

The last one is the anti-vacuity discriminator: paired with
`audit_p1_001_size_cap_runs_before_the_decoder` it shows the instrument distinguishes the
two branches rather than matching everything. **This is exactly the retroactive-vacuity
property required to ride the already-committed `inc_i_173_activation_height` without a new
height.** All three constants have live production consumers in `tx_types.rs` — none was
orphaned by the journal deletion.

### F6 — chain-derived digest — PASS, purely additive

`cargo test -p rpc --test inc_i_173_m3_maintainer_set_rpc` → **7 passed, 0 failed.**
`cargo test -p doli-core --test inc_i_173_m3_maintainer_digest` → **10 passed, 0 failed.**

Additivity confirmed two independent ways:

1. **Static** — `git diff 32e0a650 -- crates/rpc/src/methods/governance.rs` shows only `+`
   lines inside each `json!` object; no JSON key is on a `-` line. The `on-chain` branch
   gains `maintainer_set_digest` + `genesis_hash` (`:126-129`); the `derived` branch gains
   the same (`:204-205`); the `none` branch gains **`genesis_hash` only** (`:147`), with an
   explicit comment that a digest over an absent set "invites a comparison it cannot support".
2. **Live** — the deployed base binary (v6.24.1) at `127.0.0.1:8500` and `:8501` returns
   exactly the 10 pre-existing fields `enforced, initial_maintainer_count,
   last_change_block, maintainer_derivation_activation_height, maintainers,
   max_maintainers, member_count, min_maintainers, source, threshold` and **neither** new
   field. Both nodes agree. The three `*_keeps_every_existing_field` tests pin that set.

Digest is domain-separated (`b"DOLI-MAINTAINER-SET-V1"`,
`crates/core/src/maintainer/digest.rs:22`), chain-bound by `genesis_hash`, member-order
independent (sorted), and sensitive to member / threshold / `last_updated` / genesis.
`digest.rs` is a leaf module — the genesis hash arrives as `&[u8]`, so `maintainer` gains no
edge toward `chainspec`. The apply-path log anchor is
`bins/node/src/node/apply_block/governance.rs:167-180`, emitting
`MAINTAINER_SET_DIGEST=<hex>` after each applied rotation. It is **log-only** — no new
persistent state — so it does not re-extend the INC-I-174 rollback gap the way the deleted
journal did.

### F4 — routing on `is_zero_flow` — PASS

`cargo test -p doli-node --test inc_i_173_m3_f4_routing` → **10 passed, 0 failed.**

`Transaction::is_state_only()` is deleted at `crates/core/src/transaction/core.rs:456-477`
(22 `-` lines). `grep -rn 'fn is_state_only' bins crates` → **1 hit**, the negative
assertion at `inc_i_173_m3_f4_routing.rs:147`. The name survives only as prose or as an
unrelated local variable, never as a definition:

- `crates/core/src/validation/utxo.rs:239,249` — local binding `is_state_only_tx`; this
  file is at zero diff, so it is base content, not M3a residue.
- `crates/mempool/src/pool.rs:743-744`, `crates/rpc/src/methods/transaction.rs:206`,
  and 6 lines across three `crates/core/tests/*.rs` — comments explaining the deletion.

Both production callers route on `is_zero_flow()`:
`crates/rpc/src/methods/transaction.rs:203` and
`bins/node/src/node/validation_checks.rs:945-952`.

**Gate wiring is now complete over every non-test `ValidationContext::new` site — 6 of 6:**

| Site | Wired at |
|---|---|
| `bins/node/src/node/validation_checks.rs:103` | `:117` (header-only path; defensive) |
| `bins/node/src/node/validation_checks.rs:289` | `:334` (**consensus path — divergence fix**) |
| `bins/node/src/node/apply_block/tx_processing.rs:61` | `:98` (M1) |
| `bins/node/src/node/production/assembly.rs:186` | `:223` (M1) |
| `crates/mempool/src/pool.rs:363` | `:382` |
| `crates/mempool/src/pool.rs:766` | `:785` |

M3a wired the 4 remaining sites, matching the spec's "four remaining ValidationContext
sites". No site is silently weaker than its siblings (INV-VALIDATION-001).

### F7 / REQ-173-011 — cross-list totality (Must) — PASS

`cargo test -p doli-core --test inc_i_173_m3_f7_cross_list` → **4 passed, 0 failed.**
`TxType` has **24** enum discriminants (`crates/core/src/transaction/types.rs`), and
`req_173_011_total_table_is_self_consistent` asserts `rows.len() == 24`
(`inc_i_173_m3_f7_cross_list.rs:341-343`) — the table cannot silently under-cover if a
variant is added. `req_173_011_the_shape_probe_actually_fires` is the anti-vacuity guard
(both L1 and L2 must exclude a non-zero number of types, `:266-276`).

## 4. Exploratory Testing Findings

| # | What was tried | Expected | Actual | Severity |
|---|---|---|---|---|
| 1 | Look for an F5 constant whose only consumer was the deleted journal | every constant still has a production consumer | all three have live consumers in `tx_types.rs:785,804,818`. `MAX_MAINTAINER_CHANGE_ENCODED_BYTES` is test-only but that is its documented purpose | none |
| 2 | `git diff 32e0a650 \| grep '^+.*allow(dead_code)\|allow(unused'` | 0 | **0** — no suppression was added to paper over a hole | none |
| 3 | Search every untracked `.rs` for removed-item names | 0 files | **0 files** — no orphan test file survived the cut | none |
| 4 | `git diff --diff-filter=D` for files deleted vs base | 0 (revert restores, does not delete) | **0** | none |
| 5 | Verify `maintainer_set_digest` has ≥1 production consumer (not stranded by the cut) | ≥1 | 2 production sites: `apply_block/governance.rs:172` and `rpc/methods/governance.rs:128,205` | none |
| 6 | Challenge the pool.rs claim that the F4 `Registration` delta is bounded | claim holds or is a blocker | **holds** — `crates/core/src/validation/registration.rs:66-71` rejects a 0-input `Registration` post-genesis with "registration must have inputs for bond"; `is_in_genesis` is `height <= genesis_blocks` (`network/economics.rs:56-59`). Testnet at 135642 and mainnet are far past. | none |
| 7 | Probe the live testnet tip against the pinned testnet gate | tip below 133_000 | **tip 135642 — gate CROSSED by 2642 blocks** | medium (O2) |
| 8 | Compare `specs/SPECS.md` index text against the spec body status line | consistent | **inconsistent** (O1) | medium |

## 5. Failure Mode Validation

| Scenario | Triggered | Detected | Notes |
|---|---|---|---|
| Zero-fee unbounded payload flood above the gate (AUDIT-P1-001) | Yes, in-process | Yes | `audit_p1_001_signature_flood_is_rejected_above_the_gate`, `..._size_cap_runs_before_the_decoder` — the size cap fires ahead of bincode |
| Bound leaks below the gate and invalidates frozen history | Yes, in-process | Yes | 5 below-gate tests all assert acceptance / decoder-only rejection |
| Fleet trust-root divergence undetectable from logs | N/A (observability) | Yes | `MAINTAINER_SET_DIGEST=` anchor + RPC field |
| False divergence from member insertion order (AUDIT-P3-014) | Yes, in-process | Yes | `audit_p1_003_published_digest_is_independent_of_member_order`; live 8500/8501 return the same 5 keys in the same order, so the ordering hazard is latent not active on those two nodes |
| Cross-network digest collision (identical bootstrap arrays) | Yes, in-process | Yes | `audit_p1_003_published_digest_differs_between_mainnet_and_testnet` |
| Mempool/apply gate divergence above the height | Yes, in-process | Yes | `audit_p3_003_both_mempool_validation_contexts_carry_the_gate` |
| Rotation applied then reorged out (INC-I-174) | Not triggered | — | Out of M3a scope. **The M3a cut REVERSES M3's extension of this gap**: the journal that added a second unrolled-back file is gone; `apply_block/governance.rs` now only logs. The base-level gap remains, tracked as INC-I-174. |

## 6. Security Validation

| Surface | Test performed | Result |
|---|---|---|
| Unbounded fee-exempt payload (AUDIT-P1-001) | 1025-byte `extra_data`, 6-entry signature vector, 257-byte reason, 64 KiB garbage — above and below the gate | **PASS** — bounded above, inert below |
| Byte-vs-char reason cap bypass | `"\u{1F680}".repeat(100)` = 400 bytes / 100 chars | **PASS** — `audit_p1_001_reason_cap_counts_bytes_not_chars`; `tx_types.rs:818` uses `reason.len()` |
| Decoder DoS before the size check | oversized payload with the cap ordered ahead of `from_bytes` | **PASS** — `tx_types.rs:784-792` precedes `:795` |
| Digest collision / weak preimage | empty set, duplicate members, domain separation, cross-genesis | **PASS** — 10/10 in `inc_i_173_m3_maintainer_digest` |
| Digest leaks state on the `none` branch | inspect the `none` JSON | **PASS** — `genesis_hash` only, no digest (`governance.rs:137-148`) |
| Governance-tx replay across chains/states (Option E's target) | none | **Out of scope — WITHDRAWN.** The bearer-token `signing_message` is unchanged from base and remains an open risk, documented at `specs/state-only-fee-gate-architecture.md:499-500` |
| Release-signing trust root regression | hash-compare `crates/updater/src/trust_root.rs` | **PASS** — byte-identical to `32e0a650`; the INC-I-172 fail-closed blanket refusal is intact |

## 7. Specs/Docs Drift

| File | Documented behavior | Actual behavior | Severity |
|---|---|---|---|
| `specs/SPECS.md:43` | "INC-I-173, M1 IMPLEMENTED F1+F2+F3 / **F4-F7 + Options A-E PROPOSAL**" | F4–F7 are implemented; only Options A–E remain proposal, and Option E is withdrawn | medium |
| `docs/.workflow/inc-i-173-M3-implementation.md:19,40,166,503,983` | Item 4 rotation journal "**DONE**"; cites `crates/storage/src/maintainer_journal.rs` | Item 4 withdrawn; that file does not exist | low (workflow artefact) |
| `bins/node/src/node/validation_checks.rs:322` | "the live testnet tip measured **134_159**" | tip is **135642** (measured 2026-08-11 via `getChainInfo` @8500) | low (stale figure; the conclusion it supports — gate already crossed — is still correct and in fact stronger) |
| `specs/state-only-fee-gate-architecture.md:16-58` | "M3a IMPLEMENTED (F4+F5+F6+F7 only)"; Option E "built, then WITHDRAWN" | matches the tree | **no drift** |
| `specs/engine-parts.md:479` | `is_state_only` "DELETED in INC-I-173 M3 (F4/AUDIT-P3-002)" | matches | **no drift** |
| `docs/rpc_reference.md:1045-1088` | `maintainer_set_digest` on `on-chain`/`derived`, `genesis_hash` on all three, log anchor | matches the code | **no drift** |

## 8. Pre-Captured Gate Results (independently confirmed from logs, not re-run)

- **clippy** — `/tmp/m3a-clippy.log` ends `Finished dev profile … in 2m 10s`; `grep -c 'warning\|error'` = **0**.
- **full workspace test** — `/tmp/m3a-fulltest.log`: 149 `test result: ok` lines and 3 `FAILED` lines. `grep -n ' FAILED$'` gives exactly:
  - `:2142 test test_network::test_cluster_10x100 ... FAILED`
  - `:2716 test test_cluster_10x100 ... FAILED`
  - `:2909 test contention_tests::tests::inc_i_096_below_gate_rejects_remove_liquidity ... FAILED`

  These are exactly the 3 named known failures. **No M3a-related test appears in any failure block.**

## Blocking Issues

**None.**

## Non-Blocking Observations

- **OBS-1** (`specs/SPECS.md:43`) — update the index row to "M1 IMPLEMENTED (F1+F2+F3), M3a IMPLEMENTED (F4+F5+F6+F7), Options A–D PROPOSAL, Option E WITHDRAWN". The spec body is already correct; only the index drifted.
- **OBS-2** (deploy sequencing, not code) — the testnet gate `133_000` is **2642 blocks below** the live tip `135642`. On testnet the F4/F5 wiring goes live the instant the binary lands, so the testnet deploy must be a **synchronized stop-all-then-start-all**, never a rolling restart (INV-8 / INC-I-062). The source comment at `validation_checks.rs:310-330` already says this; refresh its stale `134_159` figure and close out its own "M2 must re-pin the testnet height above the then-current tip" action before deploy. Mainnet (`u64::MAX`) and devnet (`0`) are unaffected.
- **OBS-3** (`docs/.workflow/inc-i-173-M3-implementation.md`) — add a WITHDRAWN banner for items 3 and 4, or supersede the file with an M3a note. It is currently the only artefact still asserting the rotation journal shipped.
- **OBS-4** — `MAX_MAINTAINER_CHANGE_ENCODED_BYTES` is `pub` with test-only consumers. Intentional (derived-arithmetic anchor replacing prose that was wrong by 88 bytes in the unsafe direction). No change requested.
- **OBS-5** — `docs/qa/inc-i-173-M3-qa-report.md` and `docs/reviews/inc-i-173-M3-hardening-review.md` are untracked and describe the six-item M3. Keep them as history, but they should not be read as describing this branch.

## Modules Not Validated

- Options A–D and the withdrawn Option E remedy (chain-bound governance authorization) — deferred by design; the bearer-token replay risk is open and documented at `specs/state-only-fee-gate-architecture.md:499-500`.
- INC-I-174 (maintainer rotation not undone on reorg) — pre-existing at base; M3a reverts M3's extension of it but does not close it.
- Mainnet activation-height VALUE for `inc_i_173_activation_height` — still `u64::MAX`, decided at M4 per the spec's Activation Plan.

## Final Verdict

**PASS** — All Must requirements (REQ-173-011, REQ-173-014) are met, the reduction is
byte-verifiably complete with no removed-item residue in any source root, the INC-I-172
blanket-refusal trust root is restored bit-for-bit, and the four kept items pass 50/50
targeted tests with the pre-captured build, clippy, and workspace-test gates clean apart
from 3 independently confirmed known failures. Approved for commit. Address OBS-1 with the
commit and settle OBS-2 before any testnet deploy.
