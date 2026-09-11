//! INC-I-217 M10 — pure construction of a `RotateBlsKey` transaction
//! (REQ-ROT-013). No I/O, no RPC, no wallet types, no printing.
//!
//! Every consensus-visible byte comes from the core codec
//! (`rotation_auth_digest`, `RotateBlsData::encode`, `sign_rotation_pop`): a
//! second encoder here is the drift this module exists to prevent.

use crypto::{BlsSecretKey, Hash, KeyPair, PrivateKey};
use doli_core::consensus::{BASE_FEE, FEE_DIVISOR, FEE_PER_BYTE};
use doli_core::transaction::{
    rotation_auth_digest, Input, Output, RotateBlsData, Transaction, TxType, ROTATE_BLS_DATA_LEN,
};

/// Disclosure shown before every rotation. Deliberately carries no block
/// height: no RPC exposes the activation height, and a number in client text
/// goes stale.
pub const ROTATE_BLS_NOTICE: &str = "\
IMPORTANT — a BLS key rotation CANNOT BE UNDONE.

  The new key is queued now and installed at the next epoch boundary.
  Until that boundary the chain still expects the OLD key, and the boundary
  block's own attestation can be lost while the swap lands.
  The only way back to the previous key is ANOTHER ROTATION, with another fee.
  Keep the new BLS secret in this wallet: an attestation signed with a key the
  chain does not hold is not counted.";

/// A normal, spendable output identified by the outpoint that enters the
/// authorisation digest.
#[derive(Clone, Debug)]
pub struct SpendableUtxo {
    /// Hash of the transaction that created the output.
    pub tx_hash: [u8; 32],
    /// Index of the output inside that transaction.
    pub output_index: u32,
    /// Value in base units.
    pub amount: u64,
}

/// What the chain currently says about this producer.
#[derive(Clone, Debug)]
pub struct OnChainProducer {
    /// True when the key is a known producer (any non-exited status).
    pub registered: bool,
    /// BLS key the chain holds today, if any.
    pub bls_pubkey: Option<[u8; 48]>,
    /// BLS key already queued by an earlier rotation, if any.
    pub pending_new_bls_pubkey: Option<[u8; 48]>,
}

/// The rotation the preconditions allow.
#[derive(Clone, Debug)]
pub struct RotationTarget {
    /// Key being replaced.
    pub old_bls_pubkey: Option<[u8; 48]>,
    /// Key to install.
    pub new_bls_pubkey: [u8; 48],
}

/// A signed rotation and the facts an operator must be able to check.
#[derive(Debug)]
pub struct RotateTxPlan {
    /// The transaction to broadcast.
    pub tx: Transaction,
    /// Decoded `extra_data` of `tx`.
    pub payload: RotateBlsData,
    /// The 32 bytes the producer key signed.
    pub auth_digest: [u8; 32],
    /// Fee paid, in base units.
    pub fee: u64,
    /// Value of the single output.
    pub change_value: u64,
}

/// Verdict of the consent rules, with the prompt left to the caller.
#[derive(Debug, PartialEq, Eq)]
pub enum ConsentOutcome {
    /// Proceed without asking.
    Accepted,
    /// Ask the operator before proceeding.
    MustPrompt,
}

/// Every refusal this module can produce. No variant carries key material.
#[derive(Debug)]
pub enum RotateTxError {
    /// The wallet holds no BLS key to rotate to.
    NoWalletBlsKey,
    /// The key is not a registered producer.
    ProducerNotRegistered,
    /// The wallet key is already the on-chain key.
    SameKey,
    /// An earlier rotation is still queued.
    RotationAlreadyPending,
    /// No terminal and no `--yes`.
    ConsentRequired,
    /// No single output strictly covers the fee.
    NoUtxoCoversFee {
        /// Fee that had to be covered.
        fee: u64,
        /// Largest spendable amount offered.
        largest: u64,
    },
    /// The spending secret is not an Ed25519 key.
    InvalidProducerSecret,
    /// The BLS secret is not a valid scalar.
    InvalidBlsSecret,
}

impl std::fmt::Display for RotateTxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoWalletBlsKey => write!(
                f,
                "this wallet holds no BLS key, so there is nothing to rotate to. Add one first: doli --wallet <wallet> add-bls"
            ),
            Self::ProducerNotRegistered => write!(
                f,
                "this key is not a registered producer, so it has no BLS key to rotate"
            ),
            Self::SameKey => write!(
                f,
                "the wallet BLS key is already the key on chain — a rotation would pay a fee to change nothing"
            ),
            Self::RotationAlreadyPending => write!(
                f,
                "a BLS key rotation is already queued for this producer; it is installed at the next epoch boundary"
            ),
            Self::ConsentRequired => write!(
                f,
                "refusing to rotate the BLS key without confirmation: stdin is not a terminal. Re-run with --yes to confirm you accept an irreversible key change"
            ),
            Self::NoUtxoCoversFee { fee, largest } => write!(
                f,
                "no single spendable output strictly covers the {fee} base-unit fee (largest offered: {largest})"
            ),
            Self::InvalidProducerSecret => write!(
                f,
                "the wallet spending key is not a valid Ed25519 secret key"
            ),
            Self::InvalidBlsSecret => {
                write!(f, "the wallet BLS key is not a valid BLS secret key")
            }
        }
    }
}

impl std::error::Error for RotateTxError {}

/// The 32 bytes the producer signs: the core digest over the genesis hash, the
/// new key and the SPENT outpoint (REQ-ROT-SEC-003 replay bind).
#[must_use]
pub fn rotation_auth_message(
    genesis_hash: &[u8; 32],
    new_bls_pubkey: &[u8; 48],
    fee_utxo: &SpendableUtxo,
) -> [u8; 32] {
    rotation_auth_digest(
        genesis_hash,
        new_bls_pubkey,
        &fee_utxo.tx_hash,
        fee_utxo.output_index,
    )
}

/// Refuse every rotation the node would reject at the stateful stage, before a
/// fee is spent.
///
/// # Errors
///
/// [`RotateTxError::NoWalletBlsKey`], [`RotateTxError::ProducerNotRegistered`],
/// [`RotateTxError::RotationAlreadyPending`] or [`RotateTxError::SameKey`].
pub fn check_rotation_preconditions(
    onchain: &OnChainProducer,
    wallet_bls_pubkey: Option<[u8; 48]>,
) -> Result<RotationTarget, RotateTxError> {
    let new_bls_pubkey = wallet_bls_pubkey.ok_or(RotateTxError::NoWalletBlsKey)?;
    if !onchain.registered {
        return Err(RotateTxError::ProducerNotRegistered);
    }
    if onchain.pending_new_bls_pubkey.is_some() {
        return Err(RotateTxError::RotationAlreadyPending);
    }
    if onchain.bls_pubkey == Some(new_bls_pubkey) {
        return Err(RotateTxError::SameKey);
    }
    Ok(RotationTarget {
        old_bls_pubkey: onchain.bls_pubkey,
        new_bls_pubkey,
    })
}

/// Fee for the canonical 240-byte payload, same formula as registration.
#[must_use]
pub fn rotate_bls_fee() -> u64 {
    BASE_FEE + (ROTATE_BLS_DATA_LEN as u64) * FEE_PER_BYTE / FEE_DIVISOR
}

/// Smallest output that STRICTLY covers `fee`, so the single change output is
/// never zero. Ordered by (amount, outpoint), so the node's UTXO order cannot
/// change which bytes the operator signs.
///
/// # Errors
///
/// [`RotateTxError::NoUtxoCoversFee`] when no amount exceeds `fee`.
pub fn select_fee_utxo(utxos: &[SpendableUtxo], fee: u64) -> Result<SpendableUtxo, RotateTxError> {
    utxos
        .iter()
        .filter(|utxo| utxo.amount > fee)
        .min_by_key(|utxo| (utxo.amount, utxo.tx_hash, utxo.output_index))
        .cloned()
        .ok_or_else(|| RotateTxError::NoUtxoCoversFee {
            fee,
            largest: utxos.iter().map(|utxo| utxo.amount).max().unwrap_or(0),
        })
}

/// Build and fully sign the 1-in / 1-out rotation transaction.
///
/// # Errors
///
/// [`RotateTxError::InvalidProducerSecret`] or [`RotateTxError::InvalidBlsSecret`].
pub fn build_rotate_bls_tx(
    genesis_hash: &[u8; 32],
    producer_secret_hex: &str,
    bls_secret_hex: &str,
    fee_utxo: &SpendableUtxo,
    fee: u64,
) -> Result<RotateTxPlan, RotateTxError> {
    let producer = KeyPair::from_private_key(
        PrivateKey::from_hex(producer_secret_hex)
            .map_err(|_| RotateTxError::InvalidProducerSecret)?,
    );
    let bls_secret =
        BlsSecretKey::from_hex(bls_secret_hex).map_err(|_| RotateTxError::InvalidBlsSecret)?;
    let bls_public = bls_secret.public_key();
    let new_bls_pubkey = *bls_public.as_bytes();

    let auth_digest = rotation_auth_message(genesis_hash, &new_bls_pubkey, fee_utxo);
    let payload = RotateBlsData {
        producer: *producer.public_key().as_bytes(),
        new_bls_pubkey,
        bls_pop: *crypto::sign_rotation_pop(
            &bls_secret,
            &bls_public,
            genesis_hash,
            producer.public_key().as_bytes(),
        )
        .map_err(|_| RotateTxError::InvalidBlsSecret)?
        .as_bytes(),
        signature: *crypto::signature::sign(&auth_digest, producer.private_key()).as_bytes(),
    };

    let mut input = Input::new(Hash::from_bytes(fee_utxo.tx_hash), fee_utxo.output_index);
    input.public_key = Some(*producer.public_key());
    let change_value = fee_utxo.amount.saturating_sub(fee);
    let change_address =
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, producer.public_key().as_bytes());

    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::RotateBlsKey,
        inputs: vec![input],
        outputs: vec![Output::normal(change_value, change_address)],
        extra_data: payload.encode().to_vec(),
    };
    // Signed LAST: the signing message commits to the outputs and extra_data set above.
    for index in 0..tx.inputs.len() {
        let message = tx.signing_message_for_input(index);
        tx.inputs[index].signature = crypto::signature::sign_hash(&message, producer.private_key());
    }

    Ok(RotateTxPlan {
        tx,
        payload,
        auth_digest,
        fee,
        change_value,
    })
}

/// Operator-facing summary of what the rotation does. Never carries a secret.
#[must_use]
pub fn rotation_report_lines(
    plan: &RotateTxPlan,
    old_bls_pubkey: Option<[u8; 48]>,
    effective_height: Option<u64>,
) -> Vec<String> {
    let mut lines = vec![
        format!(
            "  Current BLS key: {}",
            old_bls_pubkey.map_or_else(|| "(none on chain)".to_string(), hex::encode)
        ),
        format!(
            "  New BLS key:     {}",
            hex::encode(plan.payload.new_bls_pubkey)
        ),
        format!("  Fee:             {} base units", plan.fee),
        format!("  Change output:   {} base units", plan.change_value),
    ];
    lines.push(match effective_height {
        Some(height) => format!("  Takes effect:    height {height} (next epoch boundary)"),
        None => "  Takes effect:    at the next epoch boundary".to_string(),
    });
    lines
}

/// Decide whether the caller may proceed, must prompt, or must refuse.
///
/// A non-terminal stdin is never consent (INV-CLI-003).
///
/// # Errors
///
/// [`RotateTxError::ConsentRequired`] with no terminal and no `--yes`.
pub fn rotation_consent(yes: bool, is_terminal: bool) -> Result<ConsentOutcome, RotateTxError> {
    if yes {
        return Ok(ConsentOutcome::Accepted);
    }
    if is_terminal {
        return Ok(ConsentOutcome::MustPrompt);
    }
    Err(RotateTxError::ConsentRequired)
}

/// Only an explicit y/yes is consent.
#[must_use]
pub fn rotation_answer_accepted(answer: &str) -> bool {
    let answer = answer.trim();
    answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes")
}
