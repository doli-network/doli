//! INC-I-255 M5 shared fixtures: a devnet node, the accused's real sibling
//! blocks A and B for one slot, and signed decoy blocks for the same slot.
//!
//! OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.

#![allow(dead_code)]

use crypto::{Hash, PublicKey};
use doli_core::block::{Block, BlockHeader};
use doli_core::tpop::heartbeat::hash_chain_vdf;
use doli_core::transaction::SlashData;
use doli_core::Network;

use super::inc_i_236_m4_common::{feed, header_block};
use super::inc_i_255_m1_common::*;
use crate::block_sig::sign_header;

/// The 64 KiB `SlashData` bound of `slash_evidence.rs`.
pub const SLASH_DATA_LIMIT: usize = 64 * 1024;
/// Missed producers that push one header past `SLASH_DATA_LIMIT` (32 B each).
pub const OVERSIZE_MISSED: usize = 2_100;

/// Devnet node (gate at the devnet default) plus the accused's two signed,
/// VDF-valid sibling blocks for `SLOT` on top of genesis.
pub struct Scene {
    pub w: World,
    pub a: Block,
    pub b: Block,
}

pub async fn scene() -> Scene {
    let mut w = world().await;
    let genesis = w.node.chain_state.read().await.best_hash;
    let pk = w.accused_pk();
    let kp = w.accused.clone();
    let a = header_block(&mut w.node, 1, SLOT, genesis, &pk, Some(&kp), 0).await;
    let b = header_block(&mut w.node, 1, SLOT, genesis, &pk, Some(&kp), 1).await;
    assert_ne!(a.hash(), b.hash(), "FIXTURE: sibling headers must conflict");
    Scene { w, a, b }
}

impl Scene {
    pub fn pk(&self) -> PublicKey {
        self.w.accused_pk()
    }

    /// Tx id of the canonical slash built from `x` and `y`.
    pub fn id_of(&self, x: &Block, y: &Block) -> Hash {
        canonical(self.pk(), x.header.clone(), y.header.clone()).hash()
    }

    pub fn ab_id(&self) -> Hash {
        self.id_of(&self.a, &self.b)
    }

    pub async fn feed(&mut self, block: &Block) {
        feed(&mut self.w.node, block.clone()).await;
    }

    pub async fn slashes(&self) -> Vec<Hash> {
        resident_slashes(&self.w.node, &self.pk()).await
    }

    /// Signed, VDF-valid orphan block by the accused at `SLOT` (never applies).
    pub async fn orphan_base(&mut self, salt: u64) -> Block {
        let pk = self.pk();
        let kp = self.w.accused.clone();
        let parent = Hash::from_bytes([0x0D; 32]);
        header_block(&mut self.w.node, 1, SLOT, parent, &pk, Some(&kp), salt).await
    }

    /// `base` with `edit` applied to its header, re-signed by the accused.
    pub fn resigned(&self, base: &Block, edit: impl FnOnce(&mut BlockHeader)) -> Block {
        let mut h = base.header.clone();
        edit(&mut h);
        sign_header(&mut h, &self.w.accused);
        Block::new(h, base.transactions.clone())
    }

    pub async fn decoy_foreign_genesis(&mut self) -> Block {
        let base = self.orphan_base(7).await;
        let d = self.resigned(&base, |h| h.genesis_hash = Hash::from_bytes([0x66; 32]));
        assert_ne!(
            d.header.genesis_hash,
            chain_genesis(),
            "FIXTURE: foreign genesis"
        );
        assert_signed(&d);
        d
    }

    pub async fn decoy_bad_vdf(&mut self, i: u8) -> Block {
        let base = self.orphan_base(20 + u64::from(i)).await;
        let d = self.resigned(&base, |h| h.vdf_output.value = vec![0xE0 ^ i; 32]);
        assert!(
            !vdf_valid(&d.header),
            "FIXTURE: decoy {i} VDF must be garbage"
        );
        assert_signed(&d);
        d
    }

    /// Signature, genesis and VDF all valid; no pair with it fits `SLASH_DATA_LIMIT`.
    pub async fn decoy_oversize(&mut self) -> Block {
        let base = self.orphan_base(9).await;
        let pk = self.pk();
        let d = self.resigned(&base, |h| h.missed_producers = vec![pk; OVERSIZE_MISSED]);
        assert_eq!(
            d.header.genesis_hash,
            chain_genesis(),
            "FIXTURE: chain genesis"
        );
        assert!(
            vdf_valid(&d.header),
            "FIXTURE: oversize decoy VDF must be valid"
        );
        assert_signed(&d);
        for other in [&self.a, &self.b] {
            let n = encoded_len(pk, &d.header, &other.header);
            assert!(n > SLASH_DATA_LIMIT, "FIXTURE: pair with decoy is {n} B");
        }
        d
    }
}

pub fn assert_signed(b: &Block) {
    let r = doli_core::validation::verify_block_signature(&b.header);
    assert!(r.is_ok(), "FIXTURE: decoy signed by the accused; got {r:?}");
}

pub fn vdf_valid(h: &BlockHeader) -> bool {
    let iters = Network::Devnet.heartbeat_vdf_iterations();
    hash_chain_vdf(&h.vdf_input(), iters).to_vec() == h.vdf_output.value
}

pub fn encoded_len(pk: PublicKey, x: &BlockHeader, y: &BlockHeader) -> usize {
    let (lo, hi) = by_hash(x.clone(), y.clone());
    let data = SlashData::double_production(pk, lo, hi);
    bincode::serialize(&data).expect("encode").len()
}
