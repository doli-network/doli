//! Tests for M11 `getOracleStatus`.
//!
//! Split from `tests_oracle.rs` so each test file stays within the
//! 800-LOC budget (Rule 19). Shared fixtures live in
//! `super::tests::*` and are marked `pub(super)` to cross the
//! `tests`/`tests_m11` module boundary inside the `oracle` parent.
//!
//! OUTPUT CONTRACT:
//!   getOracleStatus() -> { active, health, trust_model, structural_share,
//!                          sunset_threshold, sunset_triggered,
//!                          last_update_height, attester_count,
//!                          activation_height, centralization_disclosure }
//!
//! INPUT PARTITIONS:
//!   activation       = { pre (u64::MAX), post }
//!   structural_share = { >=6000 bps (healthy), 5500-5999 (warning),
//!                         <5500 bps (halted) }
//!   utxo_state       = { has OraclePrice UTXO, none, multiple-pairs }
//!   attester_window  = { current_epoch=0 (no closed),
//!                        attestations in closed epoch,
//!                        attestations in current epoch (excluded) }

use super::tests::{build_m10_ctx, insert_attestation_block, make_attestation_tx, pair_id_fixture};
use serde_json::Value;
#[allow(unused_imports)]
use storage::{Outpoint, UtxoEntry};

// ---------- partition: activation = pre (u64::MAX) → active = false ----------
#[tokio::test]
async fn m11_pre_activation_returns_active_false() {
    let t = build_m10_ctx();
    // Default ctx has oracle_activation_height = u64::MAX (mainnet
    // default per NetworkParams). current_height = 0.
    let resp = t.ctx.get_oracle_status(Value::Null).await.unwrap();
    assert!(
        !resp["active"].as_bool().unwrap(),
        "pre-activation must report active=false (got: {:?})",
        resp["active"]
    );
    assert_eq!(
        resp["activation_height"].as_u64().unwrap(),
        u64::MAX,
        "activation_height must echo NetworkParams.oracle_activation_height (= u64::MAX pre-activation)"
    );
}

// ---------- partition: trust_model byte-equality + sunset_threshold = 0.55 ----------
#[tokio::test]
async fn m11_trust_model_and_sunset_threshold_locked() {
    let t = build_m10_ctx();
    let resp = t.ctx.get_oracle_status(Value::Null).await.unwrap();
    assert_eq!(resp["trust_model"].as_str().unwrap(), "structural-anchored");
    // 5500 bps / 10_000 = 0.55 exactly (no float drift; 5500/10000 is
    // representable in IEEE 754).
    assert_eq!(
        resp["sunset_threshold"].as_f64().unwrap(),
        0.55,
        "sunset_threshold must be SUNSET_THRESHOLD_BPS / 10_000 = 0.55"
    );
}

// ---------- partition: structural_share < 0.55 (no eligible bonds) ----------
#[tokio::test]
async fn m11_sunset_triggered_when_no_eligible_bonds() {
    let t = build_m10_ctx();
    // No producer_set attached, no bond_snapshot → compute_structural_share_bps
    // returns None → sunset_triggered=true, structural_share=0.0.
    let resp = t.ctx.get_oracle_status(Value::Null).await.unwrap();
    assert!(resp["sunset_triggered"].as_bool().unwrap());
    assert_eq!(resp["structural_share"].as_f64().unwrap(), 0.0);
}

// ---------- partition: centralization_disclosure byte-equal to spec §6 ----------
#[tokio::test]
async fn m11_centralization_disclosure_byte_equal_to_spec() {
    let t = build_m10_ctx();
    let resp = t.ctx.get_oracle_status(Value::Null).await.unwrap();
    let disclosure = resp["centralization_disclosure"].as_str().unwrap();

    // Drift gate: extract spec §6 paragraph from the spec file and
    // assert byte-equality. The spec uses Markdown blockquote syntax
    // ("> ") to format the disclosure; we strip those prefixes to
    // compare bare text. If the spec ever changes the disclosure, this
    // test fails and forces the production CENTRALIZATION_DISCLOSURE
    // const to be updated in lockstep.
    let spec = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../specs/oracle-structural-anchored-economics.md"
    ))
    .expect("spec file must be readable from repo root");
    let spec_disclosure = extract_section6_disclosure(&spec);
    assert_eq!(
        disclosure, spec_disclosure,
        "centralization_disclosure must byte-equal the §6 paragraph in \
         specs/oracle-structural-anchored-economics.md (drift gate)"
    );
}

// ---------- partition: pinned-hash third tripwire (defense-in-depth) ----------
// AUDIT-P3-002: the byte-equal drift gate above (vs spec §6) catches
// UNILATERAL edits to either the spec or the production constant. A
// coordinated dual-edit (malicious or careless) updates both files
// together and the byte-equal gate passes silently.
//
// Pin a BLAKE3 hash of the canonical disclosure text as a THIRD source.
// Any change to the disclosure now requires updating three places: the
// spec markdown, the production constant, AND this pinned hash. Code
// review across three independent change sites is the human gate the
// audit asked for.
//
// To update: rebuild the disclosure, hash it (blake3), paste hex below.
#[tokio::test]
async fn m11_centralization_disclosure_pinned_hash() {
    let t = build_m10_ctx();
    let resp = t.ctx.get_oracle_status(Value::Null).await.unwrap();
    let disclosure = resp["centralization_disclosure"].as_str().unwrap();

    const PINNED_BLAKE3_HEX: &str =
        "289a18e0830fba7f851fea73b5577a8ba649c1fbe6690f638d01aa9daa1651c6";
    let actual = ::crypto::hash::hash(disclosure.as_bytes()).to_hex();
    assert_eq!(
        actual, PINNED_BLAKE3_HEX,
        "AUDIT-P3-002: centralization_disclosure BLAKE3 hash drifted. \
         If you intentionally updated the disclosure text in BOTH the spec \
         (specs/oracle-structural-anchored-economics.md §S6) AND the production \
         constant (crates/rpc/src/methods/oracle_status.rs::CENTRALIZATION_DISCLOSURE), \
         compute the new BLAKE3 hex and update PINNED_BLAKE3_HEX here. This \
         three-file change set is the dual-edit defense the audit required."
    );
}

/// Extract the verbatim §6 disclosure paragraph from the spec markdown.
/// Strips the "> " blockquote prefix and joins the two paragraphs with
/// a single blank line, matching the production CENTRALIZATION_DISCLOSURE
/// const format.
fn extract_section6_disclosure(spec: &str) -> String {
    let s6_start = spec
        .find("## S6 Centralization Disclosure")
        .expect("§6 header");
    let rest = &spec[s6_start..];
    let s7_start = rest.find("\n## S7 ").expect("§7 header");
    let s6_body = &rest[..s7_start];

    let mut lines: Vec<String> = Vec::new();
    for line in s6_body.lines() {
        if let Some(stripped) = line.strip_prefix("> ") {
            lines.push(stripped.to_string());
        } else if line.trim_start() == ">" {
            lines.push(String::new());
        }
    }
    lines.join("\n")
}

// ---------- partition: last_update_height = cached value from state_db meta ----------
// AUDIT-P2-001: getOracleStatus now reads the cached last_update_height
// from state_db meta (META_ORACLE_LAST_UPDATE_HEIGHT), written by the
// aggregator after each successful OraclePrice UTXO insert. This
// replaces the previous full-UTXO-set scan (iter_all()) — an unbounded
// DoS surface on an unauthenticated public RPC.
//
// The aggregator writes the HEIGHT OF THE WRITE — which by construction
// equals the latest OraclePrice insert height (the previous max-scan
// computed the same value, since the aggregator was the only writer).
#[tokio::test]
async fn m11_last_update_height_from_state_db_meta_cache() {
    let t = build_m10_ctx();

    // Simulate aggregator writes at successive epoch boundaries:
    // first at h=500 (for pair_a), then at h=1200 (for pair_b).
    // The cache holds the latest write (1200).
    t.ctx
        .state_db
        .as_ref()
        .expect("state_db is wired in build_m10_ctx")
        .put_oracle_last_update_height(500);
    t.ctx
        .state_db
        .as_ref()
        .unwrap()
        .put_oracle_last_update_height(1200);

    let resp = t.ctx.get_oracle_status(Value::Null).await.unwrap();
    assert_eq!(
        resp["last_update_height"].as_u64().unwrap(),
        1200,
        "must report the cached last_update_height (latest aggregator write); \
         no longer derived from scanning the entire UTXO set"
    );
}

// ---------- partition: last_update_height = null when no OraclePrice UTXOs ----------
#[tokio::test]
async fn m11_last_update_height_null_when_no_oracle_price_utxos() {
    let t = build_m10_ctx();
    let resp = t.ctx.get_oracle_status(Value::Null).await.unwrap();
    assert!(
        resp["last_update_height"].is_null(),
        "must be null when no OraclePrice UTXOs exist"
    );
}

// ---------- partition: attester_count from most-recently CLOSED epoch ----------
#[tokio::test]
async fn m11_attester_count_from_closed_epoch() {
    let t = build_m10_ctx();
    let pair_id = pair_id_fixture();
    // Mainnet blocks_per_reward_epoch = 360.
    // Place current_height in epoch 2 (height >= 720). Most-recently
    // closed epoch is 1 (heights [360, 720)).
    t.ctx.chain_state.write().await.best_height = 800;

    // Two distinct attesters in epoch 1
    insert_attestation_block(
        &t.block_store,
        360,
        vec![make_attestation_tx([0xAAu8; 32], pair_id, 1, 100)],
    );
    insert_attestation_block(
        &t.block_store,
        361,
        vec![make_attestation_tx([0xBBu8; 32], pair_id, 1, 200)],
    );
    // Same signer again in epoch 1 — should NOT inflate the count
    insert_attestation_block(
        &t.block_store,
        362,
        vec![make_attestation_tx([0xAAu8; 32], pair_id, 1, 150)],
    );
    // An attestation in epoch 2 (the CURRENT epoch) — should NOT count
    // because the query targets the closed epoch (=1).
    insert_attestation_block(
        &t.block_store,
        720,
        vec![make_attestation_tx([0xCCu8; 32], pair_id, 2, 300)],
    );

    let resp = t.ctx.get_oracle_status(Value::Null).await.unwrap();
    assert_eq!(
        resp["attester_count"].as_u64().unwrap(),
        2,
        "attester_count must be distinct-signer count over the most-recently CLOSED epoch (1), got {:?}",
        resp["attester_count"]
    );
}

// ---------- partition: attester_count=0 before first epoch closes ----------
#[tokio::test]
async fn m11_attester_count_zero_at_genesis() {
    let t = build_m10_ctx();
    // Default best_height=0 → current_epoch=0 → no closed epoch yet.
    let resp = t.ctx.get_oracle_status(Value::Null).await.unwrap();
    assert_eq!(
        resp["attester_count"].as_u64().unwrap(),
        0,
        "no epoch has closed yet (current_epoch=0)"
    );
}

// ---------- partition: structural_share >= 55% → active=true via inner helper ----------
//
// The full RPC path embeds the mainnet `STRUCTURAL_PUBKEY_HASHES_HEX`
// constant (a hash preimage that can't be forged with arbitrary
// keypairs). The pure-function helper `build_oracle_status_response`
// accepts an arbitrary structural set so this partition can be exercised
// with mock hashes. The handler's wiring is exercised by
// `m11_pre_activation_returns_active_false` and the other partition
// tests.
#[test]
fn m11_active_true_when_post_activation_and_structural_share_at_threshold() {
    let mock_struct_hash = crypto::hash::hash_with_domain(b"MOCK_STRUCT", b"N1");
    let other_hash = crypto::hash::hash_with_domain(b"MOCK_OTHER", b"X");

    let mut bond_snapshot = std::collections::HashMap::new();
    // 6000 (structural) + 4000 (other) = 10000 → 60% structural share.
    bond_snapshot.insert(mock_struct_hash, 6_000u64);
    bond_snapshot.insert(other_hash, 4_000u64);

    let mut registered_at = std::collections::HashMap::new();
    // Both registered well before the current epoch's "one_epoch_ago"
    // threshold so they're eligible.
    registered_at.insert(mock_struct_hash, 0u64);
    registered_at.insert(other_hash, 0u64);

    // current_height inside epoch 5; activation_height=0 (post-activation).
    let resp = super::build_oracle_status_response(super::OracleStatusInputs {
        current_height: 1_800,
        activation_height: 0,
        structural_hashes: &[mock_struct_hash],
        registered_at: &registered_at,
        bond_snapshot: &bond_snapshot,
        blocks_per_epoch: 360,
        last_update_height: Some(1_790),
        attester_count: 7,
    });

    assert!(
        resp["active"].as_bool().unwrap(),
        "post-activation + structural_share >= 55% must yield active=true (got: {:?})",
        resp
    );
    assert!(!resp["sunset_triggered"].as_bool().unwrap());
    assert!(
        (resp["structural_share"].as_f64().unwrap() - 0.60).abs() < 1e-9,
        "expected structural_share=0.60, got {}",
        resp["structural_share"]
    );
    assert_eq!(resp["last_update_height"].as_u64().unwrap(), 1_790);
    assert_eq!(resp["attester_count"].as_u64().unwrap(), 7);
    assert_eq!(resp["activation_height"].as_u64().unwrap(), 0);
    // D.3: health should be "healthy" at 60% share.
    assert_eq!(resp["health"].as_str().unwrap(), "healthy");
}

// ---------- partition: structural_share < 55% via inner helper ----------
#[test]
fn m11_sunset_triggered_when_structural_share_below_threshold() {
    let mock_struct_hash = crypto::hash::hash_with_domain(b"MOCK_STRUCT", b"N1");
    let other_hash = crypto::hash::hash_with_domain(b"MOCK_OTHER", b"X");

    // 5000 / 10000 = 50% (below 55%)
    let mut bond_snapshot = std::collections::HashMap::new();
    bond_snapshot.insert(mock_struct_hash, 5_000u64);
    bond_snapshot.insert(other_hash, 5_000u64);
    let mut registered_at = std::collections::HashMap::new();
    registered_at.insert(mock_struct_hash, 0u64);
    registered_at.insert(other_hash, 0u64);

    let resp = super::build_oracle_status_response(super::OracleStatusInputs {
        current_height: 1_800,
        activation_height: 0,
        structural_hashes: &[mock_struct_hash],
        registered_at: &registered_at,
        bond_snapshot: &bond_snapshot,
        blocks_per_epoch: 360,
        last_update_height: None,
        attester_count: 0,
    });

    assert!(resp["sunset_triggered"].as_bool().unwrap());
    assert!(!resp["active"].as_bool().unwrap());
    assert!(
        (resp["structural_share"].as_f64().unwrap() - 0.50).abs() < 1e-9,
        "expected structural_share=0.50, got {}",
        resp["structural_share"]
    );
    // D.3: health should be "halted_recoverable" at 50% share.
    assert_eq!(resp["health"].as_str().unwrap(), "halted_recoverable");
}

// ---------- partition: response has all 10 documented fields ----------
#[tokio::test]
async fn m11_response_shape_has_all_documented_fields() {
    let t = build_m10_ctx();
    let resp = t.ctx.get_oracle_status(Value::Null).await.unwrap();
    let obj = resp.as_object().expect("must be JSON object");
    for field in [
        "active",
        "health",
        "trust_model",
        "structural_share",
        "sunset_threshold",
        "sunset_triggered",
        "last_update_height",
        "attester_count",
        "activation_height",
        "centralization_disclosure",
    ] {
        assert!(
            obj.contains_key(field),
            "M11 response must contain field {:?}, got keys={:?}",
            field,
            obj.keys().collect::<Vec<_>>()
        );
    }
}

// ===========================================================================
// D.3 — RPC health field tests
// ===========================================================================

// OUTPUT CONTRACT: health field returns "healthy" when share >= 60%
#[test]
fn m11_health_healthy_above_warning_threshold() {
    let mock_struct_hash = crypto::hash::hash_with_domain(b"MOCK_STRUCT", b"N1");
    let other_hash = crypto::hash::hash_with_domain(b"MOCK_OTHER", b"X");

    // 7000 / 10000 = 70% (well above 60%)
    let mut bond_snapshot = std::collections::HashMap::new();
    bond_snapshot.insert(mock_struct_hash, 7_000u64);
    bond_snapshot.insert(other_hash, 3_000u64);
    let mut registered_at = std::collections::HashMap::new();
    registered_at.insert(mock_struct_hash, 0u64);
    registered_at.insert(other_hash, 0u64);

    let resp = super::build_oracle_status_response(super::OracleStatusInputs {
        current_height: 1_800,
        activation_height: 0,
        structural_hashes: &[mock_struct_hash],
        registered_at: &registered_at,
        bond_snapshot: &bond_snapshot,
        blocks_per_epoch: 360,
        last_update_height: None,
        attester_count: 0,
    });

    assert_eq!(resp["health"].as_str().unwrap(), "healthy");
}

// OUTPUT CONTRACT: health field returns "warning" when share 55-59%
#[test]
fn m11_health_warning_in_warning_zone() {
    let mock_struct_hash = crypto::hash::hash_with_domain(b"MOCK_STRUCT", b"N1");
    let other_hash = crypto::hash::hash_with_domain(b"MOCK_OTHER", b"X");

    // 5700 / 10000 = 57% (in warning zone: 55-59.99%)
    let mut bond_snapshot = std::collections::HashMap::new();
    bond_snapshot.insert(mock_struct_hash, 5_700u64);
    bond_snapshot.insert(other_hash, 4_300u64);
    let mut registered_at = std::collections::HashMap::new();
    registered_at.insert(mock_struct_hash, 0u64);
    registered_at.insert(other_hash, 0u64);

    let resp = super::build_oracle_status_response(super::OracleStatusInputs {
        current_height: 1_800,
        activation_height: 0,
        structural_hashes: &[mock_struct_hash],
        registered_at: &registered_at,
        bond_snapshot: &bond_snapshot,
        blocks_per_epoch: 360,
        last_update_height: None,
        attester_count: 0,
    });

    assert_eq!(resp["health"].as_str().unwrap(), "warning");
    // active should still be true — warning zone aggregates
    assert!(resp["active"].as_bool().unwrap());
    assert!(!resp["sunset_triggered"].as_bool().unwrap());
}

// OUTPUT CONTRACT: health field returns "halted_recoverable" when share < 55%
#[test]
fn m11_health_halted_below_sunset_threshold() {
    let mock_struct_hash = crypto::hash::hash_with_domain(b"MOCK_STRUCT", b"N1");
    let other_hash = crypto::hash::hash_with_domain(b"MOCK_OTHER", b"X");

    // 4000 / 10000 = 40% (below 55%)
    let mut bond_snapshot = std::collections::HashMap::new();
    bond_snapshot.insert(mock_struct_hash, 4_000u64);
    bond_snapshot.insert(other_hash, 6_000u64);
    let mut registered_at = std::collections::HashMap::new();
    registered_at.insert(mock_struct_hash, 0u64);
    registered_at.insert(other_hash, 0u64);

    let resp = super::build_oracle_status_response(super::OracleStatusInputs {
        current_height: 1_800,
        activation_height: 0,
        structural_hashes: &[mock_struct_hash],
        registered_at: &registered_at,
        bond_snapshot: &bond_snapshot,
        blocks_per_epoch: 360,
        last_update_height: None,
        attester_count: 0,
    });

    assert_eq!(resp["health"].as_str().unwrap(), "halted_recoverable");
    assert!(!resp["active"].as_bool().unwrap());
    assert!(resp["sunset_triggered"].as_bool().unwrap());
}

// OUTPUT CONTRACT: health returns "halted_recoverable" when no eligible bonds
#[tokio::test]
async fn m11_health_halted_when_no_eligible_bonds() {
    let t = build_m10_ctx();
    let resp = t.ctx.get_oracle_status(Value::Null).await.unwrap();
    assert_eq!(
        resp["health"].as_str().unwrap(),
        "halted_recoverable",
        "no eligible bonds -> halted_recoverable"
    );
}
