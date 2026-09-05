//! INC-I-178 M0 — characterization lock on block identity and build determinism.
//!
//! GREEN on current code by construction. Zero production code changes. This is
//! the executable "no header change" contract for M4 and the pre-AH byte-identity
//! baseline for M5.
//!
//! OUTPUT CONTRACT
//!
//! F2: `BlockHeader::hash(&self) -> Hash`
//!   Observable outputs: O1 returned Hash (the only output; `&self`, no mutation)
//!   Paths: PA fork_id.is_zero() (field skipped), PB fork_id non-zero (field hashed)
//!   INPUT PARTITIONS: one per field — 12 hashed header fields (O1 MUST change);
//!     vdf_proof plus the 3 body fields (O1 MUST NOT change); plus the PA/PB
//!     fork_id zero-vs-non-zero pair. 16/16 cells.
//!
//! F3: `Node::build_block_content(&mut self, prev_hash, prev_slot, height, current_slot, our_pubkey)
//!      -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>`
//!   Observable outputs:
//!     O1 returned header (all fields)
//!     O2 returned transaction list
//!     O3 returned body_bitfield bytes
//!     O4 `&mut self` mutation — none observable on the non-epoch-boundary path;
//!        asserted by re-running and getting the same O1/O2/O3
//!     O5 serialized `Block` bytes + `Block::hash()`
//!   Paths: PA base-only universe (the only path reachable without a mid-epoch extra)
//!   INPUT PARTITIONS: PAa N=12 every second producer attesting (sparse),
//!     PAb N=12 all producers attesting (full).
//!   NON-PINNABLE INPUT: `SystemTime::now()` read at assembly.rs:371 feeds
//!     header.timestamp, header.slot and the slot stamped into the coinbase.
//!     M0 must not add a production seam to inject it, so both builds are placed
//!     inside ONE devnet slot (1 s) and the pair is retried if the second ticks
//!     between them — the input then takes the same value and O5 is exact.
//!   Shared harness: `inc_i_178_m0_common`.

use crypto::PublicKey;

use super::inc_i_178_m0_common::{
    assemble, build_pair_in_one_slot, make_node, record_attesters, safe_build_height,
    sample_header, unix_now, N_SMALL,
};
use crypto::{Hash, KeyPair};
use doli_core::transaction::Transaction;
use doli_core::Block;
use vdf::{VdfOutput, VdfProof};

// REQ-BLS-003 AC-3 — Decision: a failure is the executable proof that M4 changed the header
// field list, which would be a wire break no activation height can gate
// (`BlockHeader::hash()` takes no height or params input).
#[test]
fn req_bls_003_ac3_block_header_hash_covers_the_header_only() {
    let base = sample_header(crypto::hash::hash(b"m0-fork"));
    let base_hash = base.hash();

    // --- hashed header fields: every mutation MUST change the hash ---
    let mut h = base.clone();
    h.version = 3;
    assert_ne!(h.hash(), base_hash, "version must be in the header hash");

    let mut h = base.clone();
    h.prev_hash = crypto::hash::hash(b"other-prev");
    assert_ne!(h.hash(), base_hash, "prev_hash must be in the header hash");

    let mut h = base.clone();
    h.merkle_root = crypto::hash::hash(b"other-merkle");
    assert_ne!(
        h.hash(),
        base_hash,
        "merkle_root must be in the header hash"
    );

    let mut h = base.clone();
    h.presence_root = crypto::hash::hash(b"other-presence");
    assert_ne!(
        h.hash(),
        base_hash,
        "presence_root must be in the header hash — it carries D6's commitment"
    );

    let mut h = base.clone();
    h.genesis_hash = crypto::hash::hash(b"other-genesis");
    assert_ne!(
        h.hash(),
        base_hash,
        "genesis_hash must be in the header hash"
    );

    let mut h = base.clone();
    h.missed_producers = vec![*KeyPair::generate().public_key()];
    assert_ne!(
        h.hash(),
        base_hash,
        "missed_producers must be in the header hash"
    );

    let mut h = base.clone();
    h.data_root = crypto::hash::hash(b"other-data");
    assert_ne!(h.hash(), base_hash, "data_root must be in the header hash");

    let mut h = base.clone();
    h.timestamp += 1;
    assert_ne!(h.hash(), base_hash, "timestamp must be in the header hash");

    let mut h = base.clone();
    h.slot += 1;
    assert_ne!(h.hash(), base_hash, "slot must be in the header hash");

    let mut h = base.clone();
    h.producer = *KeyPair::generate().public_key();
    assert_ne!(h.hash(), base_hash, "producer must be in the header hash");

    let mut h = base.clone();
    h.vdf_output = VdfOutput {
        value: vec![9u8; 32],
    };
    assert_ne!(
        h.hash(),
        base_hash,
        "vdf_output.value must be in the header hash"
    );

    // PB: a non-zero fork_id is hashed.
    let mut h = base.clone();
    h.fork_id = crypto::hash::hash(b"other-fork");
    assert_ne!(
        h.hash(),
        base_hash,
        "a non-zero fork_id must be in the header hash"
    );
    // PA vs PB: the zero fork_id is SKIPPED, so the two branches differ.
    let mut zero_fork = base.clone();
    zero_fork.fork_id = Hash::ZERO;
    assert_ne!(
        zero_fork.hash(),
        base_hash,
        "the fork_id zero/non-zero branch must be observable in the header hash"
    );

    // --- not hashed: one header field + every body field ---
    let mut h = base.clone();
    h.vdf_proof = VdfProof { pi: vec![1, 2, 3] };
    assert_eq!(
        h.hash(),
        base_hash,
        "vdf_proof is NOT in the header hash today"
    );

    let mut block = Block::new(base.clone(), Vec::new());
    assert_eq!(
        block.hash(),
        base_hash,
        "Block::hash() must be exactly BlockHeader::hash()"
    );

    block.transactions = vec![Transaction::new_coinbase(
        1_000,
        doli_core::consensus::reward_pool_pubkey_hash(),
        1,
        1,
    )];
    assert_eq!(
        block.hash(),
        base_hash,
        "transactions are body-side: merkle_root is their only header commitment"
    );

    block.aggregate_bls_signature = vec![0xABu8; 96];
    assert_eq!(
        block.hash(),
        base_hash,
        "aggregate_bls_signature is body-side: M4 must keep it out of the header hash"
    );

    block.attestation_bitfield = vec![0xFFu8; 6];
    assert_eq!(
        block.hash(),
        base_hash,
        "attestation_bitfield is body-side: presence_root is its only header commitment"
    );
}

// ============================================================
// REQ-BLS-005 — pre-AH deterministic build (mixed-fleet byte identity)
// ============================================================

// REQ-BLS-005 AC-1 — Decision: a failure means a new binary would emit a different bitfield
// than an old one below the activation height, breaking the rolling-deploy assumption.
#[tokio::test]
async fn req_bls_005_ac1_builder_bitfield_and_presence_root_are_deterministic() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let height = safe_build_height(&node);
    let slot = node.params.timestamp_to_slot(unix_now());
    let subset: Vec<PublicKey> = producers
        .iter()
        .step_by(2)
        .map(|k| *k.public_key())
        .collect();
    record_attesters(&mut node, slot, &subset);

    let ((h1, tx1, bf1), (h2, tx2, bf2)) = build_pair_in_one_slot(&mut node, height).await;

    assert!(!bf1.is_empty(), "setup: the bitfield must be non-empty");
    assert_eq!(
        bf1, bf2,
        "O3: the encoder must be byte-identical across builds — attested_in_minute() \
         iterates a HashMap, so iteration ORDER must not leak into the bytes"
    );
    assert_eq!(
        h1.presence_root, h2.presence_root,
        "O1: presence_root must be byte-identical across builds"
    );
    assert_eq!(
        h1.presence_root.as_bytes(),
        crypto::hash::hash(&bf1).as_bytes(),
        "AC-1: presence_root == BLAKE3(bitfield) is the pre-AH preimage"
    );
    assert_eq!(
        bincode::serialize(&tx1).unwrap(),
        bincode::serialize(&tx2).unwrap(),
        "O2: the transaction list must be byte-identical across builds"
    );
}

// REQ-BLS-005 AC-1 — Decision: a failure means the pre-AH block bytes are not reproducible,
// so a mixed old/new fleet below the activation height would fork on block hash.
#[tokio::test]
async fn req_bls_005_ac1_prebuilt_block_bytes_are_byte_identical_within_one_slot() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let height = safe_build_height(&node);
    let slot = node.params.timestamp_to_slot(unix_now());
    let all: Vec<PublicKey> = producers.iter().map(|k| *k.public_key()).collect();
    record_attesters(&mut node, slot, &all);

    let ((h1, tx1, bf1), (h2, tx2, bf2)) = build_pair_in_one_slot(&mut node, height).await;

    let b1 = assemble(h1, tx1, bf1);
    let b2 = assemble(h2, tx2, bf2);

    assert!(
        !b1.attestation_bitfield.is_empty(),
        "setup: the comparison must cover a non-empty bitfield"
    );
    assert_eq!(
        b1.attestation_bitfield, b2.attestation_bitfield,
        "O5: body bitfield bytes must match"
    );
    assert_eq!(
        b1.aggregate_bls_signature, b2.aggregate_bls_signature,
        "O5: the aggregate field must stay empty pre-AH"
    );
    assert_eq!(
        bincode::serialize(&b1).unwrap(),
        bincode::serialize(&b2).unwrap(),
        "O5: two pre-AH builds from identical inputs must serialize to identical bytes"
    );
    assert_eq!(
        b1.hash(),
        b2.hash(),
        "O5: two pre-AH builds from identical inputs must share one Block::hash() — \
         this is the mixed-fleet condition below the activation height"
    );
}
