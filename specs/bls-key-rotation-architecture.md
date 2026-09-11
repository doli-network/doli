# Architecture: Producer BLS Key Rotation

> Run 552 · `/omega-new-feature` · 2026-09-11 · Architect (parallel mode: synthesis of
> `docs/.workflow/architecture-perspective-{skeptic,explorer,analogist}.md`).
> Requirements: [bls-key-rotation-requirements.md](./bls-key-rotation-requirements.md) +
> [-security.md](./bls-key-rotation-requirements-security.md).
> Companion (failure modes, security model, gauntlet): [bls-key-rotation-architecture-security.md](./bls-key-rotation-architecture-security.md).
> Every `file:line` below was read in the worktree at base `1f08d975`.

## Stupid Simple First

**One new, last-declared, activation-gated `RotateBlsKey` transaction that spends one ordinary UTXO
(the spent input is the replay bind and the fee), carries the new BLS key plus an inner Ed25519
signature and a bound proof of possession in `extra_data`, and is queued as an epoch-deferred
`PendingProducerUpdate` that both apply implementations flush with one unconditional
`p.bls_pubkey = new` — an invalid payload is SKIPPED at apply, never a block rejection.**

*Non-foreclosure*: the ceiling is the epoch flush — O(rotations) 48-byte assigns — and the O(n)
uniqueness scan (≈20 µs at n = 1000, 25× cheaper than the one PoP pairing it guards). Growth path
past 10 000 producers is a reverse `bls_pubkey → producer` index rebuilt from `producers` on load;
nothing in the wire format or the state root has to move. Ed25519 rotation later = `-V2` domain
tags and a second payload, never a key-kind byte in the V1 preimage (analogist anti-pattern A1).

## Scope

In: `crates/core` (tx type, payload, preimages, stateless validation, network param, block rule),
`crates/crypto` (one DST + two functions in a new module), `crates/storage` (appended variant,
flush arm, admissibility resolver), `crates/mempool` (one filter module), `bins/node` (both apply
implementations, undo predicate, rollback pool clear, metric, logs), `crates/rpc` (two additive
optional fields on `PendingUpdateInfo`), `crates/wallet` + `bins/cli` (`import-bls`, `rotate-bls`),
docs. Out (binding): pinning any height, `MIN_PEER_PROTOCOL_VERSION`, any version bump,
`HardForkSchedule`, canonical encodings, INC-I-169 F1, GUI.

## Design Space Analysis

| Gate | Finding |
|---|---|
| Existing patterns | (1) new frozen `TxType` at `u64::MAX` (`PriceAttestation=16`, `ZKSettle=31`); (2) `extra_data`-authenticated deferred producer mutation that SKIPS on failure (`apply_block/tx_processing.rs:475-535`, DelegateBond); (3) parity module = thin wrapper calling ONE core predicate (`crates/mempool/src/vesting_bound.rs:1-30`); (4) hardened signing message = private `-V1` domain, `genesis_hash` first, fixed-width tail, published preimage (`crates/core/src/maintainer/authmsg.rs:89-153`); (5) epoch-boundary scratch reset `parent_sig_pool.clear()` at `apply_block/post_commit.rs:421` under `is_epoch_boundary_with` (`:194`) |
| Stack constraints | bincode variant index = declaration ordinal (`block.rs:246-248`); `signing_message` hashes the discriminant (`core.rs:522`); `serialize_canonical(&self)` has no height parameter (`set_persistence.rs:78-113`) so **no new `ProducerInfo` field, ever**; `pending_updates` is bincode-persisted and snap-synced (`producer/types.rs:240`, `network/src/sync/manager/types.rs:190`) but **not** rooted |
| Performance reqs | Law 3: 1000s of producers, 10 s slots. Budget below. |
| Dependency direction | core ← storage ← mempool ← node; rpc/cli read core+storage. No new edge. The new node→pool clear in `rollback.rs` uses an existing Node field. |
| Anti-overengineering | SSF above. Subtractions taken: no `expiry_height`, no `ROTATION_TX_MAX_LIFETIME`, no reject-block rule, no 6-site stateful parity, no BLS-key mempool snapshot, no reaper arm, no `pendingBlsRotation` field, no `lastRotationHeight`, no `wire_ordinal()` accessor, no wallet `TxType` entry, no `--bls-key-file` node flag, no "Exit-pending blocks rotation" rule, no per-block rotation cap. |
| Forced prediction | PREDICTION: `post_commit.rs` clears the attestation pool at the boundary → deferral gets the pool flush free. CONFIRMED (`:194`, `:421`). IF WRONG the design would have needed a per-rotator eviction API on `ParentSignaturePool` (`crates/core/src/attestation/pool.rs:21-100` has only `clear`). |

```
━━━ DESIGN SPACE GATE QUALITY AUDIT ━━━
Existing patterns identified:      5
Tech stack constraints documented: 4
Performance requirements present:  yes (Law 3; numbers in Performance Budget)
Anti-overengineering gate:         PASS (12 subtractions listed)
Forced predictions made:           1 (confirmed)
Constraint table entries:          8 (companion §Constraint Table)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Overview — the eight decisions

| # | Decision | Verdict | Evidence pointers |
|---|---|---|---|
| D1 | Wire number | **`RotateBlsKey = 32`, declared last** → ordinal 24 / discriminant 32; tombstone 23 kept; `tests.rs:74` unchanged; wallet enum untouched (CLI serialises via core `Transaction`) | skeptic A9 (`conf 0.7 measured`); `types.rs:139,143-175`; `tests.rs:74`; wallet `builder.rs:107,174` |
| D2 | Fee / replay | **1-in/1-out fee-paying; the spent outpoint is the replay bind AND is inside the inner signature**; `expiry_height` DELETED; `allows_empty_io => false` | explorer A4; `validation/utxo.rs:126-139` (spent-input check is the oldest rule); skeptic A1/A3 eliminate the window |
| D3 | Invalid at apply | **SKIP** (DelegateBond D4 precedent), logged + counted; below the AH the tx type itself makes the block invalid | analogist D4 (`tx_processing.rs:488-529`); skeptic A2/A3 |
| D4 | Timing | **Epoch-deferred**; flush arm unconditional on status; convergence test asserts the queue and spans the boundary | CLAUDE.md law; skeptic A5/A6; `post_commit.rs:421` |
| D5 | Pool poison | Boundary flush and `parent_sig_pool.clear()` share one predicate (`is_epoch_boundary_with`) — pinned by a test; **plus** clear the pool whenever a rollback restores a `ProducerSet` | `post_commit.rs:194,421`; `state_update.rs:181`; `rollback.rs:178-191`; `ingress.rs:63-90`; `keys.rs:13-28` |
| D6 | Un-upgraded nodes | Below `bls_key_rotation_activation_height` (= `u64::MAX` everywhere) a block containing the type is **invalid**, so no honest node ever gossips one pre-pin; post-pin protection is the pinning session's job (census + `MIN_PEER_PROTOCOL_VERSION`, needs user approval) | skeptic A4; `gossip/validation.rs:96-105`; `behaviour_events.rs:70-74` |
| D7 | Signing message | `authmsg.rs` disciplines verbatim; two private `-V1` tags; ONE published preimage encoder used by node and CLI; PoP bound to the Ed25519 pubkey and genesis | analogist D1/P3; `authmsg.rs:89-153` |
| D8 | Uniqueness | O(n) scan over `producers` values + queued rotations, in storage, called by apply and rebuild only; no reverse index | explorer §(4); `set_core.rs:25` |

Also: observability = two additive fields on the existing `PendingUpdateInfo` mapper (no new RPC
field), `[BLS_ROTATE]` log tokens, one `IntCounter` with a wiring test; operator path =
`doli wallet import-bls` as its own early milestone (M2); node `--bls-key-file` NOT added.

## Modules

### Module 1 — `crates/core/src/transaction/rotate_bls.rs` (new, ≈150 lines)

`RotateBlsKeyData { producer: PublicKey, new_bls_pubkey: [u8;48], bls_pop: [u8;96], signature: [u8;64] }`
— **240 bytes**, fixed layout `producer(32) || new_bls_pubkey(48) || bls_pop(96) || signature(64)`;
`from_bytes` returns `None` for any other length. Owns the two preimages (below), the private
domain constants, `GOLDEN_ROTATE_PREIMAGE_HEX`, and `Transaction::rotate_bls_data()`.
Interface: `to_bytes/from_bytes`, `signing_message_preimage(genesis_hash, new_bls_pubkey, outpoint)`,
`signing_message(..) -> [u8;32]` (= BLAKE3 of the returned preimage vector, never re-streamed),
`pop_message(genesis_hash, producer, new_bls_pubkey) -> Vec<u8>`.

### Module 2 — `crates/crypto/src/bls_rotation.rs` (new, ≈60 lines)

`const ROTATE_POP_DST: &[u8] = b"DOLI-ROTATE-POP-V1"` (private, distinct from `ATTESTATION_DST`
`bls.rs:60` and `POP_DST` `bls.rs:66`); `sign_rotation_pop(sk, msg) -> [u8;96]`,
`verify_rotation_pop(pk, msg, sig) -> bool` built on the existing primitives. `bls.rs` (1079 lines)
is not edited.

### Module 3 — `crates/core/src/validation/rotate_bls.rs` (new, ≈120 lines) — the ONE predicate

```
pub enum RotateReason { BelowActivation, Shape, Length, BadPoint, BadPop, BadSig,
                        NotProducer, KeyInUse, AlreadyPending, SameKey }
pub fn rotate_stateless(tx, ctx) -> Result<RotateBlsKeyData, ValidationError>
    // gate(height ≥ AH) → 1 input / 1 Normal output → len 240 → G1 decode+subgroup+non-identity
    // → inner Ed25519 over signing_message(genesis, new_key, tx.inputs[0].outpoint) by payload.producer
    // → PoP under ROTATE_POP_DST over pop_message(genesis, producer, new_key)
pub fn rotate_verdict(data: &RotateBlsKeyData, inputs: &RotationInputs) -> Result<(), RotateReason>
    // NotProducer → SameKey → AlreadyPending → KeyInUse   (cheap state checks; no crypto)
```
`RotationInputs { sender_registered: bool, current_bls: Option<Vec<u8>>, key_in_use: bool,
already_pending: bool }` is resolved ONCE by the caller (storage Module 4) — the `vesting_bound.rs`
shape. Order of evaluation is a wire contract: state checks first so a colluding block cannot make
validators pay pairings for producers that will be skipped anyway.

### Module 4 — `crates/storage/src/producer/rotation.rs` (new, ≈80 lines) + 1 appended variant

`PendingProducerUpdate::RotateBlsKey { pubkey: PublicKey, new_bls_pubkey: Vec<u8>, height: u64 }`
appended AFTER `RequestWithdrawal` (`producer/types.rs:178-210`). `ProducerSet` gains
`resolve_rotation_inputs(&self, data) -> RotationInputs` (scans `producers.values()` and
`pending_updates` for `RotateBlsKey` targets), `pending_rotations(&self) -> Vec<(PublicKey, Vec<u8>)>`,
and the flush arm in `apply_pending_updates_with_cap` (`set_core.rs:106-177`):
```rust
PendingProducerUpdate::RotateBlsKey { pubkey, new_bls_pubkey, height } => {
    if let Some(p) = self.get_by_pubkey_mut(&pubkey) {
        tracing::info!("[BLS_ROTATE] applied producer={:.8} old={:.8} new={:.8} h={}", ..);
        p.bls_pubkey = new_bls_pubkey;
    }
}
```
No `status` branch, no `warn!` — nothing ever removes a producer from the map (0 hits for
`producers.remove` in `crates/storage/src/producer/`), so apply and rebuild reach this arm with the
same map regardless of the `[Exit, Rotate]` vs `[RequestWithdrawal, Rotate]` queue asymmetry
(`rewards.rs:1297-1310`). The three exhaustive matches (`set_core.rs:113-174,193-201,236-242`) are
compile-forced. `pending_updates_for` (`:190`) already serves the RPC.

### Module 5 — apply #1 and #2 (`apply_block/tx_processing.rs`, `rewards.rs:1258`)

Both arms are the same six lines:
```rust
TxType::RotateBlsKey => if let Some(d) = tx.rotate_bls_data() {
    let inputs = producers.resolve_rotation_inputs(&d);
    match rotate_verdict(&d, &inputs) {
        Ok(()) => { info!("[BLS_ROTATE] queued producer={:.8} new={:.8} h={}", ..);
                    producers.queue_update(PendingProducerUpdate::RotateBlsKey { .. height }) }
        Err(r) => warn!("[BLS_ROTATE] skipped producer={:.8} reason={:?} h={}", ..),
    }
}
```
The rebuild arm consults only `(tx, producers-as-rebuilt)` — the purity rule skeptic A1 asked for.
`rotate_stateless` is NOT re-run at apply for already-mined blocks in rebuild: it ran at block
validation (below) and is a pure function of `(tx, genesis, height, AH)`; the rebuild walks accepted
blocks only. `helpers.rs:15-28` gains `| TxType::RotateBlsKey` (undo snapshot non-empty).
`state_update.rs:202`: read `producers.pending_rotations().len()` before the flush and
`BLS_ROTATIONS_APPLIED.inc_by(n)` after it.

### Module 6 — block rule + mempool/builder filter

`validation/block.rs`: a block containing `TxType::RotateBlsKey` at `height < AH` is **invalid**
(`[ERRTX-ROT002]`). Above the AH, block-level consensus for the tx is ONLY the structural part of
`rotate_stateless`: exactly 1 input, exactly 1 Normal output, `extra_data.len() == 240`
(`[ERRTX-ROT001]`), plus the existing input-signature / spent-input / fee rules every 1-in/1-out tx
already obeys. The cryptographic and stateful parts (point, PoP, inner signature, membership,
uniqueness, one-pending) are NOT block-invalidating — see D3: such a tx is a valid value transfer
whose payload is skipped at apply. `validation/transaction.rs:305` dispatch gains the arm so the
generic validator never sees a rotation without the shape check.
`crates/mempool/src/rotation_filter.rs` (new, ≈50 lines): `rotation_admissible(tx, ctx) -> Result<(), String>`
= `rotate_stateless` (full crypto, so the mempool never relays garbage) — no state, no snapshot, no
reaper arm (no height-dependent validity exists; a spent input already evicts via `pool.rs:1372-1380`).
The builder (`assembly.rs:200`) calls the same function to exclude, as it does for `vesting_bound`.

### Module 7 — RPC, metrics, CLI, wallet

`PendingUpdateInfo` (`rpc/types/producer.rs:121-127`) += `new_bls_pubkey: Option<String>`,
`effective_at_height: Option<u64>` (both `skip_serializing_if`, derived: next boundary from
`chain_state.best_height`); mapper `rpc/methods/producer.rs:11-33` += one arm → visible in
`getProducer` **and** `getProducers` (`:196`) — the CLI must read `getProducers` (`register.rs:29-37`).
`bins/node/src/metrics.rs`: `BLS_ROTATIONS_APPLIED: IntCounter` (`doli_producer_bls_rotation_total`)
registered in `REGISTRY`, with a test that applies a boundary and asserts the delta (INC-I-187).
`crates/wallet/src/wallet.rs`: `import_bls_key(secret_hex, force)` beside `add_bls_key` (`:401-414`)
— sign→verify round-trip, prints the derived 48-byte key, `--force` to overwrite.
`bins/cli/src/cmd_producer/rotate.rs` (new): reads `getProducers`, refuses if wallet key == on-chain
key, if no wallet BLS key, or if a rotation is already pending; selects one Normal UTXO; builds a core
`Transaction { tx_type: RotateBlsKey, inputs: [utxo], outputs: [change] }` via core `serialize`;
signs the inner message through Module 1's encoder (never a second encoder); prints old key, new key,
fee, "effective at height H (next epoch boundary); attestations for the boundary block itself may be
lost; cannot be undone except by another rotation"; requires consent (`--yes`).

## Data Flow

```
CLI rotate-bls ─(getProducers: on-chain key, pending list)─► build tx: 1 input (fee+bind) │ 1 change output │ extra_data 240 B
      │ inner sig = Ed25519(producer_sk, BLAKE3(DOMAIN‖genesis‖new_key‖prev_tx_hash‖idx))
      │ pop       = BLS_sign(new_sk, ROTATE_POP_DST, genesis‖producer_pk‖new_key)
      ▼
mempool add_transaction ── rotation_admissible = rotate_stateless(tx, ctx) ── refuse [ERRTX-ROT0xx] / admit
      ▼
builder assembly.rs ── same function ── exclude / include
      ▼
block validation ── height ≥ AH else INVALID; 1-in/1-out; len 240; existing input-signature + spent-input checks
      ▼
apply_block tx_processing ── resolve_rotation_inputs → rotate_verdict → queue RotateBlsKey | skip (log, no state)
      │  undo: block_mutates_producer_set == true → full ProducerSet snapshot (pending_updates inside, unrooted)
      ▼
epoch boundary (state_update.rs:181 is_boundary) ── flush: p.bls_pubkey = new (rooted NOW) ── metric += n
      ▼
post_commit.rs:194/421 (same predicate) ── parent_sig_pool.clear() ── halves pooled under the OLD key are gone
      ▼
next blocks: ingress.rs:67 verifies halves against NEW key; keys.rs:13-28 aggregates with NEW key; bit set; qualifies
rebuild (rewards.rs:1258) replays the identical arm; rollback restores the snapshot and clears the pool.
```

## Preimage byte layouts (wire contracts; a field-list change bumps `-V1` → `-V2`)

```
INNER (Ed25519, verified WITH payload.producer as the key — data.rs:288-291 precedent):
  "DOLI-ROTATE-BLS-V1" ‖ genesis_hash (variable, FIRST) ‖ new_bls_pubkey[48] ‖ prev_tx_hash[32] ‖ output_index u32 LE
  FIXED_TAIL_LEN = 48 + 32 + 4 = 84 ; digest = BLAKE3-256(preimage) ; no length prefixes
POP (BLS under private DST "DOLI-ROTATE-POP-V1"):
  genesis_hash ‖ producer_ed25519[32] ‖ new_bls_pubkey[48]
```
Why the outpoint is inside the inner signature: without it a third party could re-wrap the
`(producer, new_key, pop, sig)` tuple around a fresh input of their own and re-apply an old A→B
authorisation after the producer moved to C. With it, the authorisation is single-use by construction
because the outpoint can be spent once (`validation/utxo.rs:126-139`). This is what makes both the
360-block window and the A→B→A concern (analogist P2) disappear rather than merely shrink.
Collision audit: no other DOLI preimage starts with either tag (`authmsg.rs:99`, `data.rs:769-785`).

## Design Decision Hypotheses

**D1 wire number** — `conf(0.9, measured)`. Alternatives: `=23` ELIMINATED (tombstone `tests.rs:74`,
skill `core/SKILL.md:672`, wallet table Fact 3); `=24` ELIMINATED (discriminant 24 is tombstoned B.1
and `serialization_compat.rs:425` requires wire id ∈ core `from_u32`); `=32` SURVIVES: above every
discriminant, ordinal 24, no table can alias. Tests on both surfaces: a golden table
`[(variant, bincode ordinal, `as u32`)]` for all 25 variants (`ZKSettle → (23, 31)`,
`RotateBlsKey → (24, 32)`); `from_u32(23)==None` stays; `from_u32(32)==Some`; wallet
`serialization_compat.rs:364` count stays 24 (the wallet enum gets **no** entry — the CLI uses core).

**D2 fee/replay** — `conf(0.8, observed)`. Alternatives: (a) 0-in/0-out + expiry (Analyst) —
WEAKENED to ELIMINATED by skeptic A1 (1-block margin) + A3 (free × height-dependent × reject);
(b) CAS on current key (explorer A3) — ELIMINATED: requires immediate mutation (D4) and still has the
cycle; (c) anchor hash (A5) — ELIMINATED: block-store access inside validation, INC-I-147 shape;
(d) UTXO consumption (A4) — SURVIVES and, with the outpoint inside the signature, closes the
third-party re-wrap that A4 alone leaves open. Cost: the producer needs one spendable UTXO (one RPC
call to check; any wallet can fund it). `utxo.rs:222-227` legacy expression untouched (INV-COMPAT-001
trivially true). Simplicity tiebreak: deletes a constant, a payload field, a rejection reason and a
reaper arm.

**D3 skip** — `conf(0.75, observed)`. Reject-the-block (Analyst SEC-008) is ELIMINATED by A3's
mechanism even without expiry: uniqueness and one-pending are state races across gossip latency
(explorer §(4)), so a rejecting rule needs six bit-identical stateful sites (`pool.rs:512,928`,
`assembly.rs:211`, `validation_checks/mod.rs:106,292`, `tx_processing.rs:61`) = 18 fail-open
wirings. With skip, the ONLY consensus-critical sites are the two apply arms (already mandatory), a
mempool/builder omission costs bytes not liveness, and the intra-block duplicate is handled by
sequential apply (second sees `already_pending`). Why D4 (DelegateBond skip) applies and the
Registration reject does not: Registration creates Bond UTXOs that would be orphaned by a skip;
rotation creates none — its change output is an ordinary UTXO valid regardless of the payload. Why
the fee makes skip safe against spam: the sender pays for a no-op, and a non-producer's payload is
skipped at the first (µs) state check before any pairing. Below the AH the type itself is illegal
(block invalid) — bit-identical with today's "undecodable" verdict on every reachable input.

**D4 deferred** — `conf(0.8, observed)`. Immediate (explorer A2) SURVIVED the skeptic on
correctness (`bls_pubkey` is not a universe input, `commit.rs:32-79`) but is ELIMINATED here by D5:
the existing boundary `parent_sig_pool.clear()` is the pool-poison remedy, and immediate mutation
would need a new eviction API + a switch height at "any block". The one-epoch latency is bounded
(360 blocks) for producers already at zero for 37 epochs. A5 resolved: REQ-ROT-006's test asserts
`pending_update_count()` and the absence of any `RotateBlsKey` for the producer after rollback, then
runs to the next boundary and compares `serialize_canonical()` across apply / rebuild / disk-reload —
the root at `h-1` alone is NOT load-bearing. A6 resolved by the unconditional arm; REQ-ROT-008's
"no-op with a `warn!`" clause is deleted.

**D5 pool** — `conf(0.75, observed)`. Poison path: a half for the boundary block B that arrives
BEFORE B is applied is verified under the OLD key (`ingress.rs:63-90`) and pooled; B applies (flush
NEW); a builder aggregates it; validators verify under NEW (`keys.rs:13-28`) → whole-block aggregate
failure. Closed because `post_commit.rs:421` runs `clear()` in the same commit, under the same
predicate as the flush. Pinned by INV-ATTEST-003 (proposed): "after the commit of any block that
flushes a `RotateBlsKey`, `parent_sig_pool` holds no entry for that attester". Reorg residue: a
rollback across the flush restores the OLD key while halves verified under NEW may remain →
`rollback.rs:184-186` restore branch and the `:189` rebuild branch both call
`self.parent_sig_pool.clear()` (one line each; a reorg already disrupts attestation). Epoch-0 note:
the flush runs every block in epoch 0 (`state_update.rs:180`) but the clear only at boundaries — no
network is in epoch 0 and the AH is `u64::MAX`; tests must place the AH ≥ `blocks_per_epoch`.

**D6 un-upgraded** — `conf(0.7, observed)`. Pre-pin: no valid block can carry the type, so the P4
cascade (`gossip/validation.rs:96-105` → `behaviour_events.rs:70-74`) is unreachable; a malicious
upgraded builder's block is rejected by everyone (old: undecodable; new: rule) and only the builder
is scored. Post-pin (out of scope): the pinning session MUST (i) bump `MIN_PEER_PROTOCOL_VERSION`
(`crates/network/src/protocols/status.rs`) in the same release so stragglers partition at handshake
— **user approval required, not in this change**; (ii) record the SEC-010 census; (iii) run GS-022
(companion); (iv) note that snap-sync payloads with the new variant are equally undecodable by old
nodes (`producer/types.rs:240`), and that no `TxType` has ever been activated post-genesis on mainnet.

**D7 preimage** — `conf(0.85, measured)`: term-for-term the shape DOLI already hardened in
`authmsg.rs:132-146`. **D8 uniqueness** — `conf(0.8, inferred)`: reverse index rejected as a second
source of truth that must be rebuilt identically on snap-sync, restart and rebuild.

```
━━━ DESIGN DECISION QUALITY AUDIT ━━━
Major decisions identified:            8
Alternatives per decision (avg):       3.4 (from the three perspective reports; no re-exploration)
  basis=measured:                      2   basis=observed: 5   basis=inferred: 1   basis=assumed: 0
Confidence range for winner:           0.70–0.90
Decisions with flat distribution:      0
Decisions with conf >= 0.8 + assumed:  0
Constraint table entries used:         8
━━━ SIMPLICITY AUDIT ━━━
Subtraction alternatives explored:     12 (listed in Design Space Analysis)
"Do nothing" alternatives explored:    1 (explorer A1 — kept as M2, not as the answer: 2 lost-key cases)
Winner complexity cost:                5 new source files (rotate_bls.rs ×2, bls_rotation.rs, producer/rotation.rs, rotation_filter.rs), 1 CLI file, 1 appended enum variant, 1 network param
Simpler alternative that was close:    explorer A2+A3 (immediate + CAS), conf 0.6 — loses on D5
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Performance Budgets

| Operation | Cost | Basis |
|---|---|---|
| Block with no rotation | +0 (one `matches!` arm, one dispatch arm) | observed |
| Per rotation tx, validators | input Ed25519 ≈55 µs (existing) + inner Ed25519 ≈55 µs + G1 decode/subgroup ≈100 µs + PoP pairing ≈493 µs (`ingress.rs:141` in-tree figure) + state scan ≈20 µs @ n=1000 ≈ **0.75 ms** | inferred |
| Worst block | bounded by distinct registered producers (state checks skip the rest before crypto): 93 × 0.75 ms = 70 ms today; 1000 → 0.75 s of a 10 s slot, each paying a fee | inferred |
| Epoch flush | +O(rotations) 48-byte assigns; ≤ 1 per producer per epoch | observed |
| Undo snapshot | a rotation block writes a full ProducerSet snapshot (~300 KB @ n=1000) — same as any Registration block | observed |
| RPC | +≤120 B per pending rotation in `pendingUpdates`; absent otherwise | observed |
| Mempool | no new snapshot; stateless verify ≈0.7 ms per candidate, priced by fee | inferred |

```
━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      +0.75ms per rotation tx per validator; +0 for blocks without one (inferred)
  Memory:   +~120B per queued rotation in pending_updates; +0 steady state (observed)
  IO:       +1 full ProducerSet undo snapshot per rotation-carrying block, ~300KB at n=1000, same class as a Registration block (observed)
  Network:  +~400B block bytes per rotation (1 input + 1 output + 240B extra_data); +≤120B per pending rotation in getProducers (observed)
  Disk:     +~400B per rotation in the block store, forever; no new column family (observed)
  Latency:  +0 on the block path without rotations; boundary flush +O(rotations) assigns (observed)
Inevitability: AVOIDABLE
Cheaper alternative: operator-side key restore only (explorer A1, M2 here) — zero consensus cost but covers 33 of 35 exposed producers and none of the 2 live lost-key cases or a leaked key
Why this proposal anyway: the two live producers earning 0 have no client-side remedy (seed-derivable key lost, census 2026-09-11) and a leaked BLS key has no remedy at all without on-chain rotation
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```
Dev cost: 5 new source files + 1 CLI file + 1 docs file; ≈14 edited files; ≈18 new tests (golden
vectors ×3, RED repro, parity, undo, rebuild, boundary convergence, pool invariant, metric wiring,
CLI encoder golden, import-bls round-trip).

## Consensus-shape checklist (INC-I-075) and deploy questions

(1) user-submittable tx reaches this path: **YES**. (2) producer-action/attestation pattern reaches
it: **YES** (attestation verification reads the rotated key). (3) bit-identical for all reachable
inputs: **NO above the AH** ⇒ activation height REQUIRED — `bls_key_rotation_activation_height`,
`u64::MAX` on mainnet, testnet, devnet; below it the verdict on a block carrying ordinal 24 is
"invalid" on the new binary and "undecodable" on the old — the same verdict. Consensus RULES change:
**YES** (AH). Block CONTENT change: **YES** (new tx type, pending-update variant in snap-sync
payloads) ⇒ **synchronized deploy at pin time**, not at merge. No `HardForkSchedule` entry; no
`CURRENT_PROTOCOL_VERSION` / `EPOCH_STATE_FORMAT_VERSION` change (INV-4). No genesis reset (#0 rule).

## Milestones

| ID | Name | Scope (Modules) | Scope (Requirements) | Dependencies |
|----|------|-----------------|----------------------|--------------|
| M1 | Reproduction test (RED) | `bins/node/tests/bls_rotation_repro.rs` (new), `bins/node/src/lib.rs` (test access only if needed) | REQ-ROT-010 | — |
| M2 | Operator-side restore (no consensus) | `crates/wallet/src/wallet.rs`, `crates/wallet/tests/`, `bins/cli/src/cmd_wallet_bls.rs` (new), `bins/cli/src/main.rs` dispatch | REQ-ROT-014 | — |
| M3 | Wire freeze + `types.rs` split | `crates/core/src/transaction/tests.rs` (golden ordinal+discriminant table), `crates/core/src/transaction/input.rs` (new: `Input` moved out, re-exported), `crates/core/src/transaction/types.rs`, `crates/core/src/transaction/mod.rs`, `crates/storage/src/producer/tests.rs` (PendingProducerUpdate ×7 + `serialize_canonical` golden) | REQ-ROT-001 | M1 |
| M4 | New TxType + payload + preimages | `crates/core/src/transaction/types.rs`, `crates/core/src/transaction/rotate_bls.rs` (new), `crates/core/src/transaction/mod.rs`, `crates/crypto/src/bls_rotation.rs` (new), `crates/crypto/src/lib.rs`, `crates/wallet/tests/serialization_compat.rs` | REQ-ROT-001, REQ-ROT-002, REQ-ROT-003, REQ-ROT-SEC-001, REQ-ROT-SEC-002, REQ-ROT-SEC-009 | M3 |
| M5 | Activation gate + block rule + stateless verdict | `crates/core/src/network_params/defaults.rs`, `crates/core/src/network_params/mod.rs`, `crates/core/src/validation/rotate_bls.rs` (new), `crates/core/src/validation/transaction.rs`, `crates/core/src/validation/block.rs`, `crates/core/src/validation/types.rs`, `crates/core/src/validation/mod.rs` | REQ-ROT-004, REQ-ROT-SEC-003, REQ-ROT-SEC-007, REQ-ROT-SEC-008 | M4 |
| M6 | Mempool + builder filter | `crates/mempool/src/rotation_filter.rs` (new), `crates/mempool/src/lib.rs`, `crates/mempool/src/pool.rs`, `bins/node/src/node/production/assembly.rs`, `crates/mempool/tests/rotation_parity.rs` (new) | REQ-ROT-009 | M5 |
| M7 | Apply #1 + storage + undo | `crates/storage/src/producer/types.rs`, `crates/storage/src/producer/rotation.rs` (new), `crates/storage/src/producer/set_core.rs`, `crates/storage/src/producer/mod.rs`, `bins/node/src/node/apply_block/tx_processing.rs`, `bins/node/src/node/apply_block/helpers.rs`, `bins/node/src/node/apply_block/state_update.rs` | REQ-ROT-005, REQ-ROT-006, REQ-ROT-008, REQ-ROT-SEC-004, REQ-ROT-SEC-005, REQ-ROT-SEC-006 | M6 |
| M8 | Apply #2 + reorg + pool invariant | `bins/node/src/node/rewards.rs`, `bins/node/src/node/rollback.rs`, `bins/node/tests/bls_rotation_convergence.rs` (new), `bins/node/tests/fork_recovery.rs` | REQ-ROT-007, REQ-ROT-011 | M7 |
| M9 | Observability | `crates/rpc/src/types/producer.rs`, `crates/rpc/src/methods/producer.rs`, `bins/node/src/metrics.rs`, `bins/node/src/node/apply_block/state_update.rs`, `bins/node/tests/bls_rotation_metrics.rs` (new) | REQ-ROT-012 | M8 |
| M10 | CLI rotate-bls | `bins/cli/src/cmd_producer/rotate.rs` (new), `bins/cli/src/cmd_producer/mod.rs`, `bins/cli/src/cmd_producer/dispatch.rs`, `bins/cli/tests/rotate_bls_golden.rs` (new) | REQ-ROT-013 | M9 |
| M11 | Docs + specs + gauntlet registration | `specs/protocol.md`, `specs/attestation-bls-architecture.md`, `docs/cli.md`, `docs/rpc_reference.md`, `docs/becoming_a_producer.md`, `docs/bls-key-recovery.md`, `docs/bugfixes/inc-i-162-wallet-bls-derivation-analysis.md`, `scripts/gauntlet-seed.sql` | REQ-ROT-015, REQ-ROT-016, REQ-ROT-SEC-010 | M10 |

M2 is moved ahead of the consensus chain (Analyst had it in M9) because it is the only remedy
available before pinning and has no consensus surface (Q7, explorer signal 4). M3 is the split
skeptic A10 demanded, with the golden vectors landing BEFORE the move so the move is provably
byte-neutral. CLI landing-zone collision with `feat/bond-lock-disclosure-cli` (condition C1) is
deferred to M10 by this ordering.

## Requirement Traceability

| Requirement | Architecture section | Milestone | Note |
|---|---|---|---|
| REQ-ROT-001 | D1; Module 1 | M3, M4 | discriminant **32** (supersedes "23" in the requirements) |
| REQ-ROT-002 | Preimage layouts; Module 1 | M4 | payload **240 B** (no `expiry_height`) |
| REQ-ROT-003 | D2; Module 6 | M4, M5 | `allows_empty_io => false`; **1-in/1-out** supersedes 0-in/0-out |
| REQ-ROT-004 | D6; checklist | M5 | |
| REQ-ROT-005 | D4; Module 4 | M7 | |
| REQ-ROT-006 | D4 (A5 criteria); Module 5 | M7, M8 | queue assertion + boundary root, negative control on the queue |
| REQ-ROT-007 | Module 5; D4 | M8 | |
| REQ-ROT-008 | Module 4 (unconditional arm) | M7 | "no-op with `warn!`" clause deleted |
| REQ-ROT-009 | D3; Module 6 | M6 | restated: same verdict for the same `(tx, ctx)`; consensus-critical parity = the two apply arms |
| REQ-ROT-010 | Data Flow | M1 | |
| REQ-ROT-011 | D5; Module 5 | M8 | + boundary-straddle loss documented |
| REQ-ROT-012 | Module 7 | M9 | `PendingUpdateInfo` fields, not `pendingBlsRotation`; "closes the detection gap" claim dropped (skeptic A8) |
| REQ-ROT-013 | Module 7 | M10 | discloses fee + effective height |
| REQ-ROT-014 | Module 7 | M2 | Finding C round-trip |
| REQ-ROT-015, 016 | companion | M11 | |
| REQ-ROT-021 | — | — | **Won't**: no data source (skeptic A8) |
| REQ-ROT-SEC-001 | D7; Module 1 | M4 | preimage now binds the spent outpoint instead of `expiry_height` |
| REQ-ROT-SEC-002 | D7; Module 2 | M4 | |
| REQ-ROT-SEC-003 | D2 | M5 | **superseded**: replay bind = consumed input inside the signature |
| REQ-ROT-SEC-004 | D8; Module 4 | M7 | |
| REQ-ROT-SEC-005 | Module 4/5 | M7 | enforced at apply (skip); mempool/builder hygiene via CLI pre-check |
| REQ-ROT-SEC-006 | Module 3/4 | M7 | |
| REQ-ROT-SEC-007 | Module 3 | M5 | |
| REQ-ROT-SEC-008 | D3 | M5 | **redefined**: below-AH and shape → block invalid; payload failures → skip |
| REQ-ROT-SEC-009 | D7 | M4 | |
| REQ-ROT-SEC-010 | D6; companion | M11 | pinning precondition, plus `MIN_PEER_PROTOCOL_VERSION` (user decision) |

## What I don't understand (disclosure)

1. I did not read `crates/core/src/validation/utxo.rs` around `allows_empty_io => false` to confirm
   "exactly 1 output" is enforceable there rather than in the new dispatch arm; Module 6 places the
   shape rule in the rotation arm regardless, so the design does not depend on it.
2. The ~100 µs G1 subgroup-check figure is inferred from blst behaviour, not measured in-tree.
3. Whether the two affected operators hold a spendable Normal UTXO is unmeasured (explorer gap); the
   CLI must print the exact remedy ("send ≥ fee + dust to <address>") when none is found.
