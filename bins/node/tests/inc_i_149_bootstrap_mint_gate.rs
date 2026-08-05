//! INC-I-149: A producer started with `--producer` on an EMPTY data directory,
//! joining a long-running chain, mints its OWN height-1 block instead of waiting
//! to sync. The self-produced block 1 then survives below the snap-sync horizon
//! as a permanent fossil orphan, so the node disagrees with the entire fleet on
//! block 1 forever while agreeing on every other height (gauntlet GS-001 fails).
//!
//! Root cause (controlled experiment, `docs/bugfixes/inc-i-149-bootstrap-mint-analysis.md`):
//! the production decision path treats LOCAL height as a proxy for NETWORK age.
//! At local height 1 the node classifies itself as "in genesis" and skips the
//! sync-before-produce guards, even though its peers report ~84,000 blocks ahead.
//!
//! CORRECTNESS PROPERTY UNDER TEST — a node must decide whether producing is safe
//! from evidence that SURVIVES A DISK WIPE, never from local height. Two inputs:
//!
//!   * `evidence` derived from peers — `Unknown` (no peer status received yet) /
//!     `AtGenesis` (peers connected, none reports any blocks) / `HasHistory`
//!     (some peer reports height > 0)
//!   * `has_bootstrap_nodes` — durable configuration; survives a wipe and states
//!     operator intent ("there is a network out there, go find it")
//!
//! TRUTH TABLE (local best_height = 0 in every row, so the decision height is 1 —
//! that is the wiped-disk shape; see `docs/bugfixes/inc-i-149-structural-design.md`):
//!
//! | row | has_bootstrap | evidence   | may produce? | why |
//! |-----|---------------|------------|--------------|-----|
//! | R1  | true          | HasHistory | NO           | the observed defect: chain has history, we are at height 1 |
//! | R2  | true          | Unknown    | NO           | absence of evidence is not evidence of genesis (pre-status window) |
//! | R3  | true          | AtGenesis  | YES          | genuine fresh-genesis FLEET (INC-I-115 shape) |
//! | R4  | false         | Unknown    | YES          | origin node starting a brand-new chain; nobody to wait for |
//! | R5  | false         | HasHistory | NO           | a wiped SEED must not mint either |
//!
//! Row R4 makes genesis liveness work by CONFIGURATION rather than by timeout:
//! if a fix breaks R4, a new network can never start.
//!
//! Written BEFORE the fix; they observe BEHAVIOUR (was a block minted at the
//! decision height?), not any particular guard, so they stay valid wherever the
//! fix lands.
//!
//! Requirement: REQ-PROD-001 (Must)  — no mint at height 1 when peers are known to be ahead
//! Requirement: REQ-PROD-002 (Must)  — real fresh genesis preserved, zero added delay
//! Requirement: REQ-PROD-003 (Must)  — no mint at height 1 in the pre-status window when
//!                                     bootstrap nodes are configured (the structural row)
//! Requirement: REQ-PROD-005 (Must)  — reproduction test exists and FAILS before the fix

// OUTPUT CONTRACT: fn Node::try_produce_block(&mut self) -> Result<()>
// O1: (mutable params) — none; the entry point is receiver-only (&mut self)
// O2: self.last_produced_slot — Option<u64>; Some(slot) iff a block was minted for that slot
// O2: self.chain_state.best_height — u64; incremented iff a block was minted AND applied
// O3: return — Result<()>; Ok(()) on EVERY deferral path AND on success (never Err here), so
//     it can NEVER distinguish mint from defer — asserted only to prove no path errors out
// O4: block_store[decision_height] — Option<Block>; Some iff a block was minted AND applied.
//     This is the FOSSIL: the artifact that outlives the process and breaks GS-001
// O5: (global/static) — LAST_WARNING / LAST_FORK_WARNING atomics; not part of the contract
// O6: (channel/event) — broadcast_header/broadcast_block; unobservable here (network=None) and
//     irrelevant: apply_block precedes broadcast, so the fossil exists even if it is suppressed
// PATHS:
//   PATH-A deferred — the production decision returns early; no mint.
//                     O2(last_produced_slot)=None, O2(best_height) unchanged,
//                     O3=Ok(()), O4=None
//   PATH-B minted   — production runs through apply_block; block on disk.
//                     O2(last_produced_slot)=Some(slot), O2(best_height)+1,
//                     O3=Ok(()), O4=Some(block)
// INPUT PARTITIONS: two independent inputs — (has_bootstrap_nodes, peer evidence) —
//   over a third contextual dimension (local best_height, which fixes the decision
//   height at local + 1). Evidence is classified exactly as the design specifies:
//   peer_count()==0 -> Unknown; else best_peer_height()>0 -> HasHistory; else AtGenesis.
//
//   TRUTH-TABLE ROWS — local=0, decision h=1 (the wiped-disk shape):
//   R1  bootstrap=true,  HasHistory (one peer at 84_505)  -> REQUIRED PATH-A
//       (the observed defect; closed by the behind-network guard at height 1)
//   R2  bootstrap=true,  Unknown (ZERO peers, no status)  -> REQUIRED PATH-A
//       (THE STRUCTURAL ROW, REQ-PROD-003 — pre-status window; RED before the fix)
//   R3  bootstrap=true,  AtGenesis (1 peer at height 0)   -> REQUIRED PATH-B
//       (genuine fresh-genesis FLEET, INC-I-115 shape; passes before AND after)
//   R4  bootstrap=false, Unknown (zero peers)             -> REQUIRED PATH-B
//       (origin node. MATCHED PAIR with R2 — same state, same budget, one input differs)
//   R5  bootstrap=false, HasHistory (one peer at 84_505)  -> REQUIRED PATH-A
//       (wiped SEED; the empty bootstrap list waives only the no-evidence case)
//   NO-REGRESSION PARTITIONS (pre-existing behind-network guard, decision h > 1):
//   P3  bootstrap=true, local=0 (decision h=1), peer at 0 + network tip 2 -> PATH-B
//       (blocks_behind=2 <= max_behind=3. NOTE — formally a HasHistory input that MUST
//        mint, so it bounds R1/R5 by MAGNITUDE, not by evidence class; see the SPEC
//        RECONCILIATION note on assert_materially_behind.)
//   P4  bootstrap=true, HasHistory, local=1 (decision h=2), tip 84_505 -> PATH-A
//       (local height 1 is reached via the R3 shape — bootstrap configured + one peer
//        at height 0 — because the R2 shape it previously used is now forbidden)
//   P4c bootstrap=true, HasHistory, local=1 (decision h=2), tip 1      -> PATH-B
//       (non-vacuity control for P4: byte-identical setup, only the tip differs)
//   OMITTED: (bootstrap=false, AtGenesis) is stated YES by the design and is R3 with
//   the bootstrap list emptied; subsumed by R3 x R4, which vary both inputs already.
// MATRIX: 4 asserted outputs {last_produced_slot, best_height, return, block_store}
//         x 8 partitions {R1,R2,R3,R4,R5,P3,P4,P4c} = 32 cells, every cell asserted
//         (PATH-A via assert_path_a, PATH-B via assert_path_b, O3 via the expect()
//          inside drive_production on every poll). O1/O5/O6 carry no observable
//          contract in this harness and are excluded above.
// ANTI-VACUITY PAIRING (each PATH-A row has a PATH-B control differing in ONE input):
//         R1 <-> R3   (bootstrap=true fixed; evidence HasHistory vs AtGenesis)
//         R2 <-> R4   (evidence=Unknown fixed; bootstrap true vs false)  <- strongest
//         R2 <-> R3   (bootstrap=true fixed; evidence Unknown vs AtGenesis)
//         R5 <-> R4   (bootstrap=false fixed; evidence HasHistory vs Unknown)
//         P4 <-> P4c  (decision height 2 fixed; network tip far-ahead vs 0)
//         Every PATH-A row also asserts can_produce()==Authorized first, so a negative
//         result cannot come from an unrelated upstream block.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::Network;
use doli_node::node::Node;
use network::{PeerId, ProductionAuthorization};
use tempfile::TempDir;

/// Network tip reported by the peers in the reproduction. Measured value from
/// the third reproduction (n13, 2026-08-04): `best_peer_h=84505` while the node
/// was still at `h=0`.
const NETWORK_TIP_FAR_AHEAD: u64 = 84_505;

/// Poll cadence for driving the production entry point. The real node drives it
/// from a 1 Hz timer (`event_loop.rs:14`); 100 ms only makes the test faster.
const POLL: Duration = Duration::from_millis(100);

/// Observation window for a "must NOT mint" partition. Strictly longer than one
/// full slot (10 s on testnet-shaped params) so the window covers EVERY possible
/// slot offset and at least one slot boundary. Without this, a negative result
/// could merely mean "we happened to poll outside the eligibility window".
const OBSERVE_NO_MINT: Duration = Duration::from_secs(12);

/// Deadline for a "must mint" partition. One slot plus margin.
const OBSERVE_MINT: Duration = Duration::from_secs(25);

/// Observation budget shared by the R2/R4 MATCHED PAIR.
///
/// R2 and R4 have byte-identical node state — local height 0, zero peers, network
/// tip 0, same params, same single producer — and differ in exactly ONE input:
/// `has_bootstrap_nodes`. Running the negative (R2) and the positive (R4) under
/// the SAME budget is the strongest anti-vacuity control available here: R4
/// minting inside this budget proves the budget sufficient, so R2 not minting
/// inside it cannot be a timing artefact.
const OBSERVE_MATCHED_PAIR: Duration = OBSERVE_MINT;

/// `max_behind` applied by the existing behind-network guard below height 10
/// (`production/mod.rs`). Separates "HasHistory and materially behind" (R1/R5,
/// must defer) from "HasHistory but within tolerance" (P3, must mint).
const GENESIS_MAX_BEHIND: u64 = 3;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Build a single-producer node whose production and validation params agree.
///
/// `Node::new_for_test` is hardwired to `Network::Devnet`, whose `slot_duration`
/// is 1 s — shorter than the hardcoded propagation delay at `production/mod.rs`,
/// so a Devnet test node can never reach `apply_block`. Validation rebuilds params
/// from `ConsensusParams::for_network(config.network)` (`validation_checks.rs`),
/// NOT from `self.params`, so both must be switched together or the block is
/// rejected `InvalidSlot`. Testnet shape (10 s slots) makes the real production
/// entry point drivable end to end.
///
/// One producer means the node is always rank 0 and `is_producer_eligible_ms`
/// short-circuits to `true` (`consensus/selection.rs`), leaving the propagation
/// floor as the only timing constraint — ~95 % of every slot is a mint window.
async fn make_producer_node(with_bootstrap_nodes: bool) -> (Node, KeyPair, TempDir) {
    let temp = TempDir::new().unwrap();
    let producer = KeyPair::generate();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), vec![producer.clone()])
        .await
        .expect("Node::new_for_test failed");

    node.config.network = Network::Testnet;
    node.params = ConsensusParams::testnet();

    if with_bootstrap_nodes {
        // Faithful to the reproduction: n12/n13 are joining nodes with the seed
        // configured as a bootstrap peer.
        node.config.bootstrap_nodes = vec!["/ip4/127.0.0.1/tcp/30300".to_string()];
    }

    (node, producer, temp)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn current_slot(node: &Node) -> u32 {
    node.params.timestamp_to_slot(now_secs())
}

/// Register a peer that reports `height` — exactly what `on_peer_status` does on
/// receipt of a `StatusResponse` (`network_events.rs:226`).
async fn add_peer_at(node: &Node, height: u64, slot: u32) {
    set_peer_status(node, PeerId::random(), height, slot).await;
}

/// Register — or UPDATE — a peer under a caller-chosen identity.
///
/// `add_peer` inserts by `PeerId`, so calling it twice with the SAME id models a
/// second `StatusResponse` from that peer rather than a second peer joining.
/// P4/P4c need this: their peer reports 0 during setup and 1 afterwards, and
/// `add_peer_at`'s random id would leave a phantom peer pinned at 0 forever.
async fn set_peer_status(node: &Node, peer: PeerId, height: u64, slot: u32) {
    let mut sm = node.sync_manager.write().await;
    sm.add_peer(peer, height, Hash::ZERO, slot);
}

/// Raise the network tip without registering a peer. Used where registering a peer
/// would additionally trip the sync state machine and block production for a
/// DIFFERENT reason (see `assert_gate_authorizes`), making the partition vacuous.
async fn set_network_tip(node: &Node, height: u64) {
    let mut sm = node.sync_manager.write().await;
    sm.update_network_tip_height(height);
}

/// NON-VACUITY PRECONDITION for every "must NOT mint" partition. Proves the
/// sync/peer production gate is NOT the thing refusing to mint, so a negative
/// result can only come from the decision under test. Without it, "no block was
/// produced" would pass the moment anything upstream started blocking.
async fn assert_gate_authorizes(node: &Node, partition: &str) {
    let slot = current_slot(node);
    let auth = node.sync_manager.write().await.can_produce(slot);
    assert_eq!(
        auth,
        ProductionAuthorization::Authorized,
        "{partition}: NON-VACUITY PRECONDITION FAILED — SyncManager::can_produce returned {auth:?}. \
         The production gate itself is refusing, so this partition cannot prove anything about \
         the height-vs-peer-height decision. Fix the harness, do not weaken the assertion."
    );
}

/// Drive the real production entry point until the chain advances or `budget`
/// elapses. Returns `true` if a block was minted and applied.
async fn drive_production(node: &mut Node, budget: Duration) -> bool {
    let start_height = node.chain_state.read().await.best_height;
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        node.try_produce_block()
            .await
            .expect("O3: try_produce_block must return Ok(()) on every path");
        if node.chain_state.read().await.best_height > start_height {
            return true;
        }
        tokio::time::sleep(POLL).await;
    }
    false
}

/// Assert PATH-A (deferred): all four observable outputs.
async fn assert_path_a(node: &Node, decision_height: u64, local_height: u64, partition: &str) {
    assert_eq!(
        node.last_produced_slot, None,
        "{partition}: O2 last_produced_slot must stay None on PATH-A"
    );
    assert_eq!(
        node.chain_state.read().await.best_height,
        local_height,
        "{partition}: O2 best_height must stay at {local_height} on PATH-A"
    );
    assert!(
        node.block_store
            .get_block_by_height(decision_height)
            .expect("block_store read")
            .is_none(),
        "{partition}: O4 block_store must hold NO block at height {decision_height} on PATH-A — \
         a block here is the fossil orphan that breaks GS-001"
    );
}

/// Assert PATH-B (minted): all four observable outputs.
async fn assert_path_b(node: &Node, decision_height: u64, partition: &str) {
    assert!(
        node.last_produced_slot.is_some(),
        "{partition}: O2 last_produced_slot must be Some(slot) on PATH-B"
    );
    assert_eq!(
        node.chain_state.read().await.best_height,
        decision_height,
        "{partition}: O2 best_height must advance to {decision_height} on PATH-B"
    );
    assert!(
        node.block_store
            .get_block_by_height(decision_height)
            .expect("block_store read")
            .is_some(),
        "{partition}: O4 block_store must hold the block at height {decision_height} on PATH-B"
    );
}

// ---------------------------------------------------------------------------
// Truth-table inputs
// ---------------------------------------------------------------------------

/// What the node actually KNOWS about the network's age, derived ONLY from
/// evidence that survives a data-directory wipe.
///
/// Mirrors `NetworkEvidence` in `docs/bugfixes/inc-i-149-structural-design.md`, but
/// computed from the seams that existed BEFORE the fix (`peer_count()` /
/// `best_peer_height()`), so these tests were runnable — and RED — before the
/// implementation existed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Evidence {
    /// No peer status has arrived. Absence of evidence is NOT evidence of genesis.
    Unknown,
    /// Peers connected, NONE reports any blocks — genuine fresh genesis.
    AtGenesis,
    /// Some peer reports height > 0 — an empty local disk means WE are behind.
    HasHistory,
}

/// `peer_count()` counts peers whose STATUS arrived (`peers.rs`; `add_peer` is the
/// only inserter), so `Unknown` genuinely means "nobody has told us anything yet".
async fn evidence_of(node: &Node) -> Evidence {
    let sm = node.sync_manager.read().await;
    if sm.peer_count() == 0 {
        Evidence::Unknown
    } else if sm.best_peer_height() > 0 {
        Evidence::HasHistory
    } else {
        Evidence::AtGenesis
    }
}

/// Assert BOTH truth-table inputs for a row plus the local height. Not ceremony:
/// R2 and R4 differ ONLY in `has_bootstrap_nodes`, so if that input silently failed
/// to be set the two rows would collapse into one and prove nothing.
async fn assert_row_inputs(
    node: &Node,
    row: &str,
    expect_bootstrap: bool,
    expect_evidence: Evidence,
    expect_local_height: u64,
) {
    let has_bootstrap = !node.config.bootstrap_nodes.is_empty();
    assert_eq!(
        has_bootstrap, expect_bootstrap,
        "{row}: INPUT 1 (has_bootstrap_nodes) must be {expect_bootstrap} — this is the durable \
         configuration input that survives a disk wipe; if it is wrong the row is not the row"
    );
    let evidence = evidence_of(node).await;
    assert_eq!(
        evidence, expect_evidence,
        "{row}: INPUT 2 (peer evidence) must be {expect_evidence:?}"
    );
    assert_eq!(
        node.chain_state.read().await.best_height,
        expect_local_height,
        "{row}: local best_height must be {expect_local_height} (decision height {})",
        expect_local_height + 1
    );
}

/// Assert the row is MATERIALLY behind — beyond the genesis `max_behind` tolerance.
///
/// SPEC RECONCILIATION (flagged, not silently resolved): the design's truth table
/// states `HasHistory => MUST NOT produce` for any `has_bootstrap_nodes`, but P3
/// (peer evidence present, network tip 2, decision height 1) is a `HasHistory`
/// case that MUST mint. Both cannot hold literally. The reading that keeps every
/// row satisfiable is by MAGNITUDE: a gap beyond `max_behind` defers (R1/R5, gap
/// 84_505), a gap inside it still participates (P3, gap 2). This pins that
/// reading so the two classes can never be conflated by accident.
async fn assert_materially_behind(node: &Node, row: &str) {
    let tip = node.sync_manager.read().await.best_peer_height();
    let local = node.chain_state.read().await.best_height;
    let blocks_behind = tip.saturating_sub(local);
    assert!(
        blocks_behind > GENESIS_MAX_BEHIND,
        "{row}: this row must be MATERIALLY behind (blocks_behind {blocks_behind} must exceed \
         max_behind {GENESIS_MAX_BEHIND}); otherwise it is a P3-shaped catch-up case that is \
         REQUIRED to mint, and the row would be asserting the opposite of the contract"
    );
}

// ===== R2 — THE STRUCTURAL ROW =====

// Requirement: REQ-PROD-003 (Must)
// Acceptance: Given has_bootstrap_nodes == true and NO peer status has been
// received (peer_count == 0), a node at decision height 1 does NOT produce.

/// R2: bootstrap nodes configured, ZERO peers — evidence is `Unknown`.
/// The node MUST NOT mint block 1.
///
/// THE STRUCTURAL ROW — the pre-status window no peer-height predicate can close:
/// with no peer registered `best_peer_height()` is 0, so `network_height_ahead` is
/// false and the behind-network guard is INERT, exactly as inert as at a genuine
/// fresh genesis. Local state is byte-identical in both situations; only the
/// durable config ("there is a network out there, go find it") separates them.
///
/// Operationally this is the whole incident: a wiped producer starts, its timer
/// fires at 1 Hz from the first second, and libp2p dial + identify + status take
/// hundreds of ms to seconds. Every tick inside that window can mint the fossil.
///
/// Written RED: it failed before the fix because `production/scheduling.rs` gated
/// the entire joining-node block — including the `peer_count == 0` wait — behind
/// `has_bootstrap_nodes && !in_genesis`, and `in_genesis` is true at height 1, so
/// a node with no peers had NO production guard at all.
#[tokio::test]
async fn r2_bootstrap_configured_zero_peers_unknown_evidence_must_not_mint_block_1() {
    let (mut node, _kp, _tmp) = make_producer_node(true).await;

    assert_row_inputs(&node, "R2", true, Evidence::Unknown, 0).await;
    assert_eq!(
        node.sync_manager.read().await.best_peer_height(),
        0,
        "R2: with no peer status received, best_peer_height() is 0 — INDISTINGUISHABLE from a \
         real fresh genesis, which is precisely why no height-derived or peer-height-derived \
         predicate can decide this row."
    );
    assert_gate_authorizes(&node, "R2").await;

    let minted = drive_production(&mut node, OBSERVE_MATCHED_PAIR).await;

    assert!(
        !minted,
        "R2 (STRUCTURAL): the node minted block 1 with bootstrap nodes configured and NOT ONE \
         peer status received. Absence of evidence is not evidence of genesis: an operator who \
         configured bootstrap nodes has stated there is a network to join, so the node must \
         wait for peer evidence first. R4 is the matched control — same state, same budget, no \
         bootstrap nodes — and it MUST mint; if R4 passes and this fails, the harness is sound \
         and the gate is missing."
    );
    assert_path_a(&node, 1, 0, "R2").await;
}

// ===== R5 — WIPED SEED =====

// Requirement: REQ-PROD-001 (Must)
// Acceptance: Given has_bootstrap_nodes == false and a peer reporting 84_505,
// a node at decision height 1 does NOT produce.

/// R5: no bootstrap nodes configured (a SEED), rejoining after a wipe with a peer
/// reporting height 84_505 — evidence is `HasHistory`. The node MUST NOT mint.
///
/// A wiped seed is not exempt: once ANY peer reports history, the origin-node
/// waiver on the `has_bootstrap_nodes == false` row is spent, because the question
/// "am I starting a chain or rejoining one?" is now answered.
///
/// Paired control: R4 — same `has_bootstrap_nodes == false`, evidence `Unknown` —
/// MUST mint. Together they prove the empty bootstrap list is neither a blanket
/// permission nor a blanket prohibition.
#[tokio::test]
async fn r5_no_bootstrap_peers_have_history_must_not_mint_block_1() {
    let (mut node, _kp, _tmp) = make_producer_node(false).await;

    let slot = current_slot(&node);
    add_peer_at(&node, NETWORK_TIP_FAR_AHEAD, slot).await;

    assert_row_inputs(&node, "R5", false, Evidence::HasHistory, 0).await;
    assert_materially_behind(&node, "R5").await;
    assert_gate_authorizes(&node, "R5").await;

    let minted = drive_production(&mut node, OBSERVE_NO_MINT).await;

    assert!(
        !minted,
        "R5: a wiped SEED (no bootstrap nodes configured) minted block 1 while a peer reported \
         height {NETWORK_TIP_FAR_AHEAD}. An empty bootstrap list waives the NO-EVIDENCE case \
         only (row R4); once a peer reports history the node knows it is rejoining, not \
         originating, and must sync first."
    );
    assert_path_a(&node, 1, 0, "R5").await;
}

// ---------------------------------------------------------------------------
// R1 (formerly P1) — THE OBSERVED DEFECT
// ---------------------------------------------------------------------------

// Requirement: REQ-PROD-001 (Must)
// Acceptance: Given best_height == 0 and best_peer_height() == 84_505, when
// try_produce_block() runs, it returns without calling apply_block.

/// R1: empty data dir (local height 0), bootstrap nodes configured, peers report
/// height 84_505 — evidence is `HasHistory`. The node MUST NOT mint block 1.
///
/// Measured conditions (all three reproductions, `~/testnet/logs/`): local
/// best_height 0, peer_count 1, best_peer_height already at the network tip
/// (18_990 / 83_519 / 84_507), mint ~30 s after start.
///
/// Paired control: R3 — same `has_bootstrap_nodes == true`, evidence `AtGenesis`
/// instead of `HasHistory` — MUST mint, proving the deferral here is caused by
/// the evidence, not by having bootstrap nodes configured.
#[tokio::test]
async fn r1_p1_bootstrap_configured_peers_have_history_must_not_mint_block_1() {
    let (mut node, _kp, _tmp) = make_producer_node(true).await;

    let slot = current_slot(&node);
    add_peer_at(&node, NETWORK_TIP_FAR_AHEAD, slot).await;

    // Preconditions: exactly the measured shape of the incident.
    assert_row_inputs(&node, "R1", true, Evidence::HasHistory, 0).await;
    {
        let sm = node.sync_manager.read().await;
        assert_eq!(sm.peer_count(), 1, "R1: peer_count must be 1 (as measured)");
        assert_eq!(
            sm.best_peer_height(),
            NETWORK_TIP_FAR_AHEAD,
            "R1: peers must report the network tip"
        );
    }
    assert_materially_behind(&node, "R1").await;
    assert_gate_authorizes(&node, "R1").await;

    let minted = drive_production(&mut node, OBSERVE_NO_MINT).await;

    assert!(
        !minted,
        "R1 (INC-I-149 OBSERVED DEFECT): the node minted its own block 1 while its peers \
         reported height {NETWORK_TIP_FAR_AHEAD}. Production at the first block must be \
         conditioned on PEER-REPORTED network height, not on local height alone — this block \
         can never be canonical and survives as a permanent fossil orphan below the snap-sync \
         horizon, so the node disagrees with the whole fleet on block 1 forever."
    );
    assert_path_a(&node, 1, 0, "R1").await;
}

// ---------------------------------------------------------------------------
// R4 (formerly P2) — ORIGIN NODE / GENESIS LIVENESS BY CONFIGURATION
// ---------------------------------------------------------------------------

// Requirement: REQ-PROD-002 (Must)
// Acceptance: Given has_bootstrap_nodes == false and no peer status received,
// a node at decision height 1 is NOT deferred — with zero added delay.

/// R4: origin/seed node of a brand-new chain — no bootstrap nodes configured and
/// no peers at all, so evidence is `Unknown`. The node MUST still mint block 1.
///
/// THE ROW THAT MAKES GENESIS WORK BY CONFIGURATION RATHER THAN BY TIMEOUT. An
/// origin node is definitionally the chain's starting point: nobody to wait for,
/// and an empty bootstrap list is the operator saying exactly that. If a fix
/// breaks this row a new network can never start — no amount of waiting produces
/// a peer that does not exist. This is the case the 2026-02-12 commit protects.
///
/// MATCHED PAIR with R2: identical local state (height 0, zero peers, tip 0) and
/// identical budget (`OBSERVE_MATCHED_PAIR`), differing ONLY in
/// `has_bootstrap_nodes`. This row minting is what makes R2 non-vacuous.
#[tokio::test]
async fn r4_p2_no_bootstrap_zero_peers_unknown_evidence_must_mint_block_1() {
    let (mut node, _kp, _tmp) = make_producer_node(false).await;

    assert_row_inputs(&node, "R4", false, Evidence::Unknown, 0).await;
    {
        let sm = node.sync_manager.read().await;
        assert_eq!(sm.peer_count(), 0, "R4: genuinely solo bootstrap");
        assert_eq!(
            sm.best_peer_height(),
            0,
            "R4: nobody reports any height — this is what real genesis looks like"
        );
    }
    // Same non-vacuity precondition as R2 — the pair differs in ONE input only.
    assert_gate_authorizes(&node, "R4").await;

    let minted = drive_production(&mut node, OBSERVE_MATCHED_PAIR).await;

    assert!(
        minted,
        "R4 (GENESIS PRESERVATION / MATCHED CONTROL FOR R2): a node bootstrapping a genuinely \
         fresh chain, with no bootstrap nodes configured and no peer reporting any height, MUST \
         still produce block 1 — blocking this breaks new-chain bootstrap entirely. It is also \
         R2's control: if this row stops minting, R2 proves nothing."
    );
    assert_path_b(&node, 1, "R4").await;
}

// ===== R3 (formerly P2b) — FRESH-GENESIS FLEET =====

// Requirement: REQ-PROD-002 (Must)
// Acceptance: fresh-genesis fleet — peers connected, all reporting height 0 — still produces.

/// R3: real fresh-genesis FLEET shape (the INC-I-115 configuration): bootstrap
/// nodes configured, a peer connected, and that peer reports height 0 because
/// nobody has any blocks yet — evidence is `AtGenesis`. The node MUST still mint.
///
/// Paired control for R1 (same `has_bootstrap_nodes == true`, evidence differs)
/// AND the discriminator for R2, which also has `has_bootstrap_nodes == true` and
/// differs only in `Unknown` vs `AtGenesis`. That split — "no peer has spoken" vs
/// "peers have spoken and all say zero" — is the entire content of the missing
/// concept. A fix that cannot tell them apart fails one of these two rows.
#[tokio::test]
async fn r3_p2b_bootstrap_configured_peers_at_genesis_must_mint_block_1() {
    let (mut node, _kp, _tmp) = make_producer_node(true).await;

    let slot = current_slot(&node);
    add_peer_at(&node, 0, slot).await;

    assert_row_inputs(&node, "R3", true, Evidence::AtGenesis, 0).await;
    {
        let sm = node.sync_manager.read().await;
        assert_eq!(sm.peer_count(), 1, "R3: connected to the fleet");
        assert_eq!(
            sm.best_peer_height(),
            0,
            "R3: the fleet is at genesis — every peer reports height 0"
        );
    }

    let minted = drive_production(&mut node, OBSERVE_MINT).await;

    assert!(
        minted,
        "R3 (GENESIS PRESERVATION): at a real fresh genesis every peer reports height 0. \
         Being connected to the fleet must not by itself defer block 1, or no new chain can \
         ever start (INC-I-115)."
    );
    assert_path_b(&node, 1, "R3").await;
}

// ---------------------------------------------------------------------------
// P3 — GENESIS CATCH-UP PRESERVATION
// ---------------------------------------------------------------------------

// Requirement: REQ-PROD-002 (Must)
// Acceptance: network_tip_height == 2 while we are at best_height == 0 —
// blocks_behind = 2, which is within the existing max_behind = 3 for height < 10,
// so production must NOT be deferred.

/// P3: fresh network where a couple of blocks already exist and we are only
/// slightly behind. A node 1-3 blocks behind at genesis must still participate.
///
/// A peer IS registered (so this partition never doubles as a "no peer evidence"
/// case, which REQ-PROD-003 reserves), but at height 0, with the height-2 tip
/// supplied through `update_network_tip_height`. Registering the peer directly at
/// height 2 would trip the sync state machine (`should_sync` fires at `min_gap = 1`
/// when local height is 0), blocking production through `can_produce` and making
/// the partition vacuous. `best_peer_height()` returns the max of the two sources,
/// so the decision under test sees exactly 2.
#[tokio::test]
async fn p3_fresh_network_two_blocks_ahead_must_still_mint_block_1() {
    let (mut node, _kp, _tmp) = make_producer_node(true).await;

    let slot = current_slot(&node);
    add_peer_at(&node, 0, slot).await;
    set_network_tip(&node, 2).await;

    assert_eq!(
        node.sync_manager.read().await.peer_count(),
        1,
        "P3: peer evidence exists — this is not a 'no peers yet' case"
    );
    assert_eq!(
        node.sync_manager.read().await.best_peer_height(),
        2,
        "P3: the network has 2 blocks; we are 2 behind, within the existing tolerance"
    );

    let minted = drive_production(&mut node, OBSERVE_MINT).await;

    assert!(
        minted,
        "P3 (GENESIS CATCH-UP): a node 1-3 blocks behind at genesis must still participate. \
         blocks_behind = 2 is within the existing max_behind = 3 for height < 10, so this \
         must not be deferred."
    );
    assert_path_b(&node, 1, "P3").await;
}

// ---------------------------------------------------------------------------
// P4 — NO-REGRESSION (already works today; must keep working)
// ---------------------------------------------------------------------------

// Requirement: REQ-PROD-001 (Must)
// Acceptance: the existing behind-network deferral for decision height > 1 is unchanged.

/// P4: local chain already advanced (best_height 1, so the decision height is 2)
/// and the network is 84_505 blocks ahead. The node MUST defer.
///
/// Pre-existing behaviour — it guards against a fix that repairs height 1 by
/// breaking the height > 1 path. Local height is established by actually minting
/// block 1 (network tip 0), not by fabricating a chain state: a fabricated tip
/// would make the partition vacuous, because block assembly would then fail for
/// unrelated reasons. `p4c` is the paired control proving height 2 is reachable.
///
/// SETUP SHAPE (changed when the INC-I-149 gate landed): block 1 is minted as row
/// R3 — bootstrap configured AND one peer at height 0 (`AtGenesis`). It previously
/// minted with ZERO peers, which is row R2 and is now correctly forbidden: the old
/// scaffolding depended on the exact permissiveness the fix removes. R3 is chosen
/// over R4 (drop the bootstrap nodes) deliberately — P4 guards a JOINING producer,
/// the incident's own configuration, and the R4 shape would make it a seed that
/// bypasses the new evidence gate entirely, silently losing coverage. The setup is
/// not an assumption: it is the configuration row `r3_p2b_...` asserts mints.
///
/// CORRECTION to the previous rationale: the post-setup peer was documented as
/// feeding the echo-chamber gate (`genesis_bypass`, `production_gate.rs`).
/// Inaccurate — `new_for_test` calls `set_min_peers_for_production(0)`, so
/// `peers.len() < min_peers` can never fire here at any peer count. The peer IS
/// load-bearing, for a different reason: at decision height 2 the node is still
/// `in_genesis` (testnet `genesis_blocks = 36`) and still on the bootstrap path,
/// so the evidence gate applies — with no peer the evidence would be `Unknown`
/// and P4 would defer for the WRONG reason. Its height advances 0 -> 1 via
/// `set_peer_status` (same PeerId = a second StatusResponse), staying level with
/// us so the sync state machine is idle. The far-ahead tip comes from
/// `update_network_tip_height`: a peer registered 84_505 ahead would start
/// header-first sync and block production via `can_produce`, making P4 vacuous.
#[tokio::test]
async fn p4_local_height_above_one_with_network_far_ahead_must_defer() {
    let (mut node, _kp, _tmp) = make_producer_node(true).await;

    // Establish local height 1 the honest way, via the legal R3 (AtGenesis) shape.
    let peer = PeerId::random();
    let slot = current_slot(&node);
    set_peer_status(&node, peer, 0, slot).await;
    assert_row_inputs(&node, "P4 setup", true, Evidence::AtGenesis, 0).await;

    let bootstrapped = drive_production(&mut node, OBSERVE_MINT).await;
    assert!(
        bootstrapped,
        "P4 setup: block 1 must mint via the R3 shape (bootstrap configured, one peer at \
         height 0, network tip 0). If this fails the setup path itself is illegal and P4 \
         must be re-scaffolded, NOT weakened — row r3 asserts this configuration mints."
    );
    assert_eq!(node.chain_state.read().await.best_height, 1);

    // Now the network is revealed to be far ahead. Decision height is 2.
    let slot = current_slot(&node);
    set_peer_status(&node, peer, 1, slot).await;
    set_network_tip(&node, NETWORK_TIP_FAR_AHEAD).await;
    assert_eq!(
        node.sync_manager.read().await.best_peer_height(),
        NETWORK_TIP_FAR_AHEAD
    );
    assert_gate_authorizes(&node, "P4").await;

    let start_slot = node.last_produced_slot;
    let minted = drive_production(&mut node, OBSERVE_NO_MINT).await;

    assert!(
        !minted,
        "P4 (NO-REGRESSION): at decision height 2 with the network {NETWORK_TIP_FAR_AHEAD} \
         blocks ahead, production is already deferred today. A fix for height 1 must not \
         break this."
    );
    assert_eq!(
        node.chain_state.read().await.best_height,
        1,
        "P4: O2 best_height must stay at 1"
    );
    assert_eq!(
        node.last_produced_slot, start_slot,
        "P4: O2 last_produced_slot must not advance past the block-1 slot"
    );
    assert!(
        node.block_store
            .get_block_by_height(2)
            .expect("block_store read")
            .is_none(),
        "P4: O4 block_store must hold no block at height 2"
    );
}

// Requirement: REQ-PROD-001 (Must)
// Acceptance: non-vacuity control — decision height 2 IS reachable in this harness.

/// P4c: paired control for P4. BYTE-IDENTICAL setup — same bootstrap config, same
/// single peer, same R3-shaped block-1 mint, same peer status advance 0 -> 1 — with
/// exactly ONE difference: `update_network_tip_height` is never called, so the
/// network stays level with us instead of 84_505 ahead, and the node must go on to
/// mint block 2. Without this, P4's "no block at height 2" could merely mean "this
/// harness cannot mint at height 2 at all". Both rows moved from the (now illegal)
/// R2 setup shape to the R3 shape TOGETHER, which is what preserves the pairing:
/// the only variable between them is still the network tip.
#[tokio::test]
async fn p4c_control_local_height_above_one_with_quiet_network_does_mint() {
    let (mut node, _kp, _tmp) = make_producer_node(true).await;

    let peer = PeerId::random();
    let slot = current_slot(&node);
    set_peer_status(&node, peer, 0, slot).await;
    assert_row_inputs(&node, "P4c setup", true, Evidence::AtGenesis, 0).await;

    let bootstrapped = drive_production(&mut node, OBSERVE_MINT).await;
    assert!(
        bootstrapped,
        "P4c setup: block 1 must be minted via the R3 shape (bootstrap configured, one peer \
         at height 0)"
    );
    assert_eq!(node.chain_state.read().await.best_height, 1);

    // Same peer status advance as P4 — but the network is quiet: nobody is ahead of us.
    let slot = current_slot(&node);
    set_peer_status(&node, peer, 1, slot).await;

    assert_eq!(
        node.sync_manager.read().await.best_peer_height(),
        1,
        "P4c: the network is level with us — nothing should defer production"
    );

    let minted = drive_production(&mut node, OBSERVE_MINT).await;

    assert!(
        minted,
        "P4c (CONTROL): with a quiet network the node must mint block 2. If this fails, P4 \
         proves nothing — its negative result would just mean height 2 is unreachable here."
    );
    assert_path_b(&node, 2, "P4c").await;
}
