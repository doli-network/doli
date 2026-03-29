# Architecture & Security Sync Findings — 2026-03-29

## Source: Code is SOT. All drifts listed below were verified against source code.

---

## specs/architecture.md

| # | Location | Drift | Fix |
|---|----------|-------|-----|
| 1 | L352 VDF Block Production | Says "Compute VDF proof (<1ms at 1,000 iterations)" — correct for network default but step also references consensus constant 800,000 in VDF section (L370). Line 370 says "Iterations: 1,000 (network default; consensus constant T_BLOCK = 800,000)". Matches code. | No fix needed — already accurate |
| 2 | L772 storage crate lines | Says `~4,500` lines | Not re-counting lines — acceptable estimate, no fix |
| 3 | Entire file | Thoroughly synced in previous pass | No changes needed |

**Verdict**: specs/architecture.md is clean. No drift found against code.

---

## docs/architecture.md

| # | Location | Drift | Fix |
|---|----------|-------|-----|
| 1 | L264 GossipSub mesh params | Says "mainnet mesh_n=12/mesh_n_low=8/mesh_n_high=24" — CORRECT per defaults.rs. Says "testnet mesh_n=25/mesh_n_low=20/mesh_n_high=50" — CORRECT. Says "devnet mesh_n=12/mesh_n_low=8/mesh_n_high=24" — CORRECT. | No fix needed |
| 2 | L272 Testnet max_peers ref | Says "INC-I-009 Yamux buffer explosion" but code comment says INC-I-012 | Fix: change to INC-I-012 |
| 3 | L312 RPC methods count | Says "39 methods across 16 files" — dispatch.rs has exactly 39 method arms, methods/ has 16 .rs files | No fix needed |
| 4 | L127 TxType count | Says "27 transaction types" — code has 27 variants (0-28, skipping 16,23) | No fix needed (27 is correct) |

**Verdict**: 1 fix needed (incident number reference).

---

## specs/security_model.md

| # | Location | Drift | Fix |
|---|----------|-------|-----|
| 1 | Entire file | Thoroughly synced in previous pass (2026-03-29) | No changes needed |

**Verdict**: specs/security_model.md is clean. No drift found against code.

---

## docs/security_model.md

| # | Location | Drift | Fix |
|---|----------|-------|-----|
| 1 | L320-327 EquivocationProof struct | Shows only `block_hash_1` and `block_hash_2` — code uses full `BlockHeader` (both headers stored). This was fixed in specs/security_model.md (section 6.1.1) but NOT in docs/security_model.md | Fix: update struct to show full BlockHeaders |
| 2 | L4.2 Slashing "Inactivity" row | Says "50 missed slots" — code says `MAX_FAILURES = 50` / `INACTIVITY_THRESHOLD = 50` which is correct. But also says "Removal from active set" — code actually has `LIVENESS_WINDOW_MIN = 500` and re-entry mechanism, not permanent removal | Fix: clarify that inactivity exclusion is temporary with re-entry slots |

**Verdict**: 2 fixes needed.

---

## Summary

| File | Drifts Found | Fixes Applied |
|------|--------------|---------------|
| specs/architecture.md | 0 | 0 |
| docs/architecture.md | 1 | 1 |
| specs/security_model.md | 0 | 0 |
| docs/security_model.md | 2 | 2 |
| **Total** | **3** | **3** |
