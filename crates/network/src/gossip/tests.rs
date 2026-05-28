use super::*;

#[test]
fn test_topic_constants() {
    assert_eq!(BLOCKS_TOPIC, "/doli/blocks/1");
    assert_eq!(TRANSACTIONS_TOPIC, "/doli/txs/1");
    assert_eq!(PRODUCERS_TOPIC, "/doli/producers/1");
    assert_eq!(VOTES_TOPIC, "/doli/votes/1");
    assert_eq!(HEARTBEATS_TOPIC, "/doli/heartbeats/1");
    assert_eq!(TIER1_BLOCKS_TOPIC, "/doli/t1/blocks/1");
    assert_eq!(HEADERS_TOPIC, "/doli/headers/1");
    assert_eq!(ATTESTATION_TOPIC, "/doli/attestations/1");
}

#[test]
fn test_region_topic_format() {
    assert_eq!(region_topic(0), "/doli/r0/blocks/1");
    assert_eq!(region_topic(1), "/doli/r1/blocks/1");
    assert_eq!(region_topic(42), "/doli/r42/blocks/1");
}

#[test]
fn test_mesh_config_invariants() {
    let config = MeshConfig {
        mesh_n: 12,
        mesh_n_low: 8,
        mesh_n_high: 24,
        gossip_lazy: 12,
    };
    assert!(config.mesh_n >= config.mesh_n_low);
    assert!(config.mesh_n <= config.mesh_n_high);
    assert!(config.gossip_lazy >= config.mesh_n);
}

#[test]
fn test_gossip_error_display() {
    let e = GossipError::Config("bad config".into());
    assert!(e.to_string().contains("bad config"));
    let e = GossipError::Subscribe("topic failed".into());
    assert!(e.to_string().contains("topic failed"));
}

#[test]
fn test_tx_batch_roundtrip() {
    let tx1 = doli_core::Transaction::new_coinbase(100, crypto::Hash::ZERO, 0, 0);
    let tx2 = doli_core::Transaction::new_coinbase(200, crypto::Hash::ZERO, 1, 0);

    let encoded = encode_tx_batch(&[tx1.clone(), tx2.clone()]);
    assert_eq!(encoded[0], TX_MSG_BATCH);

    let decoded = decode_tx_message(&encoded).expect("decode should succeed");
    assert_eq!(decoded.len(), 2);
    assert_eq!(decoded[0].hash(), tx1.hash());
    assert_eq!(decoded[1].hash(), tx2.hash());
}

#[test]
fn test_tx_single_legacy_decode() {
    let tx = doli_core::Transaction::new_coinbase(500, crypto::Hash::ZERO, 42, 0);
    let raw = tx.serialize();

    let decoded = decode_tx_message(&raw).expect("legacy decode should succeed");
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].hash(), tx.hash());
}

#[test]
fn test_tx_batch_empty_returns_none() {
    assert!(decode_tx_message(&[]).is_none());
    // Batch prefix with count=0
    let mut data = vec![TX_MSG_BATCH];
    data.extend_from_slice(&0u32.to_le_bytes());
    assert!(decode_tx_message(&data).is_none());
}

#[test]
fn test_gossipsub_creation_with_universal_mesh() {
    let keypair = libp2p::identity::Keypair::generate_ed25519();
    let mesh = MeshConfig {
        mesh_n: 12,
        mesh_n_low: 8,
        mesh_n_high: 24,
        gossip_lazy: 12,
    };
    let gs = new_gossipsub(&keypair, &mesh);
    assert!(gs.is_ok(), "gossipsub creation must succeed");
}

// ===========================================================================
// Guard tests — gossip transmit cap vs Era-0 block size invariant (INC-I-091)
// ===========================================================================
//
// RESOLVED (INC-I-091): GOSSIP_MAX_TRANSMIT_SIZE was 1 MB while consensus
// permits 2 MB blocks at Era 0. A producer could build a 1-2 MB block that
// passed validation but was rejected by gossipsub.publish()
// (PublishError::MessageTooLarge). Fixed two ways:
//   1. Gossip cap raised to BASE_BLOCK_SIZE + GOSSIP_ENVELOPE_MARGIN (~2 MB)
//      so a full Era-0 block fits.
//   2. Production gates block size to ~1 MB until
//      NetworkParams::large_block_activation_height, so blocks stay gossipable
//      by not-yet-upgraded nodes during a one-by-one rollout.
//
// These tests are now GUARDS: they pass today and fail loudly if the cap is
// ever dropped back below the Era-0 block size.
//
// SCOPE: Full-block gossip supports Era 0 only (<=2 MB). Era 1+ blocks (4-32 MB)
// require announce-then-fetch propagation (gossip the header, pull the body
// over the 16 MB sync path), NOT a larger gossip message — do NOT raise the
// cap to MAX_BLOCK_SIZE_CAP (reopens a memory-DoS surface, INC-I-009/014).
//
// Source locations:
//   gossip cap:    crates/network/src/gossip/config.rs (GOSSIP_MAX_TRANSMIT_SIZE)
//   consensus cap: crates/core/src/consensus/constants.rs:430 (BASE_BLOCK_SIZE)
//   prod gate:     bins/node/src/node/production/assembly.rs (large_block_ah)
//
// OUTPUT CONTRACT: invariant = "every consensus-valid Era-0 block is gossipable"
//   O1: GOSSIP_MAX_TRANSMIT_SIZE >= BASE_BLOCK_SIZE          (bool)
//   O2: GOSSIP_MAX_TRANSMIT_SIZE >= a concrete ~1.5 MB block (bool)
//   O3: GOSSIP_MAX_TRANSMIT_SIZE <  Era-1 block (scope bound) (bool)
// PATHS:
//   P1: cap fits Era-0 limit  -> guard passes (correct, post-fix)
//   P2: cap < Era-0 limit     -> guard fails (regression: bug reintroduced)
//   P3: cap >= Era-1 limit    -> guard fails (cap over-raised; use fetch)
// INPUT PARTITIONS:
//   Pa: Era-0 constant limit (BASE_BLOCK_SIZE)   -> test_gossip_transmit_cap_fits_era0_block
//   Pb: Era-0 vs Era-1 boundary (max_block_size) -> test_gossip_cap_supports_era0_not_larger_eras
//   Pc: concrete serialized ~1.5 MB Era-0 block  -> test_large_era0_block_fits_gossip_cap
// MATRIX: 3 outputs x 3 partitions, one assertion cell per test (O1/Pa, O3/Pb, O2/Pc)

/// Guard: gossip transmit cap MUST be >= the Era-0 block size, otherwise a
/// consensus-valid Era-0 block cannot propagate via gossip.
#[test]
fn test_gossip_transmit_cap_fits_era0_block() {
    let gossip_cap = GOSSIP_MAX_TRANSMIT_SIZE;
    let era0_block = doli_core::consensus::BASE_BLOCK_SIZE;

    assert!(
        gossip_cap >= era0_block,
        "gossip max_transmit_size ({gossip_cap} bytes) must be >= the Era-0 \
         block size BASE_BLOCK_SIZE ({era0_block} bytes), or consensus-valid \
         Era-0 blocks cannot be gossiped (INC-I-091).",
    );
}

/// Scope guard: full-block gossip supports Era 0 only. The cap fits Era 0 but
/// is intentionally NOT large enough for Era 1+ (4-32 MB) — those require
/// announce-then-fetch, not a bigger gossip message. This test documents and
/// pins that boundary.
#[test]
fn test_gossip_cap_supports_era0_not_larger_eras() {
    use doli_core::consensus::{max_block_size, BLOCKS_PER_ERA};

    let gossip_cap = GOSSIP_MAX_TRANSMIT_SIZE;

    // Era 0 fits.
    assert!(
        gossip_cap >= max_block_size(0),
        "gossip cap ({gossip_cap}) must fit the Era-0 block ({}).",
        max_block_size(0),
    );

    // Era 1+ intentionally does NOT fit full-block gossip — guard against
    // anyone bumping the cap to chase Era scaling (use announce-then-fetch).
    let era1_block = max_block_size(BLOCKS_PER_ERA);
    assert!(
        gossip_cap < era1_block,
        "gossip cap ({gossip_cap}) >= Era-1 block ({era1_block}): if Era 1+ \
         blocks are intended, switch to announce-then-fetch propagation \
         instead of enlarging the gossip message (see INC-I-091 notes).",
    );
}

/// Behavioral guard — construct a consensus-valid Era-0 block larger than the
/// OLD 1 MiB cap (~1.5 MB) and prove it now FITS the raised gossip cap. Before
/// INC-I-091 this block was un-gossipable; it must now propagate.
///
/// Strategy: create a block with a single transaction carrying enough
/// extra_data to push total serialized size past 1 MB but under 2 MB.
#[test]
fn test_large_era0_block_fits_gossip_cap() {
    use doli_core::block::{Block, BlockHeader};
    use doli_core::consensus::{max_block_size, BASE_BLOCK_SIZE};
    use doli_core::transaction::{Output, OutputType, TxType};
    use doli_core::Transaction;

    const OLD_GOSSIP_CAP: usize = 1024 * 1024; // the pre-INC-I-091 1 MiB cap

    let gossip_cap = GOSSIP_MAX_TRANSMIT_SIZE;

    // Build a minimal block skeleton to measure serialization overhead
    let empty_block = Block::new(
        BlockHeader {
            version: 2,
            prev_hash: crypto::Hash::ZERO,
            merkle_root: crypto::Hash::ZERO,
            presence_root: crypto::Hash::ZERO,
            genesis_hash: crypto::Hash::ZERO,
            timestamp: 1_700_000_000,
            slot: 1,
            producer: crypto::PublicKey::from_bytes([0u8; 32]),
            vdf_output: vdf::VdfOutput { value: Vec::new() },
            vdf_proof: vdf::VdfProof::empty(),
            missed_producers: Vec::new(),
            data_root: crypto::Hash::ZERO,
            fork_id: crypto::Hash::ZERO,
        },
        vec![],
    );

    let overhead = empty_block.size();
    // Target: 1.5 MB — above the old 1 MiB cap, below the 2 MB consensus limit
    let target_size: usize = 1_500_000;
    let payload_size = target_size.saturating_sub(overhead);

    // Create a transaction with a large extra_data payload
    let fat_tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: Vec::new(),
        outputs: vec![Output {
            output_type: OutputType::Normal,
            amount: 1_000,
            pubkey_hash: crypto::Hash::ZERO,
            lock_until: 0,
            extra_data: vec![0xAB; payload_size],
        }],
        extra_data: Vec::new(),
    };

    let block = Block::new(empty_block.header.clone(), vec![fat_tx]);
    let serialized = block.serialize();
    let block_size = serialized.len();

    // Precondition: block is within consensus limits (Era 0)
    let consensus_limit = max_block_size(0);
    assert_eq!(consensus_limit, BASE_BLOCK_SIZE);
    assert!(
        block_size <= consensus_limit,
        "Test setup error: block ({block_size} bytes) exceeds consensus limit \
         ({consensus_limit}). Reduce payload_size.",
    );

    // Precondition: this block is bigger than the OLD 1 MiB cap, so it was
    // un-gossipable before INC-I-091 — proving the test is meaningful.
    assert!(
        block_size > OLD_GOSSIP_CAP,
        "Test setup error: block ({block_size} bytes) does not exceed the old \
         1 MiB cap ({OLD_GOSSIP_CAP}); increase payload_size.",
    );

    // THE INVARIANT: a consensus-valid Era-0 block must now be gossipable.
    assert!(
        gossip_cap >= block_size,
        "REGRESSION (INC-I-091): consensus-valid Era-0 block of {block_size} \
         bytes exceeds gossip max_transmit_size of {gossip_cap} bytes. It would \
         pass validate_block() but gossipsub.publish() would reject it with \
         PublishError::MessageTooLarge. Raise GOSSIP_MAX_TRANSMIT_SIZE.",
    );
}
