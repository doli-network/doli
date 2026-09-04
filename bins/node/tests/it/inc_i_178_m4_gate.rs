//! INC-I-178 M4 — D5/D8: the three consensus decode sites behind ONE activation
//! height, and the pre-AH byte identity that licenses calling the switch rolling-safe.
//!
//! Requirements: **REQ-BLS-004**, **REQ-BLS-005**, **REQ-BLS-014** (all Must).
//! TDD RED, EXPECTED: `doli_node::node::attestation::commit` does not exist at the M4
//! branch point, so this module does not compile against HEAD.
//!
//! Every gate-driven test reads its heights back OUT of the `NetworkParams` value the
//! code under test is handed (`gated()` / `sides()`), never from a literal in an
//! assertion. Rule `18779b1e` / INV-GOV-001: a test that hardcodes a gate literal
//! passes on a tree where the gate has silently moved.
//!
//! OUTPUT CONTRACT — ENUMERATION OF OBSERVABLE OUTPUTS.
//!
//!   F1: `attestation_bls_active(&NetworkParams, u64) -> bool`            (PURE)
//!       O1 return value. PATHS: P-below, P-at, P-above.
//!   F2: `encoder_universe(&NetworkParams, u64, &[PublicKey], &[PublicKey]) -> Vec<PublicKey>`
//!       O2 return value — ordered key list the BUILDER indexes bits against
//!   F3: `post_commit_universe(..) -> Vec<PublicKey>`
//!       O3 return value — ordered key list `apply_block` credits attendance against
//!   F4: `stray_bit_universe_width(..) -> usize`
//!       O4 return value — the denominator `validate_attestation_bitfield_vec` gets
//!   F5: `build_attestation_commitment(..) -> AttestationCommitment`
//!       O5 `.bitfield`, O6 `.aggregate`, O7 `.presence_root`
//!       (post-AH behaviour lives in `inc_i_178_m4_commit.rs`; this module drives ONLY
//!        the pre-AH arm, against the frozen M0 golden store)
//!       O8 mutable params / receiver / store writes / statics / channels — NONE for
//!          F1-F5; all five are pure over borrowed slices. Asserted negatively where
//!          a caller could be surprised (the pool is re-read after the call).
//!       PATHS for F2-F5: P-pre  (`height == AH - 1`) and P-post (`height == AH`).
//!       INPUT PARTITIONS:
//!         I1 the M3 divergent epoch: `producer_list \ active_at(h) != {}` (a mid-epoch
//!            EXIT) AND `active_at(h) \ producer_list != {}` (mid-epoch ADDS)
//!         I2 a `producer_list` carrying a duplicate key (C5/F14)
//!         I3 every one of the 66 frozen M0 golden vectors (REQ-BLS-005 AC-1)
//!
//!   MATRIX 8 outputs x 2 paths x 3 partitions: O1 by the gate test; O2/O3/O4 on
//!   both paths over I1 and I2; O5/O6/O7 on P-pre over I3; O8 structural.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crypto::{Hash, PublicKey};
use doli_core::attestation::ParentSignaturePool;
use doli_core::network_params::NetworkParams;
use doli_core::{attestation_universe, presence_commitment, Network};
use doli_node::node::attestation::commit::{
    attestation_bls_active, build_attestation_commitment, encoder_universe, post_commit_universe,
    stray_bit_universe_width,
};

// ---------------------------------------------------------------------------
// Gate-derived fixtures
// ---------------------------------------------------------------------------

/// A params value carrying an arbitrary probe gate. The VALUE is arbitrary on purpose:
/// the shipped default is `u64::MAX` on every network (D8), so a test that wanted to
/// exercise the post-AH arm at the shipped height could never run.
fn gated(ah: u64) -> NetworkParams {
    let mut p = NetworkParams::defaults(Network::Devnet);
    p.inc_i_178_attestation_bls_activation_height = ah;
    p
}

/// `(pre, post)` read back OUT of the params the code under test will read. Nothing
/// downstream may spell a height literal.
fn sides(p: &NetworkParams) -> (u64, u64) {
    let ah = p.inc_i_178_attestation_bls_activation_height;
    assert!(ah > 0, "the probe gate must leave room for a pre-AH height");
    (ah - 1, ah)
}

/// Deterministic, injective test pubkey whose byte order is NOT seed order — so a
/// universe that accidentally sorts `base` shows up as a mismatch.
fn pk(seed: u16) -> PublicKey {
    let mut b = [0u8; 32];
    let m = seed.wrapping_mul(40_503);
    b[0] = (m >> 8) as u8;
    b[1] = m as u8;
    b[2] = (seed >> 8) as u8;
    b[3] = seed as u8;
    PublicKey::from_bytes(b)
}

/// The M3 measured epoch (`inc_i_178_m3_index_parity.rs::divergent_epoch`): 45 frozen
/// base producers, one of them (seed 7) EXITED mid-epoch, plus five mid-epoch ADDS.
/// Both directions non-empty is the only shape in which the three sites disagree.
fn divergent_epoch() -> (Vec<PublicKey>, Vec<PublicKey>) {
    let base: Vec<PublicKey> = (0..45u16).map(pk).collect();
    let active: Vec<PublicKey> = (0..45u16)
        .filter(|s| *s != 7)
        .chain(100..105u16)
        .map(pk)
        .collect();
    (base, active)
}

/// `assembly.rs:408-424` verbatim, recomputed here so the expected pre-AH width is
/// independent of the function under test.
fn legacy_encoder_universe(base: &[PublicKey], active: &[PublicKey]) -> Vec<PublicKey> {
    let base_set: HashSet<&PublicKey> = base.iter().collect();
    let mut extra: Vec<PublicKey> = active
        .iter()
        .filter(|pk| !base_set.contains(pk))
        .copied()
        .collect();
    extra.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    let mut universe = base.to_vec();
    universe.extend(extra);
    universe
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root must resolve")
}

// ===========================================================================
// F1 — the gate itself.
// ===========================================================================

/// REQ-BLS-005 — Decision: a failure means the predicate every other M4 site consults
/// is off by one or inverted. An off-by-one makes the activation height in the ledger a
/// different height from the one the fleet actually switches at, so the margin computed
/// from auto-update telemetry (C16) covers the wrong block — and a half-upgraded mainnet
/// splits one block early or one block late with no way to tell which.
#[test]
fn req_bls_005_m4_the_gate_predicate_is_inclusive_at_the_activation_height() {
    let p = gated(4_242);
    let (pre, post) = sides(&p);

    assert!(!attestation_bls_active(&p, pre), "strictly below: pre-AH");
    assert!(
        attestation_bls_active(&p, post),
        "AT the height: post-AH (`height >= AH`, the shape every other DOLI gate uses)"
    );
    assert!(attestation_bls_active(&p, post + 1), "above: post-AH");
    assert!(
        !attestation_bls_active(&p, 0),
        "genesis is always pre-AH here"
    );

    // Mainnet and devnet ship frozen: the whole reachable height range is pre-AH.
    for network in [Network::Mainnet, Network::Devnet] {
        let shipped = NetworkParams::defaults(network);
        assert!(
            !attestation_bls_active(&shipped, u64::MAX - 1),
            "{network:?}: the shipped gate is frozen, so no reachable height is post-AH"
        );
    }

    // Testnet is PINNED (2026-09-05, v6.27.0): the shipped value is a real height and the
    // predicate is inclusive at it. Derived from the shipped params, never a literal.
    let testnet = NetworkParams::defaults(Network::Testnet);
    let pin = testnet.inc_i_178_attestation_bls_activation_height;
    assert!(pin != u64::MAX, "testnet must carry a real pinned height");
    assert!(
        !attestation_bls_active(&testnet, pin - 1),
        "testnet: pin - 1 is pre-AH"
    );
    assert!(
        attestation_bls_active(&testnet, pin),
        "testnet: the pin itself is post-AH"
    );
}

// ===========================================================================
// F2/F3/F4 — one universe for three sites, post-AH.
// ===========================================================================

/// REQ-BLS-004 — Decision: a failure means the builder, `apply_block` and the stray-bit
/// validator still index bits against three different key lists AFTER the gate. That is
/// the v6.17.1 death-spiral shape: the same block credits different producers depending
/// on which site reads it, and an honestly built block is rejected by the validator that
/// uses the narrowest denominator. If the three cannot be made to agree, the aggregate
/// verifier M5 adds is verifying against a key set no other site shares.
#[test]
fn req_bls_004_m4_post_ah_the_three_consensus_sites_share_one_universe() {
    let p = gated(4_242);
    let (_pre, post) = sides(&p);
    let (base, active) = divergent_epoch();

    // The partition is real, asserted before it is relied on.
    let active_set: HashSet<&PublicKey> = active.iter().collect();
    assert_eq!(
        base.iter().filter(|k| !active_set.contains(k)).count(),
        1,
        "I1: producer_list \\ active_at(h) != {{}} — one mid-epoch exit"
    );
    let base_set: HashSet<&PublicKey> = base.iter().collect();
    assert_eq!(
        active.iter().filter(|k| !base_set.contains(k)).count(),
        5,
        "I1: active_at(h) \\ producer_list != {{}} — five mid-epoch adds"
    );

    let enc = encoder_universe(&p, post, &base, &active);
    let pc = post_commit_universe(&p, post, &base, &active);
    let stray = stray_bit_universe_width(&p, post, &base, &active);
    let canonical = attestation_universe(&base, &active);

    assert_eq!(
        enc, canonical,
        "O2: the encoder uses the canonical universe"
    );
    assert_eq!(
        pc, canonical,
        "O3: post_commit uses the SAME list, not a copy"
    );
    assert_eq!(
        enc, pc,
        "O2 == O3: encoder/decoder index parity (Full Bitfield Decode pillar)"
    );
    assert_eq!(
        stray,
        enc.len(),
        "O4: the stray-bit denominator is the universe WIDTH, not active_at(h).len()"
    );

    // Anti-vacuity: the three would also agree if the fn returned an empty list.
    assert_eq!(
        canonical.len(),
        50,
        "45 base + 5 adds, the exit stays indexed"
    );
    assert_ne!(
        stray,
        active.len(),
        "O4: post-AH the denominator must have LEFT active_at(h).len() = {}",
        active.len()
    );
}

/// REQ-BLS-005 AC-1 — Decision: a failure means the pre-AH arm is not the current
/// expression. Below the activation height an upgraded node must build and validate
/// exactly what the old binary does; if the three sites already agree at `AH - 1`, the
/// new binary is a consensus change the moment it is deployed, and the rolling deploy
/// the whole D8 shape exists to permit becomes a chain split with no gate to blame.
#[test]
fn req_bls_005_m4_pre_ah_the_three_sites_keep_their_historical_divergence() {
    let p = gated(4_242);
    let (pre, post) = sides(&p);
    let (base, active) = divergent_epoch();

    let enc = encoder_universe(&p, pre, &base, &active);
    let pc = post_commit_universe(&p, pre, &base, &active);
    let stray = stray_bit_universe_width(&p, pre, &base, &active);

    // The two hand copies were byte-identical to each other before M4 — that is the
    // record, and it must not change either.
    let legacy = legacy_encoder_universe(&base, &active);
    assert_eq!(enc, legacy, "O2 x P-pre: assembly.rs:408-424 verbatim");
    assert_eq!(pc, legacy, "O3 x P-pre: post_commit.rs:34-57 verbatim");

    // The validator's THIRD, narrower source is the divergence M4 closes.
    assert_eq!(
        stray,
        active.len(),
        "O4 x P-pre: validation_checks.rs:434-437 verbatim — active_at(h).len()"
    );
    assert_ne!(
        stray,
        enc.len(),
        "O4 x P-pre: the recorded defect — an honestly built block of width {} is \
         rejected against a denominator of {}",
        enc.len(),
        stray
    );

    // And the gate is what separates the two worlds, on the SAME inputs.
    assert_eq!(
        stray_bit_universe_width(&p, post, &base, &active),
        enc.len(),
        "the only difference between the two heights is the gate"
    );
}

/// REQ-BLS-004 (C5/F14) — Decision: a failure means a key can appear twice in the
/// universe. Two indices would then map to one producer: the encoder sets one bit, the
/// decoder credits the wrong index, and M5's aggregate verifier gathers that key twice
/// — `fast_aggregate_verify` over a duplicated key rejects a signature that is correct.
/// `base` losing its prefix position is the other half: every historical block's indices
/// are defined by `epoch_state.producer_list` order, so re-sorting it re-credits history.
#[test]
fn req_bls_004_m4_post_ah_the_universe_is_duplicate_free_and_keeps_base_as_its_prefix() {
    let p = gated(4_242);
    let (_pre, post) = sides(&p);

    // I2: a `producer_list` that repeats a key, plus the mid-epoch add AND exit. The
    // duplicate also appears in `active`, so both dedup directions are exercised.
    let (mut base, active) = divergent_epoch();
    base.push(pk(3));
    assert_eq!(base.len(), 46, "I2: the raw list carries a duplicate");

    let universe = encoder_universe(&p, post, &base, &active);

    let mut seen: HashSet<[u8; 32]> = HashSet::new();
    for (i, k) in universe.iter().enumerate() {
        assert!(
            seen.insert(*k.as_bytes()),
            "C5: universe[{i}] repeats a key already at a lower index"
        );
    }

    // The deduped base, first-occurrence order preserved.
    let mut base_seen: HashSet<[u8; 32]> = HashSet::new();
    let deduped: Vec<PublicKey> = base
        .iter()
        .filter(|k| base_seen.insert(*k.as_bytes()))
        .copied()
        .collect();
    assert_eq!(deduped.len(), 45, "the duplicate collapsed to one entry");
    assert_eq!(
        &universe[..deduped.len()],
        deduped.as_slice(),
        "F14: base keeps its FIRST-occurrence order as the universe prefix; \
         re-sorting it re-credits every historical bit"
    );
    assert_eq!(
        universe.len(),
        50,
        "45 deduped base + 5 mid-epoch adds; the duplicate adds no index"
    );

    // Every site must agree on the deduped width, or the dedup is only the encoder's.
    assert_eq!(post_commit_universe(&p, post, &base, &active), universe);
    assert_eq!(stray_bit_universe_width(&p, post, &base, &active), 50);
}

/// REQ-BLS-014 — Decision: this is the node-side half of the M3 divergence proof
/// (`crates/core/tests/inc_i_178_m3_index_parity.rs` keeps the pre-AH half, which
/// records widths `[50, 44, 45, 45, 49]` across five sites). A failure means the THREE
/// consensus sites M4 unifies still answer with more than one width after the gate, so
/// REQ-BLS-014's "a test constructs an epoch where producer_list holds a producer
/// inactive at H, and asserts an honestly-built block is accepted" cannot hold.
#[test]
fn req_bls_014_m4_post_ah_one_width_replaces_the_recorded_disagreement() {
    let p = gated(4_242);
    let (pre, post) = sides(&p);
    let (base, active) = divergent_epoch();

    let pre_widths = vec![
        encoder_universe(&p, pre, &base, &active).len(),
        post_commit_universe(&p, pre, &base, &active).len(),
        stray_bit_universe_width(&p, pre, &base, &active),
    ];
    assert_eq!(
        pre_widths,
        vec![50, 50, 49],
        "P-pre: the recorded consensus-site widths (the two hand copies agreed with \
         each other and disagreed with the validator)"
    );
    assert_eq!(
        pre_widths.iter().collect::<HashSet<_>>().len(),
        2,
        "P-pre: two distinct widths is the defect being carried, not fixed"
    );

    let post_widths = vec![
        encoder_universe(&p, post, &base, &active).len(),
        post_commit_universe(&p, post, &base, &active).len(),
        stray_bit_universe_width(&p, post, &base, &active),
    ];
    assert_eq!(
        post_widths.iter().collect::<HashSet<_>>().len(),
        1,
        "P-post: ONE width for all three sites, got {post_widths:?}"
    );
    assert_eq!(post_widths[0], 50, "P-post: the canonical universe width");
}

// ===========================================================================
// F5 x P-pre — the mixed-fleet proof (REQ-BLS-005 AC-1).
// ===========================================================================

fn golden_vectors() -> Vec<serde_json::Value> {
    let path = repo_root()
        .join("crates")
        .join("core")
        .join("tests")
        .join("fixtures")
        .join("attestation_baseline_vectors.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("M0 golden store missing at {}: {e}", path.display()));
    let doc: serde_json::Value =
        serde_json::from_str(&raw).expect("golden store must be valid JSON");
    doc["vectors"].as_array().expect("vectors array").clone()
}

/// REQ-BLS-005 AC-1 — Decision: THIS is the single test that licenses calling the M4
/// switch rolling-safe. A failure means a node running the new binary emits different
/// bitfield bytes or a different `presence_root` than the fleet BELOW the activation
/// height — i.e. the change is live the moment the binary is deployed, with ~30 external
/// mainnet auto-update producers and no stop-all available. Everything else in M4 can be
/// re-decided; this one cannot be worked around, only respected.
#[test]
fn req_bls_005_m4_pre_ah_the_builder_reproduces_every_frozen_golden_vector() {
    let p = gated(4_242);
    let (pre, _post) = sides(&p);
    let vectors = golden_vectors();
    assert_eq!(vectors.len(), 66, "the frozen store must not shrink");

    let parent = crypto::hash::hash(b"inc-i-178-m4-golden-parent");

    for v in &vectors {
        let id = v["id"].as_str().expect("id");
        let n = v["n"].as_u64().expect("n") as usize;
        let attested_idx: Vec<usize> = v["attested"]
            .as_array()
            .expect("attested")
            .iter()
            .map(|x| x.as_u64().expect("index") as usize)
            .collect();

        let universe: Vec<PublicKey> = (0..n as u16).map(pk).collect();
        let attested: Vec<PublicKey> = attested_idx.iter().map(|i| universe[*i]).collect();

        // ANTI-VACUITY: a pool that would produce a DIFFERENT bitfield post-AH. If the
        // pre-AH arm ever reads the pool, these vectors stop reproducing.
        let mut pool = ParentSignaturePool::new();
        let attested_set: HashSet<[u8; 32]> = attested.iter().map(|k| *k.as_bytes()).collect();
        for k in universe
            .iter()
            .filter(|k| !attested_set.contains(k.as_bytes()))
            .take(3)
        {
            pool.insert(parent, *k, [0x5au8; 96]);
        }

        let c = build_attestation_commitment(&p, pre, &universe, &attested, &pool, &parent);

        assert!(
            c.aggregate.is_empty(),
            "{id}: O6 x P-pre — production emits an EMPTY aggregate today \
             (production/mod.rs:601-616); a 96-byte field below the AH is new block content"
        );

        if attested.is_empty() {
            // `assembly.rs:391-393`: no attesters -> empty body bitfield and the
            // Hash::ZERO sentinel, NOT an encoded all-zero bitfield.
            assert!(
                c.bitfield.is_empty(),
                "{id}: O5 x P-pre — empty body bitfield"
            );
            assert_eq!(
                c.presence_root,
                Hash::ZERO,
                "{id}: O7 x P-pre — today's empty-case sentinel"
            );
            assert_eq!(
                v["legacy_presence_root_hex"].as_str().unwrap(),
                Hash::ZERO.to_hex(),
                "{id}: cross-check — the store agrees this vector is the empty shape"
            );
        } else {
            assert_eq!(
                hex::encode(&c.bitfield),
                v["bitfield_hex"].as_str().unwrap(),
                "{id}: O5 x P-pre — bitfield bytes must be byte-identical to the store"
            );
            assert_eq!(
                c.presence_root.to_hex(),
                v["presence_root_hex"].as_str().unwrap(),
                "{id}: O7 x P-pre — presence_root = BLAKE3(bitfield), exactly as today"
            );
            // The pre-AH root must NOT be the new commitment, or the gate is inert.
            assert_ne!(
                c.presence_root,
                presence_commitment(&c.bitfield, &c.aggregate),
                "{id}: pre-AH must use the LEGACY preimage, not the D6 one"
            );
        }

        // O8: the pool the builder was handed is unchanged and still non-trivial.
        assert!(
            pool.total_signatures() <= 3,
            "{id}: O8 — the builder must not write into the pool"
        );
    }
}

// ===========================================================================
// D8 — "nothing else moved": no version bump, no HardForkSchedule entry.
// ===========================================================================

/// REQ-BLS-005 AC-2 — Decision: a failure means M4 shipped a deploy vehicle CLAUDE.md
/// forbids. A `CURRENT_PROTOCOL_VERSION` bump triggers `delete_epoch_state()` on restart
/// (`init.rs:727`), which rebuilds non-deterministically and forks at the next epoch
/// boundary — that is INC-I-054, verbatim. A `HardForkSchedule` entry is worse:
/// `current_fork_id()` evaluates at `u64::MAX`, so a "future" entry changes `fork_id`
/// IMMEDIATELY on the first node that installs the binary, partitioning it from the
/// fleet with no height to wait for.
#[test]
fn req_bls_005_m4_no_protocol_bump_and_no_hardfork_entry() {
    assert_eq!(
        network::protocols::status::CURRENT_PROTOCOL_VERSION,
        8,
        "INV-4: the EpochState serialization format is unchanged by M4, so this must \
         not move. A bump deletes epoch_state on every restart."
    );
    assert_eq!(
        network::protocols::status::EPOCH_STATE_FORMAT_VERSION,
        1,
        "the epoch_state layout is untouched (D8 evidence)"
    );

    let counts = [
        (Network::Mainnet, 0usize),
        (Network::Testnet, 2usize),
        (Network::Devnet, 0usize),
    ];
    for (network, expected) in counts {
        let schedule = updater::HardForkSchedule::for_network(network);
        assert_eq!(
            schedule.all().len(),
            expected,
            "{network:?}: M4 must add no HardForkSchedule entry"
        );
        for fork in schedule.all() {
            for change in &fork.consensus_changes {
                let lower = change.to_lowercase();
                assert!(
                    !lower.contains("bls")
                        && !lower.contains("attestation")
                        && !lower.contains("inc-i-178"),
                    "{network:?}: INC-I-178 must not appear in the hard-fork schedule, \
                     found {change:?} at height {}",
                    fork.activation_height
                );
            }
        }
    }
}
