//! INC-I-178 M4 — D6: what the builder commits to at and above the activation height.
//!
//! Requirements: **REQ-BLS-001**, **REQ-BLS-003**, **REQ-BLS-010** (all Must).
//! TDD RED, EXPECTED: `doli_node::node::attestation::commit` does not exist at the M4
//! branch point, so this module does not compile against HEAD.
//!
//! SCOPE NOTE. The aggregate VERIFIER is M5 (D7). Nothing here drives a production
//! verify path: [`req_bls_003_m4_post_ah_the_aggregate_verifies_over_exactly_the_set_bits`]
//! calls `bls_verify_aggregate` inside the TEST, as an independent check that the
//! builder aggregated the right signatures. Whether a peer's stripped or forged
//! aggregate is REJECTED on the apply path is M5's contract, not this module's.
//!
//! OUTPUT CONTRACT — ENUMERATION OF OBSERVABLE OUTPUTS.
//!
//!   F1: `build_attestation_commitment(&NetworkParams, u64, &[PublicKey],
//!        &[PublicKey], &ParentSignaturePool, &Hash) -> AttestationCommitment`  (PURE)
//!       O1 `.bitfield`       — one bit per universe index
//!       O2 `.aggregate`      — 96 bytes, or empty
//!       O3 `.presence_root`  — the header commitment
//!       O4 mutable params — NONE; the pool is borrowed shared. Asserted negatively
//!          by re-reading `pool.total_signatures()` after every call.
//!       O5 receiver/self — NONE (free function)
//!       O6 persistent store writes / statics / channels — NONE
//!       PATHS: P-pre (`height < AH`) and P-post (`height >= AH`).
//!       INPUT PARTITIONS on P-post:
//!         I1 a strict, non-empty subset of the universe has pooled signatures
//!         I2 ZERO pooled signatures for this parent          (REQ-BLS-010 liveness)
//!         I3 a pooled signature from a key OUTSIDE the universe
//!         I4 a pooled signature for a DIFFERENT parent hash
//!         I5 the full universe attested
//!
//!   F2: `Node::build_block_content(..)` — the REAL builder, driven end to end
//!       O7 `header.presence_root`, O8 the body bitfield it returns
//!       PATHS: P-pre and P-post, selected by the node's own gate copy.
//!
//!   F3: `Node::apply_block(Block, ValidationMode)` at an epoch boundary
//!       O9 `self.parent_sig_pool` — must be emptied beside `minute_tracker.reset()`
//!       PATHS: P-boundary and P-non-boundary.
//!
//!   MATRIX 9 outputs x 2 paths x 5 partitions: O1/O2/O3 claimed per partition below;
//!   O4/O5/O6 structural; O7/O8 on both paths; O9 on both boundary paths.

use std::collections::HashSet;

use crypto::{
    bls_sign, bls_verify_aggregate, BlsKeyPair, BlsPublicKey, BlsSignature, Hash, KeyPair,
    PublicKey,
};
use doli_core::attestation::{bls_attest_msg, ParentSignaturePool};
use doli_core::network_params::NetworkParams;
use doli_core::validation::ValidationMode;
use doli_core::{
    attestation_universe, decode_attestation_bitfield_vec, presence_commitment, Network,
};
use doli_node::node::attestation::commit::build_attestation_commitment;
use network::PeerId;

use crate::inc_i_178_m0_common::{
    active_at, assemble, build_via_production, dual, make_node, record_attesters, register_bls,
    safe_build_height, unix_now, N_SMALL,
};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn gated(ah: u64) -> NetworkParams {
    let mut p = NetworkParams::defaults(Network::Devnet);
    p.inc_i_178_attestation_bls_activation_height = ah;
    p
}

/// `(pre, post)` read back OUT of the params, never spelled as a literal
/// (rule `18779b1e` / INV-GOV-001).
fn sides(p: &NetworkParams) -> (u64, u64) {
    let ah = p.inc_i_178_attestation_bls_activation_height;
    assert!(ah > 0, "the probe gate must leave room for a pre-AH height");
    (ah - 1, ah)
}

fn pk(seed: u16) -> PublicKey {
    let mut b = [0u8; 32];
    let m = seed.wrapping_mul(40_503);
    b[0] = (m >> 8) as u8;
    b[1] = m as u8;
    b[2] = (seed >> 8) as u8;
    b[3] = seed as u8;
    PublicKey::from_bytes(b)
}

/// One universe member with a real BLS keypair.
struct Member {
    pk: PublicKey,
    bls: BlsKeyPair,
}

fn members(n: u16) -> Vec<Member> {
    (0..n)
        .map(|s| Member {
            pk: pk(s),
            bls: BlsKeyPair::generate(),
        })
        .collect()
}

fn sign(m: &Member, parent: &Hash) -> BlsSignature {
    bls_sign(&bls_attest_msg(parent), m.bls.secret_key()).expect("BLS signing must succeed")
}

fn pool_with(parent: &Hash, signers: &[&Member]) -> ParentSignaturePool {
    let mut pool = ParentSignaturePool::new();
    for m in signers {
        pool.insert(*parent, m.pk, *sign(m, parent).as_bytes());
    }
    pool
}

// ===========================================================================
// F1 x P-post — O1: the bitfield is the pool, projected onto the universe.
// ===========================================================================

/// REQ-BLS-001 — Decision: a failure means bit *i* does not mean "universe[i] signed
/// THIS block's parent". That is the whole point of the redesign: the retired scheme
/// keyed signatures by `(pubkey, minute)`, so up to six different messages shared one
/// key and no aggregate over them could ever verify. If the projection is wrong, M5's
/// verifier gathers keys for bits whose owners never signed, and every honest block is
/// rejected network-wide the moment the gate opens.
#[test]
fn req_bls_001_m4_post_ah_a_bit_is_set_exactly_for_a_pooled_parent_signature() {
    let p = gated(4_242);
    let (_pre, post) = sides(&p);
    let universe_members = members(20);
    let universe: Vec<PublicKey> = universe_members.iter().map(|m| m.pk).collect();
    let parent = crypto::hash::hash(b"m4-parent-i1");

    // I1: a strict, non-contiguous, non-empty subset.
    let signer_idx = [0usize, 3, 7, 8, 19];
    let signers: Vec<&Member> = signer_idx.iter().map(|i| &universe_members[*i]).collect();
    let pool = pool_with(&parent, &signers);

    let c = build_attestation_commitment(&p, post, &universe, &[], &pool, &parent);

    assert_eq!(
        c.bitfield.len(),
        universe.len().div_ceil(8),
        "O1: the bitfield is sized by the universe, not by the signer count"
    );
    let set: Vec<usize> = decode_attestation_bitfield_vec(&c.bitfield, universe.len());
    assert_eq!(
        set,
        signer_idx.to_vec(),
        "O1: exactly the pooled signers, in index order"
    );

    // The complement, asserted explicitly: "exactly" must exclude, not only include.
    let set_lookup: HashSet<usize> = set.iter().copied().collect();
    for i in 0..universe.len() {
        assert_eq!(
            set_lookup.contains(&i),
            signer_idx.contains(&i),
            "O1: universe[{i}] bit disagrees with its pool membership"
        );
    }

    // O4: the pool the builder read is untouched.
    assert_eq!(
        pool.total_signatures(),
        signer_idx.len(),
        "O4: pool not mutated"
    );
    assert_eq!(pool.parent_count(), 1, "O4: no parent added or evicted");
}

/// REQ-BLS-001 — Decision: a failure means a signature over a DIFFERENT parent can set
/// a bit on this block. The pool holds up to `MAX_PARENTS = 8` parents at once (a reorg
/// window), so this is not hypothetical: during any fork the builder is holding
/// signatures for sibling blocks. Crediting them produces an aggregate over two
/// different messages, which cannot verify under any key set.
#[test]
fn req_bls_001_m4_post_ah_a_signature_for_another_parent_sets_no_bit() {
    let p = gated(4_242);
    let (_pre, post) = sides(&p);
    let universe_members = members(20);
    let universe: Vec<PublicKey> = universe_members.iter().map(|m| m.pk).collect();

    let parent = crypto::hash::hash(b"m4-parent-a");
    let sibling = crypto::hash::hash(b"m4-parent-b");
    assert_ne!(parent, sibling, "the two parents must be distinct");

    let mut pool = pool_with(&parent, &[&universe_members[2]]);
    // I4: the same member signs the SIBLING; only the parent signature may count.
    for m in &universe_members {
        pool.insert(sibling, m.pk, *sign(m, &sibling).as_bytes());
    }

    let c = build_attestation_commitment(&p, post, &universe, &[], &pool, &parent);
    assert_eq!(
        decode_attestation_bitfield_vec(&c.bitfield, universe.len()),
        vec![2],
        "O1: only the signature keyed on THIS parent sets a bit"
    );
}

/// REQ-BLS-001 — Decision: a failure means a pooled signature from a key that is not in
/// the universe influences the block. There is no index to give it, so the only ways to
/// "use" it are to shift every later bit by one or to fold it into the aggregate without
/// a bit — the first re-credits every producer after it, the second makes the aggregate
/// unverifiable against the key set the bits name.
#[test]
fn req_bls_001_m4_post_ah_a_pooled_key_outside_the_universe_is_ignored_entirely() {
    let p = gated(4_242);
    let (_pre, post) = sides(&p);
    let universe_members = members(20);
    let universe: Vec<PublicKey> = universe_members.iter().map(|m| m.pk).collect();
    let parent = crypto::hash::hash(b"m4-parent-i3");

    let signers: Vec<&Member> = vec![&universe_members[1], &universe_members[5]];
    let baseline = build_attestation_commitment(
        &p,
        post,
        &universe,
        &[],
        &pool_with(&parent, &signers),
        &parent,
    );

    // I3: a stranger with a perfectly valid signature over the same message.
    let stranger = Member {
        pk: pk(9_999),
        bls: BlsKeyPair::generate(),
    };
    assert!(
        !universe.contains(&stranger.pk),
        "the stranger must be outside the universe"
    );
    let mut with_stranger = signers.clone();
    with_stranger.push(&stranger);
    let polluted = build_attestation_commitment(
        &p,
        post,
        &universe,
        &[],
        &pool_with(&parent, &with_stranger),
        &parent,
    );

    assert_eq!(
        polluted.bitfield, baseline.bitfield,
        "O1: no bit for a stranger"
    );
    assert_eq!(
        polluted.aggregate, baseline.aggregate,
        "O2: the stranger's signature must NOT be folded into the aggregate"
    );
    assert_eq!(
        polluted.presence_root, baseline.presence_root,
        "O3: the commitment is unchanged, so a stranger cannot alter the block hash"
    );
    assert!(
        !baseline.aggregate.is_empty(),
        "anti-vacuity: the baseline aggregated"
    );
}

// ===========================================================================
// F1 x P-post — O2: the aggregate.
// ===========================================================================

/// REQ-BLS-003 — Decision: a failure means the aggregate is not the aggregate of exactly
/// the signatures whose bits are set. M5's verifier does `fast_aggregate_verify` over the
/// keys the bits name; if the builder folded in one extra signature, or dropped one, or
/// aggregated in a different order than the bits imply, every honest block fails
/// verification at the gate and the network stops accepting blocks.
#[test]
fn req_bls_003_m4_post_ah_the_aggregate_verifies_over_exactly_the_set_bits() {
    let p = gated(4_242);
    let (_pre, post) = sides(&p);
    let universe_members = members(20);
    let universe: Vec<PublicKey> = universe_members.iter().map(|m| m.pk).collect();
    let parent = crypto::hash::hash(b"m4-parent-agg");

    let signer_idx = [0usize, 3, 7, 8, 19];
    let signers: Vec<&Member> = signer_idx.iter().map(|i| &universe_members[*i]).collect();
    let c = build_attestation_commitment(
        &p,
        post,
        &universe,
        &[],
        &pool_with(&parent, &signers),
        &parent,
    );

    assert_eq!(
        c.aggregate.len(),
        96,
        "O2: a BLS12-381 G2 aggregate is 96 bytes"
    );
    let aggregate =
        BlsSignature::try_from_slice(&c.aggregate).expect("O2: must be a valid G2 point");

    let keys: Vec<BlsPublicKey> = decode_attestation_bitfield_vec(&c.bitfield, universe.len())
        .iter()
        .map(|i| *universe_members[*i].bls.public_key())
        .collect();
    assert_eq!(keys.len(), signer_idx.len(), "one key per set bit");
    assert!(
        bls_verify_aggregate(&bls_attest_msg(&parent), &aggregate, &keys).is_ok(),
        "O2: the aggregate must verify against the keys its own bits name"
    );

    // Anti-vacuity, both directions: one key too many and one key too few must FAIL,
    // or "it verifies" would be true of any aggregate.
    let mut too_many = keys.clone();
    too_many.push(*universe_members[1].bls.public_key());
    assert!(
        bls_verify_aggregate(&bls_attest_msg(&parent), &aggregate, &too_many).is_err(),
        "an extra key must break the pairing"
    );
    let too_few = &keys[..keys.len() - 1];
    assert!(
        bls_verify_aggregate(&bls_attest_msg(&parent), &aggregate, too_few).is_err(),
        "a missing key must break the pairing"
    );
    // And the message is the parent, not some other block.
    assert!(
        bls_verify_aggregate(
            &bls_attest_msg(&crypto::hash::hash(b"m4-parent-other")),
            &aggregate,
            &keys
        )
        .is_err(),
        "the aggregate must be bound to THIS parent's message"
    );
}

/// REQ-BLS-010 — Decision: a failure means a producer that holds no BLS-signed
/// attestation for its parent cannot build. That is a liveness regression with a
/// network-wide blast radius: during the mixed-fleet window (BRIDGE, Release N) and any
/// gossip partition, a producer legitimately holds zero signatures. If building errors
/// instead of emitting an empty commitment, the fleet stops producing exactly when it is
/// already degraded — the failure mode the redesign is forbidden from introducing.
#[test]
fn req_bls_010_m4_post_ah_zero_pooled_signatures_still_builds_a_commitment() {
    let p = gated(4_242);
    let (_pre, post) = sides(&p);
    let universe_members = members(20);
    let universe: Vec<PublicKey> = universe_members.iter().map(|m| m.pk).collect();
    let parent = crypto::hash::hash(b"m4-parent-i2");

    // I2: an EMPTY pool, and a pool holding only OTHER parents — both are "zero for
    // this parent", and a `signatures_for(parent).unwrap()` would panic on the first.
    let empty = ParentSignaturePool::new();
    let mut elsewhere = ParentSignaturePool::new();
    let other = crypto::hash::hash(b"m4-parent-elsewhere");
    for m in &universe_members {
        elsewhere.insert(other, m.pk, *sign(m, &other).as_bytes());
    }

    for (label, pool) in [("empty pool", &empty), ("other parents only", &elsewhere)] {
        let c = build_attestation_commitment(&p, post, &universe, &[], pool, &parent);

        assert!(
            c.bitfield.is_empty(),
            "{label}: O1 — an empty bitfield, no bits set"
        );
        assert!(
            c.aggregate.is_empty(),
            "{label}: O2 — `bls_aggregate` returns Err(EmptyAggregation) on an empty \
             set, so the builder must not call it; an empty field is the canonical value"
        );
        assert_eq!(
            c.presence_root,
            presence_commitment(&[], &[]),
            "{label}: O3 — the canonical empty commitment (D6), NOT Hash::ZERO"
        );
        assert_ne!(
            c.presence_root,
            Hash::ZERO,
            "{label}: O3 — Hash::ZERO is the legacy 'no attestation data' sentinel"
        );
    }

    // I5, the opposite extreme, so the empty result is not simply what the function
    // always returns: the FULL universe attesting must produce a full bitfield.
    let all: Vec<&Member> = universe_members.iter().collect();
    let full =
        build_attestation_commitment(&p, post, &universe, &[], &pool_with(&parent, &all), &parent);
    assert_eq!(
        decode_attestation_bitfield_vec(&full.bitfield, universe.len()).len(),
        universe.len(),
        "I5: every universe member attested"
    );
    assert_eq!(full.aggregate.len(), 96, "I5: a real aggregate");
}

/// REQ-BLS-003 — Decision: a failure means `presence_root` does not commit to the pair
/// the block carries, so the block hash is compatible with more than one (bitfield,
/// aggregate) pair. A relay could then strip the 96-byte aggregate and the block would
/// still hash the same — and a stripped aggregate does not fail verification, it SKIPS
/// it (REQ-BLS-002 AC-3). This assertion is what makes the field un-strippable.
#[test]
fn req_bls_003_m4_the_commitment_binds_the_pair_on_both_sides_of_the_gate() {
    let p = gated(4_242);
    let (pre, post) = sides(&p);
    let universe_members = members(20);
    let universe: Vec<PublicKey> = universe_members.iter().map(|m| m.pk).collect();
    let parent = crypto::hash::hash(b"m4-parent-bind");
    let attested: Vec<PublicKey> = [0usize, 3, 7].iter().map(|i| universe[*i]).collect();
    let signers: Vec<&Member> = [0usize, 3, 7]
        .iter()
        .map(|i| &universe_members[*i])
        .collect();
    let pool = pool_with(&parent, &signers);

    let post_c = build_attestation_commitment(&p, post, &universe, &attested, &pool, &parent);
    assert_eq!(
        post_c.presence_root,
        presence_commitment(&post_c.bitfield, &post_c.aggregate),
        "O3 x P-post: the D6 commitment over the pair the block carries"
    );
    assert_ne!(
        post_c.presence_root,
        crypto::hash::hash(&post_c.bitfield),
        "O3 x P-post: the legacy rule must no longer produce this root"
    );

    let pre_c = build_attestation_commitment(&p, pre, &universe, &attested, &pool, &parent);
    assert_eq!(
        pre_c.presence_root,
        crypto::hash::hash(&pre_c.bitfield),
        "O3 x P-pre: BLAKE3(bitfield), byte-identical to the old binary"
    );
    assert!(
        pre_c.aggregate.is_empty(),
        "O2 x P-pre: production emits no aggregate today"
    );
    assert_ne!(
        pre_c.presence_root, post_c.presence_root,
        "the gate must actually change the root; equal roots mean an inert gate"
    );
}

// ===========================================================================
// F2 — the REAL builder, end to end through the real ingress.
// ===========================================================================

/// REQ-BLS-003 — Decision: a failure means the pure function is correct but the shipped
/// builder does not call it, or calls it with a different universe or a different parent
/// hash. That is the `86bac138` shape this repo has already paid for: a correct
/// validator with no caller. Only driving `build_block_content` proves the block a node
/// would actually gossip carries the commitment.
#[tokio::test]
async fn req_bls_003_m4_the_real_builder_commits_to_the_pool_at_the_activation_height() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let height = safe_build_height(&node);
    let parent = node.chain_state.read().await.best_hash;

    // The gate is the build height itself, so the SAME height is post-AH here and
    // pre-AH in the sibling test — derived, never a literal.
    node.inc_i_178_attestation_bls_activation_height = height;

    // Real ingress: a strict subset dual-signs and is pooled by the production path.
    let signers: Vec<&KeyPair> = producers.iter().step_by(2).collect();
    let peer = PeerId::random();
    let slot = node.params.timestamp_to_slot(unix_now());
    let tip = node.best_height().await;
    let mut bls_keys: Vec<(PublicKey, BlsKeyPair)> = Vec::new();
    for kp in &signers {
        let bls = BlsKeyPair::generate();
        register_bls(&node, kp.public_key(), &bls).await;
        node.record_direct_attestation(dual(kp, &bls, parent, slot, tip), peer)
            .await;
        bls_keys.push((*kp.public_key(), bls));
    }
    assert_eq!(
        node.parent_sig_pool.total_signatures(),
        signers.len(),
        "precondition: the production ingress pooled every signature"
    );

    // Attendance for the whole set, so a pre-AH-style minute projection would produce
    // a DIFFERENT (wider) bitfield than the pool does — the anti-vacuity for O8.
    let all: Vec<PublicKey> = producers.iter().map(|k| *k.public_key()).collect();
    record_attesters(&mut node, slot, &all);

    let (header, _txs, bf) = build_via_production(&mut node, height).await;

    let active = active_at(&node, height).await;
    let universe = attestation_universe(&node.epoch_state.producer_list, &active);
    let set = decode_attestation_bitfield_vec(&bf, universe.len());
    let signer_pks: HashSet<[u8; 32]> =
        signers.iter().map(|k| *k.public_key().as_bytes()).collect();

    assert_eq!(
        set.len(),
        signers.len(),
        "O8: bits come from the POOL, not from the {} attending producers",
        all.len()
    );
    for i in &set {
        assert!(
            signer_pks.contains(universe[*i].as_bytes()),
            "O8: universe[{i}] has a bit but no pooled signature for the parent"
        );
    }

    // O7: the header commits to the pair. The aggregate is recomputed here from the
    // bits, so a builder that committed to a different aggregate fails.
    let expected_aggregate = {
        let sigs: Vec<BlsSignature> = set
            .iter()
            .map(|i| {
                let (_, bls) = bls_keys
                    .iter()
                    .find(|(p, _)| p.as_bytes() == universe[*i].as_bytes())
                    .expect("a set bit must belong to a signer");
                bls_sign(&bls_attest_msg(&parent), bls.secret_key()).expect("sign")
            })
            .collect();
        crypto::bls_aggregate(&sigs).expect("aggregate")
    };
    assert_eq!(
        header.presence_root,
        presence_commitment(&bf, expected_aggregate.as_bytes()),
        "O7: presence_root == presence_commitment(bitfield, aggregate-of-the-set-bits)"
    );
    assert_ne!(
        header.presence_root,
        crypto::hash::hash(&bf),
        "O7: the legacy rule must not still be in force at the activation height"
    );
}

/// REQ-BLS-005 AC-1 — Decision: the sibling of the test above on the other side of the
/// gate, on the same node and the same height. A failure means an upgraded node emits a
/// different header BELOW the activation height than the fleet it is joining, so the
/// rolling deploy the two-release plan depends on becomes a chain split.
#[tokio::test]
async fn req_bls_005_m4_the_real_builder_is_byte_identical_one_block_below_the_gate() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let height = safe_build_height(&node);
    let parent = node.chain_state.read().await.best_hash;

    // Same construction as the post-AH sibling, gate moved one block up.
    node.inc_i_178_attestation_bls_activation_height = height + 1;

    let signers: Vec<&KeyPair> = producers.iter().step_by(2).collect();
    let peer = PeerId::random();
    let slot = node.params.timestamp_to_slot(unix_now());
    let tip = node.best_height().await;
    for kp in &signers {
        let bls = BlsKeyPair::generate();
        register_bls(&node, kp.public_key(), &bls).await;
        node.record_direct_attestation(dual(kp, &bls, parent, slot, tip), peer)
            .await;
    }
    assert_eq!(
        node.parent_sig_pool.total_signatures(),
        signers.len(),
        "precondition: the pool IS populated below the gate (D2: harmless pre-AH)"
    );

    let all: Vec<PublicKey> = producers.iter().map(|k| *k.public_key()).collect();
    record_attesters(&mut node, slot, &all);

    let (header, _txs, bf) = build_via_production(&mut node, height).await;

    let active = active_at(&node, height).await;
    let universe = attestation_universe(&node.epoch_state.producer_list, &active);
    let set = decode_attestation_bitfield_vec(&bf, universe.len());

    assert_eq!(
        set.len(),
        all.len(),
        "O8 x P-pre: bits come from MINUTE ATTENDANCE, exactly as today — {} attending \
         producers, not the {} pooled signers",
        all.len(),
        signers.len()
    );
    assert_eq!(
        header.presence_root,
        crypto::hash::hash(&bf),
        "O7 x P-pre: presence_root = BLAKE3(bitfield), byte-identical to the old binary"
    );
    assert_ne!(
        header.presence_root,
        presence_commitment(&bf, &[]),
        "O7 x P-pre: the D6 preimage must NOT be reachable below the gate"
    );
}

// ===========================================================================
// F3 — the pool is epoch-scoped scratch, not an unbounded leak.
// ===========================================================================

/// REQ-BLS-010 — Decision: a failure means the parent-signature pool outlives the epoch
/// whose keys it was verified against. `minute_tracker.reset()` already runs at this
/// exact site because attendance is epoch-scoped; the pool is the same kind of
/// node-local scratch and shares the boundary. Left uncleared, it retains signatures
/// verified under the CLOSING epoch's `bls_pubkey` values, which a re-key or an exit can
/// invalidate — and it grows without an epoch-sized bound.
#[tokio::test]
async fn req_bls_010_m4_the_parent_pool_is_cleared_at_the_epoch_boundary_only() {
    use crate::inc_i_204_m41_common::{build_block, leader};

    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let bpe = node.config.network.blocks_per_reward_epoch();

    let mut height = node.best_height().await + 1;
    let mut prev = node.chain_state.read().await.best_hash;
    let mut saw_boundary = false;
    let mut saw_non_boundary = false;

    // Walk forward until BOTH cells have been observed. Boundary heights are derived
    // from the node's own params, never from a literal.
    for _ in 0..(bpe as usize * 3 + 4) {
        if saw_boundary && saw_non_boundary {
            break;
        }
        let is_boundary = doli_core::EpochSnapshot::is_epoch_boundary_with(height, bpe);

        // Re-seed the pool immediately before each applied block.
        let marker = crypto::hash::hash(format!("m4-pool-marker-{height}").as_bytes());
        node.parent_sig_pool
            .insert(marker, *producers[0].public_key(), [0x5au8; 96]);
        assert!(
            node.parent_sig_pool.total_signatures() > 0,
            "h={height}: precondition — the pool is non-empty before apply"
        );

        let slot = height as u32;
        let block = build_block(height, slot, prev, leader(&producers, slot), &node.params);
        prev = block.hash();
        node.apply_block(block, ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("apply_block failed at h={height}: {e}"));

        if is_boundary {
            assert_eq!(
                node.parent_sig_pool.parent_count(),
                0,
                "O9 x P-boundary (h={height}): every parent must be dropped beside \
                 minute_tracker.reset()"
            );
            assert_eq!(
                node.parent_sig_pool.total_signatures(),
                0,
                "O9 x P-boundary (h={height}): every signature must be dropped"
            );
            saw_boundary = true;
        } else {
            assert!(
                node.parent_sig_pool.total_signatures() > 0,
                "O9 x P-non-boundary (h={height}): clearing on EVERY block would empty \
                 the pool one block after each attestation and the builder would never \
                 find a signature for its parent"
            );
            saw_non_boundary = true;
        }
        height += 1;
    }

    assert!(
        saw_boundary,
        "the walk must cross at least one epoch boundary"
    );
    assert!(
        saw_non_boundary,
        "the walk must include a non-boundary block"
    );
}

/// REQ-BLS-010 — Decision: a failure means a rejected block clears the pool. An
/// attacker could then broadcast one invalid block per slot and keep every producer's
/// aggregate permanently empty without ever getting a block accepted.
#[tokio::test]
async fn req_bls_010_m4_a_rejected_block_does_not_clear_the_parent_pool() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let height = safe_build_height(&node);
    let marker = crypto::hash::hash(b"m4-pool-survives-reject");
    node.parent_sig_pool
        .insert(marker, *producers[0].public_key(), [0x5au8; 96]);

    // The builder emits a body bitfield only when the minute tracker holds
    // attesters. With none, the empty-bitfield bypass in validate_block_for_apply
    // skips the commitment check and the precondition below is unreachable.
    let slot = node.params.timestamp_to_slot(unix_now());
    let all: Vec<PublicKey> = producers.iter().map(|k| *k.public_key()).collect();
    record_attesters(&mut node, slot, &all);

    let (header, txs, bf) = build_via_production(&mut node, height).await;
    let mut block = assemble(header, txs, bf);
    // Corrupt the commitment so validation must refuse the block.
    block.header.presence_root = crypto::hash::hash(b"not-the-real-root");

    let verdict = node
        .validate_block_for_apply(&block, height, ValidationMode::Light)
        .await;
    assert!(verdict.is_err(), "precondition: the block must be rejected");

    assert_eq!(
        node.parent_sig_pool.total_signatures(),
        1,
        "O9: the reject path must not touch node-local attestation scratch"
    );
    assert!(
        node.parent_sig_pool.contains_parent(&marker),
        "O9: the parent survives"
    );
}
