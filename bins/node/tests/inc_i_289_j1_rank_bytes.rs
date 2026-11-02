//! INC-I-289 / AUDIT-P1-639-001: one ranking of the cached branches must copy each
//! cached block's attestation bytes at most once. The bond rank weighs every path
//! offset of a cached chain (`candidate_union(&path[i..])`), so a support view that
//! copies a block's bitfield on every support call copies about N^2/2 x B bytes for
//! one ranking over N cached blocks of B-byte bitfields. The cost is measured from
//! outside: a counting global allocator sums the bytes the test thread allocates
//! during ONE `Node::try_trigger_fork_recovery()` (one support snapshot + one rank).
//! This needs its own test binary: a `#[global_allocator]` is per binary.
//!
// OUTPUT CONTRACT: fn Node::try_trigger_fork_recovery() -> bool (branch_rank ->
//   support_view snapshot -> walk_seeds/ranked_branches -> start_fork_recovery)
//   O1: return value (a walk started)
//   O2: sync_manager walk target (fork_recovery_current_parent = the seed's parent)
//   O3: bytes the calling thread allocates during the call (the ranking's copy cost)
//   PATHS: P1 two cached orphan branches on the fork point, bond rank (gate active)
//   INPUT PARTITIONS: J1 cached S2,S3 (2 blocks, 2 producers, empty bitfields) vs
//     cached L2..L(N+1) (N = 24 blocks, 1 producer, B = 64 KiB bitfield each)
//   MATRIX: J1 x O1,O2 (control: the bond rank seeds S, the depth rank would seed L)
//           J1 x O3 (bound: bytes <= 2 x N x B + 1 MiB; the per-call copy is
//           N(N+1)/2 x B more)

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use crypto::{Hash, KeyPair};
use doli_core::{Block, Network, NetworkParams};

#[path = "support/block_sig.rs"]
mod block_sig;
#[path = "it/inc_i_289_common.rs"]
mod inc_i_289_common;

use inc_i_289_common::*;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static BYTES: Cell<u64> = const { Cell::new(0) };
}

/// `System`, plus a per-thread byte count while `COUNTING` is on.
struct Counting;

fn count(bytes: usize) {
    let on = COUNTING.try_with(Cell::get).unwrap_or(false);
    if on {
        let _ = BYTES.try_with(|b| b.set(b.get() + bytes as u64));
    }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        System.alloc(layout)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        System.alloc_zeroed(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(new_size.saturating_sub(layout.size()));
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// Cached blocks on the long branch, each with a `B`-byte bitfield.
const N: usize = 24;
const B: usize = 64 * 1024;
const SLACK: u64 = 1024 * 1024;

/// First slot at or after `from` whose leader is `k`.
fn slot_led_by(keys: &[KeyPair], k: &KeyPair, from: u32) -> u32 {
    (from..from + 30)
        .find(|s| leader(keys, *s).public_key() == k.public_key())
        .expect("fixture: every producer leads one slot in three")
}

/// A chain on `prev` from `height`: block i by `by[i]`, each at that producer's
/// next led slot after the previous block's slot (the first after `after`).
fn chain_by(r: &Rig, prev: Hash, height: u64, by: &[&KeyPair], after: u32) -> Vec<Block> {
    let (mut prev, mut slot) = (prev, after);
    by.iter()
        .enumerate()
        .map(|(i, k)| {
            slot = slot_led_by(&r.keys, k, slot + 1);
            let b = block(height + i as u64, slot, prev, k, &r.node.params);
            prev = b.hash();
            b
        })
        .collect()
}

/// (blocks, bond of the distinct producers) of `blocks` at `height`.
async fn units(r: &Rig, blocks: &[Block], height: u64) -> (u64, u64) {
    let audit = NetworkParams::load(Network::Devnet).security_audit_activation_height;
    let ps = r.node.producer_set.read().await;
    let mut seen = Vec::new();
    let mut bond = 0u64;
    for b in blocks {
        if !seen.contains(&b.header.producer) {
            seen.push(b.header.producer);
            let pi = ps.get_by_pubkey(&b.header.producer).expect("registered");
            bond += pi.selection_weight_at(height, audit);
        }
    }
    (blocks.len() as u64, bond)
}

/// Node on A1; cached S2,S3 (2 producers) and L2..L(N+1) (1 producer, B-byte
/// bitfields) on the fork point F, with S1 and L1 unknown (both are walk seeds).
async fn fixture() -> (Rig, Vec<Block>, Vec<Block>) {
    let r = rig(&[210]).await;
    let a1 = r.ours[0].clone();
    let (o1, o2) = (r.keys[1].clone(), r.keys[2].clone());
    let s = chain_by(&r, r.f_hash, F + 1, &[&o1, &o2, &o1], a1.header.slot);
    let mut l = chain_by(&r, r.f_hash, F + 1, &[&o2; N + 1], s[2].header.slot);
    for b in l.iter_mut().skip(1) {
        b.attestation_bitfield = vec![0u8; B];
    }
    for (i, b) in s.iter().enumerate().skip(1) {
        cache_at_production_weight(&r, b, F + 1 + i as u64).await;
    }
    for (i, b) in l.iter().enumerate().skip(1) {
        cache_at_production_weight(&r, b, F + 1 + i as u64).await;
    }
    peer_at(&r, F + 3, s[2].hash(), s[2].header.slot).await;
    for b in [&s[0], &l[0]] {
        let unknown = !stored(&r, b) && !cached(&r, &b.hash()).await;
        assert!(unknown, "precondition: S1, L1 unknown");
    }
    let held: usize = (r.node.fork_block_cache.read().await.values())
        .map(|b| b.attestation_bitfield.len())
        .sum();
    assert_eq!(held, N * B, "precondition: N cached bitfields of B bytes");
    let (l_blocks, l_bond) = units(&r, &l[1..], F + 2).await;
    let (s_blocks, s_bond) = units(&r, &s[1..], F + 2).await;
    assert!(
        l_blocks > s_blocks && s_bond > l_bond,
        "precondition: L has more blocks ({l_blocks} > {s_blocks}), S more bond \
         ({s_bond} > {l_bond})"
    );
    (r, s, l)
}

// AUDIT-P1-639-001 control — Decision: a failure means the ranking did not run in the bond unit over the cached branches, so the byte bound below measures nothing.
#[tokio::test]
async fn j1_control_bond_rank_seeds_the_bond_heavier_branch() {
    let (mut r, s, l) = fixture().await;
    let started = r.node.try_trigger_fork_recovery().await;
    let target = (r.node.sync_manager.read().await).fork_recovery_current_parent();
    assert!(started, "J1 control: no walk started");
    assert_eq!(
        target,
        Some(s[0].hash()),
        "J1 control: the walk did not seed S2 (fetch S1) by bond; L1 = {:.8} would be the \
         depth seed",
        l[0].hash()
    );
}

// AUDIT-P1-639-001 — Decision: a failure means one ranking copies the cached attestation bytes once per path offset (quadratic in the cached blocks), which an orphan flood turns into a CPU and memory stall.
#[tokio::test]
async fn j1_one_ranking_copies_each_cached_bitfield_at_most_once() {
    let (mut r, s, _l) = fixture().await;
    BYTES.with(|b| b.set(0));
    COUNTING.with(|c| c.set(true));
    let started = r.node.try_trigger_fork_recovery().await;
    COUNTING.with(|c| c.set(false));
    let bytes = BYTES.with(Cell::get);
    let target = (r.node.sync_manager.read().await).fork_recovery_current_parent();
    assert!(
        started && target == Some(s[0].hash()),
        "J1: the bond rank did not run"
    );
    let (n, b) = (N as u64, B as u64);
    let bound = 2 * n * b + SLACK;
    let quadratic = n * (n + 1) / 2 * b;
    eprintln!("[J1] N={n} B={b} bytes={bytes} bound={bound} per-call-copy={quadratic}");
    assert!(
        bytes <= bound,
        "J1: one ranking allocated {bytes} bytes over N={n} cached blocks of B={b}-byte \
         bitfields; bound 2*N*B+1MiB = {bound}; a per-call copy adds N(N+1)/2*B = {quadratic}"
    );
}
