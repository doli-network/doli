# UTXO / extra_data / snap-sync scalability — verified fact base

**Role:** Analyst, `/omega-redesign` Step 0. **Proposal-only.** This document contains **no architecture**.
Its job is to give the architect real numbers so the design argument is not run on recollection.
**Date:** 2026-09-14. **Source of truth:** code. Every number carries a `file:line`.
Anything not read from code is marked **UNVERIFIED**.

---

## 0. Premise corrections (PRIOR-KNOWLEDGE-GATE)

The refined prompt flagged four anchors. All four are now resolved against code.

| User recollection | Verdict | Verified fact |
|---|---|---|
| "500 KB `extra_data`" | **Close, per-OUTPUT not per-tx** | `BASE_EXTRA_DATA_SIZE = 524_288` (512 KiB) **per output**, `crates/core/src/transaction/output.rs:15`. No per-tx cap in consensus. |
| "a snap-sync parameter programmed to increase every few years" | **WRONG SUBSYSTEM** | The era-doubling schedule exists but applies to **block size** (`max_block_size`, `crates/core/src/consensus/constants.rs:503-511`) and **extra_data** (`output.rs:33-40`). **Snap-sync has no growth schedule — it has a FIXED 16 MiB ceiling** (§1). The schedule makes the snap-sync problem *worse*, not better. |
| "NFTs go into extra_data" | **PARTLY** | `Output::nft()` stores only a **`content_hash`**, not the image (`output.rs:250-268`). The image path is `OutputType::EncryptedContent`, which stores the **ciphertext inline** (`types.rs:339-341`). |
| "NFT technology that allows splitting into shares" | **DOES NOT EXIST** | `FractionalizeNft = 29` and `RedeemNft = 30` are **PERMANENTLY TOMBSTONED, DO NOT REUSE** (`crates/core/src/transaction/types.rs:113-123`). Removed 2026-05-26. Fractional ownership is only *composable* via `Multisig + MintAsset + BurnAsset`, ≤127 shareholders. |

⚠ **CONTRADICTION vs. `CLAUDE.md`.** CLAUDE.md states "Oracle + DeFi gates are `u64::MAX` (frozen pre-activation)".
Code says mainnet `defi_activation_height: 0` (`network_params/defaults.rs:173`) and `amm_activation_height: 0` (`:196`).
**DeFi and AMM are LIVE on mainnet from genesis.** Only `oracle_activation_height` is `u64::MAX` (`:204`).
CLAUDE.md is drifted; registered in §8.

---

## 1. Hard limits, verified

| Limit | Constant / fn | Value | Defined at | **Enforced at** | Height-gated? | In NetworkParams? |
|---|---|---|---|---|---|---|
| **Snap-sync wire frame** | `MAX_SYNC_SIZE` | **16 MiB** (16_777_216) | `crates/network/src/protocols/sync.rs:22` | `sync.rs:268` (req), `sync.rs:294` (resp) | **No** | **No** |
| extra_data per **output** | `BASE_EXTRA_DATA_SIZE` → `max_extra_data_size(h)` | 512 KiB → 8 MiB cap | `crates/core/src/transaction/output.rs:15,18,33` | `crates/core/src/validation/transaction.rs:382-391` | **Yes — era schedule** | No (hard constant) |
| extra_data per **tx** | — | **NONE** | — | — | — | — |
| Max **block size** | `max_block_size(h)` | 2 MB → 32 MB cap (era 4+) | `crates/core/src/consensus/constants.rs:456,459,503-511` | `crates/core/src/validation/block.rs:133-139` (Full), `:305-311` (Light) | **Yes — era schedule** | `params.rs:171` mirror |
| Max **tx size** | `MempoolPolicy::max_tx_size` | 600 KiB | `crates/mempool/src/policy.rs:28` | `crates/mempool/src/pool.rs:491,894` | No | **No — MEMPOOL POLICY, NOT CONSENSUS** |
| Max **txs per block** | — | **NONE** (only byte budget) | — | — | — | — |
| Max inputs / outputs per tx | — | **NONE** (only non-empty checks) | `validation/transaction.rs:39,67` | — | — | — |
| Gossip frame | `GOSSIP_MAX_TRANSMIT_SIZE` | 2_065_536 B | `crates/network/src/gossip/config.rs:26-27` | `config.rs:218,353` | No | No |
| Builder user-data budget | `LARGE_BLOCK_USER_DATA_BUDGET` | 1_900_000 B | `constants.rs:449` | builder policy only | `large_block_activation_height` (mainnet **0**, `defaults.rs:223`) | Yes |
| **Value bounds** | `TOTAL_SUPPLY` | 2_522_880_000_000_000 (25,228,800 DOLI) | `constants.rs:519` | `validation/transaction.rs:365-370` per-output + running total | No | No |
| Per-tx transfer cap | — | **NONE** beyond `TOTAL_SUPPLY` and `u64` | — | `validation/transaction.rs:373-379` (checked_add) | — | — |
| **Dust / min output** | — | **NONE** — only `amount != 0` | — | `validation/transaction.rs:352-360` | No | No |
| Slot time | `SLOT_DURATION` | 10 s → **8,640 blocks/day** | `constants.rs:168` | — | No | Yes |
| Epoch length | `SLOTS_PER_EPOCH` | 360 blocks = 1 h → 24 epochs/day | `constants.rs:172` | — | No | Yes |
| Era length | `SLOTS_PER_ERA` | 12_614_400 = **3.998 years** | `constants.rs:213` | — | — | No |
| Fee floor | `BASE_FEE`/`FEE_PER_BYTE`/`FEE_DIVISOR` | 1 / 1 / 100 | `constants.rs:638,649,653` | `transaction/core.rs:696-701` | No | No |
| Blob commitment threshold | (literal) | 4096 B | `crates/core/src/block.rs:401` | `block.rs:395-417` (`data_root`) | No | No |
| Undo retention | `UNDO_KEEP_DEPTH` | (see const) | `bins/node/src/node/apply_block/mod.rs:395` | per-block prune | No | No |

### 1.1 Snapshot format — what actually crosses the wire

`StateSnapshot` (`crates/storage/src/snapshot.rs:212-221`) = `{ block_hash, block_height, chain_state_bytes, utxo_set_bytes, producer_set_bytes, state_root }`,
plus optional `block_header_bytes`, `epoch_bond_snapshot_bytes`, attestation accumulators (`crates/network/src/protocols/sync.rs:101-130`).

- `utxo_set_bytes = utxo_set.serialize_canonical()` — **the entire UTXO set**, `snapshot.rs:238`.
- **Not chunked. Not compressed. Not streamed.** One `bincode` blob, one request/response (`sync.rs:255-305`).
- Per-entry canonical encoding: **36 B outpoint + 61 B fixed + extra_data**, `crates/storage/src/utxo/types.rs:48-79`, `in_memory.rs:360-370`.
- ⇒ **`extra_data` bytes are INLINE in the snapshot, in the state root, and in every node's RAM, forever.**

**Therefore the hard ceiling is:** `16_777_216 / 97 ≈ **172,961 UTXOs**` with zero extra_data. Fewer with any.
Beyond it, `read_response` returns `InvalidData` (`sync.rs:293-299`) and **no new node can ever join the network.**

**UNVERIFIED:** whether any chunked/`GetUtxoRange` path exists. Grep for `CHUNK|chunk|MAX_SNAPSHOT` across `crates/network/src/` returned **zero** non-gossip hits. Treated as: does not exist.

---

## 2. Capability inventory

### 2.1 `TxType` — **25 variants** (`crates/core/src/transaction/types.rs`)
CLAUDE.md says 24 → drift (§8).

| # | Variant | What it does | Mainnet gate |
|---|---|---|---|
| 0 | `Transfer` | value transfer | active |
| 1 | `Registration` | register producer, mints Bond UTXOs | active |
| 2 | `Exit` | start unbonding | active |
| 3 | `ClaimReward` | claim accumulated rewards | active |
| 4 | `ClaimBond` | claim bond after unbonding | active |
| 5 | `SlashProducer` | slash with evidence | active |
| 6 | `Coinbase` | block reward → reward pool | active |
| 7 | `AddBond` | bond stacking | active |
| 8 | `RequestWithdrawal` | instant withdrawal w/ vesting penalty | `inc_i_171_…: 418_000` (`defaults.rs:266`) |
| 9 | `ClaimWithdrawal` | **TOMBSTONE — wire compat only** | n/a |
| 10 | `EpochReward` | automatic weighted-presence payout | active |
| 11 | `RemoveMaintainer` | 3/5 maintainer removal | active |
| 12 | `AddMaintainer` | 3/5 maintainer addition | active |
| 13 | `DelegateBond` | delegate weight to T1/T2 | `delegation_auth_…: 0` |
| 14 | `RevokeDelegation` | revoke delegation | active |
| 15 | `ProtocolActivation` | schedule a protocol version at a future epoch | active |
| 16 | `PriceAttestation` | oracle price submission | **FROZEN** `oracle_activation_height: u64::MAX` (`defaults.rs:204`) |
| 17 | `MintAsset` | mint fungible asset | `defi_activation_height: 0` → **ACTIVE** |
| 18 | `BurnAsset` | burn fungible asset | **ACTIVE** |
| 19 | `CreatePool` | AMM pool create | `amm_activation_height: 0` → **ACTIVE** |
| 20 | `AddLiquidity` | AMM | **ACTIVE** |
| 21 | `RemoveLiquidity` | AMM | **ACTIVE** |
| 22 | `Swap` | AMM swap | **ACTIVE** |
| 29,30 | *(FractionalizeNft, RedeemNft)* | **PERMANENTLY TOMBSTONED** (`types.rs:113-123`) | never shipped |
| 31 | `ZKSettle` | verify a ZK proof of an L2 state transition | **FROZEN** `ZK_SETTLE_ACTIVATION_HEIGHT = u64::MAX` (`crates/core/src/validation/zk.rs:44`) |
| 32 | `RotateBlsKey` | rotate producer BLS key | `bls_key_rotation_…: 457_855` (`defaults.rs:258`) — **crossed 2026-09-14** |

**Free discriminants:** 23–28 (6 slots), 33+. 11–12 in `OutputType` are tombstoned.

### 2.2 `OutputType` — **14 variants** (`types.rs`)

| # | Variant | Where the payload lives | Mainnet |
|---|---|---|---|
| 0 | `Normal` | `extra_data` **must be empty** (`validation/transaction.rs:~404`) | active |
| 1 | `Bond` | 4-byte `creation_slot` (`output.rs:162`) | active |
| 2 | `Multisig` | condition-prefixed extra_data | active |
| 3 | `Hashlock` | condition-prefixed (32-B hash) | active |
| 4 | `HTLC` | condition-prefixed | active |
| 5 | `Vesting` | condition-prefixed | active |
| 6 | `NFT` | `[condition][1B ver][32B token_id][32B **content_hash**]` — **hash only, not the image** (`output.rs:250-268`) | active |
| 7 | `FungibleAsset` | `[condition][1B ver][32B asset_id][8B supply][1B ticker_len][ticker≤12]` (`output.rs:55-58`) | active |
| 8 | `BridgeHTLC` | condition-prefixed | active |
| 9 | `Pool` | reserves in extra_data; `amount==0` allowed | **ACTIVE** |
| 10 | `LPShare` | `[condition][1B ver][32B pool_id]` | **ACTIVE** |
| 11,12 | *(Collateral, LendingDeposit)* | **TOMBSTONED** (`types.rs:247-252,322-324`) | never |
| 13 | `ZKRollup` | verifying_key + state_root in extra_data; `amount==0` allowed | **FROZEN** |
| 14 | `EncryptedContent` | `[ciphertext_len\|ciphertext\|wrapped_key\|nonce(12)\|content_hash(32)]` — **the only type that stores bulk payload inline** (`types.rs:339-341`) | `encrypted_content_activation_height: 0` → **ACTIVE** |
| 15 | `OraclePrice` | price data; `amount==0` allowed | **FROZEN** |

### 2.3 Covenant / condition primitives — **13 `Condition` variants** (`crates/core/src/conditions/mod.rs`)

`Signature(Hash)`, `Multisig{threshold,keys}`, `Hashlock(Hash)`, `Timelock(h)`, `TimelockExpiry(h)`,
`And`, `Or`, `Threshold{n,conditions}`, `AmountGuard`, `OutputTypeGuard`, `RecipientGuard`, `MaxDeltaGuard`, `ReserveRatioGuard`.
Bounds: `MAX_CONDITION_DEPTH`, `MAX_CONDITION_OPS`, ≤127 keys, encoded size ≤ `MAX_EXTRA_DATA_SIZE` (`conditions/encoding.rs:26`).
**6 composed templates** (`conditions/templates.rs:36,70,105,143,190,229`): `vault`, `escrow`, `htlc_payment`, `subscription`, `agent_allowance`, `escrow_loan`.

### 2.4 "Anchor a 32-byte hash cheaply" — **the primitive already exists**

| Path | extra_data bytes | UTXO entry bytes | Fee (base units) |
|---|---|---|---|
| `NFT` + `Condition::Signature` | 34 (cond: 1 ver + 1 tag + 32, `encoding.rs:23,36,185`) + 1 + 32 + 32 = **99** | 36 + 61 + 99 = **196** | 1 + 99/100 = **1** |
| `Hashlock` + `Condition::Hashlock` | ~34 | ~131 | **1** |

⇒ **DOLI can already anchor a document hash today, on mainnet, for 1 base unit (0.00000001 DOLI).**
The gap is **not capability**. The gap is that **every anchor is a permanent 196-byte UTXO that every node must hold in RAM and re-transmit in every future snap-sync**, and nothing prices or bounds that.

### 2.5 `data_root` — the one existing "commit, don't store" seam

`crates/core/src/block.rs:395-417`: any output with `extra_data.len() >= 4096` has `BLAKE3(extra_data)` folded into a sorted
`data_root` in the **block header** (`block.rs:61-66`), committed by the header hash (`block.rs:91`).
**But the bytes still live in the UTXO entry regardless.** `data_root` is a commitment *in addition to*, not *instead of*, storage.

### 2.6 What `specs/l2-settlement.md` reserves

| Reserved | Value | Spec ref |
|---|---|---|
| `TxType::ZKSettle` | 31 | §4.2; code `types.rs:134` |
| `OutputType::ZKRollup` | 13 | §4.1; code `types.rs:255` |
| Blob semantics | proof lives in `ZKRollup.extra_data`, auto-committed by `data_root` (≥4096 B) | §3 line 51, §6 line 234, §8.2 recommendation Option 1 |
| Activation | `ProtocolActivation` → future epoch; currently `u64::MAX` | §7; code `zk.rs:44` |
| L1 scope | verifier + commitment anchor **only**; L1 does **not** store rollup tx data | §1 line 17, §2 |

---

## 3. Data-flow map of the scaling-relevant path

```
Block (gossip ≤2,065,536 B  crates/network/src/gossip/config.rs:26)
  └─> validate_block_with_mode         crates/core/src/validation/block.rs:112
        ├─ block.size() ≤ max_block_size(h)              block.rs:133 / :305
        └─ per-output extra_data ≤ max_extra_data_size(h) validation/transaction.rs:382
  └─> apply_block()                     bins/node/src/node/apply_block/ (dir)
        ├─ UtxoSet::InMemory  (HashMap<Outpoint,UtxoEntry>)  utxo/in_memory.rs:22-23
        └─ UtxoSet::RocksDb   (CF_UTXO, bincode values)      utxo/set.rs:442-446
  └─> update_chain_state_for_block      apply_block/state_update.rs:41
        state root is NOT computed here (M2 lazy)            state_update.rs:38-40
  └─> serve_state_root()  [lazy + memoized] bins/node/src/node/state_root_serve.rs:73
        └─ compute_state_root()          crates/storage/src/snapshot.rs:24-58
              ├─ chain_state.serialize_canonical()   140 B fixed
              ├─ utxo_set.serialize_canonical()      **O(|UTXO| + Σ extra_data)**
              └─ producer_set.serialize_canonical()  O(producers)
  └─> StateSnapshot::create()            snapshot.rs:229-262
  └─> SyncResponse::StateSnapshot        crates/network/src/protocols/sync.rs:101
        └─ write/read bounded by MAX_SYNC_SIZE = 16 MiB      sync.rs:268,294
  └─> peer: UtxoSet::deserialize_canonical() → **InMemory**  utxo/set.rs:453
        then INV-SYNC-014 requires swap to state_db backend
Archive: ArchiveBlock channel → crates/storage/src/archiver.rs:24 (append-only)
```

### Complexity ledger

| Quantity | Complexity | Evidence |
|---|---|---|
| Snapshot bytes | **O(N + Σ extra_data)** | `in_memory.rs:360-370` |
| State-root compute | **O(N log N)** (sort) + 2× full materialization | `in_memory.rs:361-362`; RocksDB path collects `Vec<(Box<[u8]>,UtxoEntry)>` **then** builds the buffer → `crates/storage/src/state_db/queries.rs:483-501` |
| Transient RAM per state-root call | **≈2× serialized set**, on BOTH backends | ditto |
| Resident RAM (InMemory backend) | **O(N + Σ extra_data)**, permanent | `in_memory.rs:22-23` |
| Resident RAM (RocksDb backend) | disk-resident, but **spikes to O(full set) on every state-root/snapshot call** | `queries.rs:486-491` |
| Snap-sync wire | **O(N + Σ extra_data)** in ONE frame, hard-capped | `sync.rs:22,294` |
| Block archive | O(total block bytes), append-only, no pruning in the archive path | `archiver.rs` |
| Per-block apply | O(tx inputs+outputs) — **fine, not a bottleneck** | — |

**Nothing in this chain is sublinear in UTXO count. Nothing is incremental. There is no Merkle/IAVL/Verkle accumulator —
the "state root" is `BLAKE3` over a full re-serialization of the entire set.**

---

## 4. Growth math

### 4.1 Live measurement (probe, not recollection)

| Probe | Value |
|---|---|
| `getChainInfo` @ `127.0.0.1:8500` (local testnet, 2026-09-14) | height **191,413**, network testnet, v6.30.0 |
| `getChainStats` | **utxoCount = 23,501**, addressCount = **15**, activeProducers = **5** |
| `du -sh ~/testnet/seed/data/*` | blocks **106 MB**, checkpoints **104 MB**, state_db **3.0 MB** |

Derived: serialized snapshot ≈ 23,501 × 97 = **2,279,597 B = 2.18 MiB = 13.6 % of the 16 MiB ceiling**
— on a chain with **15 addresses and zero users**.
Growth rate: 23,501 / 191,413 = **0.1228 UTXO/block** = **1,061 UTXO/day** at 8,640 blocks/day.
Blocks on disk: 106 MB / 191,413 = **554 B/block** (near-empty).

**Assumption (stated):** the 0.1228 UTXO/block baseline is producer-driven (epoch rewards, coinbase change) and
scales roughly with producer count. Mainnet runs ~50 active producers vs. 5 here → baseline could be ~10× higher.
**UNVERIFIED** — mainnet seed RPC is closed to externals (memory: `project_bls_key_mismatch_restored_wallets.md`).

### 4.2 Time to the 16 MiB snap-sync ceiling

| Scenario | UTXO/day | Bytes/day | Days from 23,501 UTXOs to 172,961 |
|---|---|---|---|
| Today's testnet baseline (5 producers, 0 users) | 1,061 | 103 KB | **141 days (4.6 months)** |
| Baseline × 10 (mainnet-scale producers) | 10,610 | 1.03 MB | **14 days** |
| + hash registry @ 1,000 anchors/day (196 B each) | +1,000 | +196 KB | 12.8 days |
| + hash registry @ 10,000/day | +10,000 | +1.96 MB | **7.2 days** |
| + hash registry @ 100,000/day | +100,000 | +19.6 MB | **< 1 day** |

### 4.3 Adversarial `extra_data` abuse (era 0)

- Max extra_data outputs per block: `1,900,000 / 524,288 = 3` → 1,572,864 B/block of permanent state.
- Per day: `8,640 × 1,572,864 = 13,589,544,960 B` = **12.66 GiB/day**.
- **Cost:** `minimum_fee = 1 + 524_288/100 = 5,243` base units/output × 3 × 8,640 = 135,898,560 base units
  = **1.359 DOLI/day** (`transaction/core.rs:696-701`).
- **Time to break snap-sync from today's testnet state:** headroom 16,777,216 − 2,279,597 = 14,497,619 B
  ÷ 1,572,864 B/block = **9.2 blocks ≈ 92 seconds.**

⚠ **CONTRADICTION (doc vs. arithmetic).** `crates/core/src/transaction/core.rs:695` claims
"512 KB image: 5,243 sats (~0.052 DOLI)". 5,243 base units ÷ 100,000,000 = **0.00005243 DOLI**.
The comment is wrong by **1000×**. The real price of 512 KB of permanent, every-node, forever state is
**five thousandths of a cent's worth of DOLI**. Registered in §8.

### 4.4 Government hash-registry projection (benign workload)

Assumption: 1 anchor = 1 tx, 1 `NFT`+`Signature` output (196 B UTXO, §2.4), 1 change output replacing 1 spent input (net 0).

| N anchors/day | 1 yr UTXOs added | 1 yr snapshot bytes | 5 yr snapshot bytes | vs. 16 MiB ceiling @ 1 yr |
|---|---|---|---|---|
| 1,000 | 365,000 | **71.5 MB** | 358 MB | **4.3× over** |
| 10,000 | 3,650,000 | **715 MB** | 3.58 GB | **43× over** |
| 100,000 | 36,500,000 | **7.15 GB** | 35.8 GB | **426× over** |
| 1,000,000 | 365,000,000 | **71.5 GB** | 358 GB | **4,260× over** |

Transient RAM per state-root computation ≈ 2× the serialized set (§3): at 10,000/day × 1 yr ≈ **1.4 GB spike per call**,
on every node, on both backends.
Block archive at 10,000 anchors/day: 10,000 × ~250 B tx + 554 B/block baseline ≈ **7.3 MB/day = 2.7 GB/yr** — *not* the bottleneck.
Block archive under §4.3 abuse: **12.66 GiB/day = 4.6 TB/yr** — a bottleneck, but snap-sync dies ~4 orders of magnitude sooner.

### 4.5 The era schedule makes it worse

`max_block_size` and `max_extra_data_size` both double every `SLOTS_PER_ERA` = 3.998 years, **unconditionally, no vote, no gate**
(`constants.rs:503-511`, `output.rs:33-40`). At era 1 the abuse rate doubles to 25.3 GiB/day.
`MAX_SYNC_SIZE` **does not grow**. The gap between what the chain accepts and what snap-sync can carry **widens by 2× every 4 years.**

---

## 5. Institutional memory — constraints on any redesign

**Active invariants** (from `.omega/memory.db`, `invariants` where `status='active'`):

| ID | Constraint on the redesign |
|---|---|
| **INV-SYNC-007** | Every node — *including a freshly snap-synced one* — must converge to the **bit-identical** 3-state (height→stateRoot, csHash, psHash, utxoHash). **Any new snapshot encoding must be bit-reproducible by a full-sync node.** |
| **INV-SYNC-014** | After snap-sync install, `self.utxo_set` MUST be the **state_db-backed** variant. The snapshot deserializes to `InMemory`; leaving it there is a live defect. **A chunked/streamed snapshot must land in state_db, not RAM.** |
| **INV-EPOCH-002** | `rebuild_epoch_state_from_blocks()` MUST detect incomplete block history (snap-synced node) and skip the attestation scan. **"Rebuild from blocks" is unsafe on any snap-synced node** — so any design that relies on replaying history to recover state is disqualified for snap-synced nodes. |
| **INV-UTXO-001** | `spend_transaction` errors MUST propagate; conservation `total_after` must hold. **Any UTXO-set restructuring must preserve the conservation check path.** |
| **INV-EPOCH-001** | Epoch state MUST NEVER be deleted on protocol-version mismatch; use `EPOCH_STATE_FORMAT_VERSION`. **Do not bump `CURRENT_PROTOCOL_VERSION` for a storage-format change.** |
| **INV-STORAGE-001** | Every RocksDB instance must set explicit `db_write_buffer_size` + `max_total_wal_size`. |
| **INV-STORAGE-108** | Per-block maintenance must be **O(1)** — no `compact_range_cf` from the apply loop. **Any per-block accumulator update must be O(1) amortized.** |

**Hotspots** (`hotspots`, by `times_touched`): `crates/network/src/sync/manager/mod.rs` (**critical, 23**),
`sync/manager/production_gate.rs` (critical, 18), `sync/manager/cleanup.rs` (critical, 16).
**The snap-sync manager is the single most incident-prone module in the repo.** Touch it with maximum caution.

**Postmortems that constrain a redesign:**

| Doc | Constraint |
|---|---|
| `docs/postmortems/2026-03-14-snap-sync-cascade.md` | Snap-sync re-entry is a **cascade amplifier**: 12 snap syncs on one node in 21 h fragmented 30 nodes across testnet **and mainnet**. Recovery required full wipe + rebuild from one canonical source. **A redesign that increases snap-sync frequency or duration multiplies this risk.** |
| `docs/postmortems/2026-07-21-inc-i-143-snapsync-cascade.md` | **Status: OPEN, root cause NOT established.** D1: snap-sync admits a peer-supplied anchor height **without validation**. D6: archive auto-repair is **self-referential**. D7: seed1 bootstraps to itself. **Designing on top of snap-sync means designing on top of an open incident.** |
| `docs/postmortems/2026-04-17-attestation-stability-pillars.md` | Any encoder/decoder pair MUST be verified for **index parity**. **A new snapshot encoder needs a byte-equality test against the decoder.** |

**Failed approaches:** query over `domain LIKE '%utxo%|%snap%|%storage%|%defi%|%nft%|%sync%|%l2%'` returned **zero rows** — no prior
attempt at this problem is recorded. This is **greenfield** for the architect.

---

## 6. Redesign acceptance criteria (MoSCoW)

### Behavior preservation — Must

| ID | Requirement | Priority | Acceptance criterion (measurable) |
|---|---|---|---|
| REQ-SCALE-001 | State-root determinism preserved | Must | Two nodes at the same height — one full-synced, one using the new path — return **byte-identical** `stateRoot`, `csHash`, `utxoHash`, `psHash` from `getStateRootDebug`. Satisfies INV-SYNC-007. |
| REQ-SCALE-002 | Snap-sync equivalence to full sync | Must | A snap-synced node and a genesis-synced node at the same height produce identical `utxoHash` **and** identical `utxoCount`. No divergence within 3 epochs. |
| REQ-SCALE-003 | **No genesis reset** | Must | Design activates at a **future height** via `NetworkParams` activation height. State root of every block `< AH` is unchanged (proved by replaying mainnet blocks 0..AH-1 and comparing roots). |
| REQ-SCALE-004 | Forward activation only | Must | The AH is a **new** field in `crates/core/src/network_params/`, never a reuse or a move of an existing one. `u64::MAX` on devnet until pinned. The INC-I-075 three-question checklist is answered in the commit. |
| REQ-SCALE-005 | All mainnet-activated primitives keep working | Must | Post-AH regression pass over: `Transfer`, `Registration`, `AddBond`, `RequestWithdrawal`, `EpochReward`, `DelegateBond`, `MintAsset`/`BurnAsset`, `CreatePool`/`AddLiquidity`/`RemoveLiquidity`/`Swap`, `NFT`, `EncryptedContent`, `RotateBlsKey`. Every existing UTXO remains spendable. |
| REQ-SCALE-006 | Existing invariants hold | Must | INV-SYNC-007, INV-SYNC-014, INV-EPOCH-001, INV-EPOCH-002, INV-UTXO-001, INV-STORAGE-108 each have a passing regression test referenced by ID. |
| REQ-SCALE-007 | Wire compatibility with un-upgraded peers until AH | Must | A pre-AH binary and a post-AH binary produce the same blocks and the same snapshot below the AH; deploy classification (rolling vs. synchronized, per INC-I-062) is stated explicitly. |

### Non-foreclosure — Must

| ID | Requirement | Priority | Acceptance criterion |
|---|---|---|---|
| REQ-SCALE-008 | Does not foreclose `specs/l2-settlement.md` | Must | `TxType=31` / `OutputType=13` remain reservable; `ZKRollup.extra_data` can still hold a 100–400 KB proof; `data_root` still commits it. Written confirmation against spec §4.1, §4.2, §6. |
| REQ-SCALE-009 | Does not foreclose a government hash registry **at scale** | Must | Design shows an explicit path to ≥ 100,000 anchors/day sustained for ≥ 5 years with snap-sync still functional (§4.4 row 3). |
| REQ-SCALE-010 | Does not foreclose fractional ownership | Must | The `Multisig + MintAsset + BurnAsset` composition path (`types.rs:113-123`) remains viable, and discriminants 23–28 / 33+ remain free for a future native primitive. |
| REQ-SCALE-011 | Does not foreclose the oracle | Must | `PriceAttestation=16` / `OraclePrice=15` remain activatable at a future height without a second redesign. |
| REQ-SCALE-012 | **Does not host NFT image/file payloads** (user constraint) | Must | Bulk-payload capacity **does not increase** for any new primitive introduced. If `EncryptedContent` capacity changes, it may only go down. |

### Structural properties — Should

| ID | Requirement | Priority | Acceptance criterion |
|---|---|---|---|
| REQ-SCALE-013 | Snapshot transfer bounded independent of UTXO-set size | Should | A node joining a chain with **10,000,000 UTXOs** completes state sync. No single wire frame exceeds `MAX_SYNC_SIZE`. Measured, not argued. |
| REQ-SCALE-014 | RAM bounded independent of UTXO-set size | Should | Peak RSS during state-root computation at 10,000,000 UTXOs ≤ 2 GB. Today this is ≈2× the serialized set (`queries.rs:486-491`). |
| REQ-SCALE-015 | Payload bytes never enter consensus state beyond a commitment | Should | For any new registry primitive, the UTXO entry carries **≤ 64 bytes** of payload (a commitment), not the payload. Verified by a size assertion test. |
| REQ-SCALE-016 | Permanent state is priced | Should | Cost of adding 1 MB of permanent UTXO-set bytes is ≥ **X DOLI** (X to be set by the architect + economist). Today it is 0.0000105 DOLI/MB (§4.3) — effectively zero. |
| REQ-SCALE-017 | Throughput target stated and met | Should | Sustained TPS target (current builder budget implies ~300 TPS, `constants.rs:449` + INC-I-091) is preserved or improved; measured on the local testnet. |
| REQ-SCALE-018 | State-root compute is incremental or epoch-cadenced | Should | Per-block state-root cost is O(changed entries), not O(N). Satisfies INV-STORAGE-108. |

### Could / Won't

| ID | Requirement | Priority | Note |
|---|---|---|---|
| REQ-SCALE-019 | UTXO-set pruning / expiry for anchor outputs | Could | Only if REQ-SCALE-001 (determinism) and REQ-SCALE-005 (spendability) are provably preserved. |
| REQ-SCALE-020 | Compression of the snapshot wire format | Could | A 3–5× constant-factor win. Buys time; does not change the O(N) shape. Explicitly **not** a substitute for REQ-SCALE-013. |
| REQ-SCALE-021 | Raising `MAX_SYNC_SIZE` | **Won't** | A constant-factor patch on an O(N) problem, and it re-opens the 2026-03 DoS vector the 64→16 MB reduction closed (`sync.rs:17-21`). Deferred as a symptom patch. |
| REQ-SCALE-022 | Native NFT fractionalization | **Won't** | Tombstoned 2026-05-26; composable today (§2.1). Out of scope for a scalability redesign. |
| REQ-SCALE-023 | Changing the era-doubling schedule | **Won't** (this iteration) | It is consensus history from genesis; flagged as a *worsening factor* (§4.5) for the architect to reason about, not to change here. |
| REQ-SCALE-024 | Any change to `CURRENT_PROTOCOL_VERSION` | **Won't** | INV-EPOCH-001 / INC-I-054. Storage format uses `EPOCH_STATE_FORMAT_VERSION`. |

### 6.1 Test traceability — M1 [F1] single-install snapshot

Written test-first; evidence in `docs/.workflow/m1-test-red-evidence.txt`.

| Requirement | Test ID | File | Status on unchanged code |
|---|---|---|---|
| REQ-SCALE-001 | `m1_installed_state_root_is_true_of_the_installed_backend` | `bins/node/tests/it/m1_snap_install_identity.rs` | PASS (lock) |
| REQ-SCALE-001 | `m1_cached_state_root_equals_snapshot_state_root` | `bins/node/tests/it/m1_snap_install_identity.rs` | PASS (lock) |
| REQ-SCALE-001 | `m1_root_mismatch_snapshot_is_rejected_and_installs_nothing` | `bins/node/tests/it/m1_snap_install_identity.rs` | PASS (lock) |
| REQ-SCALE-001 | `m1_envelope_mismatch_snapshot_is_rejected_and_installs_nothing` | `bins/node/tests/it/m1_snap_install_identity.rs` | PASS (lock) |
| REQ-SCALE-001 | `m1_decode_once_then_root_equals_root_from_bytes` | `crates/storage/tests/it/m1_state_root_decode_seam.rs` | PASS (lock) |
| REQ-SCALE-001 | `m1_truncated_utxo_blob_is_an_error_not_a_silent_short_set` | `crates/storage/tests/it/m1_state_root_decode_seam.rs` | PASS (lock) |
| REQ-SCALE-002 | `m1_installed_utxo_count_and_hash_match_the_snapshot_bytes` | `bins/node/tests/it/m1_snap_install_identity.rs` | PASS (lock) |
| REQ-SCALE-006 | `m1_installed_utxo_set_is_state_db_backed` (INV-SYNC-014) | `bins/node/tests/it/m1_snap_install_identity.rs` | PASS (lock) |
| REQ-SCALE-006 | `m1_decoded_set_round_trips_to_the_same_canonical_bytes` (INV-SYNC-007) | `crates/storage/tests/it/m1_state_root_decode_seam.rs` | PASS (lock) |
| REQ-SCALE-014 | `m1_probe_reports_install_peak` | `bins/node/tests/m1_snap_install_peak_alloc.rs` | PASS (instrument) |
| REQ-SCALE-014 | `m1_install_holds_at_most_two_full_set_copies` | `bins/node/tests/m1_snap_install_peak_alloc.rs` | **FAIL — measured 3.3 copies (RED)** |

Measured at N = 50,000 UTXOs: `M1_PEAK_BYTES=15778678`, `M1_SET_BYTES=4850008`,
`M1_FULL_SET_COPIES=3.3`. Floor witness `M1_DECODED_SET_COPIES=1.9` — one decoded
in-memory set already costs 1.9 serialized copies, so the 2.0 bound requires dropping
the decoded set before deriving the post-install root from the state_db backend.

### 6.2 Test traceability — M2 [F2] streaming canonical fold

Written test-first; evidence in `docs/.workflow/m2-test-red-evidence.txt`.

| Requirement | Test ID | File | Status on unchanged code |
|---|---|---|---|
| REQ-SCALE-001 | `m2_digest_equals_hash_of_the_canonical_image_in_memory` | `crates/storage/tests/m2_canonical_fold_byte_equality.rs` | **RED (API absent)** |
| REQ-SCALE-001 | `m2_digest_equals_hash_of_the_canonical_image_state_db` | `crates/storage/tests/m2_canonical_fold_byte_equality.rs` | **RED (API absent)** |
| REQ-SCALE-001 | `m2_range_concat_equals_the_canonical_body_in_memory` | `crates/storage/tests/m2_canonical_fold_byte_equality.rs` | **RED (API absent)** |
| REQ-SCALE-001 | `m2_range_concat_equals_the_canonical_body_state_db` | `crates/storage/tests/m2_canonical_fold_byte_equality.rs` | **RED (API absent)** |
| REQ-SCALE-006 | `m2_backends_agree_on_digest_and_range_bytes` (INV-SYNC-007) | `crates/storage/tests/m2_canonical_fold_byte_equality.rs` | **RED (API absent)** |
| REQ-SCALE-006 | `m2_folding_does_not_mutate_the_set` | `crates/storage/tests/m2_canonical_fold_byte_equality.rs` | **RED (API absent)** |
| REQ-SCALE-018 | `m2_range_makes_progress_when_max_bytes_is_below_one_entry` | `crates/storage/tests/m2_canonical_fold_byte_equality.rs` | **RED (API absent)** |
| REQ-SCALE-018 | `m2_range_wider_than_the_set_returns_the_whole_body_and_no_cursor` | `crates/storage/tests/m2_canonical_fold_byte_equality.rs` | **RED (API absent)** |
| REQ-SCALE-018 | `m2_range_starts_at_the_first_key_at_or_after_the_cursor` | `crates/storage/tests/m2_canonical_fold_byte_equality.rs` | **RED (API absent)** |
| REQ-SCALE-018 | `m2_range_past_the_last_key_is_empty_and_terminal` | `crates/storage/tests/m2_canonical_fold_byte_equality.rs` | **RED (API absent)** |
| REQ-SCALE-001 | `m2_canonical_digest_is_err_on_an_undecodable_entry` (AP-7) | `crates/storage/tests/m2_canonical_fold_fail_loud.rs` | **RED (API absent)** |
| REQ-SCALE-001 | `m2_canonical_range_is_err_on_an_undecodable_entry` (AP-7) | `crates/storage/tests/m2_canonical_fold_fail_loud.rs` | **RED (API absent)** |
| REQ-SCALE-001 | `m2_digest_over_a_corrupt_set_is_never_the_digest_of_the_survivors` | `crates/storage/tests/m2_canonical_fold_fail_loud.rs` | **RED (API absent)** |
| REQ-SCALE-001 | `m2_empty_set_digest_and_range_agree_with_the_canonical_image` | `crates/storage/tests/m2_canonical_fold_fail_loud.rs` | **RED (API absent)** |
| REQ-SCALE-001 | `m2_single_entry_digest_and_range_agree_with_the_canonical_image` | `crates/storage/tests/m2_canonical_fold_fail_loud.rs` | **RED (API absent)** |
| REQ-SCALE-001 | `m2_pre_m2_serialize_canonical_silently_drops_the_undecodable_entry` | `crates/storage/tests/m2_canonical_fold_fail_loud.rs` | PASS (PRE-M2 companion — delete with `serialize_canonical()`) |
| REQ-SCALE-014 | `m2_probe_reports_state_root_peak` | `crates/storage/tests/m2_canonical_digest_peak_alloc.rs` | PASS (instrument) |
| REQ-SCALE-001 | `m2_probe_backends_agree_on_utxo_hash` | `crates/storage/tests/m2_canonical_digest_peak_alloc.rs` | PASS (lock) |
| REQ-SCALE-006 | `m2_utxo_size_monitor_reports_the_pinned_canonical_length` | `crates/storage/tests/it/m2_caller_port_locks.rs` | PASS (lock) |
| REQ-SCALE-006 | `m2_utxo_size_monitor_counts_every_recomputation` | `crates/storage/tests/it/m2_caller_port_locks.rs` | PASS (lock) |
| REQ-SCALE-006 | `m2_utxo_size_monitor_on_an_empty_set_is_the_eight_byte_header` | `crates/storage/tests/it/m2_caller_port_locks.rs` | PASS (lock) |
| REQ-SCALE-006 | `m2_utxo_size_monitor_equals_serialize_canonical_len` | `crates/storage/tests/it/m2_caller_port_locks.rs` | PASS (PRE-M2 companion) |
| REQ-SCALE-001 | `m2_state_root_over_the_fixed_state_is_the_pinned_golden` (INV-SYNC-007) | `crates/storage/tests/it/m2_caller_port_locks.rs` | PASS (lock) |
| REQ-SCALE-001 | `m2_pinned_state_root_is_backend_independent` | `crates/storage/tests/it/m2_caller_port_locks.rs` | PASS (lock) |

Measured at N = 100,000 UTXOs on unchanged code: `M2_SET_BYTES=10853285`,
`M2_PEAK_BYTES_INMEM=21000100` (1.93 full-set copies), `M2_PEAK_BYTES_ROCKS=19401302`
(1.79 copies), `M2_UTXO_HASH=e19b721e42c50867f3b32eaefed1d29933ada2e6ff09b99f390da1ceab03cdca`.

---

## 7. What I do not understand (stated before any design)

1. **What actually drives the 0.1228 UTXO/block baseline on a 15-address testnet.** I measured it; I did not trace the mechanism
   (epoch reward outputs? coinbase change? reward-pool churn?). This matters because it sets the floor growth rate. **UNVERIFIED.**
2. **Mainnet's real UTXO count.** Seed RPC is closed to externals. All mainnet projections here are extrapolations from testnet.
3. **Whether any snap-sync path is already chunked** in a way my grep missed (e.g. a `GetHeaders`-style paging over UTXOs).
   I found none, and I searched `crates/network/src/` for `CHUNK|chunk|MAX_SNAPSHOT|snapshot_size`.
4. **Whether `checkpoints/` (104 MB on the testnet seed) is bounded.** `--auto-checkpoint` rotates last 5 per CLAUDE.md;
   I did not read the rotation code.
5. **Whether the block store prunes.** `prune_blocks_below` exists (`crates/storage/src/block_store/maintenance.rs:314`);
   I did not verify whether production ever calls it.
6. **The real resident-RAM cost of `UtxoEntry` in the `HashMap`** — I computed serialized bytes, not Rust in-memory layout + allocator overhead.

---

## 8. Specs / docs drift detected

| File | Drift | Severity |
|---|---|---|
| `CLAUDE.md` ("If You Touch") | "Oracle + DeFi gates are `u64::MAX`" — **false for DeFi/AMM on mainnet**: `defi_activation_height: 0`, `amm_activation_height: 0` (`network_params/defaults.rs:173,196`). Only the oracle is frozen. | **High** — misleads any agent reasoning about live surface area. |
| `CLAUDE.md` (code map) | "`TxType`, 24 variants" — code has **25** (`RotateBlsKey=32` added). | Low |
| `crates/core/src/transaction/core.rs:695` | "512 KB image: 5,243 sats (~0.052 DOLI)" — **off by 1000×**; real value 0.00005243 DOLI. | **High** — this comment is the only in-repo statement of what permanent state costs, and it overstates it 1000×. |
| `specs/l2-settlement.md:47-48` | "13 variants defined (0–12, contiguous). Slot 13 is the next available" / "29 variants defined" — now 14 `OutputType` (13,14,15 used) and 25 `TxType`. Spec predates `ZKRollup`/`EncryptedContent`/`OraclePrice`/`RotateBlsKey`. | Medium |

*(Not corrected in this pass — `/omega-redesign` is proposal-only. Flagged for the sync-docs step.)*

---

## 9. Open questions for the user (6 max)

1. **Scale target.** What is the target sustained rate of document-hash anchors — 1,000/day, 100,000/day, or 1,000,000/day?
   §4.4 shows these are three *architecturally different* problems (4×, 426×, and 4,260× over today's ceiling at 1 year).
2. **Permanent-state pricing.** Today 512 KB of forever-state costs 0.0000524 DOLI. Are you willing to **raise the cost of
   permanent state** (a consensus-visible fee change at a future height), or must the fee schedule stay as-is and the fix be purely structural?
3. **Anchor permanence.** Must a government document anchor be retrievable from the **UTXO set** forever, or is it acceptable that it
   lives in the **block archive** forever and is *provable* by inclusion proof while being **prunable from live state**?
   This single answer decides whether the redesign is bounded or unbounded.
4. **`extra_data` capacity direction.** You said the network must not host NFT images. Are you willing to **reduce**
   `max_extra_data_size` at a future height (forward-only, existing UTXOs grandfathered), or must the 512 KB→8 MB era schedule be honored?
5. **Who runs a node?** If government-grade registry nodes are permissioned/well-resourced, a larger state budget is acceptable.
   If DOLI must stay joinable by a hobbyist on consumer hardware, the snap-sync bound is the binding constraint. Which is it?
6. **L2 timing.** `specs/l2-settlement.md` is a complete, frozen interface (`ZKSettle=31`, `ZKRollup=13`, `u64::MAX`).
   Is L2 in scope as *the* scaling answer for this redesign, or must L1 scale on its own first with L2 kept as a separate later decision?
