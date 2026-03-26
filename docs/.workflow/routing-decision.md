# Routing Decision

## Request
"We have several modules that are getting big 1000+ lines. We need to apply modularization, max 500 lines when possible." — Workspace-wide Rust module splitting across 51 files exceeding 500 lines.

## Classification
- **Domain**: Code refactoring / Rust module decomposition
- **Complexity**: Medium (systematic but well-understood technique — no exotic domain knowledge, just disciplined module boundary analysis and mechanical splitting)
- **Task Type**: Refactoring / restructuring
- **Tier**: 1 — ROUTING UNNECESSARY (maps to existing command)

## Search Results
- **Agents scanned**: 28
- **Matches found**:
  - **NONE** — No specialist agent handles Rust modularization or large-scale code splitting
  - Core pipeline agents (architect, developer, reviewer) are fully competent for this domain
  - Non-core specialists checked: omega-topology-architect, proto-architect, proto-auditor, skill-creator, branch-strategist, curator, blockchain-network, p2p-network-engineer, stress-tester, evidence-assembler, blockchain-debug, fork-engineer, fundamentals-checker — none relevant

## Decision
- **Action**: ROUTING UNNECESSARY — use `/omega-improve`
- **Specialist**: None needed
- **Pipeline**: Standard omega-improve pipeline (Analyst -> Architect -> Test Writer -> Developer -> QA -> Reviewer)

## Justification
Modularization is a refactoring task — improving code structure without changing behavior. This is exactly what `/omega-improve` is designed for. No domain-specific expertise beyond standard Rust development is required. The architect agent already handles module boundary analysis, and the developer agent handles Rust file splitting. No memory.db routing history exists for this domain (first routing query). No specialist creation is warranted because the core pipeline agents have full competence for this work.

## Recommended Approach

Because the task spans 51 files, it should NOT be attempted as a single `/omega-improve` invocation. Instead, batch by priority:

### Priority 1 — Largest files (over 900 lines, 6 files)
```
/omega-improve --scope="sync/manager" "Split mod.rs (1227 lines) and sync_engine.rs (1087 lines) into sub-modules under 500 lines"
/omega-improve --scope="sync/reorg" "Split reorg.rs (1000 lines) into logical sub-modules"
/omega-improve --scope="wallet" "Split wallet.rs (991 lines) and rpc_client.rs (882 lines)"
/omega-improve --scope="crypto/bls" "Split bls.rs (953 lines) into sub-modules"
/omega-improve --scope="vdf/class_group" "Split class_group.rs (940 lines)"
/omega-improve --scope="mempool/pool" "Split pool.rs (926 lines)"
```

### Priority 2 — Large files (700-900 lines)
```
/omega-improve --scope="node/event_loop" "Split event_loop.rs (877 lines)"
/omega-improve --scope="transaction/core" "Split core.rs (808 lines)"
```

### Priority 3 — Remaining files (500-700 lines, ~43 files)
Batch by crate, e.g.:
```
/omega-improve --scope="crates/core" "Modularize all files over 500 lines in core crate"
/omega-improve --scope="crates/network" "Modularize all files over 500 lines in network crate"
/omega-improve --scope="crates/storage" "Modularize all files over 500 lines in storage crate"
```

### Key constraints for each batch
- Each split must preserve `pub` API (re-export from parent mod.rs)
- `cargo test` must pass after each split
- `cargo clippy -- -D warnings` must pass
- No behavioral changes — pure structural refactoring
- Commit after each successful module split

## No Specialist Creation Needed
This domain (Rust code modularization) is fully covered by the existing core pipeline. Creating a "modularization-specialist" would duplicate capabilities already in the architect and developer agents. The technique is mechanical: identify logical groupings within the file, extract to sub-modules, re-export public API, verify compilation and tests.
