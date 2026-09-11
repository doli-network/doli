<!--
OUTPUT CONTRACT: N/A — requirements document (not a test file)
INPUT PARTITIONS: N/A — requirements document (not a test file)
-->

# Security Requirements: Producer BLS Key Rotation

Companion to [bls-key-rotation-requirements.md](./bls-key-rotation-requirements.md) (run 552, 2026-09-11).
Split out per Global Rule 19 (module size). Global Rule 17 (security chain) applies: this transaction
arrives from the network and rebinds a consensus identity, so every requirement below is **Must**.

---

## Trust Boundaries

Every point where external, attacker-chosen bytes enter the system, and what consumes them.

| # | Entry point | Attacker-controlled bytes | Consumed by | Trust level |
|---|-------------|---------------------------|-------------|-------------|
| TB-1 | Gossiped `RotateBlsKey` tx → mempool (`crates/mempool/src/pool.rs`) | `extra_data` (248 B), `tx_type`, `version` | stateless validation, admission | **UNTRUSTED** |
| TB-2 | `RotateBlsKey` tx inside a received block → `apply_block` | same | contextual validation, `tx_processing.rs` | **UNTRUSTED** |
| TB-3 | Same tx replayed from the block store during `rebuild_producer_set_from_blocks` (`bins/node/src/node/rewards.rs:1117`) | same | reorg / rollback rebuild | **UNTRUSTED** (already-mined bytes are not already-validated on this path) |
| TB-4 | `new_bls_pubkey` written into `ProducerInfo.bls_pubkey` | 48 B | `attestation/keys.rs:13-28` → `verify.rs:106` → `bls_fast_aggregate_verify` (`crates/crypto/src/bls.rs:695`) | **UNTRUSTED until REQ-ROT-SEC-002 + SEC-007 pass** |
| TB-5 | RPC `submitTransaction` from a local/remote client | whole tx | same as TB-1 | **UNTRUSTED** |
| TB-6 | CLI-built payload (`bins/cli/src/cmd_producer/`) | operator-chosen key | signing, submission | semi-trusted; still validated by TB-1 |

**Trust model assumed** (question Q1 for the user): all six entry points are treated as fully
untrusted. No field is trusted because "the CLI built it". This mirrors the `Bond extra_data` rule
in CLAUDE.md ("Never trust raw tx `extra_data`").

---

## Threat Model

### T1 — Attestation identity theft by BLS key squatting (skeptic C1, STRONG)
The proof of possession signs the **BLS public key bytes and nothing else** under `POP_DST`
(`crates/crypto/src/bls.rs:616,636`; `validate_bls_pop` adds nothing, `validation/registration.rs:259-275`).
Every PoP ever used is public — it ships as `RegistrationData.bls_pop` in on-chain `extra_data`
(`crates/core/src/transaction/data.rs:45`). There is **no `bls_pubkey` uniqueness check anywhere**
(verified: the only writes are `apply_block/tx_processing.rs:245`, `rewards.rs:1289`,
`rewards.rs:1537`, `apply_block/genesis_completion.rs:134`; the duplicate guard at
`validation/registration.rs:166` tests the **Ed25519** key). `bls_fast_aggregate_verify`
(`bls.rs:695`) does not deduplicate keys. The attestation preimage is the block hash alone
(`crates/core/src/attestation/message.rs:14-16`), so it is not bound to the attester.

⇒ Attacker producer A rotates its on-chain `bls_pubkey` to victim V's key, replaying V's published
PoP, then copies V's gossiped BLS half and signs its own Ed25519 half. Both verify; A harvests
presence weight, which feeds weighted rewards and the 54/60 qualification (`bins/node/src/node/rewards.rs:41`).
Today this costs Exit + re-registration; rotation removes all friction.
**Closed by SEC-002 (bound PoP) and SEC-004 (uniqueness).**

### T2 — Unauthenticated rotation (skeptic C2, STRONG)
The brief proposes copying `Exit`. `TxType::allows_empty_io` disqualifies `Exit` **by name**:
*"C1/F3: ExitData has no signature; validate_exit_data does no crypto check"*
(`crates/core/src/transaction/types.rs:130-131`). Copying it would let anyone rotate anyone's key.
**Closed by SEC-001.**

### T3 — The outer transaction signature authenticates nothing (INC-I-169 F1, **still unfixed on this base**)
`Transaction::signing_message()` excludes `extra_data`
(`crates/core/src/transaction/core.rs:517-537`, comment at `:536`), and it is the only signing
input of the sole verification site (`crates/core/src/validation/utxo.rs:124`). A 1-in/1-out
"fee-paying" rotation would therefore still be payload-malleable: a scheduled producer can
substitute `extra_data` in an in-flight rotation and the input signature stays valid (INC-I-169
[E1][E3][E5]). **The rotation payload must carry its own inner signature regardless of tx shape.**
**Closed by SEC-001 + SEC-009.**

### T4 — Replay (skeptic C3, STRONG)
DOLI's only shipped producer-auth signature, `DelegateBondData::signing_message()`
(`transaction/data.rs:284-299`), commits to `DOMAIN || delegate || bond_count` — no nonce, no
height, no chain id — and is infinitely replayable. Under epoch deferral the on-chain key is still
the OLD one for the whole window, so "bind to the current on-chain BLS key" (brief item 3) does
**not** stop a re-submission inside the window. **Closed by SEC-003 + SEC-005 + SEC-009.**

### T5 — Free-transaction spam
`allows_empty_io => true` puts the tx through `is_zero_flow` (`validation/utxo.rs:222-227`), which
exempts it from fee **and** balance checks. A rotation therefore costs nothing.
**Bounded by SEC-005: at most one pending rotation per producer, flushed at the epoch boundary ⇒
at most one rotation per producer per epoch (360 blocks, `consensus/constants.rs:188`).**

### T6 — Stealth attestation takeover by a compromised Ed25519 key (skeptic C9)
**Assessed as NOT a privilege escalation.** The Ed25519 producer key already controls `Exit`,
`RequestWithdrawal` and the reward destination (`bins/node/src/node/rewards.rs:359` pays to
`hash_with_domain(ADDRESS_DOMAIN, producer_info.public_key)`). An attacker holding it already owns
the producer economically. Rotation adds one new capability only: moving the **attestation**
identity without touching bonds.

*Trade-off considered and rejected:* requiring an **old-BLS countersignature** would close T6
completely, but it is self-defeating — the entire motivating population (2 live, 33 latent, per
`docs/.workflow/bls-mismatch-census-2026-09-11.md`) has **lost** the old BLS secret. A
countersignature with a lost-key exception reopens the hole for exactly the attacker who claims key
loss. **Recommendation: no countersignature.** The T1 half of the attack (stealing *another*
producer's attestation identity) is closed by SEC-004; the residual (moving your own attestation
identity) is bounded by SEC-005 and made loud by REQ-ROT-012 observability.

### T7 — Silent divergence from a straggler node
A node without this code cannot decode a block carrying the new `TxType` at all
(`Block::deserialize` is `bincode::deserialize(bytes).ok()`, `crates/core/src/block.rs:246-248`):
the failure is a loud stall/partition, not a silent state-root fork. **Closed at pin time by
SEC-010, not at merge time** — the gate is `u64::MAX` on every network at merge.

---

## Security Requirements

| ID | Requirement | Priority | Acceptance Criteria |
|----|-------------|----------|---------------------|
| REQ-ROT-SEC-001 | The payload carries an inner Ed25519 signature over `BLAKE3(ROTATE_BLS_SIGNING_DOMAIN \|\| genesis_hash \|\| new_bls_pubkey \|\| expiry_height)`, verified against `ProducerInfo.public_key` of the producer named in the payload. | Must | see below |
| REQ-ROT-SEC-002 | The proof of possession uses a **new** DST, distinct from `POP_DST` and `ATTESTATION_DST`, over `producer_ed25519 \|\| new_bls_pubkey \|\| genesis_hash`. | Must | see below |
| REQ-ROT-SEC-003 | The payload carries `expiry_height`; a rotation is valid only when `block_height <= expiry_height <= block_height + ROTATION_TX_MAX_LIFETIME` (= `BLOCKS_PER_REWARD_EPOCH` = 360). No new `ProducerInfo` field. | Must | see below |
| REQ-ROT-SEC-004 | `new_bls_pubkey` must not equal any other producer's `bls_pubkey`, nor any pending rotation target, nor the sender's own current key. | Must | see below |
| REQ-ROT-SEC-005 | At most one pending rotation per producer, enforced identically at mempool admission, block builder and block validation. | Must | see below |
| REQ-ROT-SEC-006 | Sender must be a registered, non-exited, non-unbonding producer present in the producer set (or in `pending_updates` as a `Register`). | Must | see below |
| REQ-ROT-SEC-007 | Field validation: `extra_data` exactly 248 bytes; `new_bls_pubkey` a valid 48-byte compressed G1 point in the correct subgroup, not the identity and not all-zero; `bls_pop` 96 bytes; `signature` 64 bytes. | Must | see below |
| REQ-ROT-SEC-008 | An invalid rotation makes the carrying **block invalid** (no skip arm). Mempool/builder parity makes this unreachable for an honest builder. | Must | see below |
| REQ-ROT-SEC-009 | Both signed messages commit to the network genesis hash, so a testnet rotation cannot be replayed on mainnet or devnet. | Must | see below |
| REQ-ROT-SEC-010 | A fleet-version census is a recorded **precondition on pinning** the activation height (not on merging). | Must | see below |

### REQ-ROT-SEC-001: Actor authentication
- [ ] Given a payload signed by producer P's Ed25519 key, when validated, then it is accepted.
- [ ] Given the same payload with the signature replaced by another producer's signature, when validated, then `[ERRTX-ROT003] rotation signature invalid` and the block is rejected.
- [ ] Given an all-zero signature (`Signature::default()`), when validated, then rejected — verification fails closed.
- [ ] Given a payload whose `new_bls_pubkey` byte 0 is flipped after signing, when validated, then rejected (the signature commits to the key).
- [ ] Given a payload whose `expiry_height` is changed after signing, when validated, then rejected.
- [ ] The verdict is identical at `validate_transaction` (stateless), mempool admission, block builder and `apply_block`.

### REQ-ROT-SEC-002: PoP domain separation
- [ ] `ROTATE_POP_DST != crypto::bls::POP_DST` and `!= crypto::ATTESTATION_DST` (asserted by a test over the three byte strings).
- [ ] Given a **registration** PoP for key K copied into a rotation payload for key K, when validated, then rejected — the registration PoP does not satisfy the rotation DST or the bound message.
- [ ] Given a valid rotation PoP for producer A, when the same PoP is placed in a rotation payload naming producer B (same `new_bls_pubkey`), then rejected — the PoP message commits to the Ed25519 key.
- [ ] Given a rotation PoP for key K under `ROTATE_POP_DST`, when used as a `RegistrationData.bls_pop`, then registration rejects it.
- [ ] The new write path is PoP-covered by construction, closing the rotation arm of spec precondition **P8**.

### REQ-ROT-SEC-003: Replay binding
- [ ] Given a rotation mined at height h with `expiry_height = h + 10`, when the identical tx bytes are re-submitted at height `h + 11`, then rejected with `[ERRTX-ROT005] rotation expired`.
- [ ] Given `expiry_height = h + 361`, when validated at height h, then rejected — lifetime exceeds one epoch.
- [ ] Given `expiry_height < h`, when validated at height h, then rejected.
- [ ] Given a rotation mined at h and re-submitted at `h + 1` (inside both the window and the deferral window), then rejected by REQ-ROT-SEC-005 (one pending rotation).
- [ ] `bincode::serialize(&ProducerInfo)` for a producer that has never rotated is **byte-identical** before and after this change (golden vector) — no field was added.

### REQ-ROT-SEC-004: `bls_pubkey` uniqueness
- [ ] Given producer V with on-chain key K, when producer A submits a rotation to K, then rejected with `[ERRTX-ROT006] bls key already in use`.
- [ ] Given producer A with a pending rotation to K, when producer B submits a rotation to K in the same epoch, then B is rejected.
- [ ] Given producer A rotating to its own current key, then rejected (no-op rotation).
- [ ] The uniqueness scan covers exited and unbonding producers, not only active ones.
- [ ] The scan is deterministic and order-independent (same verdict regardless of `HashMap` iteration order).

### REQ-ROT-SEC-005 / 006 / 007 / 008 / 009 / 010
- [ ] **SEC-005**: a second rotation for the same producer is rejected at mempool `add_transaction`, at the builder, and at block validation; the pending set is derived from BOTH `ProducerSet::pending_updates` and mempool-resident entries, mirroring `crates/mempool/src/pending_registrations.rs`.
- [ ] **SEC-006**: a rotation from a pubkey absent from the producer set is rejected; from an exited producer is rejected; from a producer whose `Register` is still pending is accepted (and lands after the Register at the same boundary).
- [ ] **SEC-007**: payloads of 247 and 249 bytes are rejected; a 48-byte non-subgroup point is rejected; the all-zero 48-byte key is rejected; the identity point is rejected; an empty `extra_data` is rejected.
- [ ] **SEC-008**: a block containing any rotation that fails any check above is rejected in full (`ValidationError`), and no producer state is mutated. Rationale recorded: this matches the `Registration` "one pending per producer" precedent (`validation/registration.rs:166-177` + `pending_registrations.rs`), **not** the AddBond/UTXO rationale (INC-I-203) — rotation creates no UTXOs. Residual griefing risk (a tx valid at admission time that becomes invalid at block time costs the honest includer its slot) is identical in kind to `Registration`'s, which has been live since genesis, and is accepted.
- [ ] **SEC-009**: the same payload bytes valid on testnet are rejected on mainnet and on devnet (test drives all three `NetworkParams`).
- [ ] **SEC-010**: the pinning session must record a fleet census showing 100 % of reachable producers on a binary that decodes the new `TxType`, because an un-upgraded node cannot parse **any** block containing one (`block.rs:246-248`). This is a precondition on pinning, not on merging; at merge the gate is `u64::MAX` everywhere.

---

## Reconciliation with the skeptic analysis

| Skeptic point | Verdict | Basis |
|---|---|---|
| **C1** PoP replayable across producers; rotation weaponises it | **ACCEPTED** | Verified: `bls.rs:616,636`, `registration.rs:259-275`, no uniqueness check anywhere (only 4 `bls_pubkey` writes exist), `bls.rs:695` does not dedup → REQ-ROT-SEC-002 + SEC-004 |
| **C2** "signed with its Ed25519 key" is not a mechanism this codebase has; `Exit` is disqualified by name | **ACCEPTED** | `types.rs:130-131` verbatim → REQ-ROT-SEC-001; brief item 1 overridden |
| **C3** Replay has no precedent; brief items 3 and 4 contradict each other | **ACCEPTED** | `data.rs:284-299` has no nonce/height/chain → REQ-ROT-SEC-003 replaces "bind to the current on-chain key" |
| **C4** Deferral buys nothing and hides divergence for an epoch | **PARTLY ACCEPTED / PARTLY REFUTED** | Accepted: `pending_updates` is outside `serialize_canonical` (`set_persistence.rs:79-113`). Refuted as a fork carrier here: with a new `TxType` an un-coded node cannot parse the block (`block.rs:246-248`), and a constant gate makes same-binary gate skew impossible. Deferral retained; see decision (b) |
| **C5** `rollback.rs` mirrors nothing; the second apply is `rewards.rs:1117` | **ACCEPTED** | Verified: 7 `match` arms at `rewards.rs:1259…1494`, 4 call sites → REQ-ROT-007 |
| **C6** The wire criterion measures the wrong number; old nodes do not reject cleanly | **ACCEPTED** (conclusion **REFUTED**) | Ordinal reasoning accepted → REQ-ROT-001 declares last and proves it with a golden vector. The conclusion "prefer an `extra_data` extension" is refuted: see decision (a) — an old node would apply the **base** semantics of the overloaded type |
| **C7** Wrong layer for most of the population; `add_bls_key` already imports | **PARTLY ACCEPTED / FACT REFUTED** | Fact refuted: `Wallet::add_bls_key()` **generates** `BlsKeyPair::generate()` and errors if a key exists (`wallet.rs:401-414`) — it cannot import. Concern accepted: client-side recovery works (census producer `13cf5207` self-recovered in 6 h) → REQ-ROT-014 (Should). Census settles the population: 2 live lost-key, 33 latent |
| **C8** Skip-vs-reject is undecided | **ACCEPTED, decided** | REJECT the block, on the `Registration` "one pending per producer" precedent rather than the AddBond/UTXO rationale → REQ-ROT-SEC-008 |
| **C9** Stealth attestation takeover | **ACCEPTED, re-scoped** | Not a privilege escalation (the Ed25519 key already controls exit, withdrawal and the reward destination, `rewards.rs:359`). The cross-producer half is closed by SEC-004; countersignature rejected as self-defeating → security doc T6, REQ-ROT-020 (Won't) |
| **C10** A new `ProducerInfo` field forks at deploy, not at activation | **ACCEPTED** | Verified `set_persistence.rs:79-113` + `snapshot.rs:25-58`; `serialize_canonical(&self)` has no height parameter → hard constraint on REQ-ROT-SEC-003 and REQ-ROT-012 |
| Hidden reqs 1-9 | **ALL ACCEPTED** | → SEC-004, SEC-002, SEC-003, REQ-ROT-007, SEC-005, SEC-008, REQ-ROT-012, SEC-010, REQ-ROT-003 |
| "Missing dimension — security" | **ACCEPTED** | Global Rule 17 chain armed; 10 `REQ-ROT-SEC-*` requirements, all Must |

---
