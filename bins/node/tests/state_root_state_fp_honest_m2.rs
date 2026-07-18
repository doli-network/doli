//! State-Root Lazy Tier-0 — M2 honest `[STATE_FP] sr=` regression (RUN 460).
//!
//! M2 DELETES the eager per-block state-root compute
//! (`apply_block/state_update.rs` Phase 2/3). The root becomes fully lazy —
//! computed only on `serve_state_root()` (memoized since M1). This deletion
//! introduces ONE latent bug that this file exists to forbid:
//!
//!   REQ-SROOT-008 (Must) — the per-block `[STATE_FP]` log line's `sr=` field
//!   must NEVER print a state root from a PREVIOUS height as if it were the
//!   current height's root.
//!
//! Under lazy compute, right after a new block N is applied, the memo
//! (`self.cached_state_root`, an `Option<(Hash /*root*/, Hash /*best_hash*/,
//! u64 /*best_height*/)>`) may still hold height N-1's tuple (or `None`,
//! if nothing has served the root yet). The current reader at
//! `apply_block/mod.rs:427-435` does:
//!
//!     self.cached_state_root.read().await
//!         .map(|(sr, _, _)| /* 16-hex of sr */)   // <-- ignores best_hash!
//!         .unwrap_or_else(|| "none".to_string())
//!
//! With the eager compute deleted, that `.map(|(sr, _, _)| ...)` prints
//! whatever root is memoized — which, for the just-applied block, is STALE
//! (the prior height's root) — mislabeled as `h=N`. This is a silent
//! divergence-diagnosis poison: operators grep `[STATE_FP]` across nodes to
//! compare roots at a height; a stale `sr=` makes matching nodes look
//! divergent (or divergent nodes look matching).
//!
//! ── HONEST FIX (what M2 must land) ───────────────────────────────────────────
//! Key the printed `sr=` on the JUST-APPLIED block's hash. Only print the hex
//! root when the memoized tuple's stored `best_hash == block_hash` (the block
//! being logged); otherwise print `"none"`. In scope at the reader are both
//! `block_hash` (the applied block) and `height`.
//!
//! ── TESTABILITY CONTRACT (what M2 must expose) ───────────────────────────────
//! The honest-`sr=` semantic is a PURE function of (memo tuple, current block
//! hash) and MUST be unit-testable without scraping logs or driving a full
//! block-apply. M2 MUST extract it as a pure, publicly-reachable free function:
//!
//!     // in bins/node/src/node/apply_block/  (re-exported from `doli_node::node`)
//!     pub fn state_fp_sr_field(
//!         memo: Option<(crypto::Hash, crypto::Hash, u64)>,  // (root, best_hash, best_height)
//!         current_block_hash: crypto::Hash,
//!     ) -> String;
//!
//! and the `[STATE_FP]` reader becomes:
//!
//!     let state_root =
//!         state_fp_sr_field(*self.cached_state_root.read().await, block_hash);
//!
//! Until `doli_node::node::state_fp_sr_field` exists, this file does not compile
//! — that is the intended RED state; it goes GREEN when the developer lands the
//! honest M2 fix. (The test-gate placement rule forbids inline `#[cfg(test)]` in
//! the impl file, and the reader is buried inside async `apply_block`, so a pure
//! seam is the only log-free way to lock REQ-SROOT-008.)
//
// OUTPUT CONTRACT: fn state_fp_sr_field(memo: Option<(Hash,Hash,u64)>, cur: Hash) -> String
// O1: return — the String printed as the `sr=` field.
// PATHS (partitioned on the memo state relative to the block being logged):
//   P1 memo NONE                         → "none"
//   P2 memo Some, stored best_hash == cur → 16-hex prefix of root.to_hex()
//   P3 memo Some, stored best_hash != cur → "none"   (STALE prior-height root)
// MATRIX:
//   O1×P1 → test_sr_field_none_memo_prints_none
//   O1×P2 → test_sr_field_matching_hash_prints_hex_prefix
//   O1×P3 → test_sr_field_stale_hash_prints_none   (FAILS vs naive .map(|(sr,_,_)|))
// INPUT PARTITIONS (memo tuple vs the block hash being logged):
//   C-NONE:  memo == None                                          → P1
//   C-MATCH: memo == Some((root, cur, _))                          → P2
//   C-STALE: memo == Some((root, other, _)),  other != cur         → P3
//   The `best_height` slot of the tuple is intentionally varied (and set to a
//   value != the logged height in C-STALE) to prove the decision keys on
//   best_HASH, never on the height field.

use crypto::Hash;
use doli_node::node::state_fp_sr_field;

/// Mirror the production 16-char truncation of `Hash::to_hex()` exactly
/// (`apply_block/mod.rs`: `let s = sr.to_hex(); s[..s.len().min(16)]`).
fn expected_hex_prefix(root: Hash) -> String {
    let s = root.to_hex();
    s[..s.len().min(16)].to_string()
}

/// O1×P1 — REQ-SROOT-008 (Must). A cold memo prints `"none"`, never a panic or
/// an empty string. This is the common post-restart / pre-first-serve state.
#[test]
fn test_sr_field_none_memo_prints_none() {
    let block_hash = Hash::from_bytes([0x11; 32]);
    assert_eq!(
        state_fp_sr_field(None, block_hash),
        "none",
        "a cold (None) memo must print \"none\" for the sr= field"
    );
}

/// O1×P2 — REQ-SROOT-008 (Must). When the memo's stored best_hash matches the
/// block being logged, print the 16-hex prefix of that root. This is the
/// only case in which a real root value is legitimately current.
#[test]
fn test_sr_field_matching_hash_prints_hex_prefix() {
    let block_hash = Hash::from_bytes([0x22; 32]);
    let root = Hash::from_bytes([0xCD; 32]);
    // best_height slot deliberately arbitrary — the match is on best_hash only.
    let memo = Some((root, block_hash, 987_654_u64));

    assert_eq!(
        state_fp_sr_field(memo, block_hash),
        expected_hex_prefix(root),
        "when memo.best_hash == block_hash, sr= must be the 16-hex root prefix"
    );
}

/// O1×P3 — REQ-SROOT-008 (Must) — THE regression this milestone protects.
/// The memo holds a PRIOR height's tuple (its stored best_hash differs from the
/// just-applied block_hash). The honest reader must print `"none"` — it must NOT
/// emit the stale prior-height root mislabeled as the current height.
///
/// This assertion FAILS against the naive deletion (which keeps
/// `.map(|(sr, _, _)| 16-hex(sr))` and would print the stale root's prefix) and
/// PASSES only with the honest best_hash-keyed fix.
#[test]
fn test_sr_field_stale_hash_prints_none() {
    let block_hash = Hash::from_bytes([0x33; 32]); // block N being logged
    let stale_root = Hash::from_bytes([0xAB; 32]); // root computed for height N-1
    let prior_hash = Hash::from_bytes([0xEE; 32]); // N-1's best_hash (!= block_hash)
    assert_ne!(prior_hash, block_hash, "test setup: stale hash must differ");

    // best_height slot set to a WRONG height too, proving the decision does not
    // key on the height field — only on best_hash.
    let memo = Some((stale_root, prior_hash, 41_u64));

    let printed = state_fp_sr_field(memo, block_hash);
    assert_ne!(
        printed,
        expected_hex_prefix(stale_root),
        "must NOT print the stale prior-height root as the current height's sr="
    );
    assert_eq!(
        printed, "none",
        "a stale (best_hash != block_hash) memo must print \"none\", not the prior root"
    );
}

/// O1×P3 boundary — a stale memo whose stored best_height happens to EQUAL the
/// logged height (but whose best_hash still differs) must STILL print "none".
/// Guards against a fix that mistakenly keys on height instead of hash.
#[test]
fn test_sr_field_stale_hash_but_equal_height_still_none() {
    let block_hash = Hash::from_bytes([0x44; 32]);
    let prior_hash = Hash::from_bytes([0x45; 32]);
    let stale_root = Hash::from_bytes([0x46; 32]);
    let logged_height = 5000_u64;

    // Same height, DIFFERENT hash (e.g. a competing block at the same height).
    let memo = Some((stale_root, prior_hash, logged_height));

    assert_eq!(
        state_fp_sr_field(memo, block_hash),
        "none",
        "matching height with a mismatched best_hash must still print \"none\""
    );
}
