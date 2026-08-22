//! INC-I-180 M2 / S2 — mempool admission parity for the withdrawal-holdings
//! gate. INV-VALIDATION-001, Reviewer F1 (admission side), OBS-001.
//!
//! covers: pool.rs, assembly.rs, production/mod.rs, validation_checks.rs,
//!         rewards.rs
//!
//! ---------------------------------------------------------------------------
//! THIS FILE IS COMPILE-RED ON PURPOSE
//! ---------------------------------------------------------------------------
//! Diagnosis constraint C8: the mempool `ValidationContext` carries no bond
//! fields today, and the node feeds the mempool through discrete setters only —
//! `share_oracle_sunset_flag` (pool.rs:235), `share_active_producers_weighted`
//! (:250), `share_pending_producer_keys` (:271). There is NO `ProducerSet`
//! handle, so admission cannot evaluate the R1 allowance formula at all. The
//! tests below name the channel that must exist. INC-I-147 is the precedent for
//! exactly this shape.
//!
//! REQUIRED API (this is the contract, not a suggestion):
//!
//! ```ignore
//! // crates/mempool/src/ (new module, re-exported from lib.rs)
//! #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
//! pub struct ProducerHoldings {
//!     pub bond_count: u32,
//!     pub pending_addbond: u32,
//!     pub withdrawal_pending: u32,
//! }
//!
//! impl Mempool {
//!     /// Node-published snapshot of every registered producer's holdings.
//!     /// Absence from the snapshot means "not a registered producer" (R0).
//!     pub fn share_producer_holdings(
//!         &mut self,
//!         snapshot: Arc<RwLock<Vec<(PublicKey, ProducerHoldings)>>>,
//!     );
//! }
//! ```
//!
//! The producer-side publication (a `refresh_*` that fills this from the live
//! `ProducerSet` at every Node construction site) is the node's half and is
//! covered by `bins/node/tests/it/inc_i_180_builder_parity.rs`, which drives the
//! node's own mempool.
//!
//! ---------------------------------------------------------------------------
//! CONTAINMENT RELATION (stated explicitly, per the brief)
//! ---------------------------------------------------------------------------
//! The mempool sees ONE transaction against CURRENT state. It cannot know block
//! composition, so R4 and every `in_block_*` term are not checkable there.
//!
//!   mempool-reject  ⊆  builder-skip  ⊆  consensus-reject
//!
//! Being WEAKER than the builder is correct: a stronger mempool would evict
//! transactions a real block could legitimately carry. What is forbidden is the
//! other direction — admitting a transaction the gate rejects for a reason that
//! IS decidable from single-tx current state (R0, R1, R3, and the R2 split with
//! `in_block_* = 0`). `single_tx_gate_reference` below is an independent
//! re-implementation of that decidable subset of the M1 rule table, and the
//! containment test asserts admission never disagrees with it.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT
//! ---------------------------------------------------------------------------
//! Functions under test:
//!   `Mempool::add_transaction(&mut self, Transaction, &UtxoSet, BlockHeight)
//!        -> Result<AddTransactionResult, MempoolError>`
//!   `Mempool::revalidate(&mut self, &UtxoSet, BlockHeight)`
//!
//! Both take `&mut self`, so the receiver is an output:
//!   O1  the `add_transaction` accept/reject verdict
//!   O2  the rejection error identity — the fleet greps the bracketed
//!       `[ECON_WITHDRAWAL_*]` codes, so "some rejection" is not the contract
//!   O3  `Mempool::contains(&tx_hash)` immediately after admission
//!   O4  `Mempool::contains(&tx_hash)` after `revalidate` (the eviction half)
//!   O5  `Mempool::len()` — an over-rejecting or over-evicting fix moves this
//!   NOT outputs: `revalidate` returns `()`; the `UtxoSet` argument is `&`, so
//!   neither call mutates it, and neither writes to any store.
//!
//! PATHS
//!   PM-POST  `add_transaction` at a height AT/ABOVE AH #23
//!   PM-PRE   `add_transaction` at a height BELOW AH #23 (devnet gate = 20)
//!   PR       `revalidate` after the holdings snapshot moved under an
//!            already-admitted transaction
//!
//! INPUT PARTITIONS
//!   IP-R0   producer absent from the holdings snapshot
//!   IP-R1   declared > bond_count + pending_addbond - withdrawal_pending
//!   IP-R3   a Bond input owned by a DIFFERENT key rides along
//!   IP-R2F  declared == allowance && declared > 0, drains fewer than the
//!           producer's owned Bond UTXOs
//!   IP-R2P  partial withdrawal whose declared count != its Bond inputs
//!   IP-R4   input references another mempool transaction — NOT decidable from
//!           single-tx current state, so admission MUST stay permissive
//!   IP-OK   well-formed withdrawal (liveness control)
//!
//! MATRIX (every enumerated cell has an assertion)
//!   O1,O2,O3,O5 × PM-POST × {R0,R1,R3,R2F,R2P}
//!        → req_i180_003_admission_rejects_every_decidable_gate_violation  [C]
//!   O1,O3,O5 × PM-POST × IP-OK
//!        → req_i180_003_admission_still_accepts_a_well_formed_withdrawal  [C]
//!   O1,O3,O5 × PM-PRE × {R0,R1,R3,R2F,R2P,IP-OK}
//!        → req_i180_003_pre_activation_admission_is_unchanged             [C]
//!   O1,O4,O5 × PR × IP-OK-turned-invalid
//!        → req_i180_003_revalidate_evicts_a_withdrawal_that_became_invalid [C]
//!   O1 × PM-POST × all partitions incl. IP-R4
//!        → req_i180_003_admission_is_contained_in_the_gate                [C]
//!   rule-table oracle × all partitions
//!        → req_i180_003_every_partition_reaches_its_rule    [C, green once it
//!          compiles — the harness self-check that keeps the partitions honest]
//!
//! [C] = compile-red: the file names `ProducerHoldings` and
//! `share_producer_holdings`, which do not exist yet.

use std::sync::{Arc, RwLock};

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::transaction::{Input, Output, OutputType, Transaction};
use mempool::{Mempool, MempoolPolicy, ProducerHoldings};
use storage::{Outpoint, UtxoEntry, UtxoSet};

/// Devnet pre-activation band: the devnet gate is pinned to 20.
const PRE_AH: u64 = 5;
/// Far above any plausible devnet gate.
const POST_AH: u64 = 1_000_007;
/// Flushed bonds held by the named producer, so `allowance == 4` by default.
const HELD: u32 = 4;

// ─────────────────────────────────────────────────────────────── fixture

fn devnet_mempool() -> Mempool {
    Mempool::new(
        MempoolPolicy::testnet(),
        ConsensusParams::devnet(),
        Network::Devnet,
    )
}

fn bond_unit() -> u64 {
    Network::Devnet.bond_unit()
}

fn addr(pk: &PublicKey) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes())
}

fn outpoints(tag: u8, count: u32) -> Vec<(Hash, u32)> {
    let h = Hash::from_bytes([tag; 32]);
    (0..count).map(|i| (h, i)).collect()
}

fn seed_bonds(utxo: &mut UtxoSet, owner: &PublicKey, tag: u8, count: u32) {
    let h = Hash::from_bytes([tag; 32]);
    for i in 0..count {
        utxo.insert(
            Outpoint::new(h, i),
            UtxoEntry {
                output: Output::bond(bond_unit(), addr(owner), u64::MAX, 0),
                height: 1,
                is_coinbase: false,
                is_epoch_reward: false,
            },
        )
        .expect("fixture: seed Bond UTXO");
    }
}

/// A `RequestWithdrawal` naming `producer`, each input carrying and signed by
/// ITS OWN owner key — the per-input ownership that makes the R3 exclusivity
/// partition signature-valid.
fn signed_withdrawal(
    producer: &PublicKey,
    declared: u32,
    spends: &[((Hash, u32), &KeyPair)],
) -> Transaction {
    let inputs: Vec<Input> = spends
        .iter()
        .map(|((h, idx), owner)| {
            let mut inp = Input::new(*h, *idx);
            inp.public_key = Some(*owner.public_key());
            inp
        })
        .collect();
    let dest = crypto::hash::hash(b"inc-i-180-m2-admission-destination");
    let net = bond_unit() * spends.len() as u64 - bond_unit() / 100;
    let mut tx = Transaction::new_request_withdrawal(inputs, *producer, declared, dest, net);
    // `spends.len() == tx.inputs.len()` by construction above; the index is the
    // pairing, not a traversal.
    #[allow(clippy::needless_range_loop)]
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature =
            crypto::signature::sign_hash(&signing_hash, spends[i].1.private_key());
    }
    tx
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    R0,
    R1,
    R3,
    R2Full,
    R2Partial,
    R4,
    Ok,
}

/// The partitions the gate decides from single-tx CURRENT state. `R4` is
/// excluded by construction — that is the containment relation.
const DECIDABLE: [Kind; 5] = [Kind::R0, Kind::R1, Kind::R3, Kind::R2Full, Kind::R2Partial];

impl Kind {
    fn code(self) -> &'static str {
        match self {
            Kind::R0 => "[ECON_WITHDRAWAL_UNKNOWN_PRODUCER]",
            Kind::R1 => "[ECON_WITHDRAWAL_OVER_HOLDINGS]",
            Kind::R3 | Kind::R2Partial => "[ECON_WITHDRAWAL_BOND_COUNT_MISMATCH]",
            Kind::R2Full => "[ECON_WITHDRAWAL_INCOMPLETE_DRAIN]",
            Kind::R4 | Kind::Ok => "",
        }
    }
}

/// One partition: the UTXO view, the published holdings snapshot, and the
/// transactions to offer in order.
struct Case {
    utxo: UtxoSet,
    holdings: Vec<(PublicKey, ProducerHoldings)>,
    txs: Vec<Transaction>,
    /// The withdrawal under test — the LAST transaction of `txs`.
    subject: Hash,
}

fn case(kind: Kind) -> Case {
    let mut utxo = UtxoSet::new();
    let p = KeyPair::from_seed([7; 32]);
    let pk = *p.public_key();
    let held = ProducerHoldings {
        bond_count: HELD,
        pending_addbond: 0,
        withdrawal_pending: 0,
    };
    let mut holdings = vec![(pk, held)];

    let txs: Vec<Transaction> = match kind {
        Kind::R0 => {
            let stranger = KeyPair::from_seed([31; 32]);
            let spk = *stranger.public_key();
            seed_bonds(&mut utxo, &spk, 0xA0, 2);
            // The stranger is NOT in the snapshot: absence IS the R0 condition.
            vec![signed_withdrawal(
                &spk,
                1,
                &[(outpoints(0xA0, 1)[0], &stranger)],
            )]
        }
        Kind::R1 => {
            seed_bonds(&mut utxo, &pk, 0xB0, 6);
            let spends: Vec<((Hash, u32), &KeyPair)> =
                outpoints(0xB0, 5).into_iter().map(|o| (o, &p)).collect();
            vec![signed_withdrawal(&pk, HELD + 1, &spends)]
        }
        Kind::R3 => {
            let foreign = KeyPair::from_seed([41; 32]);
            seed_bonds(&mut utxo, &pk, 0xD0, 1);
            seed_bonds(&mut utxo, foreign.public_key(), 0xD1, 1);
            // The foreign key is a registered producer too — the two-key actor
            // of AUDIT-P1-001. R3 must still fire: the rider is not P's.
            holdings.push((*foreign.public_key(), held));
            let spends = vec![
                (outpoints(0xD0, 1)[0], &p),
                (outpoints(0xD1, 1)[0], &foreign),
            ];
            vec![signed_withdrawal(&pk, 1, &spends)]
        }
        Kind::R2Full => {
            seed_bonds(&mut utxo, &pk, 0xE0, 6);
            let spends: Vec<((Hash, u32), &KeyPair)> =
                outpoints(0xE0, HELD).into_iter().map(|o| (o, &p)).collect();
            vec![signed_withdrawal(&pk, HELD, &spends)]
        }
        Kind::R2Partial => {
            seed_bonds(&mut utxo, &pk, 0xF0, 3);
            let spends: Vec<((Hash, u32), &KeyPair)> =
                outpoints(0xF0, 3).into_iter().map(|o| (o, &p)).collect();
            vec![signed_withdrawal(&pk, 2, &spends)]
        }
        Kind::R4 => {
            // The parent is mempool-resident and its output is ALSO a live Bond
            // UTXO, so against CURRENT state this withdrawal is flawless. Only a
            // block's composition makes it illegal, and the mempool has no view
            // of that — admission must stay permissive here.
            seed_bonds(&mut utxo, &pk, 0xC0, 1);
            let funding = Hash::from_bytes([0xC1; 32]);
            utxo.insert(
                Outpoint::new(funding, 0),
                UtxoEntry {
                    output: Output::normal(bond_unit() * 3, addr(&pk)),
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
            .expect("fixture: fund the R4 parent");
            let mut inp = Input::new(funding, 0);
            inp.public_key = Some(pk);
            let mut parent =
                Transaction::new_transfer(vec![inp], vec![Output::normal(bond_unit(), addr(&pk))]);
            let signing_hash = parent.signing_message_for_input(0);
            parent.inputs[0].signature =
                crypto::signature::sign_hash(&signing_hash, p.private_key());
            utxo.insert(
                Outpoint::new(parent.hash(), 0),
                UtxoEntry {
                    output: Output::bond(bond_unit(), addr(&pk), u64::MAX, 0),
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
            .expect("fixture: the chained outpoint must resolve pre-block");
            let wd = signed_withdrawal(
                &pk,
                2,
                &[(outpoints(0xC0, 1)[0], &p), ((parent.hash(), 0u32), &p)],
            );
            vec![parent, wd]
        }
        Kind::Ok => {
            seed_bonds(&mut utxo, &pk, 0x80, HELD);
            vec![signed_withdrawal(&pk, 1, &[(outpoints(0x80, 1)[0], &p)])]
        }
    };

    let subject = txs.last().expect("fixture: at least one tx").hash();
    Case {
        utxo,
        holdings,
        txs,
        subject,
    }
}

/// A mempool wired with the case's holdings snapshot, plus the handle so a
/// caller can move state underneath an already-admitted transaction.
#[allow(clippy::type_complexity)]
fn wired(case: &Case) -> (Mempool, Arc<RwLock<Vec<(PublicKey, ProducerHoldings)>>>) {
    let snapshot = Arc::new(RwLock::new(case.holdings.clone()));
    let mut mempool = devnet_mempool();
    mempool.share_producer_holdings(snapshot.clone());
    (mempool, snapshot)
}

fn offer(mempool: &mut Mempool, case: &Case, height: u64) -> Vec<Result<(), String>> {
    case.txs
        .iter()
        .map(|tx| {
            mempool
                .add_transaction(tx.clone(), &case.utxo, height)
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .collect()
}

/// An INDEPENDENT re-implementation of the decidable subset of the M1 rule
/// table (brief §0), evaluated against ONE transaction and CURRENT state with
/// every `in_block_*` term at zero. The containment test compares admission to
/// this oracle; it is deliberately not the production code path.
fn single_tx_gate_reference(
    tx: &Transaction,
    utxo: &UtxoSet,
    holdings: &[(PublicKey, ProducerHoldings)],
) -> Result<(), &'static str> {
    let Some(wd) = tx.withdrawal_request_data() else {
        return Ok(());
    };
    let pk = wd.producer_pubkey;
    let owner = addr(&pk);

    // R0
    let Some((_, info)) = holdings.iter().find(|(k, _)| *k == pk) else {
        return Err("[ECON_WITHDRAWAL_UNKNOWN_PRODUCER]");
    };
    // R1
    let allowance = info
        .bond_count
        .saturating_add(info.pending_addbond)
        .saturating_sub(info.withdrawal_pending);
    if wd.bond_count > allowance {
        return Err("[ECON_WITHDRAWAL_OVER_HOLDINGS]");
    }
    // R3 / R2 need the Bond-input split over the CURRENT view.
    let (mut owned, mut all_bonds) = (0u32, 0u32);
    for inp in &tx.inputs {
        let Some(entry) = utxo.get(&Outpoint::new(inp.prev_tx_hash, inp.output_index)) else {
            continue;
        };
        if entry.output.output_type != OutputType::Bond {
            continue;
        }
        all_bonds += 1;
        if entry.output.pubkey_hash == owner {
            owned += 1;
        }
    }
    if all_bonds != owned {
        return Err("[ECON_WITHDRAWAL_BOND_COUNT_MISMATCH]");
    }
    // R2 split
    if wd.bond_count == allowance && wd.bond_count > 0 {
        let live = u32::try_from(utxo.get_bond_entries(&owner).len()).unwrap_or(u32::MAX);
        if owned != live {
            return Err("[ECON_WITHDRAWAL_INCOMPLETE_DRAIN]");
        }
    } else if wd.bond_count != owned {
        return Err("[ECON_WITHDRAWAL_BOND_COUNT_MISMATCH]");
    }
    Ok(())
}

/// Harness self-check — **GREEN today, must STAY green.** Without it the RED
/// tests below stop at the first partition and the remaining six are never
/// shown to reach the state they claim; a mis-built partition would then read
/// as a fix failure. The bracketed codes are the ones
/// `validate_block_economics` raises on the SAME shapes, verified independently
/// by `bins/node/tests/it/inc_i_180_builder_parity.rs`.
#[test]
fn req_i180_003_every_partition_reaches_its_rule() {
    for kind in DECIDABLE {
        let c = case(kind);
        let subject = c.txs.last().expect("at least one tx");
        let verdict = single_tx_gate_reference(subject, &c.utxo, &c.holdings);
        assert_eq!(
            verdict,
            Err(kind.code()),
            "harness: {kind:?} must reach {} in the single-tx rule table",
            kind.code()
        );
    }
    for kind in [Kind::R4, Kind::Ok] {
        let c = case(kind);
        let subject = c.txs.last().expect("at least one tx");
        assert_eq!(
            single_tx_gate_reference(subject, &c.utxo, &c.holdings),
            Ok(()),
            "harness: {kind:?} must be INVISIBLE to a single-tx current-state oracle"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PM-POST — admission must refuse the free block-poison surface
// ═══════════════════════════════════════════════════════════════════════════

/// O1,O2,O3,O5 × PM-POST × {R0,R1,R3,R2F,R2P} — **RED (compile-red).**
///
/// Post-AH any user can submit a structurally valid, signature-valid
/// `RequestWithdrawal` the gate rejects. It never confirms, so no fee is paid
/// and no input is spent; it re-propagates forever and every producer that
/// selects it burns a build and runs `rollback_one_block()`.
#[test]
fn req_i180_003_admission_rejects_every_decidable_gate_violation() {
    for kind in DECIDABLE {
        let c = case(kind);
        let (mut mempool, _snapshot) = wired(&c);
        let verdicts = offer(&mut mempool, &c, POST_AH);

        // O1
        let msg = verdicts
            .last()
            .expect("one verdict per tx")
            .clone()
            .err()
            .unwrap_or_else(|| {
                panic!(
                    "INC-I-180 / {kind:?}: the mempool ADMITTED a withdrawal that \
                     validate_block_economics rejects with {} at h={POST_AH}. \
                     The mempool ValidationContext carries no bond fields (C8) and \
                     the node publishes no ProducerSet to it, so the allowance is \
                     not computable at admission today.",
                    kind.code()
                )
            });
        // O2 — same error identity as the gate, not merely "some rejection".
        assert!(
            msg.contains(kind.code()),
            "{kind:?}: rejected, but not with {}. got: {msg}",
            kind.code()
        );
        // O3, O5
        assert!(
            !mempool.contains(&c.subject),
            "{kind:?}: the rejected withdrawal must not be retained"
        );
        assert_eq!(
            mempool.len(),
            c.txs.len() - 1,
            "{kind:?}: only the non-withdrawal transactions may remain"
        );
    }
}

/// O1,O3,O5 × PM-POST × IP-OK — the liveness counterweight. Every RED test
/// above is satisfiable by a mempool that refuses all withdrawals.
#[test]
fn req_i180_003_admission_still_accepts_a_well_formed_withdrawal() {
    let c = case(Kind::Ok);
    let (mut mempool, _snapshot) = wired(&c);
    let verdicts = offer(&mut mempool, &c, POST_AH);
    assert!(
        verdicts[0].is_ok(),
        "OVER-REJECTION: a well-formed post-AH withdrawal (declares 1 of an \
         allowance of {HELD}, spends exactly 1 owned Bond UTXO) was refused. \
         A mempool STRONGER than the gate evicts transactions a real block can \
         carry. got: {}",
        verdicts[0].clone().unwrap_err()
    );
    assert!(mempool.contains(&c.subject));
    assert_eq!(mempool.len(), 1);
}

// ═══════════════════════════════════════════════════════════════════════════
// PM-PRE — below AH #23 admission behaviour is unchanged
// ═══════════════════════════════════════════════════════════════════════════

/// O1,O3,O5 × PM-PRE × every partition — **must be GREEN before AND after the
/// fix.** Below the gate these transactions are perfectly valid and live
/// networks confirm them; refusing them there is a behaviour change on mainnet
/// and testnet, which M1's zero-deletion proof forbids.
#[test]
fn req_i180_003_pre_activation_admission_is_unchanged() {
    for kind in [
        Kind::R0,
        Kind::R1,
        Kind::R3,
        Kind::R2Full,
        Kind::R2Partial,
        Kind::R4,
        Kind::Ok,
    ] {
        let c = case(kind);
        let (mut mempool, _snapshot) = wired(&c);
        let verdicts = offer(&mut mempool, &c, PRE_AH);
        for (i, verdict) in verdicts.iter().enumerate() {
            assert!(
                verdict.is_ok(),
                "pre-AH invariance / {kind:?}: tx {i} was refused at h={PRE_AH}, below \
                 AH #23. Admission strictness must be height-aware. got: {}",
                verdict.clone().unwrap_err()
            );
        }
        assert!(mempool.contains(&c.subject));
        assert_eq!(mempool.len(), c.txs.len());
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PR — revalidate must EVICT, not re-offer
// ═══════════════════════════════════════════════════════════════════════════

/// O1,O4,O5 × PR — **RED (compile-red).**
///
/// `revalidate` (pool.rs:1197) re-checks only input resolution and duplicate
/// registrations; it deliberately does not re-run
/// `validate_transaction_with_utxos`. A withdrawal admitted while it was legal
/// and made illegal by a later block therefore survives forever, and the
/// builder re-offers it every slot. The inputs below all still exist, so the
/// existing input-existence check cannot be what evicts.
#[test]
fn req_i180_003_revalidate_evicts_a_withdrawal_that_became_invalid() {
    let c = case(Kind::Ok);
    let (mut mempool, snapshot) = wired(&c);

    mempool
        .add_transaction(c.txs[0].clone(), &c.utxo, POST_AH)
        .expect("harness: the well-formed withdrawal must be admitted");
    assert!(mempool.contains(&c.subject));

    // State moves underneath it: the producer's bonds are gone (an Exit
    // confirmed, or an earlier withdrawal drained the allowance). The held
    // transaction now declares 1 against an allowance of 0.
    {
        let mut guard = snapshot.write().expect("snapshot lock");
        for (_, h) in guard.iter_mut() {
            h.bond_count = 0;
        }
    }
    for input in &c.txs[0].inputs {
        assert!(
            c.utxo
                .get(&Outpoint::new(input.prev_tx_hash, input.output_index))
                .is_some(),
            "harness: inputs must still exist so input-existence cannot evict"
        );
    }

    mempool.revalidate(&c.utxo, POST_AH);

    // O4, O5
    assert!(
        !mempool.contains(&c.subject),
        "INC-I-180 / OBS-001: a withdrawal that BECAME gate-invalid survived \
         revalidate. It is re-offered to the builder every slot, so one stale \
         transaction poisons every block this node builds until it expires."
    );
    assert_eq!(mempool.len(), 0, "the mempool must be empty after eviction");
}

// ═══════════════════════════════════════════════════════════════════════════
// The containment relation
// ═══════════════════════════════════════════════════════════════════════════

/// O1 × PM-POST × all partitions incl. IP-R4 — **RED (compile-red).**
///
///   admitted(tx)  ⟹  single_tx_gate_reference(tx) == Ok
///
/// Mempool WEAKER than the builder is correct and is asserted for `R4`, the one
/// partition whose illegality lives in block composition rather than in current
/// state. Mempool STRONGER than the gate is a bug and is asserted against for
/// every partition.
///
/// SCOPE, corrected after QA round 1 (ISSUE-002): `single_tx_gate_reference` is
/// a model of ADMISSION, so this row proves `mempool == mempool-model` over the
/// partitions above — real coverage, but not the `mempool-reject ⊆ builder-skip`
/// relation. That relation is FALSE for `in_block_addbond`, which raises the
/// allowance rather than lowering it, and both halves of the truth are driven
/// end to end against the live gate in `bins/node/tests/it/`:
/// `inc_i180_m2_admission_over_rejects_the_addbond_window` (the exception) and
/// `inc_i180_m2_admission_reject_implies_gate_reject_without_a_credit` (the half
/// that holds).
#[test]
fn req_i180_003_admission_is_contained_in_the_gate() {
    for kind in [
        Kind::R0,
        Kind::R1,
        Kind::R3,
        Kind::R2Full,
        Kind::R2Partial,
        Kind::R4,
        Kind::Ok,
    ] {
        let c = case(kind);
        let (mut mempool, _snapshot) = wired(&c);
        let verdicts = offer(&mut mempool, &c, POST_AH);
        let admitted = verdicts.last().expect("one verdict per tx").is_ok();
        let subject = c.txs.last().expect("at least one tx");
        let reference = single_tx_gate_reference(subject, &c.utxo, &c.holdings);

        if kind == Kind::R4 {
            assert!(
                reference.is_ok(),
                "harness: R4 must be INVISIBLE to a single-tx current-state oracle, \
                 otherwise it is not the weaker-is-correct partition"
            );
        }

        assert_eq!(
            admitted,
            reference.is_ok(),
            "CONTAINMENT / {kind:?}: admission and the single-tx rule table disagree. \
             admitted={admitted} reference={reference:?}. Admitting what the gate \
             refuses is the free block-poison surface (Reviewer F1); refusing what it \
             allows evicts transactions a real block can carry."
        );
    }
}
