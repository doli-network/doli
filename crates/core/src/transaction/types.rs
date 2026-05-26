use crypto::{Hash, Signature};
use serde::{Deserialize, Serialize};

/// Transaction type
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u32)]
pub enum TxType {
    /// Regular transfer transaction
    Transfer = 0,
    /// Producer registration transaction
    Registration = 1,
    /// Producer exit transaction (starts unbonding period)
    Exit = 2,
    /// Claim accumulated rewards
    ClaimReward = 3,
    /// Claim bond after unbonding period completes
    ClaimBond = 4,
    /// Slash a misbehaving producer (with evidence)
    SlashProducer = 5,
    /// Coinbase transaction (block reward to producer)
    Coinbase = 6,
    /// Add bonds to increase stake (bond stacking)
    AddBond = 7,
    /// Request withdrawal of bonds (instant, with vesting penalty)
    RequestWithdrawal = 8,
    /// Reserved — DO NOT REUSE. Tombstone for wire compat (was ClaimWithdrawal).
    ClaimWithdrawal = 9,
    /// Epoch reward transaction (automatic weighted presence rewards at epoch boundary)
    ///
    /// This is the primary reward mechanism. At each epoch boundary, rewards are
    /// automatically distributed to all producers based on their weighted presence:
    /// - reward = Σ(block_reward × producer_weight / total_present_weight)
    /// - No manual claim needed - rewards go directly to producer wallets
    EpochReward = 10,
    /// Remove a maintainer from the auto-update system
    ///
    /// Requires 3/5 signatures from OTHER maintainers (target cannot sign own removal).
    /// Cannot reduce maintainer count below MIN_MAINTAINERS (3).
    RemoveMaintainer = 11,
    /// Add a new maintainer to the auto-update system
    ///
    /// Requires 3/5 signatures from current maintainers.
    /// Target must be a registered producer.
    /// Cannot exceed MAX_MAINTAINERS (5).
    AddMaintainer = 12,
    /// Delegate bond weight to a Tier 1/2 validator.
    ///
    /// The delegate receives the staker's weight for selection purposes.
    /// Rewards are split: delegate keeps DELEGATE_REWARD_PCT (10%),
    /// stakers receive STAKER_REWARD_PCT (90%).
    DelegateBond = 13,
    /// Revoke delegation (DELEGATION_UNBONDING_SLOTS delay applies).
    RevokeDelegation = 14,
    /// On-chain protocol activation (3/5 maintainer multisig).
    ///
    /// Schedules new consensus rules to activate at a future epoch boundary.
    /// All nodes switch simultaneously — deterministic, zero coordination.
    ProtocolActivation = 15,
    /// Phase 2.1 oracle price attestation.
    ///
    /// Submitted by a bonded producer (the "attester") containing a price
    /// observation for an asset pair, scoped to a single epoch. At the
    /// epoch-boundary block, all valid attestations are aggregated by
    /// bond-weighted median into the per-pair `OraclePrice` UTXO
    /// (OutputType=15, introduced in M5).
    ///
    /// Payload (144 bytes, stored in `extra_data`): see
    /// [`PriceAttestationData`](crate::transaction::data::PriceAttestationData).
    ///
    /// Inputs and outputs are EMPTY (data-only tx, same pattern as
    /// `DelegateBond`, `RemoveMaintainer`, `ProtocolActivation`).
    ///
    /// Gated by `oracle_activation_height` (NetworkParams, M1 d80f127f).
    /// Pre-activation: every node REJECTS with `[ERRTX-ORACLE001]` (M4).
    ///
    /// Spec: `specs/oracle-structural-anchored-economics.md` §1.1.
    PriceAttestation = 16,
    /// Mint new units of a fungible asset (issuer-only, requires matching asset_id).
    MintAsset = 17,
    /// Burn units of a fungible asset (holder burns own tokens, provably destroyed).
    BurnAsset = 18,
    /// Create a new AMM pool with initial liquidity
    CreatePool = 19,
    /// Add liquidity to an existing pool
    AddLiquidity = 20,
    /// Remove liquidity from a pool (burn LP shares)
    RemoveLiquidity = 21,
    /// Swap assets through a pool
    Swap = 22,
    // ─── TOMBSTONED: discriminants 24-28 (native lending subsystem) ───
    // B.1 DeFi L1 Foundations Architecture (2026-05-26).
    // These discriminants are PERMANENTLY RETIRED. DO NOT REUSE.
    //
    //   24 = CreateLoan      (tombstoned)
    //   25 = RepayLoan       (tombstoned)
    //   26 = LiquidateLoan   (tombstoned)
    //   27 = LendingDeposit  (tombstoned)
    //   28 = LendingWithdraw (tombstoned)
    //
    // The native lending subsystem was removed because:
    // - defi_activation_height = u64::MAX since inception (never activated)
    // - No lending UTXOs exist on any chain (devnet/testnet/mainnet)
    // - Bilateral lending is handled by the escrow-loan covenant template
    //   (AmountGuard + RecipientGuard + Timelock on standard outputs)
    // - Pooled lending belongs on L2 (Aave-on-L1 anti-pattern)
    //
    // See specs/defi-l1-foundations-architecture.md B.1 for full rationale.
    // ──────────────────────────────────────────────────────────────────
    /// Lock an NFT and mint fungible fraction tokens
    FractionalizeNft = 29,
    /// Burn all fraction tokens and unlock the original NFT
    RedeemNft = 30,
    /// L2 settlement — verify a zero-knowledge proof of an L2 state transition.
    ///
    /// Consumes exactly one `ZKRollup` output (previous committed state) and
    /// produces exactly one `ZKRollup` output (new committed state). Optional
    /// `Normal` outputs are allowed (fees, change, L2→L1 withdrawals justified
    /// by the proof).
    ///
    /// L1's only job: call `verify_zk_proof(verifying_key, prev_root, next_root, proof)`.
    /// If valid, the settlement commits atomically via the UTXO model. No fraud window.
    ///
    /// Gated by `ZK_SETTLE_ACTIVATION_HEIGHT` — set to `u64::MAX` until a
    /// `ProtocolActivation` tx schedules activation at a future epoch.
    ///
    /// See `specs/l2-settlement.md` for the full interface specification.
    ZKSettle = 31,
}

impl TxType {
    pub fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::Transfer),
            1 => Some(Self::Registration),
            2 => Some(Self::Exit),
            3 => Some(Self::ClaimReward),
            4 => Some(Self::ClaimBond),
            5 => Some(Self::SlashProducer),
            6 => Some(Self::Coinbase),
            7 => Some(Self::AddBond),
            8 => Some(Self::RequestWithdrawal),
            9 => Some(Self::ClaimWithdrawal),
            10 => Some(Self::EpochReward),
            11 => Some(Self::RemoveMaintainer),
            12 => Some(Self::AddMaintainer),
            13 => Some(Self::DelegateBond),
            14 => Some(Self::RevokeDelegation),
            15 => Some(Self::ProtocolActivation),
            16 => Some(Self::PriceAttestation),
            17 => Some(Self::MintAsset),
            18 => Some(Self::BurnAsset),
            19 => Some(Self::CreatePool),
            20 => Some(Self::AddLiquidity),
            21 => Some(Self::RemoveLiquidity),
            22 => Some(Self::Swap),
            // ── TOMBSTONED (B.1): DO NOT REUSE discriminants 24-28 ──
            // Native lending removed 2026-05-26. See comment block above.
            24..=28 => None,
            29 => Some(Self::FractionalizeNft),
            30 => Some(Self::RedeemNft),
            31 => Some(Self::ZKSettle),
            _ => None,
        }
    }
}

/// Output type
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum OutputType {
    /// Normal spendable output (default: single signature)
    Normal = 0,
    /// Bond output (time-locked, protocol-governed withdrawal)
    Bond = 1,
    /// Multisig output (threshold-of-N signatures, also used for escrow)
    Multisig = 2,
    /// Hashlock output (requires preimage reveal)
    Hashlock = 3,
    /// HTLC output (hashlock + timelock OR expiry refund)
    HTLC = 4,
    /// Vesting output (signature + timelock)
    Vesting = 5,
    /// NFT output (non-fungible token with metadata + covenant conditions)
    NFT = 6,
    /// Fungible asset output (user-issued token with fixed supply)
    FungibleAsset = 7,
    /// Bridge HTLC output (cross-chain atomic swap with target chain metadata)
    BridgeHTLC = 8,
    /// AMM pool output (reserves + TWAP state)
    Pool = 9,
    /// Liquidity provider share (transferable)
    LPShare = 10,
    // ─── TOMBSTONED: discriminants 11-12 (native lending outputs) ───
    // B.1 DeFi L1 Foundations Architecture (2026-05-26).
    // These discriminants are PERMANENTLY RETIRED. DO NOT REUSE.
    //
    //   11 = Collateral      (tombstoned — was locked loan collateral)
    //   12 = LendingDeposit  (tombstoned — was lending pool deposit receipt)
    //
    // See specs/defi-l1-foundations-architecture.md B.1 for full rationale.
    // ──────────────────────────────────────────────────────────────────
    /// L2 rollup committed state (verifying_key + state_root in extra_data).
    ///
    /// Holds `amount = 0`. Consumable only by a `ZKSettle` tx with a valid
    /// zero-knowledge proof. Each rollup is its own trust domain — the
    /// verifying_key lives in the UTXO, not in a maintainer-governed registry.
    /// Permissionless by construction.
    ///
    /// See `specs/l2-settlement.md` §4.1 for the extra_data layout.
    ZKRollup = 13,
    /// Encrypted content (privacy-first NFT replacement).
    ///
    /// Content is AES-256-GCM encrypted with a unique symmetric key.
    /// The key is ECIES-wrapped with the owner's public key.
    /// Only the owner can decrypt. Transfer re-wraps the key for new owner.
    /// Publication is off-chain (owner shares key externally if desired).
    ///
    /// extra_data layout: [ciphertext_len(4 LE) | ciphertext | wrapped_key(80) |
    ///                     nonce(12) | content_hash(32)]
    EncryptedContent = 14,
    /// Phase 2.1 oracle aggregated price (system-only UTXO).
    ///
    /// Per-pair singleton UTXO created by `apply_block` at the epoch
    /// boundary (Phase 2.1 Oracle M6 aggregator). Holds the
    /// bond-weighted median of all valid `PriceAttestation` txs that
    /// landed in the closing epoch. User transactions cannot create or
    /// spend `OraclePrice` outputs — the validation arm hard-rejects
    /// any user attempt to mint one with `[ERRTX-ORACLE004]`.
    ///
    /// extra_data layout (50 bytes, fixed):
    ///   offset  0,  u64 LE       price_cents          (last aggregated)
    ///   offset  8,  u64 LE       last_update_height   (epoch boundary)
    ///   offset 16,  u16 LE       contributor_count    (attesters aggregated)
    ///   offset 18,  [u8; 32]     pair_id              (asset pair id)
    ///
    /// Deterministic address: `BLAKE3("ORACLE_PRICE" || pair_id)` — see
    /// [`Output::oracle_price_address`]. Mirrors the REWARD_POOL
    /// pattern (`consensus::reward_pool_address`).
    ///
    /// `amount` is set to 0 (price is in `extra_data`; no DOLI is
    /// locked). `is_native_amount = false`, `is_conditioned = false`
    /// — the spend path is exclusively `apply_block` at the next
    /// epoch boundary, never via signature/condition evaluation.
    ///
    /// Snap-sync: included in the state root via standard UtxoSet
    /// canonical serialization (`UtxoEntry::serialize_canonical_bytes`)
    /// — `output_type as u8 == 15` lands in the hash by the same path
    /// every other variant takes.
    ///
    /// Spec: `specs/oracle-structural-anchored-economics.md` §1.2.
    OraclePrice = 15,
}

impl OutputType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Normal),
            1 => Some(Self::Bond),
            2 => Some(Self::Multisig),
            3 => Some(Self::Hashlock),
            4 => Some(Self::HTLC),
            5 => Some(Self::Vesting),
            6 => Some(Self::NFT),
            7 => Some(Self::FungibleAsset),
            8 => Some(Self::BridgeHTLC),
            9 => Some(Self::Pool),
            10 => Some(Self::LPShare),
            // ── TOMBSTONED (B.1): DO NOT REUSE discriminants 11-12 ──
            // Native lending outputs removed 2026-05-26.
            11 | 12 => None,
            13 => Some(Self::ZKRollup),
            14 => Some(Self::EncryptedContent),
            15 => Some(Self::OraclePrice),
            _ => None,
        }
    }

    /// Returns true if this output type uses covenant conditions in extra_data.
    ///
    /// Conditioned outputs have `extra_data` laid out as
    /// `[condition_bytes][type-specific metadata]`. When spent, the node
    /// decodes the condition prefix via `Condition::decode_prefix` and
    /// evaluates it against the witness in `tx.extra_data`.
    ///
    /// NOTE: EncryptedContent is NOT conditioned — its extra_data layout is
    /// `[ciphertext_len | ciphertext | wrapped_key | nonce | content_hash]`,
    /// NOT a condition-prefixed encoding. It uses standard signature verification
    /// on pubkey_hash, same as Normal outputs.
    /// (Fix for AUDIT-NFT-001: EncryptedContent was incorrectly listed here,
    /// causing verify_input_conditions to try condition decoding on non-condition
    /// bytes, making ALL EncryptedContent UTXOs permanently unspendable.)
    ///
    /// `LPShare` uses condition-prefixed layout:
    /// `[condition_bytes][1B version][32B pool_id]`. Default constructor
    /// attaches `Condition::Signature(owner)` so existing call sites stay
    /// ergonomic. Custom conditions (AmountGuard, etc.) are supported via
    /// `Output::lp_share_with_condition()`.
    pub fn is_conditioned(&self) -> bool {
        matches!(
            self,
            Self::Multisig
                | Self::Hashlock
                | Self::HTLC
                | Self::Vesting
                | Self::NFT
                | Self::FungibleAsset
                | Self::BridgeHTLC
                | Self::LPShare
        )
    }

    /// Returns true if `amount` on this output type is denominated in native DOLI.
    ///
    /// Non-native types store token units, LP shares, or zero (Pool) in the
    /// `amount` field.  Summing those as DOLI would corrupt supply calculations,
    /// balance queries, and fee accounting.
    pub fn is_native_amount(&self) -> bool {
        matches!(
            self,
            Self::Normal
                | Self::Bond
                | Self::Multisig
                | Self::Hashlock
                | Self::HTLC
                | Self::Vesting
                | Self::BridgeHTLC
                | Self::NFT
                | Self::EncryptedContent
        )
    }
}

/// Sighash type controlling what parts of the transaction an input's signature covers.
///
/// Modeled after Bitcoin's SIGHASH flags. Used for partial signing (PSBT-style)
/// where different parties sign different inputs of the same transaction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SighashType {
    /// Sign ALL inputs and ALL outputs (default, backwards-compatible).
    /// Both parties must have the complete transaction before signing.
    #[default]
    All = 0,
    /// Sign only THIS input + ALL outputs.
    /// Allows other parties to add their own inputs after the signer has signed.
    /// Used for NFT marketplace: seller signs their NFT input, buyer adds payment inputs later.
    AnyoneCanPay = 1,
}

impl SighashType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::All),
            1 => Some(Self::AnyoneCanPay),
            _ => None,
        }
    }
}

/// Transaction input (reference to a previous output)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    /// Hash of the transaction containing the output
    pub prev_tx_hash: Hash,
    /// Index of the output in that transaction
    pub output_index: u32,
    /// Signature proving ownership
    pub signature: Signature,
    /// Sighash type: what this input's signature covers.
    /// Default: All (backwards-compatible with v1 transactions).
    #[serde(default)]
    pub sighash_type: SighashType,
    /// Number of outputs this input's signature commits to (AnyoneCanPay only).
    /// 0 = all outputs (backward compat with pre-v3.7.1 transactions).
    /// N > 0 = sighash covers only the first N outputs, allowing the buyer
    /// to append additional outputs (e.g. change) without invalidating
    /// the seller's signature.
    #[serde(default)]
    pub committed_output_count: u32,
    /// Public key of the spender (P2PKH: reveals pubkey at spend time).
    /// Pre-fork transactions have `None` (signature verification skipped).
    /// Post-fork transactions MUST have `Some(pk)` for signature enforcement.
    ///
    /// Part of the bincode wire format since v5.1.0 (P0-001 hard fork).
    /// Old chain data (pre-fork) is deserialized via `LegacyInputV3` / `deserialize_block_compat`
    /// which converts to `Input` with `public_key: None`.
    pub public_key: Option<crypto::PublicKey>,
}

impl Input {
    /// Create a new input (default sighash: All, no pubkey)
    pub fn new(prev_tx_hash: Hash, output_index: u32) -> Self {
        Self {
            prev_tx_hash,
            output_index,
            signature: Signature::default(),
            sighash_type: SighashType::All,
            committed_output_count: 0,
            public_key: None,
        }
    }

    /// Create a new input with AnyoneCanPay sighash type.
    /// The signature covers only this input + all outputs (not other inputs).
    pub fn new_anyone_can_pay(prev_tx_hash: Hash, output_index: u32) -> Self {
        Self {
            prev_tx_hash,
            output_index,
            signature: Signature::default(),
            sighash_type: SighashType::AnyoneCanPay,
            committed_output_count: 0,
            public_key: None,
        }
    }

    /// Create an AnyoneCanPay input that commits to only the first N outputs.
    /// The buyer can append additional outputs (e.g. change) without
    /// invalidating the seller's signature.
    pub fn new_anyone_can_pay_partial(
        prev_tx_hash: Hash,
        output_index: u32,
        committed_output_count: u32,
    ) -> Self {
        Self {
            prev_tx_hash,
            output_index,
            signature: Signature::default(),
            sighash_type: SighashType::AnyoneCanPay,
            committed_output_count,
            public_key: None,
        }
    }

    /// Builder method: set the spender's public key for signature verification.
    pub fn with_public_key(mut self, pk: crypto::PublicKey) -> Self {
        self.public_key = Some(pk);
        self
    }

    /// Create an outpoint identifier
    pub fn outpoint(&self) -> (Hash, u32) {
        (self.prev_tx_hash, self.output_index)
    }

    /// Serialize for signing
    pub fn serialize_for_signing(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(self.prev_tx_hash.as_bytes());
        bytes.extend_from_slice(&self.output_index.to_le_bytes());
        bytes
    }
}
