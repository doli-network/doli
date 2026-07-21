//! INC-I-145: `repairArchiveFromPeer` peer-tip parse must accept the real
//! `getChainInfo` response shape.
//!
//! `getChainInfo` serializes `ChainInfoResponse` with
//! `#[serde(rename_all = "camelCase")]` (types/chain.rs), so the tip field is
//! `bestHeight`. The original pointer chain only checked `/result/height` and
//! `/result/best_height`, so the parse failed on every call and the method
//! returned -32603 unconditionally (confirmed on mainnet v6.23.10).
//!
//! Included via `#[cfg(test)] #[path = "tests_inc_i145.rs"] mod tests_inc_i145;`
//! in `guardian.rs`, so `super::*` resolves to the `guardian` module.
//!
//! OUTPUT CONTRACT: fn parse_peer_chain_tip(body: &serde_json::Value) -> Option<u64>
//!   O1: Some(tip) — a numeric tip found at /result/bestHeight (real getChainInfo
//!       shape), or at legacy fallbacks /result/height, /result/best_height
//!   O2: None — no recognized tip field, or field present but not a u64
//! PATHS: bestHeight hit | height hit | best_height hit | no match | non-numeric
//! INPUT PARTITIONS:
//!   P1: real ChainInfoResponse serialized by the production serializer (camelCase)
//!   P2: legacy shape {"result":{"height":N}}
//!   P3: legacy shape {"result":{"best_height":N}}
//!   P4: result object without any tip field / missing result entirely
//!   P5: tip field present but non-numeric (string) — must not parse
//! MATRIX: P1→O1(Some N), P2→O1(Some N), P3→O1(Some N), P4→O2(None), P5→O2(None)

use super::parse_peer_chain_tip;
use crate::types::ChainInfoResponse;

/// P1 → O1. The acceptance test: a getChainInfo-shaped body produced by the
/// REAL serializer must parse to the peer tip. FAILS before the INC-I-145 fix
/// (pointer chain missed `bestHeight`), PASSES after.
#[test]
fn inc_i145_parses_real_get_chain_info_shape() {
    let response = ChainInfoResponse {
        network: "mainnet".to_string(),
        version: "6.23.10".to_string(),
        best_hash: "ab".repeat(32),
        best_height: 110_140,
        best_slot: 999,
        genesis_hash: "cd".repeat(32),
        reward_pool_balance: 0,
    };
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "result": serde_json::to_value(&response).unwrap(),
        "id": 0
    });
    assert_eq!(
        parse_peer_chain_tip(&body),
        Some(110_140),
        "peer tip must parse from the real getChainInfo shape (bestHeight, camelCase); body={body}"
    );
}

/// P2 → O1. Legacy fallback `/result/height` still parses.
#[test]
fn inc_i145_parses_legacy_height_field() {
    let body = serde_json::json!({"jsonrpc": "2.0", "result": {"height": 42}, "id": 0});
    assert_eq!(parse_peer_chain_tip(&body), Some(42));
}

/// P3 → O1. Legacy fallback `/result/best_height` still parses.
#[test]
fn inc_i145_parses_legacy_best_height_field() {
    let body = serde_json::json!({"jsonrpc": "2.0", "result": {"best_height": 7}, "id": 0});
    assert_eq!(parse_peer_chain_tip(&body), Some(7));
}

/// P4 → O2. No recognized tip field (and no result at all) yields None.
#[test]
fn inc_i145_rejects_body_without_tip() {
    let no_tip = serde_json::json!({"jsonrpc": "2.0", "result": {"bestHash": "aa"}, "id": 0});
    assert_eq!(parse_peer_chain_tip(&no_tip), None);

    let no_result = serde_json::json!({"jsonrpc": "2.0", "error": {"code": -32601}, "id": 0});
    assert_eq!(parse_peer_chain_tip(&no_result), None);
}

/// P5 → O2. A non-numeric tip field must not parse.
#[test]
fn inc_i145_rejects_non_numeric_tip() {
    let body = serde_json::json!({"jsonrpc": "2.0", "result": {"bestHeight": "110140"}, "id": 0});
    assert_eq!(parse_peer_chain_tip(&body), None);
}
