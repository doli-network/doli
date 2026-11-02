//! INC-I-289 K-2 (SEC-CF-639-004, operator ruling 2): below the INC-I-288 gate no
//! reorg door reads the bond support view, so a fork event must not build it. A
//! built view copies the attestation facts of every cached fork block, so N cached
//! blocks of B-byte bitfields cost about N x B bytes per fork event for nothing.
//! The cost is measured from outside: a counting global allocator sums the bytes
//! the test thread allocates during ONE `Node::handle_completed_fork_recovery()`
//! (the recovered-fork door). Below the gate the call runs in a child process with
//! the gate env override far above the tip; above it (devnet default) in-process.
//! This needs its own test binary: a `#[global_allocator]` is per binary.
//!
// OUTPUT CONTRACT: fn Node::handle_completed_fork_recovery(CompletedRecovery) -> Result<()>
//   (eligibility gate -> evaluate_recovered_fork: record weights -> cache walked
//   blocks -> support view (the site) -> check_reorg_supported -> execute_reorg)
//   O1: return value (Ok)
//   O2: chain tip after the call (the walked 3-block branch beats our 2 blocks)
//   O3: bytes the calling thread allocates during the call
//   PATHS: P1 walked branch on the fork point F, heavier, reorg executed
//   INPUT PARTITIONS: local tip below the gate (child, gate 1e9) | above the gate
//     (devnet default, gate <= F); cache: N = 24 orphans with B = 64 KiB bitfields
//   MATRIX: below x O1,O2 (count rule adopts the walked branch), below x O3
//           (bytes < N*B/2); above x O1,O2, above x O3 (bytes >= N*B: the view
//           copies every cached bitfield, so the counter sees the copy)

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use doli_core::Block;
use network::sync::CompletedRecovery;

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

/// Our slots on F; the walked branch and the cached orphans use later slots.
const S: u32 = 210;
/// Cached orphan blocks, each with a `B`-byte bitfield.
const N: usize = 24;
const B: usize = 64 * 1024;

/// Node on A1,A2 (2 blocks on F); cached orphans L2..L(N+1) on F (L1 unknown), each
/// with a B-byte bitfield; returns the walked branch W1..W3 on F (3 blocks).
async fn fixture() -> (Rig, Vec<Block>) {
    let r = rig(&[S, S + 1]).await;
    let slots: Vec<u32> = (S + 10..S + 11 + N as u32).collect();
    let l = chain_on(&r, r.f_hash, F + 1, &slots);
    for (i, b) in l.iter().enumerate().skip(1) {
        let mut b = b.clone();
        b.attestation_bitfield = vec![0u8; B];
        cache_at_production_weight(&r, &b, F + 1 + i as u64).await;
    }
    let unknown = !stored(&r, &l[0]) && !cached(&r, &l[0].hash()).await;
    assert!(unknown, "precondition: L1 unknown (L2.. are orphans)");
    let held: usize = (r.node.fork_block_cache.read().await.values())
        .map(|b| b.attestation_bitfield.len())
        .sum();
    assert_eq!(held, N * B, "precondition: N cached bitfields of B bytes");
    let walked = chain_on(&r, r.f_hash, F + 1, &[S + 3, S + 4, S + 5]);
    (r, walked)
}

/// Bytes the test thread allocates during one recovered-fork door; checks O1, O2.
async fn door_bytes(r: &mut Rig, walked: &[Block]) -> u64 {
    let rec = CompletedRecovery {
        blocks: walked.to_vec(),
        connection_point: r.f_hash,
        suppliers: Vec::new(),
    };
    BYTES.with(|b| b.set(0));
    COUNTING.with(|c| c.set(true));
    let res = r.node.handle_completed_fork_recovery(rec).await;
    COUNTING.with(|c| c.set(false));
    let bytes = BYTES.with(Cell::get);
    res.expect("O1: the recovered-fork door returns Ok");
    assert_on(r, &walked[2], F + 3, "O2: the walked branch is adopted").await;
    bytes
}

// K-2 INC-I-289 control — Decision: a failure means the counter does not see the view's copy of the cached bitfields above the gate, so the below-gate bound measures nothing.
#[tokio::test]
async fn k2_control_above_gate_the_door_copies_every_cached_bitfield() {
    let (mut r, walked) = fixture().await;
    let gate = (r.node.config.network.params()).inc_i_288_fork_choice_activation_height;
    assert!(gate <= F, "fixture: the node runs above the gate ({gate})");
    let bytes = door_bytes(&mut r, &walked).await;
    let floor = (N * B) as u64;
    eprintln!("[K2 above] N={N} B={B} bytes={bytes} floor N*B={floor}");
    assert!(
        bytes >= floor,
        "K-2 control: above the gate the door allocated {bytes} bytes, below N*B = {floor}"
    );
}

// K-2 SEC-CF-639-004 — Decision: a failure means mainnet (gate u64::MAX) still copies every cached fork block's attestation bytes on each recovered fork for a view nothing reads, which an orphan flood turns into memory churn.
#[tokio::test]
async fn k2_below_gate_the_door_copies_no_cached_bitfield() {
    if !in_below_ah_child("k2_below_gate_the_door_copies_no_cached_bitfield") {
        return;
    }
    let (mut r, walked) = fixture().await;
    let bytes = door_bytes(&mut r, &walked).await;
    let bound = (N * B / 2) as u64;
    eprintln!("[K2 below] N={N} B={B} bytes={bytes} bound N*B/2={bound}");
    assert!(
        bytes < bound,
        "K-2: below the gate the door allocated {bytes} bytes over N={N} cached blocks of \
         B={B}-byte bitfields; bound N*B/2 = {bound} (a built view copies N*B = {})",
        N * B
    );
}
