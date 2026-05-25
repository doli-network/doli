use crate::consensus::ConsensusParams;
use crate::network::Network;
use crate::transaction::Output;
use crate::types::BlockHeight;
use crypto::{Hash, PublicKey};

/// Information about an unspent transaction output.
///
/// This is returned by UTXO lookups to provide all information needed
/// for transaction validation.
#[derive(Clone, Debug)]
pub struct UtxoInfo {
    /// The output data.
    pub output: Output,
    /// The public key that can spend this output.
    /// For pay-to-pubkey-hash, this must be provided separately.
    pub pubkey: Option<PublicKey>,
    /// Whether this output has been spent.
    pub spent: bool,
}

/// Trait for UTXO set access during validation.
///
/// Implement this trait to provide the validator with access to the UTXO set
/// for contextual validation (signature verification, balance checks, etc.).
///
/// # Example
///
/// ```rust,ignore
/// struct MyUtxoSet { /* ... */ }
///
/// impl UtxoProvider for MyUtxoSet {
///     fn get_utxo(&self, tx_hash: &Hash, output_index: u32) -> Option<UtxoInfo> {
///         // Look up in database
///     }
/// }
/// ```
pub trait UtxoProvider {
    /// Look up an unspent output.
    ///
    /// Returns `None` if the output doesn't exist or has been spent.
    fn get_utxo(&self, tx_hash: &Hash, output_index: u32) -> Option<UtxoInfo>;
}

/// Registration chain state for anti-Sybil verification.
///
/// Tracks the chained VDF registration state needed to validate
/// that new registrations form a proper chain.
#[derive(Clone, Debug, Default)]
pub struct RegistrationChainState {
    /// Hash of the last registration transaction (Hash::ZERO before any registration)
    pub last_registration_hash: Hash,
    /// Current global registration sequence number
    pub registration_sequence: u64,
}

impl RegistrationChainState {
    /// Create a new registration chain state
    pub fn new(last_hash: Hash, sequence: u64) -> Self {
        Self {
            last_registration_hash: last_hash,
            registration_sequence: sequence,
        }
    }

    /// Get the expected prev_registration_hash for the next registration
    pub fn expected_prev_hash(&self) -> Hash {
        self.last_registration_hash
    }

    /// Get the expected sequence number for the next registration
    pub fn expected_sequence(&self) -> u64 {
        if self.last_registration_hash == Hash::ZERO {
            0 // First registration
        } else {
            self.registration_sequence + 1
        }
    }
}

/// Validation mode for synced blocks.
///
/// Controls whether VDF proof verification is performed during block validation.
/// After snap sync, gap blocks use `Light` mode (VDF already trusted via state root
/// quorum), while recent blocks near the tip use `Full` mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationMode {
    /// Full validation including VDF proof verification.
    /// Used for: gossip blocks, last epoch of sync (360 blocks).
    Full,
    /// Light validation: everything except VDF proof verification.
    /// Used for: gap blocks after snap sync where state root was
    /// already verified by peer quorum (2+ peers).
    Light,
    /// Replay mode for disaster recovery: replays blocks already in the store
    /// through the canonical `apply_block()` path.
    ///
    /// Like `Light`, skips VDF proof verification (blocks are trusted — they
    /// came from the node's own backup).
    ///
    /// Additionally bypasses:
    /// - Block dedup check (blocks ARE in the store; that's intentional)
    /// - Recovery mode gate (replay must proceed regardless)
    /// - Snap sync height guard (replaying from genesis)
    Replay,
}

/// Block validation context.
///
/// Holds all the context needed to validate a block or transaction,
/// including consensus parameters, current time, and chain state.
#[derive(Clone, Debug)]
pub struct ValidationContext {
    /// Consensus parameters (slot duration, block reward, etc.).
    pub params: ConsensusParams,
    /// Network type (determines VDF iterations and other parameters).
    pub network: Network,
    /// Current wall-clock time (unix timestamp).
    pub current_time: u64,
    /// Height of the block being validated.
    pub current_height: BlockHeight,
    /// Slot of the previous block.
    pub prev_slot: u32,
    /// Timestamp of the previous block.
    pub prev_timestamp: u64,
    /// Hash of the previous block (for producer selection).
    pub prev_hash: crypto::Hash,
    /// Active producers for the current epoch (legacy, for backward compatibility).
    pub active_producers: Vec<crypto::PublicKey>,
    /// Active producers with their effective weights for weighted selection (Option C).
    /// If set, weighted selection is used for anti-grinding protection.
    pub active_producers_weighted: Vec<(crypto::PublicKey, u64)>,
    /// Bootstrap producers (sorted by pubkey) for fallback rank validation during bootstrap.
    /// When non-empty, bootstrap validation uses deterministic fallback ranks instead of
    /// accepting any producer. Populated from GSet/known_producers in the node layer.
    pub bootstrap_producers: Vec<crypto::PublicKey>,
    /// Live bootstrap producers (sorted by pubkey) for liveness-filtered scheduling.
    /// Empty = no liveness filter active (use bootstrap_producers for all ranks).
    pub live_bootstrap_producers: Vec<crypto::PublicKey>,
    /// Stale bootstrap producers (sorted by pubkey) for re-entry slot scheduling.
    pub stale_bootstrap_producers: Vec<crypto::PublicKey>,
    /// Registration chain state for chained VDF anti-Sybil verification.
    pub registration_chain: RegistrationChainState,
    /// Public keys with pending (epoch-deferred) registrations.
    /// Used to reject duplicate registrations before epoch activation.
    pub pending_producer_keys: Vec<crypto::PublicKey>,
    /// BLS public keys for active producers, sorted by Ed25519 pubkey (same as bitfield order).
    ///
    /// Index N corresponds to the producer at sorted index N in the bitfield.
    /// Empty Vec at index N means that producer has no BLS key (pre-BLS registration).
    #[allow(dead_code)]
    pub producer_bls_keys: Vec<Vec<u8>>,
    /// Epoch-frozen producer list. The scheduling denominator:
    /// `expected = epoch_producer_list[slot % epoch_producer_list.len()]`.
    /// Never changes mid-epoch.
    pub epoch_producer_list: Vec<crypto::PublicKey>,
    /// Height at which Input.public_key becomes mandatory for signature verification.
    /// Before this height: public_key=None accepted (legacy, no sig verification).
    /// At or after: public_key must be Some, signature + pubkey_hash verified.
    pub sig_verification_height: u64,
    /// Height at which the INC-I-026 scheduler fix activates.
    /// At or after: producer eligibility is `slot % epoch_producer_list.len()`
    /// (pure function, no local state). Default `u64::MAX` = fix disabled.
    /// All deployed networks have activation_height=0 (always active).
    pub inc_i_026_scheduler_activation_height: u64,
    /// Expected fork_id for the current height.
    pub expected_fork_id: crypto::Hash,
    /// Height at which fork_id enforcement activates.
    pub fork_id_activation_height: u64,
    /// Height at which new NFT outputs are rejected in favor of EncryptedContent.
    pub encrypted_content_activation_height: u64,
    /// Height at which EncryptedContent v1 metadata (MIME + royalties) is validated.
    pub encrypted_content_v2_activation_height: u64,
    /// Unified activation height for all consensus-breaking security audit fixes.
    pub security_audit_activation_height: u64,
    /// INC-I-088 Phase 0: height at which the 7 non-AMM DeFi tx types
    /// (CreateLoan, RepayLoan, LiquidateLoan, LendingDeposit,
    /// LendingWithdraw, FractionalizeNft, RedeemNft) become valid for
    /// submission. The 4 AMM tx types (CreatePool, AddLiquidity,
    /// RemoveLiquidity, Swap) were decoupled into
    /// [`Self::amm_activation_height`] per HC-6 / INC-I-075 (AMM
    /// Foundations M1, 2026-05-25). Default `u64::MAX` on all networks =
    /// non-AMM DeFi disabled.
    pub defi_activation_height: u64,
    /// AMM Foundations M1: height at which the 4 AMM tx types
    /// (CreatePool, AddLiquidity, RemoveLiquidity, Swap) become valid for
    /// submission. Strictly `<` gate — at `current_height ==
    /// amm_activation_height` AMM transactions are accepted by the gate.
    /// Sourced from [`crate::network_params::NetworkParams::amm_activation_height`].
    /// Independent of [`Self::defi_activation_height`] (HC-6).
    /// Three-question gate (INC-I-075): Q1=YES (AMM txs are
    /// user-submittable), Q2=NO (validator rejection only),
    /// Q3=NO (accept→reject) → activation height REQUIRED.
    pub amm_activation_height: u64,
    /// Phase 2.1 Oracle: height at which `PriceAttestation` (TxType=16)
    /// transactions become valid for submission. Strictly `<` gate —
    /// at `current_height == oracle_activation_height` attestations are
    /// accepted. Default `u64::MAX` on all networks = oracle disabled
    /// (mirrors `defi_activation_height` pattern). Sourced from
    /// `NetworkParams::oracle_activation_height` (M1, d80f127f).
    /// Three-question gate (INC-I-075): Q1=YES (user-submittable
    /// PriceAttestation tx), Q2=YES (producer-includable in blocks),
    /// Q3=NO (new accept paths) → activation height REQUIRED.
    pub oracle_activation_height: u64,
    /// Phase 2.1 Oracle M8 sunset flag.
    ///
    /// `true` when the most recent epoch boundary's structural-share
    /// metric fell strictly below `SUNSET_THRESHOLD_BPS` (5500 =
    /// 55.00%). Once set, `PriceAttestation` (TxType=16) txs are
    /// rejected with `[ERRTX-ORACLE003]` and the epoch-boundary
    /// aggregator skips the median computation (the last committed
    /// `OraclePrice` UTXO is left in place — readable but stale).
    ///
    /// The orchestrator (`bins/node/src/node/apply_block/oracle.rs`)
    /// maintains the live boolean and the node wires it into every
    /// `ValidationContext` construction site. Recovery requires a
    /// binary upgrade — no on-chain governance can flip it back.
    ///
    /// Default `false`. Pre-activation (the default
    /// `oracle_activation_height = u64::MAX`) this flag is never
    /// set; the M4 validator's height gate fires first and the
    /// sunset gate is unreachable.
    pub oracle_sunset_triggered: bool,
}

impl ValidationContext {
    /// Create a new validation context.
    #[must_use]
    pub fn new(
        params: ConsensusParams,
        network: Network,
        current_time: u64,
        current_height: BlockHeight,
    ) -> Self {
        Self {
            params,
            network,
            current_time,
            current_height,
            prev_slot: 0,
            prev_timestamp: 0,
            prev_hash: crypto::Hash::ZERO,
            active_producers: Vec::new(),
            active_producers_weighted: Vec::new(),
            bootstrap_producers: Vec::new(),
            live_bootstrap_producers: Vec::new(),
            stale_bootstrap_producers: Vec::new(),
            registration_chain: RegistrationChainState::default(),
            pending_producer_keys: Vec::new(),
            producer_bls_keys: Vec::new(),
            epoch_producer_list: Vec::new(),
            sig_verification_height: u64::MAX,
            inc_i_026_scheduler_activation_height: u64::MAX,
            expected_fork_id: crypto::Hash::ZERO,
            fork_id_activation_height: u64::MAX,
            encrypted_content_activation_height: u64::MAX,
            encrypted_content_v2_activation_height: u64::MAX,
            security_audit_activation_height: u64::MAX,
            defi_activation_height: u64::MAX,
            amm_activation_height: u64::MAX,
            oracle_activation_height: u64::MAX,
            oracle_sunset_triggered: false,
        }
    }

    /// Set the encrypted content activation height (see field doc).
    #[must_use]
    pub fn with_encrypted_content_activation_height(mut self, height: u64) -> Self {
        self.encrypted_content_activation_height = height;
        self
    }

    /// Set the EncryptedContent v2 (MIME + royalties) activation height.
    #[must_use]
    pub fn with_encrypted_content_v2_activation_height(mut self, height: u64) -> Self {
        self.encrypted_content_v2_activation_height = height;
        self
    }

    /// Set the security audit activation height (all consensus-breaking audit fixes).
    #[must_use]
    pub fn with_security_audit_activation_height(mut self, height: u64) -> Self {
        self.security_audit_activation_height = height;
        self
    }

    /// Set the DeFi activation height (INC-I-088 Phase 0 safety gate).
    #[must_use]
    pub fn with_defi_activation_height(mut self, height: u64) -> Self {
        self.defi_activation_height = height;
        self
    }

    /// Set the AMM activation height (AMM Foundations M1 gate).
    #[must_use]
    pub fn with_amm_activation_height(mut self, height: u64) -> Self {
        self.amm_activation_height = height;
        self
    }

    /// Set the Phase 2.1 Oracle activation height (PriceAttestation gate).
    #[must_use]
    pub fn with_oracle_activation_height(mut self, height: u64) -> Self {
        self.oracle_activation_height = height;
        self
    }

    /// Set the Phase 2.1 Oracle M8 sunset flag.
    #[must_use]
    pub fn with_oracle_sunset_triggered(mut self, triggered: bool) -> Self {
        self.oracle_sunset_triggered = triggered;
        self
    }

    /// Set the INC-I-026 scheduler activation height (see field doc).
    #[must_use]
    pub fn with_inc_i_026_scheduler_activation_height(mut self, height: u64) -> Self {
        self.inc_i_026_scheduler_activation_height = height;
        self
    }

    /// Set epoch-frozen producer list.
    #[must_use]
    pub fn with_epoch_producer_list(mut self, list: Vec<crypto::PublicKey>) -> Self {
        self.epoch_producer_list = list;
        self
    }

    /// Set previous block info.
    #[must_use]
    pub fn with_prev_block(mut self, slot: u32, timestamp: u64, hash: crypto::Hash) -> Self {
        self.prev_slot = slot;
        self.prev_timestamp = timestamp;
        self.prev_hash = hash;
        self
    }

    /// Set active producers for producer eligibility validation (legacy).
    #[must_use]
    pub fn with_producers(mut self, producers: Vec<crypto::PublicKey>) -> Self {
        self.active_producers = producers;
        self
    }

    /// Set active producers with weights for weighted selection (Option C - anti-grinding).
    ///
    /// When weights are provided, the validation uses weighted producer selection:
    /// - Phase 1: Top N producers by weight are eligible (deterministic, not grindable)
    /// - Phase 2: Hash-based ordering among eligible (limited grinding)
    #[must_use]
    pub fn with_producers_weighted(mut self, producers: Vec<(crypto::PublicKey, u64)>) -> Self {
        // Also populate legacy field for backward compatibility
        self.active_producers = producers.iter().map(|(pk, _)| *pk).collect();
        self.active_producers_weighted = producers;
        self
    }

    /// Set bootstrap producers for fallback rank validation during bootstrap.
    /// Producers must be sorted by pubkey (same order used by production side).
    #[must_use]
    pub fn with_bootstrap_producers(mut self, producers: Vec<crypto::PublicKey>) -> Self {
        self.bootstrap_producers = producers;
        self
    }

    /// Set the liveness split for bootstrap producers.
    /// Both lists must be sorted by pubkey. When `live` is non-empty,
    /// liveness-aware scheduling is used (live for normal slots, stale for re-entry).
    #[must_use]
    pub fn with_bootstrap_liveness(
        mut self,
        live: Vec<crypto::PublicKey>,
        stale: Vec<crypto::PublicKey>,
    ) -> Self {
        self.live_bootstrap_producers = live;
        self.stale_bootstrap_producers = stale;
        self
    }

    /// Set registration chain state for chained VDF validation.
    #[must_use]
    pub fn with_registration_chain(mut self, last_hash: Hash, sequence: u64) -> Self {
        self.registration_chain = RegistrationChainState::new(last_hash, sequence);
        self
    }

    /// Set pending producer keys (epoch-deferred registrations not yet active).
    #[must_use]
    pub fn with_pending_producer_keys(mut self, keys: Vec<crypto::PublicKey>) -> Self {
        self.pending_producer_keys = keys;
        self
    }

    /// Set the sig_verification_height for P0-001 enforcement.
    /// After this height, inputs MUST include their public_key.
    #[must_use]
    pub fn with_sig_verification_height(mut self, height: u64) -> Self {
        self.sig_verification_height = height;
        self
    }

    /// Set fork_id enforcement parameters.
    #[must_use]
    pub fn with_fork_id(mut self, expected: crypto::Hash, activation_height: u64) -> Self {
        self.expected_fork_id = expected;
        self.fork_id_activation_height = activation_height;
        self
    }
}
