//! INC-I-217 M6 — REQ-ROT-009: the mempool relay and consensus must return the
//! SAME verdict for a `RotateBlsKey` transaction.
//!
//! TDD RED, EXPECTED: `mempool::rotation_filter` does not exist at HEAD, so this
//! module does not compile. That compile failure IS the red.
//!
//! HONEST SCOPE: both relay surfaces already refuse an invalid rotation today,
//! incidentally, through the generic validator's `RotateBlsKey` arm. M6 closes
//! no open hole; it converts an ACCIDENTAL property — one `_ =>` fallthrough
//! away from vanishing (INV-VALIDATION-001 / INC-I-147) — into a named
//! predicate plus this executable equivalence check.
//!
//! OUTPUT CONTRACT — `rotation_filter::rotation_admissible(&Transaction,
//! &ValidationContext) -> Result<(), String>`:
//!   O1 return: `Ok(())` | `Err(msg)` carrying the bracketed `[ERRTX-ROTnnn]`
//!   O2 mutable params: NONE — both arguments are `&`
//!   O3 receiver: NONE — free function
//!   O4 persistent store writes: NONE — stateless by contract (M7 owns state)
//!   O5 statics / O6 channels / O7 logs: NONE claimed
//! OUTPUT CONTRACT — `Mempool::add_transaction(Transaction, &UtxoSet,
//! BlockHeight, Slot) -> Result<AddTransactionResult, MempoolError>`:
//!   O8 return kind + the bracketed code it carries
//!   O9 receiver `self.entries` membership of the offered tx afterwards
//!   O10 `utxo_set` UNMUTATED — enforced by the type
//!
//! PATHS: P-ACCEPT | P-GATE (ROT002) | P-SHAPE (ROT001/ROT009/ROT010) |
//!   P-POINT (ROT004) | P-BIND (ROT008) | P-AUTH (ROT003) | P-POP (ROT007) |
//!   P-PASSTHROUGH (`tx_type != RotateBlsKey`).
//! INPUT PARTITIONS: the 10 named classes C1..C10, each exactly ONE mutation
//!   from the positive fixture, so no negative can pass for a second reason.
//! MATRIX: O1 x O8 x every class (the parity loop and the admission loop);
//!   O9 x every class; P-PASSTHROUGH once; plus the two source-level wiring
//!   rows — the only observable for WHICH call produced the verdict once both
//!   calls return the same error.
//! NOT CLAIMED: stateful verdicts (NotProducer / SameKey / AlreadyPending /
//!   KeyInUse) are M7 by design, not deferred scope.
//!
//! NO MOCKED CRYPTO: the positive carries a real BLS keypair, a real rotation
//! PoP under the rotation DST and a real Ed25519 authorisation over
//! `rotation_auth_digest`. Recipe copied from
//! `crates/core/tests/it/inc_i_217_m5_rotate_stateless.rs` — a mempool
//! integration test cannot import another crate's test module.

use std::path::Path;

use crypto::{BlsKeyPair, Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::genesis::genesis_hash;
use doli_core::transaction::{
    rotation_auth_digest, Input, Output, RotateBlsData, Transaction, TxType,
};
use doli_core::validation::{validate_transaction, ValidationContext};
use doli_core::Network;
use mempool::rotation_filter::rotation_admissible;
use mempool::{Mempool, MempoolPolicy};
use storage::{Outpoint, UtxoEntry, UtxoSet};

const NETWORK: Network = Network::Mainnet;

/// Injected rotation gate for the predicate/consensus parity rows. The shipped
/// value is `u64::MAX` on every network, so the above-gate corpus is otherwise
/// unreachable. The RELAY rows cannot inject — `add_transaction` reads the
/// shipped params and `bls_key_rotation_activation_height` is deliberately NOT
/// env-overridable — so they derive their heights from `shipped_gate()`.
const INJECTED_GATE: u64 = 500;
const ABOVE_INJECTED_GATE: u64 = 1_000;

const INPUT_VALUE: u64 = 5_000_000;
const OUTPUT_VALUE: u64 = 1_000_000;
const SLOT: u32 = 1;

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

fn producer_kp() -> KeyPair {
    KeyPair::from_seed([0x61; 32])
}

fn other_kp() -> KeyPair {
    KeyPair::from_seed([0x62; 32])
}

fn new_bls() -> BlsKeyPair {
    BlsKeyPair::from_seed(&[0x63; 32]).expect("32-byte BLS seed is valid")
}

fn outpoint_a() -> (Hash, u32) {
    (crypto::hash::hash(b"inc-i-217-m6-outpoint-a"), 3)
}

fn outpoint_b() -> (Hash, u32) {
    (crypto::hash::hash(b"inc-i-217-m6-outpoint-b"), 11)
}

fn producer_address() -> Hash {
    crypto::hash::hash_with_domain(
        crypto::ADDRESS_DOMAIN,
        producer_kp().public_key().as_bytes(),
    )
}

/// Ed25519 authorisation over the 32-byte `rotation_auth_digest` — raw digest
/// bytes, no second domain tag, exactly what the M10 CLI signs.
fn auth_signature(kp: &KeyPair, new_bls_pubkey: &[u8; 48], outpoint: (Hash, u32)) -> [u8; 64] {
    let digest = rotation_auth_digest(
        genesis_hash(NETWORK).as_bytes(),
        new_bls_pubkey,
        outpoint.0.as_bytes(),
        outpoint.1,
    );
    *crypto::signature::sign(&digest, kp.private_key()).as_bytes()
}

/// Proof of possession under the ROTATION DST, bound to `(new key, genesis,
/// producer)`.
fn rotation_pop(bls: &BlsKeyPair, producer: &PublicKey) -> [u8; 96] {
    *crypto::sign_rotation_pop(
        bls.secret_key(),
        bls.public_key(),
        genesis_hash(NETWORK).as_bytes(),
        producer.as_bytes(),
    )
    .expect("rotation PoP signing cannot fail for a valid key")
    .as_bytes()
}

fn well_formed_payload(outpoint: (Hash, u32)) -> RotateBlsData {
    let kp = producer_kp();
    let bls = new_bls();
    let new_bls_pubkey = *bls.public_key().as_bytes();
    RotateBlsData {
        producer: *kp.public_key().as_bytes(),
        new_bls_pubkey,
        bls_pop: rotation_pop(&bls, kp.public_key()),
        signature: auth_signature(&kp, &new_bls_pubkey, outpoint),
    }
}

/// A 1-in / 1-out `RotateBlsKey` carrying `payload`. The input reveals the
/// producer key — the fee payer IS the producer.
fn tx_carrying(payload: &RotateBlsData, outpoint: (Hash, u32)) -> Transaction {
    let mut input = Input::new(outpoint.0, outpoint.1);
    input.public_key = Some(*producer_kp().public_key());
    Transaction {
        version: 1,
        tx_type: TxType::RotateBlsKey,
        inputs: vec![input],
        outputs: vec![Output::normal(OUTPUT_VALUE, producer_address())],
        extra_data: payload.encode().to_vec(),
    }
}

/// Sign every input LAST, after all mutations: admission checks input
/// signatures, so a mutant signed before mutation would be refused for the
/// wrong reason and its class would agree by accident.
fn signed(mut tx: Transaction) -> Transaction {
    let kp = producer_kp();
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&signing_hash, kp.private_key());
    }
    tx
}

fn well_formed() -> Transaction {
    let outpoint = outpoint_a();
    signed(tx_carrying(&well_formed_payload(outpoint), outpoint))
}

fn mutate_payload(edit: impl FnOnce(&mut RotateBlsData)) -> Transaction {
    let outpoint = outpoint_a();
    let mut payload = well_formed_payload(outpoint);
    edit(&mut payload);
    signed(tx_carrying(&payload, outpoint))
}

fn mutate_tx(edit: impl FnOnce(&mut Transaction)) -> Transaction {
    let outpoint = outpoint_a();
    let mut tx = tx_carrying(&well_formed_payload(outpoint), outpoint);
    edit(&mut tx);
    signed(tx)
}

/// Re-signs the authorisation over the mutated key, so only the point rule is
/// broken by intent.
fn tx_with_new_key(new_bls_pubkey: [u8; 48]) -> Transaction {
    let outpoint = outpoint_a();
    let mut payload = well_formed_payload(outpoint);
    payload.new_bls_pubkey = new_bls_pubkey;
    payload.signature = auth_signature(&producer_kp(), &new_bls_pubkey, outpoint);
    signed(tx_carrying(&payload, outpoint))
}

// ---------------------------------------------------------------------------
// The 10 classes
// ---------------------------------------------------------------------------

struct Class {
    name: &'static str,
    /// `None` means both surfaces must ACCEPT.
    code: Option<&'static str>,
    /// `true` places this class BELOW the gate on both surfaces.
    below_gate: bool,
    tx: Transaction,
}

impl Class {
    fn accept(name: &'static str, tx: Transaction) -> Self {
        Self {
            name,
            code: None,
            below_gate: false,
            tx,
        }
    }

    fn reject(name: &'static str, code: &'static str, tx: Transaction) -> Self {
        Self {
            name,
            code: Some(code),
            below_gate: false,
            tx,
        }
    }

    fn reject_below_gate(name: &'static str, code: &'static str, tx: Transaction) -> Self {
        Self {
            name,
            code: Some(code),
            below_gate: true,
            tx,
        }
    }

    /// Height for the injected-gate rows.
    fn injected_height(&self) -> u64 {
        if self.below_gate {
            INJECTED_GATE - 1
        } else {
            ABOVE_INJECTED_GATE
        }
    }

    /// Height for the RELAY rows, DERIVED from the shipped gate (INV-GOV-001):
    /// never a literal, so a future pin moves these rows with it.
    fn relay_height(&self) -> u64 {
        let gate = shipped_gate();
        if self.below_gate {
            gate.saturating_sub(1)
        } else {
            gate
        }
    }
}

/// C1..C10, in contract order. `input_count` uses TWO inputs and `output_shape`
/// TWO outputs on purpose: the EMPTY variants are caught by the generic
/// `[ERRTX001]` / `[ERRTX002]` guards before the rotation arm runs, so they
/// would make the two surfaces disagree for a reason M6 does not own.
fn all_classes() -> Vec<Class> {
    let mut identity = [0u8; 48];
    identity[0] = 0xc0;
    vec![
        Class::accept("C1_valid", well_formed()),
        Class::reject_below_gate("C2_below_activation", "[ERRTX-ROT002]", well_formed()),
        Class::reject(
            "C3_input_count",
            "[ERRTX-ROT001]",
            mutate_tx(|tx| {
                let mut extra = Input::new(outpoint_b().0, outpoint_b().1);
                extra.public_key = Some(*producer_kp().public_key());
                tx.inputs.push(extra);
            }),
        ),
        Class::reject(
            "C4_output_shape",
            "[ERRTX-ROT009]",
            mutate_tx(|tx| tx.outputs.push(Output::normal(1, producer_address()))),
        ),
        Class::reject(
            "C5_bad_payload",
            "[ERRTX-ROT010]",
            mutate_tx(|tx| tx.extra_data.truncate(239)),
        ),
        Class::reject(
            "C6_bad_point",
            "[ERRTX-ROT004]",
            tx_with_new_key([0xFF; 48]),
        ),
        Class::reject(
            "C7_producer_mismatch",
            "[ERRTX-ROT008]",
            mutate_payload(|p| p.producer = *other_kp().public_key().as_bytes()),
        ),
        Class::reject(
            "C8_bad_signature",
            "[ERRTX-ROT003]",
            mutate_payload(|p| p.signature = [0u8; 64]),
        ),
        Class::reject(
            "C9_bad_pop",
            "[ERRTX-ROT007]",
            mutate_payload(|p| p.bls_pop = [0u8; 96]),
        ),
        Class::reject(
            "C10_identity_point",
            "[ERRTX-ROT004]",
            tx_with_new_key(identity),
        ),
    ]
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn shipped_gate() -> u64 {
    NETWORK.params().bls_key_rotation_activation_height
}

/// A context with the INJECTED gate — the parity rows the relay cannot reach
/// from shipped params.
fn ctx_at(height: u64) -> ValidationContext {
    ValidationContext::new(ConsensusParams::mainnet(), NETWORK, 0, height)
        .with_bls_key_rotation_activation_height(INJECTED_GATE)
}

/// The context `pool.rs` itself builds for the rotation arm: the SHIPPED gate at
/// the caller's tip height. This is the mirror consensus verdict for a relay row.
fn relay_mirror_ctx(height: u64) -> ValidationContext {
    ValidationContext::new(ConsensusParams::mainnet(), NETWORK, 0, height)
        .with_bls_key_rotation_activation_height(shipped_gate())
}

/// Extract the bracketed `[ERRTX-ROTnnn]` a message carries, so the two surfaces
/// are compared on the CODE, not on prose.
fn rot_code(msg: &str) -> Option<String> {
    let start = msg.find("[ERRTX-ROT")?;
    let end = msg[start..].find(']')? + start + 1;
    Some(msg[start..end].to_string())
}

fn seed_inputs(utxo: &mut UtxoSet, tx: &Transaction) {
    for input in &tx.inputs {
        let entry = UtxoEntry {
            output: Output::normal(INPUT_VALUE, producer_address()),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        utxo.insert(Outpoint::new(input.prev_tx_hash, input.output_index), entry)
            .expect("fixture: seed the rotation fee input");
    }
}

/// Offer `class.tx` to a fresh mempool at the class's relay height. Returns the
/// verdict and whether the transaction is resident afterwards (O8 + O9).
fn admit(class: &Class) -> (Result<(), String>, bool) {
    let mut pool = Mempool::new(
        MempoolPolicy::testnet(),
        ConsensusParams::mainnet(),
        NETWORK,
    );
    let mut utxo = UtxoSet::new();
    seed_inputs(&mut utxo, &class.tx);
    let tx_hash = class.tx.hash();
    let verdict = pool
        .add_transaction(class.tx.clone(), &utxo, class.relay_height(), SLOT)
        .map(|_| ())
        .map_err(|e| e.to_string());
    (verdict, pool.contains(&tx_hash))
}

fn repo_file(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("wiring probe: cannot read {}: {e}", path.display()))
}

// ===========================================================================
// The parity loop — the metric the M6 probe reads
// ===========================================================================

// REQ-ROT-009 — Decision: whether the relay's NAMED rotation verdict is the same
// verdict consensus reaches, class by class and code by code. A failure names a
// class where either a transaction is relayed that no block can contain (wasted
// slot, gossip amplification) or one is refused that every block would accept
// (censorship of an honest producer's rotation) — the two directions M6 exists
// to exclude.
#[test]
fn req_rot_009_relay_and_consensus_agree_on_every_rotation_class() {
    // Paired positive first: a broken fixture would make every negative below
    // "agree" for the wrong reason.
    let control = well_formed();
    assert!(
        validate_transaction(&control, &ctx_at(ABOVE_INJECTED_GATE)).is_ok(),
        "PAIRED POSITIVE BROKEN: the unmutated rotation must validate above the \
         gate, or every agreement below proves nothing"
    );

    let mut disagreements: Vec<String> = Vec::new();
    for class in all_classes() {
        let ctx = ctx_at(class.injected_height());
        let relay = rotation_admissible(&class.tx, &ctx);
        let consensus = validate_transaction(&class.tx, &ctx);

        let agreed = match (&relay, &consensus) {
            (Ok(()), Ok(())) => class.code.is_none(),
            (Err(r), Err(c)) => {
                let relay_code = rot_code(r);
                let consensus_code = rot_code(&c.to_string());
                relay_code.is_some()
                    && relay_code == consensus_code
                    && relay_code.as_deref() == class.code
            }
            _ => false,
        };

        if agreed {
            // Load-bearing: the M6 outcome probe counts these lines.
            println!("ROT-CLASS-AGREE {}", class.name);
        } else {
            disagreements.push(format!(
                "{}: expected {:?}, relay={:?}, consensus={:?}",
                class.name,
                class.code,
                relay,
                consensus.map_err(|e| e.to_string())
            ));
        }
    }

    assert!(
        disagreements.is_empty(),
        "REQ-ROT-009: relay and consensus must agree on accept/reject AND on the \
         [ERRTX-ROTnnn] code for every class; disagreements: {disagreements:#?}"
    );
}

// ===========================================================================
// The relay surface — `Mempool::add_transaction`
// ===========================================================================

// REQ-ROT-009 — Decision: whether the whole admission path, not just the
// predicate in isolation, refuses each invalid class with the code consensus
// uses. A failure means the mempool is MORE PERMISSIVE than consensus for that
// class: every node relays and stores a transaction no block can ever contain,
// a free memory-and-bandwidth amplifier for an attacker.
#[test]
fn req_rot_009_the_mempool_refuses_every_invalid_rotation_class() {
    let mut failures: Vec<String> = Vec::new();
    for class in all_classes().into_iter().filter(|c| c.code.is_some()) {
        let expected = class.code.expect("filtered to rejecting classes");
        let height = class.relay_height();

        // Precondition: consensus itself rejects at this height. Without it,
        // "the relay rejected" could not be read as "the relay agreed".
        if validate_transaction(&class.tx, &relay_mirror_ctx(height)).is_ok() {
            failures.push(format!(
                "{}: consensus ACCEPTED at height {height}; the relay row is unreadable",
                class.name
            ));
            continue;
        }

        let (verdict, resident) = admit(&class);
        match &verdict {
            Ok(()) => failures.push(format!(
                "{}: MORE PERMISSIVE THAN CONSENSUS — admitted at height {height}",
                class.name
            )),
            Err(msg) if rot_code(msg).as_deref() != Some(expected) => failures.push(format!(
                "{}: refused with {:?}, consensus says {expected} ({msg})",
                class.name,
                rot_code(msg)
            )),
            Err(_) => {}
        }
        if resident {
            failures.push(format!("{}: refused but left RESIDENT (O9)", class.name));
        }
    }

    assert!(
        failures.is_empty(),
        "REQ-ROT-009: admission must refuse every invalid rotation class with the \
         consensus code and keep none resident; {failures:#?}"
    );
}

// REQ-ROT-009 — Decision: whether the relay refuses EVERY rotation, valid ones
// included. An admission predicate that cannot be satisfied is indistinguishable
// from a feature that never shipped — no producer could relay a rotation — and
// the nine rejection rows above would all still pass.
#[test]
fn req_rot_009_a_valid_rotation_is_admitted_at_the_gate_height() {
    let class = Class::accept("C1_valid", well_formed());
    let height = class.relay_height();
    assert!(
        validate_transaction(&class.tx, &relay_mirror_ctx(height)).is_ok(),
        "precondition: consensus must accept the positive at height {height} (the \
         shipped gate); if THIS fails the fixture is wrong, not the relay"
    );

    let (verdict, resident) = admit(&class);

    assert_eq!(
        verdict,
        Ok(()),
        "REQ-ROT-009: a rotation consensus accepts must be admitted. A \
         non-rotation code here (MPTX*/fee) means the FIXTURE is wrong — seeded \
         UTXO, address or fee — not the predicate"
    );
    assert!(
        resident,
        "REQ-ROT-009 (O9): an admitted rotation must be resident and relayable"
    );
}

// REQ-ROT-009 — Decision: whether the early return keys on `tx_type` rather than
// on "looks like a rotation". A predicate that judged other types would refuse
// every Transfer at every node — the mempool would stop accepting traffic
// entirely, which no rotation-only test would notice.
#[test]
fn req_rot_009_a_non_rotation_transaction_passes_the_predicate_untouched() {
    let mut input = Input::new(outpoint_a().0, outpoint_a().1);
    input.public_key = Some(*producer_kp().public_key());
    let transfer = signed(Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![input],
        outputs: vec![Output::normal(OUTPUT_VALUE, producer_address())],
        extra_data: Vec::new(),
    });

    // Below the gate AND above it: neither side of the rotation gate may touch a
    // transaction that is not a rotation.
    assert_eq!(rotation_admissible(&transfer, &ctx_at(1)), Ok(()));
    assert_eq!(
        rotation_admissible(&transfer, &ctx_at(ABOVE_INJECTED_GATE)),
        Ok(()),
        "REQ-ROT-009: a non-rotation transaction must pass the predicate \
         unchanged at every height"
    );

    let class = Class::accept("transfer", transfer);
    let (verdict, resident) = admit(&class);
    assert_eq!(
        verdict,
        Ok(()),
        "REQ-ROT-009: ordinary traffic must still be admitted with the predicate wired in"
    );
    assert!(resident);
}

// ===========================================================================
// The tip-height boundary — intentional, documented under-admission
// ===========================================================================

// REQ-ROT-009 — Decision: whether the one-block skew between admission (chain
// TIP height) and block validation (BLOCK height) errs safe. A failure in the
// asserted direction is harmless censorship for one block; a failure in the
// INVERSE direction — the relay accepting at a height consensus rejects — is the
// amplification bug, and the same-height row below is what catches it.
#[test]
fn req_rot_009_the_tip_height_boundary_under_admits_by_exactly_one_block() {
    let tx = well_formed();
    let tip = ctx_at(INJECTED_GATE - 1);
    let next_block = ctx_at(INJECTED_GATE);

    let relay = rotation_admissible(&tx, &tip)
        .expect_err("at tip = gate - 1 the relay must refuse the rotation");
    assert_eq!(
        rot_code(&relay).as_deref(),
        Some("[ERRTX-ROT002]"),
        "the refusal must be the GATE, not some other rule: {relay}"
    );

    // The direction that must never invert: at the SAME height consensus refuses
    // too, so the relay is never more permissive than consensus.
    let same_height = validate_transaction(&tx, &tip)
        .expect_err("consensus must also refuse below the gate at the same height");
    assert_eq!(
        rot_code(&same_height.to_string()).as_deref(),
        Some("[ERRTX-ROT002]")
    );

    // ...and the very next block accepts it. That is the accepted, conservative
    // skew, consistent with `vesting_bound` and `addbond_cap`.
    assert!(
        validate_transaction(&tx, &next_block).is_ok(),
        "REQ-ROT-009: the block AT the gate must accept the rotation the relay \
         refused one height earlier"
    );
    assert_eq!(rotation_admissible(&tx, &next_block), Ok(()));
}

// ===========================================================================
// Wiring — source level, because both calls return the same error
// ===========================================================================

// REQ-ROT-009 — Decision: whether the BUILDER consults the named predicate. A
// builder that skips it packs a rotation its own node refuses to relay, and no
// runtime assertion can see this from here: a mempool integration test cannot
// link `bins/node`.
#[test]
fn req_rot_009_the_block_builder_calls_the_named_predicate() {
    let assembly = repo_file("bins/node/src/node/production/assembly.rs");
    assert!(
        assembly.contains("rotation_admissible("),
        "REQ-ROT-009: assembly.rs must call rotation_admissible before \
         validate_transaction_with_utxos, or the builder's rotation verdict is \
         once again an accident of the generic validator"
    );
}

// REQ-ROT-009 — Decision: whether the relay's rotation verdict COMES FROM the
// named predicate or is still the generic validator's incidental byproduct. The
// two return identical errors, so behaviour cannot distinguish them — and an
// unwired predicate is exactly the INV-VALIDATION-001 shape M6 exists to remove:
// one `_ =>` edit away from a silently permissive relay.
#[test]
fn req_rot_009_the_relay_calls_the_named_predicate_before_the_generic_validator() {
    let pool = repo_file("crates/mempool/src/pool.rs");
    let named = pool
        .find("rotation_admissible(")
        .expect("REQ-ROT-009: pool.rs must call rotation_admissible in add_transaction");
    let generic = pool
        .find("validate_transaction(&tx, &ctx)")
        .expect("pool.rs must still call the generic validator");
    assert!(
        named < generic,
        "REQ-ROT-009: the named predicate must run BEFORE the generic validator, \
         so a rotation's verdict is the one the predicate gives and not a second \
         opinion nobody reaches"
    );
}

// REQ-ROT-009 — Decision: whether the relay rows above are still evaluated at a
// reachable height. The shipped gate is `u64::MAX` today, so those rows run AT
// `u64::MAX`; when the gate is pinned they must move with it. A failure here
// says the derivation broke, not that the pin is wrong.
#[test]
fn req_rot_009_the_relay_rows_track_the_shipped_gate() {
    let gate = shipped_gate();
    assert!(
        gate > 0,
        "a zero rotation gate leaves no below-gate height for the C2 relay row"
    );
    let below = Class::reject_below_gate("probe", "[ERRTX-ROT002]", well_formed());
    let above = Class::accept("probe", well_formed());
    assert_eq!(above.relay_height(), gate);
    assert_eq!(below.relay_height(), gate - 1);
}
