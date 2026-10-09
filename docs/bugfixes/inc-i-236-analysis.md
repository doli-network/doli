# INC-I-236 Analysis: Blocks Are Not Authenticated by Their Producer

Run 567 · Analyst · Status: ANALYSIS (no source code changed)

## Scope
- `crates/core/src/block.rs` (BlockHeader, hash, vdf_input)
- `crates/vdf/src/{lib.rs,proof.rs}` (block_input, VdfProof)
- `crates/core/src/validation/{block.rs,producer.rs,tx_types.rs,utxo.rs}`
- `crates/core/src/network_params/` (new activation height)
- `bins/node/src/node/production/{mod.rs,assembly.rs}` (sign at build)
- `bins/node/src/node/{block_handling.rs,rewards.rs}` (equivocation detection and auto-slash)
- `crates/network/src/gossip/{validation.rs,staleness.rs,config.rs}`, `crates/network/src/sync/{equivocation.rs,headers.rs}`

## Summary (plain language)
Any node can make a block that says "producer X made this". No check asks X to prove it.
The only "proof" is a VDF. The VDF uses public data only, so anyone can compute it (1,000 hash
iterations on mainnet). The fix: the producer signs the block hash with its key. We put the
signature in a header slot that exists today, is always empty, and is not hashed. Old blocks and
old nodes stay the same. Enforcement starts at a new future activation height.

## Claim Verification (hypothesis -> verdict, with evidence)

| # | Claim | Verdict | Evidence |
|---|-------|---------|----------|
| C1 | BlockHeader carries the producer pubkey but no signature | CONFIRMED | `crates/core/src/block.rs:19-72` (fields: version, prev_hash, merkle_root, presence_root, genesis_hash, timestamp, slot, producer, vdf_output, vdf_proof, missed_producers, data_root, fork_id). `Block` (`block.rs:156-177`) adds only txs, BLS aggregate, bitfield. No `sign(` call in `bins/node/src/node/production/`. |
| C2 | `vdf_input()` uses only public data | CONFIRMED | `block.rs:103-110` -> `vdf::block_input` (`crates/vdf/src/lib.rs:76-91`) = BLAKE3("DOLI_VDF_BLOCK_V1" \|\| prev_hash \|\| merkle_root \|\| slot \|\| producer). Mainnet iterations 1,000, locked (`network_params/defaults.rs:50`, `env_loader.rs:130`). |
| C3 | Validation has no other author binding | CONFIRMED | `validate_header` (`validation/block.rs:12-108`): genesis, fork_id, version, time/slot checks only. `validate_vdf` (`validation/producer.rs:12-53`) recomputes the public VDF. `validate_producer_eligibility` (`producer.rs:241+`) checks `producer == schedule[slot % len]`: public. |
| C4 | Network layer authenticates the producer | REFUTED (it does not) | gossipsub `MessageAuthenticity::Signed` (`crates/network/src/gossip/config.rs:234,364`) signs with the libp2p PEER key, not the producer key; `classify_block_gossip` (`gossip/validation.rs:97-121`) does decode + staleness only. |
| C5 | BLS attestations bind the block to its producer | REFUTED (they do not) | The aggregate signs the attested PARENT hash (`block.rs:161-167`); it binds attesters to the parent, not the author to this block. |
| C6 | Forged block passes consensus validation | CONFIRMED (offline repro) | `cargo test -p doli-core --test it inc_i_236` -> 2 passed (forged block `Ok` under `ValidationMode::Full`; control with an unscheduled producer rejected). |
| C7 | `validate_slash_data` trusts the public VDF as proof of authorship | CONFIRMED (function level) | `validation/tx_types.rs:155-249`: checks same producer, same slot, different hash, target match, and `validate_vdf` on both headers. No signature check. `reporter_signature` is never verified (`transaction/types.rs:196`). No check that the producer was scheduled for that slot. |
| C8 | A third party can get a bond slashed/burned via that path TODAY | REFUTED (latent) | A slash tx must be 0-in/0-out (`tx_types.rs:159-170`). It is not state-only: `allows_empty_io()` = false (`transaction/types.rs:197`), and the pre-gate list excludes it too (`validation/utxo.rs:239-247`). So the fee check (`utxo.rs` after line 261, `BASE_FEE = 1` at `consensus/constants.rs:638`) rejects it: actual fee 0 < 1. The builder skips it (`production/assembly.rs:282`); apply rejects it in Full/Light (`apply_block/tx_processing.rs:112`). `PendingProducerUpdate::Slash` is queued only from `tx_processing.rs:306`. |
| C9 | Automatic equivocation handling can be triggered by forged headers | CONFIRMED (low impact) | `EquivocationDetector::check_block` runs BEFORE validation (`block_handling.rs:177`); `handle_equivocation` (`rewards.rs:445`) creates a slash tx with an ephemeral reporter key and calls `add_system_transaction` (fee bypass). Result: "SLASHING" log spam and mempool pollution. It is never mined (C8). |

### Reproduction test review
- File: `crates/core/tests/it/inc_i_236_forged_block_authn.rs`, registered in `crates/core/tests/it/main.rs:28`.
- Result: `cargo test -p doli-core --test it inc_i_236 -- --nocapture` -> exit 0, 2 passed, output "INC-I-236 CONFIRMED: block at slot 5000 accepted ... NO producer signature".
- It does demonstrate C1-C3/C6 against the real `validate_block_with_mode(.., Full)` path.
- Limits:
  1. The assertion is INVERTED: it passes while the bug exists. The doctor TDD rule needs a test that FAILS now. The test-writer must add the post-fix contract test (block at height >= AH without a valid signature -> `Err`; with a valid signature -> `Ok`; below AH -> `Ok` either way). That test fails today.
  2. The context is synthetic (`epoch_producer_list = [victim]`, empty body, testnet params). Node-level checks (`bins/node/src/node/validation_checks/`, economics, coinbase) are not exercised. An attacker can satisfy them: the coinbase pays the reward pool and needs no producer key.
  3. It does not cover C7/C8.

## Real Attack Surface

### (a) Forged block in another producer's slot -- LIVE on every network
- Who: any party with a P2P connection. Needs only the scheduled producer's public key (public in the schedule) and the parent hash.
- Cost: 1,000 BLAKE3 iterations (microseconds).
- What the attacker controls in that slot: the full block content.
  - Censorship: it chooses which txs go in.
  - Attestation bitfield: it can OMIT attesters' bits. It cannot ADD bits without their BLS signatures (post `inc_i_178` AH). Omission lowers on-chain presence, which feeds reward qualification. (HYPOTHESIS: magnitude per epoch not measured.)
  - `missed_producers`: it must match the expected value, so no free choice there.
  - It can fill the slot of an OFFLINE producer, which hides the liveness miss.
  - It can publish TWO different blocks for one slot. This splits first-seen views and makes the victim look like an equivocator to every detector (C9).
- What limits it:
  - It gains no coins directly: coinbase and fees go to the reward pool.
  - It must race the honest producer. The gossip path drops equal-weight blocks (`wedge_escape.rs:57`). The peer-downloaded fork-recovery path uses a lower-hash tie-break (`fork_recovery.rs:76`), and the attacker can grind the hash cheaply. (HYPOTHESIS: the race win rate is not measured.)
- Severity: CRITICAL as a consensus-integrity defect (block authorship is unauthenticated). The impact is censorship, reward griefing, and fork churn, not theft.

### (b) Fabricated equivocation -> bond slash -- LATENT
- The evidence check is forgeable (C7). Execution is blocked today only by the fee check (C8). That block is a side effect, not a design decision: INC-I-173 C1 left `SlashProducer` out of `allows_empty_io` "routed to their own incidents".
- Any future change that makes `SlashProducer` mineable (fee exemption, a state-only lane) turns this into a third-party, zero-cost, 100% bond burn of ANY registered producer.
- Severity: HIGH latent; not exploitable on the current binary. This refutes the intake framing that (b) is the higher live severity. (a) is the live one.

## Root Cause
Block authorship is asserted (`header.producer`) and never proven: no consensus rule asks for a signature from the producer's private key. Everything else (forgeable VDF, forgeable slash evidence, pre-validation equivocation detection) follows from that one gap.

## Proposed Fix (SSF -- one proposal)
The producer signs the block hash with its Ed25519 producer key, the signature travels in the existing `header.vdf_proof.pi` bytes, and from a new activation height every node rejects any block (and any double-production slash evidence) whose signature does not verify against `header.producer`.

Why this works with no format change:
- `VdfProof { pi: Vec<u8> }` (`crates/vdf/src/proof.rs:14-17`) is already serialized in every header (Block wire, header-only gossip, storage). It is always empty (`production/assembly.rs:635`, "hash-chain VDF is self-verifying"). No validator reads it (grep: only `size()` counts its length, `block.rs:134`).
- `BlockHeader::hash()` (`block.rs:79-99`) does NOT include `vdf_proof`. So every existing block hash and stored header stays bit-identical, and the signature over `hash()` is not circular.
- Old binaries ignore `pi`, so new producers can attach signatures BEFORE the AH and a rolling deploy works.
- `SlashingEvidence::DoubleProduction` already stores two full headers (`transaction/data.rs:93-98`), including `vdf_proof`. Slash validation can verify the same signatures with NO new enum variant.

Rejected by evidence (not offered as an option): a new `Block` field. `SyncResponse::Bodies(Vec<Block>)` (`crates/network/src/protocols/sync.rs:95`) nests blocks inside a Vec. The bincode 1.3.3 trailing-byte tolerance applies only at the top level, so a new field breaks sync between mixed versions.

Message to sign: `BLAKE3("DOLI_BLOCK_SIG_V1" || header.hash())` (domain-separated; `hash()` already commits genesis_hash, slot, producer, merkle_root, and fork_id when non-zero).

## Deploy Classification
- Consensus RULES change? YES (new rejection rule) -> new NetworkParams activation height, constant gate, NOT HardForkSchedule. Forward-only. NO genesis reset.
- Block CONTENT change? YES, but only inside an existing variable-length field (`pi` 0 -> 64 bytes). Schema is unchanged and old nodes accept the bytes, so a synchronized stop-all/start-all is NOT needed for the signing half. All producers must run the new binary before the AH, or their unsigned blocks are rejected after it (the same failure mode as INC-I-193). An external version census is needed before the AH is crossed.
- Existing blocks: serialization and hashes are bit-identical (MUST; see REQ-AUTH-004).
- `CURRENT_PROTOCOL_VERSION`: no bump. EpochState format: unchanged.
- Slash hardening (REQ-AUTH-006) is bit-identical for all reachable inputs: every block carrying a SlashProducer tx is already rejected by the fee check. It may ship without the AH. The architect must confirm this with the three-question checklist (INV-12).

## Trust Boundaries
| Entry point | External data | Consumers |
|-------------|---------------|-----------|
| Gossip block topic (`gossip/validation.rs:97`) | full `Block` bytes | staleness gate, `handle_new_block`, EquivocationDetector, apply |
| Header gossip / header-first sync (`staleness.rs`, `sync/headers.rs`) | `BlockHeader` bytes | sync manager, fork recovery |
| Sync bodies (`protocols/sync.rs:95`) | `Vec<Block>` | apply_block, reorg |
| Tx gossip / RPC `sendTransaction` | `SlashProducer` tx `extra_data` (bincode `SlashData`) | mempool, builder, `validate_slash_data`, producer set |
All are untrusted. The only trusted root for authorship is the producer's registered Ed25519 public key.

## Requirements

| ID | Requirement | Priority | Acceptance Criteria |
|----|-------------|----------|---------------------|
| REQ-AUTH-001 | New activation height `block_signature_activation_height` in `NetworkParams` (mainnet, testnet, devnet), wired into `ValidationContext` at EVERY construction site (validation, apply, builder, sync) | Must | - [ ] Field exists in all three network defaults; mainnet/testnet are future heights set by the user<br>- [ ] Not in HardForkSchedule; no CURRENT_PROTOCOL_VERSION change<br>- [ ] Default `ValidationContext` value is `u64::MAX` |
| REQ-AUTH-002 | The producer signs `BLAKE3("DOLI_BLOCK_SIG_V1" \|\| header.hash())` with its producer key and stores the 64-byte signature in `header.vdf_proof.pi` for every block it builds, from the binary release onward (before the AH) | Must | - [ ] Built block: `pi.len() == 64` and verifies against `header.producer`<br>- [ ] `header.hash()` is unchanged by adding the signature<br>- [ ] The signature is computed after the header is final (no field changes after signing) |
| REQ-AUTH-003 | At height >= AH, block validation rejects a block whose `pi` is not a valid signature by `header.producer` over the REQ-AUTH-002 message; below the AH, `pi` is ignored | Must | - [ ] h >= AH, missing/empty/63-byte/wrong-key/wrong-message signature -> `Err`<br>- [ ] h >= AH, valid signature -> `Ok`<br>- [ ] h < AH, any `pi` -> verdict unchanged from today<br>- [ ] Same verdict in Full and Light modes (D6 precedent); the Replay handling decision is documented |
| REQ-AUTH-004 | Existing blocks stay bit-identical | Must | - [ ] Golden test: serialization and `hash()` of a pre-fix header/block are byte-identical before and after the change<br>- [ ] Stored headers/bodies decode unchanged; no storage migration |
| REQ-AUTH-SEC-001 | Signature verification uses strict Ed25519 verification on untrusted bytes; any length other than 64 is rejected without panicking | Must | - [ ] Malformed `pi` (0, 1, 63, 65, 10 KB bytes) -> `Err`, no panic<br>- [ ] Verification happens before any state mutation |
| REQ-AUTH-SEC-002 | A signature failure must NOT poison the block hash: a stripped/corrupted copy (same `hash()`, bad `pi`) arriving first must not stop the valid copy from being accepted later | Must | - [ ] Test: bad-signature copy then valid copy -> valid copy applied<br>- [ ] Staleness/dedup gates (`gossip/staleness.rs`) and any seen-hash cache do not record the hash on signature failure (gossipsub message-id is over full bytes, `gossip/config.rs:185-189`, so the two copies already differ there) |
| REQ-AUTH-005 | The reproduction test is turned into a contract test that FAILS before the fix | Must | - [ ] New test asserts rejection at h >= AH for the forged block and FAILS on the current code<br>- [ ] Existing inverted test removed or renamed to the below-AH case |
| REQ-AUTH-006 | `validate_slash_data` and `validate_slash_data_skip_vdf` require a valid producer signature in BOTH evidence headers (REQ-AUTH-002 message), plus the existing checks | Must | - [ ] Evidence with valid public VDFs but no/invalid signatures -> `InvalidSlash`<br>- [ ] Evidence with two validly signed, different headers for one slot -> passes this check<br>- [ ] Three-question checklist answered: verdict bit-identical for all reachable blocks today |
| REQ-AUTH-SEC-003 | `SlashingEvidence::DoubleProduction` evidence is decoded with bounded size; a malformed `SlashData` cannot panic or allocate without limit | Must | - [ ] Oversized/garbage `extra_data` -> `InvalidSlash`, no panic |
| REQ-AUTH-007 | `EquivocationDetector::check_block` records a header only after the block passes validation, including signature validation at h >= AH | Should | - [ ] Forged unsigned header pair at h >= AH -> no proof, no slash tx in mempool, no "SLASHING" log<br>- [ ] Two validly signed headers for one slot -> proof produced |
| REQ-AUTH-008 | Header-first sync verifies the signature on headers at h >= AH | Should | - [ ] A header chain with one bad signature above the AH is rejected at the header stage |
| REQ-AUTH-009 | RPC/explorer show whether a block is signed (`getBlock` exposes `producerSignature` or a `signed` flag) | Could | - [ ] `docs/rpc_reference.md` updated; field present for new blocks |
| REQ-AUTH-010 | Make `SlashProducer` mineable (fee lane) | Won't | Deferred. Only after REQ-AUTH-006 has shipped and has been verified; it is a separate incident (INC-I-173 C1). |
| REQ-AUTH-011 | Replace the hash-chain VDF with a VRF or put a secret in the VDF input | Won't | Not needed. The signature closes the root cause. |

## Milestones (5 modules -> split)
- M1: core primitive + gate. Sign/verify helper (core), REQ-AUTH-001/003/004/005/SEC-001. Contract test fails first.
- M2: production signing. REQ-AUTH-002 (node production). Can ship before the AH.
- M3: slash hardening. REQ-AUTH-006/SEC-003 (core tx_types).
- M4: network/node ordering. REQ-AUTH-SEC-002/007/008 (block_handling, staleness, sync headers).
- M5: docs + RPC. REQ-AUTH-009; `specs/protocol.md`, `specs/security_model.md`, `docs/architecture.md`.

## Architecture Context

### Module Boundaries
- `crates/vdf`: `block_input`, `VdfProof` -- depended on by core block, node production, validation.
- `crates/core/block.rs`: `BlockHeader` and `Block` types, `hash()`, `vdf_input()` -- depended on by all crates (storage, network, rpc, node, mempool).
- `crates/core/validation`: pure consensus verdicts (`validate_block_with_mode`, `validate_header`, `validate_vdf`, eligibility, `validate_slash_data`, `validate_transaction_with_utxos`) -- called by the node (apply, builder), mempool, and tests.
- `bins/node`: production (builds + VDF), block_handling (receive, detect equivocation, apply), apply_block (UTXO + producer mutations), rewards (`handle_equivocation`).
- `crates/network`: gossip decode/staleness, sync (headers, bodies, equivocation proof type), no consensus authority.
- `crates/storage`: header and body stored separately; body has legacy decoders (`block_store/types.rs:20-70`).

### Data Flows
- Build: `try_produce_block` -> `build_block_content` -> `compute_block_vdf` (`assembly.rs:599`) -> header -> broadcast. New: sign after the header is final.
- Receive: gossip bytes -> `classify_block_gossip` -> `handle_new_block` -> EquivocationDetector (`block_handling.rs:177`, pre-validation) -> validation -> `apply_block` -> 3 states.
- Sync: headers (`sync/headers.rs`) -> bodies `Vec<Block>` -> apply.
- Slash: detector -> `handle_equivocation` -> mempool system lane -> builder `validate_transaction_with_utxos` (rejects: fee) -> never in a block.

### Constraints and Invariants
- Header hash and header/body encoding of existing blocks MUST NOT change (state root, snap sync, block hashes).
- Activation heights are immutable once crossed; this feature gets its OWN height (no bundling).
- Builder/apply parity (INV-PROD-003): the AH must be set at every `ValidationContext` site, or the builder makes blocks that apply rejects.
- Encoder/decoder parity: any reader of `vdf_proof.pi` must treat it as the signature only at h >= AH.
- `missed_producers` and the schedule are unchanged.

### Blast Radius
- Direct: `block.rs` (signing message helper), `validation/block.rs` + `producer.rs` (gate), `validation/tx_types.rs` (slash), `network_params/` (AH), `production/assembly.rs` (sign), `block_handling.rs` (detector order).
- Indirect: every `ValidationContext` builder (apply `tx_processing.rs`, builder `assembly.rs`, sync, tests); explorer/RPC block JSON (`pi` becomes non-empty: hex in the `vdf_proof` display, `vdf/src/proof.rs:74`); test fixtures that build headers with `VdfProof::empty()` at heights >= AH (devnet AH choice affects many tests).
- Graph: `vdf_input` blast radius = 50 dependents at 2 hops (LOWER BOUND: graphify misses Rust method calls; confirmed with grep).

### Brittleness Check
- Signals: contract absence (author identity is an implicit convention) = 1; invariant gap (no module enforces authorship) = 1; cross-module blast radius (core, node, network) = 1. Total 3/5.
- Verdict: BRITTLE (architectural gap, not a code slip). This pushes to DEEP.

## Impact Analysis
- `production/assembly.rs`: must sign; risk medium (signing must follow the final header).
- `validation/block.rs`: new gate; risk high (consensus).
- `validation/tx_types.rs`: slash check; risk low (unreachable today).
- `block_handling.rs`: detector after validation; risk medium (ordering in a hot path).
- Regression areas: reorg/rollback re-validation of signed blocks; snap-sync Light mode; fork recovery header checks; devnet tests if devnet AH = 0.
- Hardware: one Ed25519 sign per produced block (~20 us) and one verify per received block or header (~50 us). With 10 s slots this is negligible. No new storage beyond 64 bytes per header (~550 KB/day at 8,640 blocks).

## Traceability Matrix
| Requirement ID | Priority | Test IDs | Architecture Section | Implementation Module |
|----------------|----------|----------|----------------------|-----------------------|
| REQ-AUTH-001 | Must | (test-writer) | (architect) | (developer) |
| REQ-AUTH-002 | Must | | | |
| REQ-AUTH-003 | Must | | | |
| REQ-AUTH-004 | Must | | | |
| REQ-AUTH-005 | Must | | | |
| REQ-AUTH-006 | Must | | | |
| REQ-AUTH-SEC-001 | Must | | | |
| REQ-AUTH-SEC-002 | Must | | | |
| REQ-AUTH-SEC-003 | Must | | | |
| REQ-AUTH-007 | Should | | | |
| REQ-AUTH-008 | Should | | | |
| REQ-AUTH-009 | Could | | | |

## Specs Drift Detected
- `crates/core/src/transaction/data.rs:88-92` doc comment: "valid VDFs (proving the producer actually created them)". FALSE: the VDF proves nothing about authorship.
- `crates/core/src/block.rs:3-7` module doc: "100% of the block reward goes to the producer". The code sends the coinbase to the reward pool (see the MEMORY hotfix on fee destination). Flag for `/sync-docs`.
- `specs/security_model.md` / WHITEPAPER: to be checked for "VDF proves producer identity" claims (not read in this run).

## Assumptions
| # | Technical | Plain language | Confirmed |
|---|-----------|----------------|-----------|
| 1 | The producer key in `header.producer` is the Ed25519 key the node holds in `producer_key` | The key named in the block is the key the node can sign with | No (architect to verify; rotated BLS keys are separate) |
| 2 | No external tool requires `vdf_proof` to be empty | Nothing outside breaks if that field gets 64 bytes | No (explorer repo not read) |
| 3 | Mainnet/testnet AH values are chosen by the user | You pick the start height | No |
| 4 | Devnet AH = 0 is acceptable (devnet is local and resettable) | On the local dev chain the rule is on from the start | No |

## Open Questions for the User
1. Activation heights: which future heights for mainnet and testnet? (Rule #0: forward-only, no genesis reset. All producers must run the new binary before the mainnet AH.)
2. Trust model: should every block, header, and slash evidence from the network be treated as untrusted until the producer signature verifies? (The analysis assumes yes.)

## What I Do Not Understand
- The real race win rate for a forged block against the honest producer (gossip first-seen vs. fork-recovery lower-hash tie-break). Not measured.
- How much one omitted attestation bitfield costs a producer in epoch rewards. Not measured.
- Whether any seen-hash dedup in `gossip/staleness.rs` (INC-I-142) keys on `header.hash()` before validation. Not read; this decides REQ-AUTH-SEC-002 work.
- Whether `Replay` mode should verify signatures (historical tolerance precedent, INC-I-064).

## Identified Risks
- Unsigned blocks from un-upgraded producers after the AH are rejected: those producers fork off. Mitigation: ship signing early, run a version census, and pick a far AH.
- A stripped-signature copy poisons dedup. Mitigation: REQ-AUTH-SEC-002.
- Bundling with another AH. Mitigation: own height (INC-I-054 lesson).

## Out of Scope (Won't)
- Making SlashProducer mineable (REQ-AUTH-010).
- VRF/VDF redesign (REQ-AUTH-011).

## Triage
```
━━━ TRIAGE VERDICT ━━━
Path: DEEP
Confidence: conf(0.85, code-verified C1-C9 with file:line + passing offline repro; race/reward magnitudes unmeasured)
Reasoning: Live consensus-authenticity gap on all networks; fix is a new consensus rule needing its own activation height, touching core validation, node production, block handling, network sync/dedup and slash validation (5 modules); brittleness 3/5. Slash vector is latent (fee gate), so no emergency hotfix is required before the AH design.
━━━━━━━━━━━━━━━━━━━━━━
```
