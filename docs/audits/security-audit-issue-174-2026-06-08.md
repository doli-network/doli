# Security Audit — Verification of GitHub Issue #174

**Issue:** https://github.com/doli-network/doli/issues/174
**Researcher:** maulana / khasbimln@gmail.com
**Target version (researcher):** v6.23.3
**Verified against:** `main` @ ec121f87 (v6.23.5)
**Date:** 2026-06-08
**Auditors:** 5 parallel independent (auth, crypto, logic, config, infra-SPOF)

━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (measured)
  Memory:   0 (measured)
  IO:       0 (measured)
  Network:  0 (measured)
  Disk:     0 (measured)
  Latency:  0 (measured)
Inevitability: AVOIDABLE
Cheaper alternative: NONE-NEEDED
Why this proposal anyway: Read-only audit report — no runtime impact. Per-fix cost blocks must accompany each remediation when implemented separately.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

---

## Bottom Line

**8 of 10 claims have substance.** Two are net-new vulnerabilities the researcher missed (SSRF, key reuse). Two of the researcher's "high"-severity claims are functionally dead code (P3). The most exploitable findings are **P1, not P0** as the report framing suggests — and exploitability of the headline claim (#1) depends on Nginx config we cannot verify from the repo.

**Recommend:** acknowledge the researcher, credit findings, prioritize the 5 real P1s (admin RPC trust check, SSRF, install.sh integrity, sudo TOCTOU, maintainer key reuse), defer dead-code/style findings.

---

## Verdict Matrix

| # | Researcher claim | Severity (researcher) | Verdict | Severity (verified) | Evidence |
|---|------------------|----------------------|---------|---------------------|----------|
| 1 | Admin RPC bypass via Nginx | Critical | **PARTIAL** | **P1** (was P0 if Nginx config matches) | `crates/rpc/src/server.rs:234-239` |
| 2 | `overflow-checks` missing | Critical | **CONFIRMED-but-MIS-SEVERITY** | **P2** defense-in-depth | `Cargo.toml:117-120` |
| 3 | `install.sh` no integrity check | Critical | **CONFIRMED** | **P1** | `scripts/install.sh:54-93` |
| 4 | `.unwrap()` in critical paths | High | **REFUTED** for named lines | **P3** style | `block_handling.rs:747`, `fork_recovery.rs:47`, `periodic.rs:109` |
| 5 | VDF `T_REGISTER_CAP` 5000× mismatch | High | **CONFIRMED-but-DEAD-CODE** | **P3** | `crates/vdf/src/lib.rs:61` vs `crates/core/src/consensus/vdf.rs:81` |
| 6 | BLS `from_bytes_unchecked` | High | **REFUTED** (zero callers) | **P3** API hygiene | `crates/crypto/src/bls.rs:105,339` |
| 7 | Sudo `/tmp` privilege escalation | High | **CONFIRMED** | **P1** local privesc | `scripts/install.sh:162-168`, `crates/updater/src/apply.rs:189` |
| 8 | Infrastructure SPOF (single IP) | Medium | **UNVERIFIABLE-IN-CODE** | N/A operational | DNS — not in repo |
| 9 | Maintainer threshold inconsistency | Medium | **PARTIAL** (inverted impact) | **P2** governance liveness | `crates/rpc/src/methods/governance.rs:184` |
| 10 | BSC HTLC at zero address | Medium | **UNVERIFIABLE-IN-CODE** | N/A (frontend issue) | only `BRIDGE_CHAIN_BSC` constant exists |

### Net-new findings (researcher missed)

| ID | Finding | Severity | Evidence |
|----|---------|----------|----------|
| NEW-1 | `repairArchiveFromPeer` missing from `ADMIN_METHODS` → unauthenticated SSRF | **P1** | `crates/rpc/src/server.rs:31-46` vs `dispatch.rs:77`, `guardian.rs:411-630` |
| NEW-2 | `getFleetForkDiagnostic` not in admin list, amplified SSRF risk | **P1** | `crates/rpc/src/methods/diagnostics_fleet.rs:65-178` |
| NEW-3 | Maintainer Ed25519 keys IDENTICAL on mainnet and testnet | **P2** | `crates/updater/src/constants.rs:37-67` — testnet key compromise = mainnet release signing |
| NEW-4 | `install.sh` uses `doli-network/doli` repo; updater uses `e-weil/doli` — different trust roots | **P3** | `install.sh:4` vs `constants.rs:120` |

---

## Detailed Verification

### Claim #1 — Admin RPC bypass (Critical → P1 PARTIAL)

**Architectural vulnerability is real.** `is_trusted_network(client_ip)` at `crates/rpc/src/server.rs:234-239` checks the TCP peer address only. The codebase contains **zero** references to `X-Forwarded-For` or `X-Real-IP`. Behind any reverse proxy (Nginx, Cloudflare, AWS ALB), every request appears as `127.0.0.1` and bypasses the admin auth gate.

**But the researcher overcounted the methods.** `ADMIN_METHODS` (server.rs:31-46) contains **12 methods, not 16**. `getProducers` is a public method (not admin) — the researcher's PoC works because the public method intentionally exposes producer info; the "BLS keys leak" is by design (validators are public actors).

Confirmed admin methods callable via the bypass:
- `pauseProduction` / `resumeProduction` — **CONFIRMED** halt + resume mainnet
- `createCheckpoint` — **CONFIRMED** (also leaks server path)
- `verifyChainIntegrity` — **CONFIRMED** but expensive scan, admin-gated
- `getStateSnapshot` — **CONFIRMED** 9.2MB+ admin-gated

**Severity:** P1, not P0. Exploitability depends entirely on Nginx config we cannot verify from the repo. If Nginx is configured to set the real client IP and `is_trusted_network` is patched to read it, the bypass closes immediately.

**Fix:** Read `X-Forwarded-For` / `X-Real-IP` from request headers in `is_trusted_network`, AND ship a reference Nginx config in the repo, AND default to deny when headers are absent in production mode.

### Claim #2 — `overflow-checks` missing (Critical → P2)

**Confirmed**: `Cargo.toml:117-120` lacks `overflow-checks = true`. Combined with `panic = "abort"`, an overflow would silently wrap.

**But the two named exploits don't reach.** `Slot` is `u32` at `params.rs:200`, so `slot as u64 * 10` peaks at ~43 billion (safe). `reward_epoch.rs:62` requires epoch ≈ 5×10¹⁶ to overflow, which is ~5.85 trillion years of chain operation.

The systemic concern is real (P2 defense-in-depth), but the researcher's claimed exploit paths are not exploitable. The high-value arithmetic (fees, amounts) already has explicit bounds checks.

**Fix:** Add `overflow-checks = true` to `[profile.release]`. Cheap, defensible, no behavior change for any reachable code path.

### Claim #3 — `install.sh` no integrity check (Critical → P1)

**Confirmed**. `scripts/install.sh:54-93` downloads tarball from GitHub Releases over HTTPS and runs `tar -xzf ... && sudo install` with zero checksum or signature verification. The auto-updater has full Ed25519 + SHA-256 verification (`crates/updater/src/verification.rs`) — that infrastructure exists but is unused by the bootstrap installer.

Bootstrap is the highest-risk moment: there is no prior trusted binary to verify against. A compromised GitHub release artifact gets root-installed.

**Fix:** Verify SHA-256 against shipped CHECKSUMS.txt and Ed25519 signature against shipped SIGNATURES.json before `sudo install`. Implementation is straightforward in shell (`sha256sum -c`).

### Claim #4 — `.unwrap()` in critical paths (High → P3 style)

**All three named locations are REFUTED for "crafted message crashes node":**

- `block_handling.rs:747,825` — unwraps are inside an `if has_undo` guard (line 722-723) that pre-validates ALL heights have undo data. `&mut self` event loop prevents TOCTOU.
- `fork_recovery.rs:47,120,131,142` — `recovery.blocks` is initialized with `vec![orphan_block]` and only ever appended to. Cannot be empty.
- `periodic.rs:109,122` — `panic = "abort"` makes mutex poisoning structurally impossible (any prior panic aborts the process).

The ~482 unwrap count is also wrong: actual count is ~1,445, but most are on infallible operations (RocksDB CF handles after init, pre-checked Options).

**Severity:** P3 style preference. None of the named lines is a real DoS vector.

### Claim #5 — VDF constant 5000× discrepancy (High → P3 dead code)

**Both constants exist as researcher claimed**, but **neither is used by any production code path.** Workspace grep shows zero callers. Registration validation uses `network.vdf_register_iterations()` from `NetworkParams`, defaulting to 1,000 for all networks.

Confirms MEMORY.md `feedback_no_vdf.md`: "DOLI does NOT use VDF in production". The constants are dead code with a stale doc comment (`crates/vdf/src/lib.rs:22` says 5M, line 61 says 1,000).

**Fix:** Delete both `T_REGISTER_CAP` constants. Update the doc table.

### Claim #6 — BLS `from_bytes_unchecked()` (High → P3 API hygiene)

**Zero external callers.** Workspace grep for `from_bytes_unchecked` returns only the two definitions. All untrusted BLS data (gossip, blocks, RPC, registration) flows through validated paths:
- Serde deserialization → `try_from_slice` (curve-validated) or `from_hex`
- Verification (`bls_verify`, `bls_verify_aggregate`, `bls_verify_pop`) → `to_blst()` which re-validates via `BlstPublicKey::from_bytes()`

The `pub` visibility is a defense-in-depth concern. The researcher's claim ("invalid curve points could pass signature verification") is incorrect for this codebase.

**Fix:** Restrict to `pub(crate)` or document the safety contract.

### Claim #7 — Sudo `/tmp` privilege escalation (High → P1)

**CONFIRMED end-to-end.** The exact sudoers rule the researcher quoted is installed by THREE places in the repo:
- `scripts/install.sh:162-168`
- `bins/node/postinst.sh:39-46` (deb/rpm post-install)

The Rust updater at `crates/updater/src/apply.rs:189` writes the binary to `std::env::temp_dir().join("doli-update-binary")` → `/tmp/doli-update-binary` on standard Linux. No `O_EXCL`, no `O_NOFOLLOW`, no `mktemp`, no `tempfile` crate.

**Attack:** Any local user (or process running as the `doli` user) wins the TOCTOU race between line 190 (`fs::write`) and line 207 (`sudo cp`). Replace `/tmp/doli-update-binary` with malicious binary → root-level code execution on next update.

The sudoers rule also allows `sudo rm -f /usr/bin/doli-node` — instant DoS by the `doli` user.

**Fix:** Move temp path to `/var/lib/doli/update.bin` (owned `doli:doli`, mode 0600), update sudoers source path accordingly, create file with `O_EXCL | O_NOFOLLOW`, re-verify SHA-256 after `sudo cp` succeeds.

### Claim #8 — Infrastructure SPOF (Medium → UNVERIFIABLE)

DNS resolution and web infra topology are not in the repo. Cannot confirm `187.124.95.188` or 69% bond concentration from code alone. The 5-server structural fleet (ai1-ai5 + N1-N12) is documented in MEMORY.md as the operational reality.

**Note:** This is a real operational concern but lives outside the code SoT. It's a deployment / decentralization-roadmap discussion, not a software bug.

### Claim #9 — Maintainer threshold inconsistency (Medium → P2, inverted impact)

**The numerical claim is correct: `calculate_threshold(3) = 2` while `MAINTAINER_THRESHOLD = 3`.** But the impact is opposite to what the researcher implied.

`MAINTAINER_THRESHOLD = 3` is hardcoded in the RPC pre-check (`crates/rpc/src/methods/governance.rs:184`). Consensus enforcement (`maintainer.rs:158`'s `verify_multisig`) uses the dynamic `self.threshold = calculate_threshold(N)`. With a degraded 3-member set:
- Consensus accepts 2-of-3 ✓
- RPC pre-check rejects 2-of-3 ✗

This is **too strict** at the RPC layer, not too permissive. It's a governance liveness bug in degraded sets, not a security bypass.

**Real issue (not in researcher's report):** `force_remove_maintainer` (slashing path) bypasses `MIN_MAINTAINERS`. If enough maintainers are slashed to 0 members, `calculate_threshold(0) = 0` and `verify_multisig` passes with ZERO valid signatures. Worth its own follow-up — but only reachable through the slashing flow.

**Fix:** Replace hardcoded `MAINTAINER_THRESHOLD` in `governance.rs:184` with `MaintainerSet::calculate_threshold(current_member_count)`. Add `MIN_MAINTAINERS` guard to `force_remove_maintainer`.

### Claim #10 — BSC HTLC at zero address (Medium → UNVERIFIABLE in node code)

The node codebase contains BSC only as the chain ID constant `BRIDGE_CHAIN_BSC = 6` (`crates/core/src/transaction/output.rs:75`) and a display name. **Zero contract addresses, zero RPC endpoints, zero client code for BSC.** The bridge crate has `bitcoin.rs` and `ethereum.rs` clients only.

If a frontend marketplace references zero-address BSC contracts, that's a frontend repository issue (not in `doli-network/doli`).

---

## Net-New Findings

### NEW-1: `repairArchiveFromPeer` unauthenticated SSRF (P1)

`crates/rpc/src/server.rs:31-46` defines `ADMIN_METHODS`. `repairArchiveFromPeer` is **NOT** in the list, yet it is routed at `dispatch.rs:77`. The handler (`crates/rpc/src/methods/guardian.rs:411-630`) takes a user-supplied `rpc_url`, makes outbound HTTP POST in a loop (1 request per block height up to `peer_tip`), and writes the responses to the archive directory.

**Attack:** Any unauthenticated caller can use any DOLI node as an SSRF proxy — direct it at internal services, generate thousands of requests, write attacker-controlled bytes to the archive dir.

Compare with `backfillFromPeer`, which IS in `ADMIN_METHODS` and calls `validate_backfill_url`. The asymmetry is a clear miss.

**Fix:** Add `"repairArchiveFromPeer"` to `ADMIN_METHODS` and call `validate_backfill_url`. Audit `getFleetForkDiagnostic` (same pattern, NEW-2).

### NEW-3: Maintainer keys identical mainnet ↔ testnet (P2)

`crates/updater/src/constants.rs:37-67` defines the 5 maintainer Ed25519 public keys for the auto-update signature quorum. The mainnet array and the testnet array contain **the same 5 keys**.

**Implication:** A testnet maintainer key compromise (lower-security operational practices typical for testnets) is equivalent to a mainnet maintainer key compromise. A 3-of-5 signed testnet release would pass mainnet verification.

**Fix:** Separate key sets per environment, or document explicitly why reuse is acceptable.

---

## Summary Counts

- **P0 (Critical):** 0 (researcher framing implies 3; none verified at P0)
- **P1 (High):** 5 — admin trust check (#1), install.sh integrity (#3), sudo TOCTOU (#7), SSRF (NEW-1), fleet SSRF (NEW-2)
- **P2 (Medium):** 4 — overflow-checks (#2), threshold liveness (#9), key reuse (NEW-3), `getStateSnapshot` unbounded
- **P3 (Low):** 5 — unwraps (#4), VDF dead code (#5), BLS API hygiene (#6), path leak in createCheckpoint, repo trust-root inconsistency (NEW-4)
- **Unverifiable in code:** 2 — #8 infra SPOF (DNS), #10 BSC HTLC (frontend)

## Recommended Response to Researcher

1. **Acknowledge and credit.** The researcher correctly identified 5 real P1 findings. The framing was over-severe in places (P0→P1, dead-code→style), but the bones are good.
2. **Prioritize 5 P1 fixes:** admin trust check, install.sh integrity, sudo TOCTOU, both SSRF endpoints (the latter two are net-new and were not in the report).
3. **Defer dead code / style findings** to a hygiene sweep.
4. **Out of scope here:** infra SPOF and BSC HTLC are real concerns but live outside this codebase.

## Files Generated

- `docs/.workflow/security-audit-brief.md` — initial scope
- `docs/.workflow/security-audit-auth.md` — RPC auth verification (Claim #1, #7-auth)
- `docs/.workflow/security-audit-logic.md` — overflow + unwrap verification (Claim #2, #4)
- `docs/audits/security-audit-issue-174-2026-06-08.md` — this report

Crypto (#5, #6, #9) and config (#3, #7, #10) auditors returned their full reports inline; their findings are embedded in this synthesis with exact file:line evidence.
