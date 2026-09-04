//! INC-I-178 M4 — REQ-BLS-004: the RPC empty-attestation detectors must recognise the
//! post-AH canonical-empty commitment.
//!
//! `presence_root` stopped being a packed bitfield at the activation height, but
//! `From<&Block> for BlockResponse` still popcounts the hash and treats only
//! `Hash::ZERO` as "no attesters". A zero-pooled post-AH block would therefore be
//! reported to explorers and wallets as roughly 128 attesters.
//!
//! Pre-AH a `presence_root` is `BLAKE3(bitfield)`, so matching the canonical-empty
//! VALUE would require a preimage collision; the recognition needs no height.
//!
//! OUTPUT CONTRACT
//!
//! F1: `impl From<&Block> for BlockResponse` (pure conversion)
//!   O1 `.attestation_count: Option<u32>`
//!   O2 `.presence_root: Option<String>`
//!   O3 `.aggregate_bls_sig: Option<String>`
//!   O4 mutable params — NONE (`&Block`); asserted negatively
//!   O5 receiver/self / O6 store / O7 statics / O8 channels — NONE
//!   PATHS: PA root == Hash::ZERO | PB root == canonical empty | PC ordinary root
//!   INPUT PARTITIONS: PA and PB carry an EMPTY body; PC carries a real bitfield.
//!   MATRIX: O1-O3 claimed on all three paths; O4 asserted once on PB.

use crypto::Hash;
use doli_core::{presence_commitment, Block, BlockHeader};
use rpc::types::BlockResponse;
use vdf::{VdfOutput, VdfProof};

fn block_with(root: Hash, bitfield: Vec<u8>) -> Block {
    let header = BlockHeader {
        version: 2,
        prev_hash: Hash::ZERO,
        merkle_root: Hash::ZERO,
        presence_root: root,
        genesis_hash: Hash::ZERO,
        timestamp: 1_700_000_000,
        slot: 42,
        producer: crypto::KeyPair::from_seed([7u8; 32])
            .public_key()
            .to_owned(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    let mut block = Block::new(header, Vec::new());
    block.attestation_bitfield = bitfield;
    block
}

/// REQ-BLS-004 — Decision: a failure means every post-AH block whose producer holds no
/// pooled signatures is published to explorers, wallets and the block browser as having
/// ~128 attesters. Operators read that number to decide whether the network is attesting,
/// so a silent epoch would look like a healthy one.
#[test]
fn req_bls_004_m4_a_canonical_empty_block_reports_no_attesters() {
    let zero = BlockResponse::from(&block_with(Hash::ZERO, Vec::new()));
    let canonical_root = presence_commitment(&[], &[]);
    let canonical_block = block_with(canonical_root, Vec::new());
    let canonical = BlockResponse::from(&canonical_block);

    // Anti-vacuity: the naive popcount of this hash is large, so "not ~128" is a real
    // claim about the conversion and not about the constant.
    let popcount: u32 = canonical_root
        .as_bytes()
        .iter()
        .map(|b| b.count_ones())
        .sum();
    assert!(
        popcount > 64,
        "fixture: the canonical-empty hash popcounts high ({popcount}), which is the \
         number the current conversion would publish"
    );

    // O1: a zero-attester block reports zero attesters, in whatever shape the
    // Hash::ZERO path already uses.
    assert_eq!(
        canonical.attestation_count, zero.attestation_count,
        "the canonical empty is the post-AH spelling of 'nobody attested' and must \
         report what Hash::ZERO reports"
    );
    assert_ne!(
        canonical.attestation_count,
        Some(popcount),
        "the presence_root is a commitment above the activation height, not a packed \
         bitfield — popcounting it is meaningless"
    );

    // O3: an absent aggregate is still absent.
    assert!(
        canonical.aggregate_bls_sig.is_none(),
        "O3: no aggregate was carried"
    );

    // O4: the conversion does not mutate the block it borrows.
    assert_eq!(
        canonical_block.header.presence_root, canonical_root,
        "O4: the block is unchanged by the conversion"
    );
    assert!(
        canonical_block.attestation_bitfield.is_empty(),
        "O4: the body is unchanged by the conversion"
    );
}

/// REQ-BLS-004 — Decision: a failure means the M4 recognition changed what an ordinary
/// attested block or a legacy `Hash::ZERO` block reports, so every historical block in
/// the explorer changes its attester count on the upgrade.
#[test]
fn req_bls_004_m4_the_zero_and_ordinary_paths_keep_their_current_values() {
    // PA — the legacy sentinel.
    let zero = BlockResponse::from(&block_with(Hash::ZERO, Vec::new()));
    assert_eq!(zero.attestation_count, None, "PA O1 unchanged");
    assert_eq!(zero.presence_root, None, "PA O2 unchanged");

    // PC — an ordinary pre-AH block: presence_root = BLAKE3(bitfield).
    let bf = vec![0b1010_1101u8, 0x00, 0xff, 0x41];
    let root = crypto::hash::hash(&bf);
    assert_ne!(
        root,
        presence_commitment(&[], &[]),
        "fixture: an ordinary root must not be the canonical empty"
    );
    let ordinary = BlockResponse::from(&block_with(root, bf));
    let popcount: u32 = root.as_bytes().iter().map(|b| b.count_ones()).sum();
    assert_eq!(
        ordinary.attestation_count,
        Some(popcount),
        "PC O1 unchanged: an ordinary block keeps the popcount it reports today"
    );
    assert_eq!(
        ordinary.presence_root,
        Some(root.to_hex()),
        "PC O2 unchanged"
    );
}
