# Cargo Audit — Deferred Workspace Hygiene

**Source:** AUDIT-P3 dep advisories from `docs/audits/security-audit-oracle-2026-05-29.md`
**Status:** DEFERRED — out of scope for the oracle audit fix loop.

## Why deferred

Every flagged advisory is transitive via `libp2p 0.53` (workspace-pinned in `Cargo.toml:60`). The audit explicitly notes "none oracle-specific". Upgrading to `libp2p 0.55+` requires API migration across `crates/network/` (gossipsub, kad, request-response, swarm), which is a multi-hour task with its own integration-test burden — not appropriate to bundle with surgical oracle fixes.

## Advisories (snapshot 2026-05-30)

| Advisory ID | Crate@Version | Severity | Note |
|---|---|---|---|
| RUSTSEC-2026-0119 | hickory-proto 0.24.4 | High | CPU exhaustion via O(n²) name compression — only reachable on outbound DNS encoding |
| RUSTSEC-2024-0437 | protobuf 2.28.0 | Medium | Uncontrolled recursion crash — only reachable if untrusted protobuf decoded |
| RUSTSEC-2025-0009 | ring 0.16.20 | Low | AES panic with overflow checks enabled |
| RUSTSEC-2026-0104 | rustls-webpki 0.101.7 | Medium | CRL parsing panic |
| RUSTSEC-2026-0098 | rustls-webpki 0.101.7 | Medium | URI name constraint bypass |
| RUSTSEC-2026-0099 | rustls-webpki 0.101.7 | Medium | Wildcard cert name constraints accepted |

Plus GTK3 `atk`/`atk-sys` unmaintained warnings (GUI crate, irrelevant for the consensus node).

## Recommended follow-up workflow

Run as a separate session under `/omega-improve --scope=crates/network` or equivalent:

1. Bump `libp2p` in workspace `Cargo.toml` to the latest 0.55.x line.
2. Migrate API changes in `crates/network/`:
   - `gossipsub::Behaviour` config / handler signature changes
   - `kad::Behaviour` event types
   - `request_response` codec trait signature changes
   - `swarm::Behaviour` derive macro changes
3. Re-run `cargo audit` to confirm advisories cleared.
4. Run integration tests + testnet smoke before merging.

## Why not in this batch

Per `feedback_measure_before_proposing` and SSF: surgical oracle fixes share no code with libp2p infrastructure, and bundling a major-version dep bump with consensus-adjacent edits inflates blast radius and review burden for both changes.
