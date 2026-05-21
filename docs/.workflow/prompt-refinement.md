# Prompt Refinement — INC-I-087

Original:
/omega-doctor --fix docs/.workflow/prompt-fix-diagnostic-health-counter.md
Bug location — crates/rpc/src/methods/diagnostics.rs:91-96 (three hardcoded literals).
Live counter source — WriterHeartbeat payload (crates/storage/src/diagnostic_ledger/types.rs:166-168).
Reproduce command — single curl + python diff.
Two fix approaches — shared atomics (preferred) vs latest-heartbeat lookback (stop-gap).
Wiring guidance — writer task lives in bins/node/..., introduced by M2 commits 1ffc5df8/251f5d73/259f6380.
Out-of-scope — fleet test fixture, replay path (correctly zeroed there).
Acceptance criteria — 5 concrete checks including unit tests + clippy/fmt.
Severity — Low, but fix before any monitoring consumer is wired.

Anchors detected:
- "shared atomics (preferred)" → KEEP AS HYPOTHESIS — user-suggested approach but mark as preferred-not-mandated; verify the WriterHeartbeat already exposes the live counter before assuming atomics are required.
- "three hardcoded literals" → PRESERVE — concrete code observation, not interpretive bias.

Domain context preserved:
- [code] crates/rpc/src/methods/diagnostics.rs:91-96 — handler returns hardcoded values
- [code] crates/storage/src/diagnostic_ledger/types.rs:166-168 — WriterHeartbeat carries the real counter
- [code] bins/node/... — writer task that emits the heartbeat (M2: 1ffc5df8, 251f5d73, 259f6380)
- ⚠️ CONSTRAINT: Do not modify the replay path (already zeroed correctly)
- ⚠️ CONSTRAINT: Fleet test fixture is out of scope

Refined:
The RPC handler `getDiagnosticHealth` in crates/rpc/src/methods/diagnostics.rs:91-96 returns three hardcoded literal values instead of the live counters carried in WriterHeartbeat (crates/storage/src/diagnostic_ledger/types.rs:166-168). Find the canonical, lowest-coupling way to thread the live counter values into the RPC handler. The user proposes shared atomics as the preferred mechanism and latest-heartbeat lookback as a stop-gap — evaluate both but pick whichever fits the existing wiring with minimum churn. Write a FAILING unit test that pins the current hardcoded behavior (so the FAIL→PASS transition proves the fix), implement the fix, and verify clippy/fmt pass. Out of scope: fleet test fixture, replay path. Severity: Low.
