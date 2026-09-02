//! INC-I-203 M2 — RED. Admission and residency, the halves M1 did not close.
//!
//! covers: crates/mempool/src/pool.rs, addbond_cap.rs, holdings.rs
//!
//! Analysis: `docs/bugfixes/inc-i-203-analysis.md` §C, §D, §F, §G.
//! RED evidence: `docs/.workflow/inc-i-203-M2-test-red-evidence.txt`.
//!
//! ===========================================================================
//! THIS FILE IS RUNTIME-RED ON PURPOSE
//! ===========================================================================
//! M1 stopped a producer PACKING an over-cap AddBond. It did not stop the
//! transaction ENTERING the mempool. `add_transaction` never consults
//! `addbond_cap_verdict`, `revalidate` never sheds an AddBond the ledger moved
//! out from under, and `is_outpoint_spent` keeps the submitter's funding
//! outpoint hidden from `getUtxos` / `getBalance` for the full `max_age`.
//! Proven live: producer_5 Spendable 19363.8 → 16361.4 DOLI after one submit.
//!
//! Network selects the band: mainnet and testnet pin
//! `addbond_cap_enforcement_activation_height` to `0` (`defaults.rs:160,449`),
//! devnet freezes it at `u64::MAX` (`:711`), so a testnet mempool is post-AH at
//! every height and a devnet mempool is pre-AH at every height.
//!
//! ===========================================================================
//! OUTPUT CONTRACT: fn add_transaction(...)  /  fn revalidate(...)
//! ===========================================================================
//! Functions under test:
//!   `Mempool::add_transaction(&mut self, Transaction, &UtxoSet, BlockHeight)
//!        -> Result<AddTransactionResult, MempoolError>`
//!   `Mempool::revalidate(&mut self, &UtxoSet, BlockHeight)`
//!
//! Both take `&mut self`, so the receiver is an output:
//!   O1  the `add_transaction` accept/reject verdict
//!   O2  the rejection identity — variant `MempoolError::InvalidTransaction`
//!       and the bracketed `[ADDBOND_CAP_EXCEEDED]` code the fleet greps
//!   O3  residency: `contains(&hash)` / presence in `iter()` after admission
//!   O4  residency after `revalidate` — the eviction half
//!   O5  `len()` — an over-rejecting or over-evicting fix moves this
//!   O6  the mempool-spent outpoint view: `is_outpoint_spent(op)` and the
//!       `outgoing` term of `calculate_unconfirmed_balance`. This is the
//!       user-visible harm; `getUtxos` (`balance.rs:75`) and Spendable
//!       (`balance.rs:36-37`) are computed from it.
//!   NOT outputs: `revalidate` returns `()`; the `UtxoSet` argument is `&`, so
//!   neither call mutates it, and neither writes to any store or channel.
//!
//! PATHS:
//!   PA-POST  `add_transaction` on a post-AH network (testnet, AH = 0)
//!   PA-PRE   `add_transaction` on a pre-AH network (devnet, AH = u64::MAX)
//!   PA-BLIND `add_transaction` post-AH with no holdings answer
//!   PR-OVER  `revalidate` after the snapshot moved a resident over the cap
//!   PR-UNDER `revalidate` with the resident still inside the cap
//!
//! INPUT PARTITIONS:
//!   IP-OVER   `bond_count + pending + requested` = 3001 (> CAP)
//!   IP-EXACT  the same sum = 3000 (the `>` boundary, must be allowed)
//!   IP-BLIND  no source wired, and a wired-but-EMPTY snapshot
//!   IP-MOVED  admitted at 2998, snapshot republished at 2999
//!
//! MATRIX: (every enumerated cell has an assertion)
//!   O1,O3,O5    × PA-POST  × IP-OVER   → req_bond_002_pool_admission_rejects…
//!   O2          × PA-POST  × IP-OVER   → req_bond_002_pool_rejection_carries…
//!   O1,O3,O5    × PA-POST  × IP-EXACT  → req_bond_002_pool_admission_rejects…
//!   O1,O3,O5    × PA-PRE   × IP-OVER   → req_bond_005_pool_admission_unchanged…
//!   O1,O3       × PA-BLIND × IP-BLIND  → req_bond_006_pool_admission_fails_open…
//!   O1,O4,O5,O6 × PR-OVER  × IP-MOVED  → req_bond_003_pool_revalidate_evicts…
//!   O1,O4,O5,O6 × PR-UNDER × IP-EXACT  → req_bond_003_pool_revalidate_keeps…
//!   measurement × PA-POST + PR-OVER    → inc_i_203_m2_probe_resident_over_cap…

// OUTPUT CONTRACT: fn add_transaction(...) / fn revalidate(...) — enumerated
// above. INPUT PARTITIONS: IP-OVER, IP-EXACT, IP-BLIND, IP-MOVED.

use std::sync::{Arc, RwLock};

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::transaction::{Input, Output, OutputType, Transaction, TxType};
use mempool::holdings::HoldingsSnapshot;
use mempool::{Mempool, MempoolError, MempoolPolicy, ProducerHoldings};
use storage::{Outpoint, UtxoEntry, UtxoSet};

/// `MAX_BONDS_PER_PRODUCER` (`consensus/constants.rs`).
const CAP: u32 = doli_core::MAX_BONDS_PER_PRODUCER;

/// Testnet pins the gate to 0, so every height is post-AH; devnet freezes it at
/// `u64::MAX`, so every height is pre-AH. One height serves both bands.
const HEIGHT: u64 = 100_007;

/// Flat fee left over the bond amount. `min_fee_rate` is 0 on every policy, so
/// this only has to be non-zero.
const FEE: u64 = 1_000;

// ─────────────────────────────────────────────────────────────────── fixture

fn addr(pk: &PublicKey) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes())
}

/// One submitter, one funding outpoint, one signed AddBond, one holdings
/// snapshot. `tag` keeps the funding hashes distinct across cases.
struct Case {
    utxo: UtxoSet,
    snapshot: HoldingsSnapshot,
    tx: Transaction,
    subject: Hash,
    funding: Outpoint,
    owner: Hash,
}

fn case(network: Network, bond_count: u32, requested: u32, tag: u8) -> Case {
    let kp = KeyPair::from_seed([tag; 32]);
    let pk = *kp.public_key();
    let unit = network.bond_unit();
    let bond_amount = unit * requested as u64;

    let mut utxo = UtxoSet::new();
    let funding = Outpoint::new(Hash::from_bytes([tag ^ 0x5A; 32]), 0);
    utxo.insert(
        funding,
        UtxoEntry {
            output: Output::normal(bond_amount + FEE, addr(&pk)),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        },
    )
    .expect("fixture: fund the AddBond");

    let mut inp = Input::new(funding.tx_hash, funding.index);
    inp.public_key = Some(pk);
    let mut tx = Transaction::new_add_bond(vec![inp], pk, requested, bond_amount, u64::MAX);
    let signing_hash = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&signing_hash, kp.private_key());

    let snapshot: HoldingsSnapshot = Arc::new(RwLock::new(vec![(
        pk,
        ProducerHoldings {
            bond_count,
            pending_addbond: 0,
            withdrawal_pending: 0,
        },
    )]));

    let subject = tx.hash();
    Case {
        utxo,
        snapshot,
        tx,
        subject,
        funding,
        owner: addr(&pk),
    }
}

fn mempool_for(network: Network) -> Mempool {
    match network {
        Network::Devnet => Mempool::new(
            MempoolPolicy::testnet(),
            ConsensusParams::devnet(),
            Network::Devnet,
        ),
        _ => Mempool::testnet(),
    }
}

fn wired(network: Network, case: &Case) -> Mempool {
    let mut mempool = mempool_for(network);
    mempool.share_producer_holdings(case.snapshot.clone());
    mempool
}

fn republish(case: &Case, bond_count: u32) {
    let mut guard = case.snapshot.write().expect("snapshot lock");
    for (_, h) in guard.iter_mut() {
        h.bond_count = bond_count;
    }
}

fn offer(mempool: &mut Mempool, case: &Case, height: u64) -> Result<(), MempoolError> {
    mempool
        .add_transaction(case.tx.clone(), &case.utxo, height)
        .map(|_| ())
}

fn in_iter(mempool: &Mempool, hash: &Hash) -> bool {
    mempool.iter().any(|(h, _)| h == hash)
}

/// The `outgoing` term `getBalance` subtracts from Spendable (`balance.rs:36`).
fn mempool_outgoing(mempool: &Mempool, case: &Case) -> u64 {
    mempool
        .calculate_unconfirmed_balance(&case.owner, &case.utxo)
        .1
}

/// Count over-cap AddBonds the mempool still HOLDS, resolved against the
/// snapshot the way the gate resolves them. This is the probe's metric.
fn resident_over_cap(mempool: &Mempool, snapshot: &HoldingsSnapshot) -> usize {
    let held = snapshot.read().expect("snapshot lock").clone();
    mempool
        .iter()
        .filter(|(_, entry)| entry.tx.tx_type == TxType::AddBond)
        .filter(|(_, entry)| {
            let Some(ab) = entry.tx.add_bond_data() else {
                return false;
            };
            let Some((_, h)) = held.iter().find(|(k, _)| *k == ab.producer_pubkey) else {
                return false;
            };
            let requested = entry
                .tx
                .outputs
                .iter()
                .filter(|o| o.output_type == OutputType::Bond)
                .count() as u32;
            h.bond_count
                .saturating_add(h.pending_addbond)
                .saturating_add(requested)
                > CAP
        })
        .count()
}

// ═══════════════════════════════════════════════════════════════════════════
// PA-POST — admission
// ═══════════════════════════════════════════════════════════════════════════

/// REQ-BOND-002 — Decision: a failure tells us `add_transaction` still admits,
/// gossips and stores an AddBond that `validate_block_economics` will reject, so
/// the submitter's funding outpoint stays hidden from `getUtxos` for the full
/// `max_age` (14 days) while every producer that selects it burns a slot.
///
/// **RED today**: `add_transaction` has no AddBond arm at all.
#[test]
fn req_bond_002_pool_admission_rejects_over_cap_addbond() {
    // IP-OVER: 2999 held + 0 pending + 2 requested = 3001 > 3000.
    let c = case(Network::Testnet, CAP - 1, 2, 0x21);
    let mut mempool = wired(Network::Testnet, &c);

    // O1
    let err = offer(&mut mempool, &c, HEIGHT).expect_err(
        "REQ-BOND-002: the mempool ADMITTED an AddBond that raises the producer \
         to 3001 bonds at h=100007, post-AH on testnet (AH = 0). The gate refuses \
         the block that carries it, so the transaction never confirms, no fee is \
         paid, no input is spent — and the submitter's funding UTXO is filtered \
         out of getUtxos for max_age.",
    );
    // O3
    assert!(
        !mempool.contains(&c.subject),
        "O3: a rejected AddBond must not be retained"
    );
    assert!(
        !in_iter(&mempool, &c.subject),
        "O3: the rejected AddBond must be absent from iter(), which is what the \
         builder selects from and what getMempool reports. got err: {err}"
    );
    // O5
    assert_eq!(mempool.len(), 0, "O5: nothing may remain");

    // IP-EXACT boundary: 2998 + 2 == 3000 is NOT `> 3000`. Over-rejection here
    // is censorship — the gate allows filling the cap exactly.
    let ok = case(Network::Testnet, CAP - 2, 2, 0x22);
    let mut mempool = wired(Network::Testnet, &ok);
    let verdict = offer(&mut mempool, &ok, HEIGHT);
    assert!(
        verdict.is_ok(),
        "OVER-REJECTION: filling the cap exactly (2998 + 2 = 3000) must be \
         admitted. The comparison is `>`, not `>=`. got: {:?}",
        verdict.unwrap_err()
    );
    assert!(mempool.contains(&ok.subject), "O3: the legal AddBond stays");
    assert_eq!(mempool.len(), 1, "O5");
}

/// REQ-BOND-002 — Decision: a failure tells us the rejection reached the RPC
/// caller as an opaque error, so a producer cannot tell "you are at the cap"
/// from "your transaction is malformed", and the fleet's `ADDBOND_CAP_EXCEEDED`
/// grep finds nothing.
///
/// No `crates/rpc` harness is added: `send_transaction`
/// (`transaction.rs:218-225`) is `add_transaction(...).map_err(...)?`, so the
/// `?` returns before `(self.broadcast_tx)(tx)` at :225, and
/// `MempoolError::InvalidTransaction` is `#[error("invalid transaction: {0}")]`
/// whose `to_structured_json` copies the whole message into `detail`. Asserting
/// the variant and the code at the mempool boundary settles both RPC criteria.
///
/// **RED today**: no rejection is produced to inspect.
#[test]
fn req_bond_002_pool_rejection_carries_the_addbond_cap_grep_code() {
    let c = case(Network::Testnet, CAP - 1, 2, 0x23);
    let mut mempool = wired(Network::Testnet, &c);

    let err = offer(&mut mempool, &c, HEIGHT)
        .expect_err("REQ-BOND-002: an over-cap AddBond must be refused at admission");

    // O2 — variant, matching the `withdrawal_holdings_verdict` precedent at
    // pool.rs:573 (`.map_err(MempoolError::InvalidTransaction)?`).
    assert!(
        matches!(err, MempoolError::InvalidTransaction(_)),
        "O2: the rejection must be MempoolError::InvalidTransaction so the RPC \
         layer's existing Display + to_structured_json chain carries the reason \
         to the caller unchanged. got: {err:?}"
    );
    // O2 — the code, not merely "some rejection".
    let msg = err.to_string();
    assert!(
        msg.contains("ADDBOND_CAP_EXCEEDED"),
        "O2: the message must carry the bracketed code `ValidationError::code()` \
         emits for this variant (`validation/error.rs:501`) — the fleet greps \
         codes, not prose. got: {msg}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// PA-PRE — the pre-activation band must not move
// ═══════════════════════════════════════════════════════════════════════════

/// REQ-BOND-005 — Decision: a failure tells us the new filter is height-blind,
/// so it changed admission on a band whose AH is `u64::MAX` and whose history
/// the gate never policed — a behaviour change nobody asked for.
///
/// **GREEN today, must STAY green.**
#[test]
fn req_bond_005_pool_admission_unchanged_below_activation_height() {
    let c = case(Network::Devnet, CAP - 1, 2, 0x24);
    let mut mempool = wired(Network::Devnet, &c);

    let verdict = offer(&mut mempool, &c, HEIGHT);
    assert!(
        verdict.is_ok(),
        "REQ-BOND-005: devnet freezes addbond_cap_enforcement_activation_height \
         at u64::MAX, so the gate is a no-op there and the filter must be one \
         too. got: {:?}",
        verdict.unwrap_err()
    );
    assert!(
        mempool.contains(&c.subject),
        "O3: pre-AH residency is unchanged"
    );
    assert_eq!(mempool.len(), 1, "O5");

    // And it must survive revalidate on the same band.
    republish(&c, CAP);
    mempool.revalidate(&c.utxo, HEIGHT);
    assert!(
        mempool.contains(&c.subject),
        "REQ-BOND-005: eviction is height-gated too. Below the AH nothing may be \
         shed for a cap the gate does not enforce."
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// PA-BLIND — fail OPEN when no source answered
// ═══════════════════════════════════════════════════════════════════════════

/// REQ-BOND-006 — Decision: a failure tells us the filter fails CLOSED, which
/// censors every producer while the holdings source is unwired or contended —
/// and `new_for_test` / `new_for_replay` nodes carry no source at all.
/// Under-rejection is still caught by the builder (M1) and by consensus.
///
/// **GREEN today, must STAY green.**
#[test]
fn req_bond_006_pool_admission_fails_open_when_holdings_unavailable() {
    // IP-BLIND (a): no source wired at all.
    let c = case(Network::Testnet, CAP - 1, 2, 0x25);
    let mut mempool = mempool_for(Network::Testnet);
    let verdict = offer(&mut mempool, &c, HEIGHT);
    assert!(
        verdict.is_ok(),
        "REQ-BOND-006 FAIL-CLOSED: with NO holdings source wired every lookup is \
         `Unavailable`, which `holdings.rs:9-11` fixes to mean SKIP THE CHECK. \
         got: {:?}",
        verdict.unwrap_err()
    );
    assert!(mempool.contains(&c.subject), "O3");

    // IP-BLIND (b): a wired but EMPTY snapshot. `HoldingsSources::lookup`
    // returns `Unavailable`, not `Unregistered`, for this case on purpose.
    let c = case(Network::Testnet, CAP - 1, 2, 0x26);
    {
        c.snapshot.write().expect("snapshot lock").clear();
    }
    let mut mempool = wired(Network::Testnet, &c);
    let verdict = offer(&mut mempool, &c, HEIGHT);
    assert!(
        verdict.is_ok(),
        "REQ-BOND-006 FAIL-CLOSED: an EMPTY snapshot is no answer at all — only \
         Node::new seeds it, so rejecting here refuses every AddBond on a \
         new_for_test / new_for_replay node. got: {:?}",
        verdict.unwrap_err()
    );
    assert!(mempool.contains(&c.subject), "O3");

    // And revalidate must not shed it either.
    mempool.revalidate(&c.utxo, HEIGHT);
    assert!(
        mempool.contains(&c.subject),
        "REQ-BOND-006: eviction must fail open on the same terms as admission"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// PR — revalidate must evict, and must release the inputs
// ═══════════════════════════════════════════════════════════════════════════

/// REQ-BOND-003 — Decision: a failure tells us an AddBond that was legal when
/// admitted and became over-cap when the producer's other AddBond flushed at the
/// epoch boundary lives in the mempool until `max_age`, re-offered to the
/// builder every slot, with the submitter's funding outpoint hidden from
/// `getUtxos` and subtracted from Spendable the whole time. That is the
/// 19363.8 → 16361.4 DOLI freeze observed on producer_5.
///
/// The input still resolves in the UTXO set, so the existing input-existence
/// pass in `revalidate` (pool.rs:1276) cannot be what evicts.
///
/// **RED today**: `revalidate` has no AddBond arm.
#[test]
fn req_bond_003_pool_revalidate_evicts_a_resident_addbond_that_became_over_cap() {
    // IP-MOVED: admitted at 2998 + 2 = 3000 (legal), then the producer's other
    // AddBond flushes and the snapshot republishes 2999 → 3001 (illegal).
    let c = case(Network::Testnet, CAP - 2, 2, 0x27);
    let mut mempool = wired(Network::Testnet, &c);

    offer(&mut mempool, &c, HEIGHT).expect(
        "harness: 2998 + 2 = 3000 is inside the cap and must be admitted, else \
         this case never reaches residency",
    );
    assert!(mempool.contains(&c.subject), "harness: it must be resident");
    assert!(
        mempool.is_outpoint_spent(&c.funding),
        "harness: admission must hide the funding outpoint, else the release \
         assertion below is vacuous"
    );
    let frozen = mempool_outgoing(&mempool, &c);
    assert!(
        frozen > 0,
        "harness: Spendable must be reduced while resident"
    );

    republish(&c, CAP - 1);
    assert!(
        c.utxo.get(&c.funding).is_some(),
        "harness: the input must still exist so input-existence cannot evict"
    );

    mempool.revalidate(&c.utxo, HEIGHT);

    // O4
    assert!(
        !mempool.contains(&c.subject),
        "REQ-BOND-003: an AddBond that BECAME over-cap survived revalidate. It is \
         re-offered to the builder every slot until max_age (14 days) and the \
         gate refuses every block that carries it."
    );
    assert!(!in_iter(&mempool, &c.subject), "O4: absent from iter()");
    // O5
    assert_eq!(
        mempool.len(),
        0,
        "O5: the mempool must be empty after eviction"
    );
    // O6 — the point of the eviction: the submitter gets his funds back.
    assert!(
        !mempool.is_outpoint_spent(&c.funding),
        "REQ-BOND-003: the funding outpoint is STILL reported as mempool-spent, \
         so getUtxos (balance.rs:75) still filters it out and the producer still \
         cannot spend his own coins. Eviction without input release fixes nothing \
         the submitter can see."
    );
    assert_eq!(
        mempool_outgoing(&mempool, &c),
        0,
        "REQ-BOND-003: `calculate_unconfirmed_balance` still reports the input as \
         outgoing, so Spendable (balance.rs:36-37) stays depressed by {frozen}."
    );
}

/// REQ-BOND-003 negative — Decision: a failure tells us the eviction pass is
/// over-broad and sheds AddBonds a real block would confirm, which is a
/// liveness bug the RED test above would happily accept.
///
/// **GREEN today, must STAY green.**
#[test]
fn req_bond_003_pool_revalidate_keeps_a_resident_addbond_still_within_cap() {
    let c = case(Network::Testnet, CAP - 2, 2, 0x28);
    let mut mempool = wired(Network::Testnet, &c);

    offer(&mut mempool, &c, HEIGHT).expect("harness: 2998 + 2 = 3000 is inside the cap");
    let frozen = mempool_outgoing(&mempool, &c);

    // The snapshot moves, but DOWNWARD — the producer exited some bonds.
    republish(&c, 10);
    mempool.revalidate(&c.utxo, HEIGHT);

    assert!(
        mempool.contains(&c.subject),
        "OVER-EVICTION: a within-cap AddBond (10 + 2 = 12) was shed by \
         revalidate. A mempool stricter than the gate drops transactions a real \
         block can carry."
    );
    assert_eq!(mempool.len(), 1, "O5");
    assert!(
        mempool.is_outpoint_spent(&c.funding),
        "O6: a surviving transaction must keep holding its inputs, else the \
         double-spend index is wrong"
    );
    assert_eq!(
        mempool_outgoing(&mempool, &c),
        frozen,
        "O6: Spendable must be unchanged for a surviving transaction"
    );

    // The exact boundary must survive too: 2998 + 2 = 3000 is not `> 3000`.
    republish(&c, CAP - 2);
    mempool.revalidate(&c.utxo, HEIGHT);
    assert!(
        mempool.contains(&c.subject),
        "OVER-EVICTION at the boundary: filling the cap exactly is legal"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// PROBE — the milestone's outcome metric. Asserts nothing.
// ═══════════════════════════════════════════════════════════════════════════

/// Measurement, not a gate: it must run before AND after the fix and report a
/// different number. Today `1` (admitted and never shed); after the fix `0`.
///
/// The `#[test]` wrapper lives at the binary root (`main.rs`) because libtest's
/// `--exact` matches the FULL `module::fn` name, and the milestone's probe
/// command addresses it bare. The measurement is here.
pub(crate) fn probe_resident_over_cap_addbonds() -> usize {
    let c = case(Network::Testnet, CAP - 1, 2, 0x29);
    let mut mempool = wired(Network::Testnet, &c);

    let _ = mempool.add_transaction(c.tx.clone(), &c.utxo, HEIGHT);
    mempool.revalidate(&c.utxo, HEIGHT);

    resident_over_cap(&mempool, &c.snapshot)
}
