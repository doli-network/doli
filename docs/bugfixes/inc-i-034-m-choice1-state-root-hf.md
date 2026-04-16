# INC-I-034 / M-Choice1 — EpochState-in-state-root HardForkSchedule entry (Phase-1)

**Incident**: INC-I-034 (scheduler + state-root redesign cascade)
**Milestone**: `M-Choice1` — CHOICE 1 = SAME HF (locked 2026-04-16)
**Branch**: `synmgrefactor` (no commit; orchestrator will commit with author
`Antonio Lozada <antonio@omegacortex.ai>`)
**Spec**: `specs/scheduler-state-architecture.md`,
section "State-root inclusion (timing: SAME HF — convergent, with sequenced
option surfaced)" and migration path "Phase 1: Pre-activation" items 3
(protocol version bump) and 6 (schedule entry).
**Confidence gate**: 0.7 — FAIL→PASS evidence provided for every new test.

---

## Scope (this milestone)

Phase-1 scheduling ONLY. Three surgical changes across three files. Zero
call-site wiring of the new state-root primitive (that is Phase-2 scope —
separate milestone). Zero retroactive change to any existing state root.

1. **`crates/storage/src/snapshot.rs`**: add new pure function
   `compute_state_root_with_epoch_state(&ChainState, &UtxoSet, &ProducerSet,
   Option<Hash>) -> Result<Hash, StorageError>`.
   - `None` → bit-identical to the existing `compute_state_root(cs, utxo, ps)`.
     Pre-HF callers continue producing the exact legacy 3-component hash.
   - `Some(h)` → returns
     `H(H(cs_canonical) || H(utxo_canonical) || H(ps_canonical) || h)` per
     the spec formula. Intended for post-`EPOCH_SNAPSHOT_HF` activation use.
   The legacy `compute_state_root` is NOT modified; every existing chain
   hashes identically.

2. **`crates/updater/src/hardfork.rs`**: `HardForkSchedule::for_network` now
   adds an `EPOCH_SNAPSHOT_HF` entry for **Mainnet** and **Testnet**:
   - `activation_height`: `10_000_080` (far-future placeholder,
     `= 27_778 * 360` — epoch-aligned with `BLOCKS_PER_EPOCH = 360`).
   - `min_version`: `"7.0.0"` — major-version bump, required so peer-scoring
     can partition legacy binaries at the gate.
   - `consensus_changes`: `["EpochState state root inclusion (M-Choice1)"]`.
   Devnet: no entry (devnet resets constantly and exercises activation paths
   directly via test fixtures).

3. **`crates/network/src/protocols/status.rs`**: bump
   `CURRENT_PROTOCOL_VERSION` from `3` to `4`. `MIN_PEER_PROTOCOL_VERSION`
   held at `1` — Phase-1 is pre-activation, so v2/v3 peers remain
   wire-compatible with v4 binaries until the height gate trips.

## Explicitly NOT in scope

- Wiring `compute_state_root_with_epoch_state` into any of the 15 current
  `compute_state_root` call-sites. That is **Phase-2**.
- Height-keyed dispatch (consult schedule, pass `Some` or `None`). Phase-2.
- Any non-state-root consensus change (W1 writer, rewards rewrite,
  `BlockOutcome`, `BlacklistDecision` asymmetric, RPC consumer audit).
  Separate milestones.
- Any modification of the legacy `compute_state_root` function body — it
  must remain byte-identical, and it is.

---

## Why the placeholder activation height

**CLAUDE.md Rule #0**: NO genesis reset for storage/feature changes. Bitcoin
and DOLI activate features forward-only. The `10_000_080` placeholder is
chosen to satisfy three constraints simultaneously:

- **Safely >> current tip on every network** (mainnet is under ~40k;
  testnet/devnet lower). If shipped as-is, the HF never actually triggers
  on a running chain — harmless Phase-1 behavior.
- **Epoch-aligned**: `10_000_080 = 27_778 * 360` where
  `BLOCKS_PER_EPOCH = 360`. Matches the spec's operational formula
  `floor((current_height + 7200) / 360) * 360`.
- **Above the `FAR_FUTURE_MIN = 1_000_000` lower bound** the test-writer
  pins (`test_m_choice1_schedule_has_epoch_snapshot_hf`). The bound exists
  precisely to prevent an accidental low height from reaching production.

Operators compute and write the real activation height at deploy time (see
"Operator deploy checklist" below).

---

## Three-layer coverage per F-16

| Layer | Phase-1 contribution | Phase-2 responsibility (deferred) |
|-------|----------------------|------------------------------------|
| (a) Trigger removal | — | Switch the 15 `compute_state_root` call-sites to consult `HardForkSchedule::EPOCH_SNAPSHOT_HF` and pass `Some(H(EpochSnapshot))` post-activation. |
| (b) Recovery        | — | O(1) state-root mismatch surfaces at block apply the moment a pre-HF binary tries to process a post-HF block (or vice versa), thanks to the 4-component vs 3-component distinction pinned in the new function. |
| (c) Architectural invariant | `EpochSnapshot` hash is now a first-class input available to the state-root computation, and the schedule entry is committed to consensus. The primitive exists in the binary; the schedule exists for all peers. | Enforce the invariant by wiring the call-sites and making the schedule gate authoritative. |

Phase-1 lays the rails; Phase-2 runs the train.

---

## Test evidence (FAIL → PASS)

### Before (HEAD of `synmgrefactor`, pre-implementation)

```
$ cargo test -p storage --lib m_choice1 --no-run
error[E0425]: cannot find function `compute_state_root_with_epoch_state` in this scope
   --> crates/storage/src/snapshot.rs:391:29  (test 1, None path)
   --> crates/storage/src/snapshot.rs:408:13  (test 1, drift sanity)
   --> crates/storage/src/snapshot.rs:441:13  (test 2, Some path)
   --> crates/storage/src/snapshot.rs:491:18  (test 3, h1)
   --> crates/storage/src/snapshot.rs:492:18  (test 3, h2)
error: could not compile `storage` (lib test) due to 5 previous errors

$ cargo test -p updater --lib m_choice1
test hardfork::m_choice1_epoch_snapshot_hf_tests::test_m_choice1_fork_id_changes_at_activation   ... FAILED
test hardfork::m_choice1_epoch_snapshot_hf_tests::test_m_choice1_schedule_has_epoch_snapshot_hf  ... FAILED
... panicked at 'M-Choice1: HardForkSchedule::for_network(Mainnet) MUST contain an EPOCH_SNAPSHOT_HF entry ...'
test result: FAILED. 0 passed; 2 failed

$ cargo test -p network --lib m_choice1
test protocols::status::m_choice1_protocol_version_tests::test_m_choice1_current_protocol_version_is_4         ... FAILED
  left: 3
 right: 4
test protocols::status::m_choice1_protocol_version_tests::test_m_choice1_min_peer_protocol_version_held_at_1   ... ok
test result: FAILED. 1 passed; 1 failed
```

### After (this milestone)

```
$ cargo test -p storage --lib m_choice1
running 3 tests
test snapshot::m_choice1_state_root_hf_tests::test_m_choice1_compute_state_root_with_some_uses_four_components ... ok
test snapshot::m_choice1_state_root_hf_tests::test_m_choice1_compute_state_root_with_none_equals_legacy ... ok
test snapshot::m_choice1_state_root_hf_tests::test_m_choice1_state_root_distinguishes_epoch_state_variants ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 170 filtered out

$ cargo test -p updater --lib m_choice1
running 2 tests
test hardfork::m_choice1_epoch_snapshot_hf_tests::test_m_choice1_fork_id_changes_at_activation ... ok
test hardfork::m_choice1_epoch_snapshot_hf_tests::test_m_choice1_schedule_has_epoch_snapshot_hf ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 34 filtered out

$ cargo test -p network --lib m_choice1
running 2 tests
test protocols::status::m_choice1_protocol_version_tests::test_m_choice1_current_protocol_version_is_4 ... ok
test protocols::status::m_choice1_protocol_version_tests::test_m_choice1_min_peer_protocol_version_held_at_1 ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 303 filtered out
```

### Full-suite regression (no existing test regresses)

```
$ cargo test -p storage --lib
test result: ok. 173 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p updater --lib
test result: ok. 36 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p network --lib
test result: ok. 304 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out

$ cargo test -p doli-node --lib      # consumer sanity
test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

### Build gate

```
$ cargo build --release --workspace
Finished `release` profile [optimized] target(s) in 1m 41s

$ cargo clippy --workspace -- -D warnings
Finished `dev` profile [optimized + debuginfo] target(s) in 11.84s
```

Clippy clean across workspace (`-D warnings`).

### `cargo fmt --check` status

Pre-existing drift present on HEAD (stash-verified):

- 9 diffs in `bins/node/tests/m_rc10_apply_after_reject_regression.rs` and
  `bins/node/tests/m_rc11_fork_guard_backfill_regression.rs` — prior
  M-RC10/M-RC11 milestones, unrelated to this milestone.
- 4 diffs in the test-writer's newly added M-Choice1 test modules inside
  `crates/storage/src/snapshot.rs:475` and
  `crates/updater/src/hardfork.rs:407,453,462` — introduced by the
  Test-Writer step. Per developer constraints ("DO NOT modify tests"), I
  leave these as-is; orchestrator or a fmt-only follow-up can resolve them.

My own new source code (the `for_network` body and the new function) passes
`cargo fmt --check` cleanly.

---

## Why a protocol version bump (CLAUDE.md "After Every Modification" step 3)

Bumping `CURRENT_PROTOCOL_VERSION: 3 -> 4` is the Phase-1 signal to
peer-scoring that THIS binary is capable of handling the height-gated
state-root transition. A v3 binary that joined a fleet of v4 binaries and
crossed the activation height would silently fork, because v3 has no
knowledge of `compute_state_root_with_epoch_state` or `EPOCH_SNAPSHOT_HF`.
Peer-scoring can now distinguish v4-capable from v3-incapable binaries
without forcing immediate wire partition (that's what
`MIN_PEER_PROTOCOL_VERSION` is for, and it is deliberately HELD at 1 until
Phase-2 activation has safely cleared mainnet).

## Why bundle the `HardForkSchedule` entry with the protocol version bump

Phase-1 is a single atomic release: the binary carries **both** the new
primitive AND the schedule entry describing the future activation. If a
binary shipped with one but not the other, two failure modes would open:

- v4 + entry, no primitive: at activation, state-root computation panics.
- v4 + primitive, no entry: primitive is dead code; Phase-2 has nowhere to
  gate. Peers can't discover that this binary will cross the gate.

Keeping them in one release — gated by the version bump — collapses both
failure modes.

---

## Operator deploy checklist (NEW — authoritative)

Before any mainnet/testnet deploy of a binary carrying these changes:

1. **Pre-testnet-deploy — compute real activation height**:
   `testnet_activation_height = floor((testnet_tip + 7200) / 360) * 360`.
   Update `crates/updater/src/hardfork.rs` Testnet arm.

2. **Validate REQ-REDESIGN-001 on testnet**: after testnet activation,
   verify byte-identical state root vs the 6.13.28 reference for at least
   3 consecutive epochs. (Phase-2 actually lights up the 4-component
   formula; in Phase-1 alone the binary should be state-root-neutral.)

3. **Pre-mainnet-deploy — compute real activation height**: same formula as
   testnet, with **24-hour lead time** for fleet upgrade verification.
   Update `crates/updater/src/hardfork.rs` Mainnet arm.

4. **Broadcast binary to ALL seeds + producers 24h before activation**.
   Confirm via RPC:
   - `getProtocolVersion` returns `4` on every node.
   - `getHardForkSchedule` (or equivalent) lists the `EPOCH_SNAPSHOT_HF`
     entry at the expected height on every node.

5. **At `activation_height`**: pre-HF binaries compute state root via the
   legacy 3-component formula; post-HF binaries compute 4-component. Pre-HF
   nodes partition off via state-root mismatch on the first block
   post-activation. This is the designed behavior — it is the safety net.

6. **Testnet first; NEVER mainnet without explicit confirmation** per
   CLAUDE.md "After Every Modification" step 7.

---

## Files touched (strict whitelist honored)

| File | Kind | Source LOC delta | Notes |
|------|------|------------------|-------|
| `crates/storage/src/snapshot.rs` | source | +71 | New pure function `compute_state_root_with_epoch_state`; legacy `compute_state_root` byte-identical. |
| `crates/network/src/protocols/status.rs` | source | +10 / -1 | `CURRENT_PROTOCOL_VERSION 3 -> 4`, extended doc history, refreshed `MIN_PEER_PROTOCOL_VERSION` doc. |
| `crates/updater/src/hardfork.rs` | source | +40 / -5 | `for_network` now seeds Mainnet + Testnet with `EPOCH_SNAPSHOT_HF`; Devnet unchanged (no entry). |
| `docs/bugfixes/inc-i-034-m-choice1-state-root-hf.md` | doc | +this file | — |
| `docs/.workflow/milestone-progress.md` | doc | 1 row updated | M-Choice1 row: PENDING → COMPLETE (local, pending commit). |

Tests (already landed by the Test-Writer step):
- `crates/storage/src/snapshot.rs` — `mod m_choice1_state_root_hf_tests` (3 tests)
- `crates/updater/src/hardfork.rs` — `mod m_choice1_epoch_snapshot_hf_tests` (2 tests)
- `crates/network/src/protocols/status.rs` — `mod m_choice1_protocol_version_tests` (2 tests)

No file outside this whitelist was modified.

---

## Call-site reliance on `CURRENT_PROTOCOL_VERSION == 3` (audit)

Grep across the workspace for `CURRENT_PROTOCOL_VERSION` and `== 3` /
`== 3u32` found **no tests or consumers that pin the literal `3`** outside
the status.rs History doc-comment and the test-writer's new Test 6 (which
pins `== 4`). Consumers:

- `crates/network/src/service/behaviour_events.rs:30, 426, 491` — uses the
  constant by name (propagates the bump automatically).
- `crates/network/src/protocols/mod.rs:11` — re-export only.
- `crates/network/src/protocols/status.rs:86, 96, 113, 133` — uses the
  constant in `StatusRequest::new` / `StatusResponse::new` constructors
  (propagates the bump automatically).

**No regressions** — every existing consumer picks up `4` without edits,
which is consistent with the clean `cargo test -p network --lib` (304/304)
and `cargo test -p doli-node --lib` (20/20) results.

---

## Handoff notes for QA / reviewer

1. **Confirm fmt drift in test modules is out-of-scope for this milestone.**
   Developer constraint forbids modifying test code; the test-writer
   introduced the drift, and an orchestrator fmt pass is the appropriate
   cleanup channel.

2. **Verify `compute_state_root` byte-identical**: the legacy function at
   `crates/storage/src/snapshot.rs:24-59` was NOT modified. Any divergence
   would silently reshape all existing state roots — reviewer should
   re-grep the diff to confirm.

3. **Verify NO call-site wiring leaked**: grep the diff for any file
   outside the 3-source + 2-docs whitelist. There should be zero matches.
   `git diff HEAD -- $(git ls-files | grep -v "docs/\|test") | grep "^+++"`
   is a useful one-liner.

4. **Devnet entry absent by design** — confirm
   `HardForkSchedule::for_network(Network::Devnet)` returns an empty
   schedule. Test-writer's Test 4 codifies this.

5. **Phase-2 next**: the obvious follow-up is to port the 15 current
   `compute_state_root` call-sites to consult the schedule. That is NOT in
   this milestone's scope; flag it on the milestone-progress backlog.

6. **Placeholder activation height (`10_000_080`)** is a ship-time
   decision, not a test-time decision. The test-writer's
   `FAR_FUTURE_MIN = 1_000_000` guard gives operators a ~99-epoch cushion
   before it bites. Operators MUST update per formula before real deploy.

---

## Deployment checklist (CLAUDE.md "After Every Modification")

1. **Build gate**: `cargo build --release && cargo clippy -- -D warnings`
   — PASS. `cargo fmt --check` — test-writer residue out of scope.
2. **Tests**: all `m_choice1` tests pass; storage/updater/network/doli-node
   full lib suites regress-clean. Evidence block above.
3. **Version protection**: `CURRENT_PROTOCOL_VERSION: 3 -> 4` — done.
   `HardForkSchedule::EPOCH_SNAPSHOT_HF` entry at far-future placeholder —
   done. `MIN_PEER_PROTOCOL_VERSION` held at 1 — correct.
4. **Documentation alignment**: this report is the primary artifact.
   `docs/.workflow/milestone-progress.md` updated. `specs/protocol.md` and
   `specs/security_model.md` updates deferred to orchestrator (it's closer
   to the commit + code-review path and can fold them into the commit
   message if desired).
5. **Binary copy / codesign**: deferred — no deploy this milestone.
6. **Commit + push**: **orchestrator responsibility**. Developer did NOT
   commit per explicit milestone constraint.
7. **Deploy consideration**: testnet first, NEVER mainnet without explicit
   confirmation. This Phase-1 change is state-root-neutral if the
   placeholder height is left in place, which means it is safe to deploy
   as a fleet-upgrade-verification beacon.
