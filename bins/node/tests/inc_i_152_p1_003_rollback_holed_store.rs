//! INC-I-152 / AUDIT-P1-003 — the legacy rollback fallback MUTATES the UTXO set
//! before it knows whether it can finish rebuilding it.
//!
//! ## The defect (verified in source at `bins/node/src/node/rollback.rs`)
//!
//! `Node::rollback_one_block` takes a legacy "no undo data → rebuild from genesis"
//! fallback whenever `state_db.get_undo(local_height)` is `None`. That branch guards
//! itself with a **single-block** pre-check — `get_block_by_height(1)?.is_none()`
//! (lines 147-153) — and then enters an unconditionally **mutating** section: it takes
//! the UTXO write lock, calls `utxo.clear()` (line 163), and only afterwards loops
//! `for height in 1..=target_height`, replaying each block's transactions
//! (`spend_transaction` line 178, `add_transaction` line 185) and bailing out with
//! `?` on the first missing height (line 168).
//!
//! The pre-check answers "does block 1 exist?". The loop needs "is `1..=target_height`
//! DENSE?". Those are different questions, and the gap between them is the node's live
//! UTXO state. A **holed** store — `{1..=14}` present, a gap, then the tip present —
//! satisfies the pre-check, so control enters the mutating section, replays the whole
//! surviving prefix, and only then discovers the hole. The error propagates out and the
//! node is left **half-replayed**, with no undo of the damage.
//!
//! ## Correction to the audit's stated mechanism — MEASURED, not assumed
//!
//! The audit describes the damage as `utxo.clear()` wiping the set. That holds only for
//! the `UtxoSet::InMemory` variant. On the **production** `UtxoSet::RocksDb` variant
//! `clear()` is an explicit **no-op** (`crates/storage/src/utxo/set.rs:68-78`), and every
//! reachable production node holds RocksDb: continuous nodes via `from_state_db`
//! (init.rs:311), snap-synced nodes because INC-I-118 converts the snapshot copy straight
//! back (fork_recovery.rs:363) — and the snap-synced node IS the INC-I-152 scenario.
//!
//! So the real production damage is the OPPOSITE of a wipe. Because `clear()` did
//! nothing, the loop's `add_transaction` calls — direct `state_db.insert_utxo` writes on
//! this variant (set.rs:146-168, "used only in rollback paths") — **re-insert the outputs
//! of the surviving prefix on top of the live set**. Any such output the chain
//! legitimately SPENT at a later height is **resurrected**: a zombie UTXO, money from
//! nothing (the INC-I-041 class). The replay then aborts at the hole, so the compensating
//! spends living in the missing blocks never run. This file proves that concrete outcome
//! rather than the audit's stated one; the invariant is identical either way: **no
//! observable state may be mutated until completeness over `1..=target_height` is
//! proven.**
//!
//! ## The shape the fix must copy
//!
//! The sibling reorg path guards the identical rebuild correctly at
//! `block_handling.rs:~599` with `ensure_blocks_present(1, target_height)`
//! (`crates/storage/src/block_store/queries.rs:193` — a dense scan naming the FIRST
//! missing height, `low == 0` a no-op), refusing with `[FORK_GUARD_BACKFILL_REQUIRED]`
//! BEFORE touching state.
//!
//! ==================== OUTPUT CONTRACT ====================
//!
//! FUNCTION UNDER TEST: `Node::rollback_one_block(&mut self) -> Result<bool>`
//!   (`bins/node/src/node/rollback.rs:10`; branch under test = the legacy
//!    no-undo-data fallback at lines 140-202)
//!
//! OBSERVABLE OUTPUTS (full enumeration — receiver mutations, return value,
//! persistent-store writes):
//!   O1: return value — `Result<bool>`.
//!   O2: `self.utxo_set` (receiver mutation + persistent `state_db` cf_utxo write on
//!       the RocksDb variant). **THE load-bearing output.** Observed two ways: a named
//!       CANARY outpoint (created inside the surviving prefix, spent inside the hole)
//!       and the byte-exact `UtxoSet::serialize_canonical()` fingerprint — the same
//!       encoding the state root is computed over.
//!   O3: `self.chain_state.{best_height,best_hash}` (rollback.rs:212-217).
//!   O4: `self.producer_set` (rollback.rs:198-201) — via `bincode::serialize`.
//!   O5: `block_store` canonical entry at `local_height` — persistent store write
//!       (`remove_canonical_entry`, rollback.rs:226-229).
//!   O6: `sync_manager` local tip (rollback.rs:231-244).
//!
//! PATH UNDER TEST:
//!   P1: legacy fallback — `state_db.get_undo(local_height) == None`. The routine shape
//!       for a snap-synced / post-wipe node: state installed at the tip, no undo log.
//!       Modelled with the production `StateDb::prune_undo_above(0)` truncation API.
//!
//! INPUT PARTITIONS on P1 (density of `block_store` over `1..=target_height`):
//!   P1a HOLED   — `1..=14` present, `15..=18` MISSING, `19` (= target) and `20`
//!                 (= local tip) present. Passes the block-1-only pre-check, so control
//!                 reaches the mutation loop. **The INC-I-152 shape**: orphan chase
//!                 pulls the genesis blocks back, snap installs state at the tip, the
//!                 middle is never fetched.
//!   P1b DENSE   — `1..=19` all present. The rebuild is legitimate and MUST run.
//!   P1c NO-BLK1 — block 1 absent. The case the existing pre-check already covers;
//!                 its behavior must survive the replacement.
//!
//! MATRIX — 6 outputs x 3 partitions = 18 cells, every cell asserted:
//!   P1a | O1 not-a-post-mutation error | O2 canary still spent + bytes identical
//!       | O3 unchanged | O4 unchanged | O5 entry still present | O6 tip still 20
//!       -> `inc_i_152_p1_003_holed_store_rollback_must_not_mutate_utxo_set`   [RED]
//!   P1b | O1 Ok(true) | O2 non-empty + canary correctly still spent
//!       | O3 best_height == 19 | O4 rebuilt | O5 entry at 20 removed | O6 tip == 19
//!       -> `inc_i_152_p1_003_dense_store_rollback_still_rebuilds`       [PASS-LOCK]
//!   P1c | O1 refusal | O2 canary still spent + bytes identical | O3 unchanged
//!       | O4 unchanged | O5 entry still present | O6 tip still 20
//!       -> `inc_i_152_p1_003_missing_block_one_still_refuses`           [PASS-LOCK]
//!
//! PRE-FIX VERDICT (measured on this branch, not predicted):
//!   P1a **FAILS at O2**: the canary is RESURRECTED (`utxo.len()` 21 -> 22,
//!   `total_value()` 2000098000 -> 2000197000) because the prefix replay re-adds block
//!   10's outputs while block 17 — carrying the compensating spend — sits inside the
//!   hole. Note the discrimination that matters: pre-fix the call ALSO returns `Err`
//!   (`Rollback UTXO rebuild: missing block at height 15`), so an "an error was
//!   returned" assertion would pass on the broken code. Only O2 distinguishes "refused
//!   before touching state" from "failed after mutating it", which is why O2 is
//!   asserted FIRST in every refusal scenario.
//!   P1b and P1c **PASS** pre-fix and must keep passing post-fix — together they
//!   forbid the degenerate "fix" of refusing every rollback.
//!
//! O1 NOTE (deliberately under-constrained): whether the post-fix refusal surfaces as
//! `Ok(true)` (like the block-1 pre-check it replaces) or `Err` (like
//! `[FORK_GUARD_BACKFILL_REQUIRED]`) is the implementer's call — both are honest. What
//! this file pins: the error, if any, must NOT be the mid-rebuild error raised from
//! INSIDE the mutation section, and O2..O6 must be untouched.

use std::sync::Once;

use crypto::{Hash, KeyPair};
use doli_core::consensus::{self, ConsensusParams};
use doli_core::transaction::{Input, Output, OutputType, Transaction, TxType};
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Network};
use doli_node::node::Node;
use storage::{Outpoint, UtxoEntry};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

// ==================== Scenario geometry — the INC-I-152 post-wipe block-store shape ====================

/// Chain tip / `local_height` seen by `rollback_one_block`.
const CHAIN_LEN: u64 = 20;
/// `target_height` = `local_height - 1`, computed at rollback.rs:38.
const TARGET_HEIGHT: u64 = CHAIN_LEN - 1;
/// Blocks `1..=DENSE_PREFIX_TOP` survive the wipe — what orphan chase pulls back.
/// Includes block 1, which is why the pre-check at rollback.rs:150 waves this store
/// through, and it is the prefix the loop replays before it hits the hole.
const DENSE_PREFIX_TOP: u64 = 14;
/// First missing height — the hole opens here.
const HOLE_LOW: u64 = DENSE_PREFIX_TOP + 1; // 15
/// Last missing height. `TARGET_HEIGHT` (19) stays present because rollback.rs:79
/// needs the parent block before the fallback is even reached; the hole must
/// therefore sit strictly INSIDE `(1, target_height)`.
const HOLE_HIGH: u64 = TARGET_HEIGHT - 1; // 18

/// Height of the block whose Transfer CREATES the canary output.
/// Must be `<= DENSE_PREFIX_TOP` so the aborted replay re-adds it.
const CANARY_CREATED_AT: u64 = 10;
/// Height of the block whose Transfer SPENDS the canary output.
/// Must be inside the hole so the aborted replay never runs the compensating spend.
const CANARY_SPENT_AT: u64 = 17;

const CANARY_FUNDING_AMOUNT: u64 = 100_000;
const CANARY_AMOUNT: u64 = 99_000; // funding minus fee
const CANARY_SPEND_OUTPUT_AMOUNT: u64 = 98_000; // canary minus fee

// ==================== Environment bootstrap ====================

static ENV_INIT: Once = Once::new();
fn init_env() {
    ENV_INIT.call_once(|| {
        // Ensure the first NetworkParams::load is cached with devnet defaults.
        let _ = Network::Devnet.params();
    });
}

// ==================== Test scaffolding ====================

async fn make_node(n_producers: usize) -> (Node, Vec<KeyPair>, TempDir) {
    init_env();
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    // Keep block_reward positive throughout the test range so every block contributes
    // real state — the state we are proving does not get corrupted.
    node.params.blocks_per_era = 100_000;
    (node, producers, temp)
}

fn devnet_genesis_hash() -> Hash {
    doli_core::chainspec::ChainSpec::devnet().genesis_hash()
}

/// Assemble a block from an explicit transaction list. Height is carried implicitly
/// by the `prev_hash` chain and by the coinbase, so only the slot is needed here.
fn build_block_with_txs(
    block_slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    params: &ConsensusParams,
    txs: Vec<Transaction>,
) -> Block {
    let timestamp = params.genesis_time + (block_slot as u64 * params.slot_duration);
    let merkle_root = doli_core::block::compute_merkle_root(&txs);

    let header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash: devnet_genesis_hash(),
        timestamp,
        slot: block_slot,
        producer: *producer.public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, txs)
}

fn coinbase_for(height: u64, slot: u32, params: &ConsensusParams) -> Transaction {
    Transaction::new_coinbase(
        params.block_reward(height),
        consensus::reward_pool_pubkey_hash(),
        height,
        slot,
    )
}

/// Build a signed 1-in / 1-out Transfer. Pattern copied from
/// `bins/node/tests/inc_i_064_supply_conservation.rs:393-451`.
fn signed_transfer(spend: Outpoint, amount: u64, owner: &KeyPair, to: Hash) -> Transaction {
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input {
            prev_tx_hash: spend.tx_hash,
            output_index: spend.index,
            signature: crypto::Signature::default(),
            sighash_type: doli_core::transaction::SighashType::All,
            committed_output_count: 0,
            public_key: Some(*owner.public_key()),
        }],
        outputs: vec![Output {
            amount,
            pubkey_hash: to,
            output_type: OutputType::Normal,
            lock_until: 0,
            extra_data: vec![],
        }],
        extra_data: vec![],
    };
    let sighash = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&sighash, owner.private_key());
    tx
}

/// Apply plain coinbase-only blocks up to and including `up_to_height`, starting
/// from the node's current tip. Uses the REAL `apply_block`, so the resulting UTXO
/// set / producer set / chain state / sync tip are genuine node state.
async fn apply_plain_up_to(
    node: &mut Node,
    producers: &[KeyPair],
    up_to_height: u64,
    params: &ConsensusParams,
) {
    let mut prev = node.chain_state.read().await.best_hash;
    let start_h = node.chain_state.read().await.best_height;
    for h in (start_h + 1)..=up_to_height {
        let block = build_block_with_txs(
            h as u32,
            prev,
            &producers[(h as usize) % producers.len()],
            params,
            vec![coinbase_for(h, h as u32, params)],
        );
        prev = block.hash();
        node.apply_block(block, ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("setup: apply_block failed at h={h}: {e}"));
    }
}

/// Apply one block carrying `[coinbase, user_tx]`.
async fn apply_block_with_transfer(
    node: &mut Node,
    producers: &[KeyPair],
    height: u64,
    params: &ConsensusParams,
    user_tx: Transaction,
) {
    let prev = node.chain_state.read().await.best_hash;
    let block = build_block_with_txs(
        height as u32,
        prev,
        &producers[(height as usize) % producers.len()],
        params,
        vec![coinbase_for(height, height as u32, params), user_tx],
    );
    node.apply_block(block, ValidationMode::Light)
        .await
        .unwrap_or_else(|e| panic!("setup: apply_block with transfer failed at h={height}: {e}"));
}

// ==================== State fingerprint — the O2..O6 observation surface ====================

/// Byte-exact snapshot of every output `rollback_one_block` can mutate.
///
/// `utxo_bytes` is load-bearing: `serialize_canonical()` is the same deterministic
/// encoding the state root is computed over, so equality on it detects ANY mutation.
/// `canary_present` names the single entry whose resurrection is the concrete
/// production harm, so failures point at the harm rather than at a byte diff.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StateFingerprint {
    utxo_len: usize,                   // O2
    utxo_total_value: u64,             // O2
    utxo_bytes: Vec<u8>,               // O2
    canary_present: bool,              // O2 — the resurrection canary
    producer_bytes: Vec<u8>,           // O4
    best_height: u64,                  // O3
    best_hash: Hash,                   // O3
    sync_local_height: u64,            // O6
    tip_canonical_entry: Option<Hash>, // O5
}

async fn fingerprint(node: &Node, tip_height: u64, canary: &Outpoint) -> StateFingerprint {
    let (utxo_len, utxo_total_value, utxo_bytes, canary_present) = {
        let utxo = node.utxo_set.read().await;
        (
            utxo.len(),
            utxo.total_value(),
            utxo.serialize_canonical(),
            utxo.contains(canary),
        )
    };
    let producer_bytes = {
        let producers = node.producer_set.read().await;
        bincode::serialize(&*producers).expect("ProducerSet serialization")
    };
    let (best_height, best_hash) = {
        let cs = node.chain_state.read().await;
        (cs.best_height, cs.best_hash)
    };
    let sync_local_height = node.sync_manager.read().await.local_tip().0;
    let tip_canonical_entry = node
        .block_store
        .get_hash_by_height(tip_height)
        .expect("block_store get_hash_by_height failed");

    StateFingerprint {
        utxo_len,
        utxo_total_value,
        utxo_bytes,
        canary_present,
        producer_bytes,
        best_height,
        best_hash,
        sync_local_height,
        tip_canonical_entry,
    }
}

/// The refusal contract: a rollback that refuses MUST leave every observable output
/// exactly as it found it.
///
/// O2 is checked FIRST, because it is the only output that distinguishes "refused
/// before touching state" (correct) from "failed after mutating it" (the
/// AUDIT-P1-003 defect). Both outcomes return an error.
fn assert_refused_without_touching_state(
    scenario: &str,
    pre: &StateFingerprint,
    post: &StateFingerprint,
    result: &anyhow::Result<bool>,
) {
    // ---- O2a: the resurrection canary. THE assertion. ----
    assert!(
        !post.canary_present,
        "AUDIT-P1-003 [{scenario}] / O2: the rollback RESURRECTED a spent UTXO. The \
         canary was created by a Transfer in block {CANARY_CREATED_AT} and spent by a \
         Transfer in block {CANARY_SPENT_AT}. The legacy fallback replays the surviving \
         prefix 1..={DENSE_PREFIX_TOP} (`utxo.add_transaction`, rollback.rs:185 — a \
         direct `state_db.insert_utxo` on the production RocksDb backend), re-adding \
         block {CANARY_CREATED_AT}'s outputs, and then aborts at height {HOLE_LOW} \
         before reaching block {CANARY_SPENT_AT}, whose compensating spend is inside \
         the hole. Money is created from nothing and the node is left half-replayed. \
         Completeness must be proven with \
         `block_store.ensure_blocks_present(1, target_height)` BEFORE the write lock is \
         taken — the shape already used by the reorg guard at block_handling.rs:~599. \
         utxo.len(): {} -> {} | utxo.total_value(): {} -> {} | rollback returned: {:?}",
        pre.utxo_len,
        post.utxo_len,
        pre.utxo_total_value,
        post.utxo_total_value,
        result.as_ref().map_err(|e| e.to_string()),
    );

    // ---- O2b: nothing else in the set moved either. ----
    assert!(
        post.utxo_bytes == pre.utxo_bytes,
        "AUDIT-P1-003 [{scenario}] / O2: the UTXO set was MUTATED by a rollback that \
         could not complete — `serialize_canonical()` (the encoding the state root is \
         computed over) differs. utxo.total_value(): {} -> {}",
        pre.utxo_total_value,
        post.utxo_total_value,
    );

    // ---- O1: the refusal must not be the mid-rebuild error. ----
    // Post-fix the refusal may be Ok(true) or Err; what it may NEVER be is the error
    // raised from INSIDE the mutation section, which is itself proof that the loop ran.
    if let Err(e) = result {
        let msg = e.to_string();
        assert!(
            !msg.contains("Rollback UTXO rebuild: missing block at height"),
            "AUDIT-P1-003 [{scenario}] / O1: rollback failed with the MID-REBUILD error \
             `{msg}` — that error is only reachable from rollback.rs:168, i.e. AFTER the \
             UTXO write lock was taken and the prefix replay had already run. The \
             incompleteness must be detected before the mutation section is entered."
        );
    }

    // ---- O3: chain state must not move. ----
    assert_eq!(
        post.best_height, pre.best_height,
        "[{scenario}] / O3: chain_state.best_height moved on a refused rollback"
    );
    assert_eq!(
        post.best_hash, pre.best_hash,
        "[{scenario}] / O3: chain_state.best_hash moved on a refused rollback"
    );

    // ---- O4: producer set must not move. ----
    assert_eq!(
        post.producer_bytes,
        pre.producer_bytes,
        "[{scenario}] / O4: producer_set was mutated on a refused rollback ({} -> {} \
         bytes)",
        pre.producer_bytes.len(),
        post.producer_bytes.len()
    );

    // ---- O5: the tip's canonical entry must survive. ----
    assert_eq!(
        post.tip_canonical_entry, pre.tip_canonical_entry,
        "[{scenario}] / O5: block_store canonical entry at the tip was removed \
         (remove_canonical_entry, rollback.rs:226-229) on a refused rollback"
    );

    // ---- O6: the sync manager's local tip must survive. ----
    assert_eq!(
        post.sync_local_height, pre.sync_local_height,
        "[{scenario}] / O6: sync_manager local tip moved on a refused rollback"
    );
}

// ==================== Fixture ====================

/// Drive the node into the exact INC-I-152 pre-rollback shape:
///   * the PRODUCTION RocksDb UTXO backend,
///   * a real `CHAIN_LEN`-block chain applied through `apply_block`, containing a
///     Transfer at `CANARY_CREATED_AT` whose output is spent by a Transfer at
///     `CANARY_SPENT_AT`,
///   * NO undo log (snap-synced / post-wipe node) -> `rollback_one_block` takes the
///     legacy rebuild-from-genesis fallback.
///
/// Returns the canary outpoint: created inside the surviving prefix, spent inside
/// the hole. It must be ABSENT from the UTXO set at the end of setup.
async fn build_node_at_tip(n_producers: usize) -> (Node, Outpoint, TempDir) {
    let (mut node, producers, temp) = make_node(n_producers).await;
    let params = node.params.clone();

    // Install the PRODUCTION UTXO backend (init.rs:311; fork_recovery.rs:363 for the
    // snap-synced node). Load-bearing, not cosmetic: `new_for_test` leaves the detached
    // `InMemory` variant, and since storage Phase 3 `apply_block` writes UTXOs only
    // through `BlockBatch` into state_db (apply_block/mod.rs:189), an InMemory set on a
    // test node stays permanently EMPTY and could evidence nothing. It also matters
    // semantically — see the module doc's mechanism correction.
    {
        let mut utxo = node.utxo_set.write().await;
        *utxo = storage::UtxoSet::from_state_db(node.state_db.clone());
    }

    // Blocks 1..=(CANARY_CREATED_AT - 1): plain.
    apply_plain_up_to(&mut node, &producers, CANARY_CREATED_AT - 1, &params).await;

    // Seed a spendable output for the canary's owner. On the RocksDb backend
    // `UtxoSet::insert` writes straight through to state_db's cf_utxo, so this single
    // call is the whole funding step.
    let owner = KeyPair::generate();
    let owner_pkh =
        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, owner.public_key().as_bytes());
    let funding_outpoint = Outpoint::new(crypto::hash::hash(b"inc_i_152_p1_003_funding"), 0);
    let funding_entry = UtxoEntry {
        output: Output {
            amount: CANARY_FUNDING_AMOUNT,
            pubkey_hash: owner_pkh,
            output_type: OutputType::Normal,
            lock_until: 0,
            extra_data: vec![],
        },
        height: CANARY_CREATED_AT - 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    {
        let mut utxo = node.utxo_set.write().await;
        utxo.insert(funding_outpoint, funding_entry)
            .expect("setup: funding insert failed");
    }

    // Block CANARY_CREATED_AT carries the Transfer that CREATES the canary.
    let create_tx = signed_transfer(funding_outpoint, CANARY_AMOUNT, &owner, owner_pkh);
    let canary = Outpoint::new(create_tx.hash(), 0);
    apply_block_with_transfer(&mut node, &producers, CANARY_CREATED_AT, &params, create_tx).await;
    assert!(
        node.utxo_set.read().await.contains(&canary),
        "setup: the canary must exist after block {CANARY_CREATED_AT}"
    );

    // Blocks (CANARY_CREATED_AT + 1)..=(CANARY_SPENT_AT - 1): plain.
    apply_plain_up_to(&mut node, &producers, CANARY_SPENT_AT - 1, &params).await;

    // Block CANARY_SPENT_AT carries the Transfer that SPENDS the canary. This block
    // is inside the hole, so an aborted prefix replay never runs this spend.
    let spend_tx = signed_transfer(canary, CANARY_SPEND_OUTPUT_AMOUNT, &owner, owner_pkh);
    apply_block_with_transfer(&mut node, &producers, CANARY_SPENT_AT, &params, spend_tx).await;
    assert!(
        !node.utxo_set.read().await.contains(&canary),
        "setup: the canary must be SPENT after block {CANARY_SPENT_AT} — this is the \
         state the defect resurrects"
    );

    // Blocks (CANARY_SPENT_AT + 1)..=CHAIN_LEN: plain.
    apply_plain_up_to(&mut node, &producers, CHAIN_LEN, &params).await;

    assert_eq!(
        node.chain_state.read().await.best_height,
        CHAIN_LEN,
        "setup: chain must be at height {CHAIN_LEN}"
    );
    assert_eq!(
        node.sync_manager.read().await.local_tip().0,
        CHAIN_LEN,
        "setup: sync_manager local tip must be {CHAIN_LEN} — rollback_one_block reads \
         `local_height` from here (rollback.rs:11-14), not from chain_state"
    );

    // Erase the undo log. This is what a snap-synced or freshly-wiped node looks like:
    // state installed at the tip, nothing to undo. `prune_undo_above` is the production
    // truncation API (crates/storage/src/state_db/undo.rs:50).
    node.state_db.prune_undo_above(0);
    assert!(
        node.state_db.get_undo(CHAIN_LEN).is_none(),
        "setup: undo data at h={CHAIN_LEN} must be absent so rollback_one_block takes \
         the legacy rebuild-from-genesis fallback (rollback.rs:140)"
    );

    (node, canary, temp)
}

fn assert_block(node: &Node, h: u64, present: bool, why: &str) {
    let got = node
        .block_store
        .get_block_by_height(h)
        .expect("block_store read failed")
        .is_some();
    assert_eq!(
        got, present,
        "precondition: block at h={h} must be present={present} so that {why}"
    );
}

/// Punch the hole: drop the canonical entries for `low..=high`.
///
/// `get_block_by_height` resolves height -> hash -> body (queries.rs:171-177) and
/// `ensure_blocks_present` reads the same height index, so dropping the canonical entry
/// makes those heights invisible to BOTH the pre-check and the rebuild loop — the same
/// observable shape as a post-wipe node that never fetched those blocks at all.
fn punch_hole(node: &Node, low: u64, high: u64) {
    for h in low..=high {
        let hash = node
            .block_store
            .get_hash_by_height(h)
            .expect("block_store get_hash_by_height failed")
            .unwrap_or_else(|| panic!("setup: expected a canonical entry at h={h}"));
        node.block_store
            .remove_canonical_entry(h, hash)
            .expect("remove_canonical_entry failed");
        assert!(
            node.block_store
                .get_block_by_height(h)
                .expect("block_store read failed")
                .is_none(),
            "setup: h={h} must be invisible to get_block_by_height after the hole is punched"
        );
    }
}

// ==================== P1a — THE RED TEST (must FAIL pre-fix, on O2) ====================
//
// OUTPUT CONTRACT: Path P1 / partition P1a (HOLED store). All six cells O1..O6 are asserted;
// the per-cell mapping is the MATRIX in the module doc.
//
/// A node whose block store is holed — blocks `1..=14` present, `15..=18` missing,
/// `19` and `20` present — must have its rollback REFUSED BEFORE any state is
/// touched, exactly as `execute_reorg` refuses with `[FORK_GUARD_BACKFILL_REQUIRED]`.
///
/// Pre-fix this FAILS on O2: the block-1-only pre-check passes, the prefix replay
/// re-adds block 10's Transfer outputs (resurrecting an output the chain spent in
/// block 17, which is inside the hole), and the loop then dies at height 15 — leaving
/// the node with inflated, half-replayed state and an error.
#[tokio::test]
async fn inc_i_152_p1_003_holed_store_rollback_must_not_mutate_utxo_set() {
    let (mut node, canary, _tmp) = build_node_at_tip(3).await;

    // Engineer the INC-I-152 shape.
    punch_hole(&node, HOLE_LOW, HOLE_HIGH);

    // PRECONDITION A — the enabler: block 1 is present, so the block-1-only pre-check
    // (rollback.rs:150) does NOT fire and control proceeds into the mutation section.
    assert_block(
        &node,
        1,
        true,
        "the block-1-only pre-check waves the store through",
    );
    // PRECONDITION B — the parent at target_height is present, so the early return at
    // rollback.rs:81-84 does not fire and the legacy fallback is actually reached.
    assert_block(&node, TARGET_HEIGHT, true, "the fallback is reached");
    // PRECONDITION C — the block that CREATES the canary survived; the one that SPENDS
    // it did not. This asymmetry is the whole mechanism.
    assert_block(
        &node,
        CANARY_CREATED_AT,
        true,
        "the aborted replay re-adds it",
    );
    assert_block(
        &node,
        CANARY_SPENT_AT,
        false,
        "the aborted replay never runs the compensating spend",
    );

    // PRECONDITION D — the store really is holed, and the dense check the fix will use
    // already knows it, before the write lock is taken.
    assert!(
        node.block_store
            .ensure_blocks_present(1, TARGET_HEIGHT)
            .is_err(),
        "precondition: ensure_blocks_present(1, {TARGET_HEIGHT}) must already report the \
         hole — the information the fallback needs is available BEFORE any mutation"
    );

    // PRECONDITION E — there is real state to corrupt, and the canary is spent.
    let pre = fingerprint(&node, CHAIN_LEN, &canary).await;
    assert!(
        pre.utxo_len > 0 && pre.utxo_total_value > 0,
        "precondition: the UTXO set must be populated before the rollback \
         (len={}, total_value={})",
        pre.utxo_len,
        pre.utxo_total_value
    );
    assert!(
        !pre.canary_present,
        "precondition: the canary must be SPENT before the rollback"
    );

    // Drive the REAL entry point. The Result is captured, never unwrapped: pre-fix it
    // is `Err`, and unwrapping here would abort the test before the assertion that
    // actually matters (O2) ever runs.
    let result = node.rollback_one_block().await;

    let post = fingerprint(&node, CHAIN_LEN, &canary).await;
    assert_refused_without_touching_state("P1a HOLED", &pre, &post, &result);
}

// ==================== P1b — PASS-LOCK: a DENSE store must still rebuild ====================
//
// OUTPUT CONTRACT: Path P1 / partition P1b (DENSE store). All six cells O1..O6 are asserted;
// the per-cell mapping is the MATRIX in the module doc.
//
/// The legitimate case: no undo data, but the block store IS dense over
/// `1..=target_height`. The rebuild-from-genesis fallback must run to completion.
///
/// This is the guard against "fixing" AUDIT-P1-003 by refusing every rollback. It
/// must pass both pre- and post-fix.
#[tokio::test]
async fn inc_i_152_p1_003_dense_store_rollback_still_rebuilds() {
    let (mut node, canary, _tmp) = build_node_at_tip(3).await;

    // No hole this time — assert density explicitly so this test's premise is
    // self-evident rather than inherited.
    node.block_store
        .ensure_blocks_present(1, TARGET_HEIGHT)
        .expect("precondition: the block store must be DENSE over 1..=TARGET_HEIGHT");

    let pre = fingerprint(&node, CHAIN_LEN, &canary).await;
    assert!(
        pre.utxo_len > 0,
        "precondition: the UTXO set must be populated before the rollback"
    );

    // O1 — the rollback must complete.
    let rolled = node.rollback_one_block().await.expect(
        "P1b / O1: a rollback over a DENSE block store must NOT error — a fix for \
         AUDIT-P1-003 that refuses this case has traded a data-corruption bug for a \
         liveness bug",
    );
    assert!(
        rolled,
        "P1b / O1: rollback_one_block must report success (true) over a dense store"
    );

    let post = fingerprint(&node, CHAIN_LEN, &canary).await;

    // O3 — the chain actually rewound.
    assert_eq!(
        post.best_height, TARGET_HEIGHT,
        "P1b / O3: chain_state.best_height must be {TARGET_HEIGHT} after the rollback \
         (was {})",
        pre.best_height
    );
    assert_ne!(
        post.best_hash, pre.best_hash,
        "P1b / O3: chain_state.best_hash must move to the parent block"
    );

    // O2 — the replay ran over the whole range and stayed correct: the set is
    // non-empty, and the canary is still spent because block CANARY_SPENT_AT was
    // present and its compensating spend executed.
    assert!(
        post.utxo_len > 0 && post.utxo_total_value > 0,
        "P1b / O2: the UTXO set must survive a dense-store rollback. len={} \
         total_value={}",
        post.utxo_len,
        post.utxo_total_value
    );
    assert!(
        !post.canary_present,
        "P1b / O2: over a DENSE store the replay reaches block {CANARY_SPENT_AT}, so \
         the canary created in block {CANARY_CREATED_AT} must remain SPENT. If it is \
         present here, the rebuild is resurrecting UTXOs even when the store is \
         complete."
    );

    // O4 — the producer set survived the rebuild in a usable form.
    assert!(
        !post.producer_bytes.is_empty(),
        "P1b / O4: producer_set must still serialize after \
         rebuild_producer_set_from_blocks (rollback.rs:198-201)"
    );

    // O5 — the rolled-back block's canonical entry was purged (INC-I-144,
    // rollback.rs:226-229).
    assert!(
        post.tip_canonical_entry.is_none(),
        "P1b / O5: the canonical entry at h={CHAIN_LEN} must be removed by a completed \
         rollback"
    );

    // O6 — the sync manager followed the chain down.
    assert_eq!(
        post.sync_local_height, TARGET_HEIGHT,
        "P1b / O6: sync_manager local tip must be {TARGET_HEIGHT} after the rollback"
    );
}

// ==================== P1c — PASS-LOCK: block 1 missing must still refuse ====================
//
// OUTPUT CONTRACT: Path P1 / partition P1c (block 1 absent). All six cells O1..O6 are asserted;
// the per-cell mapping is the MATRIX in the module doc.
//
/// The job the ORIGINAL pre-check does must survive its replacement: with block 1
/// absent the rebuild-from-genesis is impossible and the rollback must be refused
/// without touching state.
///
/// Passes pre-fix (the block-1 pre-check returns early at rollback.rs:150-152) and
/// must keep passing post-fix (`ensure_blocks_present(1, target)` fails at the very
/// first height it checks).
#[tokio::test]
async fn inc_i_152_p1_003_missing_block_one_still_refuses() {
    let (mut node, canary, _tmp) = build_node_at_tip(3).await;

    // Remove block 1 only — everything else stays dense.
    punch_hole(&node, 1, 1);

    // PRECONDITION — the parent at `target_height` is still present, so the fallback
    // really is the branch under test.
    assert_block(
        &node,
        TARGET_HEIGHT,
        true,
        "the fallback is the branch under test",
    );

    let pre = fingerprint(&node, CHAIN_LEN, &canary).await;
    assert!(
        pre.utxo_len > 0,
        "precondition: the UTXO set must be populated before the rollback"
    );

    let result = node.rollback_one_block().await;

    let post = fingerprint(&node, CHAIN_LEN, &canary).await;
    assert_refused_without_touching_state("P1c NO-BLK1", &pre, &post, &result);
}
