//! INC-I-180 M2 / S1 + the INV-VALIDATION-001 three-path lock.
//!
//! covers: assembly.rs, production/mod.rs, validation_checks.rs, pool.rs,
//!         rewards.rs, lib.rs, holdings.rs, withdrawal_holdings.rs
//!
//! ---------------------------------------------------------------------------
//! THE DEFECT THIS FILE REPRODUCES
//! ---------------------------------------------------------------------------
//! M1 added five reject sites inside `validate_block_economics` with NO builder
//! and NO mempool counterpart. `assembly.rs` contains zero references to
//! `self.producer_set` and its selection-loop `ValidationContext` carries no
//! withdrawal-holdings term, so post-AH any user can submit a structurally
//! valid, signature-valid, mempool-admissible `RequestWithdrawal` that the gate
//! rejects. It never confirms, so the attacker never pays a fee and never spends
//! the input — free and infinitely repeatable. Every producer that selects it
//! burns a block build and runs `rollback_one_block()` on unauthenticated
//! demand, then the tx re-propagates.
//!
//!   INV-PROD-003: a builder weaker than apply_block deterministically
//!   constructs blocks that every node rejects, including its own.
//!   INV-PROD-002: the refusal must be a SKIP, never a build failure, never an
//!   abort, and the rollback-on-poison path must not be altered here.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT:
//! ---------------------------------------------------------------------------
//! Functions under test:
//!   `Node::build_block_content(&mut self, Hash, u32, u64, u32, PublicKey)
//!        -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>`
//!   `Node::validate_block_economics(&self, &Block, u64, ValidationMode)
//!        -> Result<()>`
//!   `Mempool::add_transaction(&mut self, Transaction, &UtxoSet, BlockHeight)
//!        -> Result<AddTransactionResult, MempoolError>`
//!
//! `build_block_content` takes `&mut self`, so its observable outputs are NOT
//! only the return value:
//!   O1  the returned transaction list — membership of each candidate
//!   O2  the returned `Result` discriminant: `Ok(Some)` vs `Ok(None)` (slot
//!       abort) vs `Err` (build failure). INV-PROD-002 forbids the last two as
//!       a response to an unwanted transaction.
//!   O3  `validate_block_economics(built_block)` at the SAME height — the whole
//!       point: a block this node built must be one this node accepts.
//!   O4  receiver mutation: `node.producer_set` and `node.utxo_set` canonical
//!       bytes across the call. A skip must mutate neither, and must not
//!       trigger the rollback path.
//!   O5  `node.mempool` membership after the build — selection is read-only,
//!       so a skipped transaction must still be there (an eviction decision
//!       belongs to `revalidate`, S2, not to the builder).
//!   O6  the mempool admission verdict for the same transaction (three-path
//!       lock only).
//!   NOT outputs: no block is stored, no gossip is emitted, no VDF is computed
//!   by `build_block_content`.
//!
//! PATHS
//!   PB-POST  build at a height AT/ABOVE AH #23
//!   PB-PRE   build at a height BELOW AH #23 (devnet gate = 20; PRE_AH = 5)
//!   PV       `validate_block_economics` on whatever was built
//!   PM       `Mempool::add_transaction` at the build height
//!
//! INPUT PARTITIONS: one per rule of the M1 table, plus two controls
//!   IP-R0     withdrawal names a producer the ProducerSet does not carry
//!   IP-R1     declared > allowance
//!   IP-R4     an input references a tx at a LOWER index in the same candidate
//!             block (the parent is a mempool ancestor, so selection order is
//!             forced, and the outpoint ALSO resolves pre-block as a Bond UTXO
//!             so the builder's existing `validate_transaction_with_utxos`
//!             cannot be what skips it)
//!   IP-R3     a Bond input owned by a DIFFERENT producer rides along
//!   IP-R2F    declared == allowance && declared > 0, but the tx leaves some of
//!             the producer's Bond UTXOs alive (incomplete drain)
//!   IP-R2P    declared != bond_inputs on a partial withdrawal
//!   IP-F6     `[partial(P), full-exit(P)]` — unsatisfiable at ANY input set,
//!             because `owned_live_bonds` is memoised pre-block while the
//!             allowance shrinks as the block is walked (SEC-FIXVERIFY2-001)
//!   IP-OK     a well-formed post-AH withdrawal (liveness control)
//!
//! MATRIX (every enumerated cell has an assertion)
//!   O1,O2,O3 × PB-POST,PV × {R0,R1,R4,R3,R2F,R2P,F6}
//!        → req_i180_003_builder_skips_every_gate_rejecting_withdrawal   [RED]
//!   O1,O2,O3 × PB-POST,PV × IP-OK
//!        → req_i180_003_builder_still_selects_a_well_formed_withdrawal
//!   O1 × PB-PRE × {R0,R1,R4,R3,R2F,R2P,F6}
//!        → req_i180_003_pre_activation_selection_is_unchanged
//!   O2,O4,O5 × PB-POST × {R0,R1,R4,R3,R2F,R2P,F6}
//!        → req_i180_003_skip_never_fails_aborts_or_rolls_back
//!   O1,O2,O3,O6 × PM,PB-POST,PV × all partitions
//!        → req_i180_003_mempool_builder_and_consensus_agree             [RED]
//!   O3 × PV × all partitions, at BOTH height bands
//!        → req_i180_003_gate_rejects_every_partition_in_a_hand_built_block
//!          (harness self-check: the two RED tests stop at the first partition,
//!           so without this row the other six are never shown to reach the
//!           state they claim and a mis-built partition reads as a fix failure)

use std::time::{SystemTime, UNIX_EPOCH};

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::transaction::{Input, Output, Transaction, TxType};
use doli_core::validation::ValidationMode;
use doli_core::Block;
use doli_node::node::Node;
use tempfile::TempDir;

use crate::inc_i_180_common::{bond_unit, build_ledger, seed_owned_bond_utxos, POST_AH, PRE_AH};

/// Flushed bonds held by the named producer in every scenario. `allowance == 4`
/// unless an in-block term moves it.
const HELD: u32 = 4;

// ─────────────────────────────────────────────────────────────── fixture

fn sign_input(tx: &mut Transaction, i: usize, kp: &KeyPair) {
    let signing_hash = tx.signing_message_for_input(i);
    tx.inputs[i].signature = crypto::signature::sign_hash(&signing_hash, kp.private_key());
}

fn addr(pk: &PublicKey) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes())
}

fn outpoints(tag: u8, count: u32) -> Vec<(Hash, u32)> {
    let h = Hash::from_bytes([tag; 32]);
    (0..count).map(|i| (h, i)).collect()
}

/// A `RequestWithdrawal` naming `producer`, spending `spends`, each input
/// carrying and signed by ITS OWN owner key. Per-input owners are what makes
/// the R3 exclusivity partition constructible: a tx spending A's and B's Bond
/// UTXOs is signature-valid when each input is signed by its own owner.
fn signed_withdrawal(
    node: &Node,
    producer: &PublicKey,
    declared: u32,
    spends: &[((Hash, u32), &KeyPair)],
) -> Transaction {
    let unit = bond_unit(node);
    let inputs: Vec<Input> = spends
        .iter()
        .map(|((h, idx), owner)| {
            let mut inp = Input::new(*h, *idx);
            inp.public_key = Some(*owner.public_key());
            inp
        })
        .collect();
    let dest = crypto::hash::hash(b"inc-i-180-m2-withdrawal-destination");
    let net = unit * spends.len() as u64 - unit / 100;
    let mut tx = Transaction::new_request_withdrawal(inputs, *producer, declared, dest, net);
    for (i, (_, owner)) in spends.iter().enumerate() {
        sign_input(&mut tx, i, owner);
    }
    tx
}

/// A high-fee `Transfer` used ONLY as the lower-index parent of the R4
/// partition. Its fee rate must exceed the withdrawal's, because
/// `select_for_block` SKIPS (does not defer) a transaction whose ancestors are
/// not yet selected — an equal-or-lower parent rate would drop the withdrawal
/// for the wrong reason and make the partition vacuous.
async fn high_fee_parent(node: &Node, kp: &KeyPair, tag: u8) -> Transaction {
    let unit = bond_unit(node);
    let funding = Hash::from_bytes([tag; 32]);
    {
        let mut utxo = node.utxo_set.write().await;
        utxo.insert(
            storage::Outpoint::new(funding, 0),
            storage::UtxoEntry {
                output: Output::normal(unit * 3, addr(kp.public_key())),
                height: 1,
                is_coinbase: false,
                is_epoch_reward: false,
            },
        )
        .expect("fixture: fund the R4 parent");
    }
    let mut inp = Input::new(funding, 0);
    inp.public_key = Some(*kp.public_key());
    let mut tx =
        Transaction::new_transfer(vec![inp], vec![Output::normal(unit, addr(kp.public_key()))]);
    sign_input(&mut tx, 0, kp);
    tx
}

/// Seed one Bond UTXO at an arbitrary outpoint, owned by `owner`.
async fn seed_bond_at(node: &Node, at: (Hash, u32), owner: &PublicKey) {
    let unit = bond_unit(node);
    let mut utxo = node.utxo_set.write().await;
    utxo.insert(
        storage::Outpoint::new(at.0, at.1),
        storage::UtxoEntry {
            output: Output::bond(unit, addr(owner), u64::MAX, 0),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        },
    )
    .expect("fixture: seed Bond UTXO");
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    R0,
    R1,
    R4,
    R3,
    R2Full,
    R2Partial,
    F6,
    Ok,
}

const REJECTING: [Kind; 7] = [
    Kind::R0,
    Kind::R1,
    Kind::R4,
    Kind::R3,
    Kind::R2Full,
    Kind::R2Partial,
    Kind::F6,
];

impl Kind {
    fn code(self) -> &'static str {
        match self {
            Kind::R0 => "[ECON_WITHDRAWAL_UNKNOWN_PRODUCER]",
            Kind::R1 => "[ECON_WITHDRAWAL_OVER_HOLDINGS]",
            Kind::R4 => "[ECON_WITHDRAWAL_SAME_BLOCK_INPUT]",
            Kind::R3 | Kind::R2Partial => "[ECON_WITHDRAWAL_BOND_COUNT_MISMATCH]",
            Kind::R2Full | Kind::F6 => "[ECON_WITHDRAWAL_INCOMPLETE_DRAIN]",
            Kind::Ok => "",
        }
    }
}

/// Everything one partition needs: a node whose ledger and UTXO view are
/// seeded, plus the transactions to offer the mempool IN ORDER.
struct Scenario {
    node: Node,
    /// The node's genesis producer key — also the named producer everywhere
    /// except `Kind::R0`.
    kp: KeyPair,
    txs: Vec<Transaction>,
    /// Hashes of the `RequestWithdrawal` transactions the gate must not see
    /// all of at once.
    withdrawals: Vec<Hash>,
    _temp: TempDir,
}

async fn scenario(kind: Kind) -> Scenario {
    let temp = TempDir::new().expect("tempdir");
    let kp = KeyPair::generate();
    let node = Node::new_for_test(temp.path().to_path_buf(), vec![kp.clone()])
        .await
        .expect("Node::new_for_test");
    let pk = *kp.public_key();

    {
        let mut guard = node.producer_set.write().await;
        *guard = build_ledger(&node, &pk, HELD, 0);
    }

    let txs: Vec<Transaction> = match kind {
        Kind::R0 => {
            // Named producer is a key the ledger never registers.
            let stranger = KeyPair::generate();
            let spk = *stranger.public_key();
            seed_owned_bond_utxos(&node, &spk, 0xA0, 2).await;
            vec![signed_withdrawal(
                &node,
                &spk,
                1,
                &[(outpoints(0xA0, 1)[0], &stranger)],
            )]
        }
        Kind::R1 => {
            seed_owned_bond_utxos(&node, &pk, 0xB0, 6).await;
            let spends: Vec<((Hash, u32), &KeyPair)> =
                outpoints(0xB0, 5).into_iter().map(|o| (o, &kp)).collect();
            vec![signed_withdrawal(&node, &pk, HELD + 1, &spends)]
        }
        Kind::R4 => {
            seed_owned_bond_utxos(&node, &pk, 0xC0, 2).await;
            let parent = high_fee_parent(&node, &kp, 0xC1).await;
            // The chained outpoint ALSO exists pre-block as a Bond UTXO, so the
            // builder's existing UTXO validation resolves it and only the new
            // R4 predicate can be what skips the withdrawal.
            seed_bond_at(&node, (parent.hash(), 0), &pk).await;
            let spends = vec![(outpoints(0xC0, 1)[0], &kp), ((parent.hash(), 0u32), &kp)];
            let wd = signed_withdrawal(&node, &pk, 2, &spends);
            vec![parent, wd]
        }
        Kind::R3 => {
            let foreign = KeyPair::generate();
            seed_owned_bond_utxos(&node, &pk, 0xD0, 1).await;
            seed_owned_bond_utxos(&node, foreign.public_key(), 0xD1, 1).await;
            let spends = vec![
                (outpoints(0xD0, 1)[0], &kp),
                (outpoints(0xD1, 1)[0], &foreign),
            ];
            vec![signed_withdrawal(&node, &pk, 1, &spends)]
        }
        Kind::R2Full => {
            seed_owned_bond_utxos(&node, &pk, 0xE0, 6).await;
            let spends: Vec<((Hash, u32), &KeyPair)> = outpoints(0xE0, HELD)
                .into_iter()
                .map(|o| (o, &kp))
                .collect();
            vec![signed_withdrawal(&node, &pk, HELD, &spends)]
        }
        Kind::R2Partial => {
            seed_owned_bond_utxos(&node, &pk, 0xF0, 3).await;
            let spends: Vec<((Hash, u32), &KeyPair)> =
                outpoints(0xF0, 3).into_iter().map(|o| (o, &kp)).collect();
            vec![signed_withdrawal(&node, &pk, 2, &spends)]
        }
        Kind::F6 => {
            // DISJOINT inputs, each tx individually valid, the PAIR
            // unsatisfiable in either order: whichever is walked second
            // declares exactly the shrunken allowance, which promotes it to a
            // FULL EXIT and demands it drain all 4 pre-block Bond UTXOs — but
            // `owned_live_bonds` was memoised BEFORE the block, so no input set
            // satisfies it.
            seed_owned_bond_utxos(&node, &pk, 0x90, HELD).await;
            let first = signed_withdrawal(&node, &pk, 1, &[(outpoints(0x90, 1)[0], &kp)]);
            let rest: Vec<((Hash, u32), &KeyPair)> = outpoints(0x90, HELD)
                .into_iter()
                .skip(1)
                .map(|o| (o, &kp))
                .collect();
            let second = signed_withdrawal(&node, &pk, HELD - 1, &rest);
            vec![first, second]
        }
        Kind::Ok => {
            seed_owned_bond_utxos(&node, &pk, 0x80, HELD).await;
            vec![signed_withdrawal(
                &node,
                &pk,
                1,
                &[(outpoints(0x80, 1)[0], &kp)],
            )]
        }
    };

    let withdrawals = txs
        .iter()
        .filter(|t| t.tx_type == TxType::RequestWithdrawal)
        .map(|t| t.hash())
        .collect();

    Scenario {
        node,
        kp,
        txs,
        withdrawals,
        _temp: temp,
    }
}

/// Offer every scenario transaction to the node's own mempool at `at_height`.
/// Returns the per-transaction admission verdict, so a caller can assert on it
/// (three-path lock) or merely require it (builder tests).
async fn offer_to_mempool(sc: &Scenario, at_height: u64) -> Vec<Result<(), String>> {
    let utxo = sc.node.utxo_set.read().await;
    let mut mempool = sc.node.mempool.write().await;
    sc.txs
        .iter()
        .map(|tx| {
            mempool
                .add_transaction(tx.clone(), &utxo, at_height)
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .collect()
}

/// Build one block at `height`. Devnet runs 1-second slots, so
/// `build_block_content` can legitimately return `Ok(None)` when the recaptured
/// timestamp lands in the next slot; that is a TIMING abort, not a verdict, and
/// is retried. A retry that never succeeds is a fixture failure, and an `Err`
/// is never retried — it is the INV-PROD-002 violation this suite watches for.
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
                "O2 / INV-PROD-002: build_block_content returned Err. The refusal to \
                 place a gate-rejecting withdrawal must be a SKIP (continue), exactly \
                 like the NFT/Pool unique-id checks — never a build failure and never \
                 an abort.",
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

fn withdrawal_count(block: &Block) -> usize {
    block
        .transactions
        .iter()
        .filter(|t| t.tx_type == TxType::RequestWithdrawal)
        .count()
}

// ═══════════════════════════════════════════════════════════════════════════
// PB-POST + PV — the builder must not construct a block it will reject
// ═══════════════════════════════════════════════════════════════════════════

/// O1,O2,O3 × PB-POST,PV × the seven rejecting partitions — **RED today.**
///
/// Two assertions per partition, and BOTH matter. "The offender is absent" on
/// its own is satisfiable by a builder that drops every withdrawal; "the block
/// validates" on its own is satisfiable by a builder that produces empty
/// blocks. Together with the liveness control below they pin the behaviour.
#[tokio::test]
async fn req_i180_003_builder_skips_every_gate_rejecting_withdrawal() {
    for kind in REJECTING {
        let mut sc = scenario(kind).await;
        offer_to_mempool(&sc, PRE_AH).await;
        let block = build_at(&mut sc, POST_AH).await;

        // O3 — the contract. A node must accept the block it just built.
        let verdict = sc
            .node
            .validate_block_economics(&block, POST_AH, ValidationMode::Light)
            .await;
        assert!(
            verdict.is_ok(),
            "INV-PROD-003 / {kind:?}: the builder assembled a block that this same \
             node REJECTS with {}. Post-AH this is free, unauthenticated block \
             poison: the attacker never pays a fee and never spends the input, while \
             every producer that selects the tx burns a build and runs \
             rollback_one_block(). assembly.rs has zero references to \
             self.producer_set and its selection ValidationContext carries no \
             withdrawal-holdings term. verdict={}",
            kind.code(),
            verdict.unwrap_err()
        );

        // O1 — and it is absent for the RIGHT reason: the gate would reject it.
        match kind {
            Kind::F6 => {
                // Harness: EACH half is individually consensus-valid, so the
                // only thing wrong with the pair is the pair.
                for tx in &sc.txs {
                    let solo = crate::inc_i_180_common::block_with(
                        &sc.node,
                        POST_AH,
                        *sc.kp.public_key(),
                        vec![tx.clone()],
                    );
                    assert!(
                        sc.node
                            .validate_block_economics(&solo, POST_AH, ValidationMode::Light)
                            .await
                            .is_ok(),
                        "harness: each F6 half must be valid ALONE, otherwise the pair \
                         is not what makes it unsatisfiable"
                    );
                }
                assert!(
                    withdrawal_count(&block) <= 1,
                    "F6 / SEC-FIXVERIFY2-001: [partial(P), full-exit(P)] in ONE block is \
                 unsatisfiable at ANY input set — owned_live_bonds is memoised over \
                 the pre-block view while the allowance shrinks as the block is \
                 walked. The builder must never construct this shape; skipping the \
                 second withdrawal for a producer that already has one selected is a \
                 sufficient discharge. built {} withdrawals",
                    withdrawal_count(&block)
                );
            }
            _ => {
                for h in &sc.withdrawals {
                    assert!(
                        !contains(&block, h),
                        "{kind:?}: the gate-rejecting withdrawal was placed in the \
                         candidate block"
                    );
                }
            }
        }
    }
}

/// O1,O2,O3 × PB-POST,PV × IP-OK — **GREEN today, must STAY green.**
///
/// The counterweight. Every RED test above is satisfiable by a builder that
/// drops all withdrawals; this one fails on exactly that fix, and on any
/// predicate that is not keyed on the NAMED producer's own holdings.
#[tokio::test]
async fn req_i180_003_builder_still_selects_a_well_formed_withdrawal() {
    let mut sc = scenario(Kind::Ok).await;
    let admitted = offer_to_mempool(&sc, POST_AH).await;
    assert!(
        admitted[0].is_ok(),
        "OVER-REJECTION at admission: a well-formed post-AH withdrawal (declares 1 of \
         an allowance of {HELD}, spends exactly 1 owned Bond UTXO) was refused. got: {}",
        admitted[0].clone().unwrap_err()
    );

    let block = build_at(&mut sc, POST_AH).await;
    // O1
    assert!(
        contains(&block, &sc.withdrawals[0]),
        "OVER-REJECTION at the builder: a well-formed post-AH withdrawal was skipped. \
         The predicate must refuse what the gate refuses, not withdrawals as a class."
    );
    // O3
    let verdict = sc
        .node
        .validate_block_economics(&block, POST_AH, ValidationMode::Light)
        .await;
    assert!(
        verdict.is_ok(),
        "the block carrying a well-formed withdrawal must validate: {}",
        verdict.unwrap_err()
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// PB-PRE — below AH #23 skipping is CENSORSHIP
// ═══════════════════════════════════════════════════════════════════════════

/// O1 × PB-PRE × the seven rejecting partitions — **GREEN today, must STAY
/// green.** Below the gate every one of these transactions is perfectly valid
/// and a live network confirms them. A height-blind predicate turns the M1
/// zero-deletion proof into a behaviour change on mainnet and testnet.
#[tokio::test]
async fn req_i180_003_pre_activation_selection_is_unchanged() {
    for kind in REJECTING {
        let mut sc = scenario(kind).await;
        let admitted = offer_to_mempool(&sc, PRE_AH).await;
        for (i, verdict) in admitted.iter().enumerate() {
            assert!(
                verdict.is_ok(),
                "pre-AH invariance / {kind:?}: tx {i} was refused admission at \
                 h={PRE_AH}, below AH #23. Admission strictness must be height-aware. \
                 got: {}",
                verdict.clone().unwrap_err()
            );
        }

        let block = build_at(&mut sc, PRE_AH).await;
        for h in &sc.withdrawals {
            assert!(
                contains(&block, h),
                "pre-AH invariance / {kind:?}: the builder CENSORED a transaction that \
                 is valid below AH #23. The predicate must be a no-op under the gate."
            );
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// INV-PROD-002 — a skip is a skip
// ═══════════════════════════════════════════════════════════════════════════

/// O2,O4,O5 × PB-POST × the seven rejecting partitions.
///
/// `build_at` already fails the suite on `Err` (O2). This test adds the two
/// receiver-mutation cells: the selection pass must leave the producer set and
/// the UTXO set byte-identical (no rollback, no mutation), and must leave the
/// mempool untouched — evicting is `revalidate`'s job, not the builder's.
#[tokio::test]
async fn req_i180_003_skip_never_fails_aborts_or_rolls_back() {
    for kind in REJECTING {
        let mut sc = scenario(kind).await;
        offer_to_mempool(&sc, PRE_AH).await;

        let before_producers = sc.node.producer_set.read().await.serialize_canonical();
        let before_utxo = sc.node.utxo_set.read().await.len();
        let before_mempool = sc.node.mempool.read().await.len();

        let _block = build_at(&mut sc, POST_AH).await;

        // O4
        assert_eq!(
            sc.node.producer_set.read().await.serialize_canonical(),
            before_producers,
            "INV-PROD-002 / {kind:?}: block ASSEMBLY mutated the producer set. A \
             refusal must be a `continue` in the selection loop; it must never reach \
             the rollback path."
        );
        assert_eq!(
            sc.node.utxo_set.read().await.len(),
            before_utxo,
            "INV-PROD-002 / {kind:?}: block assembly mutated the UTXO set"
        );
        // O5
        assert_eq!(
            sc.node.mempool.read().await.len(),
            before_mempool,
            "{kind:?}: the builder evicted from the mempool. Selection is read-only; \
             eviction of a now-invalid tx belongs to `revalidate` (S2)."
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// INV-VALIDATION-001 — the three-path lock
// ═══════════════════════════════════════════════════════════════════════════

/// O1,O2,O3,O6 × PM,PB-POST,PV × all eight partitions — **RED today.**
///
/// Drives the SAME transaction through mempool admission, the block builder and
/// `validate_block_economics`, and asserts the three verdicts are consistent:
///
///   consensus-reject  ⟹  builder-skip  AND  mempool-reject
///   consensus-accept  ⟹  builder-select AND mempool-accept
///
/// The mempool is allowed to be WEAKER than the builder in general (it cannot
/// see block composition, so R4 and every in-block term are unknowable there) —
/// but every partition below except `R4` is decidable from single-tx CURRENT
/// state, so for those the implication is exact. `R4` is asserted on the two
/// paths that can see the block.
#[tokio::test]
async fn req_i180_003_mempool_builder_and_consensus_agree() {
    for kind in REJECTING.into_iter().chain([Kind::Ok]) {
        let mut sc = scenario(kind).await;
        // O6 — admission at the height the tx would be INCLUDED at.
        let admission = offer_to_mempool(&sc, POST_AH).await;

        // Consensus leg: the gate's own verdict on a block carrying everything.
        let reference = build_reference_block(&sc, POST_AH).await;
        let consensus = sc
            .node
            .validate_block_economics(&reference, POST_AH, ValidationMode::Light)
            .await;

        if kind == Kind::Ok {
            assert!(
                consensus.is_ok(),
                "harness: the liveness control must be consensus-valid: {}",
                consensus.unwrap_err()
            );
            assert!(
                admission[0].is_ok(),
                "PARITY: consensus ACCEPTS this withdrawal but the mempool refused it. \
                 A mempool stronger than consensus evicts transactions a real block \
                 can carry. got: {}",
                admission[0].clone().unwrap_err()
            );
            let block = build_at(&mut sc, POST_AH).await;
            assert!(
                contains(&block, &sc.withdrawals[0]),
                "PARITY: consensus ACCEPTS this withdrawal but the builder skipped it"
            );
            continue;
        }

        assert!(
            consensus.is_err(),
            "harness: {kind:?} must be consensus-rejecting, otherwise the parity \
             assertions below are vacuous"
        );

        // Builder leg.
        let block = build_at(&mut sc, POST_AH).await;
        let builder_selected = sc.withdrawals.iter().any(|h| contains(&block, h));
        assert!(
            !builder_selected || withdrawal_count(&block) < sc.withdrawals.len(),
            "INV-VALIDATION-001 / {kind:?}: consensus rejects with {} but the BUILDER \
             selected the transaction — the builder is weaker than apply_block \
             (INV-PROD-003, commit eb515749, INC-I-147)",
            kind.code()
        );

        // Mempool leg. R4 is the one rule that is not decidable from single-tx
        // current state, so the mempool is permitted to admit it.
        if kind != Kind::R4 {
            let refused = admission.iter().any(|v| v.is_err());
            assert!(
                refused,
                "INV-VALIDATION-001 / {kind:?}: consensus rejects with {} but the \
                 MEMPOOL admitted every transaction at h={POST_AH}. This is the free, \
                 unauthenticated block-poison surface: the tx never confirms, so no \
                 fee is paid and no input is spent, and it re-propagates forever \
                 (Reviewer F1, OBS-001).",
                kind.code()
            );
        }
    }
}

/// O3 × PV × all eight partitions — **GREEN today, must STAY green.**
///
/// The consensus leg of the table, asserted on its own. Without it the two RED
/// tests above stop at the first partition and the remaining six are never
/// shown to reach the state they claim; a mis-built partition would then read
/// as a fix failure. Binds the bracketed CODE, not merely `is_err`.
#[tokio::test]
async fn req_i180_003_gate_rejects_every_partition_in_a_hand_built_block() {
    for kind in REJECTING {
        let sc = scenario(kind).await;
        let block = build_reference_block(&sc, POST_AH).await;
        let verdict = sc
            .node
            .validate_block_economics(&block, POST_AH, ValidationMode::Light)
            .await;
        let msg = verdict
            .err()
            .unwrap_or_else(|| panic!("harness: {kind:?} must be consensus-rejecting"))
            .to_string();
        assert!(
            msg.contains(kind.code()),
            "harness: {kind:?} must reject with {} — got: {msg}",
            kind.code()
        );

        // And below AH #23 the very same block is admitted: the partitions are
        // built from the gate's rules, not from independently invalid shapes.
        let pre = crate::inc_i_180_common::block_with(
            &sc.node,
            PRE_AH,
            *sc.kp.public_key(),
            sc.txs.clone(),
        );
        assert!(
            sc.node
                .validate_block_economics(&pre, PRE_AH, ValidationMode::Light)
                .await
                .is_ok(),
            "harness: {kind:?} must be valid BELOW the gate, else the pre-AH \
             invariance test proves nothing"
        );
    }

    let sc = scenario(Kind::Ok).await;
    let block = build_reference_block(&sc, POST_AH).await;
    assert!(
        sc.node
            .validate_block_economics(&block, POST_AH, ValidationMode::Light)
            .await
            .is_ok(),
        "harness: the liveness control must be consensus-valid post-AH"
    );
}

/// The consensus reference: the scenario's transactions in a hand-built block,
/// so the gate's verdict is observed independently of what the builder chose.
async fn build_reference_block(sc: &Scenario, height: u64) -> Block {
    crate::inc_i_180_common::block_with(&sc.node, height, *sc.kp.public_key(), sc.txs.clone())
}
