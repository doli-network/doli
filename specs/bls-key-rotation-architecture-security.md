# BLS Key Rotation — Failure Modes, Security Model, Constraint Table, Gauntlet

> Companion to [bls-key-rotation-architecture.md](./bls-key-rotation-architecture.md) (run 552,
> 2026-09-11). Split per Global Rule 19. Section numbers refer to the main document's decisions D1–D8.

## Architecture Constraint Table (from memory.db + incidents cited in the perspective reports)

| # | Past failure | Property that failed | Constraint on this design | Honoured by |
|---|---|---|---|---|
| C1 | INC-I-147 (`pending_registrations.rs:1-31`) | a `ValidationContext` Vec defaulting to empty = fail-open at admission | no stateful rotation input may live in `ValidationContext`; state is resolved by the caller into `RotationInputs`; the mempool has no stateful rotation check at all | D3, Module 3/6 |
| C2 | INC-I-180 `allowance_with` | two copies of one rule drift | ONE core predicate, thin wrappers | Module 3, 5, 6 |
| C3 | INC-I-071 (`rollback.rs:178`) | empty snapshot sentinel skipped a needed restore | `block_mutates_producer_set` arm + negative control on the **queue** | Module 5, D4 |
| C4 | INC-I-054 | activation height moved after crossing | own field, `u64::MAX`, never reused | D6 |
| C5 | INC-I-075 cascade | "currently unused" skipped a gate | three-question checklist answered; AH required | main §checklist |
| C6 | INC-I-176 (`authmsg.rs`) | second encoder, action outside the signature, digest-only signer | single published preimage encoder; `-V1` tags; CLI calls core | D7 |
| C7 | INC-I-016 shape (`behaviour_events.rs:70-74`) | undecodable gossip penalises honest forwarders | below-AH block invalid; pinning session owns `MIN_PEER_PROTOCOL_VERSION` | D6 |
| C8 | Full Bitfield Decode pillar | encoder/decoder pair drifted | the key used at pool time == the key used at aggregate time, pinned by INV-ATTEST-003 | D5 |

Invariants read from memory.db before design: INV-VALIDATION-001 (mempool/builder reachability of
apply rejections — satisfied trivially because apply never rejects a rotation), INV-VALIDATION-008
(zero-flow exemption — no longer applies: the tx is 1-in/1-out), INV-ATTEST-001/002 (attendance and
own-half admission — untouched), INV-BOND-001 (bond_count vs UTXOs — rotation touches neither).

## Failure Modes (system level)

| Scenario | Modules | Detection | Recovery | Degraded behaviour |
|---|---|---|---|---|
| Rotation block reorged out | apply, rollback | undo snapshot non-empty; test asserts the queue | snapshot restore removes the queued update; pool cleared | none (key never changed) |
| Reorg across the boundary that flushed | rollback, attestation | `[ROLLBACK]` + INV-ATTEST-003 test | OLD key restored; `parent_sig_pool.clear()`; re-flush on the new branch | rotator's halves drop until re-flush (signs with NEW key) |
| Rebuild path (`rewards.rs:1117`) reached via `rollback.rs:144,267`, `block_handling.rs:739,910` | apply #2 | `serialize_canonical` equality test across paths at the boundary | none needed — identical arm | — |
| Node restart mid-epoch with a queued rotation | storage | REQ-ROT-011 restart test | `pending_updates` is bincode-persisted with the ProducerSet | — |
| Snap-sync mid-epoch | network/sync | payload = bincode `ProducerSet` incl. `pending_updates` (`sync/manager/types.rs:190`) | queued rotation travels with the snapshot | Light mode until next boundary (existing) |
| Rotation + `Exit` same epoch | flush | none needed | both apply in order; exited producer carries the new key (harmless) | — |
| Rotation + `Slash` same epoch | flush | none needed | unconditional arm; slash status unaffected | — |
| Two rotations, same producer, one block | apply | `[BLS_ROTATE] skipped reason=AlreadyPending` | second skipped deterministically (sequential apply) | sender paid one wasted fee |
| Two producers → same key K, different blocks | apply | `skipped reason=KeyInUse` | loser skipped; CLI pre-check avoids it for honest users | loser paid a fee |
| Half for boundary block B arrives before B is applied (pool poison) | attestation | INV-ATTEST-003 test; `[BLOCK_POISON]` must never appear | `post_commit.rs:421` clears the pool in the same commit | rotator may lose the boundary minute |
| Invalid G1 point queued | apply | impossible: `rotate_verdict` decodes + subgroup-checks before queueing | — | — |
| Un-upgraded node post-pin | network | P4 penalties on honest forwarders (`gossip/validation.rs:96-105`) | pinning session: census + `MIN_PEER_PROTOCOL_VERSION` bump | out of scope (D6) |
| Malicious upgraded builder includes a rotation pre-pin | validation | block invalid on new nodes, undecodable on old | builder scored `InvalidBlock` | none |
| Metric never written (INC-I-187 class) | metrics | wiring test asserts the counter delta at a boundary | — | — |
| Wallet `import-bls` with a wrong secret | CLI | sign→verify round-trip + derived-key print BEFORE write | refuse | — |

## Security Model

### Trust boundaries (all UNTRUSTED; nothing trusted for coming from our CLI — Q1)

| TB | Entry | Bytes | Consumer | Mechanism |
|---|---|---|---|---|
| TB-1 | gossip → mempool | tx (1 input, 1 output, 240 B `extra_data`) | `rotation_filter::rotation_admissible` = `rotate_stateless` | full stateless verify; refuse with `[ERRTX-ROT0xx]` |
| TB-2 | received block → validation → apply | same | `validation/block.rs` (AH, shape), `tx_processing.rs` arm | invalid block below AH / bad shape; skip on payload failure |
| TB-3 | block store → rebuild | same | `rewards.rs` arm | identical `rotate_verdict`; bytes are already-accepted blocks |
| TB-4 | `new_bls_pubkey` → `ProducerInfo.bls_pubkey` | 48 B | `keys.rs:13-28` → `bls_fast_aggregate_verify` | G1 decode + subgroup + non-identity + bound PoP before queueing |
| TB-5 | RPC `submitTransaction` | whole tx | as TB-1 | as TB-1 |
| TB-6 | CLI-built tx | operator key | as TB-1 | core encoder; no wallet-table entry |

### Attack surface → mitigation (every REQ-ROT-SEC-* mapped)

| Threat | Vector | Mitigation | REQ |
|---|---|---|---|
| T1 key squatting (rotate to a victim's key, steal attestation credit / break the victim's aggregate) | payload with victim's `bls_pubkey` | uniqueness scan over `producers` + queued rotations at apply; PoP requires the secret → an attacker cannot produce a valid bound PoP for a key it does not hold; skip, never a block rejection | SEC-004, SEC-002 |
| T2 rogue-key / PoP replay across producers | reuse a registration PoP (bare, `bls.rs:616,636`) | rotation PoP under its own DST over `genesis‖producer_ed25519‖new_key` — a registration PoP is not a rotation PoP and a rotation PoP for producer X is not one for Y | SEC-002 |
| T3 unsigned `extra_data` (INC-I-169 F1 open) | mutate payload bytes in flight | inner Ed25519 over `DOMAIN‖genesis‖new_key‖outpoint` by the producer key; any byte change fails | SEC-001 |
| T4 intra-chain replay | re-submit an old rotation tx | spent outpoint dies at `utxo.rs:126-139`; outpoint inside the signature blocks third-party re-wrap around a fresh input | SEC-003 (superseded form) |
| T5 spam / validator CPU | flood rotations | fee-priced (1-in/1-out); state checks (µs) precede crypto (ms); worst case bounded by distinct registered producers | SEC-005, D2 |
| T6 stealth takeover with a stolen Ed25519 key | attacker rotates the victim's BLS key | accepted residual: an Ed25519 thief already controls Exit/withdrawals (Q5); rotation is visible in `getProducers.pendingUpdates` for a full epoch and in `[BLS_ROTATE]` logs, giving the victim the epoch to act | SEC-006 |
| T7 cross-network replay | replay a testnet rotation on mainnet | `genesis_hash` in both preimages; outpoints differ per chain | SEC-009 |
| T8 malformed payload → unverifiable producer forever | invalid point applied | decode + subgroup check before queue; `keys.rs:26` never sees an undecodable key from this path | SEC-007 |
| T9 grief honest builders | height-dependent or race-dependent validity | no expiry; payload failures skip; only structural rules invalidate a block | SEC-008 (redefined) |
| T10 un-upgraded fleet | undecodable blocks post-pin | pinning precondition: census + `MIN_PEER_PROTOCOL_VERSION` + GS-022 pass | SEC-010 |

### Data classification

| Data | Class | Storage | Notes |
|---|---|---|---|
| `new_bls_pubkey`, PoP, inner signature | public, untrusted until verified | block store, then `ProducerInfo.bls_pubkey` (rooted) | 48 B key is consensus-load-bearing |
| queued `PendingProducerUpdate::RotateBlsKey` | public, unrooted | ProducerSet bincode (disk, undo, snap-sync) | visible via RPC; not in the state root |
| BLS secret (wallet) | secret | `wallet.json` `bls_private_key` hex | `import-bls` never prints it; `rotate-bls` never modifies the wallet |

## Protection mechanisms (system-impact Rule 29)

| Mechanism | Trigger | Action | Scale assumption | Interactions |
|---|---|---|---|---|
| Below-AH block rule | `RotateBlsKey` present, `height < AH` | block invalid | none (constant) | same shape as oracle/DeFi frozen types; no scoring change |
| Apply-time skip | payload verdict `Err` | log + counter, no state change | O(1) per tx | never interacts with peer scoring (no rejection) |
| Boundary pool clear (existing) | `is_epoch_boundary_with` | `parent_sig_pool.clear()` | ≤ 8 parents × producers | shared trigger with epoch-state rebuild; unchanged |
| Rollback pool clear (new) | ProducerSet restore/rebuild in `rollback_one_block` | `parent_sig_pool.clear()` | same | fires only on reorgs that touched the producer set |

## Gauntlet scenarios to add (`scripts/gauntlet-seed.sql`, opt-in, chain-writing, testnet only)

- **GS-021 `bls-rotation-boundary`**: submit a valid rotation on n-k, assert `pendingUpdates` shows
  it on every node, cross the boundary, assert `blsPubkey` flips on every node at the same height,
  `doli_producer_bls_rotation_total` increments by 1 on every node, no `[BLOCK_POISON]` in any log,
  the rotator's bit is set within 2 blocks after the boundary.
- **GS-022 `bls-rotation-boundary-reorg`**: same as GS-021 with a forced single-node reorg across the
  boundary (existing GS-009-style restart of one producer at B-1); assert convergence of
  `serialize_canonical` and the absence of `REASON_AGGREGATE_INVALID` on the honest chain. Also the
  pinning-session precondition: replay an undecodable-block flood against one node running the
  previous binary and assert honest peers' scores stay above `-500` (`scoring.rs:137`).

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  0 (observed)
  Disk:     0 (observed)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: NONE-NEEDED
Why this proposal anyway: documentation companion; runtime cost is declared in the main document
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```
