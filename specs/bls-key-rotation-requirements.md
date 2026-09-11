<!--
OUTPUT CONTRACT: N/A — requirements document (not a test file)
INPUT PARTITIONS: N/A — requirements document (not a test file)
-->

# Requirements: Producer BLS Key Rotation

> Run 552 · `/omega-new-feature` · 2026-09-11 · Analyst · branch `feat/bls-key-rotation` (base `1f08d975`).
> Security requirements: [bls-key-rotation-requirements-security.md](./bls-key-rotation-requirements-security.md).
> Upstream: brief `docs/bugfixes/bls-key-rotation-session-prompt.md`; `docs/.workflow/{prompt-refinement,
> feature-evaluation,skeptic-analysis,bls-mismatch-census-2026-09-11}.md` (verdict CONDITIONAL, FVS 3.6).
> **Supersedes** open decision **O5** and analyst requirement **REQ-BLS-021 (Won't)** in
> `specs/attestation-bls-architecture.md:416` by the "rotation TxType shipped" branch, which closes
> precondition **P4** (`:562`).

---

## Stupid Simple First

**One new, last-declared, activation-gated `RotateBlsKey` transaction whose `extra_data` carries the
new BLS key plus two signatures — an Ed25519 signature by the producer over a rotation-specific
domain, and a proof of possession under a rotation-specific DST that binds the producer's Ed25519
identity — applied as an epoch-deferred `PendingProducerUpdate` in both apply implementations.**
This works because `ProducerInfo.bls_pubkey` already exists with `#[serde(default)]`
(`crates/storage/src/producer/types.rs:161-166`), so only its *value* changes and no canonical
encoding moves; the deferral queue, the undo snapshot and `getProducer` already carry the field.

*Non-foreclosure*: the same `RotateBlsKeyData` shape and DST family extend to Ed25519 producer-key
rotation later by adding one key-kind byte, without a second transaction type.

---

## Scope

**In scope**: `crates/core` (tx types + payload + validation + network params), `crates/crypto` (one
DST + two functions), `crates/storage` (one appended `PendingProducerUpdate` variant + apply arm),
`crates/mempool` (one admission module), `crates/rpc` (one derived field), `bins/node` (both apply
implementations, undo-snapshot predicate, log, metric), `bins/cli` (`rotate-bls`, `import-bls`), docs.

**Out of scope** (binding, from the brief + this analysis):
pinning the activation height on any network; deploying; the affected operators' delegation or
withdrawal decisions; the stale `maintainer_state.bin` problem (issue #200); the future-slot block
flood from producer `a9166b08…`; fixing INC-I-169 F1 (`extra_data` unsigned) — this feature works
**around** it, it does not fix it; Ed25519 producer-key rotation; any change to
`CURRENT_PROTOCOL_VERSION`, `EPOCH_STATE_FORMAT_VERSION` or any canonical encoding; any
`HardForkSchedule` entry.

---

## Summary (plain language)

Two mainnet producers earn nothing and cannot fix it. When they registered, their wallet stored a
BLS attestation key that the 24-word recovery phrase cannot reproduce (a wallet bug fixed in August).
They later restored from the phrase, so the key their node signs with no longer matches the key the
blockchain has on record. Since the attestation rules turned on at block 409,000 every attestation
they send is rejected, so they never reach the 54-of-60-minutes bar and receive no rewards — for
good. Thirty-three more producers are one wallet restore away from the same state, and the only way
out today is to leave the network and re-join, which burns up to 75 % of the stake and resets
seniority.

This change adds one new blockchain transaction that lets a producer publish a new attestation key,
proving it owns both the producer account and the new key. It ships switched off (activation height
`u64::MAX` on every network); turning it on is a separate decision session.

## User Stories

- As a **producer operator who restored a wallet from the seed phrase**, I want to publish a new
  attestation key, so that my node's attestations count again and I earn rewards without exiting.
- As a **producer operator whose BLS secret leaked**, I want to replace it, so that a thief cannot
  impersonate my attestations for the rest of the chain's life.
- As a **node operator**, I want a rotation visible in logs, metrics and `getProducer`, so that I
  can tell a rotation from a fault; and inert until an activation height is pinned, so that merging
  it cannot affect mainnet.
- As an **operator who still has the old BLS key in a file**, I want to import it into my wallet, so
  that I never need a consensus transaction at all.

---

## Architecture Context

### Module Boundaries

| Module | Responsibility | Depends on | Depended on by |
|--------|----------------|------------|----------------|
| `crates/core/src/transaction/{types,data,core}.rs` | `TxType` enum, payload structs, tx signing hash | `crates/crypto` | validation, mempool, node, cli, rpc |
| `crates/core/src/validation/{registration,tx_types,utxo}.rs` | stateless + contextual tx rules; `is_zero_flow` fee/balance exemption | transaction, storage | node apply, mempool, builder |
| `crates/core/src/network_params/` | 94 activation-height fields, per-network defaults | — | every consensus consumer |
| `crates/storage/src/snapshot.rs` | `compute_state_root` over ChainState + UtxoSet + ProducerSet | producer, utxo | node apply, sync |
| `crates/core/src/attestation/message.rs` + `crates/crypto/src/bls.rs` | preimage = block hash only (frozen, INC-I-178 R1); `POP_DST`, `ATTESTATION_DST`, sign/verify/aggregate | — | core, node |
| `crates/storage/src/producer/` | `ProducerInfo`, `PendingProducerUpdate`, `ProducerSet`, canonical serialization | crypto | node, rpc, snapshot |
| `bins/node/src/node/apply_block/` | **apply implementation #1** — queues deferred producer mutations | core, storage | block handling |
| `bins/node/src/node/rewards.rs:1117` | **apply implementation #2** — `rebuild_producer_set_from_blocks` | core, storage | rollback ×2, block_handling ×2 |
| `bins/node/src/node/attestation/` | ingress pooling, universe, commitment, verification | core, storage, crypto | block validation, production |
| `crates/mempool/src/` | admission; `pending_registrations.rs`, `vesting_bound.rs`, `addbond_cap.rs` are the parity modules | core, storage | node |

### Data Flows Through the Affected Area

1. **Write path A (apply)**: block → `apply_block/tx_processing.rs:245` writes
   `producer_info.bls_pubkey` for `Registration`, then `queue_update(PendingProducerUpdate::Register)`
   (`:247`) → epoch boundary → `ProducerSet::apply_pending_updates_with_cap` (`producer/set_core.rs`)
   → `producers` map → `serialize_canonical` → `compute_state_root`.
2. **Write path B (rebuild)**: `rewards.rs:1117` replays blocks 1..=h with its own
   `match tx.tx_type` (7 arms at `:1259, :1297, :1329, :1337, :1368, :1446, :1494`), writing
   `bls_pubkey` at `:1289`, and applies pending updates at its own boundary check (`:1544-1558`).
   **Four call sites**: `rollback.rs:144`, `rollback.rs:267`, `block_handling.rs:739`,
   `block_handling.rs:910`.
3. **Write path C/D (genesis)**: `genesis_completion.rs:134` and `rewards.rs:1537` backfill genesis
   keys from `genesis_bls_pubkeys()` (`node/genesis.rs:92`). These four are the ONLY writers.
4. **Read path (consensus)**: `attestation/verify.rs:93-106` builds the universe from
   `epoch_state.producer_list` + `active_producers_at_height` (**Ed25519 keys only**,
   `attestation/commit.rs:32-79`), then `keys::set_bit_bls_pubkeys` (`keys.rs:13-28`) looks the BLS
   key up **live** from the current `ProducerSet` and feeds `bls_fast_aggregate_verify` (`bls.rs:695`).
5. **Read path (pooling)**: `ingress.rs:67` verifies an incoming BLS half against the attester's
   on-chain key; `startup.rs:612` does the same for the node's own half and emits `[ATTEST_EGRESS]`.
6. **Read path (observability)**: `rpc/methods/producer.rs:128,224,258` already emit `blsPubkey`
   (so the brief's RPC item is verification, not work); `schedule.rs:245,276,379` emit a has-key flag.

### Architectural Constraints & Invariants

| Constraint | Why it exists | What breaks if violated |
|---|---|---|
| `bls_pubkey` is **not** a scheduler or universe input — the universe is Ed25519 (`commit.rs:32-79`) | INC-I-178 R1 froze the preimage and the universe shape | nothing today; the design must not make it one |
| `ProducerSet::serialize_canonical()` hashes **only** `producers` + `exit_history` (`set_persistence.rs:79-113`); `pending_updates` is **not** in the state root | determinism across nodes | a queued mutation is invisible to state-root comparison until the boundary |
| Adding **any** field to `ProducerInfo` changes `bincode::serialize` for **every** producer from binary start — `serialize_canonical(&self)` takes no height parameter (`:78`) | canonical bytes feed `compute_state_root` (`snapshot.rs:25-58`) | a state-root fork at **deploy**, before any activation height (skeptic C10) |
| `PendingProducerUpdate` is a bincode-serialized member of `ProducerSet`, which is the undo snapshot (`rollback.rs:185-188`) | reorg correctness | inserting a variant mid-enum reindexes bincode and breaks every stored undo snapshot and snap-sync payload |
| `TxType::allows_empty_io` is **exhaustive with no `_` arm** (`types.rs:120-152`) and is "curated by AUTHORIZATION" | INC-I-173 | a new variant is a build failure until classified — intentional |
| `Block::deserialize` = `bincode::deserialize(bytes).ok()` (`block.rs:246-248`); enum wire index is the **declaration ordinal**, not the `#[repr(u32)]` discriminant | wire compat | a variant inserted before `ZKSettle` silently renumbers `ZKSettle`; an unknown ordinal makes the **whole block** undecodable on an old node |
| Producer mutations are epoch-deferred (CLAUDE.md; `producer/types.rs:173-177`) | "prevents scheduler divergence between forks" | fork-dependent scheduler input |
| Every consensus rule reachable in `apply_block` must be reachable from mempool AND builder (INV-VALIDATION-001) | INC-I-147/180/203 | `[BLOCK_POISON] apply_block failed on self-produced block` |
| `Transaction::signing_message()` excludes `extra_data` (`core.rs:517-537`) | `b1b5fc5f`, covenant witnesses | **the outer tx signature authenticates no payload field** (INC-I-169 F1, still open) |

### Blast Radius

**Direct** (must change): `transaction/types.rs`, `transaction/data.rs`, `validation/tx_types.rs`,
`validation/utxo.rs` (gate arm only), `network_params/{defaults.rs,mod.rs}`, `crypto/src/bls.rs`,
`storage/producer/{types.rs,set_core.rs}`, `apply_block/{tx_processing.rs,helpers.rs}`,
`node/rewards.rs`, `mempool/src/{lib.rs,pool.rs,+pending_rotations.rs}`, `rpc/{types,methods}/producer.rs`,
`cli/cmd_producer/{mod.rs,dispatch.rs,+rotate.rs}`.

**Indirect** (must be proven unchanged): `snapshot.rs` state root for non-rotating producers;
`rollback.rs` undo path; `block_handling.rs` reorg path (×2); `attestation/{keys,verify,ingress,commit}.rs`;
`production/assembly.rs`; `network/sync` snap-sync payload; `rpc/methods/schedule.rs`; `bins/gui/`.

**Graph note (Rule 28)**: `graphify` is blind to Rust `self.method()` edges (MEMORY
`reference_graphify_rust_method_blind_spot.md`) — 0 hits for `rebuild_producer_set_from_blocks` and
`set_bit_bls_pubkeys`. Grep is the accepted fallback; every claim above carries a `file:line`.

---

## Design Decisions (evidence-based; not left to the architect)

### (a) New `TxType` vs an activation-gated `extra_data` extension — **NEW `TxType`, DECLARED LAST**

`RotateBlsKey = 23`, declared **after** `ZKSettle = 31` so its bincode ordinal is 24 and no existing
ordinal shifts. Evidence:
- The INC-I-078 precedent (`transaction/data.rs:232-246`, *"no new tx type number, just a longer
  extra_data"*) added an **authentication check to an existing mutation**. This feature adds a **new
  mutation**. No existing producer transaction means "rotate my BLS key", so overloading one (e.g.
  `Exit` or `Registration`) makes an un-upgraded node perform the **base** action — exit the
  producer, or reject the transaction — which is a silent, wrong state change.
- With a new ordinal an un-upgraded node cannot decode the block at all (`block.rs:246-248`): a loud
  stall/partition rather than a silent state-root fork. Precedent: `PriceAttestation = 16` shipped as
  a new `TxType` frozen at `oracle_activation_height = u64::MAX`. And `allows_empty_io` is exhaustive
  with no `_` arm, so a new variant is a build failure until classified — the `extra_data` route is not.
- The brief's criterion "must not collide (0-22 and 31 used)" **measures the wrong number**
  (skeptic C6, ACCEPTED). The binding invariant is *declare it last*. `from_u32` is a second,
  discriminant-based numbering surface (`types.rs:84-118`) that also needs the `23 => Some(...)` arm.

### (b) Epoch-deferred vs immediate mutation — **EPOCH-DEFERRED**, with the brief's criterion corrected

Skeptic open question 2 is **resolved**: `attestation/commit.rs` builds the universe from Ed25519
keys only (`:32-79`); no epoch snapshot caches BLS keys. `keys.rs:13-28` performs a **live lookup**
against the current `ProducerSet` at verify time.

⇒ **The brief's acceptance criterion "mid-epoch rotation does not change `active_producers`, the
scheduler input, the bitfield width or the presence commitment" is satisfied by the nature of the
field, not by the deferral** — it would hold under immediate mutation too. It therefore cannot be
used to justify deferral, and the test for it must assert the stronger, falsifiable property
(REQ-ROT-005).

Deferral is chosen on two other grounds: (i) CLAUDE.md law and the `PendingProducerUpdate` contract
(`producer/types.rs:173-177`) make uniform deferral an invariant worth more than one epoch of
latency; (ii) the cost is bounded at one epoch = 360 blocks ≈ 1 hour
(`consensus/constants.rs:188`) for producers that have already earned nothing for 37 epochs.

Skeptic C4 is **accepted in part**: `pending_updates` is genuinely outside the state root
(`set_persistence.rs:79-113`), so a queued rotation is invisible to root comparison for up to an
epoch. It is **refuted** as a fork carrier *for this design*: the new-`TxType` shape means a node
without the code cannot parse the block at all, and a constant gate read from
`NetworkParams::defaults()` makes "same binary, different gate value" impossible. The residual
detection gap is closed by REQ-ROT-012 (observability of the pending rotation).

Skeptic C3 is **accepted**: deferral opens a replay window in which the current on-chain key is
still the old one, so binding to "the current on-chain BLS key" (brief item 3) is **insufficient**.
Replaced by REQ-ROT-SEC-003 (expiry height) + REQ-ROT-SEC-005 (one pending rotation).

### (c) `allows_empty_io` — **new arm, value `true`**

Forced: the match is exhaustive with no `_` arm (`types.rs:123-152`), so the build fails until the
variant is classified. `true` is a **formal assertion that the apply handler authenticates the
actor**, discharged by REQ-ROT-SEC-001. Consequence: the transaction is 0-in/0-out and therefore
fee- and balance-exempt via `is_zero_flow` (`validation/utxo.rs:222-227`) — i.e. **free** — so spam
is bounded only by REQ-ROT-SEC-005 (one pending rotation per producer ⇒ at most one per epoch).
Copying `Exit` (brief item 1) is disqualified by name at `types.rs:130-131`.

---

## Requirements

| ID | Requirement | Priority | Acceptance Criteria |
|----|-------------|----------|---------------------|
| REQ-ROT-001 | `TxType::RotateBlsKey = 32` declared **last**, after `ZKSettle = 31` → wire ordinal 24 / discriminant 32; `from_u32(32)` arm added; discriminant 23 stays a permanent tombstone (`from_u32(23)` stays `None`) | Must | ordinal-stability golden vector; `from_u32` round-trip |
| REQ-ROT-002 | `RotateBlsData` payload, fixed 240 bytes, in `extra_data` | Must | exact-length encode/decode round-trip |
| REQ-ROT-003 | `allows_empty_io => true`; transaction is strictly 0-in/0-out | Must | zero-flow accepted; any input or output rejected |
| REQ-ROT-004 | `bls_key_rotation_activation_height` in `NetworkParams`, `u64::MAX` on mainnet, testnet and devnet; constant gate; no `HardForkSchedule` entry | Must | below-AH rejection on all three networks |
| REQ-ROT-005 | Epoch-deferred via `PendingProducerUpdate::RotateBlsKey`, **appended** after `RequestWithdrawal` | Must | key unchanged h..boundary-1, changes exactly at the boundary; variant golden vector |
| REQ-ROT-006 | `block_mutates_producer_set` includes `RotateBlsKey` (feature-evaluation C5) | Must | non-empty undo snapshot; reorg restores the old key |
| REQ-ROT-007 | `rebuild_producer_set_from_blocks` gains a bit-identical arm (skeptic hidden req 4) | Must | apply vs rebuild `serialize_canonical()` byte-identical |
| REQ-ROT-008 | Deterministic epoch-boundary ordering against a competing `Exit` / `Register` / `Slash` for the same producer (feature-evaluation C6) | Must | both orders defined, no panic, identical on every node |
| REQ-ROT-009 | Mempool admission, block builder and block validation share one predicate (INV-VALIDATION-001) | Must | all three paths, same tx, same verdict, 8 rejection reasons |
| REQ-ROT-010 | Reproduction test: a producer whose node key mismatches the on-chain key fails attestation and qualification | Must | RED before any fix code |
| REQ-ROT-011 | End-to-end recovery from the next epoch boundary | Must | verifying half, bit set, `[ATTEST_EGRESS]` stops, qualifies |
| REQ-ROT-012 | Observability without a canonical-encoding change (skeptic hidden req 7) | Must | log token, metric, `getProducer.pendingBlsRotation`; `ProducerInfo` bincode byte-identical |
| REQ-ROT-013 | CLI `doli producer rotate-bls` with disclosure and consent | Must | old+new key printed, consent required, wallet unmodified |
| REQ-ROT-014 | CLI `doli wallet import-bls <hex>` for operators who still hold the old key | Should | imports, re-runs the egress self-check, no consensus change |
| REQ-ROT-015 | Docs/specs sync in the same change | Should | `specs/protocol.md` tx table, `docs/{cli,rpc_reference,becoming_a_producer,rewards}.md` |
| REQ-ROT-016 | Correct `docs/bugfixes/inc-i-162-wallet-bls-derivation-analysis.md` §6 drift | Should | §6 no longer claims the BLS path is retired |
| REQ-ROT-017 | Pin the activation height on any network | Won't | N/A — separate decision session |
| REQ-ROT-018 | Ed25519 producer-key rotation | Won't | N/A — deferred; the payload shape does not foreclose it |
| REQ-ROT-019 | Time-based rotation cooldown stored on `ProducerInfo` | Won't | N/A — needs a new canonical field (skeptic C10); one-per-epoch from SEC-005 is the substitute |
| REQ-ROT-020 | Old-BLS countersignature | Won't | N/A — self-defeating for the lost-key population (security doc T6) |
| REQ-ROT-021 | `getProducer.lastRotationHeight` derived from the block index | Could | derived off canonical state only |

Security requirements **REQ-ROT-SEC-001 … REQ-ROT-SEC-010** (all Must) live in
[bls-key-rotation-requirements-security.md](./bls-key-rotation-requirements-security.md).

---

## Acceptance Criteria (detailed)

### REQ-ROT-001: Wire numbering
- [ ] Given the pre-change binary's `bincode::serialize` output for every existing `TxType`, when the new variant is added last, then every byte sequence is unchanged (golden-vector test — this **proves** the ordinal claim rather than asserting it; skeptic "What I don't understand" #4).
- [ ] Given a golden block serialized before the change, when deserialized after, then it decodes identically.
- [ ] `from_u32(23) == None`; `from_u32(24..=30) == None`; `from_u32(31) == Some(ZKSettle)`; `from_u32(32) == Some(RotateBlsKey)`; `bincode::serialize(&TxType::RotateBlsKey)` encodes ordinal 24.
  > Superseded (architecture D1): the discriminant is **32**, not 23. 23 is ZKSettle's ordinal and stays a permanent tombstone. Declaring the variant last keeps ordinal 24 and renumbers nothing.

### REQ-ROT-002: Payload
- [ ] Layout: `producer(32) || new_bls_pubkey(48) || bls_pop(96) || signature(64)` = **240 bytes**.
- [ ] `encode` → `decode` round-trips to an equal struct for 100 random payloads.
- [ ] `decode` returns `None` for lengths 0, 1, 239, 241 and 1024 — length-exact, so a trailing byte is a rejection. The `producer` field carries the sender identity the payload is verified against (cf. `DelegateBondData.delegator`, `data.rs:249`).
  > Superseded (architecture D2): **240 bytes**, and there is **no `expiry_height` field** — the replay bind is the spending outpoint, which D2 deleted the expiry window in favour of.

### REQ-ROT-003: Fee-paying classification
- [ ] `TxType::RotateBlsKey.allows_empty_io() == false`; the empty-io exempt set stays at five types.
- [ ] Given a fee-paying 1-in/1-out rotation, when validated with UTXOs, then the ordinary fee and balance rules apply unchanged.
  > Superseded (architecture D2): rotation is **not** zero-flow. `allows_empty_io => false` and the transaction spends one input and creates one output; consuming that outpoint is what makes the authorisation single-use.
- [ ] The below-`inc_i_173` legacy expression in `validation/utxo.rs` is **character-identical** to today (INV-COMPAT-001).

### REQ-ROT-004: Activation gate
- [ ] `NetworkParams::defaults()` returns `u64::MAX` for mainnet, testnet **and** devnet.
- [ ] At `height = AH - 1` the transaction is rejected `[ERRTX-ROT002] bls key rotation not active`, at mempool, builder and block validation.
- [ ] `git diff crates/updater/src/hardfork.rs` is empty; `CURRENT_PROTOCOL_VERSION` and `EPOCH_STATE_FORMAT_VERSION` are unchanged (INV-4).
- [ ] The three-question consensus-shape checklist is answered in the commit message: (1) YES — user-submittable tx; (2) YES; (3) NO ⇒ activation height REQUIRED. Both deploy questions answered: consensus RULES change = YES; block CONTENT change = YES ⇒ synchronized deploy at pin time.

### REQ-ROT-005: Epoch deferral
- [ ] Given a rotation mined at height h inside epoch E, then for every height in `h ..= (boundary-1)` `getProducer(P).blsPubkey` is byte-identical to the pre-rotation key.
- [ ] At the boundary block, `getProducer(P).blsPubkey` equals the new key, on every node.
- [ ] Over the same range, the attestation universe length, the bitfield width and `block.header.presence_root` are computed from the same inputs as without the rotation (regression assertion, since the field is not a universe input).
- [ ] `PendingProducerUpdate` golden vector: `bincode::serialize` of each of the 7 existing variants is byte-identical before and after the enum change.
- [ ] A `ProducerSet` undo snapshot written by the pre-change binary still deserializes.

### REQ-ROT-006: Undo-snapshot predicate (the compile-clean fork carrier)
- [ ] Given a block whose only producer-affecting transaction is a rotation, `block_mutates_producer_set(&block) == true`.
- [ ] The undo entry for that height has a **non-empty** `producer_snapshot` (i.e. `rollback.rs:178` does not take the sentinel branch).
- [ ] Given a reorg across the rotation block, after `rollback_one_block` the producer's `bls_pubkey` is the OLD key and `compute_state_root` equals the root at `h-1`.
- [ ] A negative control: the same test with `RotateBlsKey` removed from the predicate FAILS (the test is proven to be load-bearing).

### REQ-ROT-007: Second apply implementation
- [ ] Given blocks 1..=h containing a rotation, `rebuild_producer_set_from_blocks` produces a `ProducerSet` whose `serialize_canonical()` is byte-identical to the one built by `apply_block`.
- [ ] The rebuild path queues the rotation and flushes it at the same boundary height as the apply path.
- [ ] All four call sites are exercised: `rollback.rs:144`, `rollback.rs:267`, `block_handling.rs:739`, `block_handling.rs:910`.
- [ ] A negative control with the arm removed FAILS.

### REQ-ROT-008: Boundary ordering
- [ ] Rotation then `Exit` for the same producer: both apply in arrival order at the boundary, the producer ends exited, no panic, identical verdict on two independently-built nodes.
- [ ] `Exit` then rotation: the rotation STILL installs the key — the flush arm is UNCONDITIONAL on status (architecture D4 deletes the earlier "no-op with a `warn!`" clause; a status branch would make apply and M8's rebuild disagree). `Register` then rotation: the rotation applies after the register. `Slash` then rotation: defined, no panic. Both arrival orders of each pair are pinned byte-for-byte in `crates/storage/src/producer/tests_rotation.rs`.

### REQ-ROT-009: Three-path parity
- [ ] For each of the 8 rejection reasons (below AH, bad length, bad PoP, bad signature, non-producer, duplicate key, expired, second pending), the same transaction produces the same verdict at `Mempool::add_transaction`, at the block builder (`production/assembly.rs`) and at `apply_block`.
- [ ] The validation context builder used by the mempool populates every field the rotation predicate reads (the builder is otherwise strictly weaker — INV-VALIDATION-001).
- [ ] The byte-identical log line `[BLOCK_POISON] apply_block failed on self-produced block` never appears in the parity test run.

### REQ-ROT-010: Reproduction (milestone M1, RED first)
- [ ] Given a producer registered with BLS key A whose node holds key B, at a height above `inc_i_178_attestation_bls_activation_height`, when it signs its own attestation, then `bls_verdict` returns `BlsAttestVerdict::Invalid` and `[ATTEST_EGRESS] own BLS half does not verify against the on-chain key` is emitted.
- [ ] Its bit is never set in any block's attestation bitfield.
- [ ] Its attested-minute count over an epoch is 0 and it fails the 54/60 qualification (`rewards.rs:41`).
- [ ] The test FAILS (red) before any production code for this feature exists, and the failure output is captured as `docs/.workflow/inc-i-217-M1-test-red-evidence.txt`.

### REQ-ROT-011: End-to-end recovery
- [ ] Given the REQ-ROT-010 producer, when it submits a valid rotation to key B and the next boundary passes, then `ingress.rs` returns `BlsAttestVerdict::Valid`, its bit is set in the following block, `[ATTEST_EGRESS]` stops, and its attested minutes increase.
- [ ] State roots converge: apply → restart → rebuild-from-disk produce an identical `ProducerSet` canonical serialization.

### REQ-ROT-012: Observability
- [ ] `[BLS_ROTATE] queued producer=<8hex> new=<8hex> h=<height>` at queue time, `[BLS_ROTATE] applied producer=<8hex> old=<8hex> new=<8hex> h=<height>` at boundary apply, and `doli_producer_bls_rotation_total` increments on apply.
- [ ] `getProducer` gains `pendingBlsRotation: {newBlsPubkey, queuedAtHeight} | null`, derived from `ProducerSet::pending_updates_for(pubkey)` — **not** from a new `ProducerInfo` field.
- [ ] Golden vector: `ProducerSet::serialize_canonical()` for a set with no rotation is byte-identical before and after this change.

### REQ-ROT-013 / 014: CLI
- [ ] `doli producer rotate-bls` prints the on-chain key, the wallet key, the consequence ("attestations verify against the new key from the next epoch boundary; this cannot be undone except by another rotation") and requires an explicit confirmation, honouring `--yes`.
- [ ] It refuses when the wallet BLS key already equals the on-chain key, and when the wallet has no BLS key. After the command `wallet.json` is byte-identical (the wallet is never modified).
- [ ] `doli wallet import-bls <hex>` imports a 48-byte-public / 32-byte-secret BLS key into the primary address, requires `--force` to overwrite an existing key, and re-runs the on-chain comparison. (Note: `Wallet::add_bls_key()` today only **generates** a random key, `crates/wallet/src/wallet.rs:401-414` — it cannot import.)

---

## Impact Analysis

### Existing Code Affected

| File / module | How | Risk |
|---|---|---|
| `crates/core/src/transaction/types.rs` | new variant last + `from_u32` arm + `allows_empty_io` arm | **high** — wire numbering; mitigated by REQ-ROT-001 golden vectors |
| `crates/storage/src/producer/types.rs` | `PendingProducerUpdate` variant appended | **high** — bincode inside the undo snapshot; mitigated by REQ-ROT-005 |
| `bins/node/src/node/apply_block/helpers.rs:15-28` | `matches!` with implicit `false` fallthrough | **high** — a compile-clean omission forks the chain across a reorg (REQ-ROT-006) |
| `bins/node/src/node/rewards.rs:1117+` | second apply implementation, 4 call sites | **high** — a rotation missing here is silently dropped by every reorg (REQ-ROT-007) |
| `crates/core/src/validation/utxo.rs:222-227` | one new `true` in the gated set | medium — below-gate expression must stay character-identical |
| `crates/mempool/src/` | new `pending_rotations.rs` + wiring | medium — parity surface that already produced two incident regression suites |
| `crates/core/src/network_params/` | 95th activation height field | low — additive; permanent maintenance tax |
| `crates/rpc/` | one derived field | low |
| `bins/cli/src/cmd_producer/` | new subcommand + dispatch | low — **collides with `feat/bond-lock-disclosure-cli`** (condition C1) |
| `crates/crypto/src/bls.rs` | one DST + two functions | low — additive; existing DSTs untouched |

### What Breaks If This Changes

- **Reorg across a rotation block** → wrong key survives the rollback → `producer_set` hash diverges → chain fork. *Mitigation*: REQ-ROT-006 + REQ-ROT-007, each with a negative control.
- **A variant inserted mid-enum** (`TxType` or `PendingProducerUpdate`) → every stored undo snapshot and snap-sync payload misdecodes. *Mitigation*: REQ-ROT-001, REQ-ROT-005 golden vectors.
- **A new `ProducerInfo` field** → state-root fork at deploy, before any activation height. *Mitigation*: forbidden by REQ-ROT-012 and REQ-ROT-SEC-003; the replay bind lives in the payload, not in state.
- **Un-upgraded node after pinning** → cannot decode any block carrying a rotation → stalls/partitions. *Mitigation*: REQ-ROT-SEC-010 fleet census as a pinning precondition. `getProducer` consumers (explorer, GUI `bins/gui/src/commands/producer.rs:192`) take a nullable additive field only.

### Regression Risk Areas

- Attestation verification for **non-rotating** producers during the epoch containing a rotation.
- Snap-sync payload equality, and the `rollback.rs` sentinel branch — the one compile-clean omission.
- Epoch-boundary flush ordering when several mutation kinds for one producer land in one epoch.
- The `inc_i_173` below-gate legacy expression (frozen consensus history, ~30 external mixed-version producers).

---

## Reconciliation with `docs/.workflow/skeptic-analysis.md`

Every skeptic point C1-C10 and every one of the 9 hidden requirements is reconciled
(ACCEPTED / REFUTED with `file:line` / re-scoped) in the
[security companion](./bls-key-rotation-requirements-security.md#reconciliation-with-the-skeptic-analysis).
Headline: C1, C2, C3, C5, C8, C9, C10 and all 9 hidden requirements **ACCEPTED**; C4 accepted in
part (see decision (b)); C6's reasoning accepted but its conclusion **REFUTED** (see decision (a));
C7's code claim **REFUTED** — `Wallet::add_bls_key()` generates, it cannot import.

---

## Milestones

| ID | Name | Scope (Modules) | Scope (Requirements) | Dependencies |
|----|------|-----------------|----------------------|--------------|
| M1 | Reproduction test (RED) | `bins/node/tests/bls_rotation_repro.rs` (new, auto-discovered target) | REQ-ROT-010 | — |
| M2 | Wire numbering + payload | `crates/core/src/transaction/types.rs`, `crates/core/src/transaction/data.rs`, `crates/core/tests/` | REQ-ROT-001, REQ-ROT-002, REQ-ROT-003 | M1 |
| M3 | Crypto domains + signing messages | `crates/crypto/src/bls.rs`, `crates/crypto/src/lib.rs`, `crates/core/src/transaction/data.rs` | REQ-ROT-SEC-001, REQ-ROT-SEC-002, REQ-ROT-SEC-009 | M2 |
| M4 | Activation height + validation | `crates/core/src/network_params/defaults.rs`, `crates/core/src/network_params/mod.rs`, `crates/core/src/validation/tx_types.rs`, `crates/core/src/validation/utxo.rs`, `crates/core/src/validation/types.rs` | REQ-ROT-004, REQ-ROT-SEC-003, REQ-ROT-SEC-004, REQ-ROT-SEC-006, REQ-ROT-SEC-007, REQ-ROT-SEC-008 | M3 |
| M5 | Mempool + builder parity | `crates/mempool/src/pending_rotations.rs` (new), `crates/mempool/src/lib.rs`, `crates/mempool/src/pool.rs`, `bins/node/src/node/production/assembly.rs`, `crates/mempool/tests/` | REQ-ROT-009, REQ-ROT-SEC-005 | M4 |
| M6 | Apply path #1 + undo predicate | `crates/storage/src/producer/types.rs`, `crates/storage/src/producer/set_core.rs`, `bins/node/src/node/apply_block/tx_processing.rs`, `bins/node/src/node/apply_block/helpers.rs` | REQ-ROT-005, REQ-ROT-006, REQ-ROT-008 | M5 |
| M7 | Apply path #2 + reorg convergence | `bins/node/src/node/rewards.rs`, `bins/node/src/node/rollback.rs`, `bins/node/tests/` | REQ-ROT-007, REQ-ROT-011 | M6 |
| M8 | Observability | `crates/rpc/src/types/producer.rs`, `crates/rpc/src/methods/producer.rs`, `crates/storage/src/producer/set_core.rs`, `bins/node/src/node/apply_block/tx_processing.rs`, metrics module | REQ-ROT-012 | M7 |
| M9 | CLI | `bins/cli/src/cmd_producer/rotate.rs` (new), `bins/cli/src/cmd_producer/mod.rs`, `bins/cli/src/cmd_producer/dispatch.rs`, `bins/cli/src/cmd_wallet*`, `crates/wallet/src/wallet.rs` | REQ-ROT-013, REQ-ROT-014 | M8 |
| M10 | Docs + specs sync | `specs/protocol.md`, `specs/attestation-bls-architecture.md`, `docs/cli.md`, `docs/rpc_reference.md`, `docs/becoming_a_producer.md`, `docs/rewards.md`, `docs/bugfixes/inc-i-162-wallet-bls-derivation-analysis.md` | REQ-ROT-015, REQ-ROT-016, REQ-ROT-SEC-010 | M9 |

---

## Traceability Matrix

| Requirement ID | Priority | Test IDs | Architecture Section | Implementation Module |
|---|---|---|---|---|
| REQ-ROT-001 | Must | `crates/core/src/transaction/wire_golden_tests.rs` — `bare_txtype_bincode_encoding_is_a_u32_little_endian_variant_index`, `txtype_golden_table_pins_both_numbering_surfaces`, `txtype_ordinals_are_the_contiguous_declaration_positions_zero_to_24`, `transaction_wire_layout_places_tx_type_at_byte_offset_four`, `every_txtype_round_trips_through_real_transaction_bytes`, `tombstoned_discriminants_stay_undecodable_and_32_is_rotate_bls_key`, `wire_ordinal_24_decodes_as_rotate_bls_key_and_25_does_not_decode`, `exactly_25_discriminants_are_live`; `crates/wallet/tests/serialization_compat.rs::test_core_txtype_variant_count` | D1 | M4 — `crates/core/src/transaction/types.rs` (`TxType::RotateBlsKey = 32`, `from_u32`) |
| REQ-ROT-002 | Must | `crates/core/tests/it/inc_i_217_m4_rotate_payload.rs` — `req_rot_002_encode_produces_exactly_240_bytes`, `req_rot_002_every_field_lands_at_its_documented_offset`, `req_rot_002_non_uniform_fields_survive_a_round_trip_in_order`, `req_rot_002_decode_round_trips_every_field`, `req_rot_002_decode_rejects_every_length_but_240` | D2 | M4 — `crates/core/src/transaction/rotate_bls.rs` (`RotateBlsData`, `ROTATE_BLS_DATA_LEN`) |
| REQ-ROT-003 | Must | `crates/core/tests/it/inc_i_217_m4_rotate_payload.rs::req_rot_003_rotate_bls_key_is_not_empty_io_exempt` | D2 | M4 — `crates/core/src/transaction/types.rs` (`allows_empty_io`); `validation/utxo.rs` unchanged |
| REQ-ROT-004 | Must | `crates/core/tests/it/inc_i_217_m5_activation_height.rs` (6 tests); `crates/core/tests/it/inc_i_217_m5_block_gate.rs` — `req_rot_004_a_rotation_below_the_shipped_gate_invalidates_the_block_on_every_network`, `req_rot_004_every_mode_reports_the_gate_by_its_own_error_code`, `req_rot_004_a_block_without_a_rotation_is_unaffected_by_the_gate`, `req_rot_004_above_the_gate_a_well_formed_rotation_is_not_blocked_by_d6`, `req_rot_004_the_gate_opens_at_the_activation_height_itself` | D6 | M5 — `crates/core/src/network_params/{mod,defaults,env_loader}.rs` (`bls_key_rotation_activation_height`, `u64::MAX` on all three networks); `crates/core/src/validation/types.rs` (`ValidationContext::with_bls_key_rotation_activation_height`); `crates/core/src/validation/block.rs` (D6 gate ahead of the `ValidationMode` match); plumbed at `crates/mempool/src/pool.rs`, `bins/node/src/node/validation_checks/mod.rs`, `bins/node/src/node/production/assembly.rs`, `bins/node/src/node/apply_block/tx_processing.rs` |
| REQ-ROT-005 | Must | `crates/storage/src/producer/wire_golden_tests.rs` (6 tests: `pending_producer_update_json_golden_vectors`, `exactly_eight_pending_producer_update_variants`, `pre_change_queue_json_still_deserializes`, `serialize_canonical_golden_hash`, `serialize_canonical_golden_hash_is_not_vacuous`, `serialize_canonical_is_insertion_order_independent`); `crates/storage/src/producer/tests_rotation.rs` — `queued_rotation_changes_the_key_at_the_boundary`, `key_and_state_root_are_frozen_until_the_boundary`, `flush_is_unconditional_on_status`, `the_queue_is_fully_drained_after_the_flush`, `queued_rotation_survives_the_save_load_round_trip`, `both_pending_accessors_see_the_rotation`; `bins/node/tests/bls_rotation_repro.rs` — `req_rot_010_b2_pending_producer_update_carries_exactly_8_variants`, `req_rot_010_b3_registration_is_the_only_writer_of_bls_pubkey` | D4 | M7 — `crates/storage/src/producer/{types,set_core,rotation}.rs`; apply #1 arm `bins/node/src/node/apply_block/tx_processing.rs` (height-gated, fail-closed); fourth compile-forced match site `crates/rpc/src/methods/producer.rs::pending_update_to_info` (`"rotate_bls_key"`, minimal — REQ-ROT-012 surfacing is M9) |
| REQ-ROT-006 | Must | `bins/node/src/node/apply_block/helpers.rs::rotation_tests` — `rotation_block_mutates_the_producer_set`, `predicate_agrees_with_the_exhaustive_intent_for_every_tx_type`, `rotation_is_seen_behind_a_coinbase_and_a_transfer`, `transfer_only_block_does_not_mutate_the_producer_set`; `bins/node/tests/it/inc_i_217_m7_rotation_apply.rs` (5 tests — gate + no-residue + sentinel control + rollback byte identity). **The end-to-end non-empty `producer_snapshot` criterion is UNREACHABLE at `bls_key_rotation_activation_height = u64::MAX` and moves to M8/post-pin** | D4 | M7 — `bins/node/src/node/apply_block/helpers.rs` |
| REQ-ROT-007 | Must | (test-writer) | (architect) | `bins/node/src/node/rewards.rs` |
| REQ-ROT-008 | Must | `crates/storage/src/producer/tests_rotation.rs` — order pins (`order_pin_not_producer_wins_over_every_other_reason`, `order_pin_same_key_wins_over_already_pending_and_key_in_use`, `order_pin_already_pending_wins_over_key_in_use`, `rotate_verdict_is_pure`), queueing (`accepted_rotation_queues_exactly_one_update`, `every_skip_reason_leaves_the_set_byte_identical`), boundary ordering (`exit_and_rotation_in_one_epoch_commute`, `slash_and_rotation_in_one_epoch_commute`, `register_then_rotation_lands_registered_with_the_new_key`, `no_queue_ordering_panics_at_the_boundary`, `flush_for_an_absent_producer_is_a_silent_drain`) | D4 | M7 — `crates/storage/src/producer/{set_core,rotation}.rs`; apply #1 arm `bins/node/src/node/apply_block/tx_processing.rs` |
| REQ-ROT-009 | Must | (test-writer) | (architect) | `crates/mempool/src/pending_rotations.rs` |
| REQ-ROT-010 | Must | `bins/node/tests/bls_rotation_repro.rs` — A: `..._a1_a2_the_mismatched_half_is_invalid_at_both_the_egress_and_the_ingress`, `..._a3_the_mismatched_producers_bit_is_never_set_in_the_bitfield`, `..._a4_the_mismatched_producer_never_qualifies_for_an_epoch_reward`, `..._a4_the_mainnet_qualification_shape_is_54_of_60`; B (flipped in M4, INC-I-217): `..._b1_ordinal_32_decodes_as_rotate_bls_key`, `..._b1_the_wire_carries_exactly_25_decodable_tx_types`, `..._b2_pending_producer_update_carries_exactly_7_variants`, `..._b3_no_pending_update_variant_rewrites_a_registered_producers_bls_key`, `..._b3_registration_is_the_only_writer_of_bls_pubkey` | (architect) | `bins/node/tests/bls_rotation_repro.rs` (M1: test only) |
| REQ-ROT-011 | Must | (test-writer) | (architect) | `bins/node/tests/` |
| REQ-ROT-012 | Must | (test-writer) | (architect) | `crates/rpc/src/methods/producer.rs` |
| REQ-ROT-013 | Must | (test-writer) | (architect) | `bins/cli/src/cmd_producer/rotate.rs` |
| REQ-ROT-014 | Should | `bins/cli/tests/it/inc_i_217_import_bls_golden.rs` — `..._import_bls_force_installs_the_key_and_never_prints_the_secret`, `..._import_bls_refuses_without_force_and_leaves_the_key_intact`, `..._import_bls_into_a_wallet_without_a_bls_key_needs_no_force`, `..._import_bls_rejects_malformed_secrets_and_leaves_the_wallet_byte_identical`, `..._info_must_not_claim_the_phrase_backs_up_an_imported_bls_key`, `..._import_bls_surface_exposes_force_rpc_and_address`; `bins/cli/src/cmd_wallet_bls_tests.rs` (inert until the developer wires it) — `..._wallet_version_seed_derived_bls_stays_three`, `..._import_bls_key_forced_installs_secret_and_derives_public`, `..._import_bls_key_clears_the_seed_derived_marker`, `..._import_bls_key_survives_save_and_load`, `..._import_bls_key_refuses_without_force_and_leaves_the_receiver_untouched`, `..._import_bls_key_into_a_wallet_without_a_key_needs_no_force`, `..._import_bls_key_rejects_malformed_secrets_before_mutating` | (architect) | `bins/cli/src/cmd_wallet_bls.rs` (new), `bins/cli/src/wallet.rs`, `bins/cli/src/commands.rs`, `bins/cli/src/main.rs` — **NOT** `crates/wallet/`: `bins/cli` does not depend on that crate, and the command is flat `doli import-bls`, not `doli wallet import-bls` |
| REQ-ROT-015 / 016 | Should | N/A | (architect) | `specs/`, `docs/`, `docs/bugfixes/inc-i-162-wallet-bls-derivation-analysis.md` |
| REQ-ROT-021 | Could | (test-writer) | (architect) | `crates/rpc/src/methods/producer.rs` |
| REQ-ROT-SEC-001 | Must | `crates/core/tests/it/inc_i_217_m4_rotate_payload.rs` — `req_rot_sec_001_preimage_matches_the_pinned_spec_bytes`, `req_rot_sec_001_each_preimage_field_sits_at_its_spec_offset`, `req_rot_sec_001_preimage_carries_no_length_prefix_and_no_key_kind_byte`, `req_rot_sec_001_digest_is_the_pinned_blake3_of_the_pinned_preimage`, `req_rot_sec_001_digest_commits_to_the_spending_outpoint`, `req_rot_sec_001_digest_commits_to_the_new_bls_pubkey` | D7 | M4 (encoder only) — `crates/core/src/transaction/rotate_bls.rs` (`rotation_auth_preimage`, `rotation_auth_digest`); verification is M5 |
| REQ-ROT-SEC-002 | Must | `crates/crypto/tests/it/inc_i_217_m4_rotation_pop.rs` (8 tests, `req_rot_sec_002_*`); `crates/crypto/src/bls_rotation_tests.rs` — `req_rot_sec_002_the_three_bls_domains_are_pairwise_distinct`, `req_rot_sec_002_no_bls_domain_is_a_prefix_of_another`, `req_rot_sec_002_the_rotation_dst_is_the_pinned_byte_string` | D7 | M4 — `crates/crypto/src/bls_rotation.rs` (`ROTATE_POP_DST`, `sign_rotation_pop`, `verify_rotation_pop`) |
| REQ-ROT-SEC-009 | Must | `crates/core/tests/it/inc_i_217_m4_rotate_payload.rs::req_rot_sec_009_digest_is_bound_to_the_genesis_hash`; `crates/crypto/tests/it/inc_i_217_m4_rotation_pop.rs::req_rot_sec_009_the_pop_is_bound_to_the_genesis_hash` | D7 | M4 (both preimages carry the genesis hash) |
| REQ-ROT-SEC-003 | Must | `crates/core/tests/it/inc_i_217_m5_rotate_stateless.rs` — `req_rot_sec_003_an_authorisation_signed_over_another_outpoint_is_rejected`, `req_rot_sec_003_a_valid_tuple_lifted_onto_another_outpoint_is_rejected` | D2, D6 | M5 — `crates/core/src/validation/rotate_bls.rs` (outpoint inside `rotation_auth_digest`) |
| REQ-ROT-SEC-007 | Must | `crates/core/tests/it/inc_i_217_m5_rotate_stateless.rs` (26 tests) | D6 | M5 — `crates/core/src/validation/rotate_bls.rs::rotate_stateless`; `crates/core/src/validation/transaction.rs` (`TxType::RotateBlsKey` arm); `crates/core/src/validation/error.rs` (8 `[ERRTX-ROTnnn]` variants) |
| REQ-ROT-SEC-008 | Must | `crates/core/tests/it/inc_i_217_m5_block_gate.rs` — `req_rot_sec_008_a_malformed_rotation_above_the_gate_invalidates_the_block`, `req_rot_sec_008_the_same_block_without_the_rotation_is_accepted` | D6 | M5 — `crates/core/src/validation/block.rs` (no skip arm at block level) |
| REQ-ROT-SEC-004..006 | Must | `crates/storage/src/producer/tests_rotation.rs` — **SEC-004**: `rotating_to_own_current_key_is_same_key_not_key_in_use`, `target_equal_to_another_producers_current_key_is_key_in_use`, `target_equal_to_another_queued_rotation_target_is_key_in_use`, `target_equal_to_a_queued_registers_key_is_key_in_use`, `key_in_use_scan_is_order_independent`, `two_producers_racing_to_one_key_in_one_block_only_the_first_queues`. **SEC-005**: `second_rotation_inside_the_deferral_window_is_already_pending`, `two_rotations_for_one_producer_in_one_block_queue_only_the_first`. **SEC-006**: `absent_producer_is_not_a_producer`, `active_producer_rotating_to_a_fresh_key_is_accepted`, `unbonding_producer_is_rejected`, `exited_producer_is_rejected`, `slashed_producer_is_rejected`, `producer_with_only_a_queued_register_is_accepted`, `queued_register_bls_key_is_what_same_key_compares_against`. Mempool/builder parity (SEC-005 three-path clause) and SEC-010 remain M9+ | D8 | M7 — `crates/storage/src/producer/rotation.rs` |
| REQ-ROT-SEC-010 | Must | N/A — a precondition on PINNING, not on merging | D6 | post-pin session |

---

## Specs Drift Detected

1. **`docs/bugfixes/inc-i-162-wallet-bls-derivation-analysis.md` §6** — concludes the BLS attestation
   path is "RETIRED (no consumer remains)" and a key mismatch is "100 % silent and currently
   harmless". **False since mainnet h=409,000**: `attestation/{ingress,keys,verify,commit}.rs` are
   all live consumers and two producers earn zero because of it. → REQ-ROT-016. Register a MEMORY.md
   hotfix.
2. **`specs/attestation-bls-architecture.md:416` (O5) / REQ-BLS-021 "Won't"** — superseded by this
   document; a supersede note has been added in place (condition C4). Precondition **P4** (`:562`)
   closes by the "rotation TxType shipped" branch **when this feature merges**, not when it is
   pinned.
3. **`specs/attestation-bls-architecture.md:562` P8** ("every `bls_pubkey` write path is PoP-covered",
   *unverified*) — this feature adds a **fourth** write path. REQ-ROT-SEC-002 makes the new path
   PoP-covered by construction; P8 remains unverified for the three existing paths.
4. **The brief's own acceptance criterion** "mid-epoch rotation does not change `active_producers` /
   bitfield width / presence commitment" describes a property of the field, not of the deferral —
   corrected in decision (b) and REQ-ROT-005.

---

## What I Don't Understand (mandatory disclosure)

1. **Snap-sync interaction.** I did not read `crates/network/src/sync/`. A node that snap-syncs
   across an epoch boundary containing a rotation receives a `ProducerSet` snapshot, not the blocks.
   I believe this is safe because the snapshot carries `producers` (post-flush) and `pending_updates`
   through the same serde path, but I did **not** verify it. The architect must.
2. **`active_producers_at_height` and the tier filter.** I confirmed `bls_pubkey` is not a scheduler
   input, but I did not read the tier/`registered_at` promotion filter (INC-I-193, open CRITICAL).
   Condition C7 requires an explicit statement that rotation leaves `registered_at`, bonds and the
   Ed25519 anchor untouched — the design does, but I did not read the tier code to prove no other
   coupling exists.
3. **Metric registry shape.** I did not read the metrics module, so `doli_producer_bls_rotation_total`
   is a name, not a verified registration pattern (and INC-I-187 records 28/57 `doli_*` metrics never
   written).
4. **`bins/gui/src/commands/producer.rs`** builds a `RegistrationData` with a BLS key. I did not
   check whether the GUI needs a rotation path; it is currently out of scope by omission, not by
   decision.
5. **Whether the two affected operators still hold anything** that would make REQ-ROT-014 sufficient
   for them. The census says producer `2e43af5b…`'s wallet file is gone; nothing is recorded for
   `ec18e0cb…`.

### Contradiction found and resolved

⚠ **CONTRADICTION**: my dispatch brief states INC-I-169 is a "merged fix". The codebase disagrees:
`crates/core/src/transaction/core.rs:536` still carries `// extra_data (covenant witnesses)
intentionally excluded — SegWit-style`, `.omega/memory.db` has `INC-I-169.status = 'investigating'`,
and the diagnosis report's closing line reads "unpublished, unpatched". **Resolved in favour of the
code**: the outer transaction signature authenticates **no** `extra_data` field on this base. This
is load-bearing — it is why REQ-ROT-SEC-001 (an inner signature) is mandatory and why a
"fee-paying 1-in/1-out rotation authenticated by the input signature" is **not** an option.

---

## Assumptions

| # | Assumption (technical) | Explanation (plain language) | Confirmed |
|---|---|---|---|
| 1 | Declaring a variant last gives it the highest bincode ordinal and shifts none | Adding the new transaction at the end of the list does not renumber the existing ones | **No — must be proven by the REQ-ROT-001 golden vector before any other work** |
| 2 | `ROTATION_TX_MAX_LIFETIME = BLOCKS_PER_REWARD_EPOCH` (360) is a sensible replay window | A rotation request is only usable for about an hour | No — Q3 |
| 3 | One rotation per producer per epoch is sufficient anti-spam | A producer can change its key at most once an hour | No — Q4 |
| 4 | `pending_updates` survives snap-sync intact; rotation never touches `registered_at`, bonds, delegations or the Ed25519 key | A fast-syncing node still learns of a queued rotation; changing the attestation key does not affect seniority or stake | Partly — disclosure 1 open; C7 is a design constraint |

---

## Identified Risks

- **A compile-clean omission in `block_mutates_producer_set` or `rebuild_producer_set_from_blocks`
  forks the chain.** *Mitigation*: REQ-ROT-006/007 each carry a negative control that must FAIL when
  the arm is removed.
- **CLI landing-zone collision** with `feat/bond-lock-disclosure-cli` (`50e09185` edits
  `bins/cli/src/cmd_producer/`) — condition C1, land or abandon before M9. **Sequencing against the
  INC-I-171 activation at 418,000** — condition C8; the `u64::MAX` gate makes waiting free.
- **Permanent maintenance tax**: a 95th activation gate and a 25th live `TxType` that every
  non-exhaustive `matches!` must consider forever.
- **INC-I-193 (open, CRITICAL)**: "any re-key demotes a node" under the tier CAP=50 sorted by
  `registered_at`. This design does not touch `registered_at`; the architect must restate it (C7).

---

## Open Questions (with recommended answers, so the architect can proceed if the user is silent)

| # | Question | Recommended answer |
|---|---|---|
| Q1 | Trust model: treat every field of a received rotation as fully untrusted, including one built by our own CLI? | **Yes.** All six entry points in the security document are untrusted; nothing is trusted for having come from the CLI. |
| Q2 | New `TxType` (decision (a)) rather than an `extra_data` extension, accepting that un-upgraded nodes cannot parse a block carrying one after pinning? | **Yes** — the alternative makes an old node apply the wrong base action silently. Add a fleet census as a pinning precondition (SEC-010). |
| Q3 | Replay window length (`ROTATION_TX_MAX_LIFETIME`)? | **360 blocks (one epoch, ≈1 h).** Long enough to be mined, short enough that the pending-rotation rule covers the whole window. |
| Q4 | Anti-spam: is one rotation per producer per epoch enough, or do you want a longer cooldown? | **One per epoch**, from REQ-ROT-SEC-005. A longer cooldown needs a `ProducerInfo` field, which forks the state root at deploy (skeptic C10) — REQ-ROT-019 Won't. |
| Q5 | Old-BLS countersignature for extra safety against a compromised Ed25519 key? | **No.** Self-defeating: every motivating operator has lost the old BLS secret, and a lost-key exception reopens the hole. Rotation grants no authority the Ed25519 key does not already have. |
| Q6 | Invalid rotation: reject the whole block, or skip the transaction? | **Reject the block**, on the `Registration` precedent, with mempool/builder parity so an honest builder never includes one. |
| Q7 | Ship `doli wallet import-bls` (REQ-ROT-014) in this change, or separately? | **In this change, as a Should.** It has no consensus risk and it is the only remedy available before the activation height is pinned. |
| Q8 | Transaction name `RotateBlsKey`? | **Keep it.** It is the name already used in the brief, the evaluation and O5. |

---

## Out of Scope (Won't)

- **Pinning the activation height** on testnet or mainnet — a separate decision session (HC-6 / INC-I-075).
- **Fixing INC-I-169 F1** (unsigned `extra_data`) — worked around with an inner signature; the sweep
  across `Registration`/`Exit`/`ClaimReward`/`ClaimBond`/`SlashProducer` stays with INC-I-169/176.
- **Ed25519 producer-key rotation** — the payload shape does not foreclose it (add one key-kind byte).
- **A stored rotation cooldown or `lastRotationHeight` on `ProducerInfo`** — any new field forks the
  state root at deploy. **Old-BLS countersignature. GUI rotation support.**
- Deploying; the affected operators' delegation/withdrawal decisions; issue #200
  (`maintainer_state.bin`); the future-slot block flood from producer `a9166b08…`.
