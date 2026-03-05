# Stress Test Progress - 2026-03-04

## Status: COMPLETE

All 8 categories executed. Final report saved to:
`/Users/ilozada/repos/doli/docs/stress-tests/stress-report-2026-03-04.md`

## Summary

- **Tests executed**: 119
- **PASS**: 97
- **FAIL**: 16
- **CRASH**: 0
- **Overall verdict**: DEGRADED

## Findings (7 total)

| # | Severity | Finding |
|---|----------|---------|
| 1 | HIGH | CLI hangs indefinitely on unreachable RPC (no timeout) |
| 2 | HIGH | Systematic exit code 0 on error conditions (16+ commands) |
| 3 | MEDIUM | RPC returns plain-text errors instead of JSON-RPC for malformed requests |
| 4 | MEDIUM | RPC accepts 1MB+ request bodies (no size limit) |
| 5 | MEDIUM | Zero-amount send not validated before UTXO lookup |
| 6 | LOW | Scientific notation / decimal amounts silently accepted by send |
| 7 | LOW | RPC parameter naming inconsistency (camelCase vs snake_case) |

## Categories Completed

1. H: Edge Cases & Input Validation (14 tests)
2. A: Wallet Operations (12 tests)
3. E: Signing & Verification (12 tests)
4. F: Governance & Update (8 tests)
5. B: Transaction & Balance (20 tests)
6. C: Producer Lifecycle (18 tests)
7. D: Rewards (8 tests)
8. G: RPC Endpoint Testing (27 tests)
