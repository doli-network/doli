# Reference Docs Drift -- 2026-03-29 (Verification Pass)

Date: 2026-03-29
Scope: archiver.md, genesis.md, genesis_ceremony.md, docs/protocol.md, docs/security_model.md, index files, WHITEPAPER alignment
Method: Full source code comparison against documentation

---

## docs/archiver.md

| Line(s) | Issue | Code Reality |
|---------|-------|-------------|
| 16 (comment in archiver.rs) | archiver.rs line 16 doc comment says `.sha256` sidecar | Code actually writes `.blake3` sidecar files (archiver.rs lines 171-172). The doc comment is stale but the code is correct. docs/archiver.md correctly says `.blake3` -- no drift in the doc itself, but the source code comment is wrong |
| 370 | Code reference says `bins/node/src/node.rs` for one-shot archive catch-up in `run_periodic_tasks()` | Node was modularized -- catch-up is in `bins/node/src/node/periodic.rs` (lines 155-178), not `node.rs` |
| 372 | Code reference says `bins/node/src/main.rs` for startup catch-up and `restore_from_rpc()` | `restore_from_rpc()` is in `bins/node/src/main.rs` (confirmed present), but the code also has an `operations` module. Partial drift -- main.rs delegates to operations module |
| N/A | The doc says archiver uses `mpsc::channel` with `try_send` (line 162) | Code uses `mpsc::Receiver<ArchiveBlock>` (line 19) but the sender side uses `.send()` not `try_send()` -- need to verify sender side. Minor: the archiver struct has `rx` field only; non-blocking claim may be about the sender |

**Clean items (no drift):**
- Archive format (.block/.blake3/manifest.json) matches code exactly
- Atomic write pattern (tmp + rename) matches code (lines 161-166, 169-175)
- Catch-up idempotency (file existence check) matches code (line 69)
- BLAKE3 checksum verified on restore matches code (lines 305-318)
- `deserialize_block_compat()` for legacy blocks matches code (line 322)
- RPC method descriptions (getBlockRaw, backfillFromPeer, backfillStatus, verifyChainIntegrity) all match actual implementations

---

## docs/genesis.md

| Line(s) | Issue | Code Reality |
|---------|-------|-------------|
| 51 | Says `GENESIS_TIME` is in `crates/core/src/consensus/constants.rs` (line ~25) | Actually in `crates/core/src/consensus/constants.rs` at line 25 -- CORRECT. But the path in the doc says just `constants.rs` without the `consensus/` subdirectory |
| 60 | Says testnet genesis_time is at `defaults.rs` line ~97 | Actually at `crates/core/src/network_params/defaults.rs` line 102. Line number is close but not exact |
| 69 | Says `genesis_hash = BLAKE3(timestamp \|\| network_id \|\| slot_duration \|\| message)` | Code in `chainspec.rs:193-199` confirms this formula exactly. CORRECT |
| 283 | Says `Epoch length = 360 blocks (1 hour)` | Code: `SLOTS_PER_EPOCH = 360` (constants.rs:63). CORRECT |
| 284 | Says `Block reward = 1 DOLI (100,000,000 atomic)` | Code: `INITIAL_REWARD = 100_000_000` (constants.rs:187). CORRECT |
| 285 | Says `Bond unit = 10 DOLI (1,000,000,000 atomic)` | Code: `BOND_UNIT = 1_000_000_000` (constants.rs:214). CORRECT |
| 286 | Says `Vesting = 4 years (3,153,600 slots/quarter)` | Code: `VESTING_QUARTER_SLOTS = 3_153_600` (constants.rs:231). CORRECT |
| 287 | Says `Max bonds per producer = 3,000 (30,000 DOLI max)` | Code: `MAX_BONDS_PER_PRODUCER = 3_000` (constants.rs:222). CORRECT |
| 288 | Says `Halving interval = ~4 years (12,614,400 blocks)` | Code: `BLOCKS_PER_ERA = 12_614_400` (constants.rs:107). CORRECT |
| 289 | Says `Total supply = 25,228,800 DOLI` | Code: `TOTAL_SUPPLY = 2_522_880_000_000_000` base units = 25,228,800 DOLI. CORRECT |
| 290 | Says `Unbonding period = ~7 days (60,480 blocks)` | Code: `UNBONDING_PERIOD = 60_480` (constants.rs:242). CORRECT |
| 291 | Says `Genesis open registration = 1 hour (360 blocks)` from defaults.rs | Code mainnet: `genesis_blocks: 360` (defaults.rs:45). CORRECT |
| 292 | Says `Coinbase maturity = 6 blocks` | Code: `COINBASE_MATURITY = 6` (constants.rs:201). CORRECT |
| 298 | Testnet bond unit says `1 DOLI (100,000,000)` | Code: `bond_unit: 100_000_000` (defaults.rs:110). CORRECT |
| 302 | Testnet vesting quarter says `6 hours (2,160 slots)` | Code: `vesting_quarter_slots: 2_160` (defaults.rs:144). CORRECT |
| 303 | Testnet unbonding says `72 blocks (2 epochs)` | Code: `unbonding_period: 72` (defaults.rs:106). CORRECT |
| 270 | Testnet reset says stop ai3 (testnet seed) | Testnet has seeds on ai1, ai2, ai3 per the seed topology table (lines 34-38). Consistent |

**No material drift found in genesis.md.** All parameter values match code.

---

## docs/genesis_ceremony.md

| Line(s) | Issue | Code Reality |
|---------|-------|-------------|
| 10 | Says `<data-dir>/identity.key` for peer ID file | Code uses libp2p identity from the data directory. Should verify actual filename in node init code -- likely `node_key` based on CLAUDE.md code map |
| 21 | Says `3. Start seed nodes first (ai3 -- all 6 seeds)` | genesis.md says seeds are on ai1, ai2, ai3 (3 servers x 2 networks), NOT all on ai3. The genesis_ceremony.md incorrectly implies all 6 seeds run on ai3 |
| 47-55 | Server layout table says `ai3: Seeds (both networks) + Producers (SANTIAGO, IVAN)` | genesis.md confirms ai3 has mainnet seed + SANTIAGO/IVAN producers. But genesis_ceremony step 3 still says "ai3 -- all 6 seeds" which is wrong |
| 59 | Post-genesis check says `20+ peers` | Code defaults: mainnet max_peers=50, testnet max_peers=25. "20+ peers" is reasonable for mainnet but may be too high for testnet with 25 max |

---

## docs/protocol.md (in docs/)

| Line(s) | Issue | Code Reality |
|---------|-------|-------------|
| 116-130 | SyncRequest enum lists 4 variants: GetHeaders, GetBodies, GetBlockByHeight, GetBlockByHash | Code has 7 variants: the 4 above PLUS GetStateSnapshot, GetStateRoot, GetHeadersByHeight (protocols/sync.rs:26-78). Three missing variants |
| 136-141 | SyncResponse enum lists 4 variants: Headers, Bodies, Block, Error | Code has 6 variants: the 4 above PLUS StateSnapshot, StateRoot (protocols/sync.rs:82-120). Two missing variants |
| 189-199 | GossipSub parameters: `mesh_n: 6`, `mesh_n_low: 4`, `mesh_n_high: 12`, `gossip_lazy: 6` | Code mainnet defaults: `mesh_n: 12`, `mesh_n_low: 8`, `mesh_n_high: 24`, `gossip_lazy: 12` (defaults.rs:80-83). ALL FOUR VALUES WRONG. The doc uses the old smaller values |
| 253 | TxType enum lists values 0-10 (11 types) | Code has 27 variants: Transfer(0), Registration(1), Exit(2), ClaimReward(3), ClaimBond(4), SlashProducer(5), Coinbase(6), AddBond(7), RequestWithdrawal(8), ClaimWithdrawal(9), EpochReward(10), RemoveMaintainer(11), AddMaintainer(12), DelegateBond(13), RevokeDelegation(14), ProtocolActivation(15), MintAsset(17), BurnAsset(18), CreatePool(19), AddLiquidity(20), RemoveLiquidity(21), Swap(22), CreateLoan(24), RepayLoan(25), LiquidateLoan(26), LendingDeposit(27), LendingWithdraw(28). **16 missing types** |
| 253 | TxType numbering is wrong: doc says Coinbase=3 | Code: ClaimReward=3, Coinbase=6. The doc has completely wrong discriminant values for most types |
| 267-278 | Input struct missing `sighash_type` and `committed_output_count` fields | Code (types.rs:234-252): Input has 5 fields: prev_tx_hash, output_index, signature, sighash_type, committed_output_count |
| 273-277 | Output only shows `Normal` and `Bond` output types | Code has 13 OutputType variants: Normal(0), Bond(1), Multisig(2), Hashlock(3), HTLC(4), Vesting(5), NFT(6), FungibleAsset(7), BridgeHTLC(8), Pool(9), LPShare(10), Collateral(11), LendingDeposit(12). **11 missing output types** |
| 277 | Output struct shows `pubkey_hash: [u8; 20]` | Need to verify against actual Output struct -- likely still 20 bytes |
| 284-298 | VDF section says "Block VDF: 800K iterations (~55ms)" | Code: `T_BLOCK = 800_000` (consensus/vdf.rs:15). CORRECT |
| 296 | Says "Registration VDF base: 600M iterations (~10 minutes)" | Code: `T_REGISTER_BASE = 1_000` (consensus/vdf.rs:72), NOT 600M. **MASSIVE drift** -- 600,000x difference. WHITEPAPER also says 1,000 iterations |
| 297 | Says "Discriminant bits: 2048" | Code: `VDF_DISCRIMINANT_BITS = 1024` (consensus/vdf.rs:10). Doc says 2048, code says 1024 |

---

## docs/security_model.md (in docs/)

| Line(s) | Issue | Code Reality |
|---------|-------|-------------|
| 59 | VDF Block/Heartbeat: says "Construction: Iterated SHA-256 hash chain" | Code uses BLAKE3 (`crypto::Hasher` in heartbeat.rs:397). **SHA-256 is wrong -- it's BLAKE3** |
| 60 | VDF Block/Heartbeat: says "Iterations: ~10,000,000 (~700ms)" | Code: `T_BLOCK = 800_000` (~55ms). **Both iteration count and timing are wrong** |
| 64-71 | Registration VDF says "Wesolowski Class Groups, 2048-bit discriminant, 600M iterations (~10 min)" | Code: registration uses hash-chain VDF with `T_REGISTER_BASE = 1,000` iterations (~microseconds). **Entirely wrong construction AND parameters**. Wesolowski is only used for telemetry presence (non-consensus). Registration VDF is hash-chain based |
| 134 | Says "Maximum bonds per producer: 10,000" | Code: `MAX_BONDS_PER_PRODUCER = 3_000` (constants.rs:222). **Wrong by 3.3x** |
| 148 | Says "Inactivity (50 missed slots) -> Removal from active set" | Code: `INACTIVITY_THRESHOLD = 50` (`MAX_FAILURES = 50`, constants.rs:282). CORRECT |
| 233 | Anti-Sybil section says "Maximum 100 bonds per producer" | Code: `MAX_BONDS_PER_PRODUCER = 3_000`. **Wrong by 30x** -- contradicts even the same doc at line 134 |

**Note:** Lines 134 and 233 contradict each other (10,000 vs 100) AND both contradict code (3,000).

---

## Index Verification

### specs/SPECS.md

Files referenced in SPECS.md:
- `/WHITEPAPER.md` -- EXISTS (at repo root)
- `specs/protocol.md` -- EXISTS
- `specs/architecture.md` -- EXISTS
- `specs/security_model.md` -- EXISTS
- `specs/single-proposer-architecture.md` -- EXISTS
- `specs/single-proposer-requirements.md` -- EXISTS
- `specs/gui-architecture.md` -- EXISTS
- `specs/gui-desktop-requirements.md` -- EXISTS
- `specs/improvements/apply-block-modularization.md` -- EXISTS
- `specs/improvements/cli-modularization.md` -- EXISTS
- `specs/improvements/consensus-modularization.md` -- EXISTS
- `specs/improvements/modularization-improvement.md` -- EXISTS
- `specs/improvements/scaling-100k-producers.md` -- EXISTS
- `specs/bugfixes/production-gate-deadlock-analysis.md` -- EXISTS
- `specs/bugfixes/reward-validation-analysis.md` -- EXISTS

**Files referenced but missing:** NONE

Files on disk in specs/:
(from glob) -- all 15 .md files (including SPECS.md itself) accounted for.

**Files on disk but not in index:** NONE

**SPECS.md is clean.**

### docs/DOCS.md

Files referenced in DOCS.md:
- `/WHITEPAPER.md` -- EXISTS
- `docs/protocol.md` -- EXISTS
- `docs/architecture.md` -- EXISTS
- `docs/security_model.md` -- EXISTS
- `docs/genesis.md` -- EXISTS
- `docs/testnet.md` -- EXISTS
- `docs/devnet.md` -- EXISTS
- `docs/genesis_ceremony.md` -- EXISTS
- `docs/infrastructure.md` -- EXISTS
- `docs/rewards.md` -- EXISTS
- `docs/cli.md` -- EXISTS
- `docs/docker.md` -- EXISTS
- `docs/running_a_node.md` -- EXISTS
- `docs/becoming_a_producer.md` -- EXISTS
- `docs/rpc_reference.md` -- EXISTS
- `docs/troubleshooting.md` -- EXISTS
- `docs/archiver.md` -- EXISTS
- `docs/disaster-recovery.md` -- EXISTS
- `docs/releases.md` -- EXISTS
- `docs/buy_doli.md` -- EXISTS
- `docs/faucet-bot.md` -- EXISTS
- `docs/producer_node_quickstart.md` -- EXISTS
- `docs/producer-ux-proposal.md` -- EXISTS
- `docs/configuration_verification.md` -- EXISTS
- `docs/manifesto.md` -- EXISTS
- `docs/roadmap.md` -- EXISTS
- `docs/auto_update_system.md` -- EXISTS
- `docs/whitepaper_test_plan.md` -- EXISTS
- `docs/battle_test.md` -- EXISTS
- `docs/attack_analysis.md` -- EXISTS
- `docs/extreme_devnet_600.md` -- EXISTS

**Files referenced but missing:** NONE

Files on disk in docs/ (excluding .workflow/, legacy/):
(from glob) 31 .md files + DOCS.md = 32 files total. All 31 non-index .md files are referenced in DOCS.md.

**Files on disk but not in index:** NONE

**DOCS.md is clean.**

---

## WHITEPAPER Parameter Check

| Parameter | WHITEPAPER Value | Code Value | Match? |
|-----------|-----------------|------------|--------|
| Slot duration | 10 seconds (Section 5.2) | `SLOT_DURATION = 10` | YES |
| Epoch length | 360 slots (Section 5.2) | `SLOTS_PER_EPOCH = 360` | YES |
| Era length | 12,614,400 slots (Section 5.2) | `SLOTS_PER_ERA = 12_614_400` | YES |
| Block reward | 1 DOLI/block (Section 10.1) | `INITIAL_REWARD = 100_000_000` (= 1 DOLI) | YES |
| Total supply | 25,228,800 DOLI (Section 10.1) | `TOTAL_SUPPLY = 2_522_880_000_000_000` (= 25,228,800 DOLI) | YES |
| Bond unit | 10 DOLI (Section 7.2) | `BOND_UNIT = 1_000_000_000` (= 10 DOLI) | YES |
| Max bonds per producer | 3,000 (Section 7.3) | `MAX_BONDS_PER_PRODUCER = 3_000` | YES |
| Halving interval | 12,614,400 blocks (Section 10.1) | `HALVING_INTERVAL = 12_614_400` | YES |
| Block VDF iterations | T_BLOCK = 1,000 (Section 5.3) | `T_BLOCK = 800_000` (consensus/vdf.rs:15) | **NO** -- WHITEPAPER says 1,000, code says 800,000 |
| Registration VDF | T_REGISTER_BASE = 1,000 (Section 7.1) | `T_REGISTER_BASE = 1_000` (consensus/vdf.rs:72) | YES |
| VDF hash function | BLAKE3 (Section 5.1) | BLAKE3 via `crypto::Hasher` (heartbeat.rs) | YES |
| Unbonding period | 60,480 blocks / 7 days (Section 7.4) | `UNBONDING_PERIOD = 60_480` | YES |
| Inactivity threshold | 50 consecutive slots (Section 11.2) | `INACTIVITY_THRESHOLD = 50` | YES |
| Base block size | 2 MB (Section 6.3) | `BASE_BLOCK_SIZE = 2_000_000` | YES |
| Max block size cap | 32 MB (Section 6.3) | `MAX_BLOCK_SIZE_CAP = 32_000_000` | YES |
| Vesting schedule | 75%/50%/25%/0% (Section 7.4) | Code matches: 75/50/25/0 (constants.rs:271-276) | YES |
| Coinbase maturity | 6 confirmations (Section 10.7) | `COINBASE_MATURITY = 6` | YES |
| Commitment period | 4 years = 12,614,400 blocks (Section 7.4) | `COMMITMENT_PERIOD = VESTING_PERIOD_SLOTS = 12_614_400` | YES |
| Seniority weights | 1.0 / 1.75 / 2.50 / 3.25 / 4.0 (Section 9.1) | Need to verify in scheduler code | UNVERIFIED |
| VDF discriminant bits | Not specified in WHITEPAPER | `VDF_DISCRIMINANT_BITS = 1024` in code | N/A |

---

## Cross-Document Consistency Issues

| Issue | Documents | Detail |
|-------|-----------|--------|
| RPC method count | CLAUDE.md says "38 methods", DOCS.md says "RPC API documentation", rpc_reference.md title | Actual count from dispatch.rs is **39 methods** (getLoanList was added). CLAUDE.md is off by 1 |
| VDF construction | docs/security_model.md says SHA-256 hash chain; WHITEPAPER says BLAKE3; code uses BLAKE3 | docs/security_model.md is wrong |
| VDF iterations (block) | docs/security_model.md says 10M/700ms; docs/protocol.md says 800K/55ms; WHITEPAPER says 1,000; code says 800,000 | WHITEPAPER wrong (1K vs 800K), docs/security_model.md wrong (10M), docs/protocol.md correct (800K) |
| VDF iterations (registration) | docs/security_model.md says 600M/10min (Wesolowski); docs/protocol.md says 600M; code says 1,000 (hash-chain) | Both docs wrong. Code uses 1,000 iterations hash-chain, not Wesolowski |
| Max bonds per producer | docs/security_model.md says 10,000 AND 100 (self-contradictory); code says 3,000 | Both numbers in docs/security_model.md are wrong |
| GossipSub mesh_n | docs/protocol.md says 6/4/12; code mainnet default is 12/8/24 | docs/protocol.md values are stale |
| Testnet seeds | genesis_ceremony.md step 3 says "ai3 -- all 6 seeds" | Seeds are on ai1+ai2+ai3 (2 per server). Not all on ai3 |

---

## Summary

| Category | Count |
|----------|-------|
| **docs/archiver.md** | 2 drift items (code reference paths) |
| **docs/genesis.md** | 0 drift items (clean) |
| **docs/genesis_ceremony.md** | 2 drift items (seed location, identity key file) |
| **docs/protocol.md** | 10 drift items (major: SyncRequest/Response variants, TxType enum, GossipSub params, VDF params, Input/Output fields) |
| **docs/security_model.md** | 6 drift items (VDF hash function, VDF iterations, VDF construction type, max bonds x2) |
| **Index verification** | 0 drift items (both SPECS.md and DOCS.md are clean) |
| **WHITEPAPER alignment** | 1 drift item (T_BLOCK: paper says 1,000, code says 800,000) |
| **Cross-document consistency** | 7 issues |
| **Total drift items** | **28** |
| **Critical** | **10** (VDF construction/params wrong in 2 docs, TxType enum completely stale in docs/protocol.md, SyncRequest missing 3 variants, max bonds wrong) |
| **Minor** | **18** (code reference paths, GossipSub param values, Input/Output field additions, RPC count, identity key filename) |

---

## docs/rpc_reference.md -- RPC/CLI Sync Pass (2026-03-29)

Method: Read all files in `crates/rpc/src/methods/`, `crates/rpc/src/types/`, `crates/rpc/src/ws.rs`,
`crates/rpc/src/server.rs`, `bins/cli/src/commands.rs`, plus `consensus/constants.rs` and
`network_params/defaults.rs`. Compared every method, parameter, response field, and constant against docs.

### Method Count Verification

- **Code (dispatch.rs)**: 39 methods registered in match statement
- **Doc (rpc_reference.md)**: 39 methods in summary table
- **Result**: MATCH

### Drift Found and Fixed

| # | Section | Issue | Code Reality | Status |
|---|---------|-------|-------------|--------|
| 1 | getTransaction | Doc said "returns pending mempool transactions" | Code checks mempool FIRST, then falls back to confirmed lookup via tx_index (block_store.get_tx_block_height) | FIXED |
| 2 | getTransaction | Response missing `blockHeight` field for confirmed txs | Code returns `blockHeight` when found via tx_index | FIXED |
| 3 | getHistory | Doc showed only `address` and `limit` params | Code also accepts `before_height` (optional, for pagination) | FIXED |
| 4 | getHistory | Doc said limit default "50" | Code hard-caps at `limit.min(100)`, uses `addr_tx_index` not block scanning | FIXED |
| 5 | getNetworkParams | bondUnit was `1000000000000` (1000x too high) | Code: `self.bond_unit` = `1_000_000_000` (mainnet), `100_000_000` (testnet) | FIXED |
| 6 | getNetworkParams | coinbaseMaturity was `100` | Code: `self.coinbase_maturity` = `6` (from `COINBASE_MATURITY`) | FIXED |
| 7 | getNetworkParams | genesisTime was `0` | Code: `GENESIS_TIME = 1774540572` | FIXED |
| 8 | getProducers | Response missing fields: addressHash, bondAmount, blsPubkey, pendingWithdrawals, pendingUpdates | Code returns full `ProducerResponse` struct with all these fields | FIXED |
| 9 | getProducers | Missing "pending" status for pending registrations | Code appends pending_registrations() with status "pending" | FIXED |
| 10 | getProducer | Response missing fields: addressHash, bondAmount, bondCount, blsPubkey, pendingWithdrawals, pendingUpdates | Code returns full `ProducerResponse` | FIXED |
| 11 | getProducer | Missing statuses: "unbonding", doc only had "active"/"exited"/"slashed" | Code has 4 statuses: active, unbonding, exited, slashed | FIXED |
| 12 | getBondDetails | Response had wrong fields: `outpoint`, `quarter` (don't exist) | Code returns `BondEntryResponse` with: creationSlot, amount, ageSlots, penaltyPct, vested, maturationSlot | FIXED |
| 13 | getBondDetails | Missing top-level fields: registrationSlot, ageSlots, penaltyPct, vested, maturationSlot, withdrawalPendingCount | Code returns full `BondDetailsResponse` struct | FIXED |
| 14 | Section 13 Hex Encoding | Doc said "0x prefix" | Code uses `hex::encode()` everywhere -- plain hex, NO 0x prefix | FIXED |
| 15 | Section 14 Rate Limiting | Doc claimed 100 req/s, 200 burst, HTTP 429 | Code has NO rate limiting. Only `MAX_BODY_SIZE = 256 * 1024` (256 KB body limit) | FIXED |
| 16 | Section 16 WebSocket | Doc said "planned but not yet implemented" | Code has full WebSocket implementation at `/ws` with `new_block` and `new_tx` events via tokio broadcast channel (256 buffer) | FIXED |
| 17 | All examples | Used `0x` prefixed hashes throughout | All hashes in code are plain hex | FIXED (replaced all occurrences) |
| 18 | getBlockByHash params | Said "hex, 0x-prefixed" | Plain hex, no prefix | FIXED |

### Clean Items (No Drift)

- Method count: 39 in both code and doc (MATCH)
- getChainInfo fields: all present and correct
- getBlockByHeight: correct
- sendTransaction: correct
- getBalance: correct
- getUtxos: correct
- getMempoolInfo / getMempoolTransactions: correct
- getNetworkInfo: correct
- getNodeInfo: correct
- getEpochInfo: correct
- getSlotSchedule / getProducerSchedule / getAttestationStats: correct
- submitVote / getUpdateStatus / getMaintainerSet / submitMaintainerChange: correct
- getBlockRaw / backfillFromPeer / backfillStatus / verifyChainIntegrity: correct
- getStateRootDebug / getUtxoDiff: correct
- getStateSnapshot: correct
- getPeerInfo: correct
- Pool methods (getPoolInfo, getPoolList, getPoolPrice, getSwapQuote): correct
- Lending methods (getLoanInfo, getLoanList): correct
- Amount units table: correct (1 DOLI = 100,000,000 base units)
- Vesting quarters table: correct (75%/50%/25%/0%)

---

## docs/cli.md -- CLI Sync Pass (2026-03-29)

Method: Read `bins/cli/src/commands.rs` (full enum + all subcommands). Compared against docs/cli.md.

### Command Count Verification

- **Code (commands.rs)**: 40 top-level Commands enum variants
- **Doc (cli.md)**: 40 commands documented
- **Result**: MATCH

### Drift Found and Fixed

| # | Section | Issue | Code Reality | Status |
|---|---------|-------|-------------|--------|
| 1 | producer register | Doc said "1-3,000" bonds range | Code says `1-10000` in help text. Consensus MAX_BONDS_PER_PRODUCER=3,000 is enforced at node validation, not CLI | FIXED |
| 2 | producer add-bond | Doc said "1-3,000" bonds range | Code says `1-10000` in help text | FIXED |

### Clean Items (No Drift)

- All 40 commands present in doc
- Wallet commands: correct
- Send/receive: correct
- Producer commands (status, exit, withdraw, simulate-withdraw, delegate, revoke-delegation): correct
- Bond commands: correct (after fix)
- Network commands: correct
- Debug/admin commands: correct

---

## Updated Summary (including RPC/CLI pass)

| Category | Count |
|----------|-------|
| **docs/rpc_reference.md** | 18 drift items (all fixed) |
| **docs/cli.md** | 2 drift items (all fixed) |
| **docs/archiver.md** | 2 drift items (code reference paths) |
| **docs/genesis.md** | 0 drift items (clean) |
| **docs/genesis_ceremony.md** | 2 drift items (seed location, identity key file) |
| **docs/protocol.md** | 10 drift items (major: SyncRequest/Response, TxType, GossipSub, VDF) |
| **docs/security_model.md** | 6 drift items (VDF hash, iterations, construction, max bonds) |
| **Index verification** | 0 drift items (both SPECS.md and DOCS.md are clean) |
| **WHITEPAPER alignment** | 1 drift item (T_BLOCK) |
| **Cross-document consistency** | 7 issues |
| **Total drift items** | **48** (28 from first pass + 20 from RPC/CLI pass) |
| **Fixed this session** | **20** (18 in rpc_reference.md + 2 in cli.md) |
