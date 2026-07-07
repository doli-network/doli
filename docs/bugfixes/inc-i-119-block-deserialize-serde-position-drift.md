# INC-I-119 — Archived blocks containing post-tombstone OutputType fail to deserialize

**Status:** Investigating (root cause confirmed, fix not implemented)
**Severity:** Medium — single block lost from one chain so far; class-level defect affects all blocks containing `EncryptedContent`, `OraclePrice`, or `ZKRollup` outputs that were archived BEFORE the B.1 tombstone landed.
**Domain:** storage / archive / deserialization / defi-tombstone
**Discovered:** 2026-06-11 during mainnet seed recovery (post-incident integrity scan)
**Related:** INC-I-088 (DeFi Phase 0 tombstoning), commits `726ec8f3` (B.1 lending tombstone), `74f6e081` (B.2 NFT-frac tombstone)

---

## Symptom

`verifyChainIntegrity` on all 3 mainnet seeds + producer n1 reports the **same single gap** at h=57682:

```json
{
  "complete": false,
  "fromHeight": 1,
  "missing": ["57682"],
  "missingCount": 1,
  "scanned": 420221,
  "tip": 420221
}
```

`bridgeFromArchive` finds the archive file (BLAKE3 checksum passes) but fails to deserialize:

```json
{
  "archive_found": true,
  "blocks_imported": 0,
  "status": "warning",
  "warning": "Failed to deserialize block at height 57682 (tried current and legacy formats)",
  "commitment_deleted": true
}
```

Direct deserialize test against `/tmp/0000057682.block` (67,123 bytes — anomalously large vs neighbors' 589 bytes) on v6.23.5:

```
[  Block (current)   ] ERR: invalid value: integer `14`, expected variant index 0 <= i < 14
[   LegacyBlockV3    ] ERR: invalid hash length
[   LegacyBlockV2    ] ERR: io error: unexpected end of file
[    LegacyBlock     ] ERR: invalid hash length
```

(Control: neighbor block `0000057683.block` deserializes successfully against all four shapes.)

---

## Root cause

Commit **`726ec8f3` (2026-05-26, "refactor(defi): B.1 tombstone native lending subsystem")** physically **removed** two variants from the `OutputType` enum declaration:

```diff
-    /// Lending collateral (locked loan collateral)
-    Collateral = 11,
-    /// Lending pool deposit receipt (depositor provides DOLI, earns interest)
-    LendingDeposit = 12,
+    // ─── TOMBSTONED: discriminants 11-12 (native lending outputs) ───
+    //   11 = Collateral      (tombstoned — was locked loan collateral)
+    //   12 = LendingDeposit  (tombstoned — was lending pool deposit receipt)
```

The intent was correct (no `Collateral` or `LendingDeposit` UTXOs ever existed on any chain — verified). **But serde's enum-variant encoding uses *declaration position*, not the explicit `repr(u8)` discriminant.** Removing two variants from the declaration shifted every later variant down by 2 serde positions:

| Variant            | OLD declaration position | NEW declaration position |
|--------------------|--------------------------|--------------------------|
| `LPShare`          | 10                       | 10                       |
| ~~`Collateral`~~   | ~~11~~                   | (removed)                |
| ~~`LendingDeposit`~~ | ~~12~~                 | (removed)                |
| `ZKRollup`         | 13                       | **11**                   |
| `EncryptedContent` | **14**                   | **12**                   |
| `OraclePrice`      | 15                       | **13**                   |

Block 57682 contains an `EncryptedContent` Output that was archived under the OLD declaration order, so its serde discriminator on disk is **14**. The current binary's `OutputType` enum has 14 variants (declaration positions 0–13). Bincode sees discriminator 14, finds it out of range, and rejects with the exact error the diagnostic test produced.

**Symmetric defect in TxType:** the same commit removed `LendingDeposit = 27` and `LendingWithdraw = 28` from TxType. Commit `74f6e081` (B.2) additionally removed `FractionalizeNft = 29`. None of those positions are referenced by block 57682, but any block written before May 26 containing TxType discriminants ≥ 14 (the first removal point in TxType's declaration order — also from B.1) is also at risk.

---

## Why neighbors deserialize and 57682 doesn't

Blocks 57680, 57681, 57683, 57684 are all 488–589 bytes — empty blocks with only a coinbase Transaction. A coinbase has a single Output of type `Normal` (position 0) — unaffected by the shift.

Block 57682 is 67,123 bytes. The block contains at least one `EncryptedContent` Output (position 14 in the old layout — the only OutputType at position ≥ 13 that fits the symptom). This makes 57682 the canary: the **only** block on mainnet that was archived under the old layout AND contains a downstream-of-shift OutputType.

The defect's blast radius on mainnet is therefore likely 1 block. But on testnets where lending features were exercised under the old layout, the count could be higher. **No automated audit of the archive directory has been run** — that's a follow-up.

---

## Evidence

| Source | Result |
|---|---|
| `verifyChainIntegrity` on ai1, ai2, ai3, n1 (separate runs) | All four report `missing=[57682]` and nothing else |
| `xxd` head/tail of `0000057682.block` | Magic prefix matches neighbors. Tail ends cleanly with `ff ff ff ff 7f` (matches neighbor tails). No truncation. |
| BLAKE3 sidecar verification | `fd27f754349f6fd2…` matches re-computed hash → bytes intact |
| `bridgeFromArchive` BLAKE3 pre-check passes, then deserialize fails | Confirms format issue, not corruption |
| Diagnostic test `crates/core/tests/inspect_h57682.rs` | Exact bincode error: `invalid value: integer 14, expected variant index 0 <= i < 14` |
| `git show 726ec8f3 -- crates/core/src/transaction/types.rs` | Removed `Collateral = 11` and `LendingDeposit = 12` from OutputType |
| `git log -S 'EncryptedContent'` | Variant introduced by commit `97e75530` — predates the tombstone |

---

## Impact

| Scope | Effect |
|---|---|
| **Consensus** | None. Block 57682's state (UTXOs, producer set) is already baked into the current `stateRoot`. The block_store entry would be a redundant artifact for archival queries. |
| **Liveness** | None. Seeds operate normally at h=420221 in recovery mode. |
| **Snap sync** | None. The snap sync floor is far above h=57682. |
| **Archival queries** | `getBlockByHeight: 57682` and `getBlockByHash` for that block's hash will return `null` / error. Any future feature that walks the full block range will encounter the hole. |
| **Chain integrity commitment** | The periodic BLAKE3 chain commitment (computed every 100 blocks at the tip) cannot complete a full-chain commitment until 57682 is either filled OR explicitly tombstoned in the commitment computation. |
| **Future archive scans** | Any pre-tombstone archive directory containing blocks with `EncryptedContent`, `OraclePrice`, or `ZKRollup` outputs will fail the same way on v6.23.5 and later. Risk: testnets, partner node archives, recovery from older snapshots. |

---

## Recommendations (3 options + recommended path)

### Option A — Legacy compat shim (RECOMMENDED)

Add a `LegacyOutputTypePreTombstone` enum (15-variant layout with `Collateral` + `LendingDeposit` at positions 11–12) and matching `LegacyOutputPreTombstone` / `LegacyTransactionPreTombstone` / `LegacyBlockPreTombstone` structs to `crates/core/src/transaction/legacy.rs`. Wire as a 5th fallback in `deserialize_block_compat()`. Translate to current types via `.into_current()`; **reject** with an error any Output carrying `Collateral` or `LendingDeposit` (none exist on any chain — guaranteed safe by audit).

**Pros**
- Mirrors the existing v3.5.0 / v3.6.0 / v3.7.1 compat-shim pattern in `legacy.rs` — proven mechanism.
- Zero changes to the live `OutputType` enum or its validation logic.
- Isolated to one file. Easy to review, no consensus risk.
- Reversible: if no v6.23.5+ binary ever encounters the file again, the shim is dead weight that can be deleted in a future cleanup.

**Cons**
- Adds ~80 LOC of legacy types and converter impls.
- Symmetric work needed in TxType (LegacyTxTypePreTombstone) for blocks that contain post-position-14 TxTypes archived pre-May-26 — if any exist (audit pending).
- Future tombstones must be aware of this pattern and add their own shim when they shift positions.

**Estimated effort**: 2–3 hours dev, 1 hour test (`inspect_h57682.rs` becomes the regression test asserting h=57682 deserializes to a valid `Block` and its hash matches `block.header.hash()`).

### Option B — Re-introduce dummy variants in the live enum

Add `_TombstonedCollateral = 11` and `_TombstonedLendingDeposit = 12` directly to the live `OutputType` (and symmetrically to `TxType` for `_TombstonedLendingDeposit27` / `_TombstonedLendingWithdraw28` / `_TombstonedFractionalizeNft29`). Validation layer rejects any block containing these at apply time.

**Pros**
- Simpler diff (~6 lines per enum).
- Restores the original serde positions exactly.

**Cons**
- Pollutes consensus-visible enums with dead variants.
- Risk of accidental construction in test code, RPC handlers, or future refactors.
- Sets a precedent that tombstones must keep dummy variants forever — adds enum bloat over time.
- Re-adds names (`_TombstonedCollateral`, etc.) that the codebase already excised — partial regression of the B.1 refactor's intent.

### Option C — Accept the gap

Update `bridgeFromArchive` to silently skip undeserialize-able files (log + continue rather than error). Document h=57682 as a known permanent archival hole. Do nothing about the format defect.

**Pros**
- Zero code risk.
- No binary deploy needed.

**Cons**
- Permanent archival hole. Block content is irrecoverable on any v6.23.5+ binary.
- Same defect will fire silently on any archive directory containing pre-tombstone EncryptedContent/OraclePrice/ZKRollup blocks. Could mask future incidents.
- Violates the principle that the archive is the system-of-record for historical block bytes.

### Decision: **Option A**

Rationale: matches the existing forward-compat pattern (`LegacyBlock` / `LegacyBlockV2` / `LegacyBlockV3` already exist for v3.5/v3.6/v3.7.1 Input changes — the same mechanism cleanly extends to handle position drift). Isolated to `legacy.rs`. No consensus-visible surface change. Recovers h=57682 cleanly. Future-proof: any subsequent enum tombstone should follow the same pattern (add to `legacy.rs`, never reorder live variants without one).

---

## Implementation sketch (Option A)

**File: `crates/core/src/transaction/legacy.rs`** — add ~80 LOC at the bottom:

```rust
// ─── v6.22.x pre-tombstone format (before commit 726ec8f3 B.1 lending tombstone, 2026-05-26) ───
//
// OutputType had 16 declared variants before B.1. The 2026-05-26 tombstone removed
// Collateral=11 and LendingDeposit=12, shifting later variants' serde positions down by 2.
// Any block archived before that commit containing OutputType ≥ position 13 fails on current binary.
// INC-I-119.

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum LegacyOutputTypePreTombstone {
    Normal = 0,
    Bond = 1,
    Multisig = 2,
    Hashlock = 3,
    HTLC = 4,
    Vesting = 5,
    NFT = 6,
    FungibleAsset = 7,
    BridgeHTLC = 8,
    Pool = 9,
    LPShare = 10,
    Collateral = 11,        // tombstoned — reject at conversion
    LendingDeposit = 12,    // tombstoned — reject at conversion
    ZKRollup = 13,
    EncryptedContent = 14,
    OraclePrice = 15,
}

impl LegacyOutputTypePreTombstone {
    pub fn into_current(self) -> Option<crate::transaction::OutputType> {
        use crate::transaction::OutputType;
        Some(match self {
            Self::Normal => OutputType::Normal,
            Self::Bond => OutputType::Bond,
            Self::Multisig => OutputType::Multisig,
            Self::Hashlock => OutputType::Hashlock,
            Self::HTLC => OutputType::HTLC,
            Self::Vesting => OutputType::Vesting,
            Self::NFT => OutputType::NFT,
            Self::FungibleAsset => OutputType::FungibleAsset,
            Self::BridgeHTLC => OutputType::BridgeHTLC,
            Self::Pool => OutputType::Pool,
            Self::LPShare => OutputType::LPShare,
            Self::Collateral | Self::LendingDeposit => return None, // tombstoned, reject
            Self::ZKRollup => OutputType::ZKRollup,
            Self::EncryptedContent => OutputType::EncryptedContent,
            Self::OraclePrice => OutputType::OraclePrice,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LegacyOutputPreTombstone {
    pub amount: crate::types::Amount,
    pub output_type: LegacyOutputTypePreTombstone,
    pub lock_until: u64,
    pub extra_data: Vec<u8>,
    pub recipient: crypto::PublicKey,
    // ... mirror all current Output fields, in original declaration order
}

impl LegacyOutputPreTombstone {
    pub fn into_current(self) -> Option<crate::transaction::Output> {
        let output_type = self.output_type.into_current()?;
        Some(crate::transaction::Output {
            amount: self.amount,
            output_type,
            lock_until: self.lock_until,
            extra_data: self.extra_data,
            recipient: self.recipient,
            // ...
        })
    }
}

// Same pattern: LegacyTransactionPreTombstone (uses Vec<LegacyOutputPreTombstone>),
// then LegacyBlockPreTombstone, then chain into deserialize_block_compat as 5th fallback.
```

**File: `crates/core/src/transaction/legacy.rs:226` (`deserialize_block_compat`)** — add fallback:

```rust
pub fn deserialize_block_compat(data: &[u8]) -> Option<crate::block::Block> {
    if let Ok(block) = bincode::deserialize::<crate::block::Block>(data) {
        return Some(block);
    }
    // NEW: pre-B.1-tombstone format (handles INC-I-119: blocks archived before 2026-05-26
    // containing EncryptedContent/OraclePrice/ZKRollup at OLD serde positions)
    if let Ok(legacy) = bincode::deserialize::<LegacyBlockPreTombstone>(data) {
        if let Some(b) = legacy.into_current() {
            return Some(b);
        }
    }
    if let Ok(legacy) = bincode::deserialize::<LegacyBlockV3>(data) {
        return Some(legacy.into_current());
    }
    if let Ok(legacy) = bincode::deserialize::<LegacyBlockV2>(data) {
        return Some(legacy.into_current());
    }
    if let Ok(legacy) = bincode::deserialize::<LegacyBlock>(data) {
        return Some(legacy.into_current());
    }
    None
}
```

**File: `crates/core/tests/inspect_h57682.rs`** — repurpose as a permanent regression test (already exists, currently shaped as a one-off diagnostic):

```rust
#[test]
fn h57682_deserializes_after_inc_i_119_compat_shim() {
    let data = std::fs::read("tests/fixtures/h57682.block").unwrap();
    let block = doli_core::transaction::legacy::deserialize_block_compat(&data)
        .expect("INC-I-119: pre-tombstone block must deserialize via LegacyBlockPreTombstone");
    assert_eq!(block.header.hash().to_string()[..16], "<expected hash>");
    assert!(block.transactions.iter().any(|t| t.outputs.iter()
        .any(|o| matches!(o.output_type, OutputType::EncryptedContent))),
        "block 57682 contains an EncryptedContent output by hypothesis");
}
```

(Move `/tmp/0000057682.block` → `crates/core/tests/fixtures/h57682.block` for permanent availability.)

---

## Audit follow-ups (recommended before fix lands)

1. **Scan all 3 seed archives for at-risk blocks.** Read every `*.block` file with the **current** binary's `deserialize_block_compat`, log every height that fails. Likely candidates: ranges where DeFi tx volume was high pre-May-26. Currently believed = `[57682]` based on integrity reports; needs explicit confirmation by walking the archive.
2. **Symmetric TxType audit.** Verify whether any block contains a TxType discriminant at OLD position 14+ (i.e. variants that shifted due to B.1). Same diagnostic approach — try `bincode::deserialize::<Transaction>` against each tx in each block.
3. **Lint against future tombstones.** Add a Clippy lint or CI check that flags enum-variant removals from `OutputType` / `TxType` / any other consensus-visible enum, requiring an accompanying legacy-shim entry. (Procedural rather than mechanical, since rustc can't generally prove serde position invariance.)
4. **Document in CLAUDE.md.** Add a "If You Touch" entry: *"Removing variants from any serde-encoded consensus enum requires either (a) keeping a placeholder variant at the original declaration position, or (b) adding a legacy compat shim in `legacy.rs`."*

---

## Rollout

| Step | What | Risk |
|---|---|---|
| 1 | Implement Option A on a feature branch | None — touches only `legacy.rs` + the diagnostic test |
| 2 | Add regression test for h=57682 | None |
| 3 | Add audit script that walks every seed archive and reports undeserialize-able files | None — read-only |
| 4 | `cargo test -p doli-core` (full crate) + `cargo clippy --workspace --all-targets -- -D warnings` | Standard build gate |
| 5 | Build v6.23.6 (or next patch) with Option A | New binary, deploy to one seed first |
| 6 | On the first deployed seed: re-run `bridgeFromArchive` — expect `blocks_imported >= 1` and `missing: []` on next `verifyChainIntegrity` | Reversible (insurance tar of `data/blocks/` before, undo by `cp -r` if it fails) |
| 7 | Deploy to remaining seeds + structural producers | One at a time, verify integrity after each (memory rule #4) |
| 8 | Optional: re-archive `0000057682.block` with current format so future binaries (post next tombstone) don't re-hit the issue | Single block, low risk |

**No genesis reset required.** **No activation height required.** **No protocol version bump.** This is a pure storage-layer compat shim.

---

## Related

- INC-I-088 (DeFi Phase 0 tombstoning intent — confirmed correct in policy, defective in implementation)
- INC-I-055 / INC-I-062 (archive directory wipe principles — Principle #11 in guardian skill)
- Commit `726ec8f3` — root-cause commit (B.1 lending tombstone)
- Commit `74f6e081` — symmetric defect (B.2 NFT-frac tombstone)
- Commit `97e75530` — introduced `EncryptedContent` OutputType (the variant that ended up at the broken position)
- `.claude/skills/guardian/SKILL.md` Principle #15 — "Recovery is NOT complete without fleet-wide integrity verification" (the principle that surfaced this defect on 2026-06-11)
- `crates/core/tests/inspect_h57682.rs` — diagnostic test that confirmed the root cause
