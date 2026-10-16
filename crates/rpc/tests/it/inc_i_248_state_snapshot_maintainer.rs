//! INC-I-248 M1.5 — `getStateSnapshot` carries the maintainer state.
//! REQ-248-002 — Decision: a red here means `doli snap` receivers get no canonical maintainer set to install.
//
// OUTPUT CONTRACT: RpcContext::handle_request({method: "getStateSnapshot"})
//   O1: response.error — None on both paths (a producer set is attached)
//   O2: result["maintainerState"] — key PRESENT; object (P1) or JSON null (P2)
//   O3: maintainerState fields — members (hex, set order), threshold, lastChangeBlock
//       (= set.last_updated), lastDerivedHeight, digest (= maintainer_set_digest with
//       the context genesis), genesisHash
//   O4: existing keys height/blockHash/stateRoot/chainState/utxoSet/producerSet/
//       epochBondSnapshot/epochAccumulators/totalBytes — present and byte-equal to
//       StateSnapshot::create_gated over the same state
//   mutable params / persistent store: NONE (read-only handler)
// PATHS: P1: maintainer_state attached; P2: maintainer_state None
// INPUT PARTITIONS:
//   P1a: 5 members in NON-sorted insertion order, last_updated (20_000) !=
//        last_derived_height (25_000) — members must keep set order, the two
//        heights must not be swapped
//   P2a: no maintainer state — key present with null (an absent key also reads
//        as Null in serde_json, so presence is asserted separately)
// MATRIX: P1a: O1 O2 O3 O4 ; P2a: O1 O2 O4 — 7 cells

use std::sync::Arc;

use crypto::{Hash, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::maintainer::{maintainer_set_digest, MaintainerSet};
use doli_core::network::Network;
use mempool::{Mempool, MempoolPolicy};
use rpc::types::JsonRpcRequest;
use rpc::RpcContext;
use serde_json::{Map, Value};
use storage::{
    BlockStore, ChainState, MaintainerState, ProducerSet, StateSnapshot, UtxoSet,
    MAINTAINER_STATE_VERSION,
};
use tempfile::TempDir;
use tokio::sync::RwLock;

const SNAP_HEIGHT: u64 = 25_000;
const LAST_CHANGE: u64 = 20_000;

const EXISTING_KEYS: [&str; 9] = [
    "height",
    "blockHash",
    "stateRoot",
    "chainState",
    "utxoSet",
    "producerSet",
    "epochBondSnapshot",
    "epochAccumulators",
    "totalBytes",
];

struct Harness {
    ctx: RpcContext,
    params: ConsensusParams,
    chain_state: ChainState,
    _tempdir: TempDir,
}

fn key(seed: u8) -> PublicKey {
    *crypto::KeyPair::from_seed([seed; 32]).public_key()
}

/// Five keys in DESCENDING byte order, so set order != sorted order.
fn unsorted_members() -> Vec<PublicKey> {
    let mut m = vec![key(11), key(22), key(33), key(44), key(55)];
    m.sort_by(|a, b| b.as_bytes().cmp(a.as_bytes()));
    m
}

fn harness(maintainer: Option<MaintainerSet>) -> Harness {
    let tempdir = TempDir::new().expect("tempdir");
    let network = Network::Testnet;
    let params = ConsensusParams::for_network(network);
    let mut cs = ChainState::new(params.genesis_hash);
    cs.best_height = SNAP_HEIGHT;
    cs.best_hash = Hash::from_bytes([0x5A; 32]);
    cs.best_slot = SNAP_HEIGHT as u32;

    let chain_state = Arc::new(RwLock::new(cs.clone()));
    let utxo_set = Arc::new(RwLock::new(UtxoSet::new()));
    let block_store = Arc::new(BlockStore::open(tempdir.path()).expect("blockstore"));
    let mempool = Arc::new(RwLock::new(Mempool::new(
        MempoolPolicy::default(),
        params.clone(),
        network,
    )));
    let mut ctx = RpcContext::new_for_network(
        chain_state,
        block_store,
        utxo_set,
        mempool,
        params.clone(),
        network,
    );
    if let Some(set) = maintainer {
        ctx.maintainer_state = Some(Arc::new(RwLock::new(MaintainerState {
            version: MAINTAINER_STATE_VERSION,
            set,
            last_derived_height: SNAP_HEIGHT,
        })));
    }
    ctx = ctx.with_producer_set(Arc::new(RwLock::new(ProducerSet::new())));
    Harness {
        ctx,
        params,
        chain_state: cs,
        _tempdir: tempdir,
    }
}

async fn snapshot_result(h: &Harness) -> Map<String, Value> {
    let response = h
        .ctx
        .handle_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "getStateSnapshot".to_string(),
            params: Value::Null,
            id: Value::from(1),
        })
        .await;
    assert!(
        response.error.is_none(),
        "O1: getStateSnapshot errored: {:?}",
        response.error
    );
    match response.result.expect("O1: result present") {
        Value::Object(m) => m,
        other => panic!("O1: result must be a JSON object, got {other}"),
    }
}

fn assert_existing_keys_unchanged(h: &Harness, r: &Map<String, Value>) {
    for k in EXISTING_KEYS {
        assert!(r.contains_key(k), "O4: existing key `{k}` missing: {r:?}");
    }
    let expected = StateSnapshot::create_gated(
        &h.chain_state,
        &UtxoSet::new(),
        &ProducerSet::new(),
        h.ctx.inc_i_181_pending_root_activation_height,
    )
    .expect("expected snapshot");
    assert_eq!(r["height"], Value::from(SNAP_HEIGHT), "O4: height");
    assert_eq!(r["blockHash"], Value::from(expected.block_hash.to_hex()));
    assert_eq!(r["stateRoot"], Value::from(expected.state_root.to_hex()));
    assert_eq!(
        r["chainState"],
        Value::from(hex::encode(&expected.chain_state_bytes))
    );
    assert_eq!(
        r["utxoSet"],
        Value::from(hex::encode(&expected.utxo_set_bytes))
    );
    assert_eq!(
        r["producerSet"],
        Value::from(hex::encode(&expected.producer_set_bytes))
    );
    assert_eq!(r["totalBytes"], Value::from(expected.total_bytes()));
}

// INC-I-248 / REQ-248-002 — Decision: red means the snapshot payload lacks a verifiable maintainer set.
#[tokio::test]
async fn inc_i_248_get_state_snapshot_carries_maintainer_state() {
    let members = unsorted_members();
    let mut sorted = members.clone();
    sorted.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    assert_ne!(
        members, sorted,
        "fixture: set order must differ from sorted"
    );
    let set = MaintainerSet::with_members(members.clone(), LAST_CHANGE);
    let h = harness(Some(set.clone()));
    let r = snapshot_result(&h).await;

    assert!(
        r.contains_key("maintainerState"),
        "O2: getStateSnapshot must carry `maintainerState`; keys = {:?}",
        r.keys().collect::<Vec<_>>()
    );
    let ms = r["maintainerState"].as_object().unwrap_or_else(|| {
        panic!(
            "O2: maintainerState must be an object: {}",
            r["maintainerState"]
        )
    });

    let want_members: Vec<Value> = members.iter().map(|k| Value::from(k.to_hex())).collect();
    assert_eq!(
        ms["members"],
        Value::Array(want_members),
        "O3: members in set order"
    );
    assert_eq!(ms["threshold"], Value::from(3u64), "O3: threshold");
    assert_eq!(
        ms["lastChangeBlock"],
        Value::from(LAST_CHANGE),
        "O3: lastChangeBlock"
    );
    assert_eq!(
        ms["lastDerivedHeight"],
        Value::from(SNAP_HEIGHT),
        "O3: lastDerivedHeight"
    );
    let digest = hex::encode(maintainer_set_digest(
        &set,
        h.params.genesis_hash.as_bytes(),
    ));
    assert_eq!(ms["digest"], Value::from(digest), "O3: digest");
    assert_eq!(
        ms["genesisHash"],
        Value::from(h.params.genesis_hash.to_hex()),
        "O3: genesisHash"
    );

    assert_existing_keys_unchanged(&h, &r);
}

// INC-I-248 / REQ-248-002 — Decision: red means an old-or-stateless node is indistinguishable from a missing field.
#[tokio::test]
async fn inc_i_248_get_state_snapshot_maintainer_state_null_without_state() {
    let h = harness(None);
    let r = snapshot_result(&h).await;

    assert!(
        r.contains_key("maintainerState"),
        "O2: `maintainerState` key must be present (null) without a maintainer state; keys = {:?}",
        r.keys().collect::<Vec<_>>()
    );
    assert!(
        r["maintainerState"].is_null(),
        "O2: maintainerState must be JSON null, got {}",
        r["maintainerState"]
    );

    assert_existing_keys_unchanged(&h, &r);
}
