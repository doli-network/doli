# INC-I-083 — Testnet Regression-Validation of Fix Batch ce1a72dc..HEAD

## Nature of Task

This is **regression-validation**, not diagnosis. Every incident in scope is already
diagnosed, fixed, and committed. The goal: prove on the running testnet (already on
HEAD `641c00f6`) that each fix exhibits its designed behavior and that the prior
anomalies no longer reproduce.

## Scope — commits in `ce1a72dc..HEAD`

| Incident | Commits | Fix summary | Validation method |
|---|---|---|---|
| INC-I-079 | `1e07876a`, `ce1a72dc` (test) | economic-sim S1/S2 invariant fix | **Unit test** — `cargo test -p` economic-sim |
| INC-I-078 | `8562f3d7`, `882585bc`, `c46e9f62`, (`d885a449` AH), (`3faeccc0` merge) | per-producer `received_delegation_cap` + Ed25519 auth on DelegateBond/RevokeDelegation; mainnet AH=231_830 | Unit tests + live testnet DelegateBond cap/auth behavior |
| INC-I-080 | `2ed43260`, `0f5a841e` (AH), (`3faeccc0` merge) | AddBond cap: reject post-AH, clip pre-AH; mainnet AH=231_830 | Unit tests + live testnet AddBond cap behavior |
| INC-I-081 | `3a58dc20`, `cbaa3963`, `e25a9a97`, `52116b64`, `4349403a` | E613 sync-cascade hotfix bundle: epoch-store abort, ShallowRollback finality guard, plan_reorg ancestor fallback, direct-apply fallback, finality reset backstop | Unit tests + live testnet fleet convergence (no cascade/fork) |
| INC-I-082 | `641c00f6` | `rebuild_epoch_state_from_blocks` bit-identity + explicit target_height | Unit tests + live testnet state-root convergence after epoch boundary |

## Architecture Context

All five fixes are **consensus/sync-visible** but target distinct subsystems:

- **INC-I-078/080** — `crates/core` validation + `crates/storage` ProducerSet (`add_bonds`,
  delegation accounting). Height-gated via `NetworkParams` activation heights. Blast radius:
  any block containing AddBond/DelegateBond/RevokeDelegation txs after the testnet activation
  height. The 3-state invariant (ChainState/UtxoSet/ProducerSet) must stay identical fleet-wide.
- **INC-I-081** — `crates/network/src/sync/manager.rs` + `bins/node` fork-recovery. Triggered
  by an invalid epoch-boundary block + reorg/rollback path. Validated by *non-reproduction*:
  the fleet must stay converged through epoch boundaries with no ShallowRollback-past-finality.
- **INC-I-082** — `rebuild_epoch_state_from_blocks` (epoch_state rebuild on restart/reorg).
  Validated by state-root equality across snap-synced and full nodes after an epoch boundary.

The unifying acceptance signal across INC-I-081/082 is the **fleet 3-state convergence
invariant**: every node reports identical `(height, best_hash, state_root)` and identical
ProducerSet at the same height. Divergence = FAIL.

## Acceptance Criteria

- **AC-1 (INC-I-079)** — economic-sim unit tests pass (`cargo test`), no INV-4 halving assumption.
- **AC-2 (build)** — `cargo build --release` + `cargo clippy -- -D warnings` + `cargo fmt --check` clean.
- **AC-3 (INC-I-078/080 unit)** — delegation-cap, Ed25519-auth, and AddBond-cap unit tests pass.
- **AC-4 (INC-I-081/082 unit)** — sync-guard + rebuild bit-identity unit/regression tests pass.
- **AC-5 (testnet liveness)** — testnet producing blocks; n10 healthy after restart.
- **AC-6 (testnet convergence)** — all testnet nodes report identical `(height, best_hash,
  state_root)` and ProducerSet at a common height across ≥1 epoch boundary. No fork, no
  cascade, no ShallowRollback-past-finality in logs.
- **AC-7 (testnet cap/auth functional)** — AddBond/DelegateBond beyond cap behaves per the
  testnet activation-height rule; RevokeDelegation/DelegateBond without valid Ed25519 sig rejected.
- **AC-8 (anomaly non-reproduction)** — none of the prior anomalies (E613 cascade, snap-sync
  state divergence, silent bond clipping) reproduce during the observation window.

Any AC that cannot be evidenced with command output / measurement is reported **FAIL with the
gap** — no narrative-only PASS (Fix Confidence Gate).

━━━ TRIAGE VERDICT ━━━
Path: FAST (structured validation)
Confidence: conf(0.95, explicit)
Reasoning: No unknown root cause — all 5 incidents already diagnosed, fixed, and committed; deterministic regression-validation of known fixes; user explicitly selected structured validation over the deep pipeline. Deep parallel-investigation path is not applicable.
━━━━━━━━━━━━━━━━━━━━━━

## Validation Milestones (FAST → Step 3)

- **M0** — Testnet prep: restart n10 (systemctl, ai5), confirm it rejoins and syncs.
- **M1** — Code gate: `cargo build --release && cargo clippy -- -D warnings && cargo fmt --check`, then targeted `cargo test` for the affected crates (covers AC-1..AC-4). Read-only, local.
- **M2** — Testnet liveness + convergence: query the full fleet over an observation window
  spanning ≥1 epoch boundary; assert AC-5/AC-6/AC-8.
- **M3** — Functional cap/auth check on testnet: AC-7.
- **M4** — Consolidated QA report: per-incident PASS/FAIL with evidence; map to ACs.
