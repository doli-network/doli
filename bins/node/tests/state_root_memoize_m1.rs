//! State-Root Lazy Tier-0 — M1 memoize-on-compute behavior (RUN 459).
//!
//! Milestone M1 is behavior-ADDITIVE. It adds cache-on-compute WRITE-BACK to the
//! ONE live `GetStateRoot` handler (`validation_checks.rs:1093-1122`, serves both
//! the diagnostic RPC and snap-sync quorum votes) and makes the memo read
//! best_hash-keyed so a stale tuple from a prior height is never served as
//! current. It does NOT delete the eager per-block compute and does NOT touch the
//! `[STATE_FP] sr=` field — those are M2.
//!
//! Spec: `specs/state-root-commitment-architecture.md`
//!   "Proposed Architecture (Definite + Recommended — Tier 0)":
//!       memo hit (best_hash match) → O(1)
//!       memo miss → compute fresh (existing fallback) → WRITE BACK to memo
//!   "Migration Path" step 1 (write-back) + step 5 (these tests).
//!
//! ── TESTABILITY CONTRACT (what M1 must expose) ───────────────────────────────
//! The live handler is channel-coupled: `handle_sync_request` writes its result
//! into a libp2p `ResponseChannel`, which has no public/test constructor, so the
//! memo behavior cannot be observed by driving the handler directly. The
//! test-gate placement rule also forbids inline `#[cfg(test)]` in the impl file.
//! M1 MUST therefore expose the compute+memoize step as a pure, testable seam:
//!
//!     impl Node {
//!         /// Serve the current state root, memoizing on cold/stale-memo compute.
//!         pub async fn serve_state_root(&self) -> network::protocols::SyncResponse;
//!     }
//!
//! and the `SyncRequest::GetStateRoot` arm becomes `self.serve_state_root().await`.
//! Until this seam exists, this file does not compile — that is the intended RED
//! state; it goes GREEN when the developer lands M1. (The golden-identity VALUE
//! lock lives in `crates/storage/tests/state_root_golden_identity_test.rs` and
//! passes today independently of this seam.)
//
// OUTPUT CONTRACT: fn serve_state_root(&self) -> network::protocols::SyncResponse
// O1: return — SyncResponse::StateRoot { block_hash, block_height, state_root }
//        (Error variant only on compute failure — unreachable for a valid Node)
// O2: side-effect — write to self.cached_state_root (Arc<RwLock<Option<(Hash,Hash,u64)>>>),
//        tuple = (state_root, best_hash, best_height)
// PATHS:
//   P1 memo HIT   (cache Some, cached.hash == best_hash):
//        O1 = the cached tuple verbatim; O2 = unchanged (O(1), no recompute)
//   P2 memo COLD  (cache None):
//        O1 = StateRoot{ best_hash, best_height, legacy_root };
//        O2 = write Some((legacy_root, best_hash, best_height))
//   P3 memo STALE (cache Some, cached.hash != best_hash):
//        O1 = StateRoot{ best_hash, best_height, legacy_root } (recompute — NOT stale);
//        O2 = overwrite with Some((legacy_root, best_hash, best_height))
// MATRIX:
//        O1×P1 = memo-hit returns cached      → test_memo_hit_returns_cached_without_recompute
//        O1×P2 = cold returns legacy value    → test_cold_memo_serves_legacy_value
//        O2×P2 = cold writes back             → test_cold_memo_writes_back_to_cache
//        O1×P2 = repeat vote is O(1) memo hit → test_repeat_quorum_vote_hits_populated_memo
//        O1×P3 = stale NOT served, recomputes → test_stale_memo_not_served_recomputes
//        O2×P3 = stale overwritten            → test_stale_memo_overwritten_with_current
//        canary smoke (REQ-SROOT-007)         → test_serve_path_canary_smoke_no_panic
//
// INPUT PARTITIONS (on the memo-cache state at a fixed current tip = genesis):
//   C-COLD:  cached_state_root == None                                   → P2
//   C-HIT:   cached_state_root == Some((r, best_hash, best_height))       → P1
//            sub-classes: r == legacy_root, and r == SENTINEL (≠ legacy).
//            The SENTINEL sub-class proves the hit path returns the cached
//            bytes WITHOUT recomputing (a recompute would overwrite SENTINEL
//            with legacy_root and fail the assertion).
//   C-STALE: cached_state_root == Some((SENTINEL, wrong_hash, wrong_height)),
//            wrong_hash != best_hash                                      → P3
//   Current-tip is held constant (genesis) across all classes; the partition
//   variable is exclusively the memo tuple, which is the only input that selects
//   between P1/P2/P3.

use crypto::{Hash, KeyPair};
use doli_node::node::Node;
use network::protocols::SyncResponse;
use tempfile::TempDir;

/// A recognizable non-real root used to detect whether the memo-hit path returns
/// the cached bytes (SENTINEL) or silently recomputes (legacy root).
const SENTINEL_ROOT: Hash = Hash::from_bytes([0xAB; 32]);

async fn make_node(n_producers: usize) -> (Node, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let node = Node::new_for_test(temp.path().to_path_buf(), producers)
        .await
        .expect("Node::new_for_test failed");
    (node, temp)
}

/// Current tip (best_hash, best_height) under a read lock.
async fn current_tip(node: &Node) -> (Hash, u64) {
    let cs = node.chain_state.read().await;
    (cs.best_hash, cs.best_height)
}

/// The legacy root for the node's CURRENT state — the value M1 must serve
/// byte-identically (REQ-SROOT-001/002). Guards are dropped before return.
async fn legacy_root(node: &Node) -> Hash {
    let cs = node.chain_state.read().await;
    let utxo = node.utxo_set.read().await;
    let ps = node.producer_set.read().await;
    storage::compute_state_root(&cs, &utxo, &ps).expect("compute_state_root")
}

/// Destructure a `SyncResponse::StateRoot`, failing loudly on any other variant.
fn expect_state_root(resp: SyncResponse) -> (Hash, u64, Hash) {
    match resp {
        SyncResponse::StateRoot {
            block_hash,
            block_height,
            state_root,
        } => (block_hash, block_height, state_root),
        other => panic!("expected SyncResponse::StateRoot, got {:?}", other),
    }
}

/// P2 / O1 — REQ-SROOT-001/002 (Must).
/// On a COLD memo, the handler serves the legacy root for the current tip,
/// byte-identical, tagged with the current best_hash/best_height.
#[tokio::test]
async fn test_cold_memo_serves_legacy_value() {
    let (node, _tmp) = make_node(5).await;

    // Precondition: cold memo.
    assert!(
        node.cached_state_root.read().await.is_none(),
        "precondition: new_for_test must start with an empty (cold) memo"
    );

    let (best_hash, best_height) = current_tip(&node).await;
    let expected = legacy_root(&node).await;

    let (hash, height, root) = expect_state_root(node.serve_state_root().await);

    assert_eq!(root, expected, "cold-memo root must equal legacy compute");
    assert_eq!(hash, best_hash, "served block_hash must be the current tip");
    assert_eq!(
        height, best_height,
        "served block_height must be current tip"
    );
}

/// P2 / O2 — memo WRITE-BACK (core M1 behavior). Fails until M1 lands.
/// After a COLD-memo serve, `cached_state_root` is populated with the freshly
/// computed tuple keyed on the current best_hash.
#[tokio::test]
async fn test_cold_memo_writes_back_to_cache() {
    let (node, _tmp) = make_node(5).await;
    assert!(node.cached_state_root.read().await.is_none());

    let (best_hash, best_height) = current_tip(&node).await;
    let expected = legacy_root(&node).await;

    let _ = node.serve_state_root().await;

    let cached = *node.cached_state_root.read().await;
    assert_eq!(
        cached,
        Some((expected, best_hash, best_height)),
        "cold-memo serve MUST write back (root, best_hash, best_height)"
    );
}

/// P1 / O1 — memo HIT is O(1) and does NOT recompute.
/// Pre-seed the memo at the CURRENT best_hash with a SENTINEL root. A correct
/// memo-hit returns SENTINEL verbatim; a silent recompute would return the
/// legacy root instead and fail here.
#[tokio::test]
async fn test_memo_hit_returns_cached_without_recompute() {
    let (node, _tmp) = make_node(5).await;
    let (best_hash, best_height) = current_tip(&node).await;

    *node.cached_state_root.write().await = Some((SENTINEL_ROOT, best_hash, best_height));

    let (hash, height, root) = expect_state_root(node.serve_state_root().await);

    assert_eq!(
        root, SENTINEL_ROOT,
        "memo hit at current best_hash must return the cached root verbatim (no recompute)"
    );
    assert_eq!(hash, best_hash);
    assert_eq!(height, best_height);

    // The memo must be unchanged by a hit.
    assert_eq!(
        *node.cached_state_root.read().await,
        Some((SENTINEL_ROOT, best_hash, best_height)),
        "a memo hit must not mutate the cache"
    );
}

/// P2 → P1 — quorum-vote serve path (vote-serve requirement).
/// The first vote request on a cold memo computes fresh AND populates the memo,
/// so repeated quorum-vote requests at the SAME height are O(1) memo hits rather
/// than repeated full-state scans. Verified by proving the second serve returns
/// the value written by the first (post-seeded SENTINEL is returned unchanged).
#[tokio::test]
async fn test_repeat_quorum_vote_hits_populated_memo() {
    let (node, _tmp) = make_node(5).await;
    let (best_hash, best_height) = current_tip(&node).await;
    let expected = legacy_root(&node).await;

    // Vote #1: cold → computes and memoizes.
    let (_, _, root1) = expect_state_root(node.serve_state_root().await);
    assert_eq!(root1, expected, "first vote serves legacy root");
    assert_eq!(
        *node.cached_state_root.read().await,
        Some((expected, best_hash, best_height)),
        "first vote must populate the memo for subsequent votes"
    );

    // Prove vote #2 reads the memo (not a fresh scan): overwrite the cached root
    // with SENTINEL at the same key; a memo-backed serve returns SENTINEL.
    *node.cached_state_root.write().await = Some((SENTINEL_ROOT, best_hash, best_height));
    let (_, _, root2) = expect_state_root(node.serve_state_root().await);
    assert_eq!(
        root2, SENTINEL_ROOT,
        "repeated quorum-vote request must be served from the memo, not re-scanned"
    );
}

/// P3 / O1 — STALE memo must NOT be served as current.
/// Seed a memo whose block_hash does NOT match the current best_hash (as if left
/// over from a prior height). M1's best_hash-keyed read must ignore it and
/// recompute the current legacy root. Under the pre-M1 code (returns any Some
/// unconditionally) this FAILS — which is the point.
#[tokio::test]
async fn test_stale_memo_not_served_recomputes() {
    let (node, _tmp) = make_node(5).await;
    let (best_hash, best_height) = current_tip(&node).await;
    let expected = legacy_root(&node).await;

    let wrong_hash = Hash::from_bytes([0xEE; 32]);
    assert_ne!(wrong_hash, best_hash, "test setup: stale hash must differ");
    *node.cached_state_root.write().await =
        Some((SENTINEL_ROOT, wrong_hash, best_height.wrapping_add(7)));

    let (hash, height, root) = expect_state_root(node.serve_state_root().await);

    assert_ne!(root, SENTINEL_ROOT, "must not serve the stale cached root");
    assert_eq!(root, expected, "stale memo must trigger a fresh recompute");
    assert_eq!(
        hash, best_hash,
        "served hash must be the CURRENT tip, not stale"
    );
    assert_eq!(height, best_height, "served height must be the CURRENT tip");
}

/// P3 / O2 — after a stale-memo serve, the memo is overwritten with the current
/// tuple (so the next request is a valid hit, not another stale miss).
#[tokio::test]
async fn test_stale_memo_overwritten_with_current() {
    let (node, _tmp) = make_node(5).await;
    let (best_hash, best_height) = current_tip(&node).await;
    let expected = legacy_root(&node).await;

    let wrong_hash = Hash::from_bytes([0xEE; 32]);
    *node.cached_state_root.write().await =
        Some((SENTINEL_ROOT, wrong_hash, best_height.wrapping_add(7)));

    let _ = node.serve_state_root().await;

    assert_eq!(
        *node.cached_state_root.read().await,
        Some((expected, best_hash, best_height)),
        "stale-memo serve MUST overwrite the cache with the current tuple"
    );
}

/// REQ-SROOT-007 (F-D0-2 canary) — additive-logging smoke.
/// The canary/logging added by M1 must not break the serve path: repeated serves
/// return a well-formed StateRoot for the current tip without panicking. We do
/// NOT assert on log strings (fragile) — a passing serve is sufficient evidence
/// the additive canary path is inert with respect to behavior.
#[tokio::test]
async fn test_serve_path_canary_smoke_no_panic() {
    let (node, _tmp) = make_node(3).await;
    let (best_hash, best_height) = current_tip(&node).await;

    for _ in 0..3 {
        let (hash, height, _root) = expect_state_root(node.serve_state_root().await);
        assert_eq!(hash, best_hash);
        assert_eq!(height, best_height);
    }
}
