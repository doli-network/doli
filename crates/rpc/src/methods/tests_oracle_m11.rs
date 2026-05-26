//! Tests for M11 `getOracleStatus`.
//!
//! Split from `tests_oracle.rs` so each test file stays within the
//! 800-LOC budget (Rule 19). Shared fixtures live in
//! `super::tests::*` and are marked `pub(super)` to cross the
//! `tests`/`tests_m11` module boundary inside the `oracle` parent.
//!
//! OUTPUT CONTRACT:
//!   getOracleStatus() -> { active, trust_model, structural_share,
//!                          sunset_threshold, sunset_triggered,
//!                          last_update_height, attester_count,
//!                          activation_height, centralization_disclosure }
//!
//! INPUT PARTITIONS:
//!   activation       = { pre (u64::MAX), post }
//!   structural_share = { >=5500 bps (active), <5500 bps (sunset_triggered) }
//!   utxo_state       = { has OraclePrice UTXO, none, multiple-pairs }
//!   attester_window  = { current_epoch=0 (no closed),
//!                        attestations in closed epoch,
//!                        attestations in current epoch (excluded) }

use super::tests::{build_m10_ctx, insert_attestation_block, make_attestation_tx, pair_id_fixture};
use serde_json::Value;
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

// ---------- partition: last_update_height = max across OraclePrice UTXOs ----------
#[tokio::test]
async fn m11_last_update_height_takes_max_across_pairs() {
    let t = build_m10_ctx();
    let pair_a = crypto::hash::hash_with_domain(b"ORACLE_PAIR", b"AAA/USD");
    let pair_b = crypto::hash::hash_with_domain(b"ORACLE_PAIR", b"BBB/USD");

    // Insert two OraclePrice UTXOs with different last_update_heights.
    // M11 must report the MAX.
    let out_a = doli_core::transaction::Output::oracle_price(pair_a, 100, 500, 3);
    let (txh_a, idx_a) = doli_core::oracle::oracle_price_outpoint(&pair_a);
    let out_b = doli_core::transaction::Output::oracle_price(pair_b, 200, 1200, 5);
    let (txh_b, idx_b) = doli_core::oracle::oracle_price_outpoint(&pair_b);
    {
        let mut us = t.ctx.utxo_set.write().await;
        us.insert(
            Outpoint::new(txh_a, idx_a),
            UtxoEntry {
                output: out_a,
                height: 500,
                is_coinbase: false,
                is_epoch_reward: false,
            },
        )
        .unwrap();
        us.insert(
            Outpoint::new(txh_b, idx_b),
            UtxoEntry {
                output: out_b,
                height: 1200,
                is_coinbase: false,
                is_epoch_reward: false,
            },
        )
        .unwrap();
    }

    let resp = t.ctx.get_oracle_status(Value::Null).await.unwrap();
    assert_eq!(
        resp["last_update_height"].as_u64().unwrap(),
        1200,
        "must report max(last_update_height) across all OraclePrice UTXOs"
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
}

// ---------- partition: response has all 9 documented fields ----------
#[tokio::test]
async fn m11_response_shape_has_all_documented_fields() {
    let t = build_m10_ctx();
    let resp = t.ctx.get_oracle_status(Value::Null).await.unwrap();
    let obj = resp.as_object().expect("must be JSON object");
    for field in [
        "active",
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
