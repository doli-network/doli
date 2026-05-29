# Fundamentals Check — INC-I-096 (Design Evaluators)

## Context
Sub-agent design evaluators for INC-I-096 AMM value-conservation redesign.
Full problem space documented in `docs/.workflow/design-brief.md`.

## Fundamentals Verified
1. **Symptom clear**: 6 defects (D1-D4, H1-H2) in AMM conservation layer across 3 enforcement sites
2. **Not a config issue**: Structural architecture problem — declared pool state trusted without input binding
3. **Not a deployment issue**: AMM not yet live (amm_activation_height = u64::MAX)
4. **Root cause identified**: Per-type ad-hoc checks instead of unified conservation invariant; builder computes but validator trusts declared state
5. **Scope bounded**: AMM value-conservation layer only (4 tx types, 3 enforcement sites, 9 checks)

## Verdict
Fundamentals satisfied. Proceed with code-level design evaluation.
(Updated by restructure evaluator — prior subtraction evaluator also verified.)
