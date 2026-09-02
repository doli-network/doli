//! INC-I-204 M4.2 / REQ-FORK-002 — "error paths must not retract published state".
//! TESTS-FIRST (RED): the API these tests name does not exist yet.
//!
//! Design: `docs/.workflow/inc-i-204-M4.2-design-brief.md`.
//! Requirement + acceptance criteria: `docs/redesigns/fork-lifecycle-redesign-analysis.md:456`.
//! RED evidence (both phases): `docs/.workflow/inc-i-204-M4.2-test-red-evidence.txt`.
//!
//! ===========================================================================
//! OUTPUT CONTRACT
//! ===========================================================================
//! FUNCTIONS UNDER TEST
//!   F1 `Node::rollback_one_block(&mut self, authority: RollbackAuthority)
//!        -> Result<RollbackOutcome>`                       (node/rollback.rs)
//!   F2 `Node::handle_failed_self_apply(&mut self, block: &Block, height: u64,
//!        err: anyhow::Error) -> Result<()>`        (node/production/poison.rs)
//!
//!   Required export path for the new type: `doli_node::node::RollbackAuthority`,
//!   mirroring the existing `pub use rollback::RollbackOutcome`.
//!
//! OBSERVABLE OUTPUTS — every one is asserted through the accessor named here.
//!   O1  F1 return value        `rollback_one_block(..).await?`
//!   O2  F2 return value        `handle_failed_self_apply(..).await`
//!   O3  sync tip height        `node.sync_manager.read().await.local_tip().0`
//!   O4  in-memory chain height `node.chain_state.read().await.best_height`
//!   O5  canonical hash at N    `node.block_store.get_hash_by_height(N)?`
//!   O6  rollback budget        `node.cumulative_rollback_depth`
//!   O7  finality marker        `node.sync_manager.read().await
//!                                  .reorg_handler().last_finality_height()`
//!   O8  mempool membership     `node.mempool.read().await.contains(&tx.hash())`
//!   O9  ProducerSet parity     `node.producer_set` vs `state_db.load_producer_set()`
//!   O10 ChainState parity      `node.chain_state` vs `state_db.get_chain_state()`
//!   O11 outcome counter        `doli_poison_containment_total{outcome}`, read back
//!                              out of the RENDERED exposition text
//!
//!   NOT an output of this milestone: the log lines. `bins/node` carries no
//!   tracing-capture dev-dependency and adding one edits a non-test manifest —
//!   the same exclusion `inc_i_174_maintainer_rewind_guards.rs` records. O11 is
//!   the machine-checkable half.
//!
//! CODE PATHS
//!   P1 `ProductionSelfApply`, `local_tip != failed_height` -> RefusedNotAuthorized
//!   P2 `ProductionSelfApply`, `local_tip == failed_height` -> the rewind runs
//!   P3 `CoordinatorApproved`                               -> unguarded, as today
//!   P4 the arm's mempool purge, on every rollback outcome
//!   P5 the arm's aborted-apply state resync, on every outcome that keeps the tip
//!   P6 the rollback errors part-way through the rewind
//!
//! INPUT PARTITIONS
//!   IP-AHEAD  `failed_height == tip + 1`. The RECORDED cell: `apply_block` returned
//!             Err before mutating anything, so the block the old door retracts is
//!             the PARENT — this node's already-published tip.
//!   IP-TIP    `failed_height == tip`. The failed block DID become tip. The control
//!             for IP-AHEAD: it differs in exactly one input and must still rewind,
//!             or the narrowing has become a removal and a half-applied block stands.
//!   IP-COORD  the same fixture through the coordinator's authority. The control for
//!             the whole narrowing: this door is NOT narrowed (INC-I-190 rescue).
//!   IP-FIN    a finality marker set BELOW the tip through the real
//!             `add_attestation_weight` -> `finalize_if_ready` path.
//!   IP-CAP    `cumulative_rollback_depth` at the 50-block cap -> RefusedNoMutation.
//!   IP-ERR    a corrupt `producer_snapshot` plus a hole in the rebuild range, so the
//!             rewind mutates the UTXO set and then errors.
//!   IP-DIRT   in-memory residue planted at BOTH sites an aborted `apply_block`
//!             mutates before it can fail: the `ProducerSet` (tx loop) and
//!             `chain_state` (`update_chain_state_for_block`).
//!
//! MATRIX
//!   IP-AHEAD : O1 O3 O4 O5 O6 · IP-TIP : O1 O3 O11 · IP-COORD : O1 O3 O6
//!   IP-FIN   : O1 O3 O7       · IP-CAP : O2 O3 O8 O11
//!   IP-ERR   : O2 O8 O11      · IP-DIRT: O2 O3 O9 O10
//!
//! ANTI-VACUITY
//!   Every refusal partition carries a control that must NOT refuse (IP-TIP for the
//!   authority guard, IP-COORD for the door itself, the cleared-marker test for the
//!   finality skip, the honest tx for the purge). IP-DIRT plants a divergence and
//!   asserts it is observable BEFORE the call, so a no-op resync cannot pass.
//!
//! HARNESS NOTE — the poison spelling.
//!   The design brief names `ADDBOND_CAP_EXCEEDED` as the recorded shape. It is not
//!   reachable from `Node::new_for_test`: that constructor pins `Network::Devnet`,
//!   whose `addbond_cap_enforcement_activation_height` is `u64::MAX`
//!   (`network_params/defaults.rs:701`); `NetworkParams::load` memoises the env
//!   override in a per-process `OnceLock`, so setting it inside this shared test
//!   binary is order-dependent on whichever test touches `DEVNET_PARAMS` first; and
//!   moving `config.network` also moves `ConsensusParams::for_network`, which
//!   invalidates the genesis hash, `genesis_time` and `slot_duration` of every block
//!   the fixture built. The poison here is instead a toxic mempool transaction the
//!   builder admitted and `validate_block_economics` rejects — the same function,
//!   raised on the same side of the mutation boundary, and it is the class the arm's
//!   own comment cites ("toxic mempool TX passed builder validation but fails apply
//!   validation"). What REQ-FORK-002 turns on is that `apply_block` returned Err
//!   with nothing mutated, and every test below asserts that precondition directly.

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Transaction};
use doli_node::node::{Node, RollbackAuthority, RollbackOutcome};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

use super::inc_i_204_m0_common as reg;

/// The height this node has already published and gossiped.
const TIP: u64 = 5;
/// `rollback.rs` MAX_CUMULATIVE_ROLLBACK.
const ROLLBACK_CAP: u32 = 50;
const METRIC: &str = "doli_poison_containment_total";

// ===========================================================================
// HARNESS
// ===========================================================================

/// A devnet node with a chain applied up to `TIP`, plus the hash of every height
/// (`hashes[h]`). Held together so the `TempDir` outlives the node.
struct Staged {
    node: Node,
    producers: Vec<KeyPair>,
    hashes: Vec<Hash>,
    _tmp: TempDir,
}

fn build_block(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    params: &ConsensusParams,
    extra_txs: Vec<Transaction>,
) -> Block {
    let reward = params.block_reward(height);
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, 0);
    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);
    let mut txs = vec![coinbase];
    txs.extend(extra_txs);
    let merkle_root = doli_core::block::compute_merkle_root(&txs);
    let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();

    let header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash,
        timestamp,
        slot,
        producer: *producer.public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: crypto::Hash::ZERO,
        fork_id: crypto::Hash::ZERO,
    };
    Block::new(header, txs)
}

async fn staged() -> Staged {
    let tmp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..5).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(tmp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    let params = node.params.clone();

    let mut hashes = vec![Hash::ZERO];
    let mut prev = Hash::ZERO;
    for h in 1..=TIP {
        let b = build_block(h, h as u32, prev, &producers[0], &params, vec![]);
        prev = b.hash();
        hashes.push(prev);
        node.apply_block(b, ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("fixture: h={h} must apply: {e}"));
    }
    assert_eq!(node.chain_state.read().await.best_height, TIP);
    assert_eq!(node.sync_manager.read().await.local_tip().0, TIP);

    Staged {
        node,
        producers,
        hashes,
        _tmp: tmp,
    }
}

/// A transaction the builder admits and `validate_block_economics` rejects.
fn toxic_tx(tag: &[u8]) -> Transaction {
    Transaction::new_coinbase(1, crypto::hash::hash(tag), 999, 0)
}

/// The self-produced block at `TIP + 1` that fails apply with nothing mutated.
/// Returns the block and the error `apply_block` produced, and asserts the
/// pre-mutation precondition every test in this file depends on.
async fn poisoned_self_apply(s: &mut Staged, extra: Vec<Transaction>) -> (Block, anyhow::Error) {
    let params = s.node.params.clone();
    let block = build_block(
        TIP + 1,
        (TIP + 1) as u32,
        s.hashes[TIP as usize],
        &s.producers[0],
        &params,
        extra,
    );
    let err = s
        .node
        .apply_block(block.clone(), ValidationMode::Light)
        .await
        .expect_err("fixture: the toxic block must be rejected");
    assert_eq!(
        s.node.chain_state.read().await.best_height,
        TIP,
        "fixture: the rejection must land BEFORE any state mutation — that is the \
         whole input partition. Got a mutated chain state instead: {err}"
    );
    (block, err)
}

async fn tip_of(node: &Node) -> u64 {
    node.sync_manager.read().await.local_tip().0
}

async fn finality_marker(node: &Node) -> Option<u64> {
    node.sync_manager
        .read()
        .await
        .reorg_handler()
        .last_finality_height()
}

/// Drive the real finalize path so the marker is set at `height`.
/// `finalize_if_ready` needs `CONFIRMATION_DEPTH = 2` of applied descendants, so
/// this only reaches heights at or below `TIP - 2`.
async fn finalize_at(s: &mut Staged, height: u64) {
    {
        let mut sync = s.node.sync_manager.write().await;
        sync.add_attestation_weight(
            &s.hashes[height as usize],
            *s.producers[1].public_key(),
            1_000_000,
        );
    }
    assert_eq!(
        finality_marker(&s.node).await,
        Some(height),
        "fixture: the marker must really be set, or every finality assertion below \
         is satisfied vacuously"
    );
}

fn counter(outcome: &str) -> f64 {
    reg::ensure_registered();
    reg::exported_value(METRIC, &[("outcome", outcome)]).unwrap_or_else(|| {
        panic!("`{METRIC}{{outcome=\"{outcome}\"}}` must be zero-initialised (INC-I-187)")
    })
}

// ===========================================================================
// T1 — the recorded incident shape.
// ===========================================================================

/// IP-AHEAD x P1. O1 O3 O4 O5 O6.
///
/// REQ-FORK-002 AC-1 — Decision: a failure here says the poison arm still consumes a
/// rollback of a block it never applied, so every pre-mutation production failure
/// retracts this node's own published tip and manufactures the fork INC-I-204 is
/// about. This is the FAIL->PASS reproduction AC-4 demands.
#[tokio::test]
async fn req_fork_002_ac1_a_premutation_self_apply_failure_keeps_the_published_tip() {
    let mut s = staged().await;
    let published = s.hashes[TIP as usize];
    let budget_before = s.node.cumulative_rollback_depth;
    let (_block, _err) = poisoned_self_apply(&mut s, vec![toxic_tx(b"t1")]).await;

    let outcome = s
        .node
        .rollback_one_block(RollbackAuthority::ProductionSelfApply {
            failed_height: TIP + 1,
        })
        .await
        .expect("rollback errored");

    assert_eq!(
        outcome,
        RollbackOutcome::RefusedNotAuthorized,
        "O1 — the refusal must be its OWN outcome, distinct from RefusedNoMutation, \
         so the log, the counter and this test can tell 'the poison arm was \
         contained' apart from 'at genesis / cap reached / gapped store'"
    );
    assert_eq!(
        tip_of(&s.node).await,
        TIP,
        "O3 — REQ-FORK-002 AC-1: a production-side failure must never consume a \
         rollback of a block it did not itself apply"
    );
    assert_eq!(
        s.node.chain_state.read().await.best_height,
        TIP,
        "O4 — the sync tip and the chain state must agree on the kept tip"
    );
    assert_eq!(
        s.node
            .block_store
            .get_hash_by_height(TIP)
            .expect("block_store read"),
        Some(published),
        "O5 — the block at the kept tip must still be CANONICAL. The old door purges \
         the height-index entry for the block it rewinds (INC-I-144 fossil purge), so \
         a refusal that still ran that purge would leave the node serving a hole at a \
         height it has already gossiped"
    );
    assert_eq!(
        s.node.cumulative_rollback_depth, budget_before,
        "O6 — a refusal must not spend rollback budget. The 50-block cap is the \
         coordinator's last rung; charging it for a contained poison arm erodes the \
         rescue path that INC-I-190 depends on"
    );
}

// ===========================================================================
// T2 — the narrowing is not a removal.
// ===========================================================================

/// IP-TIP x P2. O1 O3.
///
/// REQ-FORK-002 AC-2 — Decision: a failure here says the guard over-refused and a
/// genuinely half-applied block is left standing as tip, which is strictly worse than
/// the defect being fixed. This is T1's control: it differs in exactly one input.
#[tokio::test]
async fn req_fork_002_ac2_b_a_failed_block_that_did_become_tip_is_still_undone() {
    let mut s = staged().await;

    let outcome = s
        .node
        .rollback_one_block(RollbackAuthority::ProductionSelfApply { failed_height: TIP })
        .await
        .expect("rollback errored");

    assert_eq!(
        outcome,
        RollbackOutcome::RolledBack,
        "O1 — `local_tip == failed_height` is the one case where the poison arm IS \
         undoing its own work. Refusing here removes a capability instead of \
         narrowing one"
    );
    assert_eq!(
        tip_of(&s.node).await,
        TIP - 1,
        "O3 — exactly one block undone"
    );
}

/// IP-TIP x P2 x P4, through the arm. O2 O3 O11.
///
/// REQ-FORK-002 AC-2 — Decision: a failure here says the arm and the door disagree
/// about which block failed, so the `rolled_back` series never moves and an operator
/// reading the containment counter cannot tell a rewind from a refusal.
#[tokio::test]
async fn req_fork_002_ac2_b2_the_arm_rewinds_when_the_failed_block_became_tip() {
    let mut s = staged().await;
    let block = s
        .node
        .block_store
        .get_block_by_height(TIP)
        .expect("block_store read")
        .expect("fixture: the tip block must be readable");
    let before = counter("rolled_back");

    s.node
        .handle_failed_self_apply(&block, TIP, anyhow::anyhow!("[TEST] apply failed at tip"))
        .await
        .expect("O2 — the arm must not propagate an error when the rollback succeeded");

    assert_eq!(tip_of(&s.node).await, TIP - 1, "O3");
    assert!(
        counter("rolled_back") >= before + 1.0,
        "O11 — the rewind path must move `{METRIC}{{outcome=\"rolled_back\"}}`. \
         Compared as a lower bound because the registry is process-global and this \
         binary runs its tests in parallel"
    );
}

// ===========================================================================
// T3 — the coordinator door is NOT narrowed (INC-I-190 rescue shape).
// ===========================================================================

/// IP-COORD x P3. O1 O3 O6.
///
/// REQ-FORK-002 — Decision: a failure here says M4.2 removed a capability instead of
/// narrowing one. The coordinator's `ShallowRollback` is the rung that unwedges the
/// INC-I-190 / INC-I-204 fleet; it must stay byte-identical to today, including
/// charging the budget it has always charged.
#[tokio::test]
async fn req_fork_002_c_the_coordinator_door_is_not_narrowed() {
    let mut s = staged().await;
    let budget_before = s.node.cumulative_rollback_depth;

    let outcome = s
        .node
        .rollback_one_block(RollbackAuthority::CoordinatorApproved { depth: 1 })
        .await
        .expect("rollback errored");

    assert_eq!(outcome, RollbackOutcome::RolledBack, "O1");
    assert_eq!(tip_of(&s.node).await, TIP - 1, "O3");
    assert_eq!(
        s.node.cumulative_rollback_depth,
        budget_before + 1,
        "O6 — the coordinator's rung is still counted. Silently stopping the count \
         would make the 50-block cap unreachable and the erosion it guards invisible"
    );
}

// ===========================================================================
// T4 — INV-FINALITY-001 / trap T12.
// ===========================================================================

/// IP-FIN x P2. O1 O3 O7.
///
/// REQ-FORK-002 AC-3 — Decision: a failure here says a production failure can erase
/// the finality marker. INV-FINALITY-001 records that exact mechanism: at h=57067 the
/// marker was erased to `None` by the poison rollback BEFORE a second finalization,
/// making both monotonicity clauses vacuous at that instant. Clearing is legal only
/// when a strictly-attested competitor supersedes the finalized block — NEVER as a
/// rollback side effect.
///
/// This is a structural seal, not a behaviour change: the guard means
/// `target == failed_height - 1` while the marker is at most `failed_height - 2`, so
/// `clear_finality_if_below_tip` could not fire on this path today either. The test
/// exists so a future edit to the production path cannot make it fire.
#[tokio::test]
async fn req_fork_002_ac3_d_a_production_rollback_never_clears_the_finality_marker() {
    let mut s = staged().await;
    finalize_at(&mut s, TIP - 2).await;

    let outcome = s
        .node
        .rollback_one_block(RollbackAuthority::ProductionSelfApply { failed_height: TIP })
        .await
        .expect("rollback errored");

    assert_eq!(
        outcome,
        RollbackOutcome::RolledBack,
        "O1 — the rewind must actually FIRE, or O7 below is asserted about a call \
         that did nothing"
    );
    assert_eq!(tip_of(&s.node).await, TIP - 1, "O3");
    assert_eq!(
        finality_marker(&s.node).await,
        Some(TIP - 2),
        "O7 — INV-FINALITY-001: the marker is not a production-failure side effect"
    );
}

/// IP-FIN x P3. O3 O7. The control for the test above.
///
/// REQ-FORK-002 AC-3 — Decision: a failure here says the finality skip was scoped too
/// widely and took the INV-SYNC-004 backstop (INC-I-081 Bug 4) with it — a node whose
/// tip has genuinely dropped below a stale marker would then have no way back, which
/// is the wedge INC-I-204 opened with.
#[tokio::test]
async fn req_fork_002_ac3_e_control_a_coordinator_rollback_below_the_marker_clears_it() {
    let mut s = staged().await;
    let marker = TIP - 2;
    finalize_at(&mut s, marker).await;

    // The backstop only fires once the post-rollback tip is strictly below the
    // marker, so the fixture must walk the coordinator's door down past it.
    for _ in 0..3 {
        s.node
            .rollback_one_block(RollbackAuthority::CoordinatorApproved { depth: 1 })
            .await
            .expect("rollback errored");
    }

    assert_eq!(tip_of(&s.node).await, marker - 1, "O3");
    assert_eq!(
        finality_marker(&s.node).await,
        None,
        "O7 — the coordinator path keeps the unchanged INV-SYNC-004 backstop"
    );
}

// ===========================================================================
// T5 — the purge is exact, and it runs on every path.
// ===========================================================================

/// IP-AHEAD x P4. O2 O3 O8 O11.
///
/// REQ-FORK-002 — Decision: a failure here says the containment left the toxic
/// transaction in the mempool, so the very next slot rebuilds the same poisoned block
/// and the arm re-fires forever; or it says the purge is not block-scoped and took
/// honest traffic with it, which turns one bad transaction into a local outage.
#[tokio::test]
async fn req_fork_002_f_the_purge_takes_the_blocks_txs_and_spares_the_honest_one() {
    let mut s = staged().await;
    let toxic = toxic_tx(b"t5-toxic");
    let honest = toxic_tx(b"t5-honest");
    {
        let mut mp = s.node.mempool.write().await;
        mp.add_system_transaction(toxic.clone(), TIP)
            .expect("fixture: the toxic tx must be in the mempool");
        mp.add_system_transaction(honest.clone(), TIP)
            .expect("fixture: the honest tx must be in the mempool");
    }
    let (block, err) = poisoned_self_apply(&mut s, vec![toxic.clone()]).await;
    let before = counter("tip_kept");

    s.node
        .handle_failed_self_apply(&block, TIP + 1, err)
        .await
        .expect("O2 — a contained poison arm is not an error for the caller");

    assert_eq!(tip_of(&s.node).await, TIP, "O3 — the tip is kept");
    let mp = s.node.mempool.read().await;
    assert!(
        !mp.contains(&toxic.hash()),
        "O8 — every transaction the failed block carried must be gone; the toxic one \
         is among them and the arm cannot tell which it was"
    );
    assert!(
        mp.contains(&honest.hash()),
        "O8 — a transaction that was NOT in the block must survive. This is the \
         purge's control: a fix that drained the mempool would satisfy the assertion \
         above for the wrong reason"
    );
    assert!(
        counter("tip_kept") >= before + 1.0,
        "O11 — `{METRIC}{{outcome=\"tip_kept\"}}` is this milestone's outcome metric"
    );
}

/// IP-CAP x P4. O2 O3 O8 O11.
///
/// REQ-FORK-002 — Decision: a failure here says the purge is still conditional on the
/// rollback outcome. `RefusedNoMutation` is reachable in the field (cap reached,
/// genesis, gapped store) and on that path the old arm kept the poison, so the node
/// would rebuild the same block every slot with no counter moving.
#[tokio::test]
async fn req_fork_002_g_the_purge_runs_on_the_refused_no_mutation_path_too() {
    let mut s = staged().await;
    let toxic = toxic_tx(b"t5b-toxic");
    let honest = toxic_tx(b"t5b-honest");
    {
        let mut mp = s.node.mempool.write().await;
        mp.add_system_transaction(toxic.clone(), TIP)
            .expect("fixture: toxic in mempool");
        mp.add_system_transaction(honest.clone(), TIP)
            .expect("fixture: honest in mempool");
    }
    let (block, err) = poisoned_self_apply(&mut s, vec![toxic.clone()]).await;

    // The failed block DID become tip as far as the authority is concerned, so the
    // guard passes and the CAP is what refuses — a different refusal, same duty.
    s.node.cumulative_rollback_depth = ROLLBACK_CAP;
    let before = counter("tip_kept");

    s.node
        .handle_failed_self_apply(&block, TIP, err)
        .await
        .expect("O2");

    assert_eq!(
        tip_of(&s.node).await,
        TIP,
        "O3 — the cap refused before any mutation"
    );
    let mp = s.node.mempool.read().await;
    assert!(!mp.contains(&toxic.hash()), "O8");
    assert!(mp.contains(&honest.hash()), "O8 — control");
    assert!(counter("tip_kept") >= before + 1.0, "O11");
}

/// IP-ERR x P6 x P4. O2 O8 O11.
///
/// REQ-FORK-002 — Decision: a failure here says the purge is still ordered AFTER the
/// rollback, where the `Err` arm's `return Err(e)` skips it entirely
/// (`production/mod.rs:637-643`). That is the worst cell of the three: the node has
/// both a half-rewound state AND the poison still queued.
///
/// The error is forced the way `rollback.rs` can actually fail from a test: a corrupt
/// `producer_snapshot` sends the undo path into `rebuild_producer_set_from_blocks`,
/// whose `ensure_blocks_present` guard then errors on a hole in the rebuild range
/// (the INC-I-152 / AUDIT-P1-003 shape). The UTXO undo has already run at that point,
/// so this is a genuine mid-rewind failure, not another refusal.
#[tokio::test]
async fn req_fork_002_h_the_purge_runs_even_when_the_rollback_errors() {
    let mut s = staged().await;
    let toxic = toxic_tx(b"t5c-toxic");
    let honest = toxic_tx(b"t5c-honest");
    {
        let mut mp = s.node.mempool.write().await;
        mp.add_system_transaction(toxic.clone(), TIP)
            .expect("fixture: toxic in mempool");
        mp.add_system_transaction(honest.clone(), TIP)
            .expect("fixture: honest in mempool");
    }
    let (block, err) = poisoned_self_apply(&mut s, vec![toxic.clone()]).await;

    let mut undo = s
        .node
        .state_db
        .get_undo(TIP)
        .expect("fixture: the tip must carry undo data");
    undo.producer_snapshot = vec![0xFFu8; 16];
    s.node
        .state_db
        .put_undo(TIP, &undo)
        .expect("fixture: put_undo");
    s.node
        .block_store
        .remove_canonical_entry(2, s.hashes[2])
        .expect("fixture: punch the hole the rebuild guard trips on");
    let before = counter("rollback_failed");

    let result = s.node.handle_failed_self_apply(&block, TIP, err).await;

    assert!(
        result.is_err(),
        "O2 — a rollback that failed part-way still needs manual intervention, and \
         `production/mod.rs:642` already propagates it. M4.2 moves the purge; it does \
         not silence the failure"
    );
    let mp = s.node.mempool.read().await;
    assert!(
        !mp.contains(&toxic.hash()),
        "O8 — the purge must have run BEFORE the rollback was attempted"
    );
    assert!(mp.contains(&honest.hash()), "O8 — control");
    assert!(
        counter("rollback_failed") >= before + 1.0,
        "O11 — the loudest cell must be the one an operator can alert on"
    );
}

// ===========================================================================
// T6 — the successor cover for what the old unconditional rollback discarded.
// ===========================================================================

/// IP-DIRT x P5. O2 O3 O9 O10.
///
/// REQ-FORK-002 — Decision: a failure here says the relaxation is UNSAFE. An aborted
/// `apply_block` can mutate the in-memory `ProducerSet` (tx loop) and `chain_state`
/// (`update_chain_state_for_block`) before it fails; today's unconditional rollback
/// discarded that residue as a side effect of retracting the parent. Keeping the tip
/// removes that side effect, so the residue must be discarded DIRECTLY — and an
/// in-memory `ProducerSet` that disagrees with the durable one becomes durable on the
/// very next applied block.
///
/// The residue is planted rather than produced by a real aborted apply: the planting
/// is asserted observable first, and it covers both mutation sites at once, which no
/// single natural failure point does.
#[tokio::test]
async fn req_fork_002_i_a_kept_tip_resyncs_the_aborted_applys_in_memory_residue() {
    let mut s = staged().await;
    let victim = *s.producers[0].public_key();
    let (block, err) = poisoned_self_apply(&mut s, vec![toxic_tx(b"t6")]).await;

    let durable_bonds = s
        .node
        .state_db
        .load_producer_set()
        .get_by_pubkey(&victim)
        .expect("fixture: producer 0 is durable")
        .bond_count;
    {
        let mut ps = s.node.producer_set.write().await;
        ps.get_by_pubkey_mut(&victim)
            .expect("fixture: producer 0 is in memory")
            .bond_count += 7;
    }
    {
        let mut cs = s.node.chain_state.write().await;
        cs.best_height = TIP + 1;
    }
    let planted = {
        let ps = s.node.producer_set.read().await;
        ps.get_by_pubkey(&victim).unwrap().bond_count
    };
    assert_ne!(
        planted, durable_bonds,
        "fixture: the residue must be observable BEFORE the call, or a resync that \
         does nothing passes this test"
    );

    s.node
        .handle_failed_self_apply(&block, TIP + 1, err)
        .await
        .expect("O2");

    assert_eq!(tip_of(&s.node).await, TIP, "O3 — the tip is kept");
    assert_eq!(
        s.node
            .producer_set
            .read()
            .await
            .get_by_pubkey(&victim)
            .expect("O9: the producer must survive the resync")
            .bond_count,
        durable_bonds,
        "O9 — the in-memory ProducerSet must be re-read from the StateDb, the same \
         authoritative source `Node::new()` uses at startup. Nothing durable was \
         written on this path, so the committed set IS the state at the kept tip"
    );
    let durable_cs = s
        .node
        .state_db
        .get_chain_state()
        .expect("O10: the durable chain state must be readable");
    let memory_cs = s.node.chain_state.read().await;
    assert_eq!(
        (
            memory_cs.best_height,
            memory_cs.best_hash,
            memory_cs.best_slot
        ),
        (
            durable_cs.best_height,
            durable_cs.best_hash,
            durable_cs.best_slot
        ),
        "O10 — a chain state naming a height the node never committed is the \
         INC-I-054 divergence shape: the next apply computes h+1 from a phantom tip"
    );
    assert_eq!(
        memory_cs.best_height, TIP,
        "O10 — and the value it resyncs to is the KEPT tip, not the failed height"
    );
}

// ===========================================================================
// T7 — no fork was manufactured.
// ===========================================================================

/// IP-AHEAD x P1 x P5. O2 O3.
///
/// REQ-FORK-002 AC-1 — Decision: a failure here says containment left the node unable
/// to extend its own chain, which converts the fork this milestone prevents into a
/// stall. The chain must continue from the tip the node PUBLISHED, so the next
/// canonical block is the one every peer already expects.
#[tokio::test]
async fn req_fork_002_j_the_kept_tip_extends_normally_on_the_next_slot() {
    let mut s = staged().await;
    let params = s.node.params.clone();
    let (block, err) = poisoned_self_apply(&mut s, vec![toxic_tx(b"t7")]).await;

    s.node
        .handle_failed_self_apply(&block, TIP + 1, err)
        .await
        .expect("O2");
    assert_eq!(
        tip_of(&s.node).await,
        TIP,
        "O3 — precondition for the retry"
    );

    let clean = build_block(
        TIP + 1,
        (TIP + 1) as u32,
        s.hashes[TIP as usize],
        &s.producers[0],
        &params,
        vec![],
    );
    s.node
        .apply_block(clean, ValidationMode::Light)
        .await
        .expect("the next slot must build on the kept tip");

    assert_eq!(
        tip_of(&s.node).await,
        TIP + 1,
        "O3 — the chain extends from the published tip: no competing branch was \
         created, so there is nothing for a peer to reorg around"
    );
}
