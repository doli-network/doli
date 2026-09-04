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

INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, status)
VALUES ('GS-009', 'fleet-rolling-restart',
  'Guards fleet rolling-restart safety (simultaneous STARTUP_GATE stall -> sibling fork -> permanent INTEGRITY -1). INJECTS (opt-in only, --gs009 WITH GAUNTLET_GS009_CONFIRM=1): tight-wave restarts ALL producers (n1..n12, NEVER the seed) via launchd stop/start, then ASSERTS no production stall > GS009_STALL_MAX_SLOTS (default 6), no sibling fork (>=2 distinct block hashes at one height across nodes), full producer rejoin to the canonical tip, and (INC-I-196) that every producer which resolved an OnChain release trust root still resolves a USABLE one afterwards. Perturbative like --chaos and NOT part of the default run; on any non-injected run all four assertions SKIP (never a spurious gate failure). Replays the INC-I-143 shape: a rolling producer-content deploy put many scheduled leaders behind STARTUP_GATE at once -> 34-slot stall -> competing h=108456 blocks -> INTEGRITY -1 on all 15 structural nodes. The INC-I-196 leg reads getUpdateStatus (wired to resolve_trust_root, the actual verification decision) and NOT getMaintainerSet, which reports only the persisted state the bug left intact; it asserts a PROPERTY (provenance OnChain, usable, keys >= threshold) rather than before==after, so a legitimate maintainer rotation does not fail it, and it SKIPS when no producer held an OnChain root or none answers afterwards. The same leg is the runtime check for INC-I-175: the compiled bootstrap array had publicly-known private halves, so asserting that every producer resolves its ON-CHAIN root and NOT Bootstrap is what proves the leaked compiled keys are not install authority on these hosts. The static half of INC-I-175 is enforced at build time by req_196_004 in crates/updater/tests/trust_root_fail_closed.rs, not here.',
  json('["INC-I-143","INC-I-062","INC-I-075","INC-I-089","INC-I-186","INC-I-190","INC-I-196","INC-I-175"]'),
  'gs009-no-stall,gs009-no-sibling-fork,gs009-fleet-rejoin,gs009-trust-root-provenance', json('{"nodes":6}'), 'active')
ON CONFLICT(scenario_id) DO UPDATE SET incident_ids=excluded.incident_ids, description=excluded.description, assertions=excluded.assertions, status='active';

INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, runner, status)
VALUES ('GS-015', 'newest-release-published-and-signed',
  'Guards the release-delivery path (CI published a release whose SIGNATURES.json held 0 entries, so every fail-closed `doli upgrade` refused it with "Insufficient signatures: 0/3" and nothing in the repo noticed). ASSERTS (observational, READ-ONLY, no confirm-var, part of the default run): the newest v* tag has a PUBLIC (non-draft) GitHub release that `doli release verify` accepts against the maintainer trust root (delegated to scripts/monitor-release-signed.sh), and .github/workflows/release.yml still carries the `draft: true` gate on its release-creation step -- the one thing keeping an unsigned CI artifact unreachable, whose revert is otherwise silent. It reads the GitHub release API and this local repo only: never the chain, never a node, never a mutating gh/doli subcommand. NOT AUTO-INJECTED: reproducing the trigger means publishing a real unsigned release. Preflighted on gh (present + authenticated), jq, git, a v* tag and a resolvable doli CLI -- any one absent SKIPs (rc 2), never FAILs, because a false FAIL is how a scenario earns a standing waiver and stops guarding anything. Library: scripts/gauntlet-gs015.sh; tests: scripts/test_gauntlet_gs015.sh.',
  json('["INC-I-202"]'),
  'gs015-newest-release-published-and-signed,gs015-workflow-drafts-releases', json('{"nodes":0,"read_only":true}'), 'gauntlet.sh', 'active')
ON CONFLICT(scenario_id) DO UPDATE SET incident_ids=excluded.incident_ids, description=excluded.description, assertions=excluded.assertions, runner=excluded.runner, status='active';
INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, status)
VALUES ('GS-016', 'finality-wedge-operator-escape',
  'C-12 live drill (specs/fork-lifecycle-architecture.md:283). Guards the AUDITED replacement for LB-4 — the poison-rollback bypass of the finality guard that was the fleet''s only wedge escape (13/27 nodes, INC-I-190), whose alternative was history-destroying snap sync. INJECTS (opt-in only, --gs016 WITH GAUNTLET_GS016_CONFIRM=1, testnet only): finds a live node in the recorded cell (0 < gap <= 50 with a [WEDGED] reason=finality_conflict terminal, the M3 classifier naming tip == finality after the recovery ladder ran out of rungs), names the fleet''s branch for it via forceReorgTo, and ASSERTS the node landed on the OPERATOR-NAMED branch at the named height, that verifyChainIntegrity reports no NEW missing range (REQ-FORK-011), that no snapshot was applied in the window, and that no BLOCK_POISON event fired in the window. REFUSES below h=80,700 (trap T10: under that height the testnet still runs plan_reorg''s pre-activation branch, which mainnet no longer runs, so a pass would prove nothing about the path mainnet takes). SKIPS cleanly — never a spurious failure — when the fleet does not expose forceReorgTo (probed with a malformed hash: -32602 present, -32601 absent; the live fleet runs v6.26.1, which predates the method), when no node is in the wedge cell (a healthy fleet has none by definition and this scenario will not fork a live testnet to manufacture one), or when no healthy donor can serve the branch hash. STATE-NEUTRAL for the fleet: the rescued node converges onto the branch every other node already holds; nothing is submitted to the chain and no other node is touched. Assertions key off structured telemetry ([WEDGED] reason=, [FORCE_REORG] outcome=, [SNAP_SYNC] Applying snapshot, [BLOCK_POISON]) and never on the bare word "rollback", which logs ~1/sec at depth 0. The deterministic half of C-12 is the in-process suite bins/node/tests/it/inc_i_204_m41_rescue.rs, which CI runs on every commit.',
  json('["INC-I-190","INC-I-204","INC-I-081","INC-I-147","INC-I-143"]'),
  'gs016-escape-lands-on-named-branch,gs016-no-new-gap-after-escape,gs016-no-snap-sync-in-window,gs016-no-poison-bypass-in-window', json('{"nodes":6}'), 'active')
ON CONFLICT(scenario_id) DO UPDATE SET incident_ids=excluded.incident_ids, description=excluded.description, assertions=excluded.assertions, status='active';

INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, runner, status)
VALUES ('GS-017', 'over-cap-addbond-refused',
  'Guards the AddBond cap-admission path (a producer already holding 1 bond asked for 3000 more; the CLI built, signed and submitted it, and the toxic AddBond sat in 13 of 18 mempools poisoning block assembly -- every scheduled leader that packed it discarded its own block and lost the slot). ASSERTS (observational, chain-read-only, state-neutral, no confirm-var, part of the default run): (0) PRECONDITION -- the resolved doli CLI carries M3, proved by git ancestry of the (sha) in `doli --version` against GS017_M3_COMMIT; it SKIPs and blocks the submit otherwise, so a stale CLI can never make the gauntlet inject INC-I-203 itself (an unreachable --rpc probe cannot substitute: the M3 guard runs after get_network_params, so pre-M3 and M3 die at the same connection error, and getNodeInfo carries no commit so the node''s M2 status is not determinable over RPC); (1) the CLIENT path (INC-I-203 M3, bins/cli/src/producer_ledger.rs addbond_headroom_check) refuses `producer add-bond` BEFORE it signs, at a count of EXACTLY headroom+1 -- the count is DERIVED from the live bondCount, never hardcoded, because any count at or below the headroom is one the node ACCEPTS and would bond real funds on an unattended default run; ONLY the M3 client text `Bond cap exceeded` counts, and an `RPC error` envelope, the node text [ADDBOND_CAP_EXCEEDED] or a `Submitting add-bond transaction` line all FAIL -- each proves the CLI reached the node, i.e. the client guard was absent or bypassed; (2) no `addbond` HASH survives both a pre- and a post-window mempool sweep of 8500-8517 (the sweep that would have seen 988630d9 sit on seed/n2/n6/n13 while every node reported itself healthy) -- getMempoolTransactions exposes no producer and no bond count so over-cap cannot be filtered directly, a settle of >= 2 slots separates a stuck tx from ordinary in-flight traffic, and since that RPC has no offset and no cursor every request asks for the 500-tx hard cap and getMempoolInfo.txCount above it FAILs loudly instead of sampling; (3) no NEW [BLOCK_POISON] ADDBOND_CAP_EXCEEDED past a per-log byte offset across EVERY n*.log on disk (NODECFG stops at n12 while the fleet is 17 nodes, so it supplies offsets but never the scan set) -- every log still carries pre-fix events, so an absolute count is red forever and only GROWTH is a finding. The NODE admission path is NOT exercised here and must not be reached by bypassing the CLI guard: it is covered by the 12 regression tests linked to INV-BOND-002 plus the INC-I-203 M2 live testnet evidence. NOT AUTO-INJECTED: reproducing the trigger means bonding real funds. Every precondition SKIPs (rc 2), never FAILs -- no wallet on disk, the mapped producer already at the cap, no live producer RPC, no resolvable doli CLI, no readable node log -- because one false FAIL is how a scenario earns a standing waiver and stops guarding anything. Library: scripts/gauntlet-gs017.sh; tests: scripts/test_gauntlet_gs017.sh.',
  json('["INC-I-203"]'),
  'gs017-cli-carries-m3,gs017-cli-refuses-before-signing,gs017-no-addbond-residency,gs017-no-cap-poison-in-window', json('{"nodes": 18, "producers": 5}'), 'gauntlet.sh', 'active')
ON CONFLICT(scenario_id) DO UPDATE SET incident_ids=excluded.incident_ids, description=excluded.description, assertions=excluded.assertions, runner=excluded.runner, status='active';

INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, runner, status)
VALUES ('GS-018', 'attestation-bitfield-integrity',
  'Guards the attestation bitfield / presence-root integrity path (INC-I-178: the BLS half of every attestation was carried, gossiped and stored but NEVER verified, so a producer could be credited for attendance it never signed and the aggregate signature was decorative). ASSERTS (observational, chain-read-only, state-neutral, testnet-only, no confirm-var, part of the default run): (1) gs018-presence-root-consistent -- every answering node reports the SAME presenceRoot at each of GS018_SAMPLE recent heights, sampled GS018_LAG blocks below the lowest tip so a node one slot behind is not read as a divergence; it SKIPs below GS018_MIN_NODES answering nodes because agreement over two nodes is not cross-node agreement, it NEVER reads attestationCount as a headcount (that field is a popcount of the presence_root HASH, so a verdict driven by it is driven by hash entropy), and a block carrying an aggregateBlsSig is recorded rather than failed on, since its presence IS the activation-height litmus. (2) gs018-post-ah-aggregate-verifies -- gated on that same litmus (doli_attestation_verify_total > 0, OR getAttestationStats.blocksWithBls > 0, OR aggregateBlsSig on a sampled block; no RPC exposes inc_i_178_attestation_bls_activation_height and it is u64::MAX on every network), then requires doli_attestation_verify_rejected_total == 0 across the nodes that expose the counters. (3) gs018-active-producers-dual-sign -- REQ-BLS-006 AC-2 (100% of active producers emit BLS-signed attestations), observable since INC-I-178 M7.5 shipped the per-attester counter. The evidence is the union of above-zero doli_attestation_bls_valid_attester_total{attester} labels read across the GS018_METRICS_PORTS /metrics endpoints, joined by 8-hex prefix to the getProducers rows with status==active. That chain-derived active set is the denominator -- NEVER the node count, never the registered rows. It PASSes only at matched == active, and FAILs naming the active prefixes that carry no series: those producers are not dual-signing, and pinning the activation height would strip them from every aggregate. THREE states SKIP rather than manufacture a green -- the series absent on every node (the fleet is below M7.5; keyed on the ABSENCE of the emission signal, never on a version string, since M7.5 bumps no version), fewer than GS018_MIN_NODES capable nodes (the INC-I-178 M7.6 union floor: a union read over a partially scraped fleet is not a fleet observation, and mid-rolling-deploy a thin union would report unscraped dual-signers as not dual-signing and manufacture a false FAIL), and an empty label union (a node restarted seconds ago has ingested nothing, so a zero-length observation window is not evidence that nobody dual-signs). Two false greens stay refused: getAttestationStats.hasBls is BLS-key REGISTRATION, already true for all 7 producers on the PRE-INC-I-178 build, so reading it as emission would pass before a single line of the fix shipped; and concluding 5/5 dual-signing from an ABSENCE of [ATTEST_INGEST] unverifiable-BLS-half warnings is the same false green, because that line fires only on a relayed INVALID half. Build detection is by CAPABILITY (the series doli_attestation_verify_total on /metrics), never by version: the INC-I-178 build reports 6.26.3, byte-identical to the fleet it replaces. Every precondition (RPC down, python3 missing, non-testnet fleet) is a SKIP with a written reason, never a FAIL.',
  json('["INC-I-178"]'),
  'gs018-presence-root-consistent,gs018-active-producers-dual-sign,gs018-post-ah-aggregate-verifies', json('{"nodes": 18, "producers": 5}'), 'gauntlet.sh', 'active')
ON CONFLICT(scenario_id) DO UPDATE SET incident_ids=excluded.incident_ids, description=excluded.description, assertions=excluded.assertions, runner=excluded.runner, status='active';

INSERT INTO gauntlet_scenarios (scenario_id, name, description, incident_ids, assertions, scale_params, runner, status)
VALUES ('GS-019', 'attestation-aggregate-poisoning',
  'Guards the attestation-aggregate poisoning path (INC-I-178 / INC-I-191 / INC-I-192: attester weight is self-declared and the BLS half is unverified, so a forged aggregate over a bitfield a producer never signed would be accepted and credited). INJECTS (opt-in only, --gs019 WITH GAUNTLET_GS019_CONFIRM=1, testnet-only): a forged aggregate attributed to a victim producer, then ASSERTS gs019-poison-rejected (every node rejects it and the victim is not credited), gs019-fleet-liveness-through-poison (no stall and no fork across the poison window) and gs019-victim-attendance-preserved (the impersonated producer''s attendance and reward qualification survive). WHAT IT DELIBERATELY DOES NOT DO TODAY: it injects NOTHING and all three assertions SKIP permanently with the reason ''no injection path; needs a submit RPC''. That is the measured state of the ingress surface, not a workaround -- no submitAttestation, directAttestation or sendAttestation exists anywhere in crates/rpc/src/methods/ (the dispatch table carries only the read-only getAttestationStats and the unrelated oracle PriceAttestation), and the single ingress is the libp2p gossipsub topic /doli/attestations/1, which requires a Noise-encrypted transport, mesh admission, a payload that deserializes as Attestation, a passing Ed25519 verify AND ProducerSet membership, so curl cannot reach it. A token returning PASS here would certify a poison rejection nobody ever attempted. The --gs019 flag, the GAUNTLET_GS019_CONFIRM gate, the testnet guard, the inj_tag() line and the $WORK/gs019_injected marker are all real and armed so the scenario works the day a submit RPC exists: the injector checks both consents itself and writes the marker ONLY on a delivered poison, and the assertions treat a marker present without an ingress as stale rather than as proof of injection. Retrofitting a consent gate onto a scenario that already injects is how a destructive run escapes review.',
  json('["INC-I-178"]'),
  'gs019-poison-rejected,gs019-fleet-liveness-through-poison,gs019-victim-attendance-preserved', json('{"nodes": 18, "producers": 5}'), 'gauntlet.sh', 'active')
ON CONFLICT(scenario_id) DO UPDATE SET incident_ids=excluded.incident_ids, description=excluded.description, assertions=excluded.assertions, runner=excluded.runner, status='active';

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
