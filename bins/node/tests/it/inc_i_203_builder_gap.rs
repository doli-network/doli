//! INC-I-203 M1 — RED. The block builder has no `MAX_BONDS_PER_PRODUCER` arm.
//!
//! covers: addbond_cap.rs, withdrawal_holdings.rs, lib.rs, main.rs, assembly.rs, holdings.rs, tx_types.rs, validation_checks.rs
//!
//! Analysis: `docs/bugfixes/inc-i-203-analysis.md` §C/§D/§F/§G.
//! RED evidence: `docs/.workflow/inc-i-203-M1-test-red-evidence.txt`.
//!
//! ===========================================================================
//! THE DEFECT
//! ===========================================================================
//! `check_addbond_cap` (`crates/core/src/validation/tx_types.rs:515`) has ONE
//! production caller: `validate_block_economics`
//! (`bins/node/src/node/validation_checks.rs:1212`). `select_for_block`
//! (`crates/mempool/src/pool.rs:1035`) sorts by fee rate and size; the
//! selection loop's only holdings predicate is `WithdrawalParity::allow`
//! (`production/withdrawal_holdings.rs:69`), which returns `Ok(())` for every
//! `tx_type != RequestWithdrawal`. A producer at 2999 bonds therefore packs an
//! over-cap AddBond into a block its OWN Light self-apply
//! (`production/mod.rs:620`) rejects, then rolls back and purges — the slot is
//! lost and the transaction re-propagates.
//!
//! ===========================================================================
//! HARNESS NOTE — reaching the post-activation band
//! ===========================================================================
//! `Node::new_for_test` pins `Network::Devnet`, whose
//! `addbond_cap_enforcement_activation_height` is `u64::MAX`
//! (`network_params/defaults.rs:711`), so NO height is post-AH under it. The
//! gate reads `self.config.network.params()` — not `self.params` — so the
//! band is selected by moving `config.network` to `Testnet` (AH `0`,
//! `defaults.rs:449`). `config.network` also supplies `bond_unit()`, so it is
//! moved BEFORE the ledger and the UTXOs are built. `node.params` stays
//! `ConsensusParams::devnet()`: only the AH profile is under test here.
//!
//! ===========================================================================
//! OUTPUT CONTRACT: fn over_cap_addbond_is_not_packed_at_or_above_activation_height
//! ===========================================================================
//! Function under test: `Node::build_block_content(&mut self, Hash, u32, u64,
//!   u32, PublicKey) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>`
//!   O1 returned transaction list — membership of the over-cap AddBond
//!   O2 returned `Result`/`Option` discriminant: a refusal must be a SKIP, never
//!      `Err` (build failure) and never `Ok(None)` (slot abort) — INV-PROD-002
//!   O3 receiver mutation: `node.mempool` membership after the build; selection
//!      is read-only, so a skipped tx must still be resident (eviction is M3)
//!   NOT outputs: no block is stored, no gossip is emitted, no producer or UTXO
//!   state is written by `build_block_content`.
//! PATHS: PB-POST (build at h >= AH) only; the pre-AH path is a separate test.
//! MATRIX: O1,O2,O3 × PB-POST × IP-OVER → this test. [RED]
//!
//! OUTPUT CONTRACT: fn below_activation_height_over_cap_addbond_is_still_packed
//! Function under test: `Node::build_block_content` (as above).
//!   O1 returned transaction list — the over-cap AddBond must be PRESENT
//!   O2 `Result`/`Option` discriminant — build must succeed
//! PATHS: PB-PRE (devnet profile, AH = u64::MAX, so every height is pre-AH).
//! MATRIX: O1,O2 × PB-PRE × IP-OVER → this test. [GREEN, must stay green]
//!
//! OUTPUT CONTRACT: fn unavailable_holdings_fails_open_and_packs
//! Functions under test:
//!   `Mempool::add_transaction(&mut self, Transaction, &UtxoSet, BlockHeight)
//!      -> Result<AddTransactionResult, MempoolError>`  (leg A)
//!   `Node::build_block_content` (as above)             (leg B)
//!   O1 the `add_transaction` verdict — must be `Ok` when no source answers
//!   O2 returned transaction list — the AddBond must be PRESENT (fail-open)
//!   O3 `Result`/`Option` discriminant — the build must succeed
//! PATHS: PA-UNAVAILABLE (live handle write-held, published snapshot empty, so
//!   `try_read` fails and `lookup` answers `Unavailable`);
//!   PB-ABSENT (the builder's blocking `read().await` always answers, so an
//!   absent key is `Unregistered{pending_addbond:0}` — M6 / REV-203-001 — and
//!   `0 + 0 + 0 + 2 <= 3000` still packs, for a NEW reason).
//! MATRIX: O1 × PA-UNAVAILABLE → leg A; O2,O3 × PB-ABSENT → leg B.
//!   [GREEN, must stay green]
//!
//! OUTPUT CONTRACT: fn absent_producer_over_cap_addbonds_are_not_packed
//! Function under test: `Node::build_block_content` (as above).
//!   O1 returned transaction list — BOTH absent-producer AddBonds must be absent
//!   O2 `Result`/`Option` discriminant — the refusal is a SKIP, never a build
//!      failure and never a slot abort (INV-PROD-002)
//! PATHS: PB-ABSENT at h >= AH.
//! INPUT PARTITIONS:
//!   IP-ABSENT-OVERSIZE unregistered key, one AddBond carrying 3001 Bond outputs
//!   IP-ABSENT-PENDING  absent from the flushed set, 2000 outpoints queued in
//!                      `PendingProducerUpdate::AddBond`, +1500 requested
//! MATRIX: O1,O2 × PB-ABSENT × {IP-ABSENT-OVERSIZE, IP-ABSENT-PENDING} → this
//!   test. [RED]
//!
//! OUTPUT CONTRACT: fn admission_expression_rejects_a_strict_subset_of_consensus
//! Function under test: `check_addbond_cap(u32, u32, u32, u64, u64)
//!   -> Result<(), ValidationError>` — a pure function.
//!   O1 the returned `Result` discriminant. No other output exists: no
//!   parameter is mutable, there is no receiver, and it writes nothing.
//! PATHS: P-POST (`height >= activation_height`) — the only path where the
//!   comparison runs; P-PRE is covered by the pre-AH test above.
//! INPUT PARTITIONS (a fixed-seed LCG spans all three):
//!   IP-UNDER sum < 3000, IP-EXACT sum == 3000, IP-OVER sum > 3000, each with
//!   `in_block_prior` at 0 and at a positive value.
//! MATRIX: O1 × P-POST × {IP-UNDER, IP-EXACT, IP-OVER} → this test. [GREEN]
//!
//! OUTPUT CONTRACT: fn allowance_with_is_not_the_addbond_expression
//! Function under test: `ProducerHoldings::allowance_with(&self, u32, u32) -> u32`.
//!   O1 the returned allowance. `&self`, no mutation, no writes.
//! PATHS: P-DEFICIT (`withdrawal_pending > 0`) — the only shape in which the
//!   withdrawal expression and the AddBond expression can differ.
//! MATRIX: O1 × P-DEFICIT × IP-PENDING-NONZERO → this test. [GREEN]

use std::time::{SystemTime, UNIX_EPOCH};

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::network::Network;
use doli_core::transaction::{Input, Output, OutputType, Transaction, TxType};
use doli_core::validation::check_addbond_cap;
use doli_core::Block;
use doli_node::node::Node;
use mempool::ProducerHoldings;
use tempfile::TempDir;

use crate::inc_i_180_common::bond_unit;

/// `MAX_BONDS_PER_PRODUCER` (`consensus/constants.rs:390`).
const CAP: u32 = doli_core::MAX_BONDS_PER_PRODUCER;

/// One below the cap: `HELD + REQUESTED = 3001 > 3000`.
const HELD: u32 = 2_999;

/// Bond outputs the AddBond carries — `requested` at the gate.
const REQUESTED: u32 = 2;

/// Post-genesis, not a testnet epoch start (`100001 % 36 == 29`), not a devnet
/// epoch start (`100001 % 4 == 1`). The same height serves both profiles, so
/// the two height-band tests differ ONLY in the activation profile.
const HEIGHT: u64 = 100_001;

const FUNDING_TAG: u8 = 0x2C;

// ─────────────────────────────────────────────────────────────────── fixture

fn addr(pk: &PublicKey) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes())
}

struct Scenario {
    node: Node,
    kp: KeyPair,
    /// The over-cap AddBond offered to the mempool.
    subject: Hash,
    _temp: TempDir,
}

/// Fund one Normal UTXO, build a `REQUESTED`-bond `AddBond` naming `kp`'s own
/// key, sign it and offer it to the node's mempool. Returns the tx hash.
async fn admit_add_bond(node: &Node, kp: &KeyPair, tag: u8) -> Hash {
    admit_add_bond_n(node, kp, tag, REQUESTED).await
}

/// `admit_add_bond` with the Bond-output count under the caller's control —
/// `requested` at the gate is that COUNT, so it is the term M6 varies.
async fn admit_add_bond_n(node: &Node, kp: &KeyPair, tag: u8, requested: u32) -> Hash {
    let pk = *kp.public_key();
    let unit = bond_unit(node);
    let funding = Hash::from_bytes([tag; 32]);
    let fee = unit / 100;
    {
        let mut utxo = node.utxo_set.write().await;
        utxo.insert(
            storage::Outpoint::new(funding, 0),
            storage::UtxoEntry {
                output: Output::normal(unit * requested as u64 + fee, addr(&pk)),
                height: 1,
                is_coinbase: false,
                is_epoch_reward: false,
            },
        )
        .expect("fixture: fund the AddBond");
    }
    let mut inp = Input::new(funding, 0);
    inp.public_key = Some(pk);
    let mut tx =
        Transaction::new_add_bond(vec![inp], pk, requested, unit * requested as u64, u64::MAX);
    let signing_hash = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&signing_hash, kp.private_key());
    let hash = tx.hash();
    {
        let utxo = node.utxo_set.read().await;
        let mut mempool = node.mempool.write().await;
        mempool
            .add_transaction(tx, &utxo, HEIGHT)
            .expect("fixture: the AddBond must be ADMITTED — admission has no cap arm");
    }
    hash
}

/// A node on `network`, holding `HELD` flushed bonds, with one funded Normal
/// UTXO and an over-cap `AddBond` already admitted to its mempool.
async fn scenario(network: Network) -> Scenario {
    let temp = TempDir::new().expect("tempdir");
    let kp = KeyPair::generate();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), vec![kp.clone()])
        .await
        .expect("Node::new_for_test");
    node.config.network = network;
    let pk = *kp.public_key();
    let unit = bond_unit(&node);

    {
        let mut ps = storage::ProducerSet::new();
        ps.register_genesis_producer(pk, HELD, unit)
            .expect("register_genesis_producer");
        let mut guard = node.producer_set.write().await;
        *guard = ps;
    }

    let subject = admit_add_bond(&node, &kp, FUNDING_TAG).await;

    Scenario {
        node,
        kp,
        subject,
        _temp: temp,
    }
}

/// Build one block at `HEIGHT`. Devnet slot timing can legitimately abort a
/// build when the recaptured timestamp crosses a slot boundary; that is a
/// TIMING abort, not a verdict, and is retried. `Err` is never retried — it is
/// the INV-PROD-002 violation this suite watches for.
async fn build_at(sc: &mut Scenario, height: u64) -> Block {
    let our_pubkey = *sc.kp.public_key();
    for _ in 0..12 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_secs();
        let slot = sc.node.params.timestamp_to_slot(now);
        let prev_slot = slot.saturating_sub(1);
        let built = sc
            .node
            .build_block_content(Hash::ZERO, prev_slot, height, slot, our_pubkey)
            .await
            .expect(
                "O2 / INV-PROD-002: build_block_content returned Err. Refusing to place \
                 an over-cap AddBond must be a SKIP (continue) in the selection loop, \
                 never a build failure and never a slot abort.",
            );
        if let Some((header, txs, _bitfield)) = built {
            return Block::new(header, txs);
        }
    }
    panic!("fixture: 12 consecutive slot-boundary aborts while building at h={height}");
}

fn contains(block: &Block, tx_hash: &Hash) -> bool {
    block.transactions.iter().any(|t| t.hash() == *tx_hash)
}

/// Count the Bond outputs of an AddBond — `requested`, mirroring
/// `validation_checks.rs:1208-1211`.
fn requested_bonds(tx: &Transaction) -> u32 {
    u32::try_from(
        tx.outputs
            .iter()
            .filter(|o| o.output_type == OutputType::Bond)
            .count(),
    )
    .unwrap_or(u32::MAX)
}

/// Replay the gate's per-block AddBond tally over `block` and emit one
/// `INC_I_203_OVER_CAP_ADDBOND_PACKED` line per AddBond the gate would reject.
/// This is the milestone's outcome metric: 1 today, 0 after the fix.
async fn report_over_cap_addbonds(node: &Node, block: &Block, height: u64, ah: u64) -> usize {
    let producers = node.producer_set.read().await;
    let mut in_block: std::collections::HashMap<PublicKey, u32> = std::collections::HashMap::new();
    let mut packed = 0usize;
    for tx in block.transactions.iter() {
        if tx.tx_type != TxType::AddBond {
            continue;
        }
        let Some(ab) = tx.add_bond_data() else {
            continue;
        };
        let pk = ab.producer_pubkey;
        let current = producers
            .get_by_pubkey(&pk)
            .map(|i| i.bond_count)
            .unwrap_or(0);
        let prior = in_block.get(&pk).copied().unwrap_or(0);
        let pending = producers.pending_addbond_count(&pk).saturating_add(prior);
        let requested = requested_bonds(tx);
        if check_addbond_cap(current, pending, requested, height, ah).is_err() {
            println!(
                "INC_I_203_OVER_CAP_ADDBOND_PACKED tx={} height={}",
                tx.hash(),
                height
            );
            packed += 1;
        }
        in_block.insert(pk, prior.saturating_add(requested));
    }
    packed
}

// ═══════════════════════════════════════════════════════════════════════════
// REQ-BOND-001 (Must) — the reproduction
// ═══════════════════════════════════════════════════════════════════════════

/// REQ-BOND-001 — Decision: a failure means the builder still assembles a block
/// its own Light self-apply rejects, so every slot scheduled to a capped
/// producer is burned on a rollback+purge cycle instead of extending the chain.
///
/// **RED today.** Acceptance: "Producer at 2999, mempool holds AddBond(+2) →
/// built block excludes it" (`inc-i-203-analysis.md:477`).
#[tokio::test]
async fn over_cap_addbond_is_not_packed_at_or_above_activation_height() {
    let mut sc = scenario(Network::Testnet).await;
    let ah = sc
        .node
        .config
        .network
        .params()
        .addbond_cap_enforcement_activation_height;
    assert!(
        HEIGHT >= ah,
        "harness: the profile must place h={HEIGHT} AT/ABOVE the cap gate, else the \
         assertion below is vacuous. got activation_height={ah}"
    );

    let block = build_at(&mut sc, HEIGHT).await;
    let packed = report_over_cap_addbonds(&sc.node, &block, HEIGHT, ah).await;

    // O1 — the contract.
    assert!(
        !contains(&block, &sc.subject),
        "INC-I-203 / REQ-BOND-001: the builder packed an AddBond that pushes the \
         producer to {}+{} = {} > cap {CAP}. `validate_block_economics` rejects this \
         block with [ADDBOND_CAP_EXCEEDED], so `production/mod.rs:620` catches it in \
         the Light self-apply, rolls back and purges — the slot is lost. \
         `select_for_block` (pool.rs:1035) and `WithdrawalParity::allow` \
         (production/withdrawal_holdings.rs:69, which returns Ok for every \
         tx_type != RequestWithdrawal) are both blind to the cap. \
         {packed} over-cap AddBond(s) in the built block.",
        HELD,
        REQUESTED,
        HELD + REQUESTED
    );
    assert_eq!(
        packed, 0,
        "INC-I-203 / REQ-BOND-001: {packed} AddBond(s) in the built block are rejected \
         by check_addbond_cap at h={HEIGHT}"
    );

    // O3 — selection is read-only: a skip is not an eviction (that is M3).
    assert!(
        sc.node.mempool.read().await.contains(&sc.subject),
        "REQ-BOND-001: the builder must SKIP, not evict. Eviction belongs to \
         `revalidate` (M3); removing it here would hide the transaction from the \
         operator without ever telling them why."
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// REQ-BOND-005 (Must) — below the gate, skipping is censorship
// ═══════════════════════════════════════════════════════════════════════════

/// REQ-BOND-005 — Decision: a failure means the new filter is height-blind and
/// changes devnet/replay selection below the activation height, which is a
/// consensus-visible behaviour change on a band that already has history.
///
/// **GREEN today, must STAY green.** Devnet pins the gate to `u64::MAX`
/// (`defaults.rs:711`), so the same transaction at the same height must still
/// be packed.
#[tokio::test]
async fn below_activation_height_over_cap_addbond_is_still_packed() {
    let mut sc = scenario(Network::Devnet).await;
    let ah = sc
        .node
        .config
        .network
        .params()
        .addbond_cap_enforcement_activation_height;
    assert_eq!(
        ah,
        u64::MAX,
        "harness: the devnet profile must keep the cap gate frozen, else this test \
         cannot distinguish the pre-AH band"
    );

    let block = build_at(&mut sc, HEIGHT).await;
    assert!(
        contains(&block, &sc.subject),
        "REQ-BOND-005: at h={HEIGHT} < activation_height (u64::MAX) the AddBond is \
         perfectly valid and the gate accepts it. Skipping it here is censorship and \
         a behaviour change on a band with history — the filter must be height-aware."
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// REQ-BOND-006 (Must) — no holdings source means SKIP THE CHECK, not reject
// ═══════════════════════════════════════════════════════════════════════════

/// REQ-BOND-006 — Decision: a failure means the cap filter fails CLOSED when no
/// holdings source can answer, which refuses every AddBond on a write-contended
/// node and on any producer the ProducerSet does not carry.
/// REQ-BOND-008 — Decision: leg A HANGS rather than fails if admission is wired
/// with a blocking `producer_set.read().await` instead of `try_read`, which is
/// the deadlock against an `apply_block` writer this requirement forbids; the
/// builder-side lock-ORDER half of REQ-BOND-008 is M2's, where the code exists.
///
/// **GREEN today, must STAY green.** Two legs, because the two layers reach
/// "no answer" by different routes.
///
/// Leg A mirrors `inc_i_180_holdings_fallback.rs`: `HoldingsSources::lookup`
/// uses `try_read`, so write-holding the live handle with an empty published
/// snapshot yields `HoldingsLookup::Unavailable` and admission must not refuse.
///
/// Leg B is the builder's own route, and M6 / REV-203-001 changed its REASON.
/// `WithdrawalParity::load` takes a BLOCKING `producer_set.read().await`
/// (`assembly.rs:193`), so the builder has NO genuinely source-less path: it
/// always has an answer. An absent key is therefore `Unregistered`, not
/// `Unavailable`, and the AddBond arm must EVALUATE it as the gate does —
/// `0 + pending_addbond(pk) + in_block_prior + requested`. This stranger has
/// `0 + 0 + 0 + REQUESTED = 2 <= 3000`, so it still packs. The withdrawal arm
/// keeps answering a missing entry with `[ECON_WITHDRAWAL_UNKNOWN_PRODUCER]`.
#[tokio::test]
async fn unavailable_holdings_fails_open_and_packs() {
    let mut sc = scenario(Network::Testnet).await;

    // Leg A — no source can answer at admission.
    let stranger = KeyPair::generate();
    let contended = sc.node.producer_set.write().await;
    let unit = bond_unit(&sc.node);
    let spk = *stranger.public_key();
    let funding = Hash::from_bytes([0x6B; 32]);
    {
        let mut utxo = sc.node.utxo_set.write().await;
        utxo.insert(
            storage::Outpoint::new(funding, 0),
            storage::UtxoEntry {
                output: Output::normal(unit * REQUESTED as u64 + unit / 100, addr(&spk)),
                height: 1,
                is_coinbase: false,
                is_epoch_reward: false,
            },
        )
        .expect("fixture: fund the stranger AddBond");
    }
    let mut inp = Input::new(funding, 0);
    inp.public_key = Some(spk);
    let mut tx =
        Transaction::new_add_bond(vec![inp], spk, REQUESTED, unit * REQUESTED as u64, u64::MAX);
    let signing_hash = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&signing_hash, stranger.private_key());
    let stranger_hash = tx.hash();
    let admitted = {
        let utxo = sc.node.utxo_set.read().await;
        let mut mempool = sc.node.mempool.write().await;
        mempool
            .add_transaction(tx, &utxo, HEIGHT)
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    drop(contended);
    assert!(
        admitted.is_ok(),
        "REQ-BOND-006 FAIL-CLOSED at admission: the live ProducerSet handle was \
         write-held and the published snapshot is never seeded under \
         `new_for_test`, so `HoldingsSources::lookup` answers `Unavailable`. \
         `holdings.rs:9-11` fixes that to mean SKIP THE CHECK — refusing here \
         censors every producer under write contention, and `new_for_replay` \
         backs the operator reindex path. got: {}",
        admitted.unwrap_err()
    );

    // Leg B — the builder has no holdings entry for the stranger.
    // Decision: if this flips, the AddBond arm has copied the withdrawal arm's
    // unknown-producer refusal and now censors an in-cap AddBond from every key
    // the ProducerSet does not carry — over-rejection the gate never asked for.
    let block = build_at(&mut sc, HEIGHT).await;
    assert!(
        contains(&block, &stranger_hash),
        "REQ-BOND-006 / REV-203-001 at the builder: the ProducerSet does not carry \
         this key, so `load` inserts no holdings entry. That is an ANSWER \
         (`Unregistered`), not `Unavailable` — the builder's `read().await` cannot \
         fail to answer — and the arm must EVALUATE it the way the gate does: \
         current 0 + pending 0 + in_block_prior 0 + {REQUESTED} <= {CAP}, so \
         consensus ACCEPTS this block and skipping is censorship. The withdrawal \
         arm's [ECON_WITHDRAWAL_UNKNOWN_PRODUCER] must NOT be copied here."
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// REQ-BOND-004 / REQ-BOND-010 (Must) — the strict-subset proof
// ═══════════════════════════════════════════════════════════════════════════

/// A fixed-seed xorshift64*. Deterministic across runs and platforms, and it
/// adds no dependency — a flaky property test is worse than none.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: u32) -> u32 {
        (self.next_u64() % bound as u64) as u32
    }
}

/// REQ-BOND-004, REQ-BOND-010 — Decision: a failure means the node-local filter
/// can refuse a transaction the consensus gate would have accepted in some
/// block, which is censorship the gate cannot correct.
///
/// **GREEN today.** The gate evaluates `current + pending + in_block_prior +
/// requested` (`validation_checks.rs:1197-1216`); the filter drops the
/// block-local term. Since `in_block_prior >= 0` the filter's total is always
/// `<=` the gate's, so what the filter rejects the gate also rejects — a strict
/// subset, with zero over-rejection at fixed state.
#[test]
fn admission_expression_rejects_a_strict_subset_of_consensus() {
    let mut rng = Rng(0x0000_0203_1203_2026);
    let ah = 0u64;
    let height = HEIGHT;

    // Exact-boundary rows first, then the random spread. Sums of 2999 / 3000 /
    // 3001 are where the `>` comparison flips.
    let mut tuples: Vec<(u32, u32, u32, u32)> = vec![
        (CAP - 1, 0, 0, 0),
        (CAP - 1, 0, 1, 0),
        (CAP - 1, 0, 2, 0),
        (CAP - 2, 0, 2, 0),
        (CAP, 0, 0, 0),
        (CAP, 0, 1, 0),
        (CAP - 1, 1, 0, 1),
        (CAP - 3, 1, 2, 5),
        (0, 0, CAP, 0),
        (0, 0, CAP + 1, 0),
        (u32::MAX, 1, 1, 1),
        (u32::MAX, u32::MAX, u32::MAX, u32::MAX),
    ];
    while tuples.len() < 200 {
        let current = rng.below(CAP + 8);
        let pending = rng.below(16);
        let requested = 1 + rng.below(8);
        let prior = rng.below(16);
        tuples.push((current, pending, requested, prior));
    }
    let boundary_hits = tuples
        .iter()
        .filter(|(c, p, r, _)| {
            let s = c.saturating_add(*p).saturating_add(*r);
            s == CAP - 1 || s == CAP || s == CAP + 1
        })
        .count();
    assert!(
        boundary_hits >= 3,
        "harness: the spread must straddle the cap boundary, else the containment \
         relation is proven only on rows where both sides agree trivially. hits={boundary_hits}"
    );

    let mut rejecting = 0usize;
    let mut accepting = 0usize;
    for (current, pending, requested, prior) in tuples {
        let filter = check_addbond_cap(current, pending, requested, height, ah);
        let gate = check_addbond_cap(
            current,
            pending.saturating_add(prior),
            requested,
            height,
            ah,
        );
        if filter.is_err() {
            rejecting += 1;
            assert!(
                gate.is_err(),
                "REQ-BOND-004 CENSORSHIP: the admission/builder expression rejects \
                 ({current}, {pending}, {requested}) while the gate ACCEPTS it with \
                 in_block_prior={prior}. The filter must reject a strict SUBSET of \
                 what consensus rejects — dropping a transaction that would have sat \
                 in a valid block is censorship no later layer can undo."
            );
        } else {
            accepting += 1;
        }
    }
    assert!(
        rejecting > 0 && accepting > 0,
        "harness: the spread must contain BOTH verdicts, else the implication is \
         vacuously true. rejecting={rejecting} accepting={accepting}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// REQ-BOND-004 (Must) — the parity trap
// ═══════════════════════════════════════════════════════════════════════════

/// REQ-BOND-004 — Decision: a failure means someone reused the withdrawal
/// allowance for the AddBond check, which subtracts `withdrawal_pending` the
/// gate never subtracts and so admits over-cap AddBonds the gate rejects — the
/// exact silent parity break of `inc-i-203-analysis.md:337-344`.
///
/// **GREEN today.** `allowance_with()` (`holdings.rs:36-42`) is the WITHDRAWAL
/// expression. The AddBond expression is `bond_count + pending_addbond`, fed
/// straight to `check_addbond_cap`.
#[test]
fn allowance_with_is_not_the_addbond_expression() {
    let h = ProducerHoldings {
        bond_count: HELD,
        pending_addbond: 1,
        withdrawal_pending: 7,
    };

    let withdrawal_view = h.allowance_with(0, 0);
    let addbond_view = h.bond_count.saturating_add(h.pending_addbond);

    assert_ne!(
        withdrawal_view, addbond_view,
        "REQ-BOND-004 PARITY TRAP: with withdrawal_pending={} the two expressions \
         MUST differ. If they are equal this fixture stopped exercising the trap.",
        h.withdrawal_pending
    );
    assert_eq!(
        withdrawal_view,
        addbond_view - h.withdrawal_pending,
        "the difference must be exactly withdrawal_pending — the term \
         `validation_checks.rs:1197-1216` never subtracts"
    );

    // The consequence: on this producer the two views disagree on the verdict.
    assert!(
        check_addbond_cap(h.bond_count, h.pending_addbond, REQUESTED, HEIGHT, 0).is_err(),
        "the AddBond expression must REJECT {}+{}+{REQUESTED} > {CAP}",
        h.bond_count,
        h.pending_addbond
    );
    assert!(
        check_addbond_cap(withdrawal_view, 0, REQUESTED, HEIGHT, 0).is_ok(),
        "the withdrawal allowance ACCEPTS the same AddBond — using allowance_with() \
         for the cap check would let over-cap AddBonds through the filter and back \
         into the block-poison path"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// M6 / REV-203-001 (Must) — the absent producer the gate still evaluates
// ═══════════════════════════════════════════════════════════════════════════

/// REQ-BOND-001, REQ-BOND-006 — Decision: a failure means the builder still
/// packs blocks every node rejects for a key the ProducerSet does not carry.
/// The gate evaluates `0 + pending_addbond(pk) + in_block_prior + requested`
/// for ANY key (`validation_checks.rs:1197-1218`), so two shapes poison a
/// block: an unregistered key that puts 3001 Bond outputs in ONE ~150 KB
/// AddBond (no per-tx ceiling in `validate_add_bond_data`, well under the
/// 600 KB policy cap) — a cheap persistent builder DoS, since every leader
/// re-packs it until `max_age` — and the INC-I-203 shape itself, a Register
/// mined mid-epoch that is still in `pending_updates` with a queued AddBond.
///
/// **RED today.** `allow_add_bond` maps a missing `holdings` entry to
/// `Unavailable`, which `addbond_cap_verdict` fails open on.
///
/// The node's own mempool is a DEVNET mempool (`new_for_test` builds it before
/// `config.network` is moved), so admission is pre-AH here and both
/// transactions become resident. Only the builder is under test.
#[tokio::test]
async fn absent_producer_over_cap_addbonds_are_not_packed() {
    const OVERSIZE_TAG: u8 = 0x71;
    const PENDING_TAG: u8 = 0x72;
    const QUEUED: u32 = 2_000;
    const PENDING_REQUEST: u32 = 1_500;

    let mut sc = scenario(Network::Testnet).await;

    let stranger = KeyPair::from_seed([OVERSIZE_TAG; 32]);
    let oversize = admit_add_bond_n(&sc.node, &stranger, OVERSIZE_TAG, CAP + 1).await;

    let unflushed = KeyPair::from_seed([PENDING_TAG; 32]);
    let unflushed_pk = *unflushed.public_key();
    let queued_tx = admit_add_bond_n(&sc.node, &unflushed, PENDING_TAG, PENDING_REQUEST).await;
    let unit = bond_unit(&sc.node);
    {
        let mut ps = sc.node.producer_set.write().await;
        ps.queue_update(storage::PendingProducerUpdate::AddBond {
            pubkey: unflushed_pk,
            outpoints: (0..QUEUED)
                .map(|i| (Hash::from_bytes([PENDING_TAG ^ 0x77; 32]), i))
                .collect(),
            bond_unit: unit,
            creation_slot: 0,
        });
        assert!(
            ps.get_by_pubkey(&unflushed_pk).is_none(),
            "harness: the key must be ABSENT from the flushed set — that is the \
             mid-epoch registration shape"
        );
        assert_eq!(
            ps.pending_addbond_count(&unflushed_pk),
            QUEUED,
            "harness: if this term is 0 the assertion below is vacuous"
        );
    }

    let ah = sc
        .node
        .config
        .network
        .params()
        .addbond_cap_enforcement_activation_height;
    let block = build_at(&mut sc, HEIGHT).await;
    let packed = report_over_cap_addbonds(&sc.node, &block, HEIGHT, ah).await;

    // Decision: if this flips, any funded non-producer key parks one transaction
    // that every leader on the network re-packs into a doomed block for 14 days.
    assert!(
        !contains(&block, &oversize),
        "REV-203-001 (b): the builder packed an AddBond carrying {} Bond outputs \
         from a key the ProducerSet does not carry. The gate reads current=0 and \
         evaluates 0 + 0 + {} > {CAP}, so EVERY node rejects this block — including \
         the builder's own Light self-apply. {packed} over-cap AddBond(s) packed.",
        CAP + 1,
        CAP + 1
    );
    // Decision: if this flips, INC-I-203's own reproduction shape still burns the
    // slot of every producer scheduled while the registration is unflushed.
    assert!(
        !contains(&block, &queued_tx),
        "REV-203-001 (a): the builder packed AddBond(+{PENDING_REQUEST}) for a key \
         absent from the FLUSHED set that already has {QUEUED} outpoints queued in \
         `pending_updates`. The gate counts `pending_addbond_count(pk)` for absent \
         keys too: 0 + {QUEUED} + {PENDING_REQUEST} > {CAP}. This is the mid-epoch \
         registration that opened the incident."
    );
    assert_eq!(
        packed, 0,
        "REQ-BOND-001: {packed} AddBond(s) in the built block are rejected by \
         check_addbond_cap at h={HEIGHT}"
    );
}
