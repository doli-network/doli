// OUTPUT CONTRACT: fn duplicate_registration_rejected_at_admission
// O1: mempool add_transaction accept/reject verdict
// O2: rejection error identity vs block-validation error
// O3: mempool membership after revalidate
// PATHS: mempool admission / block validation / revalidate
// INPUT PARTITIONS: first registration (valid); duplicate with disjoint inputs (invalid); registration that becomes duplicate after admission
// MATRIX: 3 outputs x 3 paths x 3 partitions
//
// INC-I-147 residual — validation-parity gap. INV-VALIDATION-001.
// regression_tests id=160 (duplicate_registration_rejected_at_admission)
// regression_tests id=161 (mempool_and_apply_agree_on_every_registration_verdict)
//
// THE DEFECT UNDER TEST (measured 2026-08-04, see docs/.workflow/investigation-code.md
// and docs/.workflow/runtime-evidence.md):
//
//   `ValidationContext::pending_producer_keys` (crates/core/src/validation/types.rs:146)
//   is a plain `Vec` defaulting to `Vec::new()` (types.rs:264), so "not populated" is
//   indistinguishable from "no pending registrations". The consumer check at
//   crates/core/src/validation/registration.rs:173 therefore silently returns `false`
//   instead of erroring. `with_pending_producer_keys` has exactly TWO grep hits
//   repo-wide: the definition (types.rs:419) and its SOLE call site,
//   bins/node/src/node/validation_checks.rs:291 (block validation). The mempool never
//   calls it. Consequence: a second producer registration for a pubkey that already has
//   an accepted-but-not-yet-active registration is ACCEPTED at admission and REJECTED by
//   block validation — every node admits it, and each holding producer burns exactly one
//   slot when scheduled (measured 6/6; 35 lost slots over four GS-010 runs).
//
//   `Mempool::revalidate` (crates/mempool/src/pool.rs:1086-1112) re-runs exactly ONE
//   check — input existence. It builds no ValidationContext and ignores its height
//   argument. The seed's toxic TXs survived 577 revalidate passes and were held for
//   49-102 minutes.
//
// THESE TESTS MUST FAIL AGAINST THE UNFIXED CODE. They are the contract, not a
// description of current behaviour.
//
// covers: crates/mempool/src/pending_registrations.rs (pending-key derivation:
//         node-published ProducerSet snapshot UNION mempool-resident registrations)
// covers: crates/mempool/src/pool.rs (both ValidationContext sites + revalidate)
// covers: crates/mempool/src/lib.rs (module registration for the above)
//
// The mempool crate cannot reach a ProducerSet on its own, so source 1 is
// delivered by the node. These tests pin the mempool-side contract; the
// producer-side publication of the same value is:
// covers: bins/node/src/node/mod.rs (mempool_pending_producer_keys field +
//         refresh_mempool_producer_snapshot publishing pending_registration_keys)
// covers: bins/node/src/node/init.rs (share_pending_producer_keys wiring at all
//         three Node construction sites)
//
// HARNESS NOTE (structural, not a workaround): the `mempool` crate has no channel
// through which a caller can publish pending-registration state — `share_*` exists only
// for the oracle sunset flag and the ACTIVE producer snapshot
// (pool.rs:220/235; bins/node/src/node/init.rs:668-679 +
// `refresh_mempool_producer_snapshot` at bins/node/src/node/mod.rs:426, which publishes
// `active_producers_at_height`, NOT `pending_registration_keys`). Each path below is
// therefore driven through the representation of "P has an accepted registration that is
// not yet an active producer" that path can actually observe today:
//   - mempool admission  -> registration #1 is IN the mempool
//   - block validation   -> a real `storage::ProducerSet` carrying
//                           `PendingProducerUpdate::Register` (exactly the value
//                           validation_checks.rs:187 reads)
//   - revalidate         -> the shared active-producer snapshot
// Same fact, three representations, one required verdict.

use std::sync::{Arc, RwLock};

use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::transaction::{Input, Output, Transaction};
use doli_core::validation::{validate_transaction, ValidationContext};
use mempool::{Mempool, MempoolError, MempoolPolicy};
use storage::{Outpoint, PendingProducerUpdate, ProducerInfo, ProducerSet, UtxoEntry, UtxoSet};

// ---------------------------------------------------------------- constants

/// Devnet `genesis_blocks` = 40, so 100 is past the genesis carve-out in
/// `validate_registration_data_inner` (registration.rs:37) and the full
/// non-genesis registration path runs.
const HEIGHT: u64 = 100;

/// `ConsensusParams::devnet().initial_bond` — era 0 bond requirement at HEIGHT.
const BOND: u64 = 100_000_000;

/// Bond + fee. `minimum_fee()` = BASE_FEE(1) + 4 bytes of bond extra_data / 100 = 1.
const FUNDING: u64 = BOND + 1_000;

// ---------------------------------------------------------------- helpers

/// Devnet mempool: VDF verification is skipped for `Network::Devnet`
/// (registration.rs:227), and every activation gate is 0, so nothing masks the
/// registration checks under test.
fn devnet_mempool() -> Mempool {
    Mempool::new(
        MempoolPolicy::testnet(),
        ConsensusParams::devnet(),
        Network::Devnet,
    )
}

/// The producer's Ed25519 key — it owns the funding UTXOs AND is the
/// `RegistrationData.public_key`, exactly as the CLI does
/// (bins/cli/src/cmd_producer/register.rs:96-98).
fn producer_keypair() -> crypto::KeyPair {
    keypair_seeded(7)
}

/// A distinct producer identity. Used only by the positive control, which must
/// prove the fix rejects *duplicates*, not *registrations*.
fn keypair_seeded(seed: u8) -> crypto::KeyPair {
    crypto::KeyPair::from_seed([seed; 32])
}

fn producer_pubkey() -> crypto::PublicKey {
    *producer_keypair().public_key()
}

/// Deterministic BLS key for the mandatory proof-of-possession
/// (registration.rs:144-154).
fn bls_keypair() -> crypto::BlsKeyPair {
    bls_keypair_seeded(3)
}

fn bls_keypair_seeded(seed: u8) -> crypto::BlsKeyPair {
    let sk = crypto::BlsSecretKey::from_bytes([seed; 32]).expect("valid BLS scalar");
    crypto::BlsKeyPair::from_secret_key(sk)
}

/// Insert a spendable Normal UTXO owned by the producer key. Returns its tx hash.
fn fund(utxo_set: &mut UtxoSet, tag: &[u8], amount: u64) -> crypto::Hash {
    fund_for(utxo_set, &producer_keypair(), tag, amount)
}

/// `fund`, for an arbitrary owner.
fn fund_for(utxo_set: &mut UtxoSet, kp: &crypto::KeyPair, tag: &[u8], amount: u64) -> crypto::Hash {
    let pubkey_hash =
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes());
    let tx_hash = crypto::hash::hash(tag);
    utxo_set
        .insert(
            Outpoint::new(tx_hash, 0),
            UtxoEntry {
                output: Output::normal(amount, pubkey_hash),
                height: 1,
                is_coinbase: false,
                is_epoch_reward: false,
            },
        )
        .expect("utxo insert");
    tx_hash
}

/// Build a fully valid `TxType::Registration` spending `funding`.
///
/// `Transaction::new_registration` (crates/core/src/transaction/core.rs:207) produces
/// the bincode-encoded `RegistrationData` but leaves `bls_pubkey` / `bls_pop` EMPTY,
/// which `registration.rs:144-153` rejects. The `mempool` crate does not depend on
/// `bincode` (crates/mempool/Cargo.toml) and no dependency may be added here, so the two
/// trailing empty-`Vec<u8>` length prefixes are replaced in place. bincode 1.x encodes a
/// `Vec<u8>` as a fixint LE `u64` length followed by the bytes, so the last 16 bytes of
/// the base encoding are two zero-length prefixes.
///
/// This is self-checking: if the encoding were wrong, `bincode::deserialize` inside
/// `validate_registration_data` fails with "invalid registration data" and the
/// registration-#1-is-accepted assertion in every test below fires with that message.
fn registration_tx(funding: crypto::Hash, bls: &crypto::BlsKeyPair) -> Transaction {
    registration_tx_for(&producer_keypair(), funding, bls)
}

/// `registration_tx`, for an arbitrary producer identity.
fn registration_tx_for(
    kp: &crypto::KeyPair,
    funding: crypto::Hash,
    bls: &crypto::BlsKeyPair,
) -> Transaction {
    let mut input = Input::new(funding, 0);
    input.public_key = Some(*kp.public_key());

    // Lock slack mirrors the CLI (register.rs:106) so the known mempool-vs-validation
    // `current_height` off-by-one cannot influence this test's verdicts.
    let lock_until = HEIGHT + ConsensusParams::devnet().blocks_per_era + 1_000;
    let mut tx = Transaction::new_registration(vec![input], *kp.public_key(), BOND, lock_until, 1);

    let bls_pk = bls.public_key().as_bytes().to_vec();
    let bls_pop = bls
        .proof_of_possession()
        .expect("BLS PoP")
        .as_bytes()
        .to_vec();

    let mut extra = tx.extra_data.clone();
    assert!(
        extra.len() > 16,
        "unexpected RegistrationData encoding (len={})",
        extra.len()
    );
    assert_eq!(
        &extra[extra.len() - 16..],
        &[0u8; 16],
        "RegistrationData layout changed: the trailing 16 bytes are no longer two \
         zero-length Vec<u8> prefixes (bls_pubkey, bls_pop)"
    );
    extra.truncate(extra.len() - 16);
    extra.extend_from_slice(&(bls_pk.len() as u64).to_le_bytes());
    extra.extend_from_slice(&bls_pk);
    extra.extend_from_slice(&(bls_pop.len() as u64).to_le_bytes());
    extra.extend_from_slice(&bls_pop);
    tx.extra_data = extra;

    // Sign LAST: the signing message covers extra_data.
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&signing_hash, kp.private_key());
    }
    tx
}

/// Mirror of the block-validation context, `bins/node/src/node/validation_checks.rs:283-341`
/// — all 18 `.with_*` calls, in the same order, with devnet params.
fn block_validation_ctx(
    pending_keys: Vec<crypto::PublicKey>,
    weighted: Vec<(crypto::PublicKey, u64)>,
) -> ValidationContext {
    let p = Network::Devnet.params();
    ValidationContext::new(
        ConsensusParams::for_network(Network::Devnet),
        Network::Devnet,
        0,
        HEIGHT,
    )
    .with_prev_block(0, 0, crypto::Hash::ZERO)
    .with_producers_weighted(weighted)
    .with_pending_producer_keys(pending_keys)
    .with_bootstrap_producers(Vec::new())
    .with_bootstrap_liveness(Vec::new(), Vec::new())
    .with_epoch_producer_list(Vec::new())
    .with_sig_verification_height(p.sig_verification_height)
    .with_inc_i_026_scheduler_activation_height(p.inc_i_026_scheduler_activation_height)
    .with_fork_id(crypto::Hash::ZERO, p.fork_id_activation_height)
    .with_encrypted_content_activation_height(p.encrypted_content_activation_height)
    .with_encrypted_content_v2_activation_height(p.encrypted_content_v2_activation_height)
    .with_security_audit_activation_height(p.security_audit_activation_height)
    .with_defi_activation_height(p.defi_activation_height)
    .with_amm_activation_height(p.amm_activation_height)
    .with_inc_i_092_activation_height(p.inc_i_092_activation_height)
    .with_inc_i_096_activation_height(p.inc_i_096_activation_height)
    .with_oracle_activation_height(p.oracle_activation_height)
    .with_oracle_sunset_triggered(false)
}

/// A real `ProducerSet` in the epoch-deferral window: registration #1 has been applied,
/// so `pending_registration_keys()` — the exact value validation_checks.rs:187 reads —
/// contains `pk`, while `active_producers_at_height()` does not.
fn producer_set_with_pending_registration(pk: crypto::PublicKey) -> ProducerSet {
    let mut set = ProducerSet::new();
    let info = ProducerInfo::new(pk, HEIGHT - 1, BOND, (crypto::Hash::ZERO, 0), 0, BOND);
    set.queue_update(PendingProducerUpdate::Register {
        info: Box::new(info),
        height: HEIGHT - 1,
    });
    set
}

/// Comparable verdict. `MempoolError::Validation` is unwrapped so the mempool's
/// "validation failed: {e}" wrapper does not create a spurious string difference — the
/// comparison is on the underlying consensus error, which is the thing that must match.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Accept,
    Reject(String),
}

fn mempool_verdict(mempool: &mut Mempool, tx: &Transaction, utxo_set: &UtxoSet) -> Verdict {
    match mempool.add_transaction(tx.clone(), utxo_set, HEIGHT) {
        Ok(_) => Verdict::Accept,
        Err(MempoolError::Validation(e)) => Verdict::Reject(e.to_string()),
        Err(other) => Verdict::Reject(other.to_string()),
    }
}

fn block_verdict(tx: &Transaction, ctx: &ValidationContext) -> Verdict {
    match validate_transaction(tx, ctx) {
        Ok(()) => Verdict::Accept,
        Err(e) => Verdict::Reject(e.to_string()),
    }
}

// ---------------------------------------------------------------- tests

/// REGRESSION: regression_tests id=160 · INV-VALIDATION-001 · INC-I-147
///
/// O1 (mempool add_transaction verdict) + O2 (rejection error identity) + O3 (membership).
/// Partitions: first registration (valid) → accepted; duplicate with DISJOINT inputs
/// (invalid) → must be rejected.
///
/// Disjoint inputs are load-bearing: sharing an input makes the existing input-existence
/// check in `revalidate` evict the duplicate, and nothing reproduces (see the GS-010 note
/// in CLAUDE.md). Funding registration #2 from a separate confirmed UTXO is exactly what
/// the gauntlet scenario and the four measured runs do.
///
/// PRE-FIX EXPECTATION: this test FAILS — `Mempool::add_transaction` (pool.rs:291-311)
/// makes 11 `.with_*` calls, none of them `with_pending_producer_keys`, so
/// `registration.rs:173` evaluates `Vec::new().contains(&P)` == false and the duplicate
/// is admitted.
#[test]
fn duplicate_registration_rejected_at_admission() {
    let mut mempool = devnet_mempool();
    let mut utxo_set = UtxoSet::new();
    let bls = bls_keypair();

    let f1 = fund(&mut utxo_set, b"inc-i-147-funding-1", FUNDING);
    let f2 = fund(&mut utxo_set, b"inc-i-147-funding-2", FUNDING);

    let reg1 = registration_tx(f1, &bls);
    let reg2 = registration_tx(f2, &bls);

    // Same producer pubkey, different transaction, DISJOINT inputs.
    assert_ne!(
        reg1.hash(),
        reg2.hash(),
        "the two registrations must differ"
    );
    let inputs1: Vec<_> = reg1
        .inputs
        .iter()
        .map(|i| (i.prev_tx_hash, i.output_index))
        .collect();
    for i in &reg2.inputs {
        assert!(
            !inputs1.contains(&(i.prev_tx_hash, i.output_index)),
            "registration #2 must fund from DISJOINT inputs"
        );
    }

    // Partition 1 — first registration is valid and must be admitted. This also proves
    // the harness builds a well-formed registration (bond, lock, BLS PoP, chain fields).
    mempool
        .add_transaction(reg1.clone(), &utxo_set, HEIGHT)
        .expect("registration #1 must be admitted (harness self-check)");
    assert!(mempool.contains(&reg1.hash()));

    // Partition 2 — P now has an accepted-but-not-active registration. A second one is
    // rejected by block validation (registration.rs:173); admission MUST agree.
    let result = mempool.add_transaction(reg2.clone(), &utxo_set, HEIGHT);

    // O1
    let err = result.err().unwrap_or_else(|| {
        panic!(
            "INC-I-147: the mempool ACCEPTED a second registration for a pubkey that \
             already has a pending registration. Block validation rejects this \
             transaction with InvalidRegistration(\"producer already has a pending \
             registration\"), so every holding producer burns a slot when scheduled. \
             mempool.len()={}",
            mempool.len()
        )
    });

    // O2 — same error identity as block validation, not merely "some rejection".
    let msg = match &err {
        MempoolError::Validation(e) => e.to_string(),
        other => other.to_string(),
    };
    assert!(
        msg.contains("pending registration"),
        "rejected, but not with the block-validation error. \
         expected an InvalidRegistration mentioning \"producer already has a pending \
         registration\", got: {msg}"
    );

    // O3
    assert!(
        !mempool.contains(&reg2.hash()),
        "the duplicate must not be retained"
    );
    assert_eq!(mempool.len(), 1, "only registration #1 may remain");
}

/// REGRESSION: regression_tests id=161 · INV-VALIDATION-001 · INC-I-147
///
/// The parity lock. Drives the SAME transaction through the mempool-admission path and
/// the block-validation path and asserts IDENTICAL verdicts, across three partitions.
/// This is what stops future context drift (a new consensus check wired into only one of
/// the four `ValidationContext` construction sites).
///
/// PRE-FIX EXPECTATION: partitions A and C agree (pass); partition B DISAGREES —
/// mempool=Accept, block validation=Reject — so this test FAILS.
#[test]
fn mempool_and_apply_agree_on_every_registration_verdict() {
    let bls = bls_keypair();
    let pk = producer_pubkey();

    // ---- Partition A: first registration, nothing pending, nothing active. -----------
    {
        let mut mempool = devnet_mempool();
        let mut utxo_set = UtxoSet::new();
        let f1 = fund(&mut utxo_set, b"inc-i-147-parity-a", FUNDING);
        let reg = registration_tx(f1, &bls);

        let empty_set = ProducerSet::new();
        let ctx = block_validation_ctx(empty_set.pending_registration_keys(), Vec::new());

        let block = block_verdict(&reg, &ctx);
        let pool = mempool_verdict(&mut mempool, &reg, &utxo_set);

        assert_eq!(
            block,
            Verdict::Accept,
            "partition A: block validation must accept a first registration"
        );
        assert_eq!(
            pool, block,
            "partition A: mempool and block validation disagree on a VALID registration"
        );
    }

    // ---- Partition B (control): P is an ACTIVE producer. -----------------------------
    // The mempool DOES receive the active-producer snapshot (AUDIT-P1-001, pool.rs:311),
    // so this partition must already agree. It runs BEFORE the defect partition on
    // purpose: it proves the parity harness itself is sound and isolates the failure in
    // partition C to the pending-keys field alone.
    {
        let mut mempool = devnet_mempool();
        let snapshot: Arc<RwLock<Vec<(crypto::PublicKey, u64)>>> =
            Arc::new(RwLock::new(vec![(pk, 1)]));
        mempool.share_active_producers_weighted(snapshot);

        let mut utxo_set = UtxoSet::new();
        let f1 = fund(&mut utxo_set, b"inc-i-147-parity-b", FUNDING);
        let reg = registration_tx(f1, &bls);

        let ctx = block_validation_ctx(Vec::new(), vec![(pk, 1)]);

        let block = block_verdict(&reg, &ctx);
        let pool = mempool_verdict(&mut mempool, &reg, &utxo_set);

        assert_eq!(
            block,
            Verdict::Reject("invalid registration: producer already registered".to_string()),
            "harness: block validation must reject a registration for an ACTIVE producer"
        );
        assert_eq!(
            pool, block,
            "partition B: mempool and block validation disagree for an ACTIVE producer"
        );
    }

    // ---- Partition C: P has a pending (epoch-deferred) registration. -----------------
    // Block side: a real ProducerSet holding PendingProducerUpdate::Register — the exact
    // value validation_checks.rs:187 feeds to with_pending_producer_keys.
    // Mempool side: registration #1 already admitted. Same fact, each path's own
    // representation (the mempool has no channel for the ProducerSet value — that is the
    // defect).
    {
        let mut mempool = devnet_mempool();
        let mut utxo_set = UtxoSet::new();
        let f1 = fund(&mut utxo_set, b"inc-i-147-parity-c1", FUNDING);
        let f2 = fund(&mut utxo_set, b"inc-i-147-parity-c2", FUNDING);
        let reg1 = registration_tx(f1, &bls);
        let reg2 = registration_tx(f2, &bls);

        mempool
            .add_transaction(reg1, &utxo_set, HEIGHT)
            .expect("registration #1 must be admitted (harness self-check)");

        let producers = producer_set_with_pending_registration(pk);
        let pending = producers.pending_registration_keys();
        assert_eq!(
            pending,
            vec![pk],
            "harness: ProducerSet must expose P as a pending registration"
        );
        let ctx = block_validation_ctx(pending, Vec::new());

        let block = block_verdict(&reg2, &ctx);
        let pool = mempool_verdict(&mut mempool, &reg2, &utxo_set);

        assert_eq!(
            block,
            Verdict::Reject(
                "invalid registration: producer already has a pending registration".to_string()
            ),
            "harness: block validation must reject the duplicate (registration.rs:173). \
             If this line fails the ValidationError Display changed — update the expected \
             string, do NOT weaken the parity assertion below."
        );
        assert_eq!(
            pool, block,
            "INC-I-147 PARITY VIOLATION: the same registration gets different verdicts. \
             mempool={pool:?} block_validation={block:?}. \
             `with_pending_producer_keys` has exactly one call site \
             (validation_checks.rs:291); the mempool never calls it, so \
             ctx.pending_producer_keys is Vec::new() at admission and \
             registration.rs:173 is a guaranteed no-op."
        );
    }
}

/// INV-VALIDATION-001 (eviction half) · INC-I-147
///
/// The half that explains the seed holding 4 toxic registration TXs for 49-102 minutes
/// across 577 revalidate passes: a non-producer never builds a block, so poisoning is not
/// available as a shedding mechanism, and `revalidate` re-runs exactly ONE check — input
/// existence (pool.rs:1086-1112). A registration that was legitimately admitted and then
/// became invalid is retained forever.
///
/// Sequence: admit while nothing is pending/active (legitimate) → registration #1 mines
/// and its producer becomes known to the mempool → revalidate → the now-invalid TX must
/// be EVICTED.
///
/// PRE-FIX EXPECTATION: this test FAILS — the transaction survives, because all of its
/// inputs still exist and `revalidate` builds no ValidationContext and ignores its
/// `_current_height` argument.
#[test]
fn revalidate_evicts_registration_that_became_duplicate() {
    let mut mempool = devnet_mempool();

    // The mempool's only existing shared-state channel. Empty at admission time.
    let snapshot: Arc<RwLock<Vec<(crypto::PublicKey, u64)>>> = Arc::new(RwLock::new(Vec::new()));
    mempool.share_active_producers_weighted(snapshot.clone());

    let mut utxo_set = UtxoSet::new();
    let bls = bls_keypair();
    let f = fund(&mut utxo_set, b"inc-i-147-revalidate", FUNDING);
    let reg = registration_tx(f, &bls);
    let reg_hash = reg.hash();

    // Legitimately admitted: at this instant P has no registration anywhere.
    mempool
        .add_transaction(reg.clone(), &utxo_set, HEIGHT)
        .expect("registration must be admitted while no other registration exists");
    assert!(mempool.contains(&reg_hash));

    // Registration #1 mines; P is now a known producer. The held TX is now invalid —
    // block validation would reject it with InvalidRegistration.
    *snapshot.write().expect("snapshot lock") = vec![(producer_pubkey(), 1)];

    // Its inputs are all still present: eviction cannot come from the input-existence
    // check, which is the ONLY check revalidate performs today.
    for input in &reg.inputs {
        assert!(
            utxo_set
                .get(&Outpoint::new(input.prev_tx_hash, input.output_index))
                .is_some(),
            "harness: inputs must still exist so the input-existence check cannot evict"
        );
    }

    mempool.revalidate(&utxo_set, HEIGHT);

    // O3
    assert!(
        !mempool.contains(&reg_hash),
        "INC-I-147: a registration that became invalid SURVIVED revalidate. \
         `Mempool::revalidate` (pool.rs:1086-1112) re-runs only input existence, builds \
         no ValidationContext and ignores its height argument, so a non-producer holds \
         the toxic TX indefinitely (measured: 4 TXs, 49-102 min, 577 revalidate passes)."
    );
    assert_eq!(mempool.len(), 0, "the mempool must be empty after eviction");
}

// OUTPUT CONTRACT: fn legitimate_registration_admitted_and_survives_revalidate
// O1: mempool add_transaction accept/reject verdict for a NON-duplicate registration
// O2: mempool membership immediately after admission
// O3: mempool membership after revalidate
// PATHS: mempool admission / revalidate
// INPUT PARTITIONS:
//   P1 first-time registration, nothing pending and nothing active anywhere
//   P2 same registration, after a revalidate pass with all inputs intact
//   P3 a SECOND, DIFFERENT producer registering while producer #1's registration
//      is mempool-resident, and while an UNRELATED third producer is active
// MATRIX: 3 outputs x 2 paths x 3 partitions
//
/// POSITIVE CONTROL for the INC-I-147 residual fix · INV-VALIDATION-001.
///
/// The three RED tests above can all be turned green by a fix that rejects or
/// evicts every registration. This test is the counterweight: it fails on any
/// over-rejecting fix, and it must pass BOTH before and after the fix (pre-fix
/// nothing rejects, so it is green; post-fix it proves the new rejection is
/// scoped to genuine duplicates).
///
/// P3 is the discriminating partition. "Pending" must be keyed on the
/// registration's OWN pubkey (`RegistrationData.public_key`,
/// crates/core/src/transaction/core.rs:449) — a fix that rejects whenever the
/// pending set is merely non-empty, or that lets a transaction count itself as
/// its own prior registration, fails here while still passing all three RED
/// tests.
#[test]
fn legitimate_registration_admitted_and_survives_revalidate() {
    let mut mempool = devnet_mempool();

    // An unrelated ACTIVE producer: being active must not block anyone else.
    let unrelated_active = keypair_seeded(21);
    let snapshot: Arc<RwLock<Vec<(crypto::PublicKey, u64)>>> =
        Arc::new(RwLock::new(vec![(*unrelated_active.public_key(), 1)]));
    mempool.share_active_producers_weighted(snapshot);

    let mut utxo_set = UtxoSet::new();

    // ---- P1: first-time registration for producer #1 must be ADMITTED. -------------
    let p1 = producer_keypair();
    let bls1 = bls_keypair();
    let f1 = fund(&mut utxo_set, b"inc-i-147-control-p1", FUNDING);
    let reg1 = registration_tx_for(&p1, f1, &bls1);
    let reg1_hash = reg1.hash();

    // O1
    mempool
        .add_transaction(reg1.clone(), &utxo_set, HEIGHT)
        .expect(
            "OVER-REJECTION: a first-time registration with nothing pending and nothing \
             active was refused admission. The INC-I-147 fix must reject duplicates, not \
             registrations.",
        );
    // O2
    assert!(
        mempool.contains(&reg1_hash),
        "registration #1 must be retained"
    );

    // ---- P3: a DIFFERENT producer registers while #1 is mempool-resident. ----------
    let p2 = keypair_seeded(9);
    let bls2 = bls_keypair_seeded(11);
    let f2 = fund_for(&mut utxo_set, &p2, b"inc-i-147-control-p3", FUNDING);
    let reg2 = registration_tx_for(&p2, f2, &bls2);
    let reg2_hash = reg2.hash();
    assert_ne!(
        p1.public_key(),
        p2.public_key(),
        "harness: the two registrations must be for DIFFERENT producers"
    );

    // O1
    mempool
        .add_transaction(reg2.clone(), &utxo_set, HEIGHT)
        .expect(
            "OVER-REJECTION: producer #2's first registration was refused because producer \
         #1 has a pending registration. `pending_producer_keys` is consumed by a \
         `contains(&reg_data.public_key)` test (registration.rs:173) — it must be keyed \
         on the registering pubkey, not on emptiness of the pending set.",
        );
    // O2
    assert!(
        mempool.contains(&reg2_hash),
        "registration #2 must be retained"
    );
    assert_eq!(mempool.len(), 2, "both distinct registrations must be held");

    // ---- P2: both must SURVIVE revalidate. -----------------------------------------
    // All inputs still exist and neither producer is active or pending anywhere
    // outside the mempool, so nothing legitimises eviction. A revalidate pass that
    // lets a transaction see ITSELF as a prior pending registration evicts both here.
    for tx in [&reg1, &reg2] {
        for input in &tx.inputs {
            assert!(
                utxo_set
                    .get(&Outpoint::new(input.prev_tx_hash, input.output_index))
                    .is_some(),
                "harness: inputs must still exist"
            );
        }
    }

    mempool.revalidate(&utxo_set, HEIGHT);

    // O3
    assert!(
        mempool.contains(&reg1_hash),
        "OVER-EVICTION: a valid registration was dropped by revalidate. A transaction \
         must not count itself as its own prior pending registration."
    );
    assert!(
        mempool.contains(&reg2_hash),
        "OVER-EVICTION: a valid registration for a second, distinct producer was dropped \
         by revalidate."
    );
    assert_eq!(mempool.len(), 2, "revalidate must evict neither");
}
