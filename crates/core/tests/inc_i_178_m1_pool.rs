//! INC-I-178 M1 — the `ParentSignaturePool` contract (spec D2).
//!
//! RED until `crates/core/src/attestation/pool.rs` exists. Every assertion here
//! is a promise M2 relies on: the pool is the only place a parent-keyed BLS
//! signature can be found, and a relay can never displace an honest one.
//!
//! OUTPUT CONTRACT
//!
//! F1: `ParentSignaturePool::insert(&mut self, parent: Hash, attester: PublicKey, sig: [u8; 96]) -> bool`
//!   Observable outputs:
//!     O1 return — `true` iff this call stored `sig`; `false` iff an entry for
//!        (parent, attester) already existed and was kept
//!     O2 `self.pool` — the (parent, attester) -> sig map
//!     O3 `self.recent` — the bounded parent window, observable only through
//!        `parent_count()` and which parents survive a later insert
//!     O4 mutable params — NONE (`parent`, `attester`, `sig` are by value)
//!     O5 persistent store / global / channel — NONE (node-local, never persisted, C12)
//!   Paths:
//!     P1 new parent, pool below capacity            -> O1=true, O2 gains 1, O3 gains 1
//!     P2 new parent, pool AT capacity K=8           -> O1=true, O2 gains 1 and loses the
//!                                                      oldest parent's whole map, O3 evicts oldest
//!     P3 known parent, new attester                 -> O1=true, O2 gains 1, O3 UNCHANGED
//!     P4 known parent, known attester (C1 re-send)  -> O1=false, O2 UNCHANGED, O3 UNCHANGED
//!   INPUT PARTITIONS:
//!     P1a first ever insert into an empty pool
//!     P1b second attester under the same parent (fan-out on the attester axis)
//!     P1c same attester under a second parent (fan-out on the parent axis)
//!     P2a 9 distinct parents, one attester each — oldest-first eviction
//!     P2b 9 distinct parents after a re-touch of the oldest — the re-touch must NOT
//!         renew it (first-seen FIFO, not LRU)
//!     P3a N=45 attesters x 8 parents (mainnet-ish fan-out)
//!     P3b N=200 attesters x 8 parents (large-fleet fan-out)
//!     P4a identical bytes re-sent
//!     P4b DIFFERENT bytes under the same key — the adversarial case; the first must win
//!
//! F2: `ParentSignaturePool::get(&self, &Hash, &PublicKey) -> Option<&[u8; 96]>`
//!   Observable outputs: O1 return only. O2..O5 NONE (`&self`).
//!   Paths: P1 hit -> Some(exact 96 bytes) | P2 unknown parent -> None
//!          P3 known parent, unknown attester -> None | P4 evicted parent -> None
//!   INPUT PARTITIONS: empty pool; populated pool; post-eviction pool.
//!
//! F3: `ParentSignaturePool::parent_count / total_signatures / signatures_for / contains_parent`
//!   Observable outputs: O1 return only. Paths: empty | populated | saturated.
//!   INPUT PARTITIONS: N in {1, 45, 200} attesters, parents in {0, 1, 8, 9}.
//!
//! MATRIX: F1 4 paths x 9 partitions -> the reachable cells are covered by the 12
//! tests below; O4/O5 are constant-NONE by type and are asserted once, structurally
//! (`insert` takes its arguments by value; the pool exposes no store handle).
//!
//! Requirement IDs: REQ-BLS-012 (drain the dead BLS surface — this pool is the
//! live replacement for the deleted minute-keyed store), REQ-BLS-010 (liveness
//! must not regress — the pool is bounded so it can never starve a producer).

use std::collections::HashMap;

use crypto::{Hash, PublicKey};
use doli_core::attestation::pool::ParentSignaturePool;

/// The K of spec D2. Pinned here so a silent widening fails the suite.
const K: usize = 8;

fn parent(tag: u8) -> Hash {
    crypto::hash::hash(&[b'p', tag])
}

fn attester(tag: u16) -> PublicKey {
    let mut b = [0u8; 32];
    b[0] = (tag & 0xff) as u8;
    b[1] = (tag >> 8) as u8;
    b[31] = 0xA7;
    PublicKey::from_bytes(b)
}

fn sig(tag: u8) -> [u8; 96] {
    let mut s = [0u8; 96];
    s[0] = tag;
    s[95] = tag ^ 0xff;
    s
}

/// Size of the referent, so a `Vec<u8>` value type (24 bytes) cannot pass.
fn ref_size<T>(_: &T) -> usize {
    std::mem::size_of::<T>()
}

fn map_value_size<K2, V>(_: &HashMap<K2, V>) -> usize {
    std::mem::size_of::<V>()
}

// REQ-BLS-012 — Decision: a pool that answers "yes" on an empty state would let
// M4's verifier build an aggregate out of nothing.
#[test]
fn m1_pool_empty_pool_yields_no_signature_for_any_parent() {
    let p = ParentSignaturePool::new();
    assert_eq!(p.parent_count(), 0);
    assert_eq!(p.total_signatures(), 0);
    for t in 0..4u8 {
        assert!(!p.contains_parent(&parent(t)));
        assert!(p.signatures_for(&parent(t)).is_none());
        assert!(p.get(&parent(t), &attester(0)).is_none());
    }
}

// REQ-BLS-012 — Decision: a byte-lossy round trip means the aggregate M2 builds
// is over a signature nobody produced.
#[test]
fn m1_pool_insert_then_read_back_returns_the_exact_96_bytes() {
    let mut pool = ParentSignaturePool::new();
    let (p, a, s) = (parent(1), attester(1), sig(0x5A));

    assert!(pool.insert(p, a, s), "first insert must report stored");
    let got = pool.get(&p, &a).expect("signature must be retrievable");
    assert_eq!(got, &s);
    assert_eq!(pool.parent_count(), 1);
    assert_eq!(pool.total_signatures(), 1);
    assert!(pool.contains_parent(&p));
}

// REQ-BLS-012 — Decision: if a later insert wins, one relay replaying a mutated
// blob evicts the honest signature and the aggregate fails to verify (C1).
#[test]
fn m1_pool_never_overwrites_an_existing_parent_attester_entry() {
    let mut pool = ParentSignaturePool::new();
    let (p, a) = (parent(1), attester(1));
    let honest = sig(0x11);
    let hostile = sig(0x22);
    assert_ne!(honest, hostile);

    assert!(pool.insert(p, a, honest));
    assert!(
        !pool.insert(p, a, hostile),
        "second insert must report that it did NOT store"
    );

    assert_eq!(pool.get(&p, &a), Some(&honest));
    assert_eq!(pool.total_signatures(), 1);
    assert_eq!(pool.parent_count(), 1);
}

// REQ-BLS-012 — Decision: a same-bytes re-send is the common gossip duplicate;
// it must be idempotent and must not double-count.
#[test]
fn m1_pool_identical_resend_is_idempotent_and_reports_not_stored() {
    let mut pool = ParentSignaturePool::new();
    let (p, a, s) = (parent(1), attester(1), sig(0x33));

    assert!(pool.insert(p, a, s));
    assert!(!pool.insert(p, a, s));
    assert!(!pool.insert(p, a, s));

    assert_eq!(pool.get(&p, &a), Some(&s));
    assert_eq!(pool.total_signatures(), 1);
}

// REQ-BLS-012 — Decision: collapsing attesters under one parent would cap the
// aggregate at one signer and make the bitfield permanently 1-bit wide.
#[test]
fn m1_pool_two_attesters_under_one_parent_are_both_retained() {
    let mut pool = ParentSignaturePool::new();
    let p = parent(1);
    let (a1, a2) = (attester(1), attester(2));
    let (s1, s2) = (sig(0x01), sig(0x02));

    assert!(pool.insert(p, a1, s1));
    assert!(pool.insert(p, a2, s2));

    assert_eq!(pool.get(&p, &a1), Some(&s1));
    assert_eq!(pool.get(&p, &a2), Some(&s2));
    assert_eq!(pool.parent_count(), 1);
    assert_eq!(pool.total_signatures(), 2);
    assert_eq!(pool.signatures_for(&p).expect("parent present").len(), 2);
}

// REQ-BLS-012 — Decision: if the attester axis were global rather than
// per-parent, a producer that attested block A could never attest sibling B —
// exactly the fork-window starvation the pool exists to avoid.
#[test]
fn m1_pool_same_attester_under_two_parents_is_retained_independently() {
    let mut pool = ParentSignaturePool::new();
    let (p1, p2) = (parent(1), parent(2));
    let a = attester(1);
    let (s1, s2) = (sig(0xAA), sig(0xBB));

    assert!(pool.insert(p1, a, s1));
    assert!(pool.insert(p2, a, s2), "a sibling parent is a distinct key");

    assert_eq!(pool.get(&p1, &a), Some(&s1));
    assert_eq!(pool.get(&p2, &a), Some(&s2));
    assert_eq!(pool.parent_count(), 2);
    assert_eq!(pool.total_signatures(), 2);
}

// REQ-BLS-010 — Decision: an unbounded parent map grows with chain height and
// eventually OOMs a producer; K=8 is the declared reorg headroom.
#[test]
fn m1_pool_keeps_exactly_the_newest_eight_parents_and_evicts_the_oldest() {
    let mut pool = ParentSignaturePool::new();
    let a = attester(1);
    for t in 0..(K as u8 + 1) {
        assert!(pool.insert(parent(t), a, sig(t)));
    }

    assert_eq!(pool.parent_count(), K);
    assert_eq!(pool.total_signatures(), K);

    assert!(
        !pool.contains_parent(&parent(0)),
        "the oldest parent must be evicted"
    );
    assert!(pool.get(&parent(0), &a).is_none());
    assert!(pool.signatures_for(&parent(0)).is_none());

    for t in 1..(K as u8 + 1) {
        assert!(pool.contains_parent(&parent(t)), "parent {t} must survive");
        assert_eq!(pool.get(&parent(t), &a), Some(&sig(t)));
    }
}

// REQ-BLS-010 — Decision: eviction must drop the evicted parent's WHOLE attester
// map, not just one entry, or the memory bound is a fiction.
#[test]
fn m1_pool_eviction_drops_every_signature_of_the_evicted_parent() {
    let mut pool = ParentSignaturePool::new();
    for t in 0..(K as u8) {
        for i in 0..5u16 {
            assert!(pool.insert(parent(t), attester(i), sig(i as u8)));
        }
    }
    assert_eq!(pool.total_signatures(), K * 5);

    assert!(pool.insert(parent(K as u8), attester(0), sig(0)));

    assert_eq!(pool.parent_count(), K);
    assert_eq!(
        pool.total_signatures(),
        (K - 1) * 5 + 1,
        "the evicted parent must take all 5 of its signatures with it"
    );
    for i in 0..5u16 {
        assert!(pool.get(&parent(0), &attester(i)).is_none());
    }
}

// REQ-BLS-010 — Decision: LRU renewal on re-touch would let a busy old parent
// pin the window forever and starve the newest parents the builder actually
// needs; the declared policy is FIRST-SEEN FIFO.
#[test]
fn m1_pool_retouching_a_parent_does_not_renew_it_in_the_window() {
    let mut pool = ParentSignaturePool::new();
    for t in 0..(K as u8) {
        assert!(pool.insert(parent(t), attester(1), sig(t)));
    }
    assert_eq!(pool.parent_count(), K);

    // Re-touch the OLDEST parent, once with a new attester and once with a
    // duplicate key. Neither may move it in the window nor grow the window.
    assert!(pool.insert(parent(0), attester(2), sig(0xC0)));
    assert!(!pool.insert(parent(0), attester(1), sig(0xC1)));
    assert_eq!(
        pool.parent_count(),
        K,
        "a re-touch must not push a duplicate window entry"
    );

    assert!(pool.insert(parent(K as u8), attester(1), sig(0xD0)));

    assert_eq!(pool.parent_count(), K);
    assert!(
        !pool.contains_parent(&parent(0)),
        "first-seen FIFO: the re-touched oldest parent is still the one evicted"
    );
    assert!(pool.get(&parent(0), &attester(2)).is_none());
    for t in 1..(K as u8 + 1) {
        assert!(pool.contains_parent(&parent(t)));
    }
}

// REQ-BLS-010 — Decision: the spec's 34 KB @ N=45 / 0.77 MB @ N=1000 budget is
// only true if the stored count is hard-bounded by K*N; an unbounded attester
// axis turns the pool into a gossip amplifier.
#[test]
fn m1_pool_total_stored_bytes_are_bounded_by_k_times_n_times_96() {
    for n in [45usize, 200usize] {
        let mut pool = ParentSignaturePool::new();
        // Far more parents than K, and every attester under every one of them.
        for t in 0..(K as u8 * 3) {
            for i in 0..n as u16 {
                pool.insert(parent(t), attester(i), sig((i % 251) as u8));
            }
        }

        let stored = pool.total_signatures();
        let counted: usize = (0..(K as u8 * 3))
            .filter_map(|t| pool.signatures_for(&parent(t)))
            .map(|m| m.len())
            .sum();

        assert_eq!(counted, stored, "N={n}: the count must be a real traversal");
        assert!(pool.parent_count() <= K, "N={n}: parent window exceeded K");
        assert!(stored <= K * n, "N={n}: {stored} signatures exceed K*N");
        assert_eq!(stored, K * n, "N={n}: the window must be fully populated");
        assert!(
            stored * 96 <= K * n * 96,
            "N={n}: byte budget exceeded ({} B)",
            stored * 96
        );
    }
}

// REQ-BLS-010 — Decision: a per-signature `Vec<u8>` adds a heap allocation and
// 24 bytes of header per attester per parent, which is the allocator churn the
// deleted minute store was built on.
#[test]
fn m1_pool_stores_signatures_inline_as_fixed_96_byte_arrays() {
    let mut pool = ParentSignaturePool::new();
    let (p, a, s) = (parent(1), attester(1), sig(0x7E));
    assert!(pool.insert(p, a, s));

    let stored = pool.get(&p, &a).expect("present");
    assert_eq!(ref_size(stored), 96, "get() must hand back a [u8; 96]");

    let map = pool.signatures_for(&p).expect("present");
    assert_eq!(
        map_value_size(map),
        96,
        "the inner map value must be [u8; 96], not Vec<u8>"
    );
}

// REQ-BLS-012 — Decision: the pool is node-local scratch (C12); if `clear()`
// left residue, an epoch reset would carry stale signatures across the boundary.
#[test]
fn m1_pool_clear_returns_it_to_the_empty_state() {
    let mut pool = ParentSignaturePool::new();
    for t in 0..(K as u8) {
        pool.insert(parent(t), attester(1), sig(t));
    }
    assert_eq!(pool.parent_count(), K);

    pool.clear();

    assert_eq!(pool.parent_count(), 0);
    assert_eq!(pool.total_signatures(), 0);
    assert!(pool.get(&parent(0), &attester(1)).is_none());

    // And the window is genuinely reusable, not merely emptied.
    assert!(pool.insert(parent(0), attester(1), sig(0)));
    assert_eq!(pool.parent_count(), 1);
}

// REQ-BLS-012 — Decision: `Default` and `new()` disagreeing would give a Node
// field a differently-configured pool than every test builds.
#[test]
fn m1_pool_default_is_the_same_empty_pool_as_new() {
    let d = ParentSignaturePool::default();
    assert_eq!(d.parent_count(), 0);
    assert_eq!(d.total_signatures(), 0);
    assert_eq!(ParentSignaturePool::MAX_PARENTS, K);
}
