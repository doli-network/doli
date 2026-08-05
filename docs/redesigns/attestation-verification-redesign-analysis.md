# Attestation / Vote Verification Scaling — Redesign Analysis (proposal-only)

- Incident: INC-I-141 · Workflow run: 460 (type `redesign`)
- Scope: attestation / vote signature verification scaling (consensus CPU, claimed O(N) per block)
- Verdict headline: **The claimed per-block O(N) signature-verification cost does NOT exist on the production block-validate path. The BLS aggregate verify that appears wired is UNREACHABLE dead code. The only genuine O(N) signature cost is per-attestation Ed25519 verification on the gossip-receive path — off the block critical path.**
- Mode: analysis only. No code changed. Every claim below is cited to `file:line`. Where the premise drifted from code, it is corrected inline.

---

## 1. Verified Mechanism (file:line)

### 1.1 Attestation object — claims CONFIRMED
`crates/core/src/attestation.rs:14-34` — `struct Attestation` carries BOTH:
- `signature: Signature` — Ed25519 over `ATTESTATION_DOMAIN || block_hash || slot` (`attestation.rs:26,60,112-116`).
- `bls_signature: Vec<u8>` — BLS12-381 (96 B), present only when the attester has a BLS key (`attestation.rs:28-33`, produced at `:74-101`).
- `Attestation::verify()` (`attestation.rs:104-109`) verifies the **Ed25519** signature ONLY. It does not touch BLS.

### 1.2 Gossip topic — claim CONFIRMED
- `VOTES_TOPIC = "/doli/votes/1"` (`crates/network/src/gossip/mod.rs:30`); publisher `publish_vote` (`crates/network/src/gossip/publish.rs:42`).
- Receive: `behaviour_events.rs:385` matches `VOTES_TOPIC` → emits `NetworkEvent::NewVote` → handled by `on_new_attestation` (`bins/node/src/node/network_events.rs:536`). The network layer does NO signature check; it only rate-limits (`behaviour_events.rs:386-388`).

### 1.3 Bitfield encoder/decoder — index parity CONFIRMED
- Encode (produce): `assembly.rs:380-406` builds `attested_indices` as `[base_list positions | base_list.len()+extra positions]`, then `encode_attestation_bitfield_vec(indices, total_len)` (`crates/core/src/attestation.rs:283-295`). Pre-activation packs into `presence_root` via `encode_attestation_bitfield` (`attestation.rs:266-274`, 256-cap).
- Decode sites all use the SAME order and `decode_attestation_bitfield_vec` / `decode_attestation_bitfield` (`attestation.rs:304-316`, `:342`): post_commit liveness (`post_commit.rs:60-66`), reward attribution (`rewards.rs:139-145`), RPC stats (`rpc/methods/schedule.rs:305-311`), and the (dead) validate path (`registration.rs:274-284`). Body commitment `presence_root == BLAKE3(attestation_bitfield)` enforced at `validation_checks.rs:395-402`.

### 1.4 BLS aggregate primitive — CONFIRMED as a single aggregate pairing
`crypto::bls_verify_aggregate` (`crates/crypto/src/bls.rs:621-646`) calls blst `sig.fast_aggregate_verify(true, msg, ATTESTATION_DST, &pk_refs)`:
- ONE aggregate pairing check (Miller loop + final exp) — **O(1) in N**.
- O(N) G1 public-key aggregation inside blst.
- `pks_validate=true` (first arg) ⇒ O(N) subgroup checks **again**, on top of the O(N) `to_blst` decompression the caller already did (`registration.rs:296` + `bls.rs:632-636`). Redundant double-validation.

### 1.5 Producer side does attach a BLS aggregate
- Every produced block sets `aggregate_bls_signature = self.aggregate_bls_signatures(current_slot)` (`production/mod.rs:569-576`), built from minute-tracker BLS sigs via `crypto::bls_aggregate` (`assembly.rs:616-644`).
- Production loads a **real** BLS key derived from the producer secret (`bins/node/src/keys.rs:40-59`, `from_secret_key`). (The `BlsKeyPair::generate()` at `init.rs:1134` is inside `new_for_test`, not production.)
- So blocks CAN and DO carry non-empty `aggregate_bls_signature`, and it is gossiped and stored — **but never verified** (see §3).

---

## 2. STEP 1 — Cost Model f(N): the actual verify path and where CPU goes

### 2.1 The production block-validate/apply path
Node apply → `validate_block_for_apply` (`validation_checks.rs:~162`) → **`validation::validate_block_with_mode(block, &ctx, mode)`** (`validation_checks.rs:418`), called from `apply_block/mod.rs:110`.

`validate_block_with_mode` (`crates/core/src/validation/block.rs:190-~460`) has two branches (`Full`, `Light|Replay`). **Neither branch calls `validate_bls_aggregate`** (verified: no occurrence of `validate_bls_aggregate` in lines 190-460). It re-implements header/size/merkle/VDF/tx checks independently of the bare `validate_block()`.

Per-block O(N) work that ACTUALLY runs at apply (all integer/hash, no elliptic-curve ops):
| Op | Site | Order | Cost @N (blst-free) |
|----|------|-------|------|
| `active_producers_at_height` (build active set) | `validation_checks.rs:405` → `storage/src/producer/set_core.rs:320` | O(N) | ~ns per producer |
| BLAKE3(attestation_bitfield) commitment | `validation_checks.rs:396` | O(N/8) bytes | sub-µs |
| `validate_attestation_bitfield_vec` (stray-bit scan) | `validation_checks.rs:408` → `attestation.rs:319-336` | O(N) bits | sub-µs |
| `decode_attestation_bitfield_vec` (post_commit liveness) | `post_commit.rs:60-66` | O(N) | ~µs |

**No Ed25519 verify, no BLS verify, no pairing on this path.** Per-block signature-verification cost ≈ **0**.

### 2.2 The gossip-receive path (where the real O(N) signature cost lives)
`on_new_attestation` (`network_events.rs:536-570`): for each received attestation → `attestation.verify()` = **1 Ed25519 verify** (`network_events.rs:538`; also a second call site at `:332`). BLS is NOT verified — the raw BLS sig is only stored (`record_with_bls`, `:559-563`).

Each node receives ~N attestations per 10 s slot (one per active attester). ⇒ **~N Ed25519 verifies per slot per node**, O(N), but amortised across the 10 s slot window and OFF the block-apply critical path.

### 2.3 Epoch-boundary path (once per epoch)
`calculate_epoch_rewards` (`rewards.rs:71,128-157`): scans every block in the epoch, `decode_attestation_bitfield_vec` per block, unions attested minutes per producer index. O(N × blocks_per_epoch). Bit ops only; once per epoch.

### 2.4 Where the time goes (rough orders; blst micro-costs are `basis=assumed`, not measured on this HW)
Ed25519 verify ≈ 50 µs; BLS aggregate pairing ≈ 1–2 ms; BLS G1 decompress+subgroup ≈ 40–150 µs each; G1 add ≈ 1 µs.

| N | Block-apply sig cost | Block-apply bitfield/set cost | Gossip Ed25519 / slot / node |
|---|---|---|---|
| 33 (today) | 0 | ~µs | ~33 × 50 µs ≈ **1.7 ms/slot** |
| 300 | 0 | ~tens of µs | ~300 × 50 µs ≈ **15 ms/slot** |
| 3000 | 0 | ~hundreds of µs | ~3000 × 50 µs ≈ **150 ms/slot** |

Against a 10 s slot, even N=3000 gossip verify is ~1.5 % of the slot. The scaling wall at 1000s of producers is **gossip message volume / mesh bandwidth (N messages/slot fanned out)**, not verify CPU.

---

## 3. Honest Verdict — is Row 2 (O(N) ⇒ ~45 % of per-block budget @300) real?

**No. It is false as stated, and its stated mechanism does not exist.**

1. **There is no per-block signature verification on the production path.** `validate_block_with_mode` never verifies attestation signatures.
2. **`validate_bls_aggregate` is unreachable dead code.** Its ONLY caller is the bare `validate_block()` (`block.rs:104`, calls it at `:175`). Repo-wide, `validate_block()` has **zero call sites** — it exists only as a definition (`block.rs:104`) and two re-exports (`lib.rs:301`, `validation/mod.rs:55`). The node uses `validate_block_with_mode` exclusively.
3. **Even if reached, it could not verify:** `ctx.producer_bls_keys` is initialised to `Vec::new()` (`validation/types.rs:271`) and is **never populated** anywhere in the tree (no builder, no assignment — only the empty init and the read sites in `registration.rs:277-296`). With an empty key vector it would decode `producer_count = 0` → empty bitfield → `Err("bitfield is empty")` (`registration.rs:286-290`), OR short-circuit to `Ok` (`:302-306`). So the aggregate BLS signature that every producer attaches and gossips is **write-only consensus data — never cryptographically checked by any node.**

**True dominant O(N) term:** per-attestation **Ed25519 verification on the gossip-receive path** (`on_new_attestation`, `network_events.rs:538`), ~N × 50 µs per slot per node — off the block critical path. The single aggregate pairing is O(1) and is not even running.

**Crossover:** the block-apply attestation cost never "bites" — it is O(N) in cheap bit/hash ops (sub-ms at N=3000). The gossip Ed25519 cost is real O(N) but only becomes material (~150 ms/slot) near N=3000, and message-volume/bandwidth dominates before CPU does.

**Consequence for the redesign framing:** the premise ("make the expensive per-block attestation verify scale") is largely moot — that verify is not running. The real design questions are: (a) do we WANT cryptographically verified finality? then wire the already-correct O(1) aggregate onto the live path; (b) throughput to 1000s of producers is a gossip-fanout problem, not a verify-CPU problem.

### SSF recommendation (single)
If the goal is verified finality at scale, the simplest fix that resolves the root cause: **populate `ctx.producer_bls_keys` once per epoch from the `ProducerSet` and call `validate_bls_aggregate` from `validate_block_with_mode` behind a forward-only activation height, with `pks_validate=false` (keys are PoP+subgroup-checked at registration) and epoch-cached decompressed pubkeys.** This works because it turns write-only aggregate data into an O(1)-pairing + O(N)-cheap-add verified check, closing a live finality gap without adding a pairing-per-attester. (If instead the goal is pure throughput, do NOT touch verify — target gossip message volume.)

---

## 4. Capability Inventory (PRIOR-KNOWLEDGE-GATE — existing primitives)

| Primitive | Location | Status |
|-----------|----------|--------|
| BLS aggregate verify (1 pairing, `fast_aggregate_verify`) | `crypto/src/bls.rs:621-646` | Exists; not on live path |
| BLS single verify | `crypto/src/bls.rs:533-546` | Used for PoP-adjacent checks |
| BLS PoP verify (anti-rogue-key) | `crypto/src/bls.rs:574-586`; enforced at registration `registration.rs:322-338` | Live (registration path) |
| BLS aggregate (combine sigs) | `crypto/src/bls.rs:595-609` | Live (producer assembly) |
| BLS pubkey deserialize/subgroup (`to_blst`) | `crypto/src/bls.rs:156-157` (`BlstPublicKey::from_bytes`) | Live where called |
| Ed25519 verify (attestation) | `attestation.rs:104-109` via `signature::verify_with_domain` | Live (gossip receive) |
| Ed25519 **batch** verify | none found | **Absent** — potential Should |
| Aggregate/pubkey **cache** spanning gossip→validate | none found | **Absent** — no epoch-scoped decompressed-key cache |
| Bitfield encode/decode/validate | `attestation.rs:266-336` | Live (multiple decoders) |

Claim "system lacks aggregation" would be **false**: aggregation exists and is correct; it is merely unwired on the verify side.

---

## 5. Blast Radius (every attestation/bitfield touch point)

| Area | Function / site | file:line | Role |
|------|-----------------|-----------|------|
| Produce — self-attest | `attest_own_block`, `create_and_broadcast_attestation` | `assembly.rs:647-663` | Ed25519+BLS sign, record |
| Produce — bitfield encode | `encode_attestation_bitfield_vec` / `_bitfield` | `assembly.rs:399-406`; `attestation.rs:266-295` | Encoder (parity source of truth) |
| Produce — aggregate | `aggregate_bls_signatures`, `bls_aggregate` | `assembly.rs:616-644`; `bls.rs:595-609` | Build agg sig |
| Produce — attach | block assembly | `production/mod.rs:569-576` | Sets `aggregate_bls_signature`, `attestation_bitfield` |
| Gossip — publish | `publish_vote`, `VOTES_TOPIC` | `gossip/publish.rs:42`; `gossip/mod.rs:30` | Broadcast attestation |
| Gossip — receive | `behaviour_events.rs:385`; `on_new_attestation` | `network_events.rs:536-570` | **1 Ed25519 verify/att** (real O(N) cost) |
| Validate (DEAD) | `validate_bls_aggregate` ← `validate_block` | `registration.rs:262-316`; `block.rs:104,175` | Unreachable; would need `producer_bls_keys` |
| Validate (LIVE) | `validate_block_with_mode`; bitfield-commit + stray-bit | `block.rs:190`; `validation_checks.rs:395-415` | O(N) hash/bit only, no sig |
| Apply — liveness | `post_commit` decode (`[ATTEST_DECODE]`) | `post_commit.rs:60-71` | O(N) decode for exclusion |
| Epoch — rewards | `calculate_epoch_rewards` decode loop | `rewards.rs:71,128-157` | O(N×blocks) attribution |
| RPC — stats | `get_attestation_stats` decode | `rpc/methods/schedule.rs:213,305-311` | O(N) decode |
| Ctx plumbing (GAP) | `producer_bls_keys` field | `validation/types.rs:152,271` | Declared, never populated |

Decoders that MUST stay index-parity-locked with the encoder (`assembly.rs:380-406`): `post_commit.rs:61`, `rewards.rs:139`, `schedule.rs:306`, `registration.rs:275`. Any redesign touching bitfield order must update all four (Full Bitfield Decode pillar, CLAUDE.md).

---

## 6. Redesign Acceptance Criteria (MoSCoW)

| ID | Requirement | Priority | Acceptance criteria |
|----|-------------|----------|---------------------|
| REQ-ATT-001 | Bit-identical behavior for all reachable inputs unless gated by a forward-only activation height | Must | - [ ] No change to accepted/rejected block set below activation height<br>- [ ] Any verify-wiring is gated by a new `NetworkParams` activation height (never reuse a crossed one)<br>- [ ] No genesis reset; no `CURRENT_PROTOCOL_VERSION` bump unless `EpochState` format changes |
| REQ-ATT-002 | Bitfield encoder/decoder index parity across ALL decode sites | Must | - [ ] Encoder order `[base_list | extra sorted by pubkey]` preserved (`assembly.rs:380-406`)<br>- [ ] `post_commit.rs:61`, `rewards.rs:139`, `schedule.rs:306`, `registration.rs:275` decode identically<br>- [ ] Parity regression test asserts encode→decode round-trip at N∈{33,300,3000} |
| REQ-ATT-003 | x86/ARM determinism + snap-sync 3-state convergence unaffected | Must | - [ ] No floating point in the hot path<br>- [ ] blst results identical across arch (deterministic)<br>- [ ] ChainState/UtxoSet/ProducerSet roots converge post-change on mixed-arch fleet |
| REQ-ATT-004 | Correct the record: attestation verify is NOT a per-block cost today | Must | - [ ] Design docs state the block-apply path performs no sig verify<br>- [ ] `validate_bls_aggregate` classified as dead-or-to-wire, not "already protecting finality" |
| REQ-ATT-005 | If verified finality is desired, wire O(1) aggregate verify onto the live path | Should | - [ ] `producer_bls_keys` populated once/epoch from `ProducerSet`<br>- [ ] `validate_bls_aggregate` invoked from `validate_block_with_mode` behind activation height<br>- [ ] `pks_validate=false` (PoP-checked at registration); decompressed pubkeys epoch-cached<br>- [ ] Cost = 1 pairing + O(N) G1 adds per block; measured < 5 ms @N=3000 |
| REQ-ATT-006 | Non-foreclosure: still scale to 1000s of producers | Should | - [ ] Gossip fanout / message-volume analysis included (the true scaling wall)<br>- [ ] No design element is superlinear in N on any per-block path |
| REQ-ATT-007 | Remove redundant double subgroup validation | Could | - [ ] Prove PoP+registration subgroup check makes `pks_validate=true` at verify redundant, then drop it |
| REQ-ATT-008 | Drop a redundant signature field only if redundancy is proven | Could | - [ ] Prove Ed25519 OR BLS is redundant for a given path before removing either (both currently serve different paths: Ed25519=gossip, BLS=aggregate) |
| REQ-ATT-009 | Genesis reset / moving a crossed activation height / needless version bump | Won't | N/A — excluded per CLAUDE.md #0 |

### Brittleness / Trust-boundary note
Attestations are external, gossiped, untrusted input. The gossip-receive Ed25519 check (`network_events.rs:538`) is the current trust boundary and it is present. The **gap** is that BLS aggregate data crosses into blocks unverified (§3) — a security-relevant omission (unverified consensus artifact), so REQ-ATT-005 carries finality-security weight even though it is priced Should (it changes consensus and needs an activation-height rollout, not an emergency patch).
