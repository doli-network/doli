# INC-I-034 — M-RC9: Silent `vec![]` in `calculate_epoch_rewards`

**Date:** 2026-04-16
**Branch:** `synmgrefactor`
**File touched:** `bins/node/src/node/rewards.rs`
**Tests:** `bins/node/tests/m_rc9_silent_vec_regression.rs` (3 tests, all PASS)
**Incident:** INC-I-034 (hydra pattern, 24 prior runs, live mainnet cascade 2026-04-16)

## Symptom

At 2026-04-16 a producer (Santiago, block range 39600–39628 missing locally)
computed an `EpochReward` transaction whose output set **silently omitted** a
subset of qualifying producers. Peers with complete block_stores computed
a different output set; the block was rejected by validators and the
network briefly forked.

## Root cause

`Node::calculate_epoch_rewards` had **two** silent-incompleteness paths in
the same scan:

1. **Outer-loop silent skip (rewards.rs ~line 41):**
   ```rust
   for h in epoch_start_height..epoch_end_height {
       if let Ok(Some(block)) = self.block_store.get_block_by_height(h) {
           // ...
       }
       // else: silently drop the minute
   }
   ```
   If the block at height `h` is missing from the local store, the loop
   body is skipped — no error, no log. `attested_minutes` therefore
   reflects only the blocks the local node happened to have.

2. **Inner `vec![]` branch (rewards.rs ~line 62):**
   ```rust
   let indices = if !block.attestation_bitfield.is_empty() {
       decode_attestation_bitfield_vec(...)
   } else if h < BITFIELD_BODY_ACTIVATION_HEIGHT {
       decode_attestation_bitfield(&block.header.presence_root, producer_count)
   } else {
       vec![]   // <-- silent drop for post-activation header-only blocks
   };
   ```
   For a post-activation block whose body `attestation_bitfield` is empty
   (snap sync gap, header-only store, or wire drift), the branch returns
   `vec![]` and that minute contributes zero attestations for every
   producer — silently.

Both paths produce the same divergence class: `attested_minutes` is a
subset of the canonical counts, and `calculate_epoch_rewards` returns a
reward vector that differs from peers with complete stores. The result
is a rejected `EpochReward` transaction, a fork block, and (as observed
on 2026-04-16) a mainnet cascade.

## Fix — FAIL-FAST

The fix follows the locked Choice 2 (RUNTIME PERIODIC) from the
redesign synthesis (REQ-REDESIGN-002 row 9). We chose fail-fast over
snapshot-from-EpochState because `attestation_accum` is populated by
`apply_block` and is NOT populated when `Node::new_for_test` writes
blocks via `put_block_canonical` alone — which is also the pattern
real mainnet rollback/recovery paths use. A snapshot fix would have
passed the unit tests but remained fragile in the field.

Key changes in `bins/node/src/node/rewards.rs`:

1. Converted the `if let Ok(Some(block)) = ...` pattern to an explicit
   `match`, so `Ok(None)` and `Err(_)` are distinguishable and both
   increment `missing_block_count`.
2. The post-activation empty-body branch now increments
   `silent_bitfield_count` before returning `vec![]`.
3. After the scan loop, if `epoch > 0` AND either counter is non-zero,
   the function logs at `error!` level:
   ```
   [ECON_EPOCH_DISTRIBUTION] incomplete_block_store: gap_count=N
   silent_bitfield_count=M — refusing to compute epoch rewards for
   epoch=E (range=X..Y). Pool accumulates to next epoch.
   ```
   and returns `Vec::new()`.
4. Epoch 0 is exempt because its genesis branch auto-qualifies every
   active producer without reading `attested_minutes` — incompleteness
   of the scan cannot cause divergence for epoch 0.
5. Return type is unchanged (`Vec<(u64, Hash)>`). Callers already
   interpret empty as "no epoch reward distributable this epoch"
   (identical shape to Tier 3 fallback and the empty-qualifier path).

## Why not change the signature to `Result`?

The signature change would ripple through every caller
(`try_produce_block`, `validation.rs`'s expected-rewards comparison, at
least two test helpers). The empty-Vec return is already the agreed
no-distribution shape per rewards.rs:133 and 158. Keeping the signature
localizes the fix to one function and 48 added lines.

## Verification

- `cargo test --test m_rc9_silent_vec_regression -p doli-node` — 3/3 PASS
  - `test_regression_complete_store_all_producers_qualify` (P1 happy path)
  - `test_adversarial_gap_in_middle_must_not_silently_undercount` (P2 gap)
  - `test_santiago_cascade_replay_mainnet_scale` (P5 mainnet-scale replay)
- `cargo build -p doli-node` — clean
- `cargo clippy -p doli-node -- -D warnings` — clean
- `cargo fmt --check` — clean
- `cargo test -p doli-node --lib` — 10/10 PASS
- `cargo test -p doli-node --test epoch_reward_explicit_inputs` — 7/7 PASS (2 pre-existing ignored)
- `cargo test -p doli-node --test fork_recovery` — 11/11 PASS
- `cargo test -p doli-node --test checkpoint_rotation` — 3/3 PASS
- `cargo test -p doli-node --test test_network` — 13/13 PASS

`bins/node/tests/epoch_state_regression.rs` was NOT touched — it has
pre-existing compile errors unrelated to M-RC9 and is out of scope for
this milestone.

## Non-goals (explicit)

- No signature change (`Vec<(u64, Hash)>` preserved).
- No reader-side change in `validation.rs` or `apply_block.rs`. Peers
  with complete stores continue to validate normally; the fail-fast
  behavior applies only to producers computing the reward locally.
- No mainnet or testnet deploy. That is an operations decision for
  the user once QA and reviewer approve.
