use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Bootstrap addrs to dial this tick while below the production peer minimum; backoff doubles to a 60 s cap.
pub fn due_bootstrap_redials(
    peer_count: usize,
    min_peers: usize,
    addrs: &[String],
    backoff: &mut HashMap<String, (u32, Instant)>,
    now: Instant,
) -> Vec<String> {
    let threshold = min_peers.max(1);
    if peer_count >= threshold {
        backoff.clear();
        return Vec::new();
    }
    let mut due = Vec::new();
    for addr in addrs {
        let (count, last) = backoff
            .entry(addr.clone())
            .or_insert((0, now - Duration::from_secs(300)));
        let backoff_secs = std::cmp::min(60, 1u64 << (*count).min(6));
        if now.saturating_duration_since(*last) >= Duration::from_secs(backoff_secs) {
            *last = now;
            *count = count.saturating_add(1);
            due.push(addr.clone());
        }
    }
    due
}

/// INFO line for a re-dial round; `None` when the round dials nothing.
pub fn redial_round_message(peer_count: usize, min_peers: usize, dialing: usize) -> Option<String> {
    if dialing == 0 {
        return None;
    }
    Some(format!(
        "[BOOTSTRAP_REDIAL] peers={}/{} dialing {} addr(s)",
        peer_count,
        min_peers.max(1),
        dialing
    ))
}
