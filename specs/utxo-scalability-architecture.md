━━━ FINDINGS — 5 total (DECISION:5) ━━━

  [F1] DECISION conf(0.88, converged) — bins/node/src/node/fork_recovery.rs:303-373 — delete the double-deserialize / double-root / full-clone in snapshot install; derive the root once, from the installed state_db backend (5/5 lenses)
  [F2] DECISION conf(0.85, converged) — crates/storage/src/state_db/queries.rs:483-500 — replace `serialize_canonical() -> Vec<u8>` with one streaming fold: `canonical_digest()` + `canonical_range()`; bit-identical `utxo_hash`, Q1 NO / Q2 NO (4/5 lenses)
  [F3] DECISION conf(0.80, converged) — crates/network/src/protocols/sync.rs:22,101-132,294 — move the state-transfer bound from the codec to a session: manifest + sorted-key-range chunks over a pinned view, with cursor, halt-marker reuse and backend re-derivation (5/5 lenses; approved-spec Tier 3-A escalation)
  [F4] DECISION conf(0.75, converged) — crates/core/src/validation/utxo.rs:282-291 + transaction/output.rs:606 — close the zero-cost state-fill channel: remove `MintAsset`/`BurnAsset` from the fee-exempt set and make `fungible_asset_metadata()` exact-length; NEW `asset_tx_fee_activation_height` (2/5 lenses, synthesizer-verified)
  [F5] DECISION conf(0.75, converged) — crates/core/src/validation/block.rs:133 + transaction.rs:346-391 — one resident-byte accounting unit `resident_bytes(tx)` and a per-block cap on bytes that ENTER the UTXO set, exempting consensus-generated outputs; NEW `resident_bytes_cap_activation_height` (4/5 lenses; precondition-blocked)

  Speculative: 11 (report-only, not actionable)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

# UTXO Scalability Architecture

**Status:** STAGES 1-4 IMPLEMENTED on branch `feature/utxo-scalability-streaming` (2026-09-15), plus the M5 manifest-admission correction (INC-I-231): `0f11daca` [F1] install once, `d8a4b508` [F2] streaming canonical fold, `c552c8c0` [F3] chunked state session (wire side), `94b419d3` [F3] staged streaming install (client install side, M4). Stage 4 in the Milestones table is that install-side work; the Migration Path's stage 4 ([F4] asset fee) is a DIFFERENT change and is not implemented. Stages 5+ ([F4] asset fee, [F5] resident-byte cap, Options A-K) remain PROPOSAL-ONLY — no code. Original synthesis: 2026-09-14, `/omega-redesign utxo-scalability`, synthesizer over 5 design evaluators; the architect's discussion paper ran under run 558 (decision 124), the user's anchoring decision is decision 126.
**Reasoning trace:** `docs/.workflow/architecture-reasoning.md` (convergence matrix, 16-filter table, 12 contradictions, UNVERIFIED list).
**Inputs:** `docs/.workflow/design-{subtraction,restructure,patterns,failures,radical}.md`, `docs/.workflow/design-brief.md`, `docs/redesigns/utxo-scalability-redesign-analysis.md` (REQ-SCALE-001..024), `docs/redesigns/utxo-scalability-architect-position.md` (a CANDIDATE; three of its claims are FALSE, see §Contradictions), `specs/utxo-storage-architecture.md` (Approved 2026-06-03).

## Problem Statement

DOLI's consensus state (the UTXO set) is shipped to every joining node as ONE `bincode` frame capped at `MAX_SYNC_SIZE = 16 MiB` (`crates/network/src/protocols/sync.rs:22,294`; body at `crates/storage/src/snapshot.rs:238`), so snap-sync dies at ≈172,961 zero-payload UTXOs (97 B/entry, `utxo/types.rs:48-79`). Every state-root/snapshot computation materializes the whole set 2-3× in RAM (`state_db/queries.rs:483-500`), and the install path materializes it 5× (`fork_recovery.rs:303-373`). Nothing prices or bounds bytes that enter the set: the priced channel costs 0.00010486 DOLI/MiB (`transaction/core.rs:696-701`) and the unpriced channel (`MintAsset`/`BurnAsset`, fee-exempt at `validation/utxo.rs:282-291`, trailing bytes accepted at `output.rs:606`, ungated at `validation/transaction.rs:101-111`) fills ≈12.66 GiB/day for ≈0 DOLI and closes snap-sync from today's testnet state in ≈92 s. A government hash registry at 100k anchors/day would add 19.6 MB/day of permanent state under the current NFT/Hashlock anchoring path (analysis §4.4). Binding constraints: no genesis reset; every rule change activates at its own NEW `NetworkParams` height; ~30 external auto-update producers so no stop-all; anchors are one Merkle root per stack per block, provable-not-queryable, never in the UTXO set (decision 126); product layer out of scope.

━━━ RESOURCE COST — SUMMARY — COST-DECLARED ━━━
Dimensions:
  CPU:      -2 full BLAKE3 passes and -1 O(N log N) sort per snapshot install; +1 CF scan per state-root compute (count prefix); +O(outputs) integer adds per validated tx (observed)
  Memory:   install peak ≈5 full-set copies -> O(chunk) + one write batch; state-root serve -2x serialized set -> O(1); at N=1e6 that is -291 MB per compute (observed)
  IO:       +1 full CF_UTXO read per completed snap install (backend re-derivation, F-10); +1 extra CF scan per root compute; per-day cf_utxo write rate capped at C x 8,640 (inferred)
  Network:  same total snapshot bytes; +1 round trip per chunk (~1 per MiB) and +~100 B manifest; no change to block gossip (inferred)
  Disk:     transient staging CF + pinned SST retention during a stream; worst-case UTXO growth becomes C x 8,640 blocks/day instead of unbounded (inferred)
  Latency:  snap-sync becomes possible above 172,961 UTXOs (impossible today); +RTT x chunk_count for small sets; no per-block latency added in the apply loop (inferred)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: a one-shot request/response makes "the whole state fits in one message" a structural requirement, and no constant (REQ-SCALE-021 Won't) or compression (REQ-SCALE-020 Could) removes a structural requirement; the pricing stages are the only part a funded attacker cannot bypass
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Relationship to `specs/utxo-storage-architecture.md` (Approved 2026-06-03)

This redesign is NOT greenfield (the analyst's "zero failed approaches" §5 is contradicted by that spec). This spec **AMENDS and ESCALATES** it; it does not supersede it.

| Approved-spec item | Disposition here |
|---|---|
| Tier 3-A chunked snap sync — deferred "when monitor approaches 6-month warning" (`:47,95`) | **ESCALATED** as [F3]. The 12 MB trigger is unfit: the adversarial fill crosses the remaining headroom in ≈92 s (AP-5); escalation is on adversarial arithmetic, not on the gauge (testnet 2.18 MiB, 13.6%). |
| Tier 2-C streaming state root — deferred "until `serialize_canonical` > 500 ms" (`:46`) | **ESCALATED** as [F2], because [F3] cannot serve ranges without it and REQ-SCALE-014 (RAM ≤ 2 GB at 10M UTXOs) is unreachable otherwise. |
| Tier 3-B incremental UTXO hash (`:48`) | UNCHANGED — governed by `specs/state-root-commitment-architecture.md` Tier 1 (LtHash, own AH). Out of scope. |
| Tier 3-C ContentStore wiring (`:49`) | Option H proposes DELETION of `content_store.rs` (0 callers). Adopting H supersedes that deferral explicitly. |
| Constraints Preserved: canonical serialization, root formula, undo format, `UNDO_KEEP_DEPTH`, protocol/epoch versions, existing AHs, block format (`:55-64`) | **PRESERVED** by [F1]-[F5]. |
| Constraints Preserved: "Snap sync wire format and `MAX_SYNC_SIZE = 16 MB`" (`:57`) | **AMENDED**: the constant keeps its value and its job (it bounds 5 payload classes, INC-I-012 F13); the wire protocol gains 2 request + 2 response variants. The legacy single-frame variant is retained as a bridge. |
| Constraints Preserved: "`OutputType` layout, `extra_data` field", "Era growth schedule" (`:58,65`) | PRESERVED by [F1]-[F5]. Options B, D, E would contradict them and each requires an explicit supersession decision, never a silent override. |

## Evaluation Summary

| Evaluator | Lens | Top proposal | Confidence | Key finding |
|---|---|---|---|---|
| Subtractionist | removal | P1 delete 5 materializations in snap install | conf(0.62, measured) | REQ-SCALE-014 is unreachable by chunking alone; the InMemory detour is the prerequisite. `Transaction.extra_data` is the covenant witness — cannot be zeroed. |
| Restructurer | boundaries | R5 four authorities price wire bytes, none prices resident bytes | conf(0.68, measured) | Codec bound is serving as a state-size bound; serialization and digest are one call; same whole-set idiom in rollback/reorg; `atomic_replace` self-read hazard. |
| Pattern Matcher | patterns | P1 Anchor = OP_RETURN semantics, batching off-chain | conf(0.68, observed) | Approved spec already governs; 3 of 5 industry patterns are 2/3-built (blob tier, ContentStore, LtHash); AP-2 inverted SegWit; AP-7 fail-silent serializer. |
| Failure Analyst | failures | MintAsset zero-cost fill kills every `minimum_fee`-based pricing | conf(0.68, measured) | 16 filters F-01..F-16; chunking has 4 BLOCKERs unless pinned/cursor/halt/re-derive; new OutputType = undecodable message (F-13); "one site" is FALSE. |
| Radical Simplifier | minimal | "Stream, don't materialize; spend, don't store" (P2+P3+P1) | conf(0.65, measured) | 0 rules, 0 AH, ~265 LOC; ceiling ≈3.1M UTXOs (pessimistic); does NOT cover the adversarial channel (~34 min to 300 MB). |

## Convergence Matrix

| Change | Sub | Res | Pat | Fail | Rad | Count | Independent evidence? | Verdict |
|---|---|---|---|---|---|---|---|---|
| Delete whole-set materializations in snap install | P1 | S-3 | P2 | Q9 | P2/P3 | 5/5 | YES (install chain / boundary / geth analogy / RAM-claim disproof / first principles) | DEFINITE [F1] |
| Streaming canonical fold (digest + range) | P1 | R1 | (P2) | P3 | P2 | 4/5 | YES (callers / flat-hash property / verify pass / install) | DEFINITE [F2] |
| Chunked state-transfer session | (pre) | R2 | P2 | P2,P3 | P3 | 5/5 | YES (codec-vs-session / order parity / serve-lock analysis / const) | RECOMMENDED [F3] |
| Close MintAsset/BurnAsset unpriced channel | c5 | signal | AP-4 | P1 | — | 2/5 +2 signals | YES, same code sites read independently | RECOMMENDED [F4] |
| Per-block resident-byte cap (backstop) | c5 | R5 | P3 | Q4 | P4 | 4/5 | YES (authority table / fee recycling / undo analysis / ceiling math) | RECOMMENDED [F5] |
| tx-level `extra_data` is NOT the anchor carrier | P4 kill | R4 alt | AP-2 | — | c5 | 4/5 | YES | CONSTRAINT (verified) |
| Anchors must not accumulate in the UTXO set | — | R4 | P1 | P4 | P1/P5 | 4/5 goal / mechanism DIVERGES | — | OPTIONS A/B |
| `OutputType::Anchor` (new discriminant) | — | R4 | P1 | P4,F-13 | P5 (do NOT) | 2 for / 1 against | — | OPTION B |
| Self-spending anchor chain (0 chain LOC) | — | — | — | — | P1 | 1/5 | — | OPTION A (SSF) |
| Refundable per-byte deposit | — | — | P3 | — | — | 1/5 | — | OPTION C |
| EncryptedContent forward cap / residency table | P6 | — | P4 | — | P4 census | 2/5 | census UNVERIFIED | OPTION D |
| Freeze era-doubling of `max_extra_data_size` | P2 | — | AP-3 | — | — | 2/5 | conflicts REQ-SCALE-023 + approved spec | OPTION E |
| Canonical witness envelope (no trailing bytes) | P4 | — | — | F-02 | — | 2/5 | YES | OPTION F |
| Inclusion-proof RPC at submission + archive role | — | — | P5 | P5,F-14/15 | gap 4 | 2/5 | YES | OPTION G |
| Delete `mmr.rs` / `content_store.rs` (0 callers) | — | S-1 | signal | — | — | 1+1 | synthesizer-verified | OPTION H |
| Demote `UtxoSet::InMemory` to test-only | P5 | signal | — | — | c6 | 1/5 +2 | after [F1] only | OPTION I |
| Retire `atomic_replace` in rollback/reorg | signal | R3 | — | — | — | 1/5 | killed naive form | OPTION J |
| Delete dead `ConsensusParams::max_block_size` | P3 | — | — | — | — | 1/5 | synthesizer-verified 0 callers | OPTION K |

## Definite Changes (High Convergence)

- ARCHITECTURAL: [F1] Install a verified snapshot ONCE — delete the second `deserialize_canonical`, the InMemory `compute_state_root`, and the `iter_all()` clone in `apply_snap_snapshot`; derive the post-install root from the installed state_db backend.
    Convergence: Subtractionist P1, Restructurer S-3/install chain, Pattern Matcher P2, Failure Analyst Q9 (+F-10), Radical P2/P3 — 5/5, independent evidence.
    Evidence: `fork_recovery.rs:303` (root from bytes), `:331` (second deserialize of the same bytes), `:363` (second root from the InMemory copy), `:370` + `utxo/set.rs:279-284` (`.clone().collect()`), `:389` (InMemory copy discarded); `writes.rs:187-232` (one WriteBatch); synthesizer verified this session.
    Confidence: conf(0.88, converged)
    Seam eliminated: `compute_state_root_from_bytes` owns "verify" AND "decode" so its caller decodes again — return the decoded set (or stream it) from the verifier. The residual root check moves from the InMemory copy (which proves nothing the first check did not) to the installed backend (F-10: a stream/frame hash proves transport, not storage; `queries.rs:483-500` silently drops undecodable values). Q1 NO / Q2 NO — rolling. Blast radius: `fork_recovery.rs`, `storage/snapshot.rs` (signature; CLI consumer `cmd_snap.rs:242`), `utxo/set.rs`. apply_block / rollback / rebuild / sync-manager untouched. Forecloses nothing; prerequisite for [F3]'s install side and Option I. INV-SYNC-014 preserved (backend swap kept).

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      -1 full deserialize, -1 O(N log N) sort, -1 InMemory BLAKE3 pass; +1 backend BLAKE3 pass replacing it (observed at fork_recovery.rs:303,331,363)
      Memory:   install peak from ≈5 simultaneous full copies to wire bytes + one write batch (≈2 x S); at S=970 MB roughly 5 GB -> 2 GB (inferred, no RSS benchmark)
      IO:       +1 full CF_UTXO read after install for backend re-derivation (observed cost shape at queries.rs:483-500)
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  install wall-clock drops by the removed passes; utxo/chain_state write guards held for a shorter window, narrowing the INC-I-143 cascade window (inferred)
    Inevitability: AVOIDABLE
    Cheaper alternative: delete the redundant passes and keep trusting the InMemory-derived root — smaller diff, but installs a set nobody re-serialized from the durable backend (F-10)
    Why this proposal anyway: it is the only change that lowers install RAM without touching consensus, the wire format, or the 23-incident sync manager, and it makes REQ-SCALE-014 reachable, which chunking alone does not
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: [F2] Split `serialize_canonical() -> Vec<u8>` into one streaming fold `fold_canonical_utxo<F: FnMut(&[u8])>` exposed as `canonical_digest() -> Hash` and `canonical_range(start_key, max_bytes) -> (bytes, next_key)`; delete `serialize_canonical()` when its last caller (`snapshot.rs:236`) moves to the range API.
    Convergence: Restructurer R1, Radical P2, Subtractionist P1 (iter_all → streaming), Failure Analyst P3 (verification pass must be streaming) — 4/5, independent evidence.
    Evidence: `snapshot.rs:30-32` (`utxo_hash = hash(serialize_canonical())` — flat BLAKE3 over an ordered byte string, so chunked hashing is bit-identical), `queries.rs:483-500` (collect Vec, then second Vec), `in_memory.rs:359-374` (the only sorting implementation), `set.rs:437-441` (byte-identity is consensus-critical), `utxo_size_monitor.rs:48` (caller wants `.len()` only). Verified (f) this session.
    Confidence: conf(0.85, converged)
    Seam eliminated: ORDER+ENCODING (consensus, INV-SYNC-007), DIGEST, MATERIALIZATION and TRANSPORT FRAMING are fused in one `Vec<u8>` signature; only the first is consensus, yet touching the others looks like a consensus change. Q1 NO / Q2 NO — rolling. Cost honestly stated: the 8-byte count prefix precedes the body and MUST NOT come from `utxo_len()` (STOR028, `queries.rs:476-482`), so streaming is two CF passes (count, then stream). Binding test: `canonical_digest() == hash(serialize_canonical())` over a randomized ≥100k set on BOTH backends before the old function is deleted. Sub-item: replace the silent `filter_map(ok())` at `queries.rs:486-491` with a hard error (AP-7 / Q9) — Q1 NO on valid inputs; a corrupt node fails loud instead of emitting a self-consistent wrong root. Forecloses nothing; precondition for [F3] and the state-root spec's Tier 1 digest.

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      unchanged hash work; -N bincode-to-struct round trips and -1 full Vec copy per root compute; +1 CF scan for the count prefix (observed at queries.rs:483-500)
      Memory:   -3 x 97 B x N peak per state-root compute on the RocksDb backend, -2x serialized set on InMemory; at N=1e6 that is -291 MB (observed)
      IO:       +1 extra full CF scan per root compute (2 passes instead of 1) (observed: queries.rs:476-482 forbids utxo_len())
      Network:  0 (observed)
      Disk:     0 (observed)
      Latency:  +1 CF scan per root compute (≈+0.3 s at N=1e6 on a 300 MB/s scan, assumed disk figure); root is lazy/memoized (M2 shipped, state_update.rs:38-40) so this is per-serve, not per-block (inferred)
    Inevitability: AVOIDABLE
    Cheaper alternative: add only canonical_digest() and keep serialize_canonical() for the snapshot — one function cheaper, but snapshot.rs:236 then cannot chunk and [F3] is impossible
    Why this proposal anyway: three of four callers pay a full-set allocation for a 32-byte answer, the fused signature is what makes a transport fix look like a consensus change, and the byte sequence (hence utxo_hash) is unchanged
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Recommended Changes (Medium Convergence)

- ARCHITECTURAL: [F3] Move the state-transfer bound from the CODEC to a SESSION: `GetStateManifest{block_hash}` → `{state_root, cs_bytes, ps_bytes, utxo_count, utxo_bytes_total}` and `GetStateChunk{block_hash, start_key, max_bytes}` → `{entries, next_key}`, served from a pinned RocksDB view keyed by `block_hash`, installed via a staging CF promoted atomically, verified by re-deriving the root from the installed backend.
    Convergence: Restructurer R2, Pattern Matcher P2, Radical P3, Failure Analyst P2+P3 (as constraints), Subtractionist (as prerequisite ordering) — 5/5, independent evidence; this is the approved spec's own deferred Tier 3-A.
    Evidence: `sync.rs:22,268,294` (plain `const`, in no block/root/signature), `sync.rs:101-132` (9-field one-shot variant), `sync.rs:28-85` (paging idiom already present), order parity `in_memory.rs:360-363` vs `queries.rs:486-491` (verified by Failure Analyst Q9), `state_snapshot_serve.rs:67-81` (three read guards held across O(N)), `snap_sync.rs:200-208` (F4 Gate 1 exact root equality), `snap_sync.rs:122-132` (no cursor), `snap_sync.rs:265-269` (blacklist on any error), `scoring.rs:187,228` (0 production callers — verified), `writes.rs:201` (delete-all before rewrite), `fork_recovery.rs:388-413` (halt marker), duplicate serve handler `state_snapshot_serve.rs:81` + `event_loop.rs:544` (verified).
    Confidence: conf(0.80, converged) — 0.85 by count, −0.15 for the naive form failing F-06..F-11/F-16, +0.10 for the constraints below resolving them. Lands in the #1 hotspot (`sync/manager/mod.rs`, critical, 23) with OPEN INC-I-143.
    Binding design constraints (each is a filter, not a preference): F-06 pinned view (`db.snapshot()` or short-lived `Checkpoint`, TTL + concurrent-serve cap); F-07 cursor in `SyncPipelineData::SnapDownloading`; F-08 stale-chunk ≠ equivocation; F-09 never cite peer scoring as mitigation — equivocation costs bandwidth, bounded by manifest cross-check against ≥2 quorum peers; F-10 root from installed backend; F-11 reuse `rebuild_in_progress`/`rebuild_halt_reason`, half-installed node refuses to serve; Restructurer c2 — write chunks to a staging CF and promote, never stream-read the CF `atomic_replace` deletes; c7 `MAX_SYNC_SIZE` value unchanged; F-16 keep the 3-attempt cap semantics and add no re-entry path; testnet soak with a seed under repeated snap requests before mainnet. Download stage only — NO admission/quorum logic change. Collapse the duplicate serve handler while there. Q1 NO / Q2 NO — rolling with negotiation: old peer answers `SyncResponse::Error` → legacy single-frame path (BRIDGE, see Migration). Blast radius: `protocols/sync.rs`, `sync/manager/snap_sync.rs`, `state_snapshot_serve.rs`, `event_loop.rs`, `fork_recovery.rs`, `state_db/queries.rs`. apply_block / rollback / rebuild / validation: zero. Forecloses nothing (per-chunk consensus proofs need the state-root spec's merkelized component; not required here).

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      serve -O(N) per request -> O(chunk) per chunk; receiver +1 incremental BLAKE3 update per chunk, same total bytes hashed; +2 constant-time pin/release calls per stream (observed at queries.rs:483-501, inferred for pinning)
      Memory:   serve peak -2x serialized set -> O(max_bytes); install peak -O(N) -> O(chunk) + one per-chunk write batch (observed at fork_recovery.rs:300-375)
      IO:       same total bytes; writes batched per chunk into a staging CF then promoted (~2x write amplification during sync only); pinned SSTs defer compaction for the stream lifetime (inferred)
      Network:  same total bytes; +1 round trip per chunk (≈1 per MiB) and +~100 B manifest (inferred)
      Disk:     +transient staging CF (size of the snapshot) and +superseded-SST retention bounded by TTL x write rate (assumed, not measured)
      Latency:  snap-sync at 10M UTXOs: impossible today -> ≈97 s at 10 MB/s (inferred); block production no longer blocked by a full O(N) serialization under three locks (observed at snapshot.rs:229-262)
    Inevitability: INEVITABLE
    Cheaper alternative: NONE-EXISTS
    Why this proposal anyway: raising MAX_SYNC_SIZE is REQ-SCALE-021 Won't and re-opens the INC-I-012 F13 allocation DoS; zstd is REQ-SCALE-020 Could and keeps the O(N) frame shape; a one-shot request/response structurally requires the whole state in one message and only a session removes that
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: [F4] Every state-creating transaction passes the byte fee and every typed `extra_data` parser is exact-length: remove `MintAsset`/`BurnAsset` from `fee_exempt` and change `fungible_asset_metadata()` from `meta.len() < 42 + ticker_len` to equality, behind NEW `asset_tx_fee_activation_height`.
    Convergence: Failure Analyst P1 (Q1, 0.68 measured), Restructurer R5 straddle #2 + cross-lens signal; Subtractionist c5 and Pattern Matcher AP-4 supply the "fee is not a bound" caveat — 2/5 proposing, all five gates re-verified by the synthesizer this session.
    Evidence: `validation/utxo.rs:282-291` (exempt set), `output.rs:606` (`<`, not `==`), `validation/transaction.rs:101-111` (only the 4 AMM types are gated; `defi_activation_height` is never read by the validator — verified), `validation/utxo.rs:271-276` + `core.rs:707-713` (native conservation trivially passes), `tx_processing.rs:141-162` (duplicate-id check covers NFT/Pool only). Arithmetic: 3 × 512 KiB × 8,640 blocks = 12.66 GiB/day for ~1 base unit/tx.
    Confidence: conf(0.75, converged)
    Class of bug removed by construction: an output type whose parser tolerates trailing bytes plus a tx type exempt from the only byte-priced rule is a free permanent-state channel; after this change `MintAsset` needs a native input to pay `1 + bytes/100` (UX change, not a capability loss — REQ-SCALE-005 holds), and padding beyond the typed layout is invalid. Q1 YES → `asset_tx_fee_activation_height` (NEW field, `u64::MAX` on devnet until pinned; never bundled with [F5]'s field — INC-I-054). Q2 NO (validation only; builder mirrors) → rolling before the AH. INC-I-075: (1) user tx reaches it YES; (2) producer/attestation NO; (3) bit-identical NO → AH required. F-12: wire at all 7 `ValidationContext::new` sites (`tx_processing.rs:61`, `validation_checks/mod.rs:106,298`, `assembly.rs:211`, `pool.rs:512,935`, `rotation_filter.rs:32`) with a test that fails on a miss; note `rotation_filter.rs:32` hardcodes mainnet params (live defect, outside scope). F-05 apply→rollback→re-apply root identity at AH−1/AH/AH+1 on both backends. Existing padded assets are grandfathered (stay resident). Blast radius: `validation/utxo.rs`, `transaction/output.rs`, `network_params/`, mempool + builder mirrors. apply_block / rollback / snap-sync: zero. Forecloses nothing (REQ-SCALE-010 composition path keeps working with a fee).

    ━━━ RESOURCE COST — NEGLIGIBLE ━━━
    Dimensions:
      CPU:      0 (observed: one integer compare per asset tx and one length equality per FungibleAsset output, inside a pass that already walks every output)
      Memory:   0 (observed)
      IO:       0 (observed)
      Network:  0 (observed)
      Disk:     0 (observed: forward growth from this channel becomes fee-priced; no on-disk format change)
      Latency:  0 (observed)
    Inevitability: INEVITABLE
    Cheaper alternative: NONE-EXISTS
    Why this proposal anyway: a mempool-policy rejection is bypassed by a self-building producer (policy.rs:28 is not consensus), and every pricing design in this round is void while these two types never reach minimum_fee()
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

- ARCHITECTURAL: [F5] One accounting unit for the resource the network pays for forever: `resident_bytes(tx) = Σ over state-resident outputs of (97 + extra_data.len())` defined in `crates/core` beside a single residency predicate, enforced as a per-block cap `Σ resident_bytes ≤ C` in block validation (the only authority binding on a self-paying producer), mirrored (never redefined) by builder and mempool, behind NEW `resident_bytes_cap_activation_height`.
    Convergence: Restructurer R5 (0.68), Radical P4 (0.50, census-blocked), Pattern Matcher P3 ("the cap remains necessary as the backstop"), Failure Analyst Q4 (a validation rule needs no undo state — F-03 satisfied), architect S3 — 4/5, independent evidence.
    Evidence: five wire-byte bounds and zero resident-byte bounds (`validation/block.rs:133`, `validation/transaction.rs:382-391`, `assembly.rs:168`/`constants.rs:449`, `policy.rs:28`, `core.rs:696-701`); 97 B/entry `utxo/types.rs:48-79`; fees fold into the coinbase → reward pool (`assembly.rs:370-378`) so a price is a partial refund to a producer-attacker; dust gradient ≈26 GB/day (architect A#3) is invisible to an `extra_data`-only cap.
    Confidence: conf(0.75, converged) — below Definite because four preconditions are unmet, not because the lenses disagree.
    Preconditions (all MUST precede pinning): (i) EncryptedContent mainnet occupancy census — UNVERIFIED by every lens, seed RPC closed to externals; a cap below the observed p100 bricks `doli nft mint`; (ii) exempt consensus-generated outputs (Coinbase, EpochReward, Registration bonds) or an epoch-boundary block under load is unbuildable — chain-halt class (R5 kill test); (iii) exempt or separately bound `ZKRollup` (100-400 KB proofs, `specs/l2-settlement.md`) or REQ-SCALE-008 is foreclosed; (iv) an economist sets `C` as if the DOLI price were zero and sizes it against block production (a producer exceeding its own cap misses its slot). Q1 YES → own NEW field. Q2 NO (validation; builder mirrors the budget). INC-I-075: (1) YES; (2) YES — a producer can build a violating block; (3) NO → AH mandatory + pre/post-AH block-equality test. F-12 at 7 sites; F-05 at AH±1. Height-aware rules must be honoured at the replay call site (`rollback.rs:311` passes `height`). Blast radius: `transaction/{core,output}.rs`, `validation/{transaction,block}.rs`, `production/assembly.rs`, `mempool/policy.rs`, `network_params/`. apply_block / rollback / rebuild / fork-recovery: none. Forecloses nothing structural; the price mechanism (fee vs deposit) is a separate Option.

    ━━━ RESOURCE COST — COST-DECLARED ━━━
    Dimensions:
      CPU:      +O(outputs) integer adds per validated tx and one per-block sum, inside a pass that already walks every output (inferred from validation/transaction.rs:346-391)
      Memory:   +8 B per block validation; worst-case resident growth bounded to C x 8,640/day instead of unbounded (inferred)
      IO:       per-day cf_utxo write rate capped by the same constant; no new reads or writes per block (inferred)
      Network:  0 (observed)
      Disk:     converts unbounded worst-case state growth into a stated per-year ceiling (e.g. C=64 KiB -> ≈553 MB/day, ≈202 GB/yr) (inferred)
      Latency:  0 in the apply loop; the sum runs in validation (observed)
    Inevitability: INEVITABLE
    Cheaper alternative: NONE-EXISTS
    Why this proposal anyway: a dust floor plus per-tx output cap prices COUNT not resident BYTES and leaves the 512 KiB vector open; a fee alone is recycled to the attacker through the reward pool; only a hard per-block bound makes worst-case growth a number an operator can plan disk for
    ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## SSF Candidate (Radical Minimum) — present this ALONE first

**"Stream, don't materialize; spend, don't store."** = [F1] + [F2] + [F3]-transport + Option A (self-spending anchor chain, 0 chain LOC). Zero consensus rules, zero AH fields, zero new output types, fully rolling. ~265 chain LOC as stated by the Radical Simplifier; ≈350 once F-06/F-07/F-11 (pinned view, cursor, halt reuse) are added — the Radical's P3 as written omits them and the Failure Analyst shows it cannot complete a single sync on a live chain without them.

| Property | Value | Basis |
|---|---|---|
| Ceiling, root computed on demand (M2 lazy shipped) | scan binds on serve, not per block; conservative planning floor ≈3.1M UTXOs (2 passes at 300 MB/s in a 2 s budget); disk binds at ≈97 GB / 1e9 entries | `utxo/types.rs:48-79`, `state_update.rs:38-40`; 300 MB/s is ASSUMED, not benchmarked |
| Benign headroom | ≈289 days at 10,610 UTXO/day (mainnet-scale extrapolation, UNVERIFIED) | analysis §4.2 |
| Registry anchors | consume 0 of that budget at any volume (Option A) | Radical P1 |
| **Adversarial channel it does NOT cover** | 12.66 GiB/day paid fill reaches 300 MB in ≈34 min; the `MintAsset` variant does it for ≈0 DOLI | Failure Analyst Q1, Radical §ceiling |
| Deviation from decision 126 | the CURRENT tip root per stack is a resident UTXO (historic roots are spent, in blocks); letter says "never enter the UTXO set" | Radical P1 risk line |

**Radical vs full proposal:** identical on the transport tier ([F1]-[F3] ARE the radical minimum plus the failure filters). On the bounding tier the radical (no pricing, conf 0.65) is within 0.1 of [F4]/[F5] (conf 0.75): **per the SSF rule the radical is presented alone first.** [F4]/[F5] are offered only because the Radical Simplifier's own report names the uncovered adversarial channel and calls the cap "the only place an AH is worth spending."

━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      -2 BLAKE3 passes and -1 sort per install; +1 CF scan per root compute; +1 Ed25519 verify per anchor tx (observed)
  Memory:   install ≈5 copies -> O(chunk); root compute -3 x 97 B x N; +196 B resident per registry stack (observed)
  IO:       +1 backend re-derivation read per install; +1 CF scan per root compute; +1 delete/+1 put per anchor instead of +1 put (observed)
  Network:  same snapshot bytes, +1 RTT per chunk; +~100 B per anchor tx (inferred)
  Disk:     transient staging CF during sync; UTXO CF unchanged by anchoring (inferred)
  Latency:  snap-sync possible above 172,961 UTXOs; +RTT x chunks for small sets; 0 in the apply loop (inferred)
Inevitability: AVOIDABLE
Cheaper alternative: [F1] alone plus zstd on the single frame (~20 LOC, 3-5x) — buys roughly 2 years and no structural change, but REQ-SCALE-013 requires independence from set size
Why this proposal anyway: it satisfies every MUST criterion with zero consensus surface and can be shipped and reverted without an activation height
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

## Options for User Decision (conf < 0.7 or divergent additions; all low-evidence tagged)

| Opt | What | From | conf | Q1 / AH field | Q2 | Filter result | Trade-off / what it forecloses |
|---|---|---|---|---|---|---|---|
| **A** | Self-spending anchor chain: each stack keeps ONE live UTXO; anchor N spends N−1 and carries the 32 B batched root in SIGNED `Output.extra_data` (`core.rs:531-534`, `output.rs:757-766`). 0 chain LOC. | Radical P1 | 0.62 measured | NO / none | NO | F-14 COND (historic roots live only in blocks → archive dependency identical to B); F-15 via G | Resident state = 196 B × stacks; correct only if stacks < ~100k (**stack cardinality is UNDEFINED in decision 126 — the single highest-leverage open question**). Requires the user to accept the tip-resident deviation. Forecloses nothing; B can be added later at no cost. |
| **B** | `OutputType::Anchor` (discriminant 16), zero-amount, ≤64 B, NEVER inserted: one `is_state_resident(&Output)` predicate consulted by ALL FOUR insert implementations (`batch.rs:197`, `in_memory.rs:82`, `set.rs:155`, `writes.rs:409`) AND the undo loop (`tx_processing.rs:193-196`); `oracle.rs:265` audited. | Pattern Matcher P1, architect S2, Restructurer R4 | 0.68 observed → 0.58 after F-13 (−0.15) and F-04 (−0.05), +0.10 REQ-SCALE-015 resolved | YES / `anchor_output_activation_height` | YES post-AH only | F-04, F-05, F-12 COND; **F-13**: old nodes `Reject` the block as undecodable (`gossip/validation.rs:102-105`) → deploy = rolling + verified version census + pre-AH relay refusal; BLS precedent is untested (no rotation tx ran on old binaries) | Saves 196 B × stacks of state; costs 5 mirrored consensus sites (the Full-Bitfield-Decode defect class; `BUG-001` precedent at `batch.rs:152-154`) and the strictest deploy class in this spec. Rollback benign today (`set.rs:224-236` checks existence before decrement — verified) but the undo record still lies without the predicate. Does not touch ZKSettle/ZKRollup (REQ-SCALE-008 OK). |
| **C** | Refundable per-byte deposit (Cardano/CKB): locked amount ∝ entry bytes, released on spend; replaces the FEE half of pricing, not the CAP. | Pattern Matcher P3 | 0.55 inferred | YES / `state_deposit_activation_height` | NO | F-01 PASS; F-05/F-12 COND; **kill test UNRUN**: a consensus-generated output (EpochReward share, coinbase change) below the floor halts the chain at the AH — `node/rewards.rs` never read | Survives fee recycling (a deposit is paid to nobody). Inexpressible for `amount==0` types (Pool/ZKRollup/OraclePrice) → exemption table. Needs economist + the minimum-share read before adoption. Alternative to C: architect's height-aware `minimum_fee_at(height)` — must name the 4 CLI sites' migration (`cmd_nft/buy.rs:242,531`, `transfer.rs:301`, `cmd_wallet.rs:1076`; wallet has no tip height). |
| **D** | Forward-only cap on `EncryptedContent` creation (commitment-sized) or a per-`OutputType` residency table with EncryptedContent as sole entry. | Subtractionist P6, Pattern Matcher P4 | 0.50 observed | YES / `encrypted_content_cap_activation_height` | NO | F-02, F-05, F-12 COND; residency-table form touches canonical bytes → contradicts approved spec `:55,58`, needs supersession | Directly satisfies REQ-SCALE-012 (the type IS on-chain file hosting, CLI-reachable via `doli nft mint`); breaks that CLI path; **mainnet occupancy census is a PRECONDITION** (UNVERIFIED by all lenses). Blanket residency rule is DEAD (`ZKSettle` reads whole `extra_data`, `validation/utxo.rs:917`). |
| **E** | Delete era-doubling of `max_extra_data_size`; freeze at 512 KiB (`output.rs:33-40`, one enforcement site `validation/transaction.rs:382`). | Subtractionist P2, Pattern Matcher AP-3 | 0.60 measured | NO for every height < 12,614,400 (bit-identical until era 1, ≈3.85 y); conservative: `extra_data_freeze_activation_height` | NO | F-05/F-12 COND | Stops the accept-vs-ship gap doubling every 4 y by deletion. **Contradicts REQ-SCALE-023 (Won't) AND approved spec Constraints Preserved `:65`** — requires explicit supersession of both. Forecloses >512 KiB outputs without a future mechanism (L2 proofs 100-400 KB still fit). |
| **F** | Canonical witness envelope: `Transaction.extra_data` must parse as exactly `inputs.len()` TLV witnesses, zero trailing bytes (`core.rs:648-684` never checks). | Subtractionist P4; Failure F-02 | 0.55 measured | YES / `canonical_witness_activation_height` | NO | F-02 RESOLVES; F-05/F-12 COND | Closes a free, UNSIGNED (`core.rs:512-536`), unpriced archive-bloat channel bounded only by block size. Does NOT fix AP-2 (inverted-SegWit malleability: bytes in `hash()`, out of `signing_message()`) — that is a separate incident-class finding. Wallet survey needed (UNVERIFIED). |
| **G** | `getAnchorProof(txid)` → `(header, merkle path, tx index, leaf_count)` generated AT SUBMISSION and handed to the registry; archive retention declared as a named node role. | Failure Analyst P5, Pattern Matcher P5 (converged, still an addition) | 0.70 converged | NO / none | NO | F-14 RESOLVES; F-15 COND (`compute_merkle_root` duplicates the last leaf on odd counts, `block.rs:258-282`) | Required by BOTH A and B for historic anchors — no inclusion-proof RPC exists (grep: 0 hits). Without it, a legal artifact depends on one operator's uptime (INC-I-143 D6 OPEN, seed RPC closed). Non-consensus; chain-side RPC only, product verifier out of scope. |
| **H** | Delete `crates/storage/src/mmr.rs` (330 L) and `content_store.rs` (187 L) — 0 production callers (verified). | Restructurer S-1, Pattern Matcher signal | 0.65 measured | NO | NO | all PASS | −517 LOC, removes the wrong accumulator (state-root spec chose LtHash) and an unwired dedup store; supersedes approved-spec Tier 3-C deferral explicitly. |
| **I** | Demote `UtxoSet::InMemory` (916 L) to `#[cfg(test)]` after [F1]; keep as the canonical-order oracle. | Subtractionist P5 | 0.50 observed | NO | NO | all PASS | Removes a 2-arm dispatch from ~25 hot-path methods; touches `stats.rs:121`, `cmd_snap.rs:242-249`, `init.rs:412` (possible latent INV-SYNC-014-class defect — UNVERIFIED). Cross-backend byte-equality test must exist first. |
| **J** | Stop calling `atomic_replace` after undo-based rollback/reorg on RocksDb (`rollback.rs:384-387`, `block_handling.rs:1002-1005`); the undo log is already O(changed). | Restructurer R3 (naive streaming form KILLED: self-read of the CF being deleted, `writes.rs:201`) | 0.60 measured | NO | NO | all PASS; mirrors-apply obligation in full | −O(N) RAM and IO per reorg on every node (≈1 GB at 10M UTXOs). Touches rollback — highest-consequence path. Precedent INC-I-111 (O(N) scan patched with a cache). |
| **K** | Delete dead, divergent `ConsensusParams::max_block_size` (`params.rs:171-178`, caps at era ≥ 5; live rule `constants.rs:503-511` caps at era ≥ 4; 0 production callers — verified). | Subtractionist P3 | 0.68 measured | NO | NO | all PASS | Two implementations of one consensus rule disagreeing at era 4 is the Full-Bitfield-Decode defect class; `consensus/tests.rs:144-150` asserts agreement only at eras 0-1. |

## Constraints (from the Failure Analyst — every chosen path must satisfy)

| # | Filter | Binds |
|---|---|---|
| F-01 | MUST NOT price permanent state through `minimum_fee()` alone | [F4] precedes any pricing; C, architect S3 |
| F-02 | MUST reject trailing bytes in every typed `extra_data` parser | [F4], B (Anchor ≤64 B exact), D, F |
| F-03 | MUST NOT add a field to `UndoData` (append-hostile; decode failure → silent rebuild-from-genesis) | [F5] (validation rule, no undo state), B, C |
| F-04 | MUST implement any insert/skip rule at ALL write paths behind one predicate (4 impls + undo loop; audit `oracle.rs:265`) | B |
| F-05 | MUST test apply→rollback→re-apply→root identity at AH−1/AH/AH+1 on both backends | [F4], [F5], B, C, D, E, F |
| F-06 | MUST pin a versioned read before any multi-message state stream | [F3] |
| F-07 | MUST carry a resumption cursor | [F3] |
| F-08 | MUST NOT blacklist a peer for advancing its tip mid-stream | [F3]; enforced for the manifest itself by INC-I-231 (`state_session.rs`), which retries the quorum instead of refusing |
| F-09 | MUST NOT claim peer scoring as mitigation (0 callers, blacklist clears every 30 s) | [F3]; architect D.4 is FALSE |
| F-10 | MUST re-derive the installed root from the installed backend | [F1], [F3] |
| F-11 | MUST reuse `rebuild_in_progress` + `rebuild_halt_reason` for any non-atomic install | [F3] |
| F-12 | MUST wire any new AH at all 7 `ValidationContext::new` sites with a miss-detecting test | [F4], [F5], B, C, D, E, F |
| F-13 | MUST treat a new `OutputType`/`TxType` discriminant as an undecodable-message change: rolling + version census + pre-AH relay refusal | B |
| F-14 | MUST name archive availability as a liveness dependency or generate the proof at submission | A, B → G |
| F-15 | MUST bind proof format to (tx index, leaf count) | G |
| F-16 | MUST NOT increase snap-sync frequency/duration without a plan for the 2026-03 cascade | [F3] |
| Res-1 | Any new byte-emitting path needs a byte-equality test on BOTH backends | [F2], [F3] |
| Res-2 | `atomic_replace` deletes the CF before rewriting — never stream-read that CF into it | [F3] (staging CF), J |
| Res-3 | `minimum_fee()` is consumed by consensus + mempool + 4 CLI sites — name the CLI migration | C, architect S3 |
| Res-7 | `MAX_SYNC_SIZE` bounds 5 payload classes — never fix one class by changing the constant | [F3] |
| Rad-1 | `utxo_hash` is a flat BLAKE3 over an ordered byte string — any tree/accumulator is a different, AH-requiring change | [F2], [F3] |
| Rad-5 | Output-level `extra_data` is signed; tx-level is not — say which one you use | A, B, F |
| User | Anchors: one root per stack per block; provable-not-queryable; never in the UTXO set; no on-chain file hosting; no genesis reset; product layer out of scope | A (deviates on the tip), B, D |

## Architecture Maps

### Current
```
Block ──validate──> apply_block ──BlockBatch.add_utxo (batch.rs:197)──> state_db CF_UTXO   [forward apply: 1 site]
                                 └─ undo loop (tx_processing.rs:193-196) pushes EVERY output index
rollback.rs:311 / block_handling.rs:941 ──UtxoSet::add_transaction──> in_memory.rs:82 | writes.rs:409   [rebuild: 2 more impls]
serve_state_root ──compute_state_root──> serialize_canonical() -> Vec<u8> (2-3x set in RAM) ──BLAKE3──> utxo_hash
StateSnapshot::create ──serialize_canonical()──> ONE SyncResponse::StateSnapshot ≤ MAX_SYNC_SIZE 16 MiB   [x2 handlers]
receiver: bytes ──deserialize (root check)──> drop; ──deserialize AGAIN──> InMemory ──root AGAIN──> iter_all().clone() ──atomic_replace──> state_db ──from_state_db
pricing: 5 wire-byte bounds, 0 resident-byte bounds; MintAsset/BurnAsset skip the fee; parsers accept trailing bytes
anchors today: NFT/Hashlock output = 131-196 B permanent UTXO each
```

### Proposed (Definite + Recommended)
```
core:    resident_bytes(tx) + is_state_resident(&Output)   [one predicate, one unit]          (F5; B consults it)
validate: Σ resident_bytes ≤ C  (exempt consensus-generated, ZKRollup)   @ resident_bytes_cap_activation_height   (F5)
          MintAsset/BurnAsset pay 1 + bytes/100; fungible_asset_metadata() exact-length   @ asset_tx_fee_activation_height   (F4)
storage: fold_canonical_utxo(&mut |bytes|)  ──> canonical_digest() -> utxo_hash (bit-identical)   (F2)
                                             ──> canonical_range(start_key, max_bytes) -> (bytes, next_key)
network: GetStateManifest{block_hash} / GetStateChunk{block_hash,start_key,max_bytes}  over a pinned view (TTL)   (F3)
         legacy GetStateSnapshot retained as BRIDGE while set ≤ 16 MiB; MAX_SYNC_SIZE unchanged
receiver: cursor in SnapDownloading ──chunks──> staging CF ──incremental BLAKE3──> root == quorum_root? ──promote──>
          state_db ──re-derive root from backend (F-10)──> from_state_db; rebuild_in_progress armed throughout (F-11)
anchors: Option A (tip UTXO per stack, 0 LOC) or Option B (Anchor output, never resident); proof via G
```

## Migration Path (forward-activated, independently shippable; rolling no-AH stages first)

| Stage | Change | Q1 rules? → AH field | Q2 content? → deploy | Blast radius | Gate before next |
|---|---|---|---|---|---|
| 1 | [F1] install dedup + backend re-derivation | NO | NO → rolling | `fork_recovery.rs`, `snapshot.rs`, `utxo/set.rs` | snap-sync soak on testnet seed; INV-SYNC-014 test |
| 2 | [F2] streaming fold + AP-7 fail-loud; byte-equality test both backends; delete `serialize_canonical()` when callerless | NO | NO → rolling | `state_db/queries.rs`, `utxo/{set,in_memory}.rs`, `snapshot.rs`, `rpc/methods/stats.rs`, `utxo_size_monitor.rs` | `canonical_digest() == hash(serialize_canonical())` on ≥100k randomized entries, both backends |
| 3 | [F3] session transport with F-06..F-11 + staging CF + duplicate-handler collapse | NO | NO → rolling with negotiation | `protocols/sync.rs`, `sync/manager/snap_sync.rs` (download stage only), `state_snapshot_serve.rs`, `event_loop.rs`, `fork_recovery.rs` | testnet soak under repeated snap requests; cascade plan per F-16; INC-I-143 status reviewed |
|   | BRIDGE: legacy single-frame `GetStateSnapshot` kept live for old peers while the set is ≤ 16 MiB — TEMPORARY; delete after the fleet + externals negotiate the session protocol or the set exceeds the frame | NO | NO | same | version census |
| 4 | [F4] asset fee + exact-length parser | **YES** → `asset_tx_fee_activation_height` (NEW) | NO → rolling BEFORE the AH; pin after the ~30 externals upgrade | `validation/utxo.rs`, `output.rs`, `network_params/`, 7 ctx sites | F-05 AH±1 tests; F-12 miss-test; INC-I-075 answers in the commit |
| 5 | [F5] resident-byte cap | **YES** → `resident_bytes_cap_activation_height` (NEW, never bundled with stage 4's) | NO → rolling BEFORE the AH | `validation/{transaction,block}.rs`, `assembly.rs`, `policy.rs`, `network_params/` | preconditions (i)-(iv) closed; economist decision row; pre/post-AH block-equality test |
| 6 (opt) | Option B `OutputType::Anchor` | **YES** → `anchor_output_activation_height` (NEW) | **YES post-AH** → rolling BEFORE the AH + version census + pre-AH relay refusal (F-13) | 4 insert impls + undo loop + `types.rs` + `network_params/` + CLI/RPC | stack cardinality answered; F-04/F-05 tests; G shipped |
| 7 (opt) | Option C deposit or height-aware fee | **YES** → own NEW field | NO → rolling before AH | `core.rs`, validation, mempool, 4 CLI sites | economist; `rewards.rs` minimum-share read |
| any | Options D/E/F/H/I/J/K | D/E/F YES → own fields; H/I/J/K NO | NO | as listed | supersession decisions for D/E |

Ordering rules honoured: stages 1-3 carry no AH and no block-content change; no two rules share a field; every AH is `u64::MAX` on devnet until pinned; nothing here bumps `CURRENT_PROTOCOL_VERSION` or `EPOCH_STATE_FORMAT_VERSION` or touches `HardForkSchedule`; no genesis reset at any stage.

**Implementation status (2026-09-15).** Stages 1, 2 and 3 are SHIPPED on branch `feature/utxo-scalability-streaming` as `0f11daca`, `d8a4b508` and `c552c8c0`; each carries its Q1/Q2 answers (rules NO, content NO) in its commit body and none pins an activation height. Stage 3 ships with a DEPLOY-ORDER constraint the table did not anticipate: a new client no longer emits `GetStateSnapshot`, so the snap-sync SERVERS (the seeds) must be upgraded before the clients. Stages 4-7 and Options A-K are untouched.

## Complexity Comparison

| Metric | Current | Radical minimum (SSF, filters applied) | Full proposal ([F1]-[F5] + Option A) | + Option B |
|---|---|---|---|---|
| Modules touched | — | 7 (+ pinned view/cursor in `snap_sync.rs`, staging CF in `state_db`) | ≈12 | ≈16 |
| New interfaces | — | +2 wire variants, +2 storage fns | +2 wire, +2 storage fns, +1 cap rule, −1 fee exemption | +1 `OutputType`, +1 CLI verb, +1 RPC |
| New abstractions | — | 1 fold callback, 1 pinned view/session | + `resident_bytes` unit, + residency predicate | + "valid but non-resident output" |
| New consensus rules | — | **0** | 2 | 3 |
| New AH fields (29 exist) | — | **0** | 2 | 3 |
| New wire messages | — | 2 | 2 | 2 |
| Mirrored consensus sites | — | 0 | 0 (validation-only rules; 7 ctx wirings) | 5 |
| Deploy class | — | rolling, all | 1-3 rolling; 4-5 rolling-before-AH (Q2 NO) | stage 6 rolling + census + relay refusal (Q2 YES post-AH) |
| Chain LOC (estimate) | — | ≈350 | ≈600 | ≈750 |
| Covers adversarial fill? | no | **no** (≈34 min to 300 MB; ≈0 DOLI via MintAsset) | yes (bounded to C x 8,640/day, priced) | yes |

## Milestones (the proposal touches ≥4 modules)

| M | Delivers | Exit criterion |
|---|---|---|
| M1 | Stage 1 | snap-synced testnet node: `getStateRootDebug` byte-identical to a full node (REQ-SCALE-001/002); peak RSS during install measured, not argued |
| M2 | Stage 2 | byte-equality test both backends green; `serialize_canonical()` deleted; AP-7 fail-loud test |
| M3 | Stage 3 | testnet node joins a synthetic >16 MiB set via chunks (REQ-SCALE-013 measured); mid-stream tip advance does not blacklist; crash mid-install refuses to serve then recovers |
| M4 | Stage 4 | client install peak RAM is O(chunk) not O(set): `m4_install_peak_probe` ratio at n=50k vs n=100k is flat (REQ-SCALE-014); installed UTXO digest byte-identical before/after at both n (INV-SYNC-007); crash/abandon mid-transfer leaves no staged rows |
| M5 | Stage 5 | census artifact + economist decision row + exemption table; pre/post-AH block-equality test; cap value `C` recorded in `NetworkParams` |
| M6+ | Options as chosen | per-option gates above |

## Design Synthesis Quality Gate

```
━━━ DESIGN SYNTHESIS QUALITY GATE ━━━
Evaluators completed:           5/5
Deletion convergence items:     1 (5/5: install materializations) + 1 constraint (4/5: tx-level extra_data not a carrier)
Restructuring convergence:      4 ([F2] 4/5, [F3] 5/5, [F4] 2/5 verified, [F5] 4/5)
Addition options presented:     11 (A-K)
Failure modes identified:       16 (F-01..F-16) + 7 Restructurer + 6 Radical constraints
Failure modes applied as filters: 16/16 (table in reasoning trace, every proposal x every filter)
Radical floor gap:              current -> radical (0 rules, 0 AH, ~350 LOC) -> proposed (2 rules, 2 AH, ~600 LOC)
Contradictions found:           12
Contradictions resolved:        10/12 (C1 anchor mechanism and C10 price mechanism are USER decisions, presented as options)
Evidence independence verified: YES (per-row in the matrix; facts (a)-(h) re-verified in code this session)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Implementation Notes (as built, M1-M4)

Deviations from the proposal text above, recorded because the code is the source of truth.

### Stage 1 [F1] install once (as built, M1 — `0f11daca`)

`storage::verify_state_root_from_bytes()` returns `(root, chain_state, utxo_set, producer_set)`, so
the caller stops decoding the snapshot a second time; `compute_state_root_from_bytes()` is now a
projection of it with its signature and its [STOR033]/[STOR034]/[STOR035] text unchanged. The
decoded pairs move into the single `atomic_replace` WriteBatch through `UtxoSet::into_pairs()` /
`InMemoryUtxoStore::into_pairs()` instead of being cloned into a `Vec`. The cached root is
RE-DERIVED from the INSTALLED `state_db` backend (filter F-10: a wire hash proves transport, not
storage). `apply_snap_snapshot()` installs the in-memory 3-state only on the `Ok` arm, so an
`atomic_replace` failure can no longer leave memory holding the snapshot while disk holds the old
chain. `doli snap` (`bins/cli/src/cmd_snap.rs`) carried the identical triple-copy pattern and is
ported to the same seam; its flags and its printed output are unchanged.

Measured, not argued: install peak at n=50,000 fell 15,778,678 -> 14,629,849 bytes (full-set copies
3.3 -> 3.0) and the probe's `M1_UTXO_HASH` is bit-identical before and after.

### Stage 2 [F2] streaming canonical fold (as built, M2 — `d8a4b508`)

One encoder, `crates/storage/src/utxo/canonical.rs`: a `u64` LE count header, then
`key || UtxoEntry::serialize_canonical_bytes()` per entry in ascending key order.
`canonical_digest()`, `canonical_len()`, `canonical_range()` and `materialize()` are all built on
the same `fold`, and `serialize_canonical()` survives as a thin `materialize()` for the one
remaining production caller (the legacy snap wire path, `snapshot.rs:235`).

**Deviation — the digest takes TWO passes over one pinned view (STOR028).** A streaming hasher
cannot back-patch the LE count header, and the header must come from the rows the body actually
emits, never from a live counter an iteration can desync from. So pass 1 reads keys only
(`row_count()`) and the fail-loud pass 2 decodes and feeds the hasher; pass 2 can never emit fewer
rows than pass 1 counted. This is a read amplification the proposal did not price, and it is the
price of keeping the header coherent with the body.

**AP-7 closed.** `queries.rs:483-501` used `filter_map(|r| r.ok())` twice inside a CONSENSUS
serializer: an undecodable value was silently skipped and the node computed a
different-but-valid-looking `utxo_hash`. The fold returns `Err` on the first undecodable entry and
on the first iterator error, and never skips. The user-visible consequence is that
`getStateRootDebug` now returns a JSON-RPC internal error instead of a hash when an entry cannot be
decoded.

Measured: root-compute peak on the production (RocksDB) backend at n=100,000 fell 19,401,302 -> 1,286
bytes; `M2_UTXO_HASH` is identical before and after.

### Stage 3 [F3] chunked state-transfer session (as built, M3 — `c552c8c0`)

**Wire shape (additive only).** `GetStateManifest{block_hash}` and
`GetStateChunk{session_id,start_key,max_bytes}` are APPENDED to `SyncRequest`; `StateManifest`,
`StateChunk` and `StateSessionUnavailable{session_id,reason}` are APPENDED to `SyncResponse`.
Appending is load-bearing: bincode encodes an enum as a u32 index, so appending leaves every
existing discriminant where it is and old peers keep decoding old messages unchanged. An old
binary that receives a new variant fails inside `bincode::deserialize` in the codec's
`read_request`/`read_response` and surfaces an `io::Error(InvalidData)` — a stream failure, not a
panic. `CURRENT_PROTOCOL_VERSION`, `MIN_PEER_PROTOCOL_VERSION`, `HardForkSchedule` and
`MAX_SYNC_SIZE` (16 MiB) are ALL untouched (Res-7). Negotiation is behavioural: a peer that does
not answer a manifest request is served by the legacy single-frame `GetStateSnapshot` BRIDGE while
the set is <= 16 MiB.

**Consistency (F-06).** `UtxoSet::with_rows()` pins a NEW `rocksdb::Snapshot` per call
(`utxo/set.rs:500`), which is sufficient for a single-call fold but NOT across a multi-message
session. The session therefore holds ONE pinned view for its whole lifetime; `canonical_range()`
is driven from that single pin, never from a fresh one per chunk. The proof obligation is a test
that mutates the serving node's UTXO set mid-session and still reassembles a digest equal to the
manifest `utxo_hash`.

**Rate-limit interaction (found in code, not predicted).** `MAX_SYNC_REQUESTS_PER_INTERVAL = 24`
(`bins/node/src/node/network_events.rs:307`) applies to every request kind with no exemption, and
the client blacklists a peer on any response whose error text contains "busy"
(`sync_engine/response.rs:111,126-128`). A chunked session issues one request per chunk, so
counting chunk requests against that cap would make the new transport blacklist honest peers with
its own traffic. `GetStateChunk` is therefore exempt from that cap — chunk traffic is already
admission-controlled by `MAX_CONCURRENT_STATE_SESSIONS` plus one-outstanding-chunk-per-session —
while `GetStateManifest` stays under it, because the manifest is the request that costs a pin.
Session refusals travel as the typed `StateSessionUnavailable`, never as the `Error("busy: ...")`
string, so they cannot reach the string-matching blacklist branch.

**Honest limitation — peer scoring is NOT a mitigation (F-09).** The architect's D.4 claim that a
misbehaving peer is "scored down" is FALSE in this codebase: `record_malformed` and
`record_invalid_block` have zero production callers, and `snap.blacklisted_peers` is in-memory and
cleared every 30 s once attempts are exhausted. Nothing in Stage 3 relies on a peer paying a
durable cost for serving bad chunks. What actually protects the client is the end-of-session
digest equality against the manifest `utxo_hash` and the unchanged INC-I-143 F4 Gate-1 root
equality against the quorum root — a peer that serves wrong bytes wastes one attempt and is
refused, it is not punished. Any future claim that Stage 3 is safe "because bad peers are scored"
must be rejected until `scoring.rs` acquires production callers.

**As built — the mechanism chosen for the pin.** The session is a small actor, not a held lock. On
`GetStateManifest` the node takes an OWNED, lock-free handle to the set
(`UtxoSet::pinnable_handle()` — an `Arc` clone for the RocksDB backend, a frozen copy for the
in-memory one, which is the only honest freeze when the pin is a live `BTreeMap` the writer still
owns) and hands it to a dedicated worker thread. The worker calls `UtxoSet::with_pinned_view()` and
serves every chunk of that session from INSIDE the closure, so the `rocksdb::Snapshot` is created
once, cannot escape, and is released when the worker ends. Because the worker holds no guard on
`Node::utxo_set`, a live session can never block `apply_block` — the property failure 3a says the
read-guard approach cannot provide. Sessions are capped at `MAX_CONCURRENT_STATE_SESSIONS` and
evicted on `STATE_SESSION_TTL_SECS` / `STATE_SESSION_IDLE_TIMEOUT_SECS` or when the cursor is
exhausted.

**As built — `max_bytes` is peer-supplied.** `serve_state_chunk` clamps it to
`STATE_CHUNK_MAX_BYTES` before it reaches storage. The requester never sizes the server's
allocation.

**As built — the staged install is the live client path (M4).** `stage_utxo_bytes`,
`staged_utxo_len`, `promote_staged_utxos` and `clear_staged_utxos` shipped as a seam with M3,
locked by `bins/node/tests/it/m3_staging_install.rs` including the Res-2 exclusion of
`cf_utxo_staging` from `StateDb::deletable_cf_names()`. M4 wired them. `crates/network` defines a
backend-agnostic `UtxoChunkSink` trait (`crates/network/src/sync/chunk_sink.rs`); `bins/node`
implements it over the `StateDb` (`bins/node/src/node/utxo_chunk_sink.rs`) and installs it on the
live `SyncManager` at `bins/node/src/node/init.rs`. The session client no longer accumulates
`StateSessionClient.image`: each verified chunk body is staged and dropped, and `VerifiedSnapshot`
carries a `utxo_staged: Option<StagedUtxoMarker>` instead of a whole canonical image. Install
(`bins/node/src/node/snapshot_install.rs`) promotes the staged rows under the
`rebuild_in_progress` / `rebuild_halt_reason` markers; every abandon path clears staging; the state
root is re-derived from the INSTALLED backend (F-10).

M4 round 2 (decision 136) replaced the single promotion `WriteBatch` with a staged-install-only
chunked write path in `crates/storage/src/state_db/promote.rs`. `StateDb::atomic_replace`
(`writes.rs`) is UNCHANGED and keeps single-batch all-or-nothing for rollback and reorg replay,
which arm no marker. Phase 1 drops the live `cf_utxo` / `cf_utxo_by_pubkey` rows with two
`delete_range_cf` tombstones; Phase 2 streams staging into the live families in sub-batches
bounded by `PROMOTE_BATCH_MAX_BYTES` (128 KiB of batch payload), each its own `WriteBatch`;
Phase 3 writes the producer families and the three `CF_META` labels AFTER the UTXO rows and BEFORE
the root check; staging is cleared in a separate write on the `Ok` arm only.

Restart is **NO-RESUME**: `Node::reconcile_staged_utxos_on_startup` clears `cf_utxo_staging` on
BOTH arms and never touches the rebuild marker, so a node that crashed inside a promotion window
comes back halted and recovers by a fresh snap-sync. There is no resume-from-staging path.

The before/after numbers for `cargo test -p doli-node --test m4_install_peak_probe` are recorded in
`docs/.workflow/m4-outcome-metric.txt`. Note the probe's counting `#[global_allocator]` observes
RUST allocations only; `rocksdb::WriteBatch` is an FFI handle over C++ memory, so the batch this
milestone bounds does not appear in `M4_PEAK_RATIO` — see
`docs/.workflow/m4-developer-report-round2.md`.

Two asymmetries are deliberate and must not be "tidied":
- The STAGED post-install root-mismatch arm returns `Err` and leaves `rebuild_in_progress` ARMED,
  because staging has already overwritten the live CF and a soft refuse would leave a silently
  wrong set installed. Every pre-existing M1/legacy refusal arm keeps its soft-refuse (log,
  `snap_fallback_to_normal`, `Ok(())`) byte-for-byte.
- The in-memory UTXO backend has no `cf_utxo_staging`, so it keeps the materialised-image path.
  That path is test-only; the RocksDB backend is the one REQ-SCALE-014 is measured against.

This is a client-local install mechanism: the wire format, the served bytes, and the resulting state
are unchanged (INV-SYNC-007), so no activation height and no synchronized deploy are required.

### REQ-SCALE-014 status after M4

| Dimension | Status | Evidence |
|---|---|---|
| Client install peak, Rust heap | **MET for the install path** — O(chunk), not O(set) | `m4_install_peak_probe`: 2,097,350 B at n=50k vs 2,097,376 B at n=100k, `M4_PEAK_RATIO` 2.00 -> 1.00 against an unchanged <= 1.25 bound; 21.2x lower in absolute terms at n=100k (`docs/.workflow/m4-outcome-metric.txt`) |
| Installed state | unchanged | `M4_INSTALL_UTXO_HASH` byte-identical before/after at both n (INV-SYNC-007) |
| RocksDB write batch (C++) | **bounded by construction, NOT measured** | `write_staged_rows_in_sub_batches()` commits whenever the pending payload reaches `PROMOTE_BATCH_MAX_BYTES` (128 KiB), so at most one sub-batch is live at a time. The probe's counting `#[global_allocator]` sees RUST allocations only; `rocksdb::WriteBatch` is an FFI handle over C++ memory and is invisible to it. No probe in the repo measures this today |
| REQ-SCALE-014 as written (peak RSS <= 2 GB at 10M UTXOs) | **NOT yet claimed** | the requirement is an RSS figure at 10M entries; what M4 measured is the Rust-heap SHAPE at n<=100k. An RSS-based probe is the follow-up |

### Stage 3 [F3] manifest admission, corrected (as built, M5 — INC-I-231)

**What was wrong.** As shipped in M3, `handle_state_manifest` ran two admission gates:
exact equality against `quorum_root`, then a COUNT of connected peers whose stored
`PeerSyncStatus` equalled the manifest's `(block_hash, block_height)`. That second gate was
copied from `handle_snap_snapshot` (INC-I-143 F4 Gate 2), where the anchor arrived with the
STATE and had to be corroborated from somewhere. In the session protocol it is unsound:
`PeerSyncStatus` is refreshed only by periodic status responses, so on a 10 s-slot chain the
stored tuples lag the moving tip. A manifest served AT the quorum height then matches ZERO
peers. Field evidence: a fresh node reached `Quorum reached: 4 peers agree on root=4e246f28…,
best_height=3505`, received four IDENTICAL correct manifests, refused all four with
`corroborated by 0/4`, exhausted `snap_attempts=3` in seconds and fell back to header-first.

**The correction.** The quorum anchor is already recorded at quorum time —
`SyncPipelineData::SnapDownloading { target_hash, target_height, quorum_root }`, set at the
"Quorum reached" transition. The manifest is now corroborated against THAT, not against peer
status: admit iff `state_root == quorum_root && (block_hash, block_height) == (target_hash,
target_height)`. The state root commits height and hash, so the test is exact and costs no
peer iteration. `self.peers` is no longer read by this path at all, which is the property that
makes status-refresh latency unable to refuse a correct manifest.

This is STRICTER than what it replaces, not looser: the old count could be satisfied by any
`snap_quorum()` peers whose status happened to match, whereas the anchor cannot be satisfied by
peer status at all. INC-I-012 (a forked peer serving at a height different from the quorum
target) stays closed; INV-SYNC-005 (quorum formation filters candidates) is untouched.

**F-08 handling.** A manifest with a foreign root at a height ABOVE the anchor is the
tip-advanced peer, not an attacker: the session is cleared and `snap_fallback_to_normal()`
re-collects the quorum at the new tip. No blacklist, no `integrity_refusals` increment, no
peer-score change (F-09). It is bounded by the pre-existing `snap.attempts >= 3` cap and its
`last_snap_attempt` cooldown — no new counter was introduced. `snap_sync.rs` already documented
this exact tolerance on the VOTE side; M5 extends it to the manifest.

**Not in scope.** `handle_snap_snapshot` still carries the original status-count Gate 2. It is
dead on the request side — no code in `crates/network/src` constructs
`SyncRequest::GetStateSnapshot` any more; M3 replaced the request with
`GetStateManifest`/`GetStateChunk`, and the serve side only answers old peers. It is recorded
here so the same mechanism is not rediscovered as a second incident.

Wire format unchanged (`crates/network/src/protocols/sync.rs` empty diff), no consensus rule
change, no block-content change: rolling deploy, no activation height.
