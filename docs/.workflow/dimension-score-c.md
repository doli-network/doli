# Dimension Scores: Group C

## Feature
Agent-consumable fork-diagnostic subsystem (emitter + RocksDB-CF ledger + 3 bundle RPCs + deterministic classifier + CLI + historical-log replay tool + schema export). Workflow #346.

## Scores

### D3: Complexity Cost — Score: 2
**Assessment**: Large scope — 5+ crates touched, ~3-4k LoC net new, new dependencies, and non-trivial tooling (replay parser, schema export, compaction filter).
**Evidence**:
- Crates touched: `storage` (new CF + retention), `rpc` (3 new method modules), `core` (new `fork_observability/` module with types + classifier), `network` (emit calls in recovery.rs, sync_engine), `bins/node` (emit calls in apply_block/, block_handling.rs, fork_recovery.rs, rollback.rs), `bins/cli` (new `forks` subcommand + replay tool). That is 6 crates minimum.
- New dependencies: `schemars` (JsonSchema derive — not currently used by any crate per Cargo.toml grep), `ulid` (event IDs — absent from workspace). Both require workspace-level dep additions.
- New RocksDB CF: state_db currently has 6 CFs (`open.rs:28-35`). Adding a 7th requires modifying `open.rs`, `types.rs`, adding write/query/retention logic. No existing compaction filter or TTL pattern exists in the codebase (grep returned zero hits for `compaction_filter|TTL`) — this is novel infrastructure to build.
- Estimated new files: ~12-15 (types.rs, emitter.rs, ledger.rs, classifier.rs, retention.rs in core/storage; 3 RPC modules; CLI subcommand + replay parser; tests; schema export binary/script; docs).
- Estimated LoC: ~3,000-4,000 net new (conservative: each RPC ~150, classifier ~300, emitter ~200, ledger+retention ~400, types+schema ~300, CLI+replay ~500, tests ~800-1000, docs ~500).
**Hidden costs**: Historical-log replay tool requires parsing existing unstructured log formats (1.9 GB files with varying formats across versions). Schema versioning discipline is ongoing maintenance. Fixture tests for INC-I-083 and INC-I-081 require extracting and curating test data from real logs. Compaction filter for bounded retention is novel for this codebase.
**Counter-evidence**: The codebase already has well-modularized RPC (18 method modules), well-structured storage (block_store has 9 CFs with migration patterns), and the emit points are already identified. No module exceeds 500-line limit because the design is inherently modular.

### D6: Risk — Score: 4
**Assessment**: Genuinely safe for production IF emit-call latency is verified — the feature is read-only instrumentation with no consensus path changes.
**Top 3 risks (ranked)**:
1. **Emit-call latency in apply_block hot path** — 10s slot budget means even microsecond additions matter at scale. Mitigation: emit calls must be non-blocking (channel-based or fire-and-forget write to pre-opened CF handle). RocksDB WriteBatch is already atomic in apply_block (`state_update.rs`); adding a secondary batch for observability CF is O(1) per block. Verification step: benchmark emit overhead before merge.
2. **Unbounded storage growth if retention fails** — a broken compaction filter or misconfigured TTL could grow the DB indefinitely. Mitigation: the brief mandates bounded retention (30 days OR 100k events), and a periodic-task pruner (simpler than compaction filter) could enforce the cap since periodic.rs already runs maintenance. Fallback: disable emitter via config flag if storage grows unexpectedly.
3. **Deterministic classifier mis-classification leading downstream agent to wrong fix** — if the classifier returns `tip_race_natural` when it is actually `epoch_boundary_invalid`, the agent skips the real investigation. Mitigation: `unknown` variant with evidence_event_ids exists for escalation; classifier is a pure `match` on recorded context (testable with fixtures); INC-I-083 + INC-I-081 retroactive tests validate correctness before deploy.
**Counter-evidence**: The brief explicitly states NO consensus impact, NO block content change, NO activation height. Verified by code inspection: `apply_block/mod.rs` is a clear separation between decision logic (which remains untouched) and post-commit actions (where emit calls would live alongside existing attestation tracking). Rolling deploy is safe — old nodes simply lack the new CF and RPCs, which is harmless.

### D7: Timing — Score: 4
**Assessment**: Good timing — INC-I-082 just landed, INC-I-083 just proved the pain, testnet is operational, no conflicting consensus work in flight.
**In-flight work that helps**:
- INC-I-082 (rebuild_epoch_state fix) just merged to main (`479711b5`) — stabilizes the base for adding observability without rebuild-related noise.
- INC-I-083 investigation just completed — the exact log fixtures, pain points, and failure modes are fresh and documented in `docs/.workflow/domain-investigation-*.md` files, providing immediate test data.
- Recovery coordinator (`recovery.rs`) is already modular and pure-functional (classify is side-effect-free per its own docs), making it trivial to add a one-line emit without refactoring.
**In-flight work that conflicts**:
- INC-I-083 is still `investigating` status — if it produces code fixes to recovery.rs or sync paths, those changes could conflict with emit-call insertion points. Low severity: emit calls are additive one-liners that rebase trivially.
- No pending consensus changes in the commit history that would require activation heights or protocol version bumps that might interfere.
**Counter-evidence**: The testnet was recently destabilized (5/18 nodes frozen per INC-I-083). If it remains unstable, deploying new observability on top of instability could complicate debugging the instability itself. However, the observability is precisely what would make that debugging trivial — a bootstrap problem that resolves in favor of building it now.

## Notes
- D3 scored low (2 = high cost) primarily due to the replay tool and the novel compaction/retention infrastructure, not the core emit+RPC work which is straightforward.
- D6 scored high (4 = low risk) because the architecture cleanly separates decision logic from instrumentation, and the existing modular RPC/storage patterns provide safe extension points. The one verification gate is latency benchmarking of emit calls.
- D7 scored high (4 = good timing) because the incident that proves the need JUST happened, the codebase is stable post-INC-I-082, and no conflicting work is in progress.
