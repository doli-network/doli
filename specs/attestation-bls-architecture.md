━━━ FINDINGS — 13 total (DECISION:13) ━━━

  [F1] DECISION conf(0.90, converged) — crates/core/src/attestation.rs:379-454 — delete the write-only minute-keyed BLS store, its 0-caller readers, `record_with_bls`, `RegionAggregate`, and the per-block dead `bls_sign` at post_commit.rs:450-457 (D1)
  [F2] DECISION conf(0.88, converged) — crates/core/src/attestation.rs:375-380 — one bounded parent-hash-keyed signature pool (K=8 parents) beside the untouched minute attendance map (D2)
  [F3] DECISION conf(0.90, converged) — bins/node/src/node/startup.rs:611 (M2: `Node::sign_attestation`, bins/node/src/node/attestation/mod.rs) — attesters dual-sign (Ed25519 + BLS over the parent) on the gossip wire in Release N, before any height is pinned (D3)
  [F4] DECISION conf(0.88, converged) — bins/node/src/node/network_events.rs:366-372,602-611 — verify every BLS signature individually at ingress in ONE shared ingress body; drop on failure; never overwrite verified with unverified (D4)
  [F5] DECISION conf(0.92, converged) — bins/node/src/node/production/assembly.rs:408-427 + apply_block/post_commit.rs:34-57 — ONE canonical height-parameterised universe function `[base | (active_at(h) \ base) sorted]`; rewards consume `universe[..base_len]` (D5)
  [F6] DECISION conf(0.90, converged) — bins/node/src/node/validation_checks.rs:422-446 — post-AH `presence_root = BLAKE3(len ‖ bitfield ‖ aggregate)` via ONE named function, unconditional, canonical empty encoding (D6)
  [F7] DECISION conf(0.90, converged) — bins/node/src/node/validation_checks.rs:448 — one aggregate verifier inside `validate_block_for_apply`, AFTER `validate_block_with_mode` (VDF + eligibility), skipped-and-counted in Light mode (D7)
  [F8] DECISION conf(0.95, converged) — crates/core/src/network_params/mod.rs:810 (precedent) — one forward-only `inc_i_178_attestation_bls_activation_height`; mainnet `u64::MAX`; no HardForkSchedule; no protocol-version bump; two-release order (D8)
  [F9] DECISION conf(0.80, converged) — crates/crypto/src/bls.rs:709-714 — BLS message = parent block hash ALONE (drop `‖ slot_be`), frozen before Release N ships (R1)
  [F10] DECISION conf(0.75, measured) — crates/core/src/presence.rs (798 lines, 0 callers) — delete with its `lib.rs:232` re-export (R2)
  [F11] DECISION conf(0.75, measured) — crates/core/src/consensus/constants.rs:63 — delete the unreachable legacy header-bitfield era (Hash codec ×3 + six `h < 0` arms) (R3)
  [F12] DECISION conf(0.72, observed) — crates/core/src/attestation.rs (703 lines) — split into `core/attestation/{wire,bitfield,tracker,universe,commit}.rs`, the home of the pure functions (R4)
  [F13] DECISION conf(0.75, converged) — bins/node/src/node/validation_checks.rs:438 — exact-width committee contract (`==` not `≤`) post-AH, strictly AFTER D5 lands with the divergence test (R5)

  Speculative: 6 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# Attestation BLS Architecture

**Run:** `/omega-redesign attestation-bls` PROPOSAL-ONLY · **Incident:** INC-I-178 (open, high) · **Base:** `c3d9e827` (main) · **Date:** 2026-09-03 · **Synthesized from:** 5 evaluator reports + analyst analysis (`docs/redesigns/attestation-bls-redesign-analysis.md`). Reasoning trace: `docs/.workflow/architecture-reasoning.md`.

## Problem Statement

The on-chain attendance bitfield is a producer's unilateral, unauthenticated claim. `presence_root` validation is `BLAKE3(bitfield)` + a stray-bit bound (`validation_checks.rs:421-446`); no signature is checked; a forged bit is accepted. `Block.aggregate_bls_signature` is emitted empty (`production/mod.rs:601`), uncommitted by any hash, and has no validator. The deleted 2026-03 path (`86bac138`, `427d5050`) was cryptographically incoherent by construction: it aggregated signatures over up to six different messages (minute-union) and verified with same-message-only `fast_aggregate_verify` (`bls.rs:695`) against a message no attester signs. "Restore it" is not an option.

**Refined prompt:** close the gap so that a set bit is bound to a BLS signature the named producer actually made over ONE agreed message, hash-committed, verified on the live apply path, behind one forward-only activation height, with pre-AH bytes identical.

**User's standing decision (INC-I-178, 2026-08-17): Option A — real BLS aggregate attestation verification behind a forward-only AH.** Verdict of this synthesis: **the evidence upholds Option A.** All five evaluators designed within it; none found a structural reason to reject it; the Pattern Matcher established it is the Altair `SyncAggregate` construction (one committee, one message, one aggregate) and that CometBFT keeps N individual signatures only because its per-validator sign-bytes differ, a constraint DOLI lacks. The Failure Analyst records the one zero-fork-risk alternative (delete the field, make the spec honest) so the choice is made knowingly. Costs are measured at < 0.1 % of a slot at N=45 and N=1000 for validation (blst 0.3.17, M4 Max, single-architecture).

━━━ RESOURCE COST — SUMMARY — COST-DECLARED ━━━
Dimensions:
  CPU:      +0.87 ms/block validate at N=45, +9.1 ms at N=1000 (measured, radical bench M4 Max); +1.33/29.4 ms per block BUILT at N=45/1000 producer-side aggregate (measured); +488 µs per received attestation ⇒ +22 ms/slot at N=45, +488 ms/slot at N=1000 (measured); −1 BLS sign per applied block per node (deleted dead write, observed)
  Memory:   −≤0.5 MB/epoch sawtooth (dead store) +34 KB bounded at N=45 / +0.77 MB at N=1000 for the parent pool (observed)
  IO:       0 (observed: message is `header.prev_hash`, already in hand; producer_set already locked at validation_checks.rs:434)
  Network:  +96 B per attestation (156→252 B, +61.5 %) ⇒ +4.3 KB/slot at N=45, +96 KB/slot at N=1000 (observed from bincode layout)
  Disk:     +96 B per stored block ⇒ ≈ +830 KB/day/node (inferred, 8640 blocks/day)
  Latency:  +0.87 ms on the block-validation critical path at N=45 = 0.0087 % of a 10 s slot (measured)
Inevitability: AVOIDABLE
Cheaper alternative: leave the bitfield an unverified producer claim and delete `Block.aggregate_bls_signature` (zero fork risk, abandons the security goal — Failure Analyst subtraction note)
Why this proposal anyway: the user chose Option A; it is the smallest structure that makes a set bit unforgeable, and every measured dimension is under 0.1 % of a slot at both N=45 and the N=1000 design target except ingress verify at N=1000 (4.9 %), which carries a named batch-verify trigger
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Evaluation Summary

| Evaluator | Lens | Top Proposal | Confidence | Key Finding |
|---|---|---|---|---|
| Subtractionist | removal | delete write-only BLS store + legacy era + `add_bls_key`; universe as ONE sliced list | conf(0.7, measured) | ~330 removable lines, 0 AH; `aggregate_bls_signature` is structurally unremovable (bincode/RocksDB) so Option A must reuse it; live denominators are 3, not 4 |
| Restructurer | boundaries | pure `core::attestation::{universe,commit,verify}` + `node/attestation/keys.rs` binding | conf(0.68, measured) | the verifier died because key-gathering sat on the wrong side of `core → storage`; universe exists only during `apply_block`; `presence.rs` 798 lines, 0 callers |
| Pattern Matcher | patterns | frame as Altair `SyncAggregate`; pool keyed by message; verify at ingress; message = parent hash alone | conf(0.7, observed) | NEW liveness attack: one registered producer's garbage BLS blob poisons every aggregate (all-or-nothing) |
| Failure Analyst | failures | 20 constraints C1–C20; verify at ingest; skip+count in Light mode; verify after VDF; length-prefix preimage | conf(0.68, measured) | relay byte-flip halts the chain (F1); snap-synced nodes deadlock unless exempted (F2); no BLS key-rotation TxType (F3) |
| Radical Simplifier | minimal | ~185 non-test LOC across two releases; defer the pubkey cache; message = parent hash | conf(0.68, measured) | analyst's 202 ms verify is wrong by ~230× (869 µs measured); `pks_validate=false` task is DEAD (blst hardcodes it) |

## Convergence Matrix

Y = proposes/agrees · P = partial or as a constraint · N = opposes · – = silent. A = analyst.

| Item | Sub | Restr | Patt | Fail | Rad | A | Count | Verdict |
|---|---|---|---|---|---|---|---|---|
| Delete minute-keyed `bls_sigs` store, 0-caller readers, `record_with_bls`, per-block `bls_sign` | Y | Y | Y | Y | Y | Y | 5/5 | DEFINITE D1 |
| Parent-hash-keyed bounded signature pool | Y (kill test) | Y | Y | Y (C20) | Y | – | 5/5 | DEFINITE D2 |
| Dual-sign on gossip wire in Release N, before pin | P (SUB-C2) | Y | Y | Y (C15) | Y | Y | 5/5 | DEFINITE D3 |
| Attester signs the PARENT (block just applied) | – | Y | Y | Y | Y | Y | 4/5 | DEFINITE D3 |
| Drop `‖ slot` from BLS preimage | – | N (keeps) | Y | P (C10 dissolved) | Y | N (keeps) | 2/5+1 | RECOMMENDED R1 (code-resolved) |
| Verify each BLS signature at ingress | – | P (C7) | Y | Y (C1) | Y | – | 3/5+1 | DEFINITE D4 |
| ONE canonical height-parameterised universe fn | Y | Y | Y | Y (C4) | Y | Y | 5/5 | DEFINITE D5 |
| Rewards decoders unified now (sliced) | Y | – | – | P (must reproduce) | N (defer) | Y | 2/5 vs 1 | OPTION O2 |
| RPC decoder unified | – | N (impossible) | – | – | N | Y | 0/5 | REJECTED (see contradictions) |
| `presence_root = BLAKE3(bitfield ‖ aggregate)`, one named fn | Y | Y | Y | Y | Y | Y | 5/5 | DEFINITE D6 |
| Length-prefix the preimage | – | N (unneeded) | – | Y (C9) | – | – | 1 vs 1 | DEFINITE D6 (cheap; code-resolved) |
| Unconditional check, canonical empty (no `is_empty()` guard) | Y | P | Y | Y (C9) | N (keeps guard) | – | 3/5 vs 1 | DEFINITE D6 (failure filter) |
| Verifier inside `validate_block_for_apply` funnel | Y | Y | – | Y | Y (C3) | Y | 4/5 | DEFINITE D7 |
| Verifier placed AFTER VDF/eligibility | – | – | – | Y (C8) | N (inline early) | N (beside root check) | 1 vs 2 | DEFINITE D7 (code-resolved: F11) |
| Skip + count verifier in Light mode (snap-sync) | – | P (C2) | – | Y (C3) | – | – | 1/5+1 | DEFINITE D7 (failure filter, INV-EPOCH-002) |
| Keys via `get_by_pubkey` (exited producers resolvable) | – | – | – | – | Y (C4) | – | 1/5 | DEFINITE D7 (verified: 0 `.remove(` in producer/) |
| One AH field, mainnet `u64::MAX`, no HardForkSchedule, no version bump | Y | Y | Y | Y | Y | Y | 5/5 | DEFINITE D8 |
| Delete `presence.rs` (798 lines) | – | Y | – | – | – | – | 1/5 | RECOMMENDED R2 (synth-verified 0 callers) |
| Delete legacy header-bitfield era | Y | – | P (A5) | – | – | – | 1/5+1 | RECOMMENDED R3 (synth-verified const=0) |
| Split `attestation.rs` into a dir; pure fns in `core` | – | Y | – | – | N (node) | P | 1 vs 1 | RECOMMENDED R4 (C1 + module budget) |
| Exact-width `==` contract | – | P | Y | P (F7) | P (subsumes) | P (REQ-014) | 1/5+4 | RECOMMENDED R5 |
| Epoch decompressed-pubkey cache now | – | Y | Y | Y (C18) | N (measured) | Y (Should) | 3 vs 1 | OPTION O1 (measured beats inferred) |
| BLS-only attestations post-AH | – | – | – | – | N (defer) | – | 0/5 | OPTION O3 |
| Verifier module home: new `node/attestation/` vs inline | – | Y (new) | – | – | N (inline) | N (inline) | 1 vs 2 | OPTION O4 |
| BLS key-rotation TxType before pin | – | – | – | Y (C7) | – | N (Won't) | 1 vs 1 | OPTION O5 |
| Dedup own-attestation broadcast | Y (0.5) | Y (merge) | – | – | P | Y | 2/5 | OPTION O6 (low-evidence) |
| INC-I-146 link to BLS store | N (refuted) | – | N (refuted) | P (F12, new store) | Y (candidate) | P (plausible) | 2 N vs 1 Y | REFUTED (code: `reset()` bounds it) |

### Contradictions found and resolved (code is the tiebreaker)

| # | Contradiction | Winner | Code evidence |
|---|---|---|---|
| X1 | Verify cost: analyst 202 ms vs Radical 869 µs at N=45 | **Radical** (measured beats carried-forward) | bench of `bls.rs:676-701` primitives; blst `fast_aggregate_verify` → `aggregate(pks, false)` at `blst-0.3.17/src/lib.rs:1289` (synth-verified in the cargo registry) — the "pks_validate=false fast path" is already on; REQ-BLS-011's sub-task is DEAD |
| X2 | Live denominators: analyst 4 vs Subtractionist 3 | **Subtractionist** | `full_bitfield_decode_height: 0` at `defaults.rs:90,407,684`; `schedule.rs:230-256` builds `[base \| extra sorted]` when `use_full_decode`. Restructurer's C2 still holds for HISTORICAL epochs (RPC reads the live `get_epoch_producer_list()`), so RPC is correct for the current epoch only and stays display-only |
| X3 | INC-I-146 link: analyst "plausible", Radical "named candidate" vs Subtraction/Patterns "refuted" | **Refuted** | `attestation.rs:457-460 reset()` clears `bls_sigs` at every epoch boundary (`post_commit.rs:422`); ≤ N×60 entries; ingress writers unreachable since `427d5050` (field always empty) ⇒ ≈ 9 KB own-key only. Do not attach INC-I-146 |
| X4 | Threshold: brief 54/60 vs Failures 30/60 | **Both are live, gating different things** | `MIN_ATTESTATION_MINUTES = 30` (`constants.rs:99`) gates tier promotion/demotion (`rewards.rs:1086`, `epoch_state/mod.rs:245`); `ATTESTATION_QUALIFICATION_THRESHOLD = 54` (`attestation.rs:235`) via `attestation_qualification_threshold()` gates REWARD qualification Tier 1 (`rewards.rs:211-213`). REQ-BLS-009 must hold at BOTH; docs must name both (handoff H5) |
| X5 | Verifier placement: inline in the existing `presence_root` block (Radical C3, analyst §4.5) vs after authenticity (Failures C8) | **Failures** | `validation_checks.rs:422-446` runs BEFORE `validate_block_with_mode` at `:448`, which runs `validate_vdf` (`validation/block.rs:244`) and `validate_producer_eligibility` (`:247`) and the size cap (`:124`). Placing the pairing after `:448` keeps it inside the same funnel (single caller `apply_block/mod.rs:110`), so Radical's non-foreclosure property survives |
| X6 | Length-prefix: Restructurer "no attack surface" vs Failures "ambiguous at the empty-aggregate boundary" | **Failures, cheaply** | `aggregate_bls_signature: Vec<u8>` (`block.rs:164`) has no length enforcement at deserialization, so `(B‖x, ∅)` and `(B, x)` collide unless two other checks (exact width + `len ∈ {0,96}`) both hold; a 4-byte prefix removes the dependency for ~0 cost |
| X7 | `is_empty()` guard: Radical keeps it vs Patterns A3/Failures F9/Subtraction P5 | **Remove post-AH** | producer-controlled predicate; F9 shows an empty bitfield + garbage aggregate goes uncommitted and unchecked — the exact "field with no validator" dead-end |
| X8 | Pubkey cache: Failures C18 "liveness above N≈200" vs Radical P3 "defer" | **Radical** (measured) | 869 µs naive vs 487 µs cached at N=45; 9.1 ms vs 1.2 ms at N=1000; the DoS concern (F11) is placement, solved by X5 |
| X9 | `add_bls_key`: Radical N7 "derive from seed" vs Subtraction "cannot be repaired" | **Subtraction** | `Wallet { name, version, addresses, origin }` (`wallet.rs:54-62`) never persists the phrase; repair is impossible; deletion is the remedy (handoff H6) |
| X10 | RPC decoder "collapses to one function" (analyst) vs "cannot be correct" (Restructurer) | **Restructurer** for history, **Subtractionist** for shape | see X2 |

## Definite Changes (High Convergence)

Execute these. Every entry carries the architectural label; transitional items live only in the Migration Path. Together D1–D8 ARE the SSF candidate (see the Radical Tiebreaker section).

- ARCHITECTURAL: **D1 — Delete the write-only minute-keyed BLS store and everything that only exists to feed or read it.**
    Convergence: Subtractionist P2, Restructurer P3/P5, Pattern Matcher A1/A7, Failure Analyst M3, Radical P4/N3/N6 + analyst P3/P5 (5/5, independent: caller-grep, key-shape incoherence, pool-key ≠ message, "cannot answer who signed this parent", measured 0 callers).
    Evidence: `attestation.rs:379` `bls_sigs: HashMap<(PublicKey,u32),Vec<u8>>` — 3 writers (`post_commit.rs:456`, `network_events.rs:371`, `:607`), readers `bls_sigs_for_minute` (`:439`) and `bls_sig_count` (`:452`) with 0 callers (synth-verified grep over `crates`+`bins`); `RegionAggregate`/`from_attestations` 0 production callers (only `lib.rs:145` export + `block.rs:136` comment); per-block `crypto::bls_sign` at `post_commit.rs:450-457` on every applied block.
    Confidence: conf(0.90, converged)
    What changes: `MinuteAttestationTracker { attested, bls_sigs }` → `{ attested }` + D2's pool; one `record()` path; `RegionAggregate` gone (spec `protocol.md:1161` drift joins REQ-BLS-015). `Attestation::new_with_bls` is REWIRED by D3 (not deleted); its `Err(_) => Vec::new()` fallback (`attestation.rs:88-90`) is removed — post-AH that fallback is silent exclusion (F3). INV-12: (1) YES (2) YES (3) YES bit-identical — `record_with_bls` and `record` insert the same `attested` entry. No AH; rolling-safe; Release N.

      ━━━ RESOURCE COST — COST-DECLARED ━━━
      Dimensions:
        CPU:      −1 BLS12-381 G2 signature per applied block per producer-keyed node (observed at post_commit.rs:452; ≈ −0.5–1 ms/block, inferred magnitude)
        Memory:   −≈9 KB steady (own key only, N=45) to −0.5 MB/epoch peak in a counterfactual dual-signing fleet (observed from the struct + reset() cadence)
        IO:       0 (observed: in-memory only)
        Network:  0 (observed: no wire field removed)
        Disk:     0 (observed)
        Latency:  −one BLS sign off the apply path per block (inferred)
      Inevitability: AVOIDABLE
      Cheaper alternative: leave the map and let D2 add a second map beside it
      Why this proposal anyway: the `(pubkey, minute)` key is the mechanical cause of the old incoherence (up to six messages under one key); keeping it keeps the temptation to aggregate over mixed messages
      ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: **D2 — One bounded, parent-hash-keyed signature pool; the minute attendance map is untouched.**
    Convergence: Restructurer P3 (`ParentSignaturePool`, K=2), Pattern Matcher P2/C1 (key = bijection of the signed message), Failure Analyst M3/C20 (bound to a small slot window), Radical P4 (`bls_by_parent` + `VecDeque` cap 8), Subtractionist P2 kill test (parent-keyed, 1-2 blocks retention) — 5/5, independent evidence.
    Evidence: `assembly.rs:388-389` reads `attested_in_minute(current_minute)` — the encoder needs minute-keyed ATTENDANCE (unchanged) but parent-keyed SIGNATURES; `network_events.rs:365,:603` discard `block_hash` at write time today.
    Confidence: conf(0.88, converged)
    What changes: `pool: HashMap<Hash, HashMap<PublicKey,[u8;96]>>` + `recent: VecDeque<Hash>` capped at K=8 parents (Radical; Restructurer/Failures say 2-4 suffices — 8 is reorg headroom at 34 KB/N=45, 0.77 MB/N=1000; choose from measured reorg depth before pin, C20). Node-local, never in `EpochState` (C12), never persisted, no undo. Populated pre-AH (harmless); READ by the encoder only post-AH (Pattern Matcher: the builder switch rides the AH). INV-12 for the structure: (1) NO (2) NO (3) YES ⇒ no AH.

      ━━━ RESOURCE COST — COST-DECLARED ━━━
      Dimensions:
        CPU:      +1 hash-map insert per verified attestation (observed); −1 heap alloc per signature (`Vec<u8>` → `[u8;96]`, observed)
        Memory:   +34 KB bounded at N=45, +0.77 MB at N=1000 (observed: 8 × N × 96 B + map overhead)
        IO:       0 (observed)
        Network:  0 (observed)
        Disk:     0 (observed)
        Latency:  0 on the block path (observed: ingress task only)
      Inevitability: INEVITABLE
      Cheaper alternative: NONE-EXISTS — a same-message aggregate cannot be built from a structure keyed by minute
      Why this proposal anyway: it is the smallest structure that answers the only query the builder makes ("all signatures over parent H")
      ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: **D3 — Attesters dual-sign (Ed25519 + BLS) over the PARENT on the gossip wire, in Release N, gossip-only, before any height is pinned.**
    Convergence: analyst REQ-BLS-006, Radical N2, Restructurer egress, Pattern Matcher P4 (ordering), Failure Analyst C15, Subtractionist SUB-C2 (growing `Vec<u8>` 0→96 B is wire-safe under bincode) — 5/5 + analyst.
    Evidence (pre-M2): `startup.rs:611-614` calls `Attestation::new` ("BLS attestation aggregate is retired") — SHIPPED in M2: the egress now calls `Node::sign_attestation`, which dual-signs when `bls_key` is `Some` and falls back to Ed25519-only on a BLS error; `attestation.rs:31-33` `#[serde(default)] bls_signature` already on the wire; `post_commit.rs:446` attests the block just applied = the next block's parent; the node already BLS-signs the right message every block (`post_commit.rs:450-457`) into a dead sink.
    Confidence: conf(0.90, converged)
    What changes: ONE egress path (`create_and_broadcast_attestation`) emits `Attestation::new_with_bls` with the frozen message of R1; the duplicate BLS work at `post_commit.rs:450-457` goes (D1); own attestation is recorded through the same ingress body as peers' (D4). Keeps the Ed25519 signature (finality path, cheap fast-reject; see O3). INV-12: (1) NO (2) YES (3) YES — no validation outcome changes pre-AH; gossip bytes only ⇒ no AH; rolling-safe. Release N MUST ship and be fleet-proven before the pin (C15; the AH gates the validator, not the attester population).

      ━━━ RESOURCE COST — COST-DECLARED ━━━
      Dimensions:
        CPU:      +1 BLS sign per own attestation per applied block (≈ +0.5–1 ms, inferred from the 488 µs measured verify; sign not benchmarked) — net ≈ 0 against D1's deleted sign
        Memory:   +96 B transient per attestation (observed)
        IO:       0 (observed)
        Network:  +96 B per attestation, 156 → 252 B (+61.5 %) (observed) ⇒ +4.3 KB/slot at N=45, +96 KB/slot at N=1000
        Disk:     0 (observed: attestations are not persisted)
        Latency:  0 on the block path (observed)
      Inevitability: INEVITABLE
      Cheaper alternative: NONE-EXISTS — a verifiable aggregate requires the individual signatures to reach the builder
      Why this proposal anyway: the field already exists on the wire and the node already computes the signature; only the constructor call changes
      ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: **D4 — Verify every attestation's BLS signature individually at ingress, in ONE shared ingress body for both paths; drop the blob (keep Ed25519 attendance) on failure; never overwrite a verified signature; budget invalid attempts via peer scoring.**
    Convergence: Pattern Matcher P3/C2 (attack derived from Ed25519 covering only `block_hash‖slot`), Failure Analyst M1/C1/C2 (relay byte-flip + raw-byte dedup at `gossip/staleness.rs:206` + last-write-wins), Radical P5 (measured 488 µs), Restructurer C7 (ingress precondition uniformity) — 3/5 explicit + 1, independent evidence.
    Evidence: `attestation.rs:112-117` signed bytes = `block_hash ‖ slot_be` only; `network_events.rs:366-372`,`:602-611` clone the blob unverified; `bls.rs:650-665` `bls_aggregate` group-checks but cannot attribute a bad input; `seen_key(topic,&[data])` dedups on full raw bytes (INV-NETWORK-004 correct; the payload's unsigned field is the defect).
    Confidence: conf(0.88, converged) — filter: RESOLVES C1/C2 (chain-halt) +0.1 on the Pattern Matcher's 0.7; Radical's cheaper "bisect at build time" rejected by Failures (10 s slot budget, still needs attribution) and by Radical (LOC).
    What changes: `Node::record_attendance(&att)` — the single body: `derive_attester_weight(..).is_some()` (C19 posture preserved), `bls_signature.len() == 96`, `bls_verify(bls_attest_msg(att.block_hash), sig, on-chain pk)`, then `tracker.record(pk, minute)` (BYTE-IDENTICAL pre-AH input) + `pool.insert(att.block_hash, pk, sig)` only if verified. Invalid-BLS budget per peer through the existing `crates/network/src/scoring.rs`. INV-12: (1) NO (2) YES (3) YES for consensus output (bit-setting is the builder's local choice) ⇒ no AH; Release N. Named trigger: batch-verify at ingress when N > 500 (4.9 % of a slot at N=1000).

      ━━━ RESOURCE COST — COST-DECLARED ━━━
      Dimensions:
        CPU:      +488 µs per received attestation (measured) ⇒ +22 ms/slot at N=45 (0.22 %), +488 ms/slot at N=1000 (4.9 %, parallelisable, named trigger)
        Memory:   +≈200 B transient per verify (inferred, blst affine point)
        IO:       0 (observed: `bls_pubkey` resolved from the in-memory ProducerSet already read at the same site)
        Network:  0 (observed)
        Disk:     0 (observed)
        Latency:  0 on the block critical path; +≈0.5 ms per attestation on the gossip-ingress task (measured)
      Inevitability: INEVITABLE
      Cheaper alternative: NONE-EXISTS as a substitute — builder-side bisection still needs per-attestation attribution and runs inside the 10 s slot; it remains a valid optimisation layered on top
      Why this proposal anyway: aggregate signatures cannot attribute their own failure; without this one relayed byte-flip or one malicious member halts block production network-wide
      ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: **D5 — ONE canonical, height-parameterised universe function `attestation_universe(base, active_at_h) -> [base | (active_at_h \ base) sorted by pubkey bytes]`, pure over `&[PublicKey]`, used by the encoder, `post_commit`, the stray-bit validator, the aggregate verifier and the epoch rebuild; reward decoders consume `universe[..base_len]`.**
    Convergence: Subtractionist P5 (sliced), Restructurer P1, Pattern Matcher P5/A8, Failure Analyst C4/C13/F7, Radical M2 + analyst REQ-BLS-004 — 5/5 + analyst; independent evidence (rewards slicing kill test; two 20-line hand copies; CometBFT exact-length; fork-capable-site table + precedent `constants.rs:101-102`; G3).
    Evidence: `assembly.rs:408-427` and `post_commit.rs:34-57` are hand copies with no shared symbol; `validation_checks.rs:434-437` uses a third source (`active_at(h).len()`) that can be SMALLER than the encoder width when `producer_list` holds a mid-epoch-exited producer — a pre-existing latent honest-block rejection; `rewards.rs:90-117` is base-only via `epoch_state.producer_list` (synth-verified); all callers already project to `PublicKey` before use, so the function fits `crates/core` without a `storage` dependency (Restructurer C1).
    Confidence: conf(0.92, converged)
    What changes: extraction is bit-identical (encoder/post_commit) ⇒ no AH, Release N+1 pre-AH-safe; the stray-bit denominator switch (`active.len()` → universe width) is consensus-visible ⇒ rides the AH (Restructurer P1, Pattern Matcher C5). Rewards: `universe[..base_len]` == `epoch_state.producer_list` because `epoch_state/mod.rs:218` sorts it and base is first — locked by a property test, NOT a `debug_assert` (compiled out of release). Rebuild (`rewards.rs:565-605`) routes through the same fn and never verifies aggregates (C13). Height-parameterised so pre-AH per-site denominators are reproduced (F13). RPC stays display-only (X2/X10).

      ━━━ RESOURCE COST — COST-DECLARED ━━━
      Dimensions:
        CPU:      −one `producer_set.read()` + one `active_producers_at_height` scan per validated block (observed: `validation_checks.rs:187-194` and `:434-437` collapse to one)
        Memory:   +N × 32 B transient per decode call (observed: 1.4 KB at N=45, 32 KB at N=1000)
        IO:       0 (observed)
        Network:  0 (observed: extraction; the AH-gated denominator switch changes no bytes)
        Disk:     0 (observed)
        Latency:  −small (inferred: one fewer RwLock acquire on the validation path)
      Inevitability: AVOIDABLE
      Cheaper alternative: keep four hand-rolled copies and add a fifth for the verifier
      Why this proposal anyway: the verifier MUST reconstruct exactly the encoder's order; a fifth private copy is how the deleted validator got its own ordering and how v6.17.1's death spiral happened
      ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: **D6 — Post-AH `presence_root = BLAKE3(len(bitfield) as u32 LE ‖ bitfield ‖ aggregate)`, computed by ONE named function `presence_root_preimage(bitfield, aggregate, post_ah)` called by encoder and validator, checked UNCONDITIONALLY (no `!bitfield.is_empty()` guard), with the empty case a canonical checked value (empty bitfield ⇒ aggregate MUST be empty ⇒ root = H(0 ‖ ∅ ‖ ∅)).**
    Convergence: analyst REQ-BLS-003, Restructurer P4 (one named fn; header field structurally un-gateable — `BlockHeader::hash()` has no height/params input, `block.rs:76-97`), Pattern Matcher P5/A3/A4, Failure Analyst C9/F9/M5, Subtractionist P5 + SUB-C7 — 5/5 on the commitment; length-prefix and unconditional check resolved by X6/X7.
    Evidence: `block.rs` `hash()` covers `prev_hash`, `presence_root`, `slot` (synth-verified); `assembly.rs:392-393` emits `Hash::ZERO` on an empty attester set (sentinel = absent); `validation_checks.rs:422-424` skips the whole check when the bitfield is empty.
    Confidence: conf(0.90, converged)
    What changes: pre-AH arm stays `BLAKE3(bitfield)` verbatim with the old guard (byte-identity, B2). Post-AH: fail-closed for old binaries (they compute `BLAKE3(bitfield)` and reject — F5), commitment holds for stripped/mutated aggregates. INV-12: (1) YES (2) YES (3) NO ⇒ AH REQUIRED (`presence_root` is inside the header hash — rules AND content change, INV-DEPLOY-001).

      ━━━ RESOURCE COST — COST-DECLARED ━━━
      Dimensions:
        CPU:      +≈0.1 µs per block (observed: BLAKE3 over ≈100 extra bytes)
        Memory:   0 (observed: streaming hash over fields already in `Block`)
        IO:       0 (observed)
        Network:  0 (observed: no header/wire change; the 32-byte field keeps its size)
        Disk:     0 (observed)
        Latency:  0 (inferred: sub-microsecond)
      Inevitability: AVOIDABLE
      Cheaper alternative: leave the aggregate uncommitted and rely on the pairing failing when a relay corrupts it
      Why this proposal anyway: a STRIPPED aggregate does not fail verification, it skips it; only a commitment distinguishes "producer sent none" from "relay removed it" (REQ-BLS-002 AC-3)
      ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: **D7 — One aggregate verifier, reachable only through `Node::validate_block_for_apply`, placed AFTER `validate_block_with_mode` (VDF + producer eligibility + size cap), executed when `mode == Full`, skipped-and-counted when `Light`; keys gathered in `bins/node` via `ProducerSet::get_by_pubkey`; pure verify math in `core`; results never stored in `EpochState`; a counter that is registered AND incremented.**
    Convergence: analyst REQ-BLS-002/007, Restructurer P2/CS-4 (single funnel `apply_block/mod.rs:110`; key-gathering must be node-side, C1), Radical M5/C3/C4 (inline in the funnel; `get_by_pubkey`), Failure Analyst M2/M5/C3/C8/C12, Pattern Matcher P5 (unconditional), Subtractionist P5 — 5/5 on the verifier; placement and Light-mode skip resolved by X5 + F2.
    Evidence: ordering synth-verified (`validation_checks.rs:422-446` → `:448` → `validation/block.rs:124/244/247`); `mode: ValidationMode` is already a parameter (`validation_checks.rs:175`); `block_handling.rs:414-424` sets `Light` when `snap_sync_height.is_some() || floor_fallback_window` — exactly the known-divergent-universe states (INV-EPOCH-002); `ProducerInfo` records are never removed (0 matches for `producers.remove(` in `crates/storage/src/producer/`, synth-verified) so exited base-list producers resolve; `bls_verify_aggregate` (`bls.rs:676-701`) is written, tested and unused.
    Confidence: conf(0.90, converged) — filter: RESOLVES F2 (+0.1), F11 (+0.1) relative to the inline-early shape.
    What changes: (i) cheap commitment check (D6) stays at `:422`; (ii) after `:448`: `if post_ah && mode == Full { universe = attestation_universe(..); set = decode(bitfield, universe.len()); if set.is_empty() { aggregate MUST be empty } else { keys = set.map(universe[i] → get_by_pubkey → 48-byte key, else REJECT); bls_verify_aggregate(bls_attest_msg(prev_hash), aggregate, keys) } ; ATTEST_AGGREGATE_{VERIFIED,REJECTED}.inc() } else if post_ah { ATTEST_VERIFY_SKIPPED_DIVERGENT.inc() }`. INV-12: (1) YES (2) YES (3) NO ⇒ same AH as D6. Non-foreclosure: the call lives inside the one function every applied block passes through — not a separable entry point (`86bac138` lesson). The Light-mode exemption is a permanent, documented, counted weakening for nodes without full history (Failures M2) — it is the same hole snap sync already is.

      ━━━ RESOURCE COST — COST-DECLARED ━━━
      Dimensions:
        CPU:      +869 µs per validated block at N=45, +9.1 ms at N=1000, all bits set, naive (measured, M4 Max; survives 10×/3× hardware penalty); +1.33/29.4 ms per block BUILT at N=45/1000 (measured)
        Memory:   +N × 48 B transient key vector (observed: 2 KB at N=45, 48 KB at N=1000)
        IO:       0 (observed: message is `header.prev_hash`; ProducerSet already in memory)
        Network:  0 (observed)
        Disk:     +96 B per stored block (observed: existing field goes 0 → 96 B) ≈ +830 KB/day/node
        Latency:  +0.87 ms on the block-validation critical path at N=45 (measured) = 0.0087 % of a slot; bulk sync +87 s per 100k blocks naive (measured × count)
      Inevitability: INEVITABLE
      Cheaper alternative: NONE-EXISTS — Option A's security property IS this pairing; the cache (O1) only shaves the N-linear decompression term
      Why this proposal anyway: it is the single check that makes a set bit unforgeable, and it is N-independent except for 8.4 µs/key decompression
      ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: **D8 — One new forward-only `NetworkParams` field `inc_i_178_attestation_bls_activation_height` (mainnet `u64::MAX`, testnet a future height pinned at rollout, devnet `0`), copying the INC-I-204 five-part shape; no `HardForkSchedule` entry; no `CURRENT_PROTOCOL_VERSION` bump; two-release order.**
    Convergence: 5/5 + analyst REQ-BLS-005 (independent: each evaluator answered INV-12 = YES/YES/NO).
    Evidence: precedent `network_params/mod.rs:800-810` (doc comment answers deploy questions), `defaults.rs:264/502/751` (`u64::MAX` / `88_014` / `0`), `env_loader.rs:463-468` (mainnet override refused); AH ledger test `crates/core/tests/it/inc_i_204_m5_activation_height.rs:306-330`; gate-derived tests per `18779b1e`. `EpochState` layout is unchanged ⇒ no `delete_epoch_state()` (INC-I-054, INV-EPOCH-001). Pattern Matcher A5: `BITFIELD_BODY_ACTIVATION_HEIGHT` is a compile-time constant — the new gate MUST be a param (handoff H7 for the old one).
    Confidence: conf(0.95, converged)
    What changes: the field gates D5's denominator switch, D6, D7 and the encoder's post-AH branch in ONE predicate read through `self.config.network.params()` at all three sites (`validation_checks.rs:117`, `assembly.rs:21`, `post_commit.rs:33` prove the accessor is available). Rules YES, content YES ⇒ AH; rolling-safe strictly below it.

      ━━━ RESOURCE COST — NEGLIGIBLE ━━━
      Dimensions:
        CPU:      0 (observed: one u64 comparison per block)
        Memory:   0 (observed: one u64 in NetworkParams)
        IO:       0 (observed)
        Network:  0 (observed)
        Disk:     0 (observed)
        Latency:  0 (observed)
      Inevitability: INEVITABLE
      Cheaper alternative: NONE-EXISTS — INV-12 (1|2) YES + (3) NO mandates a gate, and the only in-header gate candidates (`fork_id`, `version`) are forbidden or a format migration
      Why this proposal anyway: it is the only gate shape CLAUDE.md permits for a rolling fleet with no stop-all
      ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Recommended Changes (Medium Convergence)

- ARCHITECTURAL: **R1 — BLS message = parent block hash ALONE: `bls_attest_msg(h) = h.as_bytes()` under `ATTESTATION_DST`; drop `‖ slot_be`; FROZEN before Release N ships.**
    Convergence: Pattern Matcher P4, Radical P2/C1 (2/5); Failure Analyst C10/F10 DISSOLVED by it (no exact-pair selection hazard when the slot is not in the message); Restructurer and analyst keep the slot (computable from the parent header at `validation_checks.rs:281-287` on the extend-tip path).
    Evidence: `BlockHeader::hash()` commits `slot` (synth-verified), so the suffix adds no binding; the child header carries `prev_hash` but not the parent's slot; `chain_state.best_slot` is tip-only and wrong on fork branches (Radical); `attestation_message` had exactly one caller left at M1 — `new_with_bls` itself (`attestation/message.rs:80`; the `post_commit.rs:451` sink named here was deleted by M1) — and M2 R1 deleted the function with it (synth-verified) — the preimage is free to change today and NOT after Release N. The Ed25519 signature keeps `block_hash ‖ slot` (distinct algorithm, distinct DST) — document it or the next reader will "fix" it.
    Confidence: conf(0.80, converged) — 2/5 + RESOLVES C10 (+0.1). Below 0.85 because two evaluators explicitly preferred the slot; the difference is a data dependency, not security. **Decision required before M2 ships** (Pattern Matcher C4: a preimage mismatch between releases fails 100 % of blocks at the AH).

      ━━━ RESOURCE COST — COST-DECLARED ━━━
      Dimensions:
        CPU:      −4 bytes hashed per sign/verify (inferred, below noise)
        Memory:   0 (observed)
        IO:       −0 to −1 block-store point read per validated block on the fork/reorg path (observed)
        Network:  0 (observed: the message is never transmitted)
        Disk:     0 (observed)
        Latency:  −0 to −1 RocksDB lookup on the validation path (observed)
      Inevitability: AVOIDABLE
      Cheaper alternative: keep `attestation_message(hash, slot)` and fetch the parent header at validation
      Why this proposal anyway: same security (slot is already committed by the hash), one fewer data dependency, and it removes a fork-branch failure mode before it is written
      ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: **R2 — Delete `crates/core/src/presence.rs` (798 lines, `PresenceCommitment`/`V2`) and the `lib.rs:232` re-export.**
    Convergence: Restructurer P5 (1/5). Synthesizer verification: grep over `crates`+`bins` finds only the re-export; code graph `blast.py` reports 0 dependents (lower bound; graphify is blind to Rust receiver calls — grep is authoritative and agrees).
    Evidence: `crates/core/src/presence.rs` (798 lines), `lib.rs:232`; it is a complete competing dead model of the subsystem being redesigned and the ghost behind `specs/protocol.md:1161`.
    Confidence: conf(0.75, measured). No AH (dead code; INV-12 (3) YES). Release N.

      ━━━ RESOURCE COST — NEGLIGIBLE ━━━
      Dimensions:
        CPU:      0 (observed: no runtime path)
        Memory:   0 (observed)
        IO:       0 (observed)
        Network:  0 (observed)
        Disk:     0 (observed: smaller binary, not a persisted format)
        Latency:  0 (observed)
      Inevitability: AVOIDABLE
      Cheaper alternative: mark `#[deprecated]` and leave it
      Why this proposal anyway: a 798-line competing presence model is the first thing a reader finds when asking "where does presence live" — the comprehension failure that let the denominators multiply
      ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: **R3 — Delete the unreachable legacy header-bitfield era: Hash-variant codec ×3 (`attestation.rs:261-274`, `:338-368`) and the six `h < BITFIELD_BODY_ACTIVATION_HEIGHT` arms (`assembly.rs:453-456`, `post_commit.rs:65-66`, `rewards.rs:143-146/818-823/1020-1024`, `schedule.rs:310-312`).**
    Convergence: Subtractionist P1 (1/5); Pattern Matcher A5 corroborates the constant; synth-verified `BITFIELD_BODY_ACTIVATION_HEIGHT: u64 = 0` (`constants.rs:63`) ⇒ `h < 0` unsatisfiable on `u64`.
    Evidence: as above; `bins/node/tests/m_rc9_silent_vec_regression.rs` documents "= 0 → always body". Residual: a non-mainnet env override of `full_bitfield_decode_height`/`rewards_epoch_list_fix_height` (`env_loader.rs:254-267`) is the difference between "provably dead" and "dead in every configuration seen" — those two ladders stay (tier-2) unless the override surface is closed.
    Confidence: conf(0.75, measured). No AH. Release N. Each decoder drops from three arms to two, making the remaining rewards-vs-encoder difference visible.

      ━━━ RESOURCE COST — NEGLIGIBLE ━━━
      Dimensions:
        CPU:      0 (observed: removes ~6 always-false comparisons per block, below noise)
        Memory:   0 (observed)
        IO:       0 (observed)
        Network:  0 (observed)
        Disk:     0 (observed)
        Latency:  0 (observed)
      Inevitability: AVOIDABLE
      Cheaper alternative: leave the dead arms and add the verifier beside them
      Why this proposal anyway: the era ladder is what makes five decode sites LOOK different; it is the resemblance the Full Bitfield Decode pillar warns about
      ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: **R4 — Split `crates/core/src/attestation.rs` (703 lines) into `core/attestation/{wire,bitfield,tracker,universe,commit}.rs` (each < 300 lines); the pure functions of D2/D5/D6/R1 live there; `verify_block_aggregate(bitfield, aggregate, ordered_keys, message)` is pure `core` math; key gathering stays in `bins/node`.**
    Convergence: Restructurer P1/P4/P5 + C1 (1/5); Radical places `attendance_universe` in `bins/node` over `&[&ProducerInfo]` (also C1-compliant); CLAUDE.md #19 (500-line budget) forces the split once D2/D5/D6 add ~80 lines to a 703-line file.
    Evidence: all universe callers already project to `PublicKey` (`assembly.rs:413-417`, `post_commit.rs:39-43`, `validation_checks.rs:188-192`) — zero new dependencies; `crates/core/Cargo.toml` has no `storage` dep and must not gain one (the `ValidationContext.producer_bls_keys` defect).
    Confidence: conf(0.72, observed). No AH (pure moves; behaviour-preserving). Release N.

      ━━━ RESOURCE COST — NEGLIGIBLE ━━━
      Dimensions:
        CPU:      0 (observed: module moves)
        Memory:   0 (observed)
        IO:       0 (observed)
        Network:  0 (observed)
        Disk:     0 (observed)
        Latency:  0 (observed)
      Inevitability: AVOIDABLE
      Cheaper alternative: append the new functions to `attestation.rs` (703 → ~780 lines) and put the universe fn in `bins/node`
      Why this proposal anyway: pure-`core` placement makes the universe and preimage rules unit-testable without a `Node` and callable by the rebuild and RPC; the split keeps every file under budget instead of growing one that is already 1.4× over
      ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: **R5 — Exact-width committee contract post-AH: `bitfield.len() == ceil(universe.len()/8)` and every set bit indexes into the universe (`==`, not `≤`), landing ONLY AFTER D5 and the divergence test (`producer_list \ active_at(h) ≠ ∅`).**
    Convergence: Pattern Matcher P5/C5 (CometBFT `len(commit.Signatures) == len(vals)`), Restructurer P1 (denominator → universe width), Radical M5 (`universe.get(i)` rejects out-of-range), Failure Analyst F7 (latent honest-block rejection today), analyst REQ-BLS-014.
    Evidence: `validation_checks.rs:435-438` uses `active_at(h).len()` with `≤`; two different numbers by construction.
    Confidence: conf(0.75, converged). AH-gated (same field). Kill test fired (Pattern Matcher): reversing the order is a chain halt — hence the hard sequencing.

      ━━━ RESOURCE COST — NEGLIGIBLE ━━━
      Dimensions:
        CPU:      0 (observed: one integer comparison)
        Memory:   0 (observed)
        IO:       0 (observed)
        Network:  0 (observed)
        Disk:     0 (observed)
        Latency:  0 (observed)
      Inevitability: AVOIDABLE
      Cheaper alternative: keep the `≤` bound
      Why this proposal anyway: `≤` hides encoder/validator width drift (the latent fork in F7) instead of exposing it as a fault
      ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Options for User Decision

Divergent additions and low-evidence items. Each is a separate choice; none is aggregated into the Definite set.

**O1 — Epoch decompressed-pubkey cache (REQ-BLS-011): build now vs DEFER with named triggers.** Now: Restructurer, Pattern Matcher (7), Failure Analyst C18, analyst (Should). Defer: Radical P3, conf(0.70, measured). Evidence: 869 µs naive vs 487 µs cached at N=45 (0.0038 % of a slot saved); 9.1 vs 1.2 ms at N=1000; pays for itself past N≈4000; the cache needs invalidation at every epoch boundary AND every ProducerSet mutation, and a stale entry is a fleet-wide false rejection. The `pks_validate=false` sub-task is DEAD (X1). **Recommendation: defer (low-evidence tag: single-architecture bench).** Triggers: bulk-sync regression > 5 %, N > 2000, or fleet host > 10× slower than the bench. Complexity cost if built: +1 module, +1 type, +2 invalidation hooks, ~150 LOC. vs radical floor: +1 module.

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      −382 µs/block at N=45, −7.9 ms at N=1000 if built (measured); +0 if deferred
      Memory:   +N × ≈200 B resident per epoch if built (inferred, blst affine): 9 KB at N=45, 200 KB at N=1000
      IO:       0 (inferred)
      Network:  0 (inferred)
      Disk:     0 (inferred)
      Latency:  −382 µs on validation at N=45 if built (measured)
    Inevitability: AVOIDABLE
    Cheaper alternative: defer — call `bls_verify_aggregate` directly (the recommended path)
    Why this proposal anyway: only if a measured trigger fires; today it buys 0.0038 % of a slot for a consensus-correctness surface
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

**O2 — Reward decoders on `universe[..base_len]` now (Subtractionist P5, analyst) vs untouched with the slice invariant locked by a test (Radical D3/C7).** Both are behaviour-preserving; the difference is whether `rewards.rs` (1586 lines, money, three identical decode blocks at `:139/:814/:1016`) is touched in this work. Failure Analyst C4 requires the shared fn to reproduce base-only below the AH either way. **Recommendation: (b) untouched + property test `attestation_universe(base, active)[..base.len()] == base`; adopt the slice under a later incident with its own replay.** conf(0.7, observed). Trigger for (a): a measured qualifier-set delta traced to the denominator.

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      0 (observed: (a) adds an O(E log E) sort of the empty-most-blocks `extra` set; (b) adds nothing)
      Memory:   0 (observed)
      IO:       0 (observed)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  0 (observed)
    Inevitability: AVOIDABLE
    Cheaper alternative: (b) leave rewards untouched — the recommended path
    Why this proposal anyway: (a) only if the user wants "one function, zero copies" in this cycle at the price of touching money code
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

**O3 — Keep dual-signing forever vs BLS-only attestations after activation (Radical D4).** BLS-only saves 64 B and ~50 µs per attestation but costs a second wire break, a second migration window, and removes the cheap Ed25519 fast-reject that fronts the 488 µs BLS verify (finality also consumes the Ed25519 path). **Recommendation: dual-sign; revisit only with a wire-version bump needed for other reasons.** conf(0.7, observed).

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      −≈50 µs per attestation if BLS-only (inferred); +0 if dual
      Memory:   0 (observed)
      IO:       0 (observed)
      Network:  −64 B per attestation if BLS-only (observed)
      Disk:     0 (observed)
      Latency:  0 (observed)
    Inevitability: AVOIDABLE
    Cheaper alternative: dual-sign (recommended) — no second wire break
    Why this proposal anyway: BLS-only only pays when a wire-version change is already being taken
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

**O4 — Verifier/key-binding home: (a) new `bins/node/src/node/attestation/{mod,keys,egress}.rs` (Restructurer P2, ~200 new lines, ~90 moved; `validation_checks.rs` net −25, `startup.rs` −55, `post_commit.rs` −12) vs (b) inline in `validation_checks.rs` (Radical C3, analyst §4.5; +45 lines to a 1614-line file with a 758-line function).** Both keep the call inside the funnel (non-foreclosure). Filter: Restructurer C8 forbids growing the over-budget files. **Recommendation: (a).** conf(0.72, observed).

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      0 (observed: same runtime path either way)
      Memory:   0 (observed)
      IO:       0 (observed)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  0 (observed)
    Inevitability: AVOIDABLE
    Cheaper alternative: (b) inline, zero new files
    Why this proposal anyway: (b) adds consensus logic to the largest file in the repo, the environment in which four denominators accumulated
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

**O5 — BLS key rotation before/with the pin (Failure Analyst C7) vs accept in writing that key loss = `Exit` + re-`Registration` (loses `registered_at` seniority under INC-I-193 tiering; vesting penalty, itself unenforced per INC-I-171).** No `TxType` among 24 rotates a BLS key; `bls_pubkey` is written only at Registration apply (`tx_processing.rs:245`), genesis completion (`genesis_completion.rs:134`) and rebuild (`rewards.rs:1302,1550`). A rotation TxType is its own AH and its own incident (analyst REQ-BLS-021 Won't). **This is a user decision with permanent economic consequences (F3); the design blocks neither path.** conf(0.62, measured) on the hazard; low-evidence on which path.

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      +1 tx validation per rotation if built (inferred); 0 if accepted in writing
      Memory:   0 (observed)
      IO:       +1 ProducerSet write per rotation if built (inferred)
      Network:  +1 tx per rotation if built (inferred)
      Disk:     +1 tx per rotation if built (inferred)
      Latency:  0 (observed)
    Inevitability: AVOIDABLE
    Cheaper alternative: accept in writing (zero code) with the C6 ProducerSet probe as the guard
    Why this proposal anyway: a rotation path is the only in-protocol recovery from a lost or mismatched key once the AH is crossed
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

**O6 — Own-attestation broadcast dedup (Subtractionist P3, conf(0.5, observed), low-evidence).** Two sites broadcast the same attestation (`post_commit.rs:446` during apply; `assembly.rs:665` after block broadcast). Both naive deletions are DEAD (`production/mod.rs:644-650` returns under `BEHIND_TIP_SUPPRESS` before `:662`; the early copy may be dropped by ingress A1 before the block is known). Surviving form: record-only at `post_commit`, broadcast at `attest_own_block` — a system-dynamics change (CLAUDE.md #29: failure-mode matrix + gauntlet), own release, never bundled with the AH release. **Recommendation: keep both broadcasts in this cycle (D1 already removes the duplicated BLS WORK); revisit under its own ticket.**

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      −1 Ed25519 sign + serialization per produced block if deduped (observed)
      Memory:   −1 transient Attestation per produced block (observed)
      IO:       0 (observed)
      Network:  −1 gossip publish −1 DirectAttestation per produced block (observed; ≈ 8640/day fleet-wide at N=45)
      Disk:     0 (observed)
      Latency:  −small on the apply path (inferred)
    Inevitability: AVOIDABLE
    Cheaper alternative: keep both broadcasts (recommended now); gossipsub dedup absorbs most of the second copy
    Why this proposal anyway: only under its own ticket with attendance monitoring — attendance feeds INV-EPOCH-004 removal
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Failure-Mode Filter (C1–C20 × Definite/Recommended)

pass = satisfied by construction · mit = mitigated by the named change · open = must be closed by a precondition/test · n/a = not affected.

| C | Constraint (abridged) | D1 | D2 | D3 | D4 | D5 | D6 | D7 | D8 | R1 | R5 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| C1 | verify each BLS sig at ingest; never overwrite verified | n/a | mit(D4) | n/a | **pass** | n/a | n/a | n/a | n/a | n/a | n/a |
| C2 | never trust an unsigned gossip field | n/a | mit(D4) | mit(D4) | **pass** | n/a | n/a | n/a | n/a | n/a | n/a |
| C3 | skip+count verify where universe known-divergent | n/a | n/a | n/a | n/a | n/a | n/a | **pass** | n/a | n/a | pass |
| C4 | ONE height-parameterised universe fn reproducing pre-AH denominators | n/a | n/a | n/a | n/a | **pass** (O2 test) | n/a | pass | n/a | n/a | pass |
| C5 | assert universe duplicate-free before aggregating | n/a | n/a | n/a | n/a | **open** → M3 assertion + F14 test | n/a | mit | n/a | n/a | n/a |
| C6 | pin only with 100 % well-formed ProducerSet keys + observed verifying attestations | n/a | n/a | mit (soak) | n/a | n/a | n/a | n/a | **open** → Preconditions | n/a | n/a |
| C7 | key-rotation path or written acceptance | n/a | n/a | n/a | n/a | n/a | n/a | n/a | **open** → O5 | n/a | n/a |
| C8 | pairing after VDF/eligibility/size cap | n/a | n/a | n/a | n/a | n/a | pass (cheap check stays early) | **pass** | n/a | n/a | n/a |
| C9 | unambiguous preimage; unconditional empty rule | n/a | n/a | n/a | n/a | n/a | **pass** | pass | n/a | n/a | pass |
| C10 | builder selects exact `(hash, slot)` pair | n/a | pass (keyed by hash) | pass | n/a | n/a | n/a | n/a | n/a | **dissolved** | n/a |
| C11 | producer without a valid aggregate still produces; fallback rate-observed | n/a | pass (empty pool ⇒ empty bitfield) | n/a | n/a | n/a | pass (canonical empty) | **closed (M6)** — chaos test `inc_i_178_m6_chaos.rs`; rate observed by `doli_attestation_bitfield_fill_ratio` | n/a | n/a | n/a |
| C12 | no verification result in EpochState | n/a | **pass** (node-local) | n/a | n/a | n/a | n/a | **pass** | pass | n/a | n/a |
| C13 | rebuild never verifies aggregates; routes through C4 fn | n/a | n/a | n/a | n/a | **pass** | n/a | pass | n/a | n/a | n/a |
| C14 | finality never reads the bitfield | pass | pass (pool ≠ finality map) | pass | pass | n/a | n/a | pass | n/a | n/a | n/a |
| C15 | dual-signing ships before the pin; fleet adoption proven | n/a | n/a | **pass** | n/a | n/a | n/a | n/a | **pass** (two-release) | pass (frozen at N) | n/a |
| C16 | AH margin from measured auto-update telemetry | n/a | n/a | n/a | n/a | n/a | n/a | n/a | **open** → Preconditions | n/a | n/a |
| C17 | REQ-BLS-009 replay includes a degraded epoch | n/a | n/a | n/a | n/a | n/a | n/a | n/a | **closed (M6)** — synthetic healthy + degraded, `inc_i_178_m6_replay.rs` | n/a | n/a |
| C18 | cache is liveness above N≈200; measure bulk sync before pin | n/a | n/a | n/a | n/a | n/a | n/a | mit (X8: measured 9.1 ms at N=1000) | **open** → x86 bench precondition | n/a | n/a |
| C19 | `.is_some()` membership at BOTH ingresses survives | n/a | n/a | n/a | **pass** (one body) | n/a | n/a | n/a | n/a | n/a | n/a |
| C20 | bound the per-parent store to a small window | n/a | **pass** (K=8) | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a |

Additional filters honoured: SUB-C1 (no field removed from `Attestation`/`Block`), SUB-C2 (0→96 B growth safe), SUB-C4 (sliced universe only), SUB-C6 (`add_bls_key` deletion not repair — H6), SUB-C7 (empty commits to a canonical value — D6); Restructurer C1–C8 (core never depends on storage; universe only at apply; header hash un-gateable; encoder/validator source unification is consensus-visible; bitfield producer-local; `.is_some()` admission — H1; ingress precondition uniformity — D4; no growth of over-budget files — O4/R4); Pattern Matcher C1–C8 (pool key bijection; per-signature validation; no producer-controlled guard; preimage frozen at Release N; `≤→==` ordering; PoP soundness — precondition P8; `crates/crypto` gains no distinct-message `aggregate_verify`; finality decoupled); Radical C1–C8 (message frozen at N; `attested` map byte-identical in N; verifier inline in the funnel — satisfied after X5; keys via `get_by_pubkey`; commitment is a DoS hardening whose test asserts the relay property; garbage-sig rejection before aggregation; rewards/RPC denominator unchanged — O2(b); `pks_validate=false` DEAD).

## SSF Candidate and Radical Tiebreaker

**SSF statement (Radical Simplifier, adopted):** "Make the attester put its BLS signature over the parent hash on the gossip wire, key the tracker by parent hash instead of by minute, and add one `bls_verify_aggregate` call next to the existing `presence_root` check behind one activation height — everything else the feature needs is already in the codebase."

**Amendment forced by the failure filter (three edits, ~10 lines):** the verify call sits after `validate_block_with_mode` and is skipped-and-counted in Light mode (F2 snap-sync deadlock, F11 pre-authentication DoS); the post-AH commitment is unconditional with a length prefix and a canonical empty value (F9). The Radical design AS WRITTEN is VULNERABLE to F2/F11/F9: conf(0.68) − 0.15 − 0.10 − 0.10 ⇒ conf(0.33, inferred). The AMENDED minimum = **D1–D8 exactly**.

| Metric | Current (main) | Radical minimum (amended = D1–D8) | Proposed (D1–D8 + R1–R5) |
|---|---|---|---|
| Non-test files touched | — | 14 (N: 7, N+1: 7) | 17 (+ `core/attestation/` dir, `node/attestation/`) |
| New files | 0 | 1 (`bitfield_universe.rs` or `core/attestation/universe.rs`) | 8 (5 core split + 3 node), each < 300 lines |
| Deleted files | 0 | 0 | 1 (`presence.rs`, 798 lines) |
| New non-test functions | — | +7 (`bls_attest_msg`, `attestation_universe`, `presence_root_preimage`, `record_attendance`, `record_bls`, `bls_for_parent`, verifier branch) | +9 (+ pure `verify_block_aggregate`, `ordered_bls_keys`) |
| New types | 0 | 0 (pool as tracker fields) | +1 (`ParentSignaturePool`) |
| Crates touched (non-test) | — | 2 (`core`, `node`); `crypto`/`rpc`/`wallet`/`cli` untouched | 2 (same) |
| Non-test LOC added / deleted / net | — | ≈ +200 / −175 (D1 + dead sink) / **≈ +25** | ≈ +330 / −1,130 (R2 −798, R3 −80, D1 −145, moves) / **≈ −800** |
| Test LOC (estimate) | — | ≈ 370 + harness ≈ 350 | ≈ 450 + harness ≈ 350 |
| Bitfield decoders / live denominators | 1 enc + 5 dec + 1 validator / **3** (X2) | 5 dec / **2 post-AH** (universe for encoder+post_commit+validator; base-only rewards; RPC display), 3 pre-AH | same (O2(a) would make it 1 fn + 2 projections) |
| AH fields | 25 | 26 | 26 |
| `HardForkSchedule` entries / `CURRENT_PROTOCOL_VERSION` | 0 / unchanged | 0 / unchanged | 0 / unchanged |
| `BlockHeader::hash()` fields | 12 | 12 | 12 |
| Bytes / block | +0 | +96 B | +96 B |
| Gossip bytes / attestation | 156 B | 252 B (+96) | 252 B |
| Verify µs/block @ N=45 / N=1000 | 0 | 869 / 9,103 (measured, M4 Max, naive) | same (+≈0.1 µs prefix hash) |
| Producer aggregate µs/block @ N=45 / N=1000 | 0 | 1,334 / 29,371 (measured) | same |
| Ingress µs/attestation | ≈50 (Ed25519) | +488 (measured) | same |
| Confidence | — | **conf(0.80, converged)** | **conf(0.80, converged)** |

**Confidence derivation.** Every D-item is ≥ 0.88 converged, but the design as a whole is capped by the one unknown all five evaluators flagged and none measured: attestation arrival timing under parent-only semantics (F4). Both consumers union across blocks (`rewards.rs:155-157`; `post_commit` epoch accumulation; 3-epoch `attested_union`), so per-block sparsity does not propagate to per-minute attendance unless none of ~6 builders receives any of a producer's ~6 attestations — a structural floor, not a measurement. REQ-BLS-009 + C17 is the gate. R1–R5 add no runtime risk and no runtime benefit; their value is comprehension and module budget, so they do not move the confidence. **Gap = 0.00 < 0.1 ⇒ the Radical Tiebreaker applies: the orchestrator presents the amended minimum (D1–D8) ALONE first; R1–R5 and O1–O6 follow only on request.** R1 is the one recommended item that must be DECIDED before M2 ships even if not adopted (the message must be frozen either way).

## Constraints (from the Failure Analyst) any chosen path must handle

C1–C20 verbatim in `docs/.workflow/design-failures.md`; applied above. Standing hazards not closed by any D/R item: F3 (no in-protocol key recovery — O5 + Preconditions), F4 (inclusion window 60 s → 10 s for slow producers; structural union floor; C17 replay), F5 (old-binary producers at the AH — Deploy Shape), F6 (attendance collapse "just above the floor" is silent — M4 adds the fallback-rate metric with an alert threshold tied to the epoch filter), F14 (duplicate pubkeys via `legacy_fallback`, `floor.rs:126-141` — M3 assertion), F15/C19 posture preserved.

## Architecture Maps

### Current (main, `c3d9e827`)
```
ATTESTER  post_commit.rs:446 ─► startup.rs:591 Attestation::new (Ed25519 over hash‖slot; bls_signature EMPTY)
          post_commit.rs:450-457 bls_sign(hash‖slot) ─► minute_tracker.bls_sigs  [0 readers]   ✗ dead work
          startup.rs:605  w == 0 → None                                                          ✗ H1
INGRESS   A1 network_events.rs:558 (block must be local)   A2 :345 (block may be unknown)
          verify() Ed25519 ─► derive_attester_weight(LOCAL ProducerSet).is_some() ─► record / record_with_bls (unverified blob)
TRACKER   attested: pk→{minute}  (1 reader: encoder)     bls_sigs: (pk,minute)→Vec<u8>  (3 writers, 0 readers)
ENCODER   assembly.rs:389-455  universe copy A = [producer_list | active_at(h)\base sorted]; presence_root = BLAKE3(bitfield) or Hash::ZERO; aggregate = Vec::new()
VALIDATE  validation_checks.rs:422-446  if !bitfield.is_empty(): root == BLAKE3(bitfield); stray bits ≤ active_at(h).len()   ◄ 3rd source, before VDF
          :448 validate_block_with_mode → header, size cap, VDF (:244), eligibility (:247)
DECODE    post_commit.rs:34-57 copy B [base|extra] → attested_sets → producer_list (INV-EPOCH-004)
          rewards.rs:139/814/1016 base-only → 54/60 rewards, 30/60 demotion (MONEY)      rpc/schedule.rs:230-256 [base|extra], display
DEAD      presence.rs (798), RegionAggregate, new_with_bls, bls_sigs_for_minute, bls_sig_count, Hash codec + 6 `h<0` arms
```

### Proposed (D1–D8 + R1–R5)
```
core/attestation/{wire,bitfield,tracker,universe,commit}.rs   (pure; no storage types)
  bls_attest_msg(parent_hash) = parent_hash                                   [R1, frozen at Release N]
  attestation_universe(base, active_at_h) -> [base | extra sorted]           [D5, ONE definition]
  presence_root_preimage(bitfield, aggregate, post_ah) -> Hash               [D6, len-prefixed post-AH]
  verify_block_aggregate(bitfield, aggregate, ordered_keys, msg) -> Result   [pure math]
  AttendanceTracker{attested}  +  ParentSignaturePool{by_parent, recent: K=8} [D2]
bins/node/src/node/attestation/{mod,keys,egress}.rs                       (the only layer seeing core + storage)
  egress.rs   ONE create_and_broadcast_attestation → Attestation::new_with_bls(bls_attest_msg(hash))  [D3]
  mod.rs      Node::record_attendance(&att): is_some() ∧ len==96 ∧ bls_verify ⇒ record + pool.insert  [D4, both ingresses]
  keys.rs     ordered_bls_keys(universe, set) via get_by_pubkey → 48-byte keys                        [D7]
ENCODER   pre-AH: attested_in_minute (byte-identical)  |  post-AH: pool.bls_for_parent(prev_hash); indices via attestation_universe; agg = bls_aggregate; root = preimage(..)
VALIDATE  :422 cheap commitment (pre-AH verbatim | post-AH unconditional, canonical empty)            [D6]
          :448 validate_block_with_mode (VDF, eligibility, size cap)
          NEW  if post_ah && mode==Full { universe; stray/width; keys; bls_verify_aggregate; VERIFIED/REJECTED.inc } else if post_ah { SKIPPED_DIVERGENT.inc }   [D7, R5]
DECODE    post_commit + rebuild → attestation_universe (same fn)    rewards ×3 base-only (== universe[..base_len], test-locked, O2)    RPC display unchanged
GATE      inc_i_178_attestation_bls_activation_height  (mainnet u64::MAX | testnet pinned at rollout | devnet 0)   [D8]
```

## Deploy Shape

- **INV-12:** (1) can a user tx reach it? **YES** — `Registration` supplies the verification key and changes `active_at(h)`. (2) producer/attestation pattern? **YES** — every block. (3) bit-identical for all reachable inputs? **NO** — bit semantics, `presence_root` preimage, new rejection paths. **⇒ AH REQUIRED.** Rules change: **YES**. Content change: **YES** (`presence_root` is inside `BlockHeader::hash()`; INV-DEPLOY-001 / INC-I-062). No `HardForkSchedule`; no `CURRENT_PROTOCOL_VERSION` bump (`EpochState` layout unchanged).
- **Release N** (D1, D2, D3, D4, R1 frozen message, R2, R3, R4): rules NO, content NO — gossip bytes only; `attested`/minute map byte-identical; encoder untouched; `presence_root = BLAKE3(bitfield)` unchanged ⇒ **rolling-safe, no AH, no stop-all.** Mainnet ships with `inc_i_178_attestation_bls_activation_height = u64::MAX` already declared (D8 may land here inert) — pinning is a separate decision session.
- **Release N+1** (D5 denominator switch, D6, D7, R5, encoder post-AH branch): rolling-safe strictly below the AH (every post-AH behaviour sits behind `height >= AH` with the current expression verbatim in the `else`). **Pin only after** the Release-N soak and the Preconditions below, first on testnet, then mainnet in a separate release.
- **At activation, old-binary producers:** they compute `BLAKE3(bitfield)` and reject every attended post-AH block (F5) but ACCEPT empty-bitfield blocks (their `is_empty()` guard) — a messy partial follow, not a clean stop; their own blocks (old preimage, empty aggregate) are rejected by the upgraded majority and orphaned; they stay connected (no `MIN_PEER_PROTOCOL_VERSION` bump) so the only signal is `[BLOCK] REJECT` volume. Mainnet has ~30 external auto-update producers and no stop-all: **the AH margin is the sole safety mechanism and MUST come from measured auto-update convergence telemetry (C16).**
- **M4 as landed (deviation, INC-I-178 run 544):** the validator applies the D6 preimage only when the block CARRIES an aggregate; a post-AH block with an empty aggregate is still checked against `BLAKE3(bitfield)`. Stripping or altering a real aggregate therefore still fails the commitment (REQ-BLS-003 AC-1 holds), but the old-binary block shape (old preimage, empty aggregate, bits set) is ACCEPTED by the M4 commitment check and must be refused by the D7 aggregate verifier in M5, which lands before any pin. Required by `bins/node/tests/it/inc_i_178_m3_midepoch_exit.rs` P2, which validates a legacy-shaped block at the AH.
- **Snap-synced nodes (INV-EPOCH-002/003):** post-snap-sync `producer_list` is the unfiltered active set (`block_handling.rs:414-418`), so index→key ordering is provably divergent even though `bls_pubkey` values are present (ProducerSet transfers as state). The verifier's universe is sourced ONLY from live `epoch_state.producer_list` + `ProducerSet.active_producers_at_height(h)` at apply time (Restructurer C2); while `snap_sync_height.is_some() || floor_fallback_window` the node is in `Light` mode, the aggregate check is skipped and counted (C3), and it begins verifying once the next epoch boundary rebuilds the list. The rebuild routes through the same universe fn and never verifies (C13).

## Preconditions Before Pinning (testnet, then mainnet)

| # | Precondition | Status now |
|---|---|---|
| P1 | On-chain `bls_pubkey` coverage: EVERY ProducerSet entry has a well-formed 48-byte key (C6; GS-012 is blind to external producers and to empty keys) | **Mainnet: SATISFIED at h=369286** (epoch 1025, v6.26.1, read-only `getProducers` via the documented explorer proxy, 2026-09-03; probe: `docs/.workflow/bls-pubkey-coverage-probe.md`): 104 producers (92 active, 12 exited), 0 empty, 0 non-48-byte, 0 duplicates; the 5 genesis producers are keyed and all exited. **Testnet: SATISFIED at h=103356**: 7 producers (5 active), 0 empty. Presence ≠ match: the residual unknown is post-registration key MISMATCH, which only a live signature check (P2 / GS-012) can measure. Re-probe at pin time (the ProducerSet is a moving set). |
| P2 | GS-012 (`scripts/gauntlet.sh:424`, wallet-vs-chain key match) at 100 % on testnet AND mainnet | not run in this cycle |
| P3 | Release-N soak: every ACTIVE producer observed emitting a verifying 96-byte BLS attestation for ≥ 1 epoch (the only proof the operator holds the secret) | **MEASURABLE since M7.5, still UNMEASURED.** M7.5 shipped the observable P3 was missing: the ingress `BlsAttestVerdict::Valid` arm publishes `doli_attestation_bls_valid_attester_total{attester="<8 hex>"}` on FIRST-SEEN halves only, plus the zero-initialised `doli_attestation_bls_valid_total` capability marker, and `scripts/gauntlet-gs018.sh::_gs018_dual_check` joins that label by prefix to `getProducers` `status == "active"`. The verdict is still SKIP until a fleet carrying the M7.5 build has ingested ≥ 1 verifying half; the ≥ 1 epoch soak requires Release N. |
| P4 | Key-rotation decision recorded (O5): rotation TxType shipped, or written acceptance that key loss = permanent removal | open |
| P5 | REQ-BLS-009 replay over ≥ 1 healthy AND ≥ 1 degraded testnet epoch (C17): qualifier delta = 0 or explained at BOTH thresholds (54/60 rewards, 30/60 demotion) | built (M6): `bins/node/tests/it/inc_i_178_m6_replay*.rs`, synthetic healthy delta = {} at both thresholds, degraded Reward54 delta = one producer sitting exactly on the threshold. Real testnet capture is M7. |
| P6 | AH margin from measured auto-update convergence (C16) | no telemetry cited |
| P7 | Bulk-sync verify cost measured on x86 fleet hardware (C18; bench is ARM64) | ARM64 only |
| P8 | Every `bls_pubkey` write path is PoP-covered or copies a PoP-verified key: `genesis_completion.rs:134`, `rewards.rs:1302,1550` (Pattern Matcher C6 gap) | unverified |

## Migration Path / Milestones

All milestones are TDD (regression tests that lock CURRENT behaviour land before the change). "Gate-derived" = read the gate through `Network::X.params()` and drive `gate-1` / `gate` (rule `18779b1e`, INV-GOV-001).

| M | Scope | AH | Key tests |
|---|---|---|---|
| **M0** | Lock current behaviour: encode→decode identity at all 5 decode sites over a replayed testnet epoch; `presence_root == BLAKE3(bitfield)` pre-AH; `attestation_authority_tests.rs` (3) green with unchanged `total_entries`; `m_rc9_silent_vec_regression.rs` green; property `attestation_universe(base, active)[..base.len()] == base` | none | regression only |
| **M1** | D1 + D2 + R2 + R3 + R4 (deletions, pool, module split) | none | zero-caller assertions; pool bound test (K=8, memory ≤ 8·N·96 B); tracker footprint = f(`attested`) only; existing bitfield harnesses green |
| **M2** | R1 message frozen (`bls_attest_msg`) + D3 dual-sign in ONE egress + D4 shared ingress verify + peer-scoring budget | none | **C1 halt mutation test**: relay flips one byte of `bls_signature` → dropped, honest signature retained, peer scored; garbage-BLS from a registered member → blob dropped, attendance kept; wire-compat: old decoder tolerates 96-byte field; C19 posture tests (both ingresses `.is_some()`); `delegated_bond_attestation.rs` green. **⇒ Release N ships after M2; soak begins (P3).** |
| **M3** | D5 universe fn (extraction, bit-identical) + D8 AH field (three defaults, env_loader refusal, ledger test) + C5/F14 duplicate-free assertion | field declared; behaviour still pre-AH | index-parity divergence test across ALL decoders for `producer_list \ active_at(h) ≠ ∅` (REQ-BLS-014); gate-derived tests on BOTH sides of the AH; mixed-fleet pre-AH hash-equality test |
| **M4** | D6 preimage fn + encoder post-AH branch + D7 verifier (after `:448`, Light-mode skip + counters registered AND incremented — INC-I-187) + O4(a) module home | gated | forged bit through `validate_block_for_apply` → REJECT; stripped/mutated aggregate → REJECT; valid bitfield + empty aggregate → REJECT; empty bitfield + garbage aggregate → REJECT; empty bitfield + empty aggregate → ACCEPT; snap-sync Light-mode skip counted; verify-after-VDF ordering test (invalid VDF never reaches the pairing) |
| **M5** | R5 exact width (after M3's divergence test) + REQ-BLS-009 replay harness (healthy + degraded epoch, both thresholds) + C11 chaos test (half the fleet stops BLS-signing → production continues, fallback rate metric moves) + gauntlet GS-018 bitfield integrity (REQ-BLS-013) + GS-019 aggregate poisoning | gated | as listed |
| **M6** | REQ-BLS-015 docs: `specs/protocol.md:1159,1161,1487-1488`, `specs/security_model.md:629-630`, WHITEPAPER §10.3 hotfix (EN+ES, mirror to `../explorer/`), `block.rs:26-28,159-164`, `attestation.rs:370-374` comments | none | doc-alignment check |
| **Pin** | Separate decision session: testnet pin → ≥ 3 epochs soak → mainnet pin in its own release after P1–P8 | — | AH ledger test updated |

- BRIDGE: during the Release-N mixed-fleet window an attestation with an EMPTY `bls_signature` (old binary) is still recorded for minute attendance (no BLS verify possible) but never enters the parent pool; post-AH such a producer earns no bit. This tolerance is transitional and is what P3 measures; it is removed from the design's expectations once the soak shows 100 % dual-signing (it costs nothing to leave the code path, since `#[serde(default)]` must remain for wire compatibility).
- **Estimated scope:** 7 milestones (M0–M6) + pin session; ≈ 17 non-test files across 2 crates (`core`, `node`); ≈ +330 / −1,130 non-test LOC (≈ +200 / −175 for D1–D8 alone); ≈ 450 test LOC + ≈ 350 harness/gauntlet; AH-free: M0–M2, M6 (and M1's deletions); AH-gated: M3 (declaration), M4, M5.

## Non-Foreclosure Statement

The design re-creates none of the three dead-ends: (a) *a field with no validator* — `aggregate_bls_signature` gains exactly one validator on `validate_block_for_apply`, the function every applied block passes through (`apply_block/mod.rs:110`), and the commitment makes stripping detectable; (b) *a validator with no caller* — the check is a call inside that funnel, not a separable `validate_block()` entry point (`86bac138` lesson), and the `[ATTEST_VERIFY]` counters make zero executions observable (registered AND incremented, INC-I-187); (c) *a spec claim not enforced* — M6 rewrites the claims to match; `RegionAggregate` and `presence.rs` are deleted rather than re-described. The 1000-producer path stays open: the pairing is N-independent (measured flat), decompression is 8.4 µs/key (9.1 ms at N=1000), the cache (O1) is pure memoization behind one call with no consensus-visible byte and no AH, ingress verification carries a batch trigger at N > 500, and `crates/crypto` gains no distinct-message primitive (Pattern Matcher C7) so a repeat of the old incoherence is a compile error.

**Module-size check:** every NEW file is < 300 lines; `attestation.rs` (703) is split, not grown; `validation_checks.rs` (1614) net −25, `startup.rs` (667) net −55, `post_commit.rs` (487) net −12, `assembly.rs` (675) net ≤ +2 (post-AH branch +30 offset by universe extraction −20 and legacy arm −8; may be moved into `node/attestation/commit.rs` for net −28), `rewards.rs` (1586) net −16 (R3) or 0; the only over-budget files that GROW are `network_params/{mod,defaults,env_loader}.rs` (+12/+3/+6 for the AH field — unavoidable, CLAUDE.md "AHs go in NetworkParams", precedent INC-I-204). No file that is under 500 lines today crosses 500. Splitting the pre-existing over-budget files is separate debt, not this redesign.

## Out-of-Scope Findings to Hand Off (one proposed incident title each)

- **H1** — "Fully-delegated active producer never broadcasts attestations (`startup.rs:602-607` `w == 0 → None`): silent INV-EPOCH-004 removal reachable via `DelegateBond`; fix is consensus-visible (`producer_list` membership) and needs its own AH" (Restructurer CS-5/C6; independent of BLS).
- **H2** — "`rewards.rs:90-96` `filter_map(get_by_pubkey)` index-shift hazard: a `producer_list` key missing from the live ProducerSet shifts every later reward index" (Restructurer; money-bearing; unproven — `set_lifecycle.rs` not read in full).
- **H3** — "Genesis producers can carry an empty on-chain `bls_pubkey` (`genesis.rs:101-104`, `genesis_completion.rs:133-135`): permanent disenfranchisement post-AH with no recovery TxType" (Failure Analyst F3; feeds P1/O5).
- **H4** — "memory.db drift: INC-I-191/192 still `open` although `13daee6f` fixed both; decision #48 still `active` though superseded" (synth-verified via `incidents` table).
- **H5** — "Two attendance thresholds documented as one: `MIN_ATTESTATION_MINUTES=30` (demotion/tier, `constants.rs:99`) vs `ATTESTATION_QUALIFICATION_THRESHOLD=54` (reward Tier 1, `attestation.rs:235`); brief/analyst say 54/60, Failure Analyst says 30/60 — both live, docs must name both" (X4).
- **H6** — "`Wallet::add_bls_key` (`crates/wallet/src/wallet.rs:401-414`, `bins/cli/src/wallet.rs:493-506`) mints a random, phrase-unrecoverable BLS key; the struct persists no phrase so it cannot be repaired — delete both copies + 3 call sites" (Subtractionist P4, X9; INV-KEY-001).
- **H7** — "`BITFIELD_BODY_ACTIVATION_HEIGHT` is a compile-time constant (`constants.rs:63`), invisible to the AH ledger test and contrary to 'AHs go in NetworkParams'" (Pattern Matcher A5).
- **H8** — "INC-I-146: the attestation BLS store is NOT a contributor (≈ 9 KB, epoch-reset sawtooth); redirect the leak hunt to `attested: HashMap<PublicKey, HashSet<u32>>` and elsewhere" (X3).
- **H9** — "In-code doc drift: `block.rs:26-28` ('presence_root always Hash::ZERO'), `block.rs:159-164`, `attestation.rs:370-374` describe features that do not exist" (Restructurer, Pattern Matcher A6) — folded into M6 if this redesign ships, else its own ticket.
- **H10** — "Peer-scoring interaction at the AH: does an old-binary node's reject storm ban the honest majority (self-eclipse)?" (Failure Analyst gap; `crates/network/src/scoring.rs` unexplored).

## Design Synthesis Quality Gate

```
━━━ DESIGN SYNTHESIS QUALITY GATE ━━━
Evaluators completed:           5/5
Deletion convergence items:     2 (3+/5 agreement: D1 dead BLS store 5/5; the legacy era and presence.rs are 1/5 each → Recommended)
Restructuring convergence:      6 (D2, D3, D5, D6, D7, D8 all 4-5/5 with independent evidence)
Addition options presented:     6 (O1–O6)
Failure modes identified:       15 (F1–F15) + 20 constraints (C1–C20) from the Failure Analyst
Failure modes applied as filters: 20/20 constraints × 10 D/R items (table above); 15/15 failure modes referenced
Radical floor gap:              current (3 denominators, 0 verification, ~1,100 dead lines) → radical minimum amended (D1–D8, ≈ +25 net LOC) → proposed (+R1–R5, ≈ −800 net LOC); confidence gap 0.00 ⇒ radical presented alone first
Contradictions found:           10 (X1–X10)
Contradictions resolved:        10/10 (all by code evidence; X4 resolved as "both live")
Evidence independence verified: YES (per-item basis listed under Convergence; same-evidence cases — Pattern Matcher A8 and Failure Analyst decoders 2-5 inherited from the analyst — were NOT counted toward D5's independence)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```
