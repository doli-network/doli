# Core Docs Sync — Findings & Fixes (2026-03-29 Pass 3)

**Date**: 2026-03-29
**Branch**: integrate/sync-fixes
**Method**: Read all source code files, then compared every claim in 4 doc files.
**Scope**: docs/architecture.md, docs/rewards.md, docs/protocol.md, docs/security_model.md

---

## docs/architecture.md — 1 drift item found

| # | Location | Drift | Fix Applied |
|---|----------|-------|-------------|
| A1 | Sec 3.3 doli-core module table | Lists `amm.rs \| Automated market maker logic` -- no `amm.rs` exists, actual module is `pool.rs` (AMM pool logic). Also missing several existing modules. | Renamed `amm.rs` to `pool.rs | AMM pool logic` and added missing modules: `chainspec.rs`, `config_validation.rs`, `finality.rs`, `heartbeat.rs`, `maintainer.rs`, `nft.rs`, `presence.rs`, `rewards.rs`, `scheduler.rs` |

## docs/protocol.md — 2 drift items found

| # | Location | Drift | Fix Applied |
|---|----------|-------|-------------|
| P1 | Sec 11 Peer Scoring | Invalid block = -50 and invalid tx = -10. Code: invalid block = -100, invalid tx = -20. Also "Protocol violation = -100" -- no such infraction exists. | Updated table with actual code values and all 6 infraction types |
| P2 | Sec 12 Rate Limiting | Claims blocks=100/min, txs=1000/min, sync=60/min, status=10/min. Code defaults: blocks=10/min, txs=50/sec, requests=20/sec, bandwidth=1MB/sec. | Updated table with actual code defaults |

## docs/security_model.md — 2 drift items found

| # | Location | Drift | Fix Applied |
|---|----------|-------|-------------|
| S1 | Sec 5.3 Rate Limits | Same wrong values as protocol.md (100/min blocks, 1000/min txs, 60/min sync). | Updated table with actual code defaults |
| S2 | Sec 5.4 Peer Scoring | Invalid block = -50, invalid tx = -10, timeout = -5, protocol violation = -100. Code: invalid block = -100, invalid tx = -20, timeout = -5*count (max -50), spam = -50, duplicate = -5, malformed = -30. | Updated table with actual code values |

## docs/rewards.md — 1 drift item found

| # | Location | Drift | Fix Applied |
|---|----------|-------|-------------|
| R1 | Sec 10 Key Constants | Lists `ATTESTATION_QUALIFICATION_THRESHOLD` and `ATTESTATION_MINUTES_PER_EPOCH` as constants with fixed values. In code, these are computed functions: `attestation_qualification_threshold(blocks_per_epoch)` and `attestation_minutes_per_epoch(blocks_per_epoch)`. Values vary by network. | Clarified as computed functions with network-dependent values |

---

## Total: 6 drift items found and fixed across 4 docs (pass 3)

Previous pass (pass 2) fixed 19 drift items.

---

*Completed: 2026-03-29*
