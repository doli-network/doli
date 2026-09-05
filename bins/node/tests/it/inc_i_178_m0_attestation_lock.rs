//! INC-I-178 M0 — characterization lock on TODAY's attestation encode/validate path.
//!
//! GREEN on current code by construction. Zero production code changes. This is
//! the contract M3 (universe extraction) and M4 (commitment rewrite) must not
//! break silently. Header-hash and build-determinism locks live in the sibling
//! `inc_i_178_m0_block_identity`.
//!
//! OUTPUT CONTRACT
//!
//! F1: `Node::validate_block_for_apply(&self, &Block, height, ValidationMode) -> Result<(), ValidationError>`
//!   Observable outputs:
//!     O1 return discriminant (Ok / Err)
//!     O2 Err message content ("presence_root mismatch" | "bits set beyond producer_count" | other)
//!     O3 `&self` mutation — NONE (read-only over chain_state/producer_set locks; no store writes)
//!     O4 `block` mutation — NONE (`&Block`)
//!   Paths (validation_checks.rs:422-446):
//!     P1 h>=AH && !bitfield.is_empty() && root==BLAKE3(bf) && no stray bits -> falls through
//!     P2 same, root != BLAKE3(bf)                       -> Err(O2="presence_root mismatch")
//!     P3 same, root ok, stray bit beyond producer_count -> Err(O2="bits set beyond producer_count")
//!     P4 bitfield.is_empty()                            -> whole check SKIPPED (legacy arm)
//!   INPUT PARTITIONS:
//!     P1a full bitfield (every producer bit set)
//!     P1b sparse bitfield (every third producer)
//!     P2a presence_root rewritten, body intact
//!     P2b body bit flipped, presence_root intact
//!     P3a stray bit above producer_count with a re-committed presence_root
//!     P4a empty body + presence_root ZERO (the builder's own empty-attester output)
//!     P4b empty body + arbitrary non-zero presence_root (the strip-the-body shape)
//!   MATRIX 4 outputs x 7 partitions: O3/O4 are constant-NONE (asserted once, on P2a);
//!     O1+O2 asserted on all 7. 16/16 reachable cells.
//!
//! F3: `Node::build_block_content(..) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>`
//!   Observable outputs used here: O1 returned header, O3 returned body_bitfield bytes.
//!   Paths: PA base-only universe (producer_list == active_at_height)
//!          PB base+extra universe (mid-epoch-activated producer)
//!          PC empty attester set -> presence_root ZERO, body_bitfield empty
//!   INPUT PARTITIONS:
//!     PAa N=12 all attesting, PAb N=12 sparse, PAc N=45 sparse, PAd N=200 sparse
//!     PBa 1 mid-epoch extra, PBb 2 extras, PBc 5 extras
//!     PCa no attester recorded at all
//!
//! F4: attestation universe ordering, `[base | (active \ base) sorted by pubkey bytes]`
//!   (assembly.rs:408-424). Observable output: O1 index assigned to each attester,
//!   read back through each of the five production decode-site length expressions.
//!   INPUT PARTITIONS: extras dropped from the MIDDLE of the sorted base at counts
//!     1 / 2 / 5, so the extra segment is never a suffix of sorted order.
//!   Locked against the REAL encoder, not against the local helper alone.
//!   Shared harness: `inc_i_178_m0_common`.

use std::collections::HashSet;

use crypto::{Hash, PublicKey};
use doli_core::validation::ValidationMode;

use super::inc_i_178_m0_common::{
    active_at, assemble, build_via_production, err_text, make_node, record_attesters,
    safe_build_height, test_universe, unix_now, N_LARGE, N_REALISTIC, N_SMALL,
};

// REQ-BLS-003 AC-2 — Decision: a failure means the D6 rewrite stopped rejecting a relay
// that strips or rewrites the bitfield, i.e. the commitment no longer binds the body.
#[tokio::test]
async fn req_bls_003_ac2_presence_root_commitment_is_enforced_for_a_full_bitfield() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let height = safe_build_height(&node);
    let slot = node.params.timestamp_to_slot(unix_now());
    let all: Vec<PublicKey> = producers.iter().map(|k| *k.public_key()).collect();
    record_attesters(&mut node, slot, &all);

    let (header, txs, bf) = build_via_production(&mut node, height).await;
    assert!(
        !bf.is_empty(),
        "P1a: a full attester set must emit a body bitfield"
    );
    let active = active_at(&node, height).await;
    let decoded = doli_core::decode_attestation_bitfield_vec(&bf, active.len());
    assert_eq!(
        decoded.len(),
        N_SMALL,
        "P1a: every producer bit must be set"
    );

    let block = assemble(header, txs, bf);

    // O1/O2 on P1a: the well-formed block passes the commitment + stray-bit gate.
    let ok = node
        .validate_block_for_apply(&block, height, ValidationMode::Light)
        .await;
    assert!(
        ok.is_ok(),
        "P1a: builder output must validate; got {}",
        err_text(&ok)
    );

    // O1/O2 on P2a: a rewritten presence_root is rejected by name.
    let mut tampered = block.clone();
    tampered.header.presence_root = crypto::hash::hash(b"relay-rewrote-this");
    let bad = node
        .validate_block_for_apply(&tampered, height, ValidationMode::Light)
        .await;
    assert!(
        err_text(&bad).contains("presence_root mismatch"),
        "P2a: expected presence_root mismatch, got {:?}",
        err_text(&bad)
    );

    // O4: the validator does not mutate the block it was handed.
    assert_eq!(
        tampered.header.presence_root,
        crypto::hash::hash(b"relay-rewrote-this"),
        "O4: validate_block_for_apply must not mutate the block"
    );
    // O3: a rejected block must leave no node state behind.
    let after = node
        .validate_block_for_apply(&block, height, ValidationMode::Light)
        .await;
    assert!(
        after.is_ok(),
        "O3: rejection must not poison node state; got {}",
        err_text(&after)
    );
}

// REQ-BLS-003 AC-2 — Decision: a failure means a sparse (partial-attendance) bitfield stopped
// round-tripping through the commitment, which is the common mainnet shape.
#[tokio::test]
async fn req_bls_003_ac2_presence_root_commitment_is_enforced_for_a_sparse_bitfield() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let height = safe_build_height(&node);
    let slot = node.params.timestamp_to_slot(unix_now());
    let subset: Vec<PublicKey> = producers
        .iter()
        .step_by(3)
        .map(|k| *k.public_key())
        .collect();
    record_attesters(&mut node, slot, &subset);

    let (header, txs, bf) = build_via_production(&mut node, height).await;
    let active = active_at(&node, height).await;
    let decoded = doli_core::decode_attestation_bitfield_vec(&bf, active.len());
    assert_eq!(
        decoded.len(),
        subset.len(),
        "P1b: exactly the seeded attesters must be encoded"
    );
    assert_eq!(
        header.presence_root.as_bytes(),
        crypto::hash::hash(&bf).as_bytes(),
        "AC-2: the builder emits presence_root == BLAKE3(attestation_bitfield)"
    );

    let block = assemble(header, txs, bf);
    let ok = node
        .validate_block_for_apply(&block, height, ValidationMode::Light)
        .await;
    assert!(
        ok.is_ok(),
        "P1b: builder output must validate; got {}",
        err_text(&ok)
    );

    // P2b: flip one body bit, leave presence_root alone.
    let mut tampered = block.clone();
    tampered.attestation_bitfield[0] ^= 0b0000_0010;
    let bad = node
        .validate_block_for_apply(&tampered, height, ValidationMode::Light)
        .await;
    assert!(
        err_text(&bad).contains("presence_root mismatch"),
        "P2b: flipping one body bit must break the commitment, got {:?}",
        err_text(&bad)
    );
}

// REQ-BLS-003 AC-2 — Decision: a failure means the stray-bit denominator moved, which is
// exactly the D5 `active.len()` -> universe-width switch that must ride an activation height.
#[tokio::test]
async fn req_bls_003_ac2_stray_bits_beyond_producer_count_are_rejected_today() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let height = safe_build_height(&node);
    let slot = node.params.timestamp_to_slot(unix_now());
    let all: Vec<PublicKey> = producers.iter().map(|k| *k.public_key()).collect();
    record_attesters(&mut node, slot, &all);

    let (header, txs, bf) = build_via_production(&mut node, height).await;
    let mut block = assemble(header, txs, bf);

    // N_SMALL = 12 -> 2 bytes, bits 12..15 are out of range. Set bit 15 and
    // re-commit so the ONLY surviving defect is the stray bit.
    let last = block.attestation_bitfield.len() - 1;
    block.attestation_bitfield[last] |= 0b1000_0000;
    block.header.presence_root =
        Hash::from_bytes(*crypto::hash::hash(&block.attestation_bitfield).as_bytes());

    let bad = node
        .validate_block_for_apply(&block, height, ValidationMode::Light)
        .await;
    assert!(
        err_text(&bad).contains("bits set beyond producer_count"),
        "P3a: a stray bit above producer_count must be rejected, got {:?}",
        err_text(&bad)
    );
}

// REQ-BLS-003 AC-3 — Decision: a failure means the legacy arm changed; today an EMPTY body
// skips the commitment check entirely, so a stripped bitfield is indistinguishable from
// "the producer had no attesters" — the hole D6 closes with the unconditional check.
#[tokio::test]
async fn req_bls_003_ac3_empty_body_bitfield_bypasses_the_commitment_check_today() {
    let (mut node, _producers, _tmp) = make_node(N_SMALL).await;
    let height = safe_build_height(&node);

    // PCa: no attester recorded -> encoder takes the `attested_pks.is_empty()` arm.
    let (header, txs, bf) = build_via_production(&mut node, height).await;
    assert!(
        bf.is_empty(),
        "PCa: an empty attester set must emit an empty body bitfield"
    );
    assert!(
        header.presence_root.is_zero(),
        "PCa: an empty attester set must emit presence_root == ZERO"
    );

    let block = assemble(header, txs, bf);
    let ok = node
        .validate_block_for_apply(&block, height, ValidationMode::Light)
        .await;
    assert!(
        ok.is_ok(),
        "P4a must be accepted today; got {}",
        err_text(&ok)
    );

    // P4b: an empty body with a garbage presence_root is ACCEPTED by the current
    // guard at validation_checks.rs:423.
    let mut stripped = block.clone();
    stripped.header.presence_root = crypto::hash::hash(b"arbitrary-uncommitted-root");
    let legacy = node
        .validate_block_for_apply(&stripped, height, ValidationMode::Light)
        .await;
    assert!(
        legacy.is_ok(),
        "P4b: today the empty-body guard SKIPS the commitment check, so an \
         uncommitted presence_root is ACCEPTED; if this now fails, D6 landed and \
         this lock must be retired deliberately. Got {}",
        err_text(&legacy)
    );
}

// ============================================================
// REQ-BLS-004 — universe ordering and encode -> decode identity
// ============================================================

/// Shared body for the encode -> decode identity property at a given producer count.
async fn assert_encode_decode_identity_at(n: usize) {
    let (mut node, producers, _tmp) = make_node(n).await;
    let height = safe_build_height(&node);
    let slot = node.params.timestamp_to_slot(unix_now());
    let attesters: Vec<PublicKey> = producers
        .iter()
        .step_by(3)
        .map(|k| *k.public_key())
        .collect();
    record_attesters(&mut node, slot, &attesters);

    let base = node.epoch_state.producer_list.clone();
    let active = active_at(&node, height).await;
    let universe = test_universe(&base, &active);
    assert_eq!(
        universe.len(),
        n,
        "PA: with no mid-epoch activation the universe is the base list"
    );

    let (header, txs, bf) = build_via_production(&mut node, height).await;
    let expected: HashSet<usize> = attesters
        .iter()
        .map(|pk| universe.iter().position(|u| u == pk).unwrap())
        .collect();

    // The five production decode sites reduce to two length expressions: the
    // post_commit site uses base_len + extra.len(); the three rewards sites and
    // the RPC site use base_len. With no extras both equal `n`.
    let post_commit_len = base.len() + (universe.len() - base.len());
    let rewards_len = base.len();
    for (label, len) in [
        ("post_commit.rs:61", post_commit_len),
        ("rewards.rs:139", rewards_len),
        ("rewards.rs:814", rewards_len),
        ("rewards.rs:1016", rewards_len),
        ("schedule.rs:306", rewards_len),
    ] {
        let decoded: HashSet<usize> = doli_core::decode_attestation_bitfield_vec(&bf, len)
            .into_iter()
            .collect();
        assert_eq!(
            decoded, expected,
            "AC-3 N={}: decode at {} must recover exactly the attested set",
            n, label
        );
    }

    let block = assemble(header, txs, bf);
    let ok = node
        .validate_block_for_apply(&block, height, ValidationMode::Light)
        .await;
    assert!(
        ok.is_ok(),
        "AC-3 N={}: the round-tripped block must still validate; got {}",
        n,
        err_text(&ok)
    );
}

// REQ-BLS-004 AC-3 — Decision: a failure means encode and decode disagree at a realistic
// producer count, which is the v6.17.1 death-spiral shape (filtered producers never re-enter).
#[tokio::test]
async fn req_bls_004_ac3_encode_decode_identity_at_45_producers() {
    assert_encode_decode_identity_at(N_REALISTIC).await;
}

// REQ-BLS-004 AC-3 — Decision: a failure at 200 producers means the round trip is
// N-sensitive, so growing the set would silently corrupt reward attribution.
#[tokio::test]
async fn req_bls_004_ac3_encode_decode_identity_at_200_producers() {
    assert_encode_decode_identity_at(N_LARGE).await;
}

// REQ-BLS-004 AC-1 — Decision: a failure means a mid-epoch-activated attester lost its bit
// index, which is the exact index-parity surface the Full Bitfield Decode pillar guards.
#[tokio::test]
async fn req_bls_004_ac1_mid_epoch_activated_attester_keeps_its_universe_index() {
    let (mut node, _producers, _tmp) = make_node(N_REALISTIC).await;
    let height = safe_build_height(&node);

    // PBa: model a mid-epoch activation by dropping a MIDDLE producer from the
    // frozen list, so `[base | extra]` differs from plain sorted order.
    let dropped_at = node.epoch_state.producer_list.len() / 2;
    let late = node.epoch_state.producer_list.remove(dropped_at);
    let base = node.epoch_state.producer_list.clone();
    let active = active_at(&node, height).await;
    assert_eq!(active.len(), N_REALISTIC, "setup: `late` is still active");
    assert_eq!(base.len(), N_REALISTIC - 1, "setup: base excludes `late`");

    let universe = test_universe(&base, &active);
    assert_eq!(
        universe.len(),
        N_REALISTIC,
        "PB: universe = base + one mid-epoch extra"
    );
    assert_eq!(
        &universe[..base.len()],
        &base[..],
        "REQ-BLS-004: universe[..base_len] MUST equal the frozen base list — this \
         prefix property is what keeps the base-only reward decoders correct"
    );
    assert_eq!(
        universe[base.len()],
        late,
        "PB: the extra producer occupies index base_len"
    );

    let slot = node.params.timestamp_to_slot(unix_now());
    let base_attester = base[3];
    record_attesters(&mut node, slot, &[base_attester, late]);

    let (header, txs, bf) = build_via_production(&mut node, height).await;
    let base_idx = universe.iter().position(|u| *u == base_attester).unwrap();
    let late_idx = universe.iter().position(|u| *u == late).unwrap();
    assert_eq!(
        late_idx,
        base.len(),
        "the REAL encoder must place the extra at base_len, not in sorted position"
    );

    // post_commit decodes the FULL universe: both attesters survive.
    let full: HashSet<usize> = doli_core::decode_attestation_bitfield_vec(&bf, universe.len())
        .into_iter()
        .collect();
    assert_eq!(
        full,
        HashSet::from([base_idx, late_idx]),
        "post_commit.rs:61 (base_len + extra) must recover BOTH attesters"
    );

    // The three rewards sites and the RPC site decode base-only: the extra is
    // truncated and every base index keeps its meaning. That asymmetry is TODAY's
    // behaviour and the reason D5 keeps `universe[..base_len]`.
    let base_only: HashSet<usize> = doli_core::decode_attestation_bitfield_vec(&bf, base.len())
        .into_iter()
        .collect();
    assert_eq!(
        base_only,
        HashSet::from([base_idx]),
        "rewards.rs:139/814/1016 and schedule.rs:306 (base_len) truncate the extra \
         while preserving every base index"
    );

    let block = assemble(header, txs, bf);
    let ok = node
        .validate_block_for_apply(&block, height, ValidationMode::Light)
        .await;
    assert!(
        ok.is_ok(),
        "PB: the stray-bit check uses active.len(), which equals the universe width \
         here, so a mid-epoch attester must validate today; got {}",
        err_text(&ok)
    );
}

// REQ-BLS-004 — Decision: a failure means the M3 extraction changed the encoder's ordering,
// which is precisely how a fifth private copy re-creates the v6.17.1 spiral.
#[tokio::test]
async fn req_bls_004_universe_prefix_property_matches_the_real_encoder() {
    for drop_count in [1usize, 2, 5] {
        let (mut node, _producers, _tmp) = make_node(N_REALISTIC).await;
        let height = safe_build_height(&node);

        // Drop `drop_count` producers spread through the list so the extras are
        // never a suffix of the sorted order.
        let mut late: Vec<PublicKey> = Vec::new();
        for k in 0..drop_count {
            let idx = (node.epoch_state.producer_list.len() / (drop_count + 1)) * (k + 1) - k;
            late.push(node.epoch_state.producer_list.remove(idx));
        }
        let base = node.epoch_state.producer_list.clone();
        let active = active_at(&node, height).await;
        let universe = test_universe(&base, &active);

        assert_eq!(
            &universe[..base.len()],
            &base[..],
            "drop_count={}: universe[..base_len] must equal base",
            drop_count
        );
        let tail = &universe[base.len()..];
        assert_eq!(tail.len(), drop_count, "drop_count={}", drop_count);
        for w in tail.windows(2) {
            assert!(
                w[0].as_bytes() < w[1].as_bytes(),
                "drop_count={}: the extra segment must be sorted by pubkey bytes",
                drop_count
            );
        }
        let base_set: HashSet<&PublicKey> = base.iter().collect();
        for pk in tail {
            assert!(
                !base_set.contains(pk),
                "drop_count={}: extras must be disjoint from base",
                drop_count
            );
        }

        // Non-vacuity: the REAL encoder must agree with this local universe.
        let slot = node.params.timestamp_to_slot(unix_now());
        let attesters: Vec<PublicKey> = late
            .iter()
            .copied()
            .chain(std::iter::once(base[0]))
            .collect();
        record_attesters(&mut node, slot, &attesters);
        let (_h, _tx, bf) = build_via_production(&mut node, height).await;
        let decoded: HashSet<usize> =
            doli_core::decode_attestation_bitfield_vec(&bf, universe.len())
                .into_iter()
                .collect();
        let expected: HashSet<usize> = attesters
            .iter()
            .map(|pk| universe.iter().position(|u| u == pk).unwrap())
            .collect();
        assert_eq!(
            decoded, expected,
            "drop_count={}: the production encoder must assign exactly the indices \
             this universe ordering predicts",
            drop_count
        );
    }
}
