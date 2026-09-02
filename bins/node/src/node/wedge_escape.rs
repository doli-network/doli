use super::*;
use crate::metrics::record_wedge_escape_outcome;
use crate::node::wedge_outcome::{classify_wedge_plan, WedgeOutcome};

// =============================================================================
// INC-I-143 F2 — FORK_GUARD wedge-escape
// =============================================================================
//
// Root cause (block_handling.rs handle_new_block):
//   The HeightOccupied{is_better:false} arm dropped a competing sibling block
//   PERMANENTLY (no cache, no reorg record, no signal), and its descendants fell
//   into the generic Orphan path (ORPHAN_CHASE requests best_height+1 — the wrong
//   height for a locally-known sibling). Result: a node wedged on the worse-slot
//   canonical block could never reorg onto a strictly-longer sibling branch.
//
// Fix (SSF): retain the sibling and re-evaluate it through the EXISTING reorg
// machinery (record_fork_block_weight → plan_reorg → execute_reorg), the same
// flow handle_completed_fork_recovery uses. No new reorg path is invented and the
// finality guard (INV-SYNC-008, applied inside plan_reorg) is preserved.
//
// Scope: NODE-LOCAL fork choice only — which locally-available branch to follow
// after a drop. It does NOT change block content, validity rules, or what a
// producer emits. No activation height required (INC-I-075 Q1/Q2 = no).
//
// Security hardening (INC-I-143 M2 audit loop — F2-LOCAL fixes, all inside this
// entry point; the shared execute_reorg / plan_reorg / classify_gossip_block are
// NOT modified):
//   FIX A (AUDIT-P0-001/P1-001/P2-003 net-new slices): eligibility-gate every
//     block against the epoch-frozen schedule BEFORE it enters the reorg graph
//     (mirrors try_apply_cached_chain in fork_recovery.rs), and require a STRICT
//     weight increase (drop the equal-weight tie-break for unsolicited gossip
//     siblings) — closes the net-new tip-rewrite and hash-grind vectors.
//   FIX B (AUDIT-P2-001/P2-002): only a descendant whose parent is in the
//     eligibility-validated retained set (self.wedge_retained_tips) may reroute
//     here; the depth of that set is capped (WEDGE_RETAINED_CAP) to bound O(n^2)
//     amplification. Enforced in the Orphan arm of block_handling.rs.
//   FIX C (AUDIT-P2-002/P2-004): a cheap accumulated-weight short-circuit skips
//     the deep plan_reorg walk when the sibling cannot out-weight our tip, and a
//     would-be-winning branch that execute_reorg cannot land (evicted body /
//     no-op) raises signal_stuck_fork instead of being silently dropped.

/// Cap on the number of eligibility-validated retained fork-block hashes tracked
/// for the Orphan-descendant reroute. Bounds gossip-sourced descendant chain
/// depth well below `MAX_REORG_DEPTH` (1000) so a flood of fabricated descendants
/// cannot amplify plan_reorg cost into O(n^2). (AUDIT-P2-002.)
pub(crate) const WEDGE_RETAINED_CAP: usize = 64;

impl Node {
    /// Retain a competing sibling block the FORK_GUARD would otherwise drop, then
    /// attempt a synchronous reorg onto its branch via the existing reorg engine.
    ///
    /// Behavior:
    /// - Record the block's weight into the reorg handler (heights derived from the
    ///   tracked parent; does NOT touch current_chain_weight — it is a fork block).
    /// - Cache the block body so execute_reorg can fetch it.
    /// - plan_reorg from the current tip to this block; switch ONLY if the sibling
    ///   branch is STRICTLY heavier. The equal-weight tie-break is deliberately
    ///   NOT applied here (unsolicited single-gossip siblings) — it remains only on
    ///   the peer-downloaded fork_recovery path, closing the hash-grind tip-
    ///   displacement vector (AUDIT-P2-003).
    /// - Otherwise leave the block retained (cached + recorded) and raise a
    ///   stuck-fork signal so the periodic peer-download recovery path can complete
    ///   the branch in production.
    ///
    /// Every block is eligibility-gated (FIX A) BEFORE it can enter the reorg
    /// graph, and only eligibility-validated retained blocks (self.wedge_retained_tips,
    /// FIX B) may seed a descendant reroute — see the module header.
    pub(crate) async fn retain_sibling_and_try_escape(&mut self, block: Block) -> Result<()> {
        let fork_tip_hash = block.hash();

        // FIX A (AUDIT-P0-001/P1-001 net-new slice): eligibility-gate the block
        // against the epoch-frozen schedule BEFORE it can enter the reorg graph.
        // Mirrors try_apply_cached_chain (fork_recovery.rs:210). A block whose
        // producer is not the deterministically-scheduled slot leader is DROPPED
        // here — never recorded, cached, or reorged — so a spoofed-weight forged
        // sibling cannot displace the tip. This does NOT re-architect the shared
        // Light-mode execute_reorg (a pre-existing, separately-filed surface); it
        // stops F2 from newly widening that surface to the tip.
        if let Err(e) = self.check_producer_eligibility(&block).await {
            debug!(
                "[WEDGE_ESCAPE] Dropping ineligible fork block {:.8} at slot {}: {} (INC-I-143)",
                fork_tip_hash, block.header.slot, e
            );
            return Ok(());
        }

        // a. Producer weight (mirror the ReorgCandidate path in block_handling.rs).
        let height = self.chain_state.read().await.best_height + 1;
        let weight = {
            let ps = self.producer_set.read().await;
            ps.get_by_pubkey(&block.header.producer)
                .map(|p| p.effective_weight(height))
                .unwrap_or(1)
        };

        // b. Record the block into the reorg handler so plan_reorg can accumulate
        //    weight/height for the competing branch (fork block: no current-weight
        //    mutation).
        // INC-I-204 M5: `BlockHeader` carries no height, so the sibling's REAL chain
        // height is its parent's stored height plus one — chain-derived, never the
        // per-process counter (INV-SYNC-012).
        let sibling_height = self
            .block_store
            .get_height_by_hash(&block.header.prev_hash)
            .ok()
            .flatten()
            .map_or(height, |h| h.saturating_add(1));
        self.sync_manager.write().await.record_fork_block_weight(
            fork_tip_hash,
            block.header.prev_hash,
            weight,
            sibling_height,
        );

        // c. Cache the body (execute_reorg fetches new-block bodies from here).
        self.cache_block_with_eviction(fork_tip_hash, block.clone())
            .await;

        // FIX B (AUDIT-P2-001/P2-002): mark this hash as an eligibility-VALIDATED
        // retained fork block. ONLY descendants whose parent is in this set may be
        // rerouted from the Orphan arm into the reorg graph — an arbitrary
        // cache-only-parent orphan cannot seed a fabricated multi-block fork.
        // Capped to bound gossip-sourced descendant chain depth (O(n^2) guard).
        if self.wedge_retained_tips.len() >= WEDGE_RETAINED_CAP {
            self.wedge_retained_tips.clear();
        }
        self.wedge_retained_tips.insert(fork_tip_hash);

        // FIX C (AUDIT-P2-002): cheap accumulated-weight short-circuit BEFORE the
        // deep (up to 1000-ancestor) plan_reorg walk. If the sibling branch cannot
        // out-weight our current tip, a reorg is impossible under the strict-weight
        // rule below — skip the walk entirely and retain the block for later.
        // INC-I-204 M5: the verdict now comes from the ONE authority
        // (`ReorgHandler::weigh_branches`). Below the fork-choice activation height it
        // reproduces this door's pre-M5 rule exactly — `fork_w <= our_w` gives up.
        let current_tip = self.chain_state.read().await.best_hash;
        let (fork_weight, our_weight, verdict) = {
            let sync = self.sync_manager.read().await;
            let finality = sync.fork_choice_finality();
            let rh = sync.reorg_handler();
            (
                rh.chain_weight(&fork_tip_hash),
                rh.chain_weight(&current_tip),
                rh.weigh_branches(&current_tip, &fork_tip_hash, finality),
            )
        };
        if matches!(
            verdict,
            network::WeightVerdict::Lighter | network::WeightVerdict::TieKeep
        ) {
            record_wedge_escape_outcome(WedgeOutcome::CannotOutweigh.reason());
            info!(
                "[WEDGE_ESCAPE] Sibling {:.8} cannot out-weight tip (fork_w={} <= our_w={}) — retained, signaling recovery (INC-I-143)",
                fork_tip_hash, fork_weight, our_weight
            );
            self.sync_manager.write().await.signal_stuck_fork();
            return Ok(());
        }

        // d. Attempt a synchronous reorg using the SAME flow as
        //    handle_completed_fork_recovery (fork_recovery.rs). plan_reorg applies
        //    the finality guard internally — a finalized tip yields None / not-heavier
        //    and no reorg happens (correct; do not bypass).
        let reorg_result = {
            let sync = self.sync_manager.read().await;
            let store = &self.block_store;
            sync.reorg_handler().plan_reorg(
                current_tip,
                fork_tip_hash,
                |h| store.get_header(h).ok().flatten().map(|hd| hd.prev_hash),
                |h| store.get_height_by_hash(h).ok().flatten(),
                sync.fork_choice_finality(),
            )
        };

        // FIX A (AUDIT-P2-003): require a STRICT weight increase. The equal-weight
        // lower-hash tie-break stays in the PEER-DOWNLOADED fork_recovery.rs path;
        // it must NOT apply to unsolicited single-gossip siblings, or an attacker
        // can hash-grind an equal-weight competitor to displace the committed tip.
        // INC-I-204 M0: name the branch before taking it. The match below is the
        // live selection, unchanged; the classifier only labels it.
        record_wedge_escape_outcome(
            classify_wedge_plan(fork_weight, our_weight, reorg_result.as_ref()).reason(),
        );

        match reorg_result {
            Some(result) if result.weight_delta > 0 => {
                info!(
                    "[WEDGE_ESCAPE] Sibling branch heavier (delta={}, fork={:.8}, ours={:.8}) — reorging (INC-I-143)",
                    result.weight_delta, fork_tip_hash, current_tip
                );
                self.execute_reorg(result, block).await?;

                // FIX C (AUDIT-P2-004): execute_reorg no-ops (returns Ok) when an
                // intermediate body was evicted from the fork cache. A would-be-
                // winning branch must NOT be silently dropped — if the tip did not
                // actually move to the sibling, signal the periodic peer-download
                // recovery so it can complete the escape.
                if self.chain_state.read().await.best_hash != fork_tip_hash {
                    record_wedge_escape_outcome(WedgeOutcome::ReorgDidNotLand.reason());
                    warn!(
                        "[WEDGE_ESCAPE] Heavier sibling {:.8} did not land (missing body) — signaling fork recovery (INC-I-143)",
                        fork_tip_hash
                    );
                    self.sync_manager.write().await.signal_stuck_fork();
                }
            }
            Some(_) => {
                // Not strictly heavier: keep the block retained (already cached +
                // recorded) and signal so the periodic recovery path can pull the
                // rest of the branch from peers and complete the escape.
                info!(
                    "[WEDGE_ESCAPE] Sibling {:.8} retained (not strictly heavier) — signaling fork recovery (INC-I-143)",
                    fork_tip_hash
                );
                self.sync_manager.write().await.signal_stuck_fork();
            }
            None => {
                // A guard STOPPED the reorg — a different diagnosis from the arm
                // above, and the one INC-I-204 needed (INC-I-204 M0 D3).
                info!(
                    "[WEDGE_ESCAPE] Sibling {:.8} retained (plan_reorg refused: finality guard, unknown ancestor, or reorg depth) — signaling fork recovery (INC-I-143)",
                    fork_tip_hash
                );
                self.sync_manager.write().await.signal_stuck_fork();
            }
        }

        Ok(())
    }
}
