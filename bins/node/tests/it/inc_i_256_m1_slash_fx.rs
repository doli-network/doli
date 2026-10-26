//! INC-I-256 M1 slash-delivery fixtures: one offence fed to the node's own
//! slash path, loss of the resident slash, and one periodic tick.
//!
// OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.
// covers: slash_submit slash_redeliver periodic mod block_handling network_events header_auth equivocation slash_admission pool transaction validation_checks init

#![allow(dead_code)]

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::transaction::Transaction;
use doli_node::node::Node;
use network::EquivocationProof;

use super::inc_i_255_m1_common::{
    canonical, chain_genesis, evidence_header, resident_slashes, SLOT,
};

/// Two signed, VDF-valid headers by one producer for `SLOT`, and the
/// canonical slash they make.
pub struct Offence {
    pub accused: PublicKey,
    pub proof: EquivocationProof,
    pub tx: Transaction,
}

impl Offence {
    pub fn by(kp: &KeyPair) -> Self {
        let g = chain_genesis();
        let a = evidence_header(kp, g, SLOT, 0xA1, 0);
        let b = evidence_header(kp, g, SLOT, 0xB2, 0);
        let accused = *kp.public_key();
        let tx = canonical(accused, a.clone(), b.clone());
        let proof = EquivocationProof {
            producer: accused,
            block_header_1: a,
            block_header_2: b,
            slot: SLOT,
        };
        Self { accused, proof, tx }
    }

    pub fn id(&self) -> Hash {
        self.tx.hash()
    }
}

/// The node's detector-to-slash path for `o`.
pub async fn offer(node: &mut Node, o: &Offence) {
    node.handle_equivocation(o.proof.clone()).await;
}

/// The slash leaves the pool (eviction, expiry, reorg or poison purge).
pub async fn lose(node: &Node, id: &Hash) {
    let gone = node.mempool.write().await.remove_transaction(id);
    assert!(
        gone.is_some(),
        "FIXTURE: the slash {id} was resident before the loss"
    );
}

pub async fn tick(node: &mut Node) {
    let r = node.run_periodic_tasks().await;
    assert!(r.is_ok(), "FIXTURE: periodic tick failed: {r:?}");
}

pub async fn resident(node: &Node, o: &Offence) -> Vec<Hash> {
    resident_slashes(node, &o.accused).await
}

pub fn metric_resident(ids: &[Hash], id: &Hash) {
    println!(
        "B2M1_METRIC SLASH_RESIDENT_AFTER_TICK={}",
        u8::from(ids.contains(id))
    );
}
