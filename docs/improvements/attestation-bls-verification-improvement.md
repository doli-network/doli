# Attestation Verification — Cost & Impact Analysis (Option A: BLS aggregate vs Option B: embedded Ed25519)

> **HISTORICAL — 2026-08 PRE-DECISION ANALYSIS. SUPERSEDED. DO NOT IMPLEMENT FROM THIS DOCUMENT.**
>
> This is the cost/impact analysis written in **August 2026** (workflow run 500), *before* the
> attestation-verification path was chosen. Its SSF recommendation — **Option B, embedded
> Ed25519 signatures** — was **NOT** the path taken. The user chose **Option A (real BLS
> aggregate verification)**, tracked as **INC-I-178**.
>
> - **Authoritative design:** `specs/attestation-bls-architecture.md` (approved 2026-09-04).
> - **Authoritative requirements:** `docs/redesigns/attestation-bls-redesign-analysis.md` §8.
> - **Authoritative behaviour:** the shipped code, and `specs/protocol.md` §"Attestation BLS
>   aggregate verification" / `specs/security_model.md`.
>
> This file is committed under **REQ-BLS-016** for provenance only: it records the measured
> numbers and the reasoning that were on the table when the decision was made. Where anything
> below disagrees with the shipped code, **the code wins** and this document is wrong by
> construction — it describes a road not taken. §10's milestone plan covers Option B and was
> replaced wholesale by the M0–M8 plan in `docs/.workflow/milestone-progress.md`.


- Workflow run: **500** (`/omega-improve`) · Agent: analyst · Mode: analysis only, **no source edits**
- Repo HEAD: `e6d72577` · Worktree: `.claude/worktrees/attestation-bls-impact` · Date: 2026-08-08
- Upstream evidence: `docs/reports/attestation-security-claims-verdict.md`, `docs/redesigns/attestation-verification-redesign-analysis.md` (INC-I-141)
- Related open work: **INC-I-162** (wallet BLS derivation, run 498 — a hard prerequisite for Option A), INC-I-154 (attestation observability), hotfix `hotfix_whitepaper_bls_attestation_false_claim`

---

> **⚠️ DECISION SUPERSEDES §1 — tracked as INC-I-178.**
> The user chose **Option A (real BLS aggregate verification)** over this doc's Option B recommendation.
> Rationale: Law 3 design target (1000s of producers) makes BLS the endgame; one consensus deploy instead of
> two; Option A has the smaller format risk (the aggregate field already exists on the wire); the published
> whitepaper claim becomes true as written. Since this doc was written, **INC-I-162 was resolved** (`bef4deef`,
> `8bca505f`, `c12e2678`, `69f33983`): Option A's prerequisite shrinks to a fleet BLS key-match audit (GS-012),
> revised estimate **4–6 weeks**. The §10 milestones below cover Option B and must be redrawn for Option A —
> see INC-I-178 entries for the M1–M5 sketch and pickup steps.

## 1. SSF Recommendation

**Embed the individual Ed25519 attestation signatures — the ones the protocol already produces and already
verifies on gossip — in the block body in bitfield-index order, and verify them at block validation behind one
new forward-only activation height in `NetworkParams`.**

This is the simplest mechanism that removes the root cause (a block commits to a set of claimed attesters but
carries zero evidence any of them attested) because it introduces **no new key, no new signature, no new gossip
byte, and no change to what a bitfield bit means** — it only moves evidence that already exists on the wire into
the block, where every node can check it deterministically. Option A (real BLS aggregate) is 100× cheaper in
bytes and is the correct end-state at 1000s of producers, but at today's scale it costs strictly more to build
and carries two risks Option B does not: it needs the bitfield's meaning changed from "attested any block this
minute" to "attested this block's parent" (touching all 5 decoders — the exact class of change that caused the
Full Bitfield Decode death spiral), and it is blocked on INC-I-162 because a producer restored from its 24-word
seed phrase gets a *random* BLS key, so activating BLS verification would silently strip rewards from every
restored producer. Choosing A today to avoid editing a whitepaper paragraph is backwards: the doc is an hour,
the consensus change is weeks.

---

## 2. Anchor Detection (deterministic-reasoning protocol, fallback — no `skeptic-analysis.md` present)

**FIRST READ:** "The whitepaper promises a BLS aggregate; the honest fix is to build the BLS aggregate."
*Setting aside.*

**CONTRADICTING SECOND INTERPRETATION:** "The whitepaper is the cheapest artifact in the system to change. The
requirement is *verifiable attendance*, not *BLS*. Pick the mechanism on engineering merit and then make the doc
describe it."

**Chosen: the second**, on evidence — §5.1 shows Option A's headline "one pairing, O(1)" property does not hold
under the bitfield semantics that actually exist in the code (`bls_verify_aggregate` uses
`fast_aggregate_verify`, which requires a **single common message**, while the bitfield summarises a 6-slot
minute spanning up to 6 distinct block hashes). The first interpretation silently assumes a cost model the code
does not support.

---

## 3. Architecture Context — the attestation pipeline as it exists at `e6d72577`

### 3.1 What an attester signs

`crates/core/src/attestation.rs:112-117` — `signing_bytes = block_hash (32 B) || slot (4 B, big-endian)`.
Signed Ed25519 with domain separation via `signature::sign_with_domain(ATTESTATION_DOMAIN, ...)` (`:60`).
A BLS variant exists (`new_with_bls`, `:74-102`, message `crypto::attestation_message(block_hash, slot)` under
DST `BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_DOLI_ATTEST_V1`, `crypto/src/bls.rs:46`) but **has zero callers.**

### 3.2 Pipeline, end to end

| # | Stage | Site | Behaviour |
|---|---|---|---|
| 1 | Attest | `bins/node/src/node/startup.rs:601` | `Attestation::new` — **Ed25519 only**. Explicit comment at `:599`: *"BLS attestation aggregate is retired… the `bls_signature` field stays empty."* |
| 2 | Publish | `gossip/publish.rs:42`, `VOTES_TOPIC` `gossip/mod.rs:30`; plus direct point-to-point delivery to the slot+1 producer (`startup.rs:617-619`) | ~156 B bincode per attestation |
| 3 | Receive + **verify** | `network_events.rs:536-538` → `Attestation::verify()` (`attestation.rs:105-109`) | **1 Ed25519 verify per attestation.** The only cryptographic check in the whole attendance path, and it is real |
| 4 | Track | `network_events.rs:557/559` → `MinuteAttestationTracker` (`attestation.rs:375-402`) | `pubkey → set<minute>`; a `bls_sigs` map exists but is fed empty vectors from gossip; only the node's **own** BLS signature is ever stored (`apply_block/post_commit.rs:420-431`) |
| 5 | Encode | `production/assembly.rs:345-408` | Order `[epoch_state.producer_list \| extra active-not-in-list, sorted by pubkey]`; `encode_attestation_bitfield_vec(indices, total_len)` |
| 6 | Commit | `production/assembly.rs:409`, `production/mod.rs:601-606` | `presence_root = BLAKE3(attestation_bitfield)`; `aggregate_bls_signature = Vec::new()` (`mod.rs:599`) |
| 7 | **Validate** | `validation_checks.rs:387-416` | Only (a) `presence_root == BLAKE3(bitfield)` and (b) no bits beyond `producer_count`. **No evidence check.** |
| 8 | Consume — liveness | `apply_block/post_commit.rs:61` | decode → 3-epoch exclusion / re-entry (`epoch_state/mod.rs`) |
| 9 | Consume — **money** | `node/rewards.rs:139` (`calculate_epoch_rewards`), `:814`, `:991` (`rebuild_epoch_state_from_blocks`) | union of minutes per index; ≥54/60 qualifies; a non-qualifier's share is **redistributed to qualifiers** (`rewards.rs:43`) |
| 10 | Enforce | `validation_checks.rs:701`, `:800` | every validator **re-derives** `calculate_epoch_rewards` and compares against the block's EpochReward tx |
| 11 | Report | `rpc/methods/schedule.rs:306` | attestation stats |

Step 10 is why this matters: the unverified bitfield is not cosmetic — it is the sole input to a
consensus-enforced money transfer.

> **Correction to the mission brief:** `WeightedRewardCalculator` is **not** a live consumer.
> `bins/node/src/node/mod.rs:53` records it was *"removed — replaced by attestation-qualified bond-weighted
> distribution."* It survives only in `crates/core/src/rewards.rs` and its own tests. Presence is consumed by
> `calculate_epoch_rewards` (rows 9-10).

### 3.3 Module boundaries and dependency direction

```
crates/crypto (Ed25519, blst BLS12-381, PoP)         ← leaf, no deps on core
  ↑
crates/core (attestation.rs, block.rs, validation/)  ← defines Attestation, Block, bitfield codec
  ↑                        ↑
crates/network (gossip)    crates/storage (block_store bodies, ProducerSet.bls_pubkey)
  ↑                        ↑
bins/node (startup, network_events, production/assembly, validation_checks, rewards, post_commit)
  ↑
crates/rpc (schedule.rs stats)
```
Direction is strictly upward. A verification change is contained in `core` + `bins/node` + (Option B only)
`storage`; it never inverts a dependency.

### 3.4 Invariants that MUST be preserved

| Invariant | Source | What breaks if violated |
|---|---|---|
| **Encoder/decoder index parity** — order `[base_list \| extra sorted by pubkey]` across **1 encoder and 5 decoders**: `assembly.rs:408`; `post_commit.rs:61`, `rewards.rs:139`, `rewards.rs:814`, `rewards.rs:991`, `rpc/schedule.rs:306` | Full Bitfield Decode pillar (v6.17.1, h=14000), CLAUDE.md | Misalignment credits the wrong producers → wrong reward set → chain-wide rejection / death spiral |
| **3 states untouched** — attestation evidence is block CONTENT, never ChainState/UtxoSet/ProducerSet | `storage/snapshot.rs:24-58` hashes only the 3 canonical serialisations | Snap-sync divergence. *Confirmed safe:* signatures never enter any of the three; `ProducerInfo.bls_pubkey` is **already** inside the ProducerSet root (`producer/set_persistence.rs:97` bincodes the whole `ProducerInfo`), so BLS keys are already consensus state, identical on every node |
| **Block hash stability** — `Block::hash() == header.hash()` (`block.rs:188-190`) | — | Body additions do **not** change the block hash → no hash-divergence fork; but body evidence is **not** header-committed (risk R3) |
| **Producer mutations deferred to epoch boundary** | CLAUDE.md; `PendingProducerUpdate` | A new verification path must read keys from the **applied** set, not pending |
| **Activation heights are forward-only and live in `NetworkParams`** | INV-PARAMS-001, INV-DEPLOY-001, `network_params/defaults.rs` | Moving a crossed height deactivates live rules (INC-I-054) |

### 3.5 Blast radius of adding verification

*Instrument disclosure:* the code graph (`graphify-out/graph.json`) confirmed `bls_verify_aggregate` has
**3 dependents, all tests** (`crypto/src/bls.rs:709,758,829`) — zero production callers, matching grep. It
returned **0 dependents for `decode_attestation_bitfield_vec`**, a known lower bound: graphify cannot resolve
Rust path-qualified `doli_core::fn()` calls (project memory `reference_graphify_rust_method_blind_spot`). The
5-decoder list above was therefore produced by grep and is labelled as such.

- **Direct:** `core/attestation.rs`, `core/block.rs`, `node/production/assembly.rs`, `node/production/mod.rs`,
  `node/validation_checks.rs`, `core/network_params/{mod,defaults,env_loader}.rs`
- **Option B only:** `storage/block_store/{types,writes,queries}.rs` (body format)
- **Option A only:** `crypto/bls.rs` (cache / `pks_validate` variant), `node/startup.rs`, `node/network_events.rs`
- **Indirect (consume the bitfield; must be re-reasoned only if *semantics* change):** `node/rewards.rs` ×3,
  `apply_block/post_commit.rs`, `core/epoch_state/mod.rs`, `rpc/methods/schedule.rs`
- **Untouched:** all of `sync/`, mempool, UTXO, VDF, snapshot / state-root

---

## 4. Assumptions used in every number below

| Symbol | Value | Basis |
|---|---|---|
| Slot / minute / epoch | 10 s; 6 slots per attestation-minute (`attestation.rs:257`); 360 blocks = 60 minutes/epoch; qualify ≥54 | code |
| Blocks per day | 8,640 | derived |
| N (active producers today) | **37 observed** at h=129,240 (INC-I-154, 2026-08-06); brief states ~45. Tables use **45** as the conservative upper bound | memory.db + brief |
| Typical popcount | ~0.9 N | assumed |
| Ed25519 sig / pubkey | 64 B / 32 B; verify ≈ 50 µs; batch-verify ≈ 3× amortised | INC-I-141 §2.4, `basis=assumed` |
| BLS12-381 | pubkey 48 B (`bls.rs:37`), signature 96 B (`bls.rs:40`); pairing verify 1.5-2 ms; G1 decompress+subgroup 40-150 µs; G1 add ~1 µs; G2 sign ~1 ms | INC-I-141 §2.4, `basis=assumed`, not measured on this hardware |
| Block budget | `BASE_BLOCK_SIZE` = 2,000,000 B (`consensus/constants.rs:462`) | code |

---

## 5. Option A — real BLS aggregate verification

### 5.1 What must exist (and the design problem that sets the price)

1. **Attesters must sign twice again.** Today they do not sign BLS at all (`startup.rs:601`). Option A must
   restore `new_with_bls`, plumb `self.bls_key` into `create_and_broadcast_attestation`, and carry +96 B (+8 B
   length prefix) per gossiped attestation (~156 B → ~260 B, **+67 %**). Ed25519 must stay — it is what gossip
   authenticates on receipt.
2. **The `fast_aggregate_verify` same-message constraint.** `crypto::bls_verify_aggregate` (`bls.rs:621-646`)
   calls `sig.fast_aggregate_verify(true, message, ATTESTATION_DST, &pk_refs)` — **one** message for all
   signers. But the bitfield claims presence over a **minute** (`assembly.rs:345-346` uses
   `attested_in_minute`), and within that minute attesters signed up to **6 different** `block_hash||slot`
   messages. The "one pairing, O(1)" figure from INC-I-141 §1.4 is therefore **not achievable without changing
   what a bit means.** Two ways out:
   - **A1 (the only sane variant if A is chosen): redefine bit *i* = "producer *i* attested THIS block's
     parent."** One common message → one pairing. Cost: a consensus-visible semantic change rippling into reward
     attribution and the 3-epoch liveness filter. Per-minute coverage is *approximately* preserved (the 6 blocks
     of a minute attest parents at slots S-1…S-6, so the union still catches essentially the same set), with
     genuine edge effects at minute boundaries that must be specified and replay-tested.
   - **A2: keep minute semantics, use distinct-message `aggregate_verify`** → N Miller loops (~0.6 ms each) and
     the producer must additionally commit *which* in-minute block each signer attested. Strictly worse than A1
     on every axis. Rejected.
3. **Validator-side re-derivation.** Decode the bitfield in canonical order, look up `ProducerInfo.bls_pubkey`
   for each set bit, aggregate the pubkeys, one pairing.
4. **Key availability — better than expected.** The original 2026-03 gap was `ctx.producer_bls_keys` never being
   populated; that field no longer exists at all. The real key list is already on-chain and already inside the
   state root: `bls_pubkey` is **mandatory at registration** with proof-of-possession
   (`validation/registration.rs:47` genesis path, `:144` normal path, `validate_bls_pop` at `:57`/`:154`,
   `:260`), and it is persisted by the live apply path (`apply_block/tx_processing.rs:238`), genesis completion
   (`apply_block/genesis_completion.rs:134`) and the rebuild path (`rewards.rs:1277`, `:1471`). **Residual
   risk:** `register_genesis_producer` (`producer/set_registration.rs:210`) initialises `bls_pubkey` empty, so a
   pre-AH audit that every active producer has a non-empty, well-formed key is mandatory.

### 5.2 Engineering cost

| Item | Detail |
|---|---|
| Block format | **No change.** `Block.aggregate_bls_signature` already exists (`block.rs:164`), is already on the gossip wire (`Block::serialize` → bincode, `block.rs:239`) and already persisted (`block_store/types.rs:13`). Option A is a **content-value** change only — its single biggest advantage |
| Modules touched | `crypto/bls.rs` (cached-pubkey + `pks_validate=false` variant), `core/attestation.rs`, `node/startup.rs`, `node/network_events.rs`, `node/production/{assembly,mod}.rs`, `node/validation_checks.rs`, `core/network_params/{mod,defaults,env_loader}.rs`, plus re-specification of `rewards.rs` ×3 / `post_commit.rs` / `epoch_state` under A1 semantics → **9-10 modules** |
| LOC | ~900-1,400 non-test |
| Test surface | Encode/decode parity at N∈{45,200,1000}; AH boundary (below / at / above); forged-bit rejection **on the live apply path**; missing/empty `bls_pubkey`; poisoned-signature bisection; x86-vs-ARM blst determinism; semantic-change regression on reward attribution across a full 360-block epoch |
| **Prerequisite** | **INC-I-162** — deterministic BLS derivation from the seed phrase, plus a re-key path for producers already restored. Non-negotiable (risk R2) |
| **Estimate** | **5-7 engineering weeks** including INC-I-162, testnet soak and AH rollout |

### 5.3 Runtime / hardware cost per node class

Per **validator**, per block (A1, one common message):

| Stage | N=45 | N=1000 | Note |
|---|---|---|---|
| Bitfield decode + stray-bit scan | <10 µs | ~150 µs | unchanged from today |
| BLS pubkey decompress + subgroup, **naive** (`pks_validate=true`, no cache — what `bls.rs:640` does today) | ~9 ms | ~200 ms | 2× redundant: caller decompresses, blst validates again |
| Same, **with epoch-scoped decompressed-key cache + `pks_validate=false`** | ~45 µs (G1 adds) | ~1 ms | keys are PoP-checked at registration |
| Aggregate pairing | ~2 ms | ~2 ms | N-independent |
| **Total (cached design)** | **≈2 ms** (0.02 % of slot) | **≈3 ms** (0.03 % of slot) | |

Per **producer**, at assembly: aggregating N stored G2 signatures. Naive (`bls_aggregate`, `bls.rs:600-607`,
deserialises and validates each) ≈ 7-11 ms @45, 150-250 ms @1000. With deserialised-point caching ≈ 90 µs @45,
2 ms @1000. Add **one self-verify of the aggregate before publish** (~2 ms); on failure, bisect to find the
poisoner — log₂(N) ≈ 6 extra verifies @45 (~12 ms), ~10 @1000 (~20 ms). This bisection is what makes it safe to
*not* verify every incoming BLS signature individually — which would cost N × ~2 ms = 90 ms/slot @45 and
**2 s/slot @1000**, the real wall.

Per **attester**, per slot: one BLS sign ≈ 1 ms. Negligible.

Bandwidth: **on-chain +96 B/block, flat in N.** Gossip: **+104 B per attestation × N per slot**, multiplied by
gossipsub mesh fanout (D≈6-8) — the dominant new bandwidth term and the one that scales with N.
Memory: +48 B per producer for the cached key set (2 KB @45, 48 KB @1000). IO unchanged. Locks: one extra
`producer_set.read()` on the validate path — the same lock already taken at `validation_checks.rs:404`.

### 5.4 Consensus deployment cost

Changes consensus **rules** (a new validity condition) and block **content** (a real signature where empty was)
→ new `NetworkParams` activation height, forward-only, **no genesis reset** (Rule #0). Blocks below the AH keep
accepting an empty aggregate, unchanged. Precedent: `inc_i_147_activation_height: 129_500`
(`network_params/defaults.rs:251`); at 8,640 blocks/day a new AH should sit ≥2 days ahead of tip.

**INV-12 three-question checklist:**
1. *Can a user-submittable tx reach this path?* **No** — attestation bits are producer-assembled, not tx-derived.
2. *Can a producer-action or attestation pattern reach it?* **Yes** — every produced block, every attestation.
3. *Is the new behaviour bit-identical for ALL reachable inputs?* **No** — post-AH, previously-valid blocks
   (empty aggregate, or a fabricated bit) become invalid.

→ **(2) YES + (3) NO ⇒ activation height REQUIRED.** No `HardForkSchedule` entry — `current_fork_id(u64::MAX)`
makes every entry active immediately (CLAUDE.md). Constant/AH gate in `NetworkParams` only.

### 5.5 Risks

- **R1 — index parity (highest).** A1 changes bit *semantics* while the 5 decoders keep the same *order*. A
  semantic change is harder to test than an order change and shares the failure surface of the Full Bitfield
  Decode death spiral (v6.17.1, h=14000), where an encoder/decoder mismatch permanently excluded producers.
- **R2 — seed-phrase BLS bricking (blocking).** `Wallet::from_seed_phrase` calls `BlsKeyPair::generate()`
  (`bins/cli/src/wallet.rs:93`; identically `crates/wallet/src/wallet.rs:99`) — a *random* BLS key unrelated to
  the phrase, while the Ed25519 key restores correctly. `bins/node/src/keys.rs:40-64` loads BLS from
  `wallet.json` and returns `None` for pre-BLS wallets. Post-AH, any producer whose runtime BLS key ≠ its
  on-chain `bls_pubkey`, or who has none, emits unaggregatable attestations → its bit is never set → it falls
  below 54/60 → loses epoch rewards and eventually drops out of the active set via the 3-epoch filter. This is a
  latent key-management defect (memory.db findings AUDIT-P2-005 / AUDIT-P2-003; INC-I-162) that Option A
  **converts into a live consensus and economic failure.**
- **R3 — unauthenticated body field.** `Block::hash()` is header-only (`block.rs:188-190`) and `presence_root`
  commits to the bitfield **but not to the aggregate**. Any relaying peer can strip or corrupt the 96 B
  signature without changing the block hash, turning a valid block invalid in transit (grief / censorship).
  Needs either a header commitment (header format change — expensive) or an explicit accept-and-refetch policy.
  *Option B inherits this identically.*
- **R4 — why 2026-03 died, and what prevents recurrence.** Introduced `30903c8b` (2026-03-10) but dead on
  arrival: the only caller of `validate_bls_aggregate` was `validate_block()`, which itself had zero call sites
  (the live path was `validate_block_with_mode`), and `ctx.producer_bls_keys` was permanently `Vec::new()`. It
  never executed once in ~4 months and was removed as provably-dead code (`86bac138`, `427d5050`, 2026-07-19) —
  **not** as a chain-halt fix. Prevention: a mandatory negative test that a forged bit is rejected *through
  `validate_block_for_apply`* (not through a bare validator), plus a live `[ATTEST_VERIFY]` counter so "zero
  executions" is visible in production.
- **R5 — liveness is not at risk (verified).** A producer can only set bits for attestations it actually holds
  (`assembly.rs:388-394` maps held pubkeys to indices), so a missing attestation costs reward accuracy, never
  block production. Consistent with INC-I-154: a producer that attested nothing for hours caused zero consensus
  disturbance.

---

## 6. Option B — embed individual Ed25519 attestation signatures

### 6.1 Design

Add a body field `attestation_signatures: Vec<[u8; 64]>`, one entry per set bit, **in bitfield index order** —
signer identity is implied by bit position, so no pubkey bytes are carried. Two message-binding variants:

- **B1:** adopt per-parent-block semantics (same as A1) → all signatures over one message, no extra data, but it
  pays the same semantic-change risk R1.
- **B2 (recommended):** keep today's minute-union semantics and carry a **3-bit selector per set bit**
  (`ceil(3·popcount/8)` bytes: +15 B @45, +338 B @1000) naming which of the ≤6 in-minute blocks the signature
  covers. The validator already holds those block hashes — they are its own chain. **No bitfield semantic change
  at all.**

Verification: for each set bit, recover the producer's Ed25519 pubkey at that index (the active list is already
computed at `validation_checks.rs:404-406`), reconstruct `block_hash||slot` from the selector, verify. Nodes that
already saw the attestation on gossip — the overwhelmingly common case (`network_events.rs:538`) — short-circuit
via an `(attester, block_hash)` verified-cache, making block-time cost near zero in steady state.

### 6.2 Engineering cost

| Item | Detail |
|---|---|
| Block format | **Changes.** New field on `Block` (`core/block.rs`) and on `BlockBody` (`storage/block_store/types.rs:10-16`), a new fallback arm in `deserialize_body` (`:38-82`), and the write/read paths (`writes.rs:36`, `queries.rs:45`) |
| Wire compatibility | Blocks gossip as bincode of `Block` (`block.rs:239` → `command_handling.rs:39`); bincode 1.3 is positional with no field names. **Assumption (`basis=assumed`, must be proven by golden vector before any deploy):** an old binary tolerates trailing bytes from a field appended last, so new blocks stay parseable by un-upgraded nodes. If false, the ~30 external producers partition the moment a populated block is gossiped, and Option B needs a full-fleet parse-capable rollout **before** the AH |
| Modules touched | `core/attestation.rs` (tracker retains sigs + selector), `core/block.rs`, `storage/block_store/{types,writes,queries}.rs`, `node/production/{assembly,mod}.rs`, `node/validation_checks.rs`, `core/network_params/*`, optional `crypto` batch-verify helper → **7-8 modules** |
| LOC | ~600-900 non-test |
| Test surface | Golden-vector wire back-compat (old binary parses a new block); index↔signature alignment at N∈{45,200,1000}; AH boundary; forged-bit rejection; wrong-selector rejection; truncated / oversized signature vector; size ceiling vs `BASE_BLOCK_SIZE` (INV-NETWORK-001) |
| **Prerequisite** | **None.** Reuses the Ed25519 key that the seed phrase *does* restore correctly |
| **Estimate** | **3-4 engineering weeks** including testnet soak and AH rollout |

### 6.3 Runtime / hardware cost

Per **validator**, per block: `popcount` Ed25519 verifies. Cold (nothing cached) ~2 ms @N=45, ~9 ms @200,
~45 ms @1000 with single-verify; ~0.7 / 3 / 15 ms with batch verification (the `ed25519-dalek` batch feature is
currently **absent** from the tree per INC-I-141 §4 — a small new dependency surface). Warm (attestations
already verified on gossip): ~0.
Per **producer**, at assembly: memcpy of stored signature bytes ≈ 0 CPU.
Memory: the tracker must retain 64 B per (attester, minute). Bounded to the current + previous minute that is
5.8 KB @45 / 128 KB @1000; retaining a whole epoch would cost 172 KB @45 / 3.8 MB @1000 — bound it to 2 minutes.
Gossip bandwidth: **zero change** — attestation messages are untouched. Block bandwidth: +2.6 KB/block @45 →
+58 KB @1000, i.e. 0.13 % → 2.9 % of `BASE_BLOCK_SIZE`.
IO: the dominant cost — see §7. Locks: unchanged (reuses the `producer_set.read()` already held at
`validation_checks.rs:404`).

### 6.4 Consensus deployment cost

Identical INV-12 answers to §5.4 — **(2) YES + (3) NO ⇒ activation height REQUIRED**, forward-only, in
`NetworkParams`, no `HardForkSchedule` entry, no genesis reset. One additional constraint from the format
change: **parse capability must reach 100 % of the fleet before the AH**, whereas Option A needs no such phase.

### 6.5 Risks

- **R3 (shared)** — the same unauthenticated-body-field strip hazard as Option A.
- **R6 — body-format misparse (B-specific, P1).** `deserialize_body` (`block_store/types.rs:38-82`) already
  chains 6 fallbacks. A new arm ordered wrongly can *successfully* misparse an old body, silently yielding wrong
  transactions → state divergence. Mitigation: version tag tried first, golden vectors for every historical arm.
- **R7 — chain growth.** The real cost, quantified in §7. Mitigated by the fact that the signature vector is
  **prunable**: it is not in the block hash, not in any state root, and reward re-derivation reads only the
  bitfield (`rewards.rs:139` decodes the bitfield, never a signature). Nodes can drop attestation signatures
  older than a few epochs, exactly as they already rely on snap sync + Light validation for deep history.
- **R8 — batch-verify dependency.** New crypto surface, but a well-trodden one, and only an optimisation:
  single-verify is affordable through N=1000.

---

## 7. Scale table — where A and B diverge

Assumes popcount ≈ 0.9 N; 8,640 blocks/day; bitfield = ceil(N/8) B (unchanged in both options).

| Metric | N=45 | N=200 | N=1000 |
|---|---|---|---|
| **Today** — attestation bytes/block | 6 B | 25 B | 125 B |
| **A** — bytes/block (96 B aggregate + bitfield) | 102 B | 121 B | 221 B |
| **B2** — bytes/block (64 B × popcount + selector + bitfield) | ~2.6 KB | ~11.6 KB | ~58.1 KB |
| **A** — validator verify CPU/block (cached) | ~2 ms | ~2 ms | ~3 ms |
| **A** — validator verify CPU/block (naive, today's `bls.rs:640`) | ~11 ms | ~42 ms | ~202 ms |
| **B2** — validator verify CPU/block (cold, batch) | ~0.7 ms | ~3 ms | ~15 ms |
| **B2** — validator verify CPU/block (warm, gossip-cached) | ~0 | ~0 | ~0 |
| **A** — producer assembly CPU/block (cached + self-verify) | ~2 ms | ~3 ms | ~5 ms |
| **B2** — producer assembly CPU/block | ~0 | ~0 | ~0 |
| **A** — daily chain growth from attestation data | 0.83 MB | 1.0 MB | 1.9 MB |
| **B2** — daily chain growth from attestation data | **22 MB** | **100 MB** | **502 MB** |
| **A** — annual growth | 0.3 GB | 0.4 GB | 0.7 GB |
| **B2** — annual growth | **8 GB** | **36 GB** | **183 GB** |
| **A** — gossip delta (attestation +67 %, × N/slot × mesh fanout) | +0.9 MB/day/node | +4 MB/day/node | +20 MB/day/node |
| **B2** — gossip delta | **0** | **0** | **0** |

**Divergence point: storage.** A is flat in N; B grows linearly. At N=45 the gap is 8 GB/yr vs 0.3 GB/yr — a
27× ratio but an absolutely modest number for a full node. Around **N≈200 (36 GB/yr)** B starts to hurt a modest
VPS, and at N=1000 (183 GB/yr) it is untenable **unless pruned** (R7). Conversely A pays its price in gossip
bandwidth — precisely the axis INC-I-141 §2.4 identified as the true scaling wall.

---

## 8. Impact — which of the six verified claims each option closes

Governing distinction: signature-backed bits prove **inclusion honesty** (a bit cannot be set for a producer who
did not attest) but **not omission honesty** (a producer can still *withhold* bits for attestations it did
receive — nothing forces inclusion).

| Claim | Option A | Option B | Notes |
|---|---|---|---|
| **1 — attestation is a declaration, not a proof** | **Closed** | **Closed** | Both make every set bit carry verifiable evidence |
| **2a — self-inflation (fake bits / mark self present while offline)** | **Closed** | **Closed** | Forging a bit requires forging a signature |
| **2b — reward theft by denial** (omit a target's bits across ≥7 minutes, needing every block in each) | **Not closed** | **Not closed** | Withholding is indistinguishable from not having received. Needs a distinct mechanism (attester-supplied inclusion proofs, or slashing on provable omission) |
| **3 — kicking from the active set** (3-epoch zero-attestation exclusion, `epoch_state/mod.rs:249-276`) | **Not closed** | **Not closed** | Same omission gap. **Out of scope but named:** `MIN_PRODUCERS_FLOOR = 3` (`consensus/constants.rs:155`) with `epoch_prune_activation_height = 0` (`network_params/defaults.rs:106`) lets the filter legitimately shrink the active set to **3**, not the whitepaper's advertised 2/3 floor. **Untouched by either option** (active hotfix `hotfix_whitepaper_2of3_floor_vs_code_floor3`) |
| **4 — invisibility** | **Partial** | **Partial** | Inclusion-side manipulation becomes *impossible* rather than merely invisible. Omission-side manipulation stays invisible → still needs **INC-I-154** observability (per-producer expected-vs-observed attendance divergence alerting) |
| **5 — BLS keys are for show** | **Closed** — gives them an ongoing consensus role | **Not closed** — arguably worsened: BLS keys stay registration-only, and the honest follow-up is whether the double-key design should be retired (INC-I-141 REQ-ATT-008) | |
| **6 — whitepaper claims a property the code lacks** (`WHITEPAPER.md:748` + ES + `bls-attestation.html`) | **Closed as written** — the aggregate sentence becomes true, needing only precision edits | **Closed by rewrite** — the mechanism becomes individual signatures, so §10.3/§10.4 EN+ES and the published page must be reworded | **Both options require the doc fix; it should ship immediately and independently — documentation-only, zero consensus risk, and it is a live false published security claim** |

---

## 9. Resource cost — recommended option (B2)

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      +0.7 ms/block cold, ~0 ms warm at N=45 (+15 ms cold at N=1000) — under 0.01 % of a 10 s slot (inferred)
  Memory:   +5.8 KB/node at N=45 (+128 KB at N=1000) for 2-minute signature retention in MinuteAttestationTracker (inferred)
  IO:       +2.6 KB written per block per node at N=45 (+58 KB at N=1000); one extra body-column write path, no new column family (inferred)
  Network:  +2.6 KB per block on the blocks topic at N=45 (+58 KB at N=1000) times gossipsub mesh fanout on the publishing hop; zero added on the attestation/votes topic (inferred)
  Disk:     +22 MB/day = +8 GB/yr at N=45; +100 MB/day = +36 GB/yr at N=200; +502 MB/day = +183 GB/yr at N=1000 (inferred)
  Latency:  +0.7 ms cold, ~0 warm added to block validation; no change to production, gossip or sync latency (inferred)
Inevitability: AVOIDABLE
Cheaper alternative: Option A (BLS aggregate) is 100x cheaper on Disk/IO (+0.3 GB/yr flat at any N) and is the correct end-state at 1000s of producers — but it costs +67 % attestation gossip bandwidth, a bitfield semantic change across all 5 decoders, and is blocked on INC-I-162 (seed-phrase BLS derivation) without which activation silently strips rewards from every restored producer.
Why this proposal anyway: at today's N of 37-45 the disk delta is 8 GB/yr — affordable and prunable, since signatures are in no hash and no state root — and B2 buys inclusion-honesty with zero new keys, zero new gossip bytes and zero change to bitfield semantics, the smallest possible blast radius against this codebase's most dangerous failure class (encoder/decoder parity).
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Per node class, steady state at N=45 (N=1000 in brackets):

| Dimension | Seed node | Structural producer | External producer (modest VPS) |
|---|---|---|---|
| **CPU** | +0.7 ms/block cold, ~0 warm [+15 ms / ~0] | same, plus ~0 at assembly (memcpy) | same; single-verify fallback (no batch) is +2 ms [+45 ms], still under 0.5 % of slot |
| **Memory** | +5.8 KB [+128 KB] with 2-minute retention | same | same |
| **IO / storage** | **+22 MB/day, +8 GB/yr** [+502 MB/day, +183 GB/yr] — the binding cost; archiver volumes must be re-sized | same | same; **pruning of signature vectors older than K epochs is a Should before N > 200** |
| **Bandwidth — gossip (votes topic)** | **0** (attestation messages unchanged) | 0 | 0 |
| **Bandwidth — blocks topic** | +2.6 KB/block ≈ +22 MB/day [+58 KB ≈ +502 MB/day] | same | same |
| **Locks** | none new — reuses the `producer_set.read()` already taken at `validation_checks.rs:404` | same | same |
| **Block budget headroom** | 0.13 % of `BASE_BLOCK_SIZE` [2.9 %] — INV-NETWORK-001 unaffected | same | same |

---

## 10. Deploy-safety answers (both questions, verbatim)

**(1) Does this change consensus RULES?** **YES.** Post-AH a block whose bitfield has set bits but lacks
matching valid signatures becomes invalid — a new validity condition. ⇒ **Activation height REQUIRED**, added as
a new field in `crates/core/src/network_params/` (never a global constant — INV-PARAMS-001,
`feedback_activation_heights_network_params`), forward-only, its own height, never reusing or bundling with a
crossed one. **No genesis reset** (Rule #0).

**(2) Does this change block CONTENT?** **YES.** A populated body field where nothing existed. That normally
demands a synchronized deploy (stop ALL, start ALL — INC-I-062 / INV-8), which is **impossible** with ~30
external producers. **The activation height performs the synchronization instead:** all nodes switch by height,
not by restart time. Two conditions make that sound — (a) the AH sits ≥2 days (≥17,280 blocks) ahead of tip when
pinned; (b) parse capability reaches the whole fleet before the AH, which is free if the trailing-bytes
assumption in §6.2 holds and requires a forced fleet-upgrade window if it does not. **Prove it with a golden
vector before pinning the height.** Block *hashes* are unaffected either way (`Block::hash()` is header-only),
so the only fork vector is validity disagreement, which the AH eliminates.

**No `HardForkSchedule` entry** for either option — `current_fork_id(u64::MAX)` makes every entry active
immediately. Constant/AH gate in `NetworkParams` only. **No `CURRENT_PROTOCOL_VERSION` bump** — the EpochState
serialization format is unchanged (INV-4).

---

## 11. Requirements — recommended option (B2) only

| ID | Requirement | Priority | Acceptance criteria |
|---|---|---|---|
| REQ-ATT-VER-001 | Blocks below the activation height validate **bit-identically** to today | **Must** | - [ ] Replay of ≥1 full mainnet epoch below the AH accepts/rejects the identical block set<br>- [ ] No change to the `presence_root`/bitfield checks at `validation_checks.rs:387-416` for h < AH<br>- [ ] An empty signature vector is accepted below the AH |
| REQ-ATT-VER-002 | Post-AH, every set bit MUST be backed by a valid Ed25519 attestation signature from the producer at that index | **Must** | - [ ] Given a block with a bit set and no matching signature, when validated at h ≥ AH, then rejected with a stable error code<br>- [ ] Given a signature valid for a different block, then rejected<br>- [ ] Given a complete valid set, then accepted<br>- [ ] Rejection is asserted through `validate_block_for_apply`, not a bare validator (anti-R4) |
| REQ-ATT-VER-003 | Encoder/decoder index parity preserved across **all 5 decoders and 1 encoder**; the signature vector is aligned to bit order | **Must** | - [ ] Order `[epoch_state.producer_list \| extra sorted by pubkey]` unchanged<br>- [ ] Round-trip encode→decode→signature-map test at N∈{45,200,1000}<br>- [ ] `post_commit.rs:61`, `rewards.rs:139/814/991`, `schedule.rs:306` yield identical index sets pre- and post-change |
| REQ-ATT-VER-004 | Bitfield **semantics** unchanged (minute-union preserved) | **Must** | - [ ] Reward attribution over a replayed 360-block epoch is byte-identical to the pre-change computation for the same bitfields<br>- [ ] 3-epoch liveness exclusion output unchanged |
| REQ-ATT-VER-005 | The 3 states and snap sync are untouched | **Must** | - [ ] ChainState/UtxoSet/ProducerSet canonical bytes unchanged by the feature<br>- [ ] State roots converge on a mixed-arch (x86/ARM) fleet across the AH<br>- [ ] Snapshot format unchanged |
| REQ-ATT-VER-006 | Wire back-compatibility proven before the AH is pinned | **Must** | - [ ] Golden vector: a block serialized by the new binary deserializes correctly under the previous release binary<br>- [ ] If it does not, the AH is not pinned until a fleet-wide parse-capable rollout is confirmed |
| REQ-ATT-VER-007 | The gossip attestation path is unchanged | **Must** | - [ ] `Attestation` struct, wire bytes, `VOTES_TOPIC` and `network_events.rs:536-570` behaviour unchanged<br>- [ ] No new gossip bytes measured on a testnet soak |
| REQ-ATT-VER-008 | Block size stays within budget | **Must** | - [ ] A serialized block at N=1000 with full popcount is < `BASE_BLOCK_SIZE`<br>- [ ] INV-NETWORK-001 re-verified against `GOSSIP_MAX_TRANSMIT_SIZE` |
| REQ-ATT-VER-009 | Verification executions are observable in production | **Should** | - [ ] `[ATTEST_VERIFY]` log/metric per block with verified count<br>- [ ] A zero-execution regression (the 2026-03 failure mode) is detectable from one grep |
| REQ-ATT-VER-010 | Attestation signatures are prunable | **Should** | - [ ] Documented that signatures are in no hash and no state root<br>- [ ] Prune policy for signatures older than K epochs, default off, landed before N > 200 |
| REQ-ATT-VER-011 | Batch verification | **Could** | - [ ] Batch path measured faster than single-verify at N ≥ 200; feature-gated |
| REQ-ATT-VER-012 | Omission (denial / kicking) resistance — claims 2b + 3 | **Won't** (this iteration) | N/A — deferred; needs a distinct mechanism, tracked separately |
| REQ-ATT-VER-013 | `MIN_PRODUCERS_FLOOR` vs the whitepaper's 2/3 floor | **Won't** (this iteration) | N/A — separate decision session |
| REQ-ATT-VER-014 | BLS aggregate migration (Option A) | **Won't** (this iteration) | N/A — revisit when *measured* chain growth, not projected N, becomes binding; requires INC-I-162 first |

**Explicit non-goals:** no genesis reset; no `CURRENT_PROTOCOL_VERSION` bump; no `HardForkSchedule` entry; no
change to bitfield ordering or semantics; no change to the attestation gossip message; no change to reward
formulas.

---

## 12. Milestones (the recommended path touches 7-8 modules)

| ID | Name | Scope (modules) | Scope (requirements) | Depends on | Independently testable |
|---|---|---|---|---|---|
| **M1** | Tracker retains signatures | `core/attestation.rs` (`MinuteAttestationTracker` + minute selector), bounded retention | 007, plus the memory bound in §9 | — | Unit tests; zero consensus effect; deployable alone |
| **M2** | Block + storage body field (unpopulated) | `core/block.rs`, `storage/block_store/{types,writes,queries}.rs` | 006, 008 | M1 | **Golden-vector wire and body back-compat tests are the gate.** Ship to 100 % of the fleet before M4 |
| **M3** | Producer-side population behind AH | `node/production/{assembly,mod}.rs`, `network_params/*` (AH field added, set to `u64::MAX`) | 003, 004, 008 | M2 | Local testnet: blocks carry signatures, no validator checks them yet |
| **M4** | Validator-side verification behind AH | `node/validation_checks.rs`, verified-cache | 001, 002, 003, 005, 009 | M3 | Local testnet first (testnet-first law): forged-bit rejection, AH boundary, mixed-version window |
| **M5** | AH pinning + doc alignment + observability | `network_params/defaults.rs` (real height), `WHITEPAPER.md` + `WHITEPAPER-es.md` §10.3/§10.4, `explorer/doli.network/bls-attestation.html`, `specs/protocol.md`, `docs/architecture.md` | 006, 009, claim 6 | M4 + testnet soak ≥1 epoch | Mainnet rollout; the doc fix may ship **before** M1 and should |

**Sequencing note:** the whitepaper correction (claim 6) is documentation-only with zero consensus risk and
should ship **immediately**, independent of M1-M5. Leaving a published false security claim in place for the
3-4 weeks of implementation is the larger exposure.

---

## 13. What I do not understand / open questions (recorded, not blocking)

1. **bincode trailing-byte tolerance** — does the current release binary's `Block::deserialize` accept a block
   with one extra appended field? This single fact decides whether Option B needs a forced fleet upgrade before
   the AH. `basis=assumed` in §6.2; resolvable by one golden-vector test, not by reading.
2. **Actual N on mainnet today** — memory.db (INC-I-154) records 37 producers at h=129,240 on 2026-08-06; the
   brief says ~45. Tables use 45. Not load-bearing for the recommendation.
3. **How many active producers have a runtime BLS key matching their on-chain `bls_pubkey`?** Requires an RPC
   sweep of the live set (out of scope here — no network access). This is the blast radius of R2 and gates any
   future Option A. `register_genesis_producer` (`set_registration.rs:210`) inits empty, so at minimum the
   genesis cohort needs an explicit audit.
4. **Minute-boundary edge effects** if A1/B1 semantics were ever adopted — I reasoned that per-minute union
   coverage is approximately preserved, but "approximately" is not good enough for a consensus change; it needs
   a replay experiment over real mainnet epochs.
5. **Actual popcount per block** — the producer can only set bits for attestations it holds; whether direct
   slot+1 delivery (`startup.rs:617-619`) plus gossip achieves near-complete coverage in practice is measurable
   from `[ATTEST_ENCODE]` / `[ATTEST_DECODE]` logs and sets the expected popcount used throughout §7. I assumed
   0.9 N.
6. **Selector encoding for B2** — 3 bits per set bit assumes ≤6 candidate blocks per minute
   (`SLOTS_PER_ATTESTATION_MINUTE = 6`). Skipped slots reduce candidates; forks could raise ambiguity. Needs a
   precise spec before M3.

---

## 14. Specs drift detected

- `WHITEPAPER.md:746-748` + `WHITEPAPER-es.md:746-748` + `explorer/doli.network/bls-attestation.html` — claim a
  fake bit "causes aggregate signature verification to fail. The block is rejected." **False at `e6d72577`.**
  Active hotfix `hotfix_whitepaper_bls_attestation_false_claim`. Fix now, independently of this work.
- `WHITEPAPER.md` §8.1 — "2/3 deadlock floor" vs the live `MIN_PRODUCERS_FLOOR = 3`. Active hotfix
  `hotfix_whitepaper_2of3_floor_vs_code_floor3`. Out of scope, named for completeness.
- `docs/redesigns/attestation-verification-redesign-analysis.md` (INC-I-141) §1.5 states producers *do* attach a
  real aggregate, and §5 lists `registration.rs:262-316` as the dead validator. Both were true when written
  (pre-2026-07-19) and are **stale at `e6d72577`**: aggregate production stopped, the validator was deleted, and
  per-attestation BLS signing stopped entirely. Its decoder inventory also omits `rewards.rs:814` and
  `rewards.rs:991`. Recommend annotating that document with a superseded-by pointer to this one.
