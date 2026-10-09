//! INC-I-236 M1 — signature-rule edge cases, signing-message domain, AH plumbing,
//! and the pre-fix golden encoding. Shared builders live in
//! `inc_i_236_forged_block_authn.rs`.
//!
// covers: block.rs network_params validation/block.rs validation/types.rs
// covers: crates/core/src/block.rs crates/core/src/network_params/mod.rs crates/core/src/network_params/defaults.rs crates/core/src/network_params/env_loader.rs crates/core/src/validation/block.rs crates/core/src/validation/types.rs crates/core/src/validation/error.rs
//!
//! PROCESS-WIDE HAZARD: calls only `NetworkParams::defaults`, never `NetworkParams::load`.
//! TDD RED: does not compile until the M1 API exists.
//!
// OUTPUT CONTRACT: fn validate_block_with_mode(&Block, &ValidationContext, ValidationMode)
//   O1 return Result<(), ValidationError>; O2 no mutable params / receiver / store
//   PATHS: P-SIG (h >= AH) x {Full, Light, Replay}
// OUTPUT CONTRACT: fn BlockHeader::signing_message(&self) -> Hash
//   O1 return digest = blake3(BLOCK_SIGNATURE_DOMAIN || hash()); O2 none (pure)
// OUTPUT CONTRACT: fn BlockHeader::hash(&self) / bincode encoding
//   O1 return hash; O2 encoded bytes — both must be byte-identical to pre-fix code
// OUTPUT CONTRACT: fn NetworkParams::defaults(network) -> NetworkParams
//   O1 return .inc_i_236_block_signature_activation_height; O2 none
// INPUT PARTITIONS
//   IP-LEN pi len in {0, 1, 63, 65, 10 KiB} and 64 zero bytes
//   IP-KEY signature by a non-producer key
//   IP-MSG signature over hash() raw | over blake3(utf8) as `doli sign` | over a sibling header
//   IP-MAL non-canonical S (S + L) | small-order producer key with identity R
//   IP-MUT header field mutated after signing
//   IP-NET mainnet | testnet | devnet
// MATRIX
//   O1(validate) x IP-LEN x 3 modes -> req_auth_sec_001_malformed_pi_lengths_rejected_without_panic
//   O1(validate) x IP-KEY x 3 modes -> req_auth_003_signature_by_other_key_rejected
//   O1(validate) x IP-MSG x 3 modes -> req_auth_003_raw_hash_signature_rejected, ..._doli_sign_..., ..._sibling_...
//   O1(validate) x IP-MAL x 3 modes -> req_auth_sec_001_non_canonical_s_rejected, ..._small_order_key_rejected
//   O1(validate) x IP-MUT x 3 modes -> req_auth_003_header_mutated_after_signing_rejected
//   O1(signing_message)             -> req_auth_003_signing_message_domain_is_not_utf8
//   O1,O2(hash/bincode)             -> req_auth_004_golden_*
//   O1(defaults) x IP-NET           -> req_auth_001_per_network_activation_heights

use super::inc_i_236_forged_block_authn::{
    armed, assert_sig_rejected, forge_block_as, sign_into_pi, signed_block,
};
use crypto::{Hash, KeyPair, PublicKey};
use doli_core::block::{Block, BlockHeader, BLOCK_SIGNATURE_DOMAIN};
use doli_core::network_params::NetworkParams;
use doli_core::Network;

fn victim() -> PublicKey {
    PublicKey::from_bytes([0x11; 32])
}

// REQ-AUTH-SEC-001 — Decision: a failure means untrusted pi bytes can panic a node or slip past the length check.
#[test]
fn req_auth_sec_001_malformed_pi_lengths_rejected_without_panic() {
    for pi in [
        Vec::new(),
        vec![0x01],
        vec![0x02; 63],
        vec![0x03; 65],
        vec![0x04; 10 * 1024],
        vec![0x00; 64],
    ] {
        let len = pi.len();
        let (mut block, ctx, _kp) = signed_block();
        block.header.vdf_proof.pi = pi;
        assert_sig_rejected(&block, &ctx, &format!("pi len {len}"));
    }
}

// REQ-AUTH-003 — Decision: a failure means any keyholder can sign blocks in someone else's slot.
#[test]
fn req_auth_003_signature_by_other_key_rejected() {
    let (mut block, ctx) = forge_block_as(victim());
    let attacker = KeyPair::from_seed([0x66; 32]);
    sign_into_pi(&mut block, &attacker);
    assert_sig_rejected(&block, &armed(ctx), "attacker key, producer = victim");
}

// REQ-AUTH-003 — Decision: a failure means signatures lack domain separation and other protocol signatures over a hash can be replayed as block signatures.
#[test]
fn req_auth_003_raw_hash_signature_rejected() {
    let (mut block, ctx, kp) = signed_block();
    let raw = block.header.hash();
    assert_ne!(
        block.header.signing_message(),
        raw,
        "signing_message must differ from hash()"
    );
    block.header.vdf_proof.pi = crypto::signature::sign(raw.as_bytes(), kp.private_key()).to_vec();
    assert_sig_rejected(&block, &ctx, "signature over raw header.hash()");
}

// REQ-AUTH-003 — Decision: a failure means a phished `doli sign` of a text string yields a usable block signature.
#[test]
fn req_auth_003_doli_sign_text_signature_rejected() {
    let (mut block, ctx, kp) = signed_block();
    for text in [
        block.header.signing_message().to_hex(),
        block.header.hash().to_hex(),
        "DOLI_BLOCK_SIG_V1".to_string(),
    ] {
        let digest = crypto::hash::hash(text.as_bytes());
        block.header.vdf_proof.pi =
            crypto::signature::sign(digest.as_bytes(), kp.private_key()).to_vec();
        assert_sig_rejected(&block, &ctx, &format!("doli-sign style over {text:?}"));
    }
}

// REQ-AUTH-003 — Decision: a failure means the digest is not bound to the domain tag, or the tag is UTF-8 and reachable by `doli sign`.
#[test]
fn req_auth_003_signing_message_domain_is_not_utf8() {
    let (block, _ctx, _kp) = signed_block();
    let domain: &[u8] = BLOCK_SIGNATURE_DOMAIN;
    assert!(!domain.is_empty());
    assert!(
        domain.contains(&0xFF),
        "domain tag must contain a 0xFF byte"
    );
    let h = block.header.hash();
    let mut preimage = domain.to_vec();
    preimage.extend_from_slice(h.as_bytes());
    assert!(
        std::str::from_utf8(&preimage).is_err(),
        "preimage must never be UTF-8"
    );
    assert_eq!(
        block.header.signing_message(),
        crypto::hash::hash_concat(&[domain, h.as_bytes()]),
        "signing_message must be blake3(BLOCK_SIGNATURE_DOMAIN || hash())"
    );
}

// REQ-AUTH-003 — Decision: a failure means a signature from one block can be lifted onto a different block.
#[test]
fn req_auth_003_sibling_header_signature_rejected() {
    let (mut block, ctx, kp) = signed_block();
    let mut sibling = block.header.clone();
    sibling.merkle_root = Hash::from_bytes([0x77; 32]);
    block.header.vdf_proof.pi =
        crypto::signature::sign(sibling.signing_message().as_bytes(), kp.private_key()).to_vec();
    assert_sig_rejected(&block, &ctx, "signature lifted from a sibling header");
}

// REQ-AUTH-003 — Decision: a failure means header fields can be rewritten after signing (signature not over the final header).
#[test]
fn req_auth_003_header_mutated_after_signing_rejected() {
    let (mut block, ctx, _kp) = signed_block();
    block.header.timestamp += 1;
    assert_sig_rejected(&block, &ctx, "timestamp mutated after signing");

    let (mut block, ctx, _kp) = signed_block();
    block.header.presence_root = Hash::from_bytes([0x99; 32]);
    assert_sig_rejected(&block, &ctx, "presence_root mutated after signing");
}

/// Ed25519 group order L, little-endian.
const ORDER_L: [u8; 32] = [
    0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10,
];

// REQ-AUTH-SEC-001 — Decision: a failure means malleable (S + L) signatures are accepted, so one block has many valid encodings.
#[test]
fn req_auth_sec_001_non_canonical_s_rejected() {
    let (mut block, ctx, _kp) = signed_block();
    let pi = &mut block.header.vdf_proof.pi;
    let mut carry = 0u16;
    for i in 0..32 {
        let sum = pi[32 + i] as u16 + ORDER_L[i] as u16 + carry;
        pi[32 + i] = sum as u8;
        carry = sum >> 8;
    }
    assert_eq!(carry, 0);
    assert_sig_rejected(&block, &ctx, "S replaced by S + L");
}

// REQ-AUTH-SEC-001 — Decision: a failure means non-strict verify is used: a small-order producer key verifies a keyless signature.
#[test]
fn req_auth_sec_001_small_order_key_rejected() {
    let mut identity = [0u8; 32];
    identity[0] = 1;
    let (mut block, ctx) = forge_block_as(PublicKey::from_bytes(identity));
    let mut pi = vec![0u8; 64];
    pi[0] = 1;
    block.header.vdf_proof.pi = pi;
    assert_sig_rejected(
        &block,
        &armed(ctx),
        "identity producer key, R = identity, S = 0",
    );
}

// REQ-AUTH-001 — Decision: a failure means the rule ships armed on mainnet/testnet before the user pins a height, or unarmed on devnet.
#[test]
fn req_auth_001_per_network_activation_heights() {
    for (network, expected) in [
        (Network::Mainnet, u64::MAX),
        (Network::Testnet, u64::MAX),
        (Network::Devnet, 0),
    ] {
        assert_eq!(
            NetworkParams::defaults(network).inc_i_236_block_signature_activation_height,
            expected,
            "{network:?}"
        );
    }
}

fn golden_header() -> BlockHeader {
    BlockHeader {
        version: 2,
        prev_hash: Hash::from_bytes([0xAB; 32]),
        merkle_root: Hash::from_bytes([0xCD; 32]),
        presence_root: Hash::from_bytes([0x01; 32]),
        genesis_hash: Hash::from_bytes([0x02; 32]),
        timestamp: 1_700_000_000,
        slot: 5_000,
        producer: PublicKey::from_bytes([0x11; 32]),
        vdf_output: vdf::VdfOutput {
            value: vec![0x33; 32],
        },
        vdf_proof: vdf::VdfProof::empty(),
        missed_producers: vec![PublicKey::from_bytes([0x55; 32])],
        data_root: Hash::from_bytes([0x05; 32]),
        fork_id: Hash::from_bytes([0x06; 32]),
    }
}

const GOLDEN_HASH: &str = "6e24525c98a7061f272c14bb2f50419d1b2f66f94f0b174d9995aabe3eec67e8";
const GOLDEN_HEADER_HEX: &str = "020000002000000000000000abababababababababababababababababababababababababababababababab2000000000000000cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd200000000000000001010101010101010101010101010101010101010101010101010101010101012000000000000000020202020202020202020202020202020202020202020202020202020202020200f153650000000088130000200000000000000011111111111111111111111111111111111111111111111111111111111111112000000000000000333333333333333333333333333333333333333333333333333333333333333300000000000000000100000000000000200000000000000055555555555555555555555555555555555555555555555555555555555555552000000000000000050505050505050505050505050505050505050505050505050505050505050520000000000000000606060606060606060606060606060606060606060606060606060606060606";
const GOLDEN_BLOCK_TAIL_HEX: &str = "000000000000000000000000000000000000000000000000";

// REQ-AUTH-004 — Decision: a failure means the change altered the header hash or wire/storage encoding of existing blocks (chain reset).
#[test]
fn req_auth_004_golden_hash_and_encoding_unchanged() {
    let header = golden_header();
    assert_eq!(header.hash().to_hex(), GOLDEN_HASH);
    let enc = bincode::serialize(&header).expect("encode header");
    assert_eq!(hex::encode(&enc), GOLDEN_HEADER_HEX);
    let decoded: BlockHeader = bincode::deserialize(&enc).expect("decode header");
    assert_eq!(decoded.hash().to_hex(), GOLDEN_HASH);

    let block = Block::new(header, Vec::new());
    let block_hex = hex::encode(bincode::serialize(&block).expect("encode block"));
    assert_eq!(
        block_hex,
        format!("{GOLDEN_HEADER_HEX}{GOLDEN_BLOCK_TAIL_HEX}")
    );
}

// REQ-AUTH-004 — Decision: a failure means the signature carrier is inside hash(), so signing would change block identity.
#[test]
fn req_auth_004_pi_does_not_change_hash() {
    let mut header = golden_header();
    header.vdf_proof.pi = vec![0x5A; 64];
    assert_eq!(header.hash().to_hex(), GOLDEN_HASH);
    let (block, _ctx, _kp) = signed_block();
    let mut stripped = block.header.clone();
    stripped.vdf_proof.pi.clear();
    assert_eq!(stripped.hash(), block.header.hash());
    assert_eq!(stripped.signing_message(), block.header.signing_message());
}
