# INC-I-236 Run A Analysis: Port the block-signature fix onto v6.44.2 and close every intake path

Run 599 · Orchestrator (resume of run 567) · Status: ANALYSIS

## Scope
- Base: `b4cff010` (v6.44.2, origin/main). Branch `fix/inc-i-236-signed-blocks`.
- Source fix: 5 commits on `fix/inc-i-236-block-signature` (merge base `e5a513f7`), read only with `git show` / `git cherry-pick`:
  `3b9b2f7d` M1 core rule + AH · `3388260f` M2 producer signs + ctx threading · `8a9bb96c` M3 slash evidence · `7151c547` M4 no header side effects before auth · `867d4e8b` M5 header-first sync.
- Modules: `crates/core/src/{block,validation,network_params}`, `crates/crypto`, `bins/node/src/node/{production,validation_checks,apply_block,block_handling,fork_recovery,header_auth,init}`, `crates/network/src/sync/`, `crates/mempool/src/pool.rs`, `crates/rpc` (backfill), `crates/storage/src/archiver.rs`, `scripts/gauntlet*`.

## Summary (plain language)
Blocks name a producer but carry no signature from it. The prior diagnosis (run 567, VERDICT conf 0.97, restored in `docs/.workflow/diagnosis-report.md`) confirmed this is a design omission present since the first commit, not a regression. The fix exists as 5 unmerged commits on an old base. This run ports them onto current main, finds every block intake path the old branch did not cover, proves pre-height bytes stay identical, adds a gauntlet scenario, and completes the incident records.

## Regression check (regression-detect hook)
Baseline given by the user: v6.44.2 is stable but unsigned. `git log` archeology in run 567 (incident entry 2944): no block-header signature ever existed (`57a3ab37` initial commit); `c913af08` wrote the false "VDF proves authorship" premise; `d2962cb5` cut VDF iterations. Verdict: pre-existing design omission. Not a regression in `e5a513f7..b4cff010`; that range only adds intake paths (sync, archive backfill, snap, trust root) that the port must now cover.

## Architecture context (from the restored system blueprint)
- `BlockHeader.hash()` excludes `vdf_proof`; `vdf_proof.pi` is empty in every produced block. Carrier for the signature = `pi` (no schema change, no protocol version bump). Rejected carriers: new Block field (breaks `Vec<Block>` bincode), new header version (old nodes reject).
- Coded preimage (old M1, kept): `BlockHeader::signing_message() = blake3(BLOCK_SIGNATURE_DOMAIN || header.hash())` with `BLOCK_SIGNATURE_DOMAIN = b"DOLI_BLOCK_SIG_V1\xFF"`. The `0xFF` byte makes the preimage non-UTF-8, so `doli sign` (blake3 of a UTF-8 message) can never produce it (W4).
- Key: Ed25519 key in `header.producer`; strict verify; exactly 64 bytes.
- Gate: `NetworkParams.inc_i_236_block_signature_activation_height` — mainnet `u64::MAX`, testnet `u64::MAX`, devnet 0. Env override `DOLI_INC_I_236_BLOCK_SIGNATURE_ACTIVATION_HEIGHT` honoured on **devnet only** (Run A tightening: the old branch honoured it on testnet too; the brief requires the override to be impossible on mainnet AND testnet builds).
- Signature check runs in `validate_block_with_mode` before `match mode` (Full/Light/Replay one verdict), in gossip intake before the equivocation detector, and in `HeaderDownloader::validate_header`.
- Blast radius of the port: 20 files overlap with the 75 main commits since `e5a513f7` (sync manager, block_lifecycle, response.rs, headers.rs, fork_recovery, init, production, validation_checks, network_params, mempool pool, test mains). Each conflict must be resolved against the NEW code (lesson 4647).

## Requirements

| ID | Requirement | Priority | Acceptance criterion |
|---|---|---|---|
| REQ-AUTH-001 | AH field in `NetworkParams` (mainnet/testnet `u64::MAX`, devnet 0), threaded into every `ValidationContext` construction site (node validation, apply, builder, mempool, sync config) | Must | Field exists; every construction site passes it; a grep census of `ValidationContext` constructors shows none missing |
| REQ-AUTH-002 | Producer signs `signing_message()` and stores the 64-byte signature in `header.vdf_proof.pi` for every block it builds, also below the AH | Must | Node test: a built block carries a valid 64-byte signature by its producer |
| REQ-AUTH-003 | At h >= AH `validate_block_with_mode` rejects missing / malformed / wrong-key / wrong-message signatures before `match mode`; below the AH `pi` is not read | Must | Core tests for each case in Full, Light, Replay; RED on `b4cff010`, GREEN at HEAD |
| REQ-AUTH-004 | Existing blocks stay byte-identical: `hash()` and bincode of pre-fix headers/blocks unchanged | Must | Golden test with a fixed header vector; same hash and bytes as v6.44.2 |
| REQ-AUTH-005 | Repro tests FAIL on `b4cff010` (names + failure lines recorded) and PASS at HEAD | Must | Evidence file `docs/.workflow/inc-i-236-run-a-red-evidence.txt` |
| REQ-AUTH-006 | Slash evidence: both `DoubleProduction` headers validly signed by the accused, same slot, same producer, same `genesis_hash`, at h >= AH | Must | Core tests: unsigned, foreign-signed, wrong-genesis evidence rejected; valid pair accepted |
| REQ-AUTH-SEC-001 | Strict Ed25519 verify on untrusted bytes; any length != 64 rejected without panic | Must | Tests with 0, 1, 63, 65, 10 KiB `pi` |
| REQ-AUTH-SEC-002 | A bad-signature copy with the same hash arriving first does not block the valid copy | Must | Node test: bad copy then good copy accepted |
| REQ-AUTH-SEC-003 | `SlashData` decode capped at 64 KiB at h >= AH; garbage or oversize evidence -> `InvalidSlash`, no panic, at every height; pre-AH decoder unchanged | Must | Tests: oversize blob rejected before decode; pre-AH verdict byte-identical |
| REQ-AUTH-007 | Gossip intake: at h >= AH the header signature is verified before `EquivocationDetector::check_block`; forged/unsigned headers produce no proof, no slash tx, no detector record, no log line at warn+ | Must | Node tests (forged unsigned + wrong key after a real block) |
| REQ-AUTH-008 | Header-first sync verifies the signature at h >= AH at the header stage; below the AH nothing changes | Must | Network tests: forged/empty signature above AH rejected at that header; unsigned chain below AH accepted |
| REQ-AUTH-012 | The `HeightOccupied{is_better:true}` arm does not signal stuck-fork for an unsigned/foreign-signed lower-slot sibling | Must | Node test |
| REQ-AUTH-013 | Replay-from-genesis with unsigned history passes at HEAD below the AH | Must | Node test replays an unsigned devnet chain with the AH set above its tip |
| REQ-AUTH-014 | A v6.44.2 build accepts a new producer's signed block below the height (pi outside hash, not read) | Must | HEAD test: signed and unsigned copies share hash/bytes except `pi`, validation below AH ignores `pi`; live proof = R-5 mixed-version phase |
| REQ-AUTH-015 | Gates read `u64::MAX` on mainnet and testnet in a test; the env override is honoured on devnet only | Must | Core test on `NetworkParams::defaults()` for all three networks and on `load_from_env` with the var set |
| REQ-AUTH-016 | Block request/response sync path verifies the signature before any side effect at h >= AH | Must | Test: forged and unsigned synced block rejected, valid accepted; no store/fork-choice/peer-score write |
| REQ-AUTH-017 | Snap sync and the Light/Replay arm after it verify the signature at h >= AH | Must | Test on the post-snap validation arm: forged/unsigned rejected, valid accepted |
| REQ-AUTH-018 | Every apply/import entry (`validate_block_for_apply`, direct-apply fallback, replay) verifies the signature at h >= AH | Must | Test per entry |
| REQ-AUTH-019 | Archive backfill import (`backfillFromPeer` RPC and CLI/archiver import) verifies the signature at h >= AH before writing the block | Must | Test: forged/unsigned backfilled block not stored; valid stored |
| REQ-AUTH-020 | Reorg / fork switch (execute_reorg, fork_recovery, wedge escape) verifies the signature of every competing-fork block at h >= AH before any state or store write | Must | Test: forged competing fork rejected, chain state unchanged |
| REQ-AUTH-021 | A rejected block causes no header store, fork-choice, peer-score (other than the existing invalid-block penalty), or detector write | Must | Test asserting the absence of each write |
| REQ-AUTH-022 | Binding: a signature moved to a header with one changed field fails; a `doli sign` signature over the preimage bytes fails as a block signature | Must | Core tests |
| REQ-AUTH-023 | Any intake path the old branch missed (from `docs/.workflow/inc-i-236-intake-census.md`) is closed with its own test | Must | Report lists every path with its test name |
| REQ-AUTH-024 | New gauntlet scenario with a runner: local multi-node cluster with a finite AH; forged and unsigned blocks injected after the height; mixed-version phase before the height | Must | Registered in `gauntlet_scenarios`; FAIL on a `b4cff010` build, PASS at HEAD; mixed phase equal tips |
| REQ-AUTH-025 | Records: run 567 closed as superseded; incident root cause, affected files, branch; `decisions` row (preimage + gate); invariant + linked regression tests + monitoring signal; AUDIT-P1-203 and finding 76 disposition | Should | Row ids in the report |
| REQ-AUTH-009 | RPC exposes a `signed` flag | Could | Not in this run unless free |
| REQ-AUTH-010 | Mineable SlashProducer (Run B) | Won't | Out of scope |
| REQ-AUTH-011 | VRF instead of VDF | Won't | Not needed |

## Milestones

| ID | Name | Scope (Modules) | Scope (Requirements) | Dependencies |
|----|------|-----------------|----------------------|--------------|
| M1 | Port + RED/GREEN + pre-height golden | crates/core, crates/crypto, bins/node/src/node/production, bins/node/src/node/validation_checks, bins/node/src/node/block_handling.rs, bins/node/src/node/header_auth.rs, crates/network/src/sync, crates/mempool/src/pool.rs | REQ-AUTH-001, REQ-AUTH-002, REQ-AUTH-003, REQ-AUTH-004, REQ-AUTH-005, REQ-AUTH-006, REQ-AUTH-007, REQ-AUTH-008, REQ-AUTH-012, REQ-AUTH-013, REQ-AUTH-014, REQ-AUTH-015, REQ-AUTH-SEC-001, REQ-AUTH-SEC-002, REQ-AUTH-SEC-003 | none |
| M2 | Intake-path coverage + binding + slash evidence | bins/node/src/node/{block_handling,fork_recovery,wedge_escape,snapshot_install,apply_block}, crates/network/src/sync/manager, crates/rpc (backfill), crates/storage/src/archiver.rs, crates/core/src/validation | REQ-AUTH-016, REQ-AUTH-017, REQ-AUTH-018, REQ-AUTH-019, REQ-AUTH-020, REQ-AUTH-021, REQ-AUTH-022, REQ-AUTH-023 | M1 |
| M3 | Gauntlet scenario + records | scripts/gauntlet*, .omega/memory.db records, docs | REQ-AUTH-024, REQ-AUTH-025 | M2 |

━━━ TRIAGE VERDICT ━━━
Path: DEEP (resumed incident; diagnosis complete at conf 0.97 in run 567, artifacts restored to docs/.workflow/)
Confidence: conf(0.95, measured — prior VERDICT + 5 commits that already pass RED→GREEN on the old base)
Reasoning: root cause confirmed and fix designed; this run is a port plus gap closure, so the pipeline resumes at the Milestone Loop.
━━━━━━━━━━━━━━━━━━━━━━
