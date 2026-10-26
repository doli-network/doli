//! B3 M1 maintainer fixtures: twin rigs with one seeded maintainer root, a
//! signed AddMaintainer / RemoveMaintainer, a block that fails in the tx loop
//! after the maintainer tx, and the maintainer observables of one node.
//!
// OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.

#![allow(dead_code)]

use std::sync::Arc;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::maintainer::{
    maintainer_set_digest, signing_message_at, MaintainerChangeData, MaintainerSignature,
    MAINTAINER_AUTH_VALID_BEFORE_UNSET,
};
use doli_core::transaction::{Transaction, TxType};
use doli_core::{Block, MaintainerSet};
use doli_node::node::Node;
use network::protocols::{SyncRequest, SyncResponse};
use network::PeerId;
use storage::MaintainerState;
use tokio::sync::RwLock;

use super::inc_i_255_m2_common::Rig;
use super::inc_i_256_m1_common::{block_at, ghost_spend, twins};

/// Seated members: 4, so one Add (to 5) and one Remove (to 3) both fit.
pub const MEMBERS: usize = 4;
const FILE: &str = "maintainer_state.bin";

/// Twin rigs at `height`, both holding the same seeded maintainer root.
pub async fn maintained_twins(height: u64) -> (Rig, Rig, Vec<KeyPair>) {
    let (mut s, mut c) = twins(height).await;
    let members: Vec<KeyPair> = (0..MEMBERS).map(|_| KeyPair::generate()).collect();
    attach(&mut s, &members);
    attach(&mut c, &members);
    assert_eq!(
        mobs(&s.node).await,
        mobs(&c.node).await,
        "PRECONDITION: twins hold the same maintainer root"
    );
    (s, c, members)
}

/// Seat `members` at height 0 in memory and in `maintainer_state.bin`.
pub fn attach(r: &mut Rig, members: &[KeyPair]) {
    let pks = members.iter().map(|k| *k.public_key()).collect();
    let state = MaintainerState {
        set: MaintainerSet::with_members(pks, 0),
        ..MaintainerState::default()
    };
    state
        .save(&r.node.config.data_dir)
        .expect("FIXTURE: persist the maintainer root");
    r.node.set_maintainer_state(Arc::new(RwLock::new(state)));
}

/// Governance tx signed by `signers` with the bytes the apply path verifies at `height`.
pub fn maintainer_tx(
    node: &Node,
    is_add: bool,
    target: &PublicKey,
    signers: &[&KeyPair],
    height: u64,
) -> Transaction {
    let gate = node
        .config
        .network
        .params()
        .inc_i_176_auth_binding_activation_height;
    let message = signing_message_at(
        node.params.genesis_hash.as_bytes(),
        is_add,
        target,
        MAINTAINER_AUTH_VALID_BEFORE_UNSET,
        height,
        gate,
    );
    let signatures = signers
        .iter()
        .map(|kp| {
            MaintainerSignature::new(
                *kp.public_key(),
                crypto::signature::sign(&message, kp.private_key()),
            )
        })
        .collect();
    Transaction {
        version: 1,
        tx_type: if is_add {
            TxType::AddMaintainer
        } else {
            TxType::RemoveMaintainer
        },
        inputs: vec![],
        outputs: vec![],
        extra_data: MaintainerChangeData::new(*target, signatures).to_bytes(),
    }
}

/// AddMaintainer(newcomer) signed by the first three seated members.
pub fn add_tx(r: &Rig, members: &[KeyPair], newcomer: &KeyPair, height: u64) -> Transaction {
    let signers: Vec<&KeyPair> = members.iter().take(3).collect();
    maintainer_tx(&r.node, true, newcomer.public_key(), &signers, height)
}

/// RemoveMaintainer(last member) signed by the other three.
pub fn remove_tx(r: &Rig, members: &[KeyPair], height: u64) -> Transaction {
    let signers: Vec<&KeyPair> = members.iter().take(3).collect();
    let target = members[MEMBERS - 1].public_key();
    maintainer_tx(&r.node, false, target, &signers, height)
}

/// [coinbase, `gov`, Transfer of a missing outpoint] at (height, slot, prev).
pub fn failing_gov_block(r: &Rig, height: u64, slot: u32, prev: Hash, gov: Transaction) -> Block {
    block_at(r, height, slot, prev, vec![gov, ghost_spend(r)])
}

/// Maintainer observables of one node: memory and the persisted file.
#[derive(Debug, Clone, PartialEq)]
pub struct MObs {
    pub digest: String,
    pub members: Vec<PublicKey>,
    pub threshold: usize,
    pub last_updated: u64,
    pub last_derived: u64,
    pub file: Vec<u8>,
}

pub async fn mobs(node: &Node) -> MObs {
    let ms = node
        .maintainer_state
        .as_ref()
        .expect("FIXTURE: a maintainer root is attached")
        .read()
        .await;
    MObs {
        digest: hex::encode(maintainer_set_digest(
            &ms.set,
            node.params.genesis_hash.as_bytes(),
        )),
        members: ms.set.members.clone(),
        threshold: ms.set.threshold,
        last_updated: ms.set.last_updated,
        last_derived: ms.last_derived_height,
        file: std::fs::read(node.config.data_dir.join(FILE)).expect("FIXTURE: read the root file"),
    }
}

/// Every maintainer observable of `got` equals the control's.
pub fn assert_mobs(got: &MObs, want: &MObs, what: &str) {
    assert_eq!(
        got.members, want.members,
        "{what}: in-memory maintainer members differ from the control (residue)"
    );
    assert_eq!(
        got.digest, want.digest,
        "{what}: in-memory maintainer set digest differs from the control"
    );
    assert_eq!(
        (got.threshold, got.last_updated, got.last_derived),
        (want.threshold, want.last_updated, want.last_derived),
        "{what}: threshold / last_updated / last_derived_height differ from the control"
    );
    assert!(
        got.file == want.file,
        "{what}: maintainer_state.bin bytes differ from the control ({} vs {} bytes)",
        got.file.len(),
        want.file.len()
    );
}

/// Answers the node's header and body requests from `chain` until they stop.
pub async fn serve_sync(s: &Rig, peer: PeerId, chain: &[Block]) -> Vec<&'static str> {
    let mut served = Vec::new();
    for _ in 0..12 {
        let Some((to, req)) = s.node.sync_manager.write().await.next_request() else {
            break;
        };
        assert_eq!(to, peer, "FIXTURE: the only peer is asked");
        let resp = match req {
            SyncRequest::GetHeaders {
                start_hash,
                max_count,
            } => {
                served.push("headers");
                let from = chain
                    .iter()
                    .position(|b| b.header.prev_hash == start_hash)
                    .unwrap_or(chain.len());
                SyncResponse::Headers(
                    chain[from..]
                        .iter()
                        .take(max_count as usize)
                        .map(|b| b.header.clone())
                        .collect(),
                )
            }
            SyncRequest::GetBodies { hashes } => {
                served.push("bodies");
                SyncResponse::Bodies(
                    chain
                        .iter()
                        .filter(|b| hashes.contains(&b.hash()))
                        .cloned()
                        .collect(),
                )
            }
            _ => {
                served.push("other");
                continue;
            }
        };
        s.node.sync_manager.write().await.handle_response(to, resp);
    }
    served
}
