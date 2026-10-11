//! INC-I-237 M0 / F-10 — frozen gossip wire vectors (data only).
//!
//! OUTPUT CONTRACT: N/A — fixture file; the assertions live in
//! `inc_i_237_m0_corpus`. INPUT PARTITIONS: N/A — fixture file.
//!
//! Era sweep: every encoder below is source-identical from v6.24.0 (mainnet
//! genesis release, 2026-07-22) to v6.43.3, with bincode 1.3.3 / prost 0.13.5 /
//! serde_json 1.0.149 pinned in both Cargo.lock files. The only format era since
//! genesis is the additive `TxType::RotateBlsKey` (v6.30.0). Bytes were produced
//! by the cited encoders at tag v6.43.3 and frozen here; never regenerate them.

#![allow(dead_code)]

/// Decode a frozen hex vector.
pub fn bytes(hex: &str) -> Vec<u8> {
    assert!(hex.len().is_multiple_of(2), "odd hex length");
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex"))
        .collect()
}

/// Genesis/slot frame the vectors are fresh in: slot 1000 starts at 1_790_000_000.
pub const GENESIS_TIME: u64 = 1_789_990_000;
pub const SLOT_DURATION: u64 = 10;
pub const NOW_UNIX: u64 = 1_790_000_005;

/// Hex of `BlockHeader::hash()` for `HEADER`.
pub const HEADER_HASH: &str = "4b3a6ee2a62c4a2668836b77910475289aac820238cd27a0cfbf8121f7002539";
/// Hex of `tx.hash()` for the txs tagged 1 and 2 (the `TX_ANNOUNCE_2` payload).
pub const TX1_HASH: &str = "8ba5b83995c887bf3a2d7a647f98d3c53d6d1b1efb46c4d1da2dd5d97bb477c3";
pub const TX2_HASH: &str = "c48cf7d3e8e54ee34a6c68a738e3ff7de3b53774df1f67b480c19c6495d3de22";
/// Public keys from `KeyPair::from_seed([9;32])` and `([10;32])`.
pub const PK9: &str = "fd1724385aa0c75b64fb78cd602fa1d991fdebf76b13c58ed702eac835e9f618";
pub const PK10: &str = "43a72e714401762df66b68c26dfbdf2682aaec9f2474eca4613e424a0fbafd3c";

/// Governance vote JSON, RPC era. v6.43.3 crates/rpc/src/methods/governance.rs:52
/// `serde_json::to_vec(&params.vote)` over `VoteMessageParams` (camelCase,
/// crates/rpc/src/types/producer.rs:25); v6.24.0 governance.rs:48, same struct.
pub const VOTE_RPC: &[u8] = b"{\"version\":\"6.43.3\",\"vote\":\"approve\",\"producerId\":\"fd1724385aa0c75b64fb78cd602fa1d991fdebf76b13c58ed702eac835e9f618\",\"timestamp\":1790000000,\"signature\":\"bb\"}";

/// Garbage that is no format on any topic (first byte not 0xAA/0xBA).
pub const GARBAGE: &[u8] = &[
    0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01, 0x02, 0x03, 0xFF, 0xFE, 0x7F, 0x80, 0x55,
];

/// legacy single-tx (Transfer, 1 in / 1 out). v6.43.3 crates/core/src/transaction/core.rs:717 `Transaction::serialize` (v6.24.0 core.rs:712, same fields).
pub const TX_LEGACY_TRANSFER: &str = concat!(
    "010000000000000001000000000000002000000000000000a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1",
    "a1a1a1a1a1a1a1a1a1a1a1a10100000040000000000000005c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c",
    "5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c",
    "0000000000000000012000000000000000ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea",
    "691446d22c010000000000000000000000e9030000000000002000000000000000b2b2b2b2b2b2b2b2b2b2b2",
    "b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b20000000000000000000000000000000000000000000000",
    "00",
);
/// legacy single-tx, RotateBlsKey era (bincode ordinal 24, added v6.30.0). v6.43.3 core.rs:717.
pub const TX_LEGACY_ROTATE_BLS_ERA: &str =
    "0100000018000000000000000000000000000000000000000000000000000000";
/// minimum-size legacy single-tx (32 bytes, empty vecs). v6.43.3 core.rs:717 (v6.24.0 core.rs:712).
pub const TX_LEGACY_MIN: &str = "0100000000000000000000000000000000000000000000000000000000000000";
/// 0xBA batch, 1 tx (= TX_LEGACY_TRANSFER). v6.43.3 crates/network/src/gossip/publish.rs:120 `encode_tx_batch` (v6.24.0 publish.rs:120, file unchanged).
pub const TX_BATCH_1: &str = concat!(
    "ba0100000009010000010000000000000001000000000000002000000000000000a1a1a1a1a1a1a1a1a1a1a1",
    "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a10100000040000000000000005c5c5c5c5c5c5c5c5c5c5c",
    "5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c",
    "5c5c5c5c5c5c5c5c5c0000000000000000012000000000000000ea4a6c63e29c520abef5507b132ec5f99547",
    "76aebebe7b92421eea691446d22c010000000000000000000000e9030000000000002000000000000000b2b2",
    "b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b20000000000000000000000000000",
    "00000000000000000000",
);
/// 0xBA batch, 3 txs (output_index/amount tags 1,2,3). v6.43.3 publish.rs:120 (v6.24.0 publish.rs:120).
pub const TX_BATCH_3: &str = concat!(
    "ba0300000009010000010000000000000001000000000000002000000000000000a1a1a1a1a1a1a1a1a1a1a1",
    "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a10100000040000000000000005c5c5c5c5c5c5c5c5c5c5c",
    "5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c",
    "5c5c5c5c5c5c5c5c5c0000000000000000012000000000000000ea4a6c63e29c520abef5507b132ec5f99547",
    "76aebebe7b92421eea691446d22c010000000000000000000000e9030000000000002000000000000000b2b2",
    "b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b20000000000000000000000000000",
    "0000000000000000000009010000010000000000000001000000000000002000000000000000a1a1a1a1a1a1",
    "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a10200000040000000000000005c5c5c5c5c5c",
    "5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c",
    "5c5c5c5c5c5c5c5c5c5c5c5c5c5c0000000000000000012000000000000000ea4a6c63e29c520abef5507b13",
    "2ec5f9954776aebebe7b92421eea691446d22c010000000000000000000000ea030000000000002000000000",
    "000000b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2000000000000000000",
    "00000000000000000000000000000009010000010000000000000001000000000000002000000000000000a1",
    "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a10300000040000000000000005c",
    "5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c",
    "5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c0000000000000000012000000000000000ea4a6c63e29c520a",
    "bef5507b132ec5f9954776aebebe7b92421eea691446d22c010000000000000000000000eb03000000000000",
    "2000000000000000b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b200000000",
    "0000000000000000000000000000000000000000",
);
/// 0xAA announce, 2 hashes (txs tagged 1 and 2). v6.43.3 publish.rs:216 `encode_tx_announce` (v6.24.0 publish.rs:216).
pub const TX_ANNOUNCE_2: &str = concat!(
    "aa020000008ba5b83995c887bf3a2d7a647f98d3c53d6d1b1efb46c4d1da2dd5d97bb477c3c48cf7d3e8e54e",
    "e34a6c68a738e3ff7de3b53774df1f67b480c19c6495d3de22",
);
/// producers legacy bincode Vec<PublicKey> (seeds [9;32], [10;32]). v6.43.3 crates/network/src/service/mod.rs:491 `broadcast_producers` (deprecated, no node caller).
pub const PRODUCERS_LEGACY_BINCODE: &str = concat!(
    "02000000000000002000000000000000fd1724385aa0c75b64fb78cd602fa1d991fdebf76b13c58ed702eac8",
    "35e9f618200000000000000043a72e714401762df66b68c26dfbdf2682aaec9f2474eca4613e424a0fbafd3c",
);
/// producers protobuf ProducerSet, 1 announcement (seed [9;32], net 1, seq 5, ts 1_790_000_000). v6.43.3 crates/core/src/discovery/proto.rs:161 `encode_producer_set` (v6.24.0 proto.rs:161, discovery/ unchanged).
pub const PRODUCERS_PROTOBUF_SET: &str = concat!(
    "0a90010a20fd1724385aa0c75b64fb78cd602fa1d991fdebf76b13c58ed702eac835e9f618100118052080f7",
    "c4d5062a4007858369ae983870f560fb149b8a43bdf8accdca01c140402982e96eebfcc61c4aa534bb5fbf0a",
    "ff1247b17d04d2dcb4737f8b8af5867ae951177fba7330b4063220c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3",
    "c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3",
);
/// producers bloom digest (new(4) + both keys, 64 bits). v6.43.3 proto.rs:173 `encode_digest` (v6.24.0 proto.rs:173).
pub const PRODUCERS_BLOOM_DIGEST: &str = "0a080020010423680301100718022040";
/// block header slot 1000, ts 1_790_000_100. v6.43.3 crates/core/src/block.rs:113 `BlockHeader::serialize` (v6.24.0 block.rs:110, comment-only diff).
pub const HEADER: &str = concat!(
    "0200000020000000000000001111111111111111111111111111111111111111111111111111111111111111",
    "2000000000000000222222222222222222222222222222222222222222222222222222222222222220000000",
    "0000000033333333333333333333333333333333333333333333333333333333333333332000000000000000",
    "4444444444444444444444444444444444444444444444444444444444444444e43bb16a00000000e8030000",
    "2000000000000000fd1724385aa0c75b64fb78cd602fa1d991fdebf76b13c58ed702eac835e9f61808000000",
    "0000000055555555555555550000000000000000000000000000000020000000000000006666666666666666",
    "6666666666666666666666666666666666666666666666662000000000000000777777777777777777777777",
    "7777777777777777777777777777777777777777",
);
/// heartbeat v1, slot 1000, no witnesses. v6.43.3 crates/core/src/heartbeat.rs:360 `Heartbeat::serialize` (v6.24.0 heartbeat.rs:360, unchanged).
pub const HEARTBEAT: &str = concat!(
    "012000000000000000fd1724385aa0c75b64fb78cd602fa1d991fdebf76b13c58ed702eac835e9f618e80300",
    "0020000000000000001111111111111111111111111111111111111111111111111111111111111111888888",
    "8888888888888888888888888888888888888888888888888888888888400000000000000099999999999999",
    "9999999999999999999999999999999999999999999999999999999999999999999999999999999999999999",
    "999999999999999999999999990000000000000000",
);
/// attestation slot 1000 h 900, 96-byte BLS sig. v6.43.3 crates/core/src/attestation/message.rs:118 `to_bytes` (v6.24.0 attestation.rs:120, same struct).
pub const ATTESTATION_BLS: &str = concat!(
    "2000000000000000aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaae8030000",
    "84030000000000002000000000000000fd1724385aa0c75b64fb78cd602fa1d991fdebf76b13c58ed702eac8",
    "35e9f61803000000000000004000000000000000bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb60000000",
    "00000000cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    "cccccccccccccccccccccccc",
);
/// attestation, empty BLS sig (non-BLS attester). v6.43.3 attestation/message.rs:118 (v6.24.0 attestation.rs:120).
pub const ATTESTATION_NO_BLS: &str = concat!(
    "2000000000000000aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaae8030000",
    "84030000000000002000000000000000fd1724385aa0c75b64fb78cd602fa1d991fdebf76b13c58ed702eac8",
    "35e9f61803000000000000004000000000000000bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb00000000",
    "00000000",
);
