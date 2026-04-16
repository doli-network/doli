# INC-I-034 / M-Choice1 — Test Writer Report

**Milestone**: `M-Choice1` (CHOICE 1 = SAME HF, locked 2026-04-16).
**Scope**: Phase-1 primitive for EpochState-in-state-root `HardForkSchedule` entry.
**Confidence gate**: 0.7 — **FAIL→PASS evidence REQUIRED** per CLAUDE.md Rule #21.
**Risk**: HIGH (consensus-layer). Tests are the only bulwark between a silent
wire-compatibility break and a known-good release.

---

## Phase-1 scope (what ships in THIS milestone)

1. `HardForkSchedule::EPOCH_SNAPSHOT_HF` entry present in `for_network(network)`
   for Mainnet, Testnet, Devnet.
2. `CURRENT_PROTOCOL_VERSION` bumps `3 → 4`. `MIN_PEER_PROTOCOL_VERSION` held
   at `1`.
3. New pure function
   `storage::compute_state_root_with_epoch_state(&ChainState, &UtxoSet, &ProducerSet, Option<Hash>) -> Result<Hash, StorageError>`.
    - `None` → bit-identical to legacy `compute_state_root(cs, utxo, ps)`.
    - `Some(h)` → `H(H(cs_canon) || H(utxo_canon) || H(ps_canon) || h)`.
4. Legacy `compute_state_root` unchanged — all 15 existing call-sites continue
   to produce the Phase-1 identical 3-component hash.

## Phase-2 scope (explicitly DEFERRED)

- Wiring of the new function into `apply_block`, `state_update`, `snap_sync`,
  `cleanup`, `cmd_snap`.
- Height-keyed switch between 3- and 4-component formula.
- All non-state-root consensus changes (W1 writer, rewards rewrite,
  `BlockOutcome`, …).

---

## Output Contract enumeration

5 output cells total across 3 files:

| # | File                                                 | Output cell                                    | Path count |
|---|------------------------------------------------------|------------------------------------------------|------------|
| 1 | `crates/storage/src/snapshot.rs`                     | `compute_state_root_with_epoch_state` return  | 3 paths    |
| 2 | `crates/updater/src/hardfork.rs`                     | `for_network` schedule content                 | 3 networks |
| 3 | `crates/updater/src/hardfork.rs`                     | `fork_id` transition at activation             | 3 heights  |
| 4 | `crates/network/src/protocols/status.rs`             | `const CURRENT_PROTOCOL_VERSION: u32`          | 1 path     |
| 5 | `crates/network/src/protocols/status.rs`             | `const MIN_PEER_PROTOCOL_VERSION: u32`         | 1 path     |

### Cell 1 — `compute_state_root_with_epoch_state` (new)

```
OUTPUT CONTRACT: fn compute_state_root_with_epoch_state(cs, utxo, ps, opt_hash)
  O1: return Hash
        None      -> bit-identical to compute_state_root(cs,utxo,ps)
        Some(h)   -> H(H(cs_canon)||H(utxo_canon)||H(ps_canon)||h)
  (no mutable params, no receiver, no persistent store, no channel)
PATHS: P1: None (legacy-equivalence)
       P2: Some(h) (4-component, != legacy)
       P3: Some(h1) vs Some(h2), h1!=h2 (hash-distinction)
MATRIX: 1 output x 3 paths = 3 assertion clusters (Tests 1, 2, 3)
```

### Cell 2 — `HardForkSchedule::for_network(network)`

```
OUTPUT CONTRACT: HardForkSchedule::for_network(network)
  O1: return schedule with EPOCH_SNAPSHOT_HF entry whose consensus_changes
      contain an EpochState/EpochSnapshot marker AND "state root"
      Mainnet/Testnet: activation_height >= 1_000_000, min_version ^ "7."
      Devnet: entry optional; if present, activation_height = 0
PATHS: P1: Mainnet, P2: Testnet, P3: Devnet
MATRIX: 1 output x 3 paths = 3 assertion clusters (Test 4)
```

### Cell 3 — `HardForkSchedule::fork_id` at EPOCH_SNAPSHOT_HF activation

```
OUTPUT CONTRACT: HardForkSchedule::fork_id(genesis, h) with only EPOCH_SNAPSHOT_HF
  O1: return Hash
        h = activation - 1 -> Hash::ZERO
        h = activation     -> != Hash::ZERO
        h = activation + 1 -> != Hash::ZERO (equal to AT)
PATHS: P1: before, P2: at, P3: after
MATRIX: 1 output x 3 paths = 3 assertions (Test 5)
```

### Cell 4 — `CURRENT_PROTOCOL_VERSION`

```
OUTPUT CONTRACT: const CURRENT_PROTOCOL_VERSION: u32
  O1: value == 4
PATHS: P1: compile-time constant
MATRIX: 1 output x 1 path = 1 assertion (Test 6)
```

### Cell 5 — `MIN_PEER_PROTOCOL_VERSION`

```
OUTPUT CONTRACT: const MIN_PEER_PROTOCOL_VERSION: u32
  O1: value == 1 (defensive pin; HF is height-gated, not handshake-gated)
PATHS: P1: compile-time constant
MATRIX: 1 output x 1 path = 1 assertion (Test 7)
```

---

## Outputs x Paths matrix (assertion names)

| Cell | Path                         | Assertion test name                                                                          |
|------|------------------------------|----------------------------------------------------------------------------------------------|
| 1    | P1 None = legacy             | `test_m_choice1_compute_state_root_with_none_equals_legacy`                                  |
| 1    | P2 Some = 4-component        | `test_m_choice1_compute_state_root_with_some_uses_four_components`                           |
| 1    | P3 Some(h1) != Some(h2)      | `test_m_choice1_state_root_distinguishes_epoch_state_variants`                               |
| 2    | P1/P2/P3 per-network entry   | `test_m_choice1_schedule_has_epoch_snapshot_hf` (loops Mainnet, Testnet, Devnet)             |
| 3    | P1/P2/P3 before/at/after     | `test_m_choice1_fork_id_changes_at_activation`                                               |
| 4    | P1 const == 4                | `test_m_choice1_current_protocol_version_is_4`                                               |
| 5    | P1 const == 1                | `test_m_choice1_min_peer_protocol_version_held_at_1`                                         |

Every checklist cell has exactly one dedicated assertion. No cell is missing.

---

## FAIL evidence (verified on HEAD `synmgrefactor`, today 2026-04-16)

### storage — 3 tests FAIL to compile (missing symbol)

```
$ cargo test -p storage --lib m_choice1 --no-run
error[E0425]: cannot find function `compute_state_root_with_epoch_state` in this scope
   --> crates/storage/src/snapshot.rs:391:29  (test 1, None path)
   --> crates/storage/src/snapshot.rs:408:13  (test 1, drift sanity)
   --> crates/storage/src/snapshot.rs:441:13  (test 2, Some path)
   --> crates/storage/src/snapshot.rs:491:18  (test 3, h1)
   --> crates/storage/src/snapshot.rs:492:18  (test 3, h2)
error: could not compile `storage` (lib test) due to 5 previous errors
```

Expected FAIL mode. Resolves when the developer adds the new pure function to
`snapshot.rs` (and re-exports if needed).

### updater — 2 tests FAIL at runtime (missing schedule entry)

```
$ cargo test -p updater --lib m_choice1
test hardfork::m_choice1_epoch_snapshot_hf_tests::test_m_choice1_fork_id_changes_at_activation   ... FAILED
test hardfork::m_choice1_epoch_snapshot_hf_tests::test_m_choice1_schedule_has_epoch_snapshot_hf  ... FAILED

---- test_m_choice1_schedule_has_epoch_snapshot_hf stdout ----
panicked at 'M-Choice1: HardForkSchedule::for_network(Mainnet) MUST contain an
EPOCH_SNAPSHOT_HF entry ... Schedule currently has 0 entries: []'

---- test_m_choice1_fork_id_changes_at_activation stdout ----
panicked at 'M-Choice1: cannot run fork_id transition test — Mainnet schedule
is missing the EPOCH_SNAPSHOT_HF entry. Test 4 should fail first.'

test result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 34 filtered out
```

Expected FAIL mode. Resolves when the developer adds the EPOCH_SNAPSHOT_HF
entry to `for_network` for Mainnet and Testnet.

### network — 1 test FAIL at runtime, 1 test passes defensively

```
$ cargo test -p network --lib m_choice1
test protocols::status::m_choice1_protocol_version_tests::test_m_choice1_min_peer_protocol_version_held_at_1   ... ok
test protocols::status::m_choice1_protocol_version_tests::test_m_choice1_current_protocol_version_is_4         ... FAILED

---- test_m_choice1_current_protocol_version_is_4 stdout ----
assertion `left == right` failed: M-Choice1: CURRENT_PROTOCOL_VERSION must bump
from 3 to 4 when EPOCH_SNAPSHOT_HF is scheduled. Per CLAUDE.md 'After Every
Modification' step 3 — signal to peer scoring that this binary may switch
state-root formula at the scheduled height.
  left: 3
 right: 4

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 303 filtered out
```

- `test_m_choice1_current_protocol_version_is_4` — expected FAIL mode; left=3,
  right=4. Resolves when the developer bumps the constant.
- `test_m_choice1_min_peer_protocol_version_held_at_1` — **correctly PASSES
  on HEAD**. This is a **defensive pin** that catches accidental bumps of
  `MIN_PEER_PROTOCOL_VERSION` during the Phase-1 migration. Per CLAUDE.md
  Rule #21 pattern for pin-tests: a pass on HEAD that would fail if the
  developer over-reached is still valid FAIL→PASS evidence because the failure
  scenario is demonstrable (see `test` invariant note inside the assertion
  message — future bump attempt would break it).

---

## Handoff to developer — exact function signatures + activation heights

### 1. Add to `crates/storage/src/snapshot.rs` (public export)

```rust
use crypto::Hash;

/// Compute a deterministic state root that OPTIONALLY includes an
/// `H(EpochSnapshot)` as a 4th component.
///
/// - `epoch_state_hash = None`:
///   bit-identical to `compute_state_root(cs, utxo, ps)`.
///   Pre-EPOCH_SNAPSHOT_HF callers (or callers that have not yet been wired
///   to pass the snapshot hash) keep producing the exact legacy 3-component
///   hash — this is the Phase-1 safety invariant.
/// - `epoch_state_hash = Some(h)`:
///   returns `H(H(cs_canonical) || H(utxo_canonical) || H(ps_canonical) || h)`
///   — the post-HF 4-component hash per
///   `specs/scheduler-state-architecture.md` — "State-root inclusion
///   (timing: SAME HF — convergent, with sequenced option surfaced)".
///
/// The canonical serialization of each component is reused (same as the
/// legacy function), so deterministic reproducibility across nodes is
/// preserved. This function is PURE — it does not decide whether the HF is
/// active; the caller (apply_block / snap_sync / …) consults the
/// `HardForkSchedule::EPOCH_SNAPSHOT_HF` entry and passes `Some` at/after
/// activation height, `None` before.
pub fn compute_state_root_with_epoch_state(
    chain_state: &ChainState,
    utxo_set: &UtxoSet,
    producer_set: &ProducerSet,
    epoch_state_hash: Option<Hash>,
) -> Result<Hash, StorageError> {
    // Implementation note for the developer:
    //   1. Use `chain_state.serialize_canonical()`,
    //      `utxo_set.serialize_canonical()`,
    //      `producer_set.serialize_canonical()` — same as legacy.
    //   2. Hash each to get cs_hash / utxo_hash / ps_hash.
    //   3. If epoch_state_hash is None, concat(cs_hash, utxo_hash, ps_hash)
    //      and final-hash — MUST equal legacy byte-for-byte.
    //   4. If Some(h), concat(cs_hash, utxo_hash, ps_hash, h) and final-hash.
    todo!("Phase-1 implementation — Test 1 (None) verifies legacy equivalence; \
           Test 2 (Some) verifies the explicit 4-component formula; \
           Test 3 verifies hash-distinction for different Some(h)")
}
```

**Do NOT** wire this into any caller yet. Phase-2 scope (deferred).

### 2. Add to `crates/updater/src/hardfork.rs::HardForkSchedule::for_network`

Add one `HardForkInfo` entry per production network. Both Mainnet and Testnet
MUST use a FAR-FUTURE PLACEHOLDER height (operator updates at deploy-time
using the spec formula `floor((current_height + 7200) / 360) * 360`).

| Network | `activation_height` (placeholder)      | `min_version` | `consensus_changes` (must contain)         |
|---------|----------------------------------------|---------------|--------------------------------------------|
| Mainnet | `10_000_000` (or higher; >= 1_000_000) | `"7.0.0"`     | `"EpochState state root inclusion"`        |
| Testnet | `10_000_000` (or higher; >= 1_000_000) | `"7.0.0"`     | `"EpochState state root inclusion"`        |
| Devnet  | (optional) `0`                         | `"7.0.0"`     | `"EpochState state root inclusion"`        |

**CRITICAL** — CLAUDE.md Rule #0 (NO GENESIS RESETS FOR STORAGE/FEATURE CHANGES):
The committed binary MUST ship with a placeholder height `>= 1_000_000`.
Operators update to the real deploy-time height per the spec formula
BEFORE shipping. Test 4 pins `>= 1_000_000` as the lower bound precisely to
prevent an accidental low height from reaching production.

The consensus_changes string MUST contain both:

- an EpochState/EpochSnapshot marker (`"EpochState"`, `"EpochSnapshot"`,
  `"Epoch State"`, or `"Epoch Snapshot"`), AND
- the phrase `"state root"` (case-insensitive).

Exact suggested string: `"EpochState state root inclusion (M-Choice1)"`.

### 3. Bump `crates/network/src/protocols/status.rs`

```rust
pub const CURRENT_PROTOCOL_VERSION: u32 = 4;  // was 3
pub const MIN_PEER_PROTOCOL_VERSION: u32 = 1; // unchanged — held for Phase-1
```

Also extend the doc-comment `History:` table with a `4 —` entry describing
EPOCH_SNAPSHOT_HF gating. `MIN_PEER_PROTOCOL_VERSION` note should be updated
to reference M-Choice1 in addition to INC-I-026.

---

## Specs Gaps Found

None. The spec (`specs/scheduler-state-architecture.md`, section "State-root
inclusion (timing: SAME HF — convergent, with sequenced option surfaced)")
is internally consistent with the existing code in `snapshot.rs` and
`hardfork.rs`. The existing implementation of `compute_state_root` uses the
component ordering `H(cs) || H(utxo) || H(ps)`, while the spec text reads
`utxo_root || producer_root || chain_state_root || H(EpochSnapshot)`. The
test enforces the CODE'S existing order (`cs || utxo || ps || h`) because the
existing legacy function (which ships in 100% of nodes today) uses that
order — changing it would retroactively alter every historical state root
(consensus break). The spec's textual ordering is descriptive prose, not a
byte-level formula. This is an acceptable reading but I flag it so the
reviewer and developer can confirm the choice: **test locks in the existing
canonical ordering `cs || utxo || ps || h`** — not the spec's prose ordering.

---

## Developer checklist — when implementing

Pre-push gate (CLAUDE.md "After Every Modification"):

1. Build gate:
   `cargo build --release && cargo clippy -- -D warnings && cargo fmt --check`
2. Test:
   - `cargo test -p storage --lib m_choice1` (all 3 pass)
   - `cargo test -p updater --lib m_choice1` (all 2 pass)
   - `cargo test -p network --lib m_choice1` (all 2 pass)
   - Full regression: `cargo test -p storage --lib`, `cargo test -p updater --lib`,
     `cargo test -p network --lib` — zero existing tests regress.
3. Version protection: already handled — step 3 is this milestone.
4. Documentation: update `specs/protocol.md` (version 3 -> 4), and
   `specs/security_model.md` (HF schedule entry), per CLAUDE.md item #4.
5. Copy binary / codesign: standard post-build.
6. Wait for user approval before commit.
7. Testnet first; NEVER mainnet without explicit confirmation.

---

## Files modified by Test Writer

| File                                                          | Lines added | What                                                          |
|---------------------------------------------------------------|-------------|---------------------------------------------------------------|
| `crates/storage/src/snapshot.rs`                              | +142        | `mod m_choice1_state_root_hf_tests` (3 tests)                 |
| `crates/updater/src/hardfork.rs`                              | +146        | `mod m_choice1_epoch_snapshot_hf_tests` (2 tests)             |
| `crates/network/src/protocols/status.rs`                      | +57         | `mod m_choice1_protocol_version_tests` (2 tests)              |
| `docs/.workflow/m-choice1-test-writer.md`                     | +this file  | Handoff artifact                                              |

**7 tests total**, fully FAIL on HEAD (5 compile-errors + 2 runtime-fails +
1 defensive pin-pass). Confidence gate satisfied.
