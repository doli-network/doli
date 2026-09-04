//! INC-I-178 M1 — node-side ingest into the parent pool (D2) and the node-side
//! half of the D1/R3 deletions.
//!
//! The behavioural tests drive the REAL gossip entry point
//! (`Node::on_new_attestation`), so what they lock is what a peer can actually
//! cause. The tripwires cover what Rust cannot express: the absence of a symbol.
//!
//! OUTPUT CONTRACT
//!
//! F1: `Node::on_new_attestation(&mut self, data: Vec<u8>, source_peer: PeerId)`
//!   Observable outputs:
//!     O1 return — unit; carries nothing
//!     O2 `self.minute_tracker` — attendance for (attester, attestation_minute(slot))
//!     O3 `self.parent_sig_pool` — NEW: (block_hash, attester) -> [u8; 96]
//!     O4 `self.sync_manager` — attestation weight, only when derived weight > 0
//!     O5 mutable params — NONE (`data` and `source_peer` are by value)
//!     O6 persistent store writes — the archive flush only; no block/UTXO write
//!   Paths (network_events.rs:558-619):
//!     P1 member attester, known canonical block, VERIFIED 96-byte BLS -> O2, O3 set
//!     P2 member attester, known canonical block, EMPTY BLS   -> O2 set, O3 UNTOUCHED
//!     P3 member attester, known block, malformed-length BLS  -> O2 set, O3 UNTOUCHED
//!     P4 NON-member attester, known block, 96-byte BLS       -> O2 UNTOUCHED, O3 UNTOUCHED
//!   INPUT PARTITIONS:
//!     P1a a single member attester
//!     P1b two member attesters under ONE parent (the fan-out the aggregate needs),
//!         each with its OWN on-chain BLS key, so the pool holds two distinct sigs
//!     P2a the Release-N mixed-fleet bridge: an old binary sends no BLS bytes at all
//!     P3a 48 bytes — the G1 length, not the G2 length the pool stores
//!     P4a a freshly generated key that is in no ProducerSet (C19 attester bound)
//!   MATRIX 6 outputs x 4 paths: O1 is unit (nothing to assert), O5 is by-value,
//!     O4/O6 are unchanged by M1 and are M0's contract, not this file's. O2 and
//!     O3 are asserted on every one of P1-P4.
//!
//! F2: the source trees `bins/node/src/` and `crates/rpc/src/` (build inputs)
//!   Observable outputs: O1 token set only. O2-O6 NONE (read-only scan).
//!   Paths: P1 forbidden token present -> fail | P2 required token missing -> fail
//!   INPUT PARTITIONS: a D1 symbol (must be gone); an R3 constant reference (must
//!     be gone from code, tolerated in comments); an M4-OWNED guard (must survive).
//!
//! Requirement IDs: REQ-BLS-012 (drain the dead BLS surface), REQ-BLS-010
//! (liveness must not regress — attendance is recorded whether or not BLS bytes
//! arrive, so no producer is starved during the mixed-fleet window).

use std::fs;
use std::path::{Path, PathBuf};

use crypto::{BlsKeyPair, KeyPair};
use doli_core::Attestation;

use crate::inc_i_178_m0_common::{
    assemble, build_via_production, dual, make_node, register_bls, safe_build_height, N_SMALL,
};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root must resolve")
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot list {}: {e}", dir.display()))
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Source with every whole-line `//` comment removed.
fn code_only(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

// REQ-BLS-012 — Decision: a surviving `bls_sign` on the apply path means every
// applied block still pays a BLS12-381 G2 signature whose output nothing reads.
#[test]
fn m1_d1_post_commit_no_longer_signs_or_writes_the_dead_store() {
    let src = read("bins/node/src/node/apply_block/post_commit.rs");
    assert!(
        !src.contains("bls_sign"),
        "D1: post_commit.rs still calls into BLS signing"
    );
    assert!(
        !src.contains("record_with_bls"),
        "D1: post_commit.rs still writes the minute-keyed BLS store"
    );
}

// REQ-BLS-012 — Decision: if either ingest site still calls `record_with_bls`,
// the signature is going into the deleted store's shape instead of the pool.
#[test]
fn m1_d1_neither_ingest_site_calls_record_with_bls() {
    let src = read("bins/node/src/node/network_events.rs");
    assert!(
        !src.contains("record_with_bls"),
        "D1: network_events.rs still writes the minute-keyed BLS store"
    );
}

// REQ-BLS-012 — Decision: a live reference to a constant pinned at 0 keeps an
// unsatisfiable branch alive in five decoders and one encoder.
#[test]
fn m1_r3_no_production_code_references_the_deleted_constant() {
    let mut files = Vec::new();
    rust_files(&repo_root().join("bins/node/src"), &mut files);
    rust_files(&repo_root().join("crates/rpc/src"), &mut files);
    assert!(files.len() > 20, "the scan found suspiciously few sources");

    for path in files {
        let src = fs::read_to_string(&path).unwrap_or_default();
        assert!(
            !code_only(&src).contains("BITFIELD_BODY_ACTIVATION_HEIGHT"),
            "R3: {} still references BITFIELD_BODY_ACTIVATION_HEIGHT in code",
            path.display()
        );
    }
}

// REQ-BLS-010 — Decision: deleting the empty-bitfield guard here would enable the
// presence_root commitment check a milestone early, rejecting every block an
// unupgraded producer builds — a fleet-wide liveness stop, not a bug.
#[test]
fn m1_r3_the_empty_bitfield_bypass_and_its_error_survive_untouched() {
    let src = read("bins/node/src/node/validation_checks.rs");
    assert!(
        src.contains("!block.attestation_bitfield.is_empty()"),
        "M4-owned: the empty-bitfield bypass guard was removed early"
    );
    assert!(
        src.contains("presence_root mismatch"),
        "M4-owned: the presence_root commitment error was removed early"
    );
}

/// Build one real block, make it canonical, and hand back its hash and slot.
async fn canonical_block(node: &mut doli_node::node::Node) -> (crypto::Hash, u32, u64) {
    let h = safe_build_height(node);
    let (header, txs, bf) = build_via_production(node, h).await;
    let block = assemble(header, txs, bf);
    let hash = block.hash();
    let slot = block.header.slot;
    node.block_store
        .put_block_canonical(&block, h)
        .expect("put_block_canonical failed");
    (hash, slot, h)
}

fn signed(kp: &KeyPair, hash: crypto::Hash, slot: u32, height: u64, bls: Vec<u8>) -> Vec<u8> {
    let mut att = Attestation::new(hash, slot, height, 1, kp.private_key(), *kp.public_key());
    att.bls_signature = bls;
    att.to_bytes()
}

/// Since M2's D4 ingress the pool is authenticated: each attester publishes an
/// on-chain `bls_pubkey` and signs `bls_attest_msg(&hash)`, so the bytes are
/// VERIFIED before they are pooled. The parent-keying contract below is
/// unchanged — only the road into the pool now runs through the `Valid` verdict.
// REQ-BLS-012 — Decision: this is the only production write into the pool; if it is absent or keyed on anything but the parent hash, M4's encoder finds nothing and the aggregate is permanently empty.
#[tokio::test]
async fn m1_d2_ingest_records_attendance_and_files_the_signature_under_the_parent() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, height) = canonical_block(&mut node).await;
    let minute = doli_core::attestation::attestation_minute(slot);

    let (a, b) = (&producers[3], &producers[7]);
    let (bls_a, bls_b) = (BlsKeyPair::generate(), BlsKeyPair::generate());
    register_bls(&node, a.public_key(), &bls_a).await;
    register_bls(&node, b.public_key(), &bls_b).await;

    let att_a = dual(a, &bls_a, hash, slot, height);
    let att_b = dual(b, &bls_b, hash, slot, height);
    let sig_a: [u8; 96] = att_a
        .bls_signature
        .clone()
        .try_into()
        .expect("a signed attestation carries 96 G2 bytes");
    let sig_b: [u8; 96] = att_b
        .bls_signature
        .clone()
        .try_into()
        .expect("a signed attestation carries 96 G2 bytes");
    assert_ne!(
        sig_a, sig_b,
        "two attesters must not be filed under one signature"
    );

    node.on_new_attestation(att_a.to_bytes(), network::PeerId::random())
        .await;
    node.on_new_attestation(att_b.to_bytes(), network::PeerId::random())
        .await;

    let attended = node.minute_tracker.attested_in_minute(minute);
    assert!(
        attended.contains(&a.public_key()) && attended.contains(&b.public_key()),
        "attendance must be recorded exactly as before M1"
    );

    assert_eq!(
        node.parent_sig_pool.get(&hash, a.public_key()),
        Some(&sig_a),
        "the 96 BLS bytes must be retrievable under the attested block hash"
    );
    assert_eq!(
        node.parent_sig_pool.get(&hash, b.public_key()),
        Some(&sig_b)
    );
    assert_eq!(node.parent_sig_pool.parent_count(), 1);
    assert_eq!(node.parent_sig_pool.total_signatures(), 2);
}

// REQ-BLS-010 — Decision: during the Release-N mixed fleet an old binary sends no
// BLS bytes; if that costs it attendance, the bitfield and every reward
// denominator shrink to the upgraded subset overnight (spec BRIDGE rule).
#[tokio::test]
async fn m1_d2_an_empty_bls_signature_still_earns_attendance_and_never_enters_the_pool() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, height) = canonical_block(&mut node).await;
    let minute = doli_core::attestation::attestation_minute(slot);
    let old = &producers[2];

    node.on_new_attestation(
        signed(old, hash, slot, height, Vec::new()),
        network::PeerId::random(),
    )
    .await;

    assert!(
        node.minute_tracker
            .attested_in_minute(minute)
            .contains(&old.public_key()),
        "the mixed-fleet bridge requires attendance without BLS bytes"
    );
    assert!(node.parent_sig_pool.get(&hash, old.public_key()).is_none());
    assert_eq!(node.parent_sig_pool.total_signatures(), 0);
    assert_eq!(node.parent_sig_pool.parent_count(), 0);
}

// REQ-BLS-012 — Decision: a wrong-length blob copied into a [u8; 96] would either
// panic on the slice conversion or store truncated bytes that can never verify.
#[tokio::test]
async fn m1_d2_a_wrong_length_bls_blob_is_dropped_without_costing_attendance() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, height) = canonical_block(&mut node).await;
    let minute = doli_core::attestation::attestation_minute(slot);
    let p = &producers[5];

    for len in [1usize, 48, 95, 97, 192] {
        node.on_new_attestation(
            signed(p, hash, slot, height, vec![0x7Fu8; len]),
            network::PeerId::random(),
        )
        .await;
    }

    assert!(
        node.minute_tracker
            .attested_in_minute(minute)
            .contains(&p.public_key()),
        "a malformed BLS blob must not cost the attester its attendance"
    );
    assert!(node.parent_sig_pool.get(&hash, p.public_key()).is_none());
    assert_eq!(node.parent_sig_pool.total_signatures(), 0);
}

// REQ-BLS-012 — Decision: the ProducerSet membership gate is what bounds the
// pool's attester axis to N (C19); if the pool were fed before that gate, any
// peer could mint unbounded keys and grow it without limit.
#[tokio::test]
async fn m1_d2_a_non_member_attester_reaches_neither_attendance_nor_the_pool() {
    let (mut node, _producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, height) = canonical_block(&mut node).await;
    let minute = doli_core::attestation::attestation_minute(slot);
    let stranger = KeyPair::generate();

    node.on_new_attestation(
        signed(&stranger, hash, slot, height, vec![0xC3u8; 96]),
        network::PeerId::random(),
    )
    .await;

    assert!(
        !node
            .minute_tracker
            .attested_in_minute(minute)
            .contains(&stranger.public_key()),
        "a non-member must not gain attendance"
    );
    assert!(node
        .parent_sig_pool
        .get(&hash, stranger.public_key())
        .is_none());
    assert_eq!(node.parent_sig_pool.total_signatures(), 0);
}
