// OUTPUT CONTRACT: due_bootstrap_redials -> O1 returned Vec<String>, O2 backoff map mutation;
//   redial_round_message -> O1 Option<String>. No receiver, no persistent store writes.
// INPUT PARTITIONS: peer_count vs max(min,1) {below, at, above}; min {0, 1, 2, usize::MAX};
//   backoff entry {absent, due, not due, 1 ms before due}; addrs {empty, normal, unusual}; dialing {0, >0}.
// Matrix: at/above -> O1 empty, O2 cleared; below + absent/due -> O1 addr, O2 (count+1, now);
//   below + not due -> O1 excluded, O2 untouched; empty addrs -> O1 empty, O2 nothing inserted.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::bootstrap_redial::{due_bootstrap_redials, redial_round_message};

type Backoff = HashMap<String, (u32, Instant)>;

fn base() -> Instant {
    Instant::now() + Duration::from_secs(3600)
}

fn at(t0: Instant, secs: u64) -> Instant {
    t0 + Duration::from_secs(secs)
}

fn addrs() -> Vec<String> {
    vec![
        "/ip4/127.0.0.1/tcp/30300".to_string(),
        "/ip4/127.0.0.1/tcp/30305".to_string(),
        "/dns4/seed1.doli.network/tcp/30300".to_string(),
    ]
}

fn seeded_backoff(list: &[String], t0: Instant) -> Backoff {
    list.iter().map(|a| (a.clone(), (5u32, t0))).collect()
}

// REQ-BSR-001 — Decision: a failure means a node holding 1 of 2 peers never re-dials its seeds and stays blocked by InsufficientPeers
// REQ-BSR-003 — Decision: this is the reproduction; passing on the threshold=1 gate would mean it cannot catch the stall
#[test]
fn one_peer_below_min_redials_all_bootstrap_addrs() {
    let t0 = base();
    let list = addrs();
    let mut backoff = Backoff::new();
    let due = due_bootstrap_redials(1, 2, &list, &mut backoff, t0);
    assert_eq!(
        due, list,
        "(1 peer, min 2) must re-dial every due addr in order"
    );
    assert_eq!(backoff.len(), list.len());
    for a in &list {
        assert_eq!(
            backoff.get(a),
            Some(&(1, t0)),
            "attempt not recorded for {a}"
        );
    }
}

// REQ-BSR-001 — Decision: a failure means a node that reached min keeps dialing seeds or keeps stale backoff state
#[test]
fn at_or_above_min_returns_empty_and_clears_backoff() {
    let t0 = base();
    let list = addrs();
    for peers in [2usize, 3, 50] {
        let mut backoff = seeded_backoff(&list, t0);
        let due = due_bootstrap_redials(peers, 2, &list, &mut backoff, at(t0, 600));
        assert!(
            due.is_empty(),
            "peers={peers} min=2 must not dial, got {due:?}"
        );
        assert!(
            backoff.is_empty(),
            "peers={peers} min=2 must clear the backoff"
        );
    }
}

// REQ-BSR-001 — Decision: a failure means the zero-peer re-dial that works today regressed under the new gate
#[test]
fn zero_peers_below_min_redials_all() {
    let t0 = base();
    let list = addrs();
    let mut backoff = Backoff::new();
    assert_eq!(due_bootstrap_redials(0, 2, &list, &mut backoff, t0), list);
    for a in &list {
        assert_eq!(backoff.get(a), Some(&(1, t0)));
    }
}

// REQ-BSR-001 — Decision: a failure means genesis and devnet nodes with min 1 changed behaviour they rely on today
#[test]
fn min_one_matches_todays_zero_peer_gate() {
    let t0 = base();
    let list = addrs();
    let mut backoff = seeded_backoff(&list, t0);
    assert!(due_bootstrap_redials(1, 1, &list, &mut backoff, at(t0, 600)).is_empty());
    assert!(backoff.is_empty());
    let mut backoff = Backoff::new();
    assert_eq!(due_bootstrap_redials(0, 1, &list, &mut backoff, t0), list);
}

// REQ-BSR-001 — Decision: a failure means test nodes with min 0 either never re-dial at zero peers or dial while connected
#[test]
fn min_zero_is_treated_as_one() {
    let t0 = base();
    let list = addrs();
    let mut backoff = Backoff::new();
    assert_eq!(due_bootstrap_redials(0, 0, &list, &mut backoff, t0), list);
    let mut backoff = seeded_backoff(&list, t0);
    assert!(due_bootstrap_redials(1, 0, &list, &mut backoff, at(t0, 600)).is_empty());
    assert!(backoff.is_empty());
}

// REQ-BSR-001 — Decision: a failure means an extreme min value overflows or inverts the threshold and silences the re-dial
#[test]
fn extreme_peer_and_min_values_do_not_overflow() {
    let t0 = base();
    let list = addrs();
    let mut backoff = Backoff::new();
    assert_eq!(
        due_bootstrap_redials(0, usize::MAX, &list, &mut backoff, t0),
        list
    );
    let mut backoff = seeded_backoff(&list, t0);
    let due = due_bootstrap_redials(usize::MAX, usize::MAX, &list, &mut backoff, at(t0, 600));
    assert!(due.is_empty());
    assert!(backoff.is_empty());
}

// REQ-BSR-002 — Decision: a failure means a stuck node dials faster than the existing backoff allows (dial storm) or slower (late recovery)
#[test]
fn stuck_node_follows_doubling_backoff_with_60s_cap() {
    let t0 = base();
    let list = vec!["/ip4/127.0.0.1/tcp/30300".to_string()];
    let mut backoff = Backoff::new();
    // Gaps after the first attempt are min(60, 1 << count): 2, 4, 8, 16, 32, 60, 60, 60.
    let attempts = [0u64, 2, 6, 14, 30, 62, 122, 182, 242];
    let mut prev: Option<u64> = None;
    for (i, &t) in attempts.iter().enumerate() {
        if let Some(p) = prev {
            let early = due_bootstrap_redials(1, 2, &list, &mut backoff, at(t0, t - 1));
            assert!(
                early.is_empty(),
                "attempt {i} fired 1 s early at +{}",
                t - 1
            );
            let edge = at(t0, t) - Duration::from_millis(1);
            assert!(due_bootstrap_redials(1, 2, &list, &mut backoff, edge).is_empty());
            assert_eq!(
                backoff[&list[0]],
                (i as u32, at(t0, p)),
                "not-due call mutated state"
            );
        }
        let due = due_bootstrap_redials(1, 2, &list, &mut backoff, at(t0, t));
        assert_eq!(due, list, "attempt {i} expected at +{t}s");
        assert_eq!(backoff[&list[0]], ((i + 1) as u32, at(t0, t)));
        prev = Some(t);
    }
}

// REQ-BSR-002 — Decision: a failure means a node that briefly reached min and then lost a peer waits a stale backoff instead of one tick
#[test]
fn falling_below_min_after_reaching_it_redials_next_tick() {
    let t0 = base();
    let list = addrs();
    let mut backoff = Backoff::new();
    for t in [0u64, 2, 6, 14, 30, 62] {
        assert_eq!(
            due_bootstrap_redials(1, 2, &list, &mut backoff, at(t0, t)),
            list
        );
    }
    assert!(due_bootstrap_redials(2, 2, &list, &mut backoff, at(t0, 63)).is_empty());
    assert!(backoff.is_empty());
    assert_eq!(
        due_bootstrap_redials(1, 2, &list, &mut backoff, at(t0, 64)),
        list
    );
}

// REQ-BSR-004 — Decision: a failure means healthy nodes at or above min send extra Connect commands and today's success path changed
#[test]
fn healthy_ticks_never_return_dial_targets() {
    let t0 = base();
    let list = addrs();
    let mut backoff = Backoff::new();
    for tick in 0..120u64 {
        let peers = 2 + (tick as usize % 4);
        let due = due_bootstrap_redials(peers, 2, &list, &mut backoff, at(t0, tick));
        assert!(
            due.is_empty(),
            "tick {tick} with {peers} peers dialed {due:?}"
        );
        assert!(backoff.is_empty());
    }
}

// REQ-BSR-005 — Decision: a failure means dial targets come from outside the operator's bootstrap list or lose their configured order
#[test]
fn dial_targets_are_an_ordered_subset_of_the_configured_list() {
    let t0 = base();
    let list = addrs();
    let mut backoff = Backoff::new();
    backoff.insert(list[1].clone(), (3, t0));
    let due = due_bootstrap_redials(1, 2, &list, &mut backoff, at(t0, 1));
    assert_eq!(due, vec![list[0].clone(), list[2].clone()]);
    assert_eq!(
        backoff[&list[1]],
        (3, t0),
        "a not-due entry must stay untouched"
    );
    assert!(backoff.keys().all(|k| list.contains(k)));
}

// REQ-BSR-005 — Decision: a failure means the function invents dial targets when the operator configured none
#[test]
fn empty_bootstrap_list_dials_nothing() {
    let mut backoff = Backoff::new();
    assert!(due_bootstrap_redials(1, 2, &[], &mut backoff, base()).is_empty());
    assert!(backoff.is_empty());
}

// REQ-BSR-005 — Decision: a failure means operator-supplied addresses are rewritten, parsed, or dropped instead of passed through unchanged
#[test]
fn unusual_address_strings_pass_through_verbatim() {
    let list = vec![
        String::new(),
        "/dns4/sëed-ü.example/tcp/1".to_string(),
        "; rm -rf / $(whoami) %s%s ../../etc/passwd\x00\r\n".to_string(),
        "x".repeat(10_000),
    ];
    let mut backoff = Backoff::new();
    assert_eq!(
        due_bootstrap_redials(0, 2, &list, &mut backoff, base()),
        list
    );
    assert!(backoff.keys().all(|k| list.contains(k)));
}

// REQ-BSR-006 — Decision: a failure means the extracted module regrew past the 200-line budget that motivated moving it out of periodic.rs
#[test]
fn bootstrap_redial_module_stays_under_200_lines() {
    let lines = include_str!("bootstrap_redial.rs").lines().count();
    assert!(lines < 200, "bootstrap_redial.rs has {lines} lines");
}

// REQ-BSR-009 — Decision: a failure means operators cannot see in INFO logs that a node below min is still re-dialing its seeds
#[test]
fn round_message_has_exact_operator_format() {
    assert_eq!(
        redial_round_message(1, 2, 3).as_deref(),
        Some("[BOOTSTRAP_REDIAL] peers=1/2 dialing 3 addr(s)")
    );
    assert_eq!(
        redial_round_message(0, 0, 2).as_deref(),
        Some("[BOOTSTRAP_REDIAL] peers=0/1 dialing 2 addr(s)")
    );
}

// REQ-BSR-009 — Decision: a failure means rounds that dial nothing spam INFO logs on healthy or backed-off nodes
#[test]
fn round_message_is_silent_when_nothing_is_dialed() {
    assert_eq!(redial_round_message(1, 2, 0), None);
    assert_eq!(redial_round_message(2, 2, 0), None);
    assert_eq!(redial_round_message(0, 0, 0), None);
}
