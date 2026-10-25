//! INC-I-255 — canonical slash evidence above the slashing gate (devnet genesis).
//!
// covers: slash_evidence slash_canonical slash_state utxo transaction data core types validation
//
// OUTPUT CONTRACT: fn validate_transaction(&Transaction, &ValidationContext)
//                  fn validate_transaction_skip_registration_vdf(&Transaction, &ValidationContext)
//                  fn validate_transaction_with_utxos(&Transaction, &ValidationContext, &impl UtxoProvider)
//   O1 return Result<(), ValidationError> (all three entry points, compared for parity)
//   no mutable params, no receiver writes, no store writes (pure)
// OUTPUT CONTRACT: fn SlashData::double_production(PublicKey, BlockHeader, BlockHeader) -> SlashData
//   O2 return value: header order, reporter_signature, producer_pubkey, headers carried unchanged
//   O3 Transaction::new_slash_producer(O2).hash() (the tx id)
// PATHS: P-AH (h >= slash gate and h >= sig gate) | P-DEC (decode + canonical re-encode)
//        | P-TGT (validate_slash_target: accused registered, not slashed, none queued)
// INPUT PARTITIONS:
//   IE-OK     two signed headers, same slot, accused, devnet genesis, ascending hash, default reporter
//   IE-TRAIL  IE-OK bytes + suffix (1 zero byte, 7 x 0xFF, a full second copy)
//   IE-ORDER  IE-OK headers in descending hash order
//   IE-REP    IE-OK with reporter_signature != default (3 values)
//   IE-PAE    valid PriceAttestationEquivocation, oracle active
//   IE-RUNA   other-key signature, same hash, different slots, foreign genesis, unsigned, oversize, wrong accused
//   ctx       at the gate, far above it, zero-flow fee branch (inc_i_173 gate 0)
// MATRIX:
//   O2,O3 x {(lo,hi), (hi,lo)}            -> inc_i_255_double_production_* (two tests)
//   O1 x P-AH x IE-OK                      -> inc_i_255_valid_canonical_slash_accepted_with_zero_fee_above_gate
//   O1 x P-DEC x IE-TRAIL                  -> inc_i_255_trailing_bytes_rejected_as_non_canonical
//   O1 x P-AH x IE-ORDER / IE-REP / IE-PAE -> one test each, message substring asserted
//   O1 x P-AH x IE-RUNA                    -> inc_i_255_run_a_rejections_still_invalid_slash_above_gate
//   O1 x P-TGT x IE-OK {active, inactive}  -> inc_i_255_slash_target_inactive_producer_refused

use crypto::{Hash, KeyPair, PublicKey, Signature};
use doli_core::block::BlockHeader;
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::tpop::heartbeat::hash_chain_vdf;
use doli_core::transaction::{PriceAttestationData, SlashData, SlashingEvidence, Transaction};
use doli_core::validation::{
    validate_slash_target, validate_transaction, validate_transaction_skip_registration_vdf,
    validate_transaction_with_utxos, SlashTargetState, UtxoInfo, UtxoProvider, ValidationContext,
    ValidationError,
};

pub(super) const HEIGHT: u64 = 1_000;
const SLOT: u32 = 5_000;
const NETWORK: Network = Network::Devnet;

pub(super) struct NoUtxos;

impl UtxoProvider for NoUtxos {
    fn get_utxo(&self, _tx_hash: &Hash, _output_index: u32) -> Option<UtxoInfo> {
        None
    }
}

fn params() -> ConsensusParams {
    ConsensusParams::devnet()
}

pub(super) fn ctx_at(height: u64, slash_gate: u64, sig_gate: u64) -> ValidationContext {
    let p = params();
    let ts = p.slot_to_timestamp(SLOT);
    ValidationContext::new(p, NETWORK, ts, height)
        .with_inc_i_236_block_signature_activation_height(sig_gate)
        .with_inc_i_255_slashing_activation_height(slash_gate)
}

pub(super) fn armed_ctxs() -> Vec<ValidationContext> {
    let mut zero_flow = ctx_at(HEIGHT, HEIGHT, HEIGHT);
    zero_flow.inc_i_173_activation_height = 0;
    vec![
        ctx_at(HEIGHT, HEIGHT, HEIGHT),
        ctx_at(HEIGHT + 10_000, HEIGHT, HEIGHT),
        zero_flow,
    ]
}

/// `armed_ctxs()` with an open evidence window for a pair at `SLOT`: parent
/// slot `SLOT` (age 1), the devnet unbonding period, and at least 20 blocks
/// of tenure above the slash gate.
pub(super) fn armed_target_ctxs() -> Vec<ValidationContext> {
    armed_ctxs()
        .into_iter()
        .map(|mut ctx| {
            ctx.prev_slot = SLOT;
            ctx.unbonding_period = NETWORK.params().unbonding_period;
            ctx.current_height = ctx
                .current_height
                .max(ctx.inc_i_255_slashing_activation_height + 20);
            ctx
        })
        .collect()
}

pub(super) fn accused() -> KeyPair {
    KeyPair::from_seed([0x55; 32])
}

pub(super) fn header(prev_byte: u8, slot: u32, producer: PublicKey) -> BlockHeader {
    let p = params();
    let prev_hash = Hash::from_bytes([prev_byte; 32]);
    let merkle_root = doli_core::block::compute_merkle_root(&[]);
    let input = vdf::block_input(&prev_hash, &merkle_root, slot, &producer);
    let value = hash_chain_vdf(&input, NETWORK.heartbeat_vdf_iterations()).to_vec();
    BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash: p.genesis_hash,
        timestamp: p.slot_to_timestamp(slot),
        slot,
        producer,
        vdf_output: vdf::VdfOutput { value },
        vdf_proof: vdf::VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    }
}

pub(super) fn sign(h: &mut BlockHeader, kp: &KeyPair) {
    let msg = h.signing_message();
    h.vdf_proof.pi = crypto::signature::sign(msg.as_bytes(), kp.private_key()).to_vec();
}

fn signed(prev_byte: u8, slot: u32, kp: &KeyPair) -> BlockHeader {
    let mut h = header(prev_byte, slot, *kp.public_key());
    sign(&mut h, kp);
    h
}

fn ascending(a: BlockHeader, b: BlockHeader) -> (BlockHeader, BlockHeader) {
    if a.hash().as_bytes() <= b.hash().as_bytes() {
        (a, b)
    } else {
        (b, a)
    }
}

/// Two signed headers of the accused for one slot, as (lower hash, higher hash).
pub(super) fn ordered_pair() -> (BlockHeader, BlockHeader) {
    let kp = accused();
    let (lo, hi) = ascending(signed(0xAB, SLOT, &kp), signed(0xCD, SLOT, &kp));
    assert!(lo.hash().as_bytes() < hi.hash().as_bytes(), "PRECONDITION");
    (lo, hi)
}

pub(super) fn raw_tx(
    accused: PublicKey,
    h1: BlockHeader,
    h2: BlockHeader,
    reporter: Signature,
) -> Transaction {
    Transaction::new_slash_producer(SlashData {
        producer_pubkey: accused,
        evidence: SlashingEvidence::DoubleProduction {
            block_header_1: h1,
            block_header_2: h2,
        },
        reporter_signature: reporter,
    })
}

/// Canonical form built WITHOUT the new constructor, so rule tests do not depend on it.
fn ordered_tx(accused: PublicKey, a: BlockHeader, b: BlockHeader) -> Transaction {
    let (lo, hi) = ascending(a, b);
    raw_tx(accused, lo, hi, Signature::default())
}

pub(super) fn canonical_tx() -> Transaction {
    let (lo, hi) = ordered_pair();
    Transaction::new_slash_producer(SlashData::double_production(
        *accused().public_key(),
        hi,
        lo,
    ))
}

pub(super) fn pae_tx() -> Transaction {
    let kp = accused();
    let att = |price_cents| {
        let mut a = PriceAttestationData {
            signer_pubkey: *kp.public_key(),
            price_cents,
            pair_id: Hash::from_bytes([0x0A; 32]),
            epoch_number: 3,
            signature: Signature::default(),
        };
        a.signature = crypto::signature::sign_hash(&a.signing_message(), kp.private_key());
        a
    };
    Transaction::new_slash_producer(SlashData {
        producer_pubkey: *kp.public_key(),
        evidence: SlashingEvidence::PriceAttestationEquivocation {
            attestation_1: att(100),
            attestation_2: att(200),
        },
        reporter_signature: Signature::default(),
    })
}

pub(super) fn with_trailing(suffix: &[u8]) -> Transaction {
    let mut tx = canonical_tx();
    tx.extra_data.extend_from_slice(suffix);
    tx
}

pub(super) fn descending_tx() -> Transaction {
    let (lo, hi) = ordered_pair();
    raw_tx(*accused().public_key(), hi, lo, Signature::default())
}

pub(super) fn reporter_tx(reporter: Signature) -> Transaction {
    let (lo, hi) = ordered_pair();
    raw_tx(*accused().public_key(), lo, hi, reporter)
}

fn oversize_tx() -> Transaction {
    let kp = accused();
    let filler = vec![*KeyPair::from_seed([0x33; 32]).public_key(); 3_000];
    let mut a = header(0xAB, SLOT, *kp.public_key());
    let mut b = header(0xCD, SLOT, *kp.public_key());
    a.missed_producers = filler.clone();
    b.missed_producers = filler;
    sign(&mut a, &kp);
    sign(&mut b, &kp);
    let tx = ordered_tx(*kp.public_key(), a, b);
    assert!(tx.extra_data.len() > 64 * 1024, "PRECONDITION: > 64 KiB");
    tx
}

/// Every Run A rejection, each otherwise canonical (ascending order, default reporter).
pub(super) fn run_a_invalid() -> Vec<(&'static str, Transaction)> {
    let kp = accused();
    let pk = *kp.public_key();
    let attacker = KeyPair::from_seed([0x99; 32]);
    let (lo, hi) = ordered_pair();

    let mut other_a = header(0xAB, SLOT, pk);
    let mut other_b = header(0xCD, SLOT, pk);
    sign(&mut other_a, &attacker);
    sign(&mut other_b, &attacker);

    let mut foreign_a = header(0xAB, SLOT, pk);
    let mut foreign_b = header(0xCD, SLOT, pk);
    foreign_a.genesis_hash = Hash::from_bytes([0x77; 32]);
    foreign_b.genesis_hash = Hash::from_bytes([0x77; 32]);
    sign(&mut foreign_a, &kp);
    sign(&mut foreign_b, &kp);

    vec![
        ("signed by another key", ordered_tx(pk, other_a, other_b)),
        (
            "same header twice",
            raw_tx(pk, lo.clone(), lo.clone(), Signature::default()),
        ),
        (
            "different slots",
            ordered_tx(pk, signed(0xAB, SLOT, &kp), signed(0xCD, SLOT + 1, &kp)),
        ),
        ("foreign genesis", ordered_tx(pk, foreign_a, foreign_b)),
        (
            "unsigned headers",
            ordered_tx(pk, header(0xAB, SLOT, pk), header(0xCD, SLOT, pk)),
        ),
        ("oversize", oversize_tx()),
        (
            "accused is not the producer",
            raw_tx(*attacker.public_key(), lo, hi, Signature::default()),
        ),
    ]
}

type Verdict = (&'static str, Result<(), ValidationError>);

pub(super) fn verdicts(tx: &Transaction, ctx: &ValidationContext) -> [Verdict; 3] {
    [
        ("validate_transaction", validate_transaction(tx, ctx)),
        (
            "validate_transaction_skip_registration_vdf",
            validate_transaction_skip_registration_vdf(tx, ctx),
        ),
        (
            "validate_transaction_with_utxos",
            validate_transaction_with_utxos(tx, ctx, &NoUtxos),
        ),
    ]
}

/// InvalidSlash on all three entry points, the same error on each; `needle` must be in the message.
pub(super) fn assert_invalid_slash(
    tx: &Transaction,
    ctx: &ValidationContext,
    needle: Option<&str>,
    what: &str,
) {
    let all = verdicts(tx, ctx);
    for (entry, v) in &all {
        match v {
            Err(ValidationError::InvalidSlash(msg)) => {
                if let Some(n) = needle {
                    assert!(
                        msg.contains(n),
                        "{what}: {entry} message {msg:?} must contain {n:?}"
                    );
                }
            }
            other => panic!("{what}: {entry} must return InvalidSlash, got {other:?}"),
        }
    }
    assert_eq!(all[0].1, all[1].1, "{what}: entry points disagree");
    assert_eq!(all[1].1, all[2].1, "{what}: entry points disagree");
}

fn evidence_headers(d: &SlashData) -> (&BlockHeader, &BlockHeader) {
    match &d.evidence {
        SlashingEvidence::DoubleProduction {
            block_header_1,
            block_header_2,
        } => (block_header_1, block_header_2),
        other => panic!("double_production must build DoubleProduction, got {other:?}"),
    }
}

// REQ-SLASH-004 — Decision: a failure means two reporters holding the same evidence build different slash ids, re-opening the per-offence flood.
#[test]
fn inc_i_255_double_production_orders_by_hash_and_zeroes_reporter() {
    let (lo, hi) = ordered_pair();
    let pk = *accused().public_key();
    for (first, second) in [(lo.clone(), hi.clone()), (hi.clone(), lo.clone())] {
        let d = SlashData::double_production(pk, first, second);
        let (h1, h2) = evidence_headers(&d);
        assert_eq!(h1, &lo, "header 1 must be the lower hash, signature kept");
        assert_eq!(h2, &hi, "header 2 must be the higher hash, signature kept");
        assert_eq!(d.reporter_signature, Signature::default());
        assert_eq!(d.producer_pubkey, pk);
    }
}

// REQ-SLASH-004 — Decision: a failure means the tx id depends on which header a node saw first, so N observers submit N slashes.
#[test]
fn inc_i_255_double_production_same_tx_id_for_either_argument_order() {
    let (lo, hi) = ordered_pair();
    let pk = *accused().public_key();
    let a =
        Transaction::new_slash_producer(SlashData::double_production(pk, lo.clone(), hi.clone()));
    let b = Transaction::new_slash_producer(SlashData::double_production(pk, hi, lo));
    assert_eq!(a.extra_data, b.extra_data);
    assert_eq!(a.hash(), b.hash());
    assert_ne!(
        a.hash(),
        descending_tx().hash(),
        "NON-VACUITY: header order must change the id"
    );
}

// REQ-SLASH-002 — Decision: a failure means a validly authenticated slash still dies at the fee floor, so slashing stays unmineable.
#[test]
fn inc_i_255_valid_canonical_slash_accepted_with_zero_fee_above_gate() {
    let tx = canonical_tx();
    assert!(
        tx.inputs.is_empty() && tx.outputs.is_empty(),
        "PRECONDITION: fee 0"
    );
    assert_eq!(
        tx.minimum_fee(),
        1,
        "PRECONDITION: the floor would demand 1"
    );
    for ctx in armed_ctxs() {
        for (entry, v) in verdicts(&tx, &ctx) {
            assert_eq!(
                v,
                Ok(()),
                "{entry} at h={} must accept the canonical slash",
                ctx.current_height
            );
        }
    }
}

// REQ-SLASH-004 — Decision: a failure means padding the encoding mints a fresh tx id for the same evidence.
#[test]
fn inc_i_255_trailing_bytes_rejected_as_non_canonical() {
    let copy = canonical_tx().extra_data;
    for suffix in [vec![0x00u8], vec![0xFFu8; 7], copy] {
        let tx = with_trailing(&suffix);
        for ctx in armed_ctxs() {
            assert_invalid_slash(
                &tx,
                &ctx,
                Some("non-canonical"),
                &format!("{}-byte suffix", suffix.len()),
            );
        }
    }
}

// REQ-SLASH-004 — Decision: a failure means swapping the two headers mints a second valid id for one offence.
#[test]
fn inc_i_255_descending_header_order_rejected() {
    let tx = descending_tx();
    for ctx in armed_ctxs() {
        assert_invalid_slash(&tx, &ctx, Some("header order"), "descending headers");
    }
}

// REQ-SLASH-002 — Decision: a failure means the unverified reporter field stays free, so any relayer re-signs and re-ids the slash.
#[test]
fn inc_i_255_non_default_reporter_signature_rejected() {
    let real = crypto::signature::sign(b"reporter", KeyPair::from_seed([0x44; 32]).private_key());
    for reporter in [
        Signature::from_bytes([7u8; 64]),
        Signature::from_bytes([0xFF; 64]),
        real,
    ] {
        let tx = reporter_tx(reporter);
        for ctx in armed_ctxs() {
            assert_invalid_slash(&tx, &ctx, Some("reporter signature"), "reporter set");
        }
    }
}

// REQ-SLASH-002 — Decision: a failure means price-attestation evidence rides the new zero-fee slash path that only block evidence earned.
#[test]
fn inc_i_255_price_attestation_equivocation_refused_above_gate() {
    let tx = pae_tx();
    let below = ctx_at(HEIGHT, HEIGHT + 1, HEIGHT).with_oracle_activation_height(0);
    assert_eq!(
        validate_transaction(&tx, &below),
        Ok(()),
        "NON-VACUITY: the attestation evidence is valid by today's rules"
    );
    for ctx in armed_ctxs() {
        assert_invalid_slash(
            &tx,
            &ctx.clone().with_oracle_activation_height(0),
            Some("DoubleProduction"),
            "price attestation equivocation, oracle active",
        );
        assert_invalid_slash(
            &tx,
            &ctx,
            None,
            "price attestation equivocation, oracle off",
        );
    }
}

// REQ-SLASH-002 — Decision: a failure means the new canonical rules replaced, instead of added to, the Run A authentication rules.
#[test]
fn inc_i_255_run_a_rejections_still_invalid_slash_above_gate() {
    for (what, tx) in run_a_invalid() {
        for ctx in armed_ctxs() {
            assert_invalid_slash(&tx, &ctx, None, what);
        }
    }
}

// REQ-SLASH-001 — Decision: a failure means a slash against a registered but no longer active producer (Exited) is accepted, a mineable no-op on a key that cannot produce.
#[test]
fn inc_i_255_slash_target_inactive_producer_refused() {
    let tx = canonical_tx();
    let pk = *accused().public_key();
    let active = SlashTargetState {
        active: true,
        oldest_bond_slot: Some(0),
        ..SlashTargetState::default()
    };
    for ctx in armed_target_ctxs() {
        let h = ctx.current_height;
        let ok = validate_slash_target(&tx, &ctx.clone().with_slash_targets([(pk, active)]));
        assert_eq!(ok, Ok(()), "NON-VACUITY at h={h}: an active accused passes");
        let r = validate_slash_target(
            &tx,
            &ctx.with_slash_targets([(pk, SlashTargetState::default())]),
        );
        assert!(
            matches!(&r, Err(ValidationError::InvalidSlash(m)) if m.contains("not an active or unbonding producer")),
            "h={h}: a registered, unslashed, unqueued but inactive accused must be refused; got {r:?}"
        );
    }
}
