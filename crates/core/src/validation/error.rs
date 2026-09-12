use serde_json::Value;
use thiserror::Error;

use crate::types::{Amount, BlockHeight};
use crypto::Hash;

/// Validation errors.
///
/// Each variant provides specific context about what validation check failed,
/// enabling precise error reporting and debugging.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// Block genesis_hash doesn't match our chain identity.
    /// This means the block was produced by a node with different genesis parameters.
    #[error("genesis hash mismatch: got={got}, expected={expected}")]
    GenesisHashMismatch {
        /// Genesis hash in the block header.
        got: crypto::Hash,
        /// Expected genesis hash from our consensus params.
        expected: crypto::Hash,
    },

    /// Block fork_id doesn't match our computed fork identity.
    /// The block was produced by a node with different active hard forks.
    #[error("fork_id mismatch: got={got}, expected={expected}")]
    ForkIdMismatch {
        got: crypto::Hash,
        expected: crypto::Hash,
    },

    /// Block or transaction version is unsupported.
    #[error("invalid version: {0}")]
    InvalidVersion(u32),

    /// Block timestamp does not advance from previous block.
    #[error("invalid timestamp: block={block}, expected>={expected}")]
    InvalidTimestamp {
        /// Timestamp in the block header.
        block: u64,
        /// Minimum expected timestamp.
        expected: u64,
    },

    /// Block timestamp is too far in the future.
    #[error("timestamp too far in future: {0}")]
    TimestampTooFuture(u64),

    /// Block slot does not match its timestamp.
    #[error("invalid slot derivation: got={got}, expected={expected}")]
    InvalidSlot {
        /// Slot in the block header.
        got: u32,
        /// Expected slot based on timestamp.
        expected: u32,
    },

    /// Block slot does not advance from previous block.
    #[error("slot not advancing: got={got}, prev={prev}")]
    SlotNotAdvancing {
        /// Slot in the block header.
        got: u32,
        /// Slot of the previous block.
        prev: u32,
    },

    /// Block slot is too far in the future.
    #[error("slot too far in future: got={got}, current={current}, max_future={max_future}")]
    SlotTooFuture {
        /// Slot in the block header.
        got: u32,
        /// Current slot based on wall clock.
        current: u32,
        /// Maximum allowed future slots.
        max_future: u64,
    },

    /// Block slot is too far in the past.
    #[error("slot too far in past: got={got}, current={current}, max_past={max_past}")]
    SlotTooPast {
        /// Slot in the block header.
        got: u32,
        /// Current slot based on wall clock.
        current: u32,
        /// Maximum allowed past slots.
        max_past: u64,
    },

    /// Merkle root does not match transactions.
    #[error("invalid merkle root: header={header}, computed={computed}")]
    InvalidMerkleRoot {
        /// Merkle root stored in the block header.
        header: Hash,
        /// Merkle root recomputed from the block's transactions.
        computed: Hash,
    },

    /// Data root (blob commitment) does not match blob hashes in block.
    #[error("invalid data root")]
    InvalidDataRoot,

    /// VDF proof is invalid.
    #[error("invalid VDF proof: {reason}")]
    InvalidVdfProof {
        /// What specifically failed in VDF verification.
        reason: String,
    },

    /// Producer is not authorized for this slot.
    #[error("invalid producer for slot: producer={producer}, slot={slot}, reason={reason}")]
    InvalidProducer {
        /// The public key of the block's claimed producer.
        producer: String,
        /// The slot the block claims to occupy.
        slot: u32,
        /// Why the producer is not eligible.
        reason: String,
    },

    /// Block exceeds maximum size.
    #[error("block too large: {size} > {max}")]
    BlockTooLarge {
        /// Actual block size in bytes.
        size: usize,
        /// Maximum allowed size.
        max: usize,
    },

    /// Block has no transactions (must have at least coinbase).
    #[error("missing coinbase transaction")]
    MissingCoinbase,

    /// Coinbase transaction is malformed.
    #[error("invalid coinbase: {0}")]
    InvalidCoinbase(String),

    /// Block-level validation failed (reward distribution, epoch rules).
    #[error("invalid block: {0}")]
    InvalidBlock(String),

    /// Regular transaction validation failed.
    #[error("invalid transaction: {0}")]
    InvalidTransaction(String),

    /// Same output spent twice within a block.
    #[error("double spend detected: tx={tx_hash}, output_index={output_index}")]
    DoubleSpend {
        /// Transaction hash of the output being double-spent.
        tx_hash: Hash,
        /// Output index within that transaction.
        output_index: u32,
    },

    /// Transaction outputs exceed inputs.
    #[error("insufficient funds: inputs={inputs}, outputs={outputs}")]
    InsufficientFunds {
        /// Total input amount.
        inputs: Amount,
        /// Total output amount.
        outputs: Amount,
    },

    /// Signature verification failed for an input.
    #[error("invalid signature for input {index}")]
    InvalidSignature {
        /// Index of the input with invalid signature.
        index: usize,
    },

    /// Attempting to spend a locked output before maturity.
    #[error("output locked until height {lock_height}, current height {current_height}")]
    OutputLocked {
        /// Height at which the output becomes spendable.
        lock_height: BlockHeight,
        /// Current blockchain height.
        current_height: BlockHeight,
    },

    /// Referenced output does not exist.
    #[error("output not found: tx={tx_hash}, index={output_index}")]
    OutputNotFound {
        /// Transaction hash containing the output.
        tx_hash: Hash,
        /// Output index within the transaction.
        output_index: u32,
    },

    /// Output has already been spent.
    #[error("output already spent: tx={tx_hash}, index={output_index}")]
    OutputAlreadySpent {
        /// Transaction hash containing the output.
        tx_hash: Hash,
        /// Output index within the transaction.
        output_index: u32,
    },

    /// Amount calculation would overflow.
    #[error("amount overflow in {context}")]
    AmountOverflow {
        /// Context where the overflow occurred.
        context: String,
    },

    /// Amount exceeds the maximum possible supply.
    #[error("amount exceeds max supply: {amount} > {max}")]
    AmountExceedsSupply {
        /// The invalid amount.
        amount: Amount,
        /// Maximum supply.
        max: Amount,
    },

    /// Registration transaction validation failed.
    #[error("invalid registration: {0}")]
    InvalidRegistration(String),

    /// Public key hash mismatch.
    #[error("pubkey hash mismatch: expected {expected}, got {got}")]
    PubkeyHashMismatch {
        /// Expected hash from the output.
        expected: Hash,
        /// Actual hash of the provided pubkey.
        got: Hash,
    },

    /// Bond output requirements not met.
    #[error("invalid bond: {0}")]
    InvalidBond(String),

    /// Reward claim validation failed.
    #[error("invalid claim: {0}")]
    InvalidClaim(String),

    /// Bond claim validation failed.
    #[error("invalid bond claim: {0}")]
    InvalidBondClaim(String),

    /// Slash producer validation failed.
    #[error("invalid slash: {0}")]
    InvalidSlash(String),

    /// Add bond transaction validation failed.
    #[error("invalid add bond: {0}")]
    InvalidAddBond(String),

    /// Withdrawal request validation failed.
    #[error("invalid withdrawal request: {0}")]
    InvalidWithdrawalRequest(String),

    /// Claim withdrawal validation failed.
    #[error("invalid claim withdrawal: {0}")]
    InvalidClaimWithdrawal(String),

    /// MintAsset validation failed.
    #[error("invalid mint asset: {0}")]
    InvalidMintAsset(String),

    /// BurnAsset validation failed.
    #[error("invalid burn asset: {0}")]
    InvalidBurnAsset(String),

    /// Epoch reward transaction validation failed.
    #[error("invalid epoch reward: {0}")]
    InvalidEpochReward(String),

    /// Epoch rewards present in non-boundary block.
    #[error("unexpected epoch reward: rewards only allowed at epoch boundaries")]
    UnexpectedEpochReward,

    /// Missing required epoch rewards at boundary.
    #[error(
        "missing epoch reward: block at epoch boundary must include rewards for epoch {epoch}"
    )]
    MissingEpochReward {
        /// The epoch that should have been rewarded.
        epoch: u64,
    },

    /// Epoch reward distribution doesn't match expected.
    #[error("epoch reward mismatch: {reason}")]
    EpochRewardMismatch {
        /// Description of what doesn't match.
        reason: String,
    },

    /// Maintainer change transaction validation failed.
    #[error("invalid maintainer change: {0}")]
    InvalidMaintainerChange(String),

    /// Delegation transaction validation failed.
    #[error("invalid delegation: {0}")]
    InvalidDelegation(String),

    /// Protocol activation transaction validation failed.
    #[error("invalid protocol activation: {0}")]
    InvalidProtocolActivation(String),

    /// Transaction fee is below the minimum required.
    #[error("insufficient fee: got {actual}, minimum {minimum} (base {base} + {extra_bytes} bytes * {per_byte}/byte)")]
    InsufficientFee {
        /// Actual fee paid (total_input - total_output).
        actual: Amount,
        /// Minimum required fee.
        minimum: Amount,
        /// Base fee component.
        base: Amount,
        /// Total extra_data bytes across all outputs.
        extra_bytes: u64,
        /// Per-byte fee rate.
        per_byte: Amount,
    },

    /// Pool transaction is invalid.
    #[error("invalid pool transaction: {0}")]
    InvalidPool(String),

    /// Swap transaction is invalid.
    #[error("invalid swap: {0}")]
    InvalidSwap(String),

    /// Liquidity operation is invalid.
    #[error("invalid liquidity operation: {0}")]
    InvalidLiquidity(String),

    /// INC-I-078: DelegateBond rejected because the target producer's
    /// `received_delegations` total would exceed the per-producer cap.
    ///
    /// Emitted only at and after `received_delegation_cap_activation_height`.
    /// Pre-activation, this check is bypassed.
    #[error("delegation cap exceeded: producer={producer} current={current} requested={requested} cap={cap}")]
    DelegationCapExceeded {
        /// Hex pubkey hash (or "<unknown>") of the target producer.
        producer: String,
        /// Current sum of `received_delegations[*].1` for the target.
        current: u64,
        /// Bond count the rejected DelegateBond would add.
        requested: u64,
        /// Active cap value (`network_params.received_delegation_cap`).
        cap: u64,
    },

    /// INC-I-078: DelegateBond or RevokeDelegation rejected because the
    /// delegator's Ed25519 signature is missing or invalid.
    ///
    /// Emitted only at and after `delegation_auth_activation_height`.
    /// Pre-activation, signatures are not checked (legacy zero-input form
    /// accepted).
    #[error("delegation signature invalid: {reason}")]
    DelegationSignatureInvalid {
        /// Human-readable description of the signature failure.
        reason: String,
    },

    /// Input is missing the required public key (post-sig-verification hard fork).
    ///
    /// After `sig_verification_height`, every input MUST include its spender's
    /// public key so that signature verification can be performed. Pre-fork
    /// transactions (serialized without public_key) are exempt.
    #[error("missing public key for input {index} (required at height >= {activation_height})")]
    MissingPublicKey {
        /// Index of the input missing its public key.
        index: usize,
        /// The activation height after which public keys are mandatory.
        activation_height: u64,
    },

    /// AMM Foundations M3 (D1): a CreatePool transaction failed the
    /// `MINIMUM_LIQUIDITY` first-deposit lock invariant.
    ///
    /// Triggered on EITHER of the two failure modes:
    ///   1. Pool UTXO declares `total_lp_shares < MINIMUM_LIQUIDITY`
    ///      (under-minted: nothing to lock).
    ///   2. Creator's first-deposit `LPShare.amount + MINIMUM_LIQUIDITY !=
    ///      total_lp_shares` (creator share exceeds the legal cap).
    ///
    /// The stable error code is `"AMM_MINIMUM_LIQUIDITY"` (log prefix
    /// `[ERRTX-AMM002]`). Agentic consumers can read `declared_total`,
    /// `creator_share`, and `minimum_liquidity` to compute the deficit.
    ///
    /// Spec: `specs/defi-foundations-economics.md` §0 D1.
    #[error(
        "[ERRTX-AMM002] minimum liquidity invariant violated: declared_total={declared_total} creator_share={creator_share} minimum_liquidity={minimum_liquidity}"
    )]
    AmmMinimumLiquidity {
        /// Pool UTXO's declared `total_lp_shares`.
        declared_total: u64,
        /// First-deposit `LPShare.amount` (the creator's share).
        creator_share: u64,
        /// Active `MINIMUM_LIQUIDITY` constant.
        minimum_liquidity: u64,
    },

    /// AMM Foundations M1: an AMM transaction was submitted before the
    /// `amm_activation_height` gate opens.
    ///
    /// Covers the 4 AMM tx types in one variant: CreatePool, AddLiquidity,
    /// RemoveLiquidity, Swap.
    ///
    /// The lending (B.1) and NFT-frac (B.2) tx types were tombstoned in
    /// the DeFi L1 Foundations Architecture (2026-05-26). Their discriminants
    /// return None from from_u32 and never reach any gate.
    ///
    ///
    /// `tx_type` is the rejected transaction's `TxType` enum discriminant
    /// (`u32`), so agentic consumers can identify which AMM operation was
    /// requested without parsing the message. The stable error code is
    /// `"AMM_NOT_ACTIVATED"` ([ERRTX-AMM001]).
    #[error(
        "[ERRTX-AMM001] amm not activated: tx_type={tx_type} activation_height={activation_height} current_height={current_height}"
    )]
    AmmNotActivated {
        /// `TxType` discriminant (19=CreatePool, 20=AddLiquidity,
        /// 21=RemoveLiquidity, 22=Swap).
        tx_type: u32,
        /// Configured `amm_activation_height` on the validation context.
        activation_height: u64,
        /// Height at which the rejection occurred.
        current_height: u64,
    },

    /// INC-I-080: AddBond rejected because the producer's bond total
    /// (`bond_count` + in-flight pending AddBonds + this request) would
    /// exceed `MAX_BONDS_PER_PRODUCER`.
    ///
    /// Emitted only at and after `addbond_cap_enforcement_activation_height`.
    /// Pre-activation, this check is bypassed and the historical clip-at-
    /// epoch-flush behavior (`ProducerInfo::add_bonds`) is preserved for
    /// replay safety. See `specs/delegation-architecture.md` (AddBond cap).
    #[error(
        "addbond cap exceeded: current={current} pending={pending} requested={requested} max={max}"
    )]
    AddBondCapExceeded {
        /// Producer's current `bond_count`.
        current: u32,
        /// Sum of bond counts over the producer's in-flight pending AddBonds.
        pending: u32,
        /// Bond count the rejected AddBond would add.
        requested: u32,
        /// Active cap (`MAX_BONDS_PER_PRODUCER`).
        max: u32,
    },

    /// [ERRBLS-001] post-AH attestation body failed verification (INC-I-178 D7).
    ///
    /// `reason` is one of the four stable labels the node also publishes as the
    /// `reason` dimension of `doli_attestation_verify_rejected_total`:
    /// `root_mismatch`, `aggregate_invalid`,
    /// `aggregate_nonempty_for_empty_bitfield`, `missing_bls_key`.
    #[error(
        "[ERRBLS-001] attestation verify failed: reason={reason} height={height} activation_height={activation_height}"
    )]
    AttestationVerifyFailed {
        /// Stable reason label; also the Prometheus `reason` label value.
        reason: String,
        /// Height of the rejected block.
        height: u64,
        /// `inc_i_178_attestation_bls_activation_height` in force.
        activation_height: u64,
    },

    /// INC-I-171: a `RequestWithdrawal` pays out more than the penalized net of
    /// the Bond inputs it spends plus its non-Bond input value.
    ///
    /// Emitted only inside
    /// `[inc_i_171_vesting_penalty_activation_height, ..._disable_height)`.
    #[error(
        "withdrawal payout exceeds vested net: payout={payout} bound={bound} bond_inputs={bond_inputs} non_bond_value={non_bond_value}"
    )]
    WithdrawalPayoutExceedsVestedNet {
        /// Value the withdrawal pays out.
        payout: u64,
        /// Ceiling: penalized net of the spent Bonds + `non_bond_value`.
        bound: u64,
        /// COUNT of spent Bond inputs, not their value.
        bond_inputs: u32,
        /// Non-Bond input total that widened the ceiling.
        non_bond_value: u64,
    },

    /// INC-I-171: `vesting_quarter_slots` is not a usable divisor.
    ///
    /// Zero would panic the penalty ladder's divide; above `u32::MAX` it does
    /// not fit [`crate::types::Slot`]. Checked after the activation gate, so a
    /// dormant rule never reads the value.
    #[error("vesting quarter invalid: quarter_slots={quarter_slots}")]
    VestingQuarterInvalid {
        /// The rejected value, as configured.
        quarter_slots: u64,
    },

    /// INC-I-171: a spent Bond input's `extra_data` does not decode to a
    /// `creation_slot`, so its age is unknown.
    ///
    /// Fail-closed (INV-VEST-013): an undecodable age is never read as fully
    /// vested. Unwired at M2; M3 is the only caller.
    #[error("withdrawal bond extra_data malformed: input_index={input_index} len={len}")]
    WithdrawalBondExtraDataMalformed {
        /// Index of the offending input within the transaction.
        input_index: u32,
        /// Byte length actually found.
        len: usize,
    },

    /// [ERRTX-ROT001] a `RotateBlsKey` must spend exactly one input, which is
    /// the input that reveals the authorising producer key.
    #[error("[ERRTX-ROT001] rotation input count: got={got} expected=1")]
    RotateBlsInputCount {
        /// Number of inputs the transaction actually carries.
        got: usize,
    },

    /// [ERRTX-ROT002] the block height is below
    /// `bls_key_rotation_activation_height`, so the type does not exist for
    /// consensus yet.
    #[error("[ERRTX-ROT002] bls key rotation not activated: current_height={current_height} activation_height={activation_height}")]
    RotateBlsNotActivated {
        /// Height of the block being validated.
        current_height: u64,
        /// Gate in force for this context.
        activation_height: u64,
    },

    /// [ERRTX-ROT003] the Ed25519 authorisation over `rotation_auth_digest`
    /// does not verify for `producer`. The digest binds the spent outpoint, so
    /// a rotation lifted onto another outpoint lands here (REQ-ROT-SEC-003).
    #[error("[ERRTX-ROT003] rotation authorisation signature invalid: producer={producer}")]
    RotateBlsBadSignature {
        /// Hex of the producer key the signature was checked against.
        producer: String,
    },

    /// [ERRTX-ROT004] `new_bls_pubkey` is not a key that can ever attest.
    #[error("[ERRTX-ROT004] rotation new bls key invalid: reason={reason}")]
    RotateBlsBadPoint {
        /// Stable label: `zero`, `identity` or `not_on_curve`.
        reason: String,
    },

    /// [ERRTX-ROT007] the proof of possession does not verify under the
    /// rotation DST. There is no fallback to the registration DST.
    #[error("[ERRTX-ROT007] rotation proof of possession invalid: producer={producer}")]
    RotateBlsBadPop {
        /// Hex of the producer key the proof of possession binds.
        producer: String,
    },

    /// [ERRTX-ROT008] `payload.producer` is not the key the spent input
    /// reveals: the fee payer IS the producer.
    #[error("[ERRTX-ROT008] rotation producer mismatch: payload={payload} input={input}")]
    RotateBlsProducerMismatch {
        /// Hex of `payload.producer`.
        payload: String,
        /// Hex of `inputs[0].public_key`, or `none` when the input reveals none.
        input: String,
    },

    /// [ERRTX-ROT009] a rotation carries exactly one `Normal` output.
    #[error("[ERRTX-ROT009] rotation output shape: outputs={outputs} first_type={first_type}")]
    RotateBlsOutputShape {
        /// Number of outputs the transaction actually carries.
        outputs: usize,
        /// Type of output 0, or `none` when there is no output.
        first_type: String,
    },

    /// [ERRTX-ROT010] `extra_data` is not a decodable canonical payload.
    /// Length-exact, so a tolerated suffix cannot become a malleability channel.
    #[error("[ERRTX-ROT010] rotation payload invalid: len={len} expected={expected}")]
    RotateBlsBadPayload {
        /// Byte length actually found.
        len: usize,
        /// Canonical length, `ROTATE_BLS_DATA_LEN`.
        expected: usize,
    },
}

impl ValidationError {
    /// Returns a stable, machine-readable error code for programmatic matching.
    ///
    /// These codes are part of the public API contract — agents and tooling
    /// can rely on them for pattern matching without parsing human-readable strings.
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::GenesisHashMismatch { .. } => "GENESIS_HASH_MISMATCH",
            Self::ForkIdMismatch { .. } => "FORK_ID_MISMATCH",
            Self::InvalidVersion(_) => "INVALID_VERSION",
            Self::InvalidTimestamp { .. } => "INVALID_TIMESTAMP",
            Self::TimestampTooFuture(_) => "TIMESTAMP_TOO_FUTURE",
            Self::InvalidSlot { .. } => "INVALID_SLOT",
            Self::SlotNotAdvancing { .. } => "SLOT_NOT_ADVANCING",
            Self::SlotTooFuture { .. } => "SLOT_TOO_FUTURE",
            Self::SlotTooPast { .. } => "SLOT_TOO_PAST",
            Self::InvalidMerkleRoot { .. } => "INVALID_MERKLE_ROOT",
            Self::InvalidDataRoot => "INVALID_DATA_ROOT",
            Self::InvalidVdfProof { .. } => "INVALID_VDF_PROOF",
            Self::InvalidProducer { .. } => "INVALID_PRODUCER",
            Self::BlockTooLarge { .. } => "BLOCK_TOO_LARGE",
            Self::MissingCoinbase => "MISSING_COINBASE",
            Self::InvalidCoinbase(_) => "INVALID_COINBASE",
            Self::InvalidBlock(_) => "INVALID_BLOCK",
            Self::InvalidTransaction(_) => "INVALID_TRANSACTION",
            Self::DoubleSpend { .. } => "DOUBLE_SPEND",
            Self::InsufficientFunds { .. } => "INSUFFICIENT_FUNDS",
            Self::InvalidSignature { .. } => "INVALID_SIGNATURE",
            Self::OutputLocked { .. } => "OUTPUT_LOCKED",
            Self::OutputNotFound { .. } => "OUTPUT_NOT_FOUND",
            Self::OutputAlreadySpent { .. } => "OUTPUT_ALREADY_SPENT",
            Self::AmountOverflow { .. } => "AMOUNT_OVERFLOW",
            Self::AmountExceedsSupply { .. } => "AMOUNT_EXCEEDS_SUPPLY",
            Self::InvalidRegistration(_) => "INVALID_REGISTRATION",
            Self::PubkeyHashMismatch { .. } => "PUBKEY_HASH_MISMATCH",
            Self::InvalidBond(_) => "INVALID_BOND",
            Self::InvalidClaim(_) => "INVALID_CLAIM",
            Self::InvalidBondClaim(_) => "INVALID_BOND_CLAIM",
            Self::InvalidSlash(_) => "INVALID_SLASH",
            Self::InvalidAddBond(_) => "INVALID_ADD_BOND",
            Self::InvalidWithdrawalRequest(_) => "INVALID_WITHDRAWAL_REQUEST",
            Self::InvalidClaimWithdrawal(_) => "INVALID_CLAIM_WITHDRAWAL",
            Self::InvalidMintAsset(_) => "INVALID_MINT_ASSET",
            Self::InvalidBurnAsset(_) => "INVALID_BURN_ASSET",
            Self::InvalidEpochReward(_) => "INVALID_EPOCH_REWARD",
            Self::UnexpectedEpochReward => "UNEXPECTED_EPOCH_REWARD",
            Self::MissingEpochReward { .. } => "MISSING_EPOCH_REWARD",
            Self::EpochRewardMismatch { .. } => "EPOCH_REWARD_MISMATCH",
            Self::InvalidMaintainerChange(_) => "INVALID_MAINTAINER_CHANGE",
            Self::InvalidDelegation(_) => "INVALID_DELEGATION",
            Self::InvalidProtocolActivation(_) => "INVALID_PROTOCOL_ACTIVATION",
            Self::InsufficientFee { .. } => "INSUFFICIENT_FEE",
            Self::InvalidPool(_) => "INVALID_POOL",
            Self::InvalidSwap(_) => "INVALID_SWAP",
            Self::InvalidLiquidity(_) => "INVALID_LIQUIDITY",
            Self::MissingPublicKey { .. } => "MISSING_PUBLIC_KEY",
            Self::DelegationCapExceeded { .. } => "DELEGATION_CAP_EXCEEDED",
            Self::DelegationSignatureInvalid { .. } => "DELEGATION_SIGNATURE_INVALID",
            Self::AddBondCapExceeded { .. } => "ADDBOND_CAP_EXCEEDED",
            Self::AmmNotActivated { .. } => "AMM_NOT_ACTIVATED",
            Self::AmmMinimumLiquidity { .. } => "AMM_MINIMUM_LIQUIDITY",
            Self::AttestationVerifyFailed { .. } => "ATTESTATION_VERIFY_FAILED",
            Self::WithdrawalPayoutExceedsVestedNet { .. } => "ECON_WITHDRAWAL_PAYOUT_EXCEEDS_NET",
            Self::VestingQuarterInvalid { .. } => "ECON_VESTING_QUARTER_INVALID",
            Self::WithdrawalBondExtraDataMalformed { .. } => {
                "ECON_WITHDRAWAL_BOND_EXTRA_DATA_MALFORMED"
            }
            Self::RotateBlsInputCount { .. } => "ROTATE_BLS_INPUT_COUNT",
            Self::RotateBlsNotActivated { .. } => "ROTATE_BLS_NOT_ACTIVATED",
            Self::RotateBlsBadSignature { .. } => "ROTATE_BLS_BAD_SIGNATURE",
            Self::RotateBlsBadPoint { .. } => "ROTATE_BLS_BAD_POINT",
            Self::RotateBlsBadPop { .. } => "ROTATE_BLS_BAD_POP",
            Self::RotateBlsProducerMismatch { .. } => "ROTATE_BLS_PRODUCER_MISMATCH",
            Self::RotateBlsOutputShape { .. } => "ROTATE_BLS_OUTPUT_SHAPE",
            Self::RotateBlsBadPayload { .. } => "ROTATE_BLS_BAD_PAYLOAD",
        }
    }

    /// Serializes this error to structured JSON for agentic consumption.
    ///
    /// Returns a JSON object with:
    /// - `error_code`: stable machine-readable code (same as `error_code()`)
    /// - `message`: human-readable description (same as `Display`)
    /// - All structured fields from the variant (when available)
    ///
    /// Agents can match on `error_code` and read fields programmatically
    /// without parsing the message string.
    pub fn to_structured_json(&self) -> Value {
        let mut obj = serde_json::json!({
            "error_code": self.error_code(),
            "message": self.to_string(),
        });

        let map = obj.as_object_mut().unwrap();

        match self {
            Self::GenesisHashMismatch { got, expected } => {
                map.insert("got".into(), Value::String(got.to_string()));
                map.insert("expected".into(), Value::String(expected.to_string()));
            }
            Self::ForkIdMismatch { got, expected } => {
                map.insert("got".into(), Value::String(got.to_string()));
                map.insert("expected".into(), Value::String(expected.to_string()));
            }
            Self::InvalidVersion(v) => {
                map.insert("version".into(), (*v).into());
            }
            Self::InvalidTimestamp { block, expected } => {
                map.insert("block_timestamp".into(), (*block).into());
                map.insert("expected_minimum".into(), (*expected).into());
            }
            Self::TimestampTooFuture(ts) => {
                map.insert("timestamp".into(), (*ts).into());
            }
            Self::InvalidSlot { got, expected } => {
                map.insert("got".into(), (*got).into());
                map.insert("expected".into(), (*expected).into());
            }
            Self::SlotNotAdvancing { got, prev } => {
                map.insert("got".into(), (*got).into());
                map.insert("prev".into(), (*prev).into());
            }
            Self::SlotTooFuture {
                got,
                current,
                max_future,
            } => {
                map.insert("got".into(), (*got).into());
                map.insert("current".into(), (*current).into());
                map.insert("max_future".into(), (*max_future).into());
            }
            Self::SlotTooPast {
                got,
                current,
                max_past,
            } => {
                map.insert("got".into(), (*got).into());
                map.insert("current".into(), (*current).into());
                map.insert("max_past".into(), (*max_past).into());
            }
            Self::InvalidMerkleRoot { header, computed } => {
                map.insert("header".into(), Value::String(header.to_string()));
                map.insert("computed".into(), Value::String(computed.to_string()));
            }
            Self::InvalidDataRoot => {}
            Self::InvalidVdfProof { reason } => {
                map.insert("reason".into(), Value::String(reason.clone()));
            }
            Self::InvalidProducer {
                producer,
                slot,
                reason,
            } => {
                map.insert("producer".into(), Value::String(producer.clone()));
                map.insert("slot".into(), (*slot).into());
                map.insert("reason".into(), Value::String(reason.clone()));
            }
            Self::BlockTooLarge { size, max } => {
                map.insert("size".into(), (*size).into());
                map.insert("max".into(), (*max).into());
            }
            Self::MissingCoinbase => {}
            Self::InvalidCoinbase(reason) => {
                map.insert("reason".into(), Value::String(reason.clone()));
            }
            Self::InvalidBlock(reason) => {
                map.insert("reason".into(), Value::String(reason.clone()));
            }
            Self::InvalidTransaction(reason) => {
                map.insert("reason".into(), Value::String(reason.clone()));
            }
            Self::DoubleSpend {
                tx_hash,
                output_index,
            } => {
                map.insert("tx_hash".into(), Value::String(tx_hash.to_hex()));
                map.insert("output_index".into(), (*output_index).into());
            }
            Self::InsufficientFunds { inputs, outputs } => {
                map.insert("inputs".into(), (*inputs).into());
                map.insert("outputs".into(), (*outputs).into());
            }
            Self::InvalidSignature { index } => {
                map.insert("input_index".into(), (*index).into());
            }
            Self::OutputLocked {
                lock_height,
                current_height,
            } => {
                map.insert("lock_height".into(), (*lock_height).into());
                map.insert("current_height".into(), (*current_height).into());
            }
            Self::OutputNotFound {
                tx_hash,
                output_index,
            } => {
                map.insert("tx_hash".into(), Value::String(tx_hash.to_hex()));
                map.insert("output_index".into(), (*output_index).into());
            }
            Self::OutputAlreadySpent {
                tx_hash,
                output_index,
            } => {
                map.insert("tx_hash".into(), Value::String(tx_hash.to_hex()));
                map.insert("output_index".into(), (*output_index).into());
            }
            Self::AmountOverflow { context } => {
                map.insert("context".into(), Value::String(context.clone()));
            }
            Self::AmountExceedsSupply { amount, max } => {
                map.insert("amount".into(), (*amount).into());
                map.insert("max_supply".into(), (*max).into());
            }
            Self::InvalidRegistration(reason) => {
                map.insert("reason".into(), Value::String(reason.clone()));
            }
            Self::PubkeyHashMismatch { expected, got } => {
                map.insert("expected".into(), Value::String(expected.to_string()));
                map.insert("got".into(), Value::String(got.to_string()));
            }
            Self::InvalidBond(reason)
            | Self::InvalidClaim(reason)
            | Self::InvalidBondClaim(reason)
            | Self::InvalidSlash(reason)
            | Self::InvalidAddBond(reason)
            | Self::InvalidWithdrawalRequest(reason)
            | Self::InvalidClaimWithdrawal(reason)
            | Self::InvalidMintAsset(reason)
            | Self::InvalidBurnAsset(reason)
            | Self::InvalidEpochReward(reason)
            | Self::InvalidMaintainerChange(reason)
            | Self::InvalidDelegation(reason)
            | Self::InvalidProtocolActivation(reason)
            | Self::InvalidPool(reason)
            | Self::InvalidSwap(reason)
            | Self::InvalidLiquidity(reason) => {
                map.insert("reason".into(), Value::String(reason.clone()));
            }
            Self::UnexpectedEpochReward => {}
            Self::MissingEpochReward { epoch } => {
                map.insert("epoch".into(), (*epoch).into());
            }
            Self::EpochRewardMismatch { reason } => {
                map.insert("reason".into(), Value::String(reason.clone()));
            }
            Self::InsufficientFee {
                actual,
                minimum,
                base,
                extra_bytes,
                per_byte,
            } => {
                map.insert("actual_fee".into(), (*actual).into());
                map.insert("minimum_fee".into(), (*minimum).into());
                map.insert("base_fee".into(), (*base).into());
                map.insert("extra_bytes".into(), (*extra_bytes).into());
                map.insert("per_byte_rate".into(), (*per_byte).into());
            }
            Self::MissingPublicKey {
                index,
                activation_height,
            } => {
                map.insert("input_index".into(), (*index).into());
                map.insert("activation_height".into(), (*activation_height).into());
            }
            Self::DelegationCapExceeded {
                producer,
                current,
                requested,
                cap,
            } => {
                map.insert("producer".into(), Value::String(producer.clone()));
                map.insert("current".into(), (*current).into());
                map.insert("requested".into(), (*requested).into());
                map.insert("cap".into(), (*cap).into());
            }
            Self::DelegationSignatureInvalid { reason } => {
                map.insert("reason".into(), Value::String(reason.clone()));
            }
            Self::AddBondCapExceeded {
                current,
                pending,
                requested,
                max,
            } => {
                map.insert("current".into(), (*current).into());
                map.insert("pending".into(), (*pending).into());
                map.insert("requested".into(), (*requested).into());
                map.insert("max".into(), (*max).into());
            }
            Self::AmmNotActivated {
                tx_type,
                activation_height,
                current_height,
            } => {
                map.insert("tx_type".into(), (*tx_type).into());
                map.insert("activation_height".into(), (*activation_height).into());
                map.insert("current_height".into(), (*current_height).into());
            }
            Self::AmmMinimumLiquidity {
                declared_total,
                creator_share,
                minimum_liquidity,
            } => {
                map.insert("declared_total".into(), (*declared_total).into());
                map.insert("creator_share".into(), (*creator_share).into());
                map.insert("minimum_liquidity".into(), (*minimum_liquidity).into());
            }
            Self::AttestationVerifyFailed {
                reason,
                height,
                activation_height,
            } => {
                map.insert("reason".into(), Value::String(reason.clone()));
                map.insert("height".into(), (*height).into());
                map.insert("activation_height".into(), (*activation_height).into());
            }
            Self::WithdrawalPayoutExceedsVestedNet {
                payout,
                bound,
                bond_inputs,
                non_bond_value,
            } => {
                map.insert("payout".into(), (*payout).into());
                map.insert("bound".into(), (*bound).into());
                map.insert("bond_inputs".into(), (*bond_inputs).into());
                map.insert("non_bond_value".into(), (*non_bond_value).into());
            }
            Self::VestingQuarterInvalid { quarter_slots } => {
                map.insert("quarter_slots".into(), (*quarter_slots).into());
            }
            Self::WithdrawalBondExtraDataMalformed { input_index, len } => {
                map.insert("input_index".into(), (*input_index).into());
                map.insert("len".into(), (*len).into());
            }
            Self::RotateBlsInputCount { got } => {
                map.insert("got".into(), (*got).into());
            }
            Self::RotateBlsNotActivated {
                current_height,
                activation_height,
            } => {
                map.insert("current_height".into(), (*current_height).into());
                map.insert("activation_height".into(), (*activation_height).into());
            }
            Self::RotateBlsBadSignature { producer } | Self::RotateBlsBadPop { producer } => {
                map.insert("producer".into(), Value::String(producer.clone()));
            }
            Self::RotateBlsBadPoint { reason } => {
                map.insert("reason".into(), Value::String(reason.clone()));
            }
            Self::RotateBlsProducerMismatch { payload, input } => {
                map.insert("payload".into(), Value::String(payload.clone()));
                map.insert("input".into(), Value::String(input.clone()));
            }
            Self::RotateBlsOutputShape {
                outputs,
                first_type,
            } => {
                map.insert("outputs".into(), (*outputs).into());
                map.insert("first_type".into(), Value::String(first_type.clone()));
            }
            Self::RotateBlsBadPayload { len, expected } => {
                map.insert("len".into(), (*len).into());
                map.insert("expected".into(), (*expected).into());
            }
        }

        obj
    }
}
