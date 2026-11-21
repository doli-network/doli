//! AUDIT-RPC-ACCESS (2026-11-21) — reproduction tests of a READ-ONLY security audit
//! of the doli-node JSON-RPC access control. C1, C2 and C5 assert the SECURE behavior
//! (red before INC-I-308, run 732); C4 stays ignored; the other tests are controls.
//! Report: docs/audits/security-audit-rpc-2026-11-21.md.
//!
//! How a source address is presented: the server runs on a free 127.0.0.1 port, so
//! the TCP peer is always loopback. A private or public client is presented the way
//! production presents it: `trusted_proxies = [127.0.0.1]` + `X-Real-IP` (server.rs
//! `resolve_client_ip`, 279-305). The DIRECT private/public peer (no proxy) cannot
//! come from a real socket; `server/auth_tests.rs`
//! `r2_every_admin_method_needs_the_token_from_every_address` pins that case.
//!
//! OUTPUT CONTRACT: `POST /` -> `handle_rpc` (server.rs:309-354)
//!   O1 response body: JSON-RPC `result`, or `error.code` (`UNAUTHORIZED` -32008 =
//!      refused by the gate; any other code = the handler or the parser answered).
//!   O2 node state through the shared handles: SyncManager production halt,
//!      `recovery_mode` flag, checkpoint directory on disk, mempool length,
//!      transaction relay count, vote relay count.
//!   PATHS: gate skipped for a non-admin name (server.rs:205-207) · trusted-network
//!      bypass (218-220) · no token configured (224-230) · bearer compared (231-249)
//!      · parse error (316-326) · dispatch miss (dispatch.rs:81).
//! INPUT PARTITIONS: method in every `Access` class of `AUDIT_TABLE`; address in
//!   {loopback, 10/8, 172.16/12, 192.168/16, the first address outside each range,
//!   link-local, CGNAT, public v4, public v6, ULA v6, v4-mapped v6}; token config in
//!   {None, Some}; Authorization in {absent, wrong, right}; body in {single request,
//!   batch array, duplicate `method` key, method-name variants}.
//!
//! C3 (public address; token configured and not configured) writes no new red test:
//! the behavior is secure and already pinned at the decision function by
//! `server/auth_tests.rs::r2_every_admin_method_needs_the_token_from_every_address`
//! and the four `force_reorg_*` tests. The C3 control below adds the HTTP path and
//! the address-range boundary, which no existing test pins.

use std::sync::atomic::Ordering;

use doli_core::Transaction;
use rpc::error::codes;
use rpc::server::{ADMIN_METHODS, OPERATOR_TOKEN_METHODS};
use serde_json::{json, Value};

use super::audit_rpc_access_fixture::Access::{
    AdminMutating, AdminReadOnly, OpenMutating, OperatorToken,
};
use super::audit_rpc_access_fixture::{
    arm, call, dispatch_methods, error_code, exposed_admin_methods, keypair, maintainer_sigs,
    observed_effect, post_raw, refused, start, summary, AUDIT_TABLE, DISPATCH_SRC, PUBLIC, TOKEN,
};

/// AUDIT-RPC-ACCESS table guard — Decision: a failure means a method was added to
/// (or removed from) the dispatch `match`, `ADMIN_METHODS` or
/// `OPERATOR_TOKEN_METHODS` and nobody decided its access class, so the C1/C2/C4
/// tests no longer cover every state-changing method. Classify it in `AUDIT_TABLE`.
///
/// Proves: `AUDIT_TABLE` names == the string-literal arms of the dispatch `match`
/// (dispatch.rs:15-82, read with `include_str!`), and each class agrees with the two
/// gate lists (server.rs:31-52, :57). PASSES today.
#[test]
fn audit_rpc_access_table_classifies_every_method() {
    let dispatched = dispatch_methods();
    assert_eq!(
        dispatched.len(),
        DISPATCH_SRC.matches("\" =>").count(),
        "the dispatch.rs arm scan missed a string-literal arm: re-read dispatch.rs"
    );
    assert!(
        !DISPATCH_SRC
            .lines()
            .any(|l| l.trim_start().starts_with('|')),
        "dispatch.rs has a multi-line `|` pattern the scan cannot read"
    );

    let mut table: Vec<&str> = AUDIT_TABLE.iter().map(|(m, _, _)| *m).collect();
    table.sort_unstable();
    let rows = table.len();
    table.dedup();
    assert_eq!(rows, table.len(), "AUDIT_TABLE has a duplicate method");

    let unclassified: Vec<&String> = dispatched
        .iter()
        .filter(|m| !table.contains(&m.as_str()))
        .collect();
    assert!(
        unclassified.is_empty(),
        "dispatched but not classified in AUDIT_TABLE: {unclassified:?}"
    );
    let stale: Vec<&&str> = table
        .iter()
        .filter(|m| !dispatched.iter().any(|d| d == **m))
        .collect();
    assert!(
        stale.is_empty(),
        "in AUDIT_TABLE but not dispatched: {stale:?}"
    );

    for &(method, class, _) in AUDIT_TABLE {
        let admin = matches!(class, OperatorToken | AdminMutating | AdminReadOnly);
        assert_eq!(
            ADMIN_METHODS.contains(&method),
            admin,
            "{method}: AUDIT_TABLE says {class:?}, ADMIN_METHODS membership disagrees"
        );
        assert_eq!(
            OPERATOR_TOKEN_METHODS.contains(&method),
            class == OperatorToken,
            "{method}: AUDIT_TABLE says {class:?}, OPERATOR_TOKEN_METHODS disagrees"
        );
    }
    for method in ADMIN_METHODS {
        assert!(
            dispatched.iter().any(|d| d == method),
            "{method} is in ADMIN_METHODS but is not dispatched"
        );
    }
}

/// AUDIT-RPC-ACCESS C1 — Decision: a failure means a process on the node host (or a
/// web page in a browser on it, or an internet client behind a same-host reverse
/// proxy that is not declared in `trusted_proxies`) can halt production, switch
/// recovery mode, write checkpoints, prune, or start a backfill with no credential.
///
/// Proves: each `AdminMutating` method is answered for a loopback TCP peer with no
/// bearer token, with a WRONG bearer token, with forwarded-for headers of a public
/// client (headers ignored: `trusted_proxies` empty, server.rs:284-286), and for a
/// cross-site browser POST (`Content-Type: text/plain`, foreign `Origin`; the body
/// is parsed whatever its type, server.rs:316 — known as INC-I-269).
/// Branch: server.rs:218-220 — `is_trusted_network(effective_ip) &&
/// !OPERATOR_TOKEN_METHODS.contains(&method)` returns `Ok(())` before the token is
/// looked at; loopback test at server.rs:268-269.
/// Fails today at: `assert!(exposed.is_empty(), ...)`.
#[tokio::test]
async fn audit_rpc_access_c1_admin_methods_answer_loopback_without_token() {
    let gated = start(Some(TOKEN), &[]).await;
    let open = start(None, &[]).await;
    let mut exposed = Vec::new();
    for (node, caller, headers) in [
        (
            &gated,
            "loopback, token configured, no Authorization header",
            &[][..],
        ),
        (
            &gated,
            "loopback, token configured, WRONG bearer token",
            &[("authorization", "Bearer not-the-token")][..],
        ),
        (
            &gated,
            "public client 203.0.113.9 behind a same-host proxy not in trusted_proxies",
            &[("x-real-ip", PUBLIC), ("x-forwarded-for", PUBLIC)][..],
        ),
        (
            &open,
            "browser page, cross-site POST text/plain, no token configured",
            &[
                ("content-type", "text/plain"),
                ("origin", "https://evil.example"),
            ][..],
        ),
    ] {
        exposed.extend(exposed_admin_methods(node, caller, headers).await);
    }
    assert!(
        exposed.is_empty(),
        "AUDIT-RPC-ACCESS C1: {} state-changing admin calls were answered for a \
         loopback caller that holds no valid token:\n{}",
        exposed.len(),
        exposed.join("\n")
    );
}

/// AUDIT-RPC-ACCESS C2 — Decision: a failure means every host on the same private
/// network as the node (same VPC, LAN, VPN, container bridge) can halt production,
/// switch recovery mode, write checkpoints, prune, or start a backfill with no
/// credential, even when the operator configured an admin token.
///
/// Proves: each `AdminMutating` method is answered with no bearer token when the
/// effective client address is in 10/8, 172.16/12 or 192.168/16.
/// Branch: server.rs:218-220 (trusted-network bypass) with server.rs:268
/// `v4.is_private()`. The address reaches the check through a declared proxy
/// (server.rs:288-294); the direct-peer case takes the same branch and is pinned by
/// `auth_tests.rs::r2_every_admin_method_needs_the_token_from_every_address`.
/// Fails today at: `assert!(exposed.is_empty(), ...)`.
#[tokio::test]
async fn audit_rpc_access_c2_admin_methods_answer_private_network_without_token() {
    let node = start(Some(TOKEN), &["127.0.0.1"]).await;
    let mut exposed = Vec::new();
    for from in ["10.0.0.5", "172.16.0.9", "172.31.255.254", "192.168.1.20"] {
        let caller = format!("private client {from}, token configured, no Authorization");
        exposed.extend(exposed_admin_methods(&node, &caller, &[("x-real-ip", from)]).await);
    }
    assert!(
        exposed.is_empty(),
        "AUDIT-RPC-ACCESS C2: {} state-changing admin calls were answered for a \
         private-network caller that holds no token:\n{}",
        exposed.len(),
        exposed.join("\n")
    );
}

/// AUDIT-RPC-ACCESS C3 control — Decision: a failure means the trusted set is wider
/// than {127/8, ::1, 10/8, 172.16/12, 192.168/16}, or a public caller is answered
/// without the token, or the harness cannot tell a refusal from an answer (then the
/// C1/C2/C5 reds prove nothing).
///
/// Proves (PASSES today): over HTTP, an address just outside each private range, a
/// link-local, CGNAT, public v4/v6, ULA and v4-mapped-v6 address is refused for
/// every `AdminMutating` method with no token and with a wrong token, and nothing
/// changes; the right token is answered and changes state; with NO token configured
/// a public caller is refused even with a bearer; `forceReorgTo` is refused from
/// loopback. Branches: server.rs:266-271, 223-249.
#[tokio::test]
async fn audit_rpc_access_c3_control_non_private_addresses_need_the_token() {
    let gated = start(Some(TOKEN), &["127.0.0.1"]).await;
    let bearer = format!("Bearer {TOKEN}");
    for from in [
        PUBLIC,
        "9.255.255.255",
        "11.0.0.1",
        "172.15.255.255",
        "172.32.0.1",
        "192.167.255.255",
        "192.169.0.1",
        "169.254.1.1",
        "100.64.0.1",
        "2001:db8::7",
        "fd00::1",
        "fe80::1",
        "::ffff:10.0.0.5",
        "::ffff:127.0.0.1",
    ] {
        for (label, auth) in [("no token", None), ("wrong token", Some("Bearer nope"))] {
            let mut headers = vec![("x-real-ip", from)];
            headers.extend(auth.map(|a| ("authorization", a)));
            let exposed = exposed_admin_methods(&gated, from, &headers).await;
            assert!(
                exposed.is_empty(),
                "{from} with {label} must be refused:\n{}",
                exposed.join("\n")
            );
        }
    }

    arm(&gated, "pauseProduction").await;
    let headers = [("x-real-ip", PUBLIC), ("authorization", bearer.as_str())];
    let resp = call(&gated, "pauseProduction", Value::Null, &headers).await;
    assert!(
        !refused(&resp) && observed_effect(&gated, "pauseProduction").await.is_some(),
        "the right token from a public address must be answered: {}",
        summary(&resp)
    );

    let resp = call(&gated, "forceReorgTo", json!({}), &[]).await;
    assert!(refused(&resp), "forceReorgTo from loopback, no token");

    let open = start(None, &["127.0.0.1"]).await;
    let exposed = exposed_admin_methods(&open, "public, no token configured", &headers).await;
    assert!(
        exposed.is_empty(),
        "no token configured: a public caller must be refused:\n{}",
        exposed.join("\n")
    );
}

/// AUDIT-RPC-ACCESS C4 — Decision: a failure means the three state-changing methods
/// outside `ADMIN_METHODS` have no transport-level access control at all: a public
/// client with no token reaches the handler, and a payload that carries valid
/// signatures enters the mempool and is relayed to gossip. The operator has no
/// setting that limits who may submit.
///
/// DESIGN CAVEAT (read before "fixing"): these methods are authorized by the
/// PAYLOAD (transaction signature, producer vote signature, 3 maintainer
/// signatures), not by the caller. A blanket admin token on `sendTransaction`
/// stops every wallet. This test records the exposure as the audit brief defines it
/// ("a caller with no credentials is refused for a method that changes anything");
/// which methods get a transport gate is a decision for the audit owner. The payload
/// gate itself holds today: see the C4 control below.
///
/// Proves: `sendTransaction`, `submitVote`, `submitMaintainerChange` are answered by
/// their handler (not by the gate) from a public, a private and a loopback address
/// with no token; a maintainer change signed by 3 fixture maintainers, sent from a
/// public address with no token, is accepted, held in the mempool and relayed.
/// Branch: server.rs:205-207 — `if !ADMIN_METHODS.contains(&method) { return Ok(()) }`.
/// Fails today at: `assert!(exposed.is_empty(), ...)`.
#[tokio::test]
#[ignore = "AUDIT-P3-RPC-001 (class C4) is out of scope for INC-I-308: payload signature is the credential"]
async fn audit_rpc_access_c4_non_admin_state_changing_methods_answer_any_address_without_token() {
    let node = start(Some(TOKEN), &["127.0.0.1"]).await;
    let mut exposed = Vec::new();
    for &(method, class, what) in AUDIT_TABLE {
        if class != OpenMutating {
            continue;
        }
        for from in [PUBLIC, "10.0.0.5", "127.0.0.1"] {
            let resp = call(&node, method, json!({}), &[("x-real-ip", from)]).await;
            if !refused(&resp) {
                exposed.push(format!(
                    "  {method} <- {from}, no token: {} | handler: {what}",
                    summary(&resp)
                ));
            }
        }
    }

    let signed = json!({
        "action": "add",
        "target_pubkey": keypair(0xaa).public_key().to_hex(),
        "signatures": maintainer_sigs(1, 3, 0xaa),
    });
    let resp = call(
        &node,
        "submitMaintainerChange",
        signed,
        &[("x-real-ip", PUBLIC)],
    )
    .await;
    if !refused(&resp) {
        exposed.push(format!(
            "  submitMaintainerChange (3 valid maintainer signatures) <- {PUBLIC}, no token: {} \
             | mempool 0 -> {}, relayed to gossip {} time(s)",
            summary(&resp),
            node.mempool.read().await.len(),
            node.tx_relays.load(Ordering::SeqCst),
        ));
    }
    assert!(
        exposed.is_empty(),
        "AUDIT-RPC-ACCESS C4: {} calls to state-changing methods outside ADMIN_METHODS \
         were answered with no token:\n{}",
        exposed.len(),
        exposed.join("\n")
    );
}

/// AUDIT-RPC-ACCESS C4 control — Decision: a failure means a caller with NO payload
/// credential (no valid signature of a key the chain knows) changes the mempool or
/// reaches gossip through an open method: then C4 is a critical defect, not a
/// design question.
///
/// Proves (PASSES today): from a public address with no token, garbage transaction
/// bytes, an unsigned maintainer transaction, a vote signed by an unregistered key
/// and a maintainer change signed by 3 non-maintainers each return an error; the
/// mempool stays empty; nothing is relayed. Handler-level pins of the same rule:
/// `tests_inc_i195_broadcast.rs::inc_i195_rejected_submissions_broadcast_nothing`.
#[tokio::test]
async fn audit_rpc_access_c4_control_payload_without_valid_signature_changes_nothing() {
    let node = start(Some(TOKEN), &["127.0.0.1"]).await;
    let unsigned = Transaction::new_add_maintainer(*keypair(0xaa).public_key(), vec![]);
    let outsider = keypair(0x77);
    let vote_sig = crypto::signature::sign(b"9.9.9:veto:1800000000", outsider.private_key());
    let attempts = [
        ("sendTransaction", json!({ "tx": "not-hex" })),
        ("sendTransaction", json!({ "tx": "00" })),
        (
            "sendTransaction",
            json!({ "tx": hex::encode(unsigned.serialize()) }),
        ),
        (
            "submitVote",
            json!({ "vote": {
                "version": "9.9.9",
                "vote": "veto",
                "producerId": outsider.public_key().to_hex(),
                "timestamp": 1_800_000_000u64,
                "signature": vote_sig.to_hex(),
            }}),
        ),
        (
            "submitMaintainerChange",
            json!({
                "action": "add",
                "target_pubkey": keypair(0xaa).public_key().to_hex(),
                "signatures": maintainer_sigs(0x20, 3, 0xaa),
            }),
        ),
    ];
    for (method, params) in attempts {
        let resp = call(&node, method, params.clone(), &[("x-real-ip", PUBLIC)]).await;
        assert!(
            error_code(&resp).is_some(),
            "{method} {params} must be rejected: {}",
            summary(&resp)
        );
        assert_eq!(node.mempool.read().await.len(), 0, "{method}: mempool");
        assert_eq!(node.tx_relays.load(Ordering::SeqCst), 0, "{method}: relay");
        assert_eq!(node.vote_relays.load(Ordering::SeqCst), 0, "{method}: vote");
    }
}

/// AUDIT-RPC-ACCESS C5 — Decision: a failure means a public client behind a DECLARED
/// reverse proxy passes as a private or loopback address by sending its own
/// forwarding header, and then C2 applies to the whole internet.
///
/// PRECONDITION (deployment): the declared proxy does not overwrite `X-Real-IP`.
/// (a) it appends to `X-Forwarded-For` (nginx `$proxy_add_x_forwarded_for`, HAProxy
/// `option forwardfor`, cloud load balancers) and sends no `X-Real-IP`: the node
/// takes the LEFTMOST entry, which the client wrote (server.rs:296-302; pinned as
/// intended by `proxy_tests.rs::trusted_proxy_x_forwarded_for_leftmost_is_used`).
/// (b) it passes a client-supplied `X-Real-IP` through: that header wins over
/// `X-Forwarded-For` (server.rs:288-294). A proxy set up as docs/buy_doli.md:415-416
/// (`X-Real-IP $remote_addr`) is NOT exposed.
///
/// Proves: with the token configured and 127.0.0.1 declared, each `AdminMutating`
/// method is answered with no token for the header values such a proxy forwards for
/// the public client 203.0.113.9. No bypass of the NAME match exists (see the C5
/// control). Fails today at: `assert!(exposed.is_empty(), ...)`.
#[tokio::test]
async fn audit_rpc_access_c5_forged_forwarding_header_passes_as_trusted_address() {
    let node = start(Some(TOKEN), &["127.0.0.1"]).await;
    let mut exposed = Vec::new();
    for (caller, headers) in [
        (
            "public 203.0.113.9 wrote XFF 10.0.0.5, proxy appended the real address",
            &[("x-forwarded-for", "10.0.0.5, 203.0.113.9")][..],
        ),
        (
            "public 203.0.113.9 wrote X-Real-IP 127.0.0.1, proxy set only XFF",
            &[("x-real-ip", "127.0.0.1"), ("x-forwarded-for", PUBLIC)][..],
        ),
    ] {
        exposed.extend(exposed_admin_methods(&node, caller, headers).await);
    }
    assert!(
        exposed.is_empty(),
        "AUDIT-RPC-ACCESS C5: {} state-changing admin calls were answered for a public \
         client that forged a forwarding header:\n{}",
        exposed.len(),
        exposed.join("\n")
    );
}

/// AUDIT-RPC-ACCESS C5 control — Decision: a failure means a request shape gets an
/// admin method past the name match (server.rs:205 and dispatch.rs:15 both compare
/// the same parsed string exactly): a batch array, a case or whitespace variant, a
/// duplicate `method` key, or the `/ws` route.
///
/// Proves (PASSES today), for a public caller with no token: name variants of
/// `pauseProduction` answer METHOD_NOT_FOUND (dispatch.rs:81); a batch array answers
/// PARSE_ERROR (server.rs:316-326; `handle_batch_rpc`, server.rs:356-378, has no
/// auth check but is dead code, not routed at server.rs:123-130); a body with two
/// `method` keys is not executed; `POST /ws` is not a JSON-RPC route and the socket
/// handler ignores client frames (ws.rs:104). Production is never halted.
#[tokio::test]
async fn audit_rpc_access_c5_control_request_shape_variants_do_not_reach_admin_handlers() {
    let node = start(Some(TOKEN), &["127.0.0.1"]).await;
    let public = [("x-real-ip", PUBLIC)];
    arm(&node, "pauseProduction").await;

    for variant in [
        "PauseProduction",
        "pauseproduction",
        "PAUSEPRODUCTION",
        " pauseProduction",
        "pauseProduction ",
        "pauseProduction\u{0}",
        "pause_production",
    ] {
        let resp = call(&node, variant, Value::Null, &public).await;
        assert_eq!(
            error_code(&resp),
            Some(i64::from(codes::METHOD_NOT_FOUND)),
            "{variant:?}: {}",
            summary(&resp)
        );
    }

    let batch = json!([
        { "jsonrpc": "2.0", "method": "pauseProduction", "id": 1 },
        { "jsonrpc": "2.0", "method": "getChainInfo", "id": 2 },
    ]);
    let resp = post_raw(&node, "/", batch.to_string(), &public).await;
    assert_eq!(
        error_code(&resp),
        Some(i64::from(codes::PARSE_ERROR)),
        "batch array: {resp}"
    );

    for body in [
        r#"{"jsonrpc":"2.0","method":"getChainInfo","method":"pauseProduction","id":1}"#,
        r#"{"jsonrpc":"2.0","method":"pauseProduction","method":"getChainInfo","id":1}"#,
    ] {
        let resp = post_raw(&node, "/", body.to_string(), &public).await;
        assert!(error_code(&resp).is_some(), "duplicate key: {resp}");
    }

    let rpc = json!({ "jsonrpc": "2.0", "method": "pauseProduction", "id": 1 });
    let resp = post_raw(&node, "/ws", rpc.to_string(), &public).await;
    assert!(resp.get("result").is_none(), "POST /ws answered: {resp}");

    assert!(
        observed_effect(&node, "pauseProduction").await.is_none(),
        "a request-shape variant halted production"
    );
}
