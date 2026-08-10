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
    /// Crash window in seconds for the UNWIRED update watchdog.
    /// NOT a live control — `updater::watchdog` has zero production callers, so no
    /// automatic rollback exists (INC-I-172 AUDIT-P1-014).
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

    /// INC-I-116: Height at which the epoch-boundary liveness prune activates.
    /// Before: proportional 2/3 floor (relative to effective_active) overrides the
    /// attestation filter when many producers are absent.
    /// After: absolute MIN_PRODUCERS_FLOOR=3 replaces the proportional floor. Absent
    /// producers are pruned from the schedule at every epoch boundary, re-included
    /// automatically when they resume attesting (no on-chain transaction needed).
    ///
    /// Three-question gate (INC-I-075):
    ///   Q1: No — no user-submittable transaction triggers this path.
    ///   Q2: YES — attestation pattern determines which producers are scheduled.
    ///   Q3: NO — post-activation, the scheduler produces a DIFFERENT producer_list
    ///       for the same inputs (fewer absent producers included).
    ///   Verdict: activation height REQUIRED.
    pub epoch_prune_activation_height: u64,

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

    /// INC-I-088 Phase 0 / B.1+B.2 update: originally gated all non-AMM
    /// DeFi tx types. All 7 are now tombstoned (5 lending B.1, 2 NFT-frac
    /// B.2). Their discriminants return None from from_u32 and never reach
    /// the validation gate. Field retained for structural compatibility.
    /// Defaults: mainnet/testnet/devnet all `u64::MAX` (disabled).
    pub defi_activation_height: u64,

    /// AMM Foundations M1 (2026-05-25): height at which the 4 AMM tx types
    /// (CreatePool, AddLiquidity, RemoveLiquidity, Swap) become valid for
    /// inclusion.
    ///
    /// Pre-activation: every node REJECTS these txs at validation time with
    /// `ValidationError::AmmNotActivated` (error code `AMM_NOT_ACTIVATED`,
    /// `[ERRTX-AMM001]`, REQ-AGENTIC-ERRORS compliant). Mempool symmetry:
    /// pre-activation admission also rejects, so upgraded producers never
    /// include an AMM tx in their blocks during a rolling deploy.
    ///
    /// Post-activation: the per-type structural validator runs normally.
    ///
    /// HC-6 / spec §0 NEVER constraint: this height MUST remain
    /// INDEPENDENT of `defi_activation_height`, `oracle_activation_height`,
    /// or any other. Never bundle. Never reuse. The AMM subsystem has its
    /// own audit/un-gating timeline distinct from lending + NFT-frac.
    ///
    /// Three-question gate verdict (INC-I-075): Q1=YES (AMM txs are
    /// user-submittable), Q2=NO (validator rejection only), Q3=NO
    /// (accept-then-reject change) → activation height REQUIRED.
    ///
    /// Defaults: mainnet `u64::MAX`, testnet `u64::MAX` (placeholder —
    /// operator pins a concrete height in a separate deploy commit after
    /// the AMM consumer code is fully implemented + audited), devnet `0`
    /// (always-on for local development). Mainnet/testnet IMMUTABILITY
    /// rule (INC-I-054): once crossed on mainnet, never move forward.
    ///
    /// Spec: `specs/defi-foundations-economics.md` §0 D1/D2 (D2 derivation
    /// rule becomes IRREVERSIBLE once this height is ever crossed).
    pub amm_activation_height: u64,

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

    /// Activation height for large (>1 MB) blocks (INC-I-091).
    ///
    /// At/after this height, producers may build blocks up to ~2 MB
    /// (`LARGE_BLOCK_SELECT_BUDGET`), unlocking ~300 TPS. Before it, blocks
    /// stay ~1 MB (`LEGACY_BLOCK_SELECT_BUDGET`) so they fit the legacy 1 MiB
    /// gossip cap and propagate to not-yet-upgraded nodes during a one-by-one
    /// rollout. This is BUILDER POLICY (block content), NOT a validation rule —
    /// validation already accepts up to `max_block_size(height)`. Laggard
    /// nodes that miss a large block on gossip recover it via orphan-chase
    /// over the 16 MB sync path, so they fall behind rather than fork.
    ///
    /// Defaults: mainnet `u64::MAX` (frozen — operator pins a concrete future
    /// height in a SEPARATE commit, only after the fleet carries the raised
    /// gossip cap), testnet `0` (always-on — small controllable fleet, lighter
    /// deploy), devnet `0`. Mainnet IMMUTABILITY rule (INC-I-054): once
    /// crossed, never move forward.
    pub large_block_activation_height: u64,

    /// Activation height for the INC-I-092 DeFi spend-path correctness fixes.
    ///
    /// At/after this height (strict `>=`), three consensus-validation changes
    /// take effect together:
    ///   - RC-A: the Pool UTXO consumed by a `Swap`/`AddLiquidity`/
    ///     `RemoveLiquidity` (input 0) is EXEMPT from signature verification.
    ///     A Pool output is `pubkey_hash = pool_id` (a domain hash, not a key
    ///     hash), so the signature path can never be satisfied — the pool was
    ///     permanently unspendable. Authorization is the AMM invariant
    ///     (`new_k >= old_k` + conservation, already enforced), mirroring the
    ///     ZKRollup proof-as-signature carve-out.
    ///   - RC-B: `CreatePool` must back its declared `reserve_a` with net DOLI
    ///     inputs (closes the `u64::MAX` reserve inflation), reject a duplicate
    ///     `pool_id` at validation (no silent burn), and reject a zero-amount
    ///     creator LPShare.
    ///
    /// Three-question gate (INC-I-075): Q1=YES (Swap/CreatePool are
    /// user-submittable), Q2=NO, Q3=NO (accept↔reject flips both directions)
    /// → activation height REQUIRED. Independent of `amm_activation_height`.
    ///
    /// Defaults: mainnet `u64::MAX` (frozen — AMM itself is not yet activated
    /// on mainnet; the operator pins this in the same window AMM is enabled,
    /// so AMM goes live already-correct). testnet a concrete near-future height
    /// (AMM is LIVE there at h=20099; ~30 external producers → no synchronized
    /// stop-all, so a rolling-deploy gate + lead time is mandatory). devnet `0`
    /// (ephemeral, correct from genesis). Mainnet IMMUTABILITY (INC-I-054):
    /// once crossed, never move forward.
    pub inc_i_092_activation_height: u64,

    /// INC-I-096: Pool-aware value conservation for AMM DOLI-outflow txs.
    ///
    /// At/after this height (strict `>=`), the native conservation check
    /// accounts for Pool reserve deltas (reserves stored in `extra_data`,
    /// invisible to the naive `total_input >= total_output` gate), and
    /// RemoveLiquidity binds reserve-deltas to LP shares burned
    /// (proportional withdrawal). Mempool mirrors the same pool-aware
    /// conservation.
    ///
    /// Without this fix, ANY RemoveLiquidity or B-to-A Swap that releases
    /// DOLI from pool reserves is falsely rejected with InsufficientFunds.
    ///
    /// Three-question gate (INC-I-075): Q1=YES (RemoveLiquidity/Swap are
    /// user-submittable), Q2=NO, Q3=NO (reject->accept for valid txs)
    /// -> activation height REQUIRED. Independent of amm/inc_i_092.
    ///
    /// Defaults: mainnet `u64::MAX` (co-pinned with amm_activation_height
    /// — AMM not live), testnet `u64::MAX` (placeholder — operator pins
    /// concrete height in separate commit), devnet `0` (always-on).
    /// Mainnet IMMUTABILITY (INC-I-054): once crossed, never move forward.
    pub inc_i_096_activation_height: u64,

    /// INC-I-147: fork-choice height-unit correction + rolled-back-block re-apply.
    ///
    /// Gates two node-local recovery defects measured on the 2026-07-31 testnet
    /// reproduction:
    ///
    /// * **D6** — `ReorgHandler`'s `BlockWeight.height` is a PER-PROCESS counter
    ///   (`real_height - init_height`), because `block_weights` is empty at process
    ///   start so the first recorded block always gets height 1. `plan_reorg` then
    ///   compares it against `last_finality_height`, which is a REAL chain height, so
    ///   on any restarted or snap-synced node no reorg can ever be approved. MEASURED:
    ///   the same block at real height 57067 was recorded as 267 by the seed
    ///   (init 56800) and 25897 by n7 (init 31170).
    /// * **D4** — `handle_new_block` short-circuits on a bare
    ///   `block_store.get_block(&hash).is_some()` check. `remove_canonical_entry`
    ///   leaves the body in place, so a rolled-back block is refused forever
    ///   (`status=already_known`, observed 159 times on n7).
    ///
    /// Deploy questions (derived, INC-I-075 / CLAUDE.md):
    /// Q1 consensus RULES — NO on the merits. `ReorgHandler`/`BlockWeight` has zero
    /// symbol reach into validation, apply, block construction, the wire protocol or
    /// the state root; nothing derived from it is gossiped or persisted; and the
    /// corrected predicate is approval-monotone (`H_syn <= H_real` into a
    /// monotone-decreasing rejection test), so it can only convert REJECT->APPROVE.
    /// A quantity that yields two different values on two nodes for the same block
    /// was never a consensus rule. Gated anyway as a rollout-coordination device and
    /// because the checklist applied literally returns "required".
    /// Q2 block CONTENT — NO. The builder never consults the reorg handler
    /// (`production/mod.rs:131` reads `state.best_hash`), so blocks are byte-identical
    /// and no synchronized deploy is needed.
    /// Protocol version — NOT bumped. No `EpochState` format change; an unnecessary
    /// bump would trigger `delete_epoch_state()` on every restart (INC-I-054).
    ///
    /// Defaults — inc_i_147_activation_height: mainnet `129_500` (pinned
    /// 2026-08-05 at live tip 120_799, ~24.2h lead, AMM-pin precedent, v6.24.1),
    /// testnet `80_700` (pinned at tip 80_544, crossed — active), devnet `0`.
    /// Mainnet IMMUTABILITY (INC-I-054): once crossed, never move forward.
    pub inc_i_147_activation_height: u64,

    /// INC-I-172 M2: maintainer trust-root derivation, governance counter and
    /// `ProtocolActivation` fail-close.
    ///
    /// ONE constant gate covering four behaviors that all decide which
    /// governance transactions take effect (full rationale in
    /// `specs/maintainer-trust-root-architecture.md` §F2/§F3/§F4):
    ///
    /// * **F2 one-shot seed** — the root was re-derived from live producer state
    ///   on EVERY applied block, so a successful `RemoveMaintainer` was reverted
    ///   ~10 s later. Above the gate the genesis seed fires only once.
    /// * **F2 canonical derivation** — `all_producers()` is a `HashMap::values()`
    ///   walk and every genesis producer ties at `registered_at == 0`, so a stable
    ///   sort picked a random 5-subset. Above the gate the order is the TOTAL
    ///   order `(registered_at, pubkey_bytes)`.
    /// * **F3 distinct-signer counter** — `verify_multisig` counted signature
    ///   ENTRIES, so three copies of ONE key cleared a "3-of-5" threshold
    ///   (AUDIT-P0-010). Above the gate: distinct signers only.
    /// * **F4 fail-close** — an unusable on-chain root silently reverted
    ///   `ProtocolActivation` to PRODUCER-KEY authority. Above the gate it fails
    ///   closed.
    ///
    /// Deploy questions (INC-I-075 three-question checklist): Q1 **YES**
    /// (`AddMaintainer`/`RemoveMaintainer`/`ProtocolActivation` are all
    /// user-submittable and all four behaviors above change which of them take
    /// effect), Q2 **YES** (producer `registered_at` is an input), Q3 **NO** ⇒
    /// **ACTIVATION HEIGHT REQUIRED**. Block CONTENT unchanged
    /// (tx/coinbase/header shapes untouched) ⇒ no synchronized deploy. Protocol
    /// version NOT bumped (no `EpochState` format change; INC-I-054).
    /// **CONSTANT GATE, never a `HardForkSchedule` entry** — `current_fork_id`
    /// evaluates the schedule at `u64::MAX`, which would activate the entry
    /// immediately and partition a rolling deploy.
    ///
    /// **Precision on Q1 (corrected 2026-08-10, INC-I-172 M2 QA OBS-004).** A
    /// `ProtocolActivation` accept/reject divergence is **NOT** state-root-visible
    /// *today*, so "activation acceptance is consensus-visible" — the reason first
    /// written here — was WRONG as stated. Two facts:
    /// `ChainState::serialize_canonical` (`crates/storage/src/chain_state.rs`) is a
    /// fixed 140-byte buffer that contains neither `active_protocol_version` nor
    /// `pending_protocol_activation`, and `is_protocol_active`
    /// (`crates/core/src/consensus/constants.rs`) has zero production callers, so
    /// `active_protocol_version` currently gates nothing. The gate is kept anyway,
    /// and that is the RIGHT call: `active_protocol_version` exists precisely to be
    /// read by a future consensus rule, and the moment anything reads it the claim
    /// becomes true retroactively over history this gate already governs.
    /// "Currently unused" is never a valid skip (INC-I-075, INV-12).
    ///
    /// Defaults — maintainer_derivation_activation_height: mainnet `172_000`
    /// (pinned 2026-08-10 at live tip 162_727 via `getChainInfo`, ~9_273 blocks
    /// ≈ 25.8 h of lead at 10 s slots, matching the INC-I-147 / AMM pin
    /// precedent), testnet `127_200` (pinned at live tip 126_801, ~400 blocks),
    /// devnet `0`. Because the mainnet height is in the FUTURE at pin time, no
    /// already-executed `ProtocolActivation` is reinterpreted.
    /// Mainnet IMMUTABILITY (INC-I-054): once crossed, never move forward.
    pub maintainer_derivation_activation_height: u64,

    /// INC-I-173: state-only fee/balance exemption derived from ONE exhaustive
    /// `TxType::allows_empty_io()` authority instead of a hand-maintained list.
    ///
    /// The fee gate in `validation/utxo.rs` carried its own 3-type `matches!`
    /// (`Registration | DelegateBond | RevokeDelegation`) that had drifted from
    /// every other "state-only" definition in the tree. `AddMaintainer` and
    /// `RemoveMaintainer` are 0-in/0-out, are admitted to the mempool, are
    /// relayed and have fully implemented apply handlers — but the block builder
    /// skipped them every slot and every node rejected a block containing one.
    /// The governance transactions INC-I-172 exists to make usable could never
    /// be mined.
    ///
    /// At/after this height (strict `>=`) the exemption is
    /// `Transaction::is_zero_flow()` = 0 inputs AND 0 outputs AND
    /// `TxType::allows_empty_io()`, whose true-set is curated by AUTHORIZATION:
    /// `{Registration, DelegateBond, RevokeDelegation, AddMaintainer,
    /// RemoveMaintainer}`. `Exit` and `SlashProducer` share the same wire shape
    /// but their apply handlers authenticate nobody, so they stay excluded
    /// (constraint C1) and are routed to their own incidents. Below the height
    /// the legacy 3-type expression is retained character-identical
    /// (INV-COMPAT-001) so a mixed fleet cannot fork.
    ///
    /// Deploy questions (INC-I-075 three-question checklist): Q1 **YES**
    /// (`AddMaintainer`/`RemoveMaintainer` are user-submittable via RPC
    /// `submitMaintainerChange`), Q2 **YES** (`SlashProducer` is node-generated
    /// on equivocation and reaches the same classification path), Q3 **NO** (a
    /// block containing a 0-fee `AddMaintainer` flips REJECT → ACCEPT) ⇒
    /// **ACTIVATION HEIGHT REQUIRED**. Block CONTENT changes above the gate, so
    /// the height converts a synchronized-deploy requirement into a
    /// fleet-upgrade deadline. Protocol version NOT bumped (no `EpochState`
    /// format change; INC-I-054). **CONSTANT GATE, never a `HardForkSchedule`
    /// entry** — `current_fork_id` evaluates the schedule at `u64::MAX`, which
    /// would activate the entry immediately and partition a rolling deploy.
    ///
    /// Defaults — inc_i_173_activation_height: mainnet `u64::MAX` (fail-closed;
    /// the real value is pinned at release against the live tip plus the
    /// external auto-update window, per the M4 sequencing in the spec), testnet
    /// `133_000` (re-pinned 2026-08-10 at live tip 130_291, 2_709 blocks
    /// ≈ 7.53 h of lead at a measured 10.00 s/block; see the re-pin history in
    /// `defaults.rs`), devnet `0`.
    /// Mainnet IMMUTABILITY (INC-I-054): once crossed, never move forward.
    pub inc_i_173_activation_height: u64,

    /// How many registered producers must exist before the maintainer trust root
    /// is seeded at all (INC-I-172 M2 review F3).
    ///
    /// `Node::maybe_bootstrap_maintainer_set` returns early while
    /// `all_producers().len()` is below this number. It was the hardcoded
    /// constant [`crate::maintainer::INITIAL_MAINTAINER_COUNT`] (5), which is a
    /// SCALE ASSUMPTION with no derivation from observed network size, and on a
    /// network with fewer than five producers it made the trust root
    /// permanently empty. Combined with devnet's
    /// `maintainer_derivation_activation_height == 0` and the F4 fail-close, an
    /// empty root is an ABSORBING state: `ProtocolActivation` fails closed
    /// forever and `AddMaintainer` cannot rescue it either, because
    /// `MaintainerSet::is_authorizable()` is false on an empty set. The repo's
    /// own devnet (`scripts/launch_testnet.sh`) runs TWO producers, so M2 killed
    /// governance there outright.
    ///
    /// This is NOT the seat count: the seed always seats at most
    /// `INITIAL_MAINTAINER_COUNT` keys. It is only the "enough producers to
    /// start" precondition.
    ///
    /// Defaults: mainnet and testnet `INITIAL_MAINTAINER_COUNT` (5) — the
    /// pre-existing value, so both are byte-identical to M2 as reviewed; devnet
    /// `2`, which restores exactly the pre-M2 devnet behavior (a 2-producer
    /// devnet derived a 2-member set with `calculate_threshold(2) == 2`).
    ///
    /// **Not an activation height and not consensus-gated.** It changes only
    /// WHEN a node-local file is first written, and mainnet/testnet keep the old
    /// value, so no pre-activation history is reinterpreted on any live network.
    /// It is deliberately NOT env-overridable: the whole point is that the
    /// assumption is visible per network in one audited place.
    pub maintainer_seed_min_producers: usize,

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

        let params = lock.get_or_init(|| env_loader::load_from_env(network));
        // INV-DEPLOY-002 (INC-I-096): in debug builds, fail fast if a network
        // enables AMM before pool-aware conservation. No effect on release hot path.
        debug_assert!(
            params.validate_amm_conservation_ordering(network).is_ok(),
            "{}",
            params
                .validate_amm_conservation_ordering(network)
                .unwrap_err()
        );
        params
    }

    /// INV-DEPLOY-002 (INC-I-096): AMM must never be validated by the
    /// pre-INC-I-096 (drainable) conservation. For every height where AMM is
    /// active, INC-I-096 pool-aware conservation must also be active — which
    /// holds for all heights iff
    /// `inc_i_096_activation_height <= amm_activation_height`.
    ///
    /// `Network::Testnet` is grandfathered: AMM activated there (h=20_099)
    /// before this fix existed. Below the gate the naive conservation rejects
    /// every DOLI-releasing AMM tx (liveness-broken but NOT drainable, since
    /// the flawed proportional-binding patch was removed), so the historical
    /// ordering violation is safe. Pinning a concrete testnet
    /// `inc_i_096_activation_height` is a separate operator deploy decision.
    ///
    /// This guard's purpose is to block a FUTURE mainnet config that would
    /// enable AMM without the conservation fix.
    pub fn validate_amm_conservation_ordering(&self, network: Network) -> Result<(), String> {
        // AMM disabled on this network → nothing to guard.
        if self.amm_activation_height == u64::MAX {
            return Ok(());
        }
        // Grandfathered historical ordering (see doc) — safe because below-gate
        // conservation rejects (never drains) AMM DOLI-outflow txs.
        if network == Network::Testnet {
            return Ok(());
        }
        if self.inc_i_096_activation_height > self.amm_activation_height {
            return Err(format!(
                "INV-DEPLOY-002 violated on {network:?}: amm_activation_height ({}) \
                 enables AMM before inc_i_096_activation_height ({}) — AMM would run on \
                 pre-INC-I-096 (drainable) conservation. Require \
                 inc_i_096_activation_height <= amm_activation_height.",
                self.amm_activation_height, self.inc_i_096_activation_height
            ));
        }
        Ok(())
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
