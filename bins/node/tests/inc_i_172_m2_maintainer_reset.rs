//! INC-I-172 M2 — CATEGORY A: reproduction tests for the maintainer-set RESET
//! BUTTON and for the non-deterministic bootstrap derivation. These compile and
//! RUN against the CURRENT tree and MUST FAIL until M2 lands.
//!
//! Findings under test
//! -------------------
//! * **AUDIT-P1-013 / FM-01** (P1) — `Node::maybe_bootstrap_maintainer_set`
//!   (`bins/node/src/node/periodic.rs:35-92`) is guarded by
//!   `state.set.is_fully_bootstrapped()`, which is `members.len() >= 5`
//!   (`crates/core/src/maintainer.rs:254-256`). It is called UNCONDITIONALLY on
//!   every applied block from `bins/node/src/node/apply_block/state_update.rs:214`
//!   — not epoch-gated, despite its own doc comment. So a SUCCESSFUL
//!   `RemoveMaintainer` (set 5 -> 4) is silently REVERTED to the genesis five on
//!   the very next block, roughly 10 s later on a live network. The on-chain
//!   trust root that M1 made authoritative therefore cannot be rotated durably.
//!   `periodic.rs:74` assigns `state.set = set` WHOLESALE, so the removal is not
//!   merely topped up — it is erased.
//! * **AUDIT-P3-014** (P3, LIVE for >5 tied producers) —
//!   `periodic.rs:53,59` builds the bootstrap list from
//!   `producers.all_producers()` (a `HashMap::values()` walk,
//!   `crates/storage/src/producer/set_core.rs:398-400`) and then applies a
//!   STABLE `sort_by_key(|p| p.registered_at)`. Every genesis producer carries
//!   `registered_at: 0` (`crates/storage/src/producer/set_registration.rs:192`),
//!   so the whole set is one tie group and the stable sort preserves the
//!   HashMap's random iteration order. With EXACTLY five genesis producers
//!   `take(5)` consumes the entire tie group, so MEMBERSHIP is stable and only
//!   the ORDER varies — which is why this is dead on today's mainnet. With MORE
//!   than five it selects a random 5-subset, and MEMBERSHIP itself diverges
//!   node by node. The identical gap exists at
//!   `bins/node/src/node/apply_block/governance.rs:116-122`
//!   (`derive_ad_hoc_maintainer_set`), which IS consensus-visible through
//!   `ProtocolActivation` acceptance.
//!
//! Requirements
//! ------------
//! * REQ-172-005 (Must) — fresh-sync / wiped nodes converge on the same root.
//! * REQ-172-010 (Must) — the root is a replay-complete function of
//!   (genesis seed, all governance actions <= H), not of live producer state.
//! * REQ-172-012 (Must) — governance decisions are durable, not advisory.
//!
//! Spec: `specs/maintainer-trust-root-architecture.md` §F2.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT
//! ---------------------------------------------------------------------------
//! Functions under test:
//!   G1: `Node::process_transaction_governance(&self, &Transaction, u64, &ProducerSet)
//!        -> Option<(u32, u64)>`
//!   G2: `Node::maybe_bootstrap_maintainer_set(&self, u64)` (returns unit; reached
//!        in production ONLY from `apply_block/state_update.rs:214`, i.e. once per
//!        applied block)
//!   G3: `Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>` — the
//!        real per-block driver that invokes G2
//!
//! OUTPUTS
//!   O1 (receiver mutation) `maintainer_state.set.members` — the trust root itself
//!   O2 (receiver mutation) `maintainer_state.set.threshold` — derived from O1;
//!      asserted separately because a silent re-derivation restores it too
//!   O3 (receiver mutation) `maintainer_state.last_derived_height`
//!   O4 (persistent store write) `<data_dir>/maintainer_state.bin` — the file the
//!      updater reads as its install trust root (`trust_root_wiring.rs:95`);
//!      asserted through a fresh `MaintainerState::load`, i.e. what a RESTARTED
//!      node would see
//!   O5 (return of G1) `Option<(u32, u64)>` — None for Add/Remove by construction
//!   O6 (return of G3) `Result<()>`
//!   O7 (mutable params) — G1/G2 take `&self`; G3 takes `&mut self` but the
//!      maintainer root is behind an `Arc<RwLock<_>>`, covered by O1..O4
//!   O8 (chain_state.best_height) — asserted to prove the block really applied,
//!      so a "root unchanged" result cannot come from a block that never landed
//!
//! PATHS
//!   PR-removed  — the governance tx was authorized; the root has 4 members
//!   PR-reverted — a later block re-derived the root back to the genesis five
//!   PD-stable   — every independently constructed node derives the same members
//!   PD-diverged — at least two nodes derive different members
//!
//! INPUT PARTITIONS
//!   IP-1  5 producers; valid 3-distinct-signer RemoveMaintainer; then ONE more
//!         applied block                          -> MUST stay PR-removed (RED today)
//!   IP-2  IP-1 but observed through the PERSISTED file after the extra block
//!                                                -> MUST stay PR-removed (RED today)
//!   IP-3  5 producers; NO governance tx; one applied block
//!                                                -> control, root == genesis five (GREEN)
//!   IP-4  8 producers all tied at registered_at == 0, N independent nodes
//!                                                -> MUST be PD-stable (RED today)
//!   IP-5  5 producers, N independent nodes       -> control: MEMBERSHIP stable even
//!                                                   today (AUDIT-P3-014 kill), so a
//!                                                   failure in IP-4 is caused by the
//!                                                   tie-group SIZE, not by the harness
//!
//! MATRIX
//!   O1 x {IP-1, IP-2, IP-3, IP-4, IP-5}          = 5 assertions
//!   O2 x {IP-1}                                   = 1 assertion
//!   O3 x {IP-1}                                   = 1 assertion
//!   O4 x {IP-2}                                   = 1 assertion
//!   O5 x {IP-1}                                   = 1 assertion
//!   O6 x {IP-1, IP-3}                             = 2 assertions (via expect)
//!   O8 x {IP-1, IP-3}                             = 2 assertions
//!   O7 — structurally covered by O1..O4.
//!
//! ANTI-VACUITY
//!   IP-1 <-> IP-3 — same node shape, same applied block; only the governance tx
//!                   differs. IP-3 passing proves the harness bootstraps a real
//!                   five-member root, so IP-1's failure is the REVERT, not a
//!                   root that never existed.
//!   IP-4 <-> IP-5 — byte-identical harness; only the producer COUNT differs
//!                   (8 vs 5). IP-5 passing proves the harness is not simply
//!                   observing per-node key generation noise.

use std::sync::Arc;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::maintainer::{MaintainerChangeData, MaintainerSignature};
use doli_core::transaction::TxType;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Transaction};
use doli_node::node::Node;
use storage::MaintainerState;
use tempfile::TempDir;
use tokio::sync::RwLock;
use vdf::{VdfOutput, VdfProof};

/// Number of independently constructed nodes sampled for the determinism
/// partitions. Each node builds its OWN `ProducerSet`, hence its own `HashMap`
/// with its own `RandomState` seed, which is the source of the divergence.
const DETERMINISM_SAMPLES: usize = 12;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

async fn make_node(producers: &[KeyPair]) -> (Node, TempDir) {
    let temp = TempDir::new().unwrap();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.to_vec())
        .await
        .expect("Node::new_for_test failed");
    node.set_maintainer_state(Arc::new(RwLock::new(MaintainerState::default())));
    (node, temp)
}

/// A coinbase-only block, copied from `bins/node/tests/inc_i_147_d4_rollback_reapply.rs`.
/// It carries no governance transaction: the whole point of FM-01 is that an
/// ORDINARY block reverts the trust root.
fn build_block(
    height: u64,
    slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    params: &ConsensusParams,
) -> Block {
    let reward = params.block_reward(height);
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, 0);
    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);
    let merkle_root = doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase));
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

    Block::new(header, vec![coinbase])
}

fn sig(kp: &KeyPair, message: &[u8]) -> MaintainerSignature {
    MaintainerSignature::new(
        *kp.public_key(),
        crypto::signature::sign(message, kp.private_key()),
    )
}

/// A `RemoveMaintainer` transaction authorized by THREE DISTINCT maintainers,
/// none of them the target. This is a legitimate governance action under both
/// today's entry-counting verifier and the M2 distinct-signer verifier, so the
/// test stays valid across the fix.
fn remove_maintainer_tx(target: &PublicKey, signers: &[&KeyPair]) -> Transaction {
    let mut data = MaintainerChangeData::new(*target, vec![]);
    let message = data.signing_message(false);
    data.signatures = signers.iter().map(|kp| sig(kp, &message)).collect();

    Transaction {
        version: 1,
        tx_type: TxType::RemoveMaintainer,
        inputs: vec![],
        outputs: vec![],
        extra_data: data.to_bytes(),
    }
}

async fn root_members(node: &Node) -> Vec<PublicKey> {
    node.maintainer_state
        .as_ref()
        .expect("maintainer_state must be attached")
        .read()
        .await
        .set
        .members
        .clone()
}

// ---------------------------------------------------------------------------
// IP-1 / IP-2 — FM-01, the reset button
// ---------------------------------------------------------------------------

/// IP-1 + IP-2. O1..O6, O8 x PR-removed. **AUDIT-P1-013 / FM-01 — MUST FAIL TODAY.**
///
/// Sequence: seed the five, remove one with a valid quorum, then apply ONE
/// ordinary block. The removal must survive.
#[tokio::test]
async fn a_removed_maintainer_must_not_return_on_the_next_block() {
    let producers: Vec<KeyPair> = (0..5).map(|_| KeyPair::generate()).collect();
    let (mut node, temp) = make_node(&producers).await;
    let params = node.params.clone();
    let data_dir = node.config.data_dir.clone();

    // --- Seed the genesis five, exactly as apply_block does at height 0. ---
    node.maybe_bootstrap_maintainer_set(0).await;
    let seeded = root_members(&node).await;
    assert_eq!(
        seeded.len(),
        5,
        "setup: the maintainer root must be seeded with the genesis five"
    );

    // --- A legitimate 3-of-5 removal. ---
    // Pick a target that is IN the seeded root, and three distinct signers that
    // are in it and are not the target.
    let target = seeded[0];
    let signers: Vec<&KeyPair> = producers
        .iter()
        .filter(|kp| *kp.public_key() != target)
        .take(3)
        .collect();
    assert_eq!(signers.len(), 3, "setup: three distinct non-target signers");

    let tx = remove_maintainer_tx(&target, &signers);
    let producer_snapshot = node.producer_set.read().await.clone();

    // O5 — Add/Remove never yield a protocol activation.
    let activation = node
        .process_transaction_governance(&tx, 1, &producer_snapshot)
        .await;
    assert!(
        activation.is_none(),
        "O5: a RemoveMaintainer must not report a ProtocolActivation"
    );

    // PRECONDITION — the removal was authorized and applied. If this fails the
    // rest of the test proves nothing.
    {
        let ms = node.maintainer_state.as_ref().unwrap().read().await;
        assert_eq!(
            ms.set.member_count(),
            4,
            "precondition: the RemoveMaintainer must have been applied (5 -> 4)"
        );
        assert!(
            !ms.set.is_maintainer(&target),
            "precondition: the target must be gone from the root"
        );
        // O2 / O3 at the moment of the removal.
        assert_eq!(ms.set.threshold, 3, "O2: a 4-member set has threshold 3");
        assert_eq!(ms.last_derived_height, 1, "O3: stamped at the tx height");
    }

    // --- ONE ordinary block. This is the whole exploit. ---
    let block = build_block(1, 1, Hash::ZERO, &producers[0], &params);
    node.apply_block(block, ValidationMode::Light)
        .await
        .expect("O6: apply_block must succeed");

    // O8 — prove the block actually landed, so "root unchanged" cannot be the
    // trivial result of a block that never applied.
    assert_eq!(
        node.chain_state.read().await.best_height,
        1,
        "O8: the ordinary block must have been applied"
    );

    // O1 — THE ASSERTION THAT FAILS TODAY.
    let after = root_members(&node).await;
    assert_eq!(
        after.len(),
        4,
        "AUDIT-P1-013 / FM-01: applying ONE ordinary block re-derived the \
         maintainer root back to the genesis five. maybe_bootstrap_maintainer_set \
         (periodic.rs:44-49) is guarded by is_fully_bootstrapped() == len >= 5 and \
         runs on EVERY applied block (state_update.rs:214), then assigns \
         `state.set = set` WHOLESALE (periodic.rs:74). A governance removal is \
         therefore reverted in ~10 s on a live network, and the on-chain trust \
         root M1 made authoritative cannot be rotated durably."
    );
    assert!(
        !after.contains(&target),
        "AUDIT-P1-013 / FM-01: the removed maintainer is back in the root"
    );

    // O2 — the re-derivation restores the 5-member threshold too.
    {
        let ms = node.maintainer_state.as_ref().unwrap().read().await;
        assert_eq!(
            ms.set.threshold, 3,
            "O2: threshold after a durable removal (4 members)"
        );
    }

    // IP-2 / O4 — what a RESTARTED node reads off disk. The updater resolves its
    // install trust root from this file (`updater/trust_root_wiring.rs:95`), so a
    // reverted root on disk re-arms the removed key for binary installs.
    let persisted = MaintainerState::load(&data_dir).expect("O4: the state file must be readable");
    assert_eq!(
        persisted.set.member_count(),
        4,
        "O4 / FM-01: the PERSISTED maintainer_state.bin was rewritten with the \
         genesis five, so a restart — and every `doli upgrade` that resolves its \
         trust root from this file — re-arms the removed key"
    );
    assert!(
        !persisted.set.is_maintainer(&target),
        "O4 / FM-01: the removed key must not be on disk"
    );

    drop(temp);
}

/// IP-3 (control). O1, O6, O8 x the untouched path. Byte-identical to IP-1
/// except that no governance transaction is submitted. Proves the harness
/// really seeds a five-member root and really applies a block, so IP-1's
/// failure is the REVERT and not a broken setup.
#[tokio::test]
async fn control_untouched_root_survives_a_block_unchanged() {
    let producers: Vec<KeyPair> = (0..5).map(|_| KeyPair::generate()).collect();
    let (mut node, temp) = make_node(&producers).await;
    let params = node.params.clone();

    node.maybe_bootstrap_maintainer_set(0).await;
    let before = root_members(&node).await;
    assert_eq!(before.len(), 5, "CONTROL setup: seeded with five");

    let block = build_block(1, 1, Hash::ZERO, &producers[0], &params);
    node.apply_block(block, ValidationMode::Light)
        .await
        .expect("O6: apply_block must succeed");

    assert_eq!(
        node.chain_state.read().await.best_height,
        1,
        "O8: the control block must have been applied"
    );

    let after = root_members(&node).await;
    assert_eq!(
        after, before,
        "CONTROL: with no governance transaction the root must be unchanged \
         across a block"
    );

    drop(temp);
}

// ---------------------------------------------------------------------------
// IP-4 / IP-5 — AUDIT-P3-014, non-deterministic bootstrap derivation
// ---------------------------------------------------------------------------

/// Derive the bootstrap root on `DETERMINISM_SAMPLES` independently constructed
/// nodes that share the SAME producer keys, and return the member list of each.
async fn sample_bootstrap_roots(producers: &[KeyPair]) -> Vec<Vec<PublicKey>> {
    let mut samples = Vec::with_capacity(DETERMINISM_SAMPLES);
    for _ in 0..DETERMINISM_SAMPLES {
        let (node, temp) = make_node(producers).await;
        node.maybe_bootstrap_maintainer_set(0).await;
        samples.push(root_members(&node).await);
        drop(temp);
    }
    samples
}

/// IP-4. O1 x PD-stable. **AUDIT-P3-014 — MUST FAIL TODAY** on any chain with
/// more than five producers tied at `registered_at == 0` (bootstrap-mode chains
/// declare `genesis_producers: vec![]`, `crates/core/src/chainspec.rs:269, and
/// every genesis registration is stamped 0).
///
/// Asserts the STRONG property M2 owes: byte-identical ORDER, not merely equal
/// membership. Order is observable — the members vector is serialized verbatim
/// into `maintainer_state.bin` and returned by the `getMaintainerSet` RPC.
#[tokio::test]
async fn bootstrap_derivation_must_be_identical_across_nodes_with_tied_producers() {
    let producers: Vec<KeyPair> = (0..8).map(|_| KeyPair::generate()).collect();
    let samples = sample_bootstrap_roots(&producers).await;

    let first = &samples[0];
    assert_eq!(first.len(), 5, "setup: the root takes the first five");

    for (i, s) in samples.iter().enumerate().skip(1) {
        assert_eq!(
            s, first,
            "AUDIT-P3-014: node {i} derived a DIFFERENT bootstrap maintainer root \
             from byte-identical inputs. periodic.rs:53 reads \
             producers.all_producers() (a HashMap::values() walk) and \
             periodic.rs:59 applies a STABLE sort on registered_at only. Every \
             genesis producer is stamped registered_at == 0, so the entire set is \
             one tie group and the stable sort preserves the random HashMap \
             order. With more than five tied producers take(5) picks a random \
             5-subset, so MEMBERSHIP — not just order — diverges node by node. \
             The identical gap at governance.rs:116-122 is consensus-visible via \
             ProtocolActivation acceptance."
        );
    }
}

/// IP-5 (control). O1 x PD-stable with EXACTLY five producers. Byte-identical
/// to IP-4 except for the producer count.
///
/// AUDIT-P3-014 established that with exactly five genesis producers `take(5)`
/// consumes the whole tie group, so MEMBERSHIP cannot vary — which is why the
/// defect is dead on today's mainnet. This control asserts membership-as-a-set
/// only; it deliberately does NOT assert order, because order legitimately
/// varies today and asserting it here would make the control red for a reason
/// unrelated to the harness.
#[tokio::test]
async fn control_five_tied_producers_yield_stable_membership() {
    let producers: Vec<KeyPair> = (0..5).map(|_| KeyPair::generate()).collect();
    let samples = sample_bootstrap_roots(&producers).await;

    let mut expected: Vec<PublicKey> = producers.iter().map(|kp| *kp.public_key()).collect();
    expected.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

    for (i, s) in samples.iter().enumerate() {
        let mut got = s.clone();
        got.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        assert_eq!(
            got, expected,
            "CONTROL: with exactly five tied producers take(5) consumes the whole \
             tie group, so MEMBERSHIP is stable even today (node {i}). This proves \
             the determinism harness is not merely observing key-generation noise."
        );
    }
}
