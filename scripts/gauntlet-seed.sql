-- gauntlet-seed.sql — canonical seed for the OMEGA gauntlet scenario registry.
--
-- Source of truth for `gauntlet_scenarios` in .omega/memory.db (which is
-- gitignored, so this file is how the mapping is version-controlled and
-- reproducible). Idempotent: INSERT ... ON CONFLICT upserts, so running it on
-- a fresh DB creates the 8 canonical archetypes and on an existing DB refreshes
-- their incident coverage.
--
-- The 8 archetypes are the canonical starter set for a networked project
-- (system-impact.md GAUNTLET §). Every Level-2+ incident whose failure mode is
-- a *system-dynamics runtime mode* (reproducible/observable on the live
-- multi-node testnet) is mapped onto exactly the archetype(s) that would catch
-- its recurrence. Incidents that are NOT system-dynamics modes are listed under
-- "DELIBERATELY UNMAPPED" at the bottom — the exclusion is explicit, not silent.
--
-- Apply:  sqlite3 .omega/memory.db < scripts/gauntlet-seed.sql
-- Verify: sqlite3 -header -column .omega/memory.db \
--           "SELECT scenario_id,name,json_array_length(incident_ids) AS n FROM gauntlet_scenarios ORDER BY scenario_id;"

BEGIN;

-- GS-001 — fresh-genesis boot: all nodes converge to one Block-1 lineage, no
-- deadlock at h=1, no divergent genesis. (boot / bootstrap convergence)
INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, status)
VALUES ('GS-001', 'fresh-genesis-boot',
  'Guards the boot/bootstrap convergence mode. Failure recurrence shows as divergent Block-1 hashes, a deadlock at h=1, or disagreeing genesis. Runner check (observational): all live nodes share one genesisHash and one Block-1 hash, and the network holds a single tip.',
  json('["INC-I-115","INC-I-084","INC-I-017","INC-I-018","INC-I-047","INC-I-048","INC-002"]'),
  'convergence,no-panic,single-block1-hash', json('{"nodes":6}'), 'active')
ON CONFLICT(scenario_id) DO UPDATE SET incident_ids=excluded.incident_ids, description=excluded.description, assertions=excluded.assertions, status='active';

-- GS-002 — small-net stall + epoch-boundary sync: a node that stalls near an
-- epoch boundary reconnects without an empty-headers loop or spurious escalation.
INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, status)
VALUES ('GS-002', 'small-net-stall-epoch-boundary',
  'Guards epoch-boundary sync under a brief stall. Recurrence shows as an empty-headers/0-headers loop, spurious deep_fork_confirmed, or spurious snap-sync at a boundary. Runner check: no new Empty-headers loop, guardian not in recovery, no new snap-sync trigger, all nodes cross boundaries with gap=0.',
  json('["INC-I-138","INC-I-012","INC-I-116","INC-I-075","INC-I-062","INC-I-059","INC-I-053","INC-I-045","INC-I-044","INC-I-046","INC-I-067","INC-I-006","INC-006","INC-I-037","INC-I-083"]'),
  'convergence,no-spurious-escalation,no-empty-headers-loop', json('{"nodes":6}'), 'active')
ON CONFLICT(scenario_id) DO UPDATE SET incident_ids=excluded.incident_ids, description=excluded.description, assertions=excluded.assertions, status='active';

-- GS-003 — snap-synced node crossing an epoch boundary: reward pool + producer
-- set stay complete, state root matches, epoch block accepted.
INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, status)
VALUES ('GS-003', 'snap-synced-node-epoch-crossing',
  'Guards snap-synced state completeness across an epoch boundary. Recurrence shows as reward-pool divergence, ProducerSet divergence, or a rejected epoch block. Runner check: stateRoot/csHash/psHash and utxoCount/producerCount agree across all nodes; reward pool consistent; no rejected/invalid-epoch markers.',
  json('["INC-I-118","INC-I-010","INC-I-082","INC-I-028","INC-I-029","INC-I-064","INC-I-080","INC-I-112","INC-I-054","INC-I-060"]'),
  'convergence,state-root-match,no-rejected-epoch-block', json('{"nodes":6}'), 'active')
ON CONFLICT(scenario_id) DO UPDATE SET incident_ids=excluded.incident_ids, description=excluded.description, assertions=excluded.assertions, status='active';

-- GS-004 — one-block fork + recovery (THE active perturbation): a launchd
-- single-node restart must rejoin without a self-fork or a rollback loop.
INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, status)
VALUES ('GS-004', 'one-block-fork-recovery',
  'Guards short-fork recovery. Active perturbation: launchd-restart one producer (non-destructive) and require it rejoins the canonical tip within 60s with no self-fork and no ShallowRollback loop. Recurrence shows as a permanent stall, a rollback loop, or a self-fork on restart.',
  json('["INC-I-089","INC-I-090","INC-I-081","INC-I-036","INC-I-049","INC-I-025","INC-I-024","INC-I-040","INC-I-039","INC-I-034","INC-I-026","INC-I-050","INC-I-053","INC-I-035","INC-I-030","INC-I-019","INC-I-022","INC-I-069","INC-I-103","INC-001","INC-003","INC-005","INC-004"]'),
  'convergence,recovery-under-60s,no-rollback-loop', json('{"nodes":6}'), 'active')
ON CONFLICT(scenario_id) DO UPDATE SET incident_ids=excluded.incident_ids, description=excluded.description, assertions=excluded.assertions, status='active';

-- GS-005 — late-joining node: full-sync + complete block store, no orphan-chase
-- storm, bounded request rate.
INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, status)
VALUES ('GS-005', 'late-joining-node',
  'Guards a node that joins/rejoins behind the tip. Recurrence shows as a stuck DownloadingHeaders loop, an incomplete block store after sync, or an orphan-chase request storm. Runner check: every node reports gap=0 and sync_fails=0, no request-rate storm in the window, block store contiguous (utxoCount/producerCount consistent with peers).',
  json('["INC-I-060","INC-002","INC-I-031","INC-I-032","INC-I-033","INC-I-023","INC-I-042","INC-I-043","INC-I-051","INC-I-138","INC-I-103","INC-I-008","INC-I-004","INC-I-005","INC-I-007","INC-I-001"]'),
  'convergence,block-store-complete,bounded-request-rate', json('{"nodes":6}'), 'active')
ON CONFLICT(scenario_id) DO UPDATE SET incident_ids=excluded.incident_ids, description=excluded.description, assertions=excluded.assertions, status='active';

-- GS-006 — stale-message flood: stale/old gossip is not re-forwarded unboundedly
-- and does not trigger a false rollback; memory stays bounded.
INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, status)
VALUES ('GS-006', 'stale-block-flood',
  'Guards stale-gossip handling. Recurrence shows as unbounded re-forwarding of stale blocks/announcements, a memory spike, or a false rollback from orphan/stale noise. Runner check: gossipsub dedup active (already-published shedding present), rollback_depth stays 0 despite orphan noise, node RSS bounded, no panic.',
  json('["INC-I-114","INC-I-137","INC-I-101","INC-I-100","INC-I-049","INC-I-008","INC-I-036","INC-I-038","INC-I-065"]'),
  'no-reforward,bounded-memory,no-panic', json('{"nodes":6}'), 'active')
ON CONFLICT(scenario_id) DO UPDATE SET incident_ids=excluded.incident_ids, description=excluded.description, assertions=excluded.assertions, status='active';

-- GS-007 — rollback + rejoin: undo-path rollback leaves no zombie state; state
-- root and integrity stay complete after rejoin.
INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, status)
VALUES ('GS-007', 'rollback-rejoin',
  'Guards rollback safety. Recurrence shows as zombie UTXOs after a checkpoint/undo rollback, integrity gaps, or a divergent state root after rejoin. Runner check: guardian recovery_mode=false with an advancing healthy checkpoint; stateRoot/utxoHash agree across nodes; no integrity-gap markers.',
  json('["INC-I-029","INC-I-041","INC-I-082","INC-I-090","INC-I-055","INC-I-030","INC-I-112","INC-I-136","INC-I-120"]'),
  'convergence,state-root-match,integrity-complete', json('{"nodes":6}'), 'active')
ON CONFLICT(scenario_id) DO UPDATE SET incident_ids=excluded.incident_ids, description=excluded.description, assertions=excluded.assertions, status='active';

-- GS-008 — scale-mismatch smoke: run ALL protection mechanisms at small N and
-- assert none self-starves (the recurring calibration defect class).
INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, status)
VALUES ('GS-008', 'scale-mismatch-smoke',
  'Guards the scale-calibration defect class: a threshold tuned for mainnet N that self-starves the N=6 testnet (or a leak that only shows under sustained small-N run). Runner check: production keeps advancing (liveness), no spurious escalation/eviction churn, RSS bounded and non-growing, busy/rate-limit rejection rate low.',
  json('["INC-I-016","INC-I-138","INC-I-117","INC-I-102","INC-I-104","INC-I-111","INC-I-108","INC-I-070","INC-I-073","INC-I-050","INC-I-091","INC-I-137","INC-I-120","INC-I-057","INC-I-038"]'),
  'busy-rate-under-10pct,no-self-starvation,convergence', json('{"nodes":6}'), 'active')
ON CONFLICT(scenario_id) DO UPDATE SET incident_ids=excluded.incident_ids, description=excluded.description, assertions=excluded.assertions, status='active';

COMMIT;

-- ── DELIBERATELY UNMAPPED (out of system-dynamics scope) ─────────────────────
-- These Level-2+ incidents are NOT runtime system-dynamics modes reproducible on
-- a live multi-node run, so no gauntlet scenario covers them (by design):
--   INC-I-086, INC-I-087  — RPC returned hardcoded/zero diagnostics (cosmetic RPC)
--   INC-I-094, INC-I-098, INC-I-096 — DeFi/AMM/covenant validation logic
--   INC-I-088            — Phase-0 safety-gate freeze (consensus config, not dynamics)
--   INC-I-085            — bond-cap validation rule (static validation)
--   INC-I-078            — delegation concentration economic audit
--   INC-I-052            — creator_hash immutability auth rule
--   INC-I-057            — mempool fee/gossip stuck tx (partially GS-008); DeFi-adjacent
-- v_gauntlet_coverage will list these as scenarios=NULL; that is the intended,
-- documented gap — the gauntlet does not fabricate coverage for non-dynamics bugs.
