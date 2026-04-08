# DOLI L2 Settlement — Interface Specification

> **Status**: Interface specification. Reference design for L2 builders. Implementation is gated on (a) a concrete proof-system decision and (b) a live L2 ready to settle — see §11.
>
> **Audience**: L2 builders evaluating DOLI as a settlement layer, and DOLI maintainers implementing the L1 hooks when activation becomes appropriate.
>
> **Principle**: **Highway first, cars follow.** L1 publishes a clean, minimal interface today so that when an L2 builder arrives, the foundation is obvious, legible, and ready.

---

## 1. Purpose

DOLI L1 is a **settlement layer** for zero-knowledge rollups. Its responsibility is limited to exactly this:

> Given a proof, a previous state commitment, and a next state commitment, decide `valid` or `invalid`. Commit the result atomically via the UTXO model.

L1 does **not** execute rollup logic. L1 does **not** sequence rollup transactions. L1 does **not** store rollup transaction data (beyond what builders voluntarily publish as blob data). L1 is a verifier and a commitment anchor — nothing more.

This minimalism is deliberate. By shrinking L1's scope to "verify proofs and anchor state", DOLI avoids governing circuits, picking winners among L2 architectures, or coupling L1 upgrades to L2 ecosystem decisions. The L1/L2 split is enforced by design.

---

## 2. Scope Boundary

| L1 is responsible for | L1 is **not** responsible for |
|---|---|
| Verifying ZK proofs against a declared verifying key | Authoring rollup circuits |
| Maintaining rollup state UTXOs (prev root → next root) | Sequencing L2 transactions |
| Providing a deterministic, hard-fork-activated `ZKSettle` TX type | Running sequencers |
| Enforcing atomic state transitions via the UTXO model | Bridging assets in or out |
| Guaranteeing invalid proofs cannot corrupt state | L2 data availability (beyond voluntary blob publication) |
| Blob space for proofs in `extra_data` (512 KB → 8 MB over eras) | Circuit upgrades and governance |
| Per-network verification cost budget enforcement | L2 user onboarding, wallets, explorers |

**L2 builders bring the circuit, the sequencer, the bridge, the DA layer, and the users. DOLI brings a verifier, a UTXO slot, and finality.**

---

## 3. Why DOLI Is Already Ready

The primitives for ZK settlement exist in the current codebase. This is not a promise; it is verifiable against the source tree.

| Primitive | Location | Evidence |
|---|---|---|
| 512 KB `extra_data` field per output | `crates/core/src/transaction/output.rs:15` | `pub const BASE_EXTRA_DATA_SIZE: usize = 524_288;` — with a comment in the source that explicitly lists "zero-knowledge proofs" as an intended use case. |
| Era-based growth to 8 MB | `crates/core/src/transaction/output.rs:24-40` | `max_extra_data_size(height)` doubles every era, capped at `MAX_EXTRA_DATA_SIZE_CAP = 8_388_608` (Era 4+). |
| Extensible `OutputType` (`u8` repr) | `crates/core/src/transaction/types.rs:125-154` | 13 variants defined (0–12, contiguous). Slot **13** is the next available value. |
| Extensible `TxType` (`u32` repr) with deliberate gaps | `crates/core/src/transaction/types.rs:7-85` | 29 variants defined. Gaps at 16, 23, and 31+ are reserved. Slot **31** is the next clean value. |
| Consensus / tx-validation layer separation | `crates/core/src/validation/` | `validate_header()`, `validate_vdf()`, and `validate_producer_eligibility()` contain zero references to `tx_type`. Adding a TX type does not touch the consensus engine. |
| Hard-fork activation without genesis reset | `crates/core/src/maintainer.rs:345-380` | `ProtocolActivationData { protocol_version, activation_epoch, signatures }` schedules future activations via 3-of-5 maintainer multisig. On-chain verification lives in `bins/node/src/node/apply_block/governance.rs:80`. |
| `data_root` blob commitment already active | `crates/core/src/validation/block.rs:113-137` | Any output with `extra_data.len() >= 4096` is folded into the header's `data_root`. ZK proofs inherit this commitment automatically. |

These are working, tested, production-deployed primitives. L2 settlement is not a new architecture — it is a minimal extension of an architecture the designers already built for this use case.

---

## 4. The L1 Interface

This section is the **complete** L1 surface area that an L2 builder must target. There is nothing else.

### 4.1 New output type — `ZKRollup`

```rust
// crates/core/src/transaction/types.rs (post-activation)
pub enum OutputType {
    // ... existing 0..=12
    ZKRollup = 13,
}
```

**Semantics:** A `ZKRollup` output encodes a rollup's committed state. Its `amount` field is zero (the output represents state commitment, not currency; it is excluded from `is_native_amount()`). Its `extra_data` field holds the rollup's state commitment and the verifying key required to check future settlements.

**Proposed `extra_data` layout** (subject to the encoding review in §8):

```text
┌────────────────────────────────────────────────────────────┐
│ version           : u16                     (2 bytes)     │
│ rollup_id         : [u8; 32]                (32 bytes)    │
│ proof_system_id   : u16                     (2 bytes)     │  -- see §8.1
│ verifying_key_len : u32                     (4 bytes)     │
│ verifying_key     : [u8; verifying_key_len] (variable)    │  -- typically 1 KB – 200 KB
│ state_root        : [u8; 32]                (32 bytes)    │
│ metadata_len      : u32                     (4 bytes)     │
│ metadata          : [u8; metadata_len]      (optional)    │
└────────────────────────────────────────────────────────────┘
```

The verifying key is carried **in the UTXO itself**. L1 does not maintain a registry of trusted verifying keys. Each rollup is its own trust domain, the same way each Bitcoin script is its own trust domain. Anyone can deploy a rollup; no maintainer vote is required to bless a circuit. Trust is bounded to whoever chooses to spend the UTXO.

**Rationale:** Carrying the key in the UTXO is strictly simpler than a governed registry. A registry would force L1 maintainers to review and bless every L2 circuit, turning L1 into an L2 gatekeeper. The UTXO-scoped approach is permissionless and composable — builders deploy without asking permission; users choose which rollups to trust; L1 only enforces the mechanical invariant "a valid proof under the declared key exists".

### 4.2 New transaction type — `ZKSettle`

```rust
// crates/core/src/transaction/types.rs (post-activation)
pub enum TxType {
    // ... existing 0..=15, 17..=22, 24..=30
    ZKSettle = 31,
}
```

**Structural invariants** (enforced in `validate_transaction()`):

1. **Exactly one** input of output type `ZKRollup` (the previous committed state).
2. **Exactly one** output of output type `ZKRollup` (the new committed state) with the same `rollup_id` and `proof_system_id` as the input.
3. Optionally, additional `Normal` outputs (fee payment, change, L2→L1 withdrawals justified by the proof).
4. The proof blob lives in the output's `extra_data` `metadata` section **or** in a dedicated proof field (see §8.2).
5. Total serialized tx size respects `max_block_size(height)` and `max_extra_data_size(height)`.

**Validity rule** (enforced in `validate_transaction_with_utxos()`):

```rust
verify_zk_proof(
    input.verifying_key,
    input.state_root,    // prev commitment
    output.state_root,   // next commitment
    proof_bytes,
) == Ok(())
```

**Atomicity:** If the proof is invalid, the transaction is rejected and the input UTXO is not consumed. The old state remains canonical. There is no fraud window, no challenge period, no rollback. This is a property of the UTXO model, not an add-on.

### 4.3 The verify API — one function, one purpose

The **entire** L1 surface for ZK verification is a single pure function:

```rust
// crates/core/src/validation/zk.rs  (new file)

pub struct ZkVerifyContext {
    /// Budget (microseconds) remaining in this block for ZK verification.
    pub budget_us_remaining: u64,
    /// Proof system identifier (see §8.1).
    pub proof_system_id: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZkVerifyError {
    InvalidProof,
    UnsupportedProofSystem(u16),
    VerifyingKeyMalformed,
    ProofTooLarge { size: usize, max: usize },
    BudgetExceeded { cost_us: u64, remaining_us: u64 },
    NonDeterministicResult, // only possible if determinism harness fails
}

/// Verify a zero-knowledge proof.
///
/// INPUTS:
///   - verifying_key: raw bytes as stored in the ZKRollup UTXO
///   - prev_state_root: 32-byte previous state commitment
///   - next_state_root: 32-byte next state commitment
///   - proof: raw proof bytes
///   - ctx: cost budget and proof system selector
///
/// OUTPUT:
///   - Ok(cost_us) if the proof is valid; cost_us is debited from the block budget
///   - Err(ZkVerifyError) otherwise
///
/// PROPERTIES:
///   - Pure: no I/O, no side effects
///   - Deterministic: identical inputs yield identical outputs on every supported platform
///   - Bounded: respects ctx.budget_us_remaining; returns BudgetExceeded rather than running over
pub fn verify_zk_proof(
    verifying_key: &[u8],
    prev_state_root: &[u8; 32],
    next_state_root: &[u8; 32],
    proof: &[u8],
    ctx: &ZkVerifyContext,
) -> Result<u64, ZkVerifyError>;
```

That is the **entire** L1 ZK surface. If this function returns `Ok`, the settlement is valid. If it returns `Err`, the transaction is rejected. There is no configuration, no callback, no shared state. A reviewer can understand it in one reading.

### 4.4 Cost budget

`verify_zk_proof` **must** respect a per-call and per-block cost budget so that a malicious rollup cannot extend block validation time beyond the slot window.

| Metric | Target | Rationale |
|---|---|---|
| Max proof size per tx | 400 KB | Fits comfortably under Era-0 `BASE_EXTRA_DATA_SIZE = 524,288` with headroom for the rest of the `extra_data` layout |
| Max verify time per tx | 100 ms on reference hardware | Single proofs complete quickly even on slower nodes |
| Max verify time per block | 2 seconds cumulative | Hard cap; a block exceeding this is invalid |
| Reference hardware | `target/release/doli-node` on M1 / x86_64 w/ AVX2 | Matches current benchmarking baselines |

Budget enforcement lives in `validate_block()` alongside the existing `max_block_size(height)` check. The per-block budget is passed into `ZkVerifyContext` and decremented by each call. When the budget hits zero, subsequent ZK verifications in the same block return `BudgetExceeded` and the block is rejected.

---

## 5. Validation Integration Points

Adding `ZKSettle` / `ZKRollup` touches exactly the files listed below. **Nothing else.** The goal of this section is to remove ambiguity for whoever implements Phase 2 — the full blast radius is visible in one table.

| File | Change | Consensus-critical? |
|---|---|---|
| `crates/core/src/transaction/types.rs` | Add enum variants; extend `OutputType::from_u8` and `TxType::from_u32`; update `is_native_amount()` to exclude `ZKRollup` | Yes (wire format) |
| `crates/core/src/transaction/core.rs` | Helpers: `new_zk_settle()`, `is_zk_settle()` | No |
| `crates/core/src/validation/transaction.rs:98` | New match arm calling `validate_zk_settle_structure()` | Yes |
| `crates/core/src/validation/tx_types.rs` | `validate_zk_settle_structure()` — layout checks on `extra_data`, rollup_id consistency, single-in/single-out rule | Yes |
| `crates/core/src/validation/utxo.rs` | Spending rule: a `ZKRollup` UTXO is consumable **only** by a `ZKSettle` tx with a valid proof | Yes |
| `crates/core/src/validation/zk.rs` *(new)* | `verify_zk_proof()` — the single verifier function | Yes (determinism-critical) |
| `crates/core/src/validation/block.rs:~144` | Per-block ZK verification budget accounting | Yes |
| `bins/node/src/node/apply_block/tx_processing.rs` | State mutation: consume old `ZKRollup`, create new `ZKRollup`; no monetary side-effect | Yes |
| Rollback / undo paths | Already handled by the generic undo log; verify coverage with a reorg test | Yes |
| `crates/mempool/src/` | Admission check: enforce proof-size cap and pre-flight verify | No (performance) |
| `crates/wallet/src/tx_builder/` | `build_zk_settle()` — programmatic constructor for sequencers | No |
| `crates/rpc/src/methods/` | `submitZkSettle`, `getZkRollupState` RPC methods | No |
| `bins/cli/src/` | Optional CLI parity — not required for v1 | No |
| `crates/network/src/protocols/status.rs` | Bump `CURRENT_PROTOCOL_VERSION` | Yes (peer gating) |
| `crates/updater/src/hardfork.rs` | Add `HardForkSchedule` entry for the activation epoch | Yes |
| `specs/protocol.md`, `docs/rpc_reference.md`, `docs/cli.md`, `docs/architecture.md` | Propagate the new TX/Output type and the RPC methods | No |

**Total: 16 touch points. Zero changes to the consensus engine** (`validate_header`, `validate_vdf`, `validate_producer_eligibility`, the scheduler, the VDF crate, BLS aggregation, slot timing, or producer selection).

---

## 6. Consensus Footprint: What Actually Changes

**Unchanged:**
- Block production (`try_produce_block`)
- Producer selection (`DeterministicScheduler`, round-robin by slot)
- VDF proof of time
- BLS attestation aggregation
- Slot timing and block header format (the `data_root` field already exists)
- Reward distribution and epoch rewards
- Bond lifecycle and producer registration

**Changed via hard-fork activation:**
- `apply_block()` learns one new TX type
- `validate_transaction()` gains one match arm
- Per-block validation gains a ZK verify budget
- Nodes running a binary without ZKSettle support must upgrade before the activation epoch

**Subtlety worth calling out:** ZK proofs automatically flow through `data_root`, the header's blob commitment. This is because `data_root` already folds any output with `extra_data.len() >= 4096` (see `validation/block.rs:118`). ZK proofs are typically 100–400 KB, so they will be committed by `data_root` from day one. **This is correct behavior** — it means proof validity is part of the chain's identity, not a soft local rule. Two honest nodes will always agree on `data_root` because `verify_zk_proof` is deterministic (see §8.3).

---

## 7. Activation Path

Activation uses the existing `ProtocolActivation` machinery. **No genesis reset. No coordinated simultaneous upgrade. No chain downtime.** This honors Rule #0 from `CLAUDE.md`: "Bitcoin activates features forward-only… never retroactively from block 0."

```text
Step 1 — Maintainers (3 of 5) sign ProtocolActivationData:
    {
      protocol_version: N + 1,
      activation_epoch: E_future,          // e.g., 30 days ahead
      description:      "ZKSettle hard fork",
      signatures:       [sig_m0, sig_m1, sig_m2],
    }

Step 2 — Submit the ProtocolActivation tx. All nodes observe the
        activation schedule on-chain as soon as the tx is mined.

Step 3 — At epoch E_future, the activation fires on every node
        simultaneously. Deterministic switch, zero coordination.
        Pre-activation: TxType 31 is rejected as unknown.
        Post-activation: TxType 31 is accepted and verified.

Step 4 — Nodes running a binary without ZKSettle support stop
        validating new blocks at E_future and must upgrade.
        This is the standard DOLI hard-fork flow — already used
        in production for previous activations.
```

---

## 8. Open Questions for L1 Implementation

These are decisions **DOLI maintainers** must make before Phase 2 (implementation) begins. They are not L2 builder problems. Each has a recommended default.

### 8.1 Proof system selection

| Option | Proof size | Verify time (ref HW) | Trusted setup | Post-quantum | Recommendation |
|---|---|---|---|---|---|
| **Plonky2** (STARK-based, Goldilocks field) | 100–250 KB | 5–30 ms | None | Yes | **Strong default for v1.** No trusted setup, deterministic, mature Rust, PQ-safe. |
| **Halo2** (universal setup, no per-circuit ceremony) | 5–50 KB | 10–100 ms | Universal | No (pairing-based) | Viable. Smaller proofs, more complex Rust story. |
| **Groth16** (SNARK, BN254 / BLS12-381) | ~200 bytes | 2–5 ms | Per-circuit ceremony | No | Smallest proofs, fastest verify, but per-circuit trusted setup is an ecosystem blocker for a permissionless L2 market. |
| **Risc0** (zkVM, STARK-based) | ~200 KB | 20–50 ms | None | Yes | Interesting for general-purpose L2 execution. Heavier integration. |

**Recommended default:** **Plonky2.** No trusted setup ceremony (critical for permissionless L2 deployment), PQ-safe, deterministic Rust implementation, reasonable proof size. The `proof_system_id` field in the `ZKRollup` extra_data allows future expansion to Halo2 or Groth16 without a second hard fork — L1 can support multiple proof systems by dispatching on `proof_system_id` inside `verify_zk_proof`.

### 8.2 Proof location — `extra_data` metadata vs. dedicated field

Two options:

1. **Store the proof inside the `metadata` section of the `ZKRollup` output's `extra_data`.** Simpler — uses existing blob machinery, no wire format change beyond the new OutputType. Proofs inherit `data_root` commitment automatically.
2. **Add a dedicated `proof` field to the `Transaction` struct.** Cleaner separation, but requires a wire format bump and touches serialization code.

**Recommendation:** Option 1. It is strictly simpler and the existing blob commitment path already does the right thing.

### 8.3 Deterministic verification across platforms

The verifier **must** produce bit-identical results on:

- macOS ARM64 (M1/M2/M3/M4)
- Linux x86_64 (AVX2, AVX-512, plain)
- Linux ARM64
- Any platform a DOLI node runs on

This is harder than it looks. Plonky2, Arkworks, and similar crates offer configurable parallel backends and SIMD feature flags that can influence bit-level output. A non-deterministic verifier is a silent fork generator — the worst class of consensus bug.

**Action required before activation:** Build a determinism harness — a fixed set of (verifying_key, prev_root, next_root, proof) tuples whose `verify_zk_proof` output is checked for bit-identity across every supported platform in CI. **Activation is gated on this harness passing for 30 consecutive days on the full CI matrix.** No exceptions.

### 8.4 Verifier bug recovery

A bug in the verifier is a consensus bug. Two options:

1. **Forward hard-fork fix.** Use the same `ProtocolActivation` path to schedule a corrected verifier. Standard DOLI flow. Acceptable.
2. **Soft-disable path.** Allow maintainers to schedule a future-epoch deactivation of `ZKSettle` if a bug is found. Worth building alongside activation as an emergency valve. Implementation is trivial — a `deactivation_epoch` field in the `ProtocolActivationData` struct.

**Recommendation:** Build **both**. Forward fix is the normal path; soft-disable is the emergency exit.

---

## 9. What L1 Does NOT Provide (Builder's Scope)

The following are **explicitly** L2 builder responsibilities. DOLI will not implement these, will not standardize them, and will not bless specific solutions. Builders are welcome to share reference implementations in the ecosystem, but they are not part of the L1 protocol.

### 9.1 Sequencer operation

Who runs the node that collects L2 transactions and produces the proof? Builder's choice. Valid patterns include:

- Centralized operator (simple, accountable, censorship-prone)
- PoA committee (trust minimized within a known set)
- Rotating subset of DOLI producers (inherits L1 liveness)
- Shared sequencer (multiple rollups share a sequencer set)
- Decentralized sequencer with its own consensus

L1 is agnostic.

### 9.2 Bridge design (L1 ↔ L2 asset flow)

**L1 → L2 deposit:** The builder designs a deposit pattern. DOLI provides no native bridge primitive. Suggested patterns:

- A `Hashlock` or `Multisig` UTXO that the sequencer unlocks on proof of L2 credit
- A dedicated `ZKRollup`-adjacent output type in the builder's own L2 convention (L1 treats it as opaque state)
- Burn-on-L1-mint-on-L2 via a watched DOLI address

**L2 → L1 withdrawal:** Likewise the builder's design. The cleanest pattern leverages the UTXO model directly: **a `ZKSettle` transaction may produce additional `Normal` outputs representing withdrawals**. The ZK proof attests that these withdrawals are justified by the committed L2 state transition. This is more atomic and cheaper than optimistic-rollup challenge windows.

L1 provides the settlement primitive; the bridge is layered on top.

### 9.3 Data availability

Where does L2 transaction data live so L2 users can reconstruct state if the sequencer disappears?

DOLI provides **one** DA primitive: any transaction output's `extra_data` field is blob space, folded into `data_root`, committed by consensus, replicated by every full node. A rollup can publish its transaction data as L1 `extra_data` blobs and inherit **full L1 data availability**. This consumes block space (and its associated fees) but is the **strongest** DA guarantee in the ecosystem — the same honest-majority assumption that secures DOLI's state.

Alternatives — Celestia, EigenDA, custom DAS, validity-bond-backed off-chain DA — are the builder's choice and the builder's trust assumption. L1 does not care.

### 9.4 Circuit authorship and upgrade

The circuit defines what "valid L2 state transition" means. Builders author it. When builders want to upgrade the circuit, they deploy a new `ZKRollup` UTXO with a new `verifying_key`. Old UTXOs can still be spent with the old key — compatibility is automatic because keys live in the UTXO itself. There is no circuit registry, no migration protocol, no versioning drama at the L1 layer.

### 9.5 User onboarding, wallets, explorer integration

Builder's problem. DOLI will expose `getZkRollupState` via RPC so block explorers can surface rollup state generically; beyond that, L2 UX (custody, signing, address formats, transaction construction) is entirely the builder's domain.

---

## 10. Security Considerations

### 10.1 Verifying-key poisoning

A malicious actor could deploy a `ZKRollup` UTXO whose verifying key accepts arbitrary proofs. This is **not an L1 problem** — it only affects whoever chooses to trust that rollup. L1's invariant ("a valid proof under the declared key exists") is preserved regardless. Users and L2 auditors are responsible for vetting the verifying keys they trust, exactly as they vet Bitcoin multisig policies or Ethereum contracts.

### 10.2 Large-proof denial-of-service

A flood of settlement transactions carrying maximum-size proofs could exhaust block space and verification time. Mitigations:

- **Proof size cap:** 400 KB per tx (§4.4).
- **Per-block verify budget:** 2 seconds cumulative (§4.4).
- **Fee market:** `extra_data` bytes are already fee-weighted by size, so large proofs are expensive to include.

### 10.3 Non-deterministic verifier

A verifier producing different results on different platforms causes an immediate fork. See §8.3 — the determinism harness is a **hard gate** on activation, not an afterthought.

### 10.4 Post-quantum posture

Plonky2 (STARK-based, hash-only) is post-quantum safe. Groth16 (pairing-based) is not. This is a 10+ year consideration, but it reinforces the Plonky2 default.

### 10.5 Verifier crate supply chain

The verifier crate (Plonky2, Arkworks, etc.) is third-party code in the consensus path. Mitigation:

- Pin the exact crate version and audit every upgrade as if it were a consensus change
- Run the full determinism harness on every upgrade
- Prefer crates with formal verification or heavy production use over newer alternatives

---

## 11. Roadmap

| Phase | Deliverable | Blocker |
|---|---|---|
| **0. Interface spec** *(this document)* | Published L1 interface, ready for L2 builders to evaluate | — |
| **1. Proof system selection** | Benchmark Plonky2 / Halo2 / Groth16 on reference hardware; pick default; build determinism harness | §8.1 + §8.3 |
| **2. L1 implementation** | Enum variants, validator, apply_block handler, wallet/RPC hooks, full test coverage | Phase 1 complete |
| **3. Security audit** | External audit of `verify_zk_proof`, the determinism harness, and the activation path | Phase 2 complete |
| **4. Testnet activation** | `ProtocolActivation` on testnet, reference ZKSettle sequencer from a volunteer L2 team | Phase 3 complete |
| **5. Mainnet activation** | `ProtocolActivation` on mainnet | Testnet soak + live L2 volunteer + incident-free window |

**Phase 0 is this document.** The remainder of the roadmap is intentionally gated on a real L2 builder declaring interest. The interface is stable and published — that is the foundation. When a builder knocks, Phase 1 begins.

---

## 12. References

**Source of truth (code):**

- `crates/core/src/transaction/output.rs:11-40` — `BASE_EXTRA_DATA_SIZE`, `max_extra_data_size()`, era growth
- `crates/core/src/transaction/types.rs:7-154` — `TxType`, `OutputType` enums and `from_*` converters
- `crates/core/src/transaction/core.rs:853-878` — `ProtocolActivation` tx constructors
- `crates/core/src/maintainer.rs:340-389` — `ProtocolActivationData` struct and signing message
- `crates/core/src/validation/block.rs:12-166` — `validate_header`, `validate_block`, `data_root` commitment path
- `crates/core/src/validation/transaction.rs:27-194` — `validate_transaction` dispatch table
- `crates/core/src/validation/producer.rs:12,200` — `validate_vdf`, `validate_producer_eligibility`
- `bins/node/src/node/apply_block/governance.rs:80` — on-chain ProtocolActivation verification
- `bins/node/src/node/apply_block/tx_processing.rs:1-80` — transaction UTXO processing dispatch

**Related specifications:**

- [`WHITEPAPER.md`](/WHITEPAPER.md) — DOLI protocol whitepaper
- [`specs/protocol.md`](./protocol.md) — Full protocol specification (wire format, consensus rules)
- [`specs/architecture.md`](./architecture.md) — Comprehensive system architecture
- [`specs/security_model.md`](./security_model.md) — Security model and threat analysis

**Design analysis:** The reasoning that led to this interface is preserved in conversation history and in the memory note on L2 settlement analysis. This spec is the distilled, decision-ready output of that analysis.
