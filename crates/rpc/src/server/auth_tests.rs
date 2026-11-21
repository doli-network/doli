//! GRET Run 4b (run 730) — `check_admin_auth`, driven directly with a constructed
//! `SocketAddr` + `HeaderMap` (a real TCP socket cannot originate from 10.0.0.5 or a
//! public address, so the axum `ConnectInfo` layer is not used here).
//!
//! OUTPUT CONTRACT: fn check_admin_auth(&RpcSharedState, &HeaderMap, &str, SocketAddr)
//!                     -> Result<(), RpcError>
//!   O1 return value: `Ok(())` admits; `Err` must be `UNAUTHORIZED` (-32008).
//!   O2 no mutation, no store, no statics; tracing only.
//!   PATHS: non-admin method · operator-token method · other admin method x
//!          {trusted address, public + no token, public + token}.
//! INPUT PARTITIONS: address in {127.0.0.1, ::1, 10.0.0.5, 192.168.1.20, 203.0.113.9,
//!   2001:db8::7, public behind a trusted proxy, public behind an unconfigured proxy};
//!   token config in {None, Some}; header in {absent, wrong, token prefix, right,
//!   unprefixed}.
//! INC-I-308 (run 732): the R-2 tests below replace the loopback / RFC 1918 bypass.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::http::{header, HeaderMap, HeaderValue};
use crypto::Hash;
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use mempool::{Mempool, MempoolPolicy};
use storage::{BlockStore, ChainState, UtxoSet};
use tempfile::TempDir;
use tokio::sync::RwLock;

use super::{check_admin_auth, RpcSharedState, ADMIN_METHODS};
use crate::error::codes;
use crate::methods::RpcContext;

const TOKEN: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
const FORCE: &str = "forceReorgTo";

fn shared(token: Option<&str>, proxies: &[&str]) -> (RpcSharedState, TempDir) {
    let dir = TempDir::new().expect("tempdir");
    let network = Network::Devnet;
    let params = ConsensusParams::for_network(network);
    let ctx = RpcContext::new_for_network(
        Arc::new(RwLock::new(ChainState::new(Hash::ZERO))),
        Arc::new(BlockStore::open(dir.path()).expect("blockstore")),
        Arc::new(RwLock::new(UtxoSet::new())),
        Arc::new(RwLock::new(Mempool::new(
            MempoolPolicy::default(),
            params.clone(),
            network,
        ))),
        params,
        network,
    );
    let state = RpcSharedState {
        context: Arc::new(ctx),
        admin_token: token.map(str::to_string),
        trusted_proxies: proxies.iter().map(|p| p.parse().unwrap()).collect(),
    };
    (state, dir)
}

fn addr(ip: &str) -> SocketAddr {
    SocketAddr::new(ip.parse().unwrap(), 40_000)
}

fn bearer(token: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
    );
    h
}

fn admitted(s: &RpcSharedState, h: &HeaderMap, method: &str, from: &str) -> bool {
    match check_admin_auth(s, h, method, addr(from)) {
        Ok(()) => true,
        Err(e) => {
            assert_eq!(
                e.code,
                codes::UNAUTHORIZED,
                "{method} from {from}: wrong code"
            );
            false
        }
    }
}

const LOCAL_AND_PRIVATE: [&str; 4] = ["127.0.0.1", "::1", "10.0.0.5", "192.168.1.20"];
const PUBLIC: [&str; 2] = ["203.0.113.9", "2001:db8::7"];

/// Decision: a failure means a caller who holds no token — a local process, a host
/// on the same private network, or a public client — can arm a reorg across the
/// finality marker on a node that never configured an operator token.
#[test]
fn force_reorg_is_refused_everywhere_when_no_token_is_configured() {
    let (s, _d) = shared(None, &[]);
    for from in LOCAL_AND_PRIVATE.iter().chain(PUBLIC.iter()) {
        assert!(
            !admitted(&s, &HeaderMap::new(), FORCE, from),
            "forceReorgTo admitted from {from} with no admin_token configured"
        );
    }
}

/// Decision: a failure means a public client behind a reverse proxy — trusted
/// (headers honoured) or unconfigured (proxy peer is 127.0.0.1) — reaches the door.
#[test]
fn force_reorg_is_refused_for_a_proxied_public_client_without_a_token() {
    let mut fwd = HeaderMap::new();
    fwd.insert("x-real-ip", HeaderValue::from_static("203.0.113.9"));

    let (trusted, _d1) = shared(None, &["127.0.0.1"]);
    assert!(
        !admitted(&trusted, &fwd, FORCE, "127.0.0.1"),
        "trusted proxy"
    );

    let (unconfigured, _d2) = shared(None, &[]);
    assert!(
        !admitted(&unconfigured, &fwd, FORCE, "127.0.0.1"),
        "unconfigured proxy: the public client is seen as 127.0.0.1"
    );

    let (with_token, _d3) = shared(Some(TOKEN), &[]);
    assert!(
        !admitted(&with_token, &fwd, FORCE, "127.0.0.1"),
        "unconfigured proxy, token configured, no bearer"
    );
}

/// Decision: a failure means the trusted-network bypass still reaches
/// `forceReorgTo` when a token exists: a missing or wrong bearer is admitted.
#[test]
fn force_reorg_refuses_a_missing_or_wrong_bearer_from_every_address() {
    let (s, _d) = shared(Some(TOKEN), &[]);
    let mut unprefixed = HeaderMap::new();
    unprefixed.insert(header::AUTHORIZATION, HeaderValue::from_static(TOKEN));
    for from in LOCAL_AND_PRIVATE.iter().chain(PUBLIC.iter()) {
        for (label, h) in [
            ("no header", HeaderMap::new()),
            ("wrong token", bearer("not-the-token")),
            ("token prefix", bearer(&TOKEN[..20])),
            ("unprefixed", unprefixed.clone()),
        ] {
            assert!(
                !admitted(&s, &h, FORCE, from),
                "forceReorgTo admitted from {from} with {label}"
            );
        }
    }
}

/// Decision: a failure means the authorized operator path is closed — the escape
/// is unreachable in an incident even with the right token.
#[test]
fn force_reorg_admits_the_right_bearer_from_loopback_and_public() {
    let (s, _d) = shared(Some(TOKEN), &[]);
    for from in ["127.0.0.1", "::1", "10.0.0.5", "203.0.113.9"] {
        assert!(
            admitted(&s, &bearer(TOKEN), FORCE, from),
            "authorized operator refused from {from}"
        );
    }
}

/// Decision: a failure means an operator-token method fell out of `ADMIN_METHODS`
/// (the gate returns `Ok` before the token rule is reached) or `forceReorgTo` lost
/// its operator-token status.
#[test]
fn operator_token_methods_are_admin_methods() {
    use super::OPERATOR_TOKEN_METHODS;
    assert!(OPERATOR_TOKEN_METHODS.contains(&FORCE));
    for m in OPERATOR_TOKEN_METHODS {
        assert!(ADMIN_METHODS.contains(m), "{m} must stay in ADMIN_METHODS");
    }
}

// INC-I-308 (run 732) R-2: every admin method needs the bearer token from every address.

const WRONG: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeefe";
const R2_ADDRS: [&str; 7] = [
    "127.0.0.1",
    "::1",
    "10.0.0.5",
    "192.168.1.10",
    "172.16.0.9",
    "8.8.8.8",
    "2001:db8::7",
];
const DISPATCH_SRC: &str = include_str!("../methods/dispatch.rs");

fn verdict(s: &RpcSharedState, h: &HeaderMap, method: &str, from: &str) -> String {
    match check_admin_auth(s, h, method, addr(from)) {
        Ok(()) => "admitted".into(),
        Err(e) if e.code == codes::UNAUTHORIZED => "refused".into(),
        Err(e) => format!("error code {}", e.code),
    }
}

fn expect_refused(s: &RpcSharedState, cases: &[(&str, HeaderMap)], label: &str) -> Vec<String> {
    let mut wrong = Vec::new();
    for &m in ADMIN_METHODS {
        for from in R2_ADDRS {
            for (h_label, h) in cases {
                let got = verdict(s, h, m, from);
                if got != "refused" {
                    wrong.push(format!("{label}: {m} from {from} with {h_label}: {got}"));
                }
            }
        }
    }
    wrong
}

fn dispatch_methods() -> Vec<String> {
    let mut names = Vec::new();
    for line in DISPATCH_SRC.lines().map(str::trim_start) {
        if !line.starts_with('"') {
            continue;
        }
        if let Some((pattern, _)) = line.split_once("=>") {
            names.extend(
                pattern
                    .split('|')
                    .map(|alt| alt.trim().trim_matches('"').to_string()),
            );
        }
    }
    names
}

/// Decision: a failure means a caller without the token — a same-host process, a LAN
/// host, a public client — reaches an admin method, or the operator's token is refused.
#[test]
fn r2_every_admin_method_needs_the_token_from_every_address() {
    assert_eq!(ADMIN_METHODS.len(), 14);
    let (s, _d) = shared(Some(TOKEN), &[]);
    let mut wrong = Vec::new();
    for &m in ADMIN_METHODS {
        for from in R2_ADDRS {
            for (label, h, want) in [
                ("no header", HeaderMap::new(), "refused"),
                ("wrong token", bearer(WRONG), "refused"),
                ("valid token", bearer(TOKEN), "admitted"),
            ] {
                let got = verdict(&s, &h, m, from);
                if got != want {
                    wrong.push(format!("{m} from {from} with {label}: {got}, want {want}"));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "R-2: {} of {} cells wrong:\n{}",
        wrong.len(),
        14 * R2_ADDRS.len() * 3,
        wrong.join("\n")
    );
}

/// Decision: a failure means a node with no token configured serves admin methods to
/// some address (the pre-INC-I-308 loopback / RFC 1918 bypass is back).
#[test]
fn r2_no_token_configured_refuses_every_admin_method_everywhere() {
    let (s, _d) = shared(None, &[]);
    let cases = [
        ("no header", HeaderMap::new()),
        ("a 64-hex bearer", bearer(TOKEN)),
        ("an empty bearer", bearer("")),
    ];
    let wrong = expect_refused(&s, &cases, "admin_token None");
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Decision: a failure means an empty, blank or short configured token arms the gate
/// (a caller who sends the same guessable value is admitted), or an empty bearer or a
/// prefix of the real token is accepted.
#[test]
fn r2_empty_or_short_configured_token_never_admits() {
    let short = &TOKEN[..31];
    let mut wrong = Vec::new();
    for cfg in ["", "   ", short] {
        let (s, _d) = shared(Some(cfg), &[]);
        let cases = [
            ("no header", HeaderMap::new()),
            ("the same value", bearer(cfg)),
        ];
        wrong.extend(expect_refused(&s, &cases, &format!("config {cfg:?}")));
    }
    let (s, _d) = shared(Some(TOKEN), &[]);
    let cases = [
        ("an empty bearer", bearer("")),
        ("a 31-char prefix", bearer(short)),
    ];
    wrong.extend(expect_refused(&s, &cases, "config 64-hex"));
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Decision: a failure means the admin gate leaked onto a public method: wallets and
/// explorers that send no token lose a read or a payload-signed submit.
#[test]
fn r2_public_methods_need_no_token_from_any_address() {
    let dispatched = dispatch_methods();
    for m in ADMIN_METHODS {
        assert!(dispatched.iter().any(|d| d == m), "parser missed {m}");
    }
    let public: Vec<&String> = dispatched
        .iter()
        .filter(|m| !ADMIN_METHODS.contains(&m.as_str()))
        .collect();
    assert!(
        public.len() >= 30,
        "only {} public methods parsed",
        public.len()
    );
    let short = &TOKEN[..31];
    let mut wrong = Vec::new();
    for cfg in [None, Some(TOKEN), Some(""), Some(short)] {
        let (s, _d) = shared(cfg, &[]);
        for m in &public {
            for from in R2_ADDRS {
                let got = verdict(&s, &HeaderMap::new(), m, from);
                if got != "admitted" {
                    wrong.push(format!("config {cfg:?}: {m} from {from}: {got}"));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Decision: a failure means a private client behind a trusted reverse proxy reaches an
/// admin method with no token (the bypass survives through `X-Real-IP`).
#[test]
fn r2_private_client_behind_a_trusted_proxy_needs_the_token() {
    let (s, _d) = shared(Some(TOKEN), &["127.0.0.1"]);
    let mut fwd = HeaderMap::new();
    fwd.insert("x-real-ip", HeaderValue::from_static("10.0.0.5"));
    let mut fwd_auth = fwd.clone();
    fwd_auth.extend(bearer(TOKEN));
    for &m in ADMIN_METHODS {
        assert!(
            !admitted(&s, &fwd, m, "127.0.0.1"),
            "{m}: proxied 10.0.0.5 with no bearer was admitted"
        );
        assert!(
            admitted(&s, &fwd_auth, m, "127.0.0.1"),
            "{m}: proxied 10.0.0.5 with the token was refused"
        );
    }
}
