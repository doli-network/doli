//! INC-I-217 M1 — REQ-ROT-010 reproduction.
//!
//! Group A is the SYMPTOM and is green: a producer registered with BLS key A whose
//! node signs with key B earns nothing, permanently. Group B locks the absence of any
//! rotation path; each of its assertions is an M4 tripwire whose message names the edit
//! that must flip it.
//!
//! OUTPUT CONTRACT — full matrix in `docs/.workflow/inc-i-217-M1-output-contract.md`.
//! F1 `Node::create_and_broadcast_attestation`: O1 return, O2 `parent_sig_pool`, O3
//!    `[ATTEST_EGRESS]` WARN, O4 sync weight (witness), O5-O7 NONE.
//! F2 `Node::on_new_attestation` -> `ingest_attestation` -> `bls_verdict`: O1 unit, O2
//!    `parent_sig_pool`, O3 `bls_ingress_scorer`, O4 `[ATTEST_INGEST]` WARN, O5
//!    `minute_tracker`, O6 sync weight (out of scope), O7-O8 NONE. `BlsAttestVerdict` is
//!    `pub(crate)`; O2+O3 are its exact behavioural proxy.
//! F3 `build_block_content`: O1 body bitfield, O2 `presence_root`, O3-O4 not asserted.
//! F4 `Node::calculate_epoch_rewards`: O1 return, O3 receiver read-only, O2/O4/O5 NONE.
//! F5 `TxType::from_u32` / F6 `PendingProducerUpdate`: O1 only, pure. F7
//!    `ProducerSet::apply_pending_updates_with_cap`: O2 `bls_pubkey` (under test), O3
//!    other fields (anti-vacuity), O4 `pending_updates` drained, O1/O5/O6 NONE.
//! INPUT PARTITIONS — I1 on-chain key A vs node key B, independently generated (the
//! wallet v1/v2 `OsRng` shape). I2 the single-variable falsifier: same attester, same
//! bytes, same preimage, on-chain key corrected. I3 `DOLI_BLOCKS_PER_REWARD_EPOCH=36`.
//! I4 the F4 bitfield is the one F3 emitted, not hand-rolled. I5 `0..=255` swept through
//! `from_u32`, so "exactly 24" is measured. I6 a 48-byte BLS key, so "unchanged" is not
//! the trivially stable empty vector.
//! RED evidence: `docs/.workflow/inc-i-217-M1-test-red-evidence.txt`.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once};

use crypto::{hash::hash_with_domain, BlsKeyPair, Hash, KeyPair, PublicKey, ADDRESS_DOMAIN};
use doli_core::attestation::{
    attestation_minute, attestation_minutes_per_epoch, attestation_qualification_threshold,
};
use doli_core::consensus::reward_pool_pubkey_hash;
use doli_core::transaction::{Output, TxType};
use doli_core::{Block, BlockHeader};
use doli_node::node::Node;
use network::PeerId;
use storage::producer::{PendingProducerUpdate, ProducerInfo, ProducerSet};
use storage::{Outpoint, UtxoEntry};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

#[path = "it/inc_i_178_m0_common.rs"]
mod m0_common;

use m0_common::{
    active_at, assemble, build_via_production, dual, make_node, register_bls, safe_build_height,
    N_SMALL,
};

/// Devnet ships 4, which makes `attestation_qualification_threshold` 0 and would let a
/// 0-minute producer qualify. 36 gives 6 minutes and a threshold of 5 — the same 90%
/// rule mainnet instantiates as 54-of-60.
const BPE: &str = "36";

// ===========================================================================
// Process boot: params override + the WARN tee that makes A2 assertable.
// ===========================================================================

static CAPTURED: Mutex<String> = Mutex::new(String::new());

#[derive(Clone, Copy)]
struct Tee;

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Ok(mut c) = CAPTURED.lock() {
            c.push_str(&String::from_utf8_lossy(buf));
        }
        let mut out = std::io::stdout();
        out.write_all(buf)?;
        out.flush()?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::stdout().flush()
    }
}

impl tracing_subscriber::fmt::MakeWriter<'_> for Tee {
    type Writer = Tee;
    fn make_writer(&self) -> Tee {
        Tee
    }
}

/// Arms the two activation heights this file needs and installs a WARN subscriber that
/// writes to the process stdout AND to `CAPTURED`. The stdout half makes the production
/// token countable from outside the binary; the `CAPTURED` half makes it assertable.
fn boot() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::env::set_var("DOLI_BLOCKS_PER_REWARD_EPOCH", BPE);
        std::env::set_var("DOLI_INC_I_208_OWN_ATTESTATION_ACTIVATION_HEIGHT", "0");
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .with_writer(Tee)
            .with_ansi(false)
            .try_init();
    });
}

fn logged(needle: &str) -> bool {
    CAPTURED.lock().map(|c| c.contains(needle)).unwrap_or(false)
}

// ===========================================================================
// Fixture
// ===========================================================================

/// The REQ-ROT-010 producer (`m`, also the node's own key) and an honest contrast
/// producer (`h`), in one node, one block, one minute.
struct Fx {
    node: Node,
    m: PublicKey,
    h: KeyPair,
    h_bls: BlsKeyPair,
    node_b: Vec<u8>,
    parent: Hash,
    slot: u32,
    height: u64,
}

async fn register_bls_bytes(node: &Node, pk: &PublicKey, bytes: Vec<u8>) {
    let mut ps = node.producer_set.write().await;
    ps.get_by_pubkey_mut(pk)
        .expect("attester must be a ProducerSet member")
        .bls_pubkey = bytes;
}

/// Build one real block and make it canonical AND the tip: the post-AH encoder pools by
/// PARENT hash, so attestations have to target what the next build reads as `best_hash`.
async fn canonical_tip(node: &mut Node) -> (Hash, u32, u64) {
    let h = safe_build_height(node);
    let (header, txs, bf) = build_via_production(node, h).await;
    let block = assemble(header, txs, bf);
    let hash = block.hash();
    let slot = block.header.slot;
    node.block_store
        .put_block_canonical(&block, h)
        .expect("put_block_canonical failed");
    node.chain_state.write().await.best_hash = hash;
    (hash, slot, h)
}

async fn mismatched() -> (Fx, TempDir) {
    boot();
    let (mut node, producers, tmp) = make_node(N_SMALL).await;
    node.inc_i_178_attestation_bls_activation_height = 0;

    let key_a = BlsKeyPair::generate();
    let key_b = BlsKeyPair::generate();
    let onchain_a = key_a.public_key().as_bytes().to_vec();
    let node_b = key_b.public_key().as_bytes().to_vec();
    assert_ne!(
        onchain_a, node_b,
        "REQ-ROT-010 fixture: the on-chain key and the node key must differ"
    );
    node.bls_key = Some(key_b);

    let m = *producers[0].public_key();
    register_bls_bytes(&node, &m, onchain_a).await;

    let h = producers[3].clone();
    let h_bls = BlsKeyPair::generate();
    register_bls(&node, h.public_key(), &h_bls).await;

    let (parent, slot, height) = canonical_tip(&mut node).await;
    (
        Fx {
            node,
            m,
            h,
            h_bls,
            node_b,
            parent,
            slot,
            height,
        },
        tmp,
    )
}

fn score_of(node: &Node, peer: &PeerId) -> i32 {
    node.bls_ingress_scorer
        .get_score(peer)
        .map(|s| s.value)
        .unwrap_or(0)
}

fn attended(node: &Node, slot: u32, pk: &PublicKey) -> bool {
    node.minute_tracker
        .attested_in_minute(attestation_minute(slot))
        .contains(&pk)
}

fn ingest_token(pk: &PublicKey) -> String {
    format!("[ATTEST_INGEST] unverifiable BLS half from {:.8}", pk)
}

/// Mirror of `commit::encoder_universe_at` at/above the AH — the index order the body
/// bitfield is written in.
async fn universe_of(node: &Node, height: u64) -> Vec<PublicKey> {
    let active = active_at(node, height).await;
    doli_core::attestation_universe(&node.epoch_state.producer_list, &active)
}

fn index_of(universe: &[PublicKey], pk: &PublicKey) -> usize {
    universe
        .iter()
        .position(|p| p == pk)
        .expect("anti-vacuity: the producer must be inside the encoder universe")
}

fn next_non_epoch_start(node: &Node, mut h: u64) -> u64 {
    let bpe = node.config.network.blocks_per_reward_epoch();
    while doli_core::consensus::reward_epoch::is_epoch_start_with(h, bpe) {
        h += 1;
    }
    h
}

fn address_hash(pk: &PublicKey) -> Hash {
    hash_with_domain(ADDRESS_DOMAIN, pk.as_bytes())
}

// ===========================================================================
// GROUP A — the symptom.
// ===========================================================================

// REQ-ROT-010 (Must) — Decision: a failure here means the mismatch is NOT what silences
// the producer, so INC-I-217's root cause is misattributed and every milestone after M1
// is fixing the wrong thing.
#[tokio::test]
async fn req_rot_010_a1_a2_the_mismatched_half_is_invalid_at_both_the_egress_and_the_ingress() {
    let (mut fx, _tmp) = mismatched().await;

    // A1 own egress (INC-I-208 gate armed to 0 by `boot`): the producer's own node
    // refuses to pool its own half.
    let att = fx
        .node
        .create_and_broadcast_attestation(fx.parent, fx.slot, fx.height)
        .await
        .expect("the node's own producer must be active and weighted");
    assert!(
        fx.node.parent_sig_pool.get(&fx.parent, &fx.m).is_none(),
        "REQ-ROT-010 A1: the own-attestation egress must NOT pool a half that does not \
         verify against the on-chain key"
    );
    assert!(
        logged("[ATTEST_EGRESS] own BLS half does not verify against the on-chain key"),
        "REQ-ROT-010 A1: the egress Invalid arm must emit its production token"
    );

    // A1/A2 peer ingress, fed the EXACT bytes the egress produced.
    let peer = PeerId::random();
    let before = score_of(&fx.node, &peer);
    fx.node.on_new_attestation(att.to_bytes(), peer).await;

    assert!(
        fx.node.parent_sig_pool.get(&fx.parent, &fx.m).is_none(),
        "REQ-ROT-010 A1: the Invalid verdict must leave the parent signature pool clean"
    );
    assert!(
        score_of(&fx.node, &peer) < before,
        "REQ-ROT-010 A1: `record_invalid_bls_attestation` is reached from the Invalid arm \
         and nowhere else, so the relaying peer's score must drop (before={before}, \
         after={})",
        score_of(&fx.node, &peer)
    );
    assert!(
        logged(&ingest_token(&fx.m)),
        "REQ-ROT-010 A2: `[ATTEST_INGEST] unverifiable BLS half from <attester>` \
         (ingress.rs:112) must fire for this attester"
    );
    assert!(
        attended(&fx.node, fx.slot, &fx.m),
        "REQ-ROT-010 A3 precondition: attendance is Ed25519-authenticated and is recorded \
         BEFORE the verdict, so the missing bit cannot be explained by absence"
    );

    // Falsifier: correct ONLY the on-chain key. The same bytes, same preimage, same
    // attester now pool — so the refusal is caused by the mismatch and nothing else.
    register_bls_bytes(&fx.node, &fx.m, fx.node_b.clone()).await;
    fx.node.on_new_attestation(att.to_bytes(), peer).await;
    assert!(
        fx.node.parent_sig_pool.get(&fx.parent, &fx.m).is_some(),
        "REQ-ROT-010 A1 falsifier: with the node key published on-chain the identical \
         attestation must verify — otherwise the test is measuring something else"
    );
}

// REQ-ROT-010 (Must) — Decision: a failure means the on-chain bit does not follow the
// BLS verdict post-AH, which would mean the mismatched producer is still counted and
// INC-I-217's "earns 0 permanently" claim is wrong.
#[tokio::test]
async fn req_rot_010_a3_the_mismatched_producers_bit_is_never_set_in_the_bitfield() {
    let (mut fx, _tmp) = mismatched().await;
    let peer = PeerId::random();

    let h_att = dual(&fx.h, &fx.h_bls, fx.parent, fx.slot, fx.height);
    fx.node.on_new_attestation(h_att.to_bytes(), peer).await;
    let m_att = fx
        .node
        .create_and_broadcast_attestation(fx.parent, fx.slot, fx.height)
        .await
        .expect("own attestation");
    fx.node.on_new_attestation(m_att.to_bytes(), peer).await;

    let probe = next_non_epoch_start(&fx.node, fx.height + 1);
    let universe = universe_of(&fx.node, probe).await;
    let m_idx = index_of(&universe, &fx.m);
    let h_idx = index_of(&universe, fx.h.public_key());
    assert_ne!(m_idx, h_idx, "fixture: the two producers must differ");

    for n in 0..3u64 {
        let height = next_non_epoch_start(&fx.node, probe + n);
        let (_, _, bf) = build_via_production(&mut fx.node, height).await;
        assert!(
            !bf.is_empty(),
            "anti-vacuity: the production encoder must emit a body bitfield at h={height}"
        );
        let set = doli_core::decode_attestation_bitfield_vec(&bf, universe.len());
        assert!(
            set.contains(&h_idx),
            "anti-vacuity: the honest producer's bit must be set at h={height}, else the \
             bitfield is empty for reasons unrelated to the key mismatch"
        );
        assert!(
            !set.contains(&m_idx),
            "REQ-ROT-010 A3: the mismatched producer's bit (index {m_idx}) must never be \
             set — it was set at h={height}"
        );
    }

    // Falsifier: publish the node key on-chain, re-deliver the identical attestation,
    // and the bit appears. Without this, A3 could pass on a dead index.
    register_bls_bytes(&fx.node, &fx.m, fx.node_b.clone()).await;
    fx.node.on_new_attestation(m_att.to_bytes(), peer).await;
    let height = next_non_epoch_start(&fx.node, probe + 4);
    let (_, _, bf) = build_via_production(&mut fx.node, height).await;
    let set = doli_core::decode_attestation_bitfield_vec(&bf, universe.len());
    assert!(
        set.contains(&m_idx),
        "REQ-ROT-010 A3 falsifier: once the on-chain key matches, index {m_idx} must be \
         set at h={height}"
    );
}

/// The epoch window under test: an epoch strictly after genesis, every height filled.
struct Window {
    epoch: u64,
    start: u64,
    end: u64,
}

fn window(node: &Node) -> Window {
    let bpe = node.config.network.blocks_per_reward_epoch();
    let epoch = (node.config.network.genesis_blocks() / bpe) + 2;
    assert!(epoch > 0, "epoch 0 auto-qualifies every producer");
    Window {
        epoch,
        start: epoch * bpe,
        end: (epoch + 1) * bpe,
    }
}

/// The epoch scan reads `slot`, `presence_root` and the body bitfield; the rest is
/// fixture. The bitfield handed in is the one the REAL production encoder emitted.
fn stored_block(height: u64, producer: &PublicKey, root: Hash, bitfield: Vec<u8>) -> Block {
    let header = BlockHeader {
        version: 2,
        prev_hash: crypto::hash::hash(&height.to_le_bytes()),
        merkle_root: Hash::ZERO,
        presence_root: root,
        genesis_hash: Hash::ZERO,
        timestamp: 1_700_000_000 + height,
        slot: height as u32,
        producer: *producer,
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

async fn seed_reward_pool(node: &Node, amount: u64) {
    let entry = UtxoEntry {
        output: Output::normal(amount, reward_pool_pubkey_hash()),
        height: 0,
        is_coinbase: true,
        is_epoch_reward: false,
    };
    let mut utxo = node.utxo_set.write().await;
    utxo.insert(
        Outpoint::new(crypto::hash::hash(b"inc-i-217-m1-pool"), 0),
        entry,
    )
    .expect("insert pool UTXO");
}

// REQ-ROT-010 (Must) — Decision: a failure means a 0-minute producer still collects an
// epoch reward, which would contradict INC-I-217's headline loss and make the whole
// rotation feature unnecessary.
#[tokio::test]
async fn req_rot_010_a4_the_mismatched_producer_never_qualifies_for_an_epoch_reward() {
    let (mut fx, _tmp) = mismatched().await;
    let peer = PeerId::random();

    let h_att = dual(&fx.h, &fx.h_bls, fx.parent, fx.slot, fx.height);
    fx.node.on_new_attestation(h_att.to_bytes(), peer).await;
    let m_att = fx
        .node
        .create_and_broadcast_attestation(fx.parent, fx.slot, fx.height)
        .await
        .expect("own attestation");
    fx.node.on_new_attestation(m_att.to_bytes(), peer).await;

    let probe = next_non_epoch_start(&fx.node, fx.height + 1);
    let (header, txs, bf) = build_via_production(&mut fx.node, probe).await;
    let built = assemble(header, txs, bf);
    let bitfield = built.attestation_bitfield.clone();
    let root = built.header.presence_root;
    assert!(
        !bitfield.is_empty(),
        "anti-vacuity: encoder emitted no body"
    );

    let bpe = fx.node.config.network.blocks_per_reward_epoch();
    let threshold = attestation_qualification_threshold(bpe);
    assert!(
        threshold > 0,
        "anti-vacuity: a threshold of 0 admits a 0-minute producer; \
         DOLI_BLOCKS_PER_REWARD_EPOCH did not take effect (bpe={bpe})"
    );

    // Index parity: the encoder writes bits over `epoch_state.producer_list`, and
    // `calculate_epoch_rewards` decodes them over the same list. If these ever diverge,
    // bit i names a different producer on each side and both assertions below are noise.
    let universe = universe_of(&fx.node, probe).await;
    assert_eq!(
        universe, fx.node.epoch_state.producer_list,
        "REQ-ROT-010 A4 precondition: encoder universe and reward decoder list must match"
    );

    let w = window(&fx.node);
    seed_reward_pool(&fx.node, 10_000_000_000).await;
    for h in w.start..w.end {
        let b = stored_block(h, &fx.m, root, bitfield.clone());
        fx.node
            .block_store
            .put_block_canonical(&b, h)
            .expect("put_block_canonical");
    }

    let outputs = fx
        .node
        .calculate_epoch_rewards(w.epoch)
        .await
        .expect("the window is complete, so the scan must not fail-fast");

    let h_addr = address_hash(fx.h.public_key());
    let m_addr = address_hash(&fx.m);
    assert!(
        outputs.iter().any(|(_, a)| *a == h_addr),
        "anti-vacuity: the honest producer must qualify (threshold {threshold} of {} \
         minutes), otherwise Tier 1 is empty and the median fallback — not the BLS \
         verdict — decides",
        attestation_minutes_per_epoch(bpe)
    );
    assert!(
        !outputs.iter().any(|(_, a)| *a == m_addr),
        "REQ-ROT-010 A4: the mismatched producer attested 0 minutes and must not appear in \
         the epoch reward outputs"
    );
    assert_eq!(
        fx.node.epoch_state.producer_list.len(),
        N_SMALL,
        "O3: the reward scan must not mutate node state"
    );
}

// REQ-ROT-010 (Must) — Decision: a failure means the 90% rule mainnet runs as 54-of-60
// has moved, so A4's scaled window no longer stands in for the production shape.
#[test]
fn req_rot_010_a4_the_mainnet_qualification_shape_is_54_of_60() {
    assert_eq!(attestation_minutes_per_epoch(360), 60);
    assert_eq!(attestation_qualification_threshold(360), 54);
    assert_eq!(attestation_minutes_per_epoch(36), 6);
    assert_eq!(attestation_qualification_threshold(36), 5);
}

// ===========================================================================
// GROUP B — no rotation path exists. Every assertion is an M4 tripwire.
// ===========================================================================

const M4_TXTYPE: &str = "REQ-ROT-010/M4 tripwire: ordinal 32 now decodes — M4 has landed; \
                         flip this assertion to is_some() and move it to the M4 suite";

// REQ-ROT-010 (Must) — Decision: a failure tells the next agent that the wire surface
// already moved, so M3's golden ordinal table would be frozen against the wrong shape.
#[test]
fn req_rot_010_b1_ordinal_32_does_not_decode_today() {
    assert!(TxType::from_u32(32).is_none(), "{M4_TXTYPE}");
}

/// Exhaustive, NO `_` arm: adding a `TxType` variant fails this file's BUILD, which is
/// the loudest form an M4 tripwire can take.
fn ordinal(t: TxType) -> u32 {
    match t {
        TxType::Transfer => 0,
        TxType::Registration => 1,
        TxType::Exit => 2,
        TxType::ClaimReward => 3,
        TxType::ClaimBond => 4,
        TxType::SlashProducer => 5,
        TxType::Coinbase => 6,
        TxType::AddBond => 7,
        TxType::RequestWithdrawal => 8,
        TxType::ClaimWithdrawal => 9,
        TxType::EpochReward => 10,
        TxType::RemoveMaintainer => 11,
        TxType::AddMaintainer => 12,
        TxType::DelegateBond => 13,
        TxType::RevokeDelegation => 14,
        TxType::ProtocolActivation => 15,
        TxType::PriceAttestation => 16,
        TxType::MintAsset => 17,
        TxType::BurnAsset => 18,
        TxType::CreatePool => 19,
        TxType::AddLiquidity => 20,
        TxType::RemoveLiquidity => 21,
        TxType::Swap => 22,
        TxType::ZKSettle => 31,
    }
}

// REQ-ROT-010 (Must) — Decision: a failure means a TxType was added, removed, or moved
// under an existing name; M3's wire freeze would lock a surface nobody reviewed, and a
// moved ordinal silently re-maps every serialized transaction on disk and on the wire.
#[test]
fn req_rot_010_b1_the_wire_carries_exactly_24_decodable_tx_types() {
    let mut live: Vec<u32> = Vec::new();
    for v in 0u32..=255 {
        if let Some(t) = TxType::from_u32(v) {
            assert_eq!(
                ordinal(t),
                v,
                "REQ-ROT-010: ordinal table inconsistent at {v}"
            );
            live.push(v);
        }
    }
    assert_eq!(
        live.len(),
        24,
        "REQ-ROT-010/M4 tripwire: the decodable TxType count changed to {} ({live:?}) — if \
         M4 added RotateBlsKey, raise this to 25 and move it to the M4 suite",
        live.len()
    );
    assert_eq!(
        live.last(),
        Some(&31),
        "REQ-ROT-010: 31 (ZKSettle) is the highest live ordinal, so 32 is the next free one"
    );
}

/// Exhaustive, NO `_` arm: adding a `PendingProducerUpdate` variant fails the BUILD.
fn tag(u: &PendingProducerUpdate) -> &'static str {
    match u {
        PendingProducerUpdate::Register { .. } => "Register",
        PendingProducerUpdate::Exit { .. } => "Exit",
        PendingProducerUpdate::Slash { .. } => "Slash",
        PendingProducerUpdate::AddBond { .. } => "AddBond",
        PendingProducerUpdate::DelegateBond { .. } => "DelegateBond",
        PendingProducerUpdate::RevokeDelegation { .. } => "RevokeDelegation",
        PendingProducerUpdate::RequestWithdrawal { .. } => "RequestWithdrawal",
    }
}

const KEY_A: [u8; 48] = [0xA1; 48];

fn all_pending(target: &PublicKey, other: &PublicKey) -> Vec<PendingProducerUpdate> {
    let fresh = ProducerInfo::new(*target, 0, 1_000, (crypto::hash::hash(b"rot"), 0), 0, 1_000);
    vec![
        PendingProducerUpdate::Register {
            info: Box::new(fresh),
            height: 1,
        },
        PendingProducerUpdate::Exit {
            pubkey: *target,
            height: 1,
        },
        PendingProducerUpdate::Slash {
            pubkey: *target,
            height: 1,
        },
        PendingProducerUpdate::AddBond {
            pubkey: *target,
            outpoints: vec![(crypto::hash::hash(b"bond"), 0)],
            bond_unit: 1_000,
            creation_slot: 1,
        },
        PendingProducerUpdate::DelegateBond {
            delegator: *target,
            delegate: *other,
            bond_count: 1,
        },
        PendingProducerUpdate::RevokeDelegation { delegator: *target },
        PendingProducerUpdate::RequestWithdrawal {
            pubkey: *target,
            bond_count: 1,
            bond_unit: 1_000,
        },
    ]
}

// REQ-ROT-010 (Must) — Decision: a failure means the deferred producer-mutation surface
// changed, and M7's rotation arm would be written against a set nobody enumerated.
#[test]
fn req_rot_010_b2_pending_producer_update_carries_exactly_7_variants() {
    let target = *KeyPair::generate().public_key();
    let other = *KeyPair::generate().public_key();
    let mut distinct: Vec<&str> = all_pending(&target, &other).iter().map(tag).collect();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        7,
        "REQ-ROT-010/M4 tripwire: PendingProducerUpdate now has {} distinct variants \
         ({distinct:?}) — if M7 added RotateBlsKey, raise this to 8 and move it to the M7 \
         suite",
        distinct.len()
    );
}

// REQ-ROT-010 (Must) — Decision: a failure means SOME deferred producer mutation already
// rewrites a registered producer's BLS key, which would make the whole rotation feature
// redundant and would be a live consensus surface nobody has reviewed.
#[test]
fn req_rot_010_b3_no_pending_update_variant_rewrites_a_registered_producers_bls_key() {
    let target = *KeyPair::generate().public_key();
    let other = *KeyPair::generate().public_key();

    for update in all_pending(&target, &other) {
        let name = tag(&update);
        let mut ps = ProducerSet::new();
        ps.register_genesis_producer(target, 4, 1_000)
            .expect("register target");
        ps.register_genesis_producer(other, 4, 1_000)
            .expect("register other");
        ps.get_by_pubkey_mut(&target).expect("target").bls_pubkey = KEY_A.to_vec();
        if name == "RevokeDelegation" {
            // Nothing to revoke otherwise: the variant would no-op and prove nothing.
            ps.queue_update(PendingProducerUpdate::DelegateBond {
                delegator: target,
                delegate: other,
                bond_count: 1,
            });
            ps.apply_pending_updates_with_cap(0);
        }
        let before = ps.get_by_pubkey(&target).expect("target").clone();

        ps.queue_update(update);
        ps.apply_pending_updates_with_cap(0);

        let after = ps.get_by_pubkey(&target).expect("target must survive");
        assert_eq!(
            after.bls_pubkey,
            KEY_A.to_vec(),
            "REQ-ROT-010 B3: PendingProducerUpdate::{name} changed the BLS key of an \
             already-registered producer"
        );
        // O3 anti-vacuity: the variant must actually have done something, or "unchanged"
        // is free. Register is the exception — it is REFUSED for an existing key, and
        // that refusal IS the finding.
        if name != "Register" {
            assert!(
                after.bond_count != before.bond_count
                    || after.status != before.status
                    || after.delegated_bonds != before.delegated_bonds
                    || after.delegated_to != before.delegated_to
                    || after.withdrawal_pending_count != before.withdrawal_pending_count,
                "anti-vacuity: PendingProducerUpdate::{name} moved nothing, so it never \
                 reached the producer and proves nothing about bls_pubkey"
            );
        }
    }
}

// B3 supplement — source scan. Not a substitute for the behavioural test above.

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root must resolve")
}

/// Every non-test `.rs` file under the given roots, with whole-line `//` comments
/// stripped so a tombstone comment cannot satisfy or break a scan.
fn production_sources(roots: &[&str]) -> Vec<(String, Vec<String>)> {
    let root = repo_root();
    let mut stack: Vec<PathBuf> = roots.iter().map(|r| root.join(r)).collect();
    let mut out = Vec::new();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            if p.is_dir() {
                if name != "tests" && name != "target" {
                    stack.push(p);
                }
            } else if name.ends_with(".rs") && name != "tests.rs" {
                let Ok(src) = std::fs::read_to_string(&p) else {
                    continue;
                };
                let rel = p
                    .strip_prefix(&root)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .to_string();
                let lines = src
                    .lines()
                    .filter(|l| !l.trim_start().starts_with("//"))
                    .map(|l| l.to_string())
                    .collect();
                out.push((rel, lines));
            }
        }
    }
    out
}

// REQ-ROT-010 (Must) — Decision: a failure means a fifth write site appeared, so the
// behavioural enumeration above is no longer exhaustive and B3's conclusion is unsound.
#[test]
fn req_rot_010_b3_registration_is_the_only_writer_of_bls_pubkey() {
    let mut sites: BTreeMap<String, usize> = BTreeMap::new();
    let mut fresh_value_writes = 0usize;

    for (file, lines) in production_sources(&["crates", "bins"]) {
        for (i, line) in lines.iter().enumerate() {
            let Some(pos) = line.find(".bls_pubkey") else {
                continue;
            };
            let rest = line[pos + ".bls_pubkey".len()..].trim_start();
            if !rest.starts_with('=') || rest.starts_with("==") {
                continue;
            }
            *sites.entry(file.clone()).or_default() += 1;
            let lo = i.saturating_sub(15);
            if lines[lo..i]
                .iter()
                .any(|l| l.contains("ProducerInfo::new_with_bonds("))
            {
                fresh_value_writes += 1;
            }
        }
    }

    let expected: BTreeMap<String, usize> = [
        ("bins/node/src/node/apply_block/genesis_completion.rs", 1),
        ("bins/node/src/node/apply_block/tx_processing.rs", 1),
        ("bins/node/src/node/rewards.rs", 2),
    ]
    .into_iter()
    .map(|(f, n)| (f.to_string(), n))
    .collect();

    assert_eq!(
        sites, expected,
        "REQ-ROT-010 B3 supplement: the set of production `.bls_pubkey =` write sites \
         changed. Four are expected, all on a freshly constructed ProducerInfo: the \
         Registration arm and the genesis-completion arm, each mirrored in the \
         rebuild-from-blocks path"
    );
    assert_eq!(
        fresh_value_writes, 4,
        "REQ-ROT-010 B3 supplement: every write must target a ProducerInfo built by \
         `new_with_bonds` within the preceding 15 lines — a value about to be registered, \
         never a member already in the set. {fresh_value_writes} of 4 qualify"
    );
}
