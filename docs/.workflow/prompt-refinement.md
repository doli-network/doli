# Prompt Refinement — INC-I-104 RocksDB Redesign

Workflow: `/omega-redesign --incident=INC-I-104` (proposal-only)
Date: 2026-06-01

## Original:

> I need a first-principles design of the correct RocksDB configuration for the 4 RocksDB instances in doli-node: block_store, state_db, utxo_store, diagnostic_ledger. The design should reflect what these databases architecturally NEED based on their workload and durability requirements — not what fits a specific VPS. If the resulting per-node memory footprint doesn't fit a given server, that's an operational decision (move nodes off that server, or upgrade it). The architecture must not be reverse-engineered from the smallest box in the fleet.
>
> Read first
>
> - docs/.workflow/diagnosis-report.md — INC-I-104 root cause: 3 of 4 RocksDB instances are uncapped (db_write_buffer_size = 0, per-CF write_buffer_size = 64 MB, max_write_buffer_number = 2). Only diagnostic_ledger was capped by commit f37febcf.
> - crates/storage/src/block_store/open.rs, crates/storage/src/state_db/open.rs, crates/storage/src/utxo_store/open.rs, crates/storage/src/diagnostic_ledger/mod.rs — current open() implementations and column-family lists.
> - .claude/skills/storage/SKILL.md — DOLI persistence layer overview.
> - CLAUDE.md "If You Touch" → storage section + "Mental Model" 3-states description.
>
> Per-DB workload to design against (architectural — not hardware)
>
> For each of the 4 instances, derive the right config from:
>
> 1. Workload profile — sustained write rate, read-vs-write ratio, key/value size distribution, hot-vs-cold CF behavior, compaction sensitivity.
> 2. Durability requirement — what data loss is acceptable on crash? block_store and state_db are consensus-critical (cannot lose recent writes — affects state root). utxo_store is rebuildable from blocks. diagnostic_ledger is observability — lossy is acceptable.
> 3. Latency requirement — what does the read path block on? state_db reads are on the apply_block hot path (per-slot latency budget). block_store reads serve sync requests (latency tolerant). utxo_store reads are on validation hot path.
> 4. Working set size — bounded by chain height? unbounded with TTL pruning?
> 5. Crash-recovery profile — how much WAL is acceptable to replay on restart?
>
> For each instance produce concrete values for:
> [list of ~15 RocksDB parameters per instance]
>
> Constraints:
> - Do not include "what fits server X" framing anywhere in the spec.
> - Do not anchor on INC-I-102's 8 MB value as the answer for the other 3 DBs.
> - Use WebSearch / WebFetch; cite sources.
> - If a setting genuinely should differ per CF, say so and tune per CF.
>
> One spec, one set of values, one architectural commitment. The fleet adapts to the spec; the spec does not adapt to the fleet.

## Anchors detected:

**NONE.** The user has pre-emptively de-anchored the request:

- **Hardware anchor** — explicitly forbidden ("don't reverse-engineer from smallest box", "what fits server X")
- **Prior-value anchor** — explicitly forbidden ("do not anchor on INC-I-102's 8 MB value")
- **Layer anchor** — none (treats memory, latency, durability, recovery as parallel design dimensions)
- **Cause anchor** — none (frames task as first-principles derivation, not "fix INC-I-104")
- **Solution anchor** — none (asks for concrete values but does not pre-commit to any)

The prompt itself is correctly framed as workload-driven and asks for per-CF differentiation as a likely outcome.

## Domain context preserved:

- **Incident**: INC-I-104 (ai5 n9-n12 RAM growth ~8 MB/min; sync coordinator sees 1 peer while transport reports 45)
- **Scope**: 4 RocksDB instances in doli-node (block_store, state_db, utxo_store, diagnostic_ledger)
- **Current state**: 3 of 4 uncapped (db_write_buffer_size=0, per-CF write_buffer_size=64 MB, max_write_buffer_number=2); only diagnostic_ledger capped at 8 MB (commit f37febcf)
- **Architectural attributes per instance**: durability tier (consensus-critical / rebuildable / observability), latency budget (validation hot path / sync / RPC / debug), workload (hot vs cold CFs), crash-recovery (WAL replay vs network re-sync vs self-heal)
- **Hard constraints** (verbatim from user, preserved):
  - ⚠️ No hardware framing
  - ⚠️ No anchoring on INC-I-102 8 MB
  - ⚠️ Per-CF differentiation expected where workload differs
  - ⚠️ WebSearch/WebFetch permitted; cite sources
  - ⚠️ One spec, one set of values

## Refined:

Use the original prompt as-is. It already meets the refinement bar — no anchoring removal needed. The redesign directive (subtraction default, workload-first, per-CF allowed) is implicit in the user's framing and made explicit in the design brief.

Refined output = same as Original, with redesign directive appended downstream in `docs/.workflow/design-brief.md`.
