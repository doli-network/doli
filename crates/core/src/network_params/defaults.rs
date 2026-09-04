//! Hardcoded default parameters for each network
//!
//! These match the original values in `consensus.rs` (the DNA).
//! Mainnet values are immutable; testnet/devnet may be overridden via env.

use crate::Network;

use super::NetworkParams;

impl NetworkParams {
    /// Get hardcoded default parameters for a network
    ///
    /// These match the original hardcoded values in consensus.rs (the DNA)
    pub fn defaults(network: Network) -> NetworkParams {
        use crate::consensus;

        match network {
            Network::Mainnet => NetworkParams {
                // Networking
                default_p2p_port: 30300,
                default_rpc_port: 8500,
                default_metrics_port: 9000,
                bootstrap_nodes: vec![
                    "/dns4/seed1.doli.network/tcp/30300".to_string(),
                    "/dns4/seed2.doli.network/tcp/30300".to_string(),
                    "/dns4/seeds.doli.network/tcp/30300".to_string(),
                ],
                bootnode_enrs: vec![], // Populated when mainnet bootnodes are deployed
                max_peers: 50, // Ethereum default (geth); GossipSub mesh handles propagation

                // Timing
                slot_duration: consensus::SLOT_DURATION,
                genesis_time: consensus::GENESIS_TIME,
                veto_period_secs: 5 * 60, // 5 minutes (early network, small maintainer set)
                grace_period_secs: 2 * 60, // 2 minutes
                bootstrap_grace_period_secs: consensus::BOOTSTRAP_GRACE_PERIOD_SECS,
                unbonding_period: consensus::UNBONDING_PERIOD, // blocks (already u64)
                inactivity_threshold: u64::from(consensus::INACTIVITY_THRESHOLD),

                // Economics
                bond_unit: consensus::BOND_UNIT,
                initial_reward: consensus::INITIAL_REWARD,
                registration_base_fee: 100_000,      // 0.001 DOLI
                max_registration_fee: 1_000_000_000, // 10 DOLI
                automatic_genesis_bond: consensus::BOND_UNIT,
                genesis_blocks: 360, // 1 hour (open registration period)

                // VDF (1000 iterations ~= 0.07ms — bond is the real Sybil protection)
                vdf_iterations: 1_000,
                heartbeat_vdf_iterations: 1_000,
                vdf_register_iterations: 1_000,

                // Time structure
                blocks_per_year: consensus::SLOTS_PER_YEAR as u64,
                blocks_per_reward_epoch: consensus::BLOCKS_PER_REWARD_EPOCH,
                coinbase_maturity: consensus::COINBASE_MATURITY,
                slots_per_reward_epoch: consensus::SLOTS_PER_REWARD_EPOCH,
                bootstrap_blocks: consensus::BOOTSTRAP_BLOCKS,

                // Update system
                min_voting_age_secs: 30 * 24 * 3600, // 30 days
                update_check_interval_secs: 10 * 60, // 10 minutes (early network)
                crash_window_secs: 3600,             // 1 hour
                max_registrations_per_block: 5,

                // Presence (telemetry)
                presence_window_ms: consensus::NETWORK_MARGIN_MS, // Use consensus margin

                // Fallback timing (locked for mainnet)
                fallback_timeout_ms: consensus::FALLBACK_TIMEOUT_MS,
                max_fallback_ranks: consensus::MAX_FALLBACK_RANKS,
                network_margin_ms: consensus::NETWORK_MARGIN_MS,

                // Vesting (locked for mainnet — consensus critical)
                vesting_quarter_slots: consensus::VESTING_QUARTER_SLOTS as u64,

                // Hard fork gates
                sig_verification_height: 0, // P0-001: enforce from genesis (clean chain)
                snap_attestation_skip_height: 0, // INC-I-010: always active (clean chain)
                // INC-I-026: scheduler fix activates at h=30500 on mainnet.
                // Forward-only activation per CLAUDE.md #0 RULE — pre-activation
                // blocks keep their state roots unchanged. Chosen as a comfortable
                // future height past the recent fork cluster (~h=24640) to give all
                // operators time to upgrade to v6.7.8+. A matching HardForkSchedule
                // entry at the SAME height in crates/updater/src/hardfork.rs stops
                // any node running <6.7.8 from producing past activation.
                inc_i_026_scheduler_activation_height: 0,
                fork_id_activation_height: 0,
                // Mainnet genesis reset — all features active from genesis
                full_bitfield_decode_height: 0,
                rewards_epoch_list_fix_height: 0,
                encrypted_content_activation_height: 0,
                encrypted_content_v2_activation_height: 0, // Fresh genesis — MIME + royalties active from block 0
                epoch_state_reorg_activation_height: 0,
                // Security audit fixes: active from genesis on the fresh chain.
                security_audit_activation_height: 0,
                // INC-I-046: Ghost exclusion active from genesis on the fresh chain.
                ghost_exclusion_activation_height: 0,
                // INC-I-116: epoch-boundary liveness prune (absolute MIN_PRODUCERS_FLOOR
                // instead of proportional 2/3 floor). ACTIVE FROM GENESIS on the fresh
                // chain — the 2026-07-08 genesis reset predates any block that could
                // have been committed under the old proportional floor, so there is no
                // historical epoch-state rebuild to stay bit-compatible with and no
                // future AH is needed. (The earlier 455040 pin, decided 2026-06-21 at
                // tip 446352, belonged to the PRE-RESET chain and no longer applies.)
                epoch_prune_activation_height: 0,
                // INC-I-190 F3: cap-bound the MIN_PRODUCERS_FLOOR fallback.
                // Pinned 332_664 on 2026-08-29 (user decision-session per HC-6 /
                // INC-I-075): mainnet tip 323_680 measured read-only at pin time
                // (~25h lead at ~10s slots). IMMUTABLE once the chain crosses it —
                // never move a crossed AH (INC-I-054). The whole fleet AND the
                // external auto-update population must run >= this binary BEFORE
                // 332_664, or the gate activates on a mixed fleet.
                inc_i_190_floor_bound_activation_height: 332_664,
                // INC-I-068 / INC-I-075: filter weight=0 producers out of the active
                // list. ACTIVE FROM GENESIS on the fresh chain — same reasoning as
                // above: the re-gate at a future height existed to make the
                // consensus-shape change activate synchronously on the pre-reset chain,
                // which no longer exists. Confirmed live on mainnet by INC-I-154.
                inc_i_068_weight_filter_activation_height: 0,

                // INC-I-078: delegation concentration mitigation (approved
                // bundle, User Gate 2026-05-17). Defaults to disabled
                // (activation height u64::MAX, cap u64::MAX) on mainnet until
                // the operator picks a concrete future height before deploy.
                // BOTH heights must be set to the SAME value to ship the
                // bundle atomically.
                // INC-I-078 mainnet activation: cap=3000 (= MAX_BONDS_PER_PRODUCER),
                // both gates flip atomically at h=254_344. Cap value chosen for
                // symmetry with the own-bonds ceiling: total influence per producer
                // <= 2 * MAX_BONDS_PER_PRODUCER, skin-in-game floor >= 50%.
                //
                // Re-pin history:
                //   231_830 -> 240_138 (2026-05-19, commit 479711b5): original
                //   lead-time analysis went stale before redeploy; no binary
                //   honored 231_830.
                //   240_138 -> 254_344 (2026-05-21): same situation — the
                //   deployed v6.21.20 fleet binary is commit 77bb3dfa, which
                //   PREDATES all INC-I-078/INC-I-080 gate code. The chain
                //   crossed 240_138 (tip 245_783 at re-pin), but since no
                //   deployed node enforces the gate, the height was never
                //   effectively activated — moving the pin forward is again
                //   the routine pre-activation case, NOT an INC-I-054
                //   violation (F3: no crossed-AND-honored height moved; F7:
                //   not retroactive — 254_344 > tip 245_783).
                //   254_344 - 245_783 = 8_561 blocks ≈ 23.8h binary-distribution
                //   lead. Once the chain CROSSES 254_344 AND a binary honoring
                //   it is deployed, this height becomes IMMUTABLE — never move
                //   it forward thereafter (INC-I-054).
                received_delegation_cap: 3000,
                received_delegation_cap_activation_height: 0,
                delegation_auth_activation_height: 0,
                // INC-I-080: AddBond cap enforcement pinned to h=254_344 —
                // co-deployed atomically with the INC-I-078 bundle (same
                // upgrade event, same lead-time analysis). Above the chain head
                // (F7: NOT retroactive). Pre-254_344 the historical clip path
                // runs; at 254_344 every upgraded node begins rejecting
                // over-cap AddBonds in lockstep. Once crossed this height is
                // IMMUTABLE — never move it forward (INC-I-054).
                addbond_cap_enforcement_activation_height: 0,
                withdrawal_holdings_gate_activation_height: 317_861,

                // INC-I-088 Phase 0: DeFi subsystems (AMM, lending, loan,
                // fractionalization) gated off on mainnet. u64::MAX = never
                // activated. Operator pins a concrete future height in a
                // separate commit ONLY AFTER the per-type validator gaps
                // are fixed and the subsystems are audit-clean. NEVER lower
                // this without an explicit decision documented in
                // specs/state-of-the-art-architecture.md.
                // Fresh mainnet genesis (2026-07-08): activated from block 0 per
                // operator directive. The 7 gated DeFi types remain tombstoned
                // (cannot be constructed), so the open gate has no reachable path.
                defi_activation_height: 0,

                // AMM Foundations M1 — mainnet activation pinned 2026-06-03.
                // Tip at pin time: 359_042. Lead time: 8_618 blocks
                // (~23.9h at 10s slots) for fleet binary distribution.
                // INDEPENDENT of defi_activation_height (HC-6 / INC-I-075).
                // Co-activates atomically at the SAME height with:
                //   - inc_i_092_activation_height (spend-path auth/funding)
                //   - inc_i_096_activation_height (pool-aware conservation)
                //   - large_block_activation_height (INC-I-091, ~300 TPS)
                // D1 (MINIMUM_LIQUIDITY=1000), D2 (fee_bps in pool_id),
                // D4 (getDefiHealthMetric) foundations locks are in code
                // pre-pin. Once the chain crosses this height it is
                // IMMUTABLE — never move it forward (INC-I-054). Spec:
                // specs/defi-foundations-economics.md §0.
                //
                // Re-pin history:
                //   u64::MAX → 367_660 (2026-06-03): initial pin; binary
                //     was never deployed to the fleet before the height
                //     was approached. No honored crossed height moved.
                //   367_660 → 375_640 (2026-06-04): fresh pin with a new
                //     deployable lead time. Tip at pin time: 366_960,
                //     lead time: 8_680 blocks (~24.1h at 10s slots).
                amm_activation_height: 0,

                // Phase 2.1 Oracle (structural-anchored): u64::MAX = frozen.
                // PriceAttestation (TxType=16) is rejected at validation and
                // mempool until the operator pins a concrete future height,
                // ONLY AFTER M2-M11 land + testnet activation experiment.
                // Independent of defi_activation_height per HC-6 / INC-I-075.
                // Spec: specs/oracle-structural-anchored-economics.md §1.10.
                oracle_activation_height: u64::MAX,

                // Large blocks (>1 MB) → ~300 TPS (INC-I-091). Mainnet
                // activation pinned 2026-06-04 at h=375_640. Tip at pin
                // time: 366_960. Lead time: 8_680 blocks (~24.1h).
                //
                // Re-pin history:
                //   308_980 → u64::MAX (2026-05-29): gossip-cap binary
                //   never deployed; chain (319_012) overtook the pin —
                //   routine pre-activation un-pin, NOT an INC-I-054
                //   violation (no honored crossed height moved).
                //   u64::MAX → 367_660 (2026-06-03): co-activates with
                //   AMM triplet for a single coordinated upgrade event.
                //   367_660 → 375_640 (2026-06-04): binary at 367_660
                //   was never deployed; re-pinned with fresh deployable
                //   lead time. No honored crossed height moved.
                //
                // Builder policy (block content), not consensus rules.
                // Once crossed IMMUTABLE (INC-I-054).
                large_block_activation_height: 0,

                // INC-I-092 DeFi spend-path fixes (RC-A pool-input auth, RC-B
                // pool_create funding). Mainnet activation pinned 2026-06-04
                // at h=375_640 — co-activated atomically with
                // amm_activation_height so AMM goes live already-correct
                // (no separate consensus event). Tip at pin time: 366_960,
                // lead time 8_680 blocks (~24.1h). IMMUTABLE once crossed
                // (INC-I-054).
                //
                // Re-pin history:
                //   u64::MAX → 367_660 (2026-06-03): initial co-pin with
                //     AMM triplet; binary never deployed.
                //   367_660 → 375_640 (2026-06-04): fresh deployable
                //     lead time. No honored crossed height moved.
                inc_i_092_activation_height: 0,
                // INC-I-096 pool-aware AMM value-conservation. Mainnet
                // activation pinned 2026-06-04 at h=375_640 — equal to
                // amm_activation_height (INV-DEPLOY-002 satisfied:
                // inc_i_096 ≤ amm). Pool-aware conservation engages the
                // same block AMM tx types become valid. Tip at pin time:
                // 366_960, lead time 8_680 blocks (~24.1h). IMMUTABLE
                // once crossed (INC-I-054).
                //
                // Re-pin history:
                //   u64::MAX → 367_660 (2026-06-03): initial co-pin with
                //     AMM triplet; binary never deployed.
                //   367_660 → 375_640 (2026-06-04): fresh deployable
                //     lead time. No honored crossed height moved.
                inc_i_096_activation_height: 0,

                // INC-I-147 fork-choice height-unit fix (D6) + rolled-back-block
                // re-apply (D4). Q1=NO on the merits, Q2=NO — gated as a rollout
                // coordination device. IMMUTABLE once crossed (INC-I-054).
                // Pinned 2026-08-05 at live tip 120_799 (~24.2h lead, AMM-pin
                // precedent), v6.24.1 release. Testnet has validated the rule
                // live since 80_700.
                inc_i_147_activation_height: 129_500,
                // INC-I-204 M5 single fork-choice authority (#23). FROZEN at
                // u64::MAX: pinning a real mainnet height is a separate user
                // decision-session (HC-6 shape). Never bundled onto #16.
                inc_i_204_fork_choice_activation_height: u64::MAX,
                // INC-I-178 M4 attestation-BLS semantics. FROZEN at u64::MAX:
                // block CONTENT changes, so pinning is a separate user
                // decision-session after the Release-N soak.
                inc_i_178_attestation_bls_activation_height: u64::MAX,

                // INC-I-172 M2 maintainer trust-root derivation. Gates the
                // one-shot genesis seed (F2), the canonical
                // (registered_at, pubkey_bytes) ordering (F2), the
                // distinct-signer k-of-n governance counter (F3) and the
                // ProtocolActivation fail-close (F4). Q1=YES, Q2=YES, Q3=NO
                // ⇒ activation height REQUIRED. Pinned 2026-08-10 at live tip
                // 162_727 (verified via getChainInfo): 9_273 blocks ≈ 25.8h of
                // lead at 10s slots, matching the INC-I-147 / AMM pin
                // precedent. The height is in the FUTURE at pin time, so no
                // already-executed ProtocolActivation is reinterpreted.
                // IMMUTABLE once crossed (INC-I-054).
                maintainer_derivation_activation_height: 172_000,

                // INC-I-173 state-only fee gate: the exemption becomes the ONE
                // exhaustive TxType::allows_empty_io() authority, so
                // AddMaintainer/RemoveMaintainer can finally be mined.
                // Q1=YES, Q2=YES, Q3=NO ⇒ activation height REQUIRED.
                // PINNED 2026-08-25 at release: measured live tip 308_866,
                // 8_995 blocks (~25 h) of manual-upgrade lead time. Ordering
                // REV-176-M1a-001 requires #22 <= #21; both sit at 317_861, so
                // the authorization binding is in force at the same block the
                // first governance tx can be mined. IMMUTABLE once crossed.
                inc_i_173_activation_height: 317_861,

                // INC-I-176 M2 maintainer-authorization message binding (#22):
                // at/above this height the signed bytes become the domain-tagged,
                // genesis-bound, expiry-carrying BLAKE3 digest instead of
                // `format!("{}:{}", action, target_hex)`. That is the MECHANISM
                // that closes AUDIT-P0-011 (release-signature collision) and
                // AUDIT-P1-016 (cross-network replay), and it takes effect
                // FORWARD-ONLY at and above #22 — never below it.
                // Q1=YES, Q2=NO, Q3=NO ⇒ activation height REQUIRED.
                //
                // AUDIT-P1-102 status: while #22 was `u64::MAX` this gate was
                // fail-CLOSED against premature activation but fail-OPEN for the
                // defects themselves — the legacy colliding message stayed in
                // force at every mainnet height, leaving AUDIT-P0-011 and
                // AUDIT-P1-016 OPEN. Pinning below CLOSES both, but only FROM
                // 317_861: every mainnet block beneath it keeps the legacy
                // message, so archived signatures stay verifiable and no history
                // is reinterpreted. Below that height both defects remain live,
                // which is why the upgrade window matters.
                //
                // PINNED 2026-08-25 at release: measured live tip 308_866,
                // 8_995 blocks (~25 h) of manual-upgrade lead time. Activating
                // this at or before #21 is what stops AddMaintainer/
                // RemoveMaintainer becoming mineable while their authorizations
                // are still replayable — the INC-I-175 surface. External
                // producers are upgraded MANUALLY for this release.
                // REV-176-M1a-001 ordering: 317_861 >= #20 (172_000) ✓ and
                // 317_861 <= #21 (317_861) ✓ — both halves hold at equality.
                inc_i_176_auth_binding_activation_height: 317_861,

                // INC-I-172 M2 review F3. Mainnet keeps the historical
                // hardcoded precondition (INITIAL_MAINTAINER_COUNT = 5), so the
                // seed path is byte-identical to M2 as reviewed. Mainnet runs
                // 30+ registered producers, so the precondition has always been
                // satisfied here.
                maintainer_seed_min_producers: crate::maintainer::INITIAL_MAINTAINER_COUNT,

                // Gossip mesh: universal config for all network sizes.
                // mesh_n=12 keeps all peers in eager-push for networks ≤24 (mesh_n_high),
                // and scales to 1000+ nodes at ~3-4 hops with 10s slot margin.
                // Ethereum runs D=8 for 800K validators — 12 is conservative.
                mesh_n: 12,
                mesh_n_low: 8,
                mesh_n_high: 24,
                gossip_lazy: 12,
            },

            Network::Testnet => NetworkParams {
                // Networking
                default_p2p_port: 40300,
                default_rpc_port: 18500,
                default_metrics_port: 19000,
                bootstrap_nodes: vec![
                    "/dns4/bootstrap1.testnet.doli.network/tcp/40300".to_string(),
                    "/dns4/bootstrap2.testnet.doli.network/tcp/40300".to_string(),
                    "/dns4/seeds.testnet.doli.network/tcp/40300".to_string(),
                ],
                bootnode_enrs: vec![], // Populated when testnet bootnodes are deployed
                max_peers: 25,         // Testnet: halved from 50 to reduce Yamux RAM (INC-I-012)
                // Each peer costs ~5MB in Yamux buffers: 25×2×5MB=250MB/node
                // At 200 nodes: ~50GB. At 50: ~12GB. Gossip mesh_n=12 fits in 25.

                // Timing
                slot_duration: consensus::SLOT_DURATION,
                genesis_time: 1777037598,  // Testnet v202 genesis 2026-04-24
                veto_period_secs: 5 * 60,  // 5 minutes (early network)
                grace_period_secs: 2 * 60, // 2 minutes
                bootstrap_grace_period_secs: consensus::BOOTSTRAP_GRACE_PERIOD_SECS,
                unbonding_period: 72, // 2 epochs (2 × 36 blocks)
                inactivity_threshold: u64::from(consensus::INACTIVITY_THRESHOLD),

                // Economics
                bond_unit: 100_000_000, // 1 DOLI
                initial_reward: consensus::INITIAL_REWARD,
                registration_base_fee: 100_000,
                max_registration_fee: 1_000_000_000,
                automatic_genesis_bond: 100_000_000, // 1 DOLI (matches testnet bond_unit)
                genesis_blocks: 36, // 1 epoch (~6 min) — matches blocks_per_reward_epoch

                // VDF (1000 iterations — same as mainnet)
                vdf_iterations: 1_000,
                heartbeat_vdf_iterations: 1_000,
                vdf_register_iterations: 1_000,

                // Time structure (shorter epochs for faster testing)
                blocks_per_year: consensus::SLOTS_PER_YEAR as u64,
                blocks_per_reward_epoch: 36, // ~6 min per epoch (10x faster than mainnet)
                coinbase_maturity: consensus::COINBASE_MATURITY,
                slots_per_reward_epoch: 36, // ~6 min per epoch
                bootstrap_blocks: consensus::BOOTSTRAP_BLOCKS,

                // Update system
                min_voting_age_secs: 30 * 24 * 3600,
                update_check_interval_secs: 10 * 60, // 10 minutes (early network)
                crash_window_secs: 3600,
                max_registrations_per_block: 5,

                // Presence (telemetry)
                presence_window_ms: consensus::NETWORK_MARGIN_MS,

                // Fallback timing (same as mainnet)
                fallback_timeout_ms: consensus::FALLBACK_TIMEOUT_MS,
                max_fallback_ranks: consensus::MAX_FALLBACK_RANKS,
                network_margin_ms: consensus::NETWORK_MARGIN_MS,

                // Vesting (1-day: 6h quarters — faster than mainnet for testing)
                vesting_quarter_slots: 2_160,

                // Hard fork gates
                sig_verification_height: 0, // P0-001: enforce from genesis (clean chain)
                snap_attestation_skip_height: 0, // Always active (clean chain)
                // INC-I-026: scheduler fix activates at h=17000 on local testnet.
                // Current diverged tips at shutdown were 14075 and 16276 — this
                // leaves ~724 blocks (~2h at 10s slots) for behind nodes to catch up
                // under the old (legacy) scheduler before the fix engages.
                inc_i_026_scheduler_activation_height: 0,
                fork_id_activation_height: 0,
                // Testnet: all features active from genesis (clean chain)
                full_bitfield_decode_height: 0,
                rewards_epoch_list_fix_height: 0,
                encrypted_content_activation_height: 0,
                // 2026-07-07 fresh-genesis redeploy: mirror mainnet activation
                // STATE — every gate enabled on mainnet (finite AH) is set to 0
                // here (active from genesis); gates frozen on mainnet (u64::MAX)
                // stay frozen. MIME + royalties is enabled on mainnet (100_000).
                encrypted_content_v2_activation_height: 0,
                epoch_state_reorg_activation_height: 0,
                // AUDIT-BRIDGE-001 + AUDIT-AUTH-003: enabled on mainnet → 0.
                security_audit_activation_height: 0,
                // INC-I-046: Ghost exclusion enabled on mainnet → 0.
                ghost_exclusion_activation_height: 0,
                // INC-I-116: epoch-boundary liveness prune. Enabled on mainnet
                // (454_977) → 0 here. Safe on a FRESH genesis chain: there is no
                // pre-AH history to rebuild, so AH=0 cannot retroactively prune
                // boundaries the chain never pruned (the integrity-divergence
                // concern only applies when zeroing an AH on a chain with history).
                epoch_prune_activation_height: 0,
                // INC-I-190 F3: cap-bound the MIN_PRODUCERS_FLOOR fallback.
                // Re-pinned 2026-08-28 (AUDIT-P1-502). The first pin, 52_000, was set
                // against a measured tip of 51_498; re-measurement gave 51_756 at 23:20
                // and 51_861 at 23:38 — ~378 blocks/h (~9.5 s/block) — so 52_000 would
                // have been crossed BEFORE the binary was deployed, making the gate a
                // retroactive rule change (INC-I-054 class) and forking a rolling
                // deploy. 58_000 is ~6_100 blocks / ~16 h of headroom from tip 51_861.
                // The long headroom costs nothing: DOLI_INC_I_190_FLOOR_BOUND_ACTIVATION_HEIGHT
                // is honored on non-mainnet, so the AH-crossing test forces the
                // crossing at any height on the local nodes.
                // Never move it once crossed.
                inc_i_190_floor_bound_activation_height: 58_000,
                // INC-I-075: Testnet never ran v6.21.16 in production — always
                // apply the INC-I-068 filter (matches current testnet runtime).
                inc_i_068_weight_filter_activation_height: 0,

                // INC-I-078: delegation cap + auth enabled on mainnet (254_344)
                // → 0 here (active from genesis). Cap=3000 matches mainnet;
                // env-overridable for tuning.
                received_delegation_cap: 3000,
                received_delegation_cap_activation_height: 0,
                delegation_auth_activation_height: 0,
                // INC-I-080: AddBond cap enabled on mainnet (254_344) → 0.
                addbond_cap_enforcement_activation_height: 0,
                withdrawal_holdings_gate_activation_height: 15_087, // re-pinned 2026-08-24 for fresh testnet genesis (tip ~15006); was 230_000

                // INC-I-088 Phase 0: DeFi gate disabled by default on testnet
                // (mirrors mainnet). Tests that exercise the post-activation
                // path override via the env var
                // `DOLI_DEFI_ACTIVATION_HEIGHT` or via
                // `ValidationContext::with_defi_activation_height`.
                defi_activation_height: u64::MAX,

                // AMM Foundations M1: AMM is enabled on mainnet (375_640) → 0
                // here (active from genesis on the fresh chain). Independent of
                // defi_activation_height (HC-6 / INC-I-075). Local experiments
                // still override via `DOLI_AMM_ACTIVATION_HEIGHT`.
                amm_activation_height: 0,

                // Phase 2.1 Oracle: FROZEN on mainnet (u64::MAX — never
                // activated) → frozen here too, mirroring mainnet state. The
                // testnet activation experiment is not re-run on this fresh
                // genesis; PriceAttestation (TxType=16) is rejected at validation
                // and mempool until an operator pins a concrete future height.
                // Local experiments override via `DOLI_ORACLE_ACTIVATION_HEIGHT`.
                oracle_activation_height: u64::MAX,

                // Large blocks (>1 MB) → ~300 TPS (INC-I-091). Testnet = 0
                // (always-on): small controllable fleet, lighter deploy, no AH
                // needed. Override via `DOLI_LARGE_BLOCK_ACTIVATION_HEIGHT`.
                large_block_activation_height: 0,

                // INC-I-092 DeFi spend-path fixes. Enabled on mainnet (375_640,
                // co-activated with AMM) → 0 here so AMM goes live already-correct
                // from genesis (INV-DEPLOY-002: inc_i_092 == amm == 0). Override
                // via `DOLI_INC_I_092_ACTIVATION_HEIGHT`.
                inc_i_092_activation_height: 0,
                // INC-I-096 pool-aware AMM value-conservation. Enabled on mainnet
                // (375_640, == amm) → 0 here. On a fresh chain AMM and the
                // conservation rule engage the same block (genesis), so no
                // grandfather window is needed (INV-DEPLOY-002: inc_i_096 <= amm,
                // 0 <= 0). Override via `DOLI_INC_I_096_ACTIVATION_HEIGHT`.
                inc_i_096_activation_height: 0,

                // INC-I-147 D6/D4 recovery fixes. Pinned 2026-08-04 at live tip
                // 80_544 with ~150 blocks (~15 min) of lead so the whole fleet
                // crosses the gate together — NOT 0, which would reinterpret
                // already-validated history under the new fork-choice rule.
                // IMMUTABLE once crossed (INC-I-054).
                // Override via `DOLI_INC_I_147_ACTIVATION_HEIGHT`.
                inc_i_147_activation_height: 80_700,
                // INC-I-204 M5 single fork-choice authority (#23). Pinned
                // 2026-09-02 by user decision at live tip 87_934 (thin ~13 min
                // lead accepted; tie parity below the gate bounds the window).
                // IMMUTABLE once crossed (INC-I-054).
                // Override via `DOLI_INC_I_204_FORK_CHOICE_ACTIVATION_HEIGHT`.
                inc_i_204_fork_choice_activation_height: 88_014,
                // INC-I-178 M4 attestation-BLS semantics. FROZEN at u64::MAX
                // until the rehearsal pin. Override via
                // `DOLI_INC_I_178_ATTESTATION_BLS_ACTIVATION_HEIGHT`.
                inc_i_178_attestation_bls_activation_height: u64::MAX,

                // INC-I-172 M2 maintainer trust-root derivation. Pinned
                // 2026-08-10 at live testnet tip 126_801 with ~400 blocks
                // (~1h) of lead so the whole local fleet crosses the gate
                // together — NOT 0, which would reinterpret already-validated
                // governance history under the new derivation.
                // IMMUTABLE once crossed (INC-I-054).
                // Override via `DOLI_MAINTAINER_DERIVATION_ACTIVATION_HEIGHT`.
                maintainer_derivation_activation_height: 15_087, // re-pinned 2026-08-24 for fresh testnet genesis (tip ~15006); was 127_200

                // INC-I-173 state-only fee gate. Strictly ABOVE the INC-I-172
                // derivation gate (127_200): the newly mineable maintainer txs
                // must not land before the trust root they mutate is derived.
                // NOT 0 — that would reinterpret already-validated testnet
                // history under the new predicate. IMMUTABLE once crossed
                // (INC-I-054).
                // Override via `DOLI_INC_I_173_ACTIVATION_HEIGHT`.
                //
                // Re-pin history:
                //   u64::MAX → 130_400 (2026-08-10): initial pin. Live testnet
                //     tip at pin time: 129_619. Measured block rate:
                //     10.02 s/block (1000-block sample, timestamps
                //     1786365479 → 1786375499). Lead time: 781 blocks
                //     ≈ 2.17 hours — enough for the whole local fleet to cross
                //     the gate together.
                //   130_400 → 133_000 (2026-08-10): QA ISSUE-001. Live testnet
                //     tip at re-pin time: 130_291. Measured block rate:
                //     10.00 s/block (1000-block sample, heights 129_286 →
                //     130_286, timestamps 1786372169 → 1786382169). New lead
                //     time: 2_709 blocks ≈ 7.53 hours. REASON: the testnet
                //     kept producing throughout M1, so the initial 2.17-hour
                //     lead decayed to ~120 blocks (≈20 min) BEFORE the change
                //     was ever deployed. A height crossed by an un-upgraded
                //     fleet nullifies the mixed-fleet purpose of the gate and
                //     would freeze a wrong value permanently (INC-I-054). The
                //     new lead must cover the remainder of M1 (review +
                //     security audit + commit) PLUS the M2 testnet deploy —
                //     which is why ~2 hours was not enough.
                //   133_000 → 137_000 (2026-08-11): M2 staged testnet deploy.
                //     133_000 was crossed (live tip 136_295) but was NEVER
                //     enforced by any deployed node — the fleet ran v6.24.1,
                //     which has no inc_i_173 gate — so moving it now changes
                //     nothing any node ever acted on (NOT an INC-I-054
                //     violation). Block rate ~10 s/block; new lead ~700 blocks
                //     ≈ 1.9 h, covering the build + synchronized restart.
                //   137_000 → 136_431 (2026-08-11): shortened lead per operator
                //     request for a faster staged test. Still un-crossed and
                //     un-enforced at re-pin (live tip ~136_37x); synchronized
                //     stop-all/start-all deploy, so instant-on if the tip
                //     overtakes it during the rebuild is still fork-safe.
                inc_i_173_activation_height: 25_500, // re-pinned 2026-08-25: 15_087 tied #20 and broke the strict #21 > #20 ordering; measured tip 24_770

                // INC-I-176 M2 maintainer-authorization message binding (#22).
                // Pinned 2026-08-13 at a MEASURED live testnet tip of 154_399
                // (read-only `getChainInfo` against the local testnet on
                // 127.0.0.1): a lead of 145_601 blocks ≈ 16.9 days at
                // SLOT_DURATION = 10s (8_640 blocks/day), i.e. ~2026-08-30.
                // NOT YET IN FORCE (audit AUDIT-P1-102): the gate is UNCROSSED —
                // re-measured tip 156_149 at 2026-08-13T18:47Z — so on testnet the
                // legacy colliding message is still what every node verifies, and
                // AUDIT-P0-011 / AUDIT-P1-016 stay OPEN here until the chain
                // reaches 300_000. M2 ships the mechanism; the height decides when
                // it takes effect, forward-only.
                // NOT 0 — that would reinterpret already-validated testnet
                // history (the real add_maintainer mined at block 136_690, txid
                // 62a3bfbd…) under a message form no archived signature covers.
                // NOT u64::MAX — that would make the binding unreachable on the
                // one network where the governance path is testable.
                // IMMUTABLE once crossed (INC-I-054).
                // Override via `DOLI_INC_I_176_AUTH_BINDING_ACTIVATION_HEIGHT`.
                //
                // The margin is deliberately generous rather than minimal because
                // the cost is asymmetric: too small and the M2
                // MAINTAINER_AUTH_VALID_BEFORE_UNSET sentinel becomes load-bearing
                // in production, forcing M2.5 to take its own height #23; too
                // large costs only a re-pin, which is free while the height is
                // UNCROSSED. The lead is sized so M2.5 (v2 payload emission), M3
                // (the expiry check) and M4 (the signer) all land BEFORE the chain
                // crosses #22.
                //
                // REV-176-M1a-001 ordering, both halves stated:
                //   LOWER  300_000 >= #20 (127_200) ✓ — the chain-bound message
                //     never arrives before the distinct-signer counter.
                //   UPPER  300_000 <= #21 (136_431) ✗ — **UNSATISFIABLE and
                //     ACCEPTED.** #21 was crossed at 136_431 while the tip is
                //     154_399, and a crossed activation height is IMMUTABLE
                //     (INV-PARAMS-001 / INC-I-054), so no value above the tip can
                //     satisfy it and any value below the tip would be retroactive.
                //     The residual — testnet already carries an unbound,
                //     domain-unseparated maintainer authorization at block 136_690
                //     — is accepted because the local testnet runs exclusively on
                //     127.0.0.1 and is unreachable from the internet, so the
                //     AUDIT-P0-011 collision has no remote attacker surface there.
                //     Pinned as a VISIBLE exception by
                //     `rev_176_m1a_001_testnet_upper_half_is_an_accepted_unsatisfiable_residual`.
                inc_i_176_auth_binding_activation_height: 15_087, // re-pinned 2026-08-24 for fresh testnet genesis (tip ~15006); was 300_000. #22>=#20 holds at equality (both 15_087); #22<=#21 holds strictly (#21=25_500).

                // INC-I-172 M2 review F3. Unchanged from the historical
                // hardcoded precondition (INITIAL_MAINTAINER_COUNT = 5): the
                // local testnet runs 12 producers + seeds, so the precondition
                // clears, and keeping 5 makes the testnet seed path
                // byte-identical to M2 as reviewed.
                maintainer_seed_min_producers: crate::maintainer::INITIAL_MAINTAINER_COUNT,

                // INC-I-015: Gossip mesh sized to max_peers for eager push to ALL
                // connected peers. At mesh_n=12, blocks reach 12 peers immediately
                // and the rest via IHAVE (lazy, 1+ heartbeat delay). At 120+ nodes
                // on localhost, this delay exceeds the 10s slot window → fork cascade
                // → CPU saturation → RAM explosion. With mesh_n=max_peers, every
                // connected peer gets eager push: zero multi-hop delay for blocks.
                // Bandwidth cost: 25 peers × 2KB/block × 1 block/10s = 5KB/s. Negligible.
                mesh_n: 25, // = max_peers: all connected peers in mesh
                mesh_n_low: 20,
                mesh_n_high: 50, // = max_peers*2: accept grafts up to total_conn_limit
                gossip_lazy: 25,
            },

            Network::Devnet => NetworkParams {
                // Networking
                default_p2p_port: 50300,
                default_rpc_port: 28500,
                default_metrics_port: 29000,
                bootstrap_nodes: vec![], // No bootstrap for local devnet
                bootnode_enrs: vec![],   // No bootnode for local devnet
                max_peers: 150,          // Devnet: local machine, 100+ nodes stress tests

                // Timing (accelerated for testing)
                slot_duration: consensus::SLOT_DURATION, // Same as mainnet for realistic testing
                genesis_time: 0,                         // Dynamic
                veto_period_secs: 60,                    // 1 minute
                grace_period_secs: 30,                   // 30 seconds
                bootstrap_grace_period_secs: 5,          // 5s for fast devnet startup
                unbonding_period: 60,                    // ~10 minutes with 10s slots
                inactivity_threshold: 30,

                // Economics (lower values for testing)
                bond_unit: 100_000_000,           // 1 DOLI (Devnet override)
                initial_reward: 2_000_000_000,    // 20 DOLI (Devnet override)
                registration_base_fee: 1_000,     // 0.00001 DOLI
                max_registration_fee: 10_000_000, // 0.1 DOLI
                automatic_genesis_bond: 100_000_000, // 1 DOLI (matches devnet bond_unit)
                genesis_blocks: 40,

                // VDF (fast for development)
                vdf_iterations: 1,
                heartbeat_vdf_iterations: 1_000,
                vdf_register_iterations: 1_000,

                // Time structure (accelerated)
                blocks_per_year: 144,       // ~24 minutes
                blocks_per_reward_epoch: 4, // ~40 seconds
                coinbase_maturity: 10,
                slots_per_reward_epoch: 30, // 30 seconds
                bootstrap_blocks: 60,

                // Update system (fast for testing)
                min_voting_age_secs: 60,         // 1 minute
                update_check_interval_secs: 10,  // 10 seconds
                crash_window_secs: 60,           // 1 minute
                max_registrations_per_block: 20, // Higher for rapid testing

                // Presence (telemetry)
                presence_window_ms: consensus::NETWORK_MARGIN_MS,

                // Fallback timing (configurable for devnet)
                fallback_timeout_ms: consensus::FALLBACK_TIMEOUT_MS,
                max_fallback_ranks: consensus::MAX_FALLBACK_RANKS,
                network_margin_ms: consensus::NETWORK_MARGIN_MS,

                // Vesting (fast for devnet testing: 10 min per quarter, 40 min full vest)
                vesting_quarter_slots: 60,

                // Hard fork gates
                sig_verification_height: 0, // Enforce from genesis on devnet
                snap_attestation_skip_height: 0, // Always active on devnet
                // INC-I-026: always active on devnet. Tests run against the fixed
                // scheduler by default — the pre-activation (legacy) path is
                // exercised via a dedicated test that overrides this value.
                inc_i_026_scheduler_activation_height: 0,
                fork_id_activation_height: 0, // Always active on devnet
                // Devnet: all features active from genesis
                full_bitfield_decode_height: 0,
                rewards_epoch_list_fix_height: 0,
                encrypted_content_activation_height: 0,
                encrypted_content_v2_activation_height: 0, // Always active on devnet
                epoch_state_reorg_activation_height: 0,
                security_audit_activation_height: 0, // Always active on devnet
                ghost_exclusion_activation_height: 0, // Always active on devnet
                // INC-I-116: always active on devnet
                epoch_prune_activation_height: 0,
                // INC-I-190 F3: cap-bound the MIN_PRODUCERS_FLOOR fallback.
                // Active from genesis — devnet has no sealed history to stay
                // bit-compatible with.
                inc_i_190_floor_bound_activation_height: 0,
                // INC-I-075: Always active on devnet (clean chain).
                inc_i_068_weight_filter_activation_height: 0,
                // INC-I-078: devnet default disabled (u64::MAX). Tests that
                // exercise cap/auth behavior set these explicitly via override
                // or env vars; default mirrors mainnet so the unit tests for
                // the feature must opt in.
                received_delegation_cap: u64::MAX,
                received_delegation_cap_activation_height: u64::MAX,
                delegation_auth_activation_height: u64::MAX,
                // INC-I-080: devnet default disabled (u64::MAX) — mirrors the
                // INC-I-078 devnet rationale. Cap tests pass explicit
                // activation heights to `check_addbond_cap` and do not depend
                // on this default; existing devnet/test flows stay
                // byte-identical (no surprise enforcement).
                addbond_cap_enforcement_activation_height: u64::MAX,
                withdrawal_holdings_gate_activation_height: 20,
                // INC-I-088 Phase 0: DeFi gate disabled by default on devnet
                // (mirrors mainnet/testnet). Devnet tests that need DeFi
                // either set `DOLI_DEFI_ACTIVATION_HEIGHT=0` in their .env or
                // override `ValidationContext` directly. Existing tx-type
                // unit tests live in the `#[cfg(test)]` modules of the
                // per-type validators and call those functions directly —
                // they do NOT go through `validate_transaction`, so the
                // gate does not affect them.
                defi_activation_height: u64::MAX,

                // AMM Foundations M1: devnet default is 0 (always-on) so
                // local AMM development can submit AMM tx types without
                // an env override. Validation still proceeds through the
                // per-type validator — the gate just opens. AMM apply_block
                // consumer code (separate session) is the next dependency.
                // Independent of defi_activation_height per HC-6.
                amm_activation_height: 0,

                // Phase 2.1 Oracle: frozen by default on devnet. Devnet tests
                // that need the oracle live override via
                // `DOLI_ORACLE_ACTIVATION_HEIGHT` in their .env.
                oracle_activation_height: u64::MAX,

                // Large blocks (>1 MB) → ~300 TPS (INC-I-091). Devnet = 0
                // (always-on for local development). Override via
                // `DOLI_LARGE_BLOCK_ACTIVATION_HEIGHT`.
                large_block_activation_height: 0,

                // INC-I-092 DeFi spend-path fixes: always-on for local
                // development (devnet is ephemeral, no rolling-deploy concern).
                inc_i_092_activation_height: 0,
                // INC-I-096 pool-aware conservation. Always-on in devnet.
                inc_i_096_activation_height: 0,
                // INC-I-147 D6/D4 recovery fixes. Always-on in devnet.
                inc_i_147_activation_height: 0,
                // INC-I-204 M5 single fork-choice authority. Always-on in devnet,
                // mirroring the gate it supersedes. Fork choice is not block
                // content, so 0 is not a genesis reset.
                inc_i_204_fork_choice_activation_height: 0,
                // INC-I-178 M4 attestation-BLS semantics. FROZEN at u64::MAX
                // even here: it changes block CONTENT, so 0 would fork every
                // live local devnet chain on the next rebuild.
                inc_i_178_attestation_bls_activation_height: u64::MAX,
                // INC-I-172 M2 maintainer trust-root derivation.
                // Always active on devnet — fresh genesis each run, no history
                // to reinterpret.
                maintainer_derivation_activation_height: 0,
                // INC-I-173 state-only fee gate. Always active on devnet —
                // fresh genesis each run, no history to reinterpret.
                inc_i_173_activation_height: 0,
                // INC-I-176 M2 maintainer-authorization message binding (#22).
                // 20, NOT 0. User-decided 2026-08-13; see the section "DEVNET
                // EXEMPTION from the `#22 <= #21` half" in
                // `specs/maintainer-authorization-architecture.md`.
                //
                // REV-176-M1a-001 ordering, both halves stated:
                //   LOWER  20 >= #20 (0) ✓ — UNCONDITIONAL on every network. The
                //     chain-bound message never arrives before the distinct-signer
                //     counter, so AUDIT-P1-016's binding is never live while
                //     AUDIT-P0-010's entry-counting defect is re-armed underneath.
                //   UPPER  20 <= #21 (0) ✗ — **EXEMPTED ON DEVNET ONLY.** That half
                //     exists to close the window [#21, #22) in which maintainer
                //     transactions are MINEABLE but UNBOUND. The window is a threat
                //     only to a chain with persistent history and value; devnet has
                //     neither (fresh genesis every run, local-only, no adversary).
                //     Devnet-only — mainnet and testnet stay bound by both halves,
                //     subject to the testnet residual recorded above.
                //
                // Why 20 and not 0, and not u64::MAX:
                //   NOT 0 — the five fenced INC-I-174 node suites operate at block
                //     heights 0-7. At 0 they would be ABOVE the gate and would need
                //     the bound message they do not build: MEASURED 15 failures
                //     across six node suites, with a positive control (u64::MAX ⇒
                //     all 29 green). At 20 they sit entirely below the gate, take
                //     the legacy arm, and pass UNMODIFIED.
                //   NOT u64::MAX — devnet is the only network where the bound arm is
                //     reachable at all (mainnet u64::MAX, testnet 300_000 far above
                //     the tip). Above height 20 the bound arm actually executes, so
                //     M3 and M4 are developed against real code instead of the
                //     legacy arm only.
                // Corollary, recorded because the audit found it overclaimed
                // elsewhere (AUDIT-P1-102): DEVNET IS THE ONLY SURFACE ON WHICH
                // THE AUDIT-P0-011 / AUDIT-P1-016 CLOSURE IS LIVE TODAY. On both
                // live networks the bound arm is unreachable at the current tip,
                // so M2's closure is a mechanism that is wired, not a defect that
                // is already shut.
                // Pinned as a VISIBLE exception by the devnet ordering tests in
                // `crates/core/tests/inc_i_176_m2_ordering.rs`.
                inc_i_176_auth_binding_activation_height: 20,

                // INC-I-172 M2 review F3. `scripts/launch_testnet.sh` boots a
                // TWO-producer devnet. With the historical hardcoded 5 the root
                // never seeded, and because the devnet gate is 0 the F4
                // fail-close then rejected every ProtocolActivation forever
                // while an empty set also refused every AddMaintainer that
                // could have repaired it — an absorbing dead-end on the one
                // network where the update path is testable at all. 2 restores
                // exactly the pre-M2 devnet behavior: a 2-member root with
                // calculate_threshold(2) == 2.
                maintainer_seed_min_producers: 2,

                // Gossip mesh: same universal config as mainnet.
                // With --no-dht, mesh_n_high=24 keeps all devnet peers in mesh.
                mesh_n: 12,
                mesh_n_low: 8,
                mesh_n_high: 24,
                gossip_lazy: 12,
            },
        }
    }
}
