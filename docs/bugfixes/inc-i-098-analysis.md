# INC-I-098 Analysis — Vault covenant rejected as malformed HTLC (ERRTX-HTLC001)

## Symptom
`doli template vault --send` builds condition `Or(And(Sig(owner), Timelock(unlock_h)), Multisig(2,[owner,cosigner]))` and the node rejects with:
`[ERRTX-HTLC001] HTLC output 0 has unsigned refund branch (requires Signature after h=272)`.

## Architecture Context
Data flow for a covenant send:
1. **CLI** (`bins/cli/src/cmd_template/vault.rs`) builds the `Condition` via `doli_core::conditions::templates::vault()`.
2. **CLI** picks the output's `OutputType` via `condition_to_output_type()` (`bins/cli/src/parsers.rs:315`).
3. Output is serialized into the tx (output_type is a tx field) and broadcast.
4. **Node validation** (`crates/core/src/validation/transaction.rs:413-466`) validates the output *by its declared `output_type`*. The covenant types `Multisig | Hashlock | HTLC | Vesting` share one path: decode the condition, gate guards by activation height. Only `OutputType::HTLC` gets the extra check at line 448:
   ```rust
   if output.output_type == OutputType::HTLC
       && ctx.current_height >= ctx.security_audit_activation_height
       && !cond.has_signed_refund()  // AUDIT-BRIDGE-001 anti-front-running
   { return Err(ERRTX-HTLC001) }
   ```
5. `has_signed_refund()` (`crates/core/src/conditions/mod.rs:370`) returns true only if the **right** branch of the top-level `Or` contains a `Signature` node.

## Root Cause (CONFIRMED — contradicts reported hypothesis)
The reported hypothesis ("validation HTLC *detection* is too permissive / shape-based") is **false**. Node validation does NOT detect HTLCs by shape — it keys off the declared `output.output_type` tag (line 448). Validation is correct.

The real defect is upstream, in the **CLI** type-inference at `bins/cli/src/parsers.rs:319-322`:
```rust
doli_core::Condition::Or(_, _) => {
    // HTLC is Or(And(Hashlock, Timelock), TimelockExpiry)
    doli_core::OutputType::HTLC      // <-- maps EVERY Or to HTLC
}
```
Every `Or(_, _)` is blindly tagged `HTLC`, even though the comment itself says a true HTLC must contain a `Hashlock`. The vault is an `Or` with **no Hashlock**, so it is mis-tagged `HTLC`, then the (correct) HTLC rule fires. The vault's signature sits in the **left** branch (`And(Sig, Timelock)`); its **right** branch is `Multisig`, which `has_signed_refund` does not count as signed → ERRTX-HTLC001.

`condition_to_output_type` is `pub(crate)` in `bins/cli` and is used ONLY for client-side tx construction. It is not referenced by `crates/core` or `bins/node`. Verified: grep finds it only in `bins/cli/src/parsers.rs` and `bins/cli/src/parsers_tests.rs`.

The existing unit test `output_type_mapping_or` (`parsers_tests.rs:613-620`) asserts a generic `Or(Timelock, TimelockExpiry)` → HTLC — i.e., it codifies the buggy mapping.

## Breadth (which templates are affected)
All top-level-`Or` templates are mis-tagged `HTLC`. Impact differs by whether the right branch happens to carry a `Signature`:

| Template | Shape | Right branch | `has_signed_refund` | Outcome today |
|----------|-------|--------------|---------------------|---------------|
| **vault** | `Or(And(Sig,Timelock), Multisig)` | Multisig | **false** | **BROKEN — ERRTX-HTLC001** |
| escrow | `Or(Multisig, And(Sig,Expiry))` | And(Sig,Expiry) | true | mis-tagged HTLC, passes by luck |
| escrow_loan | `Or(And(guards), And(Sig,deadline))` | And(Sig,…) | true | mis-tagged HTLC, passes by luck (INC-I-099 is a separate path) |
| htlc_payment | `Or(And(Hashlock,Timelock), And(Sig,Expiry))` | And(Sig,Expiry) | true | correctly HTLC |
| subscription / agent_allowance | `And(...)` | — | — | tagged Vesting, unaffected |

Only **vault** is functionally broken; escrow/escrow_loan are silently mis-typed (displayed as "htlc", subjected to HTLC rules they happen to satisfy).

## Fix (SSF — single change)
In `condition_to_output_type`, map an `Or` to `HTLC` **only when the condition tree contains a `Hashlock`** (the structural signature of a real HTLC). Otherwise map to `Multisig` — the existing generic-covenant fallback already used for guards (see the function's own doc-comment). This:
- Fixes vault: `Or` without Hashlock → `Multisig` → validation decode path passes (no HTLC check).
- Re-tags escrow / escrow_loan as `Multisig` (correct — they are not HTLCs).
- Keeps htlc_payment as `HTLC` (it contains a Hashlock).

A private recursive `condition_contains_hashlock()` helper in `parsers.rs` is the minimal vehicle.

## Consensus / Deploy Assessment (three-question checklist)
The change is to **CLI transaction construction**, NOT to any node validation rule. `crates/core` validation is untouched.
1. *User-submittable tx triggers this path?* — It builds the tx; the node validation path is unchanged.
2. *Producer/attestation triggers it?* — No.
3. *Bit-identical for all reachable inputs?* — No, for the **CLI**: new vault/escrow/escrow_loan txs carry a different `output_type` byte. But this is client-side construction; nodes validate each tx by its self-declared type with **unchanged** rules.

**Verdict: NO activation height required.** We are not relaxing or changing a consensus validation rule — we are fixing the client to emit a correctly-typed output that the existing, unchanged validation already accepts. No synchronized deploy needed (not block-content for producers). Action: ship updated `doli` CLI; users rebuild vault txs with the corrected tag. Existing on-chain outputs are unaffected.

(Optional, out of scope — not implemented: core could additionally require an `HTLC`-tagged output to actually contain a Hashlock. That is hardening, not the root cause; deferred per SSF.)

━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.9, code-traced root cause in a single CLI function; pending FAIL→PASS test)
Reasoning: Deterministic, reproducible, localized to one CLI function (`condition_to_output_type`). Validation confirmed correct. No cross-module interaction; no activation height.
━━━━━━━━━━━━━━━━━━━━━━

## Requirements
- **REQ-I098-001 (Must)**: `condition_to_output_type(vault(...))` returns a non-HTLC covenant type (`Multisig`). AC: vault output validates (no ERRTX-HTLC001) at height ≥ security_audit_activation_height.
- **REQ-I098-002 (Must)**: `condition_to_output_type(htlc_payment(...))` still returns `HTLC`. AC: true HTLCs remain subject to AUDIT-BRIDGE-001 signed-refund enforcement.
- **REQ-I098-003 (Should)**: escrow and escrow_loan map to `Multisig` (no longer mis-tagged HTLC).
