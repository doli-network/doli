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
                encrypted_content_v2_activation_height: 100_000, // MIME + royalties — deferred past disaster recovery
                epoch_state_reorg_activation_height: 0,
                // Security audit fixes (2026-04-25): all consensus-breaking fixes activate here
                security_audit_activation_height: 27_547,
                // INC-I-046: Ghost exclusion activates at epoch boundary >= 18152
                ghost_exclusion_activation_height: 18_152,
                // INC-I-075: Re-gate the INC-I-068 weight=0 filter at a future
                // mainnet height so the consensus-shape change activates
                // synchronously instead of unilaterally at deploy time.
                // Pre-H: v6.21.16 behavior (keep weight=0 in active list).
                // Post-H: v6.21.18+ behavior (filter weight=0 out).
                inc_i_068_weight_filter_activation_height: 197_800,

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
                received_delegation_cap_activation_height: 254_344,
                delegation_auth_activation_height: 254_344,
                // INC-I-080: AddBond cap enforcement pinned to h=254_344 —
                // co-deployed atomically with the INC-I-078 bundle (same
                // upgrade event, same lead-time analysis). Above the chain head
                // (F7: NOT retroactive). Pre-254_344 the historical clip path
                // runs; at 254_344 every upgraded node begins rejecting
                // over-cap AddBonds in lockstep. Once crossed this height is
                // IMMUTABLE — never move it forward (INC-I-054).
                addbond_cap_enforcement_activation_height: 254_344,

                // INC-I-088 Phase 0: DeFi subsystems (AMM, lending, loan,
                // fractionalization) gated off on mainnet. u64::MAX = never
                // activated. Operator pins a concrete future height in a
                // separate commit ONLY AFTER the per-type validator gaps
                // are fixed and the subsystems are audit-clean. NEVER lower
                // this without an explicit decision documented in
                // specs/state-of-the-art-architecture.md.
                defi_activation_height: u64::MAX,

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
                // 2026-05-20: all deferred testnet gates pinned to h=272 (fresh
                // genesis dress-rehearsal: chain currently at h≈5, ~45 min lead
                // before co-activation). MIME + royalties.
                encrypted_content_v2_activation_height: 272,
                epoch_state_reorg_activation_height: 0,
                // AUDIT-BRIDGE-001 + AUDIT-AUTH-003: co-activates at h=272.
                security_audit_activation_height: 272,
                // INC-I-046: Ghost exclusion co-activates at h=272.
                ghost_exclusion_activation_height: 272,
                // INC-I-075: Testnet never ran v6.21.16 in production — always
                // apply the INC-I-068 filter (matches current testnet runtime).
                inc_i_068_weight_filter_activation_height: 0,

                // INC-I-078: testnet delegation cap + auth co-activate at h=272
                // (atomic with INC-I-080 AddBond cap) so the testnet transition
                // exercises the same mainnet upgrade event (h=254_344) in one
                // boundary. Cap=3000 matches mainnet; env-overridable for tuning.
                received_delegation_cap: 3000,
                received_delegation_cap_activation_height: 272,
                delegation_auth_activation_height: 272,
                // INC-I-080: AddBond cap co-activates atomically with INC-I-078
                // gates at h=272.
                addbond_cap_enforcement_activation_height: 272,

                // INC-I-088 Phase 0: DeFi gate disabled by default on testnet
                // (mirrors mainnet). Tests that exercise the post-activation
                // path override via the env var
                // `DOLI_DEFI_ACTIVATION_HEIGHT` or via
                // `ValidationContext::with_defi_activation_height`.
                defi_activation_height: u64::MAX,

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
                // INC-I-088 Phase 0: DeFi gate disabled by default on devnet
                // (mirrors mainnet/testnet). Devnet tests that need DeFi
                // either set `DOLI_DEFI_ACTIVATION_HEIGHT=0` in their .env or
                // override `ValidationContext` directly. Existing tx-type
                // unit tests live in the `#[cfg(test)]` modules of the
                // per-type validators and call those functions directly —
                // they do NOT go through `validate_transaction`, so the
                // gate does not affect them.
                defi_activation_height: u64::MAX,
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
