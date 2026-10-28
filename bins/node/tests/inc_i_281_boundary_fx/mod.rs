//! INC-I-281 M2.3/M2.4 fixtures: a node on [.., F, D_B, D_{B+1}] and walked branches that
//! leave its chain at F (before the epoch boundary B) or at its own D_B.
//!
//! OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.
#![allow(dead_code)]

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::{Block, EpochState, Network};
use doli_node::node::Node;
use network::sync::{CompletedRecovery, ReorgResult};
use network::PeerId;
use tempfile::TempDir;

use crate::inc_i_236_m2_reorg_support::{block, seed_cache, Sig};

/// Last block of epoch 1 on devnet (blocks_per_reward_epoch = 4).
pub const F: u64 = 7;
/// Boundary height: judged under L_close, rotates to the next list after commit.
pub const B: u64 = F + 1;
/// D_B = L_close[111 % 4]: the producer silent through F, so L_new(D_B) keeps it.
pub const D_B_SLOT: u32 = 111;
pub const D_B1_SLOT: u32 = 113;
/// C_B = L_close[112 % 4], a producer that already produced: L_new(C_B) drops the silent one.
pub const C_B_SLOT: u32 = 112;

pub struct Fx {
    pub node: Node,
    pub keys: Vec<KeyPair>,
    pub params: ConsensusParams,
    pub f_hash: Hash,
    pub l_close: Vec<PublicKey>,
    pub l_dead: Vec<PublicKey>,
    pub l_canon: Vec<PublicKey>,
    pub d: [Block; 2],
    pub c_b: Block,
    /// First slot after D_{B+1} where L_new(C_B) and L_new(D_B) name different leaders.
    pub s1: u32,
    /// First slot after `s1` where they name the same leader.
    pub s2: u32,
    /// Height offset (a multiple of blocks_per_reward_epoch): F, B sit at F + o, B + o.
    pub o: u64,
    _tmp: TempDir,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    pub best_hash: Hash,
    pub best_height: u64,
    pub at_b: Option<Hash>,
    pub at_b1: Option<Hash>,
    pub at_b2: Option<Hash>,
    pub persisted: Option<Hash>,
}

pub fn active(es: &EpochState) -> Vec<PublicKey> {
    if es.active_list.is_empty() {
        es.producer_list.clone()
    } else {
        es.active_list.clone()
    }
}

pub fn at(list: &[PublicKey], slot: u32) -> PublicKey {
    list[slot as usize % list.len()]
}

pub fn key_of<'a>(keys: &'a [KeyPair], pk: &PublicKey) -> &'a KeyPair {
    keys.iter()
        .find(|k| k.public_key() == pk)
        .expect("fixture: producer key")
}

/// First offset that puts F + o past the genesis phase and its completion block.
pub fn post_genesis_offset() -> u64 {
    let net = Network::Devnet;
    let mut o = 0;
    while F + o <= net.genesis_blocks() + 1 {
        o += net.blocks_per_reward_epoch();
    }
    o
}

/// Base h=1..=F+o by three of four producers; the fourth (largest key) is silent from
/// h=o-3 on: in L_close, outside the three-epoch window that derives the list at B+o.
async fn base_node(tmp: &TempDir, keys: &[KeyPair], o: u64) -> (Node, ConsensusParams, Hash) {
    let mut node = Node::new_for_test(tmp.path().to_path_buf(), keys.to_vec())
        .await
        .expect("Node::new_for_test");
    let params = node.params.clone();
    let mut prev = Hash::ZERO;
    for h in 1..=F + o {
        let (by, slot) = if h > o {
            (&keys[(h - o - 1) as usize % 3], 100 + (h - o) as u32)
        } else if h + 4 <= o {
            (&keys[(h - 1) as usize % 4], h as u32)
        } else {
            (&keys[(h - 1) as usize % 3], h as u32)
        };
        let b = block(h, slot, prev, by, Sig::Valid, &params);
        prev = b.hash();
        node.apply_block(b, ValidationMode::Light)
            .await
            .expect("fixture: base block");
    }
    (node, params, prev)
}

/// The node: base, then its own D_B (by the silent producer) and D_{B+1}. L_new(C_B) is
/// read from a twin node that applies the same base and C_B.
pub async fn fixture() -> Fx {
    fixture_at(0).await
}

pub async fn fixture_at(o: u64) -> Fx {
    let mut keys: Vec<KeyPair> = (0..4).map(|_| KeyPair::generate()).collect();
    keys.sort_by(|a, b| a.public_key().as_bytes().cmp(b.public_key().as_bytes()));
    let tmp = TempDir::new().unwrap();
    let (mut node, params, f_hash) = base_node(&tmp, &keys, o).await;
    let b = B + o;
    let l_close = active(&node.epoch_state);
    let by_d_b = key_of(&keys, &at(&l_close, D_B_SLOT));
    let d_b = block(b, D_B_SLOT, f_hash, by_d_b, Sig::Valid, &params);
    node.apply_block(d_b.clone(), ValidationMode::Light)
        .await
        .expect("fixture: D_B");
    let l_dead = active(&node.epoch_state);
    let by_d_b1 = key_of(&keys, &at(&l_dead, D_B1_SLOT));
    let d_b1 = block(b + 1, D_B1_SLOT, d_b.hash(), by_d_b1, Sig::Valid, &params);
    node.apply_block(d_b1.clone(), ValidationMode::Light)
        .await
        .expect("fixture: D_B+1");
    let by_c_b = key_of(&keys, &at(&l_close, C_B_SLOT));
    let c_b = block(b, C_B_SLOT, f_hash, by_c_b, Sig::Valid, &params);
    let l_canon = {
        let twin_tmp = TempDir::new().unwrap();
        let (mut twin, _, _) = base_node(&twin_tmp, &keys, o).await;
        twin.apply_block(c_b.clone(), ValidationMode::Light)
            .await
            .expect("fixture: twin C_B");
        active(&twin.epoch_state)
    };
    assert_ne!(l_canon, l_dead, "precondition: L_new(C_B) != L_new(D_B)");
    let differs = |s: &u32| at(&l_canon, *s) != at(&l_dead, *s);
    let s1 = (D_B1_SLOT + 1..D_B1_SLOT + 64)
        .find(differs)
        .expect("fixture: s1");
    let s2 = (s1 + 1..s1 + 64)
        .find(|s| !differs(s))
        .expect("fixture: s2");
    let fx = Fx {
        node,
        keys,
        params,
        f_hash,
        l_close,
        l_dead,
        l_canon,
        d: [d_b, d_b1],
        c_b,
        s1,
        s2,
        o,
        _tmp: tmp,
    };
    assert_preconditions(&fx).await;
    fx
}

/// The P1 shape: three different lists, the closing one recoverable from undo(D_B).
async fn assert_preconditions(fx: &Fx) {
    let all: Vec<PublicKey> = fx.keys.iter().map(|k| *k.public_key()).collect();
    assert_eq!(
        fx.l_close, all,
        "precondition: L_close = all four producers"
    );
    assert_eq!(
        fx.l_dead, all,
        "precondition: D_B's producer was silent, so L_new(D_B) keeps all four"
    );
    assert_eq!(
        fx.l_canon,
        all[..3].to_vec(),
        "precondition: L_new(C_B) drops the silent producer"
    );
    let undo = fx
        .node
        .state_db
        .get_undo(fx.b())
        .expect("precondition: undo(D_B)");
    let snap = EpochState::deserialize(&undo.epoch_state_snapshot.expect("undo snapshot"))
        .expect("precondition: undo(D_B) epoch state decodes");
    assert_eq!(
        active(&snap),
        fx.l_close,
        "precondition: undo(D_B) = L_close"
    );
    assert_eq!(
        fx.state().await.best_hash,
        fx.d[1].hash(),
        "precondition: tip = D_B+1"
    );
}

impl Fx {
    pub fn b(&self) -> u64 {
        B + self.o
    }

    pub fn assert_past_genesis(&self) {
        let net = self.node.config.network;
        assert!(
            !net.is_in_genesis(F + self.o),
            "precondition: F={} past a genesis phase of {} blocks",
            F + self.o,
            net.genesis_blocks()
        );
    }

    pub fn key(&self, pk: &PublicKey) -> &KeyPair {
        key_of(&self.keys, pk)
    }

    pub fn mk(&self, prev: Hash, height: u64, slot: u32, by: &KeyPair) -> Block {
        block(height, slot, prev, by, Sig::Valid, &self.params)
    }

    /// [C_B, b1, b2]: b1 at s1 by `by1`, b2 at s2 by its leader under both lists.
    pub fn after_c_b(&self, by1: &KeyPair) -> Vec<Block> {
        let b1 = self.mk(self.c_b.hash(), self.b() + 1, self.s1, by1);
        let by2 = self.key(&at(&self.l_canon, self.s2));
        let b2 = self.mk(b1.hash(), self.b() + 2, self.s2, by2);
        vec![self.c_b.clone(), b1, b2]
    }

    /// Canonical: C_{B+1} = L_new(C_B)[s1].
    pub fn canonical(&self) -> Vec<Block> {
        self.after_c_b(self.key(&at(&self.l_canon, self.s1)))
    }

    /// C'_{B+1} = L_new(D_B)[s1]: scheduled only under the node's list.
    pub fn ineligible(&self) -> Vec<Block> {
        self.after_c_b(self.key(&at(&self.l_dead, self.s1)))
    }

    /// X_{B+1} by a key that is neither registered nor in the GSet.
    pub fn unknown(&self) -> Vec<Block> {
        self.after_c_b(&KeyPair::generate())
    }

    /// Y_B on F at C_B's slot by a registered producer that is not L_close[slot].
    pub fn closing_ineligible(&self) -> Vec<Block> {
        let y_by = self.key(&at(&self.l_close, C_B_SLOT + 1));
        let y_b = self.mk(self.f_hash, self.b(), C_B_SLOT, y_by);
        let y1 = self.mk(
            y_b.hash(),
            self.b() + 1,
            self.s1,
            self.key(&at(&self.l_canon, self.s1)),
        );
        let y2 = self.mk(
            y1.hash(),
            self.b() + 2,
            self.s2,
            self.key(&at(&self.l_canon, self.s2)),
        );
        vec![y_b, y1, y2]
    }

    /// Z_{B+1} on the node's own D_B by L_new(C_B)[s1], which is not L_new(D_B)[s1].
    pub fn on_own_boundary(&self) -> Vec<Block> {
        let z1_by = self.key(&at(&self.l_canon, self.s1));
        let z1 = self.mk(self.d[0].hash(), self.b() + 1, self.s1, z1_by);
        let z2 = self.mk(
            z1.hash(),
            self.b() + 2,
            self.s2,
            self.key(&at(&self.l_dead, self.s2)),
        );
        vec![z1, z2]
    }

    pub async fn state(&self) -> State {
        let (best_hash, best_height) = {
            let cs = self.node.chain_state.read().await;
            (cs.best_hash, cs.best_height)
        };
        let at_h = |h: u64| self.node.block_store.get_hash_by_height(h).expect("store");
        State {
            best_hash,
            best_height,
            at_b: at_h(self.b()),
            at_b1: at_h(self.b() + 1),
            at_b2: at_h(self.b() + 2),
            persisted: self.node.state_db.get_chain_state().map(|c| c.best_hash),
        }
    }

    pub fn applied(&self, b: &Block) -> bool {
        self.node
            .block_store
            .get_block(&b.hash())
            .expect("store")
            .is_some()
    }

    pub fn retained(&self, b: &Block) -> bool {
        self.node.wedge_retained_tips.contains(&b.hash())
    }
}

pub async fn run_walk(fx: &mut Fx, br: &[Block], connection_point: Hash) -> anyhow::Result<()> {
    let recovery = CompletedRecovery {
        blocks: br.to_vec(),
        connection_point,
    };
    fx.node.handle_completed_fork_recovery(recovery).await
}

pub async fn gossip(fx: &mut Fx, b: &Block) {
    fx.node
        .handle_new_block(b.clone(), PeerId::random())
        .await
        .expect("handle_new_block returns Ok");
}

/// Rollback D_{B+1}, then apply `br` on D_B through execute_reorg (no door).
pub async fn run_direct_reorg(fx: &mut Fx, br: &[Block]) -> anyhow::Result<()> {
    seed_cache(&fx.node, &br[..br.len() - 1]).await;
    let plan = ReorgResult {
        rollback: vec![fx.d[1].hash()],
        common_ancestor: fx.d[0].hash(),
        new_blocks: br.iter().map(|b| b.hash()).collect(),
        weight_delta: 1,
    };
    fx.node
        .execute_reorg(plan, br.last().unwrap().clone())
        .await
}

/// Tip = br's last block at B+2, with `br` canonical from `first_h`, on every output.
pub async fn assert_landed(fx: &Fx, br: &[Block], first_h: u64, label: &str) {
    let st = fx.state().await;
    let tip = br.last().unwrap().hash();
    assert_eq!(st.best_hash, tip, "{label}: tip ({st:?})");
    assert_eq!(st.best_height, fx.b() + 2, "{label}: height");
    for (i, b) in br.iter().enumerate() {
        let h = first_h + i as u64;
        let got = fx.node.block_store.get_hash_by_height(h).expect("store");
        assert_eq!(got, Some(b.hash()), "{label}: canonical block at h={h}");
    }
    assert_eq!(st.persisted, Some(tip), "{label}: persisted tip");
}
