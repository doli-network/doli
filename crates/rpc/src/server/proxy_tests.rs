//! ISSUE-174 #1 — `resolve_client_ip` reverse-proxy header handling.

use super::*;
use axum::http::HeaderValue;
use std::net::{IpAddr, Ipv4Addr};

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

fn headers_xff(val: &'static str) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert("x-forwarded-for", HeaderValue::from_static(val));
    h
}
fn headers_xrip(val: &'static str) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert("x-real-ip", HeaderValue::from_static(val));
    h
}

#[test]
fn no_trusted_proxies_returns_peer_ip_even_with_forged_header() {
    // ISSUE-174 #1 regression: an attacker cannot reach a "trusted" IP just by
    // setting X-Forwarded-For unless the operator opted in to header parsing.
    let peer = ip("203.0.113.42");
    let headers = headers_xff("127.0.0.1");
    assert_eq!(resolve_client_ip(peer, &headers, &[]), peer);
}

#[test]
fn peer_not_in_trusted_proxies_returns_peer_ip() {
    // Random public peer claims to be Nginx — must be ignored.
    let peer = ip("203.0.113.42");
    let headers = headers_xrip("127.0.0.1");
    let trusted = vec![ip("127.0.0.1")];
    assert_eq!(resolve_client_ip(peer, &headers, &trusted), peer);
}

#[test]
fn trusted_proxy_x_real_ip_takes_precedence() {
    // Nginx is the peer (127.0.0.1) and reports the real client.
    let peer = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let headers = headers_xrip("203.0.113.42");
    let trusted = vec![peer];
    assert_eq!(
        resolve_client_ip(peer, &headers, &trusted),
        ip("203.0.113.42")
    );
}

#[test]
fn trusted_proxy_x_forwarded_for_leftmost_is_used() {
    let peer = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let headers = headers_xff("198.51.100.7, 10.0.0.1");
    let trusted = vec![peer];
    assert_eq!(
        resolve_client_ip(peer, &headers, &trusted),
        ip("198.51.100.7")
    );
}

#[test]
fn trusted_proxy_no_headers_falls_back_to_peer() {
    // Misconfigured Nginx (not setting headers) → behavior is "trust peer",
    // which is the same vulnerable behavior we had before. Operator must
    // configure Nginx correctly when opting into header parsing.
    let peer = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let trusted = vec![peer];
    assert_eq!(resolve_client_ip(peer, &HeaderMap::new(), &trusted), peer);
}

#[test]
fn malformed_header_falls_back_to_peer() {
    let peer = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let headers = headers_xrip("not-an-ip");
    let trusted = vec![peer];
    assert_eq!(resolve_client_ip(peer, &headers, &trusted), peer);
}
