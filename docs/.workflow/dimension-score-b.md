# Dimension Scores: Group B

## Feature
Agent-consumable fork-diagnostic subsystem (workflow #346): emitter, RocksDB-CF ledger, three bundle RPCs, deterministic classifier, JSON-default CLI, historical-log replay tool.

## Scores

### D2: Impact — Score: 4
**Assessment**: High-frequency pain (7-8 fork incidents/month) with proven 2-4 hour cost per incident, reduced to single RPC call for a narrow but intensely-affected audience.
**Evidence**: MEMORY.md lists INC-I-009 through INC-I-083 (75 incidents in ~10 months). INC-I-083 required "4 parallel investigators + synthesizer + ~2h" (prompt-refinement.md:13). omega-fork/omega-swarm --deep already exist but rely on grepping 1GB log files.
**Reasoning**: Quantified impact: ~8 incidents/month x 2-4h = 16-32 hours/month of diagnostic labor. Feature reduces MTTR from 2-4h to minutes (1 RPC call + agent interpretation). However, the consumer pool is narrow (1 operator + Claude sub-agents on this single project). Indirect impact: future incidents like INC-I-082 (rebuild_epoch_state bit-identity) would surface faster through the classifier. Not a 5 because the multiplier effect is limited to this project's operational cadence -- it does not unlock new capabilities for external users or downstream systems.

### D5: Alignment — Score: 4
**Assessment**: Fits naturally into existing architectural patterns (RocksDB CFs, JSON-RPC domain modules, serde_json responses, guardian fleet queries) with one minor tension: JsonSchema as a new dependency/pattern.
**Evidence**: 
- RocksDB CF pattern established in `crates/storage/src/utxo_rocks.rs` (3 CFs: utxo, utxo_by_pubkey, unique_id)
- RPC domain-module pattern in `crates/rpc/src/methods/` (17 domain files, including `guardian.rs` which already does cross-node fleet queries via HTTP)
- `[HEALTH]` structured log line in `periodic.rs:898` -- observability-by-structured-log is precedented
- No consensus change, rolling-deploy safe, no genesis reset -- respects all Hard Constraints
- Modular design (no file >500 lines) -- matches CLAUDE.md rule
- JSON-default CLI output -- matches existing RPC response patterns (all `serde_json::json!{}`)

**Tension**: `JsonSchema` derive + published schema document (`docs/fork_observability_schema.json`) is a novel pattern -- zero existing usage found (grep returned only prompt-refinement.md). This is not a blocker but adds a new dependency (`schemars` crate) and establishes a precedent no other RPC surface follows. Not a 5 because this pattern divergence means the architect must justify why fork observability gets schema-first treatment that the other 45 RPC methods lack.

## Notes
- The existing `omega-swarm --deep` and `omega-fork` commands prove the diagnostic workflow exists but is bottlenecked on raw log access. This feature is the infrastructure those commands need to become fast.
- Counter-evidence for Impact: the operator is a single person (Antonio) and the sub-agents are hypothetical future consumers. Real-world beneficiary count = 1 human + N future Claude sessions.
- Counter-evidence for Alignment: the `getFleetForkDiagnostic(peer_rpc_urls[])` pattern (node calling other nodes' RPCs) already has precedent in `guardian.rs` (lines 441-517 show HTTP calls to peer RPCs for fleet-level data), so this is not novel.
