//! Network parameters loaded from environment variables
//!
//! This module provides configurable network parameters that can be overridden
//! via `.env` files in the data directory (`~/.doli/{network}/.env`).
//!
//! Security-critical parameters are locked for mainnet to prevent accidental
//! or malicious modification of consensus rules.
//!
//! ## Usage
//!
//! ```ignore
//! use doli_core::network_params::{load_env_for_network, NetworkParams};
//! use doli_core::Network;
//! use std::path::PathBuf;
//!
//! // Load .env file into process environment
//! let data_dir = PathBuf::from("/home/user/.doli/devnet");
//! load_env_for_network("devnet", &data_dir);
//!
//! // Now NetworkParams will read from the loaded environment
//! let params = NetworkParams::load(Network::Devnet);
//! ```

use std::sync::OnceLock;

use crate::Network;

mod chainspec_loader;
mod defaults;
mod env_loader;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_oracle;

pub use chainspec_loader::apply_chainspec_defaults;
pub use env_loader::{get_default_data_dir, init_env_for_network, load_env_for_network};

/// Cached network parameters (one per network)
static MAINNET_PARAMS: OnceLock<NetworkParams> = OnceLock::new();
static TESTNET_PARAMS: OnceLock<NetworkParams> = OnceLock::new();
static DEVNET_PARAMS: OnceLock<NetworkParams> = OnceLock::new();

/// Configurable network parameters
///
/// These parameters can be loaded from environment variables or `.env` files.
/// Default values match the hardcoded constants for backward compatibility.
#[derive(Debug, Clone)]
pub struct NetworkParams {
    // === Networking ===
    /// Default P2P port for this network
    pub default_p2p_port: u16,
    /// Default RPC port for this network
    pub default_rpc_port: u16,
    /// Default metrics port for this network
    pub default_metrics_port: u16,
    /// Bootstrap nodes (multiaddr format)
    pub bootstrap_nodes: Vec<String>,
    /// Discv5 bootnode ENRs (base64-encoded) for UDP peer discovery.
    /// Nodes auto-discover peers via these bootnodes without configuration.
    pub bootnode_enrs: Vec<String>,
    /// Maximum peer connections per node (application layer).
    /// Transport layer allows 1.5× this for handshake headroom.
    /// Mainnet/Testnet: 50 (Ethereum default). Devnet: 150.
    /// Override: DOLI_MAX_PEERS env var.
    pub max_peers: usize,

    // === Timing ===
    /// Slot duration in seconds
    pub slot_duration: u64,
    /// Genesis timestamp (Unix timestamp)
    pub genesis_time: u64,
    /// Veto period for updates in seconds
    pub veto_period_secs: u64,
    /// Grace period after update approval in seconds
    pub grace_period_secs: u64,
    /// Bootstrap grace period in seconds (wait at genesis for chain evidence)
    pub bootstrap_grace_period_secs: u64,
    /// Unbonding period in blocks
    pub unbonding_period: u64,
    /// Inactivity threshold in blocks
    pub inactivity_threshold: u64,

    // === Economics ===
    /// Bond unit size (minimum bond = 1 unit)
    pub bond_unit: u64,
    /// Initial block reward in base units
    pub initial_reward: u64,
    /// Base registration fee in base units
    pub registration_base_fee: u64,
    /// Maximum registration fee cap
    pub max_registration_fee: u64,
    /// Automatic genesis bond amount
    pub automatic_genesis_bond: u64,
    /// Genesis phase duration in blocks
    pub genesis_blocks: u64,

    // === VDF (locked for mainnet) ===
    /// VDF iterations for block production
    pub vdf_iterations: u64,
    /// Heartbeat VDF iterations
    pub heartbeat_vdf_iterations: u64,
    /// VDF iterations for registration proof
    pub vdf_register_iterations: u64,

    // === Time structure ===
    /// Blocks per simulated "year"
    pub blocks_per_year: u64,
    /// Blocks per reward epoch
    pub blocks_per_reward_epoch: u64,
    /// Coinbase maturity (blocks until spendable)
    pub coinbase_maturity: u64,
    /// Slots per reward epoch (legacy)
    pub slots_per_reward_epoch: u32,
    /// Bootstrap blocks count
    pub bootstrap_blocks: u64,

    // === Update system ===
    /// Minimum voting age in seconds
    pub min_voting_age_secs: u64,
    /// Update check interval in seconds
    pub update_check_interval_secs: u64,
    /// Crash window for automatic rollback in seconds
    pub crash_window_secs: u64,
    /// Maximum registrations per block
    pub max_registrations_per_block: u32,

    // === Presence (telemetry) ===
    /// Presence window duration in milliseconds (for telemetry only, does not affect consensus)
    pub presence_window_ms: u64,

    // === Fallback timing ===
    /// Sequential fallback timeout per rank in milliseconds
    pub fallback_timeout_ms: u64,
    /// Maximum fallback ranks per slot
    pub max_fallback_ranks: usize,
    /// Network margin / clock drift tolerance in milliseconds
    pub network_margin_ms: u64,

    // === Vesting (locked for mainnet — consensus critical) ===
    /// Vesting quarter duration in slots (default: 2,160 = 6 hours)
    pub vesting_quarter_slots: u64,

    // === Hard fork gates ===
    /// Height at which Input.public_key becomes mandatory for signature verification.
    /// Before this height: public_key=None accepted (legacy, no sig verification).
    /// At or after: public_key must be Some, signature + pubkey_hash verified.
    /// Mainnet: u64::MAX (not yet activated). Testnet/Devnet: 0 (always enforce).
    pub sig_verification_height: u64,

    /// Height at which post-snap attestation skip becomes active (INC-I-010).
    /// Before this height: epoch boundary always filters by attestation (old behavior).
    /// At or after: skip attestation filtering when block history is incomplete.
    /// Mainnet: 8500. Testnet/Devnet: 0 (always active).
    pub snap_attestation_skip_height: u64,

    /// Height at which the INC-I-026 scheduler fix activates.
    ///
    /// At or after this height: `expected = epoch_producer_list[slot % len()]`
    /// (pure function, identical on all nodes). All deployed networks use 0
    /// (always active).
    ///
    /// Per-network defaults:
    ///   Mainnet: `u64::MAX` — NOT activated. When this branch is merged to main
    ///     the value must be replaced with a chosen future height (operator pick)
    ///     before release. Setting it to any real height before merge would be a
    ///     silent consensus change for anyone running a main-branch binary.
    ///   Testnet: 17000 — activates ~17000 blocks post-genesis on the local testnet.
    ///   Devnet: 0 — always active (new behavior is the default in devnet tests).
    ///
    /// Consensus-breaking at the activation height — all nodes must run a binary
    /// containing this field set to the same value before the height arrives.
    pub inc_i_026_scheduler_activation_height: u64,

    /// Height at which fork_id enforcement activates in block headers.
    /// Before this height: fork_id = Hash::ZERO, not checked.
    /// At or after: fork_id must match local computation, blocks with mismatch are dropped.
    /// Mainnet/Testnet: set to a future height before release. Devnet: 0 (always active).
    pub fork_id_activation_height: u64,

    /// Height at which bitfield decoder switches to [base | extra sorted] (full decode).
    /// Before: only epoch_state.producer_list indices decoded (base).
    /// After: ALL indices including mid-epoch activated producers (base + extra).
    /// Mainnet: 14,000. Testnet/Devnet: 0 (active from genesis).
    pub full_bitfield_decode_height: u64,

    /// Height at which epoch reward bitfield uses epoch_state.producer_list for decoding.
    /// Before: decodes with active_producers_at_height (all active, broken indices).
    /// After: decodes with epoch_state.producer_list (attestation-filtered, same as encoder).
    /// Mainnet: 13,320. Testnet/Devnet: 0 (active from genesis).
    pub rewards_epoch_list_fix_height: u64,

    /// Height at which new NFT outputs are rejected in favor of EncryptedContent.
    /// Before: OutputType::NFT accepted. After: only EncryptedContent accepted.
    /// Mainnet: 37,500. Testnet/Devnet: 0 (active from genesis).
    pub encrypted_content_activation_height: u64,

    /// INC-I-040: Height at which execute_reorg restores epoch_state from undo data.
    /// Before: reorg skips epoch_state → stale attestation accumulators → wrong scheduling.
    /// After: reorg restores epoch_state (matching rollback_one_block behavior).
    /// Mainnet: 44,246. Testnet/Devnet: 0 (active from genesis).
    pub epoch_state_reorg_activation_height: u64,

    /// Unified activation height for all consensus-breaking security audit fixes
    /// from `docs/audits/audit-security-2026-04-24.md`.
    ///
    /// Gates AUDIT-BRIDGE-001 (HTLC signed refund), AUDIT-NFT-001 (EncryptedContent),
    /// AUDIT-REWARD-003 (bond snapshot), AUDIT-PROD-001 (selection weight),
    /// AUDIT-PROD-002 (self-delegation), AUDIT-PROD-003 (delegation cleanup).
    ///
    /// Mainnet: 27,547. Testnet: 21,450. Devnet: 0 (always active).
    pub security_audit_activation_height: u64,

    /// Height at which EncryptedContent v1 metadata (MIME + royalties) activates.
    /// Before: only v0 layout accepted. After: v1 extension fields validated.
    /// Mainnet: 71,290. Testnet: 20,690. Devnet: 0.
    pub encrypted_content_v2_activation_height: u64,

    /// INC-I-046: Height at which ghost producer exclusion activates.
    /// Before: permanently-offline producers inflate the 2/3 deadlock safety floor.
    /// After: producers absent from ALL 3 attestation lookback epochs AND registered
    /// for > GHOST_EXCLUSION_GRACE_EPOCHS are excluded from the floor denominator.
    /// Mainnet: u64::MAX (not yet activated). Testnet: 10,830. Devnet: 0.
    pub ghost_exclusion_activation_height: u64,

    /// INC-I-075: Height at which the INC-I-068 weight=0 filter activates.
    ///
    /// Before this height: fully-delegated producers (`selection_weight == 0`)
    /// REMAIN in `active_producers` and the bond snapshot — matches the
    /// v6.21.16 behavior that mainnet ran before the INC-I-068 deploy.
    /// At or after this height: weight=0 producers are filtered out — matches
    /// the current v6.21.18+ behavior.
    ///
    /// INC-I-068 changed `active_producers` derivation without an activation
    /// height. When the first DelegateBond activated on mainnet (E522), the
    /// rolling-deploy mixed cohort disagreed on `active_list.len()`, shifting
    /// the round-robin scheduler denominator. Different nodes produced
    /// different blocks at the same slot → epoch-boundary fragmentation
    /// cascade (INC-I-075). This gate restores deterministic behavior across
    /// binary versions until every node reaches the activation height.
    ///
    /// Consensus-shape change — ALL nodes must run a binary containing the
    /// same value for this field before the height arrives.
    ///
    /// Mainnet: 197_800. Testnet: 0 (always active; testnet never ran the
    /// affected v6.21.16 in production). Devnet: 0 (clean chain).
    pub inc_i_068_weight_filter_activation_height: u64,

    /// INC-I-078: Maximum total received delegated bonds per producer.
    ///
    /// `0` (or `u64::MAX`) = no cap. Any non-zero value is the inclusive upper
    /// bound on the sum of `received_delegations[*].1` for a single producer.
    /// Bounds delegation concentration without changing slot scheduling.
    ///
    /// Enforced as a height-gated rule via
    /// `received_delegation_cap_activation_height`. Pre-activation: cap is not
    /// checked (matches v6.21.x behavior — there is no limit beyond the global
    /// `MAX_BONDS_PER_PRODUCER`). Post-activation: a DelegateBond whose
    /// `bond_count` would push the target's total over the cap is rejected
    /// (primary check at block-apply, defensive check at epoch-apply).
    ///
    /// Migration is grandfathered: producers already above the cap at
    /// activation height are NOT forced to shed delegations; they simply
    /// cannot receive additional ones. See `specs/delegation-architecture.md`
    /// §2.5 (Option A).
    pub received_delegation_cap: u64,

    /// INC-I-078: Height at which `received_delegation_cap` enforcement begins.
    ///
    /// Pre-activation: cap is not checked (matches current mainnet behavior).
    /// Post-activation: DelegateBond is rejected if it would push the target
    /// producer's `received_delegations` sum above the cap.
    ///
    /// Three-question gate verdict: Q1=YES (DelegateBond is user-submittable),
    /// Q3=NO (new rejections of previously-valid transactions) → activation
    /// height REQUIRED (INC-I-075 protocol). Once crossed, this height is
    /// immutable.
    ///
    /// Defaults to `u64::MAX` everywhere; the operator picks a concrete future
    /// height before deployment. See `specs/delegation-architecture.md` §2.6 + §8.1.
    pub received_delegation_cap_activation_height: u64,

    /// INC-I-078: Height at which DelegateBond / RevokeDelegation Ed25519
    /// signature verification begins.
    ///
    /// Pre-activation: both transaction types accept the legacy on-wire layout
    /// (no signature field; zero-input txs are accepted from any submitter).
    /// Post-activation: both transactions MUST carry a valid Ed25519 signature
    /// from the delegator over a fixed commitment:
    ///   DelegateBond:      HASH("DELEGATE_BOND" || delegate || bond_count)
    ///   RevokeDelegation:  HASH("REVOKE_DELEGATION" || delegate)
    ///
    /// Closes the live forgery exploit (FM-1): without authentication, anyone
    /// who can put a transaction into a block could forge a DelegateBond /
    /// RevokeDelegation on behalf of any producer (the txs have zero inputs
    /// and zero signature today). See `specs/delegation-architecture.md` §7.3.
    ///
    /// Wire format is backward-compatible: the signature is appended to the
    /// existing `extra_data` byte layout, NOT a new tx type number (F3
    /// compliance). Old nodes will accept and store blocks containing the new
    /// field; validation only kicks in for nodes past the activation height.
    ///
    /// Three-question gate verdict: Q1=YES, Q3=NO → activation height REQUIRED.
    /// Once crossed, this height is immutable.
    ///
    /// Defaults to `u64::MAX` everywhere; operator picks a concrete future
    /// height before deployment. The cap and auth heights can co-deploy at the
    /// same value to ship the bundle atomically.
    pub delegation_auth_activation_height: u64,

    /// INC-I-080: Height at which the per-producer AddBond cap is enforced.
    ///
    /// Pre-activation: behavior is UNCHANGED — an AddBond that would push the
    /// producer past `MAX_BONDS_PER_PRODUCER` is silently clipped at epoch
    /// flush (`ProducerInfo::add_bonds`) and the excess Bond UTXOs are
    /// orphaned (the historical bug; preserved for replay safety on old
    /// blocks).
    ///
    /// Post-activation: such an AddBond is REJECTED at block-apply
    /// (`validation::check_addbond_cap` → `ValidationError::AddBondCapExceeded`),
    /// so the carrying block is invalid fleet-wide and no Bond UTXOs are ever
    /// created. Unlike the INC-I-078 DelegateBond cap (skip-in-block), AddBond
    /// must reject because the Bond outputs are real and a skip would still
    /// orphan them.
    ///
    /// Three-question gate verdict (INC-I-075): Q1=YES (AddBond is
    /// user-submittable), Q2=YES (producer-action triggered), Q3=NO (new
    /// rejection of previously accepted-then-clipped txs) → activation height
    /// REQUIRED. Once crossed, this height is immutable. No
    /// `CURRENT_PROTOCOL_VERSION` bump (EpochState unchanged); no
    /// `HardForkSchedule` entry (pure validation rule); rolling-deploy safe.
    ///
    /// Defaults: mainnet `u64::MAX` (operator pins a concrete future height in
    /// a separate commit), testnet `0` (active from genesis), devnet
    /// `u64::MAX` (disabled; cap tests opt in via explicit args — mirrors the
    /// INC-I-078 devnet default).
    pub addbond_cap_enforcement_activation_height: u64,

    /// INC-I-088 Phase 0: Height at which the 11 DeFi tx types
    /// (CreatePool, AddLiquidity, RemoveLiquidity, Swap, CreateLoan,
    /// RepayLoan, LiquidateLoan, LendingDeposit, LendingWithdraw,
    /// FractionalizeNft, RedeemNft) become valid for inclusion.
    ///
    /// Pre-activation: every node REJECTS these txs at validation time with
    /// `ValidationError::DefiNotActivated` (error code `DEFI_NOT_ACTIVATED`,
    /// REQ-AGENTIC-ERRORS compliant). Mempool symmetry: pre-activation
    /// admission also rejects, so upgraded producers never include a DeFi
    /// tx in their blocks during a rolling deploy.
    ///
    /// Post-activation: the per-type structural validator runs normally.
    /// The DeFi subsystems themselves have known semantic gaps
    /// (`LiquidateLoan` has no oracle, `validate_create_loan` does not pin
    /// `Collateral.pubkey_hash` to the derived loan address) — un-gating is
    /// a separate, post-fix decision and MUST NOT be done by simply
    /// lowering this height.
    ///
    /// Companion control: `OutputType::Collateral` is in
    /// `is_conditioned()`, which freezes any pre-existing Collateral UTXO
    /// regardless of the height of the spending tx. The two controls
    /// together fully isolate the lending subsystem.
    ///
    /// Three-question gate verdict (INC-I-075): Q1=YES (DeFi txs are
    /// user-submittable), Q2=NO (validator rejection only), Q3=NO
    /// (accept-then-reject change) → activation height REQUIRED.
    ///
    /// Defaults: mainnet/testnet/devnet all `u64::MAX` (disabled). Operator
    /// pins a concrete future height in a separate commit, and only after
    /// the lending/AMM gaps are closed.
    pub defi_activation_height: u64,

    /// Activation height for the Phase 2.1 structural-anchored oracle.
    /// At-or-after this height, `PriceAttestation` transactions (TxType=16)
    /// become valid for inclusion, the bond-weighted median aggregation
    /// runs at epoch boundary, and the `OraclePrice` UTXO (OutputType=15)
    /// is consumed-and-recreated as a system singleton per asset pair.
    ///
    /// Pre-activation: every node REJECTS PriceAttestation txs at
    /// validation time with `[ERRTX-ORACLE001]` (REQ-AGENTIC-ERRORS
    /// compliant). Mempool symmetry: pre-activation admission also
    /// rejects, so upgraded producers never include an attestation tx
    /// in their blocks during a rolling deploy.
    ///
    /// Post-activation: each active bonded producer MAY submit at most
    /// ONE attestation per epoch per pair. At the epoch-boundary block,
    /// `apply_block()` aggregates by bond-weighted median and writes the
    /// new `OraclePrice` UTXO at the deterministic system address
    /// `BLAKE3("ORACLE_PRICE" || pair_id)`. A sunset trigger HALTs new
    /// attestations when structural bond share (1-epoch lagged,
    /// anti-dilution) drops below 55%.
    ///
    /// Three-question gate verdict (INC-I-075): Q1=YES (PriceAttestation
    /// is user-submittable), Q2=YES (proposer-inclusion + epoch-boundary
    /// aggregation), Q3=NO (new validation/state) → activation height
    /// REQUIRED.
    ///
    /// HC-6 / spec §0 NEVER constraint: this height MUST remain
    /// INDEPENDENT of `defi_activation_height`, `amm_activation_height`,
    /// or any other. Never bundle. Never reuse.
    ///
    /// Defaults: mainnet/testnet/devnet all `u64::MAX` (frozen). Operator
    /// pins a concrete future height in a separate commit, ONLY AFTER
    /// the oracle subsystem is fully implemented (M2-M11), tested on
    /// testnet, and audited.
    ///
    /// Spec: `specs/oracle-structural-anchored-economics.md` §1.10.
    pub oracle_activation_height: u64,

    // === Gossip mesh ===
    /// Target number of peers in gossipsub mesh per topic
    pub mesh_n: usize,
    /// Minimum peers in gossipsub mesh before requesting more
    pub mesh_n_low: usize,
    /// Maximum peers in gossipsub mesh before pruning
    pub mesh_n_high: usize,
    /// Number of peers to lazily gossip IHAVE messages to
    pub gossip_lazy: usize,
}

impl NetworkParams {
    /// Load network parameters from environment variables
    ///
    /// Parameters are loaded from:
    /// 1. Process environment variables
    /// 2. `.env` file in the data directory (if it exists)
    ///
    /// Missing parameters fall back to network defaults.
    pub fn load(network: Network) -> &'static NetworkParams {
        let lock = match network {
            Network::Mainnet => &MAINNET_PARAMS,
            Network::Testnet => &TESTNET_PARAMS,
            Network::Devnet => &DEVNET_PARAMS,
        };

        lock.get_or_init(|| env_loader::load_from_env(network))
    }

    // === Derived parameters ===

    /// Get blocks per month (1/12 of blocks per year)
    pub fn blocks_per_month(&self) -> u64 {
        self.blocks_per_year / 12
    }

    /// Get blocks per era (4 years)
    pub fn blocks_per_era(&self) -> u64 {
        self.blocks_per_year * 4
    }

    /// Get commitment period (same as era)
    pub fn commitment_period(&self) -> u64 {
        self.blocks_per_era()
    }

    /// Get exit history retention (8 years)
    pub fn exit_history_retention(&self) -> u64 {
        self.blocks_per_era() * 2
    }

    /// Get seniority maturity in blocks (4 years)
    pub fn seniority_maturity_blocks(&self) -> u64 {
        self.blocks_per_year * 4
    }

    /// Get seniority step blocks (1 year)
    pub fn seniority_step_blocks(&self) -> u64 {
        self.blocks_per_year
    }

    /// Get minimum voting age in blocks
    pub fn min_voting_age_blocks(&self) -> u64 {
        self.min_voting_age_secs / self.slot_duration
    }

    /// Get veto period in blocks
    pub fn veto_period_blocks(&self) -> u64 {
        self.veto_period_secs / self.slot_duration
    }
}
