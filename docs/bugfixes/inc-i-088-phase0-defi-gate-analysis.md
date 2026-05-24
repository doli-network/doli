# INC-I-088 — Phase 0 Safety Gates: DeFi Activation Gate + Collateral Conditioning

## 1. Bug Description

Two pre-existing defects exposed by the state-of-the-art redesign Phase 0 audit:

**Defect 1 — 11 DeFi transaction types are ungated.**
`CreatePool`, `AddLiquidity`, `RemoveLiquidity`, `Swap`, `CreateLoan`, `RepayLoan`,
`LiquidateLoan`, `LendingDeposit`, `LendingWithdraw`, `FractionalizeNft`, `RedeemNft`
are accepted by `validate_transaction` at every height with no activation gate.
Any user can submit them via raw RPC. Validation has known semantic gaps (in
particular `LiquidateLoan` is a structural shell with no oracle, so any actor
holding the liquidation TX format can drain undercollateralized loans).
Exploitable today on any network.

**Defect 2 — `OutputType::Collateral` is missing from `OutputType::is_conditioned()`
(`crates/core/src/transaction/types.rs:222-233`).** Spend verification at
`crates/core/src/validation/utxo.rs:975` branches on `is_conditioned()`. When
the type is not conditioned, spend succeeds with a single Ed25519 signature
over the spender's pubkey hash. For Collateral that means: a borrower can
spend their own collateral UTXO with their own signature — bypassing the
intended `RepayLoan` / `LiquidateLoan` lifecycle entirely.

Both defects coexist with the same root cause class: ungated optional
subsystems treated as production-eligible.

## 2. Architecture Context

### 2.1 NetworkParams → ValidationContext wiring

```
NetworkParams (per-network defaults, env-overridable on non-mainnet)
   crates/core/src/network_params/{mod.rs, defaults.rs, env_loader.rs}
        ↓ (read at ValidationContext construction)
ValidationContext (per-block, copies activation heights as fields)
   crates/core/src/validation/types.rs
        ↓ (consumed by)
validate_transaction(tx, ctx)
   crates/core/src/validation/transaction.rs:27
        ↓ dispatch on tx.tx_type
{validate_create_pool, validate_create_loan, ...}
```

ValidationContext is constructed at 4 call sites:
- `bins/node/src/node/validation_checks.rs:103` (eligibility check)
- `bins/node/src/node/validation_checks.rs:~250` (apply path)
- `bins/node/src/node/apply_block/tx_processing.rs:65`
- `bins/node/src/node/production/assembly.rs:178`
- `crates/mempool/src/pool.rs:227, 458`

Each site calls a `.with_*_activation_height()` builder. Adding a new gate
requires touching all 6 sites — established pattern.

### 2.2 Existing activation-gate pattern

Reference precedent — `encrypted_content_activation_height`:
- Stored on `NetworkParams` with `u64::MAX` default for not-yet-activated
- Mirrored on `ValidationContext` (separate field, not a reference)
- Wired in via `.with_encrypted_content_activation_height(...)` from every
  call site that reads `params().encrypted_content_activation_height`
- Validation site: `if ctx.current_height < ctx.encrypted_content_activation_height { return Err(...) }`
- Error: `ValidationError::InvalidTransaction(format!("[ERRTX-EC000] ..."))`
  with a per-feature error code prefix

### 2.3 ValidationError architecture

`crates/core/src/validation/error.rs` defines a typed enum with:
- `#[error("...")]` Display for human-readable messages
- `error_code() -> &'static str` for stable machine-parseable codes
- `to_structured_json() -> Value` for structured agentic consumption

The user's REQ-AGENTIC-ERRORS protocol is satisfied by adding a new typed
variant with all three layers populated, NOT by reusing
`InvalidTransaction(String)` with an `[ERRTX-…]` prefix (string parsing is
weaker than a typed variant). One consolidated `DefiNotActivated` variant
gates all 11 tx types.

### 2.4 Collateral spend path

`verify_input_conditions` (`crates/core/src/validation/utxo.rs:966`) branches:
- `if utxo.output.output_type.is_conditioned()` → decode condition tree from
  `extra_data`, decode witness from `tx.extra_data`, evaluate.
- else → single-signature path (pubkey_hash + Ed25519).

Currently `Collateral` falls into the `else` branch → spendable with
plain signature. Adding `Collateral` to `is_conditioned()` routes it through
the condition path; since Collateral's `extra_data` is `CollateralMetadata`
(not condition-prefixed), `Condition::decode_prefix` fails and the spend is
rejected with `[ERRTX038]` "input X references output with invalid
condition: ...". That is the desired freeze behavior: until lending is
properly fixed, no Collateral UTXO is spendable.

This is intentional asymmetry — the DeFi gate blocks NEW Collateral creation
(via `CreateLoan` rejection); `is_conditioned()` blocks EXISTING Collateral
from being spent through any path (Transfer, RepayLoan, anything). Layered
defense.

## 3. Hard Constraints (verified against CLAUDE.md hot rules)

| ID | Rule | Compliance |
|----|------|-----------|
| C0 | NO genesis reset | ✅ New field defaults to `u64::MAX`; never activates retroactively |
| C1 (INC-I-054) | Crossed activation heights are IMMUTABLE | ✅ NEW field, never crossed |
| C2 (CURRENT_PROTOCOL_VERSION) | Bump only if EpochState changed | ✅ EpochState unchanged → no bump |
| C3 (HardForkSchedule) | Don't add for rolling deploys | ✅ Constant gate, no hardfork entry |
| C5 (INC-I-062) | Block content change → synchronized deploy | ⚠️ See §4 — needs analysis |
| C7 (INC-I-075 3-question checklist) | (1)YES (2)NO (3)NO → gate required | ✅ Gate is exactly what we are adding |
| Output Contract | Test FAILs first, then PASSes | ✅ TDD enforced by milestone loop |

### 3.1 Re-checking C5 (block content vs validation-only change)

The user's task description claims the change is rolling-deploy safe because
"validators only reject — does not change what producers put INTO blocks".
Verifying:

- **Producer side**: `bins/node/src/node/production/assembly.rs` selects TXs
  from mempool and includes them in blocks. If an upgraded producer's mempool
  rejects DeFi TXs at admission time (mempool wires the same ValidationContext),
  upgraded producers will NOT include DeFi TXs in their blocks.
- **Old producers** (pre-deploy): still accept DeFi TXs in mempool, still
  include them in blocks.
- **Cross-version interaction**: an old producer's block containing a DeFi TX
  is sent to an upgraded validator → upgraded validator REJECTS the block →
  fork.

→ **This is a block-content-affecting change for rolling deploys IF any user
ever submits a DeFi TX during the mixed-version window.** Mitigation:
mainnet has zero DeFi TXs today (per pre-deploy audit assumption); testnet
likewise. With zero DeFi TX submission, no mixed-version block ever contains
a gated TX, and rolling deploy is safe.

**Verdict**: classify as "rolling-deploy safe contingent on zero DeFi TX
volume during deploy window". Document in commit message. Recommend
synchronized restart anyway for paranoia — cost is low.

## 4. Pre-Deploy Verification Gap

The task description requires querying mainnet RPC for UTXO counts of types
7 (FungibleAsset), 9 (Pool), 10 (LPShare), 11 (Collateral), 12 (LendingDeposit).
**I cannot perform this query from the local environment** (CLAUDE.md: local
devnet only, no remote server access from this session, no public mainnet
endpoint exposed for raw UTXO scans).

This is a **USER-ACTION REQUIRED** gate before deploy:

```bash
# To be run by the user against a mainnet node they control
ssh ai2  # or any mainnet seed
doli rpc getUtxoCountByType --network mainnet  # or curl equivalent
# Verify: counts for types 7, 9, 10, 11, 12 are all 0
```

If any count > 0:
- Setting `defi_activation_height = u64::MAX` would freeze those UTXOs forever
  (spending TXs would be rejected by the gate).
- Mitigation: set mainnet to `(current_height + 100)` and document the
  grandfathered set.

If all counts = 0 (expected):
- Mainnet defi_activation_height = `u64::MAX` is safe.

**This Phase 0 work proceeds with the assumption of zero pre-existing DeFi
UTXOs and writes mainnet default as `u64::MAX`. User must run the audit and
either confirm OR overwrite the default before commit.**

## 5. Requirements

### REQ-DEFI-GATE-001 (Must) — DeFi activation field
**Acceptance**: `NetworkParams` carries a new `defi_activation_height: u64`
field. Defaults:
- Mainnet: `u64::MAX`
- Testnet: `u64::MAX`
- Devnet: `u64::MAX`
Env override `DOLI_DEFI_ACTIVATION_HEIGHT` honored on testnet/devnet only
(mainnet locked, matches the existing pattern for security-critical heights).

### REQ-DEFI-GATE-002 (Must) — ValidationContext field + builder
**Acceptance**: `ValidationContext` carries `defi_activation_height: u64`
(default `u64::MAX` in `new()`); builder `with_defi_activation_height(u64)`
exists. All 6 ValidationContext construction sites read
`params().defi_activation_height` and call the builder.

### REQ-DEFI-GATE-003 (Must) — Consolidated typed error
**Acceptance**: One new variant `ValidationError::DefiNotActivated {
tx_type: u32, activation_height: u64, current_height: u64 }` with:
- `#[error(...)]` human-readable Display
- `error_code() == "DEFI_NOT_ACTIVATED"`
- `to_structured_json()` exposes all three fields

Same variant covers all 11 tx types. The `tx_type` discriminant in the variant
identifies which one was rejected.

### REQ-DEFI-GATE-004 (Must) — Gate enforcement
**Acceptance**: In `validate_transaction` (`crates/core/src/validation/transaction.rs`),
BEFORE the existing match arm dispatch for the 11 DeFi tx types, insert:

```rust
if matches!(tx.tx_type,
    TxType::CreatePool | TxType::AddLiquidity | TxType::RemoveLiquidity |
    TxType::Swap | TxType::CreateLoan | TxType::RepayLoan |
    TxType::LiquidateLoan | TxType::LendingDeposit | TxType::LendingWithdraw |
    TxType::FractionalizeNft | TxType::RedeemNft
) && ctx.current_height < ctx.defi_activation_height {
    return Err(ValidationError::DefiNotActivated { ... });
}
```

Position: after step 1-5 of `validate_transaction` (basic structural checks
pass), but before the type-specific match. Reason: outputs validation already
gates Pool/LPShare/Collateral/LendingDeposit output creation via the existing
covenants gate, so the new check is the second layer. Gate is a fast-path
reject — no DeFi sub-validator runs.

### REQ-COLLATERAL-FREEZE-005 (Must) — Add Collateral to is_conditioned()
**Acceptance**: `OutputType::is_conditioned()` (`crates/core/src/transaction/types.rs:222-233`)
returns `true` for `OutputType::Collateral`. Effect: `verify_input_conditions`
routes Collateral spends through the condition path. Collateral's `extra_data`
is `CollateralMetadata` (not condition-prefixed) → `Condition::decode_prefix`
fails → spend rejected with `[ERRTX038]`.

NOT changing `is_native_amount()` (Collateral was never in it and shouldn't be).

### REQ-DEFI-GATE-006 (Should) — Mempool symmetry
**Acceptance**: Mempool admission path (`crates/mempool/src/pool.rs:227, 458`)
constructs ValidationContext with `defi_activation_height` wired from
`params()`. Effect: pre-activation, mempool rejects DeFi TXs at submission
time, preventing them from reaching block-assembly. Reduces (but does not
eliminate) the cross-version block-content risk in §3.1.

### REQ-DEFI-GATE-007 (Should) — env_loader.rs
**Acceptance**: `env_loader.rs::load_from_env` honors
`DOLI_DEFI_ACTIVATION_HEIGHT` on non-mainnet (matches the established
`is_mainnet ? default : env_parse` idiom).

### REQ-DEFI-GATE-008 (Won't, this session) — Mainnet UTXO audit RPC
NOT adding a new RPC to count UTXOs by type — out of scope. The user runs
the audit using existing tooling against a mainnet node they control.

## 6. Impact Analysis

| Subsystem | Impact | Risk |
|-----------|--------|------|
| Core validation | New typed error variant + 1 gate check | Low |
| NetworkParams | New field (additive, default u64::MAX) | Low — no on-disk format change |
| ValidationContext | New field + builder (additive) | Low — `new()` defaults to u64::MAX so all existing call sites stay valid before threading the wire-up |
| Node validation paths (4 call sites) | Add `.with_defi_activation_height(...)` call | Low — mechanical |
| Mempool (2 call sites) | Same | Low |
| Storage / state root / consensus encoding | NONE | None — change is rejection-only, never alters block bytes |
| EpochState format | NONE | None |
| CURRENT_PROTOCOL_VERSION | NONE — do NOT bump | Avoids INC-I-054 cascade |
| HardForkSchedule | NONE — constant gate | Avoids fork_id pollution |
| Existing tests | Most pass unchanged. Any test that constructs a CreatePool/CreateLoan/etc. tx and validates it MUST set `defi_activation_height = 0` (or use the devnet override) on the context | Medium — requires audit of validation tests |
| Auto-update / hardfork.rs | NONE | None — no entry needed |

### Cross-version deploy risk
- **Safe** if no DeFi TXs ever submitted during deploy window. Mainnet
  currently has zero per assumption (audit-pending).
- **Unsafe** if a user submits a DeFi TX during deploy: old producers include
  it, new validators reject, fork.
- **Mitigation**: synchronized restart anyway (low cost, eliminates the
  window). Plus: pre-deploy, the user broadcasts intent / freezes the chain
  via existing emergency-halt scripts if necessary.

## 7. Specs/Docs Drift

The change requires updates to:
- `specs/protocol.md` — document the new activation gate
- `specs/security_model.md` — document the freeze of Collateral spends
- `CLAUDE.md` — add `defi_activation_height` to the activation height list in
  "If You Touch / activation heights"
- `docs/architecture.md` — note the gate exists; cross-link to
  `specs/state-of-the-art-architecture.md` Phase 0 section

Deferred (acknowledged but not in this session per task scope):
- `specs/lending.md` / `specs/pool.md` — note the gate, do NOT mark DeFi as
  "production-ready"

## 8. Architecture Smells Surfaced (NOT fixing here — noted for redesign)

- `LiquidateLoan` has 2-check validator and no oracle. Gate hides this for
  now. Un-gating requires an oracle subsystem (out of scope per task).
- 11 DeFi tx types implemented with no activation gate is itself a process
  failure — every consensus-visible TX type should require a gate by
  default. Gate-by-default policy is a doc/CI concern for the redesign.

━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.95, evidence)
Reasoning: Root cause is known (user diagnosed); fix is mechanical (add field + gate + enum variant); blast radius is well-bounded (additive only); reference pattern (encrypted_content_activation_height) already exists in the codebase; CLAUDE.md hot rules all green.
━━━━━━━━━━━━━━━━━━━━━━

### Milestone Plan (FAST — single milestone, sequential)

**M0 — Phase 0 Safety Gates** (one cohesive change, ≤ 8 file edits + 2 test files)
1. Tests first (TDD per CLAUDE.md / Output Contract):
   - `test_defi_tx_types_rejected_pre_activation` — 11 sub-cases (one per TX type), each constructs minimal valid tx of that type, validates at `current_height=0` with `defi_activation_height=u64::MAX`, asserts `Err(ValidationError::DefiNotActivated { tx_type, .. })` with correct discriminant.
   - `test_defi_tx_types_accepted_at_activation` — 11 sub-cases, validates at `current_height=10` with `defi_activation_height=10`, asserts the gate passes (still hits per-type validator; outer Result may be Ok or per-type-specific error, but NOT `DefiNotActivated`).
   - `test_collateral_utxo_unspendable_with_plain_signature` — constructs Collateral UTXO with metadata, attempts to spend with a Transfer tx and valid pubkey signature, asserts spend rejected with `[ERRTX038]` (condition decode failure).
2. Code changes:
   - `crates/core/src/network_params/mod.rs` — add field
   - `crates/core/src/network_params/defaults.rs` — set u64::MAX in all 3 networks
   - `crates/core/src/network_params/env_loader.rs` — DOLI_DEFI_ACTIVATION_HEIGHT
   - `crates/core/src/validation/types.rs` — ValidationContext field + builder + default
   - `crates/core/src/validation/error.rs` — DefiNotActivated variant + error_code + to_structured_json
   - `crates/core/src/validation/transaction.rs` — gate check
   - `crates/core/src/transaction/types.rs` — Collateral in is_conditioned()
   - 6 wire-up sites: validation_checks.rs (×2), apply_block/tx_processing.rs, production/assembly.rs, mempool/pool.rs (×2)
3. Run gate: `cargo build --release && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check && cargo test -p doli-core --lib && cargo test -p doli-mempool --lib`
4. Docs sync.
5. Show diff to user, get commit approval. Do NOT push or deploy.

### Out of Scope (per user task)
- ZKSettle (separate session)
- Refactoring node god-object / wallet / RPC
- Removing DeFi code
- Fixing LiquidateLoan oracle
- Bumping CURRENT_PROTOCOL_VERSION
- Bumping any existing activation height
- Adding a HardForkSchedule entry
