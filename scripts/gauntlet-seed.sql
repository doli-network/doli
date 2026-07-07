-- gauntlet-seed.sql — canonical seed for the OMEGA gauntlet scenario registry.
--
-- Source of truth for `gauntlet_scenarios` in .omega/memory.db (which is
-- gitignored, so this file is how the mapping is version-controlled and
-- reproducible). Idempotent: INSERT ... ON CONFLICT upserts.
--
-- The 8 archetypes are the canonical starter set for a networked project
-- (system-impact.md GAUNTLET §). Every Level-2+ incident whose failure mode is
-- a *system-dynamics runtime mode* is mapped onto the archetype(s) that would
-- catch its recurrence. Non-dynamics incidents are listed under "DELIBERATELY
-- UNMAPPED" — the exclusion is explicit, not silent.
--
-- HONESTY OF DESCRIPTIONS: each description states what scripts/gauntlet.sh
-- actually does, distinguishing three execution modes:
--   * ASSERTS (observational): default run checks that the failure mode's
--     symptom is ABSENT on the live network. It does NOT create the trigger.
--   * INJECTS (default): the runner actively perturbs (GS-004 launchd restart).
--   * INJECTS (--chaos): opt-in chaos mode genuinely reproduces the trigger
--     (node-down+rejoin, data-wipe+snap-recover). See scripts/gauntlet.sh.
--   * NOT AUTO-INJECTED: reproducing the trigger needs a genesis reset or
--     block-crafting tooling the harness does not have — observational only,
--     stated plainly so the label never over-claims.
--
-- Apply:  sqlite3 .omega/memory.db < scripts/gauntlet-seed.sql

BEGIN;

INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, status)
VALUES ('GS-001', 'fresh-genesis-boot',
  'Guards boot/bootstrap convergence (divergent Block-1, deadlock at h=1, disagreeing genesis). ASSERTS (observational): all live nodes share one genesisHash + one Block-1 hash and hold a single tip; no panic. NOT AUTO-INJECTED: genuine fresh-genesis reproduction wipes all nodes and needs chainspec regeneration — run manually, not in --chaos.',
  json('["INC-I-115","INC-I-084","INC-I-017","INC-I-018","INC-I-047","INC-I-048","INC-002"]'),
  'convergence,no-panic,single-block1-hash', json('{"nodes":6}'), 'active')
ON CONFLICT(scenario_id) DO UPDATE SET incident_ids=excluded.incident_ids, description=excluded.description, assertions=excluded.assertions, status='active';

INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, status)
VALUES ('GS-002', 'small-net-stall-epoch-boundary',
  'Guards epoch-boundary sync under a brief stall (empty-headers loop, spurious deep_fork/snap). ASSERTS: no new Empty-headers loop, guardian not in recovery, no new snap trigger, convergence. INJECTS (--chaos): isolates the target node (~25s launchd down) then reconnects, so the assertions judge a real stall→resync.',
  json('["INC-I-138","INC-I-012","INC-I-116","INC-I-075","INC-I-062","INC-I-059","INC-I-053","INC-I-045","INC-I-044","INC-I-046","INC-I-067","INC-I-006","INC-006","INC-I-037","INC-I-083"]'),
  'convergence,no-spurious-escalation,no-empty-headers-loop', json('{"nodes":6}'), 'active')
ON CONFLICT(scenario_id) DO UPDATE SET incident_ids=excluded.incident_ids, description=excluded.description, assertions=excluded.assertions, status='active';

INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, status)
VALUES ('GS-003', 'snap-synced-node-epoch-crossing',
  'Guards snap-synced state completeness (reward-pool/ProducerSet divergence, rejected epoch block). ASSERTS: stateRoot/csHash/psHash + utxo/producer counts agree across nodes; no rejected/invalid-epoch markers. INJECTS (--chaos): wipes the target node data (backed up), forcing a full snap/rebuild from peers — the state-root match then proves the rebuild was bit-correct.',
  json('["INC-I-118","INC-I-010","INC-I-082","INC-I-028","INC-I-029","INC-I-064","INC-I-080","INC-I-112","INC-I-054","INC-I-060"]'),
  'convergence,state-root-match,no-rejected-epoch-block', json('{"nodes":6}'), 'active')
ON CONFLICT(scenario_id) DO UPDATE SET incident_ids=excluded.incident_ids, description=excluded.description, assertions=excluded.assertions, status='active';

INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, status)
VALUES ('GS-004', 'one-block-fork-recovery',
  'Guards short-fork/restart recovery (permanent stall, ShallowRollback loop, self-fork on restart). INJECTS (default + --chaos): launchd-restarts the target producer (non-destructive) and requires it rejoins the canonical tip <60s with no self-fork and no rollback loop. This is the one perturbation the DEFAULT (gate) run performs.',
  json('["INC-I-089","INC-I-090","INC-I-081","INC-I-036","INC-I-049","INC-I-025","INC-I-024","INC-I-040","INC-I-039","INC-I-034","INC-I-026","INC-I-050","INC-I-053","INC-I-035","INC-I-030","INC-I-019","INC-I-022","INC-I-069","INC-I-103","INC-001","INC-003","INC-005","INC-004"]'),
  'convergence,recovery-under-60s,no-rollback-loop', json('{"nodes":6}'), 'active')
ON CONFLICT(scenario_id) DO UPDATE SET incident_ids=excluded.incident_ids, description=excluded.description, assertions=excluded.assertions, status='active';

INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, status)
VALUES ('GS-005', 'late-joining-node',
  'Guards a node joining/rejoining behind the tip (stuck DownloadingHeaders, incomplete store, orphan-chase storm). ASSERTS: every node reports gap=0, consistent utxoCount, no request-rate storm. INJECTS (--chaos): the target node is taken down and (in the wipe injector) cold-joins from empty data — a real late/cold join.',
  json('["INC-I-060","INC-002","INC-I-031","INC-I-032","INC-I-033","INC-I-023","INC-I-042","INC-I-043","INC-I-051","INC-I-138","INC-I-103","INC-I-008","INC-I-004","INC-I-005","INC-I-007","INC-I-001"]'),
  'convergence,block-store-complete,bounded-request-rate', json('{"nodes":6}'), 'active')
ON CONFLICT(scenario_id) DO UPDATE SET incident_ids=excluded.incident_ids, description=excluded.description, assertions=excluded.assertions, status='active';

INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, status)
VALUES ('GS-006', 'stale-block-flood',
  'Guards stale-gossip handling (unbounded re-forward, memory spike, false rollback). ASSERTS: gossipsub dedup active (already-published shedding present), rollback_depth stays 0 despite orphan noise, RSS bounded, no panic. NOT AUTO-INJECTED: genuine stale-block flooding needs block-crafting/replay tooling the harness lacks — observational only.',
  json('["INC-I-114","INC-I-137","INC-I-101","INC-I-100","INC-I-049","INC-I-008","INC-I-036","INC-I-038","INC-I-065"]'),
  'no-reforward,bounded-memory,no-panic', json('{"nodes":6}'), 'active')
ON CONFLICT(scenario_id) DO UPDATE SET incident_ids=excluded.incident_ids, description=excluded.description, assertions=excluded.assertions, status='active';

INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, status)
VALUES ('GS-007', 'rollback-rejoin',
  'Guards rollback/rebuild safety (zombie UTXOs, integrity gaps, divergent state root after rejoin). ASSERTS: guardian recovery_mode=false with an advancing healthy checkpoint; stateRoot/utxoHash agree across nodes; no integrity-gap markers. INJECTS (--chaos): the data-wipe injector forces the target to discard and rebuild its store, then asserts no zombie state survived.',
  json('["INC-I-029","INC-I-041","INC-I-082","INC-I-090","INC-I-055","INC-I-030","INC-I-112","INC-I-136","INC-I-120"]'),
  'convergence,state-root-match,integrity-complete', json('{"nodes":6}'), 'active')
ON CONFLICT(scenario_id) DO UPDATE SET incident_ids=excluded.incident_ids, description=excluded.description, assertions=excluded.assertions, status='active';

INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, status)
VALUES ('GS-008', 'scale-mismatch-smoke',
  'Guards the scale-calibration defect class (a mainnet-N threshold that self-starves the N=6 testnet, or a leak under sustained small-N run). ASSERTS (inherently at small N): production keeps advancing (liveness), no spurious escalation/eviction churn, RSS bounded, low busy/rate-limit rate. Every protection mechanism IS running at N=6 during the window — this scenario is genuinely exercised, not merely observed.',
  json('["INC-I-016","INC-I-138","INC-I-117","INC-I-102","INC-I-104","INC-I-111","INC-I-108","INC-I-070","INC-I-073","INC-I-050","INC-I-091","INC-I-137","INC-I-120","INC-I-057","INC-I-038"]'),
  'busy-rate-under-10pct,no-self-starvation,convergence', json('{"nodes":6}'), 'active')
ON CONFLICT(scenario_id) DO UPDATE SET incident_ids=excluded.incident_ids, description=excluded.description, assertions=excluded.assertions, status='active';

COMMIT;

-- ── DELIBERATELY UNMAPPED (out of system-dynamics scope) ─────────────────────
-- Level-2+ incidents that are NOT runtime system-dynamics modes reproducible on
-- a live multi-node run, so no scenario covers them (by design):
--   INC-I-086, INC-I-087  — RPC returned hardcoded/zero diagnostics (cosmetic)
--   INC-I-094, INC-I-098, INC-I-096 — DeFi/AMM/covenant validation logic
--   INC-I-088            — Phase-0 safety-gate freeze (consensus config)
--   INC-I-085            — bond-cap validation rule (static validation)
--   INC-I-078            — delegation concentration economic audit
--   INC-I-052            — creator_hash immutability auth rule
-- v_gauntlet_coverage lists these as scenarios=NULL; that is the intended,
-- documented gap — the gauntlet does not fabricate coverage for non-dynamics bugs.
