// INC-I-281 R-4 M3a.2 (Must): execute_reorg judges a fork block's schedule with the
// block's slot as the predicate height; a Full apply uses the block's height. A block at a
// genesis-phase height with a post-genesis slot takes the round-robin arm in the reorg and
// the bootstrap arm in a Full apply, so the reorg stops on a block a Full apply accepts.
//
// OUTPUT CONTRACT: Node::execute_reorg (rollback of A, per-block schedule check at the
// block's parent, Light apply); reference verdict Node::apply_block(Full) on a twin.
// OUTPUTS:
//   O1 execute_reorg return value (Ok on both arms; a schedule refusal is not an Err)
//   O2 chain_state best_hash / best_height
//   O3 block_store canonical hash at fork_h, fork_h+1; body presence of the fork blocks
//   O4 state_db persisted chain-state tip
//   O5 Full apply of the same fork block on the same parent (twin node)
// PATHS: schedule check -> bootstrap arm (height in genesis) | round-robin arm (past genesis)
// INPUT PARTITIONS (fork block height x slot x producer):
//   G  height in genesis, slot past genesis, registered, not list[slot % len] -> applied
//   U  height past genesis, slot past genesis, registered, not list[slot % len] -> refused
//   L  as U but by list[slot % len] (control: U's refusal is the schedule)      -> applied
// MATRIX: G x {O1..O5}; U x {O1..O4}; L x {O1..O3}.
// covers: reorg_schedule.rs, block_handling.rs

#[path = "support/block_sig.rs"]
mod block_sig;
mod inc_i_236_m2_reorg_support;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::{Block, EpochState, Network};
use doli_node::node::Node;
use inc_i_236_m2_reorg_support::{block, seed_cache, Sig};
use network::sync::ReorgResult;
use tempfile::TempDir;

/// Every block here sits at slot SLOT_BASE + 2h: past the genesis phase from h=1.
const SLOT_BASE: u32 = 100;
/// Fork height inside the genesis phase (not an epoch boundary).
const GENESIS_FORK_H: u64 = 6;

struct Fx {
    node: Node,
    keys: Vec<KeyPair>,
    params: ConsensusParams,
    /// h = 1..=fork_h-1.
    base: Vec<Block>,
    /// The node's own block at fork_h.
    a: Block,
    fork_h: u64,
    /// Epoch list at the fork parent.
    list: Vec<PublicKey>,
    _tmp: TempDir,
}

#[derive(Debug, PartialEq, Eq)]
struct State {
    best_hash: Hash,
    best_height: u64,
    at_h: Option<Hash>,
    at_h1: Option<Hash>,
    persisted: Option<Hash>,
}

fn active(es: &EpochState) -> Vec<PublicKey> {
    if es.active_list.is_empty() {
        es.producer_list.clone()
    } else {
        es.active_list.clone()
    }
}

fn at(list: &[PublicKey], slot: u32) -> PublicKey {
    list[slot as usize % list.len()]
}

fn key_of<'a>(keys: &'a [KeyPair], pk: &PublicKey) -> &'a KeyPair {
    keys.iter()
        .find(|k| k.public_key() == pk)
        .expect("fixture: producer key")
}

fn slot_of(h: u64) -> u32 {
    SLOT_BASE + 2 * h as u32
}

/// First height past the genesis phase whose block and child do not cross an epoch boundary.
fn post_genesis_fork_h() -> u64 {
    let net = Network::Devnet;
    let bpe = net.blocks_per_reward_epoch();
    let mut h = net.genesis_blocks() + 2;
    while h.is_multiple_of(bpe) || (h + 1).is_multiple_of(bpe) {
        h += 1;
    }
    h
}

/// Node with base h=1..fork_h-1 (each by its round-robin leader) and its own A at fork_h.
async fn fixture(fork_h: u64) -> Fx {
    let mut keys: Vec<KeyPair> = (0..3).map(|_| KeyPair::generate()).collect();
    keys.sort_by(|a, b| a.public_key().as_bytes().cmp(b.public_key().as_bytes()));
    let tmp = TempDir::new().unwrap();
    let mut node = Node::new_for_test(tmp.path().to_path_buf(), keys.clone())
        .await
        .expect("Node::new_for_test");
    let params = node.params.clone();
    let mut base = Vec::new();
    let mut prev = Hash::ZERO;
    for h in 1..fork_h {
        let by = key_of(&keys, &at(&active(&node.epoch_state), slot_of(h)));
        let b = block(h, slot_of(h), prev, by, Sig::Valid, &params);
        prev = b.hash();
        node.apply_block(b.clone(), ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("fixture: base block h={h}: {e}"));
        base.push(b);
    }
    let list = active(&node.epoch_state);
    let a = block(
        fork_h,
        slot_of(fork_h),
        prev,
        key_of(&keys, &at(&list, slot_of(fork_h))),
        Sig::Valid,
        &params,
    );
    node.apply_block(a.clone(), ValidationMode::Light)
        .await
        .expect("fixture: A");
    Fx {
        node,
        keys,
        params,
        base,
        a,
        fork_h,
        list,
        _tmp: tmp,
    }
}

impl Fx {
    fn parent(&self) -> Hash {
        self.base.last().expect("fixture: base").hash()
    }

    fn fork_slot(&self) -> u32 {
        slot_of(self.fork_h) + 1
    }

    /// [X, X'] on the fork parent: X at fork_slot by `by`, X' by its leader one slot later.
    fn fork(&self, by: &PublicKey) -> Vec<Block> {
        let s = self.fork_slot();
        let x = block(
            self.fork_h,
            s,
            self.parent(),
            key_of(&self.keys, by),
            Sig::Valid,
            &self.params,
        );
        let by2 = key_of(&self.keys, &at(&self.list, s + 1));
        let x1 = block(
            self.fork_h + 1,
            s + 1,
            x.hash(),
            by2,
            Sig::Valid,
            &self.params,
        );
        vec![x, x1]
    }

    /// First block by a registered producer that is not list[slot % len].
    fn unscheduled(&self) -> Vec<Block> {
        self.fork(&at(&self.list, self.fork_slot() + 1))
    }

    fn scheduled(&self) -> Vec<Block> {
        self.fork(&at(&self.list, self.fork_slot()))
    }

    async fn state(&self) -> State {
        let (best_hash, best_height) = {
            let cs = self.node.chain_state.read().await;
            (cs.best_hash, cs.best_height)
        };
        let at_h = |h: u64| self.node.block_store.get_hash_by_height(h).expect("store");
        State {
            best_hash,
            best_height,
            at_h: at_h(self.fork_h),
            at_h1: at_h(self.fork_h + 1),
            persisted: self.node.state_db.get_chain_state().map(|c| c.best_hash),
        }
    }

    fn applied(&self, b: &Block) -> bool {
        self.node
            .block_store
            .get_block(&b.hash())
            .expect("store")
            .is_some()
    }

    fn assert_preconditions(&self, br: &[Block], in_genesis: bool) {
        let net = self.node.config.network;
        let slot = br[0].header.slot;
        assert!(
            self.node.snap_sync_height.is_none(),
            "precondition: snap_sync_height unset"
        );
        assert!(
            !self.node.floor_fallback_window,
            "precondition: floor_fallback_window off"
        );
        assert_eq!(
            (
                net.is_in_genesis(self.fork_h),
                net.is_in_genesis(self.fork_h + 1)
            ),
            (in_genesis, in_genesis),
            "precondition: fork heights {}..={} vs a genesis phase of {} blocks",
            self.fork_h,
            self.fork_h + 1,
            net.genesis_blocks()
        );
        assert!(
            !net.is_in_genesis(slot as u64),
            "precondition: slot {slot} past the genesis phase"
        );
        assert_eq!(self.list.len(), self.keys.len(), "precondition: full list");
        assert!(
            self.list.contains(&br[0].header.producer),
            "precondition: fork producer is registered"
        );
    }
}

/// Rollback A, then apply `br` on the fork parent through execute_reorg (no door).
async fn reorg(fx: &mut Fx, br: &[Block]) -> anyhow::Result<()> {
    seed_cache(&fx.node, &br[..br.len() - 1]).await;
    let plan = ReorgResult {
        rollback: vec![fx.a.hash()],
        common_ancestor: fx.parent(),
        new_blocks: br.iter().map(|b| b.hash()).collect(),
        weight_delta: 1,
    };
    fx.node
        .execute_reorg(plan, br.last().unwrap().clone())
        .await
}

/// Full apply of `b` on a twin holding the same base (the fork parent is its tip).
async fn full_apply_on_parent(fx: &Fx, b: &Block) -> Result<(), String> {
    let tmp = TempDir::new().unwrap();
    let mut twin = Node::new_for_test(tmp.path().to_path_buf(), fx.keys.clone())
        .await
        .expect("Node::new_for_test (twin)");
    for blk in &fx.base {
        twin.apply_block(blk.clone(), ValidationMode::Light)
            .await
            .expect("twin: base block");
    }
    twin.apply_block(b.clone(), ValidationMode::Full)
        .await
        .map_err(|e| e.to_string())?;
    let tip = twin.chain_state.read().await.best_hash;
    if tip == b.hash() {
        Ok(())
    } else {
        Err(format!("Full apply returned Ok, tip {tip} != block"))
    }
}

fn probe(cell: &str, wrong: bool) {
    println!("INC281_PROBE genesisreorg={cell} wrong={}", wrong as u8);
}

// INC-I-281 R-4 M3a.2 (Must) — Decision: a failure means execute_reorg still judges a genesis-phase block with its slot as the height, so a reorg on a young chain stops on a block a Full apply accepts.
#[tokio::test]
async fn genesis_height_late_slot() {
    let mut fx = fixture(GENESIS_FORK_H).await;
    let br = fx.unscheduled();
    let slot = br[0].header.slot;
    fx.assert_preconditions(&br, true);
    assert_ne!(
        br[0].header.producer,
        at(&fx.list, slot),
        "precondition: G is not the round-robin pick"
    );
    let full = full_apply_on_parent(&fx, &br[0]).await;
    let result = reorg(&mut fx, &br).await;
    let st = fx.state().await;
    let applied = fx.applied(&br[0]);
    probe("genesis_height_late_slot", !applied);
    assert!(
        full.is_ok(),
        "precondition: a Full apply on the same parent accepts G ({full:?})"
    );
    assert!(
        applied,
        "execute_reorg refused G at genesis-phase h={}, slot {slot}: judged by the round-robin arm a Full apply does not take ({result:?}, {st:?})",
        fx.fork_h
    );
    result.expect("reorg: Ok");
    let tip = br[1].hash();
    assert_eq!(
        st,
        State {
            best_hash: tip,
            best_height: fx.fork_h + 1,
            at_h: Some(br[0].hash()),
            at_h1: Some(tip),
            persisted: Some(tip),
        },
        "reorg lands on [G, G']"
    );
}

// INC-I-281 R-4 M3a.2 (Must) — Decision: a failure means moving the reorg predicate to the block's height also let an unscheduled block past the genesis phase through execute_reorg.
#[tokio::test]
async fn post_genesis_unscheduled() {
    let fork_h = post_genesis_fork_h();
    let mut ctl = fixture(fork_h).await;
    let lb = ctl.scheduled();
    ctl.assert_preconditions(&lb, false);
    let ctl_result = reorg(&mut ctl, &lb).await;
    let ctl_st = ctl.state().await;
    let control_landed = ctl_st.at_h == Some(lb[0].hash()) && ctl_st.best_hash == lb[1].hash();

    let mut fx = fixture(fork_h).await;
    let br = fx.unscheduled();
    let slot = br[0].header.slot;
    fx.assert_preconditions(&br, false);
    assert_ne!(
        br[0].header.producer,
        at(&fx.list, slot),
        "precondition: U is not the round-robin pick"
    );
    let result = reorg(&mut fx, &br).await;
    let st = fx.state().await;
    let applied = fx.applied(&br[0]);
    probe("post_genesis_unscheduled", applied);
    assert!(
        control_landed,
        "precondition: the scheduled sibling L applies ({ctl_result:?}, {ctl_st:?})"
    );
    assert!(
        !applied,
        "execute_reorg applied U at post-genesis h={fork_h}, not list[{slot} % len] ({result:?}, {st:?})"
    );
    assert!(!fx.applied(&br[1]), "U' applied on a refused parent");
    result.expect("reorg: a schedule refusal is Ok");
    let own = fx.a.hash();
    assert_eq!(
        st,
        State {
            best_hash: own,
            best_height: fork_h,
            at_h: Some(own),
            at_h1: None,
            persisted: Some(own),
        },
        "a schedule refusal restores the node's own tip A at fork_h"
    );
}
