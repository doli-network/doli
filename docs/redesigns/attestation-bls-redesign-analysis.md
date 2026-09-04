# Attestation / BLS Redesign — Problem Scoping Analysis (INC-I-178)

**Run type:** `/omega-redesign`, PROPOSAL-ONLY. No source, spec, or doc outside this file was modified.
**Analyst pass date:** 2026-09-03. **Base commit:** `c3d9e827` (main).
**Linked incidents:** INC-I-178 (open, high), INC-I-141 (investigating), INC-I-191 / INC-I-192 (DB says open — see §2, code says fixed).

---

## 1. Scope & refined problem

### 1.1 Scope taken

Read-only, limited to the attestation subsystem and its consumers:

| Area | Files read |
|---|---|
| Attestation type + tracker + bitfield codec | `crates/core/src/attestation.rs` |
| BLS primitives | `crates/crypto/src/bls.rs`, `crates/crypto/src/lib.rs` |
| Block format + hash commitment | `crates/core/src/block.rs` |
| Producer-side encode | `bins/node/src/node/production/{assembly.rs,mod.rs}` |
| Own-attestation sites | `bins/node/src/node/production/assembly.rs`, `bins/node/src/node/apply_block/post_commit.rs`, `bins/node/src/node/startup.rs` |
| Gossip ingress | `bins/node/src/node/network_events.rs`, `bins/node/src/node/mod.rs` |
| Validation | `bins/node/src/node/validation_checks.rs`, `crates/core/src/validation/registration.rs` |
| Finality | `crates/core/src/finality.rs`, `crates/network/src/sync/manager/production_gate.rs` |
| Rewards / epoch attendance | `bins/node/src/node/rewards.rs`, `bins/node/src/node/apply_block/post_commit.rs` |
| RPC | `crates/rpc/src/methods/schedule.rs`, `crates/rpc/src/types/block.rs` |
| Producer keys | `crates/storage/src/producer/types.rs`, `bins/node/src/node/apply_block/tx_processing.rs`, `crates/wallet/src/wallet.rs`, `bins/node/src/keys.rs` |
| Params | `crates/core/src/network_params/{mod.rs,defaults.rs,ordering.rs}`, `crates/core/src/consensus/constants.rs` |

Explicitly NOT read: DeFi, oracle, storage engine internals, sync manager beyond `add_attestation_weight`, CLI/GUI beyond the registration key path.

### 1.2 Refined problem statement

The on-chain attendance bitfield is a **producer's unilateral, unauthenticated claim** about which producers attested. Nothing in the chain binds a set bit to a signature any attester actually made. The `Block.aggregate_bls_signature` field that was supposed to do that binding is emitted empty on every block and has no validator.

The root cause is **not** "someone deleted the validator." It is a **semantic mismatch that made the aggregate un-verifiable by construction** — see §6.1. The 2026-07 deletions removed provably-dead code; they did not create the gap.

### 1.3 SSF recommendation (ONE proposal, per Rule 18)

> **Redefine one bit to one message.** Post-activation, bit *i* means "producer *i* attested **this block's parent** (`parent_hash || parent_slot`)", the producer puts the BLS aggregate of exactly those attestations in the existing `Block.aggregate_bls_signature`, and `presence_root` becomes `BLAKE3(attestation_bitfield || aggregate_bls_signature)`.
>
> This works because it is the only change that makes all three constraints hold at once: one common message makes `fast_aggregate_verify` (one pairing, N-independent) applicable; redefining the *preimage* of `presence_root` — which `BlockHeader::hash()` already commits to (`block.rs:81`) — hash-commits the aggregate **without any header or block-format change**; and the verification key set is already on-chain and PoP-verified (§5.3).

Non-foreclosure clause, mandatory: the validator MUST be reachable from `Node::validate_block_for_apply` (`validation_checks.rs`), with a negative test that drives a forged bit through *that* function, plus a live counter so "zero executions" is observable. This is the exact failure mode being escaped (§3, §6.1).

---

## 2. Institutional-memory briefing: what memory said vs what the code says

| # | Memory / briefing claim | Verdict | Evidence |
|---|---|---|---|
| 1 | Bitfield accepted with only `presence_root == BLAKE3(bitfield)` + stray-bit check | **VERIFIED** | `bins/node/src/node/validation_checks.rs:421-446` — exactly those two checks, nothing else |
| 2 | Blocks emit `aggregate_bls_signature = Vec::new()` | **VERIFIED** | `bins/node/src/node/production/mod.rs:601` |
| 3 | Aggregate validator deleted in `86bac138`; production stopped in `427d5050` | **VERIFIED (dates 2026-07-19)** | `git show --stat`; both commits present on main |
| 4 | Aggregate was dead on arrival since `30903c8b` (2026-03-10): `validate_block()` had zero call sites, `producer_bls_keys` never populated | **VERIFIED** | commit message of `86bac138`; field carried `#[allow(dead_code)]` |
| 5 | `427d5050` stopped BLS production | **PARTIALLY REFUTED** | It changed only `assembly.rs::attest_own_block` (`assembly.rs:665-673`). A **second, duplicated** own-attestation site at `apply_block/post_commit.rs:446-459` still calls `crypto::bls_sign` + `record_with_bls` on **every applied block**. Both gossip ingresses also `record_with_bls` (`network_events.rs:371`, `:607`). BLS signatures are still computed and retained; the only reader `bls_sigs_for_minute` (`attestation.rs:439`) has **zero callers**. |
| 6 | INC-I-191 / INC-I-192: `attester_weight` self-declared and unauthenticated on both ingresses; no membership check; unbounded `minute_tracker` growth | **REFUTED — already fixed on main** | `13daee6f` (2026-08-28) "derive attestation authority from local ProducerSet". Both ingresses call `Node::derive_attester_weight` (`bins/node/src/node/mod.rs:434-448`), which returns `None` for non-members. `on_new_attestation` drops non-members (`network_events.rs:583-589`); `record_direct_attestation` drops non-members before touching the tracker (`network_events.rs:350-365`). Tests: `bins/node/src/node/attestation_authority_tests.rs` (3 tests). **`.omega/memory.db` still lists both as `status='open'` — DB drift, not a code gap.** |
| 7 | Must-fix #1: `Block::hash()` is header-only, body aggregate strippable | **VERIFIED, and a cheaper fix exists** | `Block::hash()` = `header.hash()` (`block.rs:187-189`). Header commits `presence_root` (`block.rs:81`) → the **bitfield is transitively committed** post-activation; the **aggregate is not committed at all**. Because the commitment is over `presence_root`'s *preimage*, redefining that preimage closes the gap with no header change. |
| 8 | Must-fix #2: encoder order `[epoch_state.producer_list \| extra sorted by pubkey]`; index parity is the top risk; 5 live decoders | **VERIFIED, and worse than stated** | 5 live decoders confirmed (§4.3) — but they use **three different denominators**, and the stray-bit validator uses a **fourth** (§6.2). The deleted validator used a **fifth** order (`producer_bls_keys` "sorted by Ed25519 pubkey", `86bac138` `types.rs:147`). |
| 9 | Must-fix #3: attesters sign Ed25519 only, dual-sign needed | **PARTIALLY REFUTED** | The block-production attestation path is Ed25519-only (`startup.rs:591-608` calls `Attestation::new`, and `Attestation::new_with_bls` at `attestation.rs:74` has **zero callers**). But the node *does* BLS-sign into its own tracker at `post_commit.rs:450-457`. So "dual-sign" is a **gossip-wire** gap, not a key-availability gap. |
| 10 | INC-I-162 resolved: BLS key derived from BIP-39 seed | **VERIFIED, with a residual** | `crates/wallet/src/wallet.rs:93,129` and `bins/cli/src/wallet.rs:90,126` use `BlsKeyPair::from_seed(&bip39_seed)`. **Residual:** `Wallet::add_bls_key` (`wallet.rs:401-411`) and `bins/cli/src/wallet.rs:497` still call `BlsKeyPair::generate()` — a random key unrecoverable from the phrase. `bls_is_seed_derived()` (`wallet.rs:309`) exists as a detector. |
| 11 | Cost digest: 4-6 wk, 9-10 modules, ~900-1400 LOC, ~2 ms/block at N=45 with mandatory epoch key cache (202 ms naive), +96 B/block, +67 % gossip, zero block-format change | **CARRIED FORWARD, not re-derived** | Source: `.claude/worktrees/attestation-bls-impact/docs/improvements/attestation-bls-verification-improvement.md` §5.2-5.3. Its §5.5 R2 (seed-phrase bricking, "blocking") is now **retired** by item 10. Its §5.1 identifies the same-message constraint this analysis independently confirms. |
| 12 | Prior doc lives on branch `worktree-attestation-bls-impact` | **REFUTED (path)** | `git show worktree-attestation-bls-impact:docs/improvements/…` fails — the file is not committed on that branch. It exists only in the working tree at `.claude/worktrees/attestation-bls-impact/docs/improvements/attestation-bls-verification-improvement.md` (506 lines). **It is one `rm -rf` from being lost.** |
| 13 | Its SSF was Option B, superseded by USER DECISION 2026-08-17 = Option A | **VERIFIED** | `decisions` id=48 (conf 0.8, still `status='active'` in the DB — should be marked superseded); `incident_entries` INC-I-178 note, agent `manual` |
| 14 | No `failed_approaches` rows for attestation/BLS | **VERIFIED** | zero rows matching `domain LIKE '%attest%' OR approach LIKE '%BLS%' OR …` |

**Invariants that bind this work** (from `invariants`): INV-AUTH-002 (a wire value may influence consensus only if locally derived or signature-bound), INV-ATTEST-001 (attend on `is_active()`, not `weight>0`), INV-CONSENSUS-001 (any change to the eligible-producer function needs an AH; pre-AH must be bit-identical), INV-DEPLOY-001 (block-content change needs an AH), INV-PARAMS-001 (never move a crossed AH forward), INV-FINALITY-001 (finality monotonic + non-erasable), INV-KEY-001 (wallet BLS key = seed-derived = on-chain `bls_pubkey`), INV-EPOCH-001 (never bump `CURRENT_PROTOCOL_VERSION` for this).

---

## 3. Git archeology of the BLS path

| Commit | Date | What it did |
|---|---|---|
| `30903c8b` / `6ed9df93` | 2026-03-10 | **Introduced everything.** v25 genesis reset, "BLS attestation". +953 LOC `crates/crypto/src/bls.rs`, +304 `attestation.rs`, `aggregate_bls_signature` on `Block`, `producer_bls_keys` on `ValidationContext`, `validate_bls_aggregate`, `bls_pubkey`/`bls_pop` on `RegistrationData`, RPC surface. |
| `80fa5c08` | 2026-04-03 | "3 missed bitfield decode sites post-activation (BLS, RPC, startup)" — the decoder-parity class of defect, already. |
| `86bac138` | 2026-07-19 | **Deleted the validator.** Removed `validate_block()`, `validate_bls_aggregate()`, `ValidationContext.producer_bls_keys`, 2 re-exports. −151/+3. Commit message documents the INC-I-075 three-question checklist and concludes "behavior is bit-identical for ALL reachable inputs (the code never executed)". |
| `427d5050` | 2026-07-19 | **Stopped producing the aggregate.** `aggregate_bls_signatures()` deleted; `production/mod.rs` emits `Vec::new()`; `startup.rs` drops `Attestation::new_with_bls`. Explicitly called ROLLING-SAFE with no AH, on the ground that the field is uncommitted so mixed-version hashes stay identical. |
| `13daee6f` | 2026-08-28 | INC-I-191/192 fix: authority derived from the local ProducerSet at both ingresses. Not an aggregate change; the security fix that memory believes is still open. |

**What remains on main today:** the full BLS crypto library (§5.4); `bls_pubkey`/`bls_pop` mandatory + PoP-verified at registration; `ProducerInfo.bls_pubkey` populated; `Block.aggregate_bls_signature` field on wire + in storage + in RPC output, always empty; `Attestation.bls_signature` field on wire, always empty from the production path but populated on ingest; `MinuteAttestationTracker` BLS storage with a live writer and no reader; `Attestation::new_with_bls` and `RegionAggregate::from_attestations` with zero callers.

---

## 4. Current architecture map

### 4.1 Data flow (ASCII)

```
                 ┌──────────────────────── ATTESTER SIDE ─────────────────────────┐
 block applied ─►│ post_commit.rs:446  create_and_broadcast_attestation(...)       │
 (every node)    │   └► startup.rs:591  Attestation::new(...)   Ed25519 ONLY       │
                 │        signs ATTESTATION_DOMAIN || block_hash || slot           │
                 │        bls_signature = <empty>          [new_with_bls: 0 callers]│
                 │ post_commit.rs:450  ALSO bls_sign(parent) ─► minute_tracker      │  ◄── dead: no reader
                 └──────────────────┬────────────────────────────────────────────┘
                                    │ gossip topic ATTESTATION_TOPIC / SyncRequest::DirectAttestation
        ┌───────────────────────────┴───────────────────────────┐
        │ INGRESS A1  network_events.rs:558 on_new_attestation   │   INGRESS A2  network_events.rs:301/345
        │  Attestation::verify()   (block_hash||slot only)       │    on_sync_request → record_direct_attestation
        │  block must be known locally (get_height_by_hash)      │    Attestation::verify()
        │  derive_attester_weight(LOCAL ProducerSet)  ── None ─► DROP  (INC-I-191/192, 13daee6f)
        └──────────┬──────────────────────────────┬──────────────┘
                   │ weight>0                     │ is_some()
                   ▼                              ▼
   ┌───────────────────────────┐    ┌──────────────────────────────────────┐
   │ FINALITY (live gossip)    │    │ MinuteAttestationTracker             │
   │ production_gate.rs:500    │    │ attestation.rs:375  keyed (pubkey,   │
   │  add_attestation_weight   │    │  minute); stores BLS sig (unread)    │
   │ finality.rs:111/153       │    └───────────────┬──────────────────────┘
   │  67% of local denominator │                    │ attested_in_minute(minute)
   │  *** NEVER READS THE      │                    ▼
   │      BLOCK BITFIELD ***   │   ┌────────────────────────────────────────────────┐
   └───────────────────────────┘   │ ENCODER (sole site) assembly.rs:389-455         │
                                   │  base = epoch_state.producer_list               │
                                   │  extra = active_at(h) \ base, sorted by pubkey   │
                                   │  width = base.len() + extra.len()               │
                                   │  presence_root = BLAKE3(bitfield)               │
                                   │  aggregate_bls_signature = Vec::new()  (mod:601)│
                                   └───────────────┬─────────────────────────────────┘
                                                   │ block gossip
                                                   ▼
   ┌─────────────────────────────────────────────────────────────────────────────────┐
   │ VALIDATION  validation_checks.rs:421-446                                         │
   │   presence_root == BLAKE3(attestation_bitfield)       ← commitment only          │
   │   validate_attestation_bitfield_vec(bitfield, active_producers_at_height(h).len())│
   │   *** NO CHECK THAT ANY SET BIT CORRESPONDS TO A REAL ATTESTATION ***            │
   └───────────────┬───────────────────────┬───────────────────────┬─────────────────┘
                   ▼                       ▼                       ▼
      post_commit.rs:59-165       rewards.rs:139 / :814 / :1016    rpc/schedule.rs:306
      epoch_state.accumulate_block  epoch reward qualification     getAttestationStats
      → next-epoch producer_list    (54/60 minutes)                (observability)
      → SCHEDULING ELIGIBILITY      → MONEY
```

### 4.2 Both attestation ingress paths (count = 2)

| # | Site | Entry | Auth today |
|---|---|---|---|
| A1 | `bins/node/src/node/network_events.rs:558` `on_new_attestation` | gossip `ATTESTATION_TOPIC` | `Attestation::verify()` (Ed25519 over `block_hash‖slot`) + block known locally + `derive_attester_weight(...).is_some()` |
| A2 | `bins/node/src/node/network_events.rs:301` → `:345` `record_direct_attestation` | `SyncRequest::DirectAttestation` | `Attestation::verify()` + `derive_attester_weight(...).is_some()` |

Both feed the same two sinks (finality accumulator, minute tracker). Both are now membership-gated.

### 4.3 Every live bitfield encoder / decoder (count: 1 encoder, 5 decoders, 1 stray-bit validator)

| Role | file:line | Denominator used |
|---|---|---|
| **ENCODER** | `bins/node/src/node/production/assembly.rs:451` (post-AH) / `:455` (legacy) | `epoch_state.producer_list.len() + extra.len()` |
| DECODER 1 | `bins/node/src/node/apply_block/post_commit.rs:61` / `:66` | `[base \| extra sorted]` — **matches the encoder** |
| DECODER 2 | `bins/node/src/node/rewards.rs:139` / `:145` | `sorted_producers.len()` = `epoch_state.producer_list.len()` (**base only**) |
| DECODER 3 | `bins/node/src/node/rewards.rs:814` / `:819` | `sorted_for_decode.len()` (base only) |
| DECODER 4 | `bins/node/src/node/rewards.rs:1016` / `:1021` | base only |
| DECODER 5 | `crates/rpc/src/methods/schedule.rs:306` / `:311` | `sorted_producers.len()` (base only) |
| STRAY-BIT VALIDATOR | `bins/node/src/node/validation_checks.rs:438` | `active_producers_at_height(height).len()` — **a fourth denominator** |

Cross-checked against the code graph: `python3 .claude/scripts/blast.py graphify-out/graph.json "crates/core/src/attestation.rs" --hops 2` returns exactly `post_commit.rs`, `mod.rs`, `network_events.rs` (3 sites), `assembly.rs`, `rewards.rs`, `startup.rs`, `rpc/methods/schedule.rs`. No module found by the graph was missed by grep, and none missed by the graph was found by grep. (Graph is a lower bound for Rust `self.method()`; grep was authoritative.)

### 4.4 Hash-commitment boundary of the block

`BlockHeader::hash()` (`crates/core/src/block.rs:76-97`) commits: `version`, `prev_hash`, `merkle_root`, **`presence_root`**, `genesis_hash`, `missed_producers` (len + each), `data_root`, `fork_id` (only when non-zero), `timestamp`, `slot`, `producer`, `vdf_output.value`. It does **not** commit `vdf_proof`. `Block::hash()` == `header.hash()` (`block.rs:187`).

Body: `transactions` (via `merkle_root`), `attestation_bitfield` (**transitively committed** via `presence_root = BLAKE3(bitfield)`), `aggregate_bls_signature` (**not committed by anything**).

### 4.5 Module boundaries and dependency direction

- `crates/crypto` — leaf. BLS + Ed25519 primitives. Depended on by everything. No knowledge of blocks or producers.
- `crates/core` — types + pure validation. `attestation.rs` owns the codec + tracker; `block.rs` owns the hash; `validation/registration.rs` owns PoP. Depends on `crypto`. **Does not** depend on `storage` — this is why `producer_bls_keys` had to be copied into `ValidationContext` and is why it was never populated.
- `crates/storage` — `ProducerInfo.bls_pubkey` lives here (`producer/types.rs:166`). The keys the validator needs are here, on the other side of the `core` boundary.
- `crates/network` — `finality.rs` weight accumulation lives in `sync/manager/production_gate.rs`; independent of the bitfield.
- `bins/node` — the only place that can see `core` + `storage` + `network` together. Hence the encoder, the stray-bit validator, and all attendance decoders are here.

**Direction of the fix follows from this**: the aggregate validator cannot live in `crates/core/src/validation/` unless the pubkey set is injected — which is exactly the wiring that never happened. It belongs in `bins/node/src/node/validation_checks.rs`, beside the existing presence_root check, where `producer_set` is already read (`validation_checks.rs:432`).

---

## 5. Capability inventory (counted, named)

### 5.1 `TxType` — 24 variants (`crates/core/src/transaction/types.rs`)
`Transfer`=0, `Registration`=1, `Exit`=2, `ClaimReward`=3, `ClaimBond`=4, `SlashProducer`=5, `Coinbase`=6, `AddBond`=7, `RequestWithdrawal`=8, `ClaimWithdrawal`=9, `EpochReward`=10, `RemoveMaintainer`=11, `AddMaintainer`=12, `DelegateBond`=13, `RevokeDelegation`=14, `ProtocolActivation`=15, `PriceAttestation`=16, `MintAsset`=17, `BurnAsset`=18, `CreatePool`=19, `AddLiquidity`=20, `RemoveLiquidity`=21, `Swap`=22, `ZKSettle`=31.

Producer/key relevant: `Registration` (carries `bls_pubkey` + `bls_pop`), `Exit`, `AddBond`, `SlashProducer`, `DelegateBond`, `RevokeDelegation`.
**There is NO key-rotation transaction.** `ProducerInfo.bls_pubkey` is written only at `Registration` apply (`apply_block/tx_processing.rs:245`), at genesis completion (`genesis_completion.rs:134`), and on the epoch-state rebuild path (`rewards.rs:1302`, `:1550`). A producer that loses its BLS secret can only recover by `Exit` + re-`Registration`.

### 5.2 `OutputType` — 14 variants
`Normal`=0, `Bond`=1, `Multisig`=2, `Hashlock`=3, `HTLC`=4, `Vesting`=5, `NFT`=6, `FungibleAsset`=7, `BridgeHTLC`=8, `Pool`=9, `LPShare`=10, `ZKRollup`=13, `EncryptedContent`=14, `OraclePrice`=15.

### 5.3 Attestation on the wire
- Gossip topic: `ATTESTATION_TOPIC` (`crates/network/src/gossip/mod.rs:44`), published at `gossip/publish.rs:88`, dedup/staleness-classified at `gossip/staleness.rs:228,248`.
- `NetworkEvent` — 21 variants; attestation-carrying: `NewAttestation(Vec<u8>, PeerId)`, `NewVote(Vec<u8>)`, `NewHeartbeat(Vec<u8>)`.
- `SyncRequest` — 8 variants; `DirectAttestation { data }` is the second ingress.
- `Attestation` wire struct (`attestation.rs:15-34`): `block_hash`, `slot`, `height`, `attester`, `attester_weight`, `signature` (Ed25519), `bls_signature` (`#[serde(default)]`, always empty from the production path). **Signed bytes = `block_hash ‖ slot_be` only** (`attestation.rs:112-117`) — `height` and `attester_weight` are outside the signature; both are now ignored in favour of local derivation.

### 5.4 BLS primitives available (`crates/crypto/src/bls.rs`, 1101 lines)
Constants: `BLS_PUBLIC_KEY_SIZE=48`, `BLS_SIGNATURE_SIZE=96`, `BLS_KEY_INFO=b"DOLI-BLS-ATTESTATION-KEY-v1"`, `BLS_KEY_GEN_MIN_IKM=32`, `ATTESTATION_DST=b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_DOLI_ATTEST_V1"`.
Types: `BlsPublicKeyWrapped` (48 B), `BlsSecretKey` (32 B), `BlsSignature` (96 B), `BlsKeyPair`.
Functions: `bls_sign`, `bls_verify`, `bls_sign_pop`, `bls_verify_pop`, `bls_aggregate` (signature aggregation), `bls_verify_aggregate` (wraps blst `fast_aggregate_verify(true, msg, DST, pks)` — **same-message only**, aggregates the pubkeys internally), `BlsKeyPair::proof_of_possession`, `BlsSecretKey::from_seed` (EIP-2333 / draft-irtf-cfrg-bls-signature 2.3 `hkdf_mod_r`), `attestation_message(block_hash, slot) = block_hash ‖ slot_be`.
**Not present:** distinct-message `aggregate_verify`; any cached/pre-decompressed pubkey type; a `pks_validate=false` fast path (`bls_verify_aggregate` decompresses + subgroup-checks every key on every call).

### 5.5 Verification keys are already on-chain and PoP-verified
`bls_pubkey` **and** `bls_pop` are **mandatory** for every registration and PoP-verified: `crates/core/src/validation/registration.rs:46-56` (light path) and `:143-154` (full path), both calling `validate_bls_pop` → `crypto::bls_verify_pop` (`registration.rs:258-274`). Live probe of the local testnet seed (h=103207, v6.26.3): 7 producers, **7 with a non-empty `blsPubkey`**. The deleted validator did not fail for want of keys — it failed because `ValidationContext.producer_bls_keys` was never populated from `ProducerSet`.

### 5.6 Activation heights — 25 declared in `NetworkParams` (`network_params/mod.rs`)
`inc_i_026_scheduler`(182), `fork_id`(188), `encrypted_content`(205), `epoch_state_reorg`(211), `security_audit`(221), `encrypted_content_v2`(226), `ghost_exclusion`(233), `epoch_prune`(248), `inc_i_190_floor_bound`(261), `inc_i_068_weight_filter`(284), `received_delegation_cap`(318), `delegation_auth`(344), `addbond_cap_enforcement`(369), `withdrawal_holdings_gate`(374), `defi`(381), `amm`(412), `oracle`(449), `large_block`(467), `inc_i_092`(496), `inc_i_096`(518), `inc_i_147`(556), `maintainer_derivation`(611), `inc_i_173`(654), `inc_i_176_auth_binding`(783), `inc_i_204_fork_choice`(810). Plus the non-`_activation_height` gates `full_bitfield_decode_height` (mod.rs:194; **0 on all three networks**) and `rewards_epoch_list_fix_height`.

Selected values — mainnet / testnet: `inc_i_190_floor_bound` 332_664 / 58_000; `withdrawal_holdings_gate` 317_861 / 15_087; `inc_i_147` 129_500 / 80_700; `maintainer_derivation` 172_000 / 15_087; `inc_i_173` — / 25_500; `inc_i_176_auth_binding` 317_861 / 15_087; `inc_i_204_fork_choice` `u64::MAX` / 88_014; `oracle` `u64::MAX` / `u64::MAX`; `defi` 0 / `u64::MAX`. `BITFIELD_BODY_ACTIVATION_HEIGHT` is a **constant** `= 0` (`consensus/constants.rs:63`), not a param.
Ordering is runtime-enforced for exactly one pair (`#22 >= #20`) in `network_params/ordering.rs`, fail-open with `error!` substitution.

### 5.7 Test harness capabilities
`Node::new_for_test` (`bins/node/src/node/init.rs`); `bins/node/src/lib.rs` exposes the node for integration tests; `bins/node/src/node/attestation_authority_tests.rs` (in-crate, 3 tests, builds a real chain and drives both ingresses); `bins/node/tests/fork_recovery.rs` (11 integration tests); bitfield-encoding test harnesses already exist in `bins/node/tests/{inc_i_061_delegator_reward_address,inc_i_081_incomplete_store_aborts_slot,m_rc9_silent_vec_regression}.rs`.
Gauntlet: 17 scenarios (GS-001…GS-017). **GS-012** (`producer-identity-mismatch`) is the only attestation-adjacent one — read-only, guards INV-KEY-001 by checking every fleet wallet's BLS key against its on-chain registration. **No gauntlet scenario asserts bitfield integrity or the aggregate.**

---

## 6. Structural problems found

### P1 — The aggregate was cryptographically incoherent, not merely unwired (**blast radius: the whole design**)
The producer aggregated `minute_tracker.bls_sigs_for_minute(minute)` — signatures over **up to 6 different messages**, because a bit claims presence over a *minute* (`assembly.rs:389` `attested_in_minute`) and each attester signs `attestation_message(attested_block_hash, that_block_slot)`. The validator used `crypto::bls_verify_aggregate` → blst `fast_aggregate_verify` (`bls.rs:695`), which is **same-message only**. Worse, the message it used was `attestation_message(&block.hash(), block.header.slot)` — the **current** block's hash, which no attester can have signed, because attesters sign the block just *applied*, i.e. the parent (`post_commit.rs:446`). The only reason it never errored is the escape hatch `if bls_pubkeys.is_empty() { return Ok(()) }`, which always fired.
**Consequence for this redesign: "restore the deleted path" is not a viable option.** Any correct Option A must change what a bit *means*.

### P2 — Four different denominators for one bitfield (**blast radius: 5 decoders + 1 validator + epoch scheduling + rewards**)
Encoder width = `base + extra` (`assembly.rs:426`). `post_commit` decodes `base + extra`. All three `rewards.rs` decoders and the RPC decoder use **base only**. The stray-bit validator uses `active_producers_at_height(h).len()`. The deleted BLS validator used a fifth ("sorted by Ed25519 pubkey"). This is the exact failure surface of the Full Bitfield Decode death spiral (v6.17.1, h=14000). **A BLS aggregate validator must reconstruct the pubkey set over `[base | extra sorted]` (post_commit semantics), never the base-only reward semantics** — otherwise every block containing a mid-epoch-activated attester fails verification.
Sub-risk, independent of BLS: the stray-bit validator's denominator can be **smaller** than the encoder's whenever `epoch_state.producer_list` holds a producer no longer active at `h`, which would reject an honestly-built block. Not observed; worth a targeted test.

### P3 — Duplicated own-attestation logic, one copy still doing dead BLS work (**blast radius: memory footprint of every node**)
`assembly.rs:665` (`attest_own_block`, producer path) and `post_commit.rs:446-459` (every applied block, every node) are near-duplicates. `427d5050` de-BLS'd only the first. The second still calls `crypto::bls_sign` per block and stores 96 B per (pubkey, minute) into `minute_tracker` via `record_with_bls`; both gossip ingresses do the same (`network_events.rs:371`, `:607`). The sole reader `bls_sigs_for_minute` (`attestation.rs:439`) has zero callers. This is per-block signing cost and retained bytes for nothing — and plausibly a contributor to the unexplained ~15-27 MB/day/node climb tracked as INC-I-146.

### P4 — `Block.aggregate_bls_signature` has no commitment path (**blast radius: block relay**)
Not covered by `merkle_root`, not by `presence_root`, not by `header.hash()`. Today it is always empty so nothing breaks. The moment it carries a signature, any relaying peer can strip or corrupt it without changing the block hash. Fixing this by adding a header field is a header-format change (expensive). Fixing it by redefining `presence_root`'s **preimage** is free — see §7.

### P5 — Dead BLS surface that will mislead the next reader (**blast radius: comprehension**)
`Attestation::new_with_bls` (`attestation.rs:74`): 0 callers. `RegionAggregate::from_attestations` (`attestation.rs:157`): 0 non-test callers. `MinuteAttestationTracker::bls_sigs_for_minute` / `bls_sig_count`: 0 callers. `Attestation.height`: no production consumer. `Attestation.attester_weight`: no consumer since `13daee6f`. `Block.aggregate_bls_signature`: written empty, read only by RPC display (`rpc/types/block.rs:82`) and a counter (`schedule.rs:320`). Six symbols that look like a working feature and are not.

### P6 — `Wallet::add_bls_key` still generates a random BLS key (**blast radius: any producer that adds a key post-hoc**)
`crates/wallet/src/wallet.rs:405` and `bins/cli/src/wallet.rs:497` call `BlsKeyPair::generate()`. INC-I-162 fixed the create and restore paths, not this one. Post-activation, a producer whose runtime BLS key is unrecoverable from its phrase can lose it, be unable to attest, drop below 54/60, and be filtered out of the active set. This must close before any AH is pinned.

### P7 — Published specs assert behavior the code does not implement (**blast radius: trust**)
`specs/protocol.md:1159` states the aggregate "stores the aggregated BLS signatures of producers whose bits are set" — false, it is always empty. `specs/protocol.md:1487-1488` and `specs/security_model.md:629-630` describe `bls_pubkey`/`bls_pop` as "optional, default empty" — false, both are mandatory and PoP-verified (`registration.rs:46-56`). The known-active WHITEPAPER §10.3 hotfix is the same class.

---

## 7. Redesign acceptance criteria

### 7.1 Behavior that MUST be preserved (non-negotiable)
| # | Property | Why |
|---|---|---|
| B1 | Chain continuity — no genesis reset, no reorg, no state-root change for any block below the AH | CLAUDE.md #0 RULE |
| B2 | Pre-AH block bytes bit-identical to the current binary (same `presence_root`, same empty aggregate, same bit semantics) | INV-CONSENSUS-001, INV-DEPLOY-001 |
| B3 | State-root convergence for snap-synced nodes across the AH | 3-states invariant |
| B4 | Reward semantics pre-AH unchanged; post-AH the epoch qualifier set must be measurably equivalent | INV-EPOCH-002, money |
| B5 | Index parity across all decode sites under the new semantics | Full Bitfield Decode pillar |
| B6 | Attendance admission stays `is_active()`, never `weight > 0` | INV-ATTEST-001 |
| B7 | INC-I-191/192 local-derivation posture at BOTH ingresses stays intact | INV-AUTH-002 |
| B8 | Finality remains monotonic and non-erasable; finality must NOT start consuming the bitfield | INV-FINALITY-001 |
| B9 | `CURRENT_PROTOCOL_VERSION` unchanged; `EPOCH_STATE_FORMAT_VERSION` unchanged unless `EpochState` layout changes | INV-EPOCH-001, INC-I-054 |
| B10 | Existing block wire format unchanged — pre-AH peers must still deserialize post-AH blocks | ~30 external auto-update producers |
| B11 | Liveness: a producer that cannot build a valid aggregate must still be able to produce a block | INC-I-154 |

### 7.2 Structural properties to IMPROVE
| # | Property |
|---|---|
| I1 | Every set bit is bound to a signature the named producer actually made |
| I2 | The aggregate is hash-committed, so it cannot be stripped in transit |
| I3 | ONE canonical bitfield universe function shared by encoder, aggregate validator, and `post_commit` — not five hand-rolled orders |
| I4 | The verifier is reachable from the live apply path and its execution count is observable |
| I5 | Zero dead BLS symbols left behind (P3, P5 drained) |
| I6 | The published spec claim becomes true as written |

### 7.3 Metrics that matter (must be measured, not asserted)
| Metric | Target / bound |
|---|---|
| Aggregate verify µs/block, N=45 | < 3 ms with an epoch-scoped decompressed-pubkey cache; naive path (~200 ms at N=1000) is disqualifying |
| Aggregate verify µs/block, N=1000 | < 5 ms, N-independent pairing + cached key adds |
| Added bytes/block | +96 B flat (existing field) |
| Added gossip bytes/attestation | +104 B (~156 → ~260, +67 %) |
| Non-test LOC touched | budget ~900-1400; report actual |
| Modules touched | budget 9-10; report actual |
| Decoder count after | ≤ 5, all resolving the universe through **one** shared function |
| Epoch qualifier-set delta under new semantics | **Zero** on a replayed real epoch, or an explained, bounded delta |
| Fleet BLS-key match rate before AH pin | 100 % via GS-012, on both testnet and mainnet |

### 7.4 Non-foreclosure constraint (Rule 18, redesign variant)
The chosen structure MUST NOT re-create any of:
- an aggregate field with no wired validator (P1's actual failure);
- a validator with zero call sites (the `validate_block` vs `validate_block_with_mode` split);
- a spec claim the code does not enforce (P7).

Concretely, the design is only accepted if it ships with: (a) a negative test proving a forged bit is rejected **through `Node::validate_block_for_apply`**, not through a bare validator function; (b) a `[ATTEST_VERIFY]` counter metric so "zero executions in production" is visible; (c) a gauntlet scenario asserting bitfield integrity end-to-end on the live fleet.

Known evolution path, captured as **Should**, not built now: 1000s of producers in 10 s slots (the pairing is already N-independent; the cost that grows is key decompression — hence the cache is structural, not an optimisation), and omission honesty (a producer zeroing bits to deny rewards or kick a rival) which stays **Won't** for this iteration.

---

## 8. Requirements

### 8.1 Must

| ID | Requirement | Acceptance criteria | Traceability |
|---|---|---|---|
| **REQ-BLS-001** | Post-AH, bit *i* MUST mean "producer *i* attested this block's parent (`parent_hash ‖ parent_slot`)", giving one common message per block. | - [ ] Given a block at height H post-AH, when the validator recomputes the message, then it equals `attestation_message(block.header.prev_hash, parent.header.slot)`.<br>- [ ] Given a block at H < AH, when validated, then bit semantics and `presence_root` are byte-identical to the current binary.<br>- [ ] Given an attester that attested the parent, when the producer builds, then its bit is set. | `production/assembly.rs`, `validation_checks.rs` |
| **REQ-BLS-002** | Post-AH, `Block.aggregate_bls_signature` MUST be the BLS aggregate of exactly the attestations whose bits are set, and MUST verify. | - [ ] Given a correct block, when validated, then `bls_verify_aggregate` returns Ok.<br>- [ ] Given a block with one extra bit set, when validated through `validate_block_for_apply`, then the block is REJECTED.<br>- [ ] Given a block with a valid bitfield and an empty aggregate, when validated post-AH, then REJECTED.<br>- [ ] Given a zero bitfield, when validated, then the aggregate MUST be empty and the block accepted. | `validation_checks.rs`, `crypto/bls.rs` |
| **REQ-BLS-003** | Post-AH, `presence_root` MUST equal `BLAKE3(attestation_bitfield ‖ aggregate_bls_signature)`, hash-committing the aggregate with no header change. | - [ ] Given a post-AH block, when the aggregate is stripped or altered by a relay, then `presence_root` no longer matches and the block is rejected.<br>- [ ] Given `BlockHeader::hash()`, when compared before/after, then the field list is unchanged.<br>- [ ] Given H < AH, then `presence_root == BLAKE3(bitfield)` exactly as today. | `block.rs` (unchanged), `assembly.rs`, `validation_checks.rs` · **M0 tests** `bins/node/tests/it/inc_i_178_m0_attestation_lock.rs`: `req_bls_003_ac2_presence_root_commitment_is_enforced_for_a_full_bitfield`, `..._for_a_sparse_bitfield`, `req_bls_003_ac2_stray_bits_beyond_producer_count_are_rejected_today`, `req_bls_003_ac3_empty_body_bitfield_bypasses_the_commitment_check_today`; `bins/node/tests/it/inc_i_178_m0_block_identity.rs`: `req_bls_003_ac3_block_header_hash_covers_the_header_only` |
| **REQ-BLS-004** | The verification key set MUST be reconstructed from the ONE canonical universe `[epoch_state.producer_list \| (active_at(h) \ base) sorted by pubkey bytes]`, via a single shared function used by the encoder, the aggregate validator, and `post_commit`. | - [ ] Given a block containing a mid-epoch-activated attester, when validated, then the aggregate verifies.<br>- [ ] Given the shared function, when grepped, then encoder, validator and `post_commit` all call it and none re-implements it.<br>- [ ] Property test: encode→decode round-trip identity at N ∈ {45, 200, 1000}. | new shared fn in `bins/node/src/node/`, `assembly.rs:410-432`, `post_commit.rs:37-57` · **M0 tests** `bins/node/tests/it/inc_i_178_m0_attestation_lock.rs`: `req_bls_004_ac3_encode_decode_identity_at_45_producers`, `..._at_200_producers`, `req_bls_004_ac1_mid_epoch_activated_attester_keeps_its_universe_index`, `req_bls_004_universe_prefix_property_matches_the_real_encoder` (N=1000 NOT covered at M0) |
| **REQ-BLS-005** | The whole change MUST be gated behind ONE new forward-only `NetworkParams` activation height. No `HardForkSchedule` entry. No `CURRENT_PROTOCOL_VERSION` bump. | - [ ] Given H < AH, when a new-binary node builds a block, then the bytes are identical to an old-binary node's (mixed-fleet hash equality test).<br>- [ ] Given the diff, when inspected, then `crates/updater/src/hardfork.rs` and `CURRENT_PROTOCOL_VERSION` are untouched.<br>- [ ] Mainnet default ships as `u64::MAX`; pinning is a separate decision-session. | `network_params/{mod,defaults,env_loader}.rs` · **M0 tests (AC-1 baseline only)** `bins/node/tests/it/inc_i_178_m0_block_identity.rs`: `req_bls_005_ac1_builder_bitfield_and_presence_root_are_deterministic`, `req_bls_005_ac1_prebuilt_block_bytes_are_byte_identical_within_one_slot`. AC-2/AC-3 are diff-inspection criteria, not testable at M0 (no AH field exists yet) |
| **REQ-BLS-006** | Attesters MUST dual-sign (Ed25519 + BLS) on the gossip wire, and this MUST ship in an EARLIER release than the AH-gated validator. | - [ ] Given a node on release N, when it attests, then `Attestation.bls_signature` is 96 bytes.<br>- [ ] Given a fleet probe, when run before the AH is pinned, then 100 % of active producers emit BLS-signed attestations.<br>- [ ] Given release N, when deployed rolling, then no block content changes (gossip-only). | `startup.rs:591-608`, `attestation.rs:74` |
| **REQ-BLS-007** | Verification MUST be reachable from the live apply path, and its execution MUST be observable. | - [ ] A negative test drives a forged bit through `Node::validate_block_for_apply` and asserts rejection.<br>- [ ] A counter metric increments once per post-AH block validated.<br>- [ ] The counter is non-zero on testnet within one epoch of the AH. | `validation_checks.rs`, `bins/node/src/metrics.rs` |
| **REQ-BLS-008** | Every active producer's runtime BLS key MUST match its on-chain `bls_pubkey` before the AH is pinned, and `Wallet::add_bls_key` MUST derive from the seed. | - [ ] GS-012 passes 100 % on testnet AND mainnet immediately before pinning.<br>- [ ] `BlsKeyPair::generate()` no longer appears on any wallet key-creation path.<br>- [ ] A producer restoring from phrase reproduces its registered BLS key. | `wallet.rs:401-411`, `bins/cli/src/wallet.rs:497`, GS-012 |
| **REQ-BLS-009** | Post-AH epoch reward qualification MUST be equivalent to pre-AH on real data. | - [ ] Replay ≥1 full epoch (360 blocks) of real testnet blocks under both semantics; the qualifier set is identical, or every difference is explained and bounded.<br>- [ ] `54/60` threshold and the 3-epoch liveness filter are untouched. | `rewards.rs:139/814/1016`, `post_commit.rs` |
| **REQ-BLS-010** | Liveness MUST NOT regress: inability to build a valid aggregate MUST NOT block block production. | - [ ] Given a producer holding zero BLS-signed attestations, when its slot arrives, then it produces a block with an empty bitfield and empty aggregate, and the block is accepted.<br>- [ ] Chaos test: half the fleet stops BLS-signing post-AH; block production continues. | `assembly.rs`, `validation_checks.rs` |

### 8.2 Should

| ID | Requirement | Acceptance criteria |
|---|---|---|
| **REQ-BLS-011** | An epoch-scoped decompressed-BLS-pubkey cache, with a `pks_validate=false` verify variant (keys are PoP-checked at registration). | - [ ] Measured verify < 3 ms/block at N=45 and < 5 ms at N=1000.<br>- [ ] Cache invalidated at every epoch boundary and on any ProducerSet mutation.<br>- [ ] A test asserts the cached path and the naive path accept/reject identically. |
| **REQ-BLS-012** | Drain the dead BLS surface (P3, P5): delete or wire `new_with_bls`, `from_attestations`, `bls_sigs_for_minute`, `bls_sig_count`, `Attestation.height`, `attester_weight`. | - [ ] Zero non-test `pub` symbols in the attestation module without a production caller, or a row in `docs/.workflow/wiring-debt.md`.<br>- [ ] The duplicated own-attestation logic exists in exactly one place. |
| **REQ-BLS-013** | A gauntlet scenario asserting bitfield integrity end-to-end on the live fleet. | - [ ] Registered in `gauntlet_scenarios`; passes on testnet; fails if the aggregate is stripped. |
| **REQ-BLS-014** | Reconcile the stray-bit validator denominator with the encoder width (P2 sub-risk). | - [ ] A test constructs an epoch where `producer_list` holds a producer inactive at H, and asserts an honestly-built block is accepted. |
| **REQ-BLS-015** | Update `specs/protocol.md` (:1159, :1487-1488), `specs/security_model.md` (:629-630), and the WHITEPAPER §10.3 claim to match the shipped code. | - [ ] Every statement about the aggregate and about `bls_pubkey`/`bls_pop` optionality matches `registration.rs` and `validation_checks.rs`. |
| **REQ-BLS-016** | Commit the prior cost analysis (currently only in an uncommitted worktree) into the repo. | - [ ] `docs/improvements/attestation-bls-verification-improvement.md` exists on a tracked branch. |

### 8.3 Could
| ID | Requirement | Acceptance criteria |
|---|---|---|
| **REQ-BLS-017** | RPC surfaces per-block aggregate verification status. | - [ ] `getBlock` / `getAttestationStats` report verified/unverified/pre-AH. |
| **REQ-BLS-018** | Batch/parallel aggregate verification during bulk sync. | - [ ] Measured sync throughput regression < 5 % post-AH. |

### 8.4 Won't (this iteration)
| ID | Item | Why |
|---|---|---|
| **REQ-BLS-019** | Omission honesty — a producer zeroing bits to deny rewards or kick a rival | Explicitly out of scope per the run brief; requires a separate mechanism (attestation inclusion proofs or a challenge game) |
| **REQ-BLS-020** | Making finality consume the on-chain bitfield | Finality is a live-gossip weight accumulator that never reads the bitfield (§4.1); coupling them would put INV-FINALITY-001 at risk for no gain |
| **REQ-BLS-021** | BLS key rotation transaction | No `TxType` supports it (§5.1); worth its own incident, not a rider on this one |
| **REQ-BLS-022** | Header format change to carry the aggregate | Unnecessary — REQ-BLS-003 achieves the commitment through the existing `presence_root` preimage |
| **REQ-BLS-023** | Restoring the deleted `validate_bls_aggregate` verbatim | It was cryptographically incoherent (P1) and used a sixth index order |

---

## 9. Assumptions and clarifying questions (user not available mid-run)

| # | Question | Working assumption | Confirmed |
|---|---|---|---|
| 1 | Redefining a bit from "attested some block this minute" to "attested this block's parent" is a consensus-visible semantic change. Acceptable? | **Yes, and it is unavoidable** — no same-message aggregate exists under minute-union semantics (P1). REQ-BLS-009 makes acceptance conditional on a measured zero qualifier delta. | No |
| 2 | Is +67 % attestation gossip acceptable at the current fleet size? | Yes at N≈45; REQ-BLS-011 keeps the verify cost flat as N grows. | No |
| 3 | Should mainnet ship the AH as `u64::MAX` and pin later (oracle/DeFi pattern) or pin immediately? | **`u64::MAX` first.** Pinning is a separate decision-session after testnet soak and a 100 % GS-012 pass, per HC-6 / INC-I-075 precedent. | No |
| 4 | Do INC-I-191/192 need any further work? | **No.** Fixed on main by `13daee6f`; the DB rows are stale. Recommend closing them with a pointer to that commit. | No |
| 5 | Is the ~15-27 MB/day/node climb (INC-I-146) partly the unread BLS sigs in `minute_tracker` (P3)? | **Plausible, unproven.** Do not fold it into this work; note it on INC-I-146 as a candidate. | No |
| 6 | Should P3's duplicated own-attestation logic be de-duplicated in this work or separately? | **In this work** — it is the same function that must start dual-signing (REQ-BLS-006), and two copies is how `427d5050` half-landed. | No |
| 7 | Does the ordering module need a new invariant for this AH? | Assume **no runtime pair constraint** is required; the only hard rule is "future height on every network", which INV-PARAMS-001 already covers. | No |

### What I do NOT understand (mandatory disclosure)
1. **Whether the minute-boundary edge effects of REQ-BLS-001 are benign.** Under the new semantics an attester whose attestation arrives after block S was built but before S+1 gets exactly one bit instead of six. Minute-level union should still credit the minute, but I have not measured this against real attestation-arrival timing. REQ-BLS-009 exists precisely because I cannot answer it from static reading.
2. **The real mainnet BLS key coverage.** I probed only the local testnet seed (7/7). Mainnet has ~30 external auto-update producers I did not query, and a producer registered with an `add_bls_key`-generated random key (P6) would pass registration and fail post-AH.
3. **Why `full_bitfield_decode_height` is 0 on all three networks** when the Full Bitfield Decode pillar describes an activation at h=14000. Either the constant was re-pinned after a later genesis reset or the pillar doc has drifted. I did not resolve this, and it matters because it determines whether the `[base | extra]` decode is universally active.
4. **Whether `BITFIELD_BODY_ACTIVATION_HEIGHT = 0` as a compile-time constant (not a param) is deliberate.** It means the pre-body legacy branches in all 5 decoders are unreachable today, which is either safe dead code or a latent trap on a future network.
5. **The exact blst behaviour of `fast_aggregate_verify(true, …)` regarding pubkey subgroup validation.** I read the call site, not blst. The `pks_validate=false` optimisation in REQ-BLS-011 depends on this and must be benchmarked, not assumed.

---

## 10. Deploy-shape answer

### 10.1 INC-I-075 three-question consensus-shape checklist (INV-12)
1. **Can a user-submittable tx reach this path?** **YES.** `Registration` supplies the `bls_pubkey` that becomes a verification key, and can change the active set that defines the bitfield universe.
2. **Can a producer action or attestation pattern reach it?** **YES.** Every block carries the bitfield, `presence_root`, and the aggregate.
3. **Is the new behavior bit-identical for ALL reachable inputs?** **NO.** Bit semantics change, the `presence_root` preimage changes, and a new rejection path appears.

**(1|2) YES + (3) NO ⇒ ACTIVATION HEIGHT REQUIRED.**

### 10.2 Rules vs content
**Both.** Consensus *rules* change (a block can now be rejected for a bad aggregate). Block *content* changes (`presence_root` takes a different value for the same attestation set). Because `presence_root` is inside `BlockHeader::hash()` (`block.rs:81`), a mixed-version fleet **at or above** the AH would produce different block hashes for the same slot — a fork. INV-DEPLOY-001 and INC-I-062 both apply.

### 10.3 Proposed `NetworkParams` field
```
pub inc_i_178_attestation_bls_activation_height: u64
```
Defaults: mainnet `u64::MAX` (frozen pre-activation, oracle/DeFi pattern), testnet a concrete future height pinned at rollout time, devnet `0`. Add to `mod.rs`, `defaults.rs`, `env_loader.rs`. **Do NOT** add a `HardForkSchedule` entry (`current_fork_id(u64::MAX)` activates all entries immediately). **Do NOT** bump `CURRENT_PROTOCOL_VERSION` — `EpochState` serialization is unchanged.

### 10.4 Rolling vs synchronized
**Rolling-safe pre-activation, mandatory-upgrade at activation.** Pre-AH the new binary must emit byte-identical blocks (REQ-BLS-005 AC-1), so a rolling deploy is safe. At the AH, every node must already run the new binary — which is exactly what a forward-only AH buys, and why the mainnet AH must sit far enough ahead for all ~30 external auto-update producers to have upgraded. **No stop-all is possible on mainnet**, so the AH margin *is* the safety mechanism.

### 10.5 Activation ordering — the critical risk
A **two-release order is mandatory**, mirroring the INC-I-204 mainnet pin:

- **Release N (no AH, rolling-safe, gossip-only):** attesters dual-sign (REQ-BLS-006); `Wallet::add_bls_key` seed-derivation fix (REQ-BLS-008); dead-surface drain (REQ-BLS-012). Zero block-content change.
- **Gate between releases:** GS-012 at 100 % on both networks, plus a fleet probe showing every active producer emitting a 96-byte `bls_signature`. **Only then** is the height pinned.
- **Release N+1 (AH-gated):** new bit semantics, aggregate assembly, `presence_root` preimage change, validator, counter metric.

**Why the order is not negotiable.** If the validator activates before the fleet dual-signs, no producer can assemble a valid aggregate, every honest producer must publish a zero bitfield, attendance collapses to zero, and the 3-epoch liveness filter removes producers from the active set — the Full Bitfield Decode death spiral by a different door. Shipping both in one release and relying on the AH does **not** protect against this, because the AH gates the *validator*, not the *attester population*.

### 10.6 Relation to INC-I-191 / INC-I-192
**Separate, and already shipped — NOT the same activation height.** `13daee6f` changed only how a node derives an attester's authority from local state on a gossip-ingress path. It touches no block content and no consensus rule, so it correctly carries no AH. This redesign must **preserve** that posture (B7): the aggregate validator authenticates the *on-chain attendance record*; `derive_attester_weight` authenticates *live gossip*. Two different trust boundaries, two different mechanisms, one shared invariant (INV-AUTH-002). The DB rows for INC-I-191/192 should be closed against `13daee6f` rather than folded into INC-I-178.

### 10.7 What this does and does not close
Closes: inclusion honesty — a producer can no longer set a bit for a producer that did not attest (P1's stated goal), and the aggregate can no longer be stripped in transit (P4).
Does **not** close: omission honesty (REQ-BLS-019) — a producer can still *clear* bits to deny rewards or push a rival below 54/60. That remains the largest open attestation risk after this work lands, and it should be tracked as its own incident.

---

## 11. Brittleness check

```
━━━ BRITTLENESS CHECK ━━━
Signals detected: 4/5
  1. Cross-module blast radius — YES. The fix touches crypto, core/attestation, core/network_params,
     node/production, node/validation_checks, node/apply_block, node/rewards, rpc, wallet, cli — modules
     that do not share a direct dependency (core cannot see storage, which owns the keys).
  2. Invariant gaps — YES. No module enforces "a set bit corresponds to a real attestation." The
     invariant does not exist anywhere in the system today.
  3. Data flow reversal — NO. Data still flows attester → tracker → encoder → block → decoders.
     The fix adds a check on the existing direction.
  4. Shared mutable state — YES. minute_tracker is written by 3 sites (post_commit, 2 ingresses),
     read by 1 (encoder), with BLS sigs written by 3 and read by 0. No clear owner.
  5. Contract absence — YES. Encoder and its 5 decoders share no explicit interface; the "canonical
     bitfield universe" is re-implemented by hand at each site with 4 different denominators.
Verdict: BRITTLE
━━━━━━━━━━━━━━━━━━━━━━━━━
```

The bug is **architectural, not local.** REQ-BLS-004 (one shared universe function) is the structural remedy for signals 1 and 5; REQ-BLS-012 for signal 4; REQ-BLS-002 + REQ-BLS-007 create the missing invariant for signal 2. A design that adds a verifier without collapsing the five hand-rolled orders into one will re-enter this state on the next change.

---

## 12. Specs drift detected

| File:line | Drift | Correct statement |
|---|---|---|
| `specs/protocol.md:1159` | "stores the aggregated BLS signatures of producers whose bits are set" | Always `Vec::new()` since `427d5050`; never verified even before that |
| `specs/protocol.md:1487-1488` | `bls_pubkey` / `bls_pop` "optional, default empty" | Mandatory and PoP-verified (`validation/registration.rs:46-56`, `:143-154`) |
| `specs/security_model.md:629-630` | same optionality claim | same correction |
| `specs/protocol.md:1161` | "presence_root … is Hash::ZERO in the deterministic model … may contain a Merkle root of RegionAggregates" | It is `BLAKE3(attestation_bitfield)`; `RegionAggregate` has no production caller |
| WHITEPAPER §10.3 (EN+ES) | BLS rejects fake attestation bits | Known ACTIVE hotfix; unchanged by this analysis |

Per the proposal-only constraint, none of these were edited. They are REQ-BLS-015.
