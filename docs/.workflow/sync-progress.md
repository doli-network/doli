# Sync Progress — 2026-03-29

## Pass 1 (commit 30be5d16) — 137 fixes
First sync pass completed earlier today.

## Pass 2 (commit 41349a4b) — 76 fixes
Verification sync pass.

## Pass 3 (current session, earlier) — 45 additional fixes
See previous sync-progress.md for details.

## Pass 4 (current) — 30 fixes

| Chunk | Domain | Drift Found | Fixed | Agent |
|-------|--------|-------------|-------|-------|
| 1 | Core Specs (protocol, architecture, security_model) | 5 | 5 | architect |
| 2 | Core Docs (architecture, protocol, security_model, rewards) | 6 | 6 | architect (manual apply) |
| 3 | Reference Docs (rpc_reference) | 9 | 9 | architect (manual apply) |
| 4 | Operational Docs (running_a_node, testnet, auto_update_system, archiver) | 9 | 9 | architect (manual apply) |
| **TOTAL** | | **29** | **29** | — |

### Chunk 1: Core Specs (5 fixes)
- `specs/protocol.md` — 3 fixes: DelegateBond/RevokeDelegation are state-only (empty inputs/outputs), WithdrawalRequest missing bond_count + destination in extra_data
- `specs/architecture.md` — 1 fix: missing cf_undo column family in Storage Layer section
- `specs/security_model.md` — 1 fix: RegistrationData missing BLS fields, wrong file path

### Chunk 2: Core Docs (6 fixes)
- `docs/architecture.md` — 1 fix: amm.rs → pool.rs, added 9 missing core modules
- `docs/protocol.md` — 2 fixes: peer scoring values (InvalidBlock -50→-100, InvalidTx -10→-20, added Spam/Duplicate/Malformed), rate limits (100→10 blocks/min, 1000→50 tx/sec, 60→20 req/sec, added 1MB bandwidth)
- `docs/security_model.md` — 2 fixes: same peer scoring and rate limit corrections
- `docs/rewards.md` — 1 fix: attestation constants are actually computed functions (network-dependent)

### Chunk 3: Reference Docs (9 fixes)
- `docs/rpc_reference.md` — 9 fixes: getBlockByHash missing 4 optional fields (presenceRoot, aggregateBlsSig, attestationCount, presence), getTransaction missing output fields (condition, nft, asset, bridge, extraDataSize) + covenantWitnesses, getBlockRaw response rewrite (base64+blake3 not hex raw), backfillFromPeer response (started/gaps/total), backfillStatus response (running/imported/total/pct/error), verifyChainIntegrity (missingCount not missing_count, added chainCommitment, removed "lightweight" claim), WebSocket new_block tx_count not txCount, WebSocket new_tx tx_type not txType

### Chunk 4: Operational Docs (9 fixes)
- `docs/auto_update_system.md` — 9 fixes: GRACE_PERIOD (2min→1hr fallback), CHECK_INTERVAL (10min→6hr fallback), GITHUB_API_URL (doli-network→e-weil), seniority.rs→params.rs, verify.rs→verification.rs, update.rs→cmd_upgrade.rs, maintainer.rs→cmd_governance.rs, rpc/src/update.rs→methods/governance.rs, gossip.rs→gossip/mod.rs
- `docs/running_a_node.md` — 0 drift (verified clean)
- `docs/testnet.md` — 0 drift (verified clean)
- `docs/archiver.md` — 0 drift (verified clean)

## Validation Gate (Pass 4)
- specs/SPECS.md: All referenced files exist ✓
- docs/DOCS.md: All referenced files exist ✓
- No orphaned files ✓

## Cumulative Stats (all 4 passes)
- **Total drifts fixed**: ~288 (137 + 76 + 45 + 30)
- **Files touched**: 19+ spec/doc files
- **Index integrity**: Clean

## Pass 5 (current session) — 13 fixes

| Agent | Domain | Drift Found | Fixed |
|-------|--------|-------------|-------|
| Architecture | specs/architecture.md, specs/security_model.md | 0 | 0 |
| Architecture | docs/architecture.md | 1 | 1 |
| Architecture | docs/security_model.md | 2 | 2 |
| Protocol | specs/protocol.md | 0 | 0 |
| Protocol | docs/protocol.md | 4 | 4 |
| Reference | docs/rpc_reference.md | 2 | 2 |
| Reference | docs/cli.md | 1 | 1 |
| Operational | docs/rewards.md | 2 | 2 |
| Operational | docs/testnet.md | 1 | 1 |
| Operational | docs/running_a_node.md | 0 | 0 |
| Operational | docs/auto_update_system.md | 0 | 0 |
| Operational | docs/archiver.md | 0 | 0 |
| **TOTAL** | | **13** | **13** |

### Architecture + Security (3 fixes)
- `docs/architecture.md` line 272: INC-I-009 → INC-I-012 (Yamux RAM reduction)
- `docs/security_model.md` lines 320-327: EquivocationProof struct updated (block_hash → BlockHeader for VDF verification)
- `docs/security_model.md` line 150: Inactivity threshold corrected (50 missed → liveness window max(500, producers×3), re-entry every 50 slots)

### Protocol (4 fixes)
- `docs/protocol.md` line 17: GossipSub topic count 7 → 8
- `docs/protocol.md` line 213: Added missing `/doli/t1/blocks/1` topic row
- `docs/protocol.md` line 280: Transaction struct `data: Option<TxData>` → `extra_data: Vec<u8>`
- `docs/protocol.md` lines 427-438: NetworkEvent enum expanded from 10 → 20 variants (added NewHeader, ProducersAnnounced, ProducerAnnouncementsReceived, ProducerDigestReceived, NewVote, NewHeartbeat, NewAttestation, TxAnnouncement, TxFetchRequest, TxFetchResponse; fixed NewBlock to include PeerId)

### Reference (3 fixes)
- `docs/rpc_reference.md`: Error code -32007 corrected from "Epoch not complete" → "Pool not found"
- `docs/rpc_reference.md`: Removed obsolete error codes -32008 ("Already claimed") and -32009 ("No reward")
- `docs/cli.md`: Replaced incorrect Nix environment prerequisite with cargo build instructions

### Operational (3 fixes)
- `docs/rewards.md`: 7 file path references updated from `consensus.rs` → `consensus/constants.rs`
- `docs/rewards.md` line 66: "epoch end height" → "epoch start height" (matches `rewards.rs:23`)
- `docs/testnet.md`: Added missing `seeds.testnet.doli.network` bootstrap entry + DNS record

## Validation Gate (Pass 5)
- specs/SPECS.md: All referenced files exist ✓
- docs/DOCS.md: All referenced files exist ✓
- No orphaned spec files ✓
- No orphaned doc files ✓

## Cumulative Stats (all 5 passes)
- **Total drifts fixed**: ~301 (137 + 76 + 45 + 30 + 13)
- **Files touched**: 19+ spec/doc files
- **Index integrity**: Clean

## Pass 6 (current session) — 22 fixes

| Agent | Domain | Drift Found | Fixed |
|-------|--------|-------------|-------|
| Architect (core specs) | specs/protocol.md | 4 | 4 |
| Architect (core specs) | specs/architecture.md | 7 | 7 |
| Architect (core specs) | specs/security_model.md | 2 | 2 |
| Architect (reference) | docs/protocol.md | 1 | 1 |
| Architect (reference) | docs/rpc_reference.md, docs/cli.md, docs/rewards.md | 0 | 0 |
| Architect (operational) | docs/security_model.md | 1 | 1 |
| Architect (operational) | docs/running_a_node.md | 1 | 1 |
| Architect (operational) | docs/archiver.md | 4 | 4 |
| Architect (operational) | docs/architecture.md | 4 | 4 |
| Architect (operational) | docs/auto_update_system.md | 4 | 4 |
| Architect (operational) | docs/becoming_a_producer.md | 1 | 1 |
| Architect (operational) | docs/testnet.md | 1 | 1 |
| Main (residual) | specs/protocol.md | 2 | 2 |
| Main (residual) | specs/architecture.md | 1 | 1 |
| **TOTAL** | | **33** | **33** |

### Core Specs Fixes
- `specs/protocol.md`: StatusRequest missing producer_pubkey, StatusResponse fields incomplete, Producer Activation described as epoch-based (actually ACTIVATION_DELAY=10 blocks), ACTIVATION_DELAY added to Parameters Summary
- `specs/architecture.md`: Line counts updated (core 23K→25K, network 5.9K→16K, storage 5.5K→10K, rpc 1.7K→4.2K), crate details for storage+network modularized, tpop/ presence.rs→presence/ directory, removed stale producer.rs
- `specs/security_model.md`: Bond unit network-specific (mainnet=10, testnet/devnet=1)

### Reference + Protocol Fixes
- `docs/protocol.md`: VDF iterations per-network (devnet=1, not "all networks")

### Operational Docs Fixes
- `docs/security_model.md`: BLS12-381 not in current consensus
- `docs/running_a_node.md`: Platform-specific data directory paths
- `docs/archiver.md`: DNS names seed3/bootstrap3→seeds.*
- `docs/architecture.md`: block_store→directory, missing archiver.rs/updater crate, Section 3.9 expanded
- `docs/auto_update_system.md`: MaintainerTxType enum 13/14→11/12, file path corrections
- `docs/becoming_a_producer.md`: Seniority multiplier description corrected
- `docs/testnet.md`: Maintainer keys path lib.rs→constants.rs

### Verified Clean (no drift)
- `docs/rpc_reference.md`, `docs/cli.md`, `docs/rewards.md`, `docs/devnet.md`, `docs/infrastructure.md`

## Validation Gate (Pass 6)
- specs/SPECS.md: All 15 referenced files exist ✓
- docs/DOCS.md: All 31 referenced files exist ✓
- No orphaned spec files ✓
- No orphaned doc files ✓

## Cumulative Stats (all 6 passes)
- **Total drifts fixed**: ~334 (137 + 76 + 45 + 30 + 13 + 33)
- **Files touched**: 19+ spec/doc files
- **Index integrity**: Clean

## Open Questions (not fixed — need human decision)
- **WHITEPAPER T_BLOCK**: Paper says 1,000 iterations, code says 800,000. WHITEPAPER is design intent.
- **DOLI_BOND_UNIT mainnet guard**: Code has no `is_mainnet` check — bond unit is ENV-configurable on mainnet.
