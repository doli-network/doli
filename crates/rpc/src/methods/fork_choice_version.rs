//! INC-I-204 M5 / REQ-FORK-014 — `getForkChoiceVersion`, the fleet-readiness probe.
//!
//! Read-only and unauthenticated: it reports which fork-choice rule this node runs
//! at its current tip and nothing else. With mainnet and testnet frozen at
//! `u64::MAX` for the whole of M5, this endpoint plus the
//! `doli_fork_choice_post_activation_total` counters are the only evidence the
//! unified authority exists on a live network. The guardian polls it across the
//! fleet BEFORE any height is pinned.

use serde_json::Value;

use crate::error::RpcError;

use super::context::RpcContext;

/// Fork-choice rule version reported before the activation height is reached.
const FORK_CHOICE_VERSION_LEGACY: u64 = 1;
/// Fork-choice rule version reported at and above the activation height.
const FORK_CHOICE_VERSION_UNIFIED: u64 = 2;

impl RpcContext {
    /// Report the fork-choice rule in force at this node's current tip.
    pub(super) async fn get_fork_choice_version(&self) -> Result<Value, RpcError> {
        let local_height = self.chain_state.read().await.best_height;
        let activation_height = self.inc_i_204_fork_choice_activation_height;
        let active = local_height >= activation_height;

        Ok(serde_json::json!({
            "version": if active { FORK_CHOICE_VERSION_UNIFIED } else { FORK_CHOICE_VERSION_LEGACY },
            "activationHeight": activation_height,
            "active": active,
            "localHeight": local_height,
        }))
    }
}

#[cfg(test)]
mod tests {
    // OUTPUT CONTRACT — ENUMERATION-CHECKLIST.
    //   F1: RpcContext::get_fork_choice_version(&self) -> Result<Value, RpcError>
    //       O1: return — the JSON object. `&self`, no mutable params, no receiver
    //           mutation, no store writes; one channel, declared complete.
    //       Sub-observables: O1a version, O1b activationHeight, O1c active,
    //                        O1d localHeight.
    //       PATHS: P1 local_height <  AH -> version 1 / active false
    //              P2 local_height >= AH -> version 2 / active true
    //       INPUT PARTITIONS: AH = {u64::MAX (frozen live default), 0 (devnet),
    //         a mid-chain N} x local_height = {0, N-1, N}.
    //       MATRIX: 4 sub-observables x 2 paths; every cell below is named.
    use std::sync::Arc;

    use doli_core::Network;
    use mempool::{Mempool, MempoolPolicy};
    use storage::{BlockStore, ChainState, UtxoSet};
    use tempfile::TempDir;
    use tokio::sync::RwLock;

    use crate::methods::RpcContext;

    fn ctx_with(activation_height: u64, local_height: u64) -> (RpcContext, TempDir) {
        let dir = TempDir::new().expect("blockstore tempdir");
        let mut chain_state = ChainState::new(crypto::Hash::ZERO);
        chain_state.best_height = local_height;
        let params = doli_core::consensus::ConsensusParams::default();
        let mempool = Arc::new(RwLock::new(Mempool::new(
            MempoolPolicy::default(),
            params.clone(),
            Network::Mainnet,
        )));
        let mut ctx = RpcContext::new_for_network(
            Arc::new(RwLock::new(chain_state)),
            Arc::new(BlockStore::open(dir.path()).expect("blockstore")),
            Arc::new(RwLock::new(UtxoSet::new())),
            mempool,
            params,
            Network::Mainnet,
        );
        ctx.inc_i_204_fork_choice_activation_height = activation_height;
        (ctx, dir)
    }

    /// REQ-FORK-014 — Decision: a failure means the guardian cannot tell a node that
    /// still runs the legacy rule from one that has crossed the gate, so the
    /// fleet-readiness check the spec requires before pinning a height is impossible.
    #[tokio::test]
    async fn frozen_gate_reports_version_1_and_echoes_u64_max() {
        let (ctx, _d) = ctx_with(u64::MAX, 0);
        let r = ctx.get_fork_choice_version().await.expect("read-only");
        assert_eq!(r["version"], 1, "O1a: dormant window runs the legacy rule");
        assert_eq!(
            r["activationHeight"].as_u64(),
            Some(u64::MAX),
            "O1b: the echo is what proves the node's compiled gate, not the caller's guess"
        );
        assert_eq!(r["active"], false, "O1c");
        assert_eq!(r["localHeight"], 0, "O1d");
    }

    /// REQ-FORK-014 — Decision: a failure means a node that HAS crossed the gate
    /// still reports 1, so the probe would report a fleet as un-upgraded forever and
    /// no height could ever be declared safe to cross.
    #[tokio::test]
    async fn devnet_shaped_gate_reports_version_2_from_genesis() {
        let (ctx, _d) = ctx_with(0, 0);
        let r = ctx.get_fork_choice_version().await.expect("read-only");
        assert_eq!(r["version"], 2);
        assert_eq!(r["active"], true);
        assert_eq!(r["activationHeight"], 0);
    }

    /// REQ-FORK-014 / ANTI-VACUITY — Decision: a failure means the reported version
    /// does not actually track the local tip against the gate, so both tests above
    /// could pass on a constant. The transition is AT the height, matching the
    /// `>=` the fork-choice code itself uses.
    #[tokio::test]
    async fn the_transition_is_at_the_activation_height_not_after_it() {
        const GATE: u64 = 1_000;
        let (below, _d1) = ctx_with(GATE, GATE - 1);
        let (at, _d2) = ctx_with(GATE, GATE);

        let b = below.get_fork_choice_version().await.expect("read-only");
        let a = at.get_fork_choice_version().await.expect("read-only");

        assert_eq!(
            (b["version"].as_u64(), b["active"].as_bool()),
            (Some(1), Some(false))
        );
        assert_eq!(
            (a["version"].as_u64(), a["active"].as_bool()),
            (Some(2), Some(true))
        );
        assert_eq!(
            a["localHeight"].as_u64(),
            Some(GATE),
            "O1d tracks the real tip"
        );
    }
}
